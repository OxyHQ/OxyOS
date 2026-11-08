//! Settings › Storage for the program running the compositor (docs/adr/0059-storage.mdx): what the
//! volume holding the person's home occupies, by what occupies it, as only the session can measure
//! it (the image's packages, the Trash, every application's package and own data), and showing an
//! item in the file manager.
//!
//! - **Who may ask** (`SystemRequest`, the session's vocabulary, read strictly): Settings (the
//!   `settings` grant) alone:
//!   - `{"storage": {}}` ← `{"storage": {"volume", "system", "trash", "apps"}}`: measured when
//!     asked, on a thread of its own, and answered once measured; an ask while one runs joins it.
//!     The person is waiting for the figures, so it runs at the normal class, as a question to the
//!     search index does (ADR 0051, decision 13).
//!   - `{"reveal": {"uri"}}`, citing an input ← `{"revealed": {}}`: the item, one of the person's,
//!     shown selected in its folder by the file manager ([`oxyos_apps::reveal_url`]), opened by the
//!     program as the shell's URLs are.
//! - **What the documents and photos occupy** is not measured here: the search index keeps every
//!   file's blocks, and Settings asks it (`storage`, ADR 0051).
//! - **Nothing runs while the page is not shown.** Nothing is watched and nothing kept: a
//!   measurement runs only when asked, ends with its answer, and stops when no one who asked for it
//!   is still connected. A walk is bounded by what it measures: the Trash's folders (their sizes
//!   kept in the Trash's own cache) and each application's own data.

use oxyos_apps::{Listing, Origin, OwnData};
use oxyos_compositor_core::scene::SystemRequest;
use oxyos_compositor_core::{ClientId, Scene};
use oxyos_files::{trash, usage, Scope};
use oxyos_install::ipc::{Answer, Request};
use oxyos_install::socket::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::io::{self, Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

/// How long the install service may take to list the account's applications.
const LIST_TIMEOUT: Duration = Duration::from_secs(10);

/// What Settings asks, read strictly.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum Ask {
    Storage {},
    Reveal { uri: String },
}

const ASKS: [&str; 2] = ["storage", "reveal"];

fn ours(text: &str) -> bool {
    serde_json::from_str::<HashMap<String, serde::de::IgnoredAny>>(text)
        .is_ok_and(|asked| asked.len() == 1 && asked.keys().all(|key| ASKS.contains(&key.as_str())))
}

/// A volume's figures, in bytes, as `statvfs` gives them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Volume {
    /// Every block (`f_blocks`).
    pub capacity: u64,
    /// The blocks free (`f_bfree`), the administrator's reserve among them.
    pub free: u64,
    /// The blocks free for the person (`f_bavail`).
    pub available: u64,
}

impl Volume {
    /// What is written on it.
    pub fn used(&self) -> u64 {
        self.capacity.saturating_sub(self.free)
    }

    /// The blocks kept free for the administrator, so the system goes on working when the person
    /// fills the disk: the system's.
    pub fn reserved(&self) -> u64 {
        self.free.saturating_sub(self.available)
    }
}

/// What the measuring reads of the machine: the real one, or the development compositor's played
/// disk (`--storage FILE`), whose figures a scenario fixes.
pub trait Disk: Send + Sync {
    /// The figures of the volume holding `path`.
    fn volume(&self, path: &Path) -> io::Result<Volume>;
    /// The volume's name (its file system's label), if it has one.
    fn name(&self, path: &Path) -> Option<String>;
    /// What the system's image occupies on the volume holding `home`.
    fn system(&self, home: &Path) -> u64;
}

/// The machine's own disk.
pub struct MachineDisk;

impl Disk for MachineDisk {
    fn volume(&self, path: &Path) -> io::Result<Volume> {
        let stat = rustix::fs::statvfs(path)?;
        let bytes = |blocks: u64| blocks.saturating_mul(stat.f_frsize);
        Ok(Volume {
            capacity: bytes(stat.f_blocks),
            free: bytes(stat.f_bfree),
            available: bytes(stat.f_bavail),
        })
    }

