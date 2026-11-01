//! The one GPU a process of the OS may use (docs/adr/0031-gpu-buffers.mdx, decision 14): the
//! compositor composes on the GPU that drives its displays and names it to its clients; every
//! client draws on that GPU, and nothing of the OS opens, wakes or enumerates any other. A GPU
//! asleep is left asleep: opening another GPU's DRM node wakes it, and a driver whose GPU fails to
//! wake can hang the whole machine (an NVIDIA GPU on nouveau did, 2026-11-01).
//!
//! What is read is sysfs, which wakes no device: a device's kernel driver
//! (`/sys/dev/char/MAJOR:MINOR/device/driver`), a card's connectors and `boot_vga`.
//!
//! - [`Drivers::for_device`]: the Vulkan drivers (ICD manifests) of the GPU whose DRM node is a
//!   `dev_t`, found by its kernel driver; a CPU implementation of Vulkan (Mesa's lavapipe) alone
//!   for no GPU (`0`).
//! - [`Drivers::load`]: those drivers' libraries, opened, to create a Vulkan instance with them
//!   alone (`VK_LUNARG_direct_driver_loading`, exclusive): the loader then loads no other driver,
//!   and no other driver opens its GPU. [`Drivers::driver_files`] is the same choice for a process
//!   whose Vulkan instances are not its own to create (`VK_DRIVER_FILES`).
//! - [`same_device`]: whether a Vulkan physical device is the one whose node is a `dev_t`
//!   (`VK_EXT_physical_device_drm`), never "the first discrete one".
//! - [`display_cards`]: the DRM cards that drive displays, in the order a session tries them; a
//!   card with no connector (a render-only GPU) is never among them.

#![allow(unsafe_code)]

use ash::vk;
use std::collections::HashMap;
use std::ffi::CStr;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Where sysfs is found: the system's root (tests give a tree of their own).
pub const SYSTEM_ROOT: &str = "/";

/// The Vulkan drivers' libraries (as the manifests name them) for a GPU's kernel driver. Mesa's,
/// and NVIDIA's own for its own kernel driver; a kernel driver Vulkan has no driver for has none.
pub fn libraries_for(kernel_driver: &str) -> &'static [&'static str] {
    match kernel_driver {
        // Intel: ANV (Skylake and newer) and HASVK (Ivy Bridge to Broadwell); each claims only
        // the Intel GPUs it supports.
        "i915" | "xe" => &["libvulkan_intel.so", "libvulkan_intel_hasvk.so"],
        // AMD: RADV, on amdgpu only (the old radeon kernel driver has no Vulkan).
        "amdgpu" => &["libvulkan_radeon.so"],
        "nouveau" => &["libvulkan_nouveau.so"],
        "nvidia" => &["libGLX_nvidia.so.0"],
        // A virtual machine's GPU: Venus, or gfxstream's.
        "virtio_gpu" => &["libvulkan_virtio.so", "libvulkan_gfxstream.so"],
        "msm" => &["libvulkan_freedreno.so"],
        "panfrost" | "panthor" => &["libvulkan_panfrost.so"],
        "v3d" => &["libvulkan_broadcom.so"],
        "asahi" => &["libvulkan_asahi.so"],
        "powervr" => &["libvulkan_powervr_mesa.so"],
        _ => &[],
    }
}

/// Vulkan on the CPU (Mesa's lavapipe), for no GPU: it opens no DRM node.
pub const CPU_LIBRARIES: &[&str] = &["libvulkan_lvp.so"];

/// A `dev_t`'s major and minor numbers (glibc's encoding, the kernel's `new_encode_dev`).
pub fn major_minor(device: u64) -> (u64, u64) {
    let major = ((device >> 32) & 0xffff_f000) | ((device >> 8) & 0xfff);
    let minor = ((device >> 12) & 0xffff_ff00) | (device & 0xff);
    (major, minor)
}

/// A `dev_t` of a major and minor number.
pub fn makedev(major: u64, minor: u64) -> u64 {
    ((major & 0xffff_f000) << 32)
        | ((major & 0xfff) << 8)
        | ((minor & 0xffff_ff00) << 12)
        | (minor & 0xff)
}

