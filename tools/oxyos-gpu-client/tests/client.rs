//! The GPU client against a compositor played here, through a real socket, with the kernel's
//! DMA-BUFs and timelines and a real Vulkan device: its frames arrive as DMA-BUFs attached plane
//! by plane, each submitted with an acquire point that signals once its pixels are drawn, a press
//! answered with another colour, and an image drawn into again only after its release point
//! signalled. Without udmabuf, a render node with timelines or a Vulkan device that exports
//! DMA-BUF, it is skipped and says so (tools/gpu-vm runs it).

use oxyos_dmabuf::{DmaBuf, Mapping, RenderNode, Timeline};
use oxyos_ipc::{Connection, Listener};
use oxyos_protocol::{Event, Format, PointerKind, Rect, Request, ServerConnection, Size};
use rustix::fd::AsFd;
use std::sync::Arc;
use std::time::Duration;

const BLUE: u32 = 0x1c_64_d8;
const ORANGE: u32 = 0xe8_59_0c;
const WHITE: u32 = 0xff_ff_ff;

fn usable() -> Option<Arc<RenderNode>> {
    if oxyos_dmabuf::shared_dmabuf(4096).is_err() {
        eprintln!("no udmabuf here, skipped; tools/gpu-vm runs it");
        return None;
    }
    let node = RenderNode::with_timelines(None).map(Arc::new);
    if node.is_none() {
        eprintln!("no render node with timeline syncobjs here, skipped; tools/gpu-vm runs it");
    }
    node
}

/// A frame the client submitted: its buffer, the acquire and release points.
struct Frame {
    buffer: u64,
    timeline: u32,
    acquire: u64,
    release: u64,
}

/// A buffer attached: its planes' DMA-BUFs, each plane's offset and stride, its size.
type Attached = (Vec<DmaBuf>, Vec<(u32, u32)>, Size);

struct Played {
    server: Connection,
    node: Arc<RenderNode>,
    timelines: std::collections::HashMap<u32, Timeline>,
    buffers: std::collections::HashMap<u64, Attached>,
    planes: Vec<(DmaBuf, u32, u32)>,
    sync: Option<(u32, u64, u64)>,
}

impl Played {
    /// Reads requests until a Submit, keeping the buffers and timelines.
    fn next_frame(&mut self) -> Frame {
        loop {
            let (request, fd) = self.server.recv_request().unwrap();
            match request {
                Request::AttachDmabufPlane { offset, stride, .. } => {
                    self.planes
                        .push((DmaBuf::from_fd(fd.unwrap()).unwrap(), offset, stride));
                }
                Request::AttachDmabuf {
                    buffer,
                    format,
                    modifier,
                    extent,
                    planes,
                } => {
                    assert_eq!((format, modifier), (Format::Argb8888, 0));
                    assert_eq!(planes as usize, self.planes.len());
                    let planes = std::mem::take(&mut self.planes);
                    let layouts = planes.iter().map(|(_, o, s)| (*o, *s)).collect();
                    let bufs = planes.into_iter().map(|(b, _, _)| b).collect();
                    self.buffers.insert(buffer, (bufs, layouts, extent));
                }
                Request::ImportTimeline { timeline } => {
                    self.timelines.insert(
                        timeline,
                        Timeline::import(&self.node, fd.unwrap().as_fd()).unwrap(),
                    );
                }
                Request::SetSync {
                    acquire_point,
                    release_point,
                    acquire_timeline,
                    release_timeline,
                    ..
                } => {
                    assert_eq!(acquire_timeline, release_timeline, "a timeline per buffer");
                    self.sync = Some((acquire_timeline, acquire_point, release_point));
                }
                Request::Submit { buffer, .. } => {
                    // Shared memory (no GPU buffer at all): the caller skips.
                    let (timeline, acquire, release) = match self.sync.take() {
                        Some(sync) => sync,
                        None if self.buffers.is_empty() => (0, 0, 1),
                        None => panic!("SetSync before a GPU buffer's Submit"),
                    };
                    assert!(release > acquire);
                    return Frame {
                        buffer,
                        timeline,
                        acquire,
                        release,
                    };
                }
                _ => {}
            }
        }
    }

    /// The frame's pixel at (x, y), 0x00RRGGBB, once its acquire point signalled.
    fn pixel(&self, frame: &Frame, x: u32, y: u32) -> u32 {
        let timeline = &self.timelines[&frame.timeline];
        let ready = timeline.eventfd(frame.acquire, false).unwrap();
        let mut fds = [rustix::event::PollFd::new(
            &ready,
            rustix::event::PollFlags::IN,
        )];
        let timeout = rustix::event::Timespec {
            tv_sec: 10,
            tv_nsec: 0,
        };
        assert_eq!(
            rustix::event::poll(&mut fds, Some(&timeout)).unwrap(),
            1,
            "the acquire point signals"
        );
        let (buffers, layouts, _) = &self.buffers[&frame.buffer];
        let mapping = Mapping::new(buffers[0].try_clone().unwrap()).unwrap();
        let mut out = [0; 4];
        mapping
            .read_rows(layouts[0].0 + x * 4, layouts[0].1, 4, y..y + 1, &mut out)
            .unwrap();
        u32::from_le_bytes(out) & 0x00ff_ffff
    }
}

