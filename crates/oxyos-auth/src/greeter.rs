//! Between the login screen's compositor (the greeter, running as the login program's own user,
//! `oxyos-login`) and the login program (`oxyos-login`, root), which alone checks passwords and
//! starts sessions ([ADR 0020](../../docs/adr/0020-oxy-account-login.mdx), step 2). The greeter
//! has no authority: it hands over what the person typed and shows the answer.
//!
//! - **Transport:** a `SOCK_SEQPACKET` Unix socket, `/run/oxyos/login/greeter`; one message a
//!   packet. The login program accepts only its greeter's user, by the kernel's peer credentials.
//! - **Greeter → login program:**
//!   - `Attempt`: `a`, the account's length (one byte, 1 to [`MAX_ACCOUNT_BYTES`]), the account
//!     (a Unix user's name), then the password (the rest, at most [`MAX_SECRET_BYTES`]);
//!   - `Shown`: `s`, once, when the greeter's first frame is on a display (the boot logo may go,
//!     docs/adr/0017-boot-logo.mdx).
//! - **Login program → greeter:** `Answer`: `r`, the verdict (one byte: 0 accepted, 1 rejected,
//!   2 busy, 3 throttled), the seconds until another attempt (four bytes, little-endian).
//!   Accepted: the person is signed in, and the greeter is about to be ended.
//!
//! The password is overwritten in memory on both sides once used.

use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::net::{AddressFamily, RecvFlags, SendFlags, SocketAddrUnix, SocketFlags, SocketType};
use std::io;
use std::path::Path;
use zeroize::Zeroizing;

/// Where the login program listens for its greeter.
pub const SOCKET: &str = "/run/oxyos/login/greeter";
/// The longest account: a Unix user's name.
pub const MAX_ACCOUNT_BYTES: usize = 31;
/// The longest password: Linux-PAM's longest answer.
pub const MAX_SECRET_BYTES: usize = 512;
const MAX_PACKET: usize = 2 + MAX_ACCOUNT_BYTES + MAX_SECRET_BYTES;

/// What the login program said of an attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Accepted,
    Rejected,
    Busy,
    Throttled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Answer {
    pub verdict: Verdict,
    pub wait_s: u32,
}

/// A message to the login program.
pub enum ToLogin {
    Attempt {
        account: String,
        secret: crate::Secret,
    },
    Shown,
}

impl std::fmt::Debug for ToLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToLogin::Attempt { account, .. } => write!(f, "Attempt {{ account: {account:?}, … }}"),
            ToLogin::Shown => f.write_str("Shown"),
        }
    }
}

fn invalid(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.to_owned())
}

impl ToLogin {
    fn encode(&self) -> io::Result<Zeroizing<Vec<u8>>> {
        let mut packet = Zeroizing::new(Vec::with_capacity(MAX_PACKET));
        match self {
            ToLogin::Attempt { account, secret } => {
                if account.is_empty() || account.len() > MAX_ACCOUNT_BYTES {
                    return Err(invalid("an account is 1 to 31 bytes"));
                }
                if secret.0.len() > MAX_SECRET_BYTES {
                    return Err(invalid("a password is at most 512 bytes"));
                }
                packet.push(b'a');
                packet.push(account.len() as u8);
                packet.extend_from_slice(account.as_bytes());
                packet.extend_from_slice(&secret.0);
            }
            ToLogin::Shown => packet.push(b's'),
        }
        Ok(packet)
    }

    pub fn decode(packet: &[u8]) -> io::Result<Self> {
        match packet {
            [b's'] => Ok(ToLogin::Shown),
            [b'a', length, rest @ ..] => {
                let length = usize::from(*length);
                if length == 0 || length > MAX_ACCOUNT_BYTES || rest.len() < length {
                    return Err(invalid("not an account"));
                }
                let (account, secret) = rest.split_at(length);
                if secret.len() > MAX_SECRET_BYTES {
                    return Err(invalid("a password is at most 512 bytes"));
                }
                let account = std::str::from_utf8(account)
                    .map_err(|_| invalid("not an account"))?
                    .to_owned();
                Ok(ToLogin::Attempt {
                    account,
                    secret: crate::Secret::new(secret.to_vec()),
                })
            }
            _ => Err(invalid("not a message to the login program")),
        }
    }
}

