//! The volume of PipeWire's default output and input through WirePlumber's own command, `wpctl`
//! (docs/adr/0025-hardware-keys-and-control-center.mdx): WirePlumber is the session manager
//! that keeps the default devices and their volumes (`pipewire-audio` brings it), and `wpctl` is
//! how it is told to change them from outside, as GNOME's and every other PipeWire desktop's
//! keys end up doing. Volumes are WirePlumber's, on its cubic scale (what a person hears as
//! even steps), never above 100%.
//!
//! Each call is its own process with a deadline: a PipeWire that does not answer makes the call
//! fail, never the session wait.

use crate::{Control, Level, FULL_LEVEL};
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long `wpctl` may take.
pub const DEADLINE: Duration = Duration::from_secs(2);

/// What `wpctl get-volume` says: `Volume: 0.41`, `Volume: 0.41 [MUTED]`. The level (at most
/// all of it) and the mute; `None` if it says something else.
pub fn parse_volume(text: &str) -> Option<(u32, bool)> {
    let rest = text.trim().strip_prefix("Volume:")?.trim();
    let mut words = rest.split_whitespace();
    let volume: f64 = words.next()?.parse().ok()?;
    if !volume.is_finite() || volume < 0.0 {
        return None;
    }
    let muted = match words.next() {
        None => false,
        Some("[MUTED]") => true,
        Some(_) => return None,
    };
    let level = (volume.min(1.0) * f64::from(FULL_LEVEL)).round() as u32;
    Some((level, muted))
}

/// A level as `wpctl set-volume` takes it: a fraction, never above 1.
pub fn volume_text(level: u32) -> String {
    format!(
        "{:.4}",
        f64::from(level.min(FULL_LEVEL)) / f64::from(FULL_LEVEL)
    )
}

/// The default node a control is.
fn node(control: Control) -> Option<&'static str> {
    match control {
        Control::OutputVolume => Some("@DEFAULT_AUDIO_SINK@"),
        Control::InputVolume => Some("@DEFAULT_AUDIO_SOURCE@"),
        _ => None,
    }
}

/// `wpctl`, by the program to run (`wpctl` from the `PATH` in a session).
pub struct Wpctl {
    program: PathBuf,
}

impl Default for Wpctl {
    fn default() -> Self {
        Self::new(PathBuf::from("wpctl"))
    }
}

impl Wpctl {
    pub fn new(program: PathBuf) -> Self {
        Self { program }
    }

    /// Runs it with `args`: what it wrote, if it ended well within the deadline.
    fn run(&self, args: &[&str]) -> io::Result<String> {
        let mut child = Command::new(&self.program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let deadline = Instant::now() + DEADLINE;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("{} did not answer", self.program.display()),
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let mut out = String::new();
        if let Some(mut stdout) = child.stdout.take() {
            stdout.read_to_string(&mut out)?;
        }
        if !status.success() {
            return Err(io::Error::other(format!(
                "{} {}: {status}",
                self.program.display(),
                args.join(" ")
            )));
        }
        Ok(out)
    }

    /// A volume now; `None` where there is no default device (`wpctl` fails, as it does when
    /// there is none) or no PipeWire.
    pub fn level(&self, control: Control) -> io::Result<Option<Level>> {
        let Some(node) = node(control) else {
            return Ok(None);
        };
        let out = match self.run(&["get-volume", node]) {
            Ok(out) => out,
            Err(error) if error.kind() == io::ErrorKind::TimedOut => return Err(error),
            // No such device, or nothing to ask.
            Err(_) => return Ok(None),
        };
        let (value, muted) = parse_volume(&out)
            .ok_or_else(|| io::Error::other(format!("wpctl said {:?}", out.trim())))?;
        Ok(Some(Level::new(value, muted)))
    }

    /// Sets a volume and unmutes it, as a volume key does.
    pub fn set_level(&self, control: Control, level: u32) -> io::Result<()> {
        let node = node(control).ok_or_else(|| io::Error::other("not a volume"))?;
        self.run(&["set-volume", node, &volume_text(level)])?;
        self.run(&["set-mute", node, "0"]).map(drop)
    }

    pub fn set_muted(&self, control: Control, muted: bool) -> io::Result<()> {
        let node = node(control).ok_or_else(|| io::Error::other("not a volume"))?;
        self.run(&["set-mute", node, if muted { "1" } else { "0" }])
            .map(drop)
    }
}
