//! The touchpad's swipes (docs/adr/0045-app-expose.mdx, docs/adr/0046-interactive-gestures.mdx):
//! three or four fingers swiped up open Mission Control, swiped down App Exposé, and swiped across
//! go to the desktop beside, as a Mac's trackpad does (System Settings › Trackpad › More Gestures).
//! They follow the fingers, as a Mac's do: the windows go toward their places in Mission Control or
//! App Exposé, or the desktops slide, as far as the fingers have gone, and when they lift the
//! motion goes on to where the swipe was going or back, keeping the fingers' speed. They are the
//! system's, as Super's keys are: no application is told of them, whichever surface is under the
//! pointer. Two fingers are a scroll and a pinch, which go to the surface under the pointer
//! (`pointer_scroll`, `pointer_pinch`), and libinput never reports them as a swipe.
//!
//! The scene follows the fingers and decides; the compositor draws (`swipe_drive`): how far a
//! swipe has gone is `SwipeDrive`'s, and once the fingers lift, where its motion goes and how fast
//! it goes there. The shell shows Mission Control and App Exposé (`SystemShortcut`, the swipe's
//! `Open` as its fingers begin to drive it and `Close` if they let it go back), and says which of
//! its surfaces they are (`SetGestureOverlay`), which the compositor then draws as far as the
//! swipe has gone. A swipe back over what is open closes it the same way: the shell is told only
//! once the fingers have let it go past closing. Across, the scene shows the desktop beside the
//! active one as the fingers bring it (`show_beside`), and activates it once they let it go there.
//!
//! With Reduce Motion nothing follows the fingers: once they lift, a swipe that would have gone
//! through opens, closes or changes desktop at once, and the compositor crossfades it.
//!
//! Which fingers make which gesture is the person's (Settings › Trackpad › More Gestures, the
//! 8 November 2026 amendment of docs/adr/0046-interactive-gestures.mdx, `TrackpadGestures`):
//! each swipe three fingers (and four, as on a Mac), four alone, or off; and the thumb and three
//! fingers pinched open Launchpad, and spread close it, the same way as the swipes: the shell told
//! `Search` (the launcher, which Super alone opens) to open or close it, the compositor drawing
//! Launchpad's surfaces as far as the fingers have pinched (`GestureOverlay::Launchpad`).

use super::{DismissReason, GesturePhase, OutputId, Role, Scene, SurfaceKey};
use oxyos_protocol::{GestureOverlay, ShortcutIntent, SystemShortcut};
use std::collections::VecDeque;

/// How far the fingers go along a swipe (logical pixels, libinput's unaccelerated motion) to take
/// Mission Control or App Exposé from closed to open: GNOME Shell's touchpad swipes' base height,
/// measured on the same motion, about a third of a touchpad's height.
pub const OVERLAY_DISTANCE: f64 = 300.0;
/// How far they go across to bring the desktop beside all the way in: GNOME Shell's base width.
pub const SPACE_DISTANCE: f64 = 400.0;
/// How far the fingers go before the swipe is up, down or across, whichever they have gone
/// further: from there on it is only that. The way is counted from where they began, so nothing
/// jumps.
pub const AXIS_LOCK: f64 = 8.0;
/// The fingers' speed, along the swipe, past which a release goes where they were going however
/// far they went (a flick), in logical pixels a second: GNOME Shell's touchpad threshold (0.3 a
/// millisecond). Slower, it goes to whichever end is nearer.
pub const FLICK: f64 = 300.0;
/// The fingers' speed is their motion over the last tenth of a second; motion older than that,
/// or fingers still for longer than `STOPPED_NS` before they lift, counts for nothing (Android's
/// VelocityTracker's horizon and stop, which the Bloom host's pans follow too).
const HORIZON_NS: u64 = 100_000_000;
const STOPPED_NS: u64 = 40_000_000;
/// A swipe past the first or the last desktop pulls against it, as a Mac's spaces bounce: the
/// desktops go at most this share of the width past it, approaching it as UIKit's rubber band does
/// (`RUBBER` its coefficient).
const PAST_THE_END: f64 = 0.15;
const RUBBER: f64 = 0.55;
/// How far the fingers pinch to take Launchpad from closed to open: the natural logarithm of how
/// much closer they come, half (to about 60 % of their spread when they began; spread back to
/// about 165 % to close it), counted on a logarithm so that pinching in and spreading out are the
/// same way along.
pub const PINCH_DISTANCE: f64 = 0.5;
/// How far they pinch, on the same logarithm, before the pinch is in or out (about 5 %).
pub const PINCH_LOCK: f64 = 0.05;
/// A pinch's flick, in its progress a second: the same share of the way a second as a swipe's
/// (`FLICK` over `OVERLAY_DISTANCE`).
pub const PINCH_FLICK: f64 = FLICK / OVERLAY_DISTANCE;

