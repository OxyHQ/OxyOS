//! Surfaces arriving and going, animated (docs/adr/0011-transitions.mdx): a window grows a
//! little and fades in, a menu fades out, the launcher settles, the lock screen fades; a window
//! minimized goes into the dock and comes back out of it, a switch of workspace slides the
//! windows aside, a notification slides in. What a surface shows is never changed: it is drawn
//! whole, scaled about its centre and moved, at an opacity. A surface going away is drawn from
//! its last frame, which the compositor keeps until its transition ends. A window zoomed (or
//! maximized, tiled, restored) has its frame go from one rectangle to the other, a Mac's zoom
//! (docs/adr/0016-windows.mdx): its content, old or new, stretched into the rectangle between.

use oxyos_compositor_core::{
    Appearance, AppearanceChange as Change, AppearanceKind as Kind, SurfaceKey,
};
use oxyos_protocol::{Format, Rect, Size};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

/// Where a motion moves a surface at its farthest from where it is: nowhere, a share of its own
/// size (thousandths: a notification beside itself), by a distance (a workspace's windows by the
/// output's width), or to a point (a window into the dock). A zoom's is from one rectangle to
/// another (`Between`): the surface is drawn in the rectangle that far between them, whatever its
/// own, so its width and height scale each by their own factor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shift {
    None,
    Own { x_permille: i32, y_permille: i32 },
    By { x: i32, y: i32 },
    To { x: i32, y: i32 },
    Between { from: Rect, to: Rect },
}

/// How a surface arrives or goes: how long; the scale it starts from (arriving) or ends at
/// (going), in thousandths; where it is moved; and whether it fades. A zero duration: at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Motion {
    pub duration: Duration,
    pub scale_permille: u32,
    pub shift: Shift,
    pub fades: bool,
}

const fn motion(ms: u64, scale_permille: u32) -> Motion {
    Motion {
        duration: Duration::from_millis(ms),
        scale_permille,
        shift: Shift::None,
        fades: true,
    }
}

/// What a motion needs to know of the output: its rectangle, and where the dock is (the
/// shell's bottom panel's centre), if there is one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub output: Rect,
    pub dock: Option<(i32, i32)>,
}

/// The ADR's motions for a surface of `kind` arriving or going by `change`.
pub fn motions(kind: Kind, change: Change, place: &Place) -> Motion {
    // Into the dock (its centre; without one, the output's bottom edge's) and out of it, a
    // Mac's minimize: shrinking to a tenth as it moves there, fading at the end.
    let dock = place.dock.unwrap_or((
        place.output.x + place.output.width as i32 / 2,
        place.output.y + place.output.height as i32,
    ));
    let width = place.output.width as i32;
    match (kind, change) {
        (_, Change::Minimized | Change::Restored) => Motion {
            shift: Shift::To {
                x: dock.0,
                y: dock.1,
            },
            ..motion(300, 100)
        },
        // A workspace's windows slide by the output's width, a Mac's spaces: the later
        // workspace's from the right, the one left to the left; they do not fade.
        (_, Change::WorkspaceIn { forward } | Change::WorkspaceOut { forward }) => {
            let leaving = matches!(change, Change::WorkspaceOut { .. });
            Motion {
                shift: Shift::By {
                    x: if forward == leaving { -width } else { width },
                    y: 0,
                },
                fades: false,
                ..motion(300, 1000)
            }
        }
        // A Mac's zoom: the frame goes to its new rectangle, easing out, unfaded (`zoom`).
        (_, Change::Zoomed) => Motion {
            fades: false,
            ..motion(250, 1000)
        },
        (Kind::Window, Change::Arrived) => motion(180, 960),
        (Kind::Window, _) => motion(150, 960),
        // As a Mac's menus: at once, then fading out.
        (Kind::Popup, Change::Arrived) => motion(0, 1000),
        (Kind::Popup, _) => motion(150, 1000),
        (Kind::Overlay, Change::Arrived) => motion(200, 1040),
        (Kind::Overlay, _) => motion(150, 1040),
        (Kind::Panel, Change::Arrived) => motion(250, 1000),
        (Kind::Panel, _) => motion(0, 1000),
        // A notification slides in from beside itself at the right edge, a Mac's, and fades out.
        (Kind::Notification, Change::Arrived) => Motion {
            shift: Shift::Own {
                x_permille: 1100,
                y_permille: 0,
            },
            ..motion(250, 1000)
        },
        (Kind::Notification, _) => motion(200, 1000),
        (Kind::Lock, _) => motion(250, 1000),
        (Kind::Candidates, _) => motion(0, 1000),
    }
}

