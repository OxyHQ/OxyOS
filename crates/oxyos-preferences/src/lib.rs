//! The user's preferences in an OxyOS session (docs/adr/0019-settings.mdx): their keys, types and
//! defaults, the file that keeps them (`$XDG_CONFIG_HOME/oxyos/preferences`, `key = value`
//! lines) and the validation of a change. The session is the file's one writer.
//!
//! Reading never fails: a missing or unreadable file, an unknown key or a bad value is the
//! default for what it names, so preferences cannot stop a session from starting.
//!
//! The file also keeps the displays' settings (`displays`): each display's mode and scale, and
//! the arrangement of each set of displays. They are the session's to change, through its
//! displays (docs/adr/0018-several-displays.mdx), never through [`Preferences::set`].

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

pub mod displays;
/// The user's folders (xdg-user-dirs).
pub mod user_dirs;
pub use displays::{Arrangement, DisplayMode, DisplaySettings};

/// The longest value a preference takes, in bytes.
pub const MAX_VALUE_BYTES: usize = 64;
/// The longest accent, in bytes: every client is told it in one event (`Preferences`, whose text
/// is one 32-byte chunk), so a change of how they draw arrives whole. Bloom's names are short.
pub const MAX_ACCENT_BYTES: usize = 32;

/// Light, dark, or following the sun where the system's time zone is (ADR 0012).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
    #[default]
    Auto,
}

impl Appearance {
    fn name(self) -> &'static str {
        match self {
            Appearance::Light => "light",
            Appearance::Dark => "dark",
            Appearance::Auto => "auto",
        }
    }
}

/// Where the lock screen shows what notifications say (ADR 0026), as a Mac's *Show previews*:
/// always, or only once the session is unlocked (it then shows how many each application has).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Previews {
    Always,
    #[default]
    Unlocked,
}

impl Previews {
    fn name(self) -> &'static str {
        match self {
            Previews::Always => "always",
            Previews::Unlocked => "unlocked",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preferences {
    pub appearance: Appearance,
    /// One of Bloom's accent colours, by name (a Bloom application that does not know it keeps
    /// its own): lowercase letters, digits and dashes, at most [`MAX_ACCENT_BYTES`].
    pub accent: String,
    /// Every transition takes no time (ADR 0011).
    pub reduce_motion: bool,
    /// Glass is drawn opaque.
    pub reduce_transparency: bool,
    /// The wallpaper: a folder under the wallpapers' directory; none is the session's own.
    pub wallpaper: Option<String>,
    /// Minutes idle before the session locks; 0, never.
    pub lock_after: u32,
    /// The screen reader is on when the session starts (ADR 0014).
    pub screen_reader: bool,
    pub clock_24h: bool,
    /// Do Not Disturb (ADR 0026): notifications are kept in the Notification Center and shown as
    /// no banner, and the screen reader says none.
    pub do_not_disturb: bool,
    /// What the lock screen shows of notifications.
    pub notification_previews: Previews,
    /// Each display's mode and scale, by its identity, the one changed last at the end.
    pub displays: Vec<(String, DisplaySettings)>,
    /// Each set of displays' arrangement, by the set's key, the one changed last at the end.
    pub arrangements: Vec<(String, Arrangement)>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            appearance: Appearance::default(),
            accent: "oxy".to_owned(),
            reduce_motion: false,
            reduce_transparency: false,
            wallpaper: None,
            lock_after: 5,
            screen_reader: false,
            clock_24h: true,
            do_not_disturb: false,
            notification_previews: Previews::default(),
            displays: Vec::new(),
            arrangements: Vec::new(),
        }
    }
}

/// Why a change was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    UnknownKey,
    BadValue,
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Refused::UnknownKey => "no such preference",
            Refused::BadValue => "not a value this preference takes",
        })
    }
}

impl std::error::Error for Refused {}

/// The keys, in the order the file lists them.
pub const KEYS: &[&str] = &[
    "appearance",
    "accent",
    "reduce_motion",
    "reduce_transparency",
    "wallpaper",
    "lock_after",
    "screen_reader",
    "clock_24h",
    "do_not_disturb",
    "notification_previews",
];

fn boolean(value: &str) -> Result<bool, Refused> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(Refused::BadValue),
    }
}

/// A name of at most `max` bytes: lowercase letters, digits and dashes, beginning with a letter.
fn name(value: &str, max: usize) -> Result<String, Refused> {
    let mut chars = value.chars();
    let valid = value.len() <= max
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if valid {
        Ok(value.to_owned())
    } else {
        Err(Refused::BadValue)
    }
}

