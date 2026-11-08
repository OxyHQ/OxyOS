//! Every rule of installing, counting boots and erasing, without root: the programs are a host of
//! the tests' own that records each command and answers as the real ones do, and the EFI system
//! partition, the image and the target are directories of the tests' own.

use crate::check::{self, Outcome, Refused};
use crate::disks::{self, Unsuitable};
use crate::entries::{self, Entries, Entry, Tries};
use crate::host::{Command, Failed, Host, Output};
use crate::install::{self, Developer, Options, Paths, Step};
use crate::layout::{self, Image, RECOVERY_TYPE};

/// The root's type on the machine the tests run on (x86-64 or AArch64), which an install there
/// writes and the layout looks for.
fn root_type() -> &'static str {
    layout::root_type()
}

/// Another architecture's root type.
fn other_root_type() -> &'static str {
    if root_type() == layout::ROOT_TYPE_X86_64 {
        layout::ROOT_TYPE_ARM64
    } else {
        layout::ROOT_TYPE_X86_64
    }
}
use crate::network;
use crate::request::{self, EraseRequest, Found, State, Verdict};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use zeroize::Zeroizing;

/// A directory of the test's own, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "oxyos-recovery-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

type Answer = Box<dyn FnMut(&Command) -> Option<Output>>;

/// Records every command; each answer is the first of `answers` that answers it, else success
/// with nothing said.
struct Recorder {
    commands: Vec<String>,
    inputs: Vec<(String, Vec<u8>)>,
    answers: Vec<Answer>,
}

impl Recorder {
    fn new() -> Self {
        Self {
            commands: Vec::new(),
            inputs: Vec::new(),
            answers: Vec::new(),
        }
    }
    fn answer(mut self, f: impl FnMut(&Command) -> Option<Output> + 'static) -> Self {
        self.answers.push(Box::new(f));
        self
    }
    fn ran(&self, prefix: &str) -> bool {
        self.commands.iter().any(|c| c.starts_with(prefix))
    }
    fn position(&self, prefix: &str) -> usize {
        self.commands
            .iter()
            .position(|c| c == prefix || c.starts_with(&format!("{prefix} ")))
            .unwrap_or_else(|| panic!("{prefix} never ran: {:#?}", self.commands))
    }
}

impl Host for Recorder {
    fn execute(&mut self, command: &Command) -> Result<Output, Failed> {
        self.commands.push(command.line());
        if let Some(input) = &command.input {
            self.inputs.push((command.line(), input.to_vec()));
        }
        for answer in &mut self.answers {
            if let Some(output) = answer(command) {
                return Ok(output);
            }
        }
        Ok(Output::ok(""))
    }
}

fn status(code: i32) -> Output {
    Output {
        status: code,
        stdout: String::new(),
        stderr: "said no".into(),
    }
}

const DISK_UUID: &str = "7c3c2a0e-1111-4c4c-9d9d-000000000001";
const ROOT_UUID: &str = "ef10455e-ac38-45f5-9276-c3d1802da7d3";
const ROOT_HASH: &str = "5c3fbb2f6e2bd4f0a8e2c2bfbbd0d6c1e5e7ab0f1f2d3c4b5a6978877665544a";
const VERSION: &str = "6.12.48+deb13-amd64";
const RECOVERY_KEY: &str =
    "cbdefghi-jklnrtuv-cbdefghi-jklnrtuv-cbdefghi-jklnrtuv-cbdefghi-jklnrtuv";

/// lsblk's answer for a machine booted from the live medium (`sr0`), with `vda` laid out by an
/// install (`encrypted`: the root is LUKS2) or empty.
fn lsblk(laid_out: bool, encrypted: bool, root_partuuid: &str) -> String {
    let children = if laid_out {
        let root_fs = if encrypted { "crypto_LUKS" } else { "ext4" };
        let root = root_type();
        format!(
            r#""children": [
              {{"name": "vda1", "path": "/dev/vda1", "type": "part", "size": 1073741824, "rm": false, "ro": false, "tran": null, "model": null, "ptuuid": "{DISK_UUID}",
               "parttype": "c12a7328-f81f-11d2-ba4b-00a0c93ec93b", "partuuid": "2fe9c64f-d3e6-4682-9ac3-a2b654efe2c3", "partlabel": "ESP", "fstype": "vfat", "uuid": "E939-3989", "mountpoints": [null]}},
              {{"name": "vda2", "path": "/dev/vda2", "type": "part", "size": 3221225472, "rm": false, "ro": false, "tran": null, "model": null, "ptuuid": "{DISK_UUID}",
               "parttype": "{RECOVERY_TYPE}", "partuuid": "5f36a183-8d45-427c-8f11-4d0e1bd183a6", "partlabel": "OxyOS Recovery", "fstype": "ext4", "uuid": "067ef982-9c1b-4a71-ad0e-590a5250c6bd", "mountpoints": [null]}},
              {{"name": "vda3", "path": "/dev/vda3", "type": "part", "size": 12883832832, "rm": false, "ro": false, "tran": null, "model": null, "ptuuid": "{DISK_UUID}",
               "parttype": "{root}", "partuuid": "{root_partuuid}", "partlabel": "OxyOS", "fstype": "{root_fs}", "uuid": "9222c73f-1d6c-4556-be7a-d109c63ee37e", "mountpoints": [null]}}
            ]"#
        )
    } else {
        r#""children": []"#.to_string()
    };
    format!(
        r#"{{"blockdevices": [
          {{"name": "loop0", "path": "/dev/loop0", "type": "loop", "size": 791674880, "rm": false, "ro": true, "tran": null, "model": null, "ptuuid": null, "parttype": null, "partuuid": null, "partlabel": null, "fstype": "squashfs", "uuid": null, "mountpoints": ["/run/live/rootfs/filesystem.squashfs"]}},
          {{"name": "sr0", "path": "/dev/sr0", "type": "rom", "size": 1226964992, "rm": true, "ro": false, "tran": "sata", "model": "QEMU DVD-ROM", "ptuuid": "626856dd", "parttype": null, "partuuid": null, "partlabel": null, "fstype": "iso9660", "uuid": null, "mountpoints": ["/run/live/medium"]}},
          {{"name": "vda", "path": "/dev/vda", "type": "disk", "size": 17179869184, "rm": "0", "ro": "0", "tran": null, "model": "QEMU HARDDISK", "ptuuid": "{DISK_UUID}", "parttype": null, "partuuid": null, "partlabel": null, "fstype": null, "uuid": null, "mountpoints": [null], {children}}}
        ]}}"#
    )
}

