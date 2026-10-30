// A frame composed into the output (docs/adr/0004), pixel by pixel: each pixel of the region a
// dispatch covers takes the background and then every layer over it, bottom to top, with the
// same arithmetic as `compose_cpu` (and, for corners and shadows, `src/shape.rs`; for filtered
// sampling, `src/filter.rs`), so the two agree byte for byte; it is written once. Layers under
// the topmost one that covers the pixel whole, opaque, are never seen there and are skipped.
// Pixels are 0x00RRGGBB in the output and 0xAARRGGBB (B, G, R, A in memory) in a layer.

struct Layer {
    // The layer's rectangle on the output (logical pixels), and the size of its pixels.
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    src_width: u32,
    src_height: u32,
    // Where the layer's pixels start in `src`, in words.
    src_offset: u32,
    out_width: u32,
    out_height: u32,
    // 0: opaque copy, 1: premultiplied source-over, 2: fill with `color`, 3: a shadow, its
    // profile's premultiplied pixel for its distance from `shadow_*`, not within `clip_*`.
    mode: u32,
    color: u32,
    // The part of the layer drawn (within its clip), from (base_x, base_y) within the layer,
    // count_x × count_y pixels.
    base_x: u32,
    base_y: u32,
    count_x: u32,
    count_y: u32,
    // 0 to 255: the source's premultiplied pixel is scaled by it before it is blended
    // (docs/adr/0011-transitions.mdx); at 255, as without it.
    opacity: u32,
    // The corners' radius (docs/adr/0016-windows.mdx), at most half the shorter side: 0, square.
    radius: u32,
    // A shadow's shape and what it is beneath, relative to the layer, with their radii (as
    // `radius`), and its scale in 256ths of its profile's.
    shadow_x: i32,
    shadow_y: i32,
    shadow_width: u32,
    shadow_height: u32,
    shadow_radius: u32,
    clip_x: i32,
    clip_y: i32,
    clip_width: u32,
    clip_height: u32,
    clip_radius: u32,
    scale: u32,
    // How the pixels are sampled: 1, filtered (`Layer::filtered`: `Filter::Box`, drawn at
    // another size, docs/adr/0011-transitions.mdx, step 5); 0, nearest neighbour (at 1:1, the
    // pixels as they are).
    sampling: u32,
}

// The region of the output a dispatch composes, the output's size, its background and how many
// layers `layers` holds, bottom to top.
struct Frame {
    base_x: u32,
    base_y: u32,
    count_x: u32,
    count_y: u32,
    out_width: u32,
    out_height: u32,
    background: u32,
    layers: u32,
}

var<push_constant> frame: Frame;
@group(0) @binding(0) var<storage, read> src: array<u32>;
@group(0) @binding(1) var<storage, read_write> dst: array<u32>;
// The frame's layers, bottom to top: each one's `base_*` and `count_*` the part of it drawn.
@group(0) @binding(2) var<storage, read> layers: array<Layer>;

// Half-pixel distances beyond this from a shadow's shape are taken as this (`shape::FAR`).
const FAR: i32 = 8192;

// `shape::isqrt`.
fn isqrt(n: u32) -> u32 {
    var rest = n;
    var root = 0u;
    var bit = 1u << 30u;
    for (var i = 0u; i < 16u; i++) {
        if (rest >= root + bit) {
            rest -= root + bit;
            root = (root >> 1u) + bit;
        } else {
            root >>= 1u;
        }
        bit >>= 2u;
    }
    return root;
}

// `shape::corner_coverage`'s distance toward a corner circle's centre, in half pixels.
fn toward(at: u32, length: u32, radius: u32) -> u32 {
    if (at < radius) {
        return 2u * radius - (2u * at + 1u);
    }
    if (at >= length - radius) {
        return 2u * at + 1u - 2u * (length - radius);
    }
    return 0u;
}

// `shape::corner_coverage`.
fn corner_coverage(x: u32, y: u32, width: u32, height: u32, radius: u32) -> u32 {
    let dx = toward(x, width, radius);
    let dy = toward(y, height, radius);
    if (dx == 0u || dy == 0u) {
        return 255u;
    }
    let distance = i32(isqrt((dx * dx + dy * dy) << 10u));
    let inside = u32(clamp(32 - (distance - 64 * i32(radius)), 0, 64));
    return inside * 255u / 64u;
}

