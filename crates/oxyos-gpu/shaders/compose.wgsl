// One layer of a frame, composed into the output (docs/adr/0004): the same arithmetic as
// `compose_cpu`, so the two agree byte for byte. Pixels are 0x00RRGGBB in the output and
// 0xAARRGGBB (B, G, R, A in memory) in a layer.

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
    // 0: opaque copy, 1: premultiplied source-over, 2: fill with `color`.
    mode: u32,
    color: u32,
    // The part of the layer this dispatch composes (a damaged region's), from (base_x,
    // base_y) within the layer, count_x × count_y pixels.
    base_x: u32,
    base_y: u32,
    count_x: u32,
    count_y: u32,
    // 0 to 255: the source's premultiplied pixel is scaled by it before it is blended
    // (docs/adr/0011-transitions.mdx); at 255, as without it.
    opacity: u32,
}

var<push_constant> layer: Layer;
@group(0) @binding(0) var<storage, read> src: array<u32>;
@group(0) @binding(1) var<storage, read_write> dst: array<u32>;

fn channel(source: u32, under: u32, keep: u32, shift: u32) -> u32 {
    let s = ((source >> shift) & 0xffu) * layer.opacity / 255u;
    let d = (under >> shift) & 0xffu;
    return min(s + d * keep / 255u, 255u) << shift;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= layer.count_x || id.y >= layer.count_y) {
        return;
    }
    let lx = layer.base_x + id.x;
    let ly = layer.base_y + id.y;
    if (lx >= layer.width || ly >= layer.height) {
        return;
    }
    let x = layer.x + i32(lx);
    let y = layer.y + i32(ly);
    if (x < 0 || y < 0 || u32(x) >= layer.out_width || u32(y) >= layer.out_height) {
        return;
    }
    let index = u32(y) * layer.out_width + u32(x);
    // The pixel, premultiplied, with its alpha (opaque ones and fills have 255).
    var source = layer.color & 0xffffffu;
    var alpha = 255u;
    if (layer.mode != 2u) {
        // Nearest neighbour, as the reference: the source pixel under the output pixel's corner.
        let sx = lx * layer.src_width / layer.width;
        let sy = ly * layer.src_height / layer.height;
        let pixel = src[layer.src_offset + sy * layer.src_width + sx];
        source = pixel & 0xffffffu;
        if (layer.mode == 1u) {
            alpha = pixel >> 24u;
        }
    }
    // Scaled by the layer's opacity, then source-over: at 255 an opaque pixel is copied and a
    // premultiplied one blended, as without an opacity.
    let keep = 255u - alpha * layer.opacity / 255u;
    let under = dst[index];
    dst[index] = channel(source, under, keep, 16u) | channel(source, under, keep, 8u) | channel(source, under, keep, 0u);
}
