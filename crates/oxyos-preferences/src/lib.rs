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
//!
//! And the person's lists of applications (docs/adr/0034-the-dock.mdx): those kept in the dock, in
//! its order (`dock`, absent until the person changes it: the registry's then), and those the
//! session opens at login (`open_at_login`), ids separated by commas. The shell changes them
//! whole, through [`Preferences::set_list`], never through [`Preferences::set`].

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

pub mod displays;
/// The session's language and region (docs/adr/0040-language-and-region.mdx).
pub mod locale;
/// The user's folders (xdg-user-dirs).
pub mod user_dirs;
/// The person's wallpapers (docs/adr/0048-wallpapers.mdx).
pub mod wallpaper;
pub use displays::{Arrangement, DisplayMode, DisplaySettings};
pub use locale::{environment_locale, session_locale, HourCycle, LANGUAGES};

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

/// How many fingers swipe one of a trackpad's gestures (Settings › Trackpad › More Gestures,
/// docs/adr/0046-interactive-gestures.mdx, amended 8 November 2026): three, as a Mac's trackpad
/// ships them, which four fingers swipe too, as on a Mac; or four, three then doing nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SwipeFingers {
    #[default]
    Three,
    Four,
}

impl SwipeFingers {
    fn name(self) -> &'static str {
        match self {
            SwipeFingers::Three => "three",
            SwipeFingers::Four => "four",
        }
    }

    /// The fewest fingers that swipe it.
    pub fn fewest(self) -> u8 {
        match self {
            SwipeFingers::Three => 3,
            SwipeFingers::Four => 4,
        }
    }
}

/// The trackpad's gestures the person chooses, as a Mac's System Settings › Trackpad › More
/// Gestures offers them (docs/adr/0046-interactive-gestures.mdx, amended 8 November 2026). Each
/// swipe is three fingers, four, or off; Mission Control (up) and App Exposé (down) share their
/// fingers, as a Mac's do: choosing four for one chooses four for the other, unless it is off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrackpadGestures {
    /// The fingers that swipe up and down.
    pub vertical: SwipeFingers,
    /// Mission Control, swiped up.
    pub mission_control: bool,
    /// App Exposé, swiped down.
    pub app_expose: bool,
    /// The desktops, swiped across; none, off.
    pub desktops: Option<SwipeFingers>,
    /// Launchpad, pinched with the thumb and three fingers.
    pub launchpad: bool,
}

impl Default for TrackpadGestures {
    fn default() -> Self {
        Self {
            vertical: SwipeFingers::Three,
            mission_control: true,
            app_expose: true,
            desktops: Some(SwipeFingers::Three),
            launchpad: true,
        }
    }
}

/// A swipe's fingers as the file and Settings give them: `three`, `four`, or `off` (none).
fn swipe_fingers(value: &str) -> Result<Option<SwipeFingers>, Refused> {
    match value {
        "three" => Ok(Some(SwipeFingers::Three)),
        "four" => Ok(Some(SwipeFingers::Four)),
        "off" => Ok(None),
        _ => Err(Refused::BadValue),
    }
}

fn swipe_fingers_name(fingers: Option<SwipeFingers>) -> &'static str {
    fingers.map_or("off", SwipeFingers::name)
}

impl TrackpadGestures {
    /// Mission Control's fingers, none when it is off.
    pub fn mission_control_fingers(&self) -> Option<SwipeFingers> {
        self.mission_control.then_some(self.vertical)
    }

    /// App Exposé's fingers, none when it is off.
    pub fn app_expose_fingers(&self) -> Option<SwipeFingers> {
        self.app_expose.then_some(self.vertical)
    }
}

/// When Low Power Mode is on (docs/adr/0057-battery-and-energy.mdx), as a Mac's Battery settings
/// offer it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LowPowerMode {
    #[default]
    Never,
    Always,
    /// While the machine runs on its battery.
    OnBattery,
    /// While it runs on its power adapter.
    OnAdapter,
}

impl LowPowerMode {
    pub fn name(self) -> &'static str {
        match self {
            LowPowerMode::Never => "never",
            LowPowerMode::Always => "always",
            LowPowerMode::OnBattery => "battery",
            LowPowerMode::OnAdapter => "adapter",
        }
    }

    /// Whether it is on, the machine on its adapter (`on_ac`) or not.
    pub fn on(self, on_ac: bool) -> bool {
        match self {
            LowPowerMode::Never => false,
            LowPowerMode::Always => true,
            LowPowerMode::OnBattery => !on_ac,
            LowPowerMode::OnAdapter => on_ac,
        }
    }
}

