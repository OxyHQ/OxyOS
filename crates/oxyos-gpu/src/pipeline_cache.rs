//! The compositor's compiled pipelines kept across sessions (docs/adr/0031-gpu-buffers.mdx,
//! decision 12, *Pipelines kept*): the driver's Vulkan pipeline cache, as `vkGetPipelineCacheData`
//! gives it, in a file of the session's own cache directory, given back to the driver at the next
//! start so that its compute pipelines are not compiled again.
//!
//! - **Per GPU and driver.** The file is named by the device's vendor, device and
//!   `pipelineCacheUUID` (which a driver changes with its build), so another GPU or another
//!   driver's version never reads it; the files of others in the directory are removed when one
//!   is written (a driver updated leaves its old cache behind once).
//! - **Validated.** A header of its own (a magic, the data's length and an FNV-1a checksum) and
//!   the Vulkan header inside (`VkPipelineCacheHeaderVersionOne`: its length, its version, the
//!   vendor, the device and the UUID, each as the device has them) are checked before the driver
//!   is given a byte; a file that fails is removed.
//! - **Bounded**: at most [`MAX_BYTES`], read or written.
//! - **Written whole**: aside, then renamed, so a reader finds the file before or this one.
//! - **Private**: its directory 0700 and the file 0600, in the person's own cache directory
//!   (`$XDG_CACHE_HOME/oxyos/compositor`): never another account's.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// The most a cache may be: the compositor's two compute pipelines take a few hundred KiB.
pub const MAX_BYTES: usize = 4 << 20;

const MAGIC: &[u8; 8] = b"OXYOSPC1";
/// The header of our own: the magic, the data's length and its checksum.
const HEADER: usize = 8 + 8 + 8;
/// `VkPipelineCacheHeaderVersionOne`'s length: its length, version, vendor and device (4 bytes
/// each) and the UUID.
const VULKAN_HEADER: usize = 16 + 16;

/// Who a cache is for: the device's vendor, device and `pipelineCacheUUID`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    pub vendor: u32,
    pub device: u32,
    pub uuid: [u8; 16],
}

impl Identity {
    /// Its file in `directory`.
    pub fn file(&self, directory: &Path) -> PathBuf {
        let uuid: String = self.uuid.iter().map(|byte| format!("{byte:02x}")).collect();
        directory.join(format!(
            "{:04x}-{:04x}-{uuid}.vkpc",
            self.vendor, self.device
        ))
    }

    /// Whether `data` is a Vulkan pipeline cache for this device: its header's length, version
    /// one, the vendor, the device and the UUID.
    pub fn owns(&self, data: &[u8]) -> bool {
        let word =
            |at: usize| u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
        data.len() >= VULKAN_HEADER
            && word(0) as usize >= VULKAN_HEADER
            && word(0) as usize <= data.len()
            && word(4) == 1 // VK_PIPELINE_CACHE_HEADER_VERSION_ONE
            && word(8) == self.vendor
            && word(12) == self.device
            && data[16..32] == self.uuid
    }
}

