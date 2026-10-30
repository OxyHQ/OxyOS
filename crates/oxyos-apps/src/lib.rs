//! The applications an OxyOS session has (docs/adr/0013-launching-applications.mdx): Desktop
//! Entries (freedesktop.org's Desktop Entry Specification) in `oxyos/applications` under the XDG
//! data directories, each named by its application's id. Only OxyOS's own directory is read: the
//! system's applications speak no OxyOS display protocol.
//!
//! An entry may ask for grants (`X-OxyOS-Grants`, docs/adr/0019-settings.mdx). They are honored
//! only for an entry only an administrator could have written ([`administrator_written`]), and
//! a launch keeps them only if what it runs is an administrator's too ([`Launcher::launch`]).

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// What a launched application may do beyond an application's own, asked for by its entry
/// (`X-OxyOS-Grants`, a `;`-separated list).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Grants {
    /// `settings`: it changes the user's preferences (docs/adr/0019-settings.mdx).
    pub settings: bool,
}

impl Grants {
    /// The grants a list names; names this system does not know grant nothing.
    fn named(list: &str) -> Self {
        let mut grants = Self::default();
        for name in list.split(';').map(str::trim) {
            if name == "settings" {
                grants.settings = true;
            }
        }
        grants
    }
}

/// How an application is started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Launch {
    /// A native OxyOS client: the program and its arguments, from `Exec`.
    Exec(Vec<String>),
    /// A Bloom application, run by the session's Bloom host: its module, and its bundle when the
    /// entry names one (relative to the entry; else the session's).
    Bloom {
        module: String,
        bundle: Option<PathBuf>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    /// Its id: the entry's file name without `.desktop` (`so.oxy.allo`).
    pub id: String,
    /// Its name in the session's language, else its own.
    pub name: String,
    /// Its icon's file, if it has one (`Icon`, absolute or relative to the entry).
    pub icon: Option<PathBuf>,
    pub launch: Launch,
    /// Its place in the dock, from 1, when it is kept there (`X-OxyOS-Pinned`).
    pub pinned: Option<u32>,
    /// What its entry asks for (`X-OxyOS-Grants`), if only an administrator could have written
    /// the entry; nothing otherwise.
    pub grants: Grants,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// An application id: a reverse domain name, as the Desktop Entry Specification recommends,
/// of ASCII letters, digits, `.`, `-` and `_`, at most 128 bytes.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// The `[Desktop Entry]` group's keys.
fn keys(text: &str) -> BTreeMap<String, String> {
    let mut group = None;
    let mut keys = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            group = Some(&line[1..line.len() - 1]);
            continue;
        }
        if group != Some("Desktop Entry") {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            keys.entry(key.trim().to_string())
                .or_insert_with(|| unescape(value.trim()));
        }
    }
    keys
}

/// A string value's escapes (`\s`, `\n`, `\t`, `\r`, `\\`).
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// `Exec`'s program and arguments: quoted arguments as the specification quotes them. Field
/// codes (`%f`, `%u`, ...) are refused: OxyOS opens no files with applications yet.
fn exec(value: &str) -> Result<Vec<String>, Error> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut started = false;
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if quoted => {
                if let Some(next) = chars.next() {
                    word.push(next);
                }
            }
            ' ' if !quoted => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            '%' => {
                if chars.peek() == Some(&'%') {
                    chars.next();
                    word.push('%');
                    started = true;
                } else {
                    return Err(Error("Exec has field codes".into()));
                }
            }
            c => {
                word.push(c);
                started = true;
            }
        }
    }
    if quoted {
        return Err(Error("Exec has an unclosed quote".into()));
    }
    if started {
        words.push(word);
    }
    if words.is_empty() {
        return Err(Error("Exec is empty".into()));
    }
    Ok(words)
}

