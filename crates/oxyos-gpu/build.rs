//! Compiles `shaders/compose.wgsl` to SPIR-V with naga, twice: `$OUT_DIR/compose.spv`, reading
//! every layer's pixels from a storage buffer, and `$OUT_DIR/compose_images.spv`, which can also
//! read a layer from a client's GPU buffer, sampled in place (docs/adr/0031-gpu-buffers.mdx): an
//! array of images indexed per layer, which needs non-uniform indexing of sampled images.

use naga::back::spv;
use naga::valid::{Capabilities, ValidationFlags, Validator};
use std::path::PathBuf;

/// How many client GPU buffers one frame composes, at most (`vulkan::IMAGE_SLOTS`).
const IMAGE_SLOTS: u32 = 64;

fn compile(wgsl: &str, capabilities: Capabilities, name: &str) {
    let module = naga::front::wgsl::parse_str(wgsl)
        .unwrap_or_else(|error| panic!("{}", error.emit_to_string(wgsl)));
    let info = Validator::new(ValidationFlags::all(), capabilities)
        .validate(&module)
        .unwrap_or_else(|error| panic!("invalid shader: {error:?}"));
    let options = spv::Options {
        lang_version: (1, 0),
        ..spv::Options::default()
    };
    let words = spv::write_vec(&module, &info, &options, None).expect("SPIR-V");
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join(name);
    std::fs::write(out, bytes).expect("write SPIR-V");
}

fn main() {
    let source = "shaders/compose.wgsl";
    println!("cargo:rerun-if-changed={source}");
    let wgsl = std::fs::read_to_string(source).expect("read the shader");
    compile(&wgsl, Capabilities::PUSH_CONSTANT, "compose.spv");
    let images = wgsl
        .replace(
            "//@images-binding",
            &format!(
                "@group(0) @binding(3) var images: binding_array<texture_2d<f32>, {IMAGE_SLOTS}>;"
            ),
        )
        .replace(
            "//@images-texel",
            "if (layers[i].image != 0u) {\n\
             let c = textureLoad(images[layers[i].image - 1u], vec2<i32>(i32(x), i32(y)), 0);\n\
             let v = vec4<u32>(round(c * 255.0));\n\
             return (v.a << 24u) | (v.r << 16u) | (v.g << 8u) | v.b;\n\
             }",
        );
    compile(
        &images,
        Capabilities::PUSH_CONSTANT
            | Capabilities::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING,
        "compose_images.spv",
    );
}
