//! Why a buffer request is refused: the reason the compositor's log names beside the code the
//! client is told, for each condition `InvalidBuffer` covers. The code is the one `handle` gives.

use super::*;

/// The request's refusal, explained; `handle` would have given its code.
fn explain(h: &mut Harness, client: ClientId, request: Request) -> Result<(), Refusal> {
    let refusal = h.scene.handle_explained(client, request);
    h.pump();
    refusal
}

fn refused(reason: BufferRefusal) -> Result<(), Refusal> {
    Err(Refusal {
        code: ErrorCode::InvalidBuffer,
        reason: Some(reason),
    })
}

const SIZE: Size = Size::new(400, 300);

fn attach(buffer: u64, extent: Size, stride: u32) -> Request {
    Request::AttachBuffer {
        buffer,
        format: Format::Argb8888,
        extent,
        stride,
        offset: 0,
    }
}

/// A window of SIZE, shown, and its configure serial.
fn window(h: &mut Harness) -> (SurfaceKey, u64) {
    let window = h.toplevel(APP, 1, SIZE.width, SIZE.height);
    let serial = h.configures[&window].0;
    (window, serial)
}

fn submit(surface: u32, serial: u64, sequence: u64, buffer: u64, extent: Size) -> Request {
    Request::Submit {
        surface,
        sequence,
        buffer,
        configure: serial,
        extent,
    }
}

#[test]
fn an_attached_buffer_refused_says_why() {
    let mut h = Harness::new();
    let (window, serial) = window(&mut h);
    assert_eq!(
        explain(&mut h, APP, attach(0, SIZE, 1600)),
        refused(BufferRefusal::ZeroId)
    );
    assert_eq!(
        explain(&mut h, APP, attach(60, SIZE, 1599)),
        refused(BufferRefusal::BadLayout { buffer: 60 })
    );
    h.attach(APP, 61, SIZE);
    assert_eq!(
        explain(&mut h, APP, attach(61, SIZE, 1600)),
        refused(BufferRefusal::AlreadyAttached {
            buffer: 61,
            destroyed: false
        })
    );
    // Destroyed while shown: its id is still taken until it is released.
    h.request(APP, submit(1, serial, 50, 61, SIZE)).unwrap();
    h.frame();
    h.request(APP, Request::DestroyBuffer { buffer: 61 })
        .unwrap();
    assert_eq!(
        explain(&mut h, APP, attach(61, SIZE, 1600)),
        refused(BufferRefusal::AlreadyAttached {
            buffer: 61,
            destroyed: true
        })
    );
    // A GPU buffer is checked as any.
    let dmabuf = |buffer| Request::AttachDmabuf {
        buffer,
        format: Format::Argb8888,
        modifier: 0,
        extent: SIZE,
        planes: 1,
    };
    assert_eq!(
        explain(&mut h, APP, dmabuf(0)),
        refused(BufferRefusal::ZeroId)
    );
    assert_eq!(
        explain(&mut h, APP, dmabuf(61)),
        refused(BufferRefusal::AlreadyAttached {
            buffer: 61,
            destroyed: true
        })
    );
    assert!(h.rendered(window));
}

#[test]
fn a_frame_refused_says_why() {
    let mut h = Harness::new();
    let (_, serial) = window(&mut h);
    assert_eq!(
        explain(&mut h, APP, submit(1, serial, 50, 777, SIZE)),
        refused(BufferRefusal::Unknown { buffer: 777 })
    );
    // Another client's buffer is not the client's.
    h.attach(OTHER, 70, SIZE);
    assert_eq!(
        explain(&mut h, APP, submit(1, serial, 50, 70, SIZE)),
        refused(BufferRefusal::Unknown { buffer: 70 })
    );
    h.attach(APP, 71, Size::new(800, 600));
    assert_eq!(
        explain(&mut h, APP, submit(1, serial, 50, 71, SIZE)),
        refused(BufferRefusal::ExtentMismatch {
            buffer: 71,
            attached: Size::new(800, 600),
            frame: SIZE,
        })
    );
    // The buffer's extent, but not the surface's.
    assert_eq!(
        explain(&mut h, APP, submit(1, serial, 50, 71, Size::new(800, 600))),
        refused(BufferRefusal::SurfaceExtent {
            frame: Size::new(800, 600),
            surface: SIZE,
        })
    );
    h.attach(APP, 72, SIZE);
    assert_eq!(
        explain(&mut h, APP, submit(1, serial, 0, 72, SIZE)),
        refused(BufferRefusal::InvalidFrame)
    );
    // Shown, then submitted again before its release.
    h.request(APP, submit(1, serial, 50, 72, SIZE)).unwrap();
    h.frame();
    assert_eq!(
        explain(&mut h, APP, submit(1, serial, 51, 72, SIZE)),
        refused(BufferRefusal::StillHeld { buffer: 72 })
    );
    // Destroyed while shown.
    h.request(APP, Request::DestroyBuffer { buffer: 72 })
        .unwrap();
    assert_eq!(
        explain(&mut h, APP, submit(1, serial, 51, 72, SIZE)),
        refused(BufferRefusal::Destroyed { buffer: 72 })
    );
}

