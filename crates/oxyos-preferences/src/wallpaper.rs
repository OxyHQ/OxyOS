//! The person's wallpapers (docs/adr/0048-wallpapers.mdx): what a choice is, and where each is
//! kept. A choice is one value:
//!
//! ```text
//!                                         the catalog's default
//! oxyos                                   a system wallpaper, Automatic
//! horizon/night                           a system wallpaper's variant
//! color:1f4fd8                            a solid colour
//! photo:fit:000000:/home/ana/Pictures/a.jpg          a photo, how it fits, the colour behind
//! folder:fill:000000:30:/home/ana/Pictures/Trips     a folder's photos in turn, every 30 minutes
//! shuffle:gradients:login                 a category's wallpapers in turn, at each login
//! ```
//!
//! It is kept for each display, and for each desktop of a display while *Show on all desktops* is
//! off, as lines of the preferences file the session writes:
//!
//! ```text
//! wallpaper = horizon
//! wallpaper_all_desktops = false
//! wallpaper.DEL-A0B4-7Y9K2N3 = color:1f4fd8
//! wallpaper.BOE-0A4B-00000001.2 = gradients-ocean
//! ```
//!
//! The first is the default, which a display with no choice of its own shows; then a display's own,
//! by its identity (docs/adr/0018-several-displays.mdx); then a desktop's on a display, counted
//! from 1. A display shows on a desktop that desktop's choice, else its own, else the default.

use crate::displays::{is_identity, keep};
use crate::Refused;
use std::fmt;

/// The longest choice, in bytes.
pub const MAX_CHOICE_BYTES: usize = 1100;
/// The longest path a choice names, in bytes.
pub const MAX_PATH_BYTES: usize = 1024;
/// The most minutes between a shuffle's pictures: a week.
pub const MAX_EVERY_MINUTES: u32 = 7 * 24 * 60;
/// The most displays whose wallpapers are kept, and desktops' choices kept: past them, the one
/// chosen longest ago goes.
pub const MAX_DISPLAYS: usize = 16;
pub const MAX_DESKTOP_CHOICES: usize = 16 * 16;
/// The most desktops a choice is kept for (the scene's workspaces).
pub const MAX_DESKTOP: u32 = 16;

/// How a photo fills its display, as a Mac's: Fill Screen, Fit to Screen, Stretch to Fill
/// Screen, Centre Image, Tile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fit {
    #[default]
    Fill,
    Fit,
    Stretch,
    Center,
    Tile,
}

impl Fit {
    pub fn name(self) -> &'static str {
        match self {
            Fit::Fill => "fill",
            Fit::Fit => "fit",
            Fit::Stretch => "stretch",
            Fit::Center => "center",
            Fit::Tile => "tile",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "fill" => Fit::Fill,
            "fit" => Fit::Fit,
            "stretch" => Fit::Stretch,
            "center" => Fit::Center,
            "tile" => Fit::Tile,
            _ => return None,
        })
    }
}

/// How often a shuffle shows another picture: each time the session starts, or every so many
/// minutes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Every {
    Login,
    Minutes(u32),
}

impl Every {
    pub fn text(self) -> String {
        match self {
            Every::Login => "login".to_owned(),
            Every::Minutes(minutes) => minutes.to_string(),
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        if text == "login" {
            return Some(Every::Login);
        }
        // Digits alone: no sign, no space.
        if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        text.parse()
            .ok()
            .filter(|minutes| (1..=MAX_EVERY_MINUTES).contains(minutes))
            .map(Every::Minutes)
    }
}

/// A colour, `rrggbb`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub [u8; 3]);

impl Rgb {
    pub fn parse(text: &str) -> Option<Self> {
        if text.len() != 6 || !text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&text[i..i + 2], 16).ok();
        Some(Rgb([byte(0)?, byte(2)?, byte(4)?]))
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [r, g, b] = self.0;
        write!(f, "{r:02x}{g:02x}{b:02x}")
    }
}