/// Transitions recorded until `take_seen`.
const MAX_SEEN: usize = 64;
/// How long past its end a zoom keeps the window in its new rectangle, its old frame stretched
/// there, waiting for the client's frame of the new size; then the window is drawn where it is.
const ZOOM_HOLD: Duration = Duration::from_secs(1);

/// A surface's last frame, kept while it goes, with how it was drawn if a window (its corners and
/// shadow go with it).
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub rect: Rect,
    pub extent: Size,
    pub format: Format,
    /// Shared with the frame they were composed from: drawing them again copies nothing.
    pub pixels: Rc<Vec<u8>>,
    pub look: Option<oxyos_compositor_core::WindowLook>,
    /// The windows a shell surface showed live over its pixels (docs/adr/0022-window-views.mdx),
    /// which go with it.
    pub views: Vec<ViewSnapshot>,
}

/// A window a shell surface going showed live: its frame's pixels when the surface began going,
/// and where they were drawn (on the output, as the surface was at `Snapshot::rect`).
#[derive(Clone, Debug)]
pub struct ViewSnapshot {
    pub rect: Rect,
    pub clip: Rect,
    pub radius: u32,
    pub opacity: u8,
    pub extent: Size,
    pub format: Format,
    pub pixels: Rc<Vec<u8>>,
}

struct Running {
    /// Which transition this is, for what is recorded of it (`Seen`).
    id: u64,
    kind: Kind,
    started: Duration,
    motion: Motion,
}

/// A window's frame zooming: where the surface was when it began (`at`), and whether it is now
/// where it is going (`motion`'s `Shift::Between`'s end), its new frame shown, or somewhere else
/// (moved or resized again: the zoom is over).
struct Zoom {
    running: Running,
    at: Rect,
    reached: bool,
    superseded: bool,
}

impl Zoom {
    fn ends(&self, now: Duration) -> bool {
        let elapsed = now.saturating_sub(self.running.started);
        let duration = self.running.motion.duration;
        self.superseded || elapsed >= duration && (self.reached || elapsed >= duration + ZOOM_HOLD)
    }
}

/// What a transition did, as the development compositor reports it: its kind, how the surface
/// came or went, the opacities of the first and last frames drawn in it, how many frames those
/// were, and whether it ran to its end (not interrupted: a window restored as it went, going
/// dropped by the lock).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seen {
    pub kind: Kind,
    pub change: Change,
    pub first: Option<u8>,
    pub last: Option<u8>,
    pub frames: u32,
    pub ended: bool,
}

impl Seen {
    pub fn arriving(&self) -> bool {
        matches!(
            self.change,
            Change::Arrived | Change::Restored | Change::WorkspaceIn { .. }
        )
    }
}

/// Where a surface is in its transition: the opacity (0 to 255) and scale (thousandths) to draw
/// it at now, and how far along its shift it is (thousandths).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub opacity: u8,
    pub scale_permille: u32,
    pub shift: Shift,
    pub shift_permille: u32,
}

impl Frame {
    pub const WHOLE: Frame = Frame {
        opacity: 255,
        scale_permille: 1000,
        shift: Shift::None,
        shift_permille: 0,
    };

    /// `rect` drawn at this scale about its centre, then moved along its shift; with a zoom's
    /// shift, the rectangle `shift_permille` of the way between its two, edge by edge.
    pub fn place(&self, rect: Rect) -> Rect {
        if let Shift::Between { from, to } = self.shift {
            let k = f64::from(self.shift_permille.min(1000)) / 1000.0;
            let along = |a: i64, b: i64| (a as f64 + (b - a) as f64 * k).round() as i64;
            let left = along(i64::from(from.x), i64::from(to.x));
            let top = along(i64::from(from.y), i64::from(to.y));
            let right = along(from.right(), to.right()).max(left + 1);
            let bottom = along(from.bottom(), to.bottom()).max(top + 1);
            return Rect::new(
                left as i32,
                top as i32,
                (right - left) as u32,
                (bottom - top) as u32,
            );
        }
        let scaled = |length: u32| {
            ((u64::from(length) * u64::from(self.scale_permille) + 500) / 1000) as u32
        };
        let (width, height) = if self.scale_permille == 1000 {
            (rect.width, rect.height)
        } else {
            (scaled(rect.width).max(1), scaled(rect.height).max(1))
        };
        let along = |distance: i64| (distance * i64::from(self.shift_permille) / 1000) as i32;
        let centre = (
            i64::from(rect.x) + i64::from(rect.width) / 2,
            i64::from(rect.y) + i64::from(rect.height) / 2,
        );
        let (dx, dy) = match self.shift {
            Shift::None => (0, 0),
            Shift::Own {
                x_permille,
                y_permille,
            } => (
                along(i64::from(rect.width) * i64::from(x_permille) / 1000),
                along(i64::from(rect.height) * i64::from(y_permille) / 1000),
            ),
            Shift::By { x, y } => (along(i64::from(x)), along(i64::from(y))),
            Shift::To { x, y } => (
                along(i64::from(x) - centre.0),
                along(i64::from(y) - centre.1),
            ),
            Shift::Between { .. } => unreachable!("placed above"),
        };
        Rect::new(
            rect.x + (rect.width as i32 - width as i32) / 2 + dx,
            rect.y + (rect.height as i32 - height as i32) / 2 + dy,
            width,
            height,
        )
    }
}

