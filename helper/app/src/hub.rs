//! The hub: what the helper knows, as named topics, and the events that say it changed.
//!
//! Every component publishes into the hub and every client reads from it, so there is one place where "the state of the TV
//! side" lives and one clock for change (`seq`, a counter that rises with every event). A *topic* holds the current JSON value
//! of one thing (`screen`, `receiver`, `weather`, `news`); setting a topic to a value it already holds is silent. An *event*
//! with no retained value (`settings` changed) is `emit`ted. Clients take a snapshot, which carries the `seq` it is current
//! to, and then wait for events after that `seq`: nothing between the two is lost, because both happen under one lock.
//!
//! Events are kept in a ring of [`RING_CAPACITY`]. A client that fell behind the ring, or whose `seq` is from before a restart
//! of the helper, is told to `reset`, which means: read the snapshot again.
//!
//! Seam for later phases: plugin topics are `plugin.<id>.<name>` and the MCP server reads the same snapshot; neither needs a
//! new mechanism.

use crate::timeutil::now_ms;
use parking_lot::{Condvar, Mutex};
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

/// How many events are kept for clients that are slow to ask.
pub const RING_CAPACITY: usize = 256;
/// The longest one wait for events may be, whatever the client asks.
pub const MAX_WAIT: Duration = Duration::from_secs(30);

/// One thing that happened.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub seq: u64,
    /// Unix milliseconds.
    pub at: u64,
    pub topic: String,
    /// `changed` for a topic that was set; otherwise what the emitter says.
    pub kind: String,
    pub data: Value,
}

impl Event {
    pub fn to_json(&self) -> Value {
        json!({"seq": self.seq, "at": self.at, "topic": self.topic, "kind": self.kind, "data": self.data})
    }
}

/// Every topic's current value, and the `seq` the snapshot is current to.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub seq: u64,
    pub topics: BTreeMap<String, Value>,
}

/// The answer to "what happened after `seq` N".
#[derive(Clone, Debug, PartialEq)]
pub struct Batch {
    /// The hub's current `seq`: the next `after` to ask with.
    pub seq: u64,
    /// The client cannot be caught up from events (it fell behind the ring, or it knows another run of the helper): read the snapshot.
    pub reset: bool,
    pub events: Vec<Event>,
}

struct Inner {
    seq: u64,
    topics: BTreeMap<String, Value>,
    ring: VecDeque<Event>,
    closed: bool,
}

/// A JSON object without some of its top-level keys (any other value is returned as it is).
fn without_keys(value: &Value, keys: &[&str]) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(k, _)| !keys.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ),
        other => other.clone(),
    }
}

pub struct Hub {
    inner: Mutex<Inner>,
    changed: Condvar,
    boot: u64,
}

impl Hub {
    /// A hub for the run that started at `boot` (Unix milliseconds): the boot id clients compare to notice a restart.
    pub fn new(boot: u64) -> Self {
        Self {
            inner: Mutex::new(Inner {
                seq: 0,
                topics: BTreeMap::new(),
                ring: VecDeque::with_capacity(RING_CAPACITY),
                closed: false,
            }),
            changed: Condvar::new(),
            boot,
        }
    }

    /// The boot id: when this run of the helper started.
    pub fn boot(&self) -> u64 {
        self.boot
    }

    fn push(inner: &mut Inner, topic: &str, kind: &str, data: Value) {
        inner.seq += 1;
        if inner.ring.len() == RING_CAPACITY {
            inner.ring.pop_front();
        }
        let event = Event {
            seq: inner.seq,
            at: now_ms(),
            topic: topic.to_owned(),
            kind: kind.to_owned(),
            data,
        };
        inner.ring.push_back(event);
    }

