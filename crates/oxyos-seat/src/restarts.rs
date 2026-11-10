//! When a part of OxyOS that must always be there is started again after it ended on its own
//! (docs/adr/0061-crashes-and-resetting-an-application.mdx, decision 6): the session's shell, the
//! login program's login screen and the session's screen reader. At once the first time, then
//! after a wait that doubles each time it ends again without having run [`STEADY`], from
//! [`RESTART`] up to [`RESTART_MAX`]: never a loop, and never given up on, as launchd throttles a
//! job that keeps crashing and never stops starting it.
//!
//! One policy for every such part, so the login screen, the shell and the reader can never drift
//! apart (the login screen once restarted after a fixed second, 174 times in three minutes).

use std::time::{Duration, Instant};

/// The first wait after a second end in a row.
pub const RESTART: Duration = Duration::from_secs(1);
/// The longest wait.
pub const RESTART_MAX: Duration = Duration::from_secs(60);
/// A part that ran this long before it ended starts over: its next end is a first one again.
pub const STEADY: Duration = Duration::from_secs(60);

/// A part's ends in a row, each before it ran [`STEADY`].
#[derive(Debug, Default)]
pub struct Restarts {
    ends: u32,
}

impl Restarts {
    /// The part ended after running for `ran`: how long to wait before starting it again.
    pub fn ended(&mut self, ran: Duration) -> Duration {
        if ran >= STEADY {
            self.ends = 0;
        }
        self.ends = self.ends.saturating_add(1);
        match self.ends {
            1 => Duration::ZERO,
            n => RESTART
                .saturating_mul(1u32 << (n - 2).min(16))
                .min(RESTART_MAX),
        }
    }

    /// How many times in a row it ended before running [`STEADY`], the last one included.
    pub fn in_a_row(&self) -> u32 {
        self.ends
    }
}

/// A part kept running while it is wanted (the screen reader while the person has it on): it
/// is started again when it ends on its own, with [`Restarts`]' wait, and never when the person
/// turned it off. The program that runs it keeps the process; this keeps the decisions, so the
/// session and the development compositor make the same ones.
#[derive(Debug, Default)]
pub struct Kept {
    restarts: Restarts,
    /// When it was last started, while it runs.
    started: Option<Instant>,
    /// When it is to be started again, after it ended on its own while wanted.
    due: Option<Instant>,
}

impl Kept {
    /// It was started (by the person's switch, the preference, or [`Kept::take_due`]).
    pub fn started(&mut self, now: Instant) {
        self.started = Some(now);
        self.due = None;
    }

    /// It was stopped as asked (the person turned it off): nothing starts it again.
    pub fn stopped(&mut self) {
        self.started = None;
        self.due = None;
    }

    /// It ended on its own: it is to be started again after the wait, which is returned.
    pub fn ended(&mut self, now: Instant) -> Duration {
        let ran = self.started.take().map_or(Duration::ZERO, |started| {
            now.saturating_duration_since(started)
        });
        let wait = self.restarts.ended(ran);
        self.due = Some(now + wait);
        wait
    }

    /// When it is to be started again, for the program's turn to wake then.
    pub fn deadline(&self) -> Option<Instant> {
        self.due
    }

    /// Whether it is time to start it again; once true, it is no longer due (the caller starts
    /// it and says so with [`Kept::started`], or, failing, [`Kept::ended`] again).
    pub fn take_due(&mut self, now: Instant) -> bool {
        match self.due {
            Some(due) if now >= due => {
                self.due = None;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_is_started_again_ever_later_and_never_given_up_on() {
        let mut restarts = Restarts::default();
        let quick = Duration::from_secs(2);
        let waits: Vec<u64> = (0..10).map(|_| restarts.ended(quick).as_secs()).collect();
        assert_eq!(waits, [0, 1, 2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(restarts.in_a_row(), 10);
        // One that ran a minute starts over.
        assert_eq!(restarts.ended(STEADY), Duration::ZERO);
        assert_eq!(restarts.ended(quick), RESTART);
        // Thousands of ends never overflow, never wait past the longest and never stop the restarts.
        let waits: Vec<Duration> = (0..100_000).map(|_| restarts.ended(quick)).collect();
        assert!(waits
            .iter()
            .all(|wait| *wait <= RESTART_MAX && *wait >= RESTART));
        assert_eq!(waits.last(), Some(&RESTART_MAX));
    }

    #[test]
    fn a_kept_part_comes_back_after_its_wait_and_never_once_turned_off() {
        let start = Instant::now();
        let mut kept = Kept::default();
        kept.started(start);
        // It ends after a second: started again at once.
        assert_eq!(kept.ended(start + Duration::from_secs(1)), Duration::ZERO);
        assert!(kept.take_due(start + Duration::from_secs(1)));
        assert!(!kept.take_due(start + Duration::from_secs(1)), "once");
        kept.started(start + Duration::from_secs(1));
        // Again at once: a second's wait, not before.
        let again = start + Duration::from_secs(2);
        assert_eq!(kept.ended(again), RESTART);
        assert_eq!(kept.deadline(), Some(again + RESTART));
        assert!(!kept.take_due(again + Duration::from_millis(999)));
        assert!(kept.take_due(again + RESTART));
        // Failing to start counts as ending at once.
        assert_eq!(kept.ended(again + RESTART), RESTART * 2);
        // The person turning it off is never undone.
        kept.stopped();
        assert_eq!(kept.deadline(), None);
        assert!(!kept.take_due(again + Duration::from_secs(3600)));
    }
}
