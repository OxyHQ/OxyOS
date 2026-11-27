//! Virtual input devices for `--launch --uinput`: the director points, scrolls, types and pinches
//! through the kernel's input subsystem, so its steps reach the compositor through libinput like
//! a real keyboard's, pointer's and touchpad's do.

use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, InputEvent, KeyCode, PropType, RelativeAxisCode,
    UinputAbsSetup,
};
use std::io;
use std::path::PathBuf;
use std::time::Duration;

const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_ABS: u16 = 3;
/// The touchpad: 100 by 60 mm at 40 units a millimetre (libinput recognizes gestures in
/// millimetres, so a touchpad needs its resolution).
const PAD: (i32, i32) = (4000, 2400);
const PAD_RESOLUTION: i32 = 40;
/// How many fingers the touchpad tracks, each in a multitouch slot: four, the thumb and three
/// fingers of Launchpad's pinch and the four of a swipe (ADR 0046).
const PAD_SLOTS: i32 = 4;
/// One wheel notch in REL_WHEEL_HI_RES units; libinput turns a notch into 15 logical pixels.
const NOTCH: i32 = 120;
const PIXELS_PER_NOTCH: i32 = 15;
/// How far libinput scrolls for each millimetre two fingers move on this touchpad, in logical
/// pixels: its motion normalized to 1000 dpi (39.37 units a millimetre), slowed as it slows a
/// touchpad's scroll. Measured: libinput 1.28 scrolls 6.84 for a frame of 26 units (0.65 mm).
/// A swipe's unaccelerated motion is slowed alike (ADR 0046's distances are in it): 3.94 for a
/// frame of three fingers moving 15 units (0.375 mm).
const SCROLL_PIXELS_PER_MM: f64 = 10.52;

fn abs(code: AbsoluteAxisCode, value: i32) -> InputEvent {
    InputEvent::new(EV_ABS, code.0, value)
}

fn key(code: KeyCode, down: bool) -> InputEvent {
    InputEvent::new(EV_KEY, code.0, i32::from(down))
}

/// The touchpad's fingers at `points`, each in its slot, the first also the single-touch axes.
fn place(points: &[(i32, i32)]) -> Vec<InputEvent> {
    let mut events = Vec::new();
    for (slot, (x, y)) in points.iter().enumerate() {
        events.push(abs(AbsoluteAxisCode::ABS_MT_SLOT, slot as i32));
        events.push(abs(AbsoluteAxisCode::ABS_MT_POSITION_X, *x));
        events.push(abs(AbsoluteAxisCode::ABS_MT_POSITION_Y, *y));
    }
    if let Some(&(x, y)) = points.first() {
        events.push(abs(AbsoluteAxisCode::ABS_X, x));
        events.push(abs(AbsoluteAxisCode::ABS_Y, y));
    }
    events
}

// An `input_event` as `emit_frames` writes it, a 64-bit Linux's: its time (seconds and
// microseconds, each a `long`), type, code and value, 24 bytes.
const _: () = assert!(std::mem::size_of::<InputEvent>() == 24);

