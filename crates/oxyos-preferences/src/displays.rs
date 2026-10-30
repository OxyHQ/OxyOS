//! The displays' settings the session keeps (docs/adr/0018-several-displays.mdx), as a Mac keeps
//! them: each display's mode and scale by who it is (its identity, from its EDID), and where each
//! display of a set is, by the set. They are lines of the preferences file the session writes when
//! Settings changes them and reads when a display comes:
//!
//! ```text
//! display.DEL-A0B4-7Y9K2N3 = 3840x2160@60000 1.5
//! arrangement.BOE-0A4B-00000001+DEL-A0B4-7Y9K2N3 = DEL-A0B4-7Y9K2N3 0,0 BOE-0A4B-00000001 2560,480
//! ```
//!
//! A display's line is its mode (pixels and refresh in millihertz) and its scale; an arrangement's
//! key is the set's identities, sorted, its value each display's origin in the global space
//! (logical pixels), the primary first. A bad line is nothing, as any other.

/// The most displays and arrangements kept: past them, the one changed longest ago goes.
pub const MAX_DISPLAYS: usize = 32;
pub const MAX_ARRANGEMENTS: usize = 32;
/// The longest identity, in bytes.
pub const MAX_IDENTITY_BYTES: usize = 64;
/// Most displays in one arrangement (the scene takes at most 16 outputs).
pub const MAX_ARRANGED: usize = 16;
/// Largest pixel dimension and coordinate (the protocol's).
const MAX_DIMENSION: u32 = 16_384;
const MAX_COORDINATE: i32 = 1 << 20;

/// A display's mode: its pixels and its refresh in millihertz.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub refresh_mhz: u32,
}

/// How a display is shown: its mode and its scale (in 120ths: 150 is 1.25).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplaySettings {
    pub mode: DisplayMode,
    pub scale_120: u32,
}

/// Where each display of a set is: its identity and its origin in the global space, the primary
/// first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arrangement {
    pub displays: Vec<(String, (i32, i32))>,
}

impl Arrangement {
    /// The set it is kept for: its identities sorted, joined by `+`.
    pub fn key(&self) -> String {
        set_key(self.displays.iter().map(|(identity, _)| identity.as_str()))
    }
}

/// The key of the set of these displays: their identities sorted, joined by `+`.
pub fn set_key<'a>(identities: impl IntoIterator<Item = &'a str>) -> String {
    let mut identities: Vec<&str> = identities.into_iter().collect();
    identities.sort_unstable();
    identities.join("+")
}

/// Whether `text` is an identity: ASCII letters, digits and `-`, at most
/// [`MAX_IDENTITY_BYTES`].
pub fn is_identity(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_IDENTITY_BYTES
        && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// A scale's text: 1, 1.25, 1.5 (in 120ths, to the hundredth).
pub fn scale_text(scale_120: u32) -> String {
    let hundredths = (u64::from(scale_120) * 100 + 60) / 120;
    let text = format!("{}.{:02}", hundredths / 100, hundredths % 100);
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn parse_scale(text: &str) -> Option<u32> {
    let value: f64 = text.parse().ok()?;
    let scale_120 = (value * 120.0).round();
    (120.0..=480.0)
        .contains(&scale_120)
        .then_some(scale_120 as u32)
}

impl DisplaySettings {
    /// Its line's value: `3840x2160@60000 1.5`.
    pub fn text(&self) -> String {
        let DisplayMode {
            width,
            height,
            refresh_mhz,
        } = self.mode;
        format!(
            "{width}x{height}@{refresh_mhz} {}",
            scale_text(self.scale_120)
        )
    }

    pub fn parse(text: &str) -> Option<Self> {
        let (mode, scale) = text.split_once(' ')?;
        let (size, refresh) = mode.split_once('@')?;
        let (width, height) = size.split_once('x')?;
        let dimension = |text: &str| {
            text.parse::<u32>()
                .ok()
                .filter(|v| (1..=MAX_DIMENSION).contains(v))
        };
        Some(Self {
            mode: DisplayMode {
                width: dimension(width)?,
                height: dimension(height)?,
                refresh_mhz: refresh
                    .parse()
                    .ok()
                    .filter(|mhz| (1..=1_000_000).contains(mhz))?,
            },
            scale_120: parse_scale(scale.trim())?,
        })
    }
}

impl Arrangement {
    /// Its line's value: each display's identity and origin, the primary first.
    pub fn text(&self) -> String {
        self.displays
            .iter()
            .map(|(identity, (x, y))| format!("{identity} {x},{y}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The value of the line for the set `key`: refused unless it places that set's displays,
    /// each once, two at least.
    pub fn parse(key: &str, text: &str) -> Option<Self> {
        let words: Vec<&str> = text.split_whitespace().collect();
        if words.len() % 2 != 0 || words.len() < 4 || words.len() > 2 * MAX_ARRANGED {
            return None;
        }
        let mut displays = Vec::new();
        for pair in words.chunks_exact(2) {
            let identity = pair[0];
            let (x, y) = pair[1].split_once(',')?;
            let coordinate = |text: &str| {
                text.parse::<i32>()
                    .ok()
                    .filter(|v| (-MAX_COORDINATE..=MAX_COORDINATE).contains(v))
            };
            if !is_identity(identity) || displays.iter().any(|(i, _)| i == identity) {
                return None;
            }
            displays.push((identity.to_string(), (coordinate(x)?, coordinate(y)?)));
        }
        let arrangement = Self { displays };
        (arrangement.key() == key).then_some(arrangement)
    }
}

/// `entries` with `key`'s replaced by `value`, now the latest, the oldest gone past `max`.
pub(crate) fn keep<T>(entries: &mut Vec<(String, T)>, key: String, value: T, max: usize) {
    entries.retain(|(k, _)| *k != key);
    entries.push((key, value));
    if entries.len() > max {
        entries.drain(..entries.len() - max);
    }
}
