//! C ABI over `oxyos-apps` (docs/adr/0013-launching-applications.mdx): the applications the
//! session has, for the Bloom host run as the shell, which lists them in the launcher and the dock.

use super::fail;
use std::collections::HashMap;
use std::ffi::{c_char, CStr};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

/// What each icon's picture was measured as (`oxyos_image::shape`), by its file, with the file's
/// length and time it was measured at: a picture is decoded and measured once, again only when its
/// file changes.
#[allow(clippy::type_complexity)]
static SHAPES: Mutex<Option<HashMap<PathBuf, (u64, Option<SystemTime>, Option<f64>)>>> =
    Mutex::new(None);

/// The share of the icon's picture its own body takes when the picture is its own shape (a
/// rounded square, transparent around it: Dropsea's), else None: an SVG (drawn on the icon grid),
/// a picture that is not, or one that cannot be read.
pub(crate) fn icon_body(icon: &Path) -> Option<f64> {
    if icon.extension().is_some_and(|extension| extension == "svg") {
        return None;
    }
    let metadata = std::fs::metadata(icon).ok()?;
    let stamp = (metadata.len(), metadata.modified().ok());
    let mut shapes = SHAPES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let shapes = shapes.get_or_insert_with(HashMap::new);
    if let Some(&(length, modified, body)) = shapes.get(icon) {
        if (length, modified) == stamp {
            return body;
        }
    }
    let body = oxyos_image::shape::own_shape_of_file(icon).ok().flatten();
    shapes.insert(icon.to_path_buf(), (stamp.0, stamp.1, body));
    body
}

/// `text` as a JSON string.
pub(crate) fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// What the shell lists from, as the session launches from it: the image's system applications
/// first, which no other source's entry can take the place of (docs/adr/0030-installing-
/// applications.mdx, decision 7), the install service's entries for this account, then the
/// development directories. What they may do is the session's to judge as it launches.
struct Session {
    policy: oxyos_apps::Policy,
    installed: Option<std::path::PathBuf>,
    directories: Vec<std::path::PathBuf>,
    /// In the development compositor alone, its fixture of the install service's state directory
    /// (`OXYOS_INSTALLED_FIXTURE`, as a scenario wrote it), whose entries are taken as the
    /// service's. It changes only what this shell lists: what is launched, and as what, is the
    /// session's to judge from its own source.
    fixture: Option<std::path::PathBuf>,
}

impl Session {
    fn read() -> Self {
        let policy = oxyos_apps::Policy::system().unwrap_or_else(|error| {
            eprintln!("[apps] system applications: {error}");
            oxyos_apps::Policy::none()
        });
        let fixture = std::env::var_os("OXYOS_INSTALLED_FIXTURE").map(std::path::PathBuf::from);
        let installed = oxyos_apps::session_user().map(|user| {
            let root = fixture
                .clone()
                .unwrap_or_else(|| std::path::PathBuf::from(oxyos_apps::INSTALLED));
            oxyos_apps::installed_applications(&root, &user)
        });
        Self {
            policy,
            installed,
            directories: oxyos_apps::directories(),
            fixture,
        }
    }

    /// Root's (the session's rule), or the development compositor's fixture's.
    fn trusted(&self, path: &std::path::Path) -> bool {
        self.fixture
            .as_ref()
            .is_some_and(|root| path.starts_with(root))
            || oxyos_apps::administrator_written(path)
    }
}

/// A watch over what the shell lists from (`oxyos_apps::Watch`): readable when an application may
/// have been installed, updated or removed.
pub struct OxyOSApplicationsWatch {
    watch: oxyos_apps::Watch,
}

/// A watch over the session's applications' sources; NULL on failure (oxyos_last_error).
#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn oxyos_applications_watch_new() -> *mut OxyOSApplicationsWatch {
    let session = Session::read();
    let sources = oxyos_apps::sources(
        &session.directories,
        &session.policy,
        session.installed.as_deref(),
    );
    match oxyos_apps::Watch::new(sources) {
        Ok(watch) => Box::into_raw(Box::new(OxyOSApplicationsWatch { watch })),
        Err(error) => {
            fail(error);
            std::ptr::null_mut()
        }
    }
}

/// Frees the watch. NULL is ignored.
///
/// # Safety
/// `watch` is NULL or a pointer from `oxyos_applications_watch_new` not yet freed; it is invalid
/// after.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_applications_watch_free(watch: *mut OxyOSApplicationsWatch) {
    if !watch.is_null() {
        // SAFETY: the pointer came from Box::into_raw in oxyos_applications_watch_new, freed once.
        drop(unsafe { Box::from_raw(watch) });
    }
}

/// The watch's descriptor, readable when the applications may have changed (for poll).
///
/// # Safety
/// `watch` is a live pointer from `oxyos_applications_watch_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_applications_watch_fd(
    watch: *mut OxyOSApplicationsWatch,
) -> std::ffi::c_int {
    use rustix::fd::AsRawFd;
    // SAFETY: forwarded caller contract.
    match unsafe { watch.as_ref() } {
        Some(watch) => watch.watch.fd().as_raw_fd(),
        None => fail("null watch"),
    }
}

