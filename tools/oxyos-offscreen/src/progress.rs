//! Frames that only turn a spinner, left aside by a step's settling (issue #559,
//! tools/rn-host-spike/progress-bar.h).
//!
//! A step settles once what the application shows stays unchanged for SETTLE. A spinner (React
//! Native's ActivityIndicator, Android's ProgressBar) turns as long as its work goes on, so a screen
//! whose only motion is a spinner never stays unchanged, though a person reads it as settled: the
//! button pressed shows its work under way, and nothing else will move until it ends. The host says
//! which of its frames change nothing but its spinning rings, each frame's damage held by theirs
//! (its report's `present` lines with `progress_only`); a frame of those reads here as the frame
//! counted before it, so the spinner neither answers a step nor keeps it from settling. Nothing in
//! the host is held or turned off for it: the ring is drawn as a person sees it.

use oxyos_compositor_core::{ClientId, SurfaceKey};
use oxyos_protocol::Rect;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How long a frame its host has not said anything of yet (its report written just after the
/// frame is sent) is left aside before it counts as a change, while its surface's last frame said
/// only turned a spinner; any other surface's counts at once, as without spinners.
const UNSAID: Duration = Duration::from_millis(250);

/// A host's report, read as it is written.
#[derive(Debug, Default)]
struct Report {
    path: PathBuf,
    read: u64,
    partial: Vec<u8>,
    /// Every frame presented, by its sequence (the client's, across its surfaces): its surface and
    /// whether it changed only spinners.
    presented: HashMap<u64, (u32, bool)>,
    /// Each surface's frame said last (its sequence), and whether it only turned spinners.
    latest: HashMap<u32, (u64, bool)>,
}

impl Report {
    fn refresh(&mut self) {
        let Ok(mut file) = std::fs::File::open(&self.path) else {
            return;
        };
        if file.seek(SeekFrom::Start(self.read)).is_err() {
            return;
        }
        let mut bytes = Vec::new();
        let Ok(read) = file.read_to_end(&mut bytes) else {
            return;
        };
        self.read += read as u64;
        self.partial.extend_from_slice(&bytes);
        while let Some(end) = self.partial.iter().position(|&byte| byte == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=end).collect();
            // Most lines are other kinds: only a present line is parsed.
            if !line.windows(9).any(|window| window == b"\"present\"") {
                continue;
            }
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&line) else {
                continue;
            };
            if value["kind"] != "present" {
                continue;
            }
            let (Some(surface), Some(sequence)) =
                (value["surface"].as_u64(), value["sequence"].as_u64())
            else {
                continue;
            };
            let Ok(surface) = u32::try_from(surface) else {
                continue;
            };
            let only = value["progress_only"].as_bool() == Some(true);
            self.presented.insert(sequence, (surface, only));
            let latest = self.latest.entry(surface).or_insert((sequence, only));
            if sequence >= latest.0 {
                *latest = (sequence, only);
            }
        }
    }

    /// Whether `surface`'s frame said last only turned spinners: one turns on it.
    fn spinning(&self, surface: u32) -> bool {
        self.latest.get(&surface).is_some_and(|&(_, only)| only)
    }

    /// What the frames of `surface` after `from` up to `to` changed, as far as the report says.
    fn said(&self, surface: u32, from: u64, to: u64) -> Said {
        for sequence in from.saturating_add(1)..=to {
            match self.presented.get(&sequence) {
                Some(&(of, only)) if of != surface || only => {}
                Some(_) => return Said::Changed,
                None => return Said::Unsaid(sequence),
            }
        }
        Said::OnlySpinners
    }
}

/// What a surface's frames since the one counted changed.
#[derive(Debug, PartialEq)]
enum Said {
    /// Something but spinners, in one of them.
    Changed,
    /// Only spinners, in every one of them.
    OnlySpinners,
    /// Only spinners up to this frame (whichever surface's it is), which its host has not said
    /// anything of yet.
    Unsaid(u64),
}

#[derive(Clone, Copy, Debug)]
struct Kept {
    sequence: u64,
    rect: Rect,
    /// The first frame not said yet, and since when it has been: a frame left unsaid for UNSAID
    /// counts, but one said in time and the next not yet is no reason to.
    unsaid: Option<(u64, Instant)>,
}

/// The surfaces' frames as a step's settling counts them.
#[derive(Debug, Default)]
pub struct ProgressFrames {
    /// The application's report, then the shell's (`.shell`), as the director names them.
    reports: Vec<Report>,
    /// Which report a client's surfaces are in: the application's client, and the shell's clients
    /// (between the two, exclusive) once the shell is followed.
    application: ClientId,
    shell: Option<(ClientId, ClientId)>,
    /// Each surface's frame counted last, and where it was.
    kept: HashMap<SurfaceKey, Kept>,
}

