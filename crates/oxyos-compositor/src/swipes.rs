//! What a touchpad's swipe drives, as the compositor draws it (docs/adr/0046-interactive-gestures.mdx):
//! Mission Control's or App Exposé's surfaces as open as the fingers have them (their openness, 0
//! to 1), and the desktops as slid as the fingers have them (their position, in desktops). While
//! the fingers are down it is exactly where the scene says they are, in the frame composed after
//! their motion; once they lift, a spring takes it from there to where the scene says it goes,
//! at the fingers' speed (`motion::Released`), as a Mac's Mission Control and spaces do.

use crate::motion::{self, Released, Spring};
use oxyos_compositor_core::scene::{ClientId, SwipeDrive};
use oxyos_protocol::GestureOverlay;
use std::time::Duration;

/// How long after its motion ended at closed a swipe keeps the surfaces it drove drawn closed,
/// waiting for the shell to take them away (it was told to close them as the fingers lifted):
/// never shown whole for a frame before they go. A shell that keeps them past this shows them.
pub const CLOSED_HOLD: Duration = Duration::from_secs(1);

/// How the windows catch up with fingers that began before Mission Control or App Exposé was
/// there to draw (the shell's first frame of it comes a few frames after they began): from where
/// the windows are, on this spring, to where the fingers have them, never a jump.
pub const CATCH_UP: Spring = motion::OVERLAY_FADES_IN;

/// Where one swipe's motion is: the fingers', or a spring's from when they lifted, which waits
/// (`started` none) for what it moves to be drawn at all.
#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Fingers(f64),
    Released {
        motion: Released,
        released: Duration,
        started: Option<Duration>,
    },
}

/// A swipe's motion, by the swipe it follows (`gesture`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub gesture: u64,
    state: State,
}

impl Motion {
    /// Follows the scene's value for swipe `gesture` at `now`; `drawn`: where the motion is drawn
    /// now, if it is (its spring, once the fingers lift, starts from there, at their speed).
    fn follow(
        current: Option<Motion>,
        gesture: u64,
        value: f64,
        release: Option<oxyos_compositor_core::scene::SwipeRelease>,
        spring: impl Fn(f64) -> Spring,
        drawn: Option<f64>,
        now: Duration,
    ) -> Motion {
        let state = match (current, release) {
            // Released already: its spring goes on.
            (Some(current), Some(_))
                if current.gesture == gesture
                    && matches!(current.state, State::Released { .. }) =>
            {
                current.state
            }
            // The fingers lifted: from where they left it drawn, at their speed; or, not drawn
            // yet, from nothing once it is.
            (_, Some(release)) => State::Released {
                motion: Released::new(
                    spring(release.to),
                    drawn.unwrap_or(0.0),
                    release.velocity,
                    release.to,
                ),
                released: now,
                started: drawn.map(|_| now),
            },
            (_, None) => State::Fingers(value),
        };
        Motion { gesture, state }
    }

    /// Where it is at `now`.
    pub fn at(&self, now: Duration) -> f64 {
        match self.state {
            State::Fingers(value) => value,
            State::Released {
                motion,
                started: Some(started),
                ..
            } => {
                let elapsed = now.saturating_sub(started);
                if elapsed >= motion.settles() {
                    motion.to
                } else {
                    motion.at(elapsed)
                }
            }
            State::Released { motion, .. } => motion.from,
        }
    }

    /// Whether a spring moves it at `now` (the fingers' motion moves it only as they move).
    pub fn springing(&self, now: Duration) -> bool {
        matches!(self.state, State::Released { motion, started: Some(started), .. }
            if now.saturating_sub(started) < motion.settles())
    }

    /// Whether it is over at `now`: its spring came to rest, `hold` ago at least; or, never drawn,
    /// its fingers let it go closed, or `hold` passed.
    pub fn over(&self, now: Duration, hold: Duration) -> bool {
        match self.state {
            State::Fingers(_) => false,
            State::Released {
                motion,
                started: Some(started),
                ..
            } => now.saturating_sub(started) >= motion.settles() + hold,
            State::Released {
                motion, released, ..
            } => motion.to <= 0.0 || now.saturating_sub(released) >= CLOSED_HOLD,
        }
    }

