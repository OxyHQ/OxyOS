use super::*;
use oxy_protocol::{Format, Size, MIN_VERSION, VERSION};
use std::os::unix::fs::MetadataExt;

fn layout(width: u32, height: u32) -> BufferLayout {
    BufferLayout {
        format: Format::Xrgb8888,
        extent: Size::new(width, height),
        stride: width * 4,
        offset: 0,
    }
}

fn attach(buffer: u64, layout: BufferLayout) -> Request {
    Request::AttachBuffer {
        buffer,
        format: layout.format,
        extent: layout.extent,
        stride: layout.stride,
        offset: layout.offset,
    }
}

#[test]
fn messages_and_descriptors_travel_together() {
    let (client, server) = Connection::pair().unwrap();
    let hello = Request::Hello {
        min_version: MIN_VERSION,
        max_version: VERSION,
    };
    client.send_request(&hello, None).unwrap();
    let memory = create_shared_memory("test", 64).unwrap();
    client
        .send_request(&attach(1, layout(4, 4)), Some(memory.as_fd()))
        .unwrap();
    assert_eq!(server.recv_request().unwrap().0, hello);
    let (request, fd) = server.recv_request().unwrap();
    assert_eq!(request, attach(1, layout(4, 4)));
    let fd = fd.expect("descriptor arrives with its message");
    SharedBuffer::import(fd, layout(4, 4)).unwrap();
    server
        .send_event(&Event::Welcome { version: VERSION })
        .unwrap();
    assert_eq!(
        client.recv_event().unwrap(),
        Event::Welcome { version: VERSION }
    );
}

#[test]
fn descriptors_must_match_the_message() {
    let (client, server) = Connection::pair().unwrap();
    let memory = create_shared_memory("test", 64).unwrap();
    // The sender refuses mismatches outright.
    assert!(client.send_request(&attach(1, layout(4, 4)), None).is_err());
    assert!(client
        .send_request(&Request::Goodbye {}, Some(memory.as_fd()))
        .is_err());
    // A malformed peer bypassing the sender: descriptor on a message that takes none.
    let mut packet = Vec::new();
    write_request(&mut packet, &Request::Goodbye {}).unwrap();
    client.send_raw(&packet, &[memory.as_fd()]).unwrap();
    assert!(server.recv_request().is_err());
    // AttachBuffer without its descriptor, and with two.
    let mut packet = Vec::new();
    write_request(&mut packet, &attach(1, layout(4, 4))).unwrap();
    client.send_raw(&packet, &[]).unwrap();
    assert!(server.recv_request().is_err());
    client
        .send_raw(&packet, &[memory.as_fd(), memory.as_fd()])
        .unwrap();
    assert!(server.recv_request().is_err());
    // Events never carry descriptors.
    let mut packet = Vec::new();
    write_event(&mut packet, &Event::Welcome { version: VERSION }).unwrap();
    server.send_raw(&packet, &[memory.as_fd()]).unwrap();
    assert!(client.recv_event().is_err());
}

#[test]
fn rejects_oversized_trailing_and_garbage_packets() {
    let (client, server) = Connection::pair().unwrap();
    client.send_raw(&[0xff; PACKET_CAPACITY + 16], &[]).unwrap();
    assert_eq!(
        server.recv_request().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    let mut packet = Vec::new();
    write_request(&mut packet, &Request::Goodbye {}).unwrap();
    packet.push(0);
    client.send_raw(&packet, &[]).unwrap();
    assert!(server.recv_request().is_err(), "two messages in one packet");
    client.send_raw(&[1, 0, 0, 0, 99], &[]).unwrap();
    assert!(server.recv_request().is_err());
    // The connection still works after rejected packets; the caller decides to close it.
    client.send_request(&Request::Goodbye {}, None).unwrap();
    assert_eq!(server.recv_request().unwrap().0, Request::Goodbye {});
    drop(client);
    assert_eq!(
        server.recv_request().unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
}

#[test]
fn peer_credentials_come_from_the_kernel() {
    let directory = std::env::temp_dir().join(format!("oxy-ipc-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("s");
    let _ = std::fs::remove_file(&path);
    let listener = Listener::bind(&path).unwrap();
    let client = Connection::connect(&path).unwrap();
    let server = listener.accept().unwrap();
    let credentials = server.peer_credentials().unwrap();
    assert_eq!(credentials.pid as u32, std::process::id());
    assert_eq!(
        credentials.uid,
        std::fs::metadata(&directory).unwrap().uid()
    );
    drop(client);
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(&directory).unwrap();
}

#[test]
fn shared_memory_import_validates_type_seals_and_size() {
    // Unsealed memory could be truncated under the compositor.
    let unsealed =
        rustix::fs::memfd_create("unsealed", MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING)
            .unwrap();
    rustix::fs::ftruncate(&unsealed, 64).unwrap();
    assert_eq!(
        SharedBuffer::import(unsealed, layout(4, 4)).unwrap_err(),
        ImportError::NotSealed
    );
    // Not a regular file.
    let (pipe, _writer) = rustix::pipe::pipe().unwrap();
    assert_eq!(
        SharedBuffer::import(pipe, layout(4, 4)).unwrap_err(),
        ImportError::NotRegularFile
    );
    // Smaller than the layout needs.
    let small = create_shared_memory("small", 63).unwrap();
    assert_eq!(
        SharedBuffer::import(small, layout(4, 4)).unwrap_err(),
        ImportError::TooSmall
    );
    let bad = BufferLayout {
        stride: 3,
        ..layout(4, 4)
    };
    let memory = create_shared_memory("bad", 64).unwrap();
    assert_eq!(
        SharedBuffer::import(memory, bad).unwrap_err(),
        ImportError::InvalidLayout
    );
    // Sealed memory cannot be shrunk by the client afterwards.
    let memory = create_shared_memory("ok", 64).unwrap();
    assert!(rustix::fs::ftruncate(&memory, 0).is_err());
}

#[test]
fn rows_are_read_through_stride_and_offset() {
    let layout = BufferLayout {
        format: Format::Argb8888,
        extent: Size::new(2, 3),
        stride: 16,
        offset: 8,
    };
    let memory = create_shared_memory("rows", layout.required_bytes().unwrap()).unwrap();
    for row in 0..3_u8 {
        let pixels: Vec<u8> = (0..8).map(|i| row * 10 + i).collect();
        write_at(memory.as_fd(), 8 + 16 * u64::from(row), &pixels).unwrap();
    }
    let buffer = SharedBuffer::import(memory, layout).unwrap();
    let mut out = [0; 8];
    buffer.read_row(2, &mut out).unwrap();
    assert_eq!(out, [20, 21, 22, 23, 24, 25, 26, 27]);
    assert!(buffer.read_row(3, &mut out).is_err());
    assert!(buffer.read_row(0, &mut [0; 4]).is_err());
}

#[test]
fn a_client_that_stops_reading_never_blocks_the_sender() {
    let (client, server) = Connection::pair().unwrap();
    let event = Event::Focus {
        surface: 1,
        focused: true,
    };
    let mut sent = 0;
    while server.try_send_event(&event).unwrap() {
        sent += 1;
        assert!(sent < 1_000_000, "the queue must fill");
    }
    assert!(sent > 0);
    // Once the client drains one packet there is room again.
    assert_eq!(client.recv_event().unwrap(), event);
    assert!(server.try_send_event(&event).unwrap());
}
