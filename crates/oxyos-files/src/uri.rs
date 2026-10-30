//! `file://` URIs, as `expo-file-system` gives and takes them (docs/adr/0028-file-access.mdx).
//!
//! An application names files by URI: `file:///home/ada/My%20Notes.md`. Its path is
//! percent-decoded here; one built for an application is percent-encoded as the WHATWG URL
//! standard encodes a path (what `Paths.join` makes in JavaScript), with `%` itself encoded so a
//! name holding one reads back as itself. Only local files: no host but `localhost`, no query or
//! fragment, no other scheme (`content://`, `asset://` and the network's are Android's or
//! another API's).

use crate::Error;
use std::path::Path;

/// The absolute path a `file://` URI names (`file:///p`, `file://localhost/p`, or `file:/p` as
/// Java writes it), percent-decoded, without a trailing `/` but the root's. The path is checked
/// by the scope, not here: this only reads the URI.
pub fn path_of(uri: &str) -> Result<String, Error> {
    let refused = || Error::Refused(format!("{uri:?} is not a file:// URI"));
    let rest = uri
        .strip_prefix("file:")
        .or_else(|| uri.strip_prefix("FILE:"))
        .ok_or_else(refused)?;
    let encoded = if let Some(authority) = rest.strip_prefix("//") {
        let slash = authority.find('/').ok_or_else(refused)?;
        let host = &authority[..slash];
        if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
            return Err(Error::Refused(format!(
                "{uri:?} names a file on another computer ({host})"
            )));
        }
        &authority[slash..]
    } else if rest.starts_with('/') {
        rest
    } else {
        return Err(refused());
    };
    if encoded.contains(['?', '#']) {
        return Err(Error::Refused(format!(
            "{uri:?} has a query or a fragment, which a file has not"
        )));
    }
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = |byte: u8| char::from(byte).to_digit(16);
        if bytes[at] == b'%' && at + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex(bytes[at + 1]), hex(bytes[at + 2])) {
                out.push((high * 16 + low) as u8);
                at += 3;
                continue;
            }
        }
        out.push(bytes[at]);
        at += 1;
    }
    let mut path = String::from_utf8(out)
        .map_err(|_| Error::Refused(format!("{uri:?} names a path that is not UTF-8")))?;
    while path.len() > 1 && path.ends_with('/') {
        path.pop();
    }
    Ok(path)
}

/// Whether `byte` is written as itself in a URI's path (the WHATWG path percent-encode set's
/// complement among ASCII, `%` aside).
fn kept(byte: u8) -> bool {
    byte.is_ascii_graphic() && !b"\"#<>?`{}%".contains(&byte)
}

/// A path's part of a URI: each byte not [`kept`] as `%XX`.
pub fn encode(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        if kept(byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The `file://` URI of `path`; a folder's ends with `/`, as `expo-file-system`'s directories do.
pub fn of(path: &Path, directory: bool) -> String {
    let text = path.to_string_lossy();
    let mut uri = format!("file://{}", encode(&text));
    if directory && !uri.ends_with('/') {
        uri.push('/');
    }
    uri
}

/// The URI of the item `name` in the folder `folder` names, as the application wrote the
/// folder's (its spelling kept: a folder reached through a link stays named by the link).
pub fn child(folder: &str, name: &str, directory: bool) -> String {
    let mut uri = folder.trim_end_matches('/').to_string();
    uri.push('/');
    uri.push_str(&encode(name));
    if directory {
        uri.push('/');
    }
    uri
}

/// The URI of the folder holding what `uri` names, as the application wrote it.
pub fn parent(uri: &str) -> String {
    let trimmed = uri.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(slash) => trimmed[..=slash].to_string(),
        None => trimmed.to_string(),
    }
}
