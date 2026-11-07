//! The person's tags on their files, as a desktop's Finder keeps them (docs/adr/0049-file-tags.mdx):
//! in the item's extended attributes, so they go wherever the item goes and every program that
//! reads the convention sees them.
//!
//! - **Written:** `user.xdg.tags`, freedesktop.org's convention (what KDE's Baloo and Dolphin
//!   write): the names, UTF-8, joined by `,`, nothing added. Removing the last tag removes it.
//! - **Read:** `user.xdg.tags` when the item has it (it wins, even empty); otherwise a Mac's
//!   `com.apple.metadata:_kMDItemUserTags` as Linux shows it, `user.` (netatalk, `rsync -X` and
//!   `tar --xattrs` from a Mac) or `osx.` (the hfsplus driver): a binary property list, an array of
//!   `Name` or `Name\nN`, N Finder's colour (0 none, 1 grey, 2 green, 3 purple, 4 blue, 5 yellow,
//!   6 red, 7 orange). A tag's colour is what that attribute says of it; the freedesktop one has
//!   none.
//! - **A Mac's attribute is kept true:** an item that has it has it rewritten by a change (the same
//!   tags, the change's colours), so the item taken back to a Mac shows the person's edit; an item
//!   without it never gains it. It is read and written by the `plist` crate.
//! - **Names** compare as their Unicode lowercase; the first spelling is kept. A name is 1 to 255
//!   bytes once trimmed, with no `,`, newline or other control character (the two conventions'
//!   separators).
//! - **Links** are followed: Linux keeps no `user.` attribute on a symbolic link, so a tag on a link
//!   is a tag on what it leads to.

use crate::Error;
use rustix::fs::XattrFlags;
use rustix::io::Errno;
use serde_json::{json, Value};
use std::fs::File;
use std::io;
use std::path::Path;

/// freedesktop.org's attribute, the one written.
pub const XDG: &str = "user.xdg.tags";
/// A Mac's attribute as netatalk, `rsync -X` and `tar --xattrs` keep it.
pub const MAC: &str = "user.com.apple.metadata:_kMDItemUserTags";
/// A Mac's attribute as the hfsplus driver shows it.
pub const MAC_HFS: &str = "osx.com.apple.metadata:_kMDItemUserTags";

/// The longest tag name, in bytes.
pub const MAX_NAME_BYTES: usize = 255;

/// A tag's colour: Finder's seven.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Color {
    Grey,
    Green,
    Purple,
    Blue,
    Yellow,
    Red,
    Orange,
}

impl Color {
    /// In Finder's order of its numbers (1 to 7).
    pub const ALL: [Color; 7] = [
        Color::Grey,
        Color::Green,
        Color::Purple,
        Color::Blue,
        Color::Yellow,
        Color::Red,
        Color::Orange,
    ];

    /// Its name in answers.
    pub fn name(self) -> &'static str {
        match self {
            Color::Grey => "grey",
            Color::Green => "green",
            Color::Purple => "purple",
            Color::Blue => "blue",
            Color::Yellow => "yellow",
            Color::Red => "red",
            Color::Orange => "orange",
        }
    }

    pub fn parse(name: &str) -> Option<Color> {
        Color::ALL.into_iter().find(|color| color.name() == name)
    }

    /// Finder's number for it.
    fn finder(self) -> u8 {
        match self {
            Color::Grey => 1,
            Color::Green => 2,
            Color::Purple => 3,
            Color::Blue => 4,
            Color::Yellow => 5,
            Color::Red => 6,
            Color::Orange => 7,
        }
    }

    /// The colour of Finder's number; none for 0 and anything else.
    fn of_finder(number: u8) -> Option<Color> {
        Color::ALL
            .into_iter()
            .find(|color| color.finder() == number)
    }
}

/// A tag on an item: its name and, where the item says one, its colour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    pub color: Option<Color>,
}

impl Tag {
    pub fn new(name: &str, color: Option<Color>) -> Tag {
        Tag {
            name: name.to_string(),
            color,
        }
    }

    /// `{name, color}`, the colour null when the item says none.
    pub fn json(&self) -> Value {
        json!({ "name": self.name, "color": self.color.map(Color::name) })
    }
}