    /// Sets a topic to `value`. Returns whether it changed; a topic set to what it already holds raises no event.
    pub fn set(&self, topic: &str, value: Value) -> bool {
        let mut inner = self.inner.lock();
        if inner.topics.get(topic) == Some(&value) {
            return false;
        }
        inner.topics.insert(topic.to_owned(), value.clone());
        Self::push(&mut inner, topic, "changed", value);
        drop(inner);
        self.changed.notify_all();
        true
    }

    /// Like [`Hub::set`], but a change that touches only the top-level keys in `quiet` is stored without an event: a
    /// "last updated" stamp that moves on every poll must not wake every long poll on the TV. The retained value is always the
    /// newest, so a snapshot shows the fresh stamp.
    pub fn set_ignoring(&self, topic: &str, value: Value, quiet: &[&str]) -> bool {
        let mut inner = self.inner.lock();
        if let Some(current) = inner.topics.get(topic) {
            if current == &value {
                return false;
            }
            if without_keys(current, quiet) == without_keys(&value, quiet) {
                inner.topics.insert(topic.to_owned(), value);
                return false;
            }
        }
        inner.topics.insert(topic.to_owned(), value.clone());
        Self::push(&mut inner, topic, "changed", value);
        drop(inner);
        self.changed.notify_all();
        true
    }

    /// Raises an event that has no retained value.
    pub fn emit(&self, topic: &str, kind: &str, data: Value) {
        let mut inner = self.inner.lock();
        Self::push(&mut inner, topic, kind, data);
        drop(inner);
        self.changed.notify_all();
    }

    /// A topic's current value.
    pub fn get(&self, topic: &str) -> Option<Value> {
        self.inner.lock().topics.get(topic).cloned()
    }

    /// Every topic and the `seq` they are current to, read together.
    pub fn snapshot(&self) -> Snapshot {
        let inner = self.inner.lock();
        Snapshot {
            seq: inner.seq,
            topics: inner.topics.clone(),
        }
    }

    /// One topic and the `seq` it is current to, read together.
    pub fn topic_at(&self, topic: &str) -> (u64, Option<Value>) {
        let inner = self.inner.lock();
        (inner.seq, inner.topics.get(topic).cloned())
    }

    fn collect(
        inner: &Inner,
        boot: u64,
        after: Option<u64>,
        client_boot: Option<u64>,
        topics: Option<&[String]>,
    ) -> Batch {
        let seq = inner.seq;
        let mut reset = client_boot.is_some_and(|b| b != boot);
        let Some(after) = after else {
            return Batch {
                seq,
                reset,
                events: Vec::new(),
            };
        };
        // Newer than anything this run has produced: the client's `seq` is from before a restart.
        reset |= after > seq;
        // Events between the client's `seq` and the oldest kept one are gone.
        let oldest = inner.ring.front().map_or(seq + 1, |e| e.seq);
        reset |= after + 1 < oldest;
        if reset {
            return Batch {
                seq,
                reset: true,
                events: Vec::new(),
            };
        }
        let events = inner
            .ring
            .iter()
            .filter(|e| e.seq > after)
            .filter(|e| topics.is_none_or(|t| t.iter().any(|name| name == &e.topic)))
            .cloned()
            .collect();
        Batch {
            seq,
            reset: false,
            events,
        }
    }

    /// The events after `after` without waiting. `after: None` only reports the current `seq`.
    pub fn events_after(
        &self,
        after: Option<u64>,
        client_boot: Option<u64>,
        topics: Option<&[String]>,
    ) -> Batch {
        let inner = self.inner.lock();
        Self::collect(&inner, self.boot, after, client_boot, topics)
    }

    /// The events after `after`, waiting up to `timeout` (at most [`MAX_WAIT`]) for one to come. Returns at once when there are
    /// events, when the client must reset, when `after` is `None`, or when the hub is closed.
    pub fn wait_events(
        &self,
        after: Option<u64>,
        client_boot: Option<u64>,
        topics: Option<&[String]>,
        timeout: Duration,
    ) -> Batch {
        let deadline = Instant::now() + timeout.min(MAX_WAIT);
        let mut inner = self.inner.lock();
        loop {
            let batch = Self::collect(&inner, self.boot, after, client_boot, topics);
            if batch.reset || !batch.events.is_empty() || after.is_none() || inner.closed {
                return batch;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return batch;
            }
            self.changed.wait_for(&mut inner, left);
        }
    }