/// What the person chose to show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Choice {
    /// The catalog's default.
    #[default]
    Default,
    /// A system wallpaper, by its folder's name; a fixed variant, or *Automatic*.
    System {
        name: String,
        variant: Option<String>,
    },
    /// A solid colour.
    Color(Rgb),
    /// One of the person's photos.
    Photo { fit: Fit, margin: Rgb, path: String },
    /// The photos of one of the person's folders, in turn.
    Folder {
        fit: Fit,
        margin: Rgb,
        every: Every,
        path: String,
    },
    /// A category's wallpapers in turn (`colors`: the catalog's colours).
    Shuffle { category: String, every: Every },
}

/// A name: lowercase letters, digits and dashes, beginning with a letter, at most 64 bytes.
pub fn is_name(text: &str) -> bool {
    let mut chars = text.chars();
    text.len() <= 64
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A path a choice may name: absolute, at most [`MAX_PATH_BYTES`], no control character, no `.`
/// or `..` component and no `/` at its end (nor an empty component).
pub fn is_path(text: &str) -> bool {
    text.len() <= MAX_PATH_BYTES
        && text.len() > 1
        && text.starts_with('/')
        && !text.chars().any(char::is_control)
        && text[1..]
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

impl Choice {
    /// A choice from its text; refused unless it is one of the forms above.
    pub fn parse(text: &str) -> Result<Self, Refused> {
        Self::read(text).ok_or(Refused::BadValue)
    }

    fn read(text: &str) -> Option<Self> {
        if text.len() > MAX_CHOICE_BYTES {
            return None;
        }
        if text.is_empty() {
            return Some(Choice::Default);
        }
        let (kind, rest) = match text.split_once(':') {
            Some(split) => split,
            None => {
                let (name, variant) = match text.split_once('/') {
                    Some((name, variant)) => (name, Some(variant)),
                    None => (text, None),
                };
                if !is_name(name) || variant.is_some_and(|v| !is_name(v)) {
                    return None;
                }
                return Some(Choice::System {
                    name: name.to_owned(),
                    variant: variant.map(str::to_owned),
                });
            }
        };
        match kind {
            "color" => Some(Choice::Color(Rgb::parse(rest)?)),
            "photo" => {
                let mut parts = rest.splitn(3, ':');
                let fit = Fit::parse(parts.next()?)?;
                let margin = Rgb::parse(parts.next()?)?;
                let path = parts.next()?;
                is_path(path).then(|| Choice::Photo {
                    fit,
                    margin,
                    path: path.to_owned(),
                })
            }
            "folder" => {
                let mut parts = rest.splitn(4, ':');
                let fit = Fit::parse(parts.next()?)?;
                let margin = Rgb::parse(parts.next()?)?;
                let every = Every::parse(parts.next()?)?;
                let path = parts.next()?;
                is_path(path).then(|| Choice::Folder {
                    fit,
                    margin,
                    every,
                    path: path.to_owned(),
                })
            }
            "shuffle" => {
                let (category, every) = rest.split_once(':')?;
                (is_name(category)).then_some(())?;
                Some(Choice::Shuffle {
                    category: category.to_owned(),
                    every: Every::parse(every)?,
                })
            }
            _ => None,
        }
    }

    /// Its text, as [`Choice::parse`] reads it.
    pub fn text(&self) -> String {
        match self {
            Choice::Default => String::new(),
            Choice::System {
                name,
                variant: None,
            } => name.clone(),
            Choice::System {
                name,
                variant: Some(variant),
            } => format!("{name}/{variant}"),
            Choice::Color(color) => format!("color:{color}"),
            Choice::Photo { fit, margin, path } => {
                format!("photo:{}:{margin}:{path}", fit.name())
            }
            Choice::Folder {
                fit,
                margin,
                every,
                path,
            } => format!("folder:{}:{margin}:{}:{path}", fit.name(), every.text()),
            Choice::Shuffle { category, every } => {
                format!("shuffle:{category}:{}", every.text())
            }
        }
    }
}

/// Where a choice is made or shown: a display, by its identity, whether it is the primary, and
/// the desktop shown on it (from 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place<'a> {
    pub display: &'a str,
    pub primary: bool,
    pub desktop: u32,
}

/// The wallpapers kept: the default, each display's own, each desktop's on a display, and
/// whether a display's shows on all its desktops.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wallpapers {
    pub default: Choice,
    pub all_desktops: bool,
    /// Each display's own, by its identity, the one chosen last at the end.
    pub displays: Vec<(String, Choice)>,
    /// Each desktop's on a display (`IDENTITY.N`), the one chosen last at the end.
    pub desktops: Vec<(String, Choice)>,
}

