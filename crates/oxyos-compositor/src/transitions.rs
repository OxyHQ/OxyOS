//! Surfaces arriving and going, animated (docs/adr/0011-transitions.mdx): a window grows a
//! little and fades in, a menu fades out, the launcher's icons settle over their fading
//! backdrop, the lock screen fades; a window minimized goes into the dock and comes back out of
//! it, a switch of workspace slides the windows aside, a notification slides in. Every motion is
//! one of the session's springs (`motion.rs`). What a surface shows is never changed: it is drawn
//! whole, scaled about its centre and moved, at an opacity. A surface going away is drawn from
//! its last frame, which the compositor keeps until its transition ends. A window zoomed (or
//! maximized, tiled, restored) has its frame go from one rectangle to the other, a Mac's zoom
//! (docs/adr/0016-windows.mdx): its content, old or new, stretched into the rectangle between,
//! its corners squaring or rounding as it goes.

use crate::motion::{self, Fade, Spring};
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

/// How a surface arrives or goes: the spring its size and place go on (none: they do not
/// change), how it fades (none: it stays whole); the scale it starts from (arriving) or ends at
/// (going), in thousandths; and where it is moved. Neither spring: at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Motion {
    pub moves: Option<Spring>,
    pub fades: Option<Fade>,
    pub scale_permille: u32,
    pub shift: Shift,
}

impl Motion {
    /// Nothing animated: the surface is there, or gone, at once.
    pub const AT_ONCE: Motion = Motion {
        moves: None,
        fades: None,
        scale_permille: 1000,
        shift: Shift::None,
    };

    /// Scaled from or to `scale_permille` on `moves`, fading on `fades`.
    const fn scaled(moves: Spring, fades: Spring, scale_permille: u32) -> Motion {
        Motion {
            moves: Some(moves),
            fades: Some(Fade::new(fades)),
            scale_permille,
            shift: Shift::None,
        }
    }

    /// Faded alone, on `fades`.
    const fn faded(fades: Spring) -> Motion {
        Motion {
            moves: None,
            fades: Some(Fade::new(fades)),
            ..Motion::AT_ONCE
        }
    }

    /// How long it runs: until its springs are at rest.
    pub fn duration(&self) -> Duration {
        let moves = self.moves.map_or(Duration::ZERO, |spring| spring.settles());
        let fades = self.fades.map_or(Duration::ZERO, |fade| fade.settles());
        moves.max(fades)
    }

    /// How far its size and place are `elapsed` in, from 0 to 1 (past 1 overshooting).
    fn moved(&self, elapsed: Duration) -> f64 {
        self.moves.map_or(1.0, |spring| spring.at(elapsed))
    }

    /// How far its opacity is `elapsed` in, from 0 to 1.
    fn faded_at(&self, elapsed: Duration) -> f64 {
        self.fades.map_or(1.0, |fade| fade.at(elapsed))
    }

