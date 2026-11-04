//! The set's log is a RAM ring of a mebibyte, and the requests the shell repeats on a timer turned it over in half an hour: a still of
//! each camera on Home (1407 lines in 27 minutes) and the report of the shell's state (1777). Such a request is told the first few
//! times in a minute, and the first line after the minute says how many went unsaid.

use std::collections::HashMap;

/// Lines told per request and window.
const TOLD: u32 = 3;
/// The window, ms.
const WINDOW_MS: u64 = 60_000;
/// Requests remembered before the ones whose window is over are forgotten.
const KEEP: usize = 64;

struct Window {
    since_ms: u64,
    told: u32,
    held: u32,
}

#[derive(Default)]
pub struct Chatter {
    seen: HashMap<String, Window>,
}

impl Chatter {
    /// Whether the request `key` made at `now_ms` is told: `None` when it is held back, else `Some(n)` with `n` the requests held back since
    /// the last one told.
    pub fn allow(&mut self, key: &str, now_ms: u64) -> Option<u32> {
        if let Some(w) = self.seen.get_mut(key) {
            if now_ms.saturating_sub(w.since_ms) >= WINDOW_MS {
                let held = w.held;
                *w = Window { since_ms: now_ms, told: 1, held: 0 };
                return Some(held);
            }
            if w.told < TOLD {
                w.told += 1;
                return Some(0);
            }
            w.held += 1;
            return None;
        }
        if self.seen.len() >= KEEP {
            self.seen.retain(|_, w| now_ms.saturating_sub(w.since_ms) < WINDOW_MS);
            if self.seen.len() >= KEEP {
                // More requests than that in a window are not the kind this is for; they are told afresh.
                self.seen.clear();
            }
        }
        self.seen.insert(key.to_string(), Window { since_ms: now_ms, told: 1, held: 0 });
        Some(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_that_repeats_is_told_a_few_times_a_minute_and_the_next_line_counts_what_went_unsaid() {
        let mut c = Chatter::default();
        let still = "GET /v1/cameras/88/frame";
        let told: Vec<Option<u32>> = (0..10).map(|i| c.allow(still, i * 1_000)).collect();
        assert_eq!(told, [Some(0), Some(0), Some(0), None, None, None, None, None, None, None]);
        assert_eq!(c.allow(still, 59_999), None, "the minute is not over");
        assert_eq!(c.allow(still, 60_000), Some(8), "the first after the minute says what was held");
        assert_eq!(c.allow(still, 61_000), Some(0));
    }

    #[test]
    fn each_request_has_its_own_budget_and_a_slow_one_is_always_told() {
        let mut c = Chatter::default();
        for i in 0..5 {
            c.allow("POST /v1/ui", i * 100);
        }
        assert_eq!(c.allow("GET /v1/cameras/103/frame", 500), Some(0), "another route is not held for this one's");
        for minute in 0..4 {
            assert_eq!(c.allow("POST /v1/receiver/volume", minute * 60_000), Some(0), "once a minute is told every time");
        }
    }

    #[test]
    fn what_is_remembered_is_bounded() {
        let mut c = Chatter::default();
        for i in 0..(KEEP as u64 * 3) {
            c.allow(&format!("GET /v1/cameras/{i}/frame"), i * 100);
        }
        assert!(c.seen.len() <= KEEP, "{}", c.seen.len());
    }
}