    fn name(&self, path: &Path) -> Option<String> {
        let device = std::fs::metadata(path).ok()?.dev();
        label_of(Path::new("/dev/disk/by-label"), device)
    }

    fn system(&self, home: &Path) -> u64 {
        let Ok(device) = std::fs::metadata(home).map(|metadata| metadata.dev()) else {
            return 0;
        };
        system_on(device, Path::new("/"))
    }
}

/// A disk a scenario plays: its name, its figures and what its system occupies.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlayedDisk {
    #[serde(default)]
    pub name: Option<String>,
    pub capacity: u64,
    pub available: u64,
    #[serde(default)]
    pub reserved: u64,
    pub system: u64,
}

impl Disk for PlayedDisk {
    fn volume(&self, _: &Path) -> io::Result<Volume> {
        Ok(Volume {
            capacity: self.capacity,
            free: self.available.saturating_add(self.reserved),
            available: self.available,
        })
    }

    fn name(&self, _: &Path) -> Option<String> {
        self.name.clone()
    }

    fn system(&self, _: &Path) -> u64 {
        self.system
    }
}

/// The label of the file system on `device`, from udev's links (`/dev/disk/by-label/NAME`, its
/// name escaped as udev escapes it, `\x20` a space).
fn label_of(by_label: &Path, device: u64) -> Option<String> {
    use std::os::unix::fs::FileTypeExt;
    std::fs::read_dir(by_label)
        .ok()?
        .flatten()
        .find(|link| {
            std::fs::metadata(link.path()).is_ok_and(|metadata| {
                metadata.file_type().is_block_device() && metadata.rdev() == device
            })
        })
        .map(|link| unescape_label(&link.file_name().to_string_lossy()))
}

/// A label as udev writes it in a link's name: `\xNN` is the byte NN.
fn unescape_label(name: &str) -> String {
    let bytes = name.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'\\' && bytes.get(at + 1) == Some(&b'x') && at + 4 <= bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&name[at + 2..at + 4], 16) {
                out.push(byte);
                at += 4;
                continue;
            }
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// What the system's image occupies on the volume `device`, under the root `root`: its packages as
/// dpkg records them (`Installed-Size`) and the initramfs images made from them in `boot/`, when
/// `usr/` is on that volume; nothing when the system is a volume of its own.
fn system_on(device: u64, root: &Path) -> u64 {
    let on = |path: &Path| std::fs::metadata(path).is_ok_and(|metadata| metadata.dev() == device);
    if !on(&root.join("usr")) {
        return 0;
    }
    let packages = std::fs::read_to_string(root.join("var/lib/dpkg/status"))
        .map_or(0, |status| installed_size(&status));
    let boot = root.join("boot");
    let images = if on(&boot) {
        std::fs::read_dir(&boot)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|item| item.file_name().to_string_lossy().starts_with("initrd.img"))
            .filter_map(|item| std::fs::symlink_metadata(item.path()).ok())
            .filter(|metadata| metadata.is_file())
            .map(|metadata| usage::blocks(&metadata))
            .sum()
    } else {
        0
    };
    packages + images
}

/// The bytes dpkg's status says its installed packages occupy: each installed one's
/// `Installed-Size`, in kibibytes.
fn installed_size(status: &str) -> u64 {
    let mut total = 0;
    for stanza in status.split("\n\n") {
        let mut installed = false;
        let mut size = 0;
        for line in stanza.lines() {
            if let Some(state) = line.strip_prefix("Status:") {
                installed = state.split_whitespace().last() == Some("installed");
            } else if let Some(kib) = line.strip_prefix("Installed-Size:") {
                size = kib.trim().parse::<u64>().unwrap_or(0);
            }
        }
        if installed {
            total += size * 1024;
        }
    }
    total
}

/// What an application's own data occupies (docs/adr/0059-storage.mdx, decision 3): its data's
/// and its caches' blocks. The measure Settings › Storage shows, and the one any part that clears
/// an application's data says it freed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DataSize {
    pub data: u64,
    pub caches: u64,
}