fn x86_layout(encrypted: bool) -> disks::Layout {
    disks::parse(&lsblk(true, encrypted, ROOT_UUID)).unwrap()[0]
        .layout_for(root_type())
        .unwrap()
}

// The layout.

#[test]
fn the_definitions_lay_out_an_esp_the_recovery_and_an_encrypted_root_that_an_erase_makes_again() {
    let image = Image::live_boot();
    let definitions = layout::definitions(&image, true);
    let names: Vec<&str> = definitions.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["00-esp.conf", "10-recovery.conf", "20-root.conf"]);
    let esp = &definitions[0].1;
    assert!(
        esp.contains("Type=esp\n")
            && esp.contains("Format=vfat\n")
            && esp.contains("SizeMaxBytes=1073741824\n")
    );
    let recovery = &definitions[1].1;
    assert!(recovery.contains(&format!("Type={RECOVERY_TYPE}\n")));
    assert!(recovery.contains("SizeMinBytes=3221225472\nSizeMaxBytes=3221225472\n"));
    assert!(recovery.contains("CopyFiles=/run/live/medium/live:/live\n"));
    assert!(
        !recovery.contains("FactoryReset"),
        "an erase keeps the recovery"
    );
    let root = &definitions[2].1;
    assert!(
        root.contains("Type=root\n")
            && root.contains("Encrypt=key-file\n")
            && root.contains("FactoryReset=yes\n")
    );
    assert!(root.contains("CopyFiles=/run/live/rootfs/filesystem.squashfs:/\n"));
    assert!(
        root.contains("CopyFiles=/run/live/rootfs/filesystem.squashfs/boot:/boot\n"),
        "repart leaves /boot out of /"
    );
    assert!(!layout::definitions(&image, false)[2].1.contains("Encrypt"));
}

#[test]
fn the_recovery_entry_boots_its_partition_read_only_verified_into_the_recovery_and_no_live_user() {
    let entry = layout::recovery_entry("5f36a183", ROOT_HASH, VERSION, None);
    assert!(entry.contains("title      OxyOS Recovery\n"));
    assert!(entry.contains("sort-key   oxyos-recovery\n"));
    assert!(entry.contains("live-media=/dev/disk/by-partuuid/5f36a183 "));
    assert!(entry.contains(&format!(
        "dm-verity-root-hash=filesystem.squashfs:{ROOT_HASH} "
    )));
    assert!(entry.contains("systemd.unit=oxyos-recovery.target"));
    assert!(entry.contains(" live-config.nocomponents ") && entry.contains(" noeject "));
    assert!(!entry.contains("username="), "live-config makes no user");
    assert_eq!(
        Entry::parse(layout::RECOVERY_ENTRY).unwrap().tries,
        None,
        "the recovery is never counted"
    );
}

#[test]
fn the_loader_hides_its_menu_lists_only_oxyos_and_does_not_edit_command_lines() {
    for line in [
        "timeout menu-hidden",
        "default oxyos-*",
        "editor no",
        "auto-entries no",
        "auto-firmware yes",
    ] {
        assert!(layout::LOADER_CONF.lines().any(|l| l == line), "{line}");
    }
}

#[test]
fn the_tables_name_the_root_and_the_esp_by_uuid_and_the_key_or_a_passphrase() {
    assert_eq!(
        layout::fstab("c5", "E9"),
        "UUID=c5 / ext4 errors=remount-ro 0 1\nUUID=E9 /efi vfat umask=0077,shortname=winnt 0 2\n"
    );
    assert_eq!(
        layout::crypttab("92", Some(Path::new(layout::INSTALL_KEY))),
        "oxyos-root UUID=92 /etc/cryptsetup-keys.d/oxyos-root.key luks,discard\n"
    );
    assert_eq!(
        layout::crypttab("92", None),
        "oxyos-root UUID=92 none luks,discard\n"
    );
    assert_eq!(layout::root_type_for("aarch64"), layout::ROOT_TYPE_ARM64);
}

// The disks.

#[test]
fn lsblk_is_read_into_whole_disks_and_their_partitions() {
    let disks = disks::parse(&lsblk(true, true, ROOT_UUID)).unwrap();
    assert_eq!(
        disks.len(),
        1,
        "loop devices and optical drives are not disks"
    );
    let disk = &disks[0];
    assert_eq!(disk.path, "/dev/vda");
    assert!(
        !disk.removable && !disk.read_only,
        "\"0\" flags read as false"
    );
    assert_eq!(disk.partitions.len(), 3);
    let layout = disk.layout_for(root_type()).unwrap();
    assert_eq!(layout.esp.path, "/dev/vda1");
    assert_eq!(layout.recovery.path, "/dev/vda2");
    assert_eq!(layout.root.path, "/dev/vda3");
    assert!(layout.root.encrypted());
    assert!(
        disk.layout_for(other_root_type()).is_none(),
        "another architecture's root is not this one's"
    );
    assert!(disk
        .describe()
        .starts_with("/dev/vda — QEMU HARDDISK, 17.2 GB"));
}

#[test]
fn an_install_never_takes_the_medium_it_booted_from_a_read_only_or_a_small_disk() {
    let mut disk = disks::parse(&lsblk(false, false, ROOT_UUID))
        .unwrap()
        .remove(0);
    assert_eq!(disks::suitable(&disk, "/run/live/medium"), Ok(()));
    disk.partitions.push(disks::Partition {
        path: "/dev/vda1".into(),
        size: 1,
        parttype: None,
        partuuid: None,
        partlabel: None,
        fstype: Some("ext4".into()),
        uuid: None,
        mountpoints: vec!["/run/live/medium".into()],
        holders: vec![],
    });
    assert_eq!(
        disks::suitable(&disk, "/run/live/medium"),
        Err(Unsuitable::BootMedium)
    );
    disk.partitions.clear();
    disk.read_only = true;
    assert_eq!(
        disks::suitable(&disk, "/run/live/medium"),
        Err(Unsuitable::ReadOnly)
    );
    disk.read_only = false;
    disk.size = 8 << 30;
    assert_eq!(
        disks::suitable(&disk, "/run/live/medium"),
        Err(Unsuitable::TooSmall { size: 8 << 30 })
    );
}

