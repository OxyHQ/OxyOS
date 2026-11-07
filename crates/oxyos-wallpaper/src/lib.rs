//! Wallpapers (docs/adr/0048-wallpapers.mdx): the system's catalog, what a display shows for the
//! person's choice at a moment, a folder's photos, and photos' thumbnails.
//!
//! The session keeps each display's and desktop's choice and tells each display's shell its own
//! (`oxyos_preferences::wallpaper`); the shell's host asks [`show`] what that is now: a system
//! wallpaper's variant (*Automatic*: the appearance's, or the sun's, ADR 0012), a shuffle's or a
//! folder's current picture, a colour, and when that next changes by the clock. A shuffle needs no
//! state: time is cut into slots of its interval, and a slot's pick is a hash of the slot, the
//! display and the choice, never the previous slot's.

pub mod catalog;
pub mod thumbnail;

pub use catalog::{Automatic, Catalog, Wallpaper};
use oxyos_preferences::wallpaper::{Choice, Every, Fit, Rgb};
use std::path::{Path, PathBuf};

/// What is shown when there is nothing to show (no catalog): the shell's own blue.
pub const FALLBACK: Rgb = Rgb([0x29, 0x4a, 0xb4]);
/// The most images of a folder a shuffle takes.
pub const MAX_FOLDER_IMAGES: usize = 1000;
/// The pictures a photo or a folder's image may be: what the platform's image pipeline decodes.
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "svg"];

/// The moment and the place a choice is shown at.
#[derive(Clone, Copy, Debug)]
pub struct Moment {
    /// Unix seconds now.
    pub now: i64,
    /// When the session started (unix seconds): a shuffle's slot at each login.
    pub login: i64,
    /// The session's appearance now.
    pub dark: bool,
    /// The display it is shown on (the shell's output): each display shuffles on its own.
    pub output: u32,
    /// Where the system's time zone is: the sun a dynamic wallpaper follows; none (UTC), its
    /// preview.
    pub place: Option<oxyos_solar::Place>,
}

/// What a display draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    /// What is drawn, the same while it is: a new key is crossfaded to.
    pub key: String,
    /// The picture, a `file://` URI; none for a colour alone.
    pub file: Option<PathBuf>,
    pub fit: Fit,
    /// What is behind the picture (a colour choice: the colour).
    pub color: Rgb,
    /// When it next changes by the clock (unix seconds): a variant the sun reaches, a shuffle's
    /// next slot.
    pub next: Option<i64>,
}

impl Shown {
    fn color(color: Rgb) -> Shown {
        Shown {
            key: format!("color:{color}"),
            file: None,
            fit: Fit::Fill,
            color,
            next: None,
        }
    }

    /// Its picture's URI.
    pub fn uri(&self) -> Option<String> {
        self.file.as_deref().map(file_uri)
    }
}