/// The trackpad's gestures as the person chose them (Settings › Trackpad › More Gestures,
/// docs/adr/0046-interactive-gestures.mdx, amended 8 November 2026): for each swipe, the fewest
/// fingers that make it, or none (off). Three takes four fingers too, as a Mac's trackpad does;
/// four leaves three to nothing. `launchpad`: three or more fingers pinched (the thumb and three,
/// as libinput counts them: four, or three when it takes the thumb for resting) are Launchpad's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrackpadGestures {
    /// Mission Control, swiped up.
    pub mission_control: Option<u8>,
    /// App Exposé, swiped down.
    pub app_expose: Option<u8>,
    /// The desktops, swiped across.
    pub desktops: Option<u8>,
    pub launchpad: bool,
}

impl Default for TrackpadGestures {
    /// As a Mac's trackpad ships them, App Exposé's swipe kept on as OxyOS has had it (ADR 0045).
    fn default() -> Self {
        Self {
            mission_control: Some(3),
            app_expose: Some(3),
            desktops: Some(3),
            launchpad: true,
        }
    }
}

/// Whether `fingers` make the gesture whose fewest are `fewest` (none: it is off).
fn takes(fewest: Option<u8>, fingers: u8) -> bool {
    fewest.is_some_and(|fewest| fingers >= fewest)
}

/// How far a swipe has gone: from `base` (where what it drives was when the fingers began),
/// `travelled` logical pixels toward 1 over `distance`, and the fingers' recent motion, by when
/// their device registered it, for their speed.
#[derive(Clone, Debug, PartialEq)]
pub struct Tracker {
    base: f64,
    distance: f64,
    travelled: f64,
    samples: VecDeque<(u64, f64)>,
}

impl Tracker {
    pub fn new(base: f64, distance: f64, time_ns: u64) -> Self {
        Self {
            base,
            distance,
            travelled: 0.0,
            samples: VecDeque::from([(time_ns, 0.0)]),
        }
    }

    /// The fingers went `along` logical pixels toward 1 (negative: back), at `time_ns`.
    pub fn moved(&mut self, along: f64, time_ns: u64) {
        self.travelled += along;
        self.samples.push_back((time_ns, self.travelled));
        while self
            .samples
            .front()
            .is_some_and(|&(at, _)| time_ns.saturating_sub(at) > HORIZON_NS)
        {
            self.samples.pop_front();
        }
    }

    /// Where the fingers have taken it, unbounded: `base`, plus the way they went over
    /// `distance`. A finger's pixel is a pixel of progress over `distance`, whatever the display.
    pub fn progress(&self) -> f64 {
        self.base + self.travelled / self.distance
    }

    /// The fingers' speed at `time_ns` (when they lift), in progress a second: their motion over
    /// the last tenth of a second, from its first sample to its last; nothing if they had stopped.
    pub fn velocity(&self, time_ns: u64) -> f64 {
        let (Some(&(first, from)), Some(&(last, to))) = (self.samples.front(), self.samples.back())
        else {
            return 0.0;
        };
        if time_ns.saturating_sub(last) > STOPPED_NS || last <= first {
            return 0.0;
        }
        let seconds = (last - first) as f64 / 1e9;
        (to - from) / self.distance / seconds
    }
}

/// Where a swipe let go at `progress`, moving at `velocity` (both in its progress, a second),
/// goes: within one of where it began (`from`), between `low` and `high`, a whole step. A flick
/// (at least `flick` a second) goes on the way it was going, however short; slower, it goes to the
/// nearer step, so short of halfway it goes back.
pub fn release_target(progress: f64, velocity: f64, flick: f64, low: f64, high: f64) -> f64 {
    let to = if velocity.abs() >= flick {
        if velocity > 0.0 {
            progress.floor() + 1.0
        } else {
            progress.ceil() - 1.0
        }
    } else {
        progress.round()
    };
    to.clamp(low, high)
}

/// How far past an end (`past`, in desktops) the desktops are drawn: less and less as the fingers
/// go on, never `PAST_THE_END`.
pub fn rubber_band(past: f64) -> f64 {
    let pulled = (1.0 - 1.0 / (past.abs() * RUBBER / PAST_THE_END + 1.0)) * PAST_THE_END;
    pulled.copysign(past)
}

/// Where the fingers let a swipe go, and how fast it was going: the compositor's spring takes it
/// from where it was drawn to `to`, starting at `velocity` (progress a second).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwipeRelease {
    pub to: f64,
    pub velocity: f64,
}

