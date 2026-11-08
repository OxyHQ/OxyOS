//! C ABI over `oxyos-wallpaper` (docs/adr/0048-wallpapers.mdx): what a display shows for the
//! person's choice, for the Bloom host run as the shell; the catalog, thumbnails and a folder's
//! images, for Settings. Each writes JSON into the caller's buffer as `oxyos_applications` does.

use super::fail;
use oxyos_preferences::wallpaper::Choice;
use oxyos_wallpaper::{catalog::Catalog, file_uri, path_of, thumbnail, Moment};
use serde_json::{json, Value};
use std::ffi::{c_char, CStr};

/// `json` into `out` (`capacity` bytes, NUL-terminated when it fits): its length in bytes.
///
/// # Safety
/// `out` points to `capacity` writable bytes, or is null with `capacity` 0.
#[allow(unsafe_code)]
unsafe fn written(json: &Value, out: *mut c_char, capacity: usize) -> i64 {
    let text = json.to_string();
    let bytes = text.as_bytes();
    if out.is_null() && capacity != 0 {
        return i64::from(fail("null buffer"));
    }
    if bytes.len() < capacity {
        // SAFETY: `out` has `capacity` bytes, more than the JSON and its NUL.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.cast::<u8>(), bytes.len());
            *out.add(bytes.len()) = 0;
        }
    }
    bytes.len() as i64
}

/// A NUL-terminated UTF-8 string, or none.
///
/// # Safety
/// `text` is null or a NUL-terminated string.
#[allow(unsafe_code)]
unsafe fn text<'a>(text: *const c_char) -> Option<&'a str> {
    if text.is_null() {
        return None;
    }
    // SAFETY: per the contract.
    unsafe { CStr::from_ptr(text) }.to_str().ok()
}

/// What a display shows for `choice` (a choice's text, docs/adr/0048-wallpapers.mdx) on
/// `output` at `now`, the session having started at `login` (unix seconds), in the appearance
/// `dark`, from the system's catalog (`OXYOS_WALLPAPERS`, else the image's), the sun where the
/// system's time zone is: `{"key", "uri" (a file:// URI or null), "fit", "color" ("#rrggbb"),
/// "next" (unix seconds or null)}`. A choice that is not one shows the default.
///
/// # Safety
/// `choice` is a NUL-terminated string; `out` points to `capacity` writable bytes, or is null
/// with `capacity` 0.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_wallpaper_show(
    choice: *const c_char,
    output: u32,
    now: i64,
    login: i64,
    dark: bool,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    // SAFETY: per the contract.
    let choice = unsafe { text(choice) }
        .and_then(|text| Choice::parse(text).ok())
        .unwrap_or_default();
    let moment = Moment {
        now,
        login,
        dark,
        output,
        place: oxyos_wallpaper::place(),
    };
    let shown = oxyos_wallpaper::show(&Catalog::system(), &choice, &moment);
    let json = json!({
        "key": shown.key,
        "uri": shown.uri(),
        "fit": shown.fit.name(),
        "color": format!("#{}", shown.color),
        "next": shown.next,
    });
    // SAFETY: per the contract.
    unsafe { written(&json, out, capacity) }
}

/// The catalog as Settings shows it, its names in `language` (`en`, `es`; else English):
/// `{"default", "categories": [{"id", "name", "wallpapers": [{"id", "name", "automatic",
/// "preview", "variants": [{"id", "name", "preview"}]}]}], "colors": [{"color", "name"}]}`.
///
/// # Safety
/// `language` is null or a NUL-terminated string; `out` as `oxyos_wallpaper_show`'s.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_wallpaper_catalog(
    language: *const c_char,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    // SAFETY: per the contract.
    let language = unsafe { text(language) }.unwrap_or("en");
    let json = catalog_json(&Catalog::system(), language);
    // SAFETY: per the contract.
    unsafe { written(&json, out, capacity) }
}

pub(crate) fn catalog_json(catalog: &Catalog, language: &str) -> Value {
    let categories: Vec<Value> = catalog
        .categories
        .iter()
        .map(|category| {
            let wallpapers: Vec<Value> = catalog
                .in_category(&category.id)
                .map(|wallpaper| {
                    let variants: Vec<Value> = wallpaper
                        .variants
                        .iter()
                        .map(|variant| {
                            json!({
                                "id": variant.id,
                                "name": variant.name.get(language),
                                "preview": file_uri(&variant.file),
                            })
                        })
                        .collect();
                    json!({
                        "id": wallpaper.id,
                        "name": wallpaper.name.get(language),
                        "automatic": wallpaper.automatic.name(),
                        "preview": file_uri(&wallpaper.variants[wallpaper.preview].file),
                        "variants": variants,
                    })
                })
                .collect();
            json!({
                "id": category.id,
                "name": category.name.get(language),
                "wallpapers": wallpapers,
            })
        })
        .collect();
    let colors: Vec<Value> = catalog
        .colors
        .iter()
        .map(|color| json!({ "color": format!("#{}", color.color), "name": color.name.get(language) }))
        .collect();
    json!({
        "default": catalog.default,
        "categories": categories,
        "colors": colors,
    })
}

/// A thumbnail of the image at `uri` (a `file://` URI or a path), `width` × `height` pixels as
/// `cover` draws it, from the cache (`$XDG_CACHE_HOME/oxyos/wallpapers/thumbnails`), made now if
/// it is not there; an SVG's own URI: `{"uri"}`. OXYOS_ERROR where it cannot be made (see
/// `oxyos_last_error`).
///
/// # Safety
/// `uri` is a NUL-terminated string; `out` as `oxyos_wallpaper_show`'s.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_wallpaper_thumbnail(
    uri: *const c_char,
    width: u32,
    height: u32,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    // SAFETY: per the contract.
    let Some(path) = unsafe { text(uri) }.and_then(path_of) else {
        return i64::from(fail("not a file:// URI"));
    };
    let Some(cache) = thumbnail::directory() else {
        return i64::from(fail("no cache directory"));
    };
    match thumbnail::thumbnail(&path, width, height, &cache) {
        // SAFETY: per the contract.
        Ok(made) => unsafe { written(&json!({ "uri": file_uri(&made) }), out, capacity) },
        Err(error) => i64::from(fail(error)),
    }
}

/// A folder's images in the order a shuffle takes them (by name), at most 1000:
/// `{"images": ["file://…"]}`; none for what is not a folder.
///
/// # Safety
/// `path` is a NUL-terminated string (a path or a `file://` URI); `out` as
/// `oxyos_wallpaper_show`'s.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_wallpaper_folder(
    path: *const c_char,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    // SAFETY: per the contract.
    let Some(folder) = unsafe { text(path) }.and_then(path_of) else {
        return i64::from(fail("not a path"));
    };
    let images: Vec<String> = oxyos_wallpaper::folder_images(&folder)
        .iter()
        .map(|image| file_uri(image))
        .collect();
    // SAFETY: per the contract.
    unsafe { written(&json!({ "images": images }), out, capacity) }
}
