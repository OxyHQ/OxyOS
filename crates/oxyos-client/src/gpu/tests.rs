use crate::tests::connected;
use crate::{Client, GpuChange, GpuTable, Poll, SyncPoints};
use oxyos_ipc::Connection;
use oxyos_protocol::{Event, Format, Rect, Request, Size};
use rustix::fd::AsFd;

/// Sends `events` to the client and lets it read each.
fn tell(client: &mut Client, server: &Connection, events: &[Event]) {
    for event in events {
        server.send_event(event, None).unwrap();
        assert!(matches!(client.poll().unwrap(), Poll::Event(_)));
    }
}

const TILED: u64 = 0x0100_0000_0000_0002;

fn table(client: &mut Client, server: &Connection) {
    tell(
        client,
        server,
        &[
            Event::GpuDevice {
                device: 0xe280,
                formats: 2,
            },
            Event::GpuFormat {
                index: 0,
                format: Format::Argb8888,
                modifier: 0,
            },
            Event::GpuFormat {
                index: 1,
                format: Format::Argb8888,
                modifier: TILED,
            },
        ],
    );
}

/// The next request the compositor receives is `marker`: nothing was sent before it.
fn nothing_sent_before(client: &Client, server: &Connection) {
    client.send(&Request::Destroy { surface: 99 }).unwrap();
    assert_eq!(
        server.recv_request().unwrap().0,
        Request::Destroy { surface: 99 }
    );
}

fn memfd(bytes: u64) -> rustix::fd::OwnedFd {
    let fd = rustix::fs::memfd_create("test", rustix::fs::MemfdFlags::CLOEXEC).unwrap();
    rustix::fs::ftruncate(&fd, bytes).unwrap();
    fd
}

#[test]
fn the_table_is_known_once_every_format_arrived_and_withdrawn_with_none() {
    let (mut client, server) = connected();
    assert_eq!(client.gpu(), None);
    tell(
        &mut client,
        &server,
        &[
            Event::GpuDevice {
                device: 0xe280,
                formats: 2,
            },
            // Out of order: the table is the indices'.
            Event::GpuFormat {
                index: 1,
                format: Format::Xrgb8888,
                modifier: 0,
            },
        ],
    );
    assert_eq!(client.gpu(), None, "one of two formats");
    assert_eq!(client.take_gpu_change(), None);
    tell(
        &mut client,
        &server,
        &[Event::GpuFormat {
            index: 0,
            format: Format::Argb8888,
            modifier: TILED,
        }],
    );
    assert_eq!(
        client.gpu(),
        Some(&GpuTable {
            device: 0xe280,
            formats: vec![(Format::Argb8888, TILED), (Format::Xrgb8888, 0)],
        })
    );
    assert_eq!(client.take_gpu_change(), Some(GpuChange::Device(2)));
    assert_eq!(client.take_gpu_change(), None, "told once");
    // A new device starts a new table; until it is whole, the old one is not the compositor's.
    tell(
        &mut client,
        &server,
        &[
            Event::GpuDevice {
                device: 0xe281,
                formats: 1,
            },
            Event::GpuFormat {
                index: 0,
                format: Format::Xrgb8888,
                modifier: 0,
            },
        ],
    );
    assert_eq!(client.gpu().unwrap().device, 0xe281);
    assert_eq!(client.gpu().unwrap().formats, vec![(Format::Xrgb8888, 0)]);
    client.take_gpu_change();
    tell(
        &mut client,
        &server,
        &[Event::GpuDevice {
            device: 0,
            formats: 0,
        }],
    );
    assert_eq!(client.gpu(), None, "withdrawn");
    assert_eq!(client.take_gpu_change(), Some(GpuChange::Device(0)));
    // An entry beyond the count told is ignored.
    tell(
        &mut client,
        &server,
        &[
            Event::GpuDevice {
                device: 1,
                formats: 1,
            },
            Event::GpuFormat {
                index: 3,
                format: Format::Xrgb8888,
                modifier: 0,
            },
        ],
    );
    assert_eq!(client.gpu(), None);
}

#[test]
fn a_surfaces_scanout_formats_are_its_own_until_withdrawn() {
    let (mut client, server) = connected();
    tell(
        &mut client,
        &server,
        &[
            Event::GpuScanout {
                surface: 1,
                formats: 1,
            },
            Event::GpuScanoutFormat {
                surface: 1,
                index: 0,
                format: Format::Xrgb8888,
                modifier: TILED,
            },
        ],
    );
    assert_eq!(client.scanout_formats(1), &[(Format::Xrgb8888, TILED)]);
    assert_eq!(client.scanout_formats(2), &[]);
    assert_eq!(client.take_gpu_change(), Some(GpuChange::Scanout(1, 1)));
    tell(
        &mut client,
        &server,
        &[Event::GpuScanout {
            surface: 1,
            formats: 0,
        }],
    );
    assert_eq!(client.scanout_formats(1), &[]);
    assert_eq!(client.take_gpu_change(), Some(GpuChange::Scanout(1, 0)));
}