#[test]
fn a_disk_with_two_roots_has_no_layout_rather_than_a_guess() {
    let mut disk = disks::parse(&lsblk(true, true, ROOT_UUID))
        .unwrap()
        .remove(0);
    let mut second = disk.partitions[2].clone();
    second.path = "/dev/vda4".into();
    disk.partitions.push(second);
    assert!(disk.layout_for(root_type()).is_none());
}

#[test]
fn a_mount_through_an_opened_volume_is_found_on_its_partition() {
    let json = lsblk(true, true, ROOT_UUID).replace(
        r#""uuid": "9222c73f-1d6c-4556-be7a-d109c63ee37e", "mountpoints": [null]}"#,
        r#""uuid": "9222c73f-1d6c-4556-be7a-d109c63ee37e", "mountpoints": [null], "children": [
            {"name": "oxyos-root", "path": "/dev/mapper/oxyos-root", "type": "crypt", "size": 12867055616, "fstype": "ext4", "uuid": "c524", "mountpoints": ["/"]}]}"#,
    );
    let disks = disks::parse(&json).unwrap();
    let disk = disks::disk_with_mount(&disks, "/").unwrap();
    assert_eq!(disk.path, "/dev/vda");
    assert!(disk.layout_for(root_type()).unwrap().root.mounted_at("/"));
}

// Boot counting.

#[test]
fn entry_names_are_read_with_and_without_their_counter() {
    let entry = Entry::parse("oxyos-6.12.48+deb13-amd64+2-1.conf").unwrap();
    assert_eq!(entry.id, "oxyos-6.12.48+deb13-amd64");
    assert_eq!(entry.tries, Some(Tries { left: 2, done: 1 }));
    assert!(!entry.is_bad() && entry.is_system());
    let fresh = Entry::parse("oxyos-6.12.48+deb13-amd64+3.conf").unwrap();
    assert_eq!(fresh.tries, Some(Tries { left: 3, done: 0 }));
    // "+deb13-amd64" is no counter: it is part of the version, as systemd-boot reads it.
    let blessed = Entry::parse("oxyos-6.12.48+deb13-amd64.conf").unwrap();
    assert_eq!(
        (blessed.id.as_str(), blessed.tries),
        ("oxyos-6.12.48+deb13-amd64", None)
    );
    let bad = Entry::parse("oxyos-6.12.48+0-3.conf").unwrap();
    assert!(bad.is_bad());
    assert_eq!(
        bad.file_name_with(Some(Tries { left: 3, done: 0 })),
        "oxyos-6.12.48+3.conf"
    );
    assert_eq!(bad.file_name_with(None), "oxyos-6.12.48.conf");
    assert!(!Entry::parse(layout::RECOVERY_ENTRY).unwrap().is_system());
    assert!(!Entry::parse("debian-6.12.conf").unwrap().is_system());
    assert!(
        !Entry::parse("oxyos-recovery-check.conf")
            .unwrap()
            .is_system(),
        "a person's own entry"
    );
    assert!(Entry::parse("loader.conf.new").is_none());
}

fn esp_with(names: &[&str]) -> Scratch {
    let esp = Scratch::new("esp");
    fs::create_dir_all(esp.path().join("loader/entries")).unwrap();
    for name in names {
        fs::write(
            esp.path().join("loader/entries").join(name),
            "title OxyOS\n",
        )
        .unwrap();
    }
    esp
}

fn names(esp: &Scratch) -> Vec<String> {
    Entries::new(esp.path())
        .list()
        .unwrap()
        .into_iter()
        .map(|e| e.file_name)
        .collect()
}

#[test]
fn a_complete_boot_gives_the_entry_that_booted_its_tries_back() {
    // systemd-bless-boot has removed the counter; the booted entry is named with the one it had.
    let esp = esp_with(&[
        "oxyos-6.12.48.conf",
        "oxyos-6.12.40+1-2.conf",
        "oxyos-recovery.conf",
    ]);
    let entries = Entries::new(esp.path());
    assert_eq!(
        entries
            .rearm("oxyos-6.12.48+2-1.conf", entries::TRIES)
            .unwrap()
            .as_deref(),
        Some("oxyos-6.12.48+3.conf")
    );
    assert_eq!(
        names(&esp),
        [
            "oxyos-6.12.40+1-2.conf",
            "oxyos-6.12.48+3.conf",
            "oxyos-recovery.conf"
        ]
    );
    // Another kernel's count is its own; the recovery and other systems' entries are not OxyOS's.
    assert_eq!(entries.rearm("oxyos-recovery.conf", 3).unwrap(), None);
    assert_eq!(entries.rearm("debian-6.1.conf", 3).unwrap(), None);
    assert_eq!(
        entries.rearm("oxyos-6.12.48+3.conf", 3).unwrap(),
        None,
        "already armed"
    );
}

#[test]
fn three_starts_that_never_complete_leave_only_the_recovery_able_to_start() {
    // What systemd-boot does at each start, three times: one try fewer, one more done.
    let esp = esp_with(&["oxyos-6.12.48+3.conf", "oxyos-recovery.conf"]);
    let entries = Entries::new(esp.path());
    for _ in 0..3 {
        let entry = entries.system().unwrap().remove(0);
        let Tries { left, done } = entry.tries.unwrap();
        let next = entry.file_name_with(Some(Tries {
            left: left - 1,
            done: done + 1,
        }));
        fs::rename(
            entries.directory().join(&entry.file_name),
            entries.directory().join(next),
        )
        .unwrap();
    }
    assert_eq!(
        names(&esp),
        ["oxyos-6.12.48+0-3.conf", "oxyos-recovery.conf"]
    );
    assert!(!entries.system_may_start().unwrap());
    // The recovery's Restart gives OxyOS its tries back.
    assert_eq!(entries.rearm_all(3).unwrap(), 1);
    assert!(entries.system_may_start().unwrap());
    assert_eq!(names(&esp), ["oxyos-6.12.48+3.conf", "oxyos-recovery.conf"]);
}

