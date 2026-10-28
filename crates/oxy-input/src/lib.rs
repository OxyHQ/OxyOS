//! Real input for the compositor (#32): devices through libinput (udev, seat0), which does
//! pointer acceleration, touchpads and wheels, and a keymap through xkbcommon, because the
//! compositor owns the keymap and sends applications text, never key codes to translate
//! (docs/native-foundation.mdx).

use input::event::keyboard::{KeyState, KeyboardEventTrait};
use input::event::pointer::{Axis, ButtonState, PointerScrollEvent};
use input::event::{KeyboardEvent, PointerEvent};
use input::{Libinput, LibinputInterface};
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::Path;
use xkbcommon::xkb;
use xkbcommon::xkb::compose;

/// What the user did, in the compositor's terms: positions in the output's logical pixels
/// (`bounds`), scroll in logical pixels (positive moves the content's view down), keys as Linux
/// input codes with the text they type.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Motion {
        x: f64,
        y: f64,
    },
    Button {
        code: u32,
        pressed: bool,
    },
    Scroll {
        dx: f64,
        dy: f64,
    },
    Key {
        code: u32,
        pressed: bool,
        text: Option<String>,
    },
}

/// Opens devices for libinput as the compositor (root, or the seat's session).
struct Opener;

impl LibinputInterface for Opener {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        use rustix::fs::{open, Mode, OFlags};
        let flags = OFlags::from_bits_retain(flags as u32) | OFlags::CLOEXEC | OFlags::NONBLOCK;
        open(path, flags, Mode::empty()).map_err(|error| -error.raw_os_error())
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        drop(fd);
    }
}

/// A keymap and its state: Linux key codes in, text out. With a compose table, dead keys and
/// compose sequences type the character they compose ("´" then "o" types "ó").
pub struct Keyboard {
    state: xkb::State,
    compose: Option<compose::State>,
}

/// One key of a sequence that types a character: a Linux key code, and whether Shift is held.
pub type Stroke = (u32, bool);

impl Keyboard {
    /// The keymap for an XKB layout (`us`, `es`, ...), with the default rules and model, and
    /// the compose table of the locale the environment names (`LC_ALL`, `LC_CTYPE`, `LANG`),
    /// as xkbcommon prescribes.
    pub fn new(layout: &str) -> io::Result<Self> {
        let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
            .into_iter()
            .filter_map(std::env::var_os)
            .find(|value| !value.is_empty())
            .unwrap_or_else(|| OsString::from("C"));
        Self::with_locale(layout, &locale)
    }

    /// The same with the compose table of `locale`. Without one (on Debian the tables are in
    /// `libx11-data`), dead keys type nothing.
    pub fn with_locale(layout: &str, locale: &OsStr) -> io::Result<Self> {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_names(
            &context,
            "",
            "",
            layout,
            "",
            None,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("no XKB keymap for {layout:?}"),
            )
        })?;
        let compose = compose::Table::new_from_locale(&context, locale, compose::COMPILE_NO_FLAGS)
            .ok()
            .map(|table| compose::State::new(&table, compose::STATE_NO_FLAGS));
        Ok(Self {
            state: xkb::State::new(&keymap),
            compose,
        })
    }

    /// Whether a compose table was found: dead keys and compose sequences compose.
    pub fn composes(&self) -> bool {
        self.compose.is_some()
    }

    /// Updates the state with a key (Linux input code) and returns the text a press types:
    /// none for modifiers, editing keys and control characters, and none for a key that starts
    /// or continues a compose sequence; the key that ends it types what it composes.
    pub fn key(&mut self, code: u32, pressed: bool) -> Option<String> {
        // XKB key codes are Linux input codes plus 8.
        let keycode = xkb::Keycode::new(code + 8);
        let text = if pressed {
            let keysym = self.state.key_get_one_sym(keycode);
            match self
                .compose
                .as_mut()
                .map(|compose| (compose.feed(keysym), compose))
            {
                // A key the table takes: it starts, continues, ends or cancels a sequence.
                Some((compose::FeedResult::Accepted, compose)) => match compose.status() {
                    compose::Status::Composing => None,
                    compose::Status::Composed => {
                        let text = compose.utf8();
                        compose.reset();
                        text
                    }
                    compose::Status::Cancelled => {
                        compose.reset();
                        None
                    }
                    compose::Status::Nothing => Some(self.state.key_get_utf8(keycode)),
                },
                _ => Some(self.state.key_get_utf8(keycode)),
            }
        } else {
            None
        };
        self.state.update_key(
            keycode,
            if pressed {
                xkb::KeyDirection::Down
            } else {
                xkb::KeyDirection::Up
            },
        );
        text.filter(|text| !text.is_empty() && !text.chars().any(char::is_control))
    }
}