/// How far a transition is at `elapsed`, eased out (cubic), from 0 to 1.
fn progress(elapsed: Duration, duration: Duration) -> f64 {
    if duration.is_zero() {
        return 1.0;
    }
    let t = (elapsed.as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Where a surface is `p` of the way from `from` (the motion's far end) to whole: arriving.
fn arriving_at(motion: &Motion, p: f64) -> Frame {
    if p >= 1.0 {
        return Frame::WHOLE;
    }
    let from = f64::from(motion.scale_permille);
    Frame {
        opacity: if motion.fades {
            (255.0 * p).round() as u8
        } else {
            255
        },
        scale_permille: (from + (1000.0 - from) * p).round() as u32,
        shift: motion.shift,
        shift_permille: (1000.0 * (1.0 - p)).round() as u32,
    }
}

/// Where a zoom is at `now`: its frame that far between its rectangles, held at the end.
fn zoom_at(running: &Running, now: Duration) -> Frame {
    let p = progress(now.saturating_sub(running.started), running.motion.duration);
    Frame {
        opacity: 255,
        scale_permille: 1000,
        shift: running.motion.shift,
        shift_permille: (1000.0 * p).round() as u32,
    }
}

/// Where a surface is `p` of the way from whole to the motion's far end: going.
fn going_at(motion: &Motion, p: f64) -> Frame {
    let to = f64::from(motion.scale_permille);
    Frame {
        opacity: if motion.fades {
            (255.0 * (1.0 - p)).round() as u8
        } else {
            255
        },
        scale_permille: (1000.0 + (to - 1000.0) * p).round() as u32,
        shift: motion.shift,
        shift_permille: (1000.0 * p).round() as u32,
    }
}

/// The transitions running: surfaces arriving, surfaces going with their last frames, in the
/// order they began going, and windows zooming.
#[derive(Default)]
pub struct Transitions {
    arriving: BTreeMap<SurfaceKey, Running>,
    going: Vec<(SurfaceKey, Running, Snapshot)>,
    zooming: BTreeMap<SurfaceKey, Zoom>,
    next_id: u64,
    /// What each transition since the last `take_seen` did, by id.
    seen: BTreeMap<u64, Seen>,
    /// Reduce motion (docs/adr/0019-settings.mdx): every transition takes no time.
    reduced: bool,
}

impl Transitions {
    /// Reduce motion, the user's preference: from now on every surface arrives and goes at once,
    /// as a zero duration does; what is running finishes as it began.
    pub fn set_reduce_motion(&mut self, reduced: bool) {
        self.reduced = reduced;
    }

    pub fn reduce_motion(&self) -> bool {
        self.reduced
    }

    fn reduce(&self, motion: Motion) -> Motion {
        if self.reduced {
            Motion {
                duration: Duration::ZERO,
                ..motion
            }
        } else {
            motion
        }
    }

    /// A new transition's id, recorded as begun.
    fn begin(&mut self, kind: Kind, change: Change) -> u64 {
        // A session nobody asks keeps only the latest.
        while self.seen.len() >= MAX_SEEN {
            self.seen.pop_first();
        }
        self.next_id += 1;
        self.seen.insert(
            self.next_id,
            Seen {
                kind,
                change,
                first: None,
                last: None,
                frames: 0,
                ended: false,
            },
        );
        self.next_id
    }

    /// A surface came or went (the scene's appearance), at `now`: its transition begins, with
    /// `snapshot` (its last frame shown) for one going, where `place` puts its motion. A window
    /// zoomed begins with `zoom`, which needs its rectangles.
    pub fn appear(
        &mut self,
        appearance: Appearance,
        now: Duration,
        snapshot: Option<Snapshot>,
        place: &Place,
    ) {
        if appearance.change == Change::Zoomed {
            return;
        }
        let motion = self.reduce(motions(appearance.kind, appearance.change, place));
        if appearance.arrived() {
            self.start_arriving(
                appearance.key,
                appearance.kind,
                appearance.change,
                motion,
                now,
            );
        } else {
            self.start_going(
                appearance.key,
                appearance.kind,
                appearance.change,
                motion,
                now,
                snapshot,
            );
        }
    }

    /// `key` arrives with its first frame, as the ADR's motion for its kind.
    pub fn arrive(&mut self, key: SurfaceKey, kind: Kind, now: Duration) {
        let motion = self.reduce(motions(kind, Change::Arrived, &Place::default()));
        self.start_arriving(key, kind, Change::Arrived, motion, now);
    }

    /// `key` goes (destroyed, dismissed), drawn from `snapshot` (its last frame shown); without
    /// one it was not shown, and nothing is drawn.
    pub fn go(&mut self, key: SurfaceKey, kind: Kind, now: Duration, snapshot: Option<Snapshot>) {
        let motion = self.reduce(motions(kind, Change::Gone, &Place::default()));
        self.start_going(key, kind, Change::Gone, motion, now, snapshot);
    }

    fn start_arriving(
        &mut self,
        key: SurfaceKey,
        kind: Kind,
        change: Change,
        motion: Motion,
        now: Duration,
    ) {
        // A surface arriving again (restored, shown with its workspace) stops going.
        self.going.retain(|(going, _, _)| *going != key);
        if motion.duration.is_zero() {
            self.arriving.remove(&key);
            return;
        }
        let id = self.begin(kind, change);
        self.arriving.insert(
            key,
            Running {
                id,
                kind,
                started: now,
                motion,
            },
        );
    }

    fn start_going(
        &mut self,
        key: SurfaceKey,
        kind: Kind,
        change: Change,
        motion: Motion,
        now: Duration,
        snapshot: Option<Snapshot>,
    ) {
        self.arriving.remove(&key);
        let Some(snapshot) = snapshot.filter(|_| !motion.duration.is_zero()) else {
            return;
        };
        self.going.retain(|(going, _, _)| *going != key);
        let id = self.begin(kind, change);
        self.going.push((
            key,
            Running {
                id,
                kind,
                started: now,
                motion,
            },
            snapshot,
        ));
    }

    /// `key`'s frame zooms at `now` from `from`, where it is drawn, to `to`, where it is
    /// configured; `at` is where the surface is now. Until the end it is drawn in the rectangle
    /// between them (`zooming`), with whatever frame it shows: its old one, stretched toward
    /// `to`, until its client shows one of the new size, then that one. It ends once it is at
    /// `to` (`zoom_shown`), or a while after if its client never shows that frame; and as soon as
    /// it is anywhere else. A zoom of a window zooming begins from where it is drawn then.
    pub fn zoom(
        &mut self,
        key: SurfaceKey,
        kind: Kind,
        from: Rect,
        at: Rect,
        to: Rect,
        now: Duration,
    ) {
        let motion = self.reduce(motions(kind, Change::Zoomed, &Place::default()));
        self.zooming.remove(&key);
        if motion.duration.is_zero() || from == to {
            return;
        }
        let id = self.begin(kind, Change::Zoomed);
        self.zooming.insert(
            key,
            Zoom {
                running: Running {
                    id,
                    kind,
                    started: now,
                    motion: Motion {
                        shift: Shift::Between { from, to },
                        ..motion
                    },
                },
                at,
                reached: at == to,
                superseded: false,
            },
        );
    }

    /// Where `key` is drawn at `now` if it is zooming.
    pub fn zooming(&self, key: SurfaceKey, now: Duration) -> Option<Rect> {
        let zoom = self.zooming.get(&key)?;
        Some(zoom_at(&zoom.running, now).place(zoom.at))
    }

    /// Where the frame being composed has `key` (its own rectangle): a zoom ends once the window
    /// is where it was going, and is over if it is anywhere else but where it began.
    pub fn zoom_shown(&mut self, key: SurfaceKey, rect: Rect) {
        if let Some(zoom) = self.zooming.get_mut(&key) {
            let Shift::Between { to, .. } = zoom.running.motion.shift else {
                return;
            };
            zoom.reached = rect == to;
            zoom.superseded |= rect != to && rect != zoom.at;
        }
    }

    /// A frame of the compositor's own (the session's splash on one screen, `index`) fades out
    /// over what is shown, 400 ms; it goes as a panel does, so a lock would drop it.
    pub fn fade(&mut self, index: u32, snapshot: Snapshot, now: Duration) {
        let key = SurfaceKey {
            client: u32::MAX,
            id: u32::MAX - index,
        };
        self.going.retain(|(going, _, _)| *going != key);
        if self.reduced {
            return;
        }
        let id = self.begin(Kind::Panel, Change::Gone);
        self.going.push((
            key,
            Running {
                id,
                kind: Kind::Panel,
                started: now,
                motion: motion(400, 1000),
            },
            snapshot,
        ));
    }

    /// Where an arriving surface is at `now`; `WHOLE` for any other.
    pub fn arriving(&self, key: SurfaceKey, now: Duration) -> Frame {
        let Some(running) = self.arriving.get(&key) else {
            return Frame::WHOLE;
        };
        let p = progress(now.saturating_sub(running.started), running.motion.duration);
        arriving_at(&running.motion, p)
    }

    /// Whether `key` is arriving.
    pub fn is_arriving(&self, key: SurfaceKey) -> bool {
        self.arriving.contains_key(&key)
    }

    /// Whether `key` is arriving as a lock surface: drawn over the lock's cover until it is
    /// whole, so nothing under the lock shows through it.
    pub fn lock_arriving(&self, key: SurfaceKey) -> bool {
        self.arriving
            .get(&key)
            .is_some_and(|running| running.kind == Kind::Lock)
    }

    /// The surfaces going at `now`, oldest first: their last frames and where they are.
    pub fn going(&self, now: Duration) -> impl Iterator<Item = (&Snapshot, Kind, Frame)> {
        self.going.iter().map(move |(_, running, snapshot)| {
            let p = progress(now.saturating_sub(running.started), running.motion.duration);
            (snapshot, running.kind, going_at(&running.motion, p))
        })
    }

    /// Forgets the transitions that ended by `now`; true if any runs (or ended, its last frame
    /// still to compose).
    pub fn advance(&mut self, now: Duration) -> bool {
        let before = self.arriving.len() + self.going.len() + self.zooming.len();
        let mut ended = Vec::new();
        self.arriving.retain(|_, running| {
            let runs = now.saturating_sub(running.started) < running.motion.duration;
            if !runs {
                ended.push(running.id);
            }
            runs
        });
        self.going.retain(|(_, running, _)| {
            let runs = now.saturating_sub(running.started) < running.motion.duration;
            if !runs {
                ended.push(running.id);
            }
            runs
        });
        self.zooming.retain(|_, zoom| {
            let runs = !zoom.ends(now);
            // Moved or resized again, it did not end: it was interrupted.
            if !runs && !zoom.superseded {
                ended.push(zoom.running.id);
            }
            runs
        });
        for id in ended {
            if let Some(seen) = self.seen.get_mut(&id) {
                seen.ended = true;
            }
        }
        before > 0
    }

    /// Records the frame composed at `now`: what each transition running draws (after
    /// `advance`). A surface arriving that ended is whole in this frame, one going gone.
    pub fn record(&mut self, now: Duration) {
        let mut drawn: Vec<(u64, u8)> = self
            .arriving
            .iter()
            .map(|(key, running)| (running.id, self.arriving(*key, now).opacity))
            .collect();
        drawn.extend(
            self.going
                .iter()
                .zip(self.going(now))
                .map(|((_, running, _), (_, _, at))| (running.id, at.opacity)),
        );
        drawn.extend(self.zooming.values().map(|zoom| (zoom.running.id, 255)));
        for (id, opacity) in drawn {
            if let Some(seen) = self.seen.get_mut(&id) {
                seen.first.get_or_insert(opacity);
                seen.last = Some(opacity);
                seen.frames += 1;
            }
        }
    }

    /// What each transition since the last call did, in the order they began.
    pub fn take_seen(&mut self) -> Vec<Seen> {
        std::mem::take(&mut self.seen).into_values().collect()
    }

    /// Drops every surface going but the lock's: locked, nothing of the session is drawn.
    pub fn lock(&mut self) {
        self.going
            .retain(|(_, running, _)| running.kind == Kind::Lock);
        self.zooming.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.arriving.is_empty() && self.going.is_empty() && self.zooming.is_empty()
    }
}

impl Default for Place {
    fn default() -> Self {
        Place {
            output: Rect::new(0, 0, 1, 1),
            dock: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: SurfaceKey = SurfaceKey { client: 1, id: 1 };

    fn snapshot() -> Snapshot {
        Snapshot {
            rect: Rect::new(100, 100, 200, 100),
            extent: Size::new(200, 100),
            format: Format::Xrgb8888,
            pixels: Rc::new(vec![0; 200 * 100 * 4]),
            look: None,
            views: Vec::new(),
        }
    }

    #[test]
    fn a_window_grows_and_fades_in_easing_out() {
        let mut t = Transitions::default();
        let start = Duration::from_secs(10);
        t.arrive(KEY, Kind::Window, start);
        let at = |t: &Transitions, ms| t.arriving(KEY, start + Duration::from_millis(ms));
        assert_eq!(
            at(&t, 0),
            Frame {
                opacity: 0,
                scale_permille: 960,
                shift: Shift::None,
                shift_permille: 1000,
            }
        );
        // Halfway in time, 87.5 % of the way (1 - 0.5³).
        assert_eq!(
            at(&t, 90),
            Frame {
                opacity: 223,
                scale_permille: 995,
                shift: Shift::None,
                shift_permille: 125,
            }
        );
        assert_eq!(at(&t, 180), Frame::WHOLE);
        assert!(t.advance(start + Duration::from_millis(179)));
        assert!(
            t.advance(start + Duration::from_millis(180)),
            "its last frame"
        );
        assert!(t.is_empty());
        assert_eq!(at(&t, 10), Frame::WHOLE, "over");
    }

    #[test]
    fn a_menu_appears_at_once_and_fades_out_from_its_last_frame() {
        let mut t = Transitions::default();
        let start = Duration::from_secs(1);
        t.arrive(KEY, Kind::Popup, start);
        assert_eq!(t.arriving(KEY, start), Frame::WHOLE);
        assert!(t.is_empty());
        t.go(KEY, Kind::Popup, start, Some(snapshot()));
        let frames: Vec<Frame> = [0, 75, 149]
            .iter()
            .map(|ms| {
                t.going(start + Duration::from_millis(*ms))
                    .next()
                    .unwrap()
                    .2
            })
            .collect();
        assert_eq!(
            frames.iter().map(|f| f.opacity).collect::<Vec<_>>(),
            [255, 32, 0]
        );
        assert!(frames.iter().all(|f| f.scale_permille == 1000));
        t.advance(start + Duration::from_millis(150));
        assert!(t.is_empty());
        // Never shown, nothing goes.
        t.go(KEY, Kind::Popup, start, None);
        assert!(t.is_empty());
    }

    #[test]
    fn with_reduce_motion_everything_arrives_and_goes_at_once() {
        let mut t = Transitions::default();
        let now = Duration::from_secs(1);
        t.set_reduce_motion(true);
        t.arrive(KEY, Kind::Window, now);
        assert_eq!(t.arriving(KEY, now), Frame::WHOLE);
        t.go(KEY, Kind::Window, now, Some(snapshot()));
        t.fade(0, snapshot(), now);
        assert!(t.is_empty(), "nothing runs");
        assert!(t.take_seen().is_empty());
        // Turned off again, windows grow and fade in as before.
        t.set_reduce_motion(false);
        t.arrive(KEY, Kind::Window, now);
        assert_eq!(t.arriving(KEY, now).opacity, 0);
    }

    #[test]
    fn locking_drops_whatever_of_the_session_was_going() {
        let mut t = Transitions::default();
        let now = Duration::from_secs(1);
        t.go(KEY, Kind::Window, now, Some(snapshot()));
        let lock = SurfaceKey { client: 2, id: 1 };
        t.go(lock, Kind::Lock, now, Some(snapshot()));
        t.lock();
        assert_eq!(
            t.going(now).map(|(_, kind, _)| kind).collect::<Vec<_>>(),
            [Kind::Lock]
        );
    }

    #[test]
    fn a_restored_window_stops_going_and_arrives() {
        let mut t = Transitions::default();
        let now = Duration::from_secs(1);
        t.go(KEY, Kind::Window, now, Some(snapshot()));
        t.arrive(KEY, Kind::Window, now + Duration::from_millis(50));
        assert_eq!(t.going(now).count(), 0);
        assert_eq!(t.arriving(KEY, now + Duration::from_millis(50)).opacity, 0);
    }

    #[test]
    fn what_each_transition_drew_is_recorded() {
        let mut t = Transitions::default();
        let start = Duration::from_secs(1);
        let frame = |t: &mut Transitions, ms| {
            let now = start + Duration::from_millis(ms);
            t.advance(now);
            t.record(now);
        };
        t.arrive(KEY, Kind::Window, start);
        for ms in [0, 90, 180] {
            frame(&mut t, ms);
        }
        // A window restored as it went: its going is interrupted, not ended.
        let other = SurfaceKey { client: 2, id: 1 };
        t.go(
            other,
            Kind::Window,
            start + Duration::from_millis(190),
            Some(snapshot()),
        );
        frame(&mut t, 200);
        t.arrive(other, Kind::Window, start + Duration::from_millis(210));
        assert_eq!(
            t.take_seen(),
            [
                Seen {
                    kind: Kind::Window,
                    change: Change::Arrived,
                    first: Some(0),
                    last: Some(223),
                    frames: 2,
                    ended: true,
                },
                Seen {
                    kind: Kind::Window,
                    change: Change::Gone,
                    first: Some(207),
                    last: Some(207),
                    frames: 1,
                    ended: false,
                },
                Seen {
                    kind: Kind::Window,
                    change: Change::Arrived,
                    first: None,
                    last: None,
                    frames: 0,
                    ended: false,
                },
            ]
        );
        assert!(t.take_seen().is_empty());
    }

    #[test]
    fn a_surface_is_scaled_about_its_centre() {
        let frame = Frame {
            scale_permille: 960,
            ..Frame::WHOLE
        };
        assert_eq!(
            frame.place(Rect::new(100, 100, 200, 100)),
            Rect::new(104, 102, 192, 96)
        );
        assert_eq!(
            Frame::WHOLE.place(Rect::new(1, 2, 3, 4)),
            Rect::new(1, 2, 3, 4)
        );
    }

    const OUTPUT: Rect = Rect::new(0, 0, 1280, 720);
    const PLACE: Place = Place {
        output: OUTPUT,
        dock: Some((640, 692)),
    };

    fn appearance(change: Change) -> Appearance {
        Appearance {
            key: KEY,
            kind: Kind::Window,
            change,
        }
    }

    #[test]
    fn a_window_minimized_goes_into_the_dock_and_comes_back_out() {
        let mut t = Transitions::default();
        let start = Duration::from_secs(1);
        t.appear(
            appearance(Change::Minimized),
            start,
            Some(snapshot()),
            &PLACE,
        );
        let at = |t: &Transitions, ms| {
            let (snapshot, _, frame) = t.going(start + Duration::from_millis(ms)).next().unwrap();
            (frame.opacity, frame.place(snapshot.rect))
        };
        // Where it was, whole; then shrinking toward the dock; at the end a tenth of its size
        // on the dock's centre, faded.
        assert_eq!(at(&t, 0), (255, Rect::new(100, 100, 200, 100)));
        let (opacity, halfway) = at(&t, 150);
        assert!(opacity < 64, "{opacity}");
        let centre = |r: Rect| (r.x + r.width as i32 / 2, r.y + r.height as i32 / 2);
        assert!(halfway.width < 60 && centre(halfway).1 > 600, "{halfway:?}");
        let (_, end) = at(&t, 299);
        assert_eq!((end.width, end.height), (20, 10));
        let (x, y) = centre(end);
        assert!((x - 640).abs() <= 1 && (y - 692).abs() <= 1, "{end:?}");
        // Restored, it comes back the same way: from the dock, a tenth of its size, unseen.
        t.appear(appearance(Change::Restored), start, None, &PLACE);
        assert_eq!(t.going(start).count(), 0);
        let from = t.arriving(KEY, start);
        assert_eq!((from.opacity, from.scale_permille), (0, 100));
        let placed = from.place(Rect::new(100, 100, 200, 100));
        assert_eq!(centre(placed), (640, 692));
        assert_eq!(
            t.arriving(KEY, start + Duration::from_millis(300)),
            Frame::WHOLE
        );
    }

    #[test]
    fn a_switch_of_workspace_slides_windows_by_the_outputs_width() {
        let rect = Rect::new(100, 100, 200, 100);
        let going = |forward| {
            let mut t = Transitions::default();
            let start = Duration::from_secs(1);
            t.appear(
                appearance(Change::WorkspaceOut { forward }),
                start,
                Some(snapshot()),
                &PLACE,
            );
            let at = |ms| t.going(start + Duration::from_millis(ms)).next().unwrap().2;
            (at(0), at(300).place(rect), at(299))
        };
        // To a later workspace the window leaves to the left; to an earlier one, to the right;
        // whole and unfaded as it goes.
        let (first, last, end) = going(true);
        assert_eq!(first.place(rect), rect);
        assert_eq!(last, Rect::new(100 - 1280, 100, 200, 100));
        assert_eq!((end.opacity, end.scale_permille), (255, 1000));
        assert_eq!(going(false).1, Rect::new(100 + 1280, 100, 200, 100));
        // The workspace switched to: its windows come in from the right going forward.
        let mut t = Transitions::default();
        let start = Duration::from_secs(1);
        t.appear(
            appearance(Change::WorkspaceIn { forward: true }),
            start,
            None,
            &PLACE,
        );
        let from = t.arriving(KEY, start);
        assert_eq!(from.opacity, 255);
        assert_eq!(from.place(rect), Rect::new(100 + 1280, 100, 200, 100));
        assert_eq!(
            t.arriving(KEY, start + Duration::from_millis(300)),
            Frame::WHOLE
        );
    }

    #[test]
    fn a_zoom_draws_the_frame_between_its_rectangles_easing_out() {
        let mut t = Transitions::default();
        let start = Duration::from_secs(1);
        let from = Rect::new(190, 60, 900, 600);
        let to = Rect::new(0, 32, 1280, 688);
        t.zoom(KEY, Kind::Window, from, from, to, start);
        let at = |t: &Transitions, ms| t.zooming(KEY, start + Duration::from_millis(ms)).unwrap();
        // Where it was; halfway in time 87.5 % of the way (1 - 0.5³), each edge by its own
        // distance, so width and height scale each by their own factor; then where it goes.
        assert_eq!(at(&t, 0), from);
        assert_eq!(at(&t, 125), Rect::new(24, 36, 1232, 677));
        assert_eq!(at(&t, 250), to);
        // Its client has not shown the new size yet: it is held there, its old frame stretched.
        t.zoom_shown(KEY, from);
        assert!(t.advance(start + Duration::from_millis(300)));
        assert_eq!(at(&t, 300), to);
        // Shown at its new size, it ends.
        t.zoom_shown(KEY, to);
        assert!(
            t.advance(start + Duration::from_millis(310)),
            "its last frame"
        );
        assert!(t.is_empty());
        assert_eq!(t.zooming(KEY, start), None);
        let seen = t.take_seen();
        assert_eq!(
            (seen[0].kind, seen[0].change, seen[0].ended),
            (Kind::Window, Change::Zoomed, true)
        );
    }

    #[test]
    fn a_zoom_ends_where_its_window_is_if_never_answered_or_moved() {
        let mut t = Transitions::default();
        let start = Duration::from_secs(1);
        let (from, to) = (Rect::new(10, 10, 100, 100), Rect::new(0, 0, 400, 300));
        t.zoom(KEY, Kind::Window, from, from, to, start);
        t.zoom_shown(KEY, from);
        assert!(t.advance(start + Duration::from_millis(1249)));
        assert!(!t.is_empty(), "held a second past its end");
        t.advance(start + Duration::from_millis(1250));
        assert!(t.is_empty());
        // Moved by hand as it zooms: over at once, interrupted.
        t.take_seen();
        t.zoom(KEY, Kind::Window, from, from, to, start);
        t.zoom_shown(KEY, Rect::new(50, 10, 100, 100));
        t.advance(start + Duration::from_millis(20));
        assert!(t.is_empty());
        assert!(!t.take_seen()[0].ended);
        // A zoom of a window zooming begins where it is drawn.
        t.zoom(KEY, Kind::Window, from, from, to, start);
        let between = t.zooming(KEY, start + Duration::from_millis(100)).unwrap();
        t.zoom(
            KEY,
            Kind::Window,
            between,
            from,
            from,
            start + Duration::from_millis(100),
        );
        assert_eq!(
            t.zooming(KEY, start + Duration::from_millis(100)),
            Some(between)
        );
    }

    #[test]
    fn with_reduce_motion_a_window_zooms_at_once() {
        let mut t = Transitions::default();
        t.set_reduce_motion(true);
        let now = Duration::from_secs(1);
        t.zoom(
            KEY,
            Kind::Window,
            Rect::new(10, 10, 100, 100),
            Rect::new(10, 10, 100, 100),
            Rect::new(0, 0, 400, 300),
            now,
        );
        assert!(t.is_empty());
        assert_eq!(t.zooming(KEY, now), None);
        assert!(t.take_seen().is_empty());
    }

    #[test]
    fn a_notification_slides_in_from_beside_itself_and_fades_out() {
        let mut t = Transitions::default();
        let start = Duration::from_secs(1);
        let notification = Appearance {
            key: KEY,
            kind: Kind::Notification,
            change: Change::Arrived,
        };
        t.appear(notification, start, None, &PLACE);
        let rect = Rect::new(900, 40, 360, 80);
        let from = t.arriving(KEY, start);
        // Its own width and a tenth beyond where it rests: off the output's right edge.
        assert_eq!(from.place(rect), Rect::new(900 + 396, 40, 360, 80));
        assert_eq!(from.opacity, 0);
        assert_eq!(
            t.arriving(KEY, start + Duration::from_millis(250)),
            Frame::WHOLE
        );
        t.appear(
            Appearance {
                change: Change::Gone,
                ..notification
            },
            start,
            Some(snapshot()),
            &PLACE,
        );
        let gone = t
            .going(start + Duration::from_millis(100))
            .next()
            .unwrap()
            .2;
        assert!(gone.opacity < 255 && gone.place(rect) == rect);
    }
}