/// FNV-1a, 64 bits.
fn checksum(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The cache kept for `identity` in `directory`, if there is a valid one; an invalid one is
/// removed.
pub fn load(directory: &Path, identity: &Identity) -> Option<Vec<u8>> {
    let path = identity.file(directory);
    let file = fs::read(&path).ok()?;
    let data = parse(&file, identity);
    if data.is_none() {
        let _ = fs::remove_file(&path);
    }
    data
}

/// The data of a file's bytes, when its header, its length, its checksum and the Vulkan header
/// inside are right and it is within [`MAX_BYTES`].
fn parse(file: &[u8], identity: &Identity) -> Option<Vec<u8>> {
    if file.len() < HEADER || file.len() - HEADER > MAX_BYTES || &file[..8] != MAGIC {
        return None;
    }
    let length = u64::from_le_bytes(file[8..16].try_into().ok()?);
    let sum = u64::from_le_bytes(file[16..24].try_into().ok()?);
    let data = &file[HEADER..];
    (length == data.len() as u64 && sum == checksum(data) && identity.owns(data))
        .then(|| data.to_vec())
}

/// Keeps `data` for `identity` in `directory` (made 0700 if missing), written aside and renamed;
/// every other device's or driver's cache there is removed. Nothing is written for data that is
/// not this device's cache or is larger than [`MAX_BYTES`].
pub fn store(directory: &Path, identity: &Identity, data: &[u8]) -> std::io::Result<()> {
    if data.len() > MAX_BYTES || !identity.owns(data) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "not this device's pipeline cache, or too large",
        ));
    }
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    let path = identity.file(directory);
    let aside = directory.join(format!(".{}.new", std::process::id()));
    let _ = fs::remove_file(&aside);
    let written = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&aside)?;
        file.write_all(MAGIC)?;
        file.write_all(&(data.len() as u64).to_le_bytes())?;
        file.write_all(&checksum(data).to_le_bytes())?;
        file.write_all(data)?;
        drop(file);
        fs::rename(&aside, &path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&aside);
        return written;
    }
    // Another device's, or this one's under a driver since replaced.
    for entry in fs::read_dir(directory)?.flatten() {
        let other = entry.path();
        if other != path
            && other
                .extension()
                .is_some_and(|extension| extension == "vkpc")
        {
            let _ = fs::remove_file(other);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: Identity = Identity {
        vendor: 0x8086,
        device: 0x46a6,
        uuid: [7; 16],
    };

    /// A pipeline cache as a driver gives it: its header, then `body`.
    fn cache_of(identity: &Identity, body: &[u8]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&(VULKAN_HEADER as u32).to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&identity.vendor.to_le_bytes());
        data.extend_from_slice(&identity.device.to_le_bytes());
        data.extend_from_slice(&identity.uuid);
        data.extend_from_slice(body);
        data
    }

    fn directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "oxyos-pipeline-cache-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        directory.join("oxyos/compositor")
    }

    #[test]
    fn a_cache_is_kept_whole_and_private_and_read_back() {
        let directory = directory("kept");
        let data = cache_of(&IDENTITY, b"pipelines");
        store(&directory, &IDENTITY, &data).unwrap();
        assert_eq!(load(&directory, &IDENTITY), Some(data));
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&directory), 0o700);
        assert_eq!(mode(&IDENTITY.file(&directory)), 0o600);
        // Nothing left aside.
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        let _ = fs::remove_dir_all(directory.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn another_device_or_driver_never_reads_it_and_a_new_driver_replaces_it() {
        let directory = directory("devices");
        store(&directory, &IDENTITY, &cache_of(&IDENTITY, b"old driver")).unwrap();
        let updated = Identity {
            uuid: [8; 16],
            ..IDENTITY
        };
        assert_eq!(load(&directory, &updated), None);
        // A cache whose Vulkan header is another device's is never written as this one's.
        assert!(store(&directory, &updated, &cache_of(&IDENTITY, b"x")).is_err());
        store(&directory, &updated, &cache_of(&updated, b"new driver")).unwrap();
        // The old driver's cache is gone with it.
        assert!(!IDENTITY.file(&directory).exists());
        assert_eq!(
            load(&directory, &updated),
            Some(cache_of(&updated, b"new driver"))
        );
        let _ = fs::remove_dir_all(directory.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn a_damaged_file_is_never_given_to_the_driver_and_is_removed() {
        let directory = directory("damaged");
        let data = cache_of(&IDENTITY, b"pipelines");
        let file = IDENTITY.file(&directory);
        let damaged = |change: &dyn Fn(&mut Vec<u8>)| {
            store(&directory, &IDENTITY, &data).unwrap();
            let mut bytes = fs::read(&file).unwrap();
            change(&mut bytes);
            fs::write(&file, &bytes).unwrap();
            let loaded = load(&directory, &IDENTITY);
            (loaded, file.exists())
        };
        // A byte of the data flipped, the file cut short, longer, its magic another's.
        assert_eq!(
            damaged(&|bytes| *bytes.last_mut().unwrap() ^= 1),
            (None, false)
        );
        assert_eq!(
            damaged(&|bytes| bytes.truncate(bytes.len() - 3)),
            (None, false)
        );
        assert_eq!(damaged(&|bytes| bytes.push(0)), (None, false));
        assert_eq!(damaged(&|bytes| bytes[0] = b'X'), (None, false));
        // Too short for the Vulkan header, with a right checksum.
        assert!(!IDENTITY.owns(&data[..VULKAN_HEADER - 1]));
        let _ = fs::remove_dir_all(directory.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn a_cache_past_the_bound_is_not_kept() {
        let directory = directory("bound");
        let data = cache_of(&IDENTITY, &vec![0; MAX_BYTES]);
        assert!(store(&directory, &IDENTITY, &data).is_err());
        assert!(!IDENTITY.file(&directory).exists());
        let _ = fs::remove_dir_all(directory.parent().unwrap().parent().unwrap());
    }
}
