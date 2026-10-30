//! C ABI for what an application has of its own on OxyOS, for the Bloom host: its key-value
//! storage (`oxyos-app-storage`, React Native's AsyncStorage, in the directory `apps.rs` gives
//! it), and what it asks of the machine: its connectivity (`oxyos-net`, NetInfo) and
//! random bytes and digests (expo-crypto).

use super::fail;
use oxyos_app_storage::Storage;
use oxyos_net::connectivity::Connectivity;
use rustix::fd::{AsFd, AsRawFd};
use serde_json::{json, Value};
use std::ffi::{c_char, c_int, CStr};

/// `text` written into `out` (`capacity` bytes, NUL-terminated when it fits); its length in bytes,
/// larger than `capacity` when it did not fit.
///
/// # Safety
/// `out` points to `capacity` writable bytes, or is null with `capacity` 0.
#[allow(unsafe_code)]
unsafe fn write_out(text: &str, out: *mut c_char, capacity: usize) -> i64 {
    if out.is_null() && capacity != 0 {
        return i64::from(fail("null buffer"));
    }
    let bytes = text.as_bytes();
    if bytes.len() < capacity {
        // SAFETY: `out` has `capacity` bytes, more than the text and its NUL.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.cast::<u8>(), bytes.len());
            *out.add(bytes.len()) = 0;
        }
    }
    bytes.len() as i64
}

/// A caller's NUL-terminated UTF-8 string.
///
/// # Safety
/// `text` is NULL or a NUL-terminated string.
#[allow(unsafe_code)]
unsafe fn utf8<'a>(text: *const c_char, what: &str) -> Result<&'a str, String> {
    if text.is_null() {
        return Err(format!("null {what}"));
    }
    // SAFETY: per the contract, a NUL-terminated string.
    unsafe { CStr::from_ptr(text) }
        .to_str()
        .map_err(|_| format!("{what} is not UTF-8"))
}

fn strings(request: &Value, key: &str) -> Result<Vec<String>, String> {
    request[key]
        .as_array()
        .ok_or(format!("{key} is not a list"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or(format!("a key in {key} is not a string"))
        })
        .collect()
}

fn pairs(request: &Value) -> Result<Vec<(String, String)>, String> {
    request["pairs"]
        .as_array()
        .ok_or("pairs is not a list")?
        .iter()
        .map(|pair| match pair.as_array().map(Vec::as_slice) {
            Some([Value::String(key), Value::String(value)]) => Ok((key.clone(), value.clone())),
            _ => Err("a pair is not a key and a value, both strings".to_string()),
        })
        .collect()
}

/// One storage request (see `oxyos_app_storage`), answered.
pub fn storage_answer(directory: &str, request: &str) -> Value {
    let answer = (|| -> Result<Value, String> {
        let request: Value = serde_json::from_str(request).map_err(|error| error.to_string())?;
        let storage = Storage::new(directory);
        let error = |error: oxyos_app_storage::Error| error.to_string();
        Ok(match request["op"].as_str() {
            Some("get") => {
                let keys = strings(&request, "keys")?;
                let values = storage.get(&keys).map_err(error)?;
                Value::Array(
                    keys.into_iter()
                        .zip(values)
                        .map(|(k, v)| json!([k, v]))
                        .collect(),
                )
            }
            Some("set") => storage
                .set(&pairs(&request)?)
                .map(|()| Value::Null)
                .map_err(error)?,
            Some("merge") => storage
                .merge(&pairs(&request)?)
                .map(|()| Value::Null)
                .map_err(error)?,
            Some("remove") => storage
                .remove(&strings(&request, "keys")?)
                .map(|()| Value::Null)
                .map_err(error)?,
            Some("keys") => json!(storage.keys().map_err(error)?),
            Some("clear") => storage.clear().map(|()| Value::Null).map_err(error)?,
            _ => return Err("unknown storage operation".into()),
        })
    })();
    match answer {
        Ok(value) => json!({ "ok": value }),
        Err(error) => json!({ "error": error }),
    }
}

/// A request to the application's storage in `directory` (its data directory), as JSON:
/// `{"op": "get" | "remove", "keys": [...]}`, `{"op": "set" | "merge", "pairs": [[key, value]...]}`,
/// `{"op": "keys"}` or `{"op": "clear"}`. Answered as JSON, `{"ok": ...}` (`get`: `[[key, value or
/// null]...]`; `keys`: the keys; the rest null) or `{"error": "why"}`, written as
/// `oxyos_applications` writes. A read asked again with more room reads again; a change answers
/// `{"ok":null}` or a short error, so 4096 bytes always hold it.
///
/// # Safety
/// `directory` and `request` are NUL-terminated strings; `out` as [`write_out`].
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_app_storage(
    directory: *const c_char,
    request: *const c_char,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    // SAFETY: forwarded caller contract.
    let directory = unsafe { utf8(directory, "directory") };
    // SAFETY: forwarded caller contract.
    let request = unsafe { utf8(request, "request") };
    let (directory, request) = match (directory, request) {
        (Ok(directory), Ok(request)) => (directory, request),
        (Err(error), _) | (_, Err(error)) => return i64::from(fail(error)),
    };
    let answer = storage_answer(directory, request).to_string();
    // SAFETY: forwarded caller contract.
    unsafe { write_out(&answer, out, capacity) }
}

