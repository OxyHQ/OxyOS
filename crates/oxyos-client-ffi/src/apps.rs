//! C ABI over `oxyos-apps` (docs/adr/0013-launching-applications.mdx): the applications the
//! session has, for the Bloom host run as the shell, which lists them in the launcher and the dock.

use super::fail;
use std::ffi::c_char;

/// `text` as a JSON string.
fn json_string(text: &str) -> String {
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

/// The session's applications as JSON: `[{"id", "name", "icon" (a file, or null), "pinned" (its
/// place in the dock from 1, or null)}]`, sorted by name, in the session's language.
pub fn applications_json() -> String {
    // The shell lists applications; what they may do is the session's to judge as it launches.
    let (applications, skipped) = oxyos_apps::registry(
        &oxyos_apps::directories(),
        &oxyos_apps::languages(),
        &|_| false,
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
