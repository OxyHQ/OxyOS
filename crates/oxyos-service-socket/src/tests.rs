//! The services' sockets, end to end over real `SOCK_SEQPACKET` sockets: in a directory of the
//! test's own where one is bound, over a socket pair otherwise (the peer's credentials are the
//! kernel's: this process's). What the account service's and the install service's copies each
//! did, now done here once.

use super::*;
use rustix::fd::AsFd;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

/// The account service's bound, the one service that keeps its own.
type Small = BoundedConnection<{ 16 * 1024 }>;

/// A directory of the test's own, removed when it ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "oxyos-service-socket-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Sends `bytes` as one packet with `fds`, as a peer that does not go through [`Connection`].
fn send_raw(connection: &impl AsFd, bytes: &[u8], fds: &[BorrowedFd<'_>]) {
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(4))];
    let mut control = SendAncillaryBuffer::new(&mut space);
    if !fds.is_empty() {
        assert!(control.push(SendAncillaryMessage::ScmRights(fds)));
    }
    rustix::net::sendmsg(
        connection,
        &[IoSlice::new(bytes)],
        &mut control,
        SendFlags::NOSIGNAL,
    )
    .unwrap();
}

/// A pipe's reading end and a writing end: the reading end reads its end once every copy of the
/// writing end is closed, which is how a test sees that a descriptor sent was closed.
fn pipe() -> (File, OwnedFd) {
    let (reading, writing) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).unwrap();
    (File::from(reading), writing)
}

fn ended(mut reading: File) -> bool {
    let mut byte = [0u8; 1];
    reading.read(&mut byte).unwrap() == 0
}

#[test]
fn the_socket_replaces_an_old_socket_never_another_file() {
    let dir = TempDir::new("bind");
    let socket = dir.path().join("socket");
    drop(Listener::bind(&socket, 0o600).unwrap());
    Listener::bind(&socket, 0o600).unwrap();
    let file = dir.path().join("file");
    std::fs::write(&file, "keep").unwrap();
    let error = Listener::bind(&file, 0o600).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "keep");
}

#[test]
fn the_socket_has_the_mode_asked_for() {
    let dir = TempDir::new("mode");
    for mode in [0o600, 0o660, 0o666] {
        let socket = dir.path().join(format!("socket-{mode:o}"));
        let _listener = Listener::bind(&socket, mode).unwrap();
        let metadata = std::fs::symlink_metadata(&socket).unwrap();
        assert!(metadata.file_type().is_socket());
        assert_eq!(metadata.permissions().mode() & 0o7777, mode);
    }
    // The owner given, here this process's own user (the only one a test may give it to).
    let socket = dir.path().join("owned");
    let uid = rustix::process::geteuid().as_raw();
    drop(bind_in_place(&socket, 0o600, Some(uid), 4).unwrap());
    assert_eq!(
        std::os::unix::fs::MetadataExt::uid(&std::fs::symlink_metadata(&socket).unwrap()),
        uid
    );
}

#[test]
fn the_socket_appears_only_listening() {
    use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};
    let dir = TempDir::new("listening");
    let socket = dir.path().join("socket");
    let watch = inotify::init(CreateFlags::NONBLOCK | CreateFlags::CLOEXEC).unwrap();
    inotify::add_watch(
        &watch,
        dir.path(),
        WatchFlags::CREATE | WatchFlags::MOVED_TO,
    )
    .unwrap();
    // Made, then made again over the one a run before left: each time the name appears only by
    // a rename of a socket already listening, never by its bind (a connection between a bind and
    // its listen would be refused).
    for _ in 0..2 {
        let listener = Listener::bind(&socket, 0o666).unwrap();
        let mut buffer = [MaybeUninit::<u8>::uninit(); 4096];
        let mut reader = inotify::Reader::new(&watch, &mut buffer);
        let mut named = Vec::new();
        while let Ok(event) = reader.next() {
            if event.file_name().map(|name| name.to_bytes()) == Some(b"socket".as_slice()) {
                named.push(event.events());
            }
        }
        assert_eq!(named, [ReadFlags::MOVED_TO]);
        let connected = Connection::connect(&socket).unwrap();
        let accepted = listener.accept().unwrap();
        drop((connected, accepted));
    }
    // Nothing of the making is left beside it.
    let left: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name())
        .collect();
    assert_eq!(left, [std::ffi::OsString::from("socket")]);
}