/// Tags as JSON.
pub fn json(tags: &[Tag]) -> Value {
    tags.iter().map(Tag::json).collect()
}

/// What two spellings of a name are the same tag by: its Unicode lowercase.
pub fn key(name: &str) -> String {
    name.to_lowercase()
}

/// A tag's name as given, trimmed, if it can be one: 1 to 255 bytes, no `,` (freedesktop.org's
/// separator), newline (a Mac's) or other control character.
pub fn valid(name: &str) -> Result<&str, Error> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.len() > MAX_NAME_BYTES
        || trimmed.chars().any(|c| c == ',' || c.is_control())
    {
        return Err(Error::Invalid(format!("{name:?} cannot be a tag's name")));
    }
    Ok(trimmed)
}

/// What an item's attributes say of its tags.
#[derive(Debug, Default)]
struct Held {
    /// The tags, in their order.
    tags: Vec<Tag>,
    /// Whether it has freedesktop.org's attribute.
    xdg: bool,
    /// A Mac's attributes it has.
    macs: Vec<&'static str>,
}

/// The tags of the item at `path`, links followed; none where its volume keeps no attributes.
pub fn read(path: &Path) -> Result<Vec<Tag>, Error> {
    Ok(held(path)?.tags)
}

fn held(path: &Path) -> Result<Held, Error> {
    let names = match names(path) {
        Ok(names) => names,
        Err(errno) if errno == Errno::NOTSUP => return Ok(Held::default()),
        Err(errno) => return Err(errno.into()),
    };
    let has = |wanted: &str| names.iter().any(|name| name == wanted);
    let macs: Vec<&'static str> = [MAC, MAC_HFS].into_iter().filter(|mac| has(mac)).collect();
    let xdg = has(XDG);
    if !xdg && macs.is_empty() {
        return Ok(Held::default());
    }
    // A Mac's tags, with their colours: the first of its attributes that reads as them.
    let mut mac_tags = None;
    for mac in &macs {
        if let Some(value) = get(path, mac)? {
            if let Some(tags) = from_property_list(&value) {
                mac_tags = Some(tags);
                break;
            }
        }
    }
    let xdg_value = if xdg { get(path, XDG)? } else { None };
    let tags = match xdg_value {
        Some(value) => {
            let colours = mac_tags.unwrap_or_default();
            from_xdg(&value)
                .into_iter()
                .map(|name| {
                    let color = colours
                        .iter()
                        .find(|tag| key(&tag.name) == key(&name))
                        .and_then(|tag| tag.color);
                    Tag { name, color }
                })
                .collect()
        }
        None => mac_tags.unwrap_or_default(),
    };
    Ok(Held { tags, xdg, macs })
}

/// freedesktop.org's value: its names, trimmed, each once (case aside), none empty.
fn from_xdg(value: &[u8]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for name in String::from_utf8_lossy(value).split(',') {
        let name = name.trim();
        if !name.is_empty() && !names.iter().any(|seen| key(seen) == key(name)) {
            names.push(name.to_string());
        }
    }
    names
}

/// A Mac's value: an array of `Name` or `Name\nN`; none if it is not one.
fn from_property_list(value: &[u8]) -> Option<Vec<Tag>> {
    let list = plist::Value::from_reader(io::Cursor::new(value)).ok()?;
    let mut tags: Vec<Tag> = Vec::new();
    for item in list.as_array()? {
        let Some(text) = item.as_string() else {
            continue;
        };
        let (name, color) = match text.rsplit_once('\n') {
            Some((name, number)) => match number.trim().parse::<u8>() {
                Ok(number) => (name, Color::of_finder(number)),
                Err(_) => (text, None),
            },
            None => (text, None),
        };
        let name = name.trim();
        if !name.is_empty() && !tags.iter().any(|seen| key(&seen.name) == key(name)) {
            tags.push(Tag::new(name, color));
        }
    }
    Some(tags)
}

