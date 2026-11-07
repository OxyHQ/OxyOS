//! Writes a Display P3 ICC profile (moxcms's) to standard output: the profile
//! `scripts/make-test-images.py` embeds in the repository's test JPEG, so the test image is made
//! from nothing but the repository and its dependencies.

use std::io::Write;

fn main() {
    let profile = moxcms::ColorProfile::new_display_p3()
        .encode()
        .expect("moxcms encodes its Display P3 profile");
    std::io::stdout()
        .write_all(&profile)
        .expect("the profile is written");
}
