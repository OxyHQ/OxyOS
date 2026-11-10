//! Strong passwords (OxyOS ADR 0050 decision 8), as Safari's: 20 characters of `[a-zA-Z0-9]` in
//! four groups of five joined by hyphens, with at least one lowercase letter, one uppercase
//! letter and one digit, drawn uniformly from the system's CSPRNG (about 100 bits). A draw
//! without every class is drawn again whole, so every acceptable password is equally likely.

use crate::{Error, Result, Secret};
use rand::distributions::{Distribution, Slice};
use rand::rngs::OsRng;
use zeroize::Zeroizing;

const ALPHABET: &[u8; 62] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const GROUPS: usize = 4;
const GROUP: usize = 5;
pub const LENGTH: usize = GROUPS * GROUP + GROUPS - 1;

/// A new strong password.
pub fn generate() -> Result<Secret> {
    let letters = Slice::new(ALPHABET).map_err(|_| Error::Random)?;
    loop {
        let mut password = Zeroizing::new(String::with_capacity(LENGTH));
        for i in 0..GROUPS * GROUP {
            if i > 0 && i % GROUP == 0 {
                password.push('-');
            }
            // `OsRng` panics if the system's generator fails, rather than hand out a weak one.
            password.push(*letters.sample(&mut OsRng) as char);
        }
        let has = |class: fn(&u8) -> bool| password.bytes().any(|b| class(&b));
        if has(u8::is_ascii_lowercase) && has(u8::is_ascii_uppercase) && has(u8::is_ascii_digit) {
            return Ok(Secret::from(password));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn has_the_shape() {
        for _ in 0..500 {
            let password = generate().unwrap();
            let text = password.expose();
            assert_eq!(text.len(), 23);
            let groups: Vec<&str> = text.split('-').collect();
            assert_eq!(groups.len(), 4);
            assert!(groups
                .iter()
                .all(|g| g.len() == 5 && g.bytes().all(|b| b.is_ascii_alphanumeric())));
            assert!(text.bytes().any(|b| b.is_ascii_lowercase()));
            assert!(text.bytes().any(|b| b.is_ascii_uppercase()));
            assert!(text.bytes().any(|b| b.is_ascii_digit()));
        }
    }

    #[test]
    fn draws_from_the_whole_alphabet() {
        let mut seen = HashSet::new();
        for _ in 0..400 {
            seen.extend(generate().unwrap().expose().bytes().filter(|b| *b != b'-'));
        }
        assert_eq!(seen.len(), 62);
    }

    #[test]
    fn debug_shows_no_password() {
        let password = generate().unwrap();
        assert!(!format!("{password:?}").contains(password.expose()));
    }
}