impl Default for Wallpapers {
    fn default() -> Self {
        Self {
            default: Choice::Default,
            all_desktops: true,
            displays: Vec::new(),
            desktops: Vec::new(),
        }
    }
}

fn desktop_key(display: &str, desktop: u32) -> String {
    format!("{display}.{desktop}")
}

impl Wallpapers {
    /// What `display` shows on `desktop`: that desktop's choice, else its own, else the default.
    pub fn shown(&self, display: &str, desktop: u32) -> &Choice {
        let key = desktop_key(display, desktop);
        self.desktops
            .iter()
            .find(|(k, _)| *k == key)
            .or_else(|| self.displays.iter().find(|(k, _)| k == display))
            .map_or(&self.default, |(_, choice)| choice)
    }

    fn own(&self, display: &str) -> Option<&Choice> {
        self.displays
            .iter()
            .find(|(k, _)| k == display)
            .map(|(_, c)| c)
    }

    /// A display's own choice, or none: the default.
    fn set_own(&mut self, display: &str, choice: Option<Choice>) {
        match choice {
            Some(choice) => keep(&mut self.displays, display.to_owned(), choice, MAX_DISPLAYS),
            None => self.displays.retain(|(k, _)| k != display),
        }
    }

    /// The default, every other display shown (`shown`, their identities) with no choice of its
    /// own keeping the old one: a choice changes only the display it was made on.
    fn set_default(&mut self, choice: Choice, made_on: &str, shown: &[&str]) {
        if choice == self.default {
            return;
        }
        let old = std::mem::replace(&mut self.default, choice);
        for display in shown {
            if *display != made_on && self.own(display).is_none() {
                self.set_own(display, Some(old.clone()));
            }
        }
    }

    /// The person chose `choice` where Settings is (`at`), the displays shown being `shown`:
    /// while all desktops show a display's choice, the primary's is the default and another's its
    /// own; otherwise it is the active desktop's on that display alone.
    pub fn choose(&mut self, at: &Place, shown: &[&str], choice: Choice) {
        if !self.all_desktops {
            keep(
                &mut self.desktops,
                desktop_key(at.display, at.desktop),
                choice,
                MAX_DESKTOP_CHOICES,
            );
            return;
        }
        if at.primary {
            self.set_default(choice, at.display, shown);
            self.set_own(at.display, None);
        } else {
            self.set_own(at.display, Some(choice));
        }
    }

    /// *Show on all desktops* turned on or off. On, each display shown (`shown`: its place, with
    /// its active desktop) keeps what it shows there, on every desktop, and every desktop's choice
    /// goes; off, nothing changes until the next choice.
    pub fn set_all_desktops(&mut self, on: bool, shown: &[Place]) {
        if on == self.all_desktops {
            return;
        }
        self.all_desktops = on;
        if !on {
            return;
        }
        let now: Vec<(Place, Choice)> = shown
            .iter()
            .map(|place| {
                (
                    place.clone(),
                    self.shown(place.display, place.desktop).clone(),
                )
            })
            .collect();
        self.desktops.clear();
        let identities: Vec<&str> = shown.iter().map(|place| place.display).collect();
        // The primary's first: its choice becomes the default, the others' kept as their own.
        for (place, choice) in now.iter().filter(|(place, _)| place.primary) {
            self.set_default(choice.clone(), place.display, &identities);
            self.set_own(place.display, None);
        }
        for (place, choice) in now.iter().filter(|(place, _)| !place.primary) {
            if *choice == self.default {
                self.set_own(place.display, None);
            } else {
                self.set_own(place.display, Some(choice.clone()));
            }
        }
    }

