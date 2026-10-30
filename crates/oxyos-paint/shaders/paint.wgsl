// The Bloom host's painter on the GPU (docs/adr/0031-gpu-buffers.mdx, decision 12): each operation
// of tools/rn-host-spike/renderer.cpp's CPU painter, one dispatch over the device pixels it may
// touch, with the same arithmetic per pixel (its coverage, its sampling, its rounding), so the two
// painters look the same. Canvases, pictures, glyph masks and float planes all live in one heap of
// words; a canvas pixel is 0xAARRGGBB, premultiplied, as the CPU's Canvas.

@group(0) @binding(0) var<storage, read_write> heap: array<u32>;
@group(0) @binding(1) var<storage, read> ops: array<u32>;

struct Frame {
    // Where the operation's record starts in `ops`.
    op: u32,
}
var<push_constant> frame: Frame;

// Header words of every record.
const KIND: u32 = 0u;
const CANVAS: u32 = 1u;
const CANVAS_WIDTH: u32 = 2u;
const CANVAS_HEIGHT: u32 = 3u;
const X0: u32 = 4u;
const Y0: u32 = 5u;
const X1: u32 = 6u;
const Y1: u32 = 7u;
const SCALE: u32 = 8u;
const CLIP: u32 = 9u;
const COLOR: u32 = 13u;
const OPACITY: u32 = 17u;
const ARGS: u32 = 18u;

fn word_at(at: u32) -> u32 {
    return ops[frame.op + at];
}
fn int_at(at: u32) -> i32 {
    return bitcast<i32>(ops[frame.op + at]);
}
fn real_at(at: u32) -> f32 {
    return bitcast<f32>(ops[frame.op + at]);
}

// std::lround of a value at or above zero.
fn lround(value: f32) -> u32 {
    return u32(floor(value + 0.5));
}

