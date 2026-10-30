//! Output on a connector with atomic modesetting: two dumb buffers, a modeset for the first
//! frame and page flips after it, with the kernel's flip timestamps. Needs DRM master (the
//! card's current master, a running desktop session, must have released it).
//!
//! The pointer is shown on the CRTC's cursor plane where it has one: one of the images the
//! display keeps, each in a buffer of its own (its own arrow first), placed in every commit, so
//! moving the pointer or changing its image only commits the frame already shown again.
//!
//! Several displays share a card, each on a connector, a CRTC and planes of its own
//! (docs/adr/0018-several-displays.mdx): [`Display::on_connector`] takes what no other uses.

use crate::Card;
use drm::buffer::{Buffer as _, DrmFourcc};
use drm::control::dumbbuffer::DumbBuffer;
use drm::control::{
    atomic, connector, crtc, framebuffer, plane, property, AtomicCommitFlags, Device as _, Mode,
};
use drm::{Device as _, DriverCapability};
use std::io;
use std::rc::Rc;
use std::time::Duration;

/// A connector driven by this process: its mode, CRTC, primary plane and two buffers.
pub struct Display {
    card: Rc<Card>,
    connector: connector::Handle,
    crtc: crtc::Handle,
    plane: plane::Handle,
    mode: Mode,
    buffers: Vec<(DumbBuffer, framebuffer::Handle)>,
    back: usize,
    configured: bool,
    mode_blob: Option<property::Value<'static>>,
    /// The cursor plane and its images, if the CRTC has one.
    cursor: Option<Cursor>,
    /// Where the pointer is to be shown and with which image, and how the last commit showed it.
    pointer: Option<(i32, i32, usize)>,
    shown_pointer: Option<(i32, i32, usize)>,
    /// The framebuffer the last commit showed.
    front: Option<framebuffer::Handle>,
}

struct Cursor {
    plane: plane::Handle,
    size: (u32, u32),
    images: Vec<CursorImage>,
}

struct CursorImage {
    buffer: DumbBuffer,
    framebuffer: framebuffer::Handle,
    hotspot: (u32, u32),
}

/// A pointer's image: premultiplied 0xAARRGGBB pixels, row by row, and the point of it that is
/// the pointer's position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PointerImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
    pub hotspot: (u32, u32),
}

impl PointerImage {
    /// The display's own arrow, its tip the hotspot.
    pub fn arrow() -> Self {
        let width = ARROW.iter().map(|row| row.len()).max().unwrap_or(0) as u32;
        let height = ARROW.len() as u32;
        let mut pixels = vec![0; (width * height) as usize];
        for (y, row) in ARROW.iter().enumerate() {
            for (x, mark) in row.chars().enumerate() {
                pixels[y * width as usize + x] = match mark {
                    '#' => 0xff00_0000,
                    '.' => 0xffff_ffff,
                    _ => 0,
                };
            }
        }
        Self {
            width,
            height,
            pixels,
            hotspot: (0, 0),
        }
    }
}

/// The pointer's arrow, its tip the hotspot at the top left: `#` its outline, `.` its fill.
const ARROW: [&str; 19] = [
    "#",
    "##",
    "#.#",
    "#..#",
    "#...#",
    "#....#",
    "#.....#",
    "#......#",
    "#.......#",
    "#........#",
    "#.........#",
    "#......#####",
    "#...#..#",
    "#..##..#",
    "#.#  #..#",
    "##   #..#",
    "#     #..#",
    "      #..#",
    "       ##",
];

fn not_found(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, what.to_string())
}

impl Display {
    /// Takes DRM master and prepares the first connected connector at its preferred mode.
    pub fn open(card: Card) -> io::Result<Self> {
        card.acquire_master_lock()?;
        Self::first_connected(Rc::new(card))
    }

    /// The same with a card the session's seat manager opened and made DRM master (logind's
    /// `TakeDevice`): master is the seat manager's to drop and restore, and the kernel refuses
    /// the request to anyone but the card's opener.
    pub fn open_from_seat(card: Card) -> io::Result<Self> {
        Self::first_connected(Rc::new(card))
    }

