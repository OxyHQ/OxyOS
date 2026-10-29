//! Surfaces arriving and going, animated (docs/adr/0011-transitions.mdx): a window grows a
//! little and fades in, a menu fades out, the launcher settles, the lock screen fades. What a
//! surface shows is never changed: it is drawn whole, scaled about its centre, at an opacity.
//! A surface going away is drawn from its last frame, which the compositor keeps until its
//! transition ends.

use oxyos_compositor_core::{AppearanceKind as Kind, SurfaceKey};
use oxyos_protocol::{Format, Rect, Size};
use std::collections::BTreeMap;
use std::time::Duration;

/// How a surface of a kind arrives or goes: how long, and the scale it starts from (arriving)
/// or ends at (going), in thousandths. A zero duration: at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Motion {
    pub duration: Duration,
    pub scale_permille: u32,
}

const fn motion(ms: u64, scale_permille: u32) -> Motion {
    Motion {
        duration: Duration::from_millis(ms),
        scale_permille,
    }
}

/// The ADR's motions: (arriving, going).
pub fn motions(kind: Kind) -> (Motion, Motion) {
    match kind {
        Kind::Window => (motion(180, 960), motion(150, 960)),
        // As a Mac's menus: at once, then fading out.
        Kind::Popup => (motion(0, 1000), motion(150, 1000)),
        Kind::Overlay => (motion(200, 1040), motion(150, 1040)),
        Kind::Panel => (motion(250, 1000), motion(0, 1000)),
        Kind::Lock => (motion(250, 1000), motion(250, 1000)),
        Kind::Candidates => (motion(0, 1000), motion(0, 1000)),
    }
}

/// A surface's last frame, kept while it goes.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub rect: Rect,
    pub extent: Size,
    pub format: Format,
    pub pixels: Vec<u8>,
}

struct Running {
    kind: Kind,
    started: Duration,
    motion: Motion,
}

/// Where a surface is in its transition: the opacity (0 to 255) and scale (thousandths) to draw
/// it at now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub opacity: u8,
    pub scale_permille: u32,
}

impl Frame {
    pub const WHOLE: Frame = Frame {
        opacity: 255,
        scale_permille: 1000,
    };

    /// `rect` drawn at this scale about its centre.
    pub fn place(&self, rect: Rect) -> Rect {
        if self.scale_permille == 1000 {
            return rect;
        }
        let scaled = |length: u32| {
            ((u64::from(length) * u64::from(self.scale_permille) + 500) / 1000) as u32
        };
        let (width, height) = (scaled(rect.width).max(1), scaled(rect.height).max(1));
        Rect::new(
            rect.x + (rect.width as i32 - width as i32) / 2,
            rect.y + (rect.height as i32 - height as i32) / 2,
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

/// The transitions running: surfaces arriving, and surfaces going with their last frames, in
/// the order they began going.
#[derive(Default)]
pub struct Transitions {
    arriving: BTreeMap<SurfaceKey, Running>,
    going: Vec<(SurfaceKey, Running, Snapshot)>,
}

impl Transitions {
    pub fn arrive(&mut self, key: SurfaceKey, kind: Kind, now: Duration) {
        // A surface arriving again (restored) stops going.
        self.going.retain(|(going, _, _)| *going != key);
        let motion = motions(kind).0;
        if motion.duration.is_zero() {
            self.arriving.remove(&key);
            return;
        }
        self.arriving.insert(
            key,
            Running {
                kind,
                started: now,
                motion,
            },
        );
    }

    /// `key` goes, drawn from `snapshot` (its last frame shown); without one it was not shown,
    /// and nothing is drawn.
    pub fn go(&mut self, key: SurfaceKey, kind: Kind, now: Duration, snapshot: Option<Snapshot>) {
        self.arriving.remove(&key);
        let motion = motions(kind).1;
        let Some(snapshot) = snapshot.filter(|_| !motion.duration.is_zero()) else {
            return;
        };
        self.going.retain(|(going, _, _)| *going != key);
        self.going.push((
            key,
            Running {
                kind,
                started: now,
                motion,
            },
            snapshot,
        ));
    }

    /// A frame of the compositor's own (the session's splash) fades out over what is shown, 400
    /// ms; it goes as a panel does, so a lock would drop it.
    pub fn fade(&mut self, snapshot: Snapshot, now: Duration) {
        let key = SurfaceKey {
            client: u32::MAX,
            id: u32::MAX,
        };
        self.going.retain(|(going, _, _)| *going != key);
        self.going.push((
            key,
            Running {
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
        let from = f64::from(running.motion.scale_permille);
        Frame {
            opacity: (255.0 * p).round() as u8,
            scale_permille: (from + (1000.0 - from) * p).round() as u32,
        }
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
            let to = f64::from(running.motion.scale_permille);
            (
                snapshot,
                running.kind,
                Frame {
                    opacity: (255.0 * (1.0 - p)).round() as u8,
                    scale_permille: (1000.0 + (to - 1000.0) * p).round() as u32,
                },
            )
        })
    }

    /// Forgets the transitions that ended by `now`; true if any runs (or ended, its last frame
    /// still to compose).
    pub fn advance(&mut self, now: Duration) -> bool {
        let before = self.arriving.len() + self.going.len();
        self.arriving
            .retain(|_, running| now.saturating_sub(running.started) < running.motion.duration);
        self.going.retain(|(_, running, _)| {
            now.saturating_sub(running.started) < running.motion.duration
        });
        before > 0
    }

    /// Drops every surface going but the lock's: locked, nothing of the session is drawn.
    pub fn lock(&mut self) {
        self.going
            .retain(|(_, running, _)| running.kind == Kind::Lock);
    }

    pub fn is_empty(&self) -> bool {
        self.arriving.is_empty() && self.going.is_empty()
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
            pixels: vec![0; 200 * 100 * 4],
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
                scale_permille: 960
            }
        );
        // Halfway in time, 87.5 % of the way (1 - 0.5³).
        assert_eq!(
            at(&t, 90),
            Frame {
                opacity: 223,
                scale_permille: 995
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
    fn a_surface_is_scaled_about_its_centre() {
        let frame = Frame {
            opacity: 255,
            scale_permille: 960,
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
}
