//! When backups run, what they keep, and what they read ([ADR 0059](../../docs/adr/0059-backups.mdx),
//! decisions 2, 4 and 6): Time Machine's schedule and thinning, as restic's own `forget` policy.

use crate::engine::{Keep, Snapshot};
use crate::store::{Config, Record};
use std::path::{Path, PathBuf};

/// A backup an hour.
pub const INTERVAL: i64 = 60 * 60;
/// What a destination's unneeded data is removed after, at most once a day (`restic prune`
/// rewrites packs: it is the costly part).
pub const PRUNE_INTERVAL: i64 = 24 * 60 * 60;
/// How often a destination is checked, reading back a part of its data.
pub const CHECK_INTERVAL: i64 = 7 * 24 * 60 * 60;
/// What a weekly check reads back: every pack in about five months.
pub const CHECK_SUBSET: &str = "5%";

/// Time Machine's thinning: every backup of the last 24 hours, the last of each day of the last
/// month, the last of each week before, for as long as there is room (decision 4). restic's
/// `--keep-within-*` count from the newest backup, so a computer off for a week loses nothing to
/// the time it was off.
pub fn keep() -> Keep {
    Keep {
        arguments: [
            "--keep-within-hourly",
            "24h",
            "--keep-within-daily",
            "1m",
            "--keep-weekly",
            "unlimited",
        ]
        .map(String::from)
        .to_vec(),
    }
}

/// How long after a failure a destination is tried again (sooner when its disk is connected
/// again).
pub const RETRY: i64 = 15 * 60;

/// Whether a destination's backup is due at `now`: never backed up, or its last an hour ago or
/// more (a backup missed while the computer slept or the disk was away is due at once), and not
/// waiting to be tried again after a failure or a skip.
pub fn due(record: Option<&Record>, now: i64) -> bool {
    next(record, now) <= now
}

/// When a destination's next backup is due.
pub fn next(record: Option<&Record>, now: i64) -> i64 {
    let scheduled = match record.and_then(|r| r.last_backup) {
        Some(last) if now >= last && now < last + INTERVAL => last + INTERVAL,
        _ => now,
    };
    match record.and_then(|r| r.retry_at) {
        Some(retry) if retry > now && retry <= now + INTERVAL => scheduled.max(retry),
        _ => scheduled,
    }
}

/// Of the destinations that can be reached now, the one whose backup is the oldest: several
/// destinations take turns, as Time Machine's do.
pub fn choose<'a>(
    available: impl IntoIterator<Item = &'a str>,
    record: impl Fn(&str) -> Option<&'a Record>,
) -> Option<&'a str> {
    available
        .into_iter()
        .min_by_key(|id| record(id).and_then(|r| r.last_backup).unwrap_or(i64::MIN))
}

/// When the destination is full, the backup to remove to make room: the oldest, never the only
/// one (Time Machine deletes the oldest backups, and says so).
pub fn oldest_to_remove(snapshots: &[Snapshot]) -> Option<&Snapshot> {
    if snapshots.len() < 2 {
        return None;
    }
    snapshots
        .iter()
        .min_by_key(|s| s.seconds().unwrap_or(i64::MAX))
}

/// Whether unneeded data is removed now: after backups were thinned, once a day at most.
pub fn prune_due(record: Option<&Record>, removed: usize, now: i64) -> bool {
    removed > 0
        && record
            .and_then(|r| r.last_prune)
            .is_none_or(|last| now >= last + PRUNE_INTERVAL || now < last)
}

/// Whether the destination is checked now.
pub fn check_due(record: Option<&Record>, now: i64) -> bool {
    match record.and_then(|r| r.last_check) {
        None => record.and_then(|r| r.last_backup).is_some(),
        Some(last) => now >= last + CHECK_INTERVAL || now < last,
    }
}

/// What a backup reads and leaves out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sources {
    /// What is read.
    pub paths: Vec<PathBuf>,
    /// restic's exclude patterns, always.
    pub excludes: Vec<String>,
}

/// The homes (every account's).
pub const HOMES: &str = "/home";
/// What the service writes of the machine before each backup, for a restore to read
/// ([`crate::machine`]).
pub const MANIFEST_DIRECTORY: &str = "/var/lib/oxyos/backup/manifest";

impl Sources {
    /// A machine's (decision 6): every home, the accounts' pictures, the computer's Wi-Fi networks
    /// and the machine's manifest. What programs make again (caches, the Trash) is left out.
    pub fn machine() -> Self {
        Self {
            paths: [
                HOMES,
                MANIFEST_DIRECTORY,
                "/var/lib/oxyos/pictures",
                "/etc/NetworkManager/system-connections",
            ]
            .map(PathBuf::from)
            .to_vec(),
            excludes: [
                "/home/*/.cache",
                "/home/*/.local/share/Trash",
                // The development compositor's and builds' outputs that a person's checkout makes
                // again are theirs to mark (CACHEDIR.TAG); nothing else is guessed.
            ]
            .map(String::from)
            .to_vec(),
        }
    }

