//! The service's sandbox as systemd runs it: `oxyos-accountd.service` itself, unchanged but for
//! what it starts (a drop-in starts this test binary's [`inside_the_unit`] instead of the
//! service), sets a user up with the system's own tools, as setting an account up does (the user
//! made with its home, its verifier, made an administrator, locked), and removes it with its home.
//! A unit whose sandbox leaves `/home` read-only fails here, as it failed on a real machine
//! (`useradd` could not make the home, exit 12), and so does one that stops hiding `/root` and
//! `/run/user`, or that lets the service write outside what it needs.
//!
//! As root, on a machine whose init is systemd: `cargo test -p oxyos-accountd --no-run`, then the
//! test binary with `sudo` and `--ignored --exact unit_sandbox::the_units_sandbox_sets_a_user_up`
//! (CI's verify job does it). The test binary must be where the unit can read it (the target
//! directory under `/home` or the checkout, not under `/tmp`, which the unit's `PrivateTmp`
//! hides). It makes a user of its own (`oxy-…`, from an id no Oxy account has) on the machine
//! and removes it, whatever the unit came to.

use oxyos_account::users::{SystemUsers, Users};
use oxyos_account::{AccountId, Secret};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// The account id whose user [`inside_the_unit`] sets up; set only in the unit.
const PROBE: &str = "OXYOS_ACCOUNTD_SANDBOX_PROBE";

/// What the test put on the machine, removed when dropped: the unit, its drop-in, the directory
/// its output is written to, and the user, if the unit left it.
struct Installed {
    unit: String,
    unit_file: PathBuf,
    drop_in: PathBuf,
    run: PathBuf,
    user: String,
}

impl Drop for Installed {
    fn drop(&mut self) {
        let _ = Command::new("/usr/sbin/userdel")
            .args(["--remove", "--", &self.user])
            .output();
        let _ = fs::remove_dir_all(&self.drop_in);
        let _ = fs::remove_file(&self.unit_file);
        let _ = fs::remove_dir_all(&self.run);
        let _ = Command::new("systemctl")
            .args(["reset-failed", &self.unit])
            .output();
        let _ = Command::new("systemctl").arg("daemon-reload").output();
    }
}

#[test]
#[ignore]
fn the_units_sandbox_sets_a_user_up() {
    assert!(
        rustix::process::geteuid().is_root() && Path::new("/run/systemd/system").is_dir(),
        "run as root on a machine whose init is systemd"
    );
    let unit_text =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("oxyos-accountd.service"))
            .unwrap();
    let exe = std::env::current_exe().unwrap();
    let exe_text = exe.to_str().unwrap().to_owned();
    // The unit file's command line: no quoting, no specifier (%), no variable ($).
    assert!(
        exe_text
            .bytes()
            .all(|b| b.is_ascii_graphic() && !b"%$\"'\\;".contains(&b)),
        "the test binary's path cannot be a unit's command line: {exe_text}"
    );
    assert!(
        !exe.starts_with("/tmp") && !exe.starts_with("/var/tmp"),
        "the unit's PrivateTmp hides the test binary at {exe_text}: build it elsewhere"
    );

    // An id no Oxy account has: a legacy 24-character hex id from the process and the clock.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    let id = format!("{:08x}{nanos:016x}", std::process::id());
    let user = AccountId::parse(&id).unwrap().user_name().to_string();

    let unit = format!("oxyos-accountd-sandbox-{}.service", std::process::id());
    let installed = Installed {
        unit_file: Path::new("/run/systemd/system").join(&unit),
        drop_in: Path::new("/run/systemd/system").join(format!("{unit}.d")),
        run: Path::new("/run").join(unit.trim_end_matches(".service")),
        unit: unit.clone(),
        user: user.clone(),
    };
    fs::create_dir_all(&installed.run).unwrap();
    let out = installed.run.join("out");
    fs::write(&installed.unit_file, unit_text).unwrap();
    fs::create_dir_all(&installed.drop_in).unwrap();
    fs::write(
        installed.drop_in.join("probe.conf"),
        format!(
            "[Service]\n\
             Type=oneshot\n\
             Restart=no\n\
             ExecStart=\n\
             ExecStart={exe_text} --ignored --exact unit_sandbox::inside_the_unit --nocapture --test-threads=1\n\
             Environment={PROBE}={id}\n\
             StandardOutput=truncate:{}\n\
             StandardError=inherit\n",
            out.display()
        ),
    )
    .unwrap();
    let reloaded = Command::new("systemctl")
        .arg("daemon-reload")
        .status()
        .unwrap();
    assert!(reloaded.success(), "systemctl daemon-reload failed");

    // A oneshot unit: starting it waits for it, and fails if it does.
    let started = Command::new("systemctl")
        .args(["start", &unit])
        .output()
        .unwrap();
    let said = fs::read_to_string(&out).unwrap_or_default();
    assert!(
        started.status.success(),
        "the unit failed ({}): {}\n{said}",
        started.status,
        String::from_utf8_lossy(&started.stderr).trim()
    );
    assert!(
        said.contains("test unit_sandbox::inside_the_unit ... ok"),
        "the unit did not run the probe: {said}"
    );
    assert!(
        !Path::new("/home").join(&user).exists(),
        "the unit left {user}'s home"
    );
}

/// What fails a write that must fail: the file system read-only, or the path missing or hidden.
fn refused(path: &str) {
    let written = fs::write(path, b"");
    if written.is_ok() {
        let _ = fs::remove_file(path);
    }
    assert!(written.is_err(), "the unit may write {path}");
}

/// Whether `path` shows nothing of what it has outside the unit.
fn hidden(path: &str) {
    if let Ok(entries) = fs::read_dir(path) {
        let names: Vec<_> = entries.flatten().map(|entry| entry.file_name()).collect();
        assert!(names.is_empty(), "the unit sees {path}: {names:?}");
    }
}

/// Run by [`the_units_sandbox_sets_a_user_up`] in the unit; elsewhere it does nothing.
#[test]
#[ignore]
fn inside_the_unit() {
    let Ok(id) = std::env::var(PROBE) else {
        eprintln!("{PROBE} is not set: this runs only in the unit");
        return;
    };
    let name = AccountId::parse(&id).unwrap().user_name();
    let users = SystemUsers::new();

    // Setting the account up: the user and its home, the verifier, the administrator.
    let user = users.create(&name).expect("the user is made with its home");
    let home = fs::metadata(&user.home).expect("its home exists");
    assert!(home.is_dir());
    assert_eq!(home.uid(), user.uid, "its home is the user's");
    users
        .set_verifier(&name, &Secret::new("correct horse battery staple"))
        .expect("its verifier is set");
    users
        .make_administrator(&name)
        .expect("it is made an administrator");
    assert!(users.is_administrator(&name).unwrap());
    // Signing it out of the machine, and removing it with its home.
    users.lock(&name).expect("it is locked");
    users.remove(&name).expect("it is removed with its home");
    assert!(!user.home.exists(), "its home is left");

    // Nothing else is written, and the other homes are not seen.
    refused("/usr/oxyos-accountd-sandbox-probe");
    refused("/var/lib/oxyos-accountd-sandbox-probe");
    refused("/boot/oxyos-accountd-sandbox-probe");
    hidden("/root");
    hidden("/run/user");
}