/// What a swipe drives, for the compositor to draw (`Scene::swipe_drive`), from the fingers'
/// first motion along it until the next swipe. `gesture` tells one swipe from the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SwipeDrive {
    /// Mission Control or App Exposé, `overlay`, of the shell `client` on `output`: how open
    /// (0 closed, 1 open) the fingers have it.
    Overlay {
        gesture: u64,
        client: super::ClientId,
        output: OutputId,
        overlay: GestureOverlay,
        openness: f64,
        release: Option<SwipeRelease>,
    },
    /// The desktops, slid to `position` (in desktops: the active one's number when nothing is
    /// slid; 1.25 a quarter of the way from the second desktop to the third).
    Spaces {
        gesture: u64,
        position: f64,
        release: Option<SwipeRelease>,
    },
}

impl SwipeDrive {
    pub fn gesture(&self) -> u64 {
        match *self {
            SwipeDrive::Overlay { gesture, .. } | SwipeDrive::Spaces { gesture, .. } => gesture,
        }
    }

    pub fn release(&self) -> Option<SwipeRelease> {
        match *self {
            SwipeDrive::Overlay { release, .. } | SwipeDrive::Spaces { release, .. } => release,
        }
    }
}

/// Where the compositor draws what swipes drive now (`Scene::swipe_from`): a swipe beginning while
/// the last one's motion goes on begins from there, as a Mac's does when the fingers catch it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SwipeRest {
    /// How open Mission Control or App Exposé is drawn, while a swipe's motion takes it.
    pub openness: Option<(GestureOverlay, f64)>,
    /// Where the desktops are slid, while a swipe's motion takes them.
    pub position: Option<f64>,
}

/// What a swipe does once its fingers chose its axis.
#[derive(Clone, Debug)]
enum Act {
    /// Opening (`opening`) or closing Mission Control or App Exposé: its openness, 0 to 1.
    Overlay {
        client: super::ClientId,
        output: OutputId,
        overlay: GestureOverlay,
        opening: bool,
        tracker: Tracker,
    },
    /// Sliding the desktops: their position, from the active one, `from`.
    Spaces { from: u32, tracker: Tracker },
    /// Nothing: up over Mission Control, down over App Exposé, across over either, no shell.
    Nothing,
}

/// A swipe going on: its fingers, how far they have gone, and what it does.
#[derive(Debug, Default)]
pub(super) struct Swipe {
    going: bool,
    moved: (f64, f64),
    act: Option<Act>,
    /// Whether the compositor draws it as the fingers go (not with Reduce Motion).
    follows: bool,
    /// Its number, once its axis is chosen (`SwipeDrive`'s `gesture`).
    gesture: u64,
    /// When the fingers began, by their device's time: the way they went before the axis was
    /// chosen took them from then, not no time at all.
    began: u64,
    /// How many fingers swipe.
    fingers: u8,
    /// A pinch's (Launchpad's) spread at its last step, over its spread when it began; none for a
    /// swipe.
    pinched: Option<f64>,
}

impl Scene {
    /// Where the compositor draws what swipes drive now, for a swipe that begins while the last
    /// one's motion goes on (`SwipeRest`).
    pub fn swipe_from(&mut self, rest: SwipeRest) {
        self.swipe_rest = rest;
    }

    /// What the latest swipe drives (`SwipeDrive`): none until the fingers of one chose its axis,
    /// and none for one that does nothing or that the compositor does not draw (Reduce Motion).
    pub fn swipe_drive(&self) -> Option<SwipeDrive> {
        self.swipe_drive
    }

    /// The workspace whose windows the compositor draws beside the active one's while a swipe
    /// across slides them (`frame_list` has them; nothing else: they take no input and have no
    /// focus), or none.
    pub fn show_beside(&mut self, workspace: Option<u32>) {
        self.beside = workspace
            .filter(|&index| index < self.workspaces.count && index != self.workspaces.active);
    }

    /// The trackpad's gestures the person chose (Settings › Trackpad › More Gestures): from the
    /// next swipe or pinch on.
    pub fn set_trackpad_gestures(&mut self, gestures: TrackpadGestures) {
        self.trackpad_gestures = gestures;
    }

    pub fn trackpad_gestures(&self) -> TrackpadGestures {
        self.trackpad_gestures
    }

    /// Which of the shell's views a swipe drives `surface` is (`SetGestureOverlay`), if any.
    pub fn gesture_overlay(&self, surface: SurfaceKey) -> GestureOverlay {
        self.window_views
            .get(&surface)
            .map_or(GestureOverlay::None, |views| views.gesture)
    }

