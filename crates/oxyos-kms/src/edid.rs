//! What a display says of itself in its EDID (VESA E-EDID 1.4, the base block): who made it, its
//! product code, its serial number and its name. The session knows a display by them
//! (docs/adr/0018-several-displays.mdx): the same monitor is the same display on any port, and its
//! settings follow it. And the vertical refresh it takes (its range limits): with the connector's
//! `vrr_capable`, the range a variable refresh moves in.

/// The base block's fixed header.
const HEADER: [u8; 8] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];
/// The base block's length, and where its four 18-byte descriptors begin.
const BLOCK: usize = 128;
const DESCRIPTORS: usize = 54;
/// Display descriptors' tags: the product's serial number and its name, as text.
const SERIAL_TEXT: u8 = 0xff;
const NAME_TEXT: u8 = 0xfc;
/// The display range limits descriptor: the vertical and horizontal rates it takes.
const RANGE_LIMITS: u8 = 0xfd;

/// A display's EDID, as far as the session uses it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edid {
    /// The manufacturer's three-letter PNP id (`DEL`, `LEN`, `BOE`).
    pub manufacturer: String,
    pub product: u16,
    /// The serial number, 0 when the display gives none.
    pub serial: u32,
    /// Its serial number as text, and its name (`DELL U2723QE`), from its descriptors.
    pub serial_text: Option<String>,
    pub name: Option<String>,
    /// The vertical refresh it takes, least and most, in whole hertz (its range limits); none
    /// when it does not say.
    pub refresh_range_hz: Option<(u32, u32)>,
}

impl Edid {
    /// Reads the base block: none when it is not an EDID (its header, its checksum).
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let block = bytes.get(..BLOCK)?;
        if block[..8] != HEADER || block.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) != 0
        {
            return None;
        }
        let packed = u16::from_be_bytes([block[8], block[9]]);
        let letter = |shift: u16| {
            let code = ((packed >> shift) & 0x1f) as u8;
            (1..=26)
                .contains(&code)
                .then(|| char::from(b'A' + code - 1))
        };
        let manufacturer: String = [letter(10)?, letter(5)?, letter(0)?].iter().collect();
        let mut edid = Self {
            manufacturer,
            product: u16::from_le_bytes([block[10], block[11]]),
            serial: u32::from_le_bytes([block[12], block[13], block[14], block[15]]),
            serial_text: None,
            name: None,
            refresh_range_hz: None,
        };
        for descriptor in block[DESCRIPTORS..DESCRIPTORS + 4 * 18].chunks_exact(18) {
            // A display descriptor begins with three zero bytes, then its tag.
            if descriptor[..3] != [0, 0, 0] {
                continue;
            }
            let text = descriptor_text(&descriptor[5..]);
            match descriptor[3] {
                SERIAL_TEXT => edid.serial_text = text,
                NAME_TEXT => edid.name = text,
                RANGE_LIMITS => edid.refresh_range_hz = refresh_range(descriptor),
                _ => {}
            }
        }
        Some(edid)
    }

    /// The display's identity: its manufacturer, product and serial number
    /// (`DEL-A0B4-4C4D4E30`), the serial's text when it gives one. None without a serial: two
    /// such displays of one model are told apart by their connectors (`Output::identity`).
    pub fn identity(&self) -> Option<String> {
        let serial = match &self.serial_text {
            Some(text) => identity_safe(text),
            None if self.serial != 0 => format!("{:08X}", self.serial),
            None => return None,
        };
        Some(format!(
            "{}-{:04X}-{serial}",
            self.manufacturer, self.product
        ))
    }
}

/// A range limits descriptor's vertical rates: bytes 5 and 6, each 255 more when byte 4's low bits
/// say so (EDID 1.4: bit 0 the least's, bit 1 the most's).
fn refresh_range(descriptor: &[u8]) -> Option<(u32, u32)> {
    let offsets = descriptor[4];
    let least = u32::from(descriptor[5]) + if offsets & 0b11 == 0b11 { 255 } else { 0 };
    let most = u32::from(descriptor[6]) + if offsets & 0b10 != 0 { 255 } else { 0 };
    (least > 0 && least <= most).then_some((least, most))
}

