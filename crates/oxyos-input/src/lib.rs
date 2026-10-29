//! Real input for the compositor (#32): devices through libinput (udev, seat0), which does
//! pointer acceleration, touchpads and wheels, and a keymap through xkbcommon, because the
//! compositor owns the keymap and sends applications text, never key codes to translate
//! (docs/native-foundation.mdx).

use input::event::gesture::{
    GestureEndEvent, GestureEvent, GesturePinchEvent, GesturePinchEventTrait,
};
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
    /// The keymap's end of the session (Ctrl+Alt+Backspace, XKB's `terminate:ctrl_alt_bksp`),
    /// which no application sees as a key.
    Terminate,
    /// The session's lock (Ctrl+Super+Q, the key that types `q`, as macOS's Ctrl+Cmd+Q, ADR
    /// 0010), which no application sees as a key.
    Lock,
    /// Caps Lock came on (`true`) or went off, after the key that did it.
    CapsLock(bool),
    /// A touchpad pinch (two or more fingers spreading, closing or turning; libinput tells it from
    /// a two-finger scroll): `scale` is the fingers' spread over their spread when it began, and
    /// `rotation` how far they have turned since, in degrees, clockwise.
    Pinch {
        phase: GesturePhase,
        scale: f64,
        rotation: f64,
    },
}

/// Where a touchpad gesture is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GesturePhase {
    Begin,
    Update,
    End,
    /// Ended without effect: libinput took the fingers for another gesture, or one was lifted
    /// too soon.
    Cancel,
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
    /// A press of the keymap's end of the session (see [`Action::Terminate`]).
    Terminate,
    /// A press of the session's lock (see [`Action::Lock`]).
    Lock,
    /// The release of a key the compose table, a terminal switch, the lock or the end of the
    /// session took:
    /// nothing for anyone.
    Nothing,
}

/// How libinput opens and closes devices: directly (root), or through the session's seat
/// manager (`Input::open_with`).
pub type Open = Box<dyn FnMut(&Path) -> io::Result<OwnedFd>>;
pub type Close = Box<dyn FnMut(OwnedFd)>;

struct Opener {
    open: Option<Open>,
    close: Option<Close>,
}

impl LibinputInterface for Opener {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        if let Some(open) = &mut self.open {
            return open(path).map_err(|error| -error.raw_os_error().unwrap_or(eio()));
        }
        use rustix::fs::{open, Mode, OFlags};
        let flags = OFlags::from_bits_retain(flags as u32) | OFlags::CLOEXEC | OFlags::NONBLOCK;
        open(path, flags, Mode::empty()).map_err(|error| -error.raw_os_error())
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        match &mut self.close {
            Some(close) => close(fd),
            None => drop(fd),
        }
    }
}

/// EIO, for an open that failed without an OS error.
fn eio() -> i32 {
    rustix::io::Errno::IO.raw_os_error()
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

/// One key of a sequence that types a character: a Linux key code, and the modifier keys held
/// while it is pressed (Shift, the layout's AltGr), in the order they are pressed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stroke {
    pub code: u32,
    pub held: Vec<u32>,
}

impl Stroke {
    pub fn new(code: u32, held: &[u32]) -> Self {
        Self {
            code,
            held: held.to_vec(),
        }
    }
}

/// The left Shift key's and the right Alt key's Linux input codes.
const LEFT_SHIFT: u32 = 42;
const RIGHT_ALT: u32 = 100;

/// An XKB keymap by its names: a layout (`us`, `es`, ...), its variant and options.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    pub layout: String,
    pub variant: String,
    pub options: String,
}

impl Layout {
    pub fn named(layout: &str) -> Self {
        Self {
            layout: layout.to_string(),
            ..Self::default()
        }
    }