    /// Mission Control, App Exposé or Launchpad, those of them `which` says, if the shell `client`
    /// shows one on `output`: one of its shell surfaces there that says it is
    /// (`SetGestureOverlay`) is shown.
    fn gesture_overlay_shown(
        &self,
        client: super::ClientId,
        output: OutputId,
        which: impl Fn(GestureOverlay) -> bool,
    ) -> Option<GestureOverlay> {
        self.window_views
            .iter()
            .filter(|(key, views)| {
                key.client == client
                    && views.gesture != GestureOverlay::None
                    && which(views.gesture)
            })
            .filter(|(key, _)| {
                self.nodes.get(key).is_some_and(
                    |node| matches!(node.role, Role::Shell { output: on, .. } if on == output),
                ) && self.visible(**key)
            })
            .map(|(_, views)| views.gesture)
            .next()
    }

    /// Whether the shell `client` draws `overlay` on `output` whole, in the frame being composed
    /// (`frame_list`'s): it has a surface there said to be it (`SetGestureOverlay`), and every one
    /// of them not hidden has its first frame drawn. Mission Control and App Exposé may be several
    /// surfaces (App Exposé's backdrop, and the one its windows are moved into), each drawn when
    /// the shell gets to it: a swipe's motion begins once they all are, so that they set off
    /// together and no window starts its flight from part of the way.
    pub fn gesture_overlay_drawn(
        &self,
        client: super::ClientId,
        output: OutputId,
        overlay: GestureOverlay,
    ) -> bool {
        let mut surfaces = self
            .window_views
            .iter()
            .filter(|(key, views)| key.client == client && views.gesture == overlay)
            .filter(|(key, _)| {
                self.nodes.get(key).is_some_and(|node| {
                    matches!(node.role, Role::Shell { output: on, .. } if on == output)
                        && !node.hidden
                        && !node.dismissed
                })
            })
            .peekable();
        surfaces.peek().is_some() && surfaces.all(|(key, _)| self.visible_in(*key, true))
    }

    fn reduce_motion(&self) -> bool {
        self.preferences
            .drawing
            .is_some_and(|drawing| drawing.reduce_motion)
    }

    /// A touchpad swipe of `fingers` (three or more; libinput reports a swipe for no fewer), each
    /// step moving them by (`dx_256`, `dy_256`) in 1/256 of a logical pixel, at the input's time
    /// (`set_input_time`). Once they have gone `AXIS_LOCK` the swipe is vertical or across for
    /// good. Up opens Mission Control, down App Exposé, the shell showing the pointer's display
    /// told at once (`Open`) and the compositor drawing them as far as the fingers go; over what
    /// is open, the other way closes it as they go. Across slides the desktops. When the fingers
    /// lift, it goes on to where it was going, or back (`release_target`): the shell is told to
    /// close what the fingers let go back, or what they closed (`Close`); a desktop they brought in
    /// is activated. Cancelled (libinput's: another finger), it goes back. It reaches no
    /// application, and nothing while the session is locked or the screenshot interface is open.
    pub fn pointer_swipe(&mut self, phase: GesturePhase, fingers: u8, dx_256: i32, dy_256: i32) {
        self.not_super_alone();
        self.user_input();
        let time = self.input_time_ns;
        match phase {
            GesturePhase::Begin => {
                self.swipe = Swipe {
                    going: fingers >= 3 && !self.locked && !self.capturing(),
                    follows: !self.reduce_motion(),
                    began: time,
                    fingers,
                    ..Swipe::default()
                };
                return;
            }
            GesturePhase::Update => {}
            GesturePhase::End | GesturePhase::Cancel => {
                let swipe = std::mem::take(&mut self.swipe);
                if swipe.going && !self.locked {
                    self.release_swipe(swipe, phase == GesturePhase::Cancel, time);
                }
                return;
            }
        }
        if !self.swipe.going || self.locked {
            return;
        }
        let (dx, dy) = (f64::from(dx_256) / 256.0, f64::from(dy_256) / 256.0);
        self.swipe.moved.0 += dx;
        self.swipe.moved.1 += dy;
        if self.swipe.act.is_none() {
            let (across, along) = self.swipe.moved;
            if across.hypot(along) < AXIS_LOCK {
                return;
            }
            let (began, fingers) = (self.swipe.began, self.swipe.fingers);
            // Up, down or across, if the person's choice for it takes these fingers.
            let act = if along.abs() >= across.abs() {
                self.overlay_act(along < 0.0, began, fingers)
            } else if takes(self.trackpad_gestures.desktops, fingers) {
                self.spaces_act(began)
            } else {
                Act::Nothing
            };
            self.swipe_gestures += 1;
            self.swipe.gesture = self.swipe_gestures;
            // What went before the axis was chosen counts, from when the fingers began: nothing
            // jumps, and their speed is that way over the time it took.
            let act = match act {
                Act::Overlay {
                    client,
                    output,
                    overlay,
                    opening,
                    mut tracker,
                } => {
                    tracker.moved(toward_open(overlay, (across, along)), time);
                    Act::Overlay {
                        client,
                        output,
                        overlay,
                        opening,
                        tracker,
                    }
                }
                Act::Spaces { from, mut tracker } => {
                    tracker.moved(-across, time);
                    Act::Spaces { from, tracker }
                }
                Act::Nothing => Act::Nothing,
            };
            self.swipe.act = Some(act);
        } else {
            match self.swipe.act.as_mut() {
                Some(Act::Overlay {
                    overlay, tracker, ..
                }) => tracker.moved(toward_open(*overlay, (dx, dy)), time),
                Some(Act::Spaces { tracker, .. }) => tracker.moved(-dx, time),
                _ => {}
            }
        }
        self.drive_swipe(None);
    }

