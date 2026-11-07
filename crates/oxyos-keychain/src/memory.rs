//! Keeping the keychain's keys out of swap (OxyOS ADR 0050 decision 2): `mlock(2)` on the pages a
//! key occupies, while the service holds it.
//!
//! A key's holder pins its own buffer ([`Pinned`]) and keeps the pin as a field declared after the
//! buffer, so that dropping it overwrites the key first and lets the page go after. Pages are
//! counted: two keys on one page keep it locked until both are gone, since `munlock` does not
//! nest. Locking can fail (the process's `RLIMIT_MEMLOCK`, which the service's unit raises); a
//! holder told so fails closed: the service never holds a key it could not lock.
//!
//! Only the long-lived keys are pinned (the keybag key, the vault key's generations, the
//! member's keys, a recovery key while it is shown). Plaintexts a request decrypts live briefly in
//! buffers that are overwritten when dropped, and are not locked.

use crate::{Error, Result};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// How many pins each locked page has.
fn pages() -> &'static Mutex<HashMap<usize, usize>> {
    static PAGES: OnceLock<Mutex<HashMap<usize, usize>>> = OnceLock::new();
    PAGES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The pages a buffer occupies, locked while this lives.
#[derive(Debug)]
pub struct Pinned {
    pages: Vec<usize>,
}

fn page_size() -> usize {
    rustix::param::page_size()
}

fn lock_page(page: usize) -> std::io::Result<()> {
    #[allow(unsafe_code)]
    // SAFETY: mlock(2) reads and writes no memory of the caller's: it marks the page at `page`
    // (page aligned, one page long, part of a live allocation the caller holds) resident.
    unsafe {
        rustix::mm::mlock(page as *mut std::ffi::c_void, page_size())?;
    }
    Ok(())
}

fn unlock_page(page: usize) {
    #[allow(unsafe_code)]
    // SAFETY: munlock(2) reads and writes no memory; it only lets the page be swapped again. The
    // page may since have been freed: munlock of an unmapped range fails, harmlessly.
    unsafe {
        let _ = rustix::mm::munlock(page as *mut std::ffi::c_void, page_size());
    }
}

impl Pinned {
    /// Locks the pages of `bytes` in memory.
    pub fn new(bytes: &[u8]) -> Result<Self> {
        let size = page_size();
        let start = bytes.as_ptr() as usize & !(size - 1);
        let end = (bytes.as_ptr() as usize + bytes.len().max(1)).div_ceil(size) * size;
        let mut counted = pages().lock().unwrap_or_else(|p| p.into_inner());
        let mut pinned = Self { pages: Vec::new() };
        for page in (start..end).step_by(size) {
            let count = counted.entry(page).or_insert(0);
            if *count == 0 {
                if let Err(error) = lock_page(page) {
                    counted.remove(&page);
                    drop(counted);
                    // What this call pinned is let go by dropping it.
                    drop(pinned);
                    return Err(Error::Memory(error));
                }
            }
            *count += 1;
            pinned.pages.push(page);
        }
        Ok(pinned)
    }

    /// Locks the pages of a value (its bytes where it lives: a boxed key's heap buffer).
    pub fn of<T>(value: &T) -> Result<Self> {
        let pointer = value as *const T as *const u8;
        let length = std::mem::size_of::<T>();
        #[allow(unsafe_code)]
        // SAFETY: `value` is a live reference to `length` bytes; they are only read as a slice for
        // their address and length (Pinned::new never reads its contents).
        let bytes = unsafe { std::slice::from_raw_parts(pointer, length) };
        Self::new(bytes)
    }
}

impl Drop for Pinned {
    fn drop(&mut self) {
        let mut counted = pages().lock().unwrap_or_else(|p| p.into_inner());
        for page in self.pages.drain(..) {
            if let Some(count) = counted.get_mut(&page) {
                *count -= 1;
                if *count == 0 {
                    counted.remove(&page);
                    unlock_page(page);
                }
            }
        }
    }
}

/// How many pages are locked for the keychain now (for the tests).
pub fn locked_pages() -> usize {
    pages().lock().unwrap_or_else(|p| p.into_inner()).len()
}

/// Whether `page`'s first byte is locked, as `/proc/self/smaps` says (for the tests).
#[cfg(test)]
pub(crate) fn is_locked(address: usize) -> bool {
    let smaps = std::fs::read_to_string("/proc/self/smaps").unwrap();
    let mut inside = false;
    for line in smaps.lines() {
        if let Some((range, _)) = line.split_once(' ') {
            if let Some((from, to)) = range.split_once('-') {
                if let (Ok(from), Ok(to)) = (
                    usize::from_str_radix(from, 16),
                    usize::from_str_radix(to, 16),
                ) {
                    inside = (from..to).contains(&address);
                    continue;
                }
            }
        }
        if inside {
            if let Some(flags) = line.strip_prefix("VmFlags:") {
                return flags.split_whitespace().any(|flag| flag == "lo");
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_stays_locked_while_any_pin_holds_it() {
        // A page of its own (a large boxed buffer), so other tests' pins do not share it.
        let buffer = vec![7u8; 3 * page_size()];
        let middle = &buffer[page_size()..page_size() + 64];
        let page = middle.as_ptr() as usize & !(page_size() - 1);
        let first = match Pinned::new(middle) {
            Ok(pinned) => pinned,
            Err(error) => {
                eprintln!("skipped: this process may lock no memory ({error})");
                return;
            }
        };
        assert!(is_locked(page));
        let second = Pinned::new(&buffer[page_size() + 100..page_size() + 200]).unwrap();
        drop(first);
        assert!(is_locked(page), "still pinned by the second");
        drop(second);
        assert!(!is_locked(page));
    }
}