/// Measures `own`; the walk's interruption when `stop` says so.
pub fn data_size(own: &OwnData, stop: &dyn Fn() -> bool) -> io::Result<DataSize> {
    Ok(DataSize {
        data: usage::allocated(&own.data, stop)?,
        caches: usage::allocated(&own.caches, stop)?,
    })
}

/// An application, as Storage lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Application {
    pub id: String,
    pub name: String,
    /// `system`, `installed` or `development` (ADR 0030, decision 7).
    pub origin: &'static str,
    /// Its package, when the account has one from the install service; a system application's
    /// image is the system's, a development entry's bundle the developer's.
    pub package: Option<u64>,
    pub data: u64,
    pub caches: u64,
    /// When the account last opened it, or installed it if never since (milliseconds since
    /// 1970), as the install service knows it.
    pub last_used: Option<i64>,
    /// The person may remove it (the store's uninstall): installed, never a system application.
    pub removable: bool,
}

impl Application {
    fn total(&self) -> u64 {
        self.package.unwrap_or(0) + self.data + self.caches
    }
}

/// What the session measured.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Measured {
    pub name: Option<String>,
    pub capacity: u64,
    pub available: u64,
    pub used: u64,
    pub reserved: u64,
    /// What the system's image occupies on it (the reserve apart).
    pub system: u64,
    /// What the Trashes on it occupy.
    pub trash: u64,
    /// The session's applications, the largest first.
    pub apps: Vec<Application>,
}

/// An application's own data, by its id ([`OwnData::of`] in a session).
type OwnDataOf = Arc<dyn Fn(&str) -> Option<OwnData> + Send + Sync>;

/// What a measurement needs.
#[derive(Clone)]
struct Job {
    disk: Arc<dyn Disk>,
    scope: Scope,
    listing: Option<Arc<Listing>>,
    languages: Vec<String>,
    installd: PathBuf,
    own: OwnDataOf,
}

/// The account's installed applications, as the install service lists them; none when it is not
/// there (said).
fn installed(socket: &Path) -> BTreeMap<String, oxyos_install::install::Listed> {
    let listed = (|| -> io::Result<Vec<oxyos_install::install::Listed>> {
        let connection = Connection::connect(socket)?;
        connection.set_timeout(LIST_TIMEOUT)?;
        connection.send(&Request::List { uid: None })?;
        match connection.receive::<Answer>()? {
            Some(Some(Answer::Applications(listed))) => Ok(listed),
            other => Err(io::Error::other(format!(
                "the install service said {other:?}"
            ))),
        }
    })();
    match listed {
        Ok(listed) => listed
            .into_iter()
            .map(|one| (one.id.clone(), one))
            .collect(),
        Err(error) => {
            eprintln!("[storage] the installed applications are not known: {error}");
            BTreeMap::new()
        }
    }
}

/// Measures the volume holding the person's home, what is the system's, the Trash's and each
/// application's.
fn measure(job: &Job, stop: &dyn Fn() -> bool) -> io::Result<Measured> {
    let home = job.scope.home().to_path_buf();
    let volume = job.disk.volume(&home)?;
    let device = std::fs::metadata(&home)?.dev();
    let mut trash_bytes = 0;
    for one in trash::trashes(&job.scope) {
        if std::fs::metadata(&one.dir).is_ok_and(|metadata| metadata.dev() == device) {
            trash_bytes += trash::occupied(&one, stop)?;
        }
    }
    let registered = job
        .listing
        .as_ref()
        .map(|listing| listing.applications(&job.languages))
        .unwrap_or_default()
        .into_iter()
        .map(|application| (application.id, application.name, application.origin))
        .collect();
    let apps = applications(registered, &installed(&job.installd), &*job.own, stop)?;
    Ok(Measured {
        name: job.disk.name(&home),
        capacity: volume.capacity,
        available: volume.available,
        used: volume.used(),
        reserved: volume.reserved(),
        system: job.disk.system(&home),
        trash: trash_bytes,
        apps,
    })
}

