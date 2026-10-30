//! A package's files on disk, checked against its manifest.
//!
//! The tree is walked by descriptor: each directory is opened relative to its parent's
//! descriptor with `O_NOFOLLOW`, and each file is opened the same way and judged by `fstat` on
//! what was opened, so a link is never followed and a path swapped while the walk runs is never
//! read in place of the one judged.

use crate::manifest::{FileEntry, Manifest, Sha256, MAX_MANIFEST_SIZE, RESERVED};
use crate::signature::DeveloperSignature;
use crate::Error;
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::fs::{fstat, openat, AtFlags, Dir, FileType, Mode, OFlags, CWD};
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// The first eight bytes of a Hermes bytecode file (`HBCFileHeader`'s magic,
/// `0x1F1903C103BC1FC6`, little-endian): a package's entry must not start with them.
pub const HERMES_BYTECODE_MAGIC: [u8; 8] = [0xc6, 0x1f, 0xbc, 0x03, 0xc1, 0x03, 0x19, 0x1f];

const CHUNK: usize = 64 << 10;

fn io_error(path: &str, error: impl std::fmt::Display) -> Error {
    Error::Io {
        path: path.to_string(),
        error: error.to_string(),
    }
}

/// Opens a package's directory, refusing a link at its last component.
pub fn open_directory(path: &Path) -> Result<OwnedFd, Error> {
    let shown = path.display().to_string();
    openat(
        CWD,
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| match e {
        rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR => Error::Link(shown.clone()),
        e => io_error(&shown, e),
    })
}

/// Opens `name` in `dir` as a regular file, by descriptor, never through a link.
fn open_file(dir: BorrowedFd, name: &OsStr, shown: &str) -> Result<(File, u64), Error> {
    let fd = openat(
        dir,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|e| match e {
        rustix::io::Errno::LOOP => Error::Link(shown.to_string()),
        rustix::io::Errno::NOENT => Error::Missing(shown.to_string()),
        e => io_error(shown, e),
    })?;
    let stat = fstat(&fd).map_err(|e| io_error(shown, e))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(Error::NotRegular(shown.to_string()));
    }
    Ok((File::from(fd), stat.st_size as u64))
}

/// Reads a file of at most `limit` bytes.
fn read_bounded(
    file: &mut File,
    limit: usize,
    what: &'static str,
    shown: &str,
) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| io_error(shown, e))?;
    if bytes.len() > limit {
        return Err(Error::TooLarge(what));
    }
    Ok(bytes)
}

/// A package's manifest and signature, read from its directory; nothing is verified but their
/// form. The caller verifies the signature ([`DeveloperSignature::verify`]), the store's
/// statement and the tree ([`verify_tree`]).
#[derive(Clone, Debug)]
pub struct Package {
    /// The manifest's bytes, exactly as read: what the signatures are over.
    pub manifest_bytes: Vec<u8>,
    pub manifest: Manifest,
    pub signature: DeveloperSignature,
}

pub fn read_package(dir: BorrowedFd) -> Result<Package, Error> {
    let (mut file, _) = open_file(dir, OsStr::new("manifest.json"), "manifest.json")?;
    let manifest_bytes = read_bounded(
        &mut file,
        MAX_MANIFEST_SIZE,
        "the manifest",
        "manifest.json",
    )?;
    let manifest = Manifest::parse(&manifest_bytes)?;
    let (mut file, _) = open_file(dir, OsStr::new("manifest.sig"), "manifest.sig")?;
    let signature = DeveloperSignature::parse(&read_bounded(
        &mut file,
        64 << 10,
        "manifest.sig",
        "manifest.sig",
    )?)?;
    Ok(Package {
        manifest_bytes,
        manifest,
        signature,
    })
}

/// Copies a stream to `out` while checking it against the size and digest a manifest lists,
/// stopping as soon as it is longer than the size. The bytes copied are only good once this
/// returns `Ok`; `shown` names the file in errors.
pub fn copy_verified(
    input: &mut impl Read,
    out: &mut impl Write,
    size: u64,
    sha256: &Sha256,
    shown: &str,
) -> Result<(), Error> {
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    let mut total: u64 = 0;
    let mut buffer = vec![0; CHUNK];
    loop {
        // Read at most one byte past the size, which is enough to know it is too long.
        let room = (size - total).saturating_add(1).min(CHUNK as u64) as usize;
        let n = match input.read(&mut buffer[..room]) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(io_error(shown, e)),
        };
        total += n as u64;
        if total > size {
            return Err(Error::SizeMismatch(shown.to_string()));
        }
        context.update(&buffer[..n]);
        out.write_all(&buffer[..n])
            .map_err(|e| io_error(shown, e))?;
    }
    if total != size {
        return Err(Error::SizeMismatch(shown.to_string()));
    }
    if context.finish().as_ref() != sha256.0 {
        return Err(Error::DigestMismatch(shown.to_string()));
    }
    Ok(())
}