/// The kernel driver of the device whose character node is `device` (`i915`, `nouveau`), from
/// sysfs under `root`; none when sysfs does not say.
pub fn kernel_driver(root: &Path, device: u64) -> Option<String> {
    let (major, minor) = major_minor(device);
    let driver = root.join(format!("sys/dev/char/{major}:{minor}/device/driver"));
    let target = std::fs::read_link(driver).ok()?;
    Some(target.file_name()?.to_str()?.to_owned())
}

/// A Vulkan driver's manifest and the library it names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub path: PathBuf,
    /// As the loader opens it: absolute, relative to the manifest's folder, or a bare name the
    /// dynamic linker looks for.
    pub library: PathBuf,
}

impl Manifest {
    /// The manifest at `path`, if it names a driver for this process's architecture.
    fn read(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        let json: serde_json::Value = serde_json::from_str(&text).ok()?;
        let icd = json.get("ICD")?;
        if let Some(arch) = icd.get("library_arch").and_then(|arch| arch.as_str()) {
            let ours = if cfg!(target_pointer_width = "64") {
                "64"
            } else {
                "32"
            };
            if arch != ours {
                return None;
            }
        }
        let named = PathBuf::from(icd.get("library_path")?.as_str()?);
        let library = if named.is_absolute() || named.components().count() == 1 {
            named
        } else {
            path.parent()?.join(named)
        };
        Some(Self {
            path: path.to_owned(),
            library,
        })
    }

    /// The library's file name.
    fn library_name(&self) -> Option<&str> {
        self.library.file_name()?.to_str()
    }
}

/// The folders the Vulkan loader finds drivers' manifests in, in its order (Linux): the user's
/// and the system's configuration, then the user's and the system's data.
pub fn manifest_dirs() -> Vec<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    let home = var("HOME").map(PathBuf::from);
    let mut bases: Vec<PathBuf> = Vec::new();
    match var("XDG_CONFIG_HOME") {
        Some(config) => bases.push(config.into()),
        None => bases.extend(home.as_ref().map(|home| home.join(".config"))),
    }
    bases.extend(std::env::split_paths(
        &var("XDG_CONFIG_DIRS").unwrap_or_else(|| "/etc/xdg".into()),
    ));
    bases.push("/etc".into());
    match var("XDG_DATA_HOME") {
        Some(data) => bases.push(data.into()),
        None => bases.extend(home.as_ref().map(|home| home.join(".local/share"))),
    }
    bases.extend(std::env::split_paths(
        &var("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into()),
    ));
    bases
        .into_iter()
        .filter(|base| base.is_absolute())
        .map(|base| base.join("vulkan/icd.d"))
        .collect()
}

/// Every driver manifest in `dirs`, each folder's in name order, the first of a library kept.
pub fn manifests_in(dirs: &[PathBuf]) -> Vec<Manifest> {
    let mut found: Vec<Manifest> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect();
        paths.sort();
        for manifest in paths.iter().filter_map(|path| Manifest::read(path)) {
            if !found
                .iter()
                .any(|known| known.library_name() == manifest.library_name())
            {
                found.push(manifest);
            }
        }
    }
    found
}

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// The Vulkan drivers of one GPU (or of none: the CPU's).
#[derive(Clone, Debug)]
pub struct Drivers {
    /// The GPU's DRM node (`0`: none).
    pub device: u64,
    /// Its kernel driver, where sysfs says.
    pub kernel_driver: Option<String>,
    /// The installed drivers for it, in the loader's order.
    pub manifests: Vec<Manifest>,
}

impl Drivers {
    /// The drivers of the GPU whose DRM node (render or primary) is `device`, among those
    /// installed where the Vulkan loader finds them; lavapipe alone for `0`.
    pub fn for_device(device: u64) -> Self {
        Self::find(Path::new(SYSTEM_ROOT), device, &manifest_dirs())
    }

    /// As [`Self::for_device`], sysfs under `root` and the manifests in `dirs`.
    pub fn find(root: &Path, device: u64, dirs: &[PathBuf]) -> Self {
        let kernel_driver = (device != 0).then(|| kernel_driver(root, device)).flatten();
        let libraries = match (device, &kernel_driver) {
            (0, _) => CPU_LIBRARIES,
            (_, Some(driver)) => libraries_for(driver),
            (_, None) => &[],
        };
        let manifests = manifests_in(dirs)
            .into_iter()
            .filter(|manifest| {
                manifest
                    .library_name()
                    .is_some_and(|name| libraries.contains(&name))
            })
            .collect();
        Self {
            device,
            kernel_driver,
            manifests,
        }
    }

