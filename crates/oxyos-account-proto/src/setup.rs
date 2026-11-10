//! The setup's messages: the appearance asked for, the step under way (`oxyos_account::setup`).

use serde::{Deserialize, Serialize};

/// The appearance the setup assistant asks for, as the preferences name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Appearance {
    Light,
    Dark,
    Auto,
}

impl Appearance {
    pub fn name(self) -> &'static str {
        match self {
            Appearance::Light => "light",
            Appearance::Dark => "dark",
            Appearance::Auto => "auto",
        }
    }
}

/// A setup's step, reported as it starts (see the module's doc for what each is).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    Securing,
    Verifying,
    Preparing,
    SigningIn,
    Finishing,
    /// Oxy confirms the platform credential the machine now holds (Oxy ADR 0058 R8).
    Confirming,
}
