use oxy_compositor_core::{ClientId, Grants, Output, Scene};
use oxy_protocol::{
    read_event, read_request, write_event, write_request, Adjust, Anchor, ClientConnection, Clock,
    Event, Format, Gravity, Outcome, Positioner, Rect, Request, ServerConnection, Size,
    MIN_VERSION, VERSION,
};
use std::collections::HashMap;
use std::error::Error;
use std::fs::{self, DirBuilder};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const DEADLINE: Duration = Duration::from_secs(5);
const FRAMES: u64 = 3;

struct Probe {
    directory: PathBuf,
    children: Vec<Child>,
}

impl Drop for Probe {
    fn drop(&mut self) {
        for child in &mut self.children {
            if !matches!(child.try_wait(), Ok(Some(_))) {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
        let _ = fs::remove_file(self.directory.join("s"));
        let _ = fs::remove_dir(&self.directory);
    }
}

/// Scene shared by the connection workers plus each client's undelivered events.
struct Compositor {
    scene: Scene,
    outboxes: HashMap<ClientId, Vec<Event>>,
    popups_beyond_parent: usize,
}

impl Compositor {
    fn route(&mut self) {
        for (client, event) in self.scene.drain_events() {
            self.outboxes.entry(client).or_default().push(event);
        }
        self.scene.drain_retired();
    }
}

fn configure_stream(stream: &UnixStream) -> Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(DEADLINE))?;
    stream.set_write_timeout(Some(DEADLINE))?;
    Ok(())
}

fn expect_configure(stream: &mut UnixStream, surface: u32) -> Result<(u64, Rect, u32)> {
    loop {
        match read_event(stream)? {
            Event::Configure {
                surface: id,
                serial,
                rect,
                scale_120,
                ..
            } if id == surface => return Ok((serial, rect, scale_120)),
            Event::ProtocolError { code } => return Err(format!("protocol error {code:?}").into()),
            _ => {}
        }
    }
}

fn present(
    stream: &mut UnixStream,
    surface: u32,
    sequence: u64,
    buffer: u64,
    configure: u64,
    rect: Rect,
) -> Result<()> {
    // Metadata only: the stream transport carries no descriptors, so the buffer is registered
    // without memory behind it. `oxy-offscreen` exercises real shared-memory buffers.
    write_request(
        stream,
        &Request::AttachBuffer {
            buffer,
            format: Format::Xrgb8888,
            extent: Size::new(rect.width, rect.height),
            stride: rect.width * 4,
            offset: 0,
        },
    )?;
    write_request(
        stream,
        &Request::Submit {
            surface,
            sequence,
            buffer,
            configure,
            extent: Size::new(rect.width, rect.height),
        },
    )?;
    loop {
        match read_event(stream)? {
            Event::Feedback {
                surface: id,
                sequence: received,
                outcome: Outcome::Presented,
                clock: Clock::Synthetic,
                ..
            } if id == surface && received == sequence => return Ok(()),
            Event::ProtocolError { code } => return Err(format!("protocol error {code:?}").into()),
            _ => {}
        }
    }
}

fn client(path: &Path, with_popup: bool) -> Result<()> {
    let mut stream = UnixStream::connect(path)?;
    configure_stream(&stream)?;
    let (mut connection, hello) = ClientConnection::new(MIN_VERSION, VERSION);
    write_request(&mut stream, &hello)?;
    connection
        .receive(&read_event(&mut stream)?)
        .map_err(|code| format!("handshake: {code:?}"))?;
    write_request(
        &mut stream,
        &Request::CreateToplevel {
            surface: 1,
            size: Size::new(320, 200),
        },
    )?;
    let (configure, rect, _) = expect_configure(&mut stream, 1)?;
    for sequence in 1..=FRAMES {
        present(&mut stream, 1, sequence, sequence, configure, rect)?;
    }
    if with_popup {
        // A menu larger than its window, anchored at the window's center.
        write_request(
            &mut stream,
            &Request::CreatePopup {
                surface: 2,
                parent: 1,
                positioner: Positioner {
                    anchor_rect: Rect::new(150, 90, 20, 20),
                    size: Size::new(480, 360),
                    anchor: Anchor::Center,
                    gravity: Gravity::Center,
                    adjust: Adjust::FLIP_X
                        .union(Adjust::FLIP_Y)
                        .union(Adjust::SLIDE_X)
                        .union(Adjust::SLIDE_Y),
                    offset_x: 0,
                    offset_y: 0,
                },
            },
        )?;
        let (configure, popup, _) = expect_configure(&mut stream, 2)?;
        present(&mut stream, 2, 1, 100, configure, popup)?;
        write_request(&mut stream, &Request::Destroy { surface: 2 })?;
        // Destroying the popup releases its buffer once nothing displays it.
        loop {
            match read_event(&mut stream)? {
                Event::Released {
                    surface: 2,
                    buffer: 100,
                } => break,
                Event::ProtocolError { code } => {
                    return Err(format!("protocol error {code:?}").into())
                }
                _ => {}
            }
        }
    }
    write_request(&mut stream, &Request::Goodbye {})?;
    Ok(())
}

