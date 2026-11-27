//! The session's splash (docs/adr/0011-transitions.mdx): the Oxy mark, white, centred on black
//! over the whole output, until the shell is on screen. Its pixels are `oxyos-splash`'s, which
//! the boot logo (docs/adr/0017-boot-logo.mdx) draws its images with too.

use crate::transitions::Snapshot;
use oxyos_protocol::{Format, Rect, Size};

/// The splash for an output of `size`, as a frame the compositor draws over everything.
pub fn splash(size: Size) -> Option<Snapshot> {
    let pixels = oxyos_splash::frame(size.width, size.height)?;
    Some(Snapshot {
        rect: Rect::new(0, 0, size.width, size.height),
        extent: size,
        format: Format::Xrgb8888,
        pixels: std::rc::Rc::new(pixels.into()),
        source: None,
        look: None,
        views: Vec::new(),
        captured: true,
        secure: Vec::new(),
    })
}