    /// The card's first connected connector.
    fn first_connected(card: Rc<Card>) -> io::Result<Self> {
        let connector = card
            .outputs()?
            .into_iter()
            .find(|output| output.connected && !output.modes.is_empty())
            .ok_or_else(|| not_found("no connected output"))?;
        Self::on_connector(card, connector.id, None, &[])
    }

    /// Prepares the connector `connector` (an [`Output`](crate::Output)'s id) of `card` at `mode`
    /// (one of its [`modes`](crate::Output::modes); none: its preferred), on a CRTC and planes none
    /// of `in_use` names (the other displays' [`objects`](Self::objects)). The card stays shared:
    /// each display presents on its own CRTC.
    pub fn on_connector(
        card: Rc<Card>,
        connector: u32,
        mode: Option<&crate::Mode>,
        in_use: &[u32],
    ) -> io::Result<Self> {
        let resources = card.resource_handles()?;
        let connector = resources
            .connectors()
            .iter()
            .copied()
            .find(|&handle| u32::from(handle) == connector)
            .ok_or_else(|| not_found("no such connector"))?;
        let info = card.get_connector(connector, false)?;
        if info.state() != connector::State::Connected {
            return Err(not_found("the output is not connected"));
        }
        let drivable: Vec<Mode> = info
            .modes()
            .iter()
            .copied()
            .filter(crate::drivable)
            .collect();
        let mode = match mode {
            Some(wanted) => drivable
                .iter()
                .find(|m| wanted.is(m))
                .ok_or_else(|| not_found("the output has no such mode"))?,
            None => drivable
                .iter()
                .find(|m| {
                    m.mode_type()
                        .contains(drm::control::ModeTypeFlags::PREFERRED)
                })
                .or(drivable.first())
                .ok_or_else(|| not_found("the output has no mode"))?,
        };
        let mode = *mode;
        let free = |handle: u32| !in_use.contains(&handle);
        let lit = info
            .current_encoder()
            .and_then(|encoder| card.get_encoder(encoder).ok())
            .and_then(|encoder| encoder.crtc());
        // Of the CRTCs it may use, those no other display has.
        let possible: Vec<crtc::Handle> = info
            .encoders()
            .iter()
            .filter_map(|&encoder| card.get_encoder(encoder).ok())
            .flat_map(|encoder| resources.filter_crtcs(encoder.possible_crtcs()))
            .filter(|&crtc| free(u32::from(crtc)))
            .collect();
        let crtc =
            choose_crtc(lit, &possible).ok_or_else(|| not_found("no free CRTC for the output"))?;
        let primary = card
            .plane_handles()?
            .into_iter()
            .find(|&handle| {
                let Ok(info) = card.get_plane(handle) else {
                    return false;
                };
                free(u32::from(handle))
                    && resources
                        .filter_crtcs(info.possible_crtcs())
                        .contains(&crtc)
                    && property_value(&card, handle, "type").ok() == Some(1)
            })
            .ok_or_else(|| not_found("no primary plane for the CRTC"))?;
        let (width, height) = mode.size();
        let mut display = Self {
            card,
            connector,
            crtc,
            plane: primary,
            mode,
            buffers: Vec::new(),
            back: 0,
            configured: false,
            mode_blob: None,
            cursor: None,
            pointer: None,
            shown_pointer: None,
            front: None,
        };
        display.cursor = display.prepare_cursor(&resources, in_use).ok().flatten();
        for _ in 0..2 {
            let buffer = display.card.create_dumb_buffer(
                (u32::from(width), u32::from(height)),
                DrmFourcc::Xrgb8888,
                32,
            )?;
            let framebuffer = display.card.add_framebuffer(&buffer, 24, 32)?;
            display.buffers.push((buffer, framebuffer));
        }
        Ok(display)
    }