fn serve(mut stream: UnixStream, client: ClientId, shared: Arc<Mutex<Compositor>>) -> Result<()> {
    configure_stream(&stream)?;
    let mut connection = ServerConnection::new();
    let result = (|| -> Result<()> {
        while !connection.is_closed() {
            let request = read_request(&mut stream)?;
            if let Some(welcome) = connection
                .receive(&request)
                .map_err(|code| format!("connection: {code:?}"))?
            {
                write_event(&mut stream, &welcome)?;
                shared
                    .lock()
                    .map_err(|_| "poisoned")?
                    .scene
                    .connect(client, Grants::default())
                    .map_err(|code| format!("connect: {code:?}"))?;
                continue;
            }
            let events = {
                let mut compositor = shared.lock().map_err(|_| "poisoned")?;
                let submitted = matches!(request, Request::Submit { .. });
                if let Request::Destroy { surface } = request {
                    let key = oxy_compositor_core::SurfaceKey {
                        client,
                        id: surface,
                    };
                    let parent = oxy_compositor_core::SurfaceKey { client, id: 1 };
                    if let (Some(popup), Some(window)) = (
                        compositor.scene.global_rect(key),
                        compositor.scene.global_rect(parent),
                    ) {
                        if popup.x < window.x
                            && popup.y < window.y
                            && popup.right() > window.right()
                            && popup.bottom() > window.bottom()
                        {
                            compositor.popups_beyond_parent += 1;
                        }
                    }
                }
                if let Err(code) = compositor.scene.handle(client, request) {
                    write_event(&mut stream, &Event::ProtocolError { code })?;
                    return Err(format!("request rejected: {code:?}").into());
                }
                if submitted {
                    // Synthetic completion: no GPU, no output, no hardware timestamp.
                    compositor.scene.begin_frame();
                    compositor.scene.end_frame(0, Clock::Synthetic);
                }
                compositor.route();
                let pending = compositor.outboxes.remove(&client).unwrap_or_default();
                if connection.is_closed() {
                    // After Goodbye nothing more is delivered; the peer may already be gone.
                    Vec::new()
                } else {
                    pending
                }
            };
            for event in events {
                write_event(&mut stream, &event)?;
            }
        }
        Ok(())
    })();
    if let Ok(mut compositor) = shared.lock() {
        compositor.scene.disconnect(client);
        compositor.route();
    }
    // A peer that hangs up is a disconnect, not a compositor failure; the probe checks each
    // client's own exit status separately.
    match result {
        Err(error)
            if error.downcast_ref::<std::io::Error>().is_some_and(|error| {
                matches!(
                    error.kind(),
                    std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::UnexpectedEof
                )
            }) =>
        {
            Ok(())
        }
        other => other,
    }
}

fn probe() -> Result<()> {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.subsec_nanos();
    let directory = std::env::temp_dir().join(format!("oxy-{}-{nonce}", std::process::id()));
    DirBuilder::new().mode(0o700).create(&directory)?;
    let mut probe = Probe {
        directory,
        children: Vec::new(),
    };
    let path = probe.directory.join("s");
    let listener = UnixListener::bind(&path)?;
    listener.set_nonblocking(true)?;
    let mut scene = Scene::new();
    scene
        .add_output(Output {
            id: 1,
            rect: Rect::new(0, 0, 1920, 1080),
            scale_120: 120,
            refresh_mhz: 60_000,
        })
        .map_err(|code| format!("output: {code:?}"))?;
    let shared = Arc::new(Mutex::new(Compositor {
        scene,
        outboxes: HashMap::new(),
        popups_beyond_parent: 0,
    }));
    for index in 0..2 {
        let mut command = Command::new(std::env::current_exe()?);
        command
            .arg(if index == 0 {
                "--client-with-popup"
            } else {
                "--client"
            })
            .arg(&path)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY");
        probe.children.push(command.spawn()?);
    }
    let deadline = Instant::now() + DEADLINE;
    let mut workers = Vec::new();
    while workers.len() < 2 {
        match listener.accept() {
            Ok((stream, _)) => {
                let shared = Arc::clone(&shared);
                let client = workers.len() as ClientId + 1;
                workers.push(thread::spawn(move || serve(stream, client, shared)));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err("client connection deadline exceeded".into());
                }
                for child in &mut probe.children {
                    if let Some(status) = child.try_wait()? {
                        if !status.success() {
                            return Err("client failed before connecting".into());
                        }
                    }
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(error.into()),
        }
    }
    for worker in workers {
        worker.join().map_err(|_| "probe worker panicked")??;
    }
    for child in &mut probe.children {
        loop {
            if let Some(status) = child.try_wait()? {
                if !status.success() {
                    return Err("client failed".into());
                }
                break;
            }
            if Instant::now() >= deadline {
                return Err("client exit deadline exceeded".into());
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
    let compositor = shared.lock().map_err(|_| "poisoned")?;
    let stats = compositor.scene.stats();
    if stats.surfaces != 0 || stats.clients != 0 || stats.held_buffers != 0 {
        return Err(format!("resources left after disconnect: {stats:?}").into());
    }
    if compositor.popups_beyond_parent != 1 {
        return Err("popup was not placed beyond its parent".into());
    }
    println!(
        "{{\"mode\":\"headless-metadata\",\"protocol\":{VERSION},\"clients\":2,\"frames\":{},\"popups_beyond_parent\":1,\"graphics\":false,\"bloom\":false,\"status\":\"passed\"}}",
        2 * FRAMES + 1
    );
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [] => probe(),
        [flag, path] if flag == "--client" => client(Path::new(path), false),
        [flag, path] if flag == "--client-with-popup" => client(Path::new(path), true),
        _ => Err("usage: oxy-platform-probe [--client SOCKET | --client-with-popup SOCKET]".into()),
    }
}