#[test]
fn an_erase_condemns_every_kernel_and_never_the_recovery() {
    let esp = esp_with(&[
        "oxyos-6.12.48+3.conf",
        "oxyos-6.12.40.conf",
        "oxyos-recovery.conf",
    ]);
    let entries = Entries::new(esp.path());
    assert_eq!(entries.condemn_all().unwrap(), 2);
    assert_eq!(
        names(&esp),
        [
            "oxyos-6.12.40+0.conf",
            "oxyos-6.12.48+0-3.conf",
            "oxyos-recovery.conf"
        ]
    );
    assert!(!entries.system_may_start().unwrap());
    assert_eq!(entries.condemn_all().unwrap(), 0, "condemned once");
    assert_eq!(entries.remove_system().unwrap().len(), 2);
    assert_eq!(names(&esp), ["oxyos-recovery.conf"]);
}

#[test]
fn the_loaders_variables_are_read_as_utf16() {
    let efivars = Scratch::new("efivars");
    let mut bytes = vec![6, 0, 0, 0];
    for unit in "oxyos-6.12.48+2-1.conf".encode_utf16().chain([0]) {
        bytes.extend(unit.to_le_bytes());
    }
    fs::write(
        efivars
            .path()
            .join(format!("LoaderEntrySelected-{}", entries::LOADER_VARIABLES)),
        bytes,
    )
    .unwrap();
    assert_eq!(
        entries::loader_string(efivars.path(), "LoaderEntrySelected")
            .unwrap()
            .as_deref(),
        Some("oxyos-6.12.48+2-1.conf")
    );
    assert_eq!(
        entries::loader_string(efivars.path(), "LoaderBootCountPath").unwrap(),
        None
    );
}

// The erase request.

#[test]
fn an_armed_request_is_carried_out_only_on_its_disk_and_root() {
    let layout = x86_layout(true);
    let request = EraseRequest::arm(&layout, 1_700_000_000).unwrap();
    assert_eq!((request.state, request.encrypted), (State::Armed, true));
    assert_eq!(request.applies_to(&layout), Verdict::Proceed);
    let reinstalled = disks::parse(&lsblk(true, true, "aaaaaaaa-0000-0000-0000-000000000000"))
        .unwrap()[0]
        .layout_for(root_type())
        .unwrap();
    assert_eq!(request.applies_to(&reinstalled), Verdict::OtherRoot);
    let mut elsewhere = layout.clone();
    elsewhere.disk.ptuuid = Some("another".into());
    assert_eq!(request.applies_to(&elsewhere), Verdict::OtherDisk);
    // Once begun, the root may already be gone: it resumes on its disk.
    let erasing = EraseRequest {
        state: State::Erasing,
        ..request
    };
    assert_eq!(erasing.applies_to(&reinstalled), Verdict::Proceed);
    assert_eq!(erasing.applies_to(&elsewhere), Verdict::OtherDisk);
}

#[test]
fn a_request_is_written_whole_read_back_and_one_that_cannot_be_read_erases_nothing() {
    let esp = Scratch::new("esp");
    assert!(matches!(request::read(esp.path()).unwrap(), Found::None));
    let request = EraseRequest::arm(&x86_layout(false), 1).unwrap();
    request::write(esp.path(), &request).unwrap();
    assert!(!esp.path().join("oxyos-recovery/erase.json.new").exists());
    match request::read(esp.path()).unwrap() {
        Found::Request(read) => assert_eq!(read, request),
        other => panic!("{other:?}"),
    }
    fs::write(
        esp.path().join(request::PATH),
        "{\"version\": 1, \"state\": \"armed\"}",
    )
    .unwrap();
    assert!(matches!(
        request::read(esp.path()).unwrap(),
        Found::Unreadable(_)
    ));
    fs::write(
        esp.path().join(request::PATH),
        serde_json::to_string(&EraseRequest {
            version: 2,
            ..request
        })
        .unwrap(),
    )
    .unwrap();
    assert!(matches!(
        request::read(esp.path()).unwrap(),
        Found::Unreadable(_)
    ));
    request::remove(esp.path()).unwrap();
    request::remove(esp.path()).unwrap();
    assert!(matches!(request::read(esp.path()).unwrap(), Found::None));
}

// Installing.

/// The live medium's `/live/` and efivarfs, of the test's own.
struct Machine {
    scratch: Scratch,
    image: Image,
    paths: Paths,
}

impl Machine {
    fn new() -> Self {
        let scratch = Scratch::new("machine");
        let live = scratch.path().join("medium/live");
        fs::create_dir_all(&live).unwrap();
        fs::write(
            live.join("filesystem.squashfs.roothash"),
            format!("{ROOT_HASH}\n"),
        )
        .unwrap();
        fs::write(live.join(format!("vmlinuz-{VERSION}")), "kernel").unwrap();
        fs::write(live.join(format!("initrd.img-{VERSION}")), "initrd").unwrap();
        fs::create_dir_all(scratch.path().join("efivars")).unwrap();
        let image = Image {
            root: "/run/live/rootfs/filesystem.squashfs".into(),
            live: live.display().to_string(),
        };
        let paths = Paths {
            run: scratch.path().join("run"),
            efivars: scratch.path().join("efivars"),
        };
        Self {
            scratch,
            image,
            paths,
        }
    }

    fn target(&self) -> PathBuf {
        self.paths.run.join("target")
    }

    /// A host that answers as the machine's programs would for an install onto `vda`.
    fn host(&self, encrypted: bool) -> Recorder {
        let laid_out = std::rc::Rc::new(std::cell::Cell::new(false));
        let seen = laid_out.clone();
        let entries = self.target().join("efi/loader/entries");
        Recorder::new()
            .answer(move |c| {
                if c.program == "systemd-repart" {
                    seen.set(true);
                }
                (c.program == "lsblk").then(|| Output::ok(&lsblk(seen.get(), encrypted, ROOT_UUID)))
            })
            .answer(|c| {
                (c.program == "blkid").then(|| Output::ok("c52459b1-9b93-4541-b894-f4e224723e5d\n"))
            })
            .answer(|c| {
                (c.arguments.get(1).map(String::as_str) == Some("dpkg-query")).then(|| {
                    Output::ok(
                        "bash\ngrub-common\ngrub-efi-amd64-signed\nlive-boot\nsystemd-boot\n",
                    )
                })
            })
            .answer(|c| {
                (c.program == "systemd-cryptenroll")
                    .then(|| Output::ok(&format!("{RECOVERY_KEY}\n")))
            })
            .answer(move |c| {
                // kernel-install, through update-initramfs and Debian's systemd-boot hook.
                if c.arguments.get(1).map(String::as_str) == Some("update-initramfs") {
                    fs::create_dir_all(&entries).unwrap();
                    fs::write(
                        entries.join(format!("oxyos-{VERSION}+3.conf")),
                        "title OxyOS\n",
                    )
                    .unwrap();
                }
                None
            })
    }
}