    /// The CRTC's cursor plane and the arrow in a buffer of the size the card asks for; none
    /// without a cursor plane.
    fn prepare_cursor(
        &self,
        resources: &drm::control::ResourceHandles,
        in_use: &[u32],
    ) -> io::Result<Option<Cursor>> {
        let card = &*self.card;
        let plane = card.plane_handles()?.into_iter().find(|&handle| {
            !in_use.contains(&u32::from(handle))
                && card.get_plane(handle).is_ok_and(|info| {
                    resources
                        .filter_crtcs(info.possible_crtcs())
                        .contains(&self.crtc)
                })
                && property_value(card, handle, "type").ok() == Some(2)
        });
        let Some(plane) = plane else {
            return Ok(None);
        };
        let side = |cap| card.get_driver_capability(cap).unwrap_or(64).max(32) as u32;
        let size = (
            side(DriverCapability::CursorWidth),
            side(DriverCapability::CursorHeight),
        );
        let arrow = cursor_image(card, size, &PointerImage::arrow())?;
        Ok(Some(Cursor {
            plane,
            size,
            images: vec![arrow],
        }))
    }

    /// Keeps a pointer image, in a buffer of its own; its number for `set_pointer` (the display's
    /// own arrow is 0). Refused when larger than the cursor plane takes (the card's cursor
    /// size), or without a cursor plane.
    pub fn add_pointer_image(&mut self, image: &PointerImage) -> io::Result<usize> {
        let Some(cursor) = self.cursor.as_mut() else {
            return Err(not_found("no cursor plane"));
        };
        cursor
            .images
            .push(cursor_image(&self.card, cursor.size, image)?);
        Ok(cursor.images.len() - 1)
    }

    /// Lets go of the pointer images kept (`add_pointer_image`) but the display's own arrow, which
    /// is then shown (the display is shown at another scale, its images chosen again for it).
    pub fn clear_pointer_images(&mut self) {
        let Some(cursor) = self.cursor.as_mut() else {
            return;
        };
        for image in cursor.images.drain(1..) {
            let _ = self.card.destroy_framebuffer(image.framebuffer);
            let _ = self.card.destroy_dumb_buffer(image.buffer);
        }
        if let Some((x, y, _)) = self.pointer {
            self.pointer = Some((x, y, 0));
        }
    }

    /// The largest pointer image the cursor plane takes, if there is one.
    pub fn pointer_size(&self) -> Option<(u32, u32)> {
        self.cursor.as_ref().map(|cursor| cursor.size)
    }

    /// Whether the display can show the pointer (its CRTC has a cursor plane).
    pub fn shows_pointer(&self) -> bool {
        self.cursor.is_some()
    }

    /// Where the pointer is to be shown, in the display's pixels (none hides it), and with which
    /// image (0, the arrow, for one the display does not keep): true when the pointer shown must
    /// change, by the next commit or `present_again`.
    pub fn set_pointer(&mut self, at: Option<(i32, i32)>, image: usize) -> bool {
        let Some(cursor) = &self.cursor else {
            return false;
        };
        let image = if image < cursor.images.len() {
            image
        } else {
            0
        };
        self.pointer = at.map(|(x, y)| (x, y, image));
        self.configured && self.pointer != self.shown_pointer
    }

    /// Commits the frame shown again, with the pointer where it is now; true if a frame is shown
    /// to commit. Its flip is waited for as a frame's (`flip_done`).
    pub fn present_again(&mut self) -> io::Result<bool> {
        match self.front {
            Some(front) if self.configured => self.present_framebuffer(front).map(|()| true),
            _ => Ok(false),
        }
    }

    /// The card this display drives (to import buffers into it, for one).
    pub fn card(&self) -> &Card {
        &self.card
    }

    /// The card's objects this display uses: its CRTC and planes, which another display of the
    /// same card must not take (`on_connector`).
    pub fn objects(&self) -> Vec<u32> {
        let mut objects = vec![u32::from(self.crtc), u32::from(self.plane)];
        objects.extend(self.cursor.as_ref().map(|cursor| u32::from(cursor.plane)));
        objects
    }

