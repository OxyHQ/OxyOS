//! The engine as installed, where the session runs it, and how it is started: a private D-Bus
//! daemon on a socket in a directory only the person can enter, then fcitx5 on it with only the
//! addons OxyOS needs and the session's own configuration.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// OxyOS's input methods (ADR 0041), as the preferences name them, with fcitx5's input method
/// for each and the addon that provides it.
pub const INPUT_METHODS: &[(&str, &str)] = &[
    ("pinyin", "pinyin"),
    ("japanese", "anthy"),
    ("korean", "hangul"),
];

/// fcitx5's input method for one of OxyOS's.
pub fn fcitx_name(id: &str) -> Option<&'static str> {
    INPUT_METHODS
        .iter()
        .find(|(ours, _)| *ours == id)
        .map(|(_, theirs)| *theirs)
}

/// The addons fcitx5 loads: its D-Bus module and frontend, the keyboard engine (the layout and
/// emoji's QuickPhrase), the engines and what they depend on. Nothing that draws, notifies or
/// serves another frontend.
const ADDONS: &str = "dbus,dbusfrontend,keyboard,quickphrase,emoji,punctuation,pinyin,anthy,hangul";

/// The engine installed under `prefix` (`/usr/lib/oxyos/input-methods`, or a toolchain build).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Engine {
    pub prefix: PathBuf,
}

impl Engine {
    /// The engine under `prefix`, if fcitx5 is there.
    pub fn at(prefix: impl Into<PathBuf>) -> Option<Self> {
        let prefix = prefix.into();
        prefix
            .join("bin/fcitx5")
            .is_file()
            .then_some(Self { prefix })
    }

    /// The input methods it has, in OxyOS's order.
    pub fn available(&self) -> Vec<&'static str> {
        INPUT_METHODS
            .iter()
            .filter(|(_, theirs)| {
                self.prefix
                    .join(format!("share/fcitx5/inputmethod/{theirs}.conf"))
                    .is_file()
            })
            .map(|(ours, _)| *ours)
            .collect()
    }
}

/// Where a running engine keeps things.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    /// The bus's socket and configuration, and fcitx5's configuration: a directory of the
    /// session's runtime directory, mode 0700, made anew at each start.
    pub runtime: PathBuf,
    /// What the engines learn (pinyin's user dictionary, Anthy's history), kept with the
    /// person's data: `$XDG_DATA_HOME/oxyos/input-methods`.
    pub data: PathBuf,
}

impl Paths {
    /// The session's: `runtime_dir/oxyos-input-methods-<pid>` and the person's data directory.
    pub fn for_session(runtime_dir: &Path, data_home: &Path) -> Self {
        Self {
            runtime: runtime_dir.join(format!("oxyos-input-methods-{}", std::process::id())),
            data: data_home.join("oxyos/input-methods"),
        }
    }

    fn socket(&self) -> PathBuf {
        self.runtime.join("bus")
    }

    fn config(&self) -> PathBuf {
        self.runtime.join("config")
    }

    /// What fcitx5 and the bus say, kept beside them rather than in the session's own output.
    fn log(&self) -> PathBuf {
        self.runtime.join("engine.log")
    }

    pub fn address(&self) -> String {
        format!("unix:path={}", self.socket().display())
    }
}

/// The bus's configuration: a session bus for this user alone, with no activation and no
/// services of the system's, listening only on its socket.
fn bus_config(socket: &Path) -> String {
    format!(
        r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="false"/>
    <allow eavesdrop="false"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#,
        socket.display()
    )
}

