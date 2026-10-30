use super::*;
use drm_fourcc::DrmFourcc;

/// A blob as the kernel builds it: header, two formats, two modifier entries.
fn blob() -> Vec<u8> {
    let mut b = Vec::new();
    let header = [1u32, 0, 2, 24, 2, 32]; // version, flags, formats at 24, modifiers at 32
    for v in header {
        b.extend(v.to_ne_bytes());
    }
    for f in [DrmFourcc::Xrgb8888 as u32, DrmFourcc::Argb8888 as u32] {
        b.extend(f.to_ne_bytes());
    }
    // Linear for both formats; a tiled modifier for XRGB8888 only.
    for (mask, offset, modifier) in [(0b11u64, 0u32, 0u64), (0b01, 0, 0x0100_0000_0000_0001)] {
        b.extend(mask.to_ne_bytes());
        b.extend(offset.to_ne_bytes());
        b.extend(0u32.to_ne_bytes());
        b.extend(modifier.to_ne_bytes());
    }
    b
}

#[test]
fn in_formats_pairs_every_format_with_its_modifiers() {
    let pairs = parse_in_formats(&blob()).unwrap();
    assert_eq!(
        pairs,
        [
            (DrmFourcc::Xrgb8888, 0),
            (DrmFourcc::Argb8888, 0),
            (DrmFourcc::Xrgb8888, 0x0100_0000_0000_0001),
        ]
    );
}

#[test]
fn a_truncated_blob_is_rejected() {
    let blob = blob();
    for length in [0, 10, 30, blob.len() - 1] {
        assert!(parse_in_formats(&blob[..length]).is_err(), "{length}");
    }
}

#[test]
fn the_crtc_already_lighting_the_output_is_kept() {
    let crtc = |raw| drm::control::from_u32::<drm::control::crtc::Handle>(raw).unwrap();
    let possible = [crtc(41), crtc(58), crtc(75)];
    // The boot logo's (or the firmware's) CRTC, so the first frame is no full modeset.
    assert_eq!(
        display::choose_crtc(Some(crtc(58)), &possible),
        Some(crtc(58))
    );
    // Nothing lit, or lit by a CRTC the connector may not use: the first it may.
    assert_eq!(display::choose_crtc(None, &possible), Some(crtc(41)));
    assert_eq!(
        display::choose_crtc(Some(crtc(92)), &possible),
        Some(crtc(41))
    );
    assert_eq!(display::choose_crtc(Some(crtc(58)), &[]), None);
}
