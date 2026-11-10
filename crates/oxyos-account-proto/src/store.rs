//! An enrolled account's state, as the service tells it (`oxyos_account::store`).

use serde::{Deserialize, Serialize};

/// Whether an enrolled account can sign in: `ready`, or `signInAgain` once Oxy signed it out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    Ready,
    SignInAgain,
}
