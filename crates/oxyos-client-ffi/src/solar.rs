//! C ABI over `oxyos-solar` (docs/adr/0012-solar-wallpaper.mdx): the frame of a solar wallpaper
//! that shows the sun now, where the system's time zone is, for the Bloom host run as the shell.

use super::fail;
use oxyos_solar::Anchor;
use std::ffi::c_int;

/// A frame of a solar wallpaper and the sun it shows (its manifest's `solarAnchors`): degrees
/// above the horizon and clockwise from north.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OxyOSSolarAnchor {
    pub frame: u32,
    pub elevation: f64,
    pub azimuth: f64,
}

/// The frame of `anchors` showing the sun at `unix_seconds` where the system's time zone is,
/// written to `frame`, and when (unix seconds) the frame next changes, written to
/// `next_change`. 0 on success; 1 where the time zone has no place (UTC): there is no sun, and
/// the caller shows its own frame; OXYOS_ERROR for anchors that are none.
///
/// # Safety
/// `anchors` points to `count` anchors; `frame` and `next_change` are writable.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_solar_frame(
    anchors: *const OxyOSSolarAnchor,
    count: usize,
    unix_seconds: i64,
    frame: *mut u32,
    next_change: *mut i64,
) -> c_int {
    if anchors.is_null() || count == 0 || frame.is_null() || next_change.is_null() {
        return fail("no anchors");
    }
    // SAFETY: per the contract, `anchors` points to `count` anchors.
    let given = unsafe { std::slice::from_raw_parts(anchors, count) };
    let anchors: Vec<Anchor> = given
        .iter()
        .map(|a| Anchor {
            frame: a.frame,
            elevation: a.elevation,
            azimuth: a.azimuth,
        })
        .collect();
    let Some(place) = oxyos_solar::system_place() else {
        return 1;
    };
    match oxyos_solar::frame_at(&anchors, unix_seconds, place) {
        Some((chosen, next)) => {
            // SAFETY: per the contract, both are writable.
            unsafe {
                *frame = chosen;
                *next_change = next;
            }
            0
        }
        None => fail("anchors that are none"),
    }
}
