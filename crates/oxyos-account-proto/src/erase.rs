//! Erasing the computer, as Settings is told it (`oxyos_account::erase`).

use serde::{Deserialize, Serialize};

/// What erasing would do, as Settings shows it before asking for the password.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ErasePreview {
    /// The machine can be erased: [`ERASE_UNIT`] is installed.
    pub available: bool,
    /// The asking account is one of the machine's administrators.
    pub administrator: bool,
    /// Oxy was reached by the asking account's last call (its snapshot's `online`).
    pub online: bool,
    /// Every enrolled account, the asking one first, then by name. For an account that is not an
    /// administrator, which cannot erase, only its own: the others' names and handles are not
    /// another account's to read.
    pub accounts: Vec<ErasedAccount>,
}

/// An account the erase removes from the machine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ErasedAccount {
    /// Its display name, else its handle, else its user's name.
    pub name: String,
    /// Its handle, if it has one.
    pub handle: Option<String>,
    /// It holds a credential and the machine is online: its device session is ended online.
    pub signs_out_online: bool,
}

/// An erase armed: the machine restarts and the recovery erases it. The accounts by name, in the
/// preview's order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Erasing {
    /// Signed out of this device at Oxy (or with nothing left there to end).
    pub signed_out: Vec<String>,
    /// Not reached: its device session ends with the erased credential and expires at Oxy.
    pub not_signed_out: Vec<String>,
}