/// 1 when the applications may have changed since the last call (oxyos_applications then lists
/// them as they are now), 0 otherwise; what the descriptor held is read.
///
/// # Safety
/// `watch` is a live pointer from `oxyos_applications_watch_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_applications_watch_changed(
    watch: *mut OxyOSApplicationsWatch,
) -> u8 {
    // SAFETY: forwarded caller contract.
    match unsafe { watch.as_mut() } {
        Some(watch) => u8::from(watch.watch.changed()),
        None => 0,
    }
}

/// The session's applications as JSON: `[{"id", "name", "icon" (a file, or null), "iconBody" (the
/// share of the icon's picture its own body takes when the picture is its own shape, which the
/// shell draws as it is; null for any other, which it puts in the icon grid's body), "pinned" (its
/// place in the dock from 1, or null), "opens" (the media types its entry declares it opens, its
/// `MimeType`: what files dropped on its dock icon it takes, docs/adr/0034-the-dock.mdx)}]`, sorted
/// by name, in the session's language: `Intl`'s
/// default locale, which the host keeps the session's (docs/adr/0040-language-and-region.mdx).
pub fn applications_json() -> String {
    let session = Session::read();
    let trusted = |path: &std::path::Path| session.trusted(path);
    let (applications, skipped) = oxyos_apps::registry(
        &session.directories,
        &languages_of_tag(&oxyos_intl::default_locale()),
        &session.policy,
        session.installed.as_deref(),
        &trusted,
    );
    for line in skipped {
        eprintln!("[apps] {line}");
    }
    let items: Vec<String> = applications
        .iter()
        .map(|app| {
            format!(
                "{{\"id\":{},\"name\":{},\"icon\":{},\"iconBody\":{},\"pinned\":{},\"opens\":[{}]}}",
                json_string(&app.id),
                json_string(&app.name),
                app.icon
                    .as_ref()
                    .map_or("null".into(), |icon| json_string(&icon.to_string_lossy())),
                app.icon
                    .as_deref()
                    .and_then(icon_body)
                    .map_or("null".into(), |body| format!("{body}")),
                app.pinned.map_or("null".into(), |place| place.to_string()),
                app.opens
                    .iter()
                    .map(|media| json_string(media))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// The session's applications as JSON (see `applications_json`), written into `out` (`capacity`
/// bytes, NUL-terminated when it fits). Returns the JSON's length in bytes: larger than
/// `capacity`, call again with more room.
///
/// # Safety
/// `out` points to `capacity` writable bytes, or is null with `capacity` 0.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_applications(out: *mut c_char, capacity: usize) -> i64 {
    let json = applications_json();
    let bytes = json.as_bytes();
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

/// An application's own directory (`oxyos_apps::data_directory` or `cache_directory`, as
/// `which` is 0 or 1) by its id, the one the session gave it in `OXYOS_APP_ID`
/// (docs/adr/0030-installing-applications.mdx), written into `out` as `oxyos_applications`
/// writes; -1 for an id that is not one or without a home (see `oxyos_last_error`).
///
/// # Safety
/// `id` is a NUL-terminated string; `out` points to `capacity` writable bytes, or is null with
/// `capacity` 0.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_application_directory(
    id: *const c_char,
    which: u8,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    if id.is_null() {
        return i64::from(fail("null application id"));
    }
    // SAFETY: a NUL-terminated string (contract above).
    let Ok(id) = unsafe { CStr::from_ptr(id) }.to_str() else {
        return i64::from(fail("the application id is not UTF-8"));
    };
    let directory = match which {
        0 => oxyos_apps::data_directory(id),
        1 => oxyos_apps::cache_directory(id),
        _ => return i64::from(fail("no such directory")),
    };
    let Some(directory) = directory.as_ref().and_then(|dir| dir.to_str()) else {
        fail(format!(
            "{id} has no directory: not an application id, or no home"
        ));
        return -1;
    };
    if out.is_null() && capacity != 0 {
        return i64::from(fail("null buffer"));
    }
    let bytes = directory.as_bytes();
    if bytes.len() < capacity {
        // SAFETY: `out` has `capacity` bytes, more than the path and its NUL.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.cast::<u8>(), bytes.len());
            *out.add(bytes.len()) = 0;
        }
    }
    i64::try_from(bytes.len()).unwrap_or(i64::MAX)
}

/// The languages an entry's localized names are looked up by (`Name[es_ES]`, then `Name[es]`) for
/// a BCP 47 tag: `es-ES-u-hc-h23` as `es_ES`, `es`.
fn languages_of_tag(tag: &str) -> Vec<String> {
    let mut subtags = tag.split('-').take_while(|subtag| subtag.len() > 1);
    let language = subtags.next().unwrap_or_default();
    let region =
        subtags.find(|subtag| subtag.len() == 2 && subtag.chars().all(|c| c.is_ascii_uppercase()));
    let posix = match region {
        Some(region) => format!("{language}_{region}"),
        None => language.to_owned(),
    };
    oxyos_apps::languages_of(&posix)
}

#[cfg(test)]
mod locale_tests {
    #[test]
    fn languages_of_a_tag() {
        assert_eq!(super::languages_of_tag("es-ES-u-hc-h23"), ["es_ES", "es"]);
        assert_eq!(super::languages_of_tag("en-US"), ["en_US", "en"]);
        assert_eq!(super::languages_of_tag("sr-Latn-RS"), ["sr_RS", "sr"]);
        assert_eq!(super::languages_of_tag("es"), ["es"]);
    }
}
