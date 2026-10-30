//! A client's last moments as the compositor saw them: its commits, the frame callbacks answered or destroyed, the buffers released, and its
//! destroys, kept in a short ring per client and written as one line when a client that destroyed a surface disconnects. PlxNative's close
//! crashes (a present of the Mali blob that runs after SDL freed the window) were told from clean closes by how long the process lived after the
//! destroy, and nothing on this side said what, if anything, was sent to it in that time.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Events kept per client.
const KEPT: usize = 64;
/// How far before the first destroy the report starts.
const LEAD: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Copy)]
struct Event {
    at: Instant,
    what: &'static str,
    n: u32,
}

/// One client's ring of events and when it first destroyed a surface or a shell surface.
#[derive(Debug, Default, Clone)]
pub struct Trace {
    events: VecDeque<Event>,
    first_destroy: Option<Instant>,
}

impl Trace {
    /// Notes `what` (with a count or an object id `n`); `destroy` marks the first destroy the report is timed from.
    pub fn note(&mut self, what: &'static str, n: u32, destroy: bool) {
        self.note_at(Instant::now(), what, n, destroy);
    }

    fn note_at(&mut self, at: Instant, what: &'static str, n: u32, destroy: bool) {
        if self.events.len() == KEPT {
            self.events.pop_front();
        }
        self.events.push_back(Event { at, what, n });
        if destroy {
            self.first_destroy.get_or_insert(at);
        }
    }

    /// The report for a client that disconnected now: `None` when it never destroyed anything.
    pub fn report(&self, pid: i32) -> Option<String> {
        self.report_at(pid, Instant::now())
    }

    fn report_at(&self, pid: i32, now: Instant) -> Option<String> {
        let t0 = self.first_destroy?;
        let ms = |a: Instant, b: Instant| -> f64 {
            if a >= b { a.duration_since(b).as_secs_f64() * 1000.0 } else { -(b.duration_since(a).as_secs_f64() * 1000.0) }
        };
        let from = t0.checked_sub(LEAD).unwrap_or(t0);
        let list: Vec<String> = self
            .events
            .iter()
            .filter(|e| e.at >= from)
            .map(|e| if e.n != 0 { format!("{:+.1} {} {}", ms(e.at, t0), e.what, e.n) } else { format!("{:+.1} {}", ms(e.at, t0), e.what) })
            .collect();
        Some(format!("close trace pid {pid} (ms from its first destroy): {}; disconnect {:+.1}", list.join(", "), ms(now, t0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_that_destroyed_nothing_has_no_report() {
        let mut t = Trace::default();
        t.note("commit", 7, false);
        assert!(t.report(5).is_none());
    }

    #[test]
    fn the_report_is_timed_from_the_first_destroy_and_starts_a_little_before() {
        let base = Instant::now();
        let mut t = Trace::default();
        t.note_at(base, "commit", 7, false);
        t.note_at(base + Duration::from_millis(500), "commit", 8, false);
        t.note_at(base + Duration::from_millis(500), "callbacks-at-commit", 1, false);
        t.note_at(base + Duration::from_millis(520), "shell-destroyed", 0, true);
        t.note_at(base + Duration::from_millis(520), "surface-destroyed", 0, true);
        t.note_at(base + Duration::from_millis(570), "release", 7, false);
        let line = t.report_at(42, base + Duration::from_millis(575)).unwrap();
        assert_eq!(
            line,
            "close trace pid 42 (ms from its first destroy): -20.0 commit 8, -20.0 callbacks-at-commit 1, +0.0 shell-destroyed, +0.0 surface-destroyed, \
             +50.0 release 7; disconnect +55.0"
        );
    }

    #[test]
    fn the_ring_keeps_the_last_events() {
        let mut t = Trace::default();
        for i in 0..(KEPT as u32 + 10) {
            t.note("commit", i + 1, i == 0);
        }
        assert_eq!(t.events.len(), KEPT);
        assert_eq!(t.events.front().unwrap().n, 11);
    }
}
