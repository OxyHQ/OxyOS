//! `data:` URIs (RFC 2397): `data:[<media type>][;base64],<data>`, the image in the URI itself,
//! as expo-image's `Image` takes one for its source.
//!
//! Its data is percent-decoded, then, with `;base64`, decoded from base64 as WHATWG's Fetch
//! decodes a `data:` URL's ("forgiving-base64": padding optional, ASCII whitespace ignored), the
//! URL-safe alphabet taken as well as the standard one. Unlike a URL parser, a `#` does not end
//! it: an SVG document's colours (`fill='#fff'`) are kept, as an author writing one means them.
//! The media type is given as it is written (`image/svg+xml;charset=utf-8`), none when absent.

use crate::Error;

fn failed(reason: &str) -> Error {
    Error::Source(format!("Cannot decode the data: URI: {reason}"))
}

/// A `data:` URI's media type, if it gives one, and its bytes.
pub fn parse(uri: &str) -> Result<(Option<String>, Vec<u8>), Error> {
    let uri = uri.trim_start();
    let rest = uri
        .get(..5)
        .filter(|scheme| scheme.eq_ignore_ascii_case("data:"))
        .map(|_| &uri[5..])
        .ok_or_else(|| failed("not a data: URI"))?;
    let (header, data) = rest
        .split_once(',')
        .ok_or_else(|| failed("no ',' before its data"))?;
    let header = header.trim();
    // `;base64` at its end, any spaces before `base64`, in any case.
    let base64 = header.len() >= 6
        && header.is_char_boundary(header.len() - 6)
        && header[header.len() - 6..].eq_ignore_ascii_case("base64")
        && header[..header.len() - 6]
            .trim_end_matches(' ')
            .ends_with(';');
    let media_type = if base64 {
        let before = header[..header.len() - 6].trim_end_matches(' ');
        &before[..before.len() - 1]
    } else {
        header
    };
    let bytes = percent_decode(data.as_bytes());
    let bytes = if base64 {
        self::base64(&bytes).ok_or_else(|| failed("its data is not base64"))?
    } else {
        bytes
    };
    let media_type = media_type.trim();
    Ok((
        (!media_type.is_empty()).then(|| media_type.to_string()),
        bytes,
    ))
}

/// `%XX` turned into the byte it stands for; a `%` without two hexadecimal digits after it is
/// kept as it is, as WHATWG's percent-decode keeps it.
fn percent_decode(text: &[u8]) -> Vec<u8> {
    let digit = |byte: u8| (byte as char).to_digit(16).map(|digit| digit as u8);
    let mut bytes = Vec::with_capacity(text.len());
    let mut at = 0;
    while at < text.len() {
        if text[at] == b'%' {
            if let (Some(high), Some(low)) = (
                text.get(at + 1).copied().and_then(digit),
                text.get(at + 2).copied().and_then(digit),
            ) {
                bytes.push((high << 4) | low);
                at += 3;
                continue;
            }
        }
        bytes.push(text[at]);
        at += 1;
    }
    bytes
}

/// Base64, standard or URL-safe (`-` and `_` for `+` and `/`), its padding optional and ASCII
/// whitespace anywhere ignored; none if it is not base64 (another character, data after the
/// padding, or a length no bytes encode to).
pub fn base64(text: &[u8]) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(text.len() / 4 * 3 + 3);
    let (mut bits, mut held) = (0u32, 0u32);
    let (mut digits, mut padding) = (0usize, 0usize);
    for &byte in text {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => {
                padding += 1;
                continue;
            }
            b' ' | b'\t' | b'\n' | b'\r' | 0x0c => continue,
            _ => return None,
        };
        if padding > 0 {
            return None;
        }
        digits += 1;
        bits = (bits << 6) | u32::from(value);
        held += 6;
        if held >= 8 {
            held -= 8;
            bytes.push((bits >> held) as u8);
            bits &= (1 << held) - 1;
        }
    }
    let whole = digits % 4 != 1 && padding <= 2 && (padding == 0 || (digits + padding) % 4 == 0);
    whole.then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_is_read_with_either_alphabet_padding_or_none_and_whitespace() {
        assert_eq!(base64(b"aGVsbG8="), Some(b"hello".to_vec()));
        assert_eq!(base64(b"aGVsbG8"), Some(b"hello".to_vec()));
        assert_eq!(base64(b"aGVs\n bG8 ="), Some(b"hello".to_vec()));
        assert_eq!(base64(b"+/+/"), base64(b"-_-_"));
        assert_eq!(base64(b"+/+/"), Some(vec![0xfb, 0xff, 0xbf]));
        assert_eq!(base64(b""), Some(Vec::new()));
        // Not base64: another character, data after the padding, a single digit left over.
        assert_eq!(base64(b"aGVs*G8="), None);
        assert_eq!(base64(b"aGVsbG8=aa"), None);
        assert_eq!(base64(b"aGVsb"), None);
        assert_eq!(base64(b"aGVsbG8==="), None);
    }

    #[test]
    fn a_data_uri_is_read_in_base64_or_percent_encoded_with_its_media_type() {
        assert_eq!(
            parse("data:image/png;base64,aGVsbG8="),
            Ok((Some("image/png".into()), b"hello".to_vec()))
        );
        assert_eq!(
            parse("DATA:image/png ; BASE64 ,aGVs%62G8"),
            Ok((Some("image/png".into()), b"hello".to_vec()))
        );
        // Percent-encoded, a '#' kept, a lone '%' as it is.
        assert_eq!(
            parse("data:image/svg+xml;charset=utf-8,%3Csvg fill='#fff'/%3E 100%"),
            Ok((
                Some("image/svg+xml;charset=utf-8".into()),
                b"<svg fill='#fff'/> 100%".to_vec()
            ))
        );
        assert_eq!(parse("data:,hi"), Ok((None, b"hi".to_vec())));
        for (uri, reason) in [
            ("http://example.com/a.png", "not a data: URI"),
            ("data:image/png;base64", "no ',' before its data"),
            ("data:image/png;base64,a*b", "its data is not base64"),
        ] {
            assert_eq!(
                parse(uri),
                Err(Error::Source(format!(
                    "Cannot decode the data: URI: {reason}"
                ))),
                "{uri}"
            );
        }
    }
}