/// The application an entry describes: `id`, the entry's `text`, in `directory`, named in
/// `languages` (the most wanted first, as `es_ES`, `es`). Its grants are the entry's only if
/// `trusted`: only an administrator could have written it. None for an entry not to list
/// (`NoDisplay`, `Hidden`, not an application).
pub fn parse(
    id: &str,
    text: &str,
    directory: &Path,
    languages: &[String],
    trusted: bool,
) -> Result<Option<Application>, Error> {
    if !valid_id(id) {
        return Err(Error(format!("{id:?} is not an application id")));
    }
    let keys = keys(text);
    let flag = |key: &str| keys.get(key).is_some_and(|value| value == "true");
    let pinned = match keys.get("X-OxyOS-Pinned") {
        None => None,
        Some(place) => Some(
            place
                .parse::<u32>()
                .ok()
                .filter(|&place| place > 0)
                .ok_or_else(|| Error(format!("{id}'s X-OxyOS-Pinned is not a place from 1")))?,
        ),
    };
    if keys.get("Type").map(String::as_str) != Some("Application") {
        return Ok(None);
    }
    if flag("NoDisplay") || flag("Hidden") {
        return Ok(None);
    }
    let name = languages
        .iter()
        .find_map(|language| keys.get(&format!("Name[{language}]")))
        .or_else(|| keys.get("Name"))
        .ok_or_else(|| Error(format!("{id} has no Name")))?
        .clone();
    let relative = |value: &str| {
        let path = Path::new(value);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            directory.join(path)
        }
    };
    let launch = match (keys.get("X-OxyOS-Bloom-Module"), keys.get("Exec")) {
        (Some(module), _) => Launch::Bloom {
            module: module.clone(),
            bundle: keys.get("X-OxyOS-Bloom-Bundle").map(|b| relative(b)),
        },
        (None, Some(value)) => Launch::Exec(exec(value)?),
        (None, None) => return Err(Error(format!("{id} has neither Exec nor a Bloom module"))),
    };
    Ok(Some(Application {
        id: id.to_string(),
        name,
        icon: keys.get("Icon").map(|icon| relative(icon)),
        launch,
        pinned,
        grants: match keys.get("X-OxyOS-Grants") {
            Some(list) if trusted => Grants::named(list),
            _ => Grants::default(),
        },
    }))
}

/// What the trust rule asks of a file or directory, as `lstat` gives it (a link is not followed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub uid: u32,
    /// Its permission bits.
    pub mode: u32,
    pub kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Directory,
    Link,
    Other,
}

/// A file or directory as `lstat` gives it.
pub fn lstat(path: &Path) -> Option<Status> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path).ok()?;
    let kind = metadata.file_type();
    Some(Status {
        uid: metadata.uid(),
        mode: metadata.mode() & 0o7777,
        kind: if kind.is_symlink() {
            Kind::Link
        } else if kind.is_file() {
            Kind::File
        } else if kind.is_dir() {
            Kind::Directory
        } else {
            Kind::Other
        },
    })
}

/// Whether only an administrator could have written the file at `path` (absolute, with no `.`
/// or `..`), as `stat` describes what is there: it is a file, not a link, and it and every
/// directory from it up to `/` are owned by root and writable by no group and no one else.
/// Anyone who could change any of them could change what the path holds.
pub fn trusted_by(path: &Path, stat: &dyn Fn(&Path) -> Option<Status>) -> bool {
    use std::path::Component;
    let plain = path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::RootDir | Component::Normal(_)));
    let root_only = |status: Status| status.uid == 0 && status.mode & 0o022 == 0;
    plain
        && stat(path).is_some_and(|file| file.kind == Kind::File && root_only(file))
        && path.ancestors().skip(1).all(|directory| {
            stat(directory).is_some_and(|d| d.kind == Kind::Directory && root_only(d))
        })
}

/// Whether only an administrator could have written the file at `path`: [`trusted_by`] what is
/// on the filesystem. `path` is taken as given; [`real_path`] resolves the links on the way.
pub fn administrator_written(path: &Path) -> bool {
    trusted_by(path, &lstat)
}

/// `path` with every link of its directories resolved, its own name kept: a link there stays
/// one, and the trust rule refuses it.
pub fn real_path(path: &Path) -> Option<PathBuf> {
    let directory = path.parent()?.canonicalize().ok()?;
    Some(directory.join(path.file_name()?))
}