    /// What is read, the person's excluded items left out too.
    pub fn with(&self, config: &Config) -> (Vec<PathBuf>, Vec<String>) {
        let paths = self
            .paths
            .iter()
            .filter(|p| p.exists())
            .cloned()
            .collect::<Vec<_>>();
        let mut excludes = self.excludes.clone();
        excludes.extend(
            config
                .excluded
                .iter()
                .filter(|p| p.is_absolute())
                .map(|p| escape(p)),
        );
        (paths, excludes)
    }

    /// Whether `path` is one a backup reads (so it may be excluded, or browsed).
    pub fn covers(&self, path: &Path) -> bool {
        self.paths.iter().any(|p| path.starts_with(p))
    }
}

/// An absolute path as a restic exclude pattern that matches it alone: restic's patterns are
/// Go's `filepath.Match`, whose `*`, `?`, `[` and `\` are escaped.
pub fn escape(path: &Path) -> String {
    let mut pattern = String::new();
    for c in path.to_string_lossy().chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '\\') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(last: Option<i64>) -> Record {
        Record {
            last_backup: last,
            ..Record::default()
        }
    }

    fn snapshot(id: &str, time: &str) -> Snapshot {
        Snapshot {
            id: id.into(),
            time: time.into(),
            hostname: "m".into(),
            paths: vec![],
            tags: vec![],
        }
    }

    #[test]
    fn a_backup_is_due_an_hour_after_the_last_and_at_once_when_missed() {
        let now = 1_800_000_000;
        assert!(due(None, now));
        assert!(due(Some(&record(None)), now));
        assert!(!due(Some(&record(Some(now - 10))), now));
        assert_eq!(
            next(Some(&record(Some(now - 10))), now),
            now - 10 + INTERVAL
        );
        assert!(due(Some(&record(Some(now - INTERVAL))), now));
        // Missed (the computer slept through it): now.
        assert!(due(Some(&record(Some(now - 5 * INTERVAL))), now));
        assert_eq!(next(Some(&record(Some(now - 5 * INTERVAL))), now), now);
        // A clock set back before the last backup: due, never a wait of years.
        assert!(due(Some(&record(Some(now + 1000))), now));
        // A failure waits to be tried again.
        let failed = Record {
            last_backup: Some(now - 5 * INTERVAL),
            retry_at: Some(now + RETRY),
            ..Record::default()
        };
        assert!(!due(Some(&failed), now));
        assert_eq!(next(Some(&failed), now), now + RETRY);
        assert!(due(Some(&failed), now + RETRY));
    }

    #[test]
    fn destinations_take_turns_the_oldest_first() {
        let a = record(Some(100));
        let b = record(Some(50));
        let c = record(None);
        let records = [("a", &a), ("b", &b), ("c", &c)];
        let find = |id: &str| records.iter().find(|(i, _)| *i == id).map(|(_, r)| *r);
        assert_eq!(choose(["a", "b"], find), Some("b"));
        assert_eq!(choose(["a", "b", "c"], find), Some("c"));
        assert_eq!(choose([], find), None);
    }

    #[test]
    fn the_thinning_is_time_machines_in_restics_words() {
        assert_eq!(
            keep().arguments.join(" "),
            "--keep-within-hourly 24h --keep-within-daily 1m --keep-weekly unlimited"
        );
    }

    #[test]
    fn a_full_destination_loses_its_oldest_backup_never_its_only_one() {
        let only = [snapshot("a", "2026-11-01T10:00:00Z")];
        assert!(oldest_to_remove(&only).is_none());
        let two = [
            snapshot("new", "2026-11-02T10:00:00Z"),
            snapshot("old", "2026-10-01T10:00:00Z"),
        ];
        assert_eq!(oldest_to_remove(&two).unwrap().id, "old");
    }

    #[test]
    fn pruning_and_checking_are_rare() {
        let now = 1_800_000_000;
        assert!(!prune_due(None, 0, now));
        assert!(prune_due(None, 1, now));
        let pruned = Record {
            last_prune: Some(now - 60),
            ..Record::default()
        };
        assert!(!prune_due(Some(&pruned), 3, now));
        assert!(!check_due(None, now));
        assert!(check_due(Some(&record(Some(now))), now));
        let checked = Record {
            last_backup: Some(now),
            last_check: Some(now - 60),
            ..Record::default()
        };
        assert!(!check_due(Some(&checked), now));
    }

    #[test]
    fn excluded_items_are_matched_exactly() {
        assert_eq!(escape(Path::new("/home/a/[x]*?")), "/home/a/\\[x\\]\\*\\?");
        let sources = Sources::machine();
        assert!(sources.covers(Path::new("/home/ada/Documents")));
        assert!(!sources.covers(Path::new("/etc/passwd")));
        let config = Config {
            excluded: vec!["/home/ada/Movies".into(), "relative".into()],
            ..Config::default()
        };
        let (_, excludes) = sources.with(&config);
        assert!(excludes.contains(&"/home/ada/Movies".to_owned()));
        assert!(!excludes.contains(&"relative".to_owned()));
    }
}