    /// Desktop `removed` (from 1) is gone: its choices with it, those after it numbered down.
    pub fn remove_desktop(&mut self, removed: u32) {
        self.desktops = std::mem::take(&mut self.desktops)
            .into_iter()
            .filter_map(|(key, choice)| {
                let (display, desktop) = parse_desktop_key(&key)?;
                match desktop.cmp(&removed) {
                    std::cmp::Ordering::Less => Some((key, choice)),
                    std::cmp::Ordering::Equal => None,
                    std::cmp::Ordering::Greater => {
                        Some((desktop_key(display, desktop - 1), choice))
                    }
                }
            })
            .collect();
    }

    /// One of the file's lines, `wallpaper.REST = value`; a bad one is nothing.
    pub(crate) fn read_line(&mut self, rest: &str, value: &str) {
        let Ok(choice) = Choice::parse(value) else {
            return;
        };
        if is_identity(rest) {
            keep(&mut self.displays, rest.to_owned(), choice, MAX_DISPLAYS);
        } else if parse_desktop_key(rest).is_some() {
            keep(
                &mut self.desktops,
                rest.to_owned(),
                choice,
                MAX_DESKTOP_CHOICES,
            );
        }
    }

    /// The file's lines for the displays and desktops.
    pub(crate) fn lines(&self) -> String {
        let mut text = String::new();
        for (key, choice) in self.displays.iter().chain(&self.desktops) {
            text.push_str(&format!("wallpaper.{key} = {}\n", choice.text()));
        }
        text
    }
}