    /// The connector it drives (an [`Output`](crate::Output)'s id).
    pub fn connector(&self) -> u32 {
        u32::from(self.connector)
    }

    /// Turns the output off, its CRTC and planes freed for another (its monitor was unplugged).
    /// A frame on its way is never completed: the display is dropped after this.
    pub fn turn_off(&mut self) -> io::Result<()> {
        if !self.configured {
            return Ok(());
        }
        let card = &*self.card;
        let mut request = atomic::AtomicModeReq::new();
        request.add_property(
            self.connector,
            property_handle(card, self.connector, "CRTC_ID")?,
            property::Value::CRTC(None),
        );
        request.add_property(
            self.crtc,
            property_handle(card, self.crtc, "MODE_ID")?,
            property::Value::Blob(0),
        );
        request.add_property(
            self.crtc,
            property_handle(card, self.crtc, "ACTIVE")?,
            property::Value::Boolean(false),
        );
        let planes = [
            Some(self.plane),
            self.cursor.as_ref().map(|cursor| cursor.plane),
        ];
        for plane in planes.into_iter().flatten() {
            request.add_property(
                plane,
                property_handle(card, plane, "CRTC_ID")?,
                property::Value::CRTC(None),
            );
            request.add_property(
                plane,
                property_handle(card, plane, "FB_ID")?,
                property::Value::Framebuffer(None),
            );
        }
        card.atomic_commit(AtomicCommitFlags::ALLOW_MODESET, request)?;
        self.configured = false;
        Ok(())
    }

    pub fn size(&self) -> (u32, u32) {
        let (width, height) = self.mode.size();
        (u32::from(width), u32::from(height))
    }

    pub fn refresh_hz(&self) -> u32 {
        self.mode.vrefresh()
    }

    /// The mode it is driven at, as the card lists it ([`Output::modes`](crate::Output::modes)).
    pub fn mode(&self) -> crate::Mode {
        crate::Mode::of(&self.mode)
    }

    pub fn mode_name(&self) -> String {
        format!(
            "{}x{}@{}",
            self.mode.size().0,
            self.mode.size().1,
            self.mode.vrefresh()
        )
    }

    /// Lets go of the output for another session (a terminal switch): drops DRM master. Wait for
    /// the last flip first. The next frame presented after [`resume`](Self::resume) modesets
    /// again, since the other session may have changed the output.
    pub fn suspend(&mut self) -> io::Result<()> {
        self.card.release_master_lock()?;
        self.configured = false;
        Ok(())
    }

    /// The output was another session's meanwhile, its DRM master handled by the seat manager
    /// (logind drops and restores it): the next frame presented modesets again.
    pub fn taken_back(&mut self) {
        self.configured = false;
    }

    /// Takes DRM master again after [`suspend`](Self::suspend).
    pub fn resume(&mut self) -> io::Result<()> {
        self.card.acquire_master_lock()
    }

    /// Writes `pixels` (0x00RRGGBB, row by row at the display's size) into the back buffer.
    pub fn draw(&mut self, pixels: &[u32]) -> io::Result<()> {
        let (width, height) = self.size();
        let (buffer, _) = &mut self.buffers[self.back];
        let pitch = buffer.pitch() as usize;
        let mut mapping = self.card.map_dumb_buffer(buffer)?;
        for (y, row) in pixels
            .chunks_exact(width as usize)
            .take(height as usize)
            .enumerate()
        {
            let out = &mut mapping[y * pitch..y * pitch + width as usize * 4];
            for (pixel, bytes) in row.iter().zip(out.chunks_exact_mut(4)) {
                bytes.copy_from_slice(&pixel.to_le_bytes());
            }
        }
        Ok(())
    }

    /// Shows the back buffer: a modeset the first time, a page flip after it, each asking for a
    /// flip event (`wait_flip`).
    pub fn present(&mut self) -> io::Result<()> {
        let framebuffer = self.buffers[self.back].1;
        self.present_framebuffer(framebuffer)?;
        self.back = 1 - self.back;
        Ok(())
    }

