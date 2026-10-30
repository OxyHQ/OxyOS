//! The virtual terminal the compositor's display is on (#32). While the compositor shows on
//! it, it owns the terminal, as a display server does:
//! - graphics mode, so the console draws nothing over the frames;
//! - with device input, the terminal's keyboard off, so keys reach the compositor and not the
//!   console under it;
//! - terminal switches by request (`VT_PROCESS`): the kernel asks before switching away and
//!   says when the terminal is back, so the compositor lets go of the display and the devices
//!   and takes them again.
//!
//! Dropping the [`Terminal`] gives all of it back. The workspace forbids `unsafe`; this crate
//! denies it and allows it only on the terminal's ioctls, each with the contract it relies on.

use rustix::fd::OwnedFd;
use rustix::fs::{open, Mode, OFlags};
use rustix::ioctl::{ioctl, Getter, IntegerSetter, Opcode, Setter};
use signal_hook::consts::{SIGUSR1, SIGUSR2};
use signal_hook::SigId;
use std::io::{self, Read};
use std::os::raw::c_int;
use std::os::unix::net::UnixStream;

// linux/kd.h and linux/vt.h.
const KDSETMODE: Opcode = 0x4B3A;
const KD_TEXT: usize = 0;
const KD_GRAPHICS: usize = 1;
const KDGKBMODE: Opcode = 0x4B44;
const KDSKBMODE: Opcode = 0x4B45;
const K_OFF: usize = 4;
const VT_SETMODE: Opcode = 0x5602;
const VT_GETSTATE: Opcode = 0x5603;
const VT_RELDISP: Opcode = 0x5605;
const VT_ACTIVATE: Opcode = 0x5606;
const VT_AUTO: i8 = 0;
const VT_PROCESS: i8 = 1;
const VT_ACKACQ: usize = 2;

/// `struct vt_mode`.
#[repr(C)]
struct VtMode {
    mode: i8,
    waitv: i8,
    relsig: i16,
    acqsig: i16,
    frsig: i16,
}

/// `struct vt_stat`.
#[repr(C)]
struct VtStat {
    active: u16,
    _signal: u16,
    _state: u16,
}

/// Shows terminal `number` (`VT_ACTIVATE` on the console, root's): the login program shows the
/// terminal its login screen and sessions are on before it opens one there, as a login manager
/// does, so logind makes that session the seat's active one (docs/adr/0020-oxy-account-login.mdx).
pub fn show(number: u16) -> io::Result<()> {
    let console = open(
        "/dev/tty0",
        OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    // SAFETY: VT_ACTIVATE takes the terminal's number as an integer.
    #[allow(unsafe_code)]
    unsafe {
        ioctl(
            &console,
            IntegerSetter::<VT_ACTIVATE>::new_usize(usize::from(number)),
        )?
    };
    Ok(())
}

/// A process whose controlling terminal is `/dev/tty{number}` (a getty, a shell), from
/// `/proc/*/stat`: the terminal's session, which `TIOCGSID` reports only to its own members.
fn controlling_process(number: u16) -> io::Result<Option<(u32, String)>> {
    // The console's major is 4 and its minor the terminal's number (tty_nr's encoding).
    let wanted = (4 << 8) | u64::from(number);
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        // A process may exit while it is read.
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        if let Some(name) = controlled_by(&stat, wanted) {
            return Ok(Some((pid, name)));
        }
    }
    Ok(None)
}

/// The command name in a `/proc/PID/stat` line whose controlling terminal is `tty_nr`.
fn controlled_by(stat: &str, tty_nr: u64) -> Option<String> {
    // "pid (comm) state ppid pgrp session tty_nr ...": comm may hold spaces and parentheses.
    let (head, rest) = stat.rsplit_once(')')?;
    let name = head.split_once('(')?.1;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    (fields.get(4)?.parse::<u64>().ok()? == tty_nr).then(|| name.to_string())
}

/// A terminal's input queue, read without taking the terminal: whether typing reached the
/// console while a seat manager (logind) had its keyboard off.
pub struct Console {
    tty: OwnedFd,
}

