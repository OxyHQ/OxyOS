//! The login screen's authentication path (docs/adr/0020-oxy-account-login.mdx): the login
//! program (`oxyos-login`), which alone checks a password, for the account the lock surface named.
//! The session program at the login screen (`oxyos-session --greeter`) and the development
//! compositor (`oxyos-offscreen --login`) both hand passwords to it so.

use crate::Authentication;
use oxyos_auth::greeter::{Answer, Greeter, Verdict};
use oxyos_protocol::AuthOutcome;
use rustix::fd::BorrowedFd;

pub struct LoginProgram(pub Greeter);

fn outcome(answer: Answer) -> (AuthOutcome, u32) {
    let outcome = match answer.verdict {
        Verdict::Accepted => AuthOutcome::Accepted,
        Verdict::Rejected => AuthOutcome::Rejected,
        Verdict::Busy => AuthOutcome::Busy,
        Verdict::Throttled => AuthOutcome::Throttled,
    };
    (outcome, answer.wait_s)
}

impl Authentication for LoginProgram {
    fn attempt(
        &mut self,
        account: Option<&str>,
        mut secret: oxyos_compositor_core::scene::Secret,
    ) -> Option<(AuthOutcome, u32)> {
        // The password's memory moves to the login program's message whole, and is overwritten.
        let secret = oxyos_auth::Secret::new(std::mem::take(&mut *secret));
        self.0.attempt(account, secret).map(outcome)
    }

    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }

    fn answer(&mut self) -> Option<(AuthOutcome, u32)> {
        self.0.answer().map(outcome)
    }
}