    /// The same, as reduce motion has it (docs/adr/0019-settings.mdx): a crossfade, nothing
    /// growing, shrinking or sliding; what is at once stays so.
    fn reduced(self) -> Motion {
        if self == Motion::AT_ONCE {
            return self;
        }
        Motion::faded(motion::CROSSFADE)
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
    // Mac's minimize: shrinking to a tenth as it moves there, fading as it is nearly in.
    let dock = place.dock.unwrap_or((
        place.output.x + place.output.width as i32 / 2,
        place.output.y + place.output.height as i32,
    ));
    let width = place.output.width as i32;
    match (kind, change) {
        (_, Change::Minimized | Change::Restored) => Motion {
            moves: Some(motion::DOCK),
            fades: Some(if change == Change::Minimized {
                motion::INTO_DOCK_FADES
            } else {
                motion::OUT_OF_DOCK_FADES
            }),
            scale_permille: 100,
            shift: Shift::To {
                x: dock.0,
                y: dock.1,
            },
        },
        // A workspace's windows slide by the output's width, a Mac's spaces: the later
        // workspace's from the right, the one left to the left; they do not fade.
        (_, Change::WorkspaceIn { forward } | Change::WorkspaceOut { forward }) => {
            let leaving = matches!(change, Change::WorkspaceOut { .. });
            Motion {
                moves: Some(motion::WORKSPACE),
                fades: None,
                scale_permille: 1000,
                shift: Shift::By {
                    x: if forward == leaving { -width } else { width },
                    y: 0,
                },
            }
        }
        // A Mac's zoom: the frame goes to its new rectangle, unfaded (`zoom`).
        (_, Change::Zoomed) => Motion {
            moves: Some(motion::ZOOM),
            ..Motion::AT_ONCE
        },
        (Kind::Window, Change::Arrived) => Motion::scaled(
            motion::WINDOW_ARRIVES,
            motion::WINDOW_FADES_IN,
            motion::WINDOW_SCALE_PERMILLE,
        ),
        (Kind::Window, _) => Motion::scaled(
            motion::WINDOW_GOES,
            motion::WINDOW_FADES_OUT,
            motion::WINDOW_SCALE_PERMILLE,
        ),
        // As a Mac's menus: at once, then fading out.
        (Kind::Popup, Change::Arrived) => Motion::AT_ONCE,
        (Kind::Popup, _) => Motion::faded(motion::MENU_FADES_OUT),
        (Kind::Overlay, Change::Arrived) => Motion::scaled(
            motion::OVERLAY_ARRIVES,
            motion::OVERLAY_FADES_IN,
            motion::OVERLAY_SCALE_PERMILLE,
        ),
        (Kind::Overlay, _) => Motion::scaled(
            motion::OVERLAY_GOES,
            motion::OVERLAY_FADES_OUT,
            motion::OVERLAY_SCALE_PERMILLE,
        ),
        (Kind::Backdrop, Change::Arrived) => Motion::faded(motion::BACKDROP_FADES_IN),
        (Kind::Backdrop, _) => Motion::faded(motion::BACKDROP_FADES_OUT),
        (Kind::Panel, Change::Arrived) => Motion::faded(motion::PANEL_FADES_IN),
        (Kind::Panel, _) => Motion::AT_ONCE,
        // A notification slides in from beside itself at the right edge, a Mac's, and fades out.
        (Kind::Notification, Change::Arrived) => Motion {
            moves: Some(motion::NOTIFICATION_ARRIVES),
            fades: Some(Fade::new(motion::NOTIFICATION_FADES_IN)),
            scale_permille: 1000,
            shift: Shift::Own {
                x_permille: 1100,
                y_permille: 0,
            },
        },
        (Kind::Notification, _) => Motion::faded(motion::NOTIFICATION_FADES_OUT),
        (Kind::Lock, _) => Motion::faded(motion::LOCK_FADES),
        (Kind::Candidates, _) => Motion::AT_ONCE,
    }
}

/// Transitions recorded until `take_seen`.
const MAX_SEEN: usize = 64;
/// The frames of each transition recorded (`Seen::path`).
const MAX_PATH: usize = 64;
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
    pub pixels: Rc<crate::Pixels>,
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
    pub pixels: Rc<crate::Pixels>,
}

struct Running {
    /// Which transition this is, for what is recorded of it (`Seen`).
    id: u64,
    kind: Kind,
    started: Duration,
    motion: Motion,
    /// How long it runs (`Motion::duration`, worked out once).
    duration: Duration,
}

impl Running {
    fn new(id: u64, kind: Kind, started: Duration, motion: Motion) -> Self {
        Self {
            id,
            kind,
            started,
            motion,
            duration: motion.duration(),
        }
    }

    fn elapsed(&self, now: Duration) -> Duration {
        now.saturating_sub(self.started)
    }

    fn runs(&self, now: Duration) -> bool {
        self.elapsed(now) < self.duration
    }
}

/// A window's frame zooming: where the surface was when it began (`at`), and whether it is now
/// where it is going (`motion`'s `Shift::Between`'s end), its new frame shown, or somewhere else
/// (moved or resized again: the zoom is over). Its corners go from one radius to the other
/// (`radius`, logical pixels): a window zoomed to fill the work area has square ones.
struct Zoom {
    running: Running,
    at: Rect,
    radius: (u32, u32),
    reached: bool,
    superseded: bool,
}

impl Zoom {
    fn ends(&self, now: Duration) -> bool {
        let elapsed = self.running.elapsed(now);
        let duration = self.running.duration;
        self.superseded || elapsed >= duration && (self.reached || elapsed >= duration + ZOOM_HOLD)
    }
}

/// What a transition did, as the development compositor reports it: its kind, how the surface
/// came or went, the opacities of the first and last frames drawn in it, how many frames those
/// were, whether it ran to its end (not interrupted: a window restored as it went, going
/// dropped by the lock), and each frame drawn (`path`, the first `MAX_PATH`): when, at what
/// opacity, and how far along its size and place were.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    pub kind: Kind,
    pub change: Change,
    pub first: Option<u8>,
    pub last: Option<u8>,
    pub frames: u32,
    pub ended: bool,
    pub path: Vec<Step>,
}

