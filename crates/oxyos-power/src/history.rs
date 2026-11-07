//! The battery's charge and the display's use over the last ten days (docs/adr/0057-battery-and-
//! energy.mdx, decision 5), as Settings › Battery charts them: recorded by the session as they
//! change (UPower's signals of the charge and its state, the display turned on and off, the
//! machine going to sleep and waking, the session starting and ending), never on a clock. A
//! machine on its adapter at 100 % records nothing for hours, and nothing wakes to record it.
//!
//! The file is `$XDG_STATE_HOME/oxyos/battery-history` (`~/.local/state`), the session its one
//! writer: a line per change, `SECONDS KIND VALUE…`, appended. Ten days are about three thousand
//! lines. When the session starts, what is older than ten days is left out: the file is written
//! anew beside it and renamed over it. A line that cannot be read is skipped.
//!
//! Where no session ran (the machine off, another user's session), the charts show nothing, as a
//! Mac's show nothing where it was off.

use serde::Serialize;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// How long is kept: ten days, and the hour the last bin reaches into.
pub const KEEP: u64 = 10 * 24 * 3600 + 3600;
/// The last 24 hours, in quarters of an hour.
pub const DAY_BIN: u64 = 15 * 60;
pub const DAY_BINS: usize = 96;
/// The last ten days, in hours.
pub const DAYS_BIN: u64 = 3600;
pub const DAYS_BINS: usize = 240;

/// The battery's state as recorded: what the charts color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Charge {
    Charging,
    Discharging,
    /// On the adapter, not charging: full, or held at its charge limit.
    Plugged,
}

impl Charge {
    fn letter(self) -> char {
        match self {
            Charge::Charging => 'c',
            Charge::Discharging => 'd',
            Charge::Plugged => 'p',
        }
    }

    fn of(letter: &str) -> Option<Self> {
        Some(match letter {
            "c" => Charge::Charging,
            "d" => Charge::Discharging,
            "p" => Charge::Plugged,
            _ => return None,
        })
    }
}

/// What changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The battery's charge, in percent, and its state.
    Battery(u8, Charge),
    /// The display turned on (`true`) or off.
    Display(bool),
    /// The machine went to sleep (`true`) or woke.
    Asleep(bool),
    /// The session started (`true`) or ended.
    Session(bool),
}

impl Event {
    fn text(self) -> String {
        match self {
            Event::Battery(percent, charge) => format!("b {percent} {}", charge.letter()),
            Event::Display(on) => format!("s {}", u8::from(on)),
            Event::Asleep(asleep) => format!("z {}", u8::from(asleep)),
            Event::Session(started) => format!("o {}", u8::from(started)),
        }
    }

    fn parse(kind: &str, values: &[&str]) -> Option<Self> {
        let flag = || match values {
            ["1"] => Some(true),
            ["0"] => Some(false),
            _ => None,
        };
        Some(match kind {
            "b" => match values {
                [percent, charge] => {
                    let percent: u8 = percent.parse().ok().filter(|p| *p <= 100)?;
                    Event::Battery(percent, Charge::of(charge)?)
                }
                _ => return None,
            },
            "s" => Event::Display(flag()?),
            "z" => Event::Asleep(flag()?),
            "o" => Event::Session(flag()?),
            _ => return None,
        })
    }
}

/// One line: when (seconds since the epoch) and what.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    pub at: u64,
    pub event: Event,
}

/// Where the history is: `$XDG_STATE_HOME/oxyos/battery-history`, or `~/.local/state/…`.
pub fn path(state_home: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    let state = match state_home.filter(|p| p.is_absolute()) {
        Some(state) => state.to_owned(),
        None => home?.join(".local").join("state"),
    };
    Some(state.join("oxyos").join("battery-history"))
}

/// The session's own, from its environment.
pub fn session_path() -> Option<PathBuf> {
    let state_home = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    path(state_home.as_deref(), home.as_deref())
}

/// The lines of `text` that can be read, in order.
pub fn parse(text: &str) -> Vec<Record> {
    text.lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let at = words.next()?.parse().ok()?;
            let kind = words.next()?;
            let values: Vec<&str> = words.collect();
            Some(Record {
                at,
                event: Event::parse(kind, &values)?,
            })
        })
        .collect()
}

