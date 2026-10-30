//! An Oxy account's id, and the Unix user name it gives the account on the machine.
//!
//! Oxy's ids have two shapes (the SDK's `packages/db/src/ids.ts`): a legacy 24-character hex id
//! for accounts older than uuid ids, and a uuid v7 for every account since, both lowercase as
//! Oxy writes them. Nothing else is an account id here.
//!
//! The user is `oxy-` and the id's bytes in lowercase base32 (RFC 4648's alphabet, unpadded):
//! 20 characters for a legacy id, 26 for a uuid. A uuid written out (`oxy-` and 36 characters)
//! is longer than the 31 characters a user name may have (utmp's 32 bytes with their NUL; systemd
//! refuses longer names), and its hex without dashes still is. The encoding is lossless and the
//! two lengths differ, so two accounts never share a user; its alphabet (`a-z`, `2-7`) and its
//! prefix make a name every tool accepts (`[a-z_][a-z0-9_-]*`).

use std::fmt;

/// Longest user name: utmp's `ut_user` is 32 bytes with its NUL.
pub const MAX_USER_NAME: usize = 31;

const PREFIX: &str = "oxy-";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AccountId {
    text: String,
    bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidAccountId;

impl fmt::Display for InvalidAccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not an Oxy account id (a 24-character hex id or a uuid v7, lowercase)")
    }
}

impl std::error::Error for InvalidAccountId {}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn hex_bytes(hex: &[u8]) -> Option<Vec<u8>> {
    hex.chunks(2)
        .map(|pair| Some((hex_digit(pair[0])? << 4) | hex_digit(pair[1])?))
        .collect()
}

impl AccountId {
    pub fn parse(text: &str) -> Result<Self, InvalidAccountId> {
        let raw = text.as_bytes();
        let bytes = match raw.len() {
            24 => hex_bytes(raw),
            36 => {
                let dashes = [8, 13, 18, 23];
                let shaped = raw
                    .iter()
                    .enumerate()
                    .all(|(index, &byte)| (byte == b'-') == dashes.contains(&index))
                    && raw[14] == b'7'
                    && matches!(raw[19], b'8' | b'9' | b'a' | b'b');
                let hex: Vec<u8> = raw.iter().copied().filter(|&b| b != b'-').collect();
                if shaped {
                    hex_bytes(&hex)
                } else {
                    None
                }
            }
            _ => None,
        }
        .ok_or(InvalidAccountId)?;
        Ok(Self {
            text: text.to_owned(),
            bytes,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn user_name(&self) -> UserName {
        UserName(format!("{PREFIX}{}", base32(&self.bytes)))
    }
}

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// The Unix user of an enrolled account (`oxy-…`): made only from an [`AccountId`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UserName(String);

impl UserName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UserName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// RFC 4648 base32 in lowercase, without padding.
pub(crate) fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut text = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for &byte in bytes {
        // At most 12 bits are pending (4 left over and 8 new).
        buffer = ((buffer << 8) | u32::from(byte)) & 0xfff;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            text.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        text.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    text
}
