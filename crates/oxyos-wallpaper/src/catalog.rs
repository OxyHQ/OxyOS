//! The system's wallpapers (docs/adr/0048-wallpapers.mdx, decision 1): `catalog.json` (the
//! categories in order, the default, the colours) and each wallpaper's folder with its
//! `wallpaper.json` (its name, its category, its variants and how *Automatic* chooses among
//! them). Read strictly: an unknown key, a name that is not one, a file outside its folder or a
//! variant missing what its rule needs make the wallpaper nothing, said once; the catalog itself
//! unreadable is an empty one.

use oxyos_preferences::wallpaper::{is_name, Rgb};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The image's catalog.
pub const SYSTEM_DIRECTORY: &str = "/usr/share/oxyos/wallpapers";
/// The most wallpapers a catalog holds, and variants a wallpaper.
const MAX_WALLPAPERS: usize = 256;
const MAX_VARIANTS: usize = 32;
/// The largest JSON file read.
const MAX_JSON_BYTES: u64 = 64 * 1024;

/// Where the catalog is: `OXYOS_WALLPAPERS`, else the image's.
pub fn directory() -> PathBuf {
    std::env::var_os("OXYOS_WALLPAPERS")
        .filter(|value| !value.is_empty())
        .map_or_else(|| PathBuf::from(SYSTEM_DIRECTORY), PathBuf::from)
}

/// A name in each of the session's languages (ADR 0040); English when one is missing.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Names {
    pub en: String,
    #[serde(default)]
    pub es: Option<String>,
}