    /// The system's keyboard, as Debian configures it for the console and every desktop
    /// (`/etc/default/keyboard`, which `localectl set-x11-keymap` writes): the keymap passwords
    /// are typed with at a login. None without the file or a layout in it.
    pub fn system() -> Option<Self> {
        Self::parse(&std::fs::read_to_string("/etc/default/keyboard").ok()?)
    }

    /// `XKBLAYOUT`, `XKBVARIANT` and `XKBOPTIONS` from that file's shell assignments.
    pub fn parse(text: &str) -> Option<Self> {
        let mut layout = Self::default();
        for line in text.lines() {
            let Some((name, value)) = line.trim().split_once('=') else {
                continue;
            };
            let value = value
                .trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .to_string();
            match name.trim() {
                "XKBLAYOUT" => layout.layout = value,
                "XKBVARIANT" => layout.variant = value,
                "XKBOPTIONS" => layout.options = value,
                _ => {}
            }
        }
        (!layout.layout.is_empty()).then_some(layout)
    }
}

impl std::fmt::Display for Layout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.layout)?;
        if !self.variant.is_empty() {
            write!(f, " ({})", self.variant)?;
        }
        if !self.options.is_empty() {
            write!(f, " [{}]", self.options)?;
        }
        Ok(())
    }
}

