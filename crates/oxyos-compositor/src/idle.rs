//! The session's idle lock (docs/adr/0010-lock-screen.mdx, step 4): with no input from the person
//! for the time the user's `lock_after` says (docs/adr/0019-settings.mdx), the session locks, as
//! a Mac asks for its password once it has been left alone.
//!
//! And the display's and the machine's sleep (docs/adr/0057-battery-and-energy.mdx, decision 6),
//! on the same input: the display dims a little before it turns off, turns off after the user's
//! timer for the power source (`display_off_battery`, `display_off_adapter`), and the machine
//! sleeps after its own (`sleep_battery`, `sleep_adapter`), never before the display is off. Input
//! brings the display back at once, as it was.
//!
//! Only the person's input counts (`Scene::activity`: keys, the pointer, scrolls, gestures,
//! touches), never what clients draw or the time passing. Each deadline is computed, never
//! polled: the compositor's turn waits until the next at most, and not at all while nothing is
//! left to do (`lock_after` 0, idle held back, the session idle already). The lock's deadline is
//! when the session tells logind it is idle (`IdleHint`), and any input tells it it no longer is.
//!
//! The lock's deadline is armed again by input, by `lock_after` changing, by unlocking, and when
//! idle is no longer held back (a video ended: the session does not lock the moment it does).
//! Locked already, reaching it locks nothing again; the session is still idle for logind. The
//! display's and the machine's are armed again by input, by their timers changing (the power
//! source changing changes them), by idle no longer held back and by the machine waking. While a
//! program holds idle back (logind's idle inhibitor: a video playing), the display stays on and
//! the machine awake.

use std::time::Duration;

/// How long before it turns off the display dims, at most (half its timer when that is shorter).
pub const DIM_BEFORE: Duration = Duration::from_secs(10);

/// How far the display and the machine went for want of input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    #[default]
    Awake,
    /// The display dimmed, about to turn off.
    Dimmed,
    /// The display off.
    Off,
    /// The machine asked to sleep (the display off with it).
    Asleep,
}

#[derive(Debug, Default)]
pub struct Idle {
    /// How long with no input before the session locks; `None`, never (`lock_after = 0`).
    after: Option<Duration>,
    /// How long before the display turns off and the machine sleeps; `None`, never.
    display_off: Option<Duration>,
    sleep: Option<Duration>,
    /// The scene's count of input when last looked at.
    activity: u64,
    /// When the lock's deadline was last armed, on the compositor's clock (shifted by `passed`).
    since: Duration,
    /// When the display's and the machine's were.
    quiet_since: Duration,
    /// The session's user holds idle back (logind's idle inhibitor, a video playing).
    held: bool,
    /// The session is idle: the deadline passed with no input, and nothing holds idle back.
    idle: bool,
    phase: Phase,
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
    /// Dim the display (`Some(true)`) or bring it back as it was.
    pub dim: Option<bool>,
    /// Turn the displays off (`Some(false)`) or on again.
    pub display: Option<bool>,
    /// Put the machine to sleep.
    pub sleep: bool,
}

fn minutes(minutes: u32) -> Option<Duration> {
    (minutes > 0).then(|| Duration::from_secs(u64::from(minutes) * 60))
}

impl Idle {
    /// Locks after `minutes` with no input (0: never), the deadline armed from `now` if that
    /// changed. Applied as the session starts and whenever the user changes `lock_after`.
    pub fn set_lock_after(&mut self, minutes: u32, now: Duration) {
        let after = self::minutes(minutes);
        if after != self.after {
            self.after = after;
            self.since = now + self.passed;
        }
    }

    /// How long the session locks after (`None`: never).
    pub fn lock_after(&self) -> Option<Duration> {
        self.after
    }

    /// The display turns off after `display_off` minutes with no input and the machine sleeps
    /// after `sleep` (0: never), for the power source the machine is on now; armed from `now` if
    /// they changed. The machine never sleeps before the display is off.
    pub fn set_energy(&mut self, display_off: u32, sleep: u32, now: Duration) {
        let (display_off, sleep) = (minutes(display_off), minutes(sleep));
        if (display_off, sleep) != (self.display_off, self.sleep) {
            self.display_off = display_off;
            self.sleep = sleep;
            self.quiet_since = now + self.passed;
        }
    }

    /// The display's timer and the machine's, as set.
    pub fn energy(&self) -> (Option<Duration>, Option<Duration>) {
        (self.display_off, self.sleep)
    }

    /// The session's user began or stopped holding idle back; when they stop, the deadlines are
    /// armed from `now`.
    pub fn set_held(&mut self, held: bool, now: Duration) {
        if self.held && !held {
            self.since = now + self.passed;
            self.quiet_since = now + self.passed;
        }
        self.held = held;
    }

    /// The machine woke from sleep at `now`: the display is back on, and its timers start again.
    pub fn woke(&mut self, now: Duration) {
        self.quiet_since = now + self.passed;
    }

