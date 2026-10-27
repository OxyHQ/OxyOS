use oxy_compositor_core::Surface;
use oxy_protocol::{negotiate, read_message, write_message, Extent, Frame, Message, VERSION};
use std::error::Error;
use std::fs::{self, DirBuilder};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const DEADLINE: Duration = Duration::from_secs(5);

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

fn configure_stream(stream: &UnixStream) -> Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(DEADLINE))?;
    stream.set_write_timeout(Some(DEADLINE))?;
    Ok(())
}

fn client(path: &Path) -> Result<()> {
    let mut stream = UnixStream::connect(path)?;
    configure_stream(&stream)?;
    write_message(&mut stream, Message::Hello { version: VERSION })?;
    match read_message(&mut stream)? {
        Message::Welcome { version } => negotiate(version)?,
        _ => return Err("expected welcome".into()),
    }
    let (configure, extent) = match read_message(&mut stream)? {
        Message::Configure { serial, extent } => (serial, extent),
        _ => return Err("expected configure".into()),
    };
    for sequence in 1..=3 {
        write_message(
            &mut stream,
            Message::Submit(Frame {
                sequence,
                buffer: sequence,
                configure,
                extent,
            }),
        )?;
        match read_message(&mut stream)? {
            Message::Presented {
                sequence: received, ..
            } if received == sequence => {}
            _ => return Err("expected matching presentation feedback".into()),
        }
    }
    write_message(&mut stream, Message::Goodbye)?;
    Ok(())
}

fn serve(mut stream: UnixStream) -> Result<()> {
    configure_stream(&stream)?;
    match read_message(&mut stream)? {
        Message::Hello { version } => negotiate(version)?,
        _ => return Err("hello must be the first message".into()),
    }
    write_message(&mut stream, Message::Welcome { version: VERSION })?;
    let extent = Extent {
        width: 800,
        height: 600,
    };
    let mut surface = Surface::new(extent).map_err(|e| format!("surface: {e:?}"))?;
    write_message(&mut stream, Message::Configure { serial: 1, extent })?;
    let started = Instant::now();
    for expected in 1..=3 {
        let next = match read_message(&mut stream)? {
            Message::Submit(frame) if frame.sequence == expected => frame,
            _ => return Err("expected frame metadata".into()),
        };
        surface.submit(next).map_err(|e| format!("submit: {e:?}"))?;
        surface
            .begin_present()
            .map_err(|e| format!("begin: {e:?}"))?;
        surface
            .complete_present()
            .map_err(|e| format!("complete: {e:?}"))?;
        if surface.displayed() != Some(next) {
            return Err("state did not commit".into());
        }
        write_message(
            &mut stream,
            Message::Presented {
                sequence: next.sequence,
                timestamp_ns: started.elapsed().as_nanos() as u64,
            },
        )?;
    }
    if read_message(&mut stream)? != Message::Goodbye {
        return Err("expected goodbye".into());
    }
    Ok(())
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
    for _ in 0..2 {
        probe.children.push(
            Command::new(std::env::current_exe()?)
                .arg("--client")
                .arg(&path)
                .env_remove("DISPLAY")
                .env_remove("WAYLAND_DISPLAY")
                .spawn()?,
        );
    }
    let deadline = Instant::now() + DEADLINE;
    let mut workers = Vec::new();
    while workers.len() < 2 {
        match listener.accept() {
            Ok((stream, _)) => workers.push(thread::spawn(move || serve(stream))),
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
    println!("{{\"mode\":\"headless-metadata\",\"clients\":2,\"frames\":6,\"graphics\":false,\"bloom\":false,\"status\":\"passed\"}}");
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [] => probe(),
        [flag, path] if flag == "--client" => client(Path::new(path)),
        _ => Err("usage: oxy-platform-probe [--client SOCKET]".into()),
    }
}