    /// What they are, for a log: the GPU's kernel driver and the drivers' libraries.
    pub fn describe(&self) -> String {
        let libraries: Vec<String> = self
            .manifests
            .iter()
            .map(|manifest| manifest.library.display().to_string())
            .collect();
        let (major, minor) = major_minor(self.device);
        let device = match (self.device, &self.kernel_driver) {
            (0, _) => "no GPU".to_string(),
            (_, Some(driver)) => format!("{major}:{minor} ({driver})"),
            (_, None) => format!("{major}:{minor} (its driver unknown)"),
        };
        if libraries.is_empty() {
            format!("{device}: no Vulkan driver")
        } else {
            format!("{device}: {}", libraries.join(", "))
        }
    }

    /// The manifests' paths, `:`-separated, as `VK_DRIVER_FILES` takes them (empty for none: a
    /// caller then loads no driver at all rather than every one).
    pub fn driver_files(&self) -> String {
        let paths: Vec<&Path> = self
            .manifests
            .iter()
            .map(|manifest| manifest.path.as_path())
            .collect();
        std::env::join_paths(paths)
            .map(|joined| joined.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// The drivers' libraries, opened (each once for the process, never closed: a driver is not
    /// made to be unloaded), for an instance created with them alone. An error when none opens.
    pub fn load(&self) -> Result<Loaded, Error> {
        let mut infos = Vec::new();
        let mut failures = Vec::new();
        for manifest in &self.manifests {
            match entry_point(&manifest.library) {
                Ok(get_instance_proc_addr) => infos.push(
                    vk::DirectDriverLoadingInfoLUNARG::default()
                        .pfn_get_instance_proc_addr(Some(get_instance_proc_addr)),
                ),
                Err(error) => failures.push(format!("{}: {error}", manifest.library.display())),
            }
        }
        if infos.is_empty() {
            let why = if failures.is_empty() {
                self.describe()
            } else {
                failures.join("; ")
            };
            return Err(Error(format!("no Vulkan driver for {why}")));
        }
        Ok(Loaded { infos })
    }
}

/// A driver's `vk_icdGetInstanceProcAddr` (the loader interface every driver exports).
type GetInstanceProcAddr =
    unsafe extern "system" fn(vk::Instance, *const std::ffi::c_char) -> vk::PFN_vkVoidFunction;

/// The entry point of the driver library at `library`, opened the first time it is asked for and
/// kept for the process.
fn entry_point(library: &Path) -> Result<GetInstanceProcAddr, String> {
    static OPENED: OnceLock<Mutex<HashMap<PathBuf, Result<GetInstanceProcAddr, String>>>> =
        OnceLock::new();
    let mut opened = OPENED
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(known) = opened.get(library) {
        return known.clone();
    }
    let flags = libloading::os::unix::RTLD_NOW | libloading::os::unix::RTLD_LOCAL;
    // SAFETY: a Vulkan driver's library, named by its manifest where the Vulkan loader finds
    // them, opened as the loader opens it (its initializers run as they would under the loader);
    // it is never closed, so the entry point taken from it stays valid for the process.
    let result = unsafe { libloading::os::unix::Library::open(Some(library), flags) }
        .and_then(|library| {
            let name = b"vk_icdGetInstanceProcAddr\0";
            // SAFETY: `vk_icdGetInstanceProcAddr` has this signature in every driver (the Vulkan
            // loader-driver interface); the library is leaked below, so the pointer outlives it.
            let symbol = unsafe { library.get::<GetInstanceProcAddr>(name) };
            let symbol = symbol.map(|symbol| *symbol);
            std::mem::forget(library);
            symbol
        })
        .map_err(|error| error.to_string());
    opened.insert(library.to_owned(), result.clone());
    result
}

/// Drivers opened for an instance of their own ([`Drivers::load`]).
pub struct Loaded {
    infos: Vec<vk::DirectDriverLoadingInfoLUNARG<'static>>,
}

impl Loaded {
    /// The instance extension an instance created with them enables: without it the loader
    /// ignores the list and loads every driver installed.
    pub const EXTENSION: &'static CStr = ash::lunarg::direct_driver_loading::NAME;

    /// What `vkCreateInstance` is given (chained to its create info), with [`Self::EXTENSION`]
    /// enabled: these drivers, and no other (exclusive). A loader without the extension refuses
    /// the instance (`VK_ERROR_EXTENSION_NOT_PRESENT`) rather than load every driver.
    pub fn list(&self) -> vk::DirectDriverLoadingListLUNARG<'_> {
        vk::DirectDriverLoadingListLUNARG::default()
            .mode(vk::DirectDriverLoadingModeLUNARG::EXCLUSIVE)
            .drivers(&self.infos)
    }

    /// How many drivers were opened.
    pub fn len(&self) -> usize {
        self.infos.len()
    }

    pub fn is_empty(&self) -> bool {
        self.infos.is_empty()
    }
}

/// Whether a physical device with these DRM properties is the one whose node (render or primary)
/// is `device`; never for `0`.
pub fn same_device(drm: &vk::PhysicalDeviceDrmPropertiesEXT<'_>, device: u64) -> bool {
    let render = (drm.has_render == vk::TRUE)
        .then(|| makedev(drm.render_major as u64, drm.render_minor as u64));
    let primary = (drm.has_primary == vk::TRUE)
        .then(|| makedev(drm.primary_major as u64, drm.primary_minor as u64));
    device != 0 && (render == Some(device) || primary == Some(device))
}

/// The DRM cards that drive displays (`/dev/dri/cardN`), from sysfs under `root`: those with a
/// connector; those with one connected first (its `status`, the kernel's last answer: reading it
/// probes nothing), then the firmware's boot display (`boot_vga`), then by number. A card with no
/// connector drives no display and is never opened to find out.
pub fn display_cards(root: &Path) -> Vec<PathBuf> {
    let class = root.join("sys/class/drm");
    let Ok(entries) = std::fs::read_dir(&class) else {
        return Vec::new();
    };
    let names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    let mut cards: Vec<(bool, bool, u32, String)> = names
        .iter()
        .filter_map(|name| {
            let number: u32 = name.strip_prefix("card")?.parse().ok()?;
            let prefix = format!("{name}-");
            let connectors: Vec<&String> = names
                .iter()
                .filter(|other| other.starts_with(&prefix))
                .collect();
            if connectors.is_empty() {
                return None;
            }
            let connected = connectors.iter().any(|connector| {
                std::fs::read_to_string(class.join(connector).join("status"))
                    .is_ok_and(|status| status.trim() == "connected")
            });
            let boot = std::fs::read_to_string(class.join(name).join("device/boot_vga"))
                .is_ok_and(|value| value.trim() == "1");
            Some((!connected, !boot, number, name.clone()))
        })
        .collect();
    cards.sort();
    cards
        .into_iter()
        .map(|(_, _, _, name)| Path::new("/dev/dri").join(name))
        .collect()
}

/// The render node (its `dev_t`) of the card at `/dev/dri/cardN`, from sysfs under `root`; the
/// card's own node where it has no render node, none where sysfs does not know the card.
pub fn card_render_node(root: &Path, card: &Path) -> Option<u64> {
    let name = card.file_name()?.to_str()?;
    let class = root.join("sys/class/drm");
    let node = |dir: &Path| -> Option<u64> {
        let dev = std::fs::read_to_string(dir.join("dev")).ok()?;
        let (major, minor) = dev.trim().split_once(':')?;
        Some(makedev(major.parse().ok()?, minor.parse().ok()?))
    };
    let render = std::fs::read_dir(class.join(name).join("device/drm"))
        .ok()
        .and_then(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("renderD"))
                .find_map(|entry| node(&entry.path()))
        });
    render.or_else(|| node(&class.join(name)))
}

/// The GPU that drives the displays: the first of [`display_cards`]' render node; `0` (no GPU)
/// where no card drives one.
pub fn display_device(root: &Path) -> u64 {
    display_cards(root)
        .first()
        .and_then(|card| card_render_node(root, card))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
