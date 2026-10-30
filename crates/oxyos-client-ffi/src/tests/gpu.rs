//! GPU buffers' C ABI (ADR 0031): the constants, structs and declarations against the header,
//! and the calls through a socket: the tables told as events, refusals before anything is sent,
//! a DMA-BUF and a timeline where the kernel has them.

use super::*;
use crate::gpu::*;
use std::os::fd::{BorrowedFd, FromRawFd};

#[test]
fn the_header_matches_the_gpu_abi() {
    for (name, value) in [
        ("OXYOS_EVENT_GPU_DEVICE", i64::from(OXYOS_EVENT_GPU_DEVICE)),
        (
            "OXYOS_EVENT_BUFFER_FAILED",
            i64::from(OXYOS_EVENT_BUFFER_FAILED),
        ),
        (
            "OXYOS_EVENT_GPU_SCANOUT",
            i64::from(OXYOS_EVENT_GPU_SCANOUT),
        ),
        ("OXYOS_MODIFIER_LINEAR", OXYOS_MODIFIER_LINEAR as i64),
    ] {
        assert_eq!(define(name), value, "{name}");
    }
    // The events are the protocol's opcodes.
    assert_eq!(
        Event::GpuDevice {
            device: 0,
            formats: 0
        }
        .opcode(),
        OXYOS_EVENT_GPU_DEVICE as u8
    );
    assert_eq!(
        Event::BufferFailed { buffer: 1 }.opcode(),
        OXYOS_EVENT_BUFFER_FAILED as u8
    );
    assert_eq!(
        Event::GpuScanout {
            surface: 1,
            formats: 0
        }
        .opcode(),
        OXYOS_EVENT_GPU_SCANOUT as u8
    );

    let format = fields("OxyOSGpuFormat");
    assert_eq!(names(&format), ["format", "modifier"]);
    let rust = [
        offset_of!(OxyOSGpuFormat, format),
        offset_of!(OxyOSGpuFormat, modifier),
    ];
    assert_eq!(
        c_offsets(&format),
        (rust.to_vec(), size_of::<OxyOSGpuFormat>())
    );
    let plane = fields("OxyOSDmabufPlane");
    assert_eq!(names(&plane), ["fd", "offset", "stride"]);
    let rust = [
        offset_of!(OxyOSDmabufPlane, fd),
        offset_of!(OxyOSDmabufPlane, offset),
        offset_of!(OxyOSDmabufPlane, stride),
    ];
    assert_eq!(
        c_offsets(&plane),
        (rust.to_vec(), size_of::<OxyOSDmabufPlane>())
    );
    let sync = fields("OxyOSSync");
    assert_eq!(
        names(&sync),
        [
            "acquire_timeline",
            "acquire_point",
            "release_timeline",
            "release_point"
        ]
    );
    let rust = [
        offset_of!(OxyOSSync, acquire_timeline),
        offset_of!(OxyOSSync, acquire_point),
        offset_of!(OxyOSSync, release_timeline),
        offset_of!(OxyOSSync, release_point),
    ];
    assert_eq!(c_offsets(&sync), (rust.to_vec(), size_of::<OxyOSSync>()));

    type Formats = unsafe extern "C" fn(*mut OxyOSClient, *mut OxyOSGpuFormat, usize) -> i64;
    type ScanoutFormats =
        unsafe extern "C" fn(*mut OxyOSClient, u32, *mut OxyOSGpuFormat, usize) -> i64;
    type Attach = unsafe extern "C" fn(
        *mut OxyOSClient,
        u32,
        u64,
        u32,
        u32,
        *const OxyOSDmabufPlane,
        usize,
    ) -> i64;
    type Point = unsafe extern "C" fn(*mut OxyOSClient, u32, u64) -> c_int;
    type Submit = unsafe extern "C" fn(
        *mut OxyOSClient,
        u32,
        u64,
        u64,
        *const OxyOSRect,
        usize,
        *const OxyOSSync,
    ) -> i64;
    let device: unsafe extern "C" fn(*mut OxyOSClient, *mut u64) -> c_int = oxyos_client_gpu_device;
    let formats: Formats = oxyos_client_gpu_formats;
    let scanout: ScanoutFormats = oxyos_client_gpu_scanout_formats;
    let attach: Attach = oxyos_client_attach_dmabuf;
    let create: unsafe extern "C" fn(*mut OxyOSClient) -> i64 = oxyos_client_create_timeline;
    let import: unsafe extern "C" fn(*mut OxyOSClient, c_int) -> i64 = oxyos_client_import_timeline;
    let destroy: unsafe extern "C" fn(*mut OxyOSClient, u32) -> c_int =
        oxyos_client_destroy_timeline;
    let import_sync: unsafe extern "C" fn(*mut OxyOSClient, u32, u64, c_int) -> c_int =
        oxyos_client_timeline_import_sync_file;
    let export_sync: Point = oxyos_client_timeline_export_sync_file;
    let signal: Point = oxyos_client_timeline_signal;
    let export: unsafe extern "C" fn(*mut OxyOSClient, u32) -> c_int = oxyos_client_timeline_export;
    let eventfd: unsafe extern "C" fn(*mut OxyOSClient, u32, u64, bool) -> c_int =
        oxyos_client_timeline_eventfd;
    let submit: Submit = oxyos_client_submit_buffer_synced;
    for (name, rust) in [
        ("oxyos_client_gpu_device", type_of(device)),
        ("oxyos_client_gpu_formats", type_of(formats)),
        ("oxyos_client_gpu_scanout_formats", type_of(scanout)),
        ("oxyos_client_attach_dmabuf", type_of(attach)),
        ("oxyos_client_create_timeline", type_of(create)),
        ("oxyos_client_import_timeline", type_of(import)),
        ("oxyos_client_destroy_timeline", type_of(destroy)),
        (
            "oxyos_client_timeline_import_sync_file",
            type_of(import_sync),
        ),
        (
            "oxyos_client_timeline_export_sync_file",
            type_of(export_sync),
        ),
        ("oxyos_client_timeline_signal", type_of(signal)),
        ("oxyos_client_timeline_export", type_of(export)),
        ("oxyos_client_timeline_eventfd", type_of(eventfd)),
        ("oxyos_client_submit_buffer_synced", type_of(submit)),
    ] {
        assert_eq!(declaration(name), rust, "{name}");
    }
}

