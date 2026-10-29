//! What the pages read besides LG's settings (`host::sio`): the owners of the helper's settings layer (`HelperSettings.qml`), the topics
//! of both feeds, and the answers of the calls a page made for itself. Pure data: the model asks for a call by returning an
//! [`Effect`] and learns the answer from an `Input`, so everything here runs under a test with nothing but a clock.

use crate::host::sio::js;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The luna tags of this kind: one counter from here, so the host can tell them from the other kinds' (`0x7041` the toasts); LG's
/// settings port takes the first 64 k of the range.
pub const TAG_BASE: u64 = 0x5E00_0000;
pub const TAG_END: u64 = 0x5EFF_FFFF;

pub fn ours(tag: u64) -> bool {
    (TAG_BASE..=TAG_END).contains(&tag)
}

/// Where the calls of the pages begin: LG's settings port has the 64 k before.
pub const PAGE_TAGS: u64 = TAG_BASE + crate::host::sio::TAGS;

/// The app list's subscription (and its re-ask): two tags under the one the launch call of the shell takes.
pub const APPS_TAG: u64 = TAG_END - 0x100;

/// The counter every call of a page takes its tag from.
#[derive(Debug)]
pub struct Tags {
    next: u64,
}

impl Default for Tags {
    fn default() -> Self {
        Self { next: PAGE_TAGS }
    }
}

impl Tags {
    pub fn take(&mut self) -> u64 {
        let t = self.next;
        self.next = if self.next + 1 >= APPS_TAG { PAGE_TAGS } else { self.next + 1 };
        t
    }
}

/// One owner of the helper's settings layer (`GET/POST /v1/settings/<owner>`): the effective values, the kinds of the fields, the
/// schema as the helper serves it. A change is sent as a transaction the helper checks against the owner's schema; the value shown
/// after it is the one the helper answered with, a refused change leaves the row as it was. Changes go one at a time in the order
/// they were made, a change to a key still waiting replaced by the newer one.
#[derive(Debug, Default)]
pub struct Owner {
    pub id: String,
    /// False until the helper answers with this owner (it is down, or an older one does not have it).
    pub known: bool,
    pub version: i64,
    values: BTreeMap<String, Value>,
    kinds: BTreeMap<String, String>,
    pub schema: Value,
    /// The helper's words for the last refused change.
    pub problem: String,
    queue: Vec<(String, Value)>,
    busy: bool,
}

impl Owner {
    pub fn new(id: &str) -> Self {
        Self { id: id.to_string(), ..Self::default() }
    }

    pub fn path(&self) -> String {
        format!("/v1/settings/{}", self.id)
    }

    pub fn has(&self, key: &str) -> bool {
        self.known && self.values.contains_key(key)
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }

    pub fn num(&self, key: &str, fallback: f64) -> f64 {
        self.values.get(key).and_then(js::float).unwrap_or(fallback)
    }

    pub fn text(&self, key: &str) -> Option<String> {
        self.values.get(key).map(js::string)
    }

    /// Takes a document of the owner: `{effective: {key: {value}}, schema: {sections: [{fields: [{key, kind: {type}}]}]}, version}`.
    pub fn take(&mut self, doc: &Value) {
        self.values = doc.get("effective").and_then(Value::as_object).map(|e| e.iter().map(|(k, v)| (k.clone(), v.get("value").cloned().unwrap_or(Value::Null))).collect()).unwrap_or_default();
        self.kinds.clear();
        for section in doc.pointer("/schema/sections").and_then(Value::as_array).into_iter().flatten() {
            for field in section.get("fields").and_then(Value::as_array).into_iter().flatten() {
                if let (Some(k), Some(t)) = (field.get("key").and_then(Value::as_str), field.pointer("/kind/type").and_then(Value::as_str)) {
                    self.kinds.insert(k.to_string(), t.to_string());
                }
            }
        }
        self.schema = doc.get("schema").cloned().unwrap_or(Value::Null);
        self.version = doc.get("version").and_then(Value::as_i64).unwrap_or(-1);
        self.known = true;
    }

    /// The helper's answer to the fetch: a document, or an owner it does not have.
    pub fn fetched(&mut self, status: u16, body: &Value) {
        if status == 200 && body.get("ok").and_then(Value::as_bool) == Some(true) {
            self.take(body);
        } else {
            self.known = false;
        }
    }

    /// `value` as the row builders hand it, typed as the field is (a string from a segment, a number from a slider).
    fn typed(&self, key: &str, value: &Value) -> Option<Value> {
        let kind = self.kinds.get(key)?;
        Some(match kind.as_str() {
            "bool" => json!({"type": "bool", "value": value.as_bool().unwrap_or_else(|| js::string(value) == "true")}),
            "integer" => json!({"type": "integer", "value": js::float(value).unwrap_or(0.0).round() as i64}),
            "ordered-set" => json!({"type": "ordered", "value": value}),
            other => json!({"type": other, "value": js::string(value)}),
        })
    }

    /// Changes one key: shown at once, queued behind the change in flight. The request that goes, when one can.
    pub fn write(&mut self, key: &str, value: &Value) -> Option<Value> {
        if !self.known {
            return None;
        }
        let typed = self.typed(key, value)?;
        self.values.insert(key.to_string(), typed["value"].clone());
        self.problem.clear();
        self.queue.retain(|(k, _)| k != key);
        self.queue.push((key.to_string(), typed));
        self.next_change()
    }