impl Preferences {
    /// Changes one preference from its text, as the file and the Settings application give it.
    /// Nothing changes if it is refused.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), Refused> {
        match key {
            "appearance" => {
                self.appearance = match value {
                    "light" => Appearance::Light,
                    "dark" => Appearance::Dark,
                    "auto" => Appearance::Auto,
                    _ => return Err(Refused::BadValue),
                }
            }
            "accent" => self.accent = name(value, MAX_ACCENT_BYTES)?,
            "reduce_motion" => self.reduce_motion = boolean(value)?,
            "reduce_transparency" => self.reduce_transparency = boolean(value)?,
            "wallpaper" => {
                self.wallpaper = if value.is_empty() {
                    None
                } else {
                    Some(name(value, MAX_VALUE_BYTES)?)
                }
            }
            "lock_after" => {
                // At most a day: a longer wait is never.
                self.lock_after = value
                    .parse()
                    .ok()
                    .filter(|minutes| *minutes <= 24 * 60)
                    .ok_or(Refused::BadValue)?
            }
            "screen_reader" => self.screen_reader = boolean(value)?,
            "clock_24h" => self.clock_24h = boolean(value)?,
            "do_not_disturb" => self.do_not_disturb = boolean(value)?,
            "notification_previews" => {
                self.notification_previews = match value {
                    "always" => Previews::Always,
                    "unlocked" => Previews::Unlocked,
                    _ => return Err(Refused::BadValue),
                }
            }
            _ => return Err(Refused::UnknownKey),
        }
        Ok(())
    }

    /// One preference's text, as `set` takes it.
    pub fn get(&self, key: &str) -> Option<String> {
        Some(match key {
            "appearance" => self.appearance.name().to_owned(),
            "accent" => self.accent.clone(),
            "reduce_motion" => self.reduce_motion.to_string(),
            "reduce_transparency" => self.reduce_transparency.to_string(),
            "wallpaper" => self.wallpaper.clone().unwrap_or_default(),
            "lock_after" => self.lock_after.to_string(),
            "screen_reader" => self.screen_reader.to_string(),
            "clock_24h" => self.clock_24h.to_string(),
            "do_not_disturb" => self.do_not_disturb.to_string(),
            "notification_previews" => self.notification_previews.name().to_owned(),
            _ => return None,
        })
    }

    /// The mode and scale kept for the display `identity`, if any.
    pub fn display(&self, identity: &str) -> Option<DisplaySettings> {
        self.displays
            .iter()
            .find(|(kept, _)| kept == identity)
            .map(|(_, settings)| *settings)
    }

    /// Keeps a display's mode and scale (the latest of at most [`displays::MAX_DISPLAYS`]).
    pub fn keep_display(&mut self, identity: &str, settings: DisplaySettings) {
        displays::keep(
            &mut self.displays,
            identity.to_string(),
            settings,
            displays::MAX_DISPLAYS,
        );
    }

    /// The arrangement kept for the set of these displays, if any.
    pub fn arrangement<'a>(
        &self,
        identities: impl IntoIterator<Item = &'a str>,
    ) -> Option<&Arrangement> {
        let key = displays::set_key(identities);
        self.arrangements
            .iter()
            .find(|(kept, _)| *kept == key)
            .map(|(_, arrangement)| arrangement)
    }

    /// Keeps the arrangement of its set of displays (the latest of at most
    /// [`displays::MAX_ARRANGEMENTS`]).
    pub fn keep_arrangement(&mut self, arrangement: Arrangement) {
        displays::keep(
            &mut self.arrangements,
            arrangement.key(),
            arrangement,
            displays::MAX_ARRANGEMENTS,
        );
    }

    /// A display's or an arrangement's line of the file; anything else, or a bad one, nothing.
    fn read_display_line(&mut self, key: &str, value: &str) {
        if let Some(identity) = key.strip_prefix("display.") {
            if let (true, Some(settings)) = (
                displays::is_identity(identity),
                DisplaySettings::parse(value),
            ) {
                self.keep_display(identity, settings);
            }
        } else if let Some(set) = key.strip_prefix("arrangement.") {
            if let Some(arrangement) = Arrangement::parse(set, value) {
                self.keep_arrangement(arrangement);
            }
        }
    }

    /// Reads the file's text: each `key = value` line it can, the defaults for the rest. Blank
    /// lines and lines starting with `#` are nothing.
    pub fn parse(text: &str) -> Self {
        let mut preferences = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                let (key, value) = (key.trim(), value.trim());
                if key.starts_with("display.") || key.starts_with("arrangement.") {
                    preferences.read_display_line(key, value);
                } else {
                    // A bad line keeps the default; the next good one is still read.
                    let _ = preferences.set(key, value);
                }
            }
        }
        preferences
    }

    /// The file's text: every key, in order, then the displays and their arrangements.
    pub fn to_text(&self) -> String {
        let mut text = String::from("# OxyOS preferences, written by the session.\n");
        for key in KEYS {
            let value = self.get(key).expect("every key has a value");
            text.push_str(&format!("{key} = {value}\n"));
        }
        for (identity, settings) in &self.displays {
            text.push_str(&format!("display.{identity} = {}\n", settings.text()));
        }
        for (key, arrangement) in &self.arrangements {
            text.push_str(&format!("arrangement.{key} = {}\n", arrangement.text()));
        }
        text
    }

    /// The preferences in `path`, or the defaults where it says nothing usable.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).map_or_else(|_| Self::default(), |text| Self::parse(&text))
    }

    /// Writes the file whole: to a temporary file beside it, then renamed over it, so a reader
    /// (or a crash) never sees half of it.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(directory) = path.parent() {
            std::fs::create_dir_all(directory)?;
        }
        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".new");
        let temporary = PathBuf::from(temporary);
        std::fs::write(&temporary, self.to_text())?;
        std::fs::rename(&temporary, path)
    }
}

/// Where the user's preferences are: `$XDG_CONFIG_HOME/oxyos/preferences`, or
/// `$HOME/.config/oxyos/preferences` without it. None with neither.
pub fn path(config_home: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    let config = match config_home.filter(|p| p.is_absolute()) {
        Some(config) => config.to_owned(),
        None => home?.join(".config"),
    };
    Some(config.join("oxyos").join("preferences"))
}

/// The session's own: from its environment.
pub fn session_path() -> Option<PathBuf> {
    let config_home = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    path(config_home.as_deref(), home.as_deref())
}

#[cfg(test)]
mod tests;