    /// The swipe's `fingers` chose up (`up`) or down: what that does on the display under the
    /// pointer, the shell showing it told to open it if it opens; its way counted from `began`.
    /// Nothing unless the person's choice for what it opens or closes takes those fingers.
    fn overlay_act(&mut self, up: bool, began: u64, fingers: u8) -> Act {
        let output = self
            .output_at(i64::from(self.pointer.0), i64::from(self.pointer.1))
            .map(|output| output.id)
            .unwrap_or_else(|| self.shortcut_output());
        let Some(client) = self.shortcut_shell(output) else {
            return Act::Nothing;
        };
        // Caught while a swipe's motion takes Mission Control or App Exposé (`swipe_from`): the
        // fingers drive it either way from where it is drawn, open or closing. Launchpad, a
        // pinch's, is none of a swipe's: up over it opens Mission Control, as before it was told.
        let swiped = |overlay| overlay != GestureOverlay::Launchpad;
        let moving = self
            .swipe_rest
            .openness
            .map(|(overlay, _)| overlay)
            .filter(|overlay| swiped(*overlay));
        let shown = moving.or_else(|| self.gesture_overlay_shown(client, output, swiped));
        let (overlay, opening) = match (shown, up) {
            (Some(overlay @ GestureOverlay::MissionControl), true)
            | (Some(overlay @ GestureOverlay::AppExpose), false)
                if moving.is_some() =>
            {
                (overlay, true)
            }
            (None, true) => (GestureOverlay::MissionControl, true),
            (None, false) => (GestureOverlay::AppExpose, true),
            // The swipe back closes what is open; the same way again does nothing, as a Mac's.
            (Some(GestureOverlay::MissionControl), false) => {
                (GestureOverlay::MissionControl, false)
            }
            (Some(GestureOverlay::AppExpose), true) => (GestureOverlay::AppExpose, false),
            _ => return Act::Nothing,
        };
        // Its own swipe, opening or back, as the person chose its fingers: Mission Control's up and
        // back down, App Exposé's down and back up.
        let chosen = match overlay {
            GestureOverlay::MissionControl => self.trackpad_gestures.mission_control,
            GestureOverlay::AppExpose => self.trackpad_gestures.app_expose,
            _ => None,
        };
        if !takes(chosen, fingers) {
            return Act::Nothing;
        }
        // From where the compositor draws it if a swipe's motion still takes it, as the fingers
        // catch it; else from closed, or open.
        let base = match self.swipe_rest.openness {
            Some((drawn, openness)) if drawn == overlay => openness,
            _ if opening => 0.0,
            _ => 1.0,
        };
        self.dismiss_grab(DismissReason::OutsidePress);
        if opening && self.swipe.follows {
            self.tell_shortcut_intent(client, output, shortcut_of(overlay), ShortcutIntent::Open);
        }
        Act::Overlay {
            client,
            output,
            overlay,
            opening,
            tracker: Tracker::new(base, OVERLAY_DISTANCE, began),
        }
    }

    /// The swipe's fingers chose across: the desktops slide, unless Mission Control or App Exposé
    /// is open on the display under the pointer; their way counted from `began`.
    fn spaces_act(&mut self, began: u64) -> Act {
        let output = self
            .output_at(i64::from(self.pointer.0), i64::from(self.pointer.1))
            .map(|output| output.id)
            .unwrap_or_else(|| self.shortcut_output());
        let shown = self.shortcut_shell(output).and_then(|client| {
            self.gesture_overlay_shown(client, output, |overlay| {
                overlay != GestureOverlay::Launchpad
            })
        });
        if shown.is_some() {
            return Act::Nothing;
        }
        self.dismiss_grab(DismissReason::OutsidePress);
        let from = self.workspaces.active;
        let base = self.swipe_rest.position.unwrap_or(f64::from(from));
        Act::Spaces {
            from,
            tracker: Tracker::new(base, SPACE_DISTANCE, began),
        }
    }