struct Rgba {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

fn color() -> Rgba {
    return Rgba(real_at(COLOR), real_at(COLOR + 1u), real_at(COLOR + 2u), real_at(COLOR + 3u));
}

struct Radii {
    tl: f32,
    tr: f32,
    bl: f32,
    br: f32,
}

fn radii(at: u32) -> Radii {
    return Radii(real_at(at), real_at(at + 1u), real_at(at + 2u), real_at(at + 3u));
}

fn rounded(r: Radii) -> bool {
    return r.tl > 0.0 || r.tr > 0.0 || r.bl > 0.0 || r.br > 0.0;
}

// Painter::insideRounded.
fn inside_rounded(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, r: Radii) -> bool {
    if (px < x || py < y || px > x + w || py > y + h) {
        return false;
    }
    let limit = min(w, h) / 2.0;
    let tl = min(r.tl, limit);
    let tr = min(r.tr, limit);
    let bl = min(r.bl, limit);
    let br = min(r.br, limit);
    if (px < x + tl && py < y + tl) {
        let dx = px - (x + tl);
        let dy = py - (y + tl);
        return dx * dx + dy * dy <= tl * tl;
    }
    if (px > x + w - tr && py < y + tr) {
        let dx = px - (x + w - tr);
        let dy = py - (y + tr);
        return dx * dx + dy * dy <= tr * tr;
    }
    if (px < x + bl && py > y + h - bl) {
        let dx = px - (x + bl);
        let dy = py - (y + h - bl);
        return dx * dx + dy * dy <= bl * bl;
    }
    if (px > x + w - br && py > y + h - br) {
        let dx = px - (x + w - br);
        let dy = py - (y + h - br);
        return dx * dx + dy * dy <= br * br;
    }
    return true;
}

// Painter::coveredWhole.
fn covered_whole(lx: f32, ly: f32, s: f32, x: f32, y: f32, w: f32, h: f32, r: Radii) -> bool {
    if (lx < x || ly < y || lx + s > x + w || ly + s > y + h) {
        return false;
    }
    let limit = min(w, h) / 2.0;
    let tl = min(r.tl, limit);
    let tr = min(r.tr, limit);
    let bl = min(r.bl, limit);
    let br = min(r.br, limit);
    return (lx >= x + tl || ly >= y + tl) && (lx + s <= x + w - tr || ly >= y + tr)
        && (lx >= x + bl || ly + s <= y + h - bl) && (lx + s <= x + w - br || ly + s <= y + h - br);
}

// Painter::roundedCoverage: the 4x4 samples, without sampling where they all agree.
fn rounded_coverage(lx: f32, ly: f32, s: f32, x: f32, y: f32, w: f32, h: f32, r: Radii) -> f32 {
    if (lx + s <= x || ly + s <= y || lx >= x + w || ly >= y + h) {
        return 0.0;
    }
    if (covered_whole(lx, ly, s, x, y, w, h, r)) {
        return 1.0;
    }
    var hits = 0;
    for (var sy = 0; sy < 4; sy++) {
        for (var sx = 0; sx < 4; sx++) {
            if (inside_rounded(lx + (f32(sx) + 0.5) * s / 4.0, ly + (f32(sy) + 0.5) * s / 4.0, x, y, w, h, r)) {
                hits += 1;
            }
        }
    }
    return f32(hits) / 16.0;
}

fn pixel_index(x: u32, y: u32) -> u32 {
    return word_at(CANVAS) + y * word_at(CANVAS_WIDTH) + x;
}

// Painter::blend: straight `c` at `coverage`, source-over in floats.
fn blend(index: u32, c: Rgba, coverage: f32) {
    let a = clamp(c.a * coverage, 0.0, 1.0);
    if (a <= 0.0) {
        return;
    }
    if (a >= 1.0) {
        heap[index] = 0xff000000u | (lround(c.r * 255.0) << 16u) | (lround(c.g * 255.0) << 8u) | lround(c.b * 255.0);
        return;
    }
    let dst = heap[index];
    let da = f32((dst >> 24u) & 0xffu) / 255.0;
    let dr = f32((dst >> 16u) & 0xffu) / 255.0;
    let dg = f32((dst >> 8u) & 0xffu) / 255.0;
    let db = f32(dst & 0xffu) / 255.0;
    heap[index] = (lround((1.0 * a + da * (1.0 - a)) * 255.0) << 24u) | (lround((c.r * a + dr * (1.0 - a)) * 255.0) << 16u)
        | (lround((c.g * a + dg * (1.0 - a)) * 255.0) << 8u) | lround((c.b * a + db * (1.0 - a)) * 255.0);
}

// Painter::solid then Painter::blendSolid: a fully covered pixel, in integers.
fn blend_solid(index: u32, c: Rgba) {
    let a = clamp(c.a, 0.0, 1.0);
    if (a >= 1.0) {
        heap[index] = 0xff000000u | (lround(clamp(c.r, 0.0, 1.0) * 255.0) << 16u) | (lround(clamp(c.g, 0.0, 1.0) * 255.0) << 8u)
            | lround(clamp(c.b, 0.0, 1.0) * 255.0);
        return;
    }
    let sa = lround(a * 255.0);
    let sr = lround(clamp(c.r * a, 0.0, 1.0) * 255.0);
    let sg = lround(clamp(c.g * a, 0.0, 1.0) * 255.0);
    let sb = lround(clamp(c.b * a, 0.0, 1.0) * 255.0);
    let keep = 255u - sa;
    let dst = heap[index];
    heap[index] = ((sa + (((dst >> 24u) & 0xffu) * keep + 127u) / 255u) << 24u)
        | ((sr + (((dst >> 16u) & 0xffu) * keep + 127u) / 255u) << 16u)
        | ((sg + (((dst >> 8u) & 0xffu) * keep + 127u) / 255u) << 8u) | (sb + ((dst & 0xffu) * keep + 127u) / 255u);
}

// Source-over of a premultiplied sample (each channel 0 to 1) at `alpha`, rounded as the CPU's
// image, layer and mask drawing round.
fn over(index: u32, alpha: f32, r: f32, g: f32, b: f32) {
    let dst = heap[index];
    let keep = 1.0 - alpha;
    heap[index] = (lround(clamp(alpha + f32((dst >> 24u) & 0xffu) / 255.0 * keep, 0.0, 1.0) * 255.0) << 24u)
        | (lround(clamp(r + f32((dst >> 16u) & 0xffu) / 255.0 * keep, 0.0, 1.0) * 255.0) << 16u)
        | (lround(clamp(g + f32((dst >> 8u) & 0xffu) / 255.0 * keep, 0.0, 1.0) * 255.0) << 8u)
        | lround(clamp(b + f32(dst & 0xffu) / 255.0 * keep, 0.0, 1.0) * 255.0);
}

// ---- operations ---------------------------------------------------------------------------------

// Painter::fill: a rounded box, less a hole (a border ring).
fn fill_op(px: u32, py: u32) {
    let s = 1.0 / real_at(SCALE);
    let scale = real_at(SCALE);
    let x = real_at(ARGS);
    let y = real_at(ARGS + 1u);
    let w = real_at(ARGS + 2u);
    let h = real_at(ARGS + 3u);
    let r = radii(ARGS + 4u);
    let hole = word_at(ARGS + 8u) != 0u;
    let hr = radii(ARGS + 13u);
    let lx = f32(px) / scale;
    let ly = f32(py) / scale;
    let cx0 = real_at(CLIP);
    let cy0 = real_at(CLIP + 1u);
    let cx1 = real_at(CLIP + 2u);
    let cy1 = real_at(CLIP + 3u);
    if (lx + 1.0 / scale <= cx0 || lx >= cx1 || ly + 1.0 / scale <= cy0 || ly >= cy1) {
        return;
    }
    var coverage: f32;
    if (rounded(r) || hole) {
        coverage = rounded_coverage(lx, ly, s, x, y, w, h, r);
        if (hole && coverage > 0.0) {
            coverage -= rounded_coverage(lx, ly, s, real_at(ARGS + 9u), real_at(ARGS + 10u), real_at(ARGS + 11u), real_at(ARGS + 12u), hr);
        }
    } else {
        let bx0 = max(x, cx0);
        let by0 = max(y, cy0);
        let bx1 = min(x + w, cx1);
        let by1 = min(y + h, cy1);
        let cx = min(lx + 1.0 / scale, bx1) - max(lx, bx0);
        let cy = min(ly + 1.0 / scale, by1) - max(ly, by0);
        coverage = clamp(cx * scale, 0.0, 1.0) * clamp(cy * scale, 0.0, 1.0);
    }
    let index = pixel_index(px, py);
    if (coverage >= 1.0) {
        blend_solid(index, color());
    } else {
        blend(index, color(), clamp(coverage, 0.0, 1.0));
    }
}

// A texel of an image's source, channels 0 to 1 in R, G, B, A order: a picture's bytes, or a
// canvas's 0xAARRGGBB word.
fn texel(base: u32, width: u32, height: u32, canvas: bool, tx: i32, ty: i32) -> vec4<f32> {
    let cx = u32(clamp(tx, 0, i32(width) - 1));
    let cy = u32(clamp(ty, 0, i32(height) - 1));
    let p = heap[base + cy * width + cx];
    if (canvas) {
        return vec4<f32>(f32((p >> 16u) & 0xffu), f32((p >> 8u) & 0xffu), f32(p & 0xffu), f32(p >> 24u)) / 255.0;
    }
    return vec4<f32>(f32(p & 0xffu), f32((p >> 8u) & 0xffu), f32((p >> 16u) & 0xffu), f32(p >> 24u)) / 255.0;
}

// Painter::image: a picture (or a canvas) stretched over (dx, dy, dw, dh), bilinear at the pixel's
// centre, rounded by the box's radii, tinted or not.
fn image_op(px: u32, py: u32) {
    let scale = real_at(SCALE);
    let s = 1.0 / scale;
    let base = word_at(ARGS);
    let pw = word_at(ARGS + 1u);
    let ph = word_at(ARGS + 2u);
    let canvas = word_at(ARGS + 3u) != 0u;
    let dx = real_at(ARGS + 4u);
    let dy = real_at(ARGS + 5u);
    let dw = real_at(ARGS + 6u);
    let dh = real_at(ARGS + 7u);
    let r = radii(ARGS + 12u);
    let tinted = word_at(ARGS + 16u) != 0u;
    let opacity = real_at(OPACITY);
    let bx0 = max(dx, real_at(CLIP));
    let by0 = max(dy, real_at(CLIP + 1u));
    let bx1 = min(dx + dw, real_at(CLIP + 2u));
    let by1 = min(dy + dh, real_at(CLIP + 3u));
    let lx = f32(px) * s;
    let ly = f32(py) * s;
    let cx = min(lx + s, bx1) - max(lx, bx0);
    let cy = min(ly + s, by1) - max(ly, by0);
    var coverage = clamp(cx * scale, 0.0, 1.0) * clamp(cy * scale, 0.0, 1.0);
    if (rounded(r) && coverage > 0.0) {
        coverage *= rounded_coverage(lx, ly, s, real_at(ARGS + 8u), real_at(ARGS + 9u), real_at(ARGS + 10u), real_at(ARGS + 11u), r);
    }
    if (coverage <= 0.0) {
        return;
    }
    let u = ((lx + s / 2.0 - dx) / dw) * f32(pw) - 0.5;
    let v = ((ly + s / 2.0 - dy) / dh) * f32(ph) - 0.5;
    let u0 = i32(floor(u));
    let v0 = i32(floor(v));
    let fu = u - f32(u0);
    let fv = v - f32(v0);
    let t00 = texel(base, pw, ph, canvas, u0, v0);
    let t10 = texel(base, pw, ph, canvas, u0 + 1, v0);
    let t01 = texel(base, pw, ph, canvas, u0, v0 + 1);
    let t11 = texel(base, pw, ph, canvas, u0 + 1, v0 + 1);
    let top = t00 * (1.0 - fu) + t10 * fu;
    let bottom = t01 * (1.0 - fu) + t11 * fu;
    let sample = top * (1.0 - fv) + bottom * fv;
    var alpha = sample.w * opacity * coverage;
    if (alpha <= 0.0) {
        return;
    }
    var red: f32;
    var green: f32;
    var blue: f32;
    if (tinted) {
        let tint = color();
        red = tint.r * alpha * tint.a;
        green = tint.g * alpha * tint.a;
        blue = tint.b * alpha * tint.a;
        alpha *= tint.a;
    } else {
        let k = opacity * coverage;
        red = sample.x * k;
        green = sample.y * k;
        blue = sample.z * k;
    }
    over(pixel_index(px, py), alpha, red, green, blue);
}

// Painter::layer: a canvas drawn through an affine map, bilinear under each pixel's centre,
// transparent outside it.
fn layer_texel(base: u32, width: u32, height: u32, tx: i32, ty: i32) -> u32 {
    if (tx < 0 || ty < 0 || tx >= i32(width) || ty >= i32(height)) {
        return 0u;
    }
    return heap[base + u32(ty) * width + u32(tx)];
}

fn channels(p: u32) -> vec4<f32> {
    return vec4<f32>(f32((p >> 24u) & 0xffu), f32((p >> 16u) & 0xffu), f32((p >> 8u) & 0xffu), f32(p & 0xffu)) / 255.0;
}

fn layer_op(px: u32, py: u32) {
    let s = 1.0 / real_at(SCALE);
    let base = word_at(ARGS);
    let lw = word_at(ARGS + 1u);
    let lh = word_at(ARGS + 2u);
    let layer_scale = real_at(ARGS + 3u);
    let ax = real_at(ARGS + 4u);
    let ay = real_at(ARGS + 5u);
    let a = real_at(ARGS + 6u);
    let b = real_at(ARGS + 7u);
    let c = real_at(ARGS + 8u);
    let d = real_at(ARGS + 9u);
    let tx = real_at(ARGS + 10u);
    let ty = real_at(ARGS + 11u);
    let opacity = real_at(OPACITY);
    let lx = (f32(px) + 0.5) * s;
    let ly = (f32(py) + 0.5) * s;
    if (lx < real_at(CLIP) || lx >= real_at(CLIP + 2u) || ly < real_at(CLIP + 1u) || ly >= real_at(CLIP + 3u)) {
        return;
    }
    let localx = a * lx + c * ly + tx;
    let localy = b * lx + d * ly + ty;
    let u = (localx - ax) * layer_scale - 0.5;
    let v = (localy - ay) * layer_scale - 0.5;
    let u0 = i32(floor(u));
    let v0 = i32(floor(v));
    if (u0 < -1 || v0 < -1 || u0 >= i32(lw) || v0 >= i32(lh)) {
        return;
    }
    let fu = u - f32(u0);
    let fv = v - f32(v0);
    let p00 = layer_texel(base, lw, lh, u0, v0);
    let p10 = layer_texel(base, lw, lh, u0 + 1, v0);
    let p01 = layer_texel(base, lw, lh, u0, v0 + 1);
    let p11 = layer_texel(base, lw, lh, u0 + 1, v0 + 1);
    if ((p00 | p10 | p01 | p11) == 0u) {
        return;
    }
    let sample = ((channels(p00) * (1.0 - fu) + channels(p10) * fu) * (1.0 - fv) + (channels(p01) * (1.0 - fu) + channels(p11) * fu) * fv) * opacity;
    if (sample.x <= 0.0) {
        return;
    }
    over(pixel_index(px, py), sample.x, sample.y, sample.z, sample.w);
}

// Painter::glyphs: a run's glyph masks, each blended in the run's colour by its coverage.
fn glyphs_op(px: u32, py: u32) {
    let scale = real_at(SCALE);
    let list = word_at(ARGS);
    let count = word_at(ARGS + 1u);
    let lx = f32(px) / scale;
    let ly = f32(py) / scale;
    if (lx < real_at(CLIP) || lx >= real_at(CLIP + 2u) || ly < real_at(CLIP + 1u) || ly >= real_at(CLIP + 3u)) {
        return;
    }
    let index = pixel_index(px, py);
    for (var g = 0u; g < count; g++) {
        let at = list + g * 5u;
        let gx = bitcast<i32>(heap[at]);
        let gy = bitcast<i32>(heap[at + 1u]);
        let gw = heap[at + 2u];
        let gh = heap[at + 3u];
        let x = i32(px) - gx;
        let y = i32(py) - gy;
        if (x < 0 || y < 0 || x >= i32(gw) || y >= i32(gh)) {
            continue;
        }
        let byte = u32(y) * gw + u32(x);
        let value = (heap[heap[at + 4u] + byte / 4u] >> ((byte % 4u) * 8u)) & 0xffu;
        if (value != 0u) {
            blend(index, color(), f32(value) / 255.0);
        }
    }
}

// The 8x8 ordered (Bayer) threshold of `ditherAt`.
fn dither(x: u32, y: u32) -> f32 {
    var bayer = array<u32, 64>(
        0u, 32u, 8u, 40u, 2u, 34u, 10u, 42u, 48u, 16u, 56u, 24u, 50u, 18u, 58u, 26u,
        12u, 44u, 4u, 36u, 14u, 46u, 6u, 38u, 60u, 28u, 52u, 20u, 62u, 30u, 54u, 22u,
        3u, 35u, 11u, 43u, 1u, 33u, 9u, 41u, 51u, 19u, 59u, 27u, 49u, 17u, 57u, 25u,
        15u, 47u, 7u, 39u, 13u, 45u, 5u, 37u, 63u, 31u, 55u, 23u, 61u, 29u, 53u, 21u);
    return (f32(bayer[(y & 7u) * 8u + (x & 7u)]) + 0.5) / 64.0 - 0.5;
}

fn quantize(value: f32, threshold: f32) -> u32 {
    return u32(clamp(i32(value + threshold + 0.5), 0, 255));
}

fn plane(base: u32, size: u32, c: u32, index: u32) -> f32 {
    return bitcast<f32>(heap[base + c * size + index]);
}

// Painter::smooth: float planes stretched bilinearly at full precision and dithered as written.
fn smooth_op(px: u32, py: u32) {
    let scale = real_at(SCALE);
    let s = 1.0 / scale;
    let base = word_at(ARGS);
    let iw = word_at(ARGS + 1u);
    let ih = word_at(ARGS + 2u);
    let dx = real_at(ARGS + 3u);
    let dy = real_at(ARGS + 4u);
    let dw = real_at(ARGS + 5u);
    let dh = real_at(ARGS + 6u);
    let r = radii(ARGS + 11u);
    let opacity = real_at(OPACITY);
    let bx0 = max(dx, real_at(CLIP));
    let by0 = max(dy, real_at(CLIP + 1u));
    let bx1 = min(dx + dw, real_at(CLIP + 2u));
    let by1 = min(dy + dh, real_at(CLIP + 3u));
    let size = iw * ih;
    let lx = f32(px) * s;
    let u = ((lx + s / 2.0 - dx) / dw) * f32(iw) - 0.5;
    let u0 = i32(floor(u));
    let cover = clamp((min(lx + s, bx1) - max(lx, bx0)) * scale, 0.0, 1.0);
    let weight = u - f32(u0);
    let left = u32(clamp(u0, 0, i32(iw) - 1));
    let right = u32(clamp(u0 + 1, 0, i32(iw) - 1));
    let ly = f32(py) * s;
    let row_cover = clamp((min(ly + s, by1) - max(ly, by0)) * scale, 0.0, 1.0);
    let v = ((ly + s / 2.0 - dy) / dh) * f32(ih) - 0.5;
    let v0 = i32(floor(v));
    let fv = v - f32(v0);
    let top_row = u32(clamp(v0, 0, i32(ih) - 1)) * iw;
    let bottom_row = u32(clamp(v0 + 1, 0, i32(ih) - 1)) * iw;
    var coverage = cover * row_cover;
    if (rounded(r) && coverage > 0.0) {
        coverage *= rounded_coverage(lx, ly, s, real_at(ARGS + 7u), real_at(ARGS + 8u), real_at(ARGS + 9u), real_at(ARGS + 10u), r);
    }
    if (coverage <= 0.0) {
        return;
    }
    let k = opacity * coverage;
    var level: array<f32, 4>;
    for (var c = 0u; c < 4u; c++) {
        let tl = plane(base, size, c, top_row + left);
        let top = tl + (plane(base, size, c, top_row + right) - tl) * weight;
        let bl = plane(base, size, c, bottom_row + left);
        let bottom = bl + (plane(base, size, c, bottom_row + right) - bl) * weight;
        level[c] = (top + (bottom - top) * fv) * k;
    }
    let threshold = dither(px, py);
    let index = pixel_index(px, py);
    let alpha = level[0] / 255.0;
    if (alpha <= 0.0) {
        return;
    }
    if (level[0] >= 254.5) {
        heap[index] = 0xff000000u | (quantize(level[1], threshold) << 16u) | (quantize(level[2], threshold) << 8u)
            | quantize(level[3], threshold);
        return;
    }
    let out = heap[index];
    heap[index] = (quantize(level[0] + f32((out >> 24u) & 0xffu) * (1.0 - alpha), threshold) << 24u)
        | (quantize(level[1] + f32((out >> 16u) & 0xffu) * (1.0 - alpha), threshold) << 16u)
        | (quantize(level[2] + f32((out >> 8u) & 0xffu) * (1.0 - alpha), threshold) << 8u)
        | quantize(level[3] + f32(out & 0xffu) * (1.0 - alpha), threshold);
}

// levelsOf: a canvas as planes of levels (alpha, red, green, blue).
fn levels_op(px: u32, py: u32) {
    let source = word_at(ARGS);
    let width = word_at(ARGS + 1u);
    let height = word_at(ARGS + 2u);
    let out = word_at(ARGS + 3u);
    let index = py * width + px;
    let p = heap[source + index];
    for (var c = 0u; c < 4u; c++) {
        heap[out + c * width * height + index] = bitcast<u32>(f32((p >> (24u - 8u * c)) & 0xffu));
    }
}

// gaussianBlur's last step: planes of levels back into a canvas, rounded.
fn from_levels(px: u32, py: u32) {
    let source = word_at(ARGS);
    let width = word_at(ARGS + 1u);
    let height = word_at(ARGS + 2u);
    let index = py * width + px;
    var pixel = 0u;
    for (var c = 0u; c < 4u; c++) {
        let value = bitcast<f32>(heap[source + c * width * height + index]);
        pixel |= lround(clamp(value, 0.0, 255.0)) << (24u - 8u * c);
    }
    heap[pixel_index(px, py)] = pixel;
}

// blurPlanes' box: one box of half-width r along a line, a running sum, edges extended. `px` is
// the line, `py` the plane.
fn box_line(line: u32, plane_index: u32) {
    let input = word_at(ARGS) + plane_index * word_at(ARGS + 7u);
    let output = word_at(ARGS + 1u) + plane_index * word_at(ARGS + 7u);
    let r = i32(word_at(ARGS + 2u));
    let length = i32(word_at(ARGS + 4u));
    let stride = word_at(ARGS + 5u);
    let step = word_at(ARGS + 6u);
    let base = line * stride;
    let scale = 1.0 / f32(2 * r + 1);
    var sum = 0.0;
    for (var i = -r; i <= r; i++) {
        sum += bitcast<f32>(heap[input + base + u32(clamp(i, 0, length - 1)) * step]);
    }
    for (var i = 0; i < length; i++) {
        heap[output + base + u32(i) * step] = bitcast<u32>(sum * scale);
        sum += bitcast<f32>(heap[input + base + u32(clamp(i + r + 1, 0, length - 1)) * step])
            - bitcast<f32>(heap[input + base + u32(clamp(i - r, 0, length - 1)) * step]);
    }
}

// drawMasked: the content kept by the mask's alpha, in integers.
fn mask_op(px: u32, py: u32) {
    let mask_base = word_at(ARGS);
    let index = py * word_at(CANVAS_WIDTH) + px;
    let keep = heap[mask_base + index] >> 24u;
    if (keep == 255u) {
        return;
    }
    let at = word_at(CANVAS) + index;
    let pixel = heap[at];
    var kept = 0u;
    for (var shift = 0u; shift < 32u; shift += 8u) {
        kept |= ((((pixel >> shift) & 0xffu) * keep + 127u) / 255u) << shift;
    }
    heap[at] = kept;
}

// A box of the canvas set to one value.
fn clear_op(px: u32, py: u32) {
    heap[pixel_index(px, py)] = word_at(ARGS);
}

// A box of another canvas copied, pixel for pixel at the same place.
fn copy_op(px: u32, py: u32) {
    heap[pixel_index(px, py)] = heap[word_at(ARGS) + py * word_at(ARGS + 1u) + px];
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let kind = word_at(KIND);
    if (kind == 7u) {
        // A blur's lines: every line and plane, not pixels.
        if (id.x < word_at(ARGS + 3u) && id.y < 4u) {
            box_line(id.x, id.y);
        }
        return;
    }
    let px = word_at(X0) + id.x;
    let py = word_at(Y0) + id.y;
    if (px >= word_at(X1) || py >= word_at(Y1)) {
        return;
    }
    switch kind {
        case 1u: { fill_op(px, py); }
        case 2u: { image_op(px, py); }
        case 3u: { layer_op(px, py); }
        case 4u: { glyphs_op(px, py); }
        case 5u: { smooth_op(px, py); }
        case 6u: { levels_op(px, py); }
        case 8u: { from_levels(px, py); }
        case 9u: { mask_op(px, py); }
        case 10u: { clear_op(px, py); }
        case 11u: { copy_op(px, py); }
        default: {}
    }
}