/// `path` as a `file://` URI: every byte but an unreserved one or `/` percent-encoded.
pub fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'.' | b'_' | b'~') {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// The path a `file://` URI names (percent-decoded), or `text` itself if it is a path.
pub fn path_of(text: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let encoded = match text.strip_prefix("file://") {
        Some(rest) => rest,
        None if text.starts_with('/') => return Some(PathBuf::from(text)),
        None => return None,
    };
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    (out.first() == Some(&b'/')).then(|| PathBuf::from(std::ffi::OsString::from_vec(out)))
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// A folder's images, by name, as a shuffle takes them: files (links followed) with a picture's
/// extension, not hidden, at most [`MAX_FOLDER_IMAGES`].
pub fn folder_images(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut images: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .map(|entry| entry.path())
        .filter(|path| is_image(path) && path.is_file())
        .collect();
    images.sort();
    images.truncate(MAX_FOLDER_IMAGES);
    images
}

/// FNV-1a then SplitMix64's finalizer: a hash that is the same on every machine and release.
fn hash(parts: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for &byte in *part {
            h ^= u64::from(byte);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h ^= h >> 30;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^ (h >> 31)
}

/// The order a shuffle shows `count` pictures in during `round`: a permutation from a seeded
/// Fisher-Yates, so each is shown once a round, the first never the one the round before ended on.
fn round_order(seed: &[u8], round: i64, count: usize) -> Vec<usize> {
    let permutation = |round: i64| {
        let mut state = hash(&[seed, &round.to_le_bytes()]);
        let mut order: Vec<usize> = (0..count).collect();
        for i in (1..count).rev() {
            state = hash(&[&state.to_le_bytes()]);
            order.swap(i, (state % (i as u64 + 1)) as usize);
        }
        order
    };
    let mut order = permutation(round);
    if count > 2 && order[0] == permutation(round - 1)[count - 1] {
        order.swap(0, 1);
    }
    order
}

/// Which of `count` candidates a shuffle shows at `moment`, and when its slot ends. Each slot
/// of `every`'s minutes shows another, never the one before; at each login, one for the session.
fn pick(choice: &str, every: Every, count: usize, moment: &Moment) -> (usize, Option<i64>) {
    if count <= 1 {
        return (0, None);
    }
    let seed = [choice.as_bytes(), &moment.output.to_le_bytes()].concat();
    match every {
        Every::Login => (
            (hash(&[&seed, &moment.login.to_le_bytes()]) % count as u64) as usize,
            None,
        ),
        Every::Minutes(minutes) => {
            let length = i64::from(minutes) * 60;
            let slot = moment.now.div_euclid(length);
            let index = if count == 2 {
                // Two: one, then the other.
                (slot.rem_euclid(2) as usize + (hash(&[&seed]) % 2) as usize) % 2
            } else {
                let count_i = count as i64;
                round_order(&seed, slot.div_euclid(count_i), count)
                    [slot.rem_euclid(count_i) as usize]
            };
            (index, Some((slot + 1) * length))
        }
    }
}

fn earliest(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// A system wallpaper's variant: the fixed one, or *Automatic*'s at `moment`, and when the sun
/// next brings another.
fn variant<'a>(
    wallpaper: &'a Wallpaper,
    fixed: Option<&str>,
    moment: &Moment,
) -> (&'a catalog::Variant, Option<i64>) {
    if let Some(variant) = fixed.and_then(|id| wallpaper.variant(id)) {
        return (variant, None);
    }
    let preview = &wallpaper.variants[wallpaper.preview];
    match wallpaper.automatic {
        Automatic::None => (preview, None),
        Automatic::Appearance => {
            let want = if moment.dark {
                catalog::Appearance::Dark
            } else {
                catalog::Appearance::Light
            };
            let chosen = wallpaper
                .variants
                .iter()
                .find(|v| v.appearance == Some(want))
                .unwrap_or(preview);
            (chosen, None)
        }
        Automatic::Sun => {
            let Some(place) = moment.place else {
                return (preview, None);
            };
            let anchors: Vec<oxyos_solar::Anchor> = wallpaper
                .variants
                .iter()
                .enumerate()
                .filter_map(|(index, v)| {
                    v.sun.map(|sun| oxyos_solar::Anchor {
                        frame: index as u32,
                        elevation: sun.elevation,
                        azimuth: sun.azimuth,
                    })
                })
                .collect();
            match oxyos_solar::frame_at(&anchors, moment.now, place) {
                Some((index, next)) => (&wallpaper.variants[index as usize], Some(next)),
                None => (preview, None),
            }
        }
    }
}

fn system(wallpaper: &Wallpaper, fixed: Option<&str>, moment: &Moment) -> Shown {
    let (variant, next) = variant(wallpaper, fixed, moment);
    Shown {
        key: format!("{}/{}", wallpaper.id, variant.id),
        file: Some(variant.file.clone()),
        fit: Fit::Fill,
        color: Rgb([0, 0, 0]),
        next,
    }
}

/// The catalog's default at `moment`, or the shell's blue without one.
fn default(catalog: &Catalog, moment: &Moment) -> Shown {
    catalog
        .default
        .as_deref()
        .and_then(|id| catalog.wallpaper(id))
        .map_or_else(|| Shown::color(FALLBACK), |w| system(w, None, moment))
}

fn photo(path: &Path, fit: Fit, margin: Rgb) -> Option<Shown> {
    (is_image(path) && path.is_file()).then(|| Shown {
        key: format!("photo:{}:{margin}:{}", fit.name(), path.display()),
        file: Some(path.to_owned()),
        fit,
        color: margin,
        next: None,
    })
}

/// What a display shows for `choice` at `moment`, from `catalog`: what names nothing that is
/// there (a photo deleted, a wallpaper no longer in the catalog, an empty folder) shows the
/// catalog's default.
pub fn show(catalog: &Catalog, choice: &Choice, moment: &Moment) -> Shown {
    let shown = match choice {
        Choice::Default => None,
        Choice::System { name, variant } => catalog
            .wallpaper(name)
            .map(|w| system(w, variant.as_deref(), moment)),
        Choice::Color(color) => Some(Shown::color(*color)),
        Choice::Photo { fit, margin, path } => photo(Path::new(path), *fit, *margin),
        Choice::Folder {
            fit,
            margin,
            every,
            path,
        } => {
            let images = folder_images(Path::new(path));
            let (index, next) = pick(&choice.text(), *every, images.len(), moment);
            images
                .get(index)
                .and_then(|image| photo(image, *fit, *margin).map(|shown| Shown { next, ..shown }))
        }
        Choice::Shuffle { category, every } if category == "colors" => {
            let (index, next) = pick(&choice.text(), *every, catalog.colors.len(), moment);
            catalog.colors.get(index).map(|color| Shown {
                next,
                ..Shown::color(color.color)
            })
        }
        Choice::Shuffle { category, every } => {
            let wallpapers: Vec<&Wallpaper> = catalog.in_category(category).collect();
            let (index, next) = pick(&choice.text(), *every, wallpapers.len(), moment);
            wallpapers.get(index).map(|w| {
                let shown = system(w, None, moment);
                Shown {
                    next: earliest(shown.next, next),
                    ..shown
                }
            })
        }
    };
    shown.unwrap_or_else(|| default(catalog, moment))
}

#[cfg(test)]
mod tests;