    /// What the swipe drives now, for the compositor, with where it goes if the fingers lifted.
    fn drive_swipe(&mut self, release: Option<SwipeRelease>) {
        if !self.swipe.follows {
            self.swipe_drive = None;
            return;
        }
        let gesture = self.swipe.gesture;
        self.swipe_drive = match &self.swipe.act {
            Some(Act::Overlay {
                client,
                output,
                overlay,
                tracker,
                ..
            }) => Some(SwipeDrive::Overlay {
                gesture,
                client: *client,
                output: *output,
                overlay: *overlay,
                openness: tracker.progress().clamp(0.0, 1.0),
                release,
            }),
            Some(Act::Spaces { from, tracker }) => Some(SwipeDrive::Spaces {
                gesture,
                position: self.slid(*from, tracker.progress()),
                release,
            }),
            Some(Act::Nothing) | None => None,
        };
    }

    /// Where the desktops are drawn with the fingers at `position`: at most one desktop from
    /// `from`, and pulling against the first and the last.
    fn slid(&self, from: u32, position: f64) -> f64 {
        let from = f64::from(from);
        let position = position.clamp(from - 1.0, from + 1.0);
        let last = f64::from(self.workspaces.count.saturating_sub(1));
        if position < 0.0 {
            rubber_band(position)
        } else if position > last {
            last + rubber_band(position - last)
        } else {
            position
        }
    }

    /// The fingers lifted (or libinput cancelled the swipe, `cancelled`) at `time`: where it goes.
    fn release_swipe(&mut self, swipe: Swipe, cancelled: bool, time: u64) {
        let follows = swipe.follows;
        self.swipe = swipe;
        let release = match self.swipe.act.clone() {
            Some(Act::Overlay {
                client,
                output,
                overlay,
                opening,
                tracker,
            }) => {
                let openness = tracker.progress().clamp(0.0, 1.0);
                let velocity = tracker.velocity(time);
                let flick = if overlay == GestureOverlay::Launchpad {
                    PINCH_FLICK
                } else {
                    FLICK / OVERLAY_DISTANCE
                };
                let open = if cancelled {
                    !opening
                } else {
                    release_target(openness, velocity, flick, 0.0, 1.0) == 1.0
                };
                let shortcut = shortcut_of(overlay);
                match (opening, open) {
                    // Let go back: the shell, told to open it, closes it as the compositor draws it
                    // closing (with Reduce Motion it was never told).
                    (true, false) if follows => {
                        self.tell_shortcut_intent(client, output, shortcut, ShortcutIntent::Close)
                    }
                    (true, true) if !follows => {
                        self.tell_shortcut_intent(client, output, shortcut, ShortcutIntent::Open)
                    }
                    (false, false) => {
                        self.tell_shortcut_intent(client, output, shortcut, ShortcutIntent::Close)
                    }
                    _ => {}
                }
                Some(SwipeRelease {
                    to: if open { 1.0 } else { 0.0 },
                    velocity: if cancelled { 0.0 } else { velocity },
                })
            }
            Some(Act::Spaces { from, tracker }) => {
                let position = self.slid(from, tracker.progress());
                let velocity = if cancelled {
                    0.0
                } else {
                    tracker.velocity(time)
                };
                let last = f64::from(self.workspaces.count.saturating_sub(1));
                let low = (f64::from(from) - 1.0).max(0.0);
                let high = (f64::from(from) + 1.0).min(last);
                let to = if cancelled {
                    f64::from(from)
                } else {
                    release_target(position, velocity, FLICK / SPACE_DISTANCE, low, high)
                };
                let index = to as u32;
                if index != from {
                    if follows {
                        // The compositor's motion takes the desktops there from where the fingers
                        // left them: no slide of their own.
                        self.activate_workspace_as(index, false);
                    } else {
                        self.activate_workspace(index);
                    }
                }
                Some(SwipeRelease { to, velocity })
            }
            Some(Act::Nothing) | None => None,
        };
        if let Some(release) = release {
            self.drive_swipe(Some(release));
        }
        self.swipe = Swipe::default();
    }
}

impl Scene {
    /// Whether a pinch of `fingers` beginning now is the system's, Launchpad's: three or more
    /// (the thumb and three fingers, as libinput counts them), the person's choice on, the
    /// session unlocked and no screenshot interface open. Otherwise it is the application's, as
    /// two fingers' always are.
    pub(super) fn pinch_is_launchpads(&self, fingers: u8) -> bool {
        fingers >= 3 && self.trackpad_gestures.launchpad && !self.locked && !self.capturing()
    }

    /// Whether the pinch going on is Launchpad's.
    pub(super) fn pinching_launchpad(&self) -> bool {
        self.swipe.pinched.is_some()
    }