#[test]
fn the_peer_is_the_kernels_word() {
    let dir = TempDir::new("peer");
    let socket = dir.path().join("socket");
    let listener = Listener::bind(&socket, 0o600).unwrap();
    let client = Connection::connect(&socket).unwrap();
    let service = listener.accept().unwrap();
    let me = Peer {
        pid: std::process::id() as i32,
        uid: rustix::process::geteuid().as_raw(),
        gid: rustix::process::getegid().as_raw(),
    };
    assert_eq!(service.peer().unwrap(), me);
    assert_eq!(client.peer().unwrap(), me);
}

#[test]
fn messages_go_both_ways_and_the_end_is_none() {
    let (client, service) = Connection::pair().unwrap();
    client.send(&serde_json::json!({"list": {}})).unwrap();
    let heard: Option<Option<serde_json::Value>> = service.receive().unwrap();
    assert_eq!(heard, Some(Some(serde_json::json!({"list": {}}))));
    // A clone sends on the same connection.
    service.try_clone().unwrap().send(&7u32).unwrap();
    assert_eq!(client.receive::<u32>().unwrap(), Some(Some(7)));
    drop(service);
    assert!(client.receive::<u32>().unwrap().is_none());
}

#[test]
fn what_is_not_a_message_reads_as_none_and_the_connection_goes_on() {
    let (client, service) = Connection::pair().unwrap();
    send_raw(&client, b"not json", &[]);
    assert_eq!(service.receive::<u32>().unwrap(), Some(None));
    // JSON, but not the message asked for.
    client.send(&"a string").unwrap();
    assert_eq!(service.receive::<u32>().unwrap(), Some(None));
    client.send(&3u32).unwrap();
    assert_eq!(service.receive::<u32>().unwrap(), Some(Some(3)));
}

#[test]
fn a_message_is_at_most_the_bound() {
    fn bounded<const MAX: usize>() {
        let (client, service) = BoundedConnection::<MAX>::pair().unwrap();
        // A string of `n` characters is `n + 2` bytes of JSON.
        client.send(&"x".repeat(MAX - 2)).unwrap();
        assert_eq!(
            service.receive::<String>().unwrap(),
            Some(Some("x".repeat(MAX - 2)))
        );
        let error = client.send(&"x".repeat(MAX - 1)).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        // One sent anyway, by a peer that keeps no bound, is not a message, however long.
        for length in [MAX + 1, MAX + 2, 2 * MAX] {
            let long = format!("\"{}\"", "x".repeat(length - 2));
            send_raw(&client, long.as_bytes(), &[]);
            assert_eq!(service.receive::<String>().unwrap(), Some(None), "{length}");
        }
        client.send(&1u32).unwrap();
        assert_eq!(service.receive::<u32>().unwrap(), Some(Some(1)));
    }
    bounded::<MAX_MESSAGE>();
    bounded::<{ 16 * 1024 }>();
    // The account service's bound is its own, below the others'.
    let (client, _service) = Small::pair().unwrap();
    assert!(client.send(&"x".repeat(16 * 1024)).is_err());
}

#[test]
fn a_descriptor_comes_with_its_message() {
    let (client, service) = Connection::pair().unwrap();
    let (reading, writing) = pipe();
    client.send_with_fd(&"here", writing.as_fd()).unwrap();
    drop(writing);
    let (message, fd) = service.receive_with_fd::<String>().unwrap().unwrap();
    assert_eq!(message.as_deref(), Some("here"));
    let fd = fd.unwrap();
    rustix::io::write(&fd, b"!").unwrap();
    drop(fd);
    let mut reading = reading;
    let mut said = Vec::new();
    reading.read_to_end(&mut said).unwrap();
    assert_eq!(said, b"!");
}