impl Answer {
    fn encode(&self) -> [u8; 6] {
        let verdict = match self.verdict {
            Verdict::Accepted => 0,
            Verdict::Rejected => 1,
            Verdict::Busy => 2,
            Verdict::Throttled => 3,
        };
        let wait = self.wait_s.to_le_bytes();
        [b'r', verdict, wait[0], wait[1], wait[2], wait[3]]
    }

    fn decode(packet: &[u8]) -> io::Result<Self> {
        let [b'r', verdict, a, b, c, d] = *packet else {
            return Err(invalid("not an answer"));
        };
        let verdict = match verdict {
            0 => Verdict::Accepted,
            1 => Verdict::Rejected,
            2 => Verdict::Busy,
            3 => Verdict::Throttled,
            _ => return Err(invalid("not a verdict")),
        };
        Ok(Self {
            verdict,
            wait_s: u32::from_le_bytes([a, b, c, d]),
        })
    }
}

fn seqpacket() -> io::Result<OwnedFd> {
    Ok(rustix::net::socket_with(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC,
        None,
    )?)
}

/// One packet, `None` once the peer has closed. A packet too long for any message is an error.
fn receive(fd: BorrowedFd<'_>) -> io::Result<Option<Zeroizing<Vec<u8>>>> {
    let mut packet = Zeroizing::new(vec![0u8; MAX_PACKET + 1]);
    let (length, _) = rustix::net::recv(fd, &mut packet[..], RecvFlags::DONTWAIT)?;
    if length == 0 {
        return Ok(None);
    }
    if length > MAX_PACKET {
        return Err(invalid("a packet too long"));
    }
    packet.truncate(length);
    Ok(Some(packet))
}

/// The greeter's side: the login screen's compositor's authentication path. One attempt at a
/// time; the answer arrives on the socket, which [`Greeter::as_fd`] polls.
pub struct Greeter {
    fd: OwnedFd,
    checking: bool,
}

impl Greeter {
    pub fn connect(path: &Path) -> io::Result<Self> {
        let fd = seqpacket()?;
        rustix::net::connect(&fd, &SocketAddrUnix::new(path)?)?;
        Ok(Self {
            fd,
            checking: false,
        })
    }

    fn send(&self, message: &ToLogin) -> io::Result<()> {
        rustix::net::send(&self.fd, &message.encode()?, SendFlags::NOSIGNAL)?;
        Ok(())
    }

    /// Hands the login program what was typed for `account`: `None` while it checks, or an
    /// answer at once (busy while another is checked; rejected without an account, which the
    /// login screen always names, or if the login program cannot be told).
    pub fn attempt(&mut self, account: Option<&str>, secret: crate::Secret) -> Option<Answer> {
        if self.checking {
            return Some(Answer {
                verdict: Verdict::Busy,
                wait_s: 0,
            });
        }
        let rejected = Some(Answer {
            verdict: Verdict::Rejected,
            wait_s: 0,
        });
        let Some(account) = account else {
            return rejected;
        };
        let attempt = ToLogin::Attempt {
            account: account.to_owned(),
            secret,
        };
        match self.send(&attempt) {
            Ok(()) => {
                self.checking = true;
                None
            }
            Err(error) => {
                eprintln!("[login] telling the login program: {error}");
                rejected
            }
        }
    }

    /// Tells the login program the login screen is on a display.
    pub fn shown(&self) -> io::Result<()> {
        self.send(&ToLogin::Shown)
    }