    /// The thumb and three fingers pinching (`pinch_is_launchpads`), their spread `scale` over
    /// their spread when they began: in, they open Launchpad on the display under the pointer,
    /// the shell showing it told at once (`Search`, `Open`) and the compositor drawing it as far
    /// as they have pinched; spread over Launchpad, they close it as far as they go. Once they have
    /// pinched `PINCH_LOCK` the pinch is in or out for good, and what it does is decided; spread
    /// over nothing (a Mac's Show Desktop, which OxyOS does not have) or pinched in over Launchpad,
    /// it does nothing. Lifted, it goes on or back as a swipe does (`release_target`). It reaches
    /// no application.
    pub(super) fn launchpad_pinch(&mut self, phase: GesturePhase, fingers: u8, scale: f64) {
        self.not_super_alone();
        let time = self.input_time_ns;
        match phase {
            GesturePhase::Begin => {
                self.swipe = Swipe {
                    going: !self.locked && !self.capturing(),
                    follows: !self.reduce_motion(),
                    began: time,
                    fingers,
                    pinched: Some(1.0),
                    ..Swipe::default()
                };
                return;
            }
            GesturePhase::Update => {}
            GesturePhase::End | GesturePhase::Cancel => {
                let swipe = std::mem::take(&mut self.swipe);
                if swipe.going && !self.locked {
                    self.release_swipe(swipe, phase == GesturePhase::Cancel, time);
                }
                return;
            }
        }
        if !self.swipe.going || self.locked || scale <= 0.0 {
            return;
        }
        // How much closer the fingers came since their last step, on a logarithm: in is toward
        // open, out toward closed, the same way along either way.
        let last = self.swipe.pinched.unwrap_or(1.0);
        let inward = (last / scale).ln();
        self.swipe.pinched = Some(scale);
        self.swipe.moved.1 += inward;
        if self.swipe.act.is_none() {
            let pinched = self.swipe.moved.1;
            if pinched.abs() < PINCH_LOCK {
                return;
            }
            let act = self.launchpad_act(pinched > 0.0, self.swipe.began);
            self.swipe_gestures += 1;
            self.swipe.gesture = self.swipe_gestures;
            // What went before it was in or out counts, from when the fingers began.
            let act = match act {
                Act::Overlay {
                    client,
                    output,
                    overlay,
                    opening,
                    mut tracker,
                } => {
                    tracker.moved(pinched, time);
                    Act::Overlay {
                        client,
                        output,
                        overlay,
                        opening,
                        tracker,
                    }
                }
                act => act,
            };
            self.swipe.act = Some(act);
        } else if let Some(Act::Overlay { tracker, .. }) = self.swipe.act.as_mut() {
            tracker.moved(inward, time);
        }
        self.drive_swipe(None);
    }

    /// The pinch is in (`inward`) or out: what that does on the display under the pointer, the
    /// shell showing it told to open Launchpad if it opens; its way counted from `began`.
    fn launchpad_act(&mut self, inward: bool, began: u64) -> Act {
        let output = self
            .output_at(i64::from(self.pointer.0), i64::from(self.pointer.1))
            .map(|output| output.id)
            .unwrap_or_else(|| self.shortcut_output());
        let Some(client) = self.shortcut_shell(output) else {
            return Act::Nothing;
        };
        let launchpad = |overlay| overlay == GestureOverlay::Launchpad;
        // Caught while a pinch's motion takes it: either way from where it is drawn.
        let moving = self
            .swipe_rest
            .openness
            .filter(|(overlay, _)| launchpad(*overlay));
        let shown = moving.is_some()
            || self
                .gesture_overlay_shown(client, output, launchpad)
                .is_some();
        let opening = match (shown, inward) {
            (true, true) if moving.is_some() => true,
            (false, true) => true,
            (true, false) => false,
            _ => return Act::Nothing,
        };
        let base = match moving {
            Some((_, openness)) => openness,
            None if opening => 0.0,
            None => 1.0,
        };
        self.dismiss_grab(DismissReason::OutsidePress);
        if opening && self.swipe.follows {
            self.tell_shortcut_intent(
                client,
                output,
                shortcut_of(GestureOverlay::Launchpad),
                ShortcutIntent::Open,
            );
        }
        Act::Overlay {
            client,
            output,
            overlay: GestureOverlay::Launchpad,
            opening,
            tracker: Tracker::new(base, PINCH_DISTANCE, began),
        }
    }
}

/// The way along the swipe that opens `overlay`: up for Mission Control, down for App Exposé.
fn toward_open(overlay: GestureOverlay, (_, dy): (f64, f64)) -> f64 {
    match overlay {
        GestureOverlay::AppExpose => dy,
        _ => -dy,
    }
}

