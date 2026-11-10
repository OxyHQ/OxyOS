//! The machine's accounts as the install service needs them: a peer's uid gives its user's name
//! (its records' directory) and group (who may read them). Every person is an Oxy account and a
//! Unix user (ADR 0020).

use std::path::{Path, PathBuf};

/// A Unix user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub uid: u32,
    pub name: String,
    pub gid: u32,
}

/// Who a uid is.
pub trait Accounts {
    fn account(&self, uid: u32) -> Option<Account>;
}

/// The machine's accounts, read at each question, as the system's user database changes while
/// the service runs: the running system's through NSS (`oxyos-users`), or those of a passwd(5)
/// file of another root's (a scenario's own, `--passwd FILE`).
#[derive(Clone, Debug)]
pub enum Passwd {
    System,
    File(PathBuf),
}

impl Passwd {
    /// The running system's accounts, through NSS.
    pub fn system() -> Self {
        Self::System
    }

    /// The accounts of the passwd(5) file at `path`.
    pub fn new(path: &Path) -> Self {
        Self::File(path.to_path_buf())
    }
}

/// A user name records may be kept under: what `useradd` makes on Debian.
pub fn valid_user_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-.".contains(&c))
        && name != "."
        && name != ".."
}

impl Accounts for Passwd {
    fn account(&self, uid: u32) -> Option<Account> {
        let (name, gid) = match self {
            Passwd::System => {
                let user = oxyos_users::user_by_uid(uid).ok()??;
                (user.name, user.gid)
            }
            Passwd::File(path) => file_entry(path, uid)?,
        };
        valid_user_name(&name).then_some(Account { uid, name, gid })
    }
}

/// The name and group of `uid`'s first entry in the passwd(5) file at `path` whose name records
/// may be kept under.
fn file_entry(path: &Path, uid: u32) -> Option<(String, u32)> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines().find_map(|line| {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() < 7 {
            return None;
        }
        let name = fields[0];
        let user_id: u32 = fields[2].parse().ok()?;
        let group_id: u32 = fields[3].parse().ok()?;
        (user_id == uid && valid_user_name(name)).then(|| (name.to_string(), group_id))
    })
}

/// Accounts given by the tests.
#[derive(Clone, Debug, Default)]
pub struct Fixed(pub Vec<Account>);

impl Accounts for Fixed {
    fn account(&self, uid: u32) -> Option<Account> {
        self.0.iter().find(|a| a.uid == uid).cloned()
    }
}