/// Polls until an event the caller sees.
///
/// # Safety
/// `client` is a live client of this thread.
#[allow(unsafe_code)]
unsafe fn event(client: *mut OxyOSClient) -> OxyOSEvent {
    let mut out = OxyOSEvent::default();
    for _ in 0..1000 {
        // SAFETY: forwarded contract; `out` is a live local.
        match unsafe { oxyos_client_poll(client, &mut out) } {
            OXYOS_POLL_EVENT => return out,
            OXYOS_POLL_EMPTY => std::thread::sleep(std::time::Duration::from_millis(2)),
            other => panic!("poll: {other}"),
        }
    }
    panic!("no event");
}

#[test]
#[allow(unsafe_code)]
fn the_tables_come_as_events_and_gpu_buffers_go_as_requests() {
    let directory =
        std::env::temp_dir().join(format!("oxyos-client-ffi-gpu-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let (done, finished) = std::sync::mpsc::channel::<()>();
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        server
            .send_event(&protocol.receive(&hello).unwrap().unwrap(), None)
            .unwrap();
        for event in [
            Event::GpuDevice {
                device: 0xe280,
                formats: 2,
            },
            Event::GpuFormat {
                index: 0,
                format: oxyos_protocol::Format::Argb8888,
                modifier: 0,
            },
            Event::GpuFormat {
                index: 1,
                format: oxyos_protocol::Format::Xrgb8888,
                modifier: 0x0100_0000_0000_0002,
            },
            Event::GpuScanout {
                surface: 3,
                formats: 1,
            },
            Event::GpuScanoutFormat {
                surface: 3,
                index: 0,
                format: oxyos_protocol::Format::Xrgb8888,
                modifier: 0x0100_0000_0000_0002,
            },
            Event::BufferFailed { buffer: 7 },
        ] {
            server.send_event(&event, None).unwrap();
        }
        // Everything the client sent until it says it is done.
        let _ = finished.recv();
        let mut requests = Vec::new();
        while let Ok((request, fd)) = server.recv_request() {
            requests.push((request, fd.is_some()));
            if request == (Request::Goodbye {}) {
                break;
            }
        }
        requests
    });
    let path = CString::new(socket.to_str().unwrap()).unwrap();
    // SAFETY: the client pointer comes from oxyos_client_connect, is used by this thread only and
    // is closed once at the end; every other pointer is to a live local of the right type.
    let timeline = unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        let mut device = 0;
        assert_eq!(
            oxyos_client_gpu_device(client, &mut device),
            0,
            "no table yet"
        );
        let gpu = event(client);
        assert_eq!((gpu.kind, gpu.detail), (OXYOS_EVENT_GPU_DEVICE, 2));
        assert_eq!(oxyos_client_gpu_device(client, &mut device), 1);
        assert_eq!(device, 0xe280);
        let mut formats = [OxyOSGpuFormat::default(); 1];
        assert_eq!(oxyos_client_gpu_formats(client, formats.as_mut_ptr(), 1), 2);
        assert_eq!(
            formats[0],
            OxyOSGpuFormat {
                format: OXYOS_FORMAT_ARGB8888,
                modifier: 0
            }
        );
        let scanout = event(client);
        assert_eq!(
            (scanout.kind, scanout.surface, scanout.detail),
            (OXYOS_EVENT_GPU_SCANOUT, 3, 1)
        );
        assert_eq!(
            oxyos_client_gpu_scanout_formats(client, 3, formats.as_mut_ptr(), 1),
            1
        );
        assert_eq!(formats[0].format, OXYOS_FORMAT_XRGB8888);
        assert_eq!(
            oxyos_client_gpu_scanout_formats(client, 4, std::ptr::null_mut(), 0),
            0
        );
        let failed = event(client);
        assert_eq!((failed.kind, failed.serial), (OXYOS_EVENT_BUFFER_FAILED, 7));

        // Refused before anything is sent: a memfd, no planes, an unknown timeline.
        let memfd = rustix::fs::memfd_create("plane", rustix::fs::MemfdFlags::CLOEXEC).unwrap();
        rustix::fs::ftruncate(&memfd, 4096).unwrap();
        let plane = OxyOSDmabufPlane {
            fd: memfd.as_raw_fd(),
            offset: 0,
            stride: 64,
        };
        assert_eq!(
            oxyos_client_attach_dmabuf(client, OXYOS_FORMAT_ARGB8888, 0, 16, 16, &plane, 1),
            -1
        );
        assert_eq!(
            oxyos_client_attach_dmabuf(client, OXYOS_FORMAT_ARGB8888, 0, 16, 16, &plane, 0),
            -1
        );
        let sync = OxyOSSync {
            acquire_timeline: 9,
            acquire_point: 1,
            ..OxyOSSync::default()
        };
        assert_eq!(
            oxyos_client_submit_buffer_synced(client, 1, 1, 1, std::ptr::null(), 0, &sync),
            -1
        );
        assert_eq!(oxyos_client_timeline_signal(client, 9, 1), OXYOS_ERROR);

        // A DMA-BUF and a timeline, where the kernel has them.
        let mut timeline = None;
        if let Ok((_memory, buffer)) = oxyos_dmabuf::shared_dmabuf(64 * 16) {
            let plane = OxyOSDmabufPlane {
                fd: buffer.as_fd().as_raw_fd(),
                offset: 0,
                stride: 64,
            };
            let id =
                oxyos_client_attach_dmabuf(client, OXYOS_FORMAT_ARGB8888, 0, 16, 16, &plane, 1);
            assert!(id > 0);
            let created = oxyos_client_create_timeline(client);
            if created > 0 {
                let created = created as u32;
                assert_eq!(oxyos_client_timeline_signal(client, created, 1), 0);
                let fence = oxyos_client_timeline_export_sync_file(client, created, 1);
                assert!(fence >= 0);
                assert_eq!(
                    oxyos_client_timeline_import_sync_file(client, created, 2, fence),
                    0
                );
                drop(rustix::fd::OwnedFd::from_raw_fd(fence));
                let ready = oxyos_client_timeline_eventfd(client, created, 2, false);
                assert!(oxyos_dmabuf::sync_file_signalled(BorrowedFd::borrow_raw(
                    ready
                )));
                drop(rustix::fd::OwnedFd::from_raw_fd(ready));
                let exported = oxyos_client_timeline_export(client, created);
                assert!(exported >= 0);
                let imported = oxyos_client_import_timeline(client, exported);
                assert!(imported > 0);
                drop(rustix::fd::OwnedFd::from_raw_fd(exported));
                let sync = OxyOSSync {
                    acquire_timeline: created,
                    acquire_point: 3,
                    release_timeline: created,
                    release_point: 4,
                };
                let sequence = oxyos_client_submit_buffer_synced(
                    client,
                    1,
                    id as u64,
                    1,
                    std::ptr::null(),
                    0,
                    &sync,
                );
                assert!(sequence > 0);
                assert_eq!(oxyos_client_destroy_timeline(client, imported as u32), 0);
                timeline = Some((id as u64, created, imported as u32));
            } else {
                eprintln!("no render node with timeline syncobjs here, the rest skipped");
            }
        } else {
            eprintln!("no udmabuf here, the DMA-BUF skipped; tools/gpu-vm runs it");
        }
        done.send(()).unwrap();
        oxyos_client_close(client);
        timeline
    };
    let requests = compositor.join().unwrap();
    if let Some((buffer, created, imported)) = timeline {
        let expected = [
            (
                Request::AttachDmabufPlane {
                    buffer,
                    plane: 0,
                    offset: 0,
                    stride: 64,
                },
                true,
            ),
            (
                Request::AttachDmabuf {
                    buffer,
                    format: oxyos_protocol::Format::Argb8888,
                    modifier: 0,
                    extent: Size::new(16, 16),
                    planes: 1,
                },
                false,
            ),
            (Request::ImportTimeline { timeline: created }, true),
            (Request::ImportTimeline { timeline: imported }, true),
            (
                Request::SetSync {
                    surface: 1,
                    acquire_timeline: created,
                    acquire_point: 3,
                    release_timeline: created,
                    release_point: 4,
                },
                false,
            ),
        ];
        assert_eq!(&requests[..expected.len()], &expected);
        assert!(matches!(requests[5].0, Request::Submit { buffer: b, .. } if b == buffer));
        assert_eq!(
            requests[6].0,
            Request::DestroyTimeline { timeline: imported }
        );
    } else {
        // Nothing refused was sent.
        assert!(requests
            .iter()
            .all(|(request, _)| *request == Request::Goodbye {}));
    }
    std::fs::remove_dir_all(directory).unwrap();
}