#[test]
fn a_dmabuf_is_refused_before_anything_is_sent() {
    let (mut client, server) = connected();
    let fd = memfd(4096);
    let extent = Size::new(16, 16);
    // No table: no GPU buffers.
    assert!(client
        .attach_dmabuf(Format::Argb8888, 0, extent, &[(fd.as_fd(), 0, 64)])
        .is_err());
    table(&mut client, &server);
    // Not in the table.
    assert!(client
        .attach_dmabuf(Format::Xrgb8888, 0, extent, &[(fd.as_fd(), 0, 64)])
        .is_err());
    // No planes, five planes.
    assert!(client
        .attach_dmabuf(Format::Argb8888, 0, extent, &[])
        .is_err());
    let five = [(fd.as_fd(), 0, 64); 5];
    assert!(client
        .attach_dmabuf(Format::Argb8888, TILED, extent, &five)
        .is_err());
    // A memfd is not a DMA-BUF.
    assert!(client
        .attach_dmabuf(Format::Argb8888, 0, extent, &[(fd.as_fd(), 0, 64)])
        .is_err());
    nothing_sent_before(&client, &server);
}

#[test]
fn a_dmabufs_planes_go_with_their_descriptors_then_the_buffer() {
    let (mut client, server) = connected();
    table(&mut client, &server);
    let Ok((_memfd, buffer)) = oxyos_dmabuf::shared_dmabuf(64 * 16) else {
        eprintln!("no udmabuf here, skipped; tools/gpu-vm runs it");
        return;
    };
    let extent = Size::new(16, 16);
    // The buffer is a page: 16 rows of 64 bytes from 64 before its last kilobyte end past it.
    let past = buffer.size() as u32 - 1024 + 64;
    // A linear plane past its buffer, or more than one: refused.
    assert!(client
        .attach_dmabuf(Format::Argb8888, 0, extent, &[(buffer.as_fd(), past, 64)])
        .is_err());
    assert!(client
        .attach_dmabuf(
            Format::Argb8888,
            0,
            extent,
            &[(buffer.as_fd(), 0, 64), (buffer.as_fd(), 0, 64)]
        )
        .is_err());
    nothing_sent_before(&client, &server);
    let id = client
        .attach_dmabuf(Format::Argb8888, 0, extent, &[(buffer.as_fd(), 0, 64)])
        .unwrap();
    let (plane, fd) = server.recv_request().unwrap();
    assert_eq!(
        plane,
        Request::AttachDmabufPlane {
            buffer: id,
            plane: 0,
            offset: 0,
            stride: 64,
        }
    );
    let received = oxyos_dmabuf::DmaBuf::from_fd(fd.expect("the plane's descriptor")).unwrap();
    assert_eq!(received.size(), buffer.size());
    let (attach, fd) = server.recv_request().unwrap();
    assert!(fd.is_none(), "the buffer carries no descriptor");
    assert_eq!(
        attach,
        Request::AttachDmabuf {
            buffer: id,
            format: Format::Argb8888,
            modifier: 0,
            extent,
            planes: 1,
        }
    );
    // A tiled buffer of two planes: each with its own descriptor, in order.
    let two = client
        .attach_dmabuf(
            Format::Argb8888,
            TILED,
            extent,
            &[(buffer.as_fd(), 0, 64), (buffer.as_fd(), 512, 16)],
        )
        .unwrap();
    for (index, offset, stride) in [(0, 0, 64), (1, 512, 16)] {
        let (plane, fd) = server.recv_request().unwrap();
        assert!(fd.is_some());
        assert_eq!(
            plane,
            Request::AttachDmabufPlane {
                buffer: two,
                plane: index,
                offset,
                stride,
            }
        );
    }
    assert!(matches!(
        server.recv_request().unwrap().0,
        Request::AttachDmabuf { planes: 2, .. }
    ));
    // Submitted and detached as any buffer of the caller's; a failed import is told.
    let sequence = client.submit_buffer(1, id, 3, None).unwrap();
    assert_eq!(
        server.recv_request().unwrap().0,
        Request::Submit {
            surface: 1,
            sequence,
            buffer: id,
            configure: 3,
            extent,
        }
    );
    tell(&mut client, &server, &[Event::BufferFailed { buffer: two }]);
    assert!(client.buffer_failed(two));
    assert!(!client.buffer_failed(id));
    client.detach_buffer(two).unwrap();
    assert_eq!(
        server.recv_request().unwrap().0,
        Request::DestroyBuffer { buffer: two }
    );
}