/// A Mac's value for `tags`: a binary property list, `Name\nN` for a coloured tag and `Name` for
/// one without, as Finder writes them.
fn property_list(tags: &[Tag]) -> Result<Vec<u8>, Error> {
    let list = plist::Value::Array(
        tags.iter()
            .map(|tag| {
                plist::Value::String(match tag.color {
                    Some(color) => format!("{}\n{}", tag.name, color.finder()),
                    None => tag.name.clone(),
                })
            })
            .collect(),
    );
    let mut bytes = Vec::new();
    list.to_writer_binary(&mut bytes)
        .map_err(|error| Error::Io(io::Error::other(error)))?;
    Ok(bytes)
}

/// Changes the tags of the item at `path` (links followed): `remove` first, each name gone (case
/// aside), then `add`, each name there once. A name the item has keeps its place and spelling and
/// takes the colour given, if any. One just removed comes back where it was, spelt and coloured as
/// added (removing `work` and adding `Work` respells a tag in place). A new name takes the place
/// of the first name removed and not added back, in order (removing `Red` and adding `Crimson`
/// renames a tag in place, as Finder renames one on every item); any more go last. Names are
/// checked first ([`valid`]). Its tags now.
pub fn change(path: &Path, add: &[Tag], remove: &[String]) -> Result<Vec<Tag>, Error> {
    let removed: Vec<String> = remove
        .iter()
        .map(|name| valid(name).map(key))
        .collect::<Result<_, _>>()?;
    let mut added: Vec<Tag> = Vec::new();
    for tag in add {
        let name = valid(&tag.name)?;
        match added.iter_mut().find(|seen| key(&seen.name) == key(name)) {
            Some(seen) => seen.color = tag.color.or(seen.color),
            None => added.push(Tag::new(name, tag.color)),
        }
    }
    // A volume that keeps no attributes has no tags, and says so when one is added.
    let held = held(path)?;
    // Each tag the item has, and whether it is removed.
    let mut slots: Vec<(Tag, bool)> = held
        .tags
        .iter()
        .map(|tag| (tag.clone(), removed.contains(&key(&tag.name))))
        .collect();
    // First the names the item has: kept, or back where they were.
    let mut new = Vec::new();
    for tag in added {
        match slots
            .iter_mut()
            .find(|(held, _)| key(&held.name) == key(&tag.name))
        {
            Some((held, gone)) if *gone => {
                *held = Tag::new(&tag.name, tag.color.or(held.color));
                *gone = false;
            }
            Some((held, _)) => held.color = tag.color.or(held.color),
            None => new.push(tag),
        }
    }
    // Then the new ones: each in the place of a name removed and not added back, then last.
    for tag in new {
        match slots.iter_mut().find(|(_, gone)| *gone) {
            Some((held, gone)) => {
                *held = Tag::new(&tag.name, tag.color.or(held.color));
                *gone = false;
            }
            None => slots.push((tag, false)),
        }
    }
    // Colours are kept where a Mac's attribute keeps them; freedesktop's keeps names alone.
    let coloured = !held.macs.is_empty();
    let tags: Vec<Tag> = slots
        .into_iter()
        .filter(|(_, gone)| !gone)
        .map(|(tag, _)| Tag {
            color: tag.color.filter(|_| coloured),
            ..tag
        })
        .collect();
    if tags == held.tags {
        return Ok(tags);
    }
    let written = if tags.is_empty() {
        let mut gone = Ok(());
        for name in held.macs.iter().copied().chain(held.xdg.then_some(XDG)) {
            gone = gone.and(remove_attribute(path, name));
        }
        gone
    } else {
        let names: Vec<&str> = tags.iter().map(|tag| tag.name.as_str()).collect();
        let mut set = set(path, XDG, names.join(",").as_bytes());
        if !held.macs.is_empty() {
            let list = property_list(&tags)?;
            for mac in &held.macs {
                set = set.and_then(|()| self::set(path, mac, &list));
            }
        }
        set
    };
    match written {
        Ok(()) => Ok(tags),
        Err(errno) if errno == Errno::NOTSUP => Err(unsupported()),
        Err(errno) => Err(errno.into()),
    }
}

fn unsupported() -> Error {
    Error::Unsupported("the volume the item is on keeps no tags".into())
}

