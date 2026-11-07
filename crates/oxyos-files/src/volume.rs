//! Which volume an item is on (docs/adr/0047-finder-drags-and-the-desktop.mdx, decision 8): the
//! file system that holds it, named by its device number as `MAJOR:MINOR`. Two items are on one
//! volume exactly when a move between them is a rename (across two, `rename(2)` fails with
//! `EXDEV` and a move is a copy), which is what Finder's choice between a move and a copy rests
//! on. A volume names no path: a drag tells it to targets, which never see the dragged files'
//! URIs before the drop.

use crate::{uri, Error};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// The volume of the item a `file://` URI names: the item's own, a link's own rather than what it
/// leads to (a move moves the link), a folder's own however its URI ends.
pub fn of(item: &str) -> Result<String, Error> {
    let path = uri::path_of(item)?;
    Ok(name(std::fs::symlink_metadata(Path::new(&path))?.dev()))
}

/// The volume items dropped into the folder a `file://` URI names go to: the folder's own, or, for
/// a link to a folder, that of the folder it leads to, where they land.
pub fn inside(folder: &str) -> Result<String, Error> {
    let path = uri::path_of(folder)?;
    Ok(name(std::fs::metadata(Path::new(&path))?.dev()))
}

/// A device number as the platform names a volume.
fn name(device: u64) -> String {
    format!(
        "{}:{}",
        rustix::fs::major(device),
        rustix::fs::minor(device)
    )
}
