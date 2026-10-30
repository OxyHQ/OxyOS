//! The applications an OxyOS session has (docs/adr/0013-launching-applications.mdx): Desktop
//! Entries (freedesktop.org's Desktop Entry Specification) in `oxyos/applications` under the XDG
//! data directories, each named by its application's id. Only OxyOS's own directory is read: the
//! system's applications speak no OxyOS display protocol.
//!
//! System grants (`settings`, docs/adr/0019-settings.mdx) come from one place: the image's policy
//! of system applications ([`Policy`], `/usr/share/oxyos/system-apps.json`), which names each
//! system application's id, its entry, what it runs and its grants. An entry asks for nothing:
//! ownership proves an administrator wrote a file, not that its contents are an administrator's
//! choice (docs/adr/0030-installing-applications.mdx, decision 7). A launch has a system
//! application's grants only if its entry and what it runs are the policy's, by real path, and
//! only an administrator could have written them ([`administrator_written`],
//! [`Launcher::launch`]). A policy's id is never another directory's: a user's entry cannot take
//! a system application's name.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// What a launched application may do beyond an application's own: the system grants the policy
/// gives it ([`Policy`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Grants {
    /// `settings`: it changes the user's preferences (docs/adr/0019-settings.mdx).
    pub settings: bool,
}

impl Grants {
    /// The grants `names` name; a name this system does not know is an error, so a policy that
    /// means something else says so.
    fn named(names: &[String]) -> Result<Self, Error> {
        let mut grants = Self::default();
        for name in names {
            match name.as_str() {
                "settings" if !grants.settings => grants.settings = true,
                "settings" => return Err(Error("the grant \"settings\" is named twice".into())),
                other => return Err(Error(format!("{other:?} is not a system grant"))),
            }
        }
        Ok(grants)
    }
}

/// Where the image keeps its policy of system applications: in `/usr`, which is the image's
/// (read-only on a published install, docs/adr/0023-installing-oxyos.mdx).
pub const POLICY: &str = "/usr/share/oxyos/system-apps.json";

/// The most a policy may be, in bytes.
const POLICY_LIMIT: u64 = 64 * 1024;

/// What a system application runs, as the policy names it: absolute paths, which the launch
/// starts at their real paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Runs {
    /// A native OxyOS client: the program its entry's `Exec` must start.
    Exec(PathBuf),
    /// A Bloom application: the Bloom host and the bundle it runs, whatever the session's are.
    Bloom { host: PathBuf, bundle: PathBuf },
}

/// A system application, as the image's policy describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemApplication {
    /// Its entry: its name, icon and module, read here and nowhere else.
    pub entry: PathBuf,
    pub runs: Runs,
    pub grants: Grants,
}

/// The image's policy of system applications (docs/adr/0030-installing-applications.mdx,
/// decision 7): the one source of system grants. A JSON object:
///
/// ```json
/// { "applications": { "so.oxy.settings": {
///     "entry": "/usr/share/oxyos/applications/so.oxy.settings.desktop",
///     "bloom": { "host": "/usr/lib/oxyos/oxyos-bloom-host", "bundle": "/usr/lib/oxyos/bundle/index.js" },
///     "grants": ["settings"] } } }
/// ```
///
/// with `"exec": "/absolute/program"` in place of `"bloom"` for a native client. It is read
/// strictly: an unknown key, an id named twice, a grant this system does not know, a path that is
/// not absolute and plain, or both or neither of `exec` and `bloom` refuse the whole policy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    applications: BTreeMap<String, SystemApplication>,
}

