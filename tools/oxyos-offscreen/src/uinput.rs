//! Virtual input devices for `--launch --uinput`: the director points, scrolls, types and pinches
//! through the kernel's input subsystem, so its steps reach the compositor through libinput like
//! a real keyboard's, pointer's and touchpad's do.

use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, InputEvent, KeyCode, PropType, RelativeAxisCode,
    UinputAbsSetup,
};
use std::io;
use std::time::Duration;

const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_ABS: u16 = 3;
/// The touchpad: 100 by 60 mm at 40 units a millimetre (libinput recognizes gestures in
/// millimetres, so a touchpad needs its resolution).
const PAD: (i32, i32) = (4000, 2400);
const PAD_RESOLUTION: i32 = 40;
/// One wheel notch in REL_WHEEL_HI_RES units; libinput turns a notch into 15 logical pixels.
const NOTCH: i32 = 120;
const PIXELS_PER_NOTCH: i32 = 15;
/// How far libinput scrolls for each millimetre two fingers move on this touchpad, in logical
/// pixels: its motion normalized to 1000 dpi (39.37 units a millimetre), slowed as it slows a
/// touchpad's scroll. Measured: libinput 1.28 scrolls 6.84 for a frame of 26 units (0.65 mm).
const SCROLL_PIXELS_PER_MM: f64 = 10.52;