#[test]
fn an_install_checks_the_image_lays_the_disk_out_checks_the_recovery_and_sets_the_system_up() {
    let machine = Machine::new();
    let mut host = machine.host(true);
    let mut steps = Vec::new();
    let options = Options {
        encrypt: true,
        developer: None,
    };
    let installed = install::install(
        &mut host,
        &machine.paths,
        &machine.image,
        "/dev/vda",
        &options,
        &mut |s| steps.push(s),
    )
    .unwrap();
    assert!(installed.recovery_key.is_none());
    assert_eq!(
        steps,
        [
            Step::CheckingImage,
            Step::Partitioning,
            Step::CheckingRecovery,
            Step::SettingUp
        ]
    );

    let live = &machine.image.live;
    let run = machine.paths.run.display().to_string();
    let target = machine.target().display().to_string();
    let order = [
        format!("veritysetup verify {live}/filesystem.squashfs {live}/filesystem.squashfs.verity {ROOT_HASH}"),
        format!("systemd-repart --dry-run=no --empty=force --definitions={run}/repart.d --key-file={run}/root.key /dev/vda"),
        format!("mount -o ro /dev/vda2 {run}/recovery"),
        format!("veritysetup verify {run}/recovery/live/filesystem.squashfs"),
        format!("umount {run}/recovery"),
        format!("cryptsetup open --key-file {run}/root.key /dev/vda3 oxyos-root"),
        format!("mount /dev/mapper/oxyos-root {target}"),
        format!("mount /dev/vda1 {target}/efi"),
        format!("chroot {target} apt-get purge --yes --quiet --allow-remove-essential live-boot"),
        format!("chroot {target} dpkg-reconfigure --frontend=noninteractive systemd-boot"),
        format!("chroot {target} update-initramfs -u -k all"),
        format!("umount {target}/efi"),
        format!("umount {target}"),
        "cryptsetup close oxyos-root".to_string(),
    ];
    let positions: Vec<usize> = order.iter().map(|c| host.position(c)).collect();
    assert!(
        positions.windows(2).all(|w| w[0] < w[1]),
        "out of order: {:#?}",
        host.commands
    );
    assert!(host
        .commands
        .iter()
        .any(|c| c.contains("SYSTEMD_REPART") || c.starts_with("systemd-repart")));
    let purge = host
        .commands
        .iter()
        .find(|c| c.contains("apt-get purge"))
        .unwrap();
    assert!(
        purge.ends_with("live-tools user-setup grub-common grub-efi-amd64-signed"),
        "{purge}"
    );
    assert_eq!(
        install::live_boot_loader("grub2-common\nsystemd-boot\n grub-efi-arm64-bin\n"),
        ["grub2-common", "grub-efi-arm64-bin"]
    );

    let target = machine.target();
    let read = |p: &str| fs::read_to_string(target.join(p)).unwrap();
    assert_eq!(read("etc/crypttab"), "oxyos-root UUID=9222c73f-1d6c-4556-be7a-d109c63ee37e /etc/cryptsetup-keys.d/oxyos-root.key luks,discard\n");
    assert!(read("etc/fstab").starts_with("UUID=c52459b1-9b93-4541-b894-f4e224723e5d / ext4"));
    assert!(read("etc/kernel/cmdline").starts_with("root=/dev/mapper/oxyos-root rw quiet splash"));
    assert_eq!(read("etc/kernel/tries"), "3\n");
    assert_eq!(read("etc/kernel/entry-token"), "oxyos\n");
    assert_eq!(
        read("etc/machine-id"),
        "",
        "the first boot makes the machine id"
    );
    assert_eq!(read("efi/loader/loader.conf"), layout::LOADER_CONF);
    let key = target.join("etc/cryptsetup-keys.d/oxyos-root.key");
    assert_eq!(
        fs::read(&key).unwrap(),
        fs::read(machine.paths.run.join("root.key")).unwrap()
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(&key).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::read(machine.paths.run.join("root.key")).unwrap().len(),
        64
    );
    let entry = read(&format!("efi/loader/entries/{}", layout::RECOVERY_ENTRY));
    assert!(entry.contains("live-media=/dev/disk/by-partuuid/5f36a183-8d45-427c-8f11-4d0e1bd183a6"));
    assert!(entry.contains(ROOT_HASH) && entry.contains(VERSION));
    assert_eq!(read("efi/oxyos-recovery/linux"), "kernel");
    assert_eq!(read("efi/oxyos-recovery/initrd.img"), "initrd");
    drop(machine.scratch);
}

#[test]
fn an_install_whose_image_does_not_match_its_root_hash_touches_no_disk() {
    let machine = Machine::new();
    let mut host = machine.host(true).answer(|_| None);
    host.answers.insert(
        0,
        Box::new(|c: &Command| (c.program == "veritysetup").then(|| status(1))),
    );
    let options = Options {
        encrypt: true,
        developer: None,
    };
    let error = install::install(
        &mut host,
        &machine.paths,
        &machine.image,
        "/dev/vda",
        &options,
        &mut |_| {},
    )
    .unwrap_err();
    assert!(
        error.what.contains("does not match its root hash"),
        "{error}"
    );
    assert!(!host.ran("systemd-repart"));
}

#[test]
fn a_failure_while_setting_up_unmounts_and_closes_everything_it_opened() {
    let machine = Machine::new();
    let mut host = machine.host(true);
    host.answers.insert(
        0,
        Box::new(|c: &Command| {
            (c.arguments.get(1).map(String::as_str) == Some("dpkg-reconfigure")).then(|| status(1))
        }),
    );
    let options = Options {
        encrypt: true,
        developer: None,
    };
    let error = install::install(
        &mut host,
        &machine.paths,
        &machine.image,
        "/dev/vda",
        &options,
        &mut |_| {},
    )
    .unwrap_err();
    assert!(error.what.contains("chroot failed"), "{error}");
    let target = machine.target().display().to_string();
    let mounts = host
        .commands
        .iter()
        .filter(|c| {
            c.starts_with("mount ") && c.ends_with(&*target)
                || c.starts_with("mount ") && c.contains(&format!("{target}/"))
        })
        .count();
    let unmounts = host
        .commands
        .iter()
        .filter(|c| c.starts_with(&format!("umount {target}")))
        .count();
    assert_eq!(mounts, unmounts, "{:#?}", host.commands);
    assert!(host.commands.last().unwrap() == "cryptsetup close oxyos-root");
}