#[test]
fn a_buffer_whose_release_waits_for_the_frame_in_flight_is_refused_as_such() {
    let mut h = Harness::new();
    let (_, serial) = window(&mut h);
    for buffer in [80, 81] {
        h.attach(APP, buffer, SIZE);
    }
    h.scene.begin_frame();
    h.request(APP, submit(1, serial, 50, 80, SIZE)).unwrap();
    // 80, queued, is superseded while a frame is in flight: its release waits for that frame.
    h.request(APP, submit(1, serial, 51, 81, SIZE)).unwrap();
    assert_eq!(
        explain(&mut h, APP, submit(1, serial, 52, 80, SIZE)),
        refused(BufferRefusal::ReleaseDeferred { buffer: 80 })
    );
}

#[test]
fn a_buffer_held_by_another_surface_is_refused_as_such() {
    let mut h = Harness::new();
    let (window, serial) = window(&mut h);
    h.attach(APP, 90, SIZE);
    h.request(APP, submit(1, serial, 50, 90, SIZE)).unwrap();
    h.frame();
    let popup = h.popup(window, 2, menu(Rect::new(0, 0, 10, 10), 400, 300));
    let popup_serial = h.configures[&popup].0;
    assert_eq!(
        explain(&mut h, APP, submit(2, popup_serial, 1, 90, SIZE)),
        refused(BufferRefusal::AttachedElsewhere { buffer: 90 })
    );
}

#[test]
fn a_destroy_and_a_pointer_image_refused_say_why() {
    let mut h = Harness::new();
    let (_, serial) = window(&mut h);
    assert_eq!(
        explain(&mut h, APP, Request::DestroyBuffer { buffer: 5 }),
        refused(BufferRefusal::Unknown { buffer: 5 })
    );
    h.attach(APP, 100, SIZE);
    h.request(APP, submit(1, serial, 50, 100, SIZE)).unwrap();
    h.frame();
    h.request(APP, Request::DestroyBuffer { buffer: 100 })
        .unwrap();
    assert_eq!(
        explain(&mut h, APP, Request::DestroyBuffer { buffer: 100 }),
        refused(BufferRefusal::Destroyed { buffer: 100 })
    );
    let image = |buffer| Request::SetPointerImage {
        surface: 1,
        buffer,
        hotspot_x: 0,
        hotspot_y: 0,
        scale_120: 120,
    };
    assert_eq!(
        explain(&mut h, APP, image(5)),
        refused(BufferRefusal::Unknown { buffer: 5 })
    );
    assert_eq!(
        explain(&mut h, APP, image(100)),
        refused(BufferRefusal::Destroyed { buffer: 100 })
    );
}

#[test]
fn a_refusal_not_of_a_buffer_has_only_its_code() {
    let mut h = Harness::new();
    let (_, serial) = window(&mut h);
    h.attach(APP, 110, SIZE);
    assert_eq!(
        explain(&mut h, APP, submit(9, serial, 50, 110, SIZE)),
        Err(ErrorCode::UnknownSurface.into())
    );
    h.request(APP, submit(1, serial, 50, 110, SIZE)).unwrap();
    h.attach(APP, 111, SIZE);
    assert_eq!(
        explain(&mut h, APP, submit(1, serial, 50, 111, SIZE)),
        Err(ErrorCode::InvalidOrder.into())
    );
    assert_eq!(
        explain(&mut h, APP, Request::Destroy { surface: 9 }),
        Err(ErrorCode::UnknownSurface.into())
    );
    assert_eq!(
        h.scene.handle_explained(99, Request::Goodbye {}),
        Err(ErrorCode::InvalidOrder.into()),
        "a client not connected"
    );
}
