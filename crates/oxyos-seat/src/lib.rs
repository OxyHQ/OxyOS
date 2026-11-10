//! The compositor's seat through systemd-logind (docs/adr/0006-seat.mdx): it runs as the user
//! of a session on a seat, not as root. logind gives it the session's devices and takes them
//! back when the session goes away.
//! - [`Seat::take`] takes control of the caller's session. logind then owns the session's
//!   terminal (graphics mode, keyboard off, switches by request).
//! - [`Seat::open`] opens a device (`TakeDevice`): the DRM card already DRM master, input
//!   devices revocable.
//! - [`Seat::events`] reports what logind did since: a device paused (another session took
//!   the seat; a `Pause` is acknowledged with [`Seat::paused`]) or resumed (with a new
//!   descriptor for an input device).
//! - [`Seat::switch_to`] asks for another terminal (the user's Ctrl+Alt+Fn).
//! - The lock (docs/adr/0010-lock-screen.mdx): [`Seat::events`] also reports logind's `Lock`
//!   (`loginctl lock-session`) and the machine about to sleep; [`Seat::hold_sleep`] delays
//!   sleep until [`Seat::release_sleep`], once the session is locked on screen, and
//!   [`Seat::set_locked_hint`] tells logind whether it is locked.
//! - Idle (the idle lock, ADR 0010): [`Seat::set_idle_hint`] tells logind whether the session is
//!   idle (its `IdleHint`, which its `IdleAction` follows), and [`Seat::events`] reports when
//!   the user's processes begin or stop holding idle back with logind's own inhibitor
//!   (`systemd-inhibit --what=idle`, as a video player does).
//!
//! Dropping the [`Seat`] gives control back.
//!
//! - Backlights (docs/adr/0025-hardware-keys-and-control-center.mdx): [`Brightness`] sets a
//!   display's or a keyboard's through logind's `SetBrightness` on the caller's session, which
//!   logind allows the session's own user while the session is in front, with no root and no
//!   writable sysfs.
//!
//! [`notify`] is systemd's other interface a program on a seat speaks: telling the unit that
//! started it that it is on screen (sd_notify's `READY=1`, docs/adr/0017-boot-logo.mdx), and that
//! its main loop is alive (`WATCHDOG=1`, the unit's `WatchdogSec=`). [`restarts`] is when a part
//! that must always be there is started again after it ended on its own.
//!
//! logind's part is the default feature `logind` (the system bus); a system service that only
//! speaks to its unit (accountd, installd, backupd) takes [`notify`] and [`restarts`] without it.

pub mod notify;
pub mod restarts;

#[cfg(feature = "logind")]
mod logind;
#[cfg(feature = "logind")]
pub use logind::*;