#[test]
fn frames_drawn_on_the_gpu_arrive_as_dmabufs_with_their_points() {
    let Some(node) = usable() else {
        return;
    };
    let directory =
        std::env::temp_dir().join(format!("oxyos-gpu-client-test-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_oxyos-gpu-client"))
        .args(["--size", "320x200", "--wait-ms", "5000"])
        .env("OXYOS_COMPOSITOR_SOCKET", &socket)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let server = listener.accept().unwrap();
    server.set_timeouts(Duration::from_secs(20)).unwrap();
    let mut protocol = ServerConnection::new();
    let (hello, _) = server.recv_request().unwrap();
    server
        .send_event(&protocol.receive(&hello).unwrap().unwrap(), None)
        .unwrap();
    for event in [
        Event::GpuDevice {
            device: 0,
            formats: 1,
        },
        Event::GpuFormat {
            index: 0,
            format: Format::Argb8888,
            modifier: 0,
        },
        Event::Configure {
            surface: 1,
            serial: 1,
            rect: Rect::new(0, 0, 320, 200),
            scale_120: 120,
            fullscreen: false,
            maximized: false,
        },
    ] {
        server.send_event(&event, None).unwrap();
    }
    let mut played = Played {
        server,
        node,
        timelines: Default::default(),
        buffers: Default::default(),
        planes: Vec::new(),
        sync: None,
    };
    let first = played.next_frame();
    if played.buffers.is_empty() {
        // No Vulkan device exports DMA-BUF here: the client said so and used shared memory.
        let _ = child.kill();
        let output = child.wait_with_output().unwrap();
        eprintln!(
            "the client used shared memory, skipped: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    assert_eq!(played.buffers[&first.buffer].2, Size::new(320, 200));
    assert_eq!(played.pixel(&first, 5, 100), BLUE);
    assert_eq!(played.pixel(&first, 30, 30), WHITE);
    // A press: the next frame is orange, in another image (the first is still shown).
    played
        .server
        .send_event(
            &Event::Pointer {
                surface: 1,
                serial: 2,
                kind: PointerKind::Press,
                x: 10 << 8,
                y: 10 << 8,
                button: 272,
            },
            None,
        )
        .unwrap();
    let second = played.next_frame();
    assert_ne!(second.buffer, first.buffer);
    assert_eq!(played.pixel(&second, 5, 100), ORANGE);
    // Three images held: the fourth frame waits for a release.
    for serial in 3..5 {
        played
            .server
            .send_event(
                &Event::Pointer {
                    surface: 1,
                    serial,
                    kind: PointerKind::Press,
                    x: 10 << 8,
                    y: 10 << 8,
                    button: 272,
                },
                None,
            )
            .unwrap();
        if serial == 4 {
            // Released: the first image is drawn into again.
            played.timelines[&first.timeline]
                .signal(first.release)
                .unwrap();
        }
        let frame = played.next_frame();
        if serial == 4 {
            assert_eq!(frame.buffer, first.buffer, "the released image is reused");
            assert_eq!(played.buffers.len(), 3, "no fourth image");
        }
    }
    drop(played);
    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("[gpu-client] gpu: "), "{stderr}");
    assert!(
        stderr.contains("[gpu-client] frame 4 320x200 gpu modifier 0x0"),
        "{stderr}"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn without_a_gpu_table_the_client_draws_into_shared_memory() {
    let directory =
        std::env::temp_dir().join(format!("oxyos-gpu-client-shm-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_oxyos-gpu-client"))
        .args(["--size", "64x48", "--wait-ms", "50"])
        .env("OXYOS_COMPOSITOR_SOCKET", &socket)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let server = listener.accept().unwrap();
    server.set_timeouts(Duration::from_secs(20)).unwrap();
    let mut protocol = ServerConnection::new();
    let (hello, _) = server.recv_request().unwrap();
    server
        .send_event(&protocol.receive(&hello).unwrap().unwrap(), None)
        .unwrap();
    server
        .send_event(
            &Event::Configure {
                surface: 1,
                serial: 1,
                rect: Rect::new(0, 0, 64, 48),
                scale_120: 120,
                fullscreen: false,
                maximized: false,
            },
            None,
        )
        .unwrap();
    let shared = loop {
        let (request, fd) = server.recv_request().unwrap();
        if let Request::AttachBuffer { .. } = request {
            break oxyos_ipc::SharedBuffer::import(fd.unwrap(), request.buffer_layout().unwrap())
                .unwrap();
        }
        assert!(
            !matches!(request, Request::AttachDmabuf { .. }),
            "no GPU buffer without a table"
        );
    };
    let mut row = vec![0; 64 * 4];
    shared.read_row(2, &mut row).unwrap();
    assert_eq!(
        u32::from_le_bytes(row[8..12].try_into().unwrap()),
        0xff00_0000 | BLUE
    );
    drop(server);
    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("[gpu-client] shared memory: the compositor named no GPU"),
        "{stderr}"
    );
    std::fs::remove_dir_all(directory).unwrap();
}
