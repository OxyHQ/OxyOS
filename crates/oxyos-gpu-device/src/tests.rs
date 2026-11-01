use super::*;
use std::fs;
use std::os::unix::fs::symlink;

/// A fake machine: sysfs (`sys/`) and drivers' manifests (`icd.d/`) under a folder of its own.
struct Machine {
    root: PathBuf,
}

impl Machine {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "oxyos-gpu-device-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("icd.d")).unwrap();
        Self { root }
    }

    /// A GPU at PCI `slot` on `kernel_driver`, its card and render node, and its connectors.
    fn gpu(
        &self,
        slot: &str,
        kernel_driver: &str,
        card: u64,
        connectors: &[&str],
        boot: bool,
    ) -> &Self {
        let device = self.root.join("sys/devices/pci0000:00").join(slot);
        fs::create_dir_all(&device).unwrap();
        symlink(
            format!("../../../bus/pci/drivers/{kernel_driver}"),
            device.join("driver"),
        )
        .unwrap();
        if boot {
            fs::write(device.join("boot_vga"), "1\n").unwrap();
        }
        let render = 128 + card;
        for (minor, name) in [
            (card, format!("card{card}")),
            (render, format!("renderD{render}")),
        ] {
            let node = device.join("drm").join(&name);
            fs::create_dir_all(&node).unwrap();
            fs::write(node.join("dev"), format!("226:{minor}\n")).unwrap();
            symlink(&device, node.join("device")).unwrap();
            for (dir, link) in [
                ("sys/dev/char", format!("226:{minor}")),
                ("sys/class/drm", name),
            ] {
                fs::create_dir_all(self.root.join(dir)).unwrap();
                symlink(&node, self.root.join(dir).join(link)).unwrap();
            }
        }
        for connector in connectors {
            symlink(
                device.join("drm"),
                self.root
                    .join("sys/class/drm")
                    .join(format!("card{card}-{connector}")),
            )
            .unwrap();
        }
        self
    }

    /// A manifest named `file` for the library `library`.
    fn manifest(&self, file: &str, library: &str) -> &Self {
        fs::write(
            self.root.join("icd.d").join(file),
            format!(
                r#"{{"ICD": {{"api_version": "1.4.305", "library_path": "{library}"}}, "file_format_version": "1.0.0"}}"#
            ),
        )
        .unwrap();
        self
    }

    /// Debian's Mesa: every driver it ships.
    fn mesa(&self) -> &Self {
        self.manifest("gfxstream_vk_icd.json", "libvulkan_gfxstream.so")
            .manifest("intel_hasvk_icd.json", "libvulkan_intel_hasvk.so")
            .manifest("intel_icd.json", "libvulkan_intel.so")
            .manifest("lvp_icd.json", "libvulkan_lvp.so")
            .manifest("nouveau_icd.json", "libvulkan_nouveau.so")
            .manifest("radeon_icd.json", "libvulkan_radeon.so")
            .manifest("virtio_icd.json", "libvulkan_virtio.so")
    }

    fn drivers(&self, device: u64) -> Drivers {
        Drivers::find(&self.root, device, &[self.root.join("icd.d")])
    }

    fn libraries(&self, device: u64) -> Vec<String> {
        self.drivers(device)
            .manifests
            .iter()
            .map(|manifest| manifest.library.display().to_string())
            .collect()
    }

    fn cards(&self) -> Vec<String> {
        display_cards(&self.root)
            .iter()
            .map(|card| card.display().to_string())
            .collect()
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// The ThinkPad P14s Gen 3: Intel drives the panel; an NVIDIA GPU on nouveau, enumerated first,
/// has no connector.
fn intel_and_nvidia() -> Machine {
    let machine = Machine::new("intel-nvidia");
    machine
        .gpu("0000:03:00.0", "nouveau", 0, &[], false)
        .gpu(
            "0000:00:02.0",
            "i915",
            1,
            &["eDP-1", "DP-1", "HDMI-A-1"],
            true,
        )
        .mesa();
    machine
}

#[test]
fn intel_and_nvidia_loads_only_intels_drivers() {
    let machine = intel_and_nvidia();
    let intel = makedev(226, 129);
    assert_eq!(kernel_driver(&machine.root, intel).as_deref(), Some("i915"));
    assert_eq!(
        machine.libraries(intel),
        ["libvulkan_intel_hasvk.so", "libvulkan_intel.so"]
    );
    // Its card's node names the same GPU.
    assert_eq!(machine.libraries(makedev(226, 1)), machine.libraries(intel));
    let files = machine.drivers(intel).driver_files();
    let icd = machine.root.join("icd.d");
    assert_eq!(
        files,
        format!(
            "{}:{}",
            icd.join("intel_hasvk_icd.json").display(),
            icd.join("intel_icd.json").display()
        )
    );
    // Only the card driving the panel is ever opened by a session, and its GPU is the one a
    // compositor without a card of its own composes on.
    assert_eq!(machine.cards(), ["/dev/dri/card1"]);
    assert_eq!(
        card_render_node(&machine.root, Path::new("/dev/dri/card1")),
        Some(intel)
    );
    assert_eq!(display_device(&machine.root), intel);
}

#[test]
fn amd_and_nvidia_loads_only_radv() {
    let machine = Machine::new("amd-nvidia");
    machine
        .gpu("0000:01:00.0", "nouveau", 0, &[], false)
        .gpu("0000:05:00.0", "amdgpu", 1, &["eDP-1"], true)
        .mesa();
    assert_eq!(
        machine.libraries(makedev(226, 129)),
        ["libvulkan_radeon.so"]
    );
    assert_eq!(machine.cards(), ["/dev/dri/card1"]);
}

#[test]
fn intel_alone() {
    let machine = Machine::new("intel");
    machine
        .gpu("0000:00:02.0", "xe", 0, &["eDP-1"], true)
        .mesa();
    assert_eq!(
        machine.libraries(makedev(226, 128)),
        ["libvulkan_intel_hasvk.so", "libvulkan_intel.so"]
    );
    assert_eq!(machine.cards(), ["/dev/dri/card0"]);
}

#[test]
fn virtio_loads_venus_and_gfxstream() {
    let machine = Machine::new("virtio");
    machine
        .gpu("0000:00:01.0", "virtio_gpu", 0, &["Virtual-1"], false)
        .mesa();
    assert_eq!(
        machine.libraries(makedev(226, 128)),
        ["libvulkan_gfxstream.so", "libvulkan_virtio.so"]
    );
    assert_eq!(machine.cards(), ["/dev/dri/card0"]);
}

#[test]
fn no_gpu_is_the_cpus_vulkan_alone() {
    let machine = intel_and_nvidia();
    assert_eq!(machine.libraries(0), ["libvulkan_lvp.so"]);
    assert_eq!(machine.drivers(0).kernel_driver, None);
}

#[test]
fn an_unknown_gpu_loads_nothing_rather_than_everything() {
    let machine = Machine::new("unknown");
    machine
        .gpu("0000:00:02.0", "vmwgfx", 0, &["Virtual-1"], true)
        .mesa();
    let drivers = machine.drivers(makedev(226, 128));
    assert_eq!(drivers.kernel_driver.as_deref(), Some("vmwgfx"));
    assert!(drivers.manifests.is_empty());
    assert_eq!(drivers.driver_files(), "");
    assert!(drivers.load().is_err());
    // No card drives a display: no GPU.
    assert_eq!(display_device(&Machine::new("empty").root), 0);
    // A node sysfs does not know: nothing either.
    let missing = machine.drivers(makedev(226, 200));
    assert_eq!(missing.kernel_driver, None);
    assert!(missing.manifests.is_empty());
}

#[test]
fn the_boot_display_is_tried_first() {
    let machine = Machine::new("two-displays");
    machine
        .gpu("0000:01:00.0", "amdgpu", 0, &["DP-1"], false)
        .gpu("0000:00:02.0", "i915", 1, &["eDP-1"], true);
    assert_eq!(machine.cards(), ["/dev/dri/card1", "/dev/dri/card0"]);
}

#[test]
fn manifests_follow_the_loaders_rules() {
    let machine = Machine::new("manifests");
    let other = machine.root.join("other");
    fs::create_dir_all(&other).unwrap();
    machine
        .manifest(
            "intel_icd.x86_64.json",
            "/usr/lib/x86_64-linux-gnu/libvulkan_intel.so",
        )
        .manifest("broken.json", "not json")
        .manifest("relative.json", "lib/libvulkan_radeon.so");
    fs::write(machine.root.join("icd.d/broken.json"), "{").unwrap();
    // A later folder's manifest of a library already found is left out.
    fs::write(
        other.join("intel.json"),
        r#"{"ICD": {"library_path": "libvulkan_intel.so"}}"#,
    )
    .unwrap();
    // Another architecture's is left out.
    fs::write(
        other.join("lvp.json"),
        r#"{"ICD": {"library_path": "libvulkan_lvp.so", "library_arch": "16"}}"#,
    )
    .unwrap();
    let found = manifests_in(&[machine.root.join("icd.d"), other]);
    let libraries: Vec<PathBuf> = found
        .iter()
        .map(|manifest| manifest.library.clone())
        .collect();
    assert_eq!(
        libraries,
        [
            PathBuf::from("/usr/lib/x86_64-linux-gnu/libvulkan_intel.so"),
            machine.root.join("icd.d/lib/libvulkan_radeon.so"),
        ]
    );
}

#[test]
fn the_physical_device_is_matched_by_its_node() {
    let intel = vk::PhysicalDeviceDrmPropertiesEXT {
        has_primary: vk::TRUE,
        has_render: vk::TRUE,
        primary_major: 226,
        primary_minor: 1,
        render_major: 226,
        render_minor: 129,
        ..Default::default()
    };
    let nvidia = vk::PhysicalDeviceDrmPropertiesEXT {
        primary_minor: 0,
        render_minor: 128,
        ..intel
    };
    let compositor = makedev(226, 129);
    assert!(same_device(&intel, compositor));
    assert!(same_device(&intel, makedev(226, 1)));
    assert!(!same_device(&nvidia, compositor));
    // No node is no device; a device without DRM properties is none.
    assert!(!same_device(&intel, 0));
    assert!(!same_device(
        &vk::PhysicalDeviceDrmPropertiesEXT::default(),
        compositor
    ));
}

#[test]
fn dev_t_round_trips() {
    for (major, minor) in [(226, 0), (226, 129), (4095, 255), (5000, 70000)] {
        assert_eq!(major_minor(makedev(major, minor)), (major, minor));
    }
    // glibc's: 226:129 is 0xe281.
    assert_eq!(makedev(226, 129), 0xe281);
}