fn line(record: &Record) -> String {
    format!("{} {}\n", record.at, record.event.text())
}

/// The history, as the session keeps it: what the file has of the last ten days, and the file it
/// appends to (none: kept only for this session, a machine with no home).
pub struct History {
    records: Vec<Record>,
    file: Option<(PathBuf, std::fs::File)>,
    /// The battery as last recorded, so a signal that changed nothing shown records nothing.
    battery: Option<(u8, Charge)>,
}

impl History {
    /// What `path` keeps of the ten days before `now`; the file written anew without what is
    /// older. A file that cannot be written is kept in memory alone, said once.
    pub fn open(path: Option<PathBuf>, now: u64) -> Self {
        let mut records: Vec<Record> = path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| parse(&text))
            .unwrap_or_default();
        let read = records.len();
        records.retain(|record| record.at + KEEP > now && record.at <= now);
        records.sort_by_key(|record| record.at);
        let file = path.and_then(|path| match Self::file(&path, &records, read) {
            Ok(file) => Some((path, file)),
            Err(error) => {
                eprintln!(
                    "[power] the battery's history ({}): {error}; kept for this session only",
                    path.display()
                );
                None
            }
        });
        Self {
            records,
            file,
            battery: None,
        }
    }

    /// Opens the file to append to, written anew first when something was left out.
    fn file(path: &Path, records: &[Record], read: usize) -> io::Result<std::fs::File> {
        if let Some(directory) = path.parent() {
            std::fs::create_dir_all(directory)?;
        }
        if records.len() != read {
            let mut temporary = path.as_os_str().to_owned();
            temporary.push(".new");
            let temporary = PathBuf::from(temporary);
            let text: String = records.iter().map(line).collect();
            std::fs::write(&temporary, text)?;
            std::fs::rename(&temporary, path)?;
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    }

    /// Where it is written, if anywhere.
    pub fn path(&self) -> Option<&Path> {
        self.file.as_ref().map(|(path, _)| path.as_path())
    }

    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// Records `event` at `now`: a line appended. The battery as last recorded is not recorded
    /// again.
    pub fn record(&mut self, now: u64, event: Event) {
        if let Event::Battery(percent, charge) = event {
            if self.battery == Some((percent, charge)) {
                return;
            }
            self.battery = Some((percent, charge));
        }
        // A session that starts records its battery again once it reads it.
        if let Event::Session(true) = event {
            self.battery = None;
        }
        let record = Record { at: now, event };
        if let Some((_, file)) = self.file.as_mut() {
            if let Err(error) = file.write_all(line(&record).as_bytes()) {
                eprintln!("[power] the battery's history: {error}; kept for this session only");
                self.file = None;
            }
        }
        self.records.push(record);
        // What falls out of the ten days goes from memory; the file is trimmed at the next start.
        let first_kept = self.records.partition_point(|r| r.at + KEEP <= now);
        if first_kept > 0 {
            self.records.drain(..first_kept);
        }
    }

    /// The charts at `now`.
    pub fn charts(&self, now: u64) -> Charts {
        charts(&self.records, now)
    }
}

/// One chart: `levels.len()` bins of `bin` seconds, the last ending at `end`; each bin's charge at
/// its end (none where no session ran), whether it charged in it, and the seconds the display was
/// on in it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Chart {
    pub end: u64,
    pub bin: u64,
    pub levels: Vec<Option<u8>>,
    pub charging: Vec<bool>,
    pub screen: Vec<u32>,
}

/// The last 24 hours by the quarter of an hour and the last ten days by the hour.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Charts {
    pub day: Chart,
    pub days: Chart,
}

/// A stretch the session ran, and the battery's changes in it.
struct Span {
    from: u64,
    to: u64,
    /// Each change of the charge: from when, the percent, its state.
    battery: Vec<(u64, u8, Charge)>,
}

impl Span {
    /// The charge at `at`, if the session ran then and had read it.
    fn level_at(&self, at: u64) -> Option<(u8, Charge)> {
        if at < self.from || at > self.to {
            return None;
        }
        self.battery
            .iter()
            .rev()
            .find(|(from, _, _)| *from <= at)
            .map(|(_, percent, charge)| (*percent, *charge))
    }

