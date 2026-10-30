//! The session's idle lock (docs/adr/0010-lock-screen.mdx, step 4): with no input from the person
//! for the time the user's `lock_after` says (docs/adr/0019-settings.mdx), the session locks, as
//! a Mac asks for its password once it has been left alone.
//!
//! Only the person's input counts (`Scene::activity`: keys, the pointer, scrolls, gestures,
//! touches), never what clients draw or the time passing. The lock's deadline is computed, never
//! polled: the compositor's turn waits until it at most, and not at all while nothing can lock
//! (`lock_after` 0, idle held back, the session idle already). The same deadline is when the
//! session tells logind it is idle (`IdleHint`), and any input tells it it no longer is.
//!
//! The deadline is armed again by input, by `lock_after` changing, by unlocking, and when idle is
//! no longer held back (a video ended: the session does not lock the moment it does). Locked
//! already, reaching it locks nothing again; the session is still idle for logind.

use std::time::Duration;

#[derive(Debug, Default)]
pub struct Idle {
    /// How long with no input before the session locks; `None`, never (`lock_after = 0`).
    after: Option<Duration>,
    /// The scene's count of input when last looked at.
    activity: u64,
    /// When the deadline was last armed, on the compositor's clock (shifted by `passed`).
    since: Duration,
    /// The session's user holds idle back (logind's idle inhibitor, a video playing).
    held: bool,
    /// The session is idle: the deadline passed with no input, and nothing holds idle back.
    idle: bool,
    /// The lock as last seen, to arm the deadline again on unlocking.
    locked: bool,
    /// Time the development compositor said passed with no input (`pass`).
    passed: Duration,
}

/// What a turn of the idle lock asks of the compositor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Turn {
    /// Lock the session: its deadline passed with no input.
    pub lock: bool,
    /// Tell logind the session is idle (`Some(true)`) or no longer is.
    pub idle: Option<bool>,
}

impl Idle {
    /// Locks after `minutes` with no input (0: never), the deadline armed from `now` if that
    /// changed. Applied as the session starts and whenever the user changes `lock_after`.
    pub fn set_lock_after(&mut self, minutes: u32, now: Duration) {
        let after = (minutes > 0).then(|| Duration::from_secs(u64::from(minutes) * 60));
        if after != self.after {
            self.after = after;
            self.since = now + self.passed;
        }
    }

    /// How long the session locks after (`None`: never).
    pub fn lock_after(&self) -> Option<Duration> {
        self.after
    }

    /// The session's user began or stopped holding idle back; when they stop, the deadline is
    /// armed from `now`.
    pub fn set_held(&mut self, held: bool, now: Duration) {
        if self.held && !held {
            self.since = now + self.passed;
        }
        self.held = held;
    }

    /// The development compositor's: `time` passes at once, with no input.
    pub fn pass(&mut self, time: Duration) {
        self.passed += time;
    }

    /// When the next turn has something to do on its own, on the compositor's clock: the
    /// deadline, while one is armed. `None` while nothing can lock or make the session idle.
    pub fn deadline(&self) -> Option<Duration> {
        if self.idle || self.held {
            return None;
        }
        let deadline = self.since + self.after?;
        Some(deadline.saturating_sub(self.passed))
    }