/// Writes `frames` to `device` in one write, each ended by its SYN_REPORT and stamped `apart`
/// after the one before, the last now: the kernel takes an injected event's time when it is not in
/// the future (Linux 6.3), so libinput reads the frames that far apart however late the write was,
/// and none of its timers can fire between them. A kernel that ignores the times stamps them all
/// now, which still keeps them together.
fn emit_frames(
    device: &VirtualDevice,
    frames: &[Vec<InputEvent>],
    apart: Duration,
) -> io::Result<()> {
    use std::io::Write as _;
    use std::os::fd::AsFd as _;
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    let now = Duration::new(now.tv_sec as u64, now.tv_nsec as u32);
    let mut bytes = Vec::new();
    for (index, frame) in frames.iter().enumerate() {
        let before = u32::try_from(frames.len() - 1 - index).map_err(io::Error::other)?;
        let at = now.saturating_sub(apart * before);
        let syn = InputEvent::new(0, 0, 0);
        for event in frame.iter().chain(std::iter::once(&syn)) {
            bytes.extend((at.as_secs() as i64).to_ne_bytes());
            bytes.extend(i64::from(at.subsec_micros()).to_ne_bytes());
            bytes.extend(event.event_type().0.to_ne_bytes());
            bytes.extend(event.code().to_ne_bytes());
            bytes.extend(event.value().to_ne_bytes());
        }
    }
    std::fs::File::from(device.as_fd().try_clone_to_owned()?).write_all(&bytes)
}

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
        // A clickpad (as a laptop's) taking up to four fingers, each in a multitouch slot.
        let mut pad_keys = AttributeSet::<KeyCode>::new();
        for key in [
            KeyCode::BTN_LEFT,
            KeyCode::BTN_TOUCH,
            KeyCode::BTN_TOOL_FINGER,
            KeyCode::BTN_TOOL_DOUBLETAP,
            KeyCode::BTN_TOOL_TRIPLETAP,
            KeyCode::BTN_TOOL_QUADTAP,
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
                AbsInfo::new(0, 0, PAD_SLOTS - 1, 0, 0, 0),
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

    /// The devices' event nodes, found by their own input devices in sysfs (the kernel's names
    /// for them, never their names, which every run's devices share): what the compositor reads,
    /// and nothing else (`oxyos_input::Input::open_paths`).
    pub fn nodes(&mut self) -> io::Result<Vec<PathBuf>> {
        [&mut self.keyboard, &mut self.pointer, &mut self.touchpad]
            .into_iter()
            .map(|device| {
                let nodes = device
                    .enumerate_dev_nodes_blocking()?
                    .collect::<io::Result<Vec<_>>>()?;
                match <[PathBuf; 1]>::try_from(nodes) {
                    Ok([node]) => Ok(node),
                    Err(nodes) => Err(io::Error::other(format!(
                        "{}: {} event nodes, not one",
                        device.get_syspath()?.display(),
                        nodes.len()
                    ))),
                }
            })
            .collect()
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

    /// Pinches on the touchpad: `fingers` (two to four) 20 mm from their middle, in the pad's
    /// middle, spread to `percent` of that and turn `degrees` clockwise around it, steadily over
    /// 30 frames 10 ms apart, then lift; `sent` is called after each frame. Two are opposite each
    /// other; three or four are the thumb below and the other fingers above it, as a hand pinches
    /// Launchpad (ADR 0046). libinput tells the pinch from a scroll or a swipe by itself, counts
    /// its fingers, and reports it from when it recognizes it.
    pub fn pinch(
        &mut self,
        fingers: u8,
        percent: i32,
        degrees: i32,
        mut sent: impl FnMut(),
    ) -> io::Result<()> {
        const FRAMES: i32 = 30;
        let fingers = usize::from(fingers.clamp(2, PAD_SLOTS as u8));
        // Where each finger is around the middle, in degrees clockwise from the right: two
        // opposite; the thumb at the bottom and the others spread over the top.
        let around: &[f64] = match fingers {
            2 => &[180.0, 0.0],
            3 => &[110.0, 210.0, 330.0],
            _ => &[110.0, 200.0, 270.0, 340.0],
        };
        let centre = (f64::from(PAD.0) / 2.0, f64::from(PAD.1) / 2.0);
        let points = |frame: i32| -> Vec<(i32, i32)> {
            let done = f64::from(frame) / f64::from(FRAMES);
            let reach = f64::from(10 * PAD_RESOLUTION) * (f64::from(percent) / 100.0).powf(done);
            let turned = f64::from(degrees) * done;
            around
                .iter()
                .map(|angle| {
                    let angle = (angle + turned).to_radians();
                    (
                        (centre.0 + reach * angle.cos()).round() as i32,
                        (centre.1 + reach * angle.sin()).round() as i32,
                    )
                })
                .collect()
        };
        self.touch(&points(0), true)?;
        sent();
        for frame in 1..=FRAMES {
            std::thread::sleep(Duration::from_millis(10));
            self.touchpad.emit(&place(&points(frame)))?;
            sent();
        }
        std::thread::sleep(Duration::from_millis(10));
        self.touch(&points(FRAMES), false)?;
        sent();
        Ok(())
    }

    /// Swipes `fingers` (three or four) on the touchpad, 15 mm apart in a row across its middle:
    /// together by as far as libinput moves a swipe (dx, dy) logical pixels for (its unaccelerated
    /// motion, slowed as a scroll is, `SCROLL_PIXELS_PER_MM`), steadily over `frames` frames, `hz` a second; then they
    /// rest 100 ms, so that nothing is flicked, and lift. `sent` is called after each frame.
    /// libinput tells the swipe from a pinch by itself, counts its fingers, and reports it from
    /// when it recognizes it.
    pub fn swipe(
        &mut self,
        fingers: u8,
        (dx, dy): (i32, i32),
        frames: u32,
        hz: u32,
        mut sent: impl FnMut(),
    ) -> io::Result<()> {
        let fingers = i32::from(fingers.clamp(1, PAD_SLOTS as u8));
        let unit = f64::from(PAD_RESOLUTION) / SCROLL_PIXELS_PER_MM;
        let travel = (f64::from(dx) * unit, f64::from(dy) * unit);
        let frames = frames.max(1);
        // The row's middle where the whole swipe stays on the pad's middle.
        let start = (
            f64::from(PAD.0) / 2.0 - travel.0 / 2.0,
            f64::from(PAD.1) / 2.0 - travel.1 / 2.0,
        );
        let apart = f64::from(15 * PAD_RESOLUTION);
        let points = |frame: u32| -> Vec<(i32, i32)> {
            let done = f64::from(frame) / f64::from(frames);
            (0..fingers)
                .map(|finger| {
                    let offset = (f64::from(finger) - f64::from(fingers - 1) / 2.0) * apart;
                    (
                        (start.0 + offset + travel.0 * done).round() as i32,
                        (start.1 + travel.1 * done).round() as i32,
                    )
                })
                .collect()
        };
        let interval = Duration::from_secs(1) / hz.max(1);
        self.touch(&points(0), true)?;
        sent();
        for frame in 1..=frames {
            std::thread::sleep(interval);
            self.touchpad.emit(&place(&points(frame)))?;
            sent();
        }
        std::thread::sleep(Duration::from_millis(100));
        self.touch(&points(frames), false)?;
        sent();
        Ok(())
    }

    /// Puts fingers down at `points` (`down`), each in its slot, or lifts them all, with the tool
    /// that says how many they are.
    fn touch(&mut self, points: &[(i32, i32)], down: bool) -> io::Result<()> {
        let tool = match points.len() {
            1 => KeyCode::BTN_TOOL_FINGER,
            2 => KeyCode::BTN_TOOL_DOUBLETAP,
            3 => KeyCode::BTN_TOOL_TRIPLETAP,
            _ => KeyCode::BTN_TOOL_QUADTAP,
        };
        let mut events = Vec::new();
        for slot in 0..points.len() {
            events.push(abs(AbsoluteAxisCode::ABS_MT_SLOT, slot as i32));
            events.push(abs(
                AbsoluteAxisCode::ABS_MT_TRACKING_ID,
                if down { slot as i32 + 1 } else { -1 },
            ));
        }
        if down {
            events.extend(place(points));
        }
        events.push(key(KeyCode::BTN_TOUCH, down));
        events.push(key(tool, down));
        self.touchpad.emit(&events)
    }

    /// Scrolls with two fingers on the touchpad, 20 mm apart across its middle: both move down
    /// (up for a negative `dy`) as far as libinput scrolls `dy` logical pixels for, steadily over
    /// `frames` frames 10 ms apart, rest 150 ms (so nothing is flung) and lift; `sent` is called
    /// after each frame written. libinput tells the scroll from a pinch by itself, and turns it as
    /// the device's natural scrolling says: content following the fingers, or the view.
    ///
    /// Two fingers still for 40 ms are a hold to libinput (its quick hold, which stops a glide), so
    /// the fingers land with the frames that tell it a scroll, in one write, each stamped 10 ms
    /// after the one before (`emit_frames`): libinput reads them 10 ms apart however late the
    /// director wrote them, and the scroll is never a hold first. The frames after are written as
    /// their time comes.
    pub fn two_fingers(&mut self, dy: i32, frames: u32, mut sent: impl FnMut()) -> io::Result<()> {
        let abs = |code: AbsoluteAxisCode, value| InputEvent::new(EV_ABS, code.0, value);
        let key = |code: KeyCode, down: bool| InputEvent::new(EV_KEY, code.0, i32::from(down));
        let travel = f64::from(dy) / SCROLL_PIXELS_PER_MM * f64::from(PAD_RESOLUTION);
        let (left, right) = (
            PAD.0 / 2 - 10 * PAD_RESOLUTION,
            PAD.0 / 2 + 10 * PAD_RESOLUTION,
        );
        let top = f64::from(PAD.1) / 2.0 - travel / 2.0;
        let y_at =
            |frame: u32| (top + travel * f64::from(frame) / f64::from(frames.max(1))).round();
        let fingers = |frame: u32| {
            let y = y_at(frame) as i32;
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
        // The first frame both fingers have moved far enough by for libinput to tell a scroll
        // (1.5 mm, its threshold; 2 to be clear of it), or the last.
        let moved = |frame: u32| (y_at(frame) - y_at(0)).abs() / f64::from(PAD_RESOLUTION);
        let told = (1..=frames)
            .find(|&frame| moved(frame) >= 2.0)
            .unwrap_or(frames);
        // Landing, written as its frames' time comes: the last of them now.
        std::thread::sleep(Duration::from_millis(10) * told);
        let landing: Vec<Vec<InputEvent>> = std::iter::once(down)
            .chain((1..=told).map(fingers))
            .collect();
        emit_frames(&self.touchpad, &landing, Duration::from_millis(10))?;
        sent();
        for frame in told + 1..=frames {
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

#[cfg(test)]
mod tests {
    //! A run reads its own devices and no other: another run's, made alike at the same time, and
    //! the machine's never reach its steps. As root, with the uinput module (they make devices
    //! in the kernel): `cargo test -p oxyos-offscreen --bin oxyos-offscreen --no-run`, then the
    //! test binary with `sudo` and `--ignored --exact
    //! uinput::tests::a_run_reads_its_own_devices_and_never_another_runs` (CI's verify job does).

    use super::Devices;
    use oxyos_input::{Action, GesturePhase, Input, Layout, ScrollSource};
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    const BOUNDS: (u32, u32) = (800, 600);
    const KEY_A: u16 = 30;

    /// A run of the development compositor with `--uinput`: its devices, and its input reading
    /// them, opened as `launch` opens them.
    fn start() -> (Devices, Input) {
        let mut devices = Devices::new(BOUNDS, "us").unwrap();
        let nodes = devices.nodes().unwrap();
        let paths: Vec<_> = nodes.iter().map(PathBuf::as_path).collect();
        let input = Input::open_paths(BOUNDS, &Layout::named("us"), &paths, None).unwrap();
        (devices, input)
    }

    /// What each run's input has read since, as its event loop reads (a step sent whole would
    /// overflow what the kernel holds for a reader).
    fn read(inputs: &mut [Input], actions: &mut [Vec<Action>]) {
        for (input, actions) in inputs.iter_mut().zip(actions.iter_mut()) {
            let read = input.dispatch().unwrap();
            actions.extend(read.into_iter().map(|timed| timed.action));
        }
    }

    /// The wheel's scrolls among `actions`.
    fn wheel(actions: &[Action]) -> Vec<f64> {
        let scrolls = actions.iter().filter_map(|action| match action {
            Action::Scroll {
                source: ScrollSource::Wheel,
                dy,
                ..
            } => Some(*dy),
            _ => None,
        });
        scrolls.collect()
    }

    #[test]
    #[ignore]
    fn a_run_reads_its_own_devices_and_never_another_runs() {
        // This run (1) between two others: one started before it, whose devices the seat had as
        // it opened its own, and one after, whose devices the seat would add.
        let ((before, before_input), (ours, ours_input), (after, after_input)) =
            (start(), start(), start());
        let mut devices = [before, ours, after];
        let mut inputs = [before_input, ours_input, after_input];
        let mut actions: [Vec<Action>; 3] = Default::default();
        // The others point, scroll with the wheel and with two fingers, and type; then this
        // run's wheel turns one notch away from the person.
        for other in [0, 2] {
            let devices = &mut devices[other];
            devices.move_to(10, 10).unwrap();
            devices
                .wheel(45, || read(&mut inputs, &mut actions))
                .unwrap();
            devices
                .two_fingers(60, 6, || read(&mut inputs, &mut actions))
                .unwrap();
            devices.key(KEY_A, true).unwrap();
            devices.key(KEY_A, false).unwrap();
            read(&mut inputs, &mut actions);
        }
        devices[1]
            .wheel(-15, || read(&mut inputs, &mut actions))
            .unwrap();
        // Everything each has read, until half a second after this run read its notch.
        let deadline = Instant::now() + Duration::from_secs(5);
        while wheel(&actions[1]).is_empty() {
            assert!(
                Instant::now() < deadline,
                "no notch within 5 s: {actions:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
            read(&mut inputs, &mut actions);
        }
        let settled = Instant::now() + Duration::from_millis(500);
        while Instant::now() < settled {
            std::thread::sleep(Duration::from_millis(10));
            read(&mut inputs, &mut actions);
        }
        assert!(
            matches!(actions[1].as_slice(), [Action::Scroll { dx, dy, .. }] if *dx == 0.0 && *dy == -15.0),
            "this run's notch and nothing else, got {:?}",
            actions[1]
        );
        // Each other run read its own, and not this run's notch.
        for other in [0, 2] {
            assert_eq!(wheel(&actions[other]), [15.0; 3], "{:?}", actions[other]);
            let typed = actions[other].iter().any(|action| {
                matches!(action, Action::Key { code, pressed: true, .. } if *code == u32::from(KEY_A))
            });
            assert!(typed, "{:?}", actions[other]);
        }
    }
    /// A swipe libinput read: its fingers, and how far it went (dx, dy).
    type Swiped = (u32, f64, f64);
    /// A pinch libinput read: its fingers, and its last scale.
    type Pinched = (u32, f64);

    /// The swipes and pinches libinput reads from the virtual touchpad, each as it began and
    /// went.
    fn gestures(actions: &[Action]) -> (Vec<Swiped>, Vec<Pinched>) {
        let (mut swipes, mut pinches) = (Vec::new(), Vec::new());
        for action in actions {
            match *action {
                Action::Swipe {
                    phase: GesturePhase::Begin,
                    fingers,
                    ..
                } => swipes.push((fingers, 0.0, 0.0)),
                Action::Swipe { dx, dy, .. } => {
                    if let Some(last) = swipes.last_mut() {
                        last.1 += dx;
                        last.2 += dy;
                    }
                }
                Action::Pinch {
                    phase: GesturePhase::Begin,
                    fingers,
                    ..
                } => pinches.push((fingers, 1.0)),
                Action::Pinch { scale, .. } => {
                    if let Some(last) = pinches.last_mut() {
                        last.1 = scale;
                    }
                }
                _ => {}
            }
        }
        (swipes, pinches)
    }

    #[test]
    #[ignore]
    fn libinput_counts_the_virtual_touchpads_swipes_and_pinches_of_three_and_four_fingers() {
        // The scenarios of More Gestures (docs/adr/0046-interactive-gestures.mdx, amended 8
        // November 2026) swipe and pinch through libinput: it must see three and four fingers, a
        // swipe as far as the step asks, and the thumb and three fingers as a pinch.
        let (mut devices, input) = start();
        let mut inputs = [input];
        let mut actions: [Vec<Action>; 1] = Default::default();
        devices
            .swipe(3, (0, -360), 24, 60, || read(&mut inputs, &mut actions))
            .unwrap();
        devices
            .swipe(4, (-400, 0), 24, 60, || read(&mut inputs, &mut actions))
            .unwrap();
        devices
            .pinch(4, 50, 0, || read(&mut inputs, &mut actions))
            .unwrap();
        devices
            .pinch(2, 150, 0, || read(&mut inputs, &mut actions))
            .unwrap();
        let settled = Instant::now() + Duration::from_millis(300);
        while Instant::now() < settled {
            std::thread::sleep(Duration::from_millis(10));
            read(&mut inputs, &mut actions);
        }
        let (swipes, pinches) = gestures(&actions[0]);
        // Each swipe its fingers, and most of its way: libinput keeps the first millimetres to
        // tell a swipe from a pinch (and a pinch's first spread likewise, below).
        assert_eq!(swipes.len(), 2, "{:?}", actions[0]);
        let (fingers, dx, dy) = swipes[0];
        assert_eq!(fingers, 3);
        assert!(
            dx.abs() < 1.0 && (-360.0..-240.0).contains(&dy),
            "{:?}",
            swipes[0]
        );
        let (fingers, dx, dy) = swipes[1];
        assert_eq!(fingers, 4);
        assert!(
            dy.abs() < 1.0 && (-400.0..-270.0).contains(&dx),
            "{:?}",
            swipes[1]
        );
        // The thumb and three fingers pinched in, then two spread.
        assert_eq!(pinches.len(), 2, "{:?}", actions[0]);
        assert_eq!(pinches[0].0, 4);
        assert!((0.45..0.85).contains(&pinches[0].1), "{:?}", pinches[0]);
        assert_eq!(pinches[1].0, 2);
        assert!((1.15..1.55).contains(&pinches[1].1), "{:?}", pinches[1]);
    }
}