    /// Whether it charged at some time in `[from, to)`.
    fn charged_in(&self, from: u64, to: u64) -> bool {
        let from = from.max(self.from);
        let to = to.min(self.to);
        if from >= to {
            return false;
        }
        // The state in effect at the start, and every change within.
        self.level_at(from)
            .is_some_and(|(_, charge)| charge == Charge::Charging)
            || self
                .battery
                .iter()
                .any(|(at, _, charge)| *at >= from && *at < to && *charge == Charge::Charging)
    }
}

/// The stretches the sessions ran and the display's on-intervals, from the records.
fn spans(records: &[Record], now: u64) -> (Vec<Span>, Vec<(u64, u64)>) {
    let mut spans: Vec<Span> = Vec::new();
    let mut screen: Vec<(u64, u64)> = Vec::new();
    let mut on: Option<u64> = None;
    let mut last: Option<u64> = None;
    let mut open = false;
    for record in records {
        let at = record.at;
        match record.event {
            Event::Session(true) => {
                // The one before ended unseen (the machine turned off, a failure): at its last
                // record.
                if open {
                    if let (Some(span), Some(last)) = (spans.last_mut(), last) {
                        span.to = last;
                    }
                }
                if let (Some(from), Some(last)) = (on.take(), last) {
                    screen.push((from, last));
                }
                spans.push(Span {
                    from: at,
                    to: at,
                    battery: Vec::new(),
                });
                open = true;
            }
            Event::Session(false) => {
                if let Some(span) = spans.last_mut().filter(|_| open) {
                    span.to = at;
                }
                open = false;
                if let Some(from) = on.take() {
                    screen.push((from, at));
                }
            }
            Event::Battery(percent, charge) => {
                // A record outside a session (an older file's) begins one.
                if !open {
                    spans.push(Span {
                        from: at,
                        to: at,
                        battery: Vec::new(),
                    });
                    open = true;
                }
                if let Some(span) = spans.last_mut() {
                    span.battery.push((at, percent, charge));
                    span.to = at;
                }
            }
            Event::Display(true) => {
                if on.is_none() {
                    on = Some(at);
                }
            }
            Event::Display(false) | Event::Asleep(true) => {
                if let Some(from) = on.take() {
                    screen.push((from, at));
                }
            }
            Event::Asleep(false) => {}
        }
        if open {
            if let Some(span) = spans.last_mut() {
                span.to = span.to.max(at);
            }
        }
        last = Some(at);
    }
    // The session running now runs until now.
    if open {
        if let Some(span) = spans.last_mut() {
            span.to = now;
        }
    }
    if let Some(from) = on {
        screen.push((from, now));
    }
    (spans, screen)
}

fn chart(spans: &[Span], screen: &[(u64, u64)], now: u64, bin: u64, bins: usize) -> Chart {
    let end = now.div_ceil(bin) * bin;
    let start = end.saturating_sub(bin * bins as u64);
    let mut levels = Vec::with_capacity(bins);
    let mut charging = Vec::with_capacity(bins);
    let mut on = Vec::with_capacity(bins);
    for index in 0..bins as u64 {
        let from = start + index * bin;
        let to = from + bin;
        // The charge at the bin's end, or, where the session stopped within it, at its stop.
        let level = spans
            .iter()
            .rev()
            .filter(|span| span.from < to && span.to >= from)
            .find_map(|span| span.level_at(span.to.min(to).min(now)))
            .map(|(percent, _)| percent);
        levels.push(level);
        charging.push(spans.iter().any(|span| span.charged_in(from, to)));
        let seconds: u64 = screen
            .iter()
            .map(|(a, b)| (*b).min(to).saturating_sub((*a).max(from)))
            .sum();
        on.push(seconds.min(bin) as u32);
    }
    Chart {
        end,
        bin,
        levels,
        charging,
        screen: on,
    }
}

/// The charts of `records` at `now`.
pub fn charts(records: &[Record], now: u64) -> Charts {
    let (spans, screen) = spans(records, now);
    Charts {
        day: chart(&spans, &screen, now, DAY_BIN, DAY_BINS),
        days: chart(&spans, &screen, now, DAYS_BIN, DAYS_BINS),
    }
}