/// fcitx5's global configuration: it never switches input methods itself (the session does,
/// Super+Space), no trigger keys; the candidate keys of ADR 0041's decision 5; each field its own
/// state; nine candidates to a page, chosen with 1 to 9.
const GLOBAL_CONFIG: &str = "\
[Hotkey]
EnumerateWithTriggerKeys=False
EnumerateSkipFirst=False
[Hotkey/TriggerKeys]
[Hotkey/AltTriggerKeys]
[Hotkey/EnumerateForwardKeys]
[Hotkey/EnumerateBackwardKeys]
[Hotkey/EnumerateGroupForwardKeys]
[Hotkey/EnumerateGroupBackwardKeys]
[Hotkey/ActivateKeys]
[Hotkey/DeactivateKeys]
[Hotkey/TogglePreedit]
[Hotkey/PrevPage]
0=Up
1=Page_Up
[Hotkey/NextPage]
0=Down
1=Page_Down
[Hotkey/PrevCandidate]
0=Left
1=Shift+Tab
[Hotkey/NextCandidate]
0=Right
1=Tab

[Behavior]
ActiveByDefault=False
ShareInputState=No
PreeditEnabledByDefault=True
ShowInputMethodInformation=False
showInputMethodInformationWhenFocusIn=False
ShowFirstInputMethodInformation=False
DefaultPageSize=9
OverrideXkbOption=False
PreloadInputMethod=True
";

/// QuickPhrase opens with Control+Super+Space, the key the person pressed for emoji (a Mac's
/// Control-Command-Space), which only the session sends it; nothing else of its own.
const QUICKPHRASE_CONFIG: &str = "\
Choose Modifier=None
Spell=False
[TriggerKey]
0=Control+Super+space
";

/// The keyboard engine: emoji in QuickPhrase, and no hints of its own while typing.
const KEYBOARD_CONFIG: &str = "\
EnableEmoji=False
EnableQuickPhraseEmoji=True
EnableHintByDefault=False
";

/// Pinyin: nine candidates to a page, as the global, no cloud pinyin, no spelling.
const PINYIN_CONFIG: &str = "\
PageSize=9
CloudPinyinEnabled=False
SpellEnabled=False
";

/// The input methods fcitx5 has, in order: the keyboard layout's first (and the one emoji is
/// searched in, when the session's language is another), then the person's.
pub(crate) fn profile(group: &[String], current: &str, layout: &str) -> String {
    let mut text =
        format!("[Groups/0]\nName=OxyOS\nDefault Layout={layout}\nDefaultIM={current}\n\n");
    for (index, name) in group.iter().enumerate() {
        text.push_str(&format!(
            "[Groups/0/Items/{index}]\nName={name}\nLayout=\n\n"
        ));
    }
    text.push_str("[GroupOrder]\n0=OxyOS\n");
    text
}

/// The processes of a running engine, stopped when dropped.
pub(crate) struct Processes {
    bus: Child,
    fcitx: Child,
}

