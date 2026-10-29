//! What Scrypted's detectors say, as the bridge pushes it (`GET /v1/events/stream`, design 3.3): the event record, read with every string
//! bounded, and the ring of the last thirty that the `cameras.events` topic shows. Whether an event becomes a card is [`super::policy`]'s.

use super::policy::{KINDS, Phase};
use serde_json::{Value, json};
use std::collections::VecDeque;

/// How many events the topic keeps, newest first.
pub const RING: usize = 30;

/// One push of the bridge's.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// The event's own id (`e4021`): what `event-thumbnail` takes.
    pub id: String,
    /// The channel's sequence number (the SSE `id`), when it came with one.
    pub seq: Option<u64>,
    pub camera: String,
    pub name: String,
    /// One of the six kinds, or `other`.
    pub kind: String,
    pub label: String,
    pub classes: Vec<String>,
    /// 0 to 1; `None` when the detector gave none (the bridge leaves it out).
    pub score: Option<f64>,
    /// Epoch milliseconds.
    pub at: i64,
    pub phase: Phase,
    /// The bridge has a picture of the moment.
    pub thumbnail: bool,
    /// The NVR has the moment recorded.
    pub recording: bool,
}

fn bounded(v: &Value, max_chars: usize) -> String {
    v.as_str()
        .map(|s| {
            s.chars()
                .filter(|c| !c.is_control())
                .take(max_chars)
                .collect()
        })
        .unwrap_or_default()
}

/// Whether `id` is fit to go in a path, a query and a header: what the bridge's ids are made of.
pub fn plain_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b':' | b'='))
}

impl Event {
    /// Reads the data of an `event: event`; `None` when it names no camera or no event.
    pub fn parse(data: &Value, seq: Option<u64>) -> Option<Self> {
        let id = bounded(&data["id"], 80);
        let camera = bounded(&data["camera"], 80);
        if !plain_id(&id) || !plain_id(&camera) {
            return None;
        }
        let kind = data["kind"]
            .as_str()
            .filter(|k| KINDS.contains(k))
            .unwrap_or("other");
        let at = data["at"].as_i64().unwrap_or(0);
        // an instant that is in seconds (Scrypted's detectors are not all alike) is made milliseconds
        let at = if at != 0 && at < 100_000_000_000 {
            at * 1000
        } else {
            at
        };
        Some(Self {
            id,
            seq,
            name: bounded(&data["name"], 128),
            kind: kind.to_owned(),
            label: bounded(&data["label"], 128),
            classes: data["classes"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|c| bounded(c, 40))
                        .filter(|c| !c.is_empty())
                        .take(8)
                        .collect()
                })
                .unwrap_or_default(),
            score: data["score"]
                .as_f64()
                .filter(|s| s.is_finite())
                .map(|s| s.clamp(0.0, 1.0)),
            at,
            phase: Phase::of(data["phase"].as_str().unwrap_or("new")),
            thumbnail: data["thumbnail"].as_bool().unwrap_or(false),
            recording: data["recording"].as_bool().unwrap_or(false),
            camera,
        })
    }

    /// The record the topic and the shell read: the bridge's own fields, and the channel's sequence number.
    pub fn json(&self) -> Value {
        let mut record = json!({
            "id": self.id,
            "seq": self.seq,
            "camera": self.camera,
            "name": self.name,
            "kind": self.kind,
            "label": self.label,
            "classes": self.classes,
            "at": self.at,
            "phase": match self.phase { Phase::New => "new", Phase::Update => "update", Phase::End => "end" },
            "thumbnail": self.thumbnail,
            "recording": self.recording,
        });
        if let Some(score) = self.score {
            record["score"] = json!(score);
        }
        record
    }
}

/// The last events, newest first.
#[derive(Debug, Default)]
pub struct Ring {
    events: VecDeque<Event>,
}

impl Ring {
    pub fn push(&mut self, event: Event) {
        self.events.push_front(event);
        self.events.truncate(RING);
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }

    /// Takes away what was heard of cameras that are no longer the TV's.
    pub fn retain(&mut self, keep: impl Fn(&Event) -> bool) {
        self.events.retain(keep);
    }

