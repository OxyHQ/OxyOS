//! Minimal PNG writer (RGB, stored deflate blocks) so snapshots need no image dependency.

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1_u32, 0_u32);
    for chunk in bytes.chunks(5552) {
        for &byte in chunk {
            a += u32::from(byte);
            b += a;
        }
        a %= 65_521;
        b %= 65_521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend(kind);
    out.extend(data);
    let crc = crc32(&out[start..]);
    out.extend(crc.to_be_bytes());
}

/// Encodes 0xRRGGBB pixels, row-major.
pub fn encode(width: u32, height: u32, pixels: &[u32]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width as usize * 3 + 1) * height as usize);
    for row in pixels.chunks(width as usize) {
        raw.push(0);
        for pixel in row {
            raw.extend(&pixel.to_be_bytes()[1..]);
        }
    }
    let mut zlib = vec![0x78, 0x01];
    let blocks = raw.chunks(65_535).count();
    for (index, block) in raw.chunks(65_535).enumerate() {
        zlib.push(u8::from(index + 1 == blocks));
        let length = block.len() as u16;
        zlib.extend(length.to_le_bytes());
        zlib.extend((!length).to_le_bytes());
        zlib.extend(block);
    }
    zlib.extend(adler32(&raw).to_be_bytes());

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend(width.to_be_bytes());
    header.extend(height.to_be_bytes());
    header.extend([8, 2, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &header);
    chunk(&mut png, b"IDAT", &zlib);
    chunk(&mut png, b"IEND", &[]);
    png
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums_match_known_values() {
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
        let png = encode(2, 1, &[0xff0000, 0x00ff00]);
        assert!(png.starts_with(b"\x89PNG"));
        assert!(png.ends_with(&[0xae, 0x42, 0x60, 0x82]));
    }
}
