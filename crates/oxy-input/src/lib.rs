//! Real input for the compositor (#32): devices through libinput (udev, seat0), which does
//! pointer acceleration, touchpads and wheels, and a keymap through xkbcommon, because the
//! compositor owns the keymap and sends applications text, never key codes to translate
//! (docs/native-foundation.mdx).

use input::event::keyboard::{KeyState, KeyboardEventTrait};
use input::event::pointer::{Axis, ButtonState, PointerScrollEvent};
use input::event::{KeyboardEvent, PointerEvent};
use input::{Libinput, LibinputInterface};
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::Path;
use xkbcommon::xkb;

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

/// A keymap and its state: Linux key codes in, text out.
pub struct Keyboard {
    state: xkb::State,
}

impl Keyboard {
    /// The keymap for an XKB layout (`us`, `es`, ...), with the default rules and model.
    pub fn new(layout: &str) -> io::Result<Self> {
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
        Ok(Self {
            state: xkb::State::new(&keymap),
        })
    }

    /// Updates the state with a key (Linux input code) and returns the text a press types:
    /// none for modifiers, editing keys and control characters.
    pub fn key(&mut self, code: u32, pressed: bool) -> Option<String> {
        // XKB key codes are Linux input codes plus 8.
        let keycode = xkb::Keycode::new(code + 8);
        let text = pressed.then(|| self.state.key_get_utf8(keycode));
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
    /// The keys that type `character` on this layout: a Linux key code and whether Shift is
    /// held. `None` if no key does without a dead key or compose sequence.
    pub fn key_for(&self, character: char) -> Option<(u32, bool)> {
        let keymap = self.state.get_keymap();
        let shift = keymap.mod_get_index(xkb::MOD_NAME_SHIFT);
        let mut wanted = [0u8; 4];
        let wanted = character.encode_utf8(&mut wanted);
        for shifted in [false, true] {
            let mut state = xkb::State::new(&keymap);
            if shifted {
                state.update_mask(1 << shift, 0, 0, 0, 0, 0);
            }
            for code in 1..248 {
                if state.key_get_utf8(xkb::Keycode::new(code + 8)) == *wanted {
                    return Some((code, shifted));
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