/// A frame of a transition: its time since the transition began, its opacity, and how far its
/// size and place had gone from where they start toward where they end, in thousandths (past
/// 1000 overshooting).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub ms: u32,
    pub opacity: u8,
    pub moved_permille: i32,
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
/// it at now, and how far along its shift it is (thousandths, past 1000 or below 0
/// overshooting).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub opacity: u8,
    pub scale_permille: u32,
    pub shift: Shift,
    pub shift_permille: i32,
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
            let k = f64::from(self.shift_permille.clamp(0, 1000)) / 1000.0;
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

fn opacity(share: f64) -> u8 {
    (255.0 * share.clamp(0.0, 1.0)).round() as u8
}

/// The scale `moved` of the way from `from` to `to`, in thousandths.
fn scale_between(from: u32, to: u32, moved: f64) -> u32 {
    (f64::from(from) + (f64::from(to) - f64::from(from)) * moved)
        .round()
        .max(1.0) as u32
}

/// Where an arriving surface is `elapsed` into its motion, from the motion's far end to whole.
fn arriving_at(running: &Running, elapsed: Duration) -> Frame {
    if elapsed >= running.duration {
        return Frame::WHOLE;
    }
    let motion = &running.motion;
    let moved = motion.moved(elapsed);
    Frame {
        opacity: opacity(motion.faded_at(elapsed)),
        scale_permille: scale_between(motion.scale_permille, 1000, moved),
        shift: motion.shift,
        shift_permille: (1000.0 * (1.0 - moved)).round() as i32,
    }
}

/// Where a zoom is `elapsed` in: its frame that far between its rectangles, held at the end.
fn zoom_at(running: &Running, elapsed: Duration) -> Frame {
    let moved = if elapsed >= running.duration {
        1.0
    } else {
        running.motion.moved(elapsed)
    };
    Frame {
        opacity: 255,
        scale_permille: 1000,
        shift: running.motion.shift,
        shift_permille: (1000.0 * moved).round() as i32,
    }
}

