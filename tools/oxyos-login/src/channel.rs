//! Between the login program and each of its workers: a `SOCK_SEQPACKET` socket pair, the
//! worker's end its standard input. One message a packet:
//!
//! - **login program → worker:** `p` and the password (a user worker's, once), `o` (open the
//!   session: the login screen has ended). Closing it ends the worker, and the session it holds.
//! - **worker → login program:** `a` accepted, `r` rejected, `f` failed (PAM did not answer), `s`
//!   the session is open and its program started (and its process id, four bytes little-endian,
//!   which the login program kills if the worker cannot stop it), `e` how the session ended
//!   (docs/adr/0065: `x` and its exit status, `g` and the signal that ended it, `h` hung, `a`
//!   asked to end; then how long it ran, in seconds, four bytes little-endian). The worker's exit
//!   is the session's end.

use oxyos_auth::Secret;
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::net::{AddressFamily, RecvFlags, SendFlags, SocketFlags, SocketType};
use std::io;
use zeroize::Zeroizing;

const MAX_PACKET: usize = 1 + oxyos_auth::greeter::MAX_SECRET_BYTES;

#[derive(Debug)]
pub enum ToWorker {
    Password(Secret),
    Open,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FromWorker {
    Accepted,
    Rejected,
    Failed,
    Started(u32),
    /// How the session ended, and how many seconds it ran.
    Ended(End, u32),
}

/// How a session ended, as its worker saw it (docs/adr/0065-the-session-ending-because-of-a-problem.mdx).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// It exited with this status: 0 is the person signing out (or the session's own orderly end).
    Exited(u8),
    /// A signal nobody sent on the login program's behalf ended it (a crash: `SIGSEGV`, `SIGABRT`…).
    Signal(u8),
    /// It stopped answering its heartbeat and was ended for it.
    Hung,
    /// The login program asked it to end (it is stopping, the machine shutting down).
    Asked,
}

impl End {
    /// Whether it ended because of a problem: what the login screen and the next session say.
    pub fn problem(self) -> bool {
        matches!(self, End::Exited(1..) | End::Signal(_) | End::Hung)
    }

    /// As a session is told it (`OXYOS_SESSION_ENDED`, with how long it ran): `exit 1`,
    /// `signal 11`, `hang`.
    pub fn word(self) -> String {
        match self {
            End::Exited(status) => format!("exit {status}"),
            End::Signal(signal) => format!("signal {signal}"),
            End::Hung => "hang".into(),
            End::Asked => "asked".into(),
        }
    }

    fn encode(self) -> [u8; 2] {
        match self {
            End::Exited(status) => [b'x', status],
            End::Signal(signal) => [b'g', signal],
            End::Hung => [b'h', 0],
            End::Asked => [b'a', 0],
        }
    }

    fn decode(kind: u8, value: u8) -> Option<Self> {
        match kind {
            b'x' => Some(End::Exited(value)),
            b'g' => Some(End::Signal(value)),
            b'h' => Some(End::Hung),
            b'a' => Some(End::Asked),
            _ => None,
        }
    }
}

pub struct Channel {
    fd: OwnedFd,
}

/// A pair of ends: the login program's, and the worker's (its standard input).
pub fn pair() -> io::Result<(Channel, OwnedFd)> {
    let (ours, theirs) = rustix::net::socketpair(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC,
        None,
    )?;
    Ok((Channel { fd: ours }, theirs))
}

impl Channel {
    pub fn from_fd(fd: OwnedFd) -> Self {
        Self { fd }
    }

    fn send(&self, packet: &[u8]) -> io::Result<()> {
        rustix::net::send(&self.fd, packet, SendFlags::NOSIGNAL)?;
        Ok(())
    }

    pub fn send_password(&self, secret: Secret) -> io::Result<()> {
        let bytes = secret.into_bytes();
        let mut packet = Zeroizing::new(Vec::with_capacity(1 + bytes.len()));
        packet.push(b'p');
        packet.extend_from_slice(&bytes);
        self.send(&packet)
    }

    pub fn send_open(&self) -> io::Result<()> {
        self.send(b"o")
    }

    pub fn tell(&self, message: FromWorker) -> io::Result<()> {
        match message {
            FromWorker::Accepted => self.send(b"a"),
            FromWorker::Rejected => self.send(b"r"),
            FromWorker::Failed => self.send(b"f"),
            FromWorker::Started(pid) => {
                let pid = pid.to_le_bytes();
                self.send(&[b's', pid[0], pid[1], pid[2], pid[3]])
            }
            FromWorker::Ended(end, ran) => {
                let [kind, value] = end.encode();
                let ran = ran.to_le_bytes();
                self.send(&[b'e', kind, value, ran[0], ran[1], ran[2], ran[3]])
            }
        }
    }