    /// What tells the login program the login screen is on a display, apart from the attempts:
    /// the same connection, on a descriptor of its own (the compositor's first flip says it).
    pub fn on_screen(&self) -> io::Result<OnScreen> {
        Ok(OnScreen(self.fd.as_fd().try_clone_to_owned()?))
    }

    /// The answer to the attempt being checked, once it has come. The login program gone, it is
    /// a rejection: nobody is signed in.
    pub fn answer(&mut self) -> Option<Answer> {
        if !self.checking {
            // Drain anything unasked for.
            let _ = receive(self.fd.as_fd());
            return None;
        }
        let answer = match receive(self.fd.as_fd()) {
            Ok(Some(packet)) => Answer::decode(&packet).unwrap_or(Answer {
                verdict: Verdict::Rejected,
                wait_s: 0,
            }),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return None,
            Ok(None) | Err(_) => Answer {
                verdict: Verdict::Rejected,
                wait_s: 0,
            },
        };
        self.checking = false;
        Some(answer)
    }

    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

/// Says, once, that the login screen is on a display ([`Greeter::on_screen`]).
pub struct OnScreen(OwnedFd);

impl OnScreen {
    pub fn tell(&self) -> io::Result<()> {
        rustix::net::send(&self.0, &ToLogin::Shown.encode()?, SendFlags::NOSIGNAL)?;
        Ok(())
    }
}

/// The login program's side: its socket, for its greeter alone.
pub struct Listener {
    fd: OwnedFd,
}

impl Listener {
    /// Listens at `path` (a socket an earlier run left there is replaced, never another file),
    /// which only `uid`'s processes may connect to: the socket is `uid`'s, mode 0600, as well as
    /// checked by peer credentials at each connection.
    pub fn bind(path: &Path, uid: u32) -> io::Result<Self> {
        use std::os::unix::fs::FileTypeExt;
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_socket() => std::fs::remove_file(path)?,
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} exists and is not a socket", path.display()),
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let fd = seqpacket()?;
        rustix::net::bind(&fd, &SocketAddrUnix::new(path)?)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        if rustix::process::geteuid().as_raw() != uid {
            rustix::fs::chown(
                path,
                Some(rustix::fs::Uid::from_raw(uid)),
                None::<rustix::fs::Gid>,
            )?;
        }
        rustix::net::listen(&fd, 4)?;
        rustix::io::ioctl_fionbio(&fd, true)?;
        Ok(Self { fd })
    }

    /// A greeter connecting, if one is: only one of `uid`'s; anyone else's connection is closed
    /// unanswered.
    pub fn accept(&self, uid: u32) -> io::Result<Option<Connection>> {
        let fd = match rustix::net::accept_with(&self.fd, SocketFlags::CLOEXEC) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::AGAIN) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let peer = rustix::net::sockopt::socket_peercred(&fd)?;
        if peer.uid.as_raw() != uid {
            eprintln!(
                "[login] a connection from uid {} (pid {}), not the greeter's: closed",
                peer.uid.as_raw(),
                peer.pid.as_raw_nonzero()
            );
            return Ok(None);
        }
        Ok(Some(Connection { fd }))
    }

    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

/// The login program's connection with its greeter.
pub struct Connection {
    fd: OwnedFd,
}

impl Connection {
    /// The greeter's next message: `Ok(None)` when none is waiting; an error once the greeter
    /// has closed (`UnexpectedEof`) or sent what is no message.
    pub fn receive(&self) -> io::Result<Option<ToLogin>> {
        match receive(self.fd.as_fd()) {
            Ok(Some(packet)) => ToLogin::decode(&packet).map(Some),
            Ok(None) => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the greeter closed",
            )),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub fn answer(&self, answer: Answer) -> io::Result<()> {
        rustix::net::send(&self.fd, &answer.encode(), SendFlags::NOSIGNAL)?;
        Ok(())
    }

    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}
