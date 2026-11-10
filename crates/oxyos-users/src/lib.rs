//! The running system's users and groups, as every OxyOS program looks them up: through the Name
//! Service Switch (glibc's `getpwnam_r`, `getpwuid_r`, `getgrnam_r`, `getgrouplist`), so a
//! program sees the users `id(1)`, `getent(1)`, PAM and logind see, from whatever sources
//! `nsswitch.conf` names (`files` for `/etc/passwd`, `systemd` for the users it makes), never a
//! file read by hand that another source would contradict.
//!
//! Only the running system is looked up here. The account database of *another* root (an install
//! target, a backup restored with `useradd --prefix`, a test's root) is that root's files, which
//! the programs that work on such a root read themselves (`oxyos_account::users`'s `passwd_entry`
//! and `group_members`); NSS would answer for this system instead.
//!
//! Each answer is read when asked, as the database changes while a service runs (`useradd`, a
//! setup's new account). An entry larger than the buffer is read again with a larger one.

use std::ffi::{CStr, CString};
use std::io;
use std::path::PathBuf;

/// A user of the running system's account database (passwd(5)'s fields but the password's).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    /// passwd's GECOS field: the full name, then other fields after commas.
    pub gecos: String,
    pub home: PathBuf,
    pub shell: PathBuf,
}

/// The user named `name`, if the account database has one. A name no C string can hold (a NUL)
/// names nobody.
pub fn user_named(name: &str) -> io::Result<Option<User>> {
    let Ok(c_name) = CString::new(name) else {
        return Ok(None);
    };
    passwd(Key::Name(&c_name))
}

/// The user whose id is `uid`, if the account database has one.
pub fn user_by_uid(uid: u32) -> io::Result<Option<User>> {
    passwd(Key::Uid(uid))
}

/// The members `group(5)` lists for the group named `name` (not the users whose own group it
/// is); none if the database has no such group.
pub fn group_members(name: &str) -> io::Result<Option<Vec<String>>> {
    let Ok(c_name) = CString::new(name) else {
        return Ok(None);
    };
    group(&c_name)
}

/// Every group the user named `name` belongs to (getgrouplist: `gid`, its own, first, then those
/// listing it), as a login sets them (`initgroups`).
#[allow(unsafe_code)]
pub fn group_list(name: &CStr, gid: u32) -> io::Result<Vec<u32>> {
    let mut count: libc::c_int = 32;
    loop {
        let mut groups = vec![0 as libc::gid_t; count as usize];
        let asked = count;
        // SAFETY: `groups` holds `count` ids; getgrouplist writes at most that many and sets
        // `count` to how many there are.
        let found =
            unsafe { libc::getgrouplist(name.as_ptr(), gid, groups.as_mut_ptr(), &mut count) };
        if found >= 0 {
            groups.truncate(count as usize);
            return Ok(groups);
        }
        if count <= asked {
            // It found more than fitted but says no more: a broken database.
            count = asked.saturating_mul(2);
        }
        if count > 65536 {
            return Err(io::Error::other("too many groups"));
        }
    }
}

enum Key<'a> {
    Name(&'a CStr),
    Uid(u32),
}

/// The first buffer for an entry's strings, and the largest tried (a group listing thousands of
/// members) before the entry is said to be too large.
const FIRST_BUFFER: usize = 16 * 1024;
const LARGEST_BUFFER: usize = 1024 * 1024;

/// `pointer`'s string, a C string in an entry's buffer (or null, an empty field).
///
/// # Safety
///
/// `pointer` is null or points to a NUL-terminated string that lives while the result is made.
#[allow(unsafe_code)]
unsafe fn string(pointer: *const libc::c_char) -> String {
    if pointer.is_null() {
        return String::new();
    }
    // SAFETY: the caller's promise.
    unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned()
}

#[allow(unsafe_code)]
fn passwd(key: Key<'_>) -> io::Result<Option<User>> {
    let mut size = FIRST_BUFFER;
    loop {
        let mut buffer = vec![0 as libc::c_char; size];
        // SAFETY: an all-zero passwd is a valid value of the C struct (null pointers, zero ids).
        let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: every pointer is valid and `buffer` is as long as said; getpwnam_r and
        // getpwuid_r fill `entry` with strings inside `buffer` and set `found` to `entry` or
        // leave it null.
        let status = unsafe {
            match key {
                Key::Name(name) => libc::getpwnam_r(
                    name.as_ptr(),
                    &mut entry,
                    buffer.as_mut_ptr(),
                    buffer.len(),
                    &mut found,
                ),
                Key::Uid(uid) => libc::getpwuid_r(
                    uid,
                    &mut entry,
                    buffer.as_mut_ptr(),
                    buffer.len(),
                    &mut found,
                ),
            }
        };
        if status == libc::ERANGE && size < LARGEST_BUFFER {
            size *= 2;
            continue;
        }
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status));
        }
        if found.is_null() {
            return Ok(None);
        }
        // SAFETY: the entry's strings are null or point into `buffer`, NUL-terminated, and
        // `buffer` lives until the end of this block.
        let user = unsafe {
            User {
                name: string(entry.pw_name),
                uid: entry.pw_uid,
                gid: entry.pw_gid,
                gecos: string(entry.pw_gecos),
                home: string(entry.pw_dir).into(),
                shell: string(entry.pw_shell).into(),
            }
        };
        return Ok(Some(user));
    }
}

#[allow(unsafe_code)]
fn group(name: &CStr) -> io::Result<Option<Vec<String>>> {
    let mut size = FIRST_BUFFER;
    loop {
        let mut buffer = vec![0 as libc::c_char; size];
        // SAFETY: an all-zero group is a valid value of the C struct (null pointers, zero id).
        let mut entry: libc::group = unsafe { std::mem::zeroed() };
        let mut found: *mut libc::group = std::ptr::null_mut();
        // SAFETY: every pointer is valid and `buffer` is as long as said; getgrnam_r fills
        // `entry` with strings and the members' array inside `buffer` and sets `found` to
        // `entry` or leaves it null.
        let status = unsafe {
            libc::getgrnam_r(
                name.as_ptr(),
                &mut entry,
                buffer.as_mut_ptr(),
                buffer.len(),
                &mut found,
            )
        };
        if status == libc::ERANGE && size < LARGEST_BUFFER {
            size *= 2;
            continue;
        }
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status));
        }
        if found.is_null() {
            return Ok(None);
        }
        let mut members = Vec::new();
        let mut member = entry.gr_mem;
        // SAFETY: gr_mem is null or a null-terminated array of NUL-terminated strings, all
        // inside `buffer`, which lives until the end of this function.
        unsafe {
            while !member.is_null() && !(*member).is_null() {
                let name = string(*member);
                if !name.is_empty() {
                    members.push(name);
                }
                member = member.add(1);
            }
        }
        return Ok(Some(members));
    }
}

#[cfg(test)]
mod tests;