/// The session's applications (`registered`: each one's id, name and origin) as Storage lists
/// them, the largest first: each one's own data measured, its package and last use as the install
/// service lists them (`installed`).
fn applications(
    registered: Vec<(String, String, Origin)>,
    installed: &BTreeMap<String, oxyos_install::install::Listed>,
    own: &(dyn Fn(&str) -> Option<OwnData> + Send + Sync),
    stop: &dyn Fn() -> bool,
) -> io::Result<Vec<Application>> {
    let mut apps = Vec::new();
    for (id, name, origin) in registered {
        let Some(own) = own(&id) else {
            continue;
        };
        let size = data_size(&own, stop)?;
        let listed = installed.get(&id);
        apps.push(Application {
            package: listed.map(|listed| listed.size),
            data: size.data,
            caches: size.caches,
            last_used: listed
                .and_then(|listed| listed.last_used)
                .map(|seconds| seconds.saturating_mul(1000)),
            // Installed for the account, as the install service says: a system application, even
            // one the store updates, is never removed (ADR 0030, decision 11).
            removable: matches!(origin, Origin::Installed { .. }) && listed.is_some(),
            origin: match origin {
                Origin::System => "system",
                Origin::Installed { .. } => "installed",
                Origin::Development => "development",
            },
            id,
            name,
        });
    }
    apps.sort_by(|a, b| b.total().cmp(&a.total()).then_with(|| a.name.cmp(&b.name)));
    Ok(apps)
}

/// A measurement under way: who asked, and how it is stopped.
struct Measuring {
    askers: Vec<(ClientId, u32)>,
    stop: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<()>,
}

/// An item to show in the file manager, asked by Settings: the program opens the URL as it opens
/// the shell's and answers with [`Storage::revealed`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reveal {
    pub client: ClientId,
    pub serial: u32,
    pub url: String,
}

pub struct Storage {
    job: Job,
    done: mpsc::Receiver<io::Result<Measured>>,
    back: (mpsc::Sender<io::Result<Measured>>, Arc<UnixStream>),
    woken: UnixStream,
    measuring: Option<Measuring>,
    /// How many measurements ran (the tests' count of the work done).
    measured: u32,
}

impl Drop for Storage {
    fn drop(&mut self) {
        if let Some(measuring) = self.measuring.take() {
            measuring.stop.store(true, Ordering::Release);
            let _ = measuring.thread.join();
        }
    }
}

impl Storage {
    /// Storage over `scope`'s home, on `disk`, asking the install service at `installd`.
    pub fn new(scope: Scope, disk: Arc<dyn Disk>, installd: PathBuf) -> io::Result<Self> {
        let (wake, woken) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        woken.set_nonblocking(true)?;
        let (sender, done) = mpsc::channel();
        Ok(Self {
            job: Job {
                disk,
                scope,
                listing: None,
                languages: Vec::new(),
                installd,
                own: Arc::new(OwnData::of),
            },
            done,
            back: (sender, Arc::new(wake)),
            woken,
            measuring: None,
            measured: 0,
        })
    }

    /// Storage that finds an application's own data with `own` (the tests' homes).
    #[cfg(test)]
    fn with_own_data(mut self, own: OwnDataOf) -> Self {
        self.job.own = own;
        self
    }

    /// What wakes the compositor's turn when a measurement is done (`Compositor::wakers`).
    pub fn wake_fd(&self) -> io::Result<OwnedFd> {
        Ok(self.woken.try_clone()?.into())
    }

    /// The session's applications, read from `listing` at each measurement.
    pub fn list_applications(&mut self, listing: Listing) {
        self.job.listing = Some(Arc::new(listing));
    }

    /// The session's locale (a BCP 47 tag): the applications' names are in its languages. Cheap
    /// to call each turn.
    pub fn set_locale(&mut self, locale: &str) {
        let languages = oxyos_apps::languages_of_tag(locale);
        if languages != self.job.languages {
            self.job.languages = languages;
        }
    }

