//! The painter loads the compositor's GPU's drivers and no other (docs/adr/0031-gpu-buffers.mdx,
//! decision 14): a decoy driver, installed where the loader finds drivers and named alone by the
//! environment (`VK_DRIVER_FILES`), is never opened while the painter starts, enumerates and opens
//! its device; inotify sees any open of the decoy's library as it happens. The control proves the
//! watch: a plain instance makes the loader open the decoy.
//!
//! One test in a binary of its own: the environment is set before any thread reads it.

#![allow(unsafe_code)]

use std::mem::MaybeUninit;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

use oxyos_paint::{vk, Paint};
use rustix::fs::inotify;

/// Whether anything opened the watched decoy since the last call.
fn opened(watch: &OwnedFd) -> bool {
    let mut buffer = [MaybeUninit::<u8>::uninit(); 1024];
    let mut reader = inotify::Reader::new(watch, &mut buffer);
    let mut any = false;
    while let Ok(event) = reader.next() {
        any |= event.events().contains(inotify::ReadFlags::OPEN);
    }
    any
}

#[test]
fn a_decoy_driver_is_never_opened() {
    let decoys = std::env::temp_dir().join(format!("oxyos-paint-decoy-{}", std::process::id()));
    std::fs::create_dir_all(&decoys).unwrap();
    let library: PathBuf = decoys.join("libvulkan_decoy.so");
    std::fs::write(&library, b"not a driver: opening it is the failure").unwrap();
    let manifest = decoys.join("decoy_icd.json");
    std::fs::write(
        &manifest,
        format!(
            r#"{{"file_format_version": "1.0.0", "ICD": {{"library_path": "{}", "api_version": "1.4.305"}}}}"#,
            library.display()
        ),
    )
    .unwrap();
    std::env::set_var("VK_DRIVER_FILES", &manifest);
    std::env::set_var("VK_ICD_FILENAMES", &manifest);
    let watch =
        inotify::init(inotify::CreateFlags::NONBLOCK | inotify::CreateFlags::CLOEXEC).unwrap();
    inotify::add_watch(&watch, &library, inotify::WatchFlags::OPEN).unwrap();

    let device = oxyos_gpu_device::display_device(Path::new(oxyos_gpu_device::SYSTEM_ROOT));
    let drivers = oxyos_gpu_device::Drivers::for_device(device);
    eprintln!("the compositor's GPU: {}", drivers.describe());
    match Paint::new(device) {
        Ok(paint) => eprintln!("the painter is on {}", paint.device_name()),
        Err(error) => eprintln!("no painter (no Vulkan or none of its drivers here): {error}"),
    }
    assert!(
        !opened(&watch),
        "the painter's start opened the decoy driver"
    );

    // The control: without the painter's choice, the loader opens what the environment names.
    // SAFETY: loading the system's Vulkan loader; this test's thread alone uses it.
    let Ok(entry) = (unsafe { ash::Entry::load() }) else {
        eprintln!("no Vulkan loader: the control is skipped");
        return;
    };
    // SAFETY: the create info outlives the call; an instance made is destroyed at once.
    if let Ok(instance) = unsafe { entry.create_instance(&vk::InstanceCreateInfo::default(), None) }
    {
        // SAFETY: the instance is live and nothing was made from it.
        unsafe { instance.destroy_instance(None) };
    }
    assert!(
        opened(&watch),
        "the watch saw no open: the test proves nothing"
    );
    let _ = std::fs::remove_dir_all(&decoys);
}