impl Console {
    /// Opens `/dev/ttyN` to read and drops what was typed there before.
    pub fn open(number: u32) -> io::Result<Self> {
        let tty = open(
            format!("/dev/tty{number}"),
            OFlags::RDONLY | OFlags::NOCTTY | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        rustix::termios::tcflush(&tty, rustix::termios::QueueSelector::IFlush)?;
        Ok(Self { tty })
    }

    /// Input the console received since it was opened, in bytes of completed lines.
    pub fn input(&self) -> io::Result<u64> {
        Ok(rustix::io::ioctl_fionread(&self.tty)?)
    }
}

/// What the kernel asks of the compositor about its terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    /// Another terminal was asked for: let go of the display and the devices, then
    /// [`Terminal::release`].
    Release,
    /// The terminal is shown again: [`Terminal::acquired`], then take the display and the
    /// devices back.
    Acquire,
}

/// The terminal the compositor took, and what it was before.
pub struct Terminal {
    tty: OwnedFd,
    number: u16,
    /// The keyboard mode to give back, when the compositor turned it off.
    keyboard: Option<c_int>,
    process_mode: bool,
    graphics: bool,
    release: UnixStream,
    acquire: UnixStream,
    signals: Vec<SigId>,
}

impl Terminal {
    /// Takes the active terminal (needs root, or the seat's session): graphics mode, switches by
    /// request and, with `keyboard_off`, its keyboard off. A terminal another session has (a
    /// login prompt) is refused: it would read the keyboard too, and hang the terminal up.
    pub fn take(keyboard_off: bool) -> io::Result<Self> {
        let console = open(
            "/dev/tty0",
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        // SAFETY: VT_GETSTATE writes a `struct vt_stat`, which `VtStat` lays out.
        #[allow(unsafe_code)]
        let state = unsafe { ioctl(&console, Getter::<VT_GETSTATE, VtStat>::new())? };
        let number = state.active;
        let tty = open(
            format!("/dev/tty{number}"),
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        if let Some((pid, name)) = controlling_process(number)? {
            return Err(io::Error::new(
                io::ErrorKind::ResourceBusy,
                format!("terminal {number} is {name}'s (process {pid}); use one with no login"),
            ));
        }
        let (release, release_signal) = UnixStream::pair()?;
        let (acquire, acquire_signal) = UnixStream::pair()?;
        release.set_nonblocking(true)?;
        acquire.set_nonblocking(true)?;
        let mut terminal = Self {
            tty,
            number,
            keyboard: None,
            process_mode: false,
            graphics: false,
            release,
            acquire,
            signals: Vec::new(),
        };
        // From here on, dropping `terminal` gives back what was taken.
        terminal
            .signals
            .push(signal_hook::low_level::pipe::register(
                SIGUSR1,
                release_signal,
            )?);
        terminal
            .signals
            .push(signal_hook::low_level::pipe::register(
                SIGUSR2,
                acquire_signal,
            )?);
        let mode = VtMode {
            mode: VT_PROCESS,
            waitv: 0,
            relsig: SIGUSR1 as i16,
            acqsig: SIGUSR2 as i16,
            frsig: 0,
        };
        // SAFETY: VT_SETMODE reads a `struct vt_mode`, which `VtMode` lays out; the signals it
        // names have handlers registered above.
        #[allow(unsafe_code)]
        unsafe {
            ioctl(&terminal.tty, Setter::<VT_SETMODE, VtMode>::new(mode))?
        };
        terminal.process_mode = true;
        // SAFETY: KDSETMODE takes the mode as an integer, and KD_GRAPHICS is one.
        #[allow(unsafe_code)]
        unsafe {
            ioctl(
                &terminal.tty,
                IntegerSetter::<KDSETMODE>::new_usize(KD_GRAPHICS),
            )?
        };
        terminal.graphics = true;
        if keyboard_off {
            // SAFETY: KDGKBMODE writes the keyboard mode as an `int`.
            #[allow(unsafe_code)]
            let keyboard = unsafe { ioctl(&terminal.tty, Getter::<KDGKBMODE, c_int>::new())? };
            // SAFETY: KDSKBMODE takes the mode as an integer, and K_OFF is one.
            #[allow(unsafe_code)]
            unsafe {
                ioctl(&terminal.tty, IntegerSetter::<KDSKBMODE>::new_usize(K_OFF))?
            };
            terminal.keyboard = Some(keyboard);
            // What was typed at the console before is not counted against the compositor.
            rustix::termios::tcflush(&terminal.tty, rustix::termios::QueueSelector::IFlush)?;
        }
        Ok(terminal)
    }

    /// The terminal's number (`/dev/ttyN`).
    pub fn number(&self) -> u16 {
        self.number
    }

    /// Input the console received since the terminal was taken, in bytes of completed lines:
    /// with its keyboard off, none, however much is typed.
    pub fn console_input(&self) -> io::Result<u64> {
        Ok(rustix::io::ioctl_fionread(&self.tty)?)
    }

    /// What the kernel asked since the last call, a release before an acquire; never blocks.
    pub fn switches(&mut self) -> io::Result<Vec<Switch>> {
        let mut switches = Vec::new();
        for (stream, switch) in [
            (&mut self.release, Switch::Release),
            (&mut self.acquire, Switch::Acquire),
        ] {
            let mut bytes = [0u8; 16];
            loop {
                match stream.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(_) => {
                        if !switches.contains(&switch) {
                            switches.push(switch);
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(switches)
    }

    /// Lets the kernel switch away, once the compositor has let go of the display and devices.
    pub fn release(&self) -> io::Result<()> {
        // SAFETY: VT_RELDISP takes an integer; 1 allows the requested switch.
        #[allow(unsafe_code)]
        unsafe {
            ioctl(&self.tty, IntegerSetter::<VT_RELDISP>::new_usize(1))?
        };
        Ok(())
    }

    /// Acknowledges that the terminal is shown again.
    pub fn acquired(&self) -> io::Result<()> {
        // SAFETY: VT_RELDISP takes an integer; VT_ACKACQ acknowledges an acquire.
        #[allow(unsafe_code)]
        unsafe {
            ioctl(&self.tty, IntegerSetter::<VT_RELDISP>::new_usize(VT_ACKACQ))?
        };
        Ok(())
    }

    /// Asks for another terminal (the user's Ctrl+Alt+Fn, which the console no longer sees
    /// while its keyboard is off). The switch arrives as [`Switch::Release`].
    pub fn activate(&self, number: u16) -> io::Result<()> {
        // SAFETY: VT_ACTIVATE takes the terminal's number as an integer.
        #[allow(unsafe_code)]
        unsafe {
            ioctl(
                &self.tty,
                IntegerSetter::<VT_ACTIVATE>::new_usize(usize::from(number)),
            )?
        };
        Ok(())
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if let Some(keyboard) = self.keyboard {
            // SAFETY: KDSKBMODE takes the mode as an integer; this is the one KDGKBMODE read.
            #[allow(unsafe_code)]
            let _ = unsafe {
                ioctl(
                    &self.tty,
                    IntegerSetter::<KDSKBMODE>::new_usize(keyboard as usize),
                )
            };
        }
        if self.graphics {
            // SAFETY: KDSETMODE takes the mode as an integer, and KD_TEXT is one.
            #[allow(unsafe_code)]
            let _ = unsafe { ioctl(&self.tty, IntegerSetter::<KDSETMODE>::new_usize(KD_TEXT)) };
        }
        if self.process_mode {
            let mode = VtMode {
                mode: VT_AUTO,
                waitv: 0,
                relsig: 0,
                acqsig: 0,
                frsig: 0,
            };
            // SAFETY: VT_SETMODE reads a `struct vt_mode`, which `VtMode` lays out.
            #[allow(unsafe_code)]
            let _ = unsafe { ioctl(&self.tty, Setter::<VT_SETMODE, VtMode>::new(mode)) };
        }
        for signal in self.signals.drain(..) {
            signal_hook::low_level::unregister(signal);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::controlled_by;

    #[test]
    fn a_stat_line_names_its_controlling_terminal() {
        let getty = "1035801 (agetty) S 1 1035801 1035801 1027 1035801 4194560 5 0";
        assert_eq!(
            controlled_by(getty, (4 << 8) | 3).as_deref(),
            Some("agetty")
        );
        assert_eq!(controlled_by(getty, (4 << 8) | 8), None);
        // No controlling terminal.
        let daemon = "1300 (systemd-logind) S 1 1300 1300 0 -1 4194560";
        assert_eq!(controlled_by(daemon, (4 << 8) | 3), None);
        // A name with spaces and a parenthesis.
        let odd = "42 (a) b (c) S 1 42 42 1027 42 0";
        assert_eq!(controlled_by(odd, 1027).as_deref(), Some("a) b (c"));
    }
}