#[test]
fn a_developer_install_unlocks_with_the_developers_password_and_a_recovery_key_not_the_install_key()
{
    let machine = Machine::new();
    let mut host = machine.host(true);
    let options = Options {
        encrypt: true,
        developer: Some(Developer {
            user: "dev".into(),
            password: Zeroizing::new("correct horse".into()),
        }),
    };
    let installed = install::install(
        &mut host,
        &machine.paths,
        &machine.image,
        "/dev/vda",
        &options,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(
        installed.recovery_key.as_deref().map(String::as_str),
        Some(RECOVERY_KEY)
    );
    let run = machine.paths.run.display().to_string();
    let added = host.position(&format!(
        "cryptsetup luksAddKey --batch-mode --key-file {run}/root.key /dev/vda3 -"
    ));
    let enrolled = host.position("systemd-cryptenroll");
    let removed = host.position(&format!(
        "cryptsetup luksRemoveKey --batch-mode /dev/vda3 {run}/root.key"
    ));
    let initramfs = host.position(&format!(
        "chroot {} update-initramfs",
        machine.target().display()
    ));
    assert!(added < enrolled && enrolled < removed && removed < initramfs);
    assert!(
        host.inputs
            .iter()
            .any(|(c, i)| c.contains("luksAddKey") && i == b"correct horse"),
        "no newline in the passphrase"
    );
    assert!(host
        .inputs
        .iter()
        .any(|(c, i)| c.ends_with("chpasswd") && i == b"dev:correct horse\n"));
    assert!(host.ran(&format!(
        "chroot {} useradd --create-home --user-group --groups sudo --shell /bin/bash dev",
        machine.target().display()
    )));
    for line in &host.commands {
        assert!(
            !line.contains("correct horse"),
            "a password on a command line: {line}"
        );
    }
    let crypttab = fs::read_to_string(machine.target().join("etc/crypttab")).unwrap();
    assert!(
        crypttab.contains(" none luks"),
        "the boot asks for the password: {crypttab}"
    );
    assert!(!machine
        .target()
        .join("etc/cryptsetup-keys.d/oxyos-root.key")
        .exists());
}

#[test]
fn an_unencrypted_install_opens_nothing_and_names_its_root_by_uuid() {
    let machine = Machine::new();
    let mut host = machine.host(false);
    let options = Options {
        encrypt: false,
        developer: None,
    };
    install::install(
        &mut host,
        &machine.paths,
        &machine.image,
        "/dev/vda",
        &options,
        &mut |_| {},
    )
    .unwrap();
    assert!(!host.ran("cryptsetup"));
    assert!(host.ran(&format!(
        "systemd-repart --dry-run=no --empty=force --definitions={}/repart.d /dev/vda",
        machine.paths.run.display()
    )));
    assert!(!machine.target().join("etc/crypttab").exists());
    let cmdline = fs::read_to_string(machine.target().join("etc/kernel/cmdline")).unwrap();
    assert!(cmdline.starts_with("root=UUID=c52459b1-9b93-4541-b894-f4e224723e5d rw"));
}

#[test]
fn an_install_refuses_a_machine_that_did_not_start_through_uefi() {
    let machine = Machine::new();
    fs::remove_dir_all(&machine.paths.efivars).unwrap();
    let mut host = machine.host(true);
    let options = Options {
        encrypt: true,
        developer: None,
    };
    assert!(install::install(
        &mut host,
        &machine.paths,
        &machine.image,
        "/dev/vda",
        &options,
        &mut |_| {}
    )
    .is_err());
    assert!(host.commands.is_empty());
}

// Erasing.

#[test]
fn an_erase_condemns_the_system_destroys_the_root_and_makes_it_again_from_the_recovery() {
    let machine = Machine::new();
    let esp = esp_with(&[&format!("oxyos-{VERSION}+3.conf"), "oxyos-recovery.conf"]);
    fs::create_dir_all(esp.path().join(format!("oxyos/{VERSION}"))).unwrap();
    let layout = x86_layout(true);
    let mut request = EraseRequest::arm(&layout, 1).unwrap();
    request::write(esp.path(), &request).unwrap();
    // The request must say `erasing` and the kernels be condemned before the root is touched.
    let esp_path = esp.path().to_path_buf();
    let checked = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = checked.clone();
    let mut host = machine.host(true).answer(move |c| {
        if c.program == "cryptsetup" && c.arguments.first().map(String::as_str) == Some("erase") {
            let Found::Request(on_disk) = request::read(&esp_path).unwrap() else {
                panic!("no request")
            };
            assert_eq!(on_disk.state, State::Erasing);
            assert!(!Entries::new(&esp_path).system_may_start().unwrap());
            seen.set(true);
        }
        None
    });
    let mut steps = Vec::new();
    install::erase(
        &mut host,
        &machine.paths,
        &machine.image,
        esp.path(),
        &layout,
        &mut request,
        &mut |s| steps.push(s),
    )
    .unwrap();
    assert!(checked.get());
    assert_eq!(
        steps,
        [
            Step::Erasing,
            Step::Recreating,
            Step::SettingUp,
            Step::Finishing
        ]
    );
    let run = machine.paths.run.display().to_string();
    let order = [
        "cryptsetup erase --batch-mode /dev/vda3".to_string(),
        "wipefs --all /dev/vda3".to_string(),
        "blkdiscard --force /dev/vda3".to_string(),
        format!("systemd-repart --dry-run=no --factory-reset=yes --definitions={run}/repart.d --key-file={run}/root.key /dev/vda"),
        format!("cryptsetup open --key-file {run}/root.key /dev/vda3 oxyos-root"),
        format!("chroot {} update-initramfs -u -k all", machine.target().display()),
    ];
    let positions: Vec<usize> = order.iter().map(|c| host.position(c)).collect();
    assert!(
        positions.windows(2).all(|w| w[0] < w[1]),
        "{:#?}",
        host.commands
    );
    assert!(
        !host.ran("veritysetup"),
        "the recovery runs from its verified image"
    );
    assert!(
        !esp.path().join(format!("oxyos/{VERSION}")).exists(),
        "the old kernels are gone"
    );
    assert!(matches!(request::read(esp.path()).unwrap(), Found::None));
    // A new install key; the recovery's entry is untouched.
    assert!(machine
        .target()
        .join("etc/cryptsetup-keys.d/oxyos-root.key")
        .exists());
    assert!(!machine
        .target()
        .join(format!("efi/loader/entries/{}", layout::RECOVERY_ENTRY))
        .exists());
}

#[test]
fn a_failed_erase_leaves_the_request_and_the_system_condemned_for_the_next_try() {
    let machine = Machine::new();
    let esp = esp_with(&[&format!("oxyos-{VERSION}+3.conf"), "oxyos-recovery.conf"]);
    let layout = x86_layout(true);
    let mut request = EraseRequest::arm(&layout, 1).unwrap();
    let mut host = machine.host(true);
    host.answers.insert(
        0,
        Box::new(|c: &Command| (c.program == "systemd-repart").then(|| status(1))),
    );
    assert!(install::erase(
        &mut host,
        &machine.paths,
        &machine.image,
        esp.path(),
        &layout,
        &mut request,
        &mut |_| {}
    )
    .is_err());
    let Found::Request(left) = request::read(esp.path()).unwrap() else {
        panic!("the request is gone")
    };
    assert_eq!(left.state, State::Erasing);
    assert!(!Entries::new(esp.path()).system_may_start().unwrap());
}

#[test]
fn a_root_never_encrypted_is_discarded_securely_where_the_disk_can_else_overwritten() {
    let layout = x86_layout(false);
    let mut host = Recorder::new().answer(|c| {
        (c.program == "blkdiscard"
            && c.arguments
                .iter()
                .any(|a| a != "--force" && a.starts_with("--")))
        .then(|| status(1))
    });
    install::destroy(&mut host, &layout.root).unwrap();
    assert_eq!(
        host.commands,
        [
            "wipefs --all /dev/vda3",
            "blkdiscard --force --secure /dev/vda3",
            "blkdiscard --force /dev/vda3",
        ]
    );
    let mut nothing = Recorder::new().answer(|c| (c.program == "blkdiscard").then(|| status(1)));
    assert!(
        install::destroy(&mut nothing, &layout.root).is_err(),
        "an erase that erased nothing fails"
    );
}

// Checking the disk.

#[test]
fn an_encrypted_root_is_checked_only_with_a_secret_that_opens_it() {
    let layout = x86_layout(true);
    let mut wrong = Recorder::new().answer(|c| (c.program == "cryptsetup").then(|| status(2)));
    assert!(matches!(
        check::check(&mut wrong, &layout, Some(b"guess")),
        Err(Refused::WrongSecret)
    ));
    assert!(!wrong.ran("e2fsck"));
    let mut none = Recorder::new();
    assert!(matches!(
        check::check(&mut none, &layout, None),
        Err(Refused::Failed(_))
    ));

    let mut host = Recorder::new().answer(|c| (c.program == "e2fsck").then(|| status(1)));
    let report = check::check(&mut host, &layout, Some(b"correct horse")).unwrap();
    assert_eq!(
        (report.esp, report.root),
        (Outcome::Clean, Outcome::Repaired)
    );
    assert_eq!(
        host.commands,
        [
            "fsck.vfat -a -w /dev/vda1",
            "cryptsetup open --key-file=- /dev/vda3 oxyos-check",
            "e2fsck -f -y /dev/mapper/oxyos-check",
            "cryptsetup close oxyos-check",
        ]
    );
    assert_eq!(host.inputs[0].1, b"correct horse");
    assert_eq!(check::outcome(4), Outcome::Damaged);
}

// The network.

#[test]
fn nmclis_terse_lines_are_read_with_their_escapes_and_each_network_listed_once() {
    assert_eq!(
        network::fields(r"Cafe\:5G:72:WPA2\\x"),
        ["Cafe:5G", "72", "WPA2\\x"]
    );
    let networks = network::parse_networks(
        "Home:40:WPA2\nCafe\\:Bar:80:\n:90:WPA2\nHome:65:WPA2 WPA3\nGuest:20:--\n",
    );
    let summary: Vec<(&str, u8, bool)> = networks
        .iter()
        .map(|n| (n.ssid.as_str(), n.signal, n.secured))
        .collect();
    assert_eq!(
        summary,
        [
            ("Cafe:Bar", 80, false),
            ("Home", 65, true),
            ("Guest", 20, false)
        ]
    );
}

#[test]
fn a_wifi_password_goes_on_standard_input_never_on_the_command_line() {
    let mut host = Recorder::new();
    network::join(&mut host, "Home", Some("hunter2")).unwrap();
    assert_eq!(host.commands, ["nmcli --ask device wifi connect Home"]);
    assert_eq!(host.inputs[0].1, b"hunter2\n");
}

// A machine converted by hand (ADR 0054, decision 9).

/// The ThinkPad's disk after its conversion: Debian's EFI system partition and root (the generic
/// Linux type), OxyOS Recovery taken from the swap, and the swap that is left.
fn converted_disk() -> disks::Disk {
    let json = format!(
        r#"{{"blockdevices": [{{"name": "nvme0n1", "path": "/dev/nvme0n1", "type": "disk", "size": 512110190592, "rm": false, "ro": false, "tran": "nvme", "model": "SAMSUNG", "ptuuid": "a35f77cd-74da-481b-bb73-8527aa1c6293", "mountpoints": [null], "children": [
          {{"name": "nvme0n1p1", "path": "/dev/nvme0n1p1", "type": "part", "size": 1023410176, "parttype": "c12a7328-f81f-11d2-ba4b-00a0c93ec93b", "partuuid": "efff7fe8-2a85-4cfb-9db1-091e38ff85de", "fstype": "vfat", "uuid": "AF47-1085", "mountpoints": [null]}},
          {{"name": "nvme0n1p2", "path": "/dev/nvme0n1p2", "type": "part", "size": 484679090176, "parttype": "0fc63daf-8483-4772-8e79-3d69d8477de4", "partuuid": "129a8dff-ec84-4f37-af8a-2a048f2563a4", "fstype": "ext4", "uuid": "3e342287-6221-4fab-a3fb-61241b477e80", "mountpoints": [null]}},
          {{"name": "nvme0n1p3", "path": "/dev/nvme0n1p3", "type": "part", "size": 3221225472, "parttype": "{RECOVERY_TYPE}", "partuuid": "5b135168-8f41-419c-bf27-3955087bcda2", "partlabel": "OxyOS Recovery", "fstype": "ext4", "uuid": "99806b1d-5215-46a4-83db-9afba809d3bb", "mountpoints": ["/run/live/medium"]}},
          {{"name": "nvme0n1p4", "path": "/dev/nvme0n1p4", "type": "part", "size": 23185063936, "parttype": "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f", "partuuid": "cc30bfc8-e8dc-43d5-aca9-e16f7813ef02", "fstype": "swap", "uuid": "ad9efa6e-0ded-4e7f-9059-071d6771876c", "mountpoints": [null]}}
        ]}}]}}"#
    );
    disks::parse(&json).unwrap().remove(0)
}

