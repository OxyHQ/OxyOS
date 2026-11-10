//! The account as Oxy describes it (`oxyos_account::api`).

use serde::{Deserialize, Serialize};

/// The account as Oxy describes it to its own holder.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub avatar: Option<String>,
    /// The account's languages, primary first, as locale strings (Oxy's `languages`); none when
    /// it names none. Files kept before the platform read them have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub languages: Option<Vec<String>>,
}

impl Profile {
    /// What the person is called, as Oxy's own apps call them: the account's display name, else
    /// its username; none when it has neither. The account's user (`oxy-…`) is an identifier of
    /// the machine's and is never a name shown to anyone (ADR 0020, decision 1).
    pub fn name(&self) -> Option<&str> {
        [&self.display_name, &self.username]
            .into_iter()
            .flatten()
            .map(|name| name.trim())
            .find(|name| !name.is_empty())
    }
}