impl ProgressFrames {
    /// Reading the application's report (OXYOS_HOST_REPORT) for its client's surfaces.
    pub fn new(application: ClientId) -> Self {
        let reports = std::env::var_os("OXYOS_HOST_REPORT")
            .map(|report| Report {
                path: PathBuf::from(report),
                ..Report::default()
            })
            .into_iter()
            .collect();
        Self {
            reports,
            application,
            shell: None,
            kept: HashMap::new(),
        }
    }

    /// Reading the shell's report too (the application's with `.shell`) for the shell's clients,
    /// those between `clients.0` and `clients.1` (exclusive). Only a shell alone on the session's
    /// one output: another output's shell has a report of its own, and the director does not know
    /// which of the shell's clients is whose.
    pub fn follow_shell(&mut self, clients: (ClientId, ClientId)) {
        let Some(application) = self.reports.first() else {
            return;
        };
        let mut path = application.path.clone().into_os_string();
        path.push(".shell");
        self.reports.truncate(1);
        self.reports.push(Report {
            path: PathBuf::from(path),
            ..Report::default()
        });
        self.shell = Some(clients);
    }

    fn report_of(&self, key: SurfaceKey) -> Option<usize> {
        let index = if key.client == self.application {
            0
        } else if self
            .shell
            .is_some_and(|(low, high)| key.client > low && key.client < high)
        {
            1
        } else {
            return None;
        };
        (index < self.reports.len()).then_some(index)
    }

    /// Counts `view` (surfaces, their frames' sequences and rectangles) as it was: what a step
    /// starts from.
    pub fn start<K: Copy>(&mut self, view: &[(K, u64, Rect)], key: impl Fn(K) -> SurfaceKey) {
        for &(surface, sequence, rect) in view {
            self.kept.insert(
                key(surface),
                Kept {
                    sequence,
                    rect,
                    unsaid: None,
                },
            );
        }
    }