#[test]
fn a_descriptor_no_one_asked_for_is_closed() {
    let (client, service) = Connection::pair().unwrap();
    let (reading, writing) = pipe();
    client.send_with_fd(&5u32, writing.as_fd()).unwrap();
    drop(writing);
    assert_eq!(service.receive::<u32>().unwrap(), Some(Some(5)));
    assert!(ended(reading));
}

#[test]
fn a_packet_with_more_descriptors_than_one_is_not_a_message_and_they_are_closed() {
    let (client, service) = Connection::pair().unwrap();
    for receive_with_fd in [false, true] {
        let (first_reading, first) = pipe();
        let (second_reading, second) = pipe();
        send_raw(&client, b"5", &[first.as_fd(), second.as_fd()]);
        drop((first, second));
        if receive_with_fd {
            let (message, fd) = service.receive_with_fd::<u32>().unwrap().unwrap();
            assert_eq!(message, None);
            assert!(fd.is_none());
        } else {
            assert_eq!(service.receive::<u32>().unwrap(), Some(None));
        }
        assert!(ended(first_reading));
        assert!(ended(second_reading));
    }
}

#[test]
fn a_packet_too_long_has_its_descriptor_closed() {
    let (client, service) = Small::pair().unwrap();
    let (reading, writing) = pipe();
    let long = format!("\"{}\"", "x".repeat(16 * 1024));
    send_raw(&client, long.as_bytes(), &[writing.as_fd()]);
    drop(writing);
    let (message, fd) = service.receive_with_fd::<String>().unwrap().unwrap();
    assert_eq!(message, None);
    assert!(fd.is_none());
    assert!(ended(reading));
}

#[test]
fn a_listener_systemd_passed_accepts() {
    let dir = TempDir::new("activation");
    let socket = dir.path().join("socket");
    let fd = bind_in_place(&socket, 0o600, None, 16).unwrap();
    let listener = Listener::from_fd(fd);
    let client = Connection::connect(&socket).unwrap();
    let service = listener.accept().unwrap();
    client.send(&2u32).unwrap();
    assert_eq!(service.receive::<u32>().unwrap(), Some(Some(2)));
}

/// The service's last message, sent just before it closes, is read before the end, with a
/// descriptor or without, over a bound socket and over a pair: the kernel can report the end with
/// it still queued (`oxyos-seqpacket`). Many rounds, since each one is a race. The client polls
/// (its socket non-blocking), where the race is widest; a waiting receive goes through the same
/// read. Each receive here zeroes a message's buffer, so few rounds meet the race:
/// `oxyos-seqpacket`'s own test is the one that meets it often.
#[test]
fn the_last_message_before_the_end_is_read_before_the_end() {
    fn polled<T>(receive: impl Fn() -> io::Result<T>) -> T {
        loop {
            match receive() {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => std::hint::spin_loop(),
                received => break received.unwrap(),
            }
        }
    }
    let dir = TempDir::new("last-message");
    let socket = dir.path().join("socket");
    let listener = Listener::bind(&socket, 0o600).unwrap();
    for round in 0..2000u32 {
        let (client, service) = if round % 4 < 2 {
            let client = Connection::connect(&socket).unwrap();
            (client, listener.accept().unwrap())
        } else {
            Connection::pair().unwrap()
        };
        rustix::io::ioctl_fionbio(client.as_fd(), true).unwrap();
        let with_descriptor = round % 2 == 1;
        let sender = std::thread::spawn(move || {
            if with_descriptor {
                let null = File::open("/dev/null").unwrap();
                service.send_with_fd(&round, null.as_fd()).unwrap();
            } else {
                service.send(&round).unwrap();
            }
            // Shut down, not only closed: a process another test forks meanwhile may hold a copy
            // of the descriptor until it executes.
            rustix::net::shutdown(service.as_fd(), rustix::net::Shutdown::Both).unwrap();
        });
        let heard = if with_descriptor {
            polled(|| client.receive_with_fd::<u32>()).map(|(message, fd)| (message, fd.is_some()))
        } else {
            polled(|| client.receive::<u32>()).map(|message| (message, false))
        };
        assert_eq!(heard, Some((Some(round), with_descriptor)), "round {round}");
        sender.join().unwrap();
        assert!(polled(|| client.receive::<u32>()).is_none());
    }
}
