//! C ABI over `oxyos-intl`, what the `Intl` the Bloom host installs into Hermes formats with. It
//! lives in this library so that the host links one Rust library, not one per crate.
//!
//! Strings cross as a pointer and a length. Each result is written into the caller's buffer and
//! its length returned; a buffer too small is left alone, and the length tells the caller how much
//! to give next time. A failure returns -1, with `oxyos_last_error` giving the RangeError's message.

use super::fail;
use oxyos_intl::{Error, Part};
use std::ffi::c_char;

/// Between a part's type and its text, and between parts (ASCII's unit and record separators).
pub const UNIT_SEPARATOR: char = '\u{1f}';
pub const RECORD_SEPARATOR: char = '\u{1e}';

/// A caller's string; NULL with a length of 0 is empty.
///
/// # Safety
/// `data` is NULL with `length` 0, or points to `length` readable bytes that outlive `'a`.
#[allow(unsafe_code)]
unsafe fn text<'a>(data: *const c_char, length: usize, what: &str) -> Result<&'a str, Error> {
    if data.is_null() {
        return if length == 0 {
            Ok("")
        } else {
            Err(Error(format!("null {what}")))
        };
    }
    // SAFETY: per the contract, `data` points to `length` readable bytes.
    let bytes = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), length) };
    std::str::from_utf8(bytes).map_err(|_| Error(format!("{what} is not UTF-8")))
}

/// Writes `result` into `out` when it fits (with a NUL when there is room for one) and returns its
/// length; or -1 for a failure.
///
/// # Safety
/// `out` is NULL, or points to `capacity` writable bytes.
#[allow(unsafe_code)]
unsafe fn give(result: Result<String, Error>, out: *mut c_char, capacity: usize) -> i64 {
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            fail(error);
            return -1;
        }
    };
    let length = result.len();
    if !out.is_null() && length <= capacity {
        // SAFETY: per the contract, `out` has `capacity` writable bytes, at least `length`; the
        // result is a separate allocation.
        unsafe { std::ptr::copy_nonoverlapping(result.as_ptr(), out.cast::<u8>(), length) };
        if length < capacity {
            // SAFETY: `length` is within `capacity`.
            unsafe { *out.add(length) = 0 };
        }
    }
    i64::try_from(length).unwrap_or(i64::MAX)
}

/// Parts as `type\x1Fvalue`, joined by `\x1E`.
fn parts(parts: Vec<Part>) -> String {
    let parts: Vec<String> = parts
        .into_iter()
        .map(|part| format!("{}{UNIT_SEPARATOR}{}", part.kind, part.value))
        .collect();
    parts.join(&RECORD_SEPARATOR.to_string())
}

/// resolvedOptions as `name=value`, joined by `;`.
fn pairs(pairs: Vec<(&'static str, String)>) -> String {
    let pairs: Vec<String> = pairs
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    pairs.join(";")
}

/// The system's locale (`LC_ALL`, `LC_MESSAGES`, `LANG`) as a BCP 47 tag, `en-US` without one.
///
/// # Safety
/// `out` is NULL, or points to `capacity` writable bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_intl_default_locale(out: *mut c_char, capacity: usize) -> i64 {
    // SAFETY: forwarded caller contract.
    unsafe { give(Ok(oxyos_intl::default_locale()), out, capacity) }
}

/// `tag` canonicalized (`EN-us` as `en-US`); -1 for an invalid tag.
///
/// # Safety
/// `tag` points to `length` readable bytes (or is NULL with 0); `out` is NULL, or points to
/// `capacity` writable bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_intl_canonicalize(
    tag: *const c_char,
    length: usize,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    // SAFETY: forwarded caller contract.
    let result = unsafe { text(tag, length, "tag") }.and_then(oxyos_intl::canonicalize);
    // SAFETY: forwarded caller contract.
    unsafe { give(result, out, capacity) }
}

/// The locale to format in for `requested` (tags joined by `,`), as ECMA-402's lookup matcher
/// chooses it; -1 for an invalid tag.
///
/// # Safety
/// As `oxyos_intl_canonicalize`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_intl_resolve(
    requested: *const c_char,
    length: usize,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    // SAFETY: forwarded caller contract.
    let result = unsafe { text(requested, length, "locales") }.and_then(oxyos_intl::resolve);
    // SAFETY: forwarded caller contract.
    unsafe { give(result, out, capacity) }
}

/// A format's locale and options.
type Formatting<'a> = (&'a str, Vec<(&'a str, &'a str)>);

/// The locale and options of a format, as the caller's strings.
///
/// # Safety
/// Each points to its length's readable bytes (or is NULL with 0).
#[allow(unsafe_code)]
unsafe fn format<'a>(
    locale: *const c_char,
    locale_length: usize,
    options: *const c_char,
    options_length: usize,
) -> Result<Formatting<'a>, Error> {
    // SAFETY: forwarded caller contract.
    let locale = unsafe { text(locale, locale_length, "locale") }?;
    // SAFETY: forwarded caller contract.
    let options = unsafe { text(options, options_length, "options") }?;
    Ok((locale, oxyos_intl::options(options)?))
}

/// `Intl.DateTimeFormat`'s formatToParts of `epoch_ms` (milliseconds since the epoch, UTC).
///
/// # Safety
/// `locale` and `options` point to their lengths' readable bytes (or are NULL with 0); `out` is
/// NULL, or points to `capacity` writable bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_intl_format_date(
    locale: *const c_char,
    locale_length: usize,
    options: *const c_char,
    options_length: usize,
    epoch_ms: f64,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    // SAFETY: forwarded caller contract.
    let result = unsafe { format(locale, locale_length, options, options_length) }
        .and_then(|(locale, options)| oxyos_intl::date::format(locale, &options, epoch_ms))
        .map(parts);
    // SAFETY: forwarded caller contract.
    unsafe { give(result, out, capacity) }
}

/// `Intl.DateTimeFormat`'s resolvedOptions.
///
/// # Safety
/// As `oxyos_intl_format_date`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_intl_resolve_date(
    locale: *const c_char,
    locale_length: usize,
    options: *const c_char,
    options_length: usize,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    // SAFETY: forwarded caller contract.
    let result = unsafe { format(locale, locale_length, options, options_length) }
        .and_then(|(locale, options)| oxyos_intl::date::resolve(locale, &options))
        .map(pairs);
    // SAFETY: forwarded caller contract.
    unsafe { give(result, out, capacity) }
}

/// `Intl.NumberFormat`'s formatToParts of `value`.
///
/// # Safety
/// As `oxyos_intl_format_date`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_intl_format_number(
    locale: *const c_char,
    locale_length: usize,
    options: *const c_char,
    options_length: usize,
    value: f64,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    // SAFETY: forwarded caller contract.
    let result = unsafe { format(locale, locale_length, options, options_length) }
        .and_then(|(locale, options)| oxyos_intl::number::format(locale, &options, value))
        .map(parts);
    // SAFETY: forwarded caller contract.
    unsafe { give(result, out, capacity) }
}

/// `Intl.NumberFormat`'s resolvedOptions.
///
/// # Safety
/// As `oxyos_intl_format_date`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_intl_resolve_number(
    locale: *const c_char,
    locale_length: usize,
    options: *const c_char,
    options_length: usize,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    // SAFETY: forwarded caller contract.
    let result = unsafe { format(locale, locale_length, options, options_length) }
        .and_then(|(locale, options)| oxyos_intl::number::resolve(locale, &options))
        .map(pairs);
    // SAFETY: forwarded caller contract.
    unsafe { give(result, out, capacity) }
}