/// The directories applications are looked for in, the first wanted first: `oxyos/applications`
/// under `$XDG_DATA_HOME` (`~/.local/share`), then under each of `$XDG_DATA_DIRS`
/// (`/usr/local/share:/usr/share`).
pub fn directories() -> Vec<PathBuf> {
    let var = |name| std::env::var_os(name).filter(|value| !value.is_empty());
    let home = var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| var("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    let dirs = var("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    home.into_iter()
        .chain(std::env::split_paths(&dirs))
        .map(|dir| dir.join("oxyos/applications"))
        .collect()
}

/// The session's languages, the most wanted first, from `LC_ALL`, `LC_MESSAGES` or `LANG`
/// (`es_ES.UTF-8` gives `es_ES`, `es`).
pub fn languages() -> Vec<String> {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
        .unwrap_or_default();
    languages_of(&locale)
}

/// The languages of a POSIX locale name, the most wanted first.
pub fn languages_of(locale: &str) -> Vec<String> {
    let base = locale.split(['.', '@']).next().unwrap_or_default();
    let mut out = Vec::new();
    if !base.is_empty() && base != "C" && base != "POSIX" {
        out.push(base.to_string());
        if let Some((language, _)) = base.split_once('_') {
            out.push(language.to_string());
        }
    }
    out
}

/// An entry's text, and whether only an administrator could have written it, as `trusted` judges
/// the entry at its real path. A trusted entry is read at the path judged, which only root can
/// change, so what is read is what was judged.
fn read_entry(path: &Path, trusted: &dyn Fn(&Path) -> bool) -> std::io::Result<(String, bool)> {
    if let Some(real) = real_path(path).filter(|real| trusted(real)) {
        return Ok((std::fs::read_to_string(real)?, true));
    }
    Ok((std::fs::read_to_string(path)?, false))
}

/// The applications in `directories`, the first directory's entry for an id winning, sorted by
/// name; entries that cannot be read or are invalid are left out, each said in `skipped`. An
/// entry's grants are honored if `trusted` says only an administrator could have written it
/// (given its real path; [`administrator_written`] in a session).
pub fn registry(
    directories: &[PathBuf],
    languages: &[String],
    trusted: &dyn Fn(&Path) -> bool,
) -> (Vec<Application>, Vec<String>) {
    let mut found: BTreeMap<String, Option<Application>> = BTreeMap::new();
    let mut skipped = Vec::new();
    for directory in directories {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for path in paths {
            let Some(id) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".desktop"))
            else {
                continue;
            };
            // An earlier directory's entry (even one not to list) hides a later one's.
            if found.contains_key(id) {
                continue;
            }
            let parsed = read_entry(&path, trusted)
                .map_err(|error| Error(error.to_string()))
                .and_then(|(text, trusted)| parse(id, &text, directory, languages, trusted));
            match parsed {
                Ok(application) => {
                    found.insert(id.to_string(), application);
                }
                Err(error) => skipped.push(format!("{}: {error}", path.display())),
            }
        }
    }
    let mut applications: Vec<Application> = found.into_values().flatten().collect();
    applications.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    (applications, skipped)
}

/// The application with `id` in `directories`, if any lists it.
pub fn find(
    directories: &[PathBuf],
    languages: &[String],
    trusted: &dyn Fn(&Path) -> bool,
    id: &str,
) -> Option<Application> {
    registry(directories, languages, trusted)
        .0
        .into_iter()
        .find(|application| application.id == id)
}

#[cfg(test)]
mod tests;

/// What a session starts applications with: the compositor's socket, and for Bloom
/// applications the session's Bloom host, its bundle and their windows' size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Runtime {
    pub socket: PathBuf,
    pub bloom_host: Option<PathBuf>,
    pub bloom_bundle: Option<PathBuf>,
    pub window: (u32, u32),
    /// More of the applications' environment (a development compositor's report files).
    pub env: Vec<(std::ffi::OsString, std::ffi::OsString)>,
}

/// How `application` is started in `runtime`: the program, its arguments and its environment:
/// the compositor's socket, its id and name (its window's, until launch records give them), never
/// another display's.
pub fn command(
    application: &Application,
    runtime: &Runtime,
) -> Result<std::process::Command, Error> {
    let mut command = match &application.launch {
        Launch::Exec(words) => {
            let mut command = std::process::Command::new(&words[0]);
            command.args(&words[1..]);
            command
        }
        Launch::Bloom { module, bundle } => {
            let host = runtime
                .bloom_host
                .as_ref()
                .ok_or_else(|| Error("no Bloom host to run Bloom applications".into()))?;
            let bundle = bundle
                .as_ref()
                .or(runtime.bloom_bundle.as_ref())
                .ok_or_else(|| {
                    Error(format!("{} has no bundle, nor the session", application.id))
                })?;
            let mut command = std::process::Command::new(host);
            command
                .arg(bundle)
                .arg(module)
                .args([
                    "--window",
                    &runtime.window.0.to_string(),
                    &runtime.window.1.to_string(),
                ])
                .env("OXYOS_SCRIPT_URL", format!("file://{}", bundle.display()));
            command
        }
    };
    command
        .envs(runtime.env.iter().map(|(name, value)| (name, value)))
        .env("OXYOS_COMPOSITOR_SOCKET", &runtime.socket)
        .env("OXYOS_APP_ID", &application.id)
        .env("OXYOS_APP_TITLE", &application.name)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY");
    Ok(command)
}

