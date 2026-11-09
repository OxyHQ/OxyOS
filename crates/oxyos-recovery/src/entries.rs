//! The boot loader's entries and their boot counting ([ADR 0054](../../../docs/adr/0054-recovery-and-erasing.mdx),
//! decision 2).
//!
//! systemd-boot counts boots in an entry's file name (systemd's
//! [automatic boot assessment](https://systemd.io/AUTOMATIC_BOOT_ASSESSMENT/)):
//! `oxyos-6.12.48+3.conf` has three tries left; each start renames it, one try fewer and one more
//! done (`+2-1`), before the kernel runs; `systemd-bless-boot` removes the counter once the boot
//! reaches `boot-complete.target`. An entry with no tries left (`+0-3`) is *bad*: systemd-boot
//! sorts it last, and the recovery, which is never counted, comes first.
//!
//! systemd counts a new kernel until its first good boot. OxyOS counts every boot: after each
//! complete one, `oxyos-recovery boot-tries` gives the entry that booted its tries again
//! ([`Entries::rearm`]), so three starts in a row that never complete, whenever they happen, end in
//! the recovery. The recovery's *Restart* gives every OxyOS entry its tries back
//! ([`Entries::rearm_all`]); an erase about to happen takes them all ([`Entries::condemn_all`]),
//! so whatever starts the machine next, it is the recovery.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Tries an entry is given (`/etc/kernel/tries`, which kernel-install reads for new kernels).
pub const TRIES: u32 = 3;

/// An entry's boot counter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tries {
    pub left: u32,
    pub done: u32,
}

/// A Type 1 entry's file name, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub file_name: String,
    /// Its name without the counter and `.conf`: `oxyos-6.12.48`.
    pub id: String,
    pub tries: Option<Tries>,
}

impl Entry {
    /// The file name of a `.conf` entry: its id, and its counter when it has one. A counter that
    /// does not parse is part of the id, as systemd-boot reads it.
    pub fn parse(file_name: &str) -> Option<Self> {
        let stem = file_name.strip_suffix(".conf")?;
        if stem.is_empty() || file_name.contains('/') {
            return None;
        }
        let (id, tries) = match stem.rsplit_once('+') {
            Some((id, counter)) if !id.is_empty() => match parse_counter(counter) {
                Some(tries) => (id, Some(tries)),
                None => (stem, None),
            },
            _ => (stem, None),
        };
        Some(Self {
            file_name: file_name.to_string(),
            id: id.to_string(),
            tries,
        })
    }

    /// The file name this entry has with `tries`.
    pub fn file_name_with(&self, tries: Option<Tries>) -> String {
        match tries {
            None => format!("{}.conf", self.id),
            Some(Tries { left, done: 0 }) => format!("{}+{left}.conf", self.id),
            Some(Tries { left, done }) => format!("{}+{left}-{done}.conf", self.id),
        }
    }

    /// No tries left: systemd-boot will not start it while another entry may.
    pub fn is_bad(&self) -> bool {
        matches!(self.tries, Some(Tries { left: 0, .. }))
    }

    /// One of OxyOS's own kernels' entries (kernel-install's, named by the entry token `oxyos`),
    /// not the recovery's.
    ///
    /// kernel-install names them `<entry token>-<kernel version>`, and a kernel's version begins
    /// with a digit: the recovery's (`oxyos-recovery`) and any other entry a person added are not
    /// counted, condemned or removed.
    pub fn is_system(&self) -> bool {
        self.id
            .strip_prefix("oxyos-")
            .is_some_and(|version| version.starts_with(|c: char| c.is_ascii_digit()))
    }

    /// One of Safe Mode's entries (ADR 0054, decision 10): `oxyos-safe-mode-<kernel version>`,
    /// written beside its kernel's own. Never counted, condemned or given tries (choosing it spends
    /// none, and a pending erase never makes it the default); it goes with its kernel.
    pub fn is_safe_mode(&self) -> bool {
        self.id
            .strip_prefix(crate::layout::SAFE_MODE_PREFIX)
            .is_some_and(|version| version.starts_with(|c: char| c.is_ascii_digit()))
    }
}

fn parse_counter(counter: &str) -> Option<Tries> {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    match counter.split_once('-') {
        Some((left, done)) if digits(left) && digits(done) => Some(Tries {
            left: left.parse().ok()?,
            done: done.parse().ok()?,
        }),
        None if digits(counter) => Some(Tries {
            left: counter.parse().ok()?,
            done: 0,
        }),
        _ => None,
    }
}

