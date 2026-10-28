use super::*;
use oxy_ipc::SharedBuffer;
use oxy_protocol::ServerConnection;

/// A connected client and the compositor's end, past the handshake.
fn connected() -> (Client, Connection) {
    let (client, server) = Connection::pair().unwrap();
    let compositor = std::thread::spawn(move || {
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        let welcome = protocol.receive(&hello).unwrap().expect("welcome");
        server.send_event(&welcome).unwrap();
        server
    });
    let client = Client::handshake(client).unwrap();
    (client, compositor.join().unwrap())
}

fn pixels(extent: Size, value: u8) -> Vec<u8> {
    vec![value; (extent.width * extent.height * 4) as usize]
}

/// Receives an AttachBuffer and its descriptor, then the Submit naming it.
fn submitted(server: &Connection) -> (u64, SharedBuffer, Request) {
    let (attach, fd) = server.recv_request().unwrap();
    let (Request::AttachBuffer { buffer, .. }, Some(fd)) = (attach, fd) else {
        panic!("expected AttachBuffer, got {attach:?}");
    };
    let shared = SharedBuffer::import(fd, attach.buffer_layout().unwrap()).unwrap();
    (buffer, shared, server.recv_request().unwrap().0)
}

#[test]
fn a_frame_carries_the_pixels_in_a_sealed_buffer() {
    let (mut client, server) = connected();
    let extent = Size::new(4, 2);
    let sequence = client.present(1, 7, extent, &pixels(extent, 0x5a)).unwrap();
    let (buffer, shared, submit) = submitted(&server);
    assert_eq!(
        submit,
        Request::Submit {
            surface: 1,
            sequence,
            buffer,
            configure: 7,
            extent
        }
    );
    let mut row = vec![0; 16];
    shared.read_row(1, &mut row).unwrap();
    assert_eq!(row, vec![0x5a; 16]);
}

#[test]
fn released_buffers_are_reused_and_busy_ones_are_not() {
    let (mut client, server) = connected();
    let extent = Size::new(2, 2);
    client.present(1, 1, extent, &pixels(extent, 1)).unwrap();
    let (first, _, _) = submitted(&server);
    // The first buffer is still held: a second frame needs a second buffer.
    client.present(1, 1, extent, &pixels(extent, 2)).unwrap();
    let (second, _, _) = submitted(&server);
    assert_ne!(first, second);
    server
        .send_event(&Event::Released {
            surface: 1,
            buffer: first,
        })
        .unwrap();
    assert!(matches!(
        client.poll().unwrap(),
        Poll::Event(Event::Released { .. })
    ));
    client.present(1, 1, extent, &pixels(extent, 3)).unwrap();
    // No new AttachBuffer: the released buffer is submitted again.
    match server.recv_request().unwrap().0 {
        Request::Submit { buffer, .. } => assert_eq!(buffer, first),
        other => panic!("expected Submit, got {other:?}"),
    }
    assert_eq!(client.buffer_count(), 2);
}

#[test]
fn a_resize_destroys_idle_buffers_of_the_old_size() {
    let (mut client, server) = connected();
    let small = Size::new(2, 2);
    client.present(1, 1, small, &pixels(small, 1)).unwrap();
    let (old, _, _) = submitted(&server);
    server
        .send_event(&Event::Released {
            surface: 1,
            buffer: old,
        })
        .unwrap();
    client.poll().unwrap();
    let large = Size::new(3, 3);
    client.present(1, 2, large, &pixels(large, 1)).unwrap();
    assert_eq!(
        server.recv_request().unwrap().0,
        Request::DestroyBuffer { buffer: old }
    );
    submitted(&server);
    assert_eq!(client.buffer_count(), 1);
}

#[test]
fn pixels_must_match_the_extent() {
    let (mut client, _server) = connected();
    let error = client
        .present(1, 1, Size::new(4, 4), &pixels(Size::new(2, 2), 0))
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(client.buffer_count(), 0);
}

#[test]
fn destroying_a_surface_destroys_its_buffers() {
    let (mut client, server) = connected();
    let extent = Size::new(2, 2);
    client.present(2, 1, extent, &pixels(extent, 1)).unwrap();
    let (buffer, _, _) = submitted(&server);
    client.destroy_surface(2).unwrap();
    assert_eq!(
        server.recv_request().unwrap().0,
        Request::Destroy { surface: 2 }
    );
    assert_eq!(
        server.recv_request().unwrap().0,
        Request::DestroyBuffer { buffer }
    );
    assert_eq!(client.buffer_count(), 0);
}

#[test]
fn polling_reports_empty_and_closed_and_protocol_errors_end_the_connection() {
    let (mut client, server) = connected();
    assert_eq!(client.poll().unwrap(), Poll::Empty);
    server
        .send_event(&Event::ProtocolError {
            code: oxy_protocol::ErrorCode::InvalidBuffer,
        })
        .unwrap();
    assert!(client.poll().is_err());
    drop(server);
    assert_eq!(client.poll().unwrap(), Poll::Closed);
}

#[test]
fn a_compositor_that_closes_with_requests_unread_is_a_closed_connection() {
    let (mut client, server) = connected();
    client.send(&Request::Destroy { surface: 9 }).unwrap();
    drop(server);
    assert_eq!(client.poll().unwrap(), Poll::Closed);
    assert_eq!(client.next_event().unwrap(), None);
}