/// `IDENTITY.N`, N from 1 to [`MAX_DESKTOP`].
fn parse_desktop_key(key: &str) -> Option<(&str, u32)> {
    let (display, desktop) = key.rsplit_once('.')?;
    let desktop: u32 = desktop.parse().ok()?;
    (is_identity(display) && (1..=MAX_DESKTOP).contains(&desktop)).then_some((display, desktop))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system(name: &str) -> Choice {
        Choice::System {
            name: name.to_owned(),
            variant: None,
        }
    }

    #[test]
    fn every_form_round_trips() {
        for text in [
            "",
            "oxyos",
            "horizon/night",
            "color:1f4fd8",
            "photo:fit:000000:/home/ana/Pictures/a b.jpg",
            "photo:tile:ffffff:/media/ana/USB/x:y.png",
            "folder:fill:101010:30:/home/ana/Pictures/Trips",
            "folder:center:101010:login:/home/ana/Pictures",
            "shuffle:gradients:login",
            "shuffle:colors:1440",
        ] {
            let choice = Choice::parse(text).unwrap_or_else(|_| panic!("{text:?} refused"));
            assert_eq!(choice.text(), text);
        }
        assert_eq!(
            Choice::parse("photo:fit:000000:/home/a:b.jpg"),
            Ok(Choice::Photo {
                fit: Fit::Fit,
                margin: Rgb([0, 0, 0]),
                path: "/home/a:b.jpg".to_owned()
            })
        );
    }

    #[test]
    fn what_is_not_a_choice_is_refused() {
        for text in [
            "Oxyos",
            "../secret",
            "a/b/c",
            "oxyos/",
            "color:1F4FD8",
            "color:1f4fd",
            "color:",
            "photo:cover:000000:/a.jpg",
            "photo:fit:000000:a.jpg",
            "photo:fit:000000:/a/../b.jpg",
            "photo:fit:000000:/a/./b.jpg",
            "photo:fit:000000:/a//b.jpg",
            "photo:fit:000000:/a/",
            "photo:fit:000000:/",
            "photo:fit:000000:/a\nb",
            "photo:fit:/a.jpg",
            "folder:fill:000000:0:/a",
            "folder:fill:000000:10081:/a",
            "folder:fill:000000:+5:/a",
            "folder:fill:000000:/a",
            "shuffle:Gradients:login",
            "shuffle:gradients",
            "shuffle:gradients:hourly",
            "video:/a.mp4",
        ] {
            assert_eq!(Choice::parse(text), Err(Refused::BadValue), "{text:?}");
        }
        let long = format!("photo:fit:000000:/{}", "a".repeat(MAX_PATH_BYTES));
        assert!(Choice::parse(&long).is_err());
        let longest = format!("photo:fit:000000:/{}", "a".repeat(MAX_PATH_BYTES - 1));
        assert!(Choice::parse(&longest).is_ok());
    }

    const PANEL: &str = "BOE-0A4B-00000001";
    const MONITOR: &str = "DEL-A0B4-7Y9K2N3";

    fn at(display: &str, primary: bool, desktop: u32) -> Place<'_> {
        Place {
            display,
            primary,
            desktop,
        }
    }

    #[test]
    fn a_choice_changes_only_the_display_it_was_made_on() {
        let mut wallpapers = Wallpapers::default();
        let shown = [PANEL, MONITOR];
        // On the primary: the default, the monitor keeping the old default as its own.
        wallpapers.choose(&at(PANEL, true, 1), &shown, system("horizon"));
        assert_eq!(wallpapers.default, system("horizon"));
        assert_eq!(wallpapers.shown(PANEL, 1), &system("horizon"));
        assert_eq!(wallpapers.shown(MONITOR, 1), &Choice::Default);
        // On the monitor: its own; the panel unchanged.
        wallpapers.choose(&at(MONITOR, false, 1), &shown, system("ocean"));
        assert_eq!(wallpapers.shown(MONITOR, 3), &system("ocean"));
        assert_eq!(wallpapers.shown(PANEL, 3), &system("horizon"));
        // A display never seen shows the default.
        assert_eq!(wallpapers.shown("NEW-1", 1), &system("horizon"));
        // Alone, the panel's choice is the default with no line of its own.
        let mut alone = Wallpapers::default();
        alone.choose(&at(PANEL, true, 1), &[PANEL], system("ocean"));
        assert!(alone.displays.is_empty());
        assert_eq!(alone.default, system("ocean"));
    }

    #[test]
    fn each_desktop_keeps_its_own_while_all_desktops_is_off() {
        let mut wallpapers = Wallpapers::default();
        let shown = [PANEL, MONITOR];
        wallpapers.choose(&at(PANEL, true, 1), &shown, system("horizon"));
        wallpapers.set_all_desktops(false, &[at(PANEL, true, 1), at(MONITOR, false, 1)]);
        wallpapers.choose(&at(PANEL, true, 2), &shown, Choice::Color(Rgb([1, 2, 3])));
        assert_eq!(wallpapers.shown(PANEL, 1), &system("horizon"));
        assert_eq!(wallpapers.shown(PANEL, 2), &Choice::Color(Rgb([1, 2, 3])));
        assert_eq!(wallpapers.shown(MONITOR, 2), &Choice::Default);
        wallpapers.choose(&at(PANEL, true, 3), &shown, system("ocean"));
        // Desktop 2 removed: its choice goes, desktop 3's is desktop 2's.
        wallpapers.remove_desktop(2);
        assert_eq!(wallpapers.shown(PANEL, 2), &system("ocean"));
        assert_eq!(wallpapers.shown(PANEL, 3), &system("horizon"));
        // Turned on with desktop 2 active: what each display shows there, on every desktop.
        wallpapers.set_all_desktops(true, &[at(PANEL, true, 2), at(MONITOR, false, 2)]);
        assert!(wallpapers.desktops.is_empty());
        assert_eq!(wallpapers.default, system("ocean"));
        assert_eq!(wallpapers.shown(PANEL, 1), &system("ocean"));
        assert_eq!(wallpapers.shown(MONITOR, 1), &Choice::Default);
    }

    #[test]
    fn its_lines_round_trip_and_bad_ones_are_nothing() {
        let mut wallpapers = Wallpapers::default();
        wallpapers.read_line(MONITOR, "color:1f4fd8");
        wallpapers.read_line(&format!("{PANEL}.2"), "horizon/night");
        wallpapers.read_line(&format!("{PANEL}.0"), "oxyos");
        wallpapers.read_line(&format!("{PANEL}.17"), "oxyos");
        wallpapers.read_line("bad identity", "oxyos");
        wallpapers.read_line(PANEL, "../x");
        assert_eq!(
            wallpapers.lines(),
            format!("wallpaper.{MONITOR} = color:1f4fd8\nwallpaper.{PANEL}.2 = horizon/night\n")
        );
    }
}
