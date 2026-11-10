//! An account's PIN, as the service tells it (`oxyos_account::pin`).

use serde::{Deserialize, Serialize};

/// What the login screen and Settings are told of an account's PIN.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PinState {
    None,
    Set,
    /// Too many wrong PINs: the password, or Oxy online, first.
    Locked,
}