#[test]
fn a_converted_machine_is_found_by_the_root_its_recovery_names_and_never_taken_for_an_install() {
    let disk = converted_disk();
    assert!(
        disk.layout().is_none(),
        "no root of the architecture's type: not an install's layout"
    );
    let command_line = "boot=live live-media-path=/live oxyos.recovery.root=PARTUUID=129a8dff-ec84-4f37-af8a-2a048f2563a4 quiet";
    let root = disks::converted_root(command_line).unwrap();
    let layout = disk.converted_layout(root).unwrap();
    assert_eq!(layout.kind, disks::Kind::Converted);
    assert_eq!(
        (
            layout.esp.path.as_str(),
            layout.recovery.path.as_str(),
            layout.root.path.as_str()
        ),
        ("/dev/nvme0n1p1", "/dev/nvme0n1p3", "/dev/nvme0n1p2")
    );
    // The recovery's entry names nothing, or something malformed, or the ESP or the recovery itself.
    assert_eq!(disks::converted_root("boot=live quiet"), None);
    assert_eq!(
        disks::converted_root("oxyos.recovery.root=PARTUUID=../../etc/x"),
        None
    );
    assert!(disk
        .converted_layout("efff7fe8-2a85-4cfb-9db1-091e38ff85de")
        .is_none());
    assert!(disk
        .converted_layout("5b135168-8f41-419c-bf27-3955087bcda2")
        .is_none());
    assert!(disk
        .converted_layout("00000000-0000-0000-0000-000000000000")
        .is_none());
}