/// The machine's connectivity as it changes (`oxyos_net::connectivity`).
pub struct OxyOSConnectivity {
    connectivity: Connectivity,
}

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn oxyos_connectivity_new() -> *mut OxyOSConnectivity {
    Box::into_raw(Box::new(OxyOSConnectivity {
        connectivity: Connectivity::new(),
    }))
}

/// Frees it. NULL is ignored.
///
/// # Safety
/// `connectivity` is NULL or a pointer from `oxyos_connectivity_new` not yet freed.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_connectivity_free(connectivity: *mut OxyOSConnectivity) {
    if !connectivity.is_null() {
        // SAFETY: the pointer came from Box::into_raw in oxyos_connectivity_new and is freed once.
        drop(unsafe { Box::from_raw(connectivity) });
    }
}

/// Readable when a link, an address or a route changed; -1 where changes cannot be watched.
///
/// # Safety
/// `connectivity` is a live pointer from `oxyos_connectivity_new`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_connectivity_fd(connectivity: *mut OxyOSConnectivity) -> c_int {
    // SAFETY: per the contract, a live pointer.
    match unsafe { connectivity.as_ref() } {
        Some(connectivity) => connectivity
            .connectivity
            .fd()
            .map_or(-1, |fd| fd.as_fd().as_raw_fd()),
        None => fail("null connectivity"),
    }
}

/// Takes what the kernel said and reads the connectivity again: 1 if it changed, else 0.
///
/// # Safety
/// `connectivity` is a live pointer from `oxyos_connectivity_new`, used by one thread at a time.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_connectivity_refresh(connectivity: *mut OxyOSConnectivity) -> c_int {
    // SAFETY: per the contract, a live, exclusively used pointer.
    match unsafe { connectivity.as_mut() } {
        Some(connectivity) => c_int::from(connectivity.connectivity.refresh()),
        None => fail("null connectivity"),
    }
}

/// The connectivity as JSON, `{"type": "none" | "wifi" | "ethernet" | "cellular" | "vpn" |
/// "other", "isConnected", "interface" (or null)}`, written as `oxyos_applications` writes.
///
/// # Safety
/// `connectivity` is a live pointer from `oxyos_connectivity_new`; `out` as [`write_out`].
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_connectivity_state(
    connectivity: *mut OxyOSConnectivity,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    // SAFETY: per the contract, a live pointer.
    let Some(connectivity) = (unsafe { connectivity.as_ref() }) else {
        return i64::from(fail("null connectivity"));
    };
    let state = connectivity.connectivity.state();
    let answer = json!({
        "type": state.kind.name(),
        "isConnected": state.connected(),
        "interface": state.interface,
    })
    .to_string();
    // SAFETY: forwarded caller contract.
    unsafe { write_out(&answer, out, capacity) }
}

/// Fills `out` with `length` bytes from the kernel's CSPRNG (`getrandom(2)`). 0, or OXYOS_ERROR.
///
/// # Safety
/// `out` points to `length` writable bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_random_bytes(out: *mut u8, length: usize) -> c_int {
    if out.is_null() {
        return if length == 0 { 0 } else { fail("null buffer") };
    }
    // SAFETY: per the contract, `out` has `length` writable bytes.
    let buffer = unsafe { std::slice::from_raw_parts_mut(out, length) };
    let mut filled = 0;
    while filled < length {
        match rustix::rand::getrandom(&mut buffer[filled..], rustix::rand::GetRandomFlags::empty())
        {
            Ok(n) => filled += n,
            Err(rustix::io::Errno::INTR) => {}
            Err(error) => return fail(format!("getrandom: {error}")),
        }
    }
    0
}

