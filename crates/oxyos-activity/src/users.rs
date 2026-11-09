//! The users' names by id, from `/etc/passwd`, read once and again only when a process's user is
//! not in it (an account added since).

use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug)]
pub struct Users {
    path: PathBuf,
    names: HashMap<u32, String>,
    /// Whether it was read again for an unknown user in this sample: once a sample, at most.
    reread: bool,
}

impl Users {
    pub fn new(path: PathBuf) -> Self {
        let mut users = Self {
            path,
            names: HashMap::new(),
            reread: false,
        };
        users.read();
        users
    }

    fn read(&mut self) {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
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

    /// A new sample: an unknown user may read the file again.
    pub fn begin(&mut self) {
        self.reread = false;
    }

    /// The name of `uid`, or its number when it has none.
    pub fn name(&mut self, uid: u32) -> String {
        if !self.names.contains_key(&uid) && !self.reread {
            self.reread = true;
            self.read();
        }
        self.names
            .get(&uid)
            .cloned()
            .unwrap_or_else(|| uid.to_string())
    }
}