    /// The body of the next change, when none is in flight.
    pub fn next_change(&mut self) -> Option<Value> {
        if self.busy || self.queue.is_empty() {
            return None;
        }
        let (key, value) = self.queue.remove(0);
        self.busy = true;
        Some(json!({"changes": [{"key": key, "value": value}]}))
    }

    /// The answer to a change: the owner's new document (when no later change is waiting: its value is on screen already), or the
    /// helper's words for a refusal, after which the page asks again for what is really in force. Returns whether to fetch again.
    pub fn changed(&mut self, status: u16, body: &Value) -> bool {
        self.busy = false;
        if status == 200 && body.get("ok").and_then(Value::as_bool) == Some(true) && body.get("settings").is_some() {
            if self.queue.is_empty() {
                self.take(&body["settings"]);
            }
            false
        } else {
            self.problem = body.pointer("/error/message").and_then(Value::as_str).unwrap_or("The helper refused the change or did not answer.").to_string();
            self.queue.clear();
            true
        }
    }
}

/// The topics of the two feeds, by name: the compositor's (`foreground`, `audio`, ...) and the helper's (`profiles`, `openapps`, ...).
#[derive(Debug, Default)]
pub struct Topics {
    channel: BTreeMap<String, Value>,
    helper: BTreeMap<String, Value>,
    /// Whether the helper answers.
    pub helper_up: bool,
}

impl Topics {
    pub fn set_channel(&mut self, topic: &str, data: Value) {
        self.channel.insert(topic.to_string(), data);
    }

    pub fn set_helper(&mut self, topic: &str, data: Value) {
        self.helper.insert(topic.to_string(), data);
    }

    pub fn channel(&self, topic: &str) -> Option<&Value> {
        self.channel.get(topic).filter(|v| !v.is_null())
    }

    pub fn helper(&self, topic: &str) -> Option<&Value> {
        self.helper.get(topic).filter(|v| !v.is_null())
    }
}

/// Named documents: the answers of the calls a page made for itself (`device`, `os`, `storage`, ...), and the page's own state.
#[derive(Debug, Default)]
pub struct Bag {
    map: BTreeMap<String, Value>,
}

impl Bag {
    pub fn get(&self, key: &str) -> &Value {
        static NULL: Value = Value::Null;
        self.map.get(key).unwrap_or(&NULL)
    }

    /// A field of a document, or null.
    pub fn field(&self, key: &str, name: &str) -> &Value {
        static NULL: Value = Value::Null;
        self.get(key).get(name).unwrap_or(&NULL)
    }

    pub fn has(&self, key: &str) -> bool {
        self.map.get(key).is_some_and(|v| !v.is_null())
    }

    pub fn set(&mut self, key: &str, v: Value) {
        self.map.insert(key.to_string(), v);
    }

    pub fn remove(&mut self, key: &str) {
        self.map.remove(key);
    }

    pub fn text(&self, key: &str) -> String {
        match self.get(key) {
            Value::Null => String::new(),
            v => js::string(v),
        }
    }

    pub fn flag(&self, key: &str) -> bool {
        self.get(key).as_bool().unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_owner_of_the_helper_takes_its_document_and_sends_one_typed_change_at_a_time() {
        let mut o = Owner::new("base.ambient");
        assert!(o.write("enabled", &json!(true)).is_none(), "unknown until the helper answers");
        o.fetched(
            200,
            &json!({"ok": true, "version": 3, "effective": {"enabled": {"type": "bool", "value": false}, "level": {"type": "integer", "value": 4}},
                    "schema": {"sections": [{"id": "a", "fields": [{"key": "enabled", "kind": {"type": "bool"}}, {"key": "level", "kind": {"type": "integer"}}]}]}}),
        );
        assert!(o.known && o.has("level") && o.num("level", 0.0) == 4.0 && o.version == 3);
        let first = o.write("level", &json!(7.4)).expect("the first change goes");
        assert_eq!(first, json!({"changes": [{"key": "level", "value": {"type": "integer", "value": 7}}]}));
        assert_eq!(o.num("level", 0.0), 7.0, "shown at once");
        assert!(o.write("enabled", &json!(true)).is_none(), "one in flight: the next waits");
        assert!(o.write("enabled", &json!("false")).is_none(), "a change to a waiting key replaces it");
        assert!(!o.changed(200, &json!({"ok": true, "settings": {"effective": {"level": {"value": 7}}, "version": 4}})));
        assert_eq!(o.version, 3, "a later change is waiting: its value is on screen already, the document is not taken");
        let next = o.next_change().expect("the waiting change goes");
        assert_eq!(next["changes"][0]["value"], json!({"type": "bool", "value": false}));
        assert!(o.changed(409, &json!({"error": {"message": "refused"}})), "a refusal asks for the real values");
        assert_eq!(o.problem, "refused");
    }

    #[test]
    fn tags_stay_in_this_kinds_range_and_wrap() {
        let mut t = Tags::default();
        assert_eq!(t.take(), PAGE_TAGS);
        t.next = APPS_TAG - 1;
        assert_eq!(t.take(), APPS_TAG - 1);
        assert_eq!(t.take(), PAGE_TAGS);
        assert!(ours(TAG_BASE) && ours(PAGE_TAGS) && !ours(0x7041));
    }
}