    pub fn json(&self) -> Value {
        json!({"events": self.events.iter().map(Event::json).collect::<Vec<_>>()})
    }

    pub fn find(&self, id: &str) -> Option<&Event> {
        self.events.iter().find(|e| e.id == id)
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        json!({"id": "e4021", "camera": "273", "name": "Front Door", "kind": "person", "label": "Nitin", "classes": ["person", "face"],
               "score": 0.92, "at": 1792609200123i64, "phase": "new", "thumbnail": true, "recording": true})
    }

    #[test]
    fn an_event_is_read_as_the_bridge_sends_it_and_written_back_with_its_sequence() {
        let event = Event::parse(&sample(), Some(4021)).unwrap();
        assert_eq!(
            (
                event.id.as_str(),
                event.camera.as_str(),
                event.kind.as_str(),
                event.label.as_str()
            ),
            ("e4021", "273", "person", "Nitin")
        );
        assert_eq!(
            (
                event.at,
                event.phase,
                event.thumbnail,
                event.recording,
                event.seq
            ),
            (1792609200123, Phase::New, true, true, Some(4021))
        );
        assert_eq!(event.classes, ["person", "face"]);
        let back = event.json();
        for key in [
            "id",
            "seq",
            "camera",
            "name",
            "kind",
            "label",
            "classes",
            "score",
            "at",
            "phase",
            "thumbnail",
            "recording",
        ] {
            assert!(back.get(key).is_some(), "{key}");
        }
        assert_eq!(
            (
                back["seq"].as_u64(),
                back["phase"].as_str(),
                back["score"].as_f64()
            ),
            (Some(4021), Some("new"), Some(0.92))
        );
    }

    #[test]
    fn a_strange_event_is_bounded_or_refused() {
        let mut v = sample();
        v["id"] = json!("e 1");
        assert!(
            Event::parse(&v, None).is_none(),
            "an id with a space is not one"
        );
        let mut v = sample();
        v["camera"] = json!("../x");
        assert!(Event::parse(&v, None).is_none());
        assert!(Event::parse(&json!({"kind": "person"}), None).is_none());
        let mut v = sample();
        v["kind"] = json!("ghost");
        v["label"] = json!(format!("{}\n\u{7}x", "a".repeat(500)));
        v["classes"] = json!((0..20).map(|i| format!("c{i}")).collect::<Vec<_>>());
        v["score"] = json!(7.5);
        v["at"] = json!(1792609200i64);
        let event = Event::parse(&v, None).unwrap();
        assert_eq!(event.kind, "other");
        assert_eq!(event.label.chars().count(), 128);
        assert!(!event.label.contains('\n'));
        assert_eq!(event.classes.len(), 8);
        assert_eq!(event.score, Some(1.0));
        assert_eq!(event.at, 1792609200000, "seconds are made milliseconds");
        let mut v = sample();
        v["phase"] = json!("update");
        assert_eq!(Event::parse(&v, None).unwrap().phase, Phase::Update);
        // a detector that gave no score: the bridge leaves it out, and so does the record
        let mut v = sample();
        v.as_object_mut().unwrap().remove("score");
        let event = Event::parse(&v, None).unwrap();
        assert_eq!(event.score, None);
        assert!(event.json().get("score").is_none());
    }

    #[test]
    fn the_ring_keeps_the_last_thirty_newest_first_and_forgets_a_camera_that_went() {
        let mut ring = Ring::default();
        for i in 0..40 {
            let mut v = sample();
            v["id"] = json!(format!("e{i}"));
            v["camera"] = json!(if i % 2 == 0 { "a" } else { "b" });
            ring.push(Event::parse(&v, Some(i)).unwrap());
        }
        assert_eq!(ring.len(), RING);
        let doc = ring.json();
        assert_eq!(doc["events"][0]["id"], "e39");
        assert_eq!(doc["events"].as_array().unwrap().len(), 30);
        assert!(ring.find("e39").is_some() && ring.find("e3").is_none());
        ring.retain(|e| e.camera == "a");
        assert_eq!(ring.len(), 15);
        ring.clear();
        assert!(ring.is_empty());
        assert_eq!(ring.json()["events"], json!([]));
    }
}
