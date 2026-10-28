//! Virtual input devices for `--launch --uinput`: the director points, scrolls and types
//! through the kernel's input subsystem, so its steps reach the compositor through libinput like
//! a real keyboard's and pointer's do.

use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, InputEvent, KeyCode, RelativeAxisCode, UinputAbsSetup,
};
use std::io;

const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_ABS: u16 = 3;
const LEFT_SHIFT: u16 = 42;
/// One wheel notch in REL_WHEEL_HI_RES units; libinput turns a notch into 15 logical pixels.
const NOTCH: i32 = 120;
const PIXELS_PER_NOTCH: i32 = 15;

pub struct Devices {
    keyboard: VirtualDevice,
    pointer: VirtualDevice,
    layout: oxy_input::Keyboard,
}

impl Devices {
    /// A keyboard typing with `layout`, and an absolute pointer whose axes span the display's
    /// pixels exactly (libinput maps a range of `0..size` onto `size` pixels one to one). The
    /// pointer is a wheel mouse with the wheel's button: libinput emulates a middle button on a
    /// pointer without one, holding each left press until its release or a timeout, so a drag
    /// would press where it ends.
    pub fn new(display: (u32, u32), layout: &str) -> io::Result<Self> {
        let mut keys = AttributeSet::<KeyCode>::new();
        for code in 1..248 {
            keys.insert(KeyCode::new(code));
        }
        let keyboard = VirtualDevice::builder()?
            .name("oxy-offscreen keyboard")
            .with_keys(&keys)?
            .build()?;
        let mut buttons = AttributeSet::<KeyCode>::new();
        buttons.insert(KeyCode::BTN_LEFT);
        buttons.insert(KeyCode::BTN_RIGHT);
        buttons.insert(KeyCode::BTN_MIDDLE);
        let mut wheel = AttributeSet::<RelativeAxisCode>::new();
        wheel.insert(RelativeAxisCode::REL_WHEEL);
        wheel.insert(RelativeAxisCode::REL_WHEEL_HI_RES);
        let axis = |code, size: u32| {
            UinputAbsSetup::new(code, AbsInfo::new(0, 0, size as i32 - 1, 0, 0, 0))
        };
        let pointer = VirtualDevice::builder()?
            .name("oxy-offscreen pointer")
            .with_keys(&buttons)?
            .with_absolute_axis(&axis(AbsoluteAxisCode::ABS_X, display.0))?
            .with_absolute_axis(&axis(AbsoluteAxisCode::ABS_Y, display.1))?
            .with_relative_axes(&wheel)?
            .build()?;
        Ok(Self {
            keyboard,
            pointer,
            layout: oxy_input::Keyboard::new(layout)?,
        })
    }

    /// Moves the pointer to a point of the display, in its pixels.
    pub fn move_to(&mut self, x: i32, y: i32) -> io::Result<()> {
        self.pointer.emit(&[
            InputEvent::new(EV_ABS, AbsoluteAxisCode::ABS_X.0, x),
            InputEvent::new(EV_ABS, AbsoluteAxisCode::ABS_Y.0, y),
        ])
    }

    pub fn button(&mut self, pressed: bool) -> io::Result<()> {
        self.pointer.emit(&[InputEvent::new(
            EV_KEY,
            KeyCode::BTN_LEFT.0,
            i32::from(pressed),
        )])
    }

    /// Scrolls by `dy` logical pixels (positive moves the content's view down), in whole and
    /// partial notches as a high-resolution wheel does, calling `sent` after each notch.
    pub fn wheel(&mut self, dy: i32, mut sent: impl FnMut()) -> io::Result<()> {
        let mut units = -dy * NOTCH / PIXELS_PER_NOTCH;
        while units != 0 {
            let step = units.clamp(-NOTCH, NOTCH);
            let mut events = vec![InputEvent::new(
                EV_REL,
                RelativeAxisCode::REL_WHEEL_HI_RES.0,
                step,
            )];
            if step.abs() == NOTCH {
                events.push(InputEvent::new(
                    EV_REL,
                    RelativeAxisCode::REL_WHEEL.0,
                    step / NOTCH,
                ));
            }
            self.pointer.emit(&events)?;
            sent();
            units -= step;
        }
        Ok(())
    }

    pub fn key(&mut self, code: u16, pressed: bool) -> io::Result<()> {
        self.keyboard
            .emit(&[InputEvent::new(EV_KEY, code, i32::from(pressed))])
    }

    /// Types `character` with the key that types it on the layout; `false` if none does (a
    /// dead key and a letter, for one), and nothing was sent.
    pub fn type_char(&mut self, character: char) -> io::Result<bool> {
        let Some((code, shifted)) = self.layout.key_for(character) else {
            return Ok(false);
        };
        let code = code as u16;
        if shifted {
            self.key(LEFT_SHIFT, true)?;
        }
        self.key(code, true)?;
        self.key(code, false)?;
        if shifted {
            self.key(LEFT_SHIFT, false)?;
        }
        Ok(true)
    }
}
