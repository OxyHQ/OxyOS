//! Key repeat (docs/adr/0036-key-repeat.mdx): a key held is pressed again by the compositor, as
//! macOS's window server and X11's server repeat it, never by the application. After
//! [`REPEAT_DELAY`] held, and then every [`REPEAT_INTERVAL`], its press goes again through the
//! same path a press takes (the screen reader's keys, the hardware keys, the system shortcuts,
//! the input method, the focused surface), with the text its first press typed.
//!
//! What repeats: a key the keymap repeats (the program reading the devices says so with the
//! press: never a modifier, a lock, a dead key or the keymap's own), and only where its press
//! went to something that moves or types: the focused surface (through the input method when it
//! serves a field), the reader's Next and Previous, a hardware key's steps, the switcher's moves
//! and Super+\`. What acts (closing, quitting, a screenshot, mute) never repeats.
//!
//! What ends it: any other press (that key repeats instead, as on a Mac), the key's release, a
//! modifier pressed or released (the press was a chord of the modifiers then held: a repeat never
//! turns into another one), keyboard focus moving while it went to a surface, the lock, and the
//! program saying so (a dead key or the keymap's own pressed, the seat paused). A repeat that
//! would now go elsewhere than its press went ends instead of going there.
//!
//! The scene never reads a clock: the program tells it the time (`set_clock`) before the keys it
//! reads and every turn, calls [`Scene::repeat_keys`] each turn, and waits no longer than
//! [`Scene::next_repeat`].

use super::{modifier_of, HardwareKey, Scene, SurfaceKey};
use std::time::Duration;

/// How long a key is held before it repeats: GNOME's and Windows' default (500 ms).
pub const REPEAT_DELAY: Duration = Duration::from_millis(500);
/// Between repeats: 30 a second, GNOME's and Windows' default rate.
pub const REPEAT_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 30);

/// Where a repeating key's press went, which each repeat must go to again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    Reader,
    Hardware,
    System,
    /// The surface with keyboard focus, through the input method when it serves it.
    Surface(SurfaceKey),
}

#[derive(Debug)]
struct Held {
    key: u32,
    /// What its first press typed, which each repeat types again.
    text: Option<String>,
    route: Route,
    /// When it is next pressed again, on the scene's clock.
    next: Duration,
}

/// The key repeating, if any.
#[derive(Debug, Default)]
pub(super) struct KeyRepeat {
    held: Option<Held>,
}

impl Scene {
    /// Where a press of `key` would go now, if it repeats there: the order `keyboard` takes keys
    /// in, without acting.
    fn repeat_route(&self, key: u32) -> Option<Route> {
        if let Some(repeats) = self.reader_repeats(key) {
            return repeats.then_some(Route::Reader);
        }
        if let Some(hardware) = HardwareKey::from_code(key) {
            return hardware.repeats().then_some(Route::Hardware);
        }
        if let Some(repeats) = self.system_repeats(key) {
            return repeats.then_some(Route::System);
        }
        // A key going to no surface repeats into nothing: it does not repeat.
        self.focus.filter(|f| self.visible(*f)).map(Route::Surface)
    }

    /// Before a key from the keyboard is handled: a press, the repeating key's release or a
    /// modifier's release ends the repeat.
    pub(super) fn end_repeat_for(&mut self, key: u32, pressed: bool) {
        let ends = pressed
            || modifier_of(key) != 0
            || self
                .key_repeat
                .held
                .as_ref()
                .is_some_and(|held| held.key == key);
        if ends {
            self.key_repeat.held = None;
        }
    }

    /// After a press the keymap repeats was handled: it repeats if it went where keys repeat.
    pub(super) fn start_repeat(&mut self, key: u32, text: Option<&str>) {
        if modifier_of(key) != 0 {
            return;
        }
        self.key_repeat.held = self.repeat_route(key).map(|route| Held {
            key,
            text: text.map(str::to_string),
            route,
            next: self.clock + REPEAT_DELAY,
        });
    }

    /// Keyboard focus moved: a key repeating into a surface stops.
    pub(super) fn focus_moved_for_repeat(&mut self) {
        if matches!(
            self.key_repeat.held,
            Some(Held {
                route: Route::Surface(_),
                ..
            })
        ) {
            self.key_repeat.held = None;
        }
    }

    /// Ends the key repeating, if one is: a key the scene never sees as a key was pressed (a dead
    /// key, the keymap's own), or the seat went to another session.
    pub fn stop_key_repeat(&mut self) {
        self.key_repeat.held = None;
    }

    /// The key repeating, if any (a Linux input code).
    pub fn repeating_key(&self) -> Option<u32> {
        self.key_repeat.held.as_ref().map(|held| held.key)
    }

    /// Presses the held key again if its time has come (the program calls it every turn, after
    /// `set_clock`): once, at most, its events carrying the time it was due. A turn late by more
    /// than an interval drops the repeats it missed rather than sending them at once.
    pub fn repeat_keys(&mut self) {
        let Some(held) = &self.key_repeat.held else {
            return;
        };
        let due = held.next;
        if self.clock < due {
            return;
        }
        let (key, route, text) = (held.key, held.route, held.text.clone());
        if self.repeat_route(key) != Some(route) {
            self.key_repeat.held = None;
            return;
        }
        let next = due + REPEAT_INTERVAL;
        if let Some(held) = &mut self.key_repeat.held {
            held.next = if next > self.clock {
                next
            } else {
                self.clock + REPEAT_INTERVAL
            };
        }
        self.set_input_time(u64::try_from(due.as_nanos()).unwrap_or(u64::MAX));
        self.route_keyboard(key, true, text.as_deref());
    }

    /// When the held key is next pressed again, on the clock the program tells: its turn waits no
    /// longer. `None` while no key repeats.
    pub fn next_repeat(&self) -> Option<Duration> {
        self.key_repeat.held.as_ref().map(|held| held.next)
    }
}
