//! C ABI over `oxyos-apps` (docs/adr/0013-launching-applications.mdx): the applications the
//! session has, for the Bloom host run as the shell, which lists them in the launcher and the dock.

use super::fail;
use std::ffi::{c_char, CStr};

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

/// The session's applications as JSON: `[{"id", "name", "icon" (a file, or null), "pinned" (its
/// place in the dock from 1, or null)}]`, sorted by name, in the session's language.
pub fn applications_json() -> String {
    let session = Session::read();
    let trusted = |path: &std::path::Path| session.trusted(path);
    let (applications, skipped) = oxyos_apps::registry(
        &session.directories,
        &oxyos_apps::languages(),
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
                "{{\"id\":{},\"name\":{},\"icon\":{},\"pinned\":{}}}",
                json_string(&app.id),
                json_string(&app.name),
                app.icon
                    .as_ref()
                    .map_or("null".into(), |icon| json_string(&icon.to_string_lossy())),
                app.pinned.map_or("null".into(), |place| place.to_string())
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