/// A descriptor's 13 bytes of text: printable ASCII up to the line feed that ends it, without the
/// spaces padding it. None when empty.
fn descriptor_text(bytes: &[u8]) -> Option<String> {
    let end = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(bytes.len());
    let text: String = bytes[..end]
        .iter()
        .filter(|byte| byte.is_ascii_graphic() || **byte == b' ')
        .map(|byte| char::from(*byte))
        .collect();
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// An identity's characters: ASCII letters, digits and `-`, so that it names a preference.
pub fn identity_safe(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A base block with this manufacturer, product, serial and descriptors (tag, text), its
    /// checksum made right.
    fn block(manufacturer: &str, product: u16, serial: u32, descriptors: &[(u8, &str)]) -> Vec<u8> {
        let mut bytes = vec![0u8; BLOCK];
        bytes[..8].copy_from_slice(&HEADER);
        let letters: Vec<u16> = manufacturer
            .bytes()
            .map(|b| u16::from(b - b'A' + 1))
            .collect();
        let packed = (letters[0] << 10) | (letters[1] << 5) | letters[2];
        bytes[8..10].copy_from_slice(&packed.to_be_bytes());
        bytes[10..12].copy_from_slice(&product.to_le_bytes());
        bytes[12..16].copy_from_slice(&serial.to_le_bytes());
        // The first descriptor is the preferred timing (not a display descriptor).
        bytes[DESCRIPTORS] = 0x56;
        bytes[DESCRIPTORS + 1] = 0x5e;
        for (index, (tag, text)) in descriptors.iter().enumerate() {
            let at = DESCRIPTORS + 18 * (index + 1);
            bytes[at + 3] = *tag;
            let mut field = [b' '; 13];
            field[..text.len()].copy_from_slice(text.as_bytes());
            if text.len() < 13 {
                field[text.len()] = b'\n';
            }
            bytes[at + 5..at + 18].copy_from_slice(&field);
        }
        let sum = bytes[..BLOCK - 1]
            .iter()
            .fold(0u8, |s, b| s.wrapping_add(*b));
        bytes[BLOCK - 1] = 0u8.wrapping_sub(sum);
        bytes
    }

    #[test]
    fn a_monitor_is_its_maker_product_and_serial() {
        let edid = Edid::parse(&block(
            "DEL",
            0xa0b4,
            0x4c4d_4e30,
            &[(NAME_TEXT, "DELL U2723QE"), (SERIAL_TEXT, "7Y9K2N3")],
        ))
        .unwrap();
        assert_eq!(edid.manufacturer, "DEL");
        assert_eq!(edid.product, 0xa0b4);
        assert_eq!(edid.name.as_deref(), Some("DELL U2723QE"));
        // The serial's text, when given, is the one people read on the monitor's label.
        assert_eq!(edid.identity().as_deref(), Some("DEL-A0B4-7Y9K2N3"));
    }

    #[test]
    fn a_panel_with_a_serial_number_only_and_one_with_none() {
        // The ThinkPad's BOE panel: a name, no serial text, a serial number.
        let panel = Edid::parse(&block("BOE", 0x0a4b, 1, &[(NAME_TEXT, "NE140WUM-N62")])).unwrap();
        assert_eq!(panel.identity().as_deref(), Some("BOE-0A4B-00000001"));
        // No serial at all: the identity is left to the connector.
        let none = Edid::parse(&block("GSM", 0x5b7f, 0, &[])).unwrap();
        assert_eq!(none.identity(), None);
        assert_eq!(none.name, None);
    }

    /// The ThinkPad P14s Gen 3's panel, as its eDP-1 reports it: a CMN N140JCA-EEL, 1920×1200 at
    /// 60 Hz only, taking 40 to 60 Hz (its connector says `vrr_capable`).
    const THINKPAD: [u8; BLOCK] = [
        0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x0d, 0xae, 0x3f, 0x14, 0x00, 0x00, 0x00,
        0x00, 0x25, 0x1f, 0x01, 0x04, 0xa5, 0x1e, 0x13, 0x78, 0x03, 0x28, 0x65, 0x97, 0x59, 0x54,
        0x8e, 0x27, 0x1e, 0x50, 0x54, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
        0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x42, 0x3c, 0x80, 0xa0, 0x70, 0xb0,
        0x24, 0x40, 0x30, 0x20, 0xa6, 0x00, 0x2d, 0xbc, 0x10, 0x00, 0x00, 0x18, 0x00, 0x00, 0x00,
        0xfd, 0x00, 0x28, 0x3c, 0x43, 0x43, 0x10, 0x01, 0x0a, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20,
        0x00, 0x00, 0x00, 0xfe, 0x00, 0x43, 0x4d, 0x4e, 0x0a, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20,
        0x20, 0x20, 0x20, 0x00, 0x00, 0x00, 0xfe, 0x00, 0x4e, 0x31, 0x34, 0x30, 0x4a, 0x43, 0x41,
        0x2d, 0x45, 0x45, 0x4c, 0x0a, 0x20, 0x00, 0x39,
    ];

    #[test]
    fn a_panel_says_the_refresh_it_takes() {
        let panel = Edid::parse(&THINKPAD).unwrap();
        assert_eq!(panel.manufacturer, "CMN");
        assert_eq!(panel.product, 0x143f);
        assert_eq!(panel.refresh_range_hz, Some((40, 60)));
        // Rates past 255 Hz are given as offsets (a 360 Hz monitor's 48 to 360).
        let mut descriptor = [0u8; 18];
        descriptor[3] = RANGE_LIMITS;
        (descriptor[4], descriptor[5], descriptor[6]) = (0b10, 48, 105);
        assert_eq!(refresh_range(&descriptor), Some((48, 360)));
        // None given, or nonsense: nothing.
        assert_eq!(
            Edid::parse(&block("DEL", 1, 2, &[]))
                .unwrap()
                .refresh_range_hz,
            None
        );
        (descriptor[4], descriptor[5], descriptor[6]) = (0, 120, 60);
        assert_eq!(refresh_range(&descriptor), None);
    }

    #[test]
    fn what_is_not_an_edid_is_nothing() {
        let good = block("DEL", 1, 2, &[]);
        assert!(Edid::parse(&good).is_some());
        assert!(Edid::parse(&good[..100]).is_none());
        let mut header = good.clone();
        header[0] = 1;
        assert!(Edid::parse(&header).is_none());
        let mut checksum = good;
        checksum[20] ^= 1;
        assert!(Edid::parse(&checksum).is_none());
    }

    #[test]
    fn an_identity_names_a_preference() {
        assert_eq!(identity_safe("HDMI-A-1"), "HDMI-A-1");
        assert_eq!(identity_safe("a b/c=d"), "a-b-c-d");
    }
}
