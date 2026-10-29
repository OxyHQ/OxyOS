//! A pointer's velocity, as Android's default `VelocityTracker` strategy finds it: a second-degree
//! least-squares fit of the last 100 ms of movement (at most 20 samples), its slope at the latest
//! sample. Movement stops counting after 40 ms without any, and a pointer going up adds none, as
//! Android's tracker does. It follows one pointer, the first down, then the first left.

use crate::handler::Event;
use crate::Action;
use std::collections::{BTreeMap, VecDeque};

const HORIZON_MS: u64 = 100;
const HISTORY: usize = 20;
const STOPPED_MS: u64 = 40;

#[derive(Debug, Default)]
pub(crate) struct Tracker {
    samples: BTreeMap<u32, VecDeque<(u64, f32, f32)>>,
    active: Option<u32>,
    last_time: Option<u64>,
}

impl Tracker {
    /// `addMovement`, an event in root coordinates.
    pub(crate) fn add(&mut self, event: &Event) {
        if self
            .last_time
            .is_some_and(|last| event.time >= last + STOPPED_MS)
        {
            // Nothing moved for too long: every pointer has stopped.
            self.samples.clear();
        }
        self.last_time = Some(event.time);
        let actor = event.pointers.get(event.index).map(|p| p.id);
        match event.action {
            Action::Down => {
                self.samples.clear();
                self.active = actor;
            }
            Action::PointerDown => {
                if let Some(id) = actor {
                    self.samples.remove(&id);
                }
            }
            Action::Up | Action::PointerUp | Action::Cancel => {
                if event.action == Action::PointerUp {
                    if let Some(id) = actor {
                        self.samples.remove(&id);
                        if self.active == Some(id) {
                            self.active = event.pointers.iter().map(|p| p.id).find(|p| *p != id);
                        }
                    }
                }
                return;
            }
            Action::Move => {}
        }
        if self
            .active
            .is_none_or(|id| !event.pointers.iter().any(|p| p.id == id))
        {
            self.active = event.pointers.first().map(|p| p.id);
        }
        for pointer in &event.pointers {
            let history = self.samples.entry(pointer.id).or_default();
            history.push_back((event.time, pointer.x, pointer.y));
            while history.len() > HISTORY {
                history.pop_front();
            }
        }
    }

    /// The active pointer's velocity in pixels per second, at `now`.
    pub(crate) fn velocity(&self, now: u64) -> (f32, f32) {
        let Some(history) = self.active.and_then(|id| self.samples.get(&id)) else {
            return (0.0, 0.0);
        };
        let Some(&(newest, _, _)) = history.back() else {
            return (0.0, 0.0);
        };
        if now >= newest + STOPPED_MS {
            return (0.0, 0.0);
        }
        let recent: Vec<(f64, f64, f64)> = history
            .iter()
            .filter(|(t, _, _)| newest - t <= HORIZON_MS)
            .map(|&(t, x, y)| {
                let seconds = -((newest - t) as f64) / 1000.0;
                (seconds, f64::from(x), f64::from(y))
            })
            .collect();
        let vx = slope(recent.iter().map(|&(t, x, _)| (t, x)));
        let vy = slope(recent.iter().map(|&(t, _, y)| (t, y)));
        (vx as f32, vy as f32)
    }
}

/// The slope at t = 0 of the least-squares polynomial through the samples: second degree with
/// three or more, first with two, none with fewer.
fn slope(samples: impl Iterator<Item = (f64, f64)> + Clone) -> f64 {
    let n = samples.clone().count();
    if n < 2 {
        return 0.0;
    }
    // Sums of t^k (k = 0..4) and of v t^k (k = 0..2).
    let mut s = [0.0f64; 5];
    let mut r = [0.0f64; 3];
    for (t, v) in samples {
        let mut p = 1.0;
        for (k, sum) in s.iter_mut().enumerate() {
            *sum += p;
            if k < 3 {
                r[k] += v * p;
            }
            p *= t;
        }
    }
    if n >= 3 {
        // Normal equations for v = a + b t + c t^2, solved for b by Cramer's rule.
        let m = [[s[0], s[1], s[2]], [s[1], s[2], s[3]], [s[2], s[3], s[4]]];
        let det = det3(m);
        if det.abs() > 1e-18 {
            let mut mb = m;
            for (row, value) in mb.iter_mut().zip(r) {
                row[1] = value;
            }
            return det3(mb) / det;
        }
    }
    let det = s[0] * s[2] - s[1] * s[1];
    if det.abs() < 1e-18 {
        return 0.0;
    }
    (s[0] * r[1] - s[1] * r[0]) / det
}

fn det3(m: [[f64; 3]; 3]) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}
