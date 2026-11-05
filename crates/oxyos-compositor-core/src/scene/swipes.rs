//! The touchpad's swipes (docs/adr/0044-app-expose.mdx): three or four fingers swiped up open
//! Mission Control, swiped down App Exposé, as a Mac's trackpad does (System Settings › Trackpad
//! › More Gestures). They are the system's, as Super's keys are: no application is told of them,
//! whichever surface is under the pointer. Two fingers are a scroll and a pinch, which go to the
//! surface under the pointer (`pointer_scroll`, `pointer_pinch`), and libinput never reports them
//! as a swipe.
//!
//! The shell decides what each does with what it shows, as it does for Super+↑ and Super+↓ (the
//! same shortcuts, `SystemShortcut::MissionControl` and `AppExpose`): one opens what is closed,
//! and the other way closes it again, so the same gesture reversed goes back.

use super::{DismissReason, GesturePhase, Scene};
use oxyos_protocol::SystemShortcut;

/// How far the fingers go along one axis before a swipe counts, in 1/256 of a logical pixel (the
/// protocol's unit): 40 logical pixels, about a centimetre on a touchpad, which libinput reports
/// unaccelerated past its own threshold. A Mac's swipe acts at a similar distance.
pub const SWIPE_DISTANCE: i32 = 40 * 256;

/// A swipe going on: how far its fingers have gone, and whether it acted (once a swipe).
#[derive(Debug, Default)]
pub(super) struct Swipe {
    going: bool,
    moved: (i64, i64),
    acted: bool,
}

impl Scene {
    /// A touchpad swipe of `fingers` (three or more; libinput reports a swipe for no fewer), each
    /// step moving them by (`dx_256`, `dy_256`) in 1/256 of a logical pixel. Once its fingers have
    /// gone `SWIPE_DISTANCE` up or down, further than across, the shell showing the pointer's
    /// display is told Mission Control (up) or App Exposé (down), once a swipe, a menu open
    /// closing first. Across, it does nothing (switching desktops is not yet a swipe). It reaches no
    /// application, and nothing while the session is locked or the screenshot interface is open.
    pub fn pointer_swipe(&mut self, phase: GesturePhase, fingers: u8, dx_256: i32, dy_256: i32) {
        self.not_super_alone();
        self.user_input();
        match phase {
            GesturePhase::Begin => {
                self.swipe = Swipe {
                    going: fingers >= 3,
                    ..Swipe::default()
                };
            }
            GesturePhase::Update => {}
            GesturePhase::End | GesturePhase::Cancel => {
                self.swipe = Swipe::default();
                return;
            }
        }
        if !self.swipe.going || self.swipe.acted {
            return;
        }
        self.swipe.moved.0 += i64::from(dx_256);
        self.swipe.moved.1 += i64::from(dy_256);
        let (across, along) = self.swipe.moved;
        if along.abs() < i64::from(SWIPE_DISTANCE) || along.abs() <= across.abs() {
            return;
        }
        self.swipe.acted = true;
        if self.locked || self.capturing() {
            return;
        }
        let shortcut = if along < 0 {
            SystemShortcut::MissionControl
        } else {
            SystemShortcut::AppExpose
        };
        // On the display under the pointer, where the fingers are working, else as a key's.
        let output = self
            .output_at(i64::from(self.pointer.0), i64::from(self.pointer.1))
            .map(|output| output.id)
            .unwrap_or_else(|| self.shortcut_output());
        let Some(client) = self.shortcut_shell(output) else {
            return;
        };
        self.dismiss_grab(DismissReason::OutsidePress);
        self.tell_shortcut(client, output, shortcut);
    }
}
