//! The compiler's sandbox as systemd runs it: the transient unit the service starts as root
//! (`compile::confined`), with every one of its properties. A unit file takes properties a
//! transient unit's D-Bus call may refuse (systemd 257 refuses `RestrictAddressFamilies=none` with
//! "Invalid argument"), so only starting the unit proves them. As root, on a machine whose init is
//! systemd: `cargo test -p oxyos-install --no-run`, then the test binary with `sudo` and
//! `--ignored --exact tests::sandbox::the_compilers_sandbox_runs_under_systemd` (CI's verify job
//! does it). With `OXYOS_TEST_HERMESC` naming a `hermesc` the unit's dynamic user can read (not under
//! `/home` or `/tmp`, which the unit hides; `/usr/lib/oxyos/runtimes/1/hermesc` installed), the
//! real compiler is run in it too.

use crate::compile::{confined, Compile, Hermesc};
use oxyos_package::HERMES_BYTECODE_MAGIC;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Inside the unit: its standard input back, its uid, and what opening a socket of each family
/// gives.
const PROBE: &str = r#"
import os, socket, sys
data = sys.stdin.buffer.read()
said = []
for family, kind in ((socket.AF_UNIX, socket.SOCK_STREAM), (socket.AF_INET, socket.SOCK_DGRAM),
                     (socket.AF_INET6, socket.SOCK_STREAM), (socket.AF_NETLINK, socket.SOCK_RAW),
                     (socket.AF_PACKET, socket.SOCK_RAW)):
    try:
        socket.socket(family, kind, 0).close()
        said.append("open")
    except PermissionError:
        said.append("EPERM")
    except OSError as error:
        said.append("errno %d" % error.errno)
sys.stdout.buffer.write(data + ("|%d|" % os.getuid()).encode() + ",".join(said).encode())
"#;

/// A file of the test's own holding `bytes`, at its start.
fn file_with(path: &Path, bytes: &[u8]) -> File {
    let mut file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file
}

#[test]
#[ignore]
fn the_compilers_sandbox_runs_under_systemd() {
    assert!(
        rustix::process::geteuid().is_root() && Path::new("/run/systemd/system").is_dir(),
        "run as root on a machine whose init is systemd"
    );
    let dir = std::env::temp_dir().join(format!("oxyos-install-sandbox-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    // Files, as the service hands them: the unit gets the descriptors, never the paths.
    let input = file_with(&dir.join("in"), b"bundle");
    let output = file_with(&dir.join("out"), b"");
    let finished = confined(Path::new("/usr/bin/python3"))
        .args(["-I", "-c", PROBE])
        .stdin(Stdio::from(input))
        .stdout(Stdio::from(output.try_clone().unwrap()))
        .stderr(Stdio::piped())
        .env_clear()
        .output()
        .unwrap();
    assert!(
        finished.status.success(),
        "the unit did not start or failed: {}",
        String::from_utf8_lossy(&finished.stderr)
    );
    let mut said = String::new();
    File::open(dir.join("out"))
        .unwrap()
        .read_to_string(&mut said)
        .unwrap();
    let parts: Vec<&str> = said.split('|').collect();
    assert_eq!(parts.len(), 3, "{said:?}");
    assert_eq!(parts[0], "bundle", "its standard input did not reach it");
    let uid: u32 = parts[1].parse().unwrap();
    assert_ne!(uid, 0, "it ran as root");
    assert_eq!(
        parts[2], "EPERM,EPERM,EPERM,EPERM,EPERM",
        "a socket family was left: unix, inet, inet6, netlink, packet"
    );

    // The real compiler in the same unit, when the tests have one it can read.
    if let Some(hermesc) = std::env::var_os("OXYOS_TEST_HERMESC").map(PathBuf::from) {
        let source = file_with(
            &dir.join("index.js"),
            b"var notes = [1, 2, 3]; print(notes.length);",
        );
        let out = file_with(&dir.join("index.hbc"), b"");
        Hermesc { confined: true }
            .compile(&hermesc, &source, &out)
            .unwrap();
        let bytecode = fs::read(dir.join("index.hbc")).unwrap();
        assert_eq!(bytecode[..8], HERMES_BYTECODE_MAGIC);
        let broken = file_with(&dir.join("broken.js"), b"var = ;");
        let out = file_with(&dir.join("broken.hbc"), b"");
        assert!(Hermesc { confined: true }
            .compile(&hermesc, &broken, &out)
            .is_err());
    } else {
        eprintln!("OXYOS_TEST_HERMESC names no hermesc: the real compiler is not run in the unit");
    }
    let _ = fs::remove_dir_all(&dir);
}