    /// `view` with each surface whose frames since the one counted last changed only its spinners
    /// given that one's sequence, so that it reads unchanged; any other change is counted.
    pub fn steady<K: Copy>(
        &mut self,
        view: Vec<(K, u64, Rect)>,
        key: impl Fn(K) -> SurfaceKey,
    ) -> Vec<(K, u64, Rect)> {
        let now = Instant::now();
        let mut refreshed = false;
        let mut out = Vec::with_capacity(view.len());
        for (surface, sequence, rect) in view {
            let keyed = key(surface);
            let report = self.report_of(keyed);
            if let (Some(report), Some(kept)) = (report, self.kept.get(&keyed).copied()) {
                if kept.rect == rect && sequence == kept.sequence {
                    out.push((surface, sequence, rect));
                    continue;
                }
                if kept.rect == rect && sequence > kept.sequence {
                    if !refreshed {
                        for report in &mut self.reports {
                            report.refresh();
                        }
                        refreshed = true;
                    }
                    match self.reports[report].said(keyed.id, kept.sequence, sequence) {
                        Said::OnlySpinners => {
                            self.kept.insert(
                                keyed,
                                Kept {
                                    unsaid: None,
                                    ..kept
                                },
                            );
                            out.push((surface, kept.sequence, rect));
                            continue;
                        }
                        Said::Unsaid(first) if self.reports[report].spinning(keyed.id) => {
                            let since = match kept.unsaid {
                                Some((waiting, since)) if waiting == first => since,
                                _ => now,
                            };
                            if now.duration_since(since) < UNSAID {
                                self.kept.insert(
                                    keyed,
                                    Kept {
                                        unsaid: Some((first, since)),
                                        ..kept
                                    },
                                );
                                out.push((surface, kept.sequence, rect));
                                continue;
                            }
                        }
                        Said::Unsaid(_) | Said::Changed => {}
                    }
                }
            }
            self.kept.insert(
                keyed,
                Kept {
                    sequence,
                    rect,
                    unsaid: None,
                },
            );
            out.push((surface, sequence, rect));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(lines: &[&str]) -> (tempfile_path::Temp, ProgressFrames) {
        let temp = tempfile_path::Temp::new();
        std::fs::write(&temp.0, lines.join("\n") + "\n").unwrap();
        let frames = ProgressFrames {
            reports: vec![Report {
                path: temp.0.clone(),
                ..Report::default()
            }],
            application: 1,
            shell: None,
            kept: HashMap::new(),
        };
        (temp, frames)
    }

    mod tempfile_path {
        pub struct Temp(pub std::path::PathBuf);
        impl Temp {
            pub fn new() -> Self {
                static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Self(
                    std::env::temp_dir()
                        .join(format!("oxyos-progress-{}-{n}.jsonl", std::process::id())),
                )
            }
        }
        impl Drop for Temp {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
    }

    const RECT: Rect = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 100,
    };

    fn key(id: u32) -> SurfaceKey {
        SurfaceKey { client: 1, id }
    }

    #[test]
    fn frames_that_only_turn_a_spinner_read_as_the_one_before() {
        let (_temp, mut frames) = report(&[
            r#"{"t":1,"kind":"present","surface":1,"sequence":5,"progress_only":true}"#,
            r#"{"t":2,"kind":"present","surface":1,"sequence":6,"progress_only":true}"#,
        ]);
        frames.start(&[(1, 4, RECT)], key);
        assert_eq!(frames.steady(vec![(1, 6, RECT)], key), vec![(1, 4, RECT)]);
    }

    #[test]
    fn any_other_frame_counts() {
        let (_temp, mut frames) = report(&[
            r#"{"t":1,"kind":"present","surface":1,"sequence":5,"progress_only":true}"#,
            r#"{"t":2,"kind":"present","surface":1,"sequence":6}"#,
        ]);
        frames.start(&[(1, 4, RECT)], key);
        assert_eq!(frames.steady(vec![(1, 6, RECT)], key), vec![(1, 6, RECT)]);
        // Counted: the next spinner-only frame reads as that one.
        std::fs::write(
            &frames.reports[0].path,
            [
                r#"{"t":1,"kind":"present","surface":1,"sequence":5,"progress_only":true}"#,
                r#"{"t":2,"kind":"present","surface":1,"sequence":6}"#,
                r#"{"t":3,"kind":"present","surface":1,"sequence":7,"progress_only":true}"#,
                "",
            ]
            .join("\n"),
        )
        .unwrap();
        assert_eq!(frames.steady(vec![(1, 7, RECT)], key), vec![(1, 6, RECT)]);
    }

    #[test]
    fn another_surfaces_frames_between_are_its_own() {
        // The client's sequence runs across its surfaces: a popup's frames come between.
        let (_temp, mut frames) = report(&[
            r#"{"t":1,"kind":"present","surface":1,"sequence":5,"progress_only":true}"#,
            r#"{"t":2,"kind":"present","surface":2,"sequence":6}"#,
            r#"{"t":3,"kind":"present","surface":1,"sequence":7,"progress_only":true}"#,
        ]);
        frames.start(&[(1, 4, RECT)], key);
        assert_eq!(frames.steady(vec![(1, 7, RECT)], key), vec![(1, 4, RECT)]);
    }

    #[test]
    fn a_frame_moved_or_resized_counts() {
        let (_temp, mut frames) =
            report(&[r#"{"t":1,"kind":"present","surface":1,"sequence":5,"progress_only":true}"#]);
        frames.start(&[(1, 4, RECT)], key);
        let moved = Rect { x: 10, ..RECT };
        assert_eq!(frames.steady(vec![(1, 5, moved)], key), vec![(1, 5, moved)]);
    }

    #[test]
    fn a_frame_not_said_yet_waits_a_while() {
        // Its surface turning a spinner (its frame 4, the one counted, only did).
        let (_temp, mut frames) =
            report(&[r#"{"t":1,"kind":"present","surface":1,"sequence":4,"progress_only":true}"#]);
        frames.start(&[(1, 4, RECT)], key);
        assert_eq!(frames.steady(vec![(1, 5, RECT)], key), vec![(1, 4, RECT)]);
        frames.kept.get_mut(&key(1)).unwrap().unsaid = Some((5, Instant::now() - UNSAID));
        assert_eq!(frames.steady(vec![(1, 5, RECT)], key), vec![(1, 5, RECT)]);
    }

    #[test]
    fn without_a_spinner_a_frame_not_said_yet_counts_at_once() {
        let (_temp, mut frames) = report(&[r#"{"t":1,"kind":"present","surface":1,"sequence":4}"#]);
        frames.start(&[(1, 4, RECT)], key);
        assert_eq!(frames.steady(vec![(1, 5, RECT)], key), vec![(1, 5, RECT)]);
    }

    #[test]
    fn frames_said_one_after_another_never_wait_too_long() {
        // A spinner's frames come faster than its host says them: each time the first unsaid is a
        // later one, its wait starts again.
        let (_temp, mut frames) =
            report(&[r#"{"t":1,"kind":"present","surface":1,"sequence":5,"progress_only":true}"#]);
        frames.start(&[(1, 4, RECT)], key);
        frames.kept.get_mut(&key(1)).unwrap().unsaid = Some((5, Instant::now() - UNSAID));
        assert_eq!(frames.steady(vec![(1, 6, RECT)], key), vec![(1, 4, RECT)]);
        assert_eq!(frames.kept[&key(1)].unsaid.map(|(first, _)| first), Some(6));
    }

    #[test]
    fn another_clients_surfaces_are_as_they_are() {
        let (_temp, mut frames) = report(&[]);
        let other = |id| SurfaceKey { client: 7, id };
        frames.start(&[(1, 4, RECT)], other);
        assert_eq!(frames.steady(vec![(1, 5, RECT)], other), vec![(1, 5, RECT)]);
    }
}