    /// Shows any framebuffer of the display's size (an imported DMA-BUF, for one): a modeset
    /// the first time, a page flip after it.
    pub fn present_framebuffer(&mut self, framebuffer: framebuffer::Handle) -> io::Result<()> {
        self.present_framebuffer_after(framebuffer, None)
    }

    /// The same for a frame still being written: `completion` signals once it is done (the
    /// device's sync file for it, `VulkanCompositor::compose_to_scanout`). It goes with the commit
    /// as the plane's IN_FENCE_FD: the kernel flips to the frame once it signals, and the caller's
    /// thread goes on at once. A plane without that property is waited on here first.
    pub fn present_framebuffer_after(
        &mut self,
        framebuffer: framebuffer::Handle,
        completion: Option<std::os::fd::OwnedFd>,
    ) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        let card = &self.card;
        let mut request = atomic::AtomicModeReq::new();
        let mut flags = AtomicCommitFlags::PAGE_FLIP_EVENT;
        if !self.configured {
            let blob = card.create_property_blob(&self.mode)?;
            self.add_modeset(&mut request, blob)?;
            self.mode_blob = Some(blob);
            flags |= AtomicCommitFlags::ALLOW_MODESET;
        } else {
            flags |= AtomicCommitFlags::NONBLOCK;
        }
        request.add_property(
            self.plane,
            property_handle(card, self.plane, "FB_ID")?,
            property::Value::Framebuffer(Some(framebuffer)),
        );
        if let Some(completion) = &completion {
            match property_handle(card, self.plane, "IN_FENCE_FD") {
                Ok(handle) => request.add_property(
                    self.plane,
                    handle,
                    property::Value::SignedRange(i64::from(completion.as_raw_fd())),
                ),
                Err(_) => wait_readable(completion)?,
            }
        }
        if let Some(cursor) = &self.cursor {
            let plane = cursor.plane;
            let (width, height) = cursor.size;
            let shown = self.pointer.map(|(x, y, image)| {
                let image = &cursor.images[image];
                [
                    ("CRTC_ID", property::Value::CRTC(Some(self.crtc))),
                    (
                        "FB_ID",
                        property::Value::Framebuffer(Some(image.framebuffer)),
                    ),
                    ("SRC_X", property::Value::UnsignedRange(0)),
                    ("SRC_Y", property::Value::UnsignedRange(0)),
                    (
                        "SRC_W",
                        property::Value::UnsignedRange(u64::from(width) << 16),
                    ),
                    (
                        "SRC_H",
                        property::Value::UnsignedRange(u64::from(height) << 16),
                    ),
                    (
                        "CRTC_X",
                        property::Value::SignedRange(i64::from(x) - i64::from(image.hotspot.0)),
                    ),
                    (
                        "CRTC_Y",
                        property::Value::SignedRange(i64::from(y) - i64::from(image.hotspot.1)),
                    ),
                    ("CRTC_W", property::Value::UnsignedRange(u64::from(width))),
                    ("CRTC_H", property::Value::UnsignedRange(u64::from(height))),
                ]
            });
            match shown {
                Some(properties) => {
                    for (name, value) in properties {
                        request.add_property(plane, property_handle(card, plane, name)?, value);
                    }
                }
                None => {
                    request.add_property(
                        plane,
                        property_handle(card, plane, "CRTC_ID")?,
                        property::Value::CRTC(None),
                    );
                    request.add_property(
                        plane,
                        property_handle(card, plane, "FB_ID")?,
                        property::Value::Framebuffer(None),
                    );
                }
            }
        }
        card.atomic_commit(flags, request)?;
        self.configured = true;
        self.front = Some(framebuffer);
        self.shown_pointer = self.pointer;
        Ok(())
    }

    /// What a modeset adds to a commit: the connector on the CRTC at the display's mode (`blob`,
    /// its property blob), active, the primary plane covering it, and the planes another session
    /// left on the CRTC turned off.
    fn add_modeset(
        &self,
        request: &mut atomic::AtomicModeReq,
        blob: property::Value<'static>,
    ) -> io::Result<()> {
        let card = &*self.card;
        let (width, height) = self.size();
        request.add_property(
            self.connector,
            property_handle(card, self.connector, "CRTC_ID")?,
            property::Value::CRTC(Some(self.crtc)),
        );
        request.add_property(
            self.crtc,
            property_handle(card, self.crtc, "MODE_ID")?,
            blob,
        );
        request.add_property(
            self.crtc,
            property_handle(card, self.crtc, "ACTIVE")?,
            property::Value::Boolean(true),
        );
        for (name, value) in [
            ("CRTC_ID", property::Value::CRTC(Some(self.crtc))),
            ("SRC_X", property::Value::UnsignedRange(0)),
            ("SRC_Y", property::Value::UnsignedRange(0)),
            (
                "SRC_W",
                property::Value::UnsignedRange(u64::from(width) << 16),
            ),
            (
                "SRC_H",
                property::Value::UnsignedRange(u64::from(height) << 16),
            ),
            ("CRTC_X", property::Value::SignedRange(0)),
            ("CRTC_Y", property::Value::SignedRange(0)),
            ("CRTC_W", property::Value::UnsignedRange(u64::from(width))),
            ("CRTC_H", property::Value::UnsignedRange(u64::from(height))),
        ] {
            request.add_property(self.plane, property_handle(card, self.plane, name)?, value);
        }
        // Planes another session left on the CRTC (a cursor, an overlay) are turned off; the
        // cursor plane is the pointer's (below).
        let cursor = self.cursor.as_ref().map(|cursor| cursor.plane);
        for plane in card.plane_handles()? {
            if plane == self.plane || Some(plane) == cursor {
                continue;
            }
            let on_crtc = card
                .get_plane(plane)
                .is_ok_and(|info| info.crtc() == Some(self.crtc));
            if on_crtc {
                request.add_property(
                    plane,
                    property_handle(card, plane, "CRTC_ID")?,
                    property::Value::CRTC(None),
                );
                request.add_property(
                    plane,
                    property_handle(card, plane, "FB_ID")?,
                    property::Value::Framebuffer(None),
                );
            }
        }
        Ok(())
    }

    /// Whether the card takes the display at its mode, its CRTC and planes, before anything is
    /// shown (an atomic commit that only tests): a mode the connector lists may still be beyond
    /// what the link or the CRTC can drive.
    pub fn test(&self) -> io::Result<()> {
        let card = &*self.card;
        let blob = card.create_property_blob(&self.mode)?;
        let mut request = atomic::AtomicModeReq::new();
        self.add_modeset(&mut request, blob)?;
        request.add_property(
            self.plane,
            property_handle(card, self.plane, "FB_ID")?,
            property::Value::Framebuffer(Some(self.buffers[0].1)),
        );
        let tested = card.atomic_commit(
            AtomicCommitFlags::TEST_ONLY | AtomicCommitFlags::ALLOW_MODESET,
            request,
        );
        if let property::Value::Blob(id) = blob {
            let _ = card.destroy_property_blob(id);
        }
        tested
    }

    /// The presented frame's flip if it has happened: the kernel's timestamp of it
    /// (CLOCK_MONOTONIC). Never blocks; the card is readable once it has (`card().as_fd()`). The
    /// card's flips of other CRTCs, read here, wait for their displays.
    pub fn flip_done(&self) -> io::Result<Option<Duration>> {
        use rustix::event::{poll, PollFd, PollFlags, Timespec};
        let mut fds = [PollFd::new(&*self.card, PollFlags::IN)];
        let now = Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let readable = match poll(&mut fds, Some(&now)) {
            Ok(0) | Err(rustix::io::Errno::INTR) => false,
            Ok(_) => true,
            Err(error) => return Err(error.into()),
        };
        Ok(self.card.flips_of(self.crtc, readable)?.pop())
    }

    /// Waits for the presented frame's flip; returns the kernel's timestamp of it
    /// (CLOCK_MONOTONIC).
    pub fn wait_flip(&self) -> io::Result<Duration> {
        let mut read = false;
        loop {
            if let Some(flip) = self.card.flips_of(self.crtc, read)?.pop() {
                return Ok(flip);
            }
            read = true;
        }
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        for (buffer, framebuffer) in self.buffers.drain(..) {
            let _ = self.card.destroy_framebuffer(framebuffer);
            let _ = self.card.destroy_dumb_buffer(buffer);
        }
        if let Some(property::Value::Blob(blob)) = self.mode_blob.take() {
            let _ = self.card.destroy_property_blob(blob);
        }
        // The last display of the card gives master back; others still present on it.
        if Rc::strong_count(&self.card) == 1 {
            let _ = self.card.release_master_lock();
        }
    }
}