impl Keyboard {
    /// The keys that type `character` on this layout: one key, or a dead key and the key it
    /// composes with. `None` if neither does (or there is no compose table and it needs one).
    pub fn key_for(&self, character: char) -> Option<Vec<Stroke>> {
        let keymap = self.state.get_keymap();
        let shift = keymap.mod_get_index(xkb::MOD_NAME_SHIFT);
        let mut wanted = [0u8; 4];
        let wanted = character.encode_utf8(&mut wanted);
        // Every key's symbol and text, unshifted first.
        let mut keys = Vec::new();
        for shifted in [false, true] {
            let mut state = xkb::State::new(&keymap);
            if shifted {
                state.update_mask(1 << shift, 0, 0, 0, 0, 0);
            }
            for code in 1..248 {
                let keycode = xkb::Keycode::new(code + 8);
                keys.push((
                    (code, shifted),
                    state.key_get_one_sym(keycode),
                    state.key_get_utf8(keycode),
                ));
            }
        }
        if let Some((stroke, _, _)) = keys.iter().find(|(_, _, text)| text == wanted) {
            return Some(vec![*stroke]);
        }
        // A state of its own: cloning one only references it, and a composition may be under way.
        let table = self.compose.as_ref()?.compose_table();
        let mut compose = compose::State::new(&table, compose::STATE_NO_FLAGS);
        let dead = keys
            .iter()
            .filter(|(_, keysym, _)| xkb::keysym_get_name(*keysym).starts_with("dead_"));
        for (dead_stroke, dead_keysym, _) in dead {
            for (stroke, keysym, _) in &keys {
                compose.reset();
                compose.feed(*dead_keysym);
                compose.feed(*keysym);
                if compose.status() == compose::Status::Composed
                    && compose.utf8().as_deref() == Some(wanted)
                {
                    return Some(vec![*dead_stroke, *stroke]);
                }
            }
        }
        None
    }
}

/// The seat's devices and where the pointer is.
pub struct Input {
    libinput: Libinput,
    keyboard: Keyboard,
    bounds: (f64, f64),
    pointer: (f64, f64),
}

impl Input {
    /// Opens seat0's devices (hotplugged ones too) for an output of `bounds` logical pixels; the
    /// pointer starts at its centre.
    pub fn open(bounds: (u32, u32), layout: &str) -> io::Result<Self> {
        let mut libinput = Libinput::new_with_udev(Opener);
        libinput
            .udev_assign_seat("seat0")
            .map_err(|()| io::Error::other("libinput cannot assign seat0"))?;
        let bounds = (f64::from(bounds.0), f64::from(bounds.1));
        Ok(Self {
            libinput,
            keyboard: Keyboard::new(layout)?,
            pointer: (bounds.0 / 2.0, bounds.1 / 2.0),
            bounds,
        })
    }

    /// Whether dead keys and compose sequences compose (the locale has a compose table).
    pub fn composes(&self) -> bool {
        self.keyboard.composes()
    }

    /// What has happened since the last call; never blocks.
    pub fn dispatch(&mut self) -> io::Result<Vec<Action>> {
        self.libinput.dispatch()?;
        let mut actions = Vec::new();
        while let Some(event) = self.libinput.next() {
            match event {
                input::Event::Pointer(PointerEvent::Motion(motion)) => {
                    self.move_to(self.pointer.0 + motion.dx(), self.pointer.1 + motion.dy());
                    actions.push(self.motion());
                }
                input::Event::Pointer(PointerEvent::MotionAbsolute(motion)) => {
                    self.move_to(
                        motion.absolute_x_transformed(self.bounds.0 as u32),
                        motion.absolute_y_transformed(self.bounds.1 as u32),
                    );
                    actions.push(self.motion());
                }
                input::Event::Pointer(PointerEvent::Button(button)) => {
                    actions.push(Action::Button {
                        code: button.button(),
                        pressed: button.button_state() == ButtonState::Pressed,
                    })
                }
                input::Event::Pointer(PointerEvent::ScrollWheel(scroll)) => {
                    actions.push(scroll_action(&scroll))
                }
                input::Event::Pointer(PointerEvent::ScrollFinger(scroll)) => {
                    actions.push(scroll_action(&scroll))
                }
                input::Event::Pointer(PointerEvent::ScrollContinuous(scroll)) => {
                    actions.push(scroll_action(&scroll))
                }
                input::Event::Keyboard(KeyboardEvent::Key(key)) => {
                    let pressed = key.key_state() == KeyState::Pressed;
                    let text = self.keyboard.key(key.key(), pressed);
                    actions.push(Action::Key {
                        code: key.key(),
                        pressed,
                        text,
                    });
                }
                _ => {}
            }
        }
        Ok(actions)
    }

    fn move_to(&mut self, x: f64, y: f64) {
        self.pointer = (
            x.clamp(0.0, self.bounds.0 - 1.0),
            y.clamp(0.0, self.bounds.1 - 1.0),
        );
    }

    fn motion(&self) -> Action {
        Action::Motion {
            x: self.pointer.0,
            y: self.pointer.1,
        }
    }
}

fn scroll_action(scroll: &impl PointerScrollEvent) -> Action {
    let value = |axis| {
        if scroll.has_axis(axis) {
            scroll.scroll_value(axis)
        } else {
            0.0
        }
    };
    Action::Scroll {
        dx: value(Axis::Horizontal),
        dy: value(Axis::Vertical),
    }
}

impl AsFd for Input {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.libinput.as_fd()
    }
}

#[cfg(test)]
mod tests;