    fn receive(&self, wait: bool) -> io::Result<Option<Zeroizing<Vec<u8>>>> {
        let mut packet = Zeroizing::new(vec![0u8; MAX_PACKET + 1]);
        let flags = if wait {
            RecvFlags::empty()
        } else {
            RecvFlags::DONTWAIT
        };
        let received =
            match oxyos_seqpacket::receive(self.fd.as_fd(), &mut packet[..], false, flags) {
                Ok(Some(received)) => received,
                Ok(None) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "the other side closed",
                    ))
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                Err(error) => return Err(error),
            };
        let length = received.length;
        if length > MAX_PACKET || received.truncated {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "too long"));
        }
        packet.truncate(length);
        Ok(Some(packet))
    }

    /// The worker's side: the next message, waiting for it. The login program gone is
    /// `UnexpectedEof`.
    pub fn next(&self) -> io::Result<ToWorker> {
        let packet = self.receive(true)?.expect("a blocking receive answers");
        match packet.split_first() {
            Some((b'p', password)) => Ok(ToWorker::Password(Secret::new(password.to_vec()))),
            Some((b'o', [])) => Ok(ToWorker::Open),
            _ => Err(io::Error::new(io::ErrorKind::InvalidData, "not a message")),
        }
    }

    /// The login program's side: what the worker said, if it said anything.
    pub fn heard(&self) -> io::Result<Option<FromWorker>> {
        let Some(packet) = self.receive(false)? else {
            return Ok(None);
        };
        match &packet[..] {
            b"a" => Ok(Some(FromWorker::Accepted)),
            b"r" => Ok(Some(FromWorker::Rejected)),
            b"f" => Ok(Some(FromWorker::Failed)),
            [b's', a, b, c, d] => Ok(Some(FromWorker::Started(u32::from_le_bytes([
                *a, *b, *c, *d,
            ])))),
            [b'e', kind, value, a, b, c, d] => match End::decode(*kind, *value) {
                Some(end) => Ok(Some(FromWorker::Ended(
                    end,
                    u32::from_le_bytes([*a, *b, *c, *d]),
                ))),
                None => Err(io::Error::new(io::ErrorKind::InvalidData, "not an end")),
            },
            _ => Err(io::Error::new(io::ErrorKind::InvalidData, "not a message")),
        }
    }
}

impl AsFd for Channel {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_cross_both_ways() {
        let (manager, worker) = pair().unwrap();
        let worker = Channel::from_fd(worker);
        assert_eq!(manager.heard().unwrap(), None);
        manager
            .send_password(Secret::new(b"correct horse".to_vec()))
            .unwrap();
        manager.send_open().unwrap();
        let ToWorker::Password(password) = worker.next().unwrap() else {
            panic!("the password");
        };
        assert_eq!(&password.into_bytes()[..], b"correct horse");
        assert!(matches!(worker.next().unwrap(), ToWorker::Open));
        for message in [
            FromWorker::Accepted,
            FromWorker::Rejected,
            FromWorker::Failed,
            FromWorker::Started(4321),
            FromWorker::Ended(End::Exited(0), 3600),
            FromWorker::Ended(End::Exited(1), 2),
            FromWorker::Ended(End::Signal(11), 70_000),
            FromWorker::Ended(End::Hung, 12),
            FromWorker::Ended(End::Asked, 5),
        ] {
            worker.tell(message).unwrap();
            assert_eq!(manager.heard().unwrap(), Some(message));
        }
        drop(manager);
        assert_eq!(
            worker.next().unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn a_problem_is_a_crash_a_hang_or_a_failure_never_signing_out_or_being_asked() {
        assert!(End::Signal(11).problem());
        assert!(End::Hung.problem());
        assert!(End::Exited(1).problem());
        assert!(!End::Exited(0).problem());
        assert!(!End::Asked.problem());
        assert_eq!(End::Signal(6).word(), "signal 6");
        assert_eq!(End::Exited(1).word(), "exit 1");
        assert_eq!(End::Hung.word(), "hang");
    }

    /// The last message before an end is read before it, each way: what a worker says as it
    /// exits (its session started, say), what the login program sends as it lets the worker go.
    /// The kernel can report the end with that message still queued (`oxyos-seqpacket`). Many
    /// rounds, since each one is a race, mostly the login program's polling (busily), where the
    /// race is widest.
    #[test]
    fn the_last_message_before_the_end_is_read_before_the_end() {
        // Shut down, not only closed: a process another test forks meanwhile may hold a copy of
        // the descriptor until it executes.
        let gone = |channel: &Channel| {
            rustix::net::shutdown(channel.as_fd(), rustix::net::Shutdown::Both).unwrap()
        };
        for round in 0..10_000u32 {
            let (manager, worker) = pair().unwrap();
            let worker = Channel::from_fd(worker);
            if round % 4 != 3 {
                let exiting = std::thread::spawn(move || {
                    worker.tell(FromWorker::Started(round)).unwrap();
                    gone(&worker);
                });
                let heard = loop {
                    match manager.heard() {
                        Ok(Some(message)) => break message,
                        Ok(None) => std::hint::spin_loop(),
                        Err(error) => panic!("round {round}: {error} before the message"),
                    }
                };
                assert_eq!(heard, FromWorker::Started(round));
                exiting.join().unwrap();
                assert_eq!(
                    manager.heard().unwrap_err().kind(),
                    io::ErrorKind::UnexpectedEof
                );
            } else {
                let letting_go = std::thread::spawn(move || {
                    manager.send_open().unwrap();
                    gone(&manager);
                });
                assert!(matches!(worker.next(), Ok(ToWorker::Open)), "round {round}");
                letting_go.join().unwrap();
                assert_eq!(
                    worker.next().unwrap_err().kind(),
                    io::ErrorKind::UnexpectedEof
                );
            }
        }
    }
}
