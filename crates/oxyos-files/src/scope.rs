//! What a path may name (docs/adr/0028-file-access.mdx).
//!
//! An application gives absolute paths as text. A path is taken only if it has no `.` or `..`
//! (an application never needs them, and refusing them leaves nothing to normalize) and, once
//! its links are resolved, it is in the user's home, in a volume mounted for them under
//! `/media/USER` or `/run/media/USER`, in the application's own folders (its documents,
//! `$XDG_DATA_HOME/ID`, and its cache, `$XDG_CACHE_HOME/ID`) or in its bundle's folder, which
//! it reads and never changes. A Trash is reached only through the Trash's operations, so what it
//! holds always matches its `.trashinfo` files. The home, a volume's root and the application's
//! folders themselves can be listed, never renamed, moved, deleted or put in the Trash.
//!
//! This is the platform's rule for what applications reach, checked on canonical paths so that a
//! link cannot lead out of it. It is not a sandbox: without one (#35, #36) any program running as
//! the user reaches what the user does.

use crate::places::{self, Volume};
use crate::Error;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

/// The application a scope is for: its id and its own folders, canonical.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    pub id: String,
    /// `$XDG_DATA_HOME/ID`: what `expo-file-system` calls its document directory.
    pub documents: PathBuf,
    /// `$XDG_CACHE_HOME/ID`: its cache directory.
    pub cache: PathBuf,
    /// The folder its bundle is in (read, never changed), if it runs from one.
    pub bundle: Option<PathBuf>,
}

/// The user's part of the file system, and the application's.
#[derive(Clone, Debug)]
pub struct Scope {
    home: PathBuf,
    data_home: PathBuf,
    config_home: PathBuf,
    cache_home: PathBuf,
    media: Vec<PathBuf>,
    mountinfo: PathBuf,
    uid: u32,
    application: Option<Application>,
}

/// Which root a path is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Root {
    Home,
    /// A mounted volume, by its mount point.
    Volume(PathBuf),
    /// The application's documents.
    Documents,
    /// The application's cache.
    Cache,
    /// The application's bundle: read, never changed.
    Bundle,
}

/// What a path is reached for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