/// The digest of `length` bytes at `data` with `algorithm` (`SHA-1`, `SHA-256`, `SHA-384` or
/// `SHA-512`, as expo-crypto names them), written into `out` (`capacity` bytes). Its length in
/// bytes (larger than `capacity`: nothing written), or OXYOS_ERROR for an algorithm OxyOS does not
/// provide.
///
/// # Safety
/// `algorithm` is a NUL-terminated string; `data` points to `length` readable bytes (NULL with 0);
/// `out` to `capacity` writable bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_digest(
    algorithm: *const c_char,
    data: *const u8,
    length: usize,
    out: *mut u8,
    capacity: usize,
) -> i64 {
    // SAFETY: forwarded caller contract.
    let name = match unsafe { utf8(algorithm, "algorithm") } {
        Ok(name) => name,
        Err(error) => return i64::from(fail(error)),
    };
    let Some(algorithm) = digest_algorithm(name) else {
        return i64::from(fail(format!("OxyOS does not provide the {name} digest")));
    };
    let bytes = if data.is_null() {
        if length != 0 {
            return i64::from(fail("null data"));
        }
        &[][..]
    } else {
        // SAFETY: per the contract, `data` has `length` readable bytes.
        unsafe { std::slice::from_raw_parts(data, length) }
    };
    let digest = ring::digest::digest(algorithm, bytes);
    let digest = digest.as_ref();
    if digest.len() <= capacity && !out.is_null() {
        // SAFETY: `out` has `capacity` bytes, at least the digest's.
        unsafe { std::ptr::copy_nonoverlapping(digest.as_ptr(), out, digest.len()) };
    }
    digest.len() as i64
}

/// expo-crypto's names of the digests OxyOS computes (ring's). MD2, MD4 and MD5 are not among them.
pub fn digest_algorithm(name: &str) -> Option<&'static ring::digest::Algorithm> {
    match name {
        "SHA-1" => Some(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY),
        "SHA-256" => Some(&ring::digest::SHA256),
        "SHA-384" => Some(&ring::digest::SHA384),
        "SHA-512" => Some(&ring::digest::SHA512),
        _ => None,
    }
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn storage_requests_are_answered_as_json() {
        let dir = std::env::temp_dir().join(format!("oxyos-ffi-storage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let directory = dir.to_str().expect("utf8");
        assert_eq!(
            storage_answer(directory, r#"{"op":"set","pairs":[["a","1"],["b","2"]]}"#),
            json!({"ok": null})
        );
        assert_eq!(
            storage_answer(directory, r#"{"op":"get","keys":["a","c"]}"#),
            json!({"ok": [["a", "1"], ["c", null]]})
        );
        assert_eq!(
            storage_answer(directory, r#"{"op":"keys"}"#),
            json!({"ok": ["a", "b"]})
        );
        assert!(
            storage_answer(directory, r#"{"op":"merge","pairs":[["a","{}"]]}"#)["error"]
                .is_string()
        );
        assert!(
            storage_answer(directory, r#"{"op":"set","pairs":[["a",1]]}"#)["error"].is_string()
        );
        assert!(storage_answer(directory, r#"{"op":"drop"}"#)["error"].is_string());
        // Through the C ABI: the answer and its length.
        let (dir_c, request) = (
            CString::new(directory).expect("c"),
            CString::new(r#"{"op":"clear"}"#).expect("c"),
        );
        let mut out = [0 as c_char; 64];
        // SAFETY: NUL-terminated strings and a buffer of its length.
        let length = unsafe {
            oxyos_app_storage(
                dir_c.as_ptr(),
                request.as_ptr(),
                out.as_mut_ptr(),
                out.len(),
            )
        };
        // SAFETY: written NUL-terminated, as it fit.
        let answer = unsafe { CStr::from_ptr(out.as_ptr()) };
        assert_eq!(answer.to_str().expect("utf8"), r#"{"ok":null}"#);
        assert_eq!(length, 11);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn digests_are_fips_180s_and_md5_is_refused() {
        let mut out = [0u8; 64];
        let digest = |name: &str, out: &mut [u8]| {
            let name = CString::new(name).expect("c");
            // SAFETY: a NUL-terminated name, 3 readable bytes and a buffer of its length.
            unsafe {
                oxyos_digest(
                    name.as_ptr(),
                    b"abc".as_ptr(),
                    3,
                    out.as_mut_ptr(),
                    out.len(),
                )
            }
        };
        assert_eq!(digest("SHA-256", &mut out), 32);
        assert_eq!(
            out[..32]
                .iter()
                .fold(String::new(), |hex, b| hex + &format!("{b:02x}")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(digest("SHA-1", &mut out), 20);
        assert_eq!(digest("SHA-384", &mut out), 48);
        assert_eq!(digest("SHA-512", &mut out), 64);
        assert_eq!(
            digest("SHA-512", &mut out[..10]),
            64,
            "too small: its length, nothing written"
        );
        assert_eq!(digest("MD5", &mut out), i64::from(crate::OXYOS_ERROR));
    }

    #[test]
    fn random_bytes_fill_the_buffer() {
        let (mut a, mut b) = ([0u8; 64], [0u8; 64]);
        // SAFETY: buffers of their lengths.
        assert_eq!(unsafe { oxyos_random_bytes(a.as_mut_ptr(), a.len()) }, 0);
        // SAFETY: as above.
        assert_eq!(unsafe { oxyos_random_bytes(b.as_mut_ptr(), b.len()) }, 0);
        assert_ne!(a, b);
    }
}
