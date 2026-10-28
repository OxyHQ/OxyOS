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
    /// A key the compose table took (a dead key, or a key of a sequence), which no
    /// application sees as a key: `composition` is the sequence so far as it is shown (empty
    /// once it ends), and `text` what it composed when it ends composing something.
    Compose {
        composition: String,
        text: Option<String>,
    },
    /// The keymap's terminal switch (Ctrl+Alt+Fn on the default keymaps), which no application
    /// sees as a key: the compositor asks for terminal `n`.
    SwitchTerminal(u16),
}

/// What a key does.
#[derive(Clone, Debug, PartialEq)]
pub enum Typed {
    /// A key for the application, with the text a press types, if any.
    Key(Option<String>),
    /// A press the compose table took (see [`Action::Compose`]).
    Compose {
        composition: String,
        text: Option<String>,
    },
    /// A press of the keymap's terminal switch (see [`Action::SwitchTerminal`]).
    SwitchTerminal(u16),
    /// The release of a key the compose table or a terminal switch took: nothing for anyone.
    Nothing,
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
    /// The sequence so far, as it is shown.
    composition: String,
    /// Keys whose press the compose table or a terminal switch took, so their release is no
    /// application's either.
    taken: Vec<u32>,
}

/// Text a key may type: not empty, and no control characters (Enter and Backspace are keys).
fn typable(text: &str) -> bool {
    !text.is_empty() && !text.chars().any(char::is_control)
}

/// A key of a compose sequence as the composition shows it: a dead key as its spacing
/// accent, as GTK's input method shows them, the compose key as a middle dot, and any other
/// key as its text.
fn shown(keysym: xkb::Keysym) -> String {
    let accent = match xkb::keysym_get_name(keysym).as_str() {
        "dead_grave" => "`",
        "dead_acute" => "´",
        "dead_circumflex" => "^",
        "dead_tilde" => "~",
        "dead_macron" => "¯",
        "dead_breve" => "˘",
        "dead_abovedot" => "˙",
        "dead_diaeresis" => "¨",
        "dead_abovering" => "˚",
        "dead_doubleacute" => "˝",
        "dead_caron" => "ˇ",
        "dead_cedilla" => "¸",
        "dead_ogonek" => "˛",
        _ => {
            let text = xkb::keysym_to_utf8(keysym);
            return if typable(&text) {
                text
            } else {
                "·".to_string()
            };
        }
    };
    accent.to_string()
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

    /// The same with the compose table of `locale`, from xkbcommon's directory (`XLOCALEDIR`,
    /// `/usr/share/X11/locale` by default; the image ships the UTF-8 tables there). Without one,
    /// dead keys type nothing.
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
            composition: String::new(),
            taken: Vec::new(),
        })
    }

    /// Whether a compose table was found: dead keys and compose sequences compose.
    pub fn composes(&self) -> bool {
        self.compose.is_some()
    }

    /// Updates the state with a key (Linux input code) and says what it does. A press types
    /// its text: none for modifiers, editing keys and control characters. With a compose
    /// table, a key that starts or continues a sequence types nothing, the key that ends it
    /// types what it composes, and a sequence that composes nothing types nothing; those keys
    /// are the table's, not the application's.
    pub fn key(&mut self, code: u32, pressed: bool) -> Typed {
        // XKB key codes are Linux input codes plus 8.
        let keycode = xkb::Keycode::new(code + 8);
        let typed = if !pressed {
            match self.taken.iter().position(|taken| *taken == code) {
                Some(index) => {
                    self.taken.swap_remove(index);
                    Typed::Nothing
                }
                None => Typed::Key(None),
            }
        } else {
            let keysym = self.state.key_get_one_sym(keycode);
            let switch = xkb::keysym_get_name(keysym)
                .strip_prefix("XF86Switch_VT_")
                .and_then(|number| number.parse().ok());
            if let Some(number) = switch {
                self.taken.push(code);
                Typed::SwitchTerminal(number)
            } else {
                match self
                    .compose
                    .as_mut()
                    .map(|compose| (compose.feed(keysym), compose))
                {
                    Some((compose::FeedResult::Accepted, compose))
                        if compose.status() != compose::Status::Nothing =>
                    {
                        self.taken.push(code);
                        let text = match compose.status() {
                            compose::Status::Composing => {
                                self.composition.push_str(&shown(keysym));
                                None
                            }
                            status => {
                                self.composition.clear();
                                let text = (status == compose::Status::Composed)
                                    .then(|| compose.utf8())
                                    .flatten();
                                compose.reset();
                                text
                            }
                        };
                        Typed::Compose {
                            composition: self.composition.clone(),
                            text: text.filter(|text| typable(text)),
                        }
                    }
                    _ => Typed::Key(Some(self.state.key_get_utf8(keycode)).filter(|t| typable(t))),
                }
            }
        };
        self.state.update_key(
            keycode,
            if pressed {
                xkb::KeyDirection::Down
            } else {
                xkb::KeyDirection::Up
            },
        );
        typed
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

    /// Closes the devices while another session has the seat (a terminal switch). libinput
    /// releases the keys and buttons still down, which the next dispatch reports.
    pub fn suspend(&mut self) {
        self.libinput.suspend();
    }

    /// Opens the devices again after [`suspend`](Self::suspend).
    pub fn resume(&mut self) -> io::Result<()> {
        self.libinput
            .resume()
            .map_err(|()| io::Error::other("libinput cannot resume"))
    }

    /// What has happened since the last call; never blocks.
    pub fn dispatch(&mut self) -> io::Result<Vec<Action>> {
        match self.libinput.dispatch() {
            // A signal interrupted libinput's wait for its devices: what it has is still read.
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            result => result?,
        }
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
                    match self.keyboard.key(key.key(), pressed) {
                        Typed::Key(text) => actions.push(Action::Key {
                            code: key.key(),
                            pressed,
                            text,
                        }),
                        Typed::Compose { composition, text } => {
                            actions.push(Action::Compose { composition, text })
                        }
                        Typed::SwitchTerminal(number) => {
                            actions.push(Action::SwitchTerminal(number))
                        }
                        Typed::Nothing => {}
                    }
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