impl Scope {
    /// The scope of the user running this process: `$HOME`, `$XDG_DATA_HOME` (the Trash's),
    /// `$XDG_CONFIG_HOME` (the user directories') and `$XDG_CACHE_HOME` or their defaults, the
    /// volumes mounted under `/media/USER` and `/run/media/USER` (udisks' places) as the mount
    /// table lists them.
    pub fn for_user() -> io::Result<Scope> {
        let var = |name: &str| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        };
        let home = var("HOME").ok_or_else(|| io::Error::other("HOME is not set"))?;
        let data_home = var("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share"));
        let config_home = var("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        let cache_home = var("XDG_CACHE_HOME").unwrap_or_else(|| home.join(".cache"));
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
        let mut scope = Scope::new(
            &home,
            &data_home,
            &config_home,
            media,
            PathBuf::from("/proc/self/mountinfo"),
            rustix::process::getuid().as_raw(),
        )?;
        scope.cache_home = std::fs::canonicalize(&cache_home).unwrap_or(cache_home);
        Ok(scope)
    }

    /// A scope from its parts (tests give their own directories and mount table); the cache home
    /// is `HOME/.cache`.
    pub fn new(
        home: &Path,
        data_home: &Path,
        config_home: &Path,
        media: Vec<PathBuf>,
        mountinfo: PathBuf,
        uid: u32,
    ) -> io::Result<Scope> {
        let canonical = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.into());
        let home = std::fs::canonicalize(home)?;
        Ok(Scope {
            cache_home: canonical(&home.join(".cache")),
            home,
            data_home: canonical(data_home),
            config_home: canonical(config_home),
            media: media.iter().map(|path| canonical(path)).collect(),
            mountinfo,
            uid,
            application: None,
        })
    }

    /// This scope for the application `id` (a reverse domain name, as its Desktop Entry's), run
    /// from the bundle in `bundle` if any: its documents and cache folders are made, private to
    /// the user, if they are not there.
    pub fn for_application(mut self, id: &str, bundle: Option<&Path>) -> io::Result<Scope> {
        if !oxyos_apps::valid_id(id) {
            return Err(io::Error::other(format!(
                "{id:?} is not an application id (a reverse domain name)"
            )));
        }
        let made = |parent: &Path| -> io::Result<PathBuf> {
            let folder = parent.join(id);
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&folder)
                .map_err(|error| {
                    io::Error::new(
                        error.kind(),
                        format!("the folder {} cannot be made: {error}", folder.display()),
                    )
                })?;
            std::fs::canonicalize(&folder)
        };
        let documents = made(&self.data_home)?;
        let cache = made(&self.cache_home)?;
        let bundle = bundle.map(std::fs::canonicalize).transpose()?;
        self.application = Some(Application {
            id: id.to_string(),
            documents,
            cache,
            bundle,
        });
        Ok(self)
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

    /// The application this scope is for, if any.
    pub fn application(&self) -> Option<&Application> {
        self.application.as_ref()
    }

    /// The volumes mounted for the user now.
    pub fn volumes(&self) -> Vec<Volume> {
        let table = std::fs::read_to_string(&self.mountinfo).unwrap_or_default();
        places::volumes(&self.media, &table)
    }

    /// Every root and its folder: the home, the volumes, the application's folders.
    fn roots(&self, volumes: &[Volume]) -> Vec<(Root, PathBuf)> {
        let mut roots = vec![(Root::Home, self.home.clone())];
        roots.extend(
            volumes
                .iter()
                .map(|volume| (Root::Volume(volume.path.clone()), volume.path.clone())),
        );
        if let Some(application) = &self.application {
            roots.push((Root::Documents, application.documents.clone()));
            roots.push((Root::Cache, application.cache.clone()));
            if let Some(bundle) = &application.bundle {
                roots.push((Root::Bundle, bundle.clone()));
            }
        }
        roots
    }

    /// The root `canonical` is in, if any: the innermost (a volume mounted inside the home is
    /// that volume; the application's folders inside the home are its own).
    pub fn root_of(&self, canonical: &Path, volumes: &[Volume]) -> Option<Root> {
        self.roots(volumes)
            .into_iter()
            .filter(|(_, folder)| canonical.starts_with(folder))
            .max_by_key(|(_, folder)| folder.components().count())
            .map(|(root, _)| root)
    }

    /// Whether `path` is a root's folder itself.
    fn is_root(&self, path: &Path, volumes: &[Volume]) -> bool {
        self.roots(volumes).iter().any(|(_, folder)| folder == path)
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

    /// Whether `canonical` may be reached for `access`: inside a root, never inside a Trash,
    /// never written in the bundle.
    fn allowed(&self, canonical: &Path, access: Access, volumes: &[Volume]) -> Result<(), Error> {
        let Some(root) = self.root_of(canonical, volumes) else {
            return Err(Error::Refused(format!(
                "{} is outside the home, the mounted volumes and the application's folders",
                canonical.display()
            )));
        };
        if self.in_trash(canonical, volumes) {
            return Err(Error::Refused(
                "the Trash is reached through its own operations".into(),
            ));
        }
        if root == Root::Bundle && access == Access::Write {
            return Err(Error::Refused(
                "the application's bundle is read-only".into(),
            ));
        }
        Ok(())
    }

    /// What `path` reaches, every link resolved, whether or not it exists: the canonical path of
    /// its deepest part that exists, and the rest of it after. What reading or writing it opens.
    /// A link that leads nowhere is refused: writing through it would make what it names, which
    /// may be anywhere.
    pub fn reach(&self, path: &str, access: Access) -> Result<PathBuf, Error> {
        let path = absolute(path)?;
        let mut existing = path.to_path_buf();
        let mut rest = Vec::new();
        let base = loop {
            match std::fs::canonicalize(&existing) {
                Ok(canonical) => break canonical,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if std::fs::symlink_metadata(&existing).is_ok() {
                        return Err(Error::Refused(format!(
                            "{} is a link that leads nowhere",
                            existing.display()
                        )));
                    }
                    let (Some(parent), Some(name)) = (existing.parent(), existing.file_name())
                    else {
                        return Err(Error::Refused("the file system's root".into()));
                    };
                    rest.push(name.to_owned());
                    existing = parent.to_path_buf();
                }
                Err(error) => return Err(error.into()),
            }
        };
        let reached = rest.iter().rev().fold(base, |path, name| path.join(name));
        self.allowed(&reached, access, &self.volumes())?;
        Ok(reached)
    }

    /// The item `path` names itself (a link, not what it leads to), whether or not it exists, in
    /// a folder the scope reaches: one to create, delete, rename, move or put in the Trash. Never
    /// a root's folder (the home, a volume's root, the application's folders).
    pub fn entry(&self, path: &str, access: Access) -> Result<PathBuf, Error> {
        let path = absolute(path)?;
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return Err(Error::Refused("the file system's root".into()));
        };
        let parent = self.reach(parent.to_str().unwrap_or_default(), access)?;
        let item = parent.join(name);
        let volumes = self.volumes();
        if self.is_root(&item, &volumes) {
            return Err(Error::Refused(
                "the home, a volume's root and the application's folders stay where they are"
                    .into(),
            ));
        }
        self.allowed(&item, access, &volumes)?;
        Ok(item)
    }

    /// The folder `path` names, links followed, which must exist: one to list, watch, or (for
    /// `Write`) move or put back into.
    pub fn folder(&self, path: &str, access: Access) -> Result<PathBuf, Error> {
        let canonical = std::fs::canonicalize(absolute(path)?)?;
        self.allowed(&canonical, access, &self.volumes())?;
        if !canonical.is_dir() {
            return Err(Error::NotAFolder);
        }
        Ok(canonical)
    }

    /// The item `path` names itself, which must exist, in a folder of the scope: one to rename,
    /// move or put in the Trash. Never a root's folder, nothing in the bundle.
    pub fn item(&self, path: &str) -> Result<PathBuf, Error> {
        let item = self.entry(path, Access::Write)?;
        std::fs::symlink_metadata(&item)?;
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
