//! A minimal native OxyOS client that draws on the GPU (docs/adr/0031-gpu-buffers.mdx): one window
//! whose frames a Vulkan device draws into images exported as DMA-BUF with a modifier from the
//! compositor's table, attached once each, submitted with an acquire point (the drawing's
//! completion) and a release point it waits for before drawing into an image again. It answers
//! every configure at its size and every press with another colour, and falls back to shared
//! memory, drawn on the CPU, when the compositor names no GPU, its device cannot share with the
//! compositor's or an import fails.
//!
//! What it draws, at the window's scale: the background (blue, then orange, green and purple,
//! one press after another), a white square at logical (24, 24)–(104, 104) and a dark bar along
//! the bottom 32 logical pixels.
//!
//! ```text
//! oxyos-gpu-client [--size WIDTHxHEIGHT] [--shared-memory] [--wait-ms MS]
//! ```
//!
//! It connects to `$OXYOS_COMPOSITOR_SOCKET` and runs until the compositor closes the
//! connection. On stderr: `[gpu-client] gpu: DEVICE, modifiers …` or `[gpu-client] shared
//! memory: WHY` once, then a line per frame presented (`[gpu-client] frame N WxH gpu|shared
//! memory…`).

mod vulkan;

use oxyos_client::{Client, Poll, SyncPoints};
use oxyos_protocol::{Event, Format, PointerKind, Request, Size};
use rustix::fd::AsFd;
use std::io;
use std::time::{Duration, Instant};
use vulkan::{Fill, Gpu, Image};

const SURFACE: u32 = 1;
/// Images kept for one size: one shown, one queued, one drawn into.
const IMAGES: usize = 3;
const BACKGROUNDS: [u32; 4] = [0x1c_64_d8, 0xe8_59_0c, 0x2f_9e_44, 0x70_48_e8];
const SQUARE: u32 = 0xff_ff_ff;
const BAR: u32 = 0x21_25_29;

struct Options {
    size: Size,
    shared_memory: bool,
    wait: Duration,
}