/// A process a session started, while it runs: its application's id and what it may do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launched {
    pub id: String,
    pub grants: Grants,
}

/// The program an application with grants runs, by its real path, if only an administrator could
/// have written it (`trusted`): it is given absolute, as an entry with grants must give it. A
/// grant follows the program, not only the entry: an administrator's entry running a program
/// anyone else could change would give that program the grant.
fn administrators(program: &Path, trusted: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    if !program.is_absolute() {
        return None;
    }
    let real = program.canonicalize().ok()?;
    trusted(&real).then_some(real)
}

/// `application` and `runtime` running only what an administrator wrote, by real paths, if they
/// do: an `Exec`'s program, or the Bloom host and the bundle it runs.
fn pinned(
    application: &Application,
    runtime: &Runtime,
    trusted: &dyn Fn(&Path) -> bool,
) -> Option<(Application, Runtime)> {
    let mut application = application.clone();
    let mut runtime = runtime.clone();
    match &mut application.launch {
        Launch::Exec(words) => {
            words[0] = administrators(Path::new(&words[0]), trusted)?
                .to_str()?
                .to_string();
        }
        Launch::Bloom { bundle, .. } => {
            runtime.bloom_host = Some(administrators(runtime.bloom_host.as_deref()?, trusted)?);
            let given = bundle.clone().or(runtime.bloom_bundle.clone())?;
            *bundle = Some(administrators(&given, trusted)?);
        }
    }
    Some((application, runtime))
}

/// The processes a session started for applications (docs/adr/0013-launching-applications.mdx).
pub struct Launcher {
    pub runtime: Runtime,
    directories: Vec<PathBuf>,
    languages: Vec<String>,
    /// Whether only an administrator could have written a file at a real path
    /// ([`administrator_written`]; the tests' own rule in tests).
    trusted: Box<dyn Fn(&Path) -> bool>,
    /// Each process started, and what it was launched as, until it exits.
    children: Vec<(std::process::Child, Launched)>,
}

impl Launcher {
    pub fn new(runtime: Runtime, directories: Vec<PathBuf>, languages: Vec<String>) -> Self {
        Self {
            runtime,
            directories,
            languages,
            trusted: Box::new(administrator_written),
            children: Vec::new(),
        }
    }

    /// Starts the application `id` if the registry lists it; the pid it runs as. It has its
    /// entry's grants only if what it runs is an administrator's too, and then runs it by its
    /// real path, so what was judged is what runs.
    pub fn launch(&mut self, id: &str) -> Result<u32, Error> {
        let application = find(&self.directories, &self.languages, &*self.trusted, id)
            .ok_or_else(|| Error(format!("no application {id:?}")))?;
        let granted = (application.grants != Grants::default())
            .then(|| pinned(&application, &self.runtime, &*self.trusted))
            .flatten();
        let (mut command, grants) = match &granted {
            Some((pinned, runtime)) => (command(pinned, runtime)?, pinned.grants),
            None => (command(&application, &self.runtime)?, Grants::default()),
        };
        let child = command
            .spawn()
            .map_err(|error| Error(format!("{id}: {error}")))?;
        let pid = child.id();
        self.children.push((
            child,
            Launched {
                id: application.id,
                grants,
            },
        ));
        Ok(pid)
    }

    /// Forgets the processes that exited (they are waited for, so none lingers); the id of each.
    pub fn reap(&mut self) -> Vec<String> {
        let mut exited = Vec::new();
        self.children
            .retain_mut(|(child, launched)| match child.try_wait() {
                Ok(None) => true,
                _ => {
                    exited.push(launched.id.clone());
                    false
                }
            });
        exited
    }

    /// What a running process it started was launched as, by pid.
    pub fn application_of(&self, pid: u32) -> Option<&Launched> {
        self.children
            .iter()
            .find(|(child, _)| child.id() == pid)
            .map(|(_, launched)| launched)
    }

    /// Stops every process it started: asked first (SIGTERM), then killed after `grace`.
    pub fn end(&mut self, grace: std::time::Duration) {
        for (child, _) in &self.children {
            if let Some(pid) = rustix::process::Pid::from_raw(child.id() as i32) {
                let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
            }
        }
        let deadline = std::time::Instant::now() + grace;
        while !self.children.is_empty() && std::time::Instant::now() < deadline {
            self.reap();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        for (child, _) in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.children.clear();
    }
}
