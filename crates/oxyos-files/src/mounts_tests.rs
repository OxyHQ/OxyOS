//! The mount table read once and again only when it changed (`mounts.rs`).

use crate::places::Volume;
use crate::tests::{scope_in, text, Place};
use crate::{Access, Scope};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// A file call takes paths; each asked which volumes are mounted, and each read and parsed the
/// mount table again (2 % of the host's work in Stash). Now the table is read once while it does
/// not change.
#[test]
fn file_calls_read_the_mount_table_once_while_it_does_not_change() {
    let place = Place::new("mounts-once");
    let scope = scope_in(&place);
    let notes = place.file("home/ada/notes.txt", "notes");
    let stick = std::fs::canonicalize(place.path("media/ada/USB Stick")).expect("stick");
    for _ in 0..20 {
        scope
            .reach(text(&notes), Access::Read)
            .expect("in the home");
        scope
            .reach(text(&stick.join("photo.png")), Access::Write)
            .expect("in the volume");
        scope.entry(text(&notes), Access::Write).expect("an item");
        scope
            .folder(text(&stick), Access::Read)
            .expect("the volume");
        assert_eq!(scope.volumes().len(), 1);
    }
    assert_eq!(scope.mount_table_reads(), 1, "the table was read again");
    // A scope made from it (the application's) keeps the same table.
    let documents = place.path("data/oxyos/apps/so.oxy.probe");
    let cache = place.path("cache/oxyos/apps/so.oxy.probe");
    let application = scope
        .clone()
        .for_application("so.oxy.probe", &documents, &cache)
        .expect("application");
    application
        .reach(text(&notes), Access::Read)
        .expect("in the home");
    assert_eq!(application.mount_table_reads(), 1);
}

/// A table that is a plain file (a test's) is read again when its file changed.
#[test]
fn a_plain_mount_table_is_read_again_when_its_file_changes() {
    let place = Place::new("mounts-file");
    let scope = scope_in(&place);
    assert_eq!(scope.volumes().len(), 1);
    let media = std::fs::canonicalize(place.path("media/ada")).expect("media");
    let table = std::fs::read_to_string(scope.mountinfo()).expect("table");
    std::fs::write(
        scope.mountinfo(),
        format!(
            "{table}92 22 8:33 / {}/Card rw - vfat /dev/sdc1 rw\n",
            media.display()
        ),
    )
    .expect("table");
    let names: Vec<String> = scope.volumes().into_iter().map(|v| v.name).collect();
    assert_eq!(names, ["USB Stick", "Card"]);
    assert_eq!(scope.mount_table_reads(), 2);
    scope.volumes();
    assert_eq!(scope.mount_table_reads(), 2);
}

/// procfs's table, kept open and polled, is read once while nothing is mounted or unmounted.
#[test]
fn the_kernels_mount_table_is_read_once_while_nothing_mounts() {
    let place = Place::new("mounts-proc");
    let home = place.dir("home/ada");
    let scope = Scope::new(
        &home,
        &home.join(".local/share"),
        &home.join(".config"),
        vec![place.dir("media/ada")],
        PathBuf::from("/proc/self/mountinfo"),
        1000,
    )
    .expect("scope");
    for _ in 0..50 {
        assert!(scope.volumes().is_empty());
    }
    assert_eq!(scope.mount_table_reads(), 1);
}

/// A volume mounted while the scope is in use is a volume at once, and is gone at once when it is
/// unmounted: the kernel's own notice (`POLLPRI` on the table) and nothing else. The mounts are in
/// a mount namespace of a process of the test's own (`unshare --user --mount`), whose table the
/// scope reads (`/proc/PID/mountinfo`); where unprivileged namespaces are not allowed the test
/// says it is skipped.
#[test]
fn a_volume_mounted_or_unmounted_is_seen_at_once() {
    let place = Place::new("mounts-ns");
    let home = place.dir("home/ada");
    let media = place.dir("media/ada");
    let stick = place.dir("media/ada/Stick");
    // The child mounts and unmounts when told, and says when it has.
    let child = Command::new("unshare")
        .args(["--user", "--map-root-user", "--mount", "sh", "-c"])
        .arg(
            "echo ready; read line; mount -t tmpfs stick \"$1\" && echo mounted; \
             read line; umount \"$1\" && echo unmounted; read line",
        )
        .arg("sh")
        .arg(&stick)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        eprintln!("skipped: unshare cannot run");
        return;
    };
    let mut said = BufReader::new(child.stdout.take().expect("stdout"));
    let mut input = child.stdin.take().expect("stdin");
    let mut next = || {
        let mut line = String::new();
        said.read_line(&mut line).expect("line");
        line.trim().to_string()
    };
    if next() != "ready" {
        eprintln!("skipped: unprivileged user and mount namespaces are not allowed here");
        let _ = child.wait();
        return;
    }
    let table = PathBuf::from(format!("/proc/{}/mountinfo", child.id()));
    let scope = Scope::new(
        &home,
        &home.join(".local/share"),
        &home.join(".config"),
        vec![media.clone()],
        table,
        1000,
    )
    .expect("scope");
    let volumes = |scope: &Scope| -> Vec<PathBuf> {
        scope
            .volumes()
            .into_iter()
            .map(|Volume { path, .. }| path)
            .collect()
    };
    assert!(volumes(&scope).is_empty());
    assert!(volumes(&scope).is_empty());
    assert_eq!(scope.mount_table_reads(), 1);

    writeln!(input, "mount").expect("tell");
    if next() != "mounted" {
        eprintln!("skipped: the namespace cannot mount a tmpfs");
        drop(input);
        let _ = child.wait();
        return;
    }
    assert_eq!(volumes(&scope), [stick.clone()], "the mount was not seen");
    assert_eq!(scope.mount_table_reads(), 2);
    assert_eq!(volumes(&scope), [stick.clone()]);
    assert_eq!(scope.mount_table_reads(), 2);

    writeln!(input, "umount").expect("tell");
    assert_eq!(next(), "unmounted");
    assert!(volumes(&scope).is_empty(), "the unmount was not seen");
    assert_eq!(scope.mount_table_reads(), 3);
    writeln!(input, "end").expect("tell");
    drop(input);
    child.wait().expect("child");
}