/// A shared buffer of the caller's, for submits that need any buffer.
fn shared(client: &mut Client, server: &Connection, extent: Size) -> u64 {
    let layout = oxyos_protocol::BufferLayout {
        format: Format::Argb8888,
        extent,
        stride: extent.width * 4,
        offset: 0,
    };
    let fd = oxyos_ipc::create_shared_memory("test", layout.required_bytes().unwrap()).unwrap();
    let id = client.attach_buffer(fd.as_fd(), layout).unwrap();
    server.recv_request().unwrap();
    id
}

#[test]
fn a_synced_submit_names_only_timelines_it_has_and_points_in_order() {
    let (mut client, server) = connected();
    let extent = Size::new(8, 8);
    let buffer = shared(&mut client, &server, extent);
    let sync = |acquire, release| SyncPoints { acquire, release };
    // Timelines this connection never made.
    assert!(client
        .submit_buffer_synced(1, buffer, 1, None, Some(sync((4, 1), (0, 0))))
        .is_err());
    assert!(client
        .submit_buffer_synced(1, buffer, 1, None, Some(sync((0, 0), (4, 2))))
        .is_err());
    assert!(client.destroy_timeline(4).is_err());
    assert!(client.timeline_signal(4, 1).is_err());
    nothing_sent_before(&client, &server);
    // No synchronization: no SetSync.
    let sequence = client
        .submit_buffer_synced(1, buffer, 1, None, Some(SyncPoints::default()))
        .unwrap();
    assert!(matches!(
        server.recv_request().unwrap().0,
        Request::SetSync {
            acquire_timeline: 0,
            release_timeline: 0,
            ..
        }
    ));
    assert!(matches!(
        server.recv_request().unwrap().0,
        Request::Submit { sequence: s, .. } if s == sequence
    ));
    client
        .submit_buffer_synced(1, buffer, 1, None, None)
        .unwrap();
    assert!(matches!(
        server.recv_request().unwrap().0,
        Request::Submit { .. }
    ));

    let Ok(timeline) = client.create_timeline() else {
        eprintln!("no render node with timeline syncobjs here, skipped; tools/gpu-vm runs it");
        return;
    };
    let (import, fd) = server.recv_request().unwrap();
    assert_eq!(import, Request::ImportTimeline { timeline });
    // The descriptor sent is the timeline: what the client signals, the compositor sees.
    let node = std::sync::Arc::new(oxyos_dmabuf::RenderNode::with_timelines(None).unwrap());
    let theirs = oxyos_dmabuf::Timeline::import(&node, fd.unwrap().as_fd()).unwrap();
    client.timeline_signal(timeline, 3).unwrap();
    assert_eq!(theirs.signalled().unwrap(), 3);
    assert_eq!(client.timeline_signalled(timeline).unwrap(), 3);
    // Release not after acquire on one timeline, or point 0: refused.
    for bad in [
        sync((timeline, 5), (timeline, 5)),
        sync((timeline, 5), (timeline, 4)),
        sync((timeline, 0), (0, 0)),
    ] {
        assert!(client
            .submit_buffer_synced(1, buffer, 1, None, Some(bad))
            .is_err());
    }
    nothing_sent_before(&client, &server);
    let damage = [Rect::new(1, 1, 2, 2)];
    let sequence = client
        .submit_buffer_synced(
            1,
            buffer,
            1,
            Some(&damage),
            Some(sync((timeline, 4), (timeline, 5))),
        )
        .unwrap();
    assert_eq!(
        server.recv_request().unwrap().0,
        Request::Damage {
            surface: 1,
            rect: damage[0]
        }
    );
    assert_eq!(
        server.recv_request().unwrap().0,
        Request::SetSync {
            surface: 1,
            acquire_timeline: timeline,
            acquire_point: 4,
            release_timeline: timeline,
            release_point: 5,
        }
    );
    assert_eq!(
        server.recv_request().unwrap().0,
        Request::Submit {
            surface: 1,
            sequence,
            buffer,
            configure: 1,
            extent,
        }
    );
    // The sync-file bridge: a point's fence out, into another point.
    let fence = client.timeline_export_sync_file(timeline, 3).unwrap();
    client
        .timeline_import_sync_file(timeline, 4, fence.as_fd())
        .unwrap();
    assert!(oxyos_dmabuf::sync_file_signalled(
        client.timeline_eventfd(timeline, 4, false).unwrap().as_fd()
    ));
    // Its own syncobj, imported: one more timeline of the connection.
    let own = oxyos_dmabuf::Timeline::create(&node).unwrap();
    let imported = client
        .import_timeline(own.export().unwrap().as_fd())
        .unwrap();
    assert_ne!(imported, timeline);
    assert!(matches!(
        server.recv_request().unwrap(),
        (Request::ImportTimeline { timeline: t }, Some(_)) if t == imported
    ));
    own.signal(9).unwrap();
    assert_eq!(client.timeline_signalled(imported).unwrap(), 9);
    client.destroy_timeline(imported).unwrap();
    assert_eq!(
        server.recv_request().unwrap().0,
        Request::DestroyTimeline { timeline: imported }
    );
}