fn options() -> Result<Options, String> {
    let mut options = Options {
        size: Size::new(640, 400),
        shared_memory: false,
        wait: Duration::from_millis(500),
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--size" => {
                let value = args.next().ok_or("--size needs WIDTHxHEIGHT")?;
                let (width, height) = value.split_once('x').ok_or("--size is WIDTHxHEIGHT")?;
                options.size = Size::new(
                    width.parse().map_err(|_| "a width")?,
                    height.parse().map_err(|_| "a height")?,
                );
            }
            "--shared-memory" => options.shared_memory = true,
            "--wait-ms" => {
                let value = args.next().ok_or("--wait-ms needs a number")?;
                options.wait = Duration::from_millis(value.parse().map_err(|_| "milliseconds")?);
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(options)
}

/// The frame's rectangles in pixels at `scale_120` (the first the background), each colour
/// 0x00RRGGBB, opaque.
fn picture(extent: Size, scale_120: u32, presses: usize) -> Vec<(u32, u32, u32, u32, u32)> {
    let at = |logical: u32| logical * scale_120 / 120;
    let bar = at(32).min(extent.height);
    vec![
        (0, 0, extent.width, extent.height, BACKGROUNDS[presses % 4]),
        (at(24), at(24), at(80), at(80), SQUARE),
        (0, extent.height - bar, extent.width, bar, BAR),
    ]
}

fn fills(rects: &[(u32, u32, u32, u32, u32)]) -> Vec<Fill> {
    rects
        .iter()
        .map(|&(x, y, width, height, color)| Fill {
            x,
            y,
            width,
            height,
            rgba: [
                ((color >> 16) & 0xff) as f32 / 255.0,
                ((color >> 8) & 0xff) as f32 / 255.0,
                (color & 0xff) as f32 / 255.0,
                1.0,
            ],
        })
        .collect()
}

/// The same frame drawn on the CPU, premultiplied B, G, R, A rows.
fn pixels(extent: Size, rects: &[(u32, u32, u32, u32, u32)]) -> Vec<u8> {
    let mut out = vec![0; extent.width as usize * extent.height as usize * 4];
    for &(x, y, width, height, color) in rects {
        for row in y..(y + height).min(extent.height) {
            for column in x..(x + width).min(extent.width) {
                let at = (row as usize * extent.width as usize + column as usize) * 4;
                out[at..at + 4].copy_from_slice(&(0xff00_0000 | color).to_le_bytes());
            }
        }
    }
    out
}

/// An image attached to the connection with a timeline of its own: each frame drawn into it is
/// the next two points, its acquire and its release. A timeline's point signals with every
/// earlier one, so a release point is never on another buffer's timeline: frames are released out
/// of order (a queued one superseded while the one before it is still shown).
struct Attached {
    image: Image,
    buffer: u64,
    timeline: u32,
    /// The latest point named on its timeline, and the release point of its latest frame.
    point: u64,
    release: Option<u64>,
}

struct GpuPath {
    gpu: Gpu,
    modifiers: Vec<u64>,
    images: Vec<Attached>,
}

enum Path {
    /// Waiting for the compositor's table until the deadline.
    Undecided(Instant),
    Gpu(Box<GpuPath>),
    Shared,
}

fn shared(path: &mut Path, why: &str) {
    eprintln!("[gpu-client] shared memory: {why}");
    *path = Path::Shared;
}

/// The GPU path with the compositor's table, or why not.
fn gpu_path(client: &mut Client) -> Result<GpuPath, String> {
    let table = client.gpu().ok_or("no GPU table")?.clone();
    let offered: Vec<u64> = table
        .formats
        .iter()
        .filter(|(format, _)| *format == Format::Argb8888)
        .map(|(_, modifier)| *modifier)
        .collect();
    let gpu = Gpu::open(table.device)?;
    let modifiers = gpu.shareable(&offered);
    if modifiers.is_empty() {
        return Err(format!(
            "{} exports none of the compositor's Argb8888 modifiers {offered:x?}",
            gpu.name
        ));
    }
    eprintln!(
        "[gpu-client] gpu: {}{}, modifiers {:x?}, completion {}",
        gpu.name,
        if gpu.same_device {
            " (the compositor's device)"
        } else {
            " (another device: linear)"
        },
        modifiers,
        if gpu.exports_sync_files() {
            "as a sync file"
        } else {
            "waited for"
        }
    );
    Ok(GpuPath {
        gpu,
        modifiers,
        images: Vec::new(),
    })
}

impl GpuPath {
    fn released(&self, client: &Client, attached: &Attached) -> bool {
        attached.release.is_none_or(|point| {
            client
                .timeline_signalled(attached.timeline)
                .is_ok_and(|signalled| signalled >= point)
        })
    }

    /// Detaches and frees images of another size once the compositor no longer reads them.
    fn collect(&mut self, client: &mut Client, extent: Size) -> io::Result<()> {
        let mut index = 0;
        while index < self.images.len() {
            let attached = &self.images[index];
            let stale = (attached.image.width, attached.image.height)
                != (extent.width, extent.height)
                || client.buffer_failed(attached.buffer);
            if stale && self.released(client, attached) {
                let attached = self.images.swap_remove(index);
                client.detach_buffer(attached.buffer)?;
                client.destroy_timeline(attached.timeline)?;
                self.gpu.destroy(attached.image);
            } else {
                index += 1;
            }
        }
        Ok(())
    }

    /// An image of `extent` no frame holds, made and attached if there are fewer than three;
    /// None: all are held, and the eventfd of the first release to wait for.
    fn free(
        &mut self,
        client: &mut Client,
        extent: Size,
    ) -> Result<Result<usize, Option<rustix::fd::OwnedFd>>, String> {
        let of_size: Vec<usize> = (0..self.images.len())
            .filter(|&index| {
                let image = &self.images[index].image;
                (image.width, image.height) == (extent.width, extent.height)
            })
            .collect();
        if let Some(&index) = of_size
            .iter()
            .find(|&&index| self.released(client, &self.images[index]))
        {
            return Ok(Ok(index));
        }
        if of_size.len() < IMAGES {
            let image = self
                .gpu
                .image(extent.width, extent.height, &self.modifiers)?;
            let planes: Vec<_> = image
                .planes
                .iter()
                .map(|&(offset, stride)| (image.fd.as_fd(), offset, stride))
                .collect();
            let buffer = client
                .attach_dmabuf(Format::Argb8888, image.modifier, extent, &planes)
                .map_err(|error| format!("attach: {error}"))?;
            let timeline = client
                .create_timeline()
                .map_err(|error| format!("no timeline: {error}"))?;
            self.images.push(Attached {
                image,
                buffer,
                timeline,
                point: 0,
                release: None,
            });
            return Ok(Ok(self.images.len() - 1));
        }
        // The oldest frame's image is the likeliest released first.
        let first = of_size
            .iter()
            .map(|&index| &self.images[index])
            .filter_map(|attached| Some((attached.release?, attached.timeline)))
            .min_by_key(|&(point, _)| point);
        Ok(Err(first.and_then(|(point, timeline)| {
            client.timeline_eventfd(timeline, point, false).ok()
        })))
    }
}

fn main() {
    let options = match options() {
        Ok(options) => options,
        Err(error) => {
            eprintln!("oxyos-gpu-client: {error}");
            std::process::exit(2);
        }
    };
    if let Err(error) = run(options) {
        eprintln!("oxyos-gpu-client: {error}");
        std::process::exit(1);
    }
}

fn run(options: Options) -> io::Result<()> {
    let socket = std::env::var_os("OXYOS_COMPOSITOR_SOCKET")
        .ok_or_else(|| io::Error::other("OXYOS_COMPOSITOR_SOCKET is not set"))?;
    let mut client = Client::connect(std::path::Path::new(&socket))?;
    client.send(&Request::CreateToplevel {
        surface: SURFACE,
        size: options.size,
    })?;
    client.name_window(SURFACE, false, "GPU client")?;
    let mut path = if options.shared_memory {
        eprintln!("[gpu-client] shared memory: asked for");
        Path::Shared
    } else {
        Path::Undecided(Instant::now() + options.wait)
    };
    // The latest configure (serial, buffer size, scale) and whether a frame answers it yet.
    let mut configure: Option<(u64, Size, u32)> = None;
    let mut presses = 0usize;
    let mut dirty = false;
    let mut waiting: Option<rustix::fd::OwnedFd> = None;
    let mut frames = 0u64;
    loop {
        {
            use rustix::event::{poll, PollFd, PollFlags, Timespec};
            let mut fds = vec![PollFd::new(&client, PollFlags::IN)];
            if let Some(fd) = &waiting {
                fds.push(PollFd::new(fd, PollFlags::IN));
            }
            let timeout = Timespec {
                tv_sec: 0,
                tv_nsec: 50_000_000,
            };
            match poll(&mut fds, Some(&timeout)) {
                Ok(_) | Err(rustix::io::Errno::INTR) => {}
                Err(error) => return Err(error.into()),
            }
        }
        loop {
            match client.poll()? {
                Poll::Event(event) => match event {
                    Event::Configure {
                        surface: SURFACE,
                        serial,
                        rect,
                        scale_120,
                        ..
                    } => {
                        let pixels = |logical: u32| (logical * scale_120).div_ceil(120);
                        configure = Some((
                            serial,
                            Size::new(pixels(rect.width), pixels(rect.height)),
                            scale_120,
                        ));
                        dirty = true;
                    }
                    Event::Pointer {
                        surface: SURFACE,
                        kind: PointerKind::Press,
                        ..
                    } => {
                        presses += 1;
                        dirty = true;
                    }
                    Event::BufferFailed { buffer } => {
                        if matches!(path, Path::Gpu(_)) {
                            shared(
                                &mut path,
                                &format!("the compositor could not import buffer {buffer}"),
                            );
                            dirty = true;
                        }
                    }
                    Event::GpuDevice { formats: 0, .. } => {
                        if matches!(path, Path::Gpu(_)) {
                            shared(&mut path, "the compositor withdrew its GPU");
                            dirty = true;
                        }
                    }
                    _ => {}
                },
                Poll::Empty => break,
                Poll::Closed => return Ok(()),
            }
        }
        if let Path::Undecided(deadline) = path {
            if client.gpu().is_some() {
                path = match gpu_path(&mut client) {
                    Ok(gpu) => Path::Gpu(Box::new(gpu)),
                    Err(why) => {
                        eprintln!("[gpu-client] shared memory: {why}");
                        Path::Shared
                    }
                };
            } else if Instant::now() >= deadline {
                shared(&mut path, "the compositor named no GPU");
            }
        }
        let Some((serial, extent, scale_120)) = configure else {
            continue;
        };
        if !dirty || matches!(path, Path::Undecided(_)) {
            continue;
        }
        let rects = picture(extent, scale_120, presses);
        let mut fallback = None;
        match &mut path {
            Path::Gpu(gpu) => {
                gpu.collect(&mut client, extent)?;
                let index = match gpu.free(&mut client, extent) {
                    Ok(Ok(index)) => index,
                    Ok(Err(eventfd)) => {
                        waiting = eventfd;
                        continue;
                    }
                    Err(why) => {
                        fallback = Some(why);
                        usize::MAX
                    }
                };
                if fallback.is_none() {
                    waiting = None;
                    let attached = &mut gpu.images[index];
                    let timeline = attached.timeline;
                    let acquire = attached.point + 1;
                    let release = attached.point + 2;
                    attached.point = release;
                    match gpu.gpu.draw(&mut attached.image, &fills(&rects)) {
                        Ok(Some(done)) => {
                            client.timeline_import_sync_file(timeline, acquire, done.as_fd())?
                        }
                        // Drawn already: the acquire point is signalled at once.
                        Ok(None) => client.timeline_signal(timeline, acquire)?,
                        Err(why) => fallback = Some(why),
                    }
                    if fallback.is_none() {
                        attached.release = Some(release);
                        let sequence = client.submit_buffer_synced(
                            SURFACE,
                            attached.buffer,
                            serial,
                            None,
                            Some(SyncPoints {
                                acquire: (timeline, acquire),
                                release: (timeline, release),
                            }),
                        )?;
                        frames += 1;
                        eprintln!(
                            "[gpu-client] frame {frames} {}x{} gpu modifier {:#x} sequence {sequence}",
                            extent.width, extent.height, attached.image.modifier
                        );
                    }
                }
            }
            Path::Shared => {
                let sequence = client.present_format(
                    SURFACE,
                    serial,
                    extent,
                    Format::Argb8888,
                    &pixels(extent, &rects),
                )?;
                frames += 1;
                eprintln!(
                    "[gpu-client] frame {frames} {}x{} shared memory sequence {sequence}",
                    extent.width, extent.height
                );
            }
            Path::Undecided(_) => unreachable!("decided above"),
        }
        if let Some(why) = fallback {
            shared(&mut path, &why);
            // Drawn again on the CPU in the next turn.
            continue;
        }
        dirty = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_is_its_rectangles_at_the_windows_scale() {
        let extent = Size::new(200, 150);
        let rects = picture(extent, 150, 1);
        // At 1.25: the square from 30 to 130, the bar the bottom 40 pixels.
        assert_eq!(rects[1], (30, 30, 100, 100, SQUARE));
        assert_eq!(rects[2], (0, 110, 200, 40, BAR));
        let drawn = pixels(extent, &rects);
        let at = |x: usize, y: usize| {
            u32::from_le_bytes(drawn[(y * 200 + x) * 4..][..4].try_into().unwrap())
        };
        assert_eq!(at(5, 5), 0xff00_0000 | BACKGROUNDS[1]);
        assert_eq!(at(30, 30), 0xff00_0000 | SQUARE);
        assert_eq!(at(129, 109), 0xff00_0000 | SQUARE);
        assert_eq!(at(199, 149), 0xff00_0000 | BAR);
        // Another press, another background; four presses come back to the first.
        assert_eq!(picture(extent, 120, 2)[0].4, BACKGROUNDS[2]);
        assert_eq!(picture(extent, 120, 4)[0].4, BACKGROUNDS[0]);
    }
}