mod policy_json {
    use serde::Deserialize;
    use std::collections::BTreeMap;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct File {
        #[serde(deserialize_with = "unique")]
        pub applications: BTreeMap<String, Application>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Application {
        pub entry: String,
        pub exec: Option<String>,
        pub bloom: Option<Bloom>,
        pub grants: Vec<String>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Bloom {
        pub host: String,
        pub bundle: String,
    }

    /// A map whose keys are each named once: JSON's objects may repeat a key, and a reader that
    /// keeps the last would read another policy than one that keeps the first.
    fn unique<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<String, Application>, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = BTreeMap<String, Application>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object of applications by id")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut out = BTreeMap::new();
                while let Some((id, application)) = map.next_entry::<String, Application>()? {
                    if out.insert(id.clone(), application).is_some() {
                        return Err(serde::de::Error::custom(format!("{id:?} is named twice")));
                    }
                }
                Ok(out)
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}

/// Whether `path` is absolute with no `.` or `..`: what the trust rule can judge.
fn plain(path: &Path) -> bool {
    use std::path::Component;
    path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
}

impl Policy {
    /// No system application: what a machine without the image's policy has.
    pub fn none() -> Self {
        Self::default()
    }

    /// A policy from its text.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let file: policy_json::File =
            serde_json::from_str(text).map_err(|error| Error(format!("the policy: {error}")))?;
        let path = |id: &str, what: &str, value: String| {
            let path = PathBuf::from(value);
            if plain(&path) {
                Ok(path)
            } else {
                Err(Error(format!(
                    "{id}'s {what} is not an absolute, plain path: {}",
                    path.display()
                )))
            }
        };
        let mut applications = BTreeMap::new();
        for (id, raw) in file.applications {
            if !valid_id(&id) {
                return Err(Error(format!("{id:?} is not an application id")));
            }
            let runs = match (raw.exec, raw.bloom) {
                (Some(program), None) => Runs::Exec(path(&id, "exec", program)?),
                (None, Some(bloom)) => Runs::Bloom {
                    host: path(&id, "host", bloom.host)?,
                    bundle: path(&id, "bundle", bloom.bundle)?,
                },
                _ => {
                    return Err(Error(format!(
                        "{id} must name what it runs: exec or bloom, one of them"
                    )));
                }
            };
            let grants =
                Grants::named(&raw.grants).map_err(|Error(e)| Error(format!("{id}: {e}")))?;
            let entry = path(&id, "entry", raw.entry)?;
            if entry.file_name().and_then(|name| name.to_str()) != Some(&format!("{id}.desktop")) {
                return Err(Error(format!("{id}'s entry is not named {id}.desktop")));
            }
            applications.insert(
                id,
                SystemApplication {
                    entry,
                    runs,
                    grants,
                },
            );
        }
        Ok(Self { applications })
    }

    /// The policy at `path`, judged by `trusted` at its real path and read there. A machine with
    /// no policy has none; one there that anyone but an administrator could have written, or that
    /// is not a valid policy, is an error, and gives nothing.
    pub fn read(path: &Path, trusted: &dyn Fn(&Path) -> bool) -> Result<Self, Error> {
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Self::none()),
            _ => {}
        }
        let real = real_path(path)
            .filter(|real| trusted(real))
            .ok_or_else(|| Error(format!("{} is not an administrator's", path.display())))?;
        use std::io::Read;
        let mut text = String::new();
        std::fs::File::open(&real)
            .and_then(|file| file.take(POLICY_LIMIT + 1).read_to_string(&mut text))
            .map_err(|error| Error(format!("{}: {error}", real.display())))?;
        if text.len() as u64 > POLICY_LIMIT {
            return Err(Error(format!(
                "{} is larger than {POLICY_LIMIT} bytes",
                real.display()
            )));
        }
        Self::parse(&text)
    }

    /// The image's policy ([`POLICY`]), as an administrator wrote it; an error gives none.
    pub fn system() -> Result<Self, Error> {
        Self::read(Path::new(POLICY), &administrator_written)
    }

    /// The system application `id`, if the policy has it.
    pub fn get(&self, id: &str) -> Option<&SystemApplication> {
        self.applications.get(id)
    }

    /// Its system applications, by id.
    pub fn applications(&self) -> impl Iterator<Item = (&str, &SystemApplication)> {
        self.applications.iter().map(|(id, app)| (id.as_str(), app))
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
    /// Whether it is a system application: its entry is the one the image's policy names, and
    /// only an administrator could have written it.
    pub system: bool,
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
/// `languages` (the most wanted first, as `es_ES`, `es`). An entry grants nothing: an
/// `X-OxyOS-Grants` in it is not read (system grants are the policy's, [`Policy`]). None for an
/// entry not to list (`NoDisplay`, `Hidden`, not an application).
pub fn parse(
    id: &str,
    text: &str,
    directory: &Path,
    languages: &[String],
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
        system: false,
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
    let root_only = |status: Status| status.uid == 0 && status.mode & 0o022 == 0;
    plain(path)
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

/// A system application's entry, at the path the policy names: read only if that path is its
/// real path (no link on the way) and `trusted` judges it an administrator's, so what is read is
/// what was judged, and what an administrator chose.
fn system_entry(
    id: &str,
    system: &SystemApplication,
    languages: &[String],
    trusted: &dyn Fn(&Path) -> bool,
) -> Result<Option<Application>, Error> {
    let entry = &system.entry;
    if real_path(entry).as_deref() != Some(entry.as_path()) || !trusted(entry) {
        return Err(Error(format!(
            "{}: the policy's entry is not an administrator's",
            entry.display()
        )));
    }
    let text = std::fs::read_to_string(entry).map_err(|error| Error(error.to_string()))?;
    let directory = entry.parent().unwrap_or(Path::new("/"));
    Ok(
        parse(id, &text, directory, languages)?.map(|application| Application {
            system: true,
            ..application
        }),
    )
}

/// The applications: first the policy's system applications, from the entries it names, then
/// those in `directories`, the first directory's entry for an id winning; sorted by name. A
/// policy's id is its alone: another directory's entry for it is ignored, even when the policy's
/// cannot be read. Entries that cannot be read or are invalid are left out, each said in
/// `skipped`. `trusted` judges a file at its real path ([`administrator_written`] in a session).
pub fn registry(
    directories: &[PathBuf],
    languages: &[String],
    policy: &Policy,
    trusted: &dyn Fn(&Path) -> bool,
) -> (Vec<Application>, Vec<String>) {
    let mut found: BTreeMap<String, Option<Application>> = BTreeMap::new();
    let mut skipped = Vec::new();
    for (id, system) in policy.applications() {
        let application = system_entry(id, system, languages, trusted).unwrap_or_else(|error| {
            skipped.push(format!("{id}: {error}"));
            None
        });
        found.insert(id.to_string(), application);
    }
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
            // An earlier directory's entry (even one not to list) hides a later one's, and a
            // system application's hides every directory's.
            if found.contains_key(id) {
                continue;
            }
            let parsed = std::fs::read_to_string(&path)
                .map_err(|error| Error(error.to_string()))
                .and_then(|text| parse(id, &text, directory, languages));
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
    policy: &Policy,
    trusted: &dyn Fn(&Path) -> bool,
    id: &str,
) -> Option<Application> {
    registry(directories, languages, policy, trusted)
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

/// `program` at its real path, if it is the policy's `expected` there and only an administrator
/// could have written it (`trusted`).
fn administrators(
    program: &Path,
    expected: &Path,
    trusted: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if !program.is_absolute() {
        return None;
    }
    let real = program.canonicalize().ok()?;
    (real == expected.canonicalize().ok()? && trusted(&real)).then_some(real)
}

/// A system application and `runtime` running what its policy names, by real paths, if what is
/// there is an administrator's: the program its entry's `Exec` starts, which must be the policy's,
/// or the policy's Bloom host and bundle, whatever the session's are. A grant follows what runs,
/// not only the entry: an administrator's entry running a program anyone else could change would
/// give that program the grant.
fn pinned(
    application: &Application,
    system: &SystemApplication,
    runtime: &Runtime,
    trusted: &dyn Fn(&Path) -> bool,
) -> Option<(Application, Runtime)> {
    let mut application = application.clone();
    let mut runtime = runtime.clone();
    match (&mut application.launch, &system.runs) {
        (Launch::Exec(words), Runs::Exec(program)) => {
            words[0] = administrators(Path::new(&words[0]), program, trusted)?
                .to_str()?
                .to_string();
        }
        (
            Launch::Bloom { bundle, .. },
            Runs::Bloom {
                host,
                bundle: policy,
            },
        ) => {
            runtime.bloom_host = Some(administrators(host, host, trusted)?);
            *bundle = Some(administrators(policy, policy, trusted)?);
        }
        _ => return None,
    }
    Some((application, runtime))
}

/// The processes a session started for applications (docs/adr/0013-launching-applications.mdx).
pub struct Launcher {
    pub runtime: Runtime,
    directories: Vec<PathBuf>,
    languages: Vec<String>,
    /// Where the image's policy of system applications is ([`POLICY`]; the tests' own in tests).
    /// It is read at each launch, as the registry is.
    policy: PathBuf,
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
            policy: PathBuf::from(POLICY),
            trusted: Box::new(administrator_written),
            children: Vec::new(),
        }
    }

    /// The image's policy of system applications as it is now; one that cannot be judged or read
    /// gives none (the session says why as it starts, [`Policy::system`]).
    fn policy(&self) -> Policy {
        Policy::read(&self.policy, &*self.trusted).unwrap_or_default()
    }

    /// Starts the application `id` if the registry lists it; the pid it runs as. A system
    /// application runs what the policy names, by real path, so what was judged is what runs, and
    /// has the policy's grants; one whose program, host or bundle is not the policy's or not an
    /// administrator's is not started at all. Every other application has none.
    pub fn launch(&mut self, id: &str) -> Result<u32, Error> {
        let policy = self.policy();
        let application = find(
            &self.directories,
            &self.languages,
            &policy,
            &*self.trusted,
            id,
        )
        .ok_or_else(|| Error(format!("no application {id:?}")))?;
        let (mut command, grants) = match policy.get(id).filter(|_| application.system) {
            Some(system) => {
                let (pinned, runtime) =
                    pinned(&application, system, &self.runtime, &*self.trusted).ok_or_else(
                        || {
                            Error(format!(
                                "{id}: what it runs is not what the policy names, or not an administrator's"
                            ))
                        },
                    )?;
                (command(&pinned, &runtime)?, system.grants)
            }
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