impl Names {
    /// The name in `language` (`en`, `es`, or a tag starting with one), else English.
    pub fn get(&self, language: &str) -> &str {
        let primary = language.split(['-', '_']).next().unwrap_or("");
        match primary {
            "es" => self.es.as_deref().unwrap_or(&self.en),
            _ => &self.en,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogFile {
    version: u32,
    default: String,
    categories: Vec<CategoryFile>,
    #[serde(default)]
    colors: Vec<ColorFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CategoryFile {
    id: String,
    name: Names,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ColorFile {
    color: String,
    name: Names,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WallpaperFile {
    name: Names,
    category: String,
    automatic: Automatic,
    preview: String,
    variants: Vec<VariantFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VariantFile {
    id: String,
    name: Names,
    file: String,
    #[serde(default)]
    appearance: Option<Appearance>,
    #[serde(default)]
    sun: Option<Sun>,
}

/// How *Automatic* chooses among a wallpaper's variants.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Automatic {
    /// One variant.
    None,
    /// Light or dark, the session's appearance.
    Appearance,
    /// The variant closest to the sun now (docs/adr/0012-solar-wallpaper.mdx).
    Sun,
}

impl Automatic {
    pub fn name(self) -> &'static str {
        match self {
            Automatic::None => "none",
            Automatic::Appearance => "appearance",
            Automatic::Sun => "sun",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    Light,
    Dark,
}

/// The sun a variant shows: degrees above the horizon, and clockwise from north.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Sun {
    pub elevation: f64,
    pub azimuth: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Variant {
    pub id: String,
    pub name: Names,
    /// Its picture, in the wallpaper's folder.
    pub file: PathBuf,
    pub appearance: Option<Appearance>,
    pub sun: Option<Sun>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wallpaper {
    pub id: String,
    pub name: Names,
    pub category: String,
    pub automatic: Automatic,
    /// The variant shown as its preview, and when nothing decides (no sun where the clock is).
    pub preview: usize,
    pub variants: Vec<Variant>,
}

impl Wallpaper {
    pub fn variant(&self, id: &str) -> Option<&Variant> {
        self.variants.iter().find(|variant| variant.id == id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Category {
    pub id: String,
    pub name: Names,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Color {
    pub color: Rgb,
    pub name: Names,
}

/// The catalog: its categories in order, its wallpapers in their categories' order (each
/// category's by id), its default and its colours.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Catalog {
    pub directory: PathBuf,
    pub default: Option<String>,
    pub categories: Vec<Category>,
    pub wallpapers: Vec<Wallpaper>,
    pub colors: Vec<Color>,
}

/// The pictures a variant may be.
const EXTENSIONS: &[&str] = &["svg", "png", "jpg", "jpeg", "webp"];

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let length = file.metadata().map_err(|error| error.to_string())?.len();
    if length > MAX_JSON_BYTES {
        return Err("too large".to_owned());
    }
    serde_json::from_reader(std::io::BufReader::new(file)).map_err(|error| error.to_string())
}

impl Catalog {
    /// The catalog at `directory`; an empty one where it cannot be read, said.
    pub fn read(directory: &Path) -> Catalog {
        match Self::try_read(directory) {
            Ok(catalog) => catalog,
            Err(error) => {
                eprintln!("[wallpapers] {}: {error}", directory.display());
                Catalog {
                    directory: directory.to_owned(),
                    ..Catalog::default()
                }
            }
        }
    }

    /// The system's: [`directory`]'s.
    pub fn system() -> Catalog {
        Self::read(&directory())
    }

    fn try_read(directory: &Path) -> Result<Catalog, String> {
        let file: CatalogFile = read_json(&directory.join("catalog.json"))?;
        if file.version != 1 {
            return Err(format!("version {} is not one this reads", file.version));
        }
        let mut categories: Vec<Category> = Vec::new();
        for category in file.categories {
            if !is_name(&category.id)
                || category.id == "colors"
                || categories.iter().any(|c| c.id == category.id)
            {
                return Err(format!("category {:?} is not one", category.id));
            }
            categories.push(Category {
                id: category.id,
                name: category.name,
            });
        }
        let colors = file
            .colors
            .into_iter()
            .map(|color| {
                let hex = color.color.strip_prefix('#').unwrap_or("");
                Rgb::parse(&hex.to_ascii_lowercase())
                    .map(|rgb| Color {
                        color: rgb,
                        name: color.name,
                    })
                    .ok_or_else(|| format!("colour {:?} is not one", color.color))
            })
            .collect::<Result<Vec<_>, _>>()?;
        // Every folder with a wallpaper.json, by id.
        let mut found: BTreeMap<String, Wallpaper> = BTreeMap::new();
        let entries = std::fs::read_dir(directory).map_err(|error| error.to_string())?;
        for entry in entries.flatten() {
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !is_name(&id) || !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let folder = entry.path();
            if !folder.join("wallpaper.json").is_file() {
                continue;
            }
            if found.len() >= MAX_WALLPAPERS {
                break;
            }
            match read_wallpaper(&folder, &id, &categories) {
                Ok(wallpaper) => {
                    found.insert(id, wallpaper);
                }
                Err(error) => eprintln!("[wallpapers] {id}: {error}"),
            }
        }
        let mut wallpapers = Vec::new();
        for category in &categories {
            let ids: Vec<String> = found
                .values()
                .filter(|w| w.category == category.id)
                .map(|w| w.id.clone())
                .collect();
            for id in ids {
                wallpapers.extend(found.remove(&id));
            }
        }
        let default = Some(file.default).filter(|id| wallpapers.iter().any(|w| w.id == *id));
        if default.is_none() {
            eprintln!("[wallpapers] the catalog's default is none of its wallpapers");
        }
        Ok(Catalog {
            directory: directory.to_owned(),
            default,
            categories,
            wallpapers,
            colors,
        })
    }

    pub fn wallpaper(&self, id: &str) -> Option<&Wallpaper> {
        self.wallpapers.iter().find(|wallpaper| wallpaper.id == id)
    }

    /// A category's wallpapers, in order.
    pub fn in_category<'a>(&'a self, category: &'a str) -> impl Iterator<Item = &'a Wallpaper> {
        self.wallpapers
            .iter()
            .filter(move |wallpaper| wallpaper.category == category)
    }
}

fn read_wallpaper(folder: &Path, id: &str, categories: &[Category]) -> Result<Wallpaper, String> {
    let file: WallpaperFile = read_json(&folder.join("wallpaper.json"))?;
    if !categories.iter().any(|c| c.id == file.category) {
        return Err(format!("category {:?} is not the catalog's", file.category));
    }
    if file.variants.is_empty() || file.variants.len() > MAX_VARIANTS {
        return Err("no variants, or too many".to_owned());
    }
    let mut variants: Vec<Variant> = Vec::new();
    for variant in file.variants {
        if !is_name(&variant.id) || variants.iter().any(|v| v.id == variant.id) {
            return Err(format!("variant {:?} is not one", variant.id));
        }
        // A file of the folder itself: a plain name with a picture's extension.
        let plain = !variant.file.is_empty()
            && !variant.file.contains('/')
            && !variant.file.starts_with('.')
            && Path::new(&variant.file)
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()));
        let path = folder.join(&variant.file);
        if !plain || !path.is_file() {
            return Err(format!(
                "variant {}'s file {:?} is not one",
                variant.id, variant.file
            ));
        }
        if let Some(sun) = variant.sun {
            if !(-90.0..=90.0).contains(&sun.elevation) || !(0.0..360.0).contains(&sun.azimuth) {
                return Err(format!("variant {}'s sun is not one", variant.id));
            }
        }
        variants.push(Variant {
            id: variant.id,
            name: variant.name,
            file: path,
            appearance: variant.appearance,
            sun: variant.sun,
        });
    }
    let fits = match file.automatic {
        Automatic::None => variants.len() == 1,
        Automatic::Appearance => [Appearance::Light, Appearance::Dark]
            .iter()
            .all(|a| variants.iter().filter(|v| v.appearance == Some(*a)).count() == 1),
        Automatic::Sun => variants.iter().all(|v| v.sun.is_some()),
    };
    if !fits {
        return Err(format!(
            "its variants are not what {:?} needs",
            file.automatic.name()
        ));
    }
    let preview = variants
        .iter()
        .position(|v| v.id == file.preview)
        .ok_or_else(|| format!("preview {:?} is none of its variants", file.preview))?;
    Ok(Wallpaper {
        id: id.to_owned(),
        name: file.name,
        category: file.category,
        automatic: file.automatic,
        preview,
        variants,
    })
}
