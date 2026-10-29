//! Whether the receiver's reading may be shown, for every surface that shows it (`kit/ReceiverGate.qml`: the quick panel's tile,
//! Settings' receiver page; the volume display has its own). A value is shown only while it is in date, so a surface can never draw a
//! number that is no longer true:
//!
//! - the helper must be up and the topic must say `fresh` and `reachable`; the helper hides a value itself 6 s after the last good
//!   read and publishes that as an event;
//! - the reading's own `updatedAt` must be no older than `stale_ms`. The helper moves that stamp quietly (a poll that changes nothing
//!   raises no event), so the copy this gate holds is as old as the last real change. When it ages out the gate shows nothing and, for
//!   a surface that is on screen, asks the helper for the topic's newest copy (`GET /v1/state/receiver`): a receiver that is still
//!   answering has a new stamp there and the value comes back; a helper that cannot be reached leaves it hidden.
//!
//! `doc()` is the newest copy while it may be shown and `None` otherwise. While the gate is not `active` (its surface is hidden)
//! nothing is tracked and no timer runs; on becoming active it looks again.

use crate::kit::screen::{Effect, Input};
use serde_json::Value;

pub const TAG: u64 = 0x5200_0001;
pub const STALE_MS: u64 = 6000;

#[derive(Clone, Debug)]
pub struct Gate {
    active: bool,
    helper_up: bool,
    stale_ms: u64,
    topic: Option<Value>,
    answer: Option<Value>,
    topic_at: u64,
    answer_at: u64,
    stale: bool,
    expiry: Option<u64>,
    asking: bool,
}

impl Default for Gate {
    fn default() -> Self {
        Self { active: false, helper_up: false, stale_ms: STALE_MS, topic: None, answer: None, topic_at: 0, answer_at: 0, stale: true, expiry: None, asking: false }
    }
}

impl Gate {
    pub fn new() -> Self {
        Self::default()
    }

    fn newest(&self) -> Option<&Value> {
        match (&self.answer, &self.topic) {
            (Some(a), _) if self.answer_at >= self.topic_at => Some(a),
            (_, t) => t.as_ref(),
        }
    }

    /// The newest copy, in date or not (the tile's name and model come from it; its numbers only from [`Gate::doc`]).
    pub fn rc(&self) -> Option<&Value> {
        self.newest()
    }

    pub fn fresh(&self) -> bool {
        let r = self.newest();
        self.active && self.helper_up && r.is_some_and(|r| r.get("reachable").and_then(Value::as_bool) == Some(true) && r.get("fresh").and_then(Value::as_bool) == Some(true)) && !self.stale
    }

    pub fn doc(&self) -> Option<&Value> {
        if self.fresh() { self.newest() } else { None }
    }

