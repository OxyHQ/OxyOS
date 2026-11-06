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

#[test]
fn a_hotplug_is_a_drm_change_of_the_card_itself() {
    let event = |fields: &[&str]| {
        let mut bytes = b"change@/devices/pci0000:00/0000:00:02.0/drm/card1".to_vec();
        for field in fields {
            bytes.push(0);
            bytes.extend(field.as_bytes());
        }
        bytes.push(0);
        bytes
    };
    let plugged = event(&[
        "ACTION=change",
        "SUBSYSTEM=drm",
        "HOTPLUG=1",
        "DEVNAME=dri/card1",
        "SEQNUM=4242",
    ]);
    assert!(crate::hotplug::is_hotplug(&plugged, "dri/card1"));
    // Another card's, a change that is no hotplug, another subsystem's.
    assert!(!crate::hotplug::is_hotplug(&plugged, "dri/card0"));
    assert!(!crate::hotplug::is_hotplug(
        &event(&["ACTION=change", "SUBSYSTEM=drm", "DEVNAME=dri/card1"]),
        "dri/card1"
    ));
    assert!(!crate::hotplug::is_hotplug(
        &event(&[
            "ACTION=change",
            "SUBSYSTEM=input",
            "HOTPLUG=1",
            "DEVNAME=dri/card1"
        ]),
        "dri/card1"
    ));
    // A field only resembling one is not it; nothing, or a header alone, is no event.
    assert!(!crate::hotplug::is_hotplug(
        &event(&["SUBSYSTEM=drm", "HOTPLUG=10", "DEVNAME=dri/card1"]),
        "dri/card1"
    ));
    assert!(!crate::hotplug::is_hotplug(b"", "dri/card1"));
    assert!(!crate::hotplug::is_hotplug(b"SUBSYSTEM=drm", "dri/card1"));
}

/// The cursor plane's properties: the pointer placed by its hotspot, the plane's whole buffer its
/// source, on the display's CRTC; hidden, the plane off. A pointer moved alone commits these and
/// nothing else (`Display::present_again`), the primary plane left out of the commit.
#[test]
fn the_pointer_is_placed_by_its_hotspot_and_hidden_with_its_plane_off() {
    use drm::control::{crtc, framebuffer, property::Value};
    let crtc: crtc::Handle = drm::control::from_u32(82).unwrap();
    let image: framebuffer::Handle = drm::control::from_u32(302).unwrap();
    let shown = display::pointer_properties(crtc, (256, 256), Some((1160, 1150, image, (8, 5))));
    let value = |name: &str| {
        shown
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, value)| *value)
            .unwrap_or_else(|| panic!("no {name}"))
    };
    assert_eq!(shown.len(), 10);
    assert!(matches!(value("CRTC_ID"), Value::CRTC(Some(c)) if c == crtc));
    assert!(matches!(value("FB_ID"), Value::Framebuffer(Some(f)) if f == image));
    assert!(matches!(value("CRTC_X"), Value::SignedRange(1152)));
    assert!(matches!(value("CRTC_Y"), Value::SignedRange(1145)));
    assert!(matches!(value("SRC_W"), Value::UnsignedRange(w) if w == 256 << 16));
    assert!(matches!(value("CRTC_H"), Value::UnsignedRange(256)));
    let hidden = display::pointer_properties(crtc, (256, 256), None);
    assert!(matches!(
        hidden.as_slice(),
        [
            ("CRTC_ID", Value::CRTC(None)),
            ("FB_ID", Value::Framebuffer(None))
        ]
    ));
}