impl Keyboard {
    /// The keymap for an XKB layout (`us`, `es`, ...), with the default rules and model, and
    /// the compose table of the locale the environment names (`LC_ALL`, `LC_CTYPE`, `LANG`),
    /// as xkbcommon prescribes.
    pub fn new(layout: &Layout) -> io::Result<Self> {
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
    pub fn with_locale(layout: &Layout, locale: &OsStr) -> io::Result<Self> {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        // Ctrl+Alt+Backspace ends the session (Terminate_Server), whatever else the options say.
        let options = match layout.options.as_str() {
            "" => "terminate:ctrl_alt_bksp".to_string(),
            options => format!("{options},terminate:ctrl_alt_bksp"),
        };
        let keymap = xkb::Keymap::new_from_names(
            &context,
            "",
            "",
            &layout.layout,
            &layout.variant,
            Some(options),
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("no XKB keymap for {layout}"),
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

    /// Whether Control and Super, and no other modifier, are held: with `q`, the lock.
    fn locks(&self) -> bool {
        let held = |name| {
            self.state
                .mod_name_is_active(name, xkb::STATE_MODS_EFFECTIVE)
        };
        held(xkb::MOD_NAME_CTRL)
            && held(xkb::MOD_NAME_LOGO)
            && !held(xkb::MOD_NAME_ALT)
            && !held(xkb::MOD_NAME_SHIFT)
    }

    /// Whether Caps Lock is on: the keymap's Lock modifier is locked.
    pub fn caps_locked(&self) -> bool {
        self.state
            .mod_name_is_active(xkb::MOD_NAME_CAPS, xkb::STATE_MODS_LOCKED)
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
            let name = xkb::keysym_get_name(keysym);
            let switch = name
                .strip_prefix("XF86Switch_VT_")
                .and_then(|number| number.parse().ok());
            if let Some(number) = switch {
                self.taken.push(code);
                Typed::SwitchTerminal(number)
            } else if name == "Terminate_Server" {
                self.taken.push(code);
                Typed::Terminate
            } else if name == "q" && self.locks() {
                self.taken.push(code);
                Typed::Lock
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
        let mut wanted = [0u8; 4];
        let wanted = character.encode_utf8(&mut wanted);
        // The layout's AltGr, if it has one: a key whose symbol shifts to the third level, the
        // right Alt where it is one (the keymap also gives it to keys no keyboard has).
        let plain = xkb::State::new(&keymap);
        let level3 = |code: &u32| {
            xkb::keysym_get_name(plain.key_get_one_sym(xkb::Keycode::new(code + 8)))
                == "ISO_Level3_Shift"
        };
        let altgr = Some(RIGHT_ALT)
            .filter(level3)
            .or_else(|| (1..248).find(level3));
        let mut levels = vec![vec![], vec![LEFT_SHIFT]];
        if let Some(altgr) = altgr {
            levels.extend([vec![altgr], vec![LEFT_SHIFT, altgr]]);
        }
        // Every key's symbol and text at each level, with its modifiers held as a person holds
        // them (pressed, not a mask), the fewest first.
        let mut keys = Vec::new();
        for held in &levels {
            let mut state = xkb::State::new(&keymap);
            for &key in held {
                state.update_key(xkb::Keycode::new(key + 8), xkb::KeyDirection::Down);
            }
            for code in (1..248).filter(|code| !held.contains(code)) {
                let keycode = xkb::Keycode::new(code + 8);
                keys.push((
                    Stroke::new(code, held),
                    state.key_get_one_sym(keycode),
                    state.key_get_utf8(keycode),
                ));
            }
        }
        if let Some((stroke, _, _)) = keys.iter().find(|(_, _, text)| text == wanted) {
            return Some(vec![stroke.clone()]);
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
                    return Some(vec![dead_stroke.clone(), stroke.clone()]);
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
    /// The pinch going on: how far its fingers have turned (libinput reports each step's turn).
    pinch_rotation: f64,
}

impl Input {
    /// Opens seat0's devices (hotplugged ones too) for an output of `bounds` logical pixels; the
    /// pointer starts at its centre.
    pub fn open(bounds: (u32, u32), layout: &Layout) -> io::Result<Self> {
        Self::open_with(bounds, layout, None)
    }

    /// The same, opening and closing devices with `devices` (the session's seat manager)
    /// instead of directly.
    pub fn open_with(
        bounds: (u32, u32),
        layout: &Layout,
        devices: Option<(Open, Close)>,
    ) -> io::Result<Self> {
        let (open, close) = match devices {
            Some((open, close)) => (Some(open), Some(close)),
            None => (None, None),
        };
        let mut libinput = Libinput::new_with_udev(Opener { open, close });
        libinput
            .udev_assign_seat("seat0")
            .map_err(|()| io::Error::other("libinput cannot assign seat0"))?;
        let bounds = (f64::from(bounds.0), f64::from(bounds.1));
        Ok(Self {
            libinput,
            keyboard: Keyboard::new(layout)?,
            pointer: (bounds.0 / 2.0, bounds.1 / 2.0),
            bounds,
            pinch_rotation: 0.0,
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
                input::Event::Gesture(GestureEvent::Pinch(pinch)) => {
                    let (phase, angle) = match &pinch {
                        GesturePinchEvent::Begin(_) => (GesturePhase::Begin, 0.0),
                        GesturePinchEvent::Update(update) => {
                            (GesturePhase::Update, update.angle_delta())
                        }
                        GesturePinchEvent::End(end) if end.cancelled() => {
                            (GesturePhase::Cancel, 0.0)
                        }
                        _ => (GesturePhase::End, 0.0),
                    };
                    actions.push(self.pinch(phase, pinch.scale(), angle));
                }
                input::Event::Keyboard(KeyboardEvent::Key(key)) => {
                    let pressed = key.key_state() == KeyState::Pressed;
                    let caps = self.keyboard.caps_locked();
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
                        Typed::Terminate => actions.push(Action::Terminate),
                        Typed::Lock => actions.push(Action::Lock),
                        Typed::Nothing => {}
                    }
                    if self.keyboard.caps_locked() != caps {
                        actions.push(Action::CapsLock(!caps));
                    }
                }
                _ => {}
            }
        }
        Ok(actions)
    }

    /// A pinch step: its rotation is the sum of the turns libinput reported since it began.
    fn pinch(&mut self, phase: GesturePhase, scale: f64, angle_delta: f64) -> Action {
        if phase == GesturePhase::Begin {
            self.pinch_rotation = 0.0;
        }
        self.pinch_rotation += angle_delta;
        Action::Pinch {
            phase,
            scale,
            rotation: self.pinch_rotation,
        }
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