/// The longest a display-off or sleep timer takes, in minutes: three hours, as a Mac's longest.
pub const MAX_SLEEP_MINUTES: u32 = 180;

/// How the machine saves energy on its battery and on its adapter (docs/adr/0057-battery-and-
/// energy.mdx): minutes with no input before the display turns off and before the machine
/// sleeps, 0 never.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Energy {
    pub display_off_battery: u32,
    pub display_off_adapter: u32,
    pub sleep_battery: u32,
    pub sleep_adapter: u32,
    /// The display slightly dimmer while on battery.
    pub dim_on_battery: bool,
    pub low_power_mode: LowPowerMode,
}

impl Default for Energy {
    fn default() -> Self {
        Self {
            display_off_battery: 2,
            display_off_adapter: 10,
            sleep_battery: 10,
            sleep_adapter: 0,
            dim_on_battery: true,
            low_power_mode: LowPowerMode::default(),
        }
    }
}

impl Energy {
    /// Minutes before the display turns off, on the adapter (`on_ac`) or not; 0 never.
    pub fn display_off(&self, on_ac: bool) -> u32 {
        if on_ac {
            self.display_off_adapter
        } else {
            self.display_off_battery
        }
    }

    /// Minutes before the machine sleeps; 0 never.
    pub fn sleep(&self, on_ac: bool) -> u32 {
        if on_ac {
            self.sleep_adapter
        } else {
            self.sleep_battery
        }
    }
}

fn sleep_minutes(value: &str) -> Result<u32, Refused> {
    value
        .parse()
        .ok()
        .filter(|minutes| *minutes <= MAX_SLEEP_MINUTES)
        .ok_or(Refused::BadValue)
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
    /// A text field's caret blinks; off, it stays lit (docs/adr/0037-text-editing.mdx).
    pub caret_blink: bool,
    /// *Play user interface sound effects* (docs/adr/0044-system-sounds.mdx): the Trash's sounds
    /// play; off, they do not. Notifications' are alerts, which it does not silence.
    pub ui_sounds: bool,
    /// *Natural scrolling* on a touchpad (docs/adr/0042-scrolling.mdx, decision 6): the content
    /// follows the fingers, as a Mac's trackpad ships. Applied by libinput to every touchpad.
    pub trackpad_natural_scrolling: bool,
    /// *Natural scrolling* on every other pointing device (a mouse's wheel, a trackpoint): off, a
    /// wheel's notch turned toward the person scrolls down, as every mouse has.
    pub mouse_natural_scrolling: bool,
    /// Settings › Trackpad › More Gestures: which fingers swipe to Mission Control, App Exposé and
    /// the desktops, and whether a pinch opens Launchpad (docs/adr/0046-interactive-gestures.mdx).
    pub trackpad_gestures: TrackpadGestures,
    /// The wallpapers (docs/adr/0048-wallpapers.mdx): the default (`wallpaper`), whether a
    /// display's shows on all its desktops (`wallpaper_all_desktops`), and each display's and
    /// desktop's own (`wallpaper.DISPLAY`, `wallpaper.DISPLAY.N`), which only the session changes.
    pub wallpapers: wallpaper::Wallpapers,
    /// Minutes idle before the session locks; 0, never.
    pub lock_after: u32,
    /// The screen reader is on when the session starts (ADR 0014).
    pub screen_reader: bool,
    /// The battery's percentage shown beside its icon in the menu bar (ADR 0057).
    pub battery_percentage: bool,
    /// The display's and the machine's sleep, Low Power Mode and the battery's options (ADR 0057).
    pub energy: Energy,
    /// The menu bar's backup item, shown once a backup destination exists (ADR 0059, decision
    /// 10: *Show Backups in menu bar*).
    pub backup_in_menu_bar: bool,
    /// The language the person chose, one of [`LANGUAGES`]; none is the system's (ADR 0040).
    pub language: Option<String>,
    /// The region the person chose, as CLDR names one (two ASCII capitals); none is the
    /// system's.
    pub region: Option<String>,
    /// Twelve or 24 hours, or the region's.
    pub hour_cycle: HourCycle,
    /// Do Not Disturb (ADR 0026): notifications are kept in the Notification Center and shown as
    /// no banner, and the screen reader says none.
    pub do_not_disturb: bool,
    /// What the lock screen shows of notifications.
    pub notification_previews: Previews,
    /// *Empty Trash Automatically* (docs/adr/0062-storage.mdx): the session erases what has been
    /// in the Trash for more than 30 days. Off, as a Mac ships.
    pub empty_trash_automatically: bool,
    /// *Reopen windows when logging back in* (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx):
    /// the applications running as the person logs out, restarts or shuts down are opened again
    /// at the next login. The checkbox keeps its state, as a Mac's; on, as a Mac ships.
    pub reopen_windows: bool,
    /// Each display's mode and scale, by its identity, the one changed last at the end.
    pub displays: Vec<(String, DisplaySettings)>,
    /// Each set of displays' arrangement, by the set's key, the one changed last at the end.
    pub arrangements: Vec<(String, Arrangement)>,
    /// The applications kept in the dock, in its order; none until the person changes it (the
    /// registry's entries say then, `X-OxyOS-Pinned`).
    pub dock: Option<Vec<String>>,
    /// The applications the session opens when it starts, in order.
    pub open_at_login: Vec<String>,
    /// The applications whose notifications play no sound (docs/adr/0044-system-sounds.mdx),
    /// Settings' *Play sound for notifications* turned off for each.
    pub notification_sounds_off: Vec<String>,
    /// The input methods the person added after their keyboard layout, in order, each one of
    /// [`INPUT_METHODS`] (ADR 0041).
    pub input_sources: Vec<String>,
    /// The current input source: one of `input_sources`, or none for the keyboard layout.
    pub input_source: Option<String>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            appearance: Appearance::default(),
            accent: "oxy".to_owned(),
            reduce_motion: false,
            reduce_transparency: false,
            caret_blink: true,
            ui_sounds: true,
            trackpad_natural_scrolling: true,
            mouse_natural_scrolling: false,
            trackpad_gestures: TrackpadGestures::default(),
            wallpapers: wallpaper::Wallpapers::default(),
            lock_after: 5,
            screen_reader: false,
            battery_percentage: false,
            energy: Energy::default(),
            backup_in_menu_bar: true,
            language: None,
            region: None,
            hour_cycle: HourCycle::default(),
            do_not_disturb: false,
            notification_previews: Previews::default(),
            empty_trash_automatically: false,
            reopen_windows: true,
            displays: Vec::new(),
            arrangements: Vec::new(),
            dock: None,
            open_at_login: Vec::new(),
            notification_sounds_off: Vec::new(),
            input_sources: Vec::new(),
            input_source: None,
        }
    }
}

