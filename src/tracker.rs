//! Keeps track of what Dapan is doing right now and how long recent tasks took.
//! Every task is also logged, so slow steps show up in logs\dapan-*.log.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

const KEEP_RECENT: usize = 30;

struct Running {
    name: String,
    who: String,
    started: Instant,
    stage: String,
    /// Finished steps with their durations, e.g. "giọng 1.20s".
    steps: Vec<String>,
    step_started: Instant,
    /// Notes added during the current step; listed after its timing.
    notes: Vec<String>,
}

impl Running {
    /// Records the current step's duration followed by its notes.
    fn close_step(&mut self, now: Instant) {
        if self.stage != "bắt đầu" {
            self.steps
                .push(format!("{} {}", self.stage, secs(now - self.step_started)));
        }
        self.steps.append(&mut self.notes);
    }
}

pub struct Finished {
    pub name: String,
    pub who: String,
    pub at: DateTime<Utc>,
    pub took: Duration,
    pub ok: bool,
    pub detail: String,
}

pub struct Snapshot {
    pub running: Vec<(String, String, Duration, String)>,
    pub recent: Vec<Finished>,
}

#[derive(Default)]
pub struct Tracker {
    running: Mutex<HashMap<u64, Running>>,
    recent: Mutex<VecDeque<Finished>>,
}

pub fn secs(d: Duration) -> String {
    format!("{:.2}s", d.as_secs_f64())
}

impl Tracker {
    /// Starts tracking task `key` (any id unique while the task runs).
    pub fn start(&self, key: u64, name: impl Into<String>, who: impl Into<String>) {
        let now = Instant::now();
        let (name, who) = (name.into(), who.into());
        tracing::info!("▶ {name} · {who}");
        self.running.lock().unwrap().insert(
            key,
            Running {
                name,
                who,
                started: now,
                stage: "bắt đầu".into(),
                steps: Vec::new(),
                step_started: now,
                notes: Vec::new(),
            },
        );
    }

    /// Marks the start of a new step; the previous step's duration is recorded.
    pub fn stage(&self, key: u64, stage: &str) {
        let mut map = self.running.lock().unwrap();
        if let Some(r) = map.get_mut(&key) {
            let now = Instant::now();
            r.close_step(now);
            r.stage = stage.to_string();
            r.step_started = now;
        }
    }

    /// Adds a note to the task's summary without timing it (e.g. "thử 2 lần").
    pub fn note(&self, key: u64, note: impl Into<String>) {
        if let Some(r) = self.running.lock().unwrap().get_mut(&key) {
            r.notes.push(note.into());
        }
    }

    /// Time since task `key` started.
    pub fn elapsed(&self, key: u64) -> Option<Duration> {
        self.running
            .lock()
            .unwrap()
            .get(&key)
            .map(|r| r.started.elapsed())
    }

    pub fn finish(&self, key: u64, ok: bool) {
        let Some(mut r) = self.running.lock().unwrap().remove(&key) else {
            return;
        };
        let took = r.started.elapsed();
        r.close_step(Instant::now());
        let detail = r.steps.join(" · ");
        let mark = if ok { "✔" } else { "✘" };
        if took > Duration::from_secs(5) {
            tracing::warn!(
                "{mark} {} · {} · {} (chậm) [{detail}]",
                r.name,
                r.who,
                secs(took)
            );
        } else {
            tracing::info!("{mark} {} · {} · {} [{detail}]", r.name, r.who, secs(took));
        }
        let mut recent = self.recent.lock().unwrap();
        recent.push_front(Finished {
            name: r.name,
            who: r.who,
            at: Utc::now(),
            took,
            ok,
            detail,
        });
        recent.truncate(KEEP_RECENT);
    }

    pub fn snapshot(&self, recent: usize) -> Snapshot {
        let running = self
            .running
            .lock()
            .unwrap()
            .values()
            .map(|r| {
                (
                    r.name.clone(),
                    r.who.clone(),
                    r.started.elapsed(),
                    r.stage.clone(),
                )
            })
            .collect();
        let recent = self
            .recent
            .lock()
            .unwrap()
            .iter()
            .take(recent)
            .map(|f| Finished {
                name: f.name.clone(),
                who: f.who.clone(),
                at: f.at,
                took: f.took,
                ok: f.ok,
                detail: f.detail.clone(),
            })
            .collect();
        Snapshot { running, recent }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_steps_and_history() {
        let t = Tracker::default();
        t.start(1, "/say", "an");
        t.stage(1, "giọng");
        t.note(1, "thử 2 lần");
        t.stage(1, "phát");
        assert_eq!(t.snapshot(5).running.len(), 1);
        t.finish(1, true);
        let s = t.snapshot(5);
        assert!(s.running.is_empty());
        assert_eq!(s.recent.len(), 1);
        let d = &s.recent[0].detail;
        // The note belongs to the "giọng" step, so it comes right after its timing.
        let parts: Vec<&str> = d.split(" · ").collect();
        assert_eq!(parts.len(), 3, "{d}");
        assert!(parts[0].starts_with("giọng "), "{d}");
        assert_eq!(parts[1], "thử 2 lần");
        assert!(parts[2].starts_with("phát "), "{d}");
    }
}
