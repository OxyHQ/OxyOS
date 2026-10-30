//! Base64url without padding (RFC 4648, section 5), decoded strictly: one spelling for one
//! byte string. No padding, no whitespace, and the unused bits of the last character are zero,
//! so a signature or a key cannot be written two ways that a byte comparison would tell apart.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        let chars = chunk.len() + 1;
        for i in 0..chars {
            out.push(ALPHABET[((n >> (18 - 6 * i)) & 0x3f) as usize] as char);
        }
    }
    out
}

fn sextet(c: u8) -> Option<u32> {
    ALPHABET.iter().position(|a| *a == c).map(|p| p as u32)
}

pub(crate) fn decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    // A length of 1 modulo 4 is never an encoding.
    if bytes.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= sextet(*c)? << (18 - 6 * i);
        }
        let produced = chunk.len() - 1;
        // The bits past the last whole byte must be zero: the canonical spelling.
        let used = 8 * produced;
        if n & ((1u32 << (24 - used)) - 1) != 0 {
            return None;
        }
        for i in 0..produced {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    Some(out)
}