/// The input methods OxyOS has (ADR 0041), by the names the preferences keep: Pinyin –
/// Simplified, Japanese – Romaji and 2-Set Korean. Whether the engine installed has each is the
/// session's to say.
pub const INPUT_METHODS: &[&str] = &["pinyin", "japanese", "korean"];

/// What `input_source` keeps for the keyboard layout.
pub const LAYOUT_SOURCE: &str = "layout";

/// The most applications one of the person's lists holds.
pub const MAX_APPLICATIONS: usize = 64;

/// One of the person's lists of applications.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationList {
    /// Kept in the dock, in its order.
    Dock,
    /// Opened when the session starts.
    OpenAtLogin,
    /// Whose notifications play no sound (docs/adr/0044-system-sounds.mdx), Settings'.
    NotificationSoundsOff,
}

/// An application id, as the registry takes one (`oxyos_apps::valid_id`): a reverse domain name
/// of ASCII letters, digits, `.`, `-` and `_`, at most 128 bytes.
fn application_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// A list of application ids: each valid, none twice, at most [`MAX_APPLICATIONS`].
pub fn application_list<'a>(
    ids: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<String>, Refused> {
    let mut list: Vec<String> = Vec::new();
    for id in ids {
        if !application_id(id) || list.iter().any(|kept| kept == id) {
            return Err(Refused::BadValue);
        }
        list.push(id.to_owned());
    }
    if list.len() > MAX_APPLICATIONS {
        return Err(Refused::BadValue);
    }
    Ok(list)
}