#[test]
fn a_converted_machine_is_never_erased_whatever_asks() {
    let machine = Machine::new();
    let esp = esp_with(&["oxyos-recovery.conf"]);
    let disk = converted_disk();
    let layout = disk
        .converted_layout("129a8dff-ec84-4f37-af8a-2a048f2563a4")
        .unwrap();
    let mut request = EraseRequest {
        version: 1,
        state: State::Armed,
        disk: "a35f77cd-74da-481b-bb73-8527aa1c6293".into(),
        root: "129a8dff-ec84-4f37-af8a-2a048f2563a4".into(),
        encrypted: false,
        requested_at: 1,
    };
    let mut host = Recorder::new();
    let error = install::erase(
        &mut host,
        &machine.paths,
        &machine.image,
        esp.path(),
        &layout,
        &mut request,
        &mut |_| {},
    )
    .unwrap_err();
    assert!(error.what.contains("converted"), "{error}");
    assert!(
        host.commands.is_empty(),
        "nothing ran: {:#?}",
        host.commands
    );
    assert_eq!(request.state, State::Armed);
}

#[test]
fn a_converted_machines_recovery_entry_names_its_root_and_is_written_from_its_checked_image() {
    let machine = Machine::new();
    // What the recovery partition holds, where the (played) mount shows it.
    let live = machine.paths.run.join("recovery/live");
    fs::create_dir_all(&live).unwrap();
    fs::write(live.join("filesystem.squashfs.roothash"), ROOT_HASH).unwrap();
    fs::write(live.join(format!("vmlinuz-{VERSION}")), "kernel").unwrap();
    fs::write(live.join(format!("initrd.img-{VERSION}")), "initrd").unwrap();
    let esp = Scratch::new("esp");
    let disk = converted_disk();
    let recovery = disk.only(RECOVERY_TYPE).unwrap();
    let mut host = Recorder::new();
    let root = "129a8dff-ec84-4f37-af8a-2a048f2563a4";
    let hash =
        install::add_recovery_entry(&mut host, &machine.paths, esp.path(), recovery, Some(root))
            .unwrap();
    assert_eq!(hash, ROOT_HASH);
    let run = machine.paths.run.display().to_string();
    assert_eq!(
        host.commands,
        [
            format!("mount -o ro /dev/nvme0n1p3 {run}/recovery"),
            format!("veritysetup verify {run}/recovery/live/filesystem.squashfs {run}/recovery/live/filesystem.squashfs.verity {ROOT_HASH}"),
            format!("umount {run}/recovery"),
        ]
    );
    let entry = fs::read_to_string(
        esp.path()
            .join("loader/entries")
            .join(layout::RECOVERY_ENTRY),
    )
    .unwrap();
    assert!(entry.contains(&format!(" oxyos.recovery.root=PARTUUID={root} ")));
    assert!(
        entry.contains("live-media=/dev/disk/by-partuuid/5b135168-8f41-419c-bf27-3955087bcda2 ")
    );
    assert_eq!(
        fs::read_to_string(esp.path().join("oxyos-recovery/linux")).unwrap(),
        "kernel"
    );
    assert!(!layout::recovery_entry("p", ROOT_HASH, VERSION, None).contains("oxyos.recovery.root"));

    // An image that does not match its root hash writes nothing, and is unmounted.
    let esp = Scratch::new("esp");
    let mut refusing = Recorder::new().answer(|c| (c.program == "veritysetup").then(|| status(1)));
    assert!(install::add_recovery_entry(
        &mut refusing,
        &machine.paths,
        esp.path(),
        recovery,
        Some(root)
    )
    .is_err());
    assert!(!esp.path().join("loader").exists());
    assert_eq!(
        refusing.commands.last().unwrap(),
        &format!("umount {run}/recovery")
    );
}