pub struct Devices {
    keyboard: VirtualDevice,
    pointer: VirtualDevice,
    touchpad: VirtualDevice,
    layout: oxyos_input::Keyboard,
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
            .name("oxyos-offscreen keyboard")
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
            .name("oxyos-offscreen pointer")
            .with_keys(&buttons)?
            .with_absolute_axis(&axis(AbsoluteAxisCode::ABS_X, display.0))?
            .with_absolute_axis(&axis(AbsoluteAxisCode::ABS_Y, display.1))?
            .with_relative_axes(&wheel)?
            .build()?;
        // A clickpad (as a laptop's) taking up to three fingers, each in a multitouch slot.
        let mut pad_keys = AttributeSet::<KeyCode>::new();
        for key in [
            KeyCode::BTN_LEFT,
            KeyCode::BTN_TOUCH,
            KeyCode::BTN_TOOL_FINGER,
            KeyCode::BTN_TOOL_DOUBLETAP,
            KeyCode::BTN_TOOL_TRIPLETAP,
        ] {
            pad_keys.insert(key);
        }
        let mut properties = AttributeSet::<PropType>::new();
        properties.insert(PropType::POINTER);
        properties.insert(PropType::BUTTONPAD);
        let pad_axis = |code, max: i32| {
            UinputAbsSetup::new(code, AbsInfo::new(0, 0, max, 0, 0, PAD_RESOLUTION))
        };
        let touchpad = VirtualDevice::builder()?
            .name("oxyos-offscreen touchpad")
            .with_keys(&pad_keys)?
            .with_properties(&properties)?
            .with_absolute_axis(&pad_axis(AbsoluteAxisCode::ABS_X, PAD.0))?
            .with_absolute_axis(&pad_axis(AbsoluteAxisCode::ABS_Y, PAD.1))?
            .with_absolute_axis(&pad_axis(AbsoluteAxisCode::ABS_MT_POSITION_X, PAD.0))?
            .with_absolute_axis(&pad_axis(AbsoluteAxisCode::ABS_MT_POSITION_Y, PAD.1))?
            .with_absolute_axis(&UinputAbsSetup::new(
                AbsoluteAxisCode::ABS_MT_SLOT,
                AbsInfo::new(0, 0, 2, 0, 0, 0),
            ))?
            .with_absolute_axis(&UinputAbsSetup::new(
                AbsoluteAxisCode::ABS_MT_TRACKING_ID,
                AbsInfo::new(0, 0, 65535, 0, 0, 0),
            ))?
            .build()?;
        Ok(Self {
            keyboard,
            pointer,
            touchpad,
            layout: oxyos_input::Keyboard::new(&oxyos_input::Layout::named(layout))?,
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
        self.button_code(u32::from(KeyCode::BTN_LEFT.0), pressed)
    }

    /// Presses or releases one of the pointer's buttons by its Linux code (BTN_LEFT, BTN_RIGHT,
    /// BTN_MIDDLE).
    pub fn button_code(&mut self, code: u32, pressed: bool) -> io::Result<()> {
        let code = u16::try_from(code).map_err(io::Error::other)?;
        self.pointer
            .emit(&[InputEvent::new(EV_KEY, code, i32::from(pressed))])
    }

    /// Turns the wheel as far as libinput scrolls `dy` logical pixels for, toward the person for a
    /// positive `dy` (which moves the content's view down, unless the pointer's natural scrolling
    /// turns it), in whole and partial notches as a high-resolution wheel does, calling `sent`
    /// after each notch.
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

    /// Pinches on the touchpad: two fingers 20 mm apart in its middle spread to `percent` of that
    /// and turn `degrees` clockwise, steadily over 30 frames 10 ms apart, then lift; `sent` is
    /// called after each frame. libinput tells the pinch from a scroll by itself, and reports it
    /// from when it recognizes it.
    pub fn pinch(&mut self, percent: i32, degrees: i32, mut sent: impl FnMut()) -> io::Result<()> {
        const FRAMES: i32 = 30;
        let abs = |code: AbsoluteAxisCode, value| InputEvent::new(EV_ABS, code.0, value);
        let key = |code: KeyCode, down: bool| InputEvent::new(EV_KEY, code.0, i32::from(down));
        let fingers = |frame: i32| {
            let done = f64::from(frame) / f64::from(FRAMES);
            let half = f64::from(10 * PAD_RESOLUTION) * (f64::from(percent) / 100.0).powf(done);
            let angle = (f64::from(degrees) * done).to_radians();
            let (dx, dy) = (half * angle.cos(), half * angle.sin());
            let centre = (f64::from(PAD.0) / 2.0, f64::from(PAD.1) / 2.0);
            [
                (
                    (centre.0 - dx).round() as i32,
                    (centre.1 - dy).round() as i32,
                ),
                (
                    (centre.0 + dx).round() as i32,
                    (centre.1 + dy).round() as i32,
                ),
            ]
        };
        let place = |points: [(i32, i32); 2]| {
            let mut events = Vec::new();
            for (slot, (x, y)) in points.iter().enumerate() {
                events.push(abs(AbsoluteAxisCode::ABS_MT_SLOT, slot as i32));
                events.push(abs(AbsoluteAxisCode::ABS_MT_POSITION_X, *x));
                events.push(abs(AbsoluteAxisCode::ABS_MT_POSITION_Y, *y));
            }
            events.push(abs(AbsoluteAxisCode::ABS_X, points[0].0));
            events.push(abs(AbsoluteAxisCode::ABS_Y, points[0].1));
            events
        };
        let mut down = vec![
            abs(AbsoluteAxisCode::ABS_MT_SLOT, 0),
            abs(AbsoluteAxisCode::ABS_MT_TRACKING_ID, 1),
            abs(AbsoluteAxisCode::ABS_MT_SLOT, 1),
            abs(AbsoluteAxisCode::ABS_MT_TRACKING_ID, 2),
        ];
        down.extend(place(fingers(0)));
        down.extend([
            key(KeyCode::BTN_TOUCH, true),
            key(KeyCode::BTN_TOOL_DOUBLETAP, true),
        ]);
        self.touchpad.emit(&down)?;
        sent();
        for frame in 1..=FRAMES {
            std::thread::sleep(Duration::from_millis(10));
            self.touchpad.emit(&place(fingers(frame)))?;
            sent();
        }
        std::thread::sleep(Duration::from_millis(10));
        self.touchpad.emit(&[
            abs(AbsoluteAxisCode::ABS_MT_SLOT, 0),
            abs(AbsoluteAxisCode::ABS_MT_TRACKING_ID, -1),
            abs(AbsoluteAxisCode::ABS_MT_SLOT, 1),
            abs(AbsoluteAxisCode::ABS_MT_TRACKING_ID, -1),
            key(KeyCode::BTN_TOUCH, false),
            key(KeyCode::BTN_TOOL_DOUBLETAP, false),
        ])?;
        sent();
        Ok(())
    }

    /// Scrolls with two fingers on the touchpad, 20 mm apart across its middle: both move down
    /// (up for a negative `dy`) as far as libinput scrolls `dy` logical pixels for, steadily over
    /// `frames` frames 10 ms apart, rest 150 ms (so nothing is flung) and lift; `sent` is called
    /// after each frame. libinput tells the scroll from a pinch by itself, and turns it as the
    /// device's natural scrolling says: content following the fingers, or the view.
    pub fn two_fingers(&mut self, dy: i32, frames: u32, mut sent: impl FnMut()) -> io::Result<()> {
        let abs = |code: AbsoluteAxisCode, value| InputEvent::new(EV_ABS, code.0, value);
        let key = |code: KeyCode, down: bool| InputEvent::new(EV_KEY, code.0, i32::from(down));
        let travel = f64::from(dy) / SCROLL_PIXELS_PER_MM * f64::from(PAD_RESOLUTION);
        let (left, right) = (
            PAD.0 / 2 - 10 * PAD_RESOLUTION,
            PAD.0 / 2 + 10 * PAD_RESOLUTION,
        );
        let top = f64::from(PAD.1) / 2.0 - travel / 2.0;
        let fingers = |frame: u32| {
            let y = (top + travel * f64::from(frame) / f64::from(frames.max(1))).round() as i32;
            vec![
                abs(AbsoluteAxisCode::ABS_MT_SLOT, 0),
                abs(AbsoluteAxisCode::ABS_MT_POSITION_X, left),
                abs(AbsoluteAxisCode::ABS_MT_POSITION_Y, y),
                abs(AbsoluteAxisCode::ABS_MT_SLOT, 1),
                abs(AbsoluteAxisCode::ABS_MT_POSITION_X, right),
                abs(AbsoluteAxisCode::ABS_MT_POSITION_Y, y),
                abs(AbsoluteAxisCode::ABS_X, left),
                abs(AbsoluteAxisCode::ABS_Y, y),
            ]
        };
        let mut down = vec![
            abs(AbsoluteAxisCode::ABS_MT_SLOT, 0),
            abs(AbsoluteAxisCode::ABS_MT_TRACKING_ID, 1),
            abs(AbsoluteAxisCode::ABS_MT_SLOT, 1),
            abs(AbsoluteAxisCode::ABS_MT_TRACKING_ID, 2),
        ];
        down.extend(fingers(0));
        down.extend([
            key(KeyCode::BTN_TOUCH, true),
            key(KeyCode::BTN_TOOL_DOUBLETAP, true),
        ]);
        self.touchpad.emit(&down)?;
        sent();
        for frame in 1..=frames {
            std::thread::sleep(Duration::from_millis(10));
            self.touchpad.emit(&fingers(frame))?;
            sent();
        }
        std::thread::sleep(Duration::from_millis(150));
        self.touchpad.emit(&[
            abs(AbsoluteAxisCode::ABS_MT_SLOT, 0),
            abs(AbsoluteAxisCode::ABS_MT_TRACKING_ID, -1),
            abs(AbsoluteAxisCode::ABS_MT_SLOT, 1),
            abs(AbsoluteAxisCode::ABS_MT_TRACKING_ID, -1),
            key(KeyCode::BTN_TOUCH, false),
            key(KeyCode::BTN_TOOL_DOUBLETAP, false),
        ])?;
        sent();
        Ok(())
    }

    pub fn key(&mut self, code: u16, pressed: bool) -> io::Result<()> {
        self.keyboard
            .emit(&[InputEvent::new(EV_KEY, code, i32::from(pressed))])
    }

    /// Types `character` with the keys that type it on the layout: one key, or a dead key and
    /// the key it composes with. `false` if none do, and nothing was sent.
    pub fn type_char(&mut self, character: char) -> io::Result<bool> {
        let Some(strokes) = self.layout.key_for(character) else {
            return Ok(false);
        };
        // The modifiers (Shift, AltGr) held around the key, as a person holds them.
        for stroke in strokes {
            for &key in &stroke.held {
                self.key(key as u16, true)?;
            }
            self.key(stroke.code as u16, true)?;
            self.key(stroke.code as u16, false)?;
            for &key in stroke.held.iter().rev() {
                self.key(key as u16, false)?;
            }
        }
        Ok(true)
    }
}