    /// The development compositor's: `time` passes at once, with no input.
    pub fn pass(&mut self, time: Duration) {
        self.passed += time;
    }

    /// How far the display and the machine went.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// When each phase begins, after `quiet_since`: the dim, the display off, the sleep.
    fn phases(&self) -> [Option<Duration>; 3] {
        let off = self.display_off;
        let dim = off.map(|off| off.saturating_sub(DIM_BEFORE.min(off / 2)));
        let sleep = self
            .sleep
            .map(|sleep| off.map_or(sleep, |off| sleep.max(off)));
        [dim, off, sleep].map(|after| after.map(|after| self.quiet_since.saturating_add(after)))
    }

    /// The phase due at `now` (shifted).
    fn due(&self, now: Duration) -> Phase {
        if self.held {
            return Phase::Awake;
        }
        let [dim, off, sleep] = self.phases();
        let reached = |at: Option<Duration>| at.is_some_and(|at| now >= at);
        if reached(sleep) {
            Phase::Asleep
        } else if reached(off) {
            Phase::Off
        } else if reached(dim) {
            Phase::Dimmed
        } else {
            Phase::Awake
        }
    }

    /// When the next turn has something to do on its own, on the compositor's clock: the
    /// soonest deadline armed. `None` while nothing can lock, make the session idle, dim, turn
    /// the display off or put the machine to sleep.
    pub fn deadline(&self) -> Option<Duration> {
        if self.held {
            return None;
        }
        let lock = (!self.idle)
            .then(|| self.after.map(|after| self.since + after))
            .flatten();
        let [dim, off, sleep] = self.phases();
        let next = match self.phase {
            Phase::Awake => dim.or(off).or(sleep),
            Phase::Dimmed => off.or(sleep),
            Phase::Off => sleep,
            Phase::Asleep => None,
        };
        [lock, next]
            .into_iter()
            .flatten()
            .min()
            .map(|deadline| deadline.saturating_sub(self.passed))
    }