    /// Wakes every waiter and makes later waits return at once: the helper is stopping.
    pub fn close(&self) {
        self.inner.lock().closed = true;
        self.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn topics(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn setting_a_topic_raises_one_event_and_the_same_value_none() {
        let hub = Hub::new(1);
        assert!(hub.set("screen", json!({"on": true})));
        assert!(!hub.set("screen", json!({"on": true})));
        assert!(hub.set("screen", json!({"on": false})));
        let batch = hub.events_after(Some(0), Some(1), None);
        assert_eq!(batch.seq, 2);
        assert!(!batch.reset);
        let kinds: Vec<_> = batch
            .events
            .iter()
            .map(|e| (e.seq, e.topic.as_str(), e.kind.as_str(), e.data.clone()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (1, "screen", "changed", json!({"on": true})),
                (2, "screen", "changed", json!({"on": false})),
            ]
        );
    }

    #[test]
    fn a_quiet_key_updates_the_value_without_an_event() {
        let hub = Hub::new(1);
        assert!(hub.set_ignoring(
            "receiver",
            json!({"volume": 5, "updatedAt": 100}),
            &["updatedAt"]
        ));
        assert!(!hub.set_ignoring(
            "receiver",
            json!({"volume": 5, "updatedAt": 200}),
            &["updatedAt"]
        ));
        assert_eq!(
            hub.get("receiver"),
            Some(json!({"volume": 5, "updatedAt": 200}))
        );
        assert_eq!(hub.snapshot().seq, 1);
        assert!(hub.set_ignoring(
            "receiver",
            json!({"volume": 6, "updatedAt": 300}),
            &["updatedAt"]
        ));
        assert_eq!(hub.snapshot().seq, 2);
        // A key that appears or disappears is a change even when it is quiet's neighbour.
        assert!(hub.set_ignoring(
            "receiver",
            json!({"volume": 6, "updatedAt": 300, "extra": 1}),
            &["updatedAt"]
        ));
        assert!(!hub.set_ignoring(
            "receiver",
            json!({"volume": 6, "updatedAt": 301, "extra": 1}),
            &["updatedAt"]
        ));
    }

    #[test]
    fn a_snapshot_is_current_to_its_seq() {
        let hub = Hub::new(1);
        hub.set("a", json!(1));
        hub.set("b", json!(2));
        hub.emit("settings", "changed", json!({"owner": "base.weather"}));
        let snap = hub.snapshot();
        assert_eq!(snap.seq, 3);
        assert_eq!(snap.topics.len(), 2);
        assert_eq!(hub.topic_at("a"), (3, Some(json!(1))));
        assert_eq!(hub.topic_at("nope"), (3, None));
        // Events after the snapshot's seq are exactly what changed since.
        hub.set("a", json!(5));
        let batch = hub.events_after(Some(snap.seq), None, None);
        assert_eq!(batch.events.len(), 1);
        assert_eq!(batch.events[0].data, json!(5));
    }

    #[test]
    fn topic_filters_keep_the_seq_moving() {
        let hub = Hub::new(1);
        hub.set("a", json!(1));
        hub.set("b", json!(2));
        let only_b = topics(&["b"]);
        let batch = hub.events_after(Some(0), None, Some(&only_b));
        assert_eq!(batch.events.len(), 1);
        assert_eq!(batch.events[0].topic, "b");
        assert_eq!(batch.seq, 2);
        // Asking again from the returned seq finds nothing, and does not re-deliver.
        assert!(
            hub.events_after(Some(batch.seq), None, Some(&only_b))
                .events
                .is_empty()
        );
    }

    #[test]
    fn a_client_from_another_run_or_ahead_of_us_must_reset() {
        let hub = Hub::new(1000);
        hub.set("a", json!(1));
        assert!(hub.events_after(Some(1), Some(999), None).reset);
        assert!(!hub.events_after(Some(1), Some(1000), None).reset);
        assert!(hub.events_after(Some(50), None, None).reset);
        // Without `after` the call only reports the seq (and a boot mismatch).
        let first = hub.events_after(None, None, None);
        assert_eq!((first.seq, first.reset, first.events.len()), (1, false, 0));
        assert!(hub.events_after(None, Some(1), None).reset);
    }

    #[test]
    fn a_client_that_fell_behind_the_ring_must_reset() {
        let hub = Hub::new(1);
        for i in 0..(RING_CAPACITY as u64 + 10) {
            hub.emit("t", "tick", json!(i));
        }
        assert!(hub.events_after(Some(0), None, None).reset);
        assert!(hub.events_after(Some(5), None, None).reset);
        // A client just inside the ring is served from events.
        let seq = hub.snapshot().seq;
        let batch = hub.events_after(Some(seq - 20), None, None);
        assert!(!batch.reset);
        assert_eq!(batch.events.len(), 20);
    }

    #[test]
    fn waiting_returns_when_an_event_arrives() {
        let hub = Arc::new(Hub::new(1));
        let waiter = {
            let hub = hub.clone();
            std::thread::spawn(move || {
                hub.wait_events(Some(0), None, None, Duration::from_secs(20))
            })
        };
        std::thread::sleep(Duration::from_millis(50));
        hub.set("x", json!("go"));
        let batch = waiter.join().unwrap();
        assert_eq!(batch.events.len(), 1);
        assert_eq!(batch.events[0].data, json!("go"));
    }

    #[test]
    fn waiting_times_out_with_no_events() {
        let hub = Hub::new(1);
        let started = Instant::now();
        let batch = hub.wait_events(Some(0), None, None, Duration::from_millis(60));
        assert!(batch.events.is_empty() && !batch.reset);
        assert!(started.elapsed() >= Duration::from_millis(50));
    }

    #[test]
    fn a_wait_for_other_topics_does_not_wake_early_with_nothing() {
        let hub = Arc::new(Hub::new(1));
        let waiter = {
            let hub = hub.clone();
            std::thread::spawn(move || {
                hub.wait_events(
                    Some(0),
                    None,
                    Some(&topics(&["wanted"])),
                    Duration::from_millis(400),
                )
            })
        };
        std::thread::sleep(Duration::from_millis(30));
        hub.set("other", json!(1));
        std::thread::sleep(Duration::from_millis(30));
        hub.set("wanted", json!(2));
        let batch = waiter.join().unwrap();
        assert_eq!(batch.events.len(), 1);
        assert_eq!(batch.events[0].topic, "wanted");
    }

    #[test]
    fn closing_releases_waiters() {
        let hub = Arc::new(Hub::new(1));
        let waiter = {
            let hub = hub.clone();
            std::thread::spawn(move || {
                let started = Instant::now();
                let batch = hub.wait_events(Some(0), None, None, Duration::from_secs(30));
                (batch.events.len(), started.elapsed())
            })
        };
        std::thread::sleep(Duration::from_millis(30));
        hub.close();
        let (n, elapsed) = waiter.join().unwrap();
        assert_eq!(n, 0);
        assert!(elapsed < Duration::from_secs(5));
    }

    #[test]
    fn event_json_has_the_documented_keys() {
        let hub = Hub::new(1);
        hub.set("screen", json!({"on": true}));
        let e = &hub.events_after(Some(0), None, None).events[0];
        let v = e.to_json();
        let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["at", "data", "kind", "seq", "topic"]);
    }
}