/// A list as the file keeps it: ids separated by commas, none for an empty one.
fn list_from_file(value: &str) -> Result<Vec<String>, Refused> {
    application_list(value.split(',').map(str::trim).filter(|id| !id.is_empty()))
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
    "caret_blink",
    "ui_sounds",
    "trackpad_natural_scrolling",
    "mouse_natural_scrolling",
    "mission_control_swipe",
    "app_expose_swipe",
    "desktops_swipe",
    "launchpad_pinch",
    "wallpaper",
    "wallpaper_all_desktops",
    "lock_after",
    "screen_reader",
    "battery_percentage",
    "low_power_mode",
    "display_off_battery",
    "display_off_adapter",
    "sleep_battery",
    "sleep_adapter",
    "dim_on_battery",
    "backup_in_menu_bar",
    "language",
    "region",
    "hour_cycle",
    "do_not_disturb",
    "notification_previews",
    "empty_trash_automatically",
    "reopen_windows",
    "input_sources",
    "input_source",
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
            "caret_blink" => self.caret_blink = boolean(value)?,
            "ui_sounds" => self.ui_sounds = boolean(value)?,
            "trackpad_natural_scrolling" => self.trackpad_natural_scrolling = boolean(value)?,
            "mouse_natural_scrolling" => self.mouse_natural_scrolling = boolean(value)?,
            // Mission Control and App Exposé share their fingers, as a Mac's: three or four chosen
            // for one is the other's too.
            "mission_control_swipe" | "app_expose_swipe" => {
                let fingers = swipe_fingers(value)?;
                let gestures = &mut self.trackpad_gestures;
                if let Some(fingers) = fingers {
                    gestures.vertical = fingers;
                }
                if key == "mission_control_swipe" {
                    gestures.mission_control = fingers.is_some();
                } else {
                    gestures.app_expose = fingers.is_some();
                }
            }
            "desktops_swipe" => self.trackpad_gestures.desktops = swipe_fingers(value)?,
            "launchpad_pinch" => self.trackpad_gestures.launchpad = boolean(value)?,
            // The default; a choice where Settings is goes through the session
            // (`wallpaper::Wallpapers::choose`).
            "wallpaper" => self.wallpapers.default = wallpaper::Choice::parse(value)?,
            "wallpaper_all_desktops" => self.wallpapers.all_desktops = boolean(value)?,
            "lock_after" => {
                // At most a day: a longer wait is never.
                self.lock_after = value
                    .parse()
                    .ok()
                    .filter(|minutes| *minutes <= 24 * 60)
                    .ok_or(Refused::BadValue)?
            }
            "screen_reader" => self.screen_reader = boolean(value)?,
            "battery_percentage" => self.battery_percentage = boolean(value)?,
            "backup_in_menu_bar" => self.backup_in_menu_bar = boolean(value)?,
            "low_power_mode" => {
                self.energy.low_power_mode = match value {
                    "never" => LowPowerMode::Never,
                    "always" => LowPowerMode::Always,
                    "battery" => LowPowerMode::OnBattery,
                    "adapter" => LowPowerMode::OnAdapter,
                    _ => return Err(Refused::BadValue),
                }
            }
            "display_off_battery" => self.energy.display_off_battery = sleep_minutes(value)?,
            "display_off_adapter" => self.energy.display_off_adapter = sleep_minutes(value)?,
            "sleep_battery" => self.energy.sleep_battery = sleep_minutes(value)?,
            "sleep_adapter" => self.energy.sleep_adapter = sleep_minutes(value)?,
            "dim_on_battery" => self.energy.dim_on_battery = boolean(value)?,
            "language" => self.language = locale::language(value)?,
            "region" => self.region = locale::region(value)?,
            "hour_cycle" => self.hour_cycle = HourCycle::parse(value).ok_or(Refused::BadValue)?,
            "do_not_disturb" => self.do_not_disturb = boolean(value)?,
            "notification_previews" => {
                self.notification_previews = match value {
                    "always" => Previews::Always,
                    "unlocked" => Previews::Unlocked,
                    _ => return Err(Refused::BadValue),
                }
            }
            "empty_trash_automatically" => self.empty_trash_automatically = boolean(value)?,
            "reopen_windows" => self.reopen_windows = boolean(value)?,
            "input_sources" => {
                let mut sources: Vec<String> = Vec::new();
                for id in value.split(',').filter(|id| !id.is_empty()) {
                    if !INPUT_METHODS.contains(&id) || sources.iter().any(|kept| kept == id) {
                        return Err(Refused::BadValue);
                    }
                    sources.push(id.to_owned());
                }
                // The current one removed: the keyboard layout is current.
                if self
                    .input_source
                    .as_ref()
                    .is_some_and(|current| !sources.contains(current))
                {
                    self.input_source = None;
                }
                self.input_sources = sources;
            }
            "input_source" => {
                self.input_source = match value {
                    "" | LAYOUT_SOURCE => None,
                    id if self.input_sources.iter().any(|kept| kept == id) => Some(id.to_owned()),
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
            "caret_blink" => self.caret_blink.to_string(),
            "ui_sounds" => self.ui_sounds.to_string(),
            "trackpad_natural_scrolling" => self.trackpad_natural_scrolling.to_string(),
            "mouse_natural_scrolling" => self.mouse_natural_scrolling.to_string(),
            "mission_control_swipe" => {
                swipe_fingers_name(self.trackpad_gestures.mission_control_fingers()).to_owned()
            }
            "app_expose_swipe" => {
                swipe_fingers_name(self.trackpad_gestures.app_expose_fingers()).to_owned()
            }
            "desktops_swipe" => swipe_fingers_name(self.trackpad_gestures.desktops).to_owned(),
            "launchpad_pinch" => self.trackpad_gestures.launchpad.to_string(),
            "wallpaper" => self.wallpapers.default.text(),
            "wallpaper_all_desktops" => self.wallpapers.all_desktops.to_string(),
            "lock_after" => self.lock_after.to_string(),
            "screen_reader" => self.screen_reader.to_string(),
            "battery_percentage" => self.battery_percentage.to_string(),
            "low_power_mode" => self.energy.low_power_mode.name().to_owned(),
            "display_off_battery" => self.energy.display_off_battery.to_string(),
            "display_off_adapter" => self.energy.display_off_adapter.to_string(),
            "sleep_battery" => self.energy.sleep_battery.to_string(),
            "sleep_adapter" => self.energy.sleep_adapter.to_string(),
            "dim_on_battery" => self.energy.dim_on_battery.to_string(),
            "backup_in_menu_bar" => self.backup_in_menu_bar.to_string(),
            "language" => self.language.clone().unwrap_or_default(),
            "region" => self.region.clone().unwrap_or_default(),
            "hour_cycle" => self.hour_cycle.name().to_owned(),
            "do_not_disturb" => self.do_not_disturb.to_string(),
            "notification_previews" => self.notification_previews.name().to_owned(),
            "empty_trash_automatically" => self.empty_trash_automatically.to_string(),
            "reopen_windows" => self.reopen_windows.to_string(),
            "input_sources" => self.input_sources.join(","),
            "input_source" => self
                .input_source
                .clone()
                .unwrap_or_else(|| LAYOUT_SOURCE.to_owned()),
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

    /// Replaces one of the person's lists of applications (the shell's change, docs/adr/0034):
    /// each id valid, none twice, at most [`MAX_APPLICATIONS`]. Nothing changes if it is refused.
    pub fn set_list(&mut self, list: ApplicationList, ids: &[String]) -> Result<(), Refused> {
        let ids = application_list(ids.iter().map(String::as_str))?;
        match list {
            ApplicationList::Dock => self.dock = Some(ids),
            ApplicationList::OpenAtLogin => self.open_at_login = ids,
            ApplicationList::NotificationSoundsOff => self.notification_sounds_off = ids,
        }
        Ok(())
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
                } else if let Some(rest) = key.strip_prefix("wallpaper.") {
                    preferences.wallpapers.read_line(rest, value);
                } else if key == "dock" {
                    if let Ok(ids) = list_from_file(value) {
                        preferences.dock = Some(ids);
                    }
                } else if key == "open_at_login" {
                    if let Ok(ids) = list_from_file(value) {
                        preferences.open_at_login = ids;
                    }
                } else if key == "notification_sounds_off" {
                    if let Ok(ids) = list_from_file(value) {
                        preferences.notification_sounds_off = ids;
                    }
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
        text.push_str(&self.wallpapers.lines());
        if let Some(dock) = &self.dock {
            text.push_str(&format!("dock = {}\n", dock.join(",")));
        }
        if !self.open_at_login.is_empty() {
            text.push_str(&format!(
                "open_at_login = {}\n",
                self.open_at_login.join(",")
            ));
        }
        if !self.notification_sounds_off.is_empty() {
            text.push_str(&format!(
                "notification_sounds_off = {}\n",
                self.notification_sounds_off.join(",")
            ));
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