    /// The topic says the receiver is switched off in Settings: there is nothing to wait for.
    pub fn disabled(&self) -> bool {
        self.newest().is_some_and(|r| r.get("enabled").and_then(Value::as_bool) == Some(false))
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    fn evaluate(&mut self, now: u64) -> Vec<Effect> {
        self.expiry = None;
        let Some(r) = self.newest() else {
            self.stale = true;
            return Vec::new();
        };
        if !self.active {
            self.stale = true;
            return Vec::new();
        }
        let at = r.get("updatedAt").and_then(Value::as_f64).unwrap_or(0.0);
        let remaining = self.stale_ms as f64 - (now as f64 - at);
        let ok = r.get("fresh").and_then(Value::as_bool) == Some(true)
            && r.get("reachable").and_then(Value::as_bool) == Some(true)
            && at.is_finite()
            && at > 0.0
            && remaining > 0.0
            && remaining <= self.stale_ms as f64 + 1000.0;
        self.stale = !ok;
        if self.stale {
            return Vec::new();
        }
        self.expiry = Some(now + remaining.min(self.stale_ms as f64).ceil().max(1.0) as u64);
        Vec::new()
    }

    /// Asks the helper for the topic's newest copy.
    fn confirm(&mut self) -> Vec<Effect> {
        if self.asking || !self.helper_up {
            return Vec::new();
        }
        self.asking = true;
        vec![Effect::Fetch { tag: TAG, path: "/v1/state/receiver".into() }]
    }

    /// The surface is on screen (`true`) or gone: on becoming active the gate looks again and, if the copy it has is old, asks.
    pub fn set_active(&mut self, active: bool, now: u64) -> Vec<Effect> {
        if self.active == active {
            return Vec::new();
        }
        self.active = active;
        let mut out = self.evaluate(now);
        if active && self.stale {
            out.extend(self.confirm());
        }
        out
    }

    /// The helper answers (or stopped answering).
    pub fn helper(&mut self, up: bool, now: u64) {
        if self.helper_up != up {
            self.helper_up = up;
            self.evaluate(now);
        }
    }

    pub fn update(&mut self, input: &Input, now: u64) -> (Vec<Effect>, bool) {
        let before = self.fresh();
        let before_doc = self.doc().cloned();
        let mut out = Vec::new();
        match input {
            Input::Helper { topic, data } if topic == "up" => self.helper(data.as_bool().unwrap_or(false), now),
            Input::Helper { topic, data } if topic == "receiver" => {
                self.topic = Some(data.clone());
                self.topic_at = now;
                out = self.evaluate(now);
            }
            Input::Fetched { tag, status, data } if *tag == TAG => {
                self.asking = false;
                if *status == 200
                    && let Some(v) = data.get("value").filter(|v| !v.is_null())
                {
                    self.answer = Some(v.clone());
                    self.answer_at = now;
                    out = self.evaluate(now);
                }
            }
            Input::Tick if self.expiry.is_some_and(|e| now >= e) => {
                self.expiry = None;
                self.stale = true;
                if self.active {
                    out = self.confirm();
                }
            }
            _ => {}
        }
        let changed = before != self.fresh() || before_doc.as_ref() != self.doc();
        (out, changed)
    }

    pub fn due_at(&self) -> Option<u64> {
        self.expiry
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rx(at: u64) -> Value {
        json!({"enabled": true, "reachable": true, "fresh": true, "power": "on", "input": "TV Audio", "volumeText": "-34.5 dB", "model": "Denon AVR-S760H", "soundMode": "Dolby Surround", "updatedAt": at})
    }

    fn up(g: &mut Gate, now: u64) {
        g.update(&Input::Helper { topic: "up".into(), data: json!(true) }, now);
    }

    /// The surface comes up with nothing yet: the gate asks the helper once, and the helper has nothing newer.
    fn activate(g: &mut Gate, now: u64) {
        let e = g.set_active(true, now);
        assert!(matches!(e.as_slice(), [Effect::Fetch { .. }]), "a gate with no copy asks on coming up");
        g.update(&Input::Fetched { tag: TAG, status: 200, data: json!({"ok": true, "value": null}) }, now);
    }

    #[test]
    fn a_reading_is_shown_while_in_date_and_hidden_six_seconds_after_the_last_and_the_next_shows_again() {
        let mut g = Gate::new();
        up(&mut g, 0);
        activate(&mut g, 0);
        g.update(&Input::Helper { topic: "receiver".into(), data: rx(1000) }, 1000);
        assert!(g.doc().is_some_and(|d| d["volumeText"] == "-34.5 dB"));
        assert_eq!(g.due_at(), Some(7000));
        let (e, changed) = g.update(&Input::Tick, 7000);
        assert!(changed && g.doc().is_none(), "the number is gone");
        assert!(matches!(e.as_slice(), [Effect::Fetch { .. }]), "the helper is asked for its newest copy");
        g.update(&Input::Fetched { tag: TAG, status: 200, data: json!({"ok": true, "value": null}) }, 7100);
        assert!(g.doc().is_none(), "a helper that has nothing newer leaves it hidden");
        g.update(&Input::Helper { topic: "receiver".into(), data: rx(7200) }, 7200);
        assert!(g.doc().is_some(), "the next reading shows");
    }

    #[test]
    fn a_reading_the_helper_still_calls_fresh_is_not_shown_once_its_own_time_is_old() {
        let mut g = Gate::new();
        up(&mut g, 0);
        g.set_active(true, 0);
        g.update(&Input::Helper { topic: "receiver".into(), data: rx(1000) }, 8000);
        assert!(g.doc().is_none(), "stamped 7 s ago: stale at once");
        assert!(g.rc().is_some_and(|r| r["model"] == "Denon AVR-S760H"), "the name is still known");
    }

    #[test]
    fn a_stamp_the_helper_moves_quietly_keeps_the_reading_and_a_silent_helper_loses_it() {
        let mut g = Gate::new();
        up(&mut g, 0);
        activate(&mut g, 0);
        g.update(&Input::Helper { topic: "receiver".into(), data: rx(1000) }, 1000);
        // at 7 s the copy ages out and the helper is asked; it has a newer stamp (quietly moved at 4.6 s)
        let (e, _) = g.update(&Input::Tick, 7000);
        assert_eq!(e.len(), 1);
        g.update(&Input::Fetched { tag: TAG, status: 200, data: json!({"value": rx(4600)}) }, 7050);
        assert!(g.doc().is_some(), "the reading stays up while the helper keeps reading");
        assert_eq!(g.due_at(), Some(7050 + 3550));
        // then the stamps stop: gone
        let (e, _) = g.update(&Input::Tick, 10_600);
        assert_eq!(e.len(), 1);
        g.update(&Input::Fetched { tag: TAG, status: 200, data: json!({"value": rx(4600)}) }, 10_650);
        assert!(g.doc().is_none());
    }

    #[test]
    fn a_hidden_surface_tracks_nothing_and_looks_again_when_it_comes_up() {
        let mut g = Gate::new();
        up(&mut g, 0);
        g.update(&Input::Helper { topic: "receiver".into(), data: rx(1000) }, 1000);
        assert!(g.doc().is_none() && g.due_at().is_none(), "not active: no value, no timer");
        let e = g.set_active(true, 2000);
        assert!(g.doc().is_some() && e.is_empty());
        g.set_active(false, 3000);
        let e = g.set_active(true, 20_000);
        assert!(g.doc().is_none());
        assert!(matches!(e.as_slice(), [Effect::Fetch { .. }]), "an old copy is confirmed on coming up");
    }

    #[test]
    fn the_helper_down_hides_everything_and_a_receiver_switched_off_is_disabled() {
        let mut g = Gate::new();
        up(&mut g, 0);
        g.set_active(true, 0);
        g.update(&Input::Helper { topic: "receiver".into(), data: rx(1000) }, 1000);
        g.update(&Input::Helper { topic: "up".into(), data: json!(false) }, 1500);
        assert!(g.doc().is_none());
        let mut off = rx(2000);
        off["enabled"] = json!(false);
        g.update(&Input::Helper { topic: "receiver".into(), data: off }, 2000);
        assert!(g.disabled());
    }
}