    /// One turn, at `now`, with the scene's `activity` and whether it is `locked`.
    pub fn turn(&mut self, activity: u64, locked: bool, now: Duration) -> Turn {
        let now = now + self.passed;
        if activity != self.activity {
            self.activity = activity;
            self.since = now;
            self.quiet_since = now;
        } else if self.locked && !locked {
            self.since = now;
        }
        let idle = !self.held
            && self
                .after
                .is_some_and(|after| now >= self.since.saturating_add(after));
        let phase = self.due(now);
        let changed = |at: Phase| {
            let (was, is) = (self.phase >= at, phase >= at);
            (was != is).then_some(is)
        };
        let turn = Turn {
            lock: idle && !locked,
            idle: (idle != self.idle).then_some(idle),
            dim: changed(Phase::Dimmed),
            display: changed(Phase::Off).map(|off| !off),
            sleep: phase == Phase::Asleep && self.phase != Phase::Asleep,
        };
        self.idle = idle;
        self.phase = phase;
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
        dim: None,
        display: None,
        sleep: false,
    };
    const LOCKS: Turn = Turn {
        lock: true,
        idle: Some(true),
        ..NOTHING
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
                idle: Some(false),
                ..NOTHING
            }
        );
        // Idle again on the lock screen: logind is told, and nothing locks again.
        assert_eq!(idle.deadline(), Some(back + 5 * MINUTE));
        assert_eq!(
            idle.turn(1, true, back + 5 * MINUTE),
            Turn {
                lock: false,
                idle: Some(true),
                ..NOTHING
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
                idle: Some(true),
                ..NOTHING
            }
        );
        // Unlocked (only a password does it, and the keys typed count anyway): five minutes
        // from the unlock, even with no input counted.
        let unlocked = START + 20 * MINUTE;
        assert_eq!(
            idle.turn(0, false, unlocked),
            Turn {
                lock: false,
                idle: Some(false),
                ..NOTHING
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
                idle: Some(false),
                ..NOTHING
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

    const SECOND: Duration = Duration::from_secs(1);

    /// On battery as a Mac's defaults: the display off after two minutes, asleep after ten; the
    /// lock after five.
    fn on_battery() -> Idle {
        let mut idle = five_minutes();
        idle.set_energy(2, 10, START);
        idle
    }

    #[test]
    fn the_display_dims_turns_off_and_the_machine_sleeps_on_their_deadlines() {
        let mut idle = on_battery();
        // Ten seconds before two minutes, it dims.
        let dim = START + 2 * MINUTE - 10 * SECOND;
        assert_eq!(idle.deadline(), Some(dim));
        assert_eq!(idle.turn(0, false, dim - SECOND), NOTHING);
        assert_eq!(
            idle.turn(0, false, dim),
            Turn {
                dim: Some(true),
                ..NOTHING
            }
        );
        assert_eq!(idle.phase(), Phase::Dimmed);
        assert_eq!(idle.deadline(), Some(START + 2 * MINUTE));
        assert_eq!(
            idle.turn(0, false, START + 2 * MINUTE),
            Turn {
                display: Some(false),
                ..NOTHING
            }
        );
        // The lock at five, the display off already.
        assert_eq!(idle.deadline(), Some(START + 5 * MINUTE));
        assert_eq!(idle.turn(0, false, START + 5 * MINUTE), LOCKS);
        assert_eq!(idle.deadline(), Some(START + 10 * MINUTE));
        assert_eq!(
            idle.turn(0, true, START + 10 * MINUTE),
            Turn {
                sleep: true,
                ..NOTHING
            }
        );
        // Asleep: nothing left to wake for, and never asked to sleep again.
        assert_eq!(idle.deadline(), None);
        assert_eq!(idle.turn(0, true, START + 60 * MINUTE), NOTHING);
        // It woke (the lid opened): the display on, its timers from then.
        let woke = START + 61 * MINUTE;
        idle.woke(woke);
        assert_eq!(
            idle.turn(0, true, woke),
            Turn {
                dim: Some(false),
                display: Some(true),
                ..NOTHING
            }
        );
        assert_eq!(idle.deadline(), Some(woke + 2 * MINUTE - 10 * SECOND));
    }

    #[test]
    fn input_brings_the_display_back_as_it_was() {
        let mut idle = on_battery();
        assert_eq!(idle.turn(0, false, START + 3 * MINUTE).display, Some(false));
        // Time skipped past the dim turns the display off and dims it in one turn.
        assert_eq!(idle.phase(), Phase::Off);
        let back = START + 3 * MINUTE + SECOND;
        assert_eq!(
            idle.turn(1, false, back),
            Turn {
                dim: Some(false),
                display: Some(true),
                ..NOTHING
            }
        );
        assert_eq!(idle.phase(), Phase::Awake);
        assert_eq!(idle.deadline(), Some(back + 2 * MINUTE - 10 * SECOND));
    }

    #[test]
    fn the_machine_never_sleeps_before_the_display_is_off_and_never_means_never() {
        let mut idle = five_minutes();
        // Asleep after one minute, the display off after three: asleep at three.
        idle.set_energy(3, 1, START);
        assert!(!idle.turn(0, false, START + MINUTE).sleep);
        let turn = idle.turn(0, false, START + 3 * MINUTE);
        assert!(turn.sleep && turn.display == Some(false));
        // The adapter's defaults: the display off after ten, never asleep.
        let mut idle = five_minutes();
        idle.set_energy(10, 0, START);
        assert_eq!(
            idle.turn(0, false, START + 10 * MINUTE).display,
            Some(false)
        );
        assert_eq!(idle.turn(0, true, START + 24 * 60 * MINUTE), NOTHING);
        // A one-minute display: dimmed half a minute before... at most ten seconds before.
        let mut idle = Idle::default();
        idle.set_energy(1, 0, START);
        assert_eq!(idle.deadline(), Some(START + 50 * SECOND));
    }

    #[test]
    fn idle_held_back_keeps_the_display_on_and_the_machine_awake() {
        let mut idle = on_battery();
        assert_eq!(idle.turn(0, false, START + 2 * MINUTE).display, Some(false));
        // A video starts: the display comes back, and nothing more happens while it plays.
        idle.set_held(true, START + 3 * MINUTE);
        let turn = idle.turn(0, false, START + 3 * MINUTE);
        assert_eq!((turn.dim, turn.display), (Some(false), Some(true)));
        assert_eq!(idle.deadline(), None);
        assert_eq!(idle.turn(0, false, START + 120 * MINUTE), NOTHING);
        // It ends: two minutes from then.
        let ended = START + 120 * MINUTE;
        idle.set_held(false, ended);
        assert_eq!(idle.turn(0, false, ended + MINUTE), NOTHING);
        assert_eq!(idle.turn(0, false, ended + 2 * MINUTE).display, Some(false));
    }

    #[test]
    fn unplugging_arms_the_timers_again_from_then() {
        let mut idle = five_minutes();
        idle.set_energy(10, 0, START);
        // Eight minutes in, unplugged: the battery's two minutes from then, not at once.
        assert_eq!(idle.turn(0, false, START + 5 * MINUTE), LOCKS);
        let unplugged = START + 8 * MINUTE;
        idle.set_energy(2, 10, unplugged);
        assert_eq!(idle.turn(0, true, unplugged), NOTHING);
        assert_eq!(idle.deadline(), Some(unplugged + 2 * MINUTE - 10 * SECOND));
        // The same timers again change nothing.
        idle.set_energy(2, 10, unplugged + MINUTE);
        assert_eq!(idle.deadline(), Some(unplugged + 2 * MINUTE - 10 * SECOND));
    }
}
