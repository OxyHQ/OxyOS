//! Compiles `shaders/compose.wgsl` to SPIR-V (`$OUT_DIR/compose.spv`) with naga.

use naga::back::spv;
use naga::valid::{Capabilities, ValidationFlags, Validator};
use std::path::PathBuf;

fn main() {
    let source = "shaders/compose.wgsl";
    println!("cargo:rerun-if-changed={source}");
    let wgsl = std::fs::read_to_string(source).expect("read the shader");
    let module = naga::front::wgsl::parse_str(&wgsl)
        .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&wgsl)));
    let info = Validator::new(ValidationFlags::all(), Capabilities::PUSH_CONSTANT)
        .validate(&module)
        .unwrap_or_else(|error| panic!("invalid shader: {error:?}"));
    let options = spv::Options {
        lang_version: (1, 0),
        ..spv::Options::default()
    };
    let words = spv::write_vec(&module, &info, &options, None).expect("SPIR-V");
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("compose.spv");
    std::fs::write(out, bytes).expect("write SPIR-V");
}