    /// One turn, at `now`, with the scene's `activity` and whether it is `locked`.
    pub fn turn(&mut self, activity: u64, locked: bool, now: Duration) -> Turn {
        let now = now + self.passed;
        if activity != self.activity || (self.locked && !locked) {
            self.activity = activity;
            self.since = now;
        }
        let idle = !self.held
            && self
                .after
                .is_some_and(|after| now >= self.since.saturating_add(after));
        let turn = Turn {
            lock: idle && !locked,
            idle: (idle != self.idle).then_some(idle),
        };
        self.idle = idle;
        self.locked = locked || turn.lock;
        turn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: Duration = Duration::from_secs(1_000);
    const MINUTE: Duration = Duration::from_secs(60);
    const NOTHING: Turn = Turn {
        lock: false,
        idle: None,
    };
    const LOCKS: Turn = Turn {
        lock: true,
        idle: Some(true),
    };

    /// Locking after five minutes from `START`, with no input yet.
    fn five_minutes() -> Idle {
        let mut idle = Idle::default();
        idle.set_lock_after(5, START);
        assert_eq!(idle.turn(0, false, START), NOTHING);
        idle
    }

    #[test]
    fn it_locks_exactly_at_the_deadline_and_then_waits_for_nothing() {
        let mut idle = five_minutes();
        let deadline = START + 5 * MINUTE;
        assert_eq!(idle.deadline(), Some(deadline));
        let just_before = deadline - Duration::from_nanos(1);
        assert_eq!(idle.turn(0, false, just_before), NOTHING);
        assert_eq!(idle.turn(0, false, deadline), LOCKS);
        // Locked and idle: no deadline left to wake for, and nothing more to do.
        assert_eq!(idle.deadline(), None);
        assert_eq!(idle.turn(0, true, deadline + 60 * MINUTE), NOTHING);
    }

    #[test]
    fn any_input_arms_the_deadline_again() {
        let mut idle = five_minutes();
        let input = START + 4 * MINUTE;
        assert_eq!(idle.turn(1, false, input), NOTHING);
        assert_eq!(idle.deadline(), Some(input + 5 * MINUTE));
        // Where the first deadline was, nothing.
        assert_eq!(idle.turn(1, false, START + 5 * MINUTE), NOTHING);
        assert_eq!(idle.turn(1, false, input + 5 * MINUTE), LOCKS);
    }

    #[test]
    fn input_while_idle_tells_logind_it_no_longer_is() {
        let mut idle = five_minutes();
        let deadline = START + 5 * MINUTE;
        assert_eq!(idle.turn(0, false, deadline), LOCKS);
        // The person comes back to the lock screen: no longer idle, the session still locked.
        let back = deadline + MINUTE;
        let turn = idle.turn(1, true, back);
        assert_eq!(
            turn,
            Turn {
                lock: false,
                idle: Some(false)
            }
        );
        // Idle again on the lock screen: logind is told, and nothing locks again.
        assert_eq!(idle.deadline(), Some(back + 5 * MINUTE));
        assert_eq!(
            idle.turn(1, true, back + 5 * MINUTE),
            Turn {
                lock: false,
                idle: Some(true)
            }
        );
    }

    #[test]
    fn zero_never_locks() {
        let mut idle = Idle::default();
        idle.set_lock_after(0, START);
        assert_eq!(idle.deadline(), None);
        assert_eq!(idle.turn(0, false, START + 24 * 60 * MINUTE), NOTHING);
        // Nor does a compositor nothing set it for (the development compositor's own run).
        let mut unset = Idle::default();
        assert_eq!(unset.deadline(), None);
        assert_eq!(unset.turn(0, false, START + 24 * 60 * MINUTE), NOTHING);
    }

    #[test]
    fn changing_lock_after_arms_the_deadline_again() {
        let mut idle = five_minutes();
        // Four minutes in, the user sets it to two: two more from the change, not from START.
        let change = START + 4 * MINUTE;
        idle.set_lock_after(2, change);
        assert_eq!(idle.deadline(), Some(change + 2 * MINUTE));
        assert_eq!(idle.turn(0, false, START + 5 * MINUTE), NOTHING);
        // The same value again changes nothing.
        idle.set_lock_after(2, change + MINUTE);
        assert_eq!(idle.deadline(), Some(change + 2 * MINUTE));
        // Turned off before it, and on again: armed from when it is.
        idle.set_lock_after(0, change + MINUTE);
        assert_eq!(idle.turn(0, false, change + 10 * MINUTE), NOTHING);
        idle.set_lock_after(1, change + 10 * MINUTE);
        assert_eq!(idle.turn(0, false, change + 10 * MINUTE), NOTHING);
        assert_eq!(idle.turn(0, false, change + 11 * MINUTE), LOCKS);
    }

    #[test]
    fn locked_already_it_locks_nothing_and_unlocking_arms_it_again() {
        let mut idle = five_minutes();
        // Locked by its shortcut a minute in: the deadline passing locks nothing again.
        assert_eq!(idle.turn(0, true, START + MINUTE), NOTHING);
        assert_eq!(
            idle.turn(0, true, START + 5 * MINUTE),
            Turn {
                lock: false,
                idle: Some(true)
            }
        );
        // Unlocked (only a password does it, and the keys typed count anyway): five minutes
        // from the unlock, even with no input counted.
        let unlocked = START + 20 * MINUTE;
        assert_eq!(
            idle.turn(0, false, unlocked),
            Turn {
                lock: false,
                idle: Some(false)
            }
        );
        assert_eq!(idle.deadline(), Some(unlocked + 5 * MINUTE));
        assert_eq!(idle.turn(0, false, unlocked + 5 * MINUTE), LOCKS);
    }

    #[test]
    fn idle_held_back_never_locks_and_its_end_arms_the_deadline() {
        let mut idle = five_minutes();
        idle.set_held(true, START + MINUTE);
        assert_eq!(idle.deadline(), None);
        assert_eq!(idle.turn(0, false, START + 120 * MINUTE), NOTHING);
        // The video ends two hours in: five minutes from then, not at once.
        let ended = START + 120 * MINUTE;
        idle.set_held(false, ended);
        assert_eq!(idle.deadline(), Some(ended + 5 * MINUTE));
        assert_eq!(idle.turn(0, false, ended + MINUTE), NOTHING);
        assert_eq!(idle.turn(0, false, ended + 5 * MINUTE), LOCKS);
        // Held back while idle (locked): no longer idle, nothing unlocks.
        idle.set_held(true, ended + 6 * MINUTE);
        assert_eq!(
            idle.turn(0, true, ended + 6 * MINUTE),
            Turn {
                lock: false,
                idle: Some(false)
            }
        );
    }

    #[test]
    fn time_said_to_pass_counts_as_time_with_no_input() {
        let mut idle = five_minutes();
        idle.pass(4 * MINUTE);
        assert_eq!(idle.deadline(), Some(START + MINUTE));
        assert_eq!(idle.turn(0, false, START), NOTHING);
        idle.pass(MINUTE);
        assert_eq!(idle.turn(0, false, START), LOCKS);
    }
}
