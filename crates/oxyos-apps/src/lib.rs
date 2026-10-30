//! The applications an OxyOS session has (docs/adr/0013-launching-applications.mdx): Desktop
//! Entries (freedesktop.org's Desktop Entry Specification) in `oxyos/applications` under the XDG
//! data directories, each named by its application's id. Only OxyOS's own directory is read: the
//! system's applications speak no OxyOS display protocol.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

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
/// `languages` (the most wanted first, as `es_ES`, `es`). None for an entry not to list
/// (`NoDisplay`, `Hidden`, not an application).
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
    }))
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

/// The applications in `directories`, the first directory's entry for an id winning, sorted by
/// name; entries that cannot be read or are invalid are left out, each said in `skipped`.
pub fn registry(directories: &[PathBuf], languages: &[String]) -> (Vec<Application>, Vec<String>) {
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
pub fn find(directories: &[PathBuf], languages: &[String], id: &str) -> Option<Application> {
    registry(directories, languages)
        .0
        .into_iter()
        .find(|application| application.id == id)
}

#[cfg(test)]
mod tests;
