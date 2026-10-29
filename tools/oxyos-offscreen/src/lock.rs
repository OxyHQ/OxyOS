//! The development compositor's answer to the lock screen (docs/adr/0010-lock-screen.mdx): a
//! lock surface's password is checked against the one the scenario gives (`--lock-password`),
//! never with PAM. It is a fixture: the session's authentication path is PAM's, and the session
//! never has this one.

use oxyos_compositor::Authentication;
use oxyos_compositor_core::scene::Secret;
use oxyos_protocol::AuthOutcome;
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};

pub struct Fixture {
    password: String,
    /// What the password submitted came to, until it is answered.
    checked: Option<bool>,
    ready: OwnedFd,
    signal: OwnedFd,
}

impl Fixture {
    pub fn new(password: String) -> std::io::Result<Self> {
        let (ready, signal) = rustix::pipe::pipe_with(
            rustix::pipe::PipeFlags::CLOEXEC | rustix::pipe::PipeFlags::NONBLOCK,
        )?;
        Ok(Self {
            password,
            checked: None,
            ready,
            signal,
        })
    }
}

impl Authentication for Fixture {
    fn attempt(&mut self, secret: Secret) -> Option<(AuthOutcome, u32)> {
        if self.checked.is_some() {
            return Some((AuthOutcome::Busy, 0));
        }
        // Answered on the compositor's next turn, as an authentication path off its thread is.
        self.checked = Some(&secret[..] == self.password.as_bytes());
        let _ = rustix::io::write(&self.signal, &[1]);
        None
    }

    fn as_fd(&self) -> BorrowedFd<'_> {
        self.ready.as_fd()
    }

    fn answer(&mut self) -> Option<(AuthOutcome, u32)> {
        let right = self.checked.take()?;
        let mut drained = [0; 8];
        while rustix::io::read(&self.ready, &mut drained).is_ok_and(|read| read > 0) {}
        Some(if right {
            (AuthOutcome::Accepted, 0)
        } else {
            (AuthOutcome::Rejected, 0)
        })
    }
}