/// The entries of an EFI system partition (`<esp>/loader/entries`).
pub struct Entries {
    directory: PathBuf,
}

impl Entries {
    pub fn new(esp: &Path) -> Self {
        Self {
            directory: esp.join("loader/entries"),
        }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Every `.conf` entry, by file name.
    pub fn list(&self) -> io::Result<Vec<Entry>> {
        let mut entries = Vec::new();
        for item in fs::read_dir(&self.directory)? {
            let item = item?;
            if !item.file_type()?.is_file() {
                continue;
            }
            if let Some(entry) = item.file_name().to_str().and_then(Entry::parse) {
                entries.push(entry);
            }
        }
        entries.sort_by(|a, b| a.file_name.cmp(&b.file_name));
        Ok(entries)
    }

    /// OxyOS's kernels' entries.
    pub fn system(&self) -> io::Result<Vec<Entry>> {
        Ok(self.list()?.into_iter().filter(Entry::is_system).collect())
    }

    /// Whether some OxyOS entry may still start: one that is not bad.
    pub fn system_may_start(&self) -> io::Result<bool> {
        Ok(self.system()?.iter().any(|e| !e.is_bad()))
    }

    fn rename(&self, entry: &Entry, tries: Option<Tries>) -> io::Result<bool> {
        let name = entry.file_name_with(tries);
        if name == entry.file_name {
            return Ok(false);
        }
        fs::rename(
            self.directory.join(&entry.file_name),
            self.directory.join(&name),
        )?;
        Ok(true)
    }

    /// The directory's renames flushed to the disk: the EFI system partition is FAT, and a rename
    /// a power cut loses would count a boot that never happened, or forget one that did.
    fn flush(&self) -> io::Result<()> {
        fs::File::open(&self.directory)?.sync_all()
    }

    /// After a complete boot: the entry that booted (`booted`, the loader's `LoaderEntrySelected`,
    /// with or without its counter) gets `tries` tries again. Returns the file it renamed, none when
    /// the entry is not OxyOS's or is gone.
    pub fn rearm(&self, booted: &str, tries: u32) -> io::Result<Option<String>> {
        let Some(booted) = Entry::parse(booted) else {
            return Ok(None);
        };
        let Some(entry) = self.system()?.into_iter().find(|e| e.id == booted.id) else {
            return Ok(None);
        };
        let changed = self.rename(
            &entry,
            Some(Tries {
                left: tries,
                done: 0,
            }),
        )?;
        self.flush()?;
        Ok(changed.then(|| {
            entry.file_name_with(Some(Tries {
                left: tries,
                done: 0,
            }))
        }))
    }

    /// Every OxyOS entry gets `tries` tries again: the recovery's *Restart*, and the end of an
    /// erase or a reinstall.
    pub fn rearm_all(&self, tries: u32) -> io::Result<usize> {
        let mut changed = 0;
        for entry in self.system()? {
            changed += usize::from(self.rename(
                &entry,
                Some(Tries {
                    left: tries,
                    done: 0,
                }),
            )?);
        }
        self.flush()?;
        Ok(changed)
    }

    /// Every OxyOS entry is left with no tries: until they are given back, the machine starts the
    /// recovery, whatever happens in between (a power cut, a key held at the menu). What was done
    /// is kept.
    pub fn condemn_all(&self) -> io::Result<usize> {
        let mut changed = 0;
        for entry in self.system()? {
            let done = entry.tries.map_or(0, |t| t.done + t.left);
            changed += usize::from(self.rename(&entry, Some(Tries { left: 0, done }))?);
        }
        self.flush()?;
        Ok(changed)
    }

    /// Safe Mode's entries.
    pub fn safe_mode(&self) -> io::Result<Vec<Entry>> {
        Ok(self
            .list()?
            .into_iter()
            .filter(Entry::is_safe_mode)
            .collect())
    }

    /// Writes Safe Mode's entry for the kernel `version` from the kernel's own, as kernel-install
    /// wrote it (`<token>-<version>`, counted or not), whole or not at all: the `.new` file written
    /// and flushed, then renamed, the directory flushed. Without the kernel's entry nothing is
    /// written. The file written.
    pub fn write_safe_mode(&self, token: &str, version: &str) -> io::Result<String> {
        let entry = self.safe_mode_text(token, version)?;
        let name = crate::layout::safe_mode_entry_name(version);
        self.write_whole(&name, &entry)?;
        Ok(name)
    }

    /// Safe Mode's entry for the kernel `version`, made from the kernel's own (`<token>-<version>`,
    /// counted or not).
    fn safe_mode_text(&self, token: &str, version: &str) -> io::Result<String> {
        let id = format!("{token}-{version}");
        let kernel = self
            .list()?
            .into_iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no entry {id}")))?;
        let text = fs::read_to_string(self.directory.join(&kernel.file_name))?;
        crate::layout::safe_mode_entry(&text).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} names no kernel or command line", kernel.file_name),
            )
        })
    }

    /// `text` as the entry `name`, whole or not at all: the `.new` file written and flushed, then
    /// renamed, the directory flushed.
    fn write_whole(&self, name: &str, text: &str) -> io::Result<()> {
        let new = self.directory.join(format!("{name}.new"));
        fs::write(&new, text)?;
        fs::File::open(&new)?.sync_all()?;
        fs::rename(&new, self.directory.join(name))?;
        self.flush()
    }

    /// Safe Mode's entries as the kernels' are (ADR 0054, decision 10): one for every kernel whose
    /// entry kernel-install wrote (`<token>-<version>`, the version beginning with a digit, counted
    /// or not), written when it is missing or differs from what the kernel's entry makes, and one
    /// whose kernel's entry is gone removed. What a machine converted before Safe Mode, or one
    /// whose plugin was not installed when its kernels were, is brought to; run again, it changes
    /// nothing. The files written and the ones removed.
    pub fn sync_safe_mode(&self, token: &str) -> io::Result<(Vec<String>, Vec<String>)> {
        let prefix = format!("{token}-");
        let entries = self.list()?;
        let versions: Vec<&str> = entries
            .iter()
            .filter(|entry| !entry.is_safe_mode())
            .filter_map(|entry| entry.id.strip_prefix(&prefix))
            .filter(|version| version.starts_with(|c: char| c.is_ascii_digit()))
            .collect();
        let mut written = Vec::new();
        for version in &versions {
            let text = self.safe_mode_text(token, version)?;
            let name = crate::layout::safe_mode_entry_name(version);
            if fs::read_to_string(self.directory.join(&name))
                .ok()
                .as_deref()
                != Some(&text)
            {
                self.write_whole(&name, &text)?;
                written.push(name);
            }
        }
        let mut removed = Vec::new();
        for entry in entries.iter().filter(|entry| entry.is_safe_mode()) {
            let version = &entry.id[crate::layout::SAFE_MODE_PREFIX.len()..];
            if !versions.contains(&version) {
                fs::remove_file(self.directory.join(&entry.file_name))?;
                removed.push(entry.file_name.clone());
            }
        }
        if !removed.is_empty() {
            self.flush()?;
        }
        Ok((written, removed))
    }

    /// Removes Safe Mode's entry for the kernel `version`: whether it was there.
    pub fn remove_safe_mode_of(&self, version: &str) -> io::Result<bool> {
        match fs::remove_file(
            self.directory
                .join(crate::layout::safe_mode_entry_name(version)),
        ) {
            Ok(()) => {
                self.flush()?;
                Ok(true)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Removes every Safe Mode entry (an erase's kernels bring their own), returning their ids.
    pub fn remove_safe_mode(&self) -> io::Result<Vec<String>> {
        let mut removed = Vec::new();
        for entry in self.safe_mode()? {
            fs::remove_file(self.directory.join(&entry.file_name))?;
            removed.push(entry.id);
        }
        self.flush()?;
        Ok(removed)
    }

    /// Removes every OxyOS entry (a reinstall's kernels replace them), returning their ids.
    pub fn remove_system(&self) -> io::Result<Vec<String>> {
        let mut removed = Vec::new();
        for entry in self.system()? {
            fs::remove_file(self.directory.join(&entry.file_name))?;
            removed.push(entry.id);
        }
        self.flush()?;
        Ok(removed)
    }
}

/// The boot loader's EFI variables (systemd's vendor GUID), as efivarfs shows them: four bytes of
/// attributes, then the value.
pub const LOADER_VARIABLES: &str = "4a67b082-0a4c-41cf-b6c7-440b29bb8c4f";

/// A string variable of the boot loader's (`LoaderEntrySelected`), from efivarfs at `efivars`:
/// UTF-16LE, ending in NUL. None when the variable is not there.
pub fn loader_string(efivars: &Path, name: &str) -> io::Result<Option<String>> {
    let path = efivars.join(format!("{name}-{LOADER_VARIABLES}"));
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    Ok(Some(decode_utf16_variable(&bytes)))
}

/// An efivarfs file's value as text: its attributes skipped, UTF-16LE up to the first NUL.
pub fn decode_utf16_variable(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .get(4..)
        .unwrap_or_default()
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}