/// A buffer of the cursor plane's `size` holding `image`, transparent around it.
fn cursor_image(card: &Card, size: (u32, u32), image: &PointerImage) -> io::Result<CursorImage> {
    let (width, height) = size;
    if image.width > width
        || image.height > height
        || image.hotspot.0 >= image.width.max(1)
        || image.hotspot.1 >= image.height.max(1)
        || image.pixels.len() != (image.width * image.height) as usize
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "a {}x{} pointer image does not fit the {width}x{height} cursor plane",
                image.width, image.height
            ),
        ));
    }
    let mut buffer = card.create_dumb_buffer(size, DrmFourcc::Argb8888, 32)?;
    let pitch = buffer.pitch() as usize;
    {
        let mut mapping = card.map_dumb_buffer(&mut buffer)?;
        mapping.fill(0);
        for (y, row) in image.pixels.chunks_exact(image.width as usize).enumerate() {
            for (x, pixel) in row.iter().enumerate() {
                let at = y * pitch + x * 4;
                mapping[at..at + 4].copy_from_slice(&pixel.to_le_bytes());
            }
        }
    }
    let framebuffer = card.add_framebuffer(&buffer, 32, 32)?;
    Ok(CursorImage {
        buffer,
        framebuffer,
        hotspot: image.hotspot,
    })
}

