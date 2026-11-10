//! The users' names by id: the running system's from its account database through NSS, each user
//! asked once, and one it has none for asked again at most once a sample (an account added
//! since); or a test's own passwd(5) file's, read once and again only when a process's user is not
//! in it, at most once a sample.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Debug)]
pub struct Users {
    /// A test's own passwd(5) file; none for the system's database.
    path: Option<PathBuf>,
    names: HashMap<u32, String>,
    /// Whether it read the file again for an unknown user in this sample: once a sample, at most.
    reread: bool,
    /// The users the system's database had no name for in this sample.
    unknown: HashSet<u32>,
}

impl Users {
    pub fn new(path: Option<PathBuf>) -> Self {
        let mut users = Self {
            path,
            names: HashMap::new(),
            reread: false,
            unknown: HashSet::new(),
        };
        users.read();
        users
    }

    /// A test's file read whole; the system's database is asked user by user ([`Users::name`]).
    fn read(&mut self) {
        let Some(path) = &self.path else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        self.names = text
            .lines()
            .filter_map(|line| {
                let mut fields = line.split(':');
                let name = fields.next()?;
                let uid = fields.nth(1)?.parse().ok()?;
                Some((uid, name.to_owned()))
            })
            .collect();
    }

    /// A new sample: an unknown user may be asked for again.
    pub fn begin(&mut self) {
        self.reread = false;
        self.unknown.clear();
    }

    /// The name of `uid`, or its number when it has none.
    pub fn name(&mut self, uid: u32) -> String {
        if !self.names.contains_key(&uid) {
            match &self.path {
                Some(_) if !self.reread => {
                    self.reread = true;
                    self.read();
                }
                Some(_) => {}
                None if self.unknown.insert(uid) => {
                    if let Ok(Some(user)) = oxyos_users::user_by_uid(uid) {
                        self.names.insert(uid, user.name);
                    }
                }
                None => {}
            }
        }
        self.names
            .get(&uid)
            .cloned()
            .unwrap_or_else(|| uid.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_systems_users_are_named_through_nss_and_an_unknown_one_by_its_number() {
        let mut users = Users::new(None);
        users.begin();
        assert_eq!(users.name(0), "root");
        let me = rustix::process::getuid().as_raw();
        let mine = oxyos_users::user_by_uid(me).unwrap().unwrap().name;
        // Every user of a sample is named, not only the first one it had not seen.
        assert_eq!(users.name(me), mine);
        assert_eq!(users.name(4_000_000_001), "4000000001");
        users.begin();
        assert_eq!(users.name(4_000_000_001), "4000000001");
        assert_eq!(users.name(0), "root");
    }
}