/// Where a going surface is `elapsed` into its motion, from whole to the motion's far end.
fn going_at(running: &Running, elapsed: Duration) -> Frame {
    let motion = &running.motion;
    let (moved, faded) = if elapsed >= running.duration {
        (1.0, 1.0)
    } else {
        (motion.moved(elapsed), motion.faded_at(elapsed))
    };
    Frame {
        opacity: if motion.fades.is_some() {
            opacity(1.0 - faded)
        } else {
            255
        },
        scale_permille: scale_between(1000, motion.scale_permille, moved),
        shift: motion.shift,
        shift_permille: (1000.0 * moved).round() as i32,
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
    /// Reduce motion (docs/adr/0019-settings.mdx): every transition is a crossfade.
    reduced: bool,
}

impl Transitions {
    /// Reduce motion, the user's preference: from now on every surface arriving or going
    /// crossfades, nothing growing, shrinking or sliding, and a zoom is at once; what is running
    /// finishes as it began.
    pub fn set_reduce_motion(&mut self, reduced: bool) {
        self.reduced = reduced;
    }

    pub fn reduce_motion(&self) -> bool {
        self.reduced
    }

    fn reduce(&self, motion: Motion) -> Motion {
        if self.reduced {
            motion.reduced()
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
                path: Vec::new(),
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
        if motion.duration().is_zero() {
            self.arriving.remove(&key);
            return;
        }
        let id = self.begin(kind, change);
        self.arriving
            .insert(key, Running::new(id, kind, now, motion));
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
        let Some(snapshot) = snapshot.filter(|_| !motion.duration().is_zero()) else {
            return;
        };
        self.going.retain(|(going, _, _)| *going != key);
        let id = self.begin(kind, change);
        self.going
            .push((key, Running::new(id, kind, now, motion), snapshot));
    }

    /// `key`'s frame zooms at `now` from `from`, where it is drawn, to `to`, where it is
    /// configured; `at` is where the surface is now; its corners go from `radius.0` to
    /// `radius.1` (logical pixels). Until the end it is drawn in the rectangle between them
    /// (`zooming`), with whatever frame it shows: its old one, stretched toward `to`, until its
    /// client shows one of the new size, then that one. It ends once it is at `to`
    /// (`zoom_shown`), or a while after if its client never shows that frame; and as soon as it
    /// is anywhere else. A zoom of a window zooming begins from where it is drawn then.
    #[allow(clippy::too_many_arguments)]
    pub fn zoom(
        &mut self,
        key: SurfaceKey,
        kind: Kind,
        from: Rect,
        at: Rect,
        to: Rect,
        radius: (u32, u32),
        now: Duration,
    ) {
        self.zooming.remove(&key);
        // With reduce motion a window zooms at once: a crossfade would show its old frame
        // stretched.
        if self.reduced || from == to {
            return;
        }
        let motion = Motion {
            shift: Shift::Between { from, to },
            ..motions(kind, Change::Zoomed, &Place::default())
        };
        let id = self.begin(kind, Change::Zoomed);
        self.zooming.insert(
            key,
            Zoom {
                running: Running::new(id, kind, now, motion),
                at,
                radius,
                reached: at == to,
                superseded: false,
            },
        );
    }

    /// Where `key` is drawn at `now` if it is zooming, and its corners' radius there.
    pub fn zooming(&self, key: SurfaceKey, now: Duration) -> Option<(Rect, u32)> {
        let zoom = self.zooming.get(&key)?;
        let frame = zoom_at(&zoom.running, zoom.running.elapsed(now));
        let (from, to) = (f64::from(zoom.radius.0), f64::from(zoom.radius.1));
        let along = f64::from(frame.shift_permille.clamp(0, 1000)) / 1000.0;
        let radius = (from + (to - from) * along).round() as u32;
        Some((frame.place(zoom.at), radius))
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
    /// over what is shown; it goes as a panel does, so a lock would drop it.
    pub fn fade(&mut self, index: u32, snapshot: Snapshot, now: Duration) {
        let key = SurfaceKey {
            client: u32::MAX,
            id: u32::MAX - index,
        };
        self.going.retain(|(going, _, _)| *going != key);
        let motion = self.reduce(Motion::faded(motion::SPLASH_FADES_OUT));
        let id = self.begin(Kind::Panel, Change::Gone);
        self.going
            .push((key, Running::new(id, Kind::Panel, now, motion), snapshot));
    }

    /// Where an arriving surface is at `now`; `WHOLE` for any other.
    pub fn arriving(&self, key: SurfaceKey, now: Duration) -> Frame {
        let Some(running) = self.arriving.get(&key) else {
            return Frame::WHOLE;
        };
        arriving_at(running, running.elapsed(now))
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
            (
                snapshot,
                running.kind,
                going_at(running, running.elapsed(now)),
            )
        })
    }

    /// Forgets the transitions that ended by `now`; true if any runs (or ended, its last frame
    /// still to compose).
    pub fn advance(&mut self, now: Duration) -> bool {
        let before = self.arriving.len() + self.going.len() + self.zooming.len();
        let mut ended = Vec::new();
        self.arriving.retain(|_, running| {
            let runs = running.runs(now);
            if !runs {
                ended.push(running.id);
            }
            runs
        });
        self.going.retain(|(_, running, _)| {
            let runs = running.runs(now);
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
        // How far a frame's size and place are along their way, in thousandths.
        let moved = |running: &Running, frame: &Frame, arriving: bool| match running.motion.shift {
            Shift::None if running.motion.scale_permille == 1000 => 1000,
            Shift::None => {
                let far = i64::from(running.motion.scale_permille) - 1000;
                let at = i64::from(frame.scale_permille) - 1000;
                let gone = (1000 * at / far) as i32;
                if arriving {
                    1000 - gone
                } else {
                    gone
                }
            }
            _ if arriving => 1000 - frame.shift_permille,
            _ => frame.shift_permille,
        };
        let mut drawn: Vec<(u64, Duration, Frame, i32)> = self
            .arriving
            .values()
            .map(|running| {
                let frame = arriving_at(running, running.elapsed(now));
                (
                    running.id,
                    running.elapsed(now),
                    frame,
                    moved(running, &frame, true),
                )
            })
            .collect();
        drawn.extend(self.going.iter().map(|(_, running, _)| {
            let frame = going_at(running, running.elapsed(now));
            (
                running.id,
                running.elapsed(now),
                frame,
                moved(running, &frame, false),
            )
        }));
        drawn.extend(self.zooming.values().map(|zoom| {
            let frame = zoom_at(&zoom.running, zoom.running.elapsed(now));
            (
                zoom.running.id,
                zoom.running.elapsed(now),
                frame,
                frame.shift_permille,
            )
        }));
        for (id, elapsed, frame, moved) in drawn {
            if let Some(seen) = self.seen.get_mut(&id) {
                seen.first.get_or_insert(frame.opacity);
                seen.last = Some(frame.opacity);
                seen.frames += 1;
                if seen.path.len() < MAX_PATH {
                    seen.path.push(Step {
                        ms: elapsed.as_millis().min(u128::from(u32::MAX)) as u32,
                        opacity: frame.opacity,
                        moved_permille: moved,
                    });
                }
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
            pixels: Rc::new(vec![0; 200 * 100 * 4].into()),
            look: None,
            views: Vec::new(),
        }
    }

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    /// How long `kind`'s motion for `change` runs.
    fn lasts(kind: Kind, change: Change) -> Duration {
        motions(kind, change, &Place::default()).duration()
    }

    #[test]
    fn a_window_grows_and_fades_in_on_its_springs() {
        let mut t = Transitions::default();
        let start = Duration::from_secs(10);
        t.arrive(KEY, Kind::Window, start);
        let at = |t: &Transitions, ms: u64| t.arriving(KEY, start + Duration::from_millis(ms));
        assert_eq!(
            at(&t, 0),
            Frame {
                opacity: 0,
                scale_permille: 940,
                shift: Shift::None,
                shift_permille: 1000,
            }
        );
        // 90 ms in, worked by hand: its size on a 300 ms spring is 1 - (1 + ωt)e^(-ωt) of the
        // way (ω = 2π/0.3 s, ωt = 1.885: 0.562), 940 + 60 · 0.562 = 974; its opacity on a 180 ms
        // spring (ωt = 3.14: 0.821) is 209.
        let early = at(&t, 90);
        assert_eq!((early.opacity, early.scale_permille), (209, 974));
        // Its opacity arrives before its size: whole by 250 ms, its size still settling.
        assert_eq!(at(&t, 270).opacity, 255);
        assert!(at(&t, 270).scale_permille < 1000);
        // At rest, whole, when its size's spring is: 441 ms.
        let end = lasts(Kind::Window, Change::Arrived);
        assert_eq!(end, ms(441));
        assert_eq!(at(&t, 441), Frame::WHOLE);
        assert!(t.advance(start + ms(440)));
        assert!(t.advance(start + ms(441)), "its last frame");
        assert!(t.is_empty());
        assert_eq!(at(&t, 10), Frame::WHOLE, "over");
    }

    #[test]
    fn every_frame_of_an_arrival_and_a_departure_eases_out_monotonically() {
        for kind in [Kind::Window, Kind::Overlay, Kind::Backdrop, Kind::Popup] {
            for arriving in [true, false] {
                if kind == Kind::Popup && arriving {
                    continue;
                }
                let mut t = Transitions::default();
                let start = Duration::from_secs(1);
                if arriving {
                    t.arrive(KEY, kind, start);
                } else {
                    t.go(KEY, kind, start, Some(snapshot()));
                }
                let frame = |ms: u64| {
                    let now = start + Duration::from_millis(ms);
                    if arriving {
                        t.arriving(KEY, now)
                    } else {
                        t.going(now).next().unwrap().2
                    }
                };
                let end = lasts(
                    kind,
                    if arriving {
                        Change::Arrived
                    } else {
                        Change::Gone
                    },
                );
                let frames: Vec<Frame> = (0..end.as_millis() as u64).map(frame).collect();
                // Opacity and scale only ever go one way, and the opacity is ahead of a linear
                // fade of the same length from the first display frame on: quick, then slow to
                // arrive, never linear.
                for pair in frames.windows(2) {
                    let (a, b) = (pair[0], pair[1]);
                    assert!(
                        if arriving {
                            b.opacity >= a.opacity
                        } else {
                            b.opacity <= a.opacity
                        },
                        "{kind:?} {arriving}"
                    );
                    // Its scale only ever nears where it ends (arriving, 1000).
                    let towards = |f: Frame| {
                        let end = if arriving {
                            1000
                        } else {
                            frames[frames.len() - 1].scale_permille
                        };
                        (i64::from(f.scale_permille) - i64::from(end)).abs()
                    };
                    assert!(
                        towards(b) <= towards(a),
                        "{kind:?} {arriving} scale went back"
                    );
                }
                let fade = end.as_millis() as f64;
                for (ms, f) in frames.iter().enumerate().skip(17) {
                    let shown = if arriving { f.opacity } else { 255 - f.opacity };
                    assert!(
                        f64::from(shown) + 1.0 >= 255.0 * ms as f64 / fade,
                        "{kind:?} arriving {arriving}: {shown} at {ms} ms of {fade}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_menu_appears_at_once_and_fades_out_from_its_last_frame() {
        let mut t = Transitions::default();
        let start = Duration::from_secs(1);
        t.arrive(KEY, Kind::Popup, start);
        assert_eq!(t.arriving(KEY, start), Frame::WHOLE);
        assert!(t.is_empty());
        t.go(KEY, Kind::Popup, start, Some(snapshot()));
        let frames: Vec<Frame> = [0, 50, 100, 220]
            .iter()
            .map(|ms| {
                t.going(start + Duration::from_millis(*ms))
                    .next()
                    .unwrap()
                    .2
            })
            .collect();
        // A 150 ms spring: 50 ms in (ωt = 2.09) it is 0.62 of the way, 100 ms (4.19) 0.92.
        assert_eq!(
            frames.iter().map(|f| f.opacity).collect::<Vec<_>>(),
            [255, 97, 20, 0]
        );
        assert!(frames.iter().all(|f| f.scale_permille == 1000));
        t.advance(start + lasts(Kind::Popup, Change::Gone));
        assert!(t.is_empty());
        // Never shown, nothing goes.
        t.go(KEY, Kind::Popup, start, None);
        assert!(t.is_empty());
    }

    #[test]
    fn launchpads_icons_settle_from_larger_over_a_backdrop_that_only_fades() {
        let mut t = Transitions::default();
        let start = Duration::from_secs(1);
        let backdrop = SurfaceKey { client: 1, id: 2 };
        t.arrive(backdrop, Kind::Backdrop, start);
        t.arrive(KEY, Kind::Overlay, start);
        let at = |t: &Transitions, key, ms| t.arriving(key, start + Duration::from_millis(ms));
        let (icons, behind) = (at(&t, KEY, 0), at(&t, backdrop, 0));
        assert_eq!((icons.opacity, icons.scale_permille), (0, 1040));
        assert_eq!((behind.opacity, behind.scale_permille), (0, 1000));
        let rect = Rect::new(0, 0, 1280, 800);
        // The icons are larger than their place, about its centre; the backdrop never is.
        assert_eq!(icons.place(rect), Rect::new(-25, -16, 1331, 832));
        for ms in [16, 100, 200, 400] {
            assert_eq!(at(&t, backdrop, ms).place(rect), rect);
        }
        // Going, the icons grow back as they fade; the backdrop fades.
        t.go(KEY, Kind::Overlay, start, Some(snapshot()));
        t.go(backdrop, Kind::Backdrop, start, Some(snapshot()));
        let late = start + Duration::from_millis(120);
        let going: Vec<Frame> = t.going(late).map(|(_, _, frame)| frame).collect();
        assert!(
            going[0].scale_permille > 1020 && going[0].opacity < 64,
            "{going:?}"
        );
        assert!(
            going[1].scale_permille == 1000 && going[1].opacity < 128,
            "{going:?}"
        );
    }

    #[test]
    fn with_reduce_motion_everything_crossfades_and_nothing_moves() {
        let mut t = Transitions::default();
        let now = Duration::from_secs(1);
        t.set_reduce_motion(true);
        t.arrive(KEY, Kind::Window, now);
        let first = t.arriving(KEY, now);
        assert_eq!((first.opacity, first.scale_permille), (0, 1000));
        let rect = Rect::new(100, 100, 200, 100);
        for ms in [0, 50, 100, 200] {
            assert_eq!(
                t.arriving(KEY, now + Duration::from_millis(ms)).place(rect),
                rect
            );
        }
        let crossfade = motion::CROSSFADE.settles();
        assert_eq!(t.arriving(KEY, now + crossfade), Frame::WHOLE);
        // Minimized, it fades where it is: nothing goes into the dock.
        let place = Place {
            output: Rect::new(0, 0, 1280, 720),
            dock: Some((640, 692)),
        };
        t.appear(
            Appearance {
                key: KEY,
                kind: Kind::Window,
                change: Change::Minimized,
            },
            now,
            Some(snapshot()),
            &place,
        );
        let (_, _, going) = t.going(now + Duration::from_millis(60)).next().unwrap();
        assert!(going.opacity < 255);
        assert_eq!(going.place(rect), rect);
        // What appears at once still does; a menu fades out.
        t.arrive(SurfaceKey { client: 3, id: 1 }, Kind::Popup, now);
        assert_eq!(
            t.arriving(SurfaceKey { client: 3, id: 1 }, now),
            Frame::WHOLE
        );
        // Turned off again, windows grow and fade in as before.
        t.set_reduce_motion(false);
        t.arrive(KEY, Kind::Window, now);
        assert_eq!(t.arriving(KEY, now).scale_permille, 940);
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
        for ms in [0, 90, 441] {
            frame(&mut t, ms);
        }
        // A window restored as it went: its going is interrupted, not ended.
        let other = SurfaceKey { client: 2, id: 1 };
        t.go(
            other,
            Kind::Window,
            start + Duration::from_millis(450),
            Some(snapshot()),
        );
        frame(&mut t, 460);
        t.arrive(other, Kind::Window, start + Duration::from_millis(470));
        assert_eq!(
            t.take_seen(),
            [
                Seen {
                    kind: Kind::Window,
                    change: Change::Arrived,
                    first: Some(0),
                    last: Some(209),
                    frames: 2,
                    ended: true,
                    // Its size 567 thousandths of the way at 90 ms, from its scale (974).
                    path: vec![
                        Step {
                            ms: 0,
                            opacity: 0,
                            moved_permille: 0
                        },
                        Step {
                            ms: 90,
                            opacity: 209,
                            moved_permille: 567
                        },
                    ],
                },
                Seen {
                    kind: Kind::Window,
                    change: Change::Gone,
                    first: Some(238),
                    last: Some(238),
                    frames: 1,
                    ended: false,
                    path: vec![Step {
                        ms: 10,
                        opacity: 238,
                        moved_permille: 33
                    }],
                },
                Seen {
                    kind: Kind::Window,
                    change: Change::Arrived,
                    first: None,
                    last: None,
                    frames: 0,
                    ended: false,
                    path: Vec::new(),
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
    fn a_window_minimized_goes_into_the_dock_whole_until_nearly_in_and_comes_back_out() {
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
        let centre = |r: Rect| (r.x + r.width as i32 / 2, r.y + r.height as i32 / 2);
        // Where it was, whole; then shrinking toward the dock, still whole as it goes (its fade
        // waits 180 ms); at the end a tenth of its size on the dock's centre, faded.
        assert_eq!(at(&t, 0), (255, Rect::new(100, 100, 200, 100)));
        let (opacity, early) = at(&t, 150);
        assert_eq!(opacity, 255);
        assert!(early.width < 100 && centre(early).1 > 450, "{early:?}");
        let (opacity, _) = at(&t, 300);
        assert!(opacity < 128, "{opacity}");
        let end = lasts(Kind::Window, Change::Minimized);
        assert_eq!(end, motion::DOCK.settles());
        let (opacity, last) = at(&t, end.as_millis() as u64 - 1);
        assert_eq!(opacity, 0);
        assert_eq!((last.width, last.height), (20, 10));
        let (x, y) = centre(last);
        assert!((x - 640).abs() <= 1 && (y - 692).abs() <= 1, "{last:?}");
        // Restored, it comes back the same way: from the dock, a tenth of its size, unseen, and
        // is whole at once.
        t.appear(appearance(Change::Restored), start, None, &PLACE);
        assert_eq!(t.going(start).count(), 0);
        let from = t.arriving(KEY, start);
        assert_eq!((from.opacity, from.scale_permille), (0, 100));
        let placed = from.place(Rect::new(100, 100, 200, 100));
        assert_eq!(centre(placed), (640, 692));
        assert_eq!(
            t.arriving(KEY, start + Duration::from_millis(180)).opacity,
            255
        );
        assert_eq!(t.arriving(KEY, start + end), Frame::WHOLE);
    }

    #[test]
    fn a_switch_of_workspace_slides_windows_by_the_outputs_width() {
        let rect = Rect::new(100, 100, 200, 100);
        let end = motion::WORKSPACE.settles();
        let going = |forward| {
            let mut t = Transitions::default();
            let start = Duration::from_secs(1);
            t.appear(
                appearance(Change::WorkspaceOut { forward }),
                start,
                Some(snapshot()),
                &PLACE,
            );
            let at = |ms: Duration| t.going(start + ms).next().unwrap().2;
            (at(Duration::ZERO), at(end).place(rect), at(end / 2))
        };
        // To a later workspace the window leaves to the left; to an earlier one, to the right;
        // whole and unfaded as it goes.
        let (first, last, half) = going(true);
        assert_eq!(first.place(rect), rect);
        assert_eq!(last, Rect::new(100 - 1280, 100, 200, 100));
        assert_eq!((half.opacity, half.scale_permille), (255, 1000));
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
        assert_eq!(t.arriving(KEY, start + end), Frame::WHOLE);
    }

    #[test]
    fn a_zoom_draws_the_frame_between_its_rectangles_and_squares_its_corners() {
        let mut t = Transitions::default();
        let start = Duration::from_secs(1);
        let from = Rect::new(190, 60, 900, 600);
        let to = Rect::new(0, 32, 1280, 688);
        t.zoom(KEY, Kind::Window, from, from, to, (16, 0), start);
        let at = |t: &Transitions, ms| t.zooming(KEY, start + Duration::from_millis(ms)).unwrap();
        // Where it was, its corners round; 120 ms in, its 340 ms spring is 1 - (1 + ωt)e^(-ωt)
        // = 0.651 of the way (ωt = 2.218), each edge by its own distance, so width and height
        // scale each by their own factor, its corners 16 · 0.349 = 6; then where it goes,
        // square.
        assert_eq!(at(&t, 0), (from, 16));
        assert_eq!(at(&t, 120), (Rect::new(67, 42, 1147, 657), 6));
        let end = motion::ZOOM.settles();
        assert_eq!(at(&t, end.as_millis() as u64), (to, 0));
        // Its client has not shown the new size yet: it is held there, its old frame stretched.
        t.zoom_shown(KEY, from);
        assert!(t.advance(start + end + Duration::from_millis(50)));
        assert_eq!(at(&t, end.as_millis() as u64 + 50), (to, 0));
        // Shown at its new size, it ends.
        t.zoom_shown(KEY, to);
        assert!(
            t.advance(start + end + Duration::from_millis(60)),
            "its last frame"
        );
        assert!(t.is_empty());
        assert_eq!(t.zooming(KEY, start), None);
        let seen = t.take_seen();
        assert_eq!(
            (seen[0].kind, seen[0].change, seen[0].ended),
            (Kind::Window, Change::Zoomed, true)
        );
        // Restored, its corners round again as it goes back.
        t.zoom(KEY, Kind::Window, to, to, from, (0, 16), start);
        assert_eq!(at(&t, 0), (to, 0));
        assert_eq!(at(&t, 120).1, 10);
        assert_eq!(at(&t, end.as_millis() as u64), (from, 16));
    }

    #[test]
    fn a_zoom_ends_where_its_window_is_if_never_answered_or_moved() {
        let mut t = Transitions::default();
        let start = Duration::from_secs(1);
        let (from, to) = (Rect::new(10, 10, 100, 100), Rect::new(0, 0, 400, 300));
        let end = motion::ZOOM.settles();
        t.zoom(KEY, Kind::Window, from, from, to, (16, 0), start);
        t.zoom_shown(KEY, from);
        assert!(t.advance(start + end + ZOOM_HOLD - Duration::from_millis(1)));
        assert!(!t.is_empty(), "held a second past its end");
        t.advance(start + end + ZOOM_HOLD);
        assert!(t.is_empty());
        // Moved by hand as it zooms: over at once, interrupted.
        t.take_seen();
        t.zoom(KEY, Kind::Window, from, from, to, (16, 0), start);
        t.zoom_shown(KEY, Rect::new(50, 10, 100, 100));
        t.advance(start + Duration::from_millis(20));
        assert!(t.is_empty());
        assert!(!t.take_seen()[0].ended);
        // A zoom of a window zooming begins where it is drawn.
        t.zoom(KEY, Kind::Window, from, from, to, (16, 0), start);
        let (between, radius) = t.zooming(KEY, start + Duration::from_millis(100)).unwrap();
        t.zoom(
            KEY,
            Kind::Window,
            between,
            from,
            from,
            (radius, 16),
            start + Duration::from_millis(100),
        );
        assert_eq!(
            t.zooming(KEY, start + Duration::from_millis(100)),
            Some((between, radius))
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
            (16, 0),
            now,
        );
        assert!(t.is_empty());
        assert_eq!(t.zooming(KEY, now), None);
        assert!(t.take_seen().is_empty());
    }

    #[test]
    fn a_notification_slides_in_from_beside_itself_past_its_place_a_little_and_fades_out() {
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
        // It goes past its place by a few pixels (1.1 % of 396) and comes back.
        let leftmost = (0..600)
            .map(|ms| {
                t.arriving(KEY, start + Duration::from_millis(ms))
                    .place(rect)
                    .x
            })
            .min()
            .unwrap();
        assert!((894..900).contains(&leftmost), "{leftmost}");
        assert_eq!(
            t.arriving(KEY, start + lasts(Kind::Notification, Change::Arrived)),
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
