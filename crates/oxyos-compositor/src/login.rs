//! The login screen's authentication path (docs/adr/0020-oxy-account-login.mdx): the login
//! program (`oxyos-login`), which alone checks a password, for the account the lock surface named.
//! The session program at the login screen (`oxyos-session --greeter`) and the development
//! compositor (`oxyos-offscreen --login`) both hand passwords to it so.

use crate::Authentication;
use oxyos_auth::greeter::{Answer, Greeter, Verdict};
use oxyos_compositor_core::Scene;
use oxyos_protocol::AuthOutcome;
use rustix::fd::BorrowedFd;
use serde_json::json;

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
        passcode: bool,
        mut secret: oxyos_compositor_core::scene::Secret,
    ) -> Option<(AuthOutcome, u32)> {
        // The password's memory moves to the login program's message whole, and is overwritten.
        let secret = oxyos_auth::Secret::new(std::mem::take(&mut *secret));
        self.0.attempt(account, passcode, secret).map(outcome)
    }

    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }

    fn answer(&mut self) -> Option<(AuthOutcome, u32)> {
        self.0.answer().map(outcome)
    }
}

/// The variable the login program sets on the login screen's session when the last session ended
/// because of a problem (docs/adr/0065-the-session-ending-because-of-a-problem.mdx, decision 2).
pub const LAST_SESSION_ENDED: &str = "OXYOS_LAST_SESSION_ENDED";

/// Whether the login program said the last session ended because of a problem
/// (`OXYOS_LAST_SESSION_ENDED=problem`), taken out of the environment: what the login screen's
/// session starts is never told it.
pub fn last_session_problem() -> bool {
    let told = std::env::var_os(LAST_SESSION_ENDED);
    std::env::remove_var(LAST_SESSION_ENDED);
    told.is_some_and(|told| told == "problem")
}

/// Whether a request's text is the login screen's question, `{"lastSession": {}}`.
fn asks_last_session(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text).is_ok_and(|asked| {
        asked
            .as_object()
            .is_some_and(|asked| asked.len() == 1 && asked.contains_key("lastSession"))
    })
}

/// Answers the login screen's question (`{"lastSession": {}}`, its `shell` grant's, no input
/// needed: it changes nothing) with whether the last session ended because of a problem,
/// `{"lastSession": {"problem": bool}}`; anyone else is refused `notPermitted`. A turn's.
pub fn answer_last_session(scene: &mut Scene, problem: bool) {
    for request in scene.take_system_requests_where(|request| asks_last_session(&request.text)) {
        let shell = scene
            .grants_of(request.client)
            .is_some_and(|grants| grants.shell);
        let answer = if shell {
            json!({ "lastSession": { "problem": problem } })
        } else {
            json!({ "refused": { "reason": "notPermitted" } })
        };
        scene.system_text(request.client, request.serial, &answer.to_string());
    }
}
