//! What a path may name (docs/adr/0028-file-access.mdx).
//!
//! An application gives absolute paths as text. A path is taken only if it has no `.` or `..`
//! (an application never needs them, and refusing them leaves nothing to normalize) and, once
//! its links are resolved, it is in the user's home or in a volume mounted for them under
//! `/media/USER` or `/run/media/USER`. A Trash is reached only through the Trash's operations, so
//! what it holds always matches its `.trashinfo` files. The home and a volume's root can be listed,
//! never renamed, moved or put in the Trash.
//!
//! This is the platform's rule for what applications reach, checked on canonical paths so that a
//! link cannot lead out of it. It is not a sandbox: without one (#35, #36) any program running as
//! the user reaches what the user does.

use crate::places::{self, Volume};
use crate::Error;
use std::io;
use std::path::{Path, PathBuf};

/// The user's part of the file system.
#[derive(Clone, Debug)]
pub struct Scope {
    home: PathBuf,
    data_home: PathBuf,
    config_home: PathBuf,
    media: Vec<PathBuf>,
    mountinfo: PathBuf,
    uid: u32,
}

/// Which root a path is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Root {
    Home,
    /// A mounted volume, by its mount point.
    Volume(PathBuf),
}

impl Scope {
    /// The scope of the user running this process: `$HOME`, `$XDG_DATA_HOME` (the Trash's) and
    /// `$XDG_CONFIG_HOME` (the user directories') or their defaults, the volumes mounted under
    /// `/media/USER` and `/run/media/USER` (udisks' places) as the mount table lists them.
    pub fn for_user() -> io::Result<Scope> {
        let var = |name: &str| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        };
        let home = var("HOME").ok_or_else(|| io::Error::other("HOME is not set"))?;
        let data_home = var("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share"));
        let config_home = var("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        let user = std::env::var("USER")
            .ok()
            .filter(|user| crate::valid_name(user).is_ok())
            .or_else(|| {
                home.file_name()
                    .and_then(|name| name.to_str())
                    .map(String::from)
            })
            .unwrap_or_default();
        let media = if user.is_empty() {
            Vec::new()
        } else {
            vec![
                PathBuf::from("/media").join(&user),
                PathBuf::from("/run/media").join(&user),
            ]
        };
        Scope::new(
            &home,
            &data_home,
            &config_home,
            media,
            PathBuf::from("/proc/self/mountinfo"),
            rustix::process::getuid().as_raw(),
        )
    }

    /// A scope from its parts (tests give their own directories and mount table).
    pub fn new(
        home: &Path,
        data_home: &Path,
        config_home: &Path,
        media: Vec<PathBuf>,
        mountinfo: PathBuf,
        uid: u32,
    ) -> io::Result<Scope> {
        let canonical = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.into());
        Ok(Scope {
            home: std::fs::canonicalize(home)?,
            data_home: canonical(data_home),
            config_home: canonical(config_home),
            media: media.iter().map(|path| canonical(path)).collect(),
            mountinfo,
            uid,
        })
    }

    /// The home, canonical.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Where the user's directories are named (`user-dirs.dirs`).
    pub fn config_home(&self) -> &Path {
        &self.config_home
    }

    /// The home's Trash: `$XDG_DATA_HOME/Trash`.
    pub fn home_trash(&self) -> PathBuf {
        self.data_home.join("Trash")
    }

    /// The user's id (a volume's Trash is named by it).
    pub fn uid(&self) -> u32 {
        self.uid
    }

    /// The mount table this scope reads.
    pub fn mountinfo(&self) -> &Path {
        &self.mountinfo
    }

    /// The volumes mounted for the user now.
    pub fn volumes(&self) -> Vec<Volume> {
        let table = std::fs::read_to_string(&self.mountinfo).unwrap_or_default();
        places::volumes(&self.media, &table)
    }

    /// The root `canonical` is in, if any.
    pub fn root_of(&self, canonical: &Path, volumes: &[Volume]) -> Option<Root> {
        // A volume mounted inside the home is that volume.
        volumes
            .iter()
            .filter(|volume| canonical.starts_with(&volume.path))
            .max_by_key(|volume| volume.path.components().count())
            .map(|volume| Root::Volume(volume.path.clone()))
            .or_else(|| canonical.starts_with(&self.home).then_some(Root::Home))
    }

    /// Whether `canonical` is a Trash or inside one: the home's, or a volume's (`.Trash` and
    /// `.Trash-UID` at its root).
    fn in_trash(&self, canonical: &Path, volumes: &[Volume]) -> bool {
        canonical.starts_with(self.home_trash())
            || volumes.iter().any(|volume| {
                canonical.starts_with(volume.path.join(".Trash"))
                    || canonical.starts_with(volume.path.join(format!(".Trash-{}", self.uid)))
            })
    }

    /// The folder `path` names, links followed: one to list, watch or move into.
    pub fn folder(&self, path: &str) -> Result<PathBuf, Error> {
        let path = absolute(path)?;
        let canonical = std::fs::canonicalize(path)?;
        let volumes = self.volumes();
        if self.root_of(&canonical, &volumes).is_none() {
            return Err(Error::Refused(format!(
                "{} is outside the home and the mounted volumes",
                canonical.display()
            )));
        }
        if self.in_trash(&canonical, &volumes) {
            return Err(Error::Refused(
                "the Trash is reached through its own operations".into(),
            ));
        }
        if !canonical.is_dir() {
            return Err(Error::NotAFolder);
        }
        Ok(canonical)
    }

    /// The item `path` names itself (a link, not what it leads to), in a folder of the scope: one
    /// to rename, move or put in the Trash. Never the home or a volume's root.
    pub fn item(&self, path: &str) -> Result<PathBuf, Error> {
        let path = absolute(path)?;
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return Err(Error::Refused("the file system's root".into()));
        };
        let parent = self.folder(parent.to_str().unwrap_or_default())?;
        let item = parent.join(name);
        std::fs::symlink_metadata(&item)?;
        let volumes = self.volumes();
        if item == self.home || volumes.iter().any(|volume| volume.path == item) {
            return Err(Error::Refused(
                "the home and a volume's root stay where they are".into(),
            ));
        }
        if self.in_trash(&item, &volumes) {
            return Err(Error::Refused(
                "the Trash is reached through its own operations".into(),
            ));
        }
        Ok(item)
    }
}

/// `path` as a path, if it is absolute and has no `.` or `..`.
fn absolute(path: &str) -> Result<&Path, Error> {
    if !path.starts_with('/') {
        return Err(Error::Refused(format!("{path:?} is not an absolute path")));
    }
    if path.contains('\0') {
        return Err(Error::Refused("a path with a NUL".into()));
    }
    if path.split('/').any(|part| part == "." || part == "..") {
        return Err(Error::Refused(format!("{path:?} has `.` or `..`")));
    }
    Ok(Path::new(path))
}