    /// A turn: Settings' asks, a measurement no one is waiting for any more stopped, and one done
    /// answered. Returns the items to show in the file manager.
    pub fn serve(&mut self, scene: &mut Scene) -> Vec<Reveal> {
        let mut buffer = [0u8; 64];
        while matches!((&self.woken).read(&mut buffer), Ok(read) if read > 0) {}
        let mut reveals = Vec::new();
        for request in scene.take_system_requests_where(|request| ours(&request.text)) {
            if let Some(reveal) = self.handle(scene, request) {
                reveals.push(reveal);
            }
        }
        if let Some(measuring) = self.measuring.as_mut() {
            measuring
                .askers
                .retain(|(client, _)| scene.grants_of(*client).is_some());
            if measuring.askers.is_empty() {
                measuring.stop.store(true, Ordering::Release);
            }
        }
        for measured in self.done.try_iter().collect::<Vec<_>>() {
            let Some(measuring) = self.measuring.take() else {
                continue;
            };
            let _ = measuring.thread.join();
            let text = match measured {
                Ok(measured) => json!({ "storage": measured }).to_string(),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    eprintln!("[storage] not measured: {error}");
                    json!({"failed": {"reason": "unavailable", "detail": error.to_string()}})
                        .to_string()
                }
            };
            for (client, serial) in measuring.askers {
                scene.system_text(client, serial, &text);
            }
        }
        reveals
    }

    fn handle(&mut self, scene: &mut Scene, request: SystemRequest) -> Option<Reveal> {
        let (client, serial) = (request.client, request.serial);
        let refuse = |scene: &mut Scene, reason: &str| {
            let text = json!({"refused": {"reason": reason}}).to_string();
            scene.system_text(client, serial, &text);
        };
        let Ok(ask) = serde_json::from_str::<Ask>(&request.text) else {
            refuse(scene, "invalidRequest");
            return None;
        };
        let grants = scene.grants_of(client)?;
        if !grants.settings {
            refuse(scene, "notPermitted");
            return None;
        }
        match ask {
            Ask::Storage {} => {
                self.start(client, serial);
                None
            }
            Ask::Reveal { uri } => {
                if !request.with_input {
                    refuse(scene, "needsInput");
                    return None;
                }
                // One of the person's items, there now.
                let reached = oxyos_files::uri::path_of(&uri)
                    .and_then(|path| self.job.scope.item(&path))
                    .is_ok_and(|path| std::fs::symlink_metadata(path).is_ok());
                if !reached {
                    refuse(scene, "notFound");
                    return None;
                }
                Some(Reveal {
                    client,
                    serial,
                    url: oxyos_apps::reveal_url(&uri),
                })
            }
        }
    }

    /// Measures for `client`, joining a measurement under way.
    fn start(&mut self, client: ClientId, serial: u32) {
        if let Some(measuring) = self.measuring.as_mut() {
            if !measuring.stop.load(Ordering::Acquire) {
                measuring.askers.push((client, serial));
                return;
            }
        }
        // One stopped, still ending: it is waited for, and a new one starts.
        if let Some(measuring) = self.measuring.take() {
            let _ = measuring.thread.join();
            for _ in self.done.try_iter() {}
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (job, flag) = (self.job.clone(), stop.clone());
        let (sender, wake) = (self.back.0.clone(), self.back.1.clone());
        let spawned = std::thread::Builder::new()
            .name("storage-measure".into())
            .spawn(move || {
                let measured = measure(&job, &|| flag.load(Ordering::Acquire));
                if sender.send(measured).is_ok() {
                    let _ = (&*wake).write(&[1]);
                }
            });
        match spawned {
            Ok(thread) => {
                self.measured += 1;
                self.measuring = Some(Measuring {
                    askers: vec![(client, serial)],
                    stop,
                    thread,
                });
            }
            Err(error) => eprintln!("[storage] not measured: {error}"),
        }
    }

    /// Answers Settings' reveal: whether the file manager was opened.
    pub fn revealed(&self, scene: &mut Scene, reveal: &Reveal, opened: bool) {
        let text = if opened {
            json!({"revealed": {}})
        } else {
            json!({"failed": {"reason": "notOpened"}})
        };
        scene.system_text(reveal.client, reveal.serial, &text.to_string());
    }
}

#[cfg(test)]
mod tests;