/// Checks a stream against a size and digest, keeping nothing.
pub fn verify_stream(
    input: &mut impl Read,
    size: u64,
    sha256: &Sha256,
    shown: &str,
) -> Result<(), Error> {
    copy_verified(input, &mut io::sink(), size, sha256, shown)
}

/// Checks a package's tree against its manifest: every listed file present as a regular file
/// (not a link) of its size and digest, nothing unlisted (a file, a directory, a link, a device)
/// but `manifest.json`, `manifest.sig` and the names in `also_at_top` at its top, which must be
/// [`RESERVED`] names (`index.hbc`, which `oxyos-installd` compiles there), and the entry's
/// bundle not bytecode.
pub fn verify_tree(
    dir: BorrowedFd,
    manifest: &Manifest,
    also_at_top: &[&str],
) -> Result<(), Error> {
    for name in also_at_top {
        if !RESERVED.contains(name) {
            return Err(Error::ReservedPath(name.to_string()));
        }
    }
    let files: HashMap<&str, &FileEntry> = manifest
        .files
        .iter()
        .map(|f| (f.path.as_str(), f))
        .collect();
    let mut directories = HashSet::new();
    for file in &manifest.files {
        let path = file.path.as_str();
        let mut end = 0;
        while let Some(i) = path[end..].find('/') {
            end += i;
            directories.insert(&path[..end]);
            end += 1;
        }
    }
    let mut walk = Walk {
        manifest,
        files,
        directories,
        also_at_top,
        seen: HashSet::new(),
    };
    walk.directory(dir, "")?;
    for file in &manifest.files {
        if !walk.seen.contains(file.path.as_str()) {
            return Err(Error::Missing(file.path.to_string()));
        }
    }
    Ok(())
}

struct Walk<'a> {
    manifest: &'a Manifest,
    files: HashMap<&'a str, &'a FileEntry>,
    directories: HashSet<&'a str>,
    also_at_top: &'a [&'a str],
    seen: HashSet<String>,
}

impl Walk<'_> {
    fn directory(&mut self, dir: BorrowedFd, prefix: &str) -> Result<(), Error> {
        let shown_dir = if prefix.is_empty() { "." } else { prefix };
        let entries = Dir::read_from(dir).map_err(|e| io_error(shown_dir, e))?;
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| io_error(shown_dir, e))?;
            let name = entry.file_name().to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            names.push(name.to_vec());
        }
        names.sort();
        for name in names {
            let name = OsStr::from_bytes(&name);
            let text = name.to_string_lossy();
            let path = if prefix.is_empty() {
                text.to_string()
            } else {
                format!("{prefix}/{text}")
            };
            let at_top = prefix.is_empty();
            let stat = rustix::fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|e| io_error(&path, e))?;
            match FileType::from_raw_mode(stat.st_mode) {
                FileType::Symlink => return Err(Error::Link(path)),
                FileType::Directory => {
                    if !self.directories.contains(path.as_str()) {
                        return Err(Error::Unlisted(path));
                    }
                    let child = openat(
                        dir,
                        name,
                        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                        Mode::empty(),
                    )
                    .map_err(|e| match e {
                        rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR => {
                            Error::Link(path.clone())
                        }
                        e => io_error(&path, e),
                    })?;
                    self.directory(child.as_fd(), &path)?;
                }
                FileType::RegularFile => {
                    if at_top
                        && (path == "manifest.json"
                            || path == "manifest.sig"
                            || self.also_at_top.contains(&path.as_str()))
                    {
                        continue;
                    }
                    let Some(entry) = self.files.get(path.as_str()).copied() else {
                        return Err(Error::Unlisted(path));
                    };
                    let (mut file, size) = open_file(dir, name, &path)?;
                    if size != entry.size {
                        return Err(Error::SizeMismatch(path));
                    }
                    if entry.path == self.manifest.entry.bundle {
                        let mut head = [0; 8];
                        let mut read = 0;
                        while read < head.len() {
                            match file.read(&mut head[read..]) {
                                Ok(0) => break,
                                Ok(n) => read += n,
                                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                                Err(e) => return Err(io_error(&path, e)),
                            }
                        }
                        if read == head.len() && head == HERMES_BYTECODE_MAGIC {
                            return Err(Error::Bytecode);
                        }
                        let mut chained = head[..read].chain(&mut file);
                        verify_stream(&mut chained, entry.size, &entry.sha256, &path)?;
                    } else {
                        verify_stream(&mut file, entry.size, &entry.sha256, &path)?;
                    }
                    self.seen.insert(path);
                }
                _ => return Err(Error::NotRegular(path)),
            }
        }
        Ok(())
    }
}