    /// Where its spring goes, once the fingers lifted.
    pub fn going_to(&self) -> Option<f64> {
        match self.state {
            State::Released { motion, .. } => Some(motion.to),
            State::Fingers(_) => None,
        }
    }
}

/// Mission Control or App Exposé, driven: the shell's surfaces it is (`SetGestureOverlay`).
/// `drawn`: they are drawn (shown when the swipe began, or arrived since); `caught`: when they
/// arrived, the windows catching up with the fingers from there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayDrive {
    pub client: ClientId,
    pub output: oxyos_compositor_core::OutputId,
    pub overlay: GestureOverlay,
    pub motion: Motion,
    drawn: bool,
    caught: Option<Duration>,
}

impl OverlayDrive {
    /// How much of the way to the fingers the windows have caught up at `now`.
    fn caught_up(&self, now: Duration) -> f64 {
        self.caught.map_or(1.0, |since| {
            let elapsed = now.saturating_sub(since);
            if elapsed >= CATCH_UP.settles() {
                1.0
            } else {
                CATCH_UP.at(elapsed).min(1.0)
            }
        })
    }

    /// How open it is drawn at `now`, 0 to 1 (past 1 as its spring arrives, never below 0).
    pub fn openness(&self, now: Duration) -> f64 {
        if !self.drawn {
            return 0.0;
        }
        let value = match self.motion.state {
            State::Fingers(value) => value * self.caught_up(now),
            _ => self.motion.at(now),
        };
        value.max(0.0)
    }

    /// Whether it still drives its surfaces at `now`: until its spring came to rest open, or a
    /// while past rest closed, for the shell to take them away.
    pub fn live(&self, now: Duration) -> bool {
        let hold = if self.motion.going_to() == Some(0.0) {
            CLOSED_HOLD
        } else {
            Duration::ZERO
        };
        !self.motion.over(now, hold)
    }

    /// Whether anything moves it at `now` but the fingers: its spring, or its catching up.
    fn springing(&self, now: Duration) -> bool {
        self.motion.springing(now)
            || matches!(self.motion.state, State::Fingers(_))
                && self
                    .caught
                    .is_some_and(|since| now.saturating_sub(since) < CATCH_UP.settles())
    }
}

/// The swipes' motions: Mission Control's or App Exposé's, and the desktops'.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Drives {
    pub overlay: Option<OverlayDrive>,
    pub spaces: Option<Motion>,
}

impl Drives {
    /// Follows the scene's latest swipe at `now`: a new one replaces the motion of the last of
    /// its kind; the fingers lifting start its spring, once. `shown`: whether the shell shows the
    /// overlay a new swipe drives already (one being closed), drawn from the first frame.
    pub fn follow(
        &mut self,
        drive: Option<SwipeDrive>,
        now: Duration,
        shown: impl Fn(ClientId, GestureOverlay) -> bool,
    ) {
        match drive {
            Some(SwipeDrive::Overlay {
                gesture,
                client,
                output,
                overlay,
                openness,
                release,
            }) => {
                let same = self.overlay.filter(|drive| drive.motion.gesture == gesture);
                let (drawn, caught) =
                    same.map_or_else(|| (shown(client, overlay), None), |d| (d.drawn, d.caught));
                // Where the fingers have it drawn now, as far as the windows caught up with them.
                let caught_up = same.map_or(1.0, |drive| drive.caught_up(now));
                let at = drawn.then_some(openness * caught_up);
                let motion = Motion::follow(
                    same.map(|drive| drive.motion),
                    gesture,
                    openness,
                    release,
                    // Open on the overlay's arrival, closed on its going (ADR 0011).
                    |to| {
                        if to >= 1.0 {
                            motion::OVERLAY_ARRIVES
                        } else {
                            motion::OVERLAY_GOES
                        }
                    },
                    at,
                    now,
                );
                self.overlay = Some(OverlayDrive {
                    client,
                    output,
                    overlay,
                    motion,
                    drawn,
                    caught,
                });
            }
            Some(SwipeDrive::Spaces {
                gesture,
                position,
                release,
            }) => {
                let drawn = self
                    .spaces
                    .filter(|motion| motion.gesture == gesture)
                    .map_or(position, |motion| motion.at(now));
                self.spaces = Some(Motion::follow(
                    self.spaces.filter(|motion| motion.gesture == gesture),
                    gesture,
                    position,
                    release,
                    |_| motion::WORKSPACE,
                    Some(drawn),
                    now,
                ));
            }
            None => {}
        }
    }

