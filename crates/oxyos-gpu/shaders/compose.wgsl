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
    // profile's premultiplied pixel for its distance from `shadow_*`, not within `clip_*`; 4: a
    // material area (docs/adr/0069-the-platforms-material.mdx), its blurred cells `src_width` ×
    // `src_height` from word `src_offset` of `scratch`, the first at (`shadow_x`, `shadow_y`) on
    // the output (`material`).
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
    // A client's GPU buffer the layer is drawn from (docs/adr/0031-gpu-buffers.mdx): its slot in
    // `images`, plus one; 0, its pixels are in `src`.
    image: u32,
    // A mesh the layer is drawn through (`src/warp.rs`, the genie): its rows' count (0: none) and
    // where they start in `src`, in words, four each (height, left, right, source height, in
    // sixteenths of a pixel, as `Warp::fixed` gives them).
    warp_rows: u32,
    warp_offset: u32,
}

// The region of the output a dispatch composes (on the tiles' grid), the output's size, its
// background and how many layers `layers` holds, bottom to top, from entry `first`; where `bins`
// starts, in words, how many bins make a row of the output, and how many words each has; and where
// the pixels go: 0, the output (`dst`); 1, a material area's backdrop in `scratch`, the rectangle
// (`into_x`, `into_y`) `into_width` × `into_height` of the output, row by row from word
// `into_offset`, nothing outside it written.
struct Frame {
    base_x: u32,
    base_y: u32,
    count_x: u32,
    count_y: u32,
    out_width: u32,
    out_height: u32,
    background: u32,
    layers: u32,
    bins_offset: u32,
    bins_across: u32,
    bin_words: u32,
    first: u32,
    into: u32,
    into_x: u32,
    into_y: u32,
    into_width: u32,
    into_height: u32,
    into_offset: u32,
}

var<push_constant> frame: Frame;
@group(0) @binding(0) var<storage, read> src: array<u32>;
@group(0) @binding(1) var<storage, read_write> dst: array<u32>;
// The frame's layers, bottom to top: each one's `base_*` and `count_*` the part of it drawn.
@group(0) @binding(2) var<storage, read> layers: array<Layer>;
// The same buffer, from its start: after the layers, each bin's layers (`vulkan::Bins`), a bit
// each, bottom to top, those under the topmost one covering the whole bin with an opaque copy or
// a fill at full opacity left out (what is under it is never seen there).
@group(0) @binding(4) var<storage, read> bins: array<u32>;
// The material areas' working pixels (`vulkan::Blur`): a backdrop composed afresh, and each area's
// cells as they are blurred, 0x00RRGGBB each.
@group(0) @binding(5) var<storage, read_write> scratch: array<u32>;
//@images-binding

// Pixel (x, y) of layer `l`'s pixels, 0xAARRGGBB: from `src`, or from the client's GPU buffer
// (an 8-bit UNORM image, so each channel loads as a level / 255 and rounds back exactly).
fn texel(l: Layer, x: u32, y: u32) -> u32 {
    //@images-texel
    return src[l.src_offset + y * l.src_width + x];
}

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
fn shadow_entry(l: Layer, x: u32, y: u32, entries: u32) -> u32 {
    let radius = l.shadow_radius;
    let qx = beyond(x, l.shadow_x, l.shadow_width, radius);
    let qy = beyond(y, l.shadow_y, l.shadow_height, radius);
    var distance = 4 * max(qx, qy);
    if (qx > 0 && qy > 0) {
        distance = i32(isqrt(u32(qx * qx + qy * qy) << 4u));
    }
    distance -= 8 * i32(radius);
    let steps = clamp(distance, -(1 << 18u), 1 << 18u) * 256 + i32(entries / 2u * l.scale);
    if (steps <= 0) {
        return 0u;
    }
    return min(u32(steps) / l.scale, entries - 1u);
}