impl Drop for Processes {
    fn drop(&mut self) {
        for child in [&mut self.fcitx, &mut self.bus] {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn wait_for(what: &str, limit: Duration, mut ready: impl FnMut() -> bool) -> io::Result<()> {
    let start = Instant::now();
    while !ready() {
        if start.elapsed() > limit {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{what} did not start"),
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

impl Engine {
    /// Starts the bus and writes fcitx5's configuration for `group` (fcitx5's input methods,
    /// the layout's first), `current` and the layout; fcitx5 itself starts with
    /// [`Engine::start_fcitx`] once the bus answers.
    pub(crate) fn start_bus(&self, paths: &Paths) -> io::Result<Child> {
        // A directory of its own, made anew: nothing of an earlier run is read.
        if paths.runtime.exists() {
            fs::remove_dir_all(&paths.runtime)?;
        }
        fs::create_dir_all(&paths.runtime)?;
        fs::set_permissions(&paths.runtime, fs::Permissions::from_mode(0o700))?;
        let conf = paths.runtime.join("bus.conf");
        fs::write(&conf, bus_config(&paths.socket()))?;
        let log = fs::File::create(paths.log())?;
        let bus = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", conf.display()))
            .args(["--nofork", "--nopidfile"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()?;
        let socket = paths.socket();
        wait_for("the input methods' bus", Duration::from_secs(5), || {
            socket.exists()
        })?;
        Ok(bus)
    }

    pub(crate) fn write_config(
        &self,
        paths: &Paths,
        group: &[String],
        current: &str,
        layout: &str,
    ) -> io::Result<()> {
        let config = paths.config().join("fcitx5");
        fs::create_dir_all(config.join("conf"))?;
        fs::write(config.join("config"), GLOBAL_CONFIG)?;
        fs::write(config.join("profile"), profile(group, current, layout))?;
        fs::write(config.join("conf/quickphrase.conf"), QUICKPHRASE_CONFIG)?;
        fs::write(config.join("conf/keyboard.conf"), KEYBOARD_CONFIG)?;
        fs::write(config.join("conf/pinyin.conf"), PINYIN_CONFIG)?;
        fs::create_dir_all(&paths.data)?;
        Ok(())
    }

    /// fcitx5 on the bus, headless: every addon disabled but [`ADDONS`], its configuration the
    /// session's, its learning in the person's data. No display server is named to it.
    pub(crate) fn start_fcitx(&self, paths: &Paths, bus: Child) -> io::Result<Processes> {
        let fcitx = Command::new(self.prefix.join("bin/fcitx5"))
            .args(["--disable=all", &format!("--enable={ADDONS}")])
            .env("DBUS_SESSION_BUS_ADDRESS", paths.address())
            .env("XDG_CONFIG_HOME", paths.config())
            .env("XDG_DATA_HOME", &paths.data)
            .env("FCITX_CONFIG_HOME", paths.config().join("fcitx5"))
            .env("FCITX_DATA_HOME", paths.data.join("fcitx5"))
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("WAYLAND_SOCKET")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(fs::OpenOptions::new().append(true).open(paths.log())?)
            .spawn();
        let fcitx = match fcitx {
            Ok(fcitx) => fcitx,
            Err(error) => {
                let mut bus = bus;
                let _ = bus.kill();
                let _ = bus.wait();
                return Err(error);
            }
        };
        Ok(Processes { bus, fcitx })
    }
}

impl Engine {
    /// Why it stopped: the error, and the last of what fcitx5 said.
    pub(crate) fn failure(&self, paths: &Paths, error: &io::Error) -> String {
        let said = fs::read_to_string(paths.log()).unwrap_or_default();
        let last: Vec<&str> = said
            .lines()
            .filter(|line| line.starts_with('E') || line.starts_with('W'))
            .collect();
        let tail = last[last.len().saturating_sub(3)..].join(" / ");
        if tail.is_empty() {
            error.to_string()
        } else {
            format!("{error} ({tail})")
        }
    }
}

impl Processes {
    /// Whether fcitx5 is still running.
    pub(crate) fn running(&mut self) -> bool {
        matches!(self.fcitx.try_wait(), Ok(None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_lists_the_layout_first_then_the_input_methods() {
        let group = ["keyboard-es".to_owned(), "pinyin".to_owned()];
        let text = profile(&group, "pinyin", "es");
        assert!(text.starts_with("[Groups/0]\nName=OxyOS\nDefault Layout=es\nDefaultIM=pinyin\n"));
        assert!(text.contains("[Groups/0/Items/0]\nName=keyboard-es\n"));
        assert!(text.contains("[Groups/0/Items/1]\nName=pinyin\n"));
        assert!(text.ends_with("[GroupOrder]\n0=OxyOS\n"));
    }

    #[test]
    fn oxyos_names_its_input_methods_apart_from_the_engine() {
        assert_eq!(fcitx_name("japanese"), Some("anthy"));
        assert_eq!(fcitx_name("korean"), Some("hangul"));
        assert_eq!(fcitx_name("anthy"), None);
    }

    #[test]
    fn an_engine_is_where_fcitx5_is() {
        assert!(Engine::at("/nonexistent").is_none());
    }
}