    /// The overlay a swipe drives is drawn whole at `now`, its last surface arrived (the shell
    /// draws App Exposé's backdrop and the surface its windows are moved into one after the other,
    /// a loaded machine tens of milliseconds apart): from now on it is drawn, the windows catching
    /// up with the fingers from where they are (or the spring of fingers that already lifted
    /// starting from there), never from part of the way.
    pub fn overlay_arrived(&mut self, now: Duration) {
        let Some(drive) = self.overlay.as_mut() else {
            return;
        };
        if drive.drawn {
            return;
        }
        drive.drawn = true;
        drive.caught = Some(now);
        if let State::Released { started, .. } = &mut drive.motion.state {
            started.get_or_insert(now);
        }
    }

    /// The overlay drive still driving at `now`.
    pub fn live_overlay(&self, now: Duration) -> Option<OverlayDrive> {
        self.overlay.filter(|drive| drive.live(now))
    }

    /// Where the desktops are slid at `now`, while a swipe's motion takes them.
    pub fn spaces_at(&self, now: Duration) -> Option<f64> {
        self.spaces
            .filter(|motion| !motion.over(now, Duration::ZERO))
            .map(|motion| motion.at(now))
    }

    /// Whether a spring moves anything at `now`.
    pub fn springing(&self, now: Duration) -> bool {
        self.overlay.is_some_and(|drive| drive.springing(now))
            || self.spaces.is_some_and(|motion| motion.springing(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyos_compositor_core::scene::SwipeRelease;

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    fn overlay(gesture: u64, openness: f64, release: Option<SwipeRelease>) -> SwipeDrive {
        SwipeDrive::Overlay {
            gesture,
            client: 7,
            output: 1,
            overlay: GestureOverlay::MissionControl,
            openness,
            release,
        }
    }

    #[test]
    fn the_fingers_are_drawn_where_they_are_and_their_release_springs_on_from_there() {
        let mut drives = Drives::default();
        let start = Duration::from_secs(5);
        for (i, openness) in [0.25, 0.5, 0.75].into_iter().enumerate() {
            let now = start + ms(16 * i as u64);
            drives.follow(Some(overlay(1, openness, None)), now, |_, _| true);
            let drive = drives.live_overlay(now).unwrap();
            // Exactly where the fingers are, in the frame after their motion: no spring between.
            assert_eq!(drive.openness(now), openness);
            assert_eq!(drive.openness(now + ms(100)), openness);
            assert!(!drives.springing(now));
        }
        // Let go at 0.75 going on at 2 a second: on from there, ahead of a spring from rest.
        let lifted = start + ms(48);
        let release = Some(SwipeRelease {
            to: 1.0,
            velocity: 2.0,
        });
        drives.follow(Some(overlay(1, 0.75, release)), lifted, |_, _| true);
        let drive = drives.live_overlay(lifted).unwrap();
        assert_eq!(drive.openness(lifted), 0.75);
        let still = Released::new(motion::OVERLAY_ARRIVES, 0.75, 0.0, 1.0);
        assert!(drive.openness(lifted + ms(16)) > still.at(ms(16)) + 0.75 * 0.0);
        assert!(drives.springing(lifted + ms(16)));
        // The scene says the same release each frame: the spring is not begun again.
        drives.follow(Some(overlay(1, 0.75, release)), lifted + ms(32), |_, _| {
            true
        });
        assert_eq!(drives.overlay.unwrap().motion, drive.motion);
        // At rest open, it drives nothing more.
        let settles = Released::new(motion::OVERLAY_ARRIVES, 0.75, 2.0, 1.0).settles();
        assert_eq!(drive.openness(lifted + settles), 1.0);
        assert!(drives.live_overlay(lifted + settles).is_none());
        assert!(!drives.springing(lifted + settles));
    }

    #[test]
    fn opening_the_windows_catch_up_with_the_fingers_once_the_shell_draws_it() {
        let mut drives = Drives::default();
        // The fingers at 0.2 before the shell drew App Exposé: nothing drawn yet.
        drives.follow(Some(overlay(1, 0.2, None)), ms(0), |_, _| false);
        assert_eq!(drives.live_overlay(ms(0)).unwrap().openness(ms(0)), 0.0);
        // It arrives at 50 ms, the fingers at 0.3: the windows set off from where they are and
        // catch up, every frame further, never past the fingers.
        drives.follow(Some(overlay(1, 0.3, None)), ms(50), |_, _| false);
        drives.overlay_arrived(ms(50));
        let path: Vec<f64> = (50..=400)
            .map(|t| drives.live_overlay(ms(t)).unwrap().openness(ms(t)))
            .collect();
        assert_eq!(path[0], 0.0);
        assert!(path.windows(2).all(|w| w[1] >= w[0] && w[1] <= 0.3));
        assert!(drives.springing(ms(100)));
        assert_eq!(*path.last().unwrap(), 0.3);
        assert!(!drives.springing(ms(400)));
        // Fingers that lifted before it was drawn: its spring starts from nothing once it is.
        let mut early = Drives::default();
        let release = Some(SwipeRelease {
            to: 1.0,
            velocity: 4.0,
        });
        early.follow(Some(overlay(2, 0.25, release)), ms(0), |_, _| false);
        assert_eq!(early.live_overlay(ms(30)).unwrap().openness(ms(30)), 0.0);
        early.overlay_arrived(ms(40));
        let drive = early.live_overlay(ms(40)).unwrap();
        assert_eq!(drive.openness(ms(40)), 0.0);
        assert!(drive.openness(ms(56)) > 0.0);
        // Let go closed before it was ever drawn: over at once.
        let mut never = Drives::default();
        let back = Some(SwipeRelease {
            to: 0.0,
            velocity: 0.0,
        });
        never.follow(Some(overlay(3, 0.1, back)), ms(0), |_, _| false);
        assert!(never.live_overlay(ms(1)).is_none());
    }

    #[test]
    fn closed_it_holds_its_surfaces_closed_until_the_shell_takes_them_away() {
        let mut drives = Drives::default();
        let release = Some(SwipeRelease {
            to: 0.0,
            velocity: -0.5,
        });
        drives.follow(Some(overlay(2, 0.3, release)), ms(0), |_, _| true);
        let settles = Released::new(motion::OVERLAY_GOES, 0.3, -0.5, 0.0).settles();
        let drive = drives.live_overlay(settles).unwrap();
        assert_eq!(drive.openness(settles), 0.0);
        assert!(drives.live_overlay(settles + CLOSED_HOLD - ms(1)).is_some());
        assert!(drives.live_overlay(settles + CLOSED_HOLD).is_none());
        // A new swipe replaces it, from where its fingers are.
        drives.follow(Some(overlay(3, 0.1, None)), ms(50), |_, _| true);
        assert_eq!(drives.live_overlay(ms(50)).unwrap().openness(ms(50)), 0.1);
    }

    #[test]
    fn the_desktops_slide_with_the_fingers_and_on_to_the_one_they_brought_in() {
        let mut drives = Drives::default();
        drives.follow(
            Some(SwipeDrive::Spaces {
                gesture: 1,
                position: 0.4,
                release: None,
            }),
            ms(0),
            |_, _| false,
        );
        assert_eq!(drives.spaces_at(ms(10)), Some(0.4));
        drives.follow(
            Some(SwipeDrive::Spaces {
                gesture: 1,
                position: 0.6,
                release: Some(SwipeRelease {
                    to: 1.0,
                    velocity: 1.5,
                }),
            }),
            ms(20),
            |_, _| false,
        );
        let path: Vec<f64> = (20..600)
            .map(|t| drives.spaces_at(ms(t)).unwrap_or(1.0))
            .collect();
        assert!(path.windows(2).all(|w| w[1] >= w[0] - 1e-9));
        assert_eq!(drives.spaces_at(ms(2000)), None);
    }
}