/// What the shell is told for `overlay`: Launchpad is the launcher, which Super alone opens
/// (`Search`).
fn shortcut_of(overlay: GestureOverlay) -> SystemShortcut {
    match overlay {
        GestureOverlay::AppExpose => SystemShortcut::AppExpose,
        GestureOverlay::Launchpad => SystemShortcut::Search,
        _ => SystemShortcut::MissionControl,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000_000;

    #[test]
    fn the_fingers_way_is_progress_over_its_distance_from_where_it_began() {
        let mut tracker = Tracker::new(0.0, OVERLAY_DISTANCE, 0);
        for (step, along) in [(1, 75.0), (2, 75.0), (3, 75.0)] {
            tracker.moved(along, step * 16 * MS);
            assert!((tracker.progress() - 0.25 * step as f64).abs() < 1e-9);
        }
        // Back the other way, the same.
        tracker.moved(-150.0, 64 * MS);
        assert!((tracker.progress() - 0.25).abs() < 1e-9);
        // Caught mid-motion (a swipe beginning while the last one's spring has it at 0.4): from
        // there.
        let mut caught = Tracker::new(0.4, OVERLAY_DISTANCE, 0);
        caught.moved(-60.0, 16 * MS);
        assert!((caught.progress() - 0.2).abs() < 1e-9);
        // Across, a desktop's width is SPACE_DISTANCE.
        let mut across = Tracker::new(2.0, SPACE_DISTANCE, 0);
        across.moved(100.0, 16 * MS);
        assert!((across.progress() - 2.25).abs() < 1e-9);
    }

    #[test]
    fn the_fingers_speed_is_their_last_tenth_of_a_second_and_nothing_once_they_stopped() {
        let mut tracker = Tracker::new(0.0, OVERLAY_DISTANCE, 0);
        // 30 pixels each 10 ms: 3000 pixels a second, 10 openness a second.
        for step in 1..=20 {
            tracker.moved(30.0, step * 10 * MS);
        }
        assert!((tracker.velocity(200 * MS) - 10.0).abs() < 1e-9);
        // Only the last tenth counts: slowed to 3 pixels each 10 ms, it is 1 a second.
        for step in 21..=32 {
            tracker.moved(3.0, step * 10 * MS);
        }
        assert!((tracker.velocity(320 * MS) - 1.0).abs() < 1e-9);
        // Still for more than 40 ms before lifting: nothing.
        assert_eq!(tracker.velocity(361 * MS), 0.0);
        // One sample, or all at one time: nothing to tell a speed by.
        let mut once = Tracker::new(0.0, OVERLAY_DISTANCE, 0);
        once.moved(90.0, 0);
        assert_eq!(once.velocity(0), 0.0);
    }

    #[test]
    fn a_release_goes_to_the_nearer_end_unless_it_was_a_flick() {
        let flick = FLICK / OVERLAY_DISTANCE;
        // Slow: below halfway back, past it on.
        assert_eq!(release_target(0.3, 0.2, flick, 0.0, 1.0), 0.0);
        assert_eq!(release_target(0.6, -0.2, flick, 0.0, 1.0), 1.0);
        assert_eq!(release_target(0.5, 0.0, flick, 0.0, 1.0), 1.0);
        // A fast short flick goes through; a flick back from far along goes back.
        assert_eq!(release_target(0.1, flick, flick, 0.0, 1.0), 1.0);
        assert_eq!(release_target(0.9, -flick, flick, 0.0, 1.0), 0.0);
        // Just under the flick's speed, short: back.
        assert_eq!(release_target(0.1, flick * 0.99, flick, 0.0, 1.0), 0.0);
        // Open already and flicked on: it stays open.
        assert_eq!(release_target(1.0, 3.0, flick, 0.0, 1.0), 1.0);
        // Desktops: one at a time, never past the last.
        let flick = FLICK / SPACE_DISTANCE;
        assert_eq!(release_target(1.3, 2.0, flick, 0.0, 2.0), 2.0);
        assert_eq!(release_target(1.3, 0.1, flick, 0.0, 2.0), 1.0);
        assert_eq!(release_target(0.8, -2.0, flick, 0.0, 2.0), 0.0);
        assert_eq!(release_target(2.1, 2.0, flick, 1.0, 2.0), 2.0);
    }

    #[test]
    fn past_the_ends_the_desktops_pull_less_and_less() {
        assert_eq!(rubber_band(0.0), 0.0);
        let pulls: Vec<f64> = (1..=40).map(|i| rubber_band(f64::from(i) * 0.05)).collect();
        assert!(pulls.windows(2).all(|w| w[1] > w[0]));
        assert!(pulls.iter().all(|pull| *pull < PAST_THE_END));
        // Half a desktop past the end is about a tenth of one.
        assert!((rubber_band(0.5) - 0.097).abs() < 0.001);
        assert_eq!(rubber_band(-0.5), -rubber_band(0.5));
    }
}