/// The names of the attributes of the item at `path`, links followed.
fn names(path: &Path) -> Result<Vec<String>, Errno> {
    let list = loop {
        let size = rustix::fs::listxattr_query(path)?;
        if size == 0 {
            return Ok(Vec::new());
        }
        let mut list = Vec::with_capacity(size);
        match rustix::fs::listxattr(path, rustix::buffer::spare_capacity(&mut list)) {
            Ok(_) => break list,
            // It grew between the two calls: asked again.
            Err(Errno::RANGE) => continue,
            Err(errno) => return Err(errno),
        }
    };
    Ok(list
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
        .map(|name| String::from_utf8_lossy(name).into_owned())
        .collect())
}

/// The value of the attribute `name` of the item at `path`, links followed; none if it has none.
fn get(path: &Path, name: &str) -> Result<Option<Vec<u8>>, Errno> {
    loop {
        let size = match rustix::fs::getxattr_query(path, name) {
            Ok(size) => size,
            Err(Errno::NODATA) => return Ok(None),
            Err(errno) => return Err(errno),
        };
        if size == 0 {
            return Ok(Some(Vec::new()));
        }
        let mut value = Vec::with_capacity(size);
        match rustix::fs::getxattr(path, name, rustix::buffer::spare_capacity(&mut value)) {
            Ok(_) => return Ok(Some(value)),
            Err(Errno::RANGE) => continue,
            Err(Errno::NODATA) => return Ok(None),
            Err(errno) => return Err(errno),
        }
    }
}

fn set(path: &Path, name: &str, value: &[u8]) -> Result<(), Errno> {
    rustix::fs::setxattr(path, name, value, XattrFlags::empty())
}

/// Removes the attribute `name`; one already gone is gone.
fn remove_attribute(path: &Path, name: &str) -> Result<(), Errno> {
    match rustix::fs::removexattr(path, name) {
        Err(Errno::NODATA) => Ok(()),
        other => other,
    }
}

/// Whether the item at `path` (itself, not what a link leads to) has an attribute that may say
/// tags: one cheap look the index takes at every item before it reads any.
pub(crate) fn may_have_tags(path: &Path) -> bool {
    let list = loop {
        let Ok(size) = rustix::fs::llistxattr_query(path) else {
            return false;
        };
        if size == 0 {
            return false;
        }
        let mut list = Vec::with_capacity(size);
        match rustix::fs::llistxattr(path, rustix::buffer::spare_capacity(&mut list)) {
            Ok(_) => break list,
            Err(Errno::RANGE) => continue,
            Err(_) => return false,
        }
    };
    list.split(|byte| *byte == 0)
        .any(|name| name == XDG.as_bytes() || name == MAC.as_bytes() || name == MAC_HFS.as_bytes())
}

/// Gives the copy `to` (a file open for writing) every `user.` attribute of `from`. A volume
/// that keeps no attributes (FAT, most network shares) keeps none, as `cp -a` leaves them there.
pub(crate) fn copy_to_file(from: &Path, to: &File) -> Result<(), Error> {
    copy_with(from, |name, value| {
        rustix::fs::fsetxattr(to, name, value, XattrFlags::empty())
    })
}

/// Gives the folder `to` every `user.` attribute of the folder `from`, as [`copy_to_file`].
pub(crate) fn copy_to_folder(from: &Path, to: &Path) -> Result<(), Error> {
    copy_with(from, |name, value| set(to, name, value))
}

fn copy_with(
    from: &Path,
    mut set: impl FnMut(&str, &[u8]) -> Result<(), Errno>,
) -> Result<(), Error> {
    let names = match names(from) {
        Ok(names) => names,
        Err(Errno::NOTSUP) => return Ok(()),
        Err(errno) => return Err(errno.into()),
    };
    for name in names.iter().filter(|name| name.starts_with("user.")) {
        let Some(value) = get(from, name)? else {
            continue;
        };
        match set(name, &value) {
            Ok(()) => {}
            Err(Errno::NOTSUP) => return Ok(()),
            Err(errno) => return Err(errno.into()),
        }
    }
    Ok(())
}
