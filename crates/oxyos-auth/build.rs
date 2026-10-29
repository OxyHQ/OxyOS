// PAM's library by the name the system loads it by, so building needs no development package.
fn main() {
    println!("cargo:rustc-link-lib=dylib:+verbatim=libpam.so.0");
}