/// The CRTC to drive a connector with: the one already lighting it (the firmware's, or the boot
/// logo's) when the connector may use it, as Plymouth and mutter choose, else the first it may
/// use. The same mode on the same CRTC is taken without turning the output off; another CRTC is
/// a full modeset, the output dark for a moment (docs/adr/0017-boot-logo.mdx).
pub(crate) fn choose_crtc(
    lit: Option<crtc::Handle>,
    possible: &[crtc::Handle],
) -> Option<crtc::Handle> {
    lit.filter(|lit| possible.contains(lit))
        .or_else(|| possible.first().copied())
}

/// Waits until `fd` (a sync file) is readable: what it stands for is done.
fn wait_readable(fd: &std::os::fd::OwnedFd) -> io::Result<()> {
    use rustix::event::{poll, PollFd, PollFlags};
    loop {
        let mut fds = [PollFd::new(fd, PollFlags::IN)];
        match poll(&mut fds, None) {
            Ok(_) => return Ok(()),
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Err(error.into()),
        }
    }
}

fn property_handle(
    card: &Card,
    handle: impl drm::control::ResourceHandle,
    name: &str,
) -> io::Result<property::Handle> {
    card.get_properties(handle)?
        .as_hashmap(card)?
        .get(name)
        .map(|info| info.handle())
        .ok_or_else(|| not_found(&format!("property {name}")))
}

fn property_value(
    card: &Card,
    handle: impl drm::control::ResourceHandle,
    name: &str,
) -> io::Result<u64> {
    let properties = card.get_properties(handle)?;
    for (&property, &value) in properties.iter() {
        if card.get_property(property)?.name().to_bytes() == name.as_bytes() {
            return Ok(value);
        }
    }
    Err(not_found(&format!("property {name}")))
}
