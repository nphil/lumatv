//! The compositor's standing subscriptions to LG's services, kept alive.
//!
//! The idle timers (`nop/getTimerInfo`) and tvpower's four (prepare off, prepare on, power-off logo, power state) are made once at the
//! start. A refusal there (the service not up yet at a cold boot, or restarted later) used to end the subscription for good, with no sound:
//! no timer snapshot meant the saver and LG's standby timers never ran, and no tvpower registration meant the standby handshake went
//! unanswered. A subscription that answers `returnValue: false` is made again after 1, 2, 4 ... 30 s; one that has not answered at all in
//! [`ANSWER_WAIT`] is made again too, and the timers' is made again every [`REFRESH`] in case a service dropped it unannounced. Time is passed
//! in so a test drives it with a fake clock.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// A subscription that has not answered in this long is made again.
pub const ANSWER_WAIT: Duration = Duration::from_secs(10);
/// The longest wait between two tries after a refusal.
const RETRY_MAX: Duration = Duration::from_secs(30);
/// How often the idle timers' subscription is made anew while it answers: a snapshot is the whole list, so a second one changes nothing.
pub const REFRESH: Duration = Duration::from_secs(600);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum What {
    /// One of tvpower's registrations, by the name of its round.
    Power(&'static str),
    /// The idle timers.
    Timers,
}

#[derive(Debug)]
pub struct Watch {
    pub what: What,
    pub uri: String,
    pub payload: Value,
    pub token: u64,
    sent: Instant,
    answered: bool,
    retry_at: Option<Instant>,
    attempts: u32,
    /// Whether a periodic refresh applies to it.
    refresh: bool,
    pub restarts: u64,
}

impl Watch {
    pub fn new(what: What, uri: String, payload: Value, token: u64, now: Instant) -> Watch {
        Watch { what, uri, payload, token, sent: now, answered: false, retry_at: None, attempts: 0, refresh: what == What::Timers, restarts: 0 }
    }

    /// An answer on this subscription's token (not consumed: the handlers go on to act on it). True when it was this one's.
    pub fn note(&mut self, token: u64, payload: &Value, now: Instant) -> bool {
        if token != self.token {
            return false;
        }
        if payload.get("returnValue").and_then(Value::as_bool) == Some(false) {
            self.answered = false;
            let wait = Duration::from_secs(1u64 << self.attempts.min(5)).min(RETRY_MAX);
            self.attempts += 1;
            self.retry_at = Some(now + wait);
        } else {
            self.answered = true;
            self.attempts = 0;
            self.retry_at = None;
        }
        true
    }

    /// Whether the subscription has to be made again at `now`.
    pub fn due(&self, now: Instant) -> bool {
        match self.retry_at {
            Some(at) => now >= at,
            None if self.answered => self.refresh && now.saturating_duration_since(self.sent) >= REFRESH,
            None => now.saturating_duration_since(self.sent) >= ANSWER_WAIT,
        }
    }

    /// It was made again under `token`.
    pub fn restart(&mut self, token: u64, now: Instant) {
        self.token = token;
        self.sent = now;
        self.answered = false;
        self.retry_at = None;
        self.restarts += 1;
    }

    pub fn report(&self) -> Value {
        let what = match self.what {
            What::Power(round) => round,
            What::Timers => "timers",
        };
        json!({"what": what, "answered": self.answered, "attempts": self.attempts, "restarts": self.restarts})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watch(t0: Instant, what: What) -> Watch {
        Watch::new(what, "luna://x/y".into(), json!({"subscribe": true}), 5, t0)
    }
    fn secs(t0: Instant, s: u64) -> Instant {
        t0 + Duration::from_secs(s)
    }

    #[test]
    fn a_refused_subscription_is_made_again_after_a_growing_wait() {
        let t0 = Instant::now();
        let mut w = watch(t0, What::Timers);
        assert!(w.note(5, &json!({"returnValue": false, "errorText": "Service does not exist"}), t0));
        assert!(!w.due(t0));
        assert!(w.due(secs(t0, 1)));
        w.restart(6, secs(t0, 1));
        w.note(6, &json!({"returnValue": false}), secs(t0, 1));
        assert!(!w.due(secs(t0, 2)), "the second wait is two seconds");
        assert!(w.due(secs(t0, 3)));
        for n in 0..8 {
            w.restart(10 + n, secs(t0, 10));
            w.note(10 + n, &json!({"returnValue": false}), secs(t0, 10));
        }
        assert!(!w.due(secs(t0, 39)) && w.due(secs(t0, 40)), "never more than thirty seconds");
    }

    #[test]
    fn a_subscription_that_never_answers_is_made_again() {
        let t0 = Instant::now();
        let w = watch(t0, What::Power("PreparePowerOff"));
        assert!(!w.due(secs(t0, 9)));
        assert!(w.due(secs(t0, 10)));
    }

    #[test]
    fn an_answering_subscription_is_left_alone_but_the_timers_are_refreshed() {
        let t0 = Instant::now();
        let mut power = watch(t0, What::Power("PowerState"));
        let mut timers = watch(t0, What::Timers);
        for w in [&mut power, &mut timers] {
            assert!(w.note(5, &json!({"returnValue": true, "subscribed": true}), t0));
        }
        assert!(!power.due(secs(t0, 100_000)), "tvpower's registrations are never made twice while they answer");
        assert!(!timers.due(secs(t0, 599)));
        assert!(timers.due(secs(t0, 600)));
        timers.restart(9, secs(t0, 600));
        assert!(!timers.due(secs(t0, 601)), "the new call has ten seconds to answer");
    }

    #[test]
    fn an_answer_on_another_token_is_not_this_ones() {
        let t0 = Instant::now();
        let mut w = watch(t0, What::Timers);
        assert!(!w.note(99, &json!({"returnValue": false}), t0));
        assert!(!w.due(t0));
    }
}
