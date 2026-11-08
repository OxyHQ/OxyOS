//! What a tree occupies on the disk (docs/adr/0062-storage.mdx): its blocks, as `du -B1` counts
//! them, which is what the volume's own figures (`statvfs`) count, so the parts of Settings ›
//! Storage add up to what the volume says is used.
//!
//! - **Blocks, not lengths.** Each item counts `st_blocks` × 512: a sparse file counts what it
//!   holds, a small file the block it takes, a folder its own blocks too.
//! - **Each inode once.** A file linked twice inside the tree counts once.
//! - **Within the tree's file system.** Links are never followed, and a file system mounted inside
//!   the tree is not entered: it is another volume's.
//! - **Bounded.** The walk keeps no more than the folders still to read, and stops when `stop`
//!   says so (the page that asked went), checked between items.

use std::collections::HashSet;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// The bytes the item at `path` occupies with everything in it, or the walk's interruption
/// (`ErrorKind::Interrupted`) when `stop` said so. Nothing there is 0.
pub fn allocated(path: &Path, stop: &dyn Fn() -> bool) -> io::Result<u64> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let device = metadata.dev();
    let mut total = blocks(&metadata);
    if !metadata.is_dir() {
        return Ok(total);
    }
    let mut linked = HashSet::new();
    let mut folders: Vec<PathBuf> = vec![path.to_path_buf()];
    while let Some(folder) = folders.pop() {
        // A folder that went or cannot be read while it is walked counts what was read of it.
        let Ok(items) = std::fs::read_dir(&folder) else {
            continue;
        };
        for item in items.flatten() {
            if stop() {
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            let Ok(metadata) = item.metadata() else {
                continue;
            };
            if metadata.dev() != device {
                continue;
            }
            if metadata.nlink() > 1 && !metadata.is_dir() && !linked.insert(metadata.ino()) {
                continue;
            }
            total += blocks(&metadata);
            if metadata.is_dir() {
                folders.push(item.path());
            }
        }
    }
    Ok(total)
}

/// The bytes an item's own blocks are.
pub fn blocks(metadata: &std::fs::Metadata) -> u64 {
    metadata.blocks().saturating_mul(512)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, Write};

    fn scratch(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("oxyos-files-usage-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn du(path: &Path) -> u64 {
        allocated(path, &|| false).unwrap()
    }

    #[test]
    fn a_tree_counts_its_blocks_each_inode_once_and_never_through_a_link() {
        let root = scratch("tree");
        let folder = root.join("data");
        std::fs::create_dir_all(folder.join("inner")).unwrap();
        std::fs::write(folder.join("a.bin"), vec![1u8; 64 * 1024]).unwrap();
        std::fs::write(folder.join("inner/b.bin"), vec![2u8; 8 * 1024]).unwrap();
        let own = |path: &Path| blocks(&std::fs::symlink_metadata(path).unwrap());
        let expected = own(&folder)
            + own(&folder.join("inner"))
            + own(&folder.join("a.bin"))
            + own(&folder.join("inner/b.bin"));
        assert_eq!(du(&folder), expected);
        // At least what the files hold: blocks, not lengths of less.
        assert!(du(&folder) >= 72 * 1024);
        // A second name of a file counts nothing more.
        std::fs::hard_link(folder.join("a.bin"), folder.join("inner/a again.bin")).unwrap();
        assert_eq!(du(&folder), expected);
        // A link to something outside is its own few bytes, never what it names.
        let outside = root.join("outside.bin");
        std::fs::write(&outside, vec![3u8; 256 * 1024]).unwrap();
        std::os::unix::fs::symlink(&outside, folder.join("link")).unwrap();
        assert!(du(&folder) < expected + 256 * 1024);
        // Nothing there is nothing.
        assert_eq!(du(&root.join("gone")), 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_sparse_file_counts_what_it_holds_and_a_walk_stops_when_asked() {
        let root = scratch("sparse");
        let path = root.join("disk.img");
        let mut file = std::fs::File::create(&path).unwrap();
        file.seek(io::SeekFrom::Start(512 * 1024 * 1024)).unwrap();
        file.write_all(b"end").unwrap();
        drop(file);
        assert!(std::fs::metadata(&path).unwrap().len() > 512 * 1024 * 1024);
        assert!(du(&path) < 1024 * 1024, "a hole occupies nothing");
        std::fs::write(root.join("other"), b"x").unwrap();
        let error = allocated(&root, &|| true).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        let _ = std::fs::remove_dir_all(&root);
    }
}