// `shape::shadow_entry`'s distance beyond a side's straight part, in half pixels.
fn beyond(at: u32, start: i32, length: u32, radius: u32) -> i32 {
    let centre = 2 * start + i32(length);
    return min(abs(2 * i32(at) + 1 - centre) - i32(length - 2u * radius), FAR);
}

// `shape::shadow_entry`.
fn shadow_entry(i: u32, x: u32, y: u32, entries: u32) -> u32 {
    let radius = layers[i].shadow_radius;
    let qx = beyond(x, layers[i].shadow_x, layers[i].shadow_width, radius);
    let qy = beyond(y, layers[i].shadow_y, layers[i].shadow_height, radius);
    var distance = 4 * max(qx, qy);
    if (qx > 0 && qy > 0) {
        distance = i32(isqrt(u32(qx * qx + qy * qy) << 4u));
    }
    distance -= 8 * i32(radius);
    let steps = clamp(distance, -(1 << 18u), 1 << 18u) * 256 + i32(entries / 2u * layers[i].scale);
    if (steps <= 0) {
        return 0u;
    }
    return min(u32(steps) / layers[i].scale, entries - 1u);
}

// `beneath` in `compose_cpu`: how much of a shadow's pixel what it is beneath covers.
fn beneath(i: u32, x: u32, y: u32) -> u32 {
    let cx = i32(x) - layers[i].clip_x;
    let cy = i32(y) - layers[i].clip_y;
    if (cx < 0 || cy < 0 || cx >= i32(layers[i].clip_width) || cy >= i32(layers[i].clip_height)) {
        return 0u;
    }
    return corner_coverage(u32(cx), u32(cy), layers[i].clip_width, layers[i].clip_height, layers[i].clip_radius);
}

// `filter::ONE`: an output pixel's weights along an axis sum to it.
const ONE: u32 = 4096u;
// `filter::SNAP`.
const SNAP: u32 = 64u;

// `filter::Taps`.
struct Taps {
    first: u32,
    last: u32,
    low: i32,
    half: u32,
    length: u32,
}

// `filter::taps`.
fn taps(at: u32, length: u32, extent: u32) -> Taps {
    // Not drawn at another size beyond rounding (`filter::scaled`): nearest neighbour.
    let apart = max(extent, length) - min(extent, length);
    if (apart == 0u || (apart == 1u && max(extent, length) >= SNAP)) {
        let source = at * extent / length;
        return Taps(source, source, 0, 1u, length);
    }
    let half = max(extent, length);
    let centre = i32((2u * at + 1u) * extent);
    let low = centre - i32(half);
    let high = centre + i32(half);
    var first = 0u;
    if (low > 0) {
        first = min(u32(low) / (2u * length), extent - 1u);
    }
    let last = min(u32(high - 1) / (2u * length), extent - 1u);
    return Taps(first, last, low, half, length);
}

// `Taps::through`.
fn through(taps: Taps, at: u32) -> u32 {
    if (at == taps.last) {
        return ONE;
    }
    let end = i32(2u * (at + 1u) * taps.length) - taps.low;
    return (u32(end) * ONE + taps.half) / (2u * taps.half);
}

// `filter::sample`: the layer's pixel for output pixel (lx, ly), filtered, premultiplied, its
// alpha in the high byte; an opaque layer's alpha is 255.
fn sample(i: u32, lx: u32, ly: u32, opaque: bool) -> u32 {
    let columns = taps(lx, layers[i].width, layers[i].src_width);
    let rows = taps(ly, layers[i].height, layers[i].src_height);
    var sum = vec4<u32>(0u);
    var above = 0u;
    for (var y = rows.first; y <= rows.last; y++) {
        let to_y = through(rows, y);
        let wy = to_y - above;
        above = to_y;
        var row = vec4<u32>(0u);
        var before = 0u;
        for (var x = columns.first; x <= columns.last; x++) {
            let to_x = through(columns, x);
            let wx = to_x - before;
            before = to_x;
            let pixel = src[layers[i].src_offset + y * layers[i].src_width + x];
            var alpha = pixel >> 24u;
            if (opaque) {
                alpha = 255u;
            }
            let red = (pixel >> 16u) & 0xffu;
            let green = (pixel >> 8u) & 0xffu;
            row += wx * vec4<u32>(red, green, pixel & 0xffu, alpha);
        }
        sum += wy * row;
    }
    let rounded = (sum + vec4<u32>(1u << 23u)) >> vec4<u32>(24u);
    return (rounded.w << 24u) | (rounded.x << 16u) | (rounded.y << 8u) | rounded.z;
}