// `beneath` in `compose_cpu`: how much of a shadow's pixel what it is beneath covers.
fn beneath(l: Layer, x: u32, y: u32) -> u32 {
    let cx = i32(x) - l.clip_x;
    let cy = i32(y) - l.clip_y;
    if (cx < 0 || cy < 0 || cx >= i32(l.clip_width) || cy >= i32(l.clip_height)) {
        return 0u;
    }
    return corner_coverage(u32(cx), u32(cy), l.clip_width, l.clip_height, l.clip_radius);
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
fn sample(l: Layer, lx: u32, ly: u32, opaque: bool) -> u32 {
    let columns = taps(lx, l.width, l.src_width);
    let rows = taps(ly, l.height, l.src_height);
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
            let pixel = texel(l, x, y);
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

// `warp::SUB`, sixteenths of a pixel; the widest half box (`warp::MAX_HALF`).
const SUB: i32 = 16;
const MAX_HALF: i32 = 512;

// Row `r` of layer `l`'s mesh: height, left, right, source height.
fn warp_row(l: Layer, r: u32) -> vec4<i32> {
    let at = l.warp_offset + 4u * r;
    return vec4<i32>(i32(src[at]), i32(src[at + 1u]), i32(src[at + 2u]), i32(src[at + 3u]));
}

// `warp::Fall`: coverage 0 is none.
struct Fall {
    coverage: u32,
    x: i32,
    y: i32,
    half_x: i32,
    half_y: i32,
}

// `between` in `warp::fall`: `share` 4096ths of the way from `start` to `end`.
fn between(start: i32, end: i32, share: u32) -> i32 {
    return start + (((end - start) * i32(share) + 2048) >> 12u);
}

// `warp::span` and `Span::fall`, for one pixel.
fn fall(l: Layer, lx: u32, ly: u32) -> Fall {
    let none = Fall(0u, 0, 0, 0, 0);
    let count = l.warp_rows;
    let top = warp_row(l, 0u).x;
    let bottom = warp_row(l, count - 1u).x;
    let centre_y = i32(ly) * SUB + SUB / 2;
    let covered_y = clamp(min(centre_y + SUB / 2, bottom) - max(centre_y - SUB / 2, top), 0, SUB);
    if (covered_y == 0) {
        return none;
    }
    let y = clamp(centre_y, top, bottom - 1);
    var upper = 0u;
    var lower = count - 1u;
    while (lower - upper > 1u) {
        let middle = (upper + lower) / 2u;
        if (warp_row(l, middle).x <= y) {
            upper = middle;
        } else {
            lower = middle;
        }
    }
    let a = warp_row(l, upper);
    let b = warp_row(l, lower);
    let down = u32(b.x - a.x);
    let share = (u32(y - a.x) << 12u) / down;
    let left = between(a.y, b.y, share);
    let right = between(a.z, b.z, share);
    let centre_x = i32(lx) * SUB + SUB / 2;
    let covered_x = clamp(min(centre_x + SUB / 2, right) - max(centre_x - SUB / 2, left), 0, SUB);
    if (covered_x == 0) {
        return none;
    }
    let across = u32(right - left);
    let rate = (l.src_width << 16u) / across;
    let x = i32((u32(clamp(centre_x, left, right) - left) * rate) >> 12u);
    let half_x = clamp(i32(rate >> 9u), SUB / 2, MAX_HALF);
    let half_y = clamp((b.w - a.w) * (SUB / 2) / i32(down), SUB / 2, MAX_HALF);
    return Fall((u32(covered_x * covered_y) * 255u + 128u) >> 8u, x, between(a.w, b.w, share), half_x, half_y);
}

// `warp::taps`: the first and last source pixels, and the box's start.
fn warp_taps(centre: i32, half: i32, extent: u32) -> vec3<i32> {
    let low = centre - half;
    let high = centre + half;
    var first = 0u;
    if (low > 0) {
        first = min(u32(low) / u32(SUB), extent - 1u);
    }
    let last = min(u32(max(high - 1, 0)) / u32(SUB), extent - 1u);
    return vec3<i32>(i32(first), i32(last), low);
}

// `warp::through`.
fn warp_through(at: u32, last: u32, low: i32, half: i32) -> u32 {
    if (at == last) {
        return ONE;
    }
    let end = (i32(at) + 1) * SUB - low;
    return (u32(end) * ONE + u32(half)) / (2u * u32(half));
}

// `warp::blend_rows` and `warp::sample_columns` (the same sums, in the other order): premultiplied,
// its alpha in the high byte.
fn warp_sample(l: Layer, f: Fall, opaque: bool) -> u32 {
    let columns = warp_taps(f.x, f.half_x, l.src_width);
    let rows = warp_taps(f.y, f.half_y, l.src_height);
    var sum = vec4<u32>(0u);
    var above = 0u;
    for (var y = u32(rows.x); y <= u32(rows.y); y++) {
        let to_y = warp_through(y, u32(rows.y), rows.z, f.half_y);
        let wy = to_y - above;
        above = to_y;
        var row = vec4<u32>(0u);
        var before = 0u;
        for (var x = u32(columns.x); x <= u32(columns.y); x++) {
            let to_x = warp_through(x, u32(columns.y), columns.z, f.half_x);
            let wx = to_x - before;
            before = to_x;
            let pixel = texel(l, x, y);
            var alpha = pixel >> 24u;
            if (opaque) {
                alpha = 255u;
            }
            row += wx * vec4<u32>((pixel >> 16u) & 0xffu, (pixel >> 8u) & 0xffu, pixel & 0xffu, alpha);
        }
        sum += wy * row;
    }
    let rounded = (sum + vec4<u32>(1u << 23u)) >> vec4<u32>(24u);
    return (rounded.w << 24u) | (rounded.x << 16u) | (rounded.y << 8u) | rounded.z;
}

// `warp::corner`.
fn warp_corner(l: Layer, f: Fall) -> u32 {
    if (l.radius == 0u) {
        return 255u;
    }
    let px = min(u32(max(f.x, 0) / SUB), l.src_width - 1u);
    let py = min(u32(max(f.y, 0) / SUB), l.src_height - 1u);
    return corner_coverage(px, py, l.src_width, l.src_height, l.radius);
}

fn channel(source: u32, under: u32, keep: u32, shift: u32, opacity: u32) -> u32 {
    let s = ((source >> shift) & 0xffu) * opacity / 255u;
    let d = (under >> shift) & 0xffu;
    return min(s + d * keep / 255u, 255u) << shift;
}

// How much of pixel (lx, ly) of `layer` it covers: all of it, but at its rounded corners and, a
// shadow, within what it is beneath.
fn coverage_of(l: Layer, lx: u32, ly: u32) -> u32 {
    var coverage = corner_coverage(lx, ly, l.width, l.height, l.radius);
    if (l.mode == 3u) {
        coverage = coverage * (255u - beneath(l, lx, ly)) / 255u;
    }
    return coverage;
}

// `under` with pixel (lx, ly) of `layer` over it, `coverage` of it.
fn blend(l: Layer, lx: u32, ly: u32, coverage: u32, under: u32) -> u32 {
    // The pixel, premultiplied, with its alpha (opaque ones and fills have 255).
    var source = l.color & 0xffffffu;
    var alpha = 255u;
    if (l.mode == 3u) {
        let pixel = src[l.src_offset + shadow_entry(l, lx, ly, l.src_width)];
        source = pixel & 0xffffffu;
        alpha = pixel >> 24u;
    } else if (l.mode != 2u) {
        // Nearest neighbour, as the reference: the source pixel under the output pixel's corner;
        // or filtered.
        var pixel = 0u;
        if (l.width == l.src_width && l.height == l.src_height) {
            // At its own size, filtered or not: its pixel there (`sample` with one tap).
            pixel = texel(l, lx, ly);
        } else if (l.sampling == 1u) {
            pixel = sample(l, lx, ly, l.mode == 0u);
        } else {
            let sx = lx * l.src_width / l.width;
            let sy = ly * l.src_height / l.height;
            pixel = texel(l, sx, sy);
        }
        source = pixel & 0xffffffu;
        if (l.mode == 1u) {
            alpha = pixel >> 24u;
        }
    }
    // Scaled by the layer's opacity at this pixel (its opacity by its coverage), then
    // source-over: at 255 an opaque pixel is copied and a premultiplied one blended, as without
    // an opacity.
    let opacity = l.opacity * coverage / 255u;
    let keep = 255u - alpha * opacity / 255u;
    return channel(source, under, keep, 16u, opacity) | channel(source, under, keep, 8u, opacity) | channel(source, under, keep, 0u, opacity);
}

// Where output pixel (x, y) falls in the part of layer `l` drawn, if it does: (lx, ly, 1); else 0.
fn within(l: Layer, x: u32, y: u32) -> vec3<u32> {
    let lx = i32(x) - l.x;
    let ly = i32(y) - l.y;
    if (lx < i32(l.base_x) || ly < i32(l.base_y)
        || lx >= i32(l.base_x + l.count_x) || ly >= i32(l.base_y + l.count_y)) {
        return vec3<u32>(0u);
    }
    return vec3<u32>(u32(lx), u32(ly), 1u);
}

// A workgroup composes a tile of TILE_WIDTH × TILE_HEIGHT pixels (`vulkan::TILE`) on the tiles'
// grid, which a bin of BIN_WIDTH × BIN_HEIGHT pixels (`vulkan::BIN`) holds whole: each pixel takes
// the layers of its tile's bin, in a loop the same for every pixel of the tile. A row of a SIMD
// thread's lanes is a whole cache line of the output.
const TILE_WIDTH: u32 = 16u;
const TILE_HEIGHT: u32 = 8u;
const BIN_WIDTH: u32 = 32u;
const BIN_HEIGHT: u32 = 32u;

// `t / 255`, rounded down, for each of two fields of 16 bits (bits 0 and 16), each at most
// 255 × 255: `channel`'s divisions, two channels at a time.
fn by_255(t: u32) -> u32 {
    return ((t + 0x00010001u + ((t >> 8u) & 0x00ff00ffu)) >> 8u) & 0x00ff00ffu;
}

// `pixel` (premultiplied, alpha `alpha`) at `opacity` over `under`: `blend`'s arithmetic, red and
// blue at once and green apart, each channel at most 255 (a premultiplied channel over its alpha
// would pass it).
fn source_over(pixel: u32, alpha: u32, opacity: u32, under: u32) -> u32 {
    if (opacity == 255u && alpha == 255u) {
        return pixel & 0xffffffu;
    }
    let keep = 255u - alpha * opacity / 255u;
    var rb = by_255((pixel & 0x00ff00ffu) * opacity) + by_255((under & 0x00ff00ffu) * keep);
    var g = by_255(((pixel >> 8u) & 0xffu) * opacity) + by_255(((under >> 8u) & 0xffu) * keep);
    rb |= ((rb >> 8u) & 0x00010001u) * 0xffu;
    g |= ((g >> 8u) & 1u) * 0xffu;
    return (rb & 0x00ff00ffu) | ((g & 0xffu) << 8u);
}

// The material (docs/adr/0069-the-platforms-material.mdx), as `src/material.rs` makes it, operation
// for operation: the backdrop composed afresh into `scratch` (`main`, `into` 1), averaged into cells
// of CELL × CELL pixels (`cells`), blurred by three box passes along rows and three along columns
// (`box_blur`), then, for each pixel of the area as the frame is composed, sampled bilinearly from
// the cells, saturated and drawn over what is beneath (`material`).
const CELL: u32 = 4u;

// A cell, 0x00RRGGBB, as its three channels.
fn channels(cell: u32) -> vec3<u32> {
    return vec3<u32>((cell >> 16u) & 0xffu, (cell >> 8u) & 0xffu, cell & 0xffu);
}

fn pack_rgb(value: vec3<u32>) -> u32 {
    return (value.x << 16u) | (value.y << 8u) | value.z;
}

// `material::blurred`'s bilinear axis: for pixel `p` of the output, the two cells (of `count`
// from the one at `origin`) whose centres are either side of its centre, and the second's weight
// in eighths of a cell.
fn cell_axis(p: u32, origin: u32, count: u32) -> vec3<u32> {
    let local = p - origin;
    let cell = i32(local / CELL);
    // Eighths, −3 ..= 3.
    let offset = 2 * i32(local % CELL) - 3;
    var a = cell;
    var b = cell + 1;
    var wb = offset;
    if (offset < 0) {
        a = cell - 1;
        b = cell;
        wb = 8 + offset;
    }
    let last = i32(count) - 1;
    return vec3<u32>(u32(clamp(a, 0, last)), u32(clamp(b, 0, last)), u32(wb));
}

// One of a material area's cells, as its channels.
fn cell_at(l: Layer, column: u32, row: u32) -> vec3<i32> {
    return vec3<i32>(channels(scratch[l.src_offset + row * l.src_width + column]));
}

// `material::blurred` at output pixel (x, y): bilinear in eighths, rounded, then saturated by
// `material::SATURATION` in 4096ths and clamped.
fn blurred(l: Layer, x: u32, y: u32) -> u32 {
    let ax = cell_axis(x, u32(l.shadow_x), l.src_width);
    let ay = cell_axis(y, u32(l.shadow_y), l.src_height);
    let wx = i32(ax.z);
    let wy = i32(ay.z);
    let top = cell_at(l, ax.x, ay.x) * (8 - wx) + cell_at(l, ax.y, ay.x) * wx;
    let bottom = cell_at(l, ax.x, ay.y) * (8 - wx) + cell_at(l, ax.y, ay.y) * wx;
    let rgb = (top * (8 - wy) + bottom * wy + vec3<i32>(32)) / vec3<i32>(64);
    let red = (6677 * rgb.x - 2344 * rgb.y - 237 * rgb.z + 2048) >> 12u;
    let green = (-697 * rgb.x + 5030 * rgb.y - 237 * rgb.z + 2048) >> 12u;
    let blue = (-697 * rgb.x - 2344 * rgb.y + 7137 * rgb.z + 2048) >> 12u;
    return pack_rgb(vec3<u32>(clamp(vec3<i32>(red, green, blue), vec3<i32>(0), vec3<i32>(255))));
}

// `material::draw` at output pixel (x, y), pixel (lx, ly) of the area: the blurred backdrop over
// `under` by the area's rounded corners' coverage and its opacity.
fn material(l: Layer, lx: u32, ly: u32, x: u32, y: u32, under: u32) -> u32 {
    let alpha = corner_coverage(lx, ly, l.width, l.height, l.radius) * l.opacity / 255u;
    if (alpha == 0u) {
        return under;
    }
    let blur = channels(blurred(l, x, y));
    let below = channels(under);
    return pack_rgb((blur * alpha + below * (255u - alpha) + vec3<u32>(127u)) / vec3<u32>(255u));
}

// Pixel (x, y) with layer `l` over `color`.
fn over(l: Layer, x: u32, y: u32, color: u32) -> u32 {
    let at = within(l, x, y);
    if (at.z == 0u) {
        return color;
    }
    if (l.mode == 4u) {
        return material(l, at.x, at.y, x, y, color);
    }
    // A copy at its own size through no mesh (most of a frame: windows, the shell's surfaces),
    // filtered or not its pixel there: `blend` for it, without what it does not need.
    if (l.warp_rows == 0u && l.mode <= 1u && l.width == l.src_width && l.height == l.src_height) {
        var coverage = 255u;
        if (l.radius != 0u) {
            coverage = corner_coverage(at.x, at.y, l.width, l.height, l.radius);
            if (coverage == 0u) {
                return color;
            }
        }
        let pixel = texel(l, at.x, at.y);
        var alpha = 255u;
        if (l.mode == 1u) {
            alpha = pixel >> 24u;
        }
        return source_over(pixel, alpha, l.opacity * coverage / 255u, color);
    }
    // Through its mesh: the part of the pixel it covers, sampled where it falls.
    if (l.warp_rows != 0u) {
        let f = fall(l, at.x, at.y);
        let covered = f.coverage * warp_corner(l, f) / 255u;
        if (covered == 0u) {
            return color;
        }
        let pixel = warp_sample(l, f, l.mode == 0u);
        var alpha = 255u;
        if (l.mode == 1u) {
            alpha = pixel >> 24u;
        }
        let opacity = l.opacity * covered / 255u;
        let keep = 255u - alpha * opacity / 255u;
        return channel(pixel, color, keep, 16u, opacity) | channel(pixel, color, keep, 8u, opacity) | channel(pixel, color, keep, 0u, opacity);
    }
    let coverage = coverage_of(l, at.x, at.y);
    if (coverage == 0u) {
        return color;
    }
    return blend(l, at.x, at.y, coverage, color);
}

@compute @workgroup_size(16, 8)
fn main(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_id) local: vec3<u32>) {
    let x0 = frame.base_x + group.x * TILE_WIDTH;
    let y0 = frame.base_y + group.y * TILE_HEIGHT;
    let x = x0 + local.x;
    let y = y0 + local.y;
    if (x >= min(frame.base_x + frame.count_x, frame.out_width) || y >= min(frame.base_y + frame.count_y, frame.out_height)) {
        return;
    }
    if (frame.into == 1u && (x < frame.into_x || y < frame.into_y
        || x >= frame.into_x + frame.into_width || y >= frame.into_y + frame.into_height)) {
        return;
    }
    let bin = frame.bins_offset + ((y0 / BIN_HEIGHT) * frame.bins_across + x0 / BIN_WIDTH) * frame.bin_words;
    var color = frame.background & 0xffffffu;
    for (var word = 0u; word < frame.bin_words; word++) {
        var bits = bins[bin + word];
        while (bits != 0u) {
            // The layer read whole at once, the same for every pixel of the tile: a few wide
            // reads, where a read of each field as it is needed is one each, which a device at its
            // efficient clock waits on (Intel's, at 300 MHz) more than on the composing.
            let l = layers[frame.first + word * 32u + countTrailingZeros(bits)];
            bits &= bits - 1u;
            color = over(l, x, y, color);
        }
    }
    if (frame.into == 1u) {
        scratch[frame.into_offset + (y - frame.into_y) * frame.into_width + x - frame.into_x] = color;
        return;
    }
    dst[y * frame.out_width + x] = color;
}

// A material area's pass over its cells (`vulkan::Blur`): from word `read_at` of `scratch` to word
// `write_at`, `cols` × `rows` cells; the backdrop's rectangle, `src_width` × `src_height` pixels (`cells`);
// a box `width` cells wide, `before` of them before the cell, along rows (`axis` 0) or columns (1)
// (`box_blur`).
struct BlurPass {
    read_at: u32,
    write_at: u32,
    cols: u32,
    rows: u32,
    src_width: u32,
    src_height: u32,
    width: u32,
    before: u32,
    axis: u32,
}

var<push_constant> blur_pass: BlurPass;

// `material::blurred`'s first step: each cell the rounded average of the backdrop's pixels under
// it, fewer at the output's edge.
@compute @workgroup_size(8, 8)
fn cells(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= blur_pass.cols || id.y >= blur_pass.rows) {
        return;
    }
    let x0 = id.x * CELL;
    let y0 = id.y * CELL;
    let x1 = min(x0 + CELL, blur_pass.src_width);
    let y1 = min(y0 + CELL, blur_pass.src_height);
    var sum = vec3<u32>(0u);
    for (var y = y0; y < y1; y++) {
        for (var x = x0; x < x1; x++) {
            sum += channels(scratch[blur_pass.read_at + y * blur_pass.src_width + x]);
        }
    }
    let count = (x1 - x0) * (y1 - y0);
    scratch[blur_pass.write_at + id.y * blur_pass.cols + id.x] = pack_rgb((sum + vec3<u32>(count / 2u)) / vec3<u32>(count));
}

// `material::box_pass` at one cell: the rounded average of the `width` cells around it along its
// line, the line's edge cells continuing beyond it.
@compute @workgroup_size(8, 8)
fn box_blur(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= blur_pass.cols || id.y >= blur_pass.rows) {
        return;
    }
    var count = blur_pass.cols;
    var at = id.x;
    if (blur_pass.axis == 1u) {
        count = blur_pass.rows;
        at = id.y;
    }
    let first = i32(at) - i32(blur_pass.before);
    let last = first + i32(blur_pass.width) - 1;
    var sum = vec3<u32>(0u);
    for (var i = first; i <= last; i++) {
        let along = u32(clamp(i, 0, i32(count) - 1));
        var cell = id.y * blur_pass.cols + along;
        if (blur_pass.axis == 1u) {
            cell = along * blur_pass.cols + id.x;
        }
        sum += channels(scratch[blur_pass.read_at + cell]);
    }
    scratch[blur_pass.write_at + id.y * blur_pass.cols + id.x] = pack_rgb((sum + vec3<u32>(blur_pass.width / 2u)) / vec3<u32>(blur_pass.width));
}
