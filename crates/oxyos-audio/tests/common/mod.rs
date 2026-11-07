//! A PipeWire daemon of the test's own (`pipewire`, the pipewire-bin package), never the
//! session's: its socket in a directory of the test's, a `default` metadata and the nodes a test
//! asks for, changed with PipeWire's own tools (`pw-cli`, `pw-metadata`). No WirePlumber runs: a
//! test plays its part where it needs it (the default it applies).

#![allow(dead_code)]

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub struct Daemon {
    child: Child,
    pub dir: PathBuf,
}

pub const REMOTE: &str = "oxyos-audio-test";

/// A null sink's or source's properties as `pw-cli create-node adapter` takes them. Its
/// parameters are never cached by the daemon: a null sink's converter does not say its `Props`
/// changed (an ALSA or Bluetooth node's does), so what was set is read where it is.
pub fn null_node(name: &str, description: &str, class: &str, extra: &str) -> String {
    format!(
        "{{ factory.name = support.null-audio-sink node.name = \"{name}\" node.description = \"{description}\" media.class = {class} audio.position = [ FL FR ] object.linger = true node.cache-params = false {extra} }}"
    )
}

impl Daemon {
    /// A daemon with `nodes` (each `null_node`'s), its socket made; this process's PipeWire
    /// clients reach it (`PIPEWIRE_RUNTIME_DIR`, `PIPEWIRE_REMOTE`). One per test binary.
    pub fn start(test: &str, nodes: &[String]) -> Self {
        // A short path: a socket's must fit in 108 bytes.
        let dir = PathBuf::from(format!("/tmp/oxyos-audio-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let objects: String = nodes
            .iter()
            .map(|node| format!("    {{ factory = adapter args = {node} }}\n"))
            .collect();
        std::fs::write(
            dir.join("pipewire.conf"),
            format!(
                r#"context.properties = {{
    core.daemon = true
    core.name = {REMOTE}
    support.dbus = false
    mem.allow-mlock = false
    log.level = 1
}}
context.spa-libs = {{
    audio.convert.* = audioconvert/libspa-audioconvert
    support.*       = support/libspa-support
}}
context.modules = [
    {{ name = libpipewire-module-protocol-native }}
    {{ name = libpipewire-module-metadata }}
    {{ name = libpipewire-module-access }}
    {{ name = libpipewire-module-spa-node-factory }}
    {{ name = libpipewire-module-client-node }}
    {{ name = libpipewire-module-adapter }}
    {{ name = libpipewire-module-link-factory }}
]
context.objects = [
    {{ factory = metadata args = {{ metadata.name = default }} }}
{objects}]
"#
            ),
        )
        .unwrap();
        let child = Command::new("pipewire")
            .arg("-c")
            .arg(dir.join("pipewire.conf"))
            .env("PIPEWIRE_RUNTIME_DIR", &dir)
            .env("XDG_RUNTIME_DIR", &dir)
            .stdout(Stdio::null())
            .spawn()
            .expect("pipewire (the pipewire-bin package)");
        // Safe in the 2021 edition; one test per binary, before any thread connects.
        std::env::set_var("PIPEWIRE_RUNTIME_DIR", &dir);
        std::env::set_var("PIPEWIRE_REMOTE", REMOTE);
        let daemon = Self { child, dir };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !daemon
            .cli(&["info", "0"])
            .contains("PipeWire:Interface:Core")
        {
            assert!(Instant::now() < deadline, "the daemon did not start");
            std::thread::sleep(Duration::from_millis(20));
        }
        daemon
    }

    /// `pw-cli` on the daemon: what it printed.
    pub fn cli(&self, args: &[&str]) -> String {
        let out = Command::new("pw-cli")
            .arg("-r")
            .arg(REMOTE)
            .args(args)
            .env("PIPEWIRE_RUNTIME_DIR", &self.dir)
            .stderr(Stdio::null())
            .output()
            .expect("pw-cli (the pipewire-bin package)");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// The `default` metadata as `pw-metadata` prints it.
    pub fn metadata(&self) -> String {
        let out = Command::new("pw-metadata")
            .args(["-r", REMOTE, "-n", "default", "0"])
            .env("PIPEWIRE_RUNTIME_DIR", &self.dir)
            .stderr(Stdio::null())
            .output()
            .expect("pw-metadata (the pipewire-bin package)");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Sets a key of the `default` metadata, as WirePlumber does.
    pub fn set_metadata(&self, key: &str, value: &str) {
        let status = Command::new("pw-metadata")
            .args([
                "-r",
                REMOTE,
                "-n",
                "default",
                "0",
                key,
                value,
                "Spa:String:JSON",
            ])
            .env("PIPEWIRE_RUNTIME_DIR", &self.dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }

    /// Configures a node's ports as WirePlumber does every device's (`PortConfig`, DSP mode):
    /// before it, an adapter's converter applies no volume. `sink` for an output's input ports.
    pub fn configure(&self, name: &str, sink: bool) {
        let direction = if sink { "Input" } else { "Output" };
        let said = self.cli(&[
            "set-param",
            name,
            "PortConfig",
            &format!(
                "{{ direction: {direction}, mode: dsp, format: {{ mediaType: audio, mediaSubtype: raw, format: F32P, rate: 48000, channels: 2, position: [ FL, FR ] }} }}"
            ),
        ]);
        assert!(said.contains("PortConfig"), "{name} not configured: {said}");
    }

    /// The global id of a node by its name.
    pub fn node_id(&self, name: &str) -> Option<u32> {
        let listed = self.cli(&["ls", "Node"]);
        let mut id = None;
        for line in listed.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("id ") {
                id = rest.split(',').next().and_then(|n| n.trim().parse().ok());
            } else if line == format!("node.name = \"{name}\"") {
                return id;
            }
        }
        None
    }

    /// The daemon stopped, as PipeWire going away.
    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.stop();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