fn channel(source: u32, under: u32, keep: u32, shift: u32, opacity: u32) -> u32 {
    let s = ((source >> shift) & 0xffu) * opacity / 255u;
    let d = (under >> shift) & 0xffu;
    return min(s + d * keep / 255u, 255u) << shift;
}

// How much of pixel (lx, ly) of `layer` it covers: all of it, but at its rounded corners and, a
// shadow, within what it is beneath.
fn coverage_of(i: u32, lx: u32, ly: u32) -> u32 {
    var coverage = corner_coverage(lx, ly, layers[i].width, layers[i].height, layers[i].radius);
    if (layers[i].mode == 3u) {
        coverage = coverage * (255u - beneath(i, lx, ly)) / 255u;
    }
    return coverage;
}

// `under` with pixel (lx, ly) of `layer` over it, `coverage` of it.
fn blend(i: u32, lx: u32, ly: u32, coverage: u32, under: u32) -> u32 {
    // The pixel, premultiplied, with its alpha (opaque ones and fills have 255).
    var source = layers[i].color & 0xffffffu;
    var alpha = 255u;
    if (layers[i].mode == 3u) {
        let pixel = src[layers[i].src_offset + shadow_entry(i, lx, ly, layers[i].src_width)];
        source = pixel & 0xffffffu;
        alpha = pixel >> 24u;
    } else if (layers[i].mode != 2u) {
        // Nearest neighbour, as the reference: the source pixel under the output pixel's corner;
        // or filtered.
        var pixel = 0u;
        if (layers[i].sampling == 1u) {
            pixel = sample(i, lx, ly, layers[i].mode == 0u);
        } else {
            let sx = lx * layers[i].src_width / layers[i].width;
            let sy = ly * layers[i].src_height / layers[i].height;
            pixel = src[layers[i].src_offset + sy * layers[i].src_width + sx];
        }
        source = pixel & 0xffffffu;
        if (layers[i].mode == 1u) {
            alpha = pixel >> 24u;
        }
    }
    // Scaled by the layer's opacity at this pixel (its opacity by its coverage), then
    // source-over: at 255 an opaque pixel is copied and a premultiplied one blended, as without
    // an opacity.
    let opacity = layers[i].opacity * coverage / 255u;
    let keep = 255u - alpha * opacity / 255u;
    return channel(source, under, keep, 16u, opacity) | channel(source, under, keep, 8u, opacity) | channel(source, under, keep, 0u, opacity);
}

// Where output pixel (x, y) falls in the part of `layer` drawn, if it does: (lx, ly, 1); else 0.
fn within(i: u32, x: u32, y: u32) -> vec3<u32> {
    let lx = i32(x) - layers[i].x;
    let ly = i32(y) - layers[i].y;
    if (lx < i32(layers[i].base_x) || ly < i32(layers[i].base_y)
        || lx >= i32(layers[i].base_x + layers[i].count_x) || ly >= i32(layers[i].base_y + layers[i].count_y)) {
        return vec3<u32>(0u);
    }
    return vec3<u32>(u32(lx), u32(ly), 1u);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= frame.count_x || id.y >= frame.count_y) {
        return;
    }
    let x = frame.base_x + id.x;
    let y = frame.base_y + id.y;
    if (x >= frame.out_width || y >= frame.out_height) {
        return;
    }
    // The topmost layer covering the pixel whole with an opaque copy or a fill at full opacity:
    // the pixel takes its colour whatever is under it, so the layers under it are skipped.
    var first = 0u;
    for (var above = frame.layers; above > 0u; above--) {
        let i = above - 1u;
        if ((layers[i].mode != 0u && layers[i].mode != 2u) || layers[i].opacity != 255u) {
            continue;
        }
        let at = within(i, x, y);
        if (at.z == 1u && corner_coverage(at.x, at.y, layers[i].width, layers[i].height, layers[i].radius) == 255u) {
            first = i;
            break;
        }
    }
    var color = frame.background & 0xffffffu;
    for (var i = first; i < frame.layers; i++) {
        let at = within(i, x, y);
        if (at.z == 0u) {
            continue;
        }
        let coverage = coverage_of(i, at.x, at.y);
        if (coverage == 0u) {
            continue;
        }
        color = blend(i, at.x, at.y, coverage, color);
    }
    dst[y * frame.out_width + x] = color;
}
