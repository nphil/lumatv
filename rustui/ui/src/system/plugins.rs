//! What the plugin broker puts on screen, as lists a screen can read (`kit/PluginHub.qml` of the QML shell): the helper publishes the
//! roster (`plugins`) and, per plugin that is connected, its cards, tiles and notifications (`plugin.<id>.cards|tiles|notifications`);
//! every contribution is already validated there, so nothing here checks a plugin's words, it only keeps them. A new notification is told
//! apart from one that was already there when the helper came up: the first answer only marks what is there (1.5 s after the helper is up),
//! so a restart never replays old ones.

use crate::kit::screen::Input;
use lumatv_saver::Complication;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// After the helper comes up its first answers only mark what is already there.
pub const PRIME_MS: u64 = 1500;

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub plugin_id: String,
    pub plugin: String,
    pub id: String,
    pub channel: String,
    pub channel_title: String,
    pub channel_icon: String,
    pub revision: u64,
    pub received_at: f64,
    /// Seconds (the broker's), 0 for none.
    pub expires_at: f64,
    pub title: String,
    pub body: String,
    pub actions: Vec<Value>,
    pub kind: String,
    pub level: String,
    pub privacy: String,
    pub score: Value,
    pub importance: String,
    pub channel_kind: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub plugin_id: String,
    pub plugin: String,
    pub id: String,
    pub revision: u64,
    pub created_at: f64,
    pub expires_at: f64,
    pub priority: String,
    pub content: Value,
    pub actions: Vec<Value>,
}

#[derive(Clone, Debug, Default)]
pub struct Hub {
    /// `(id, name)` of the plugins that are connected, the helper's own too.
    connected: Vec<(String, String)>,
    cards: BTreeMap<String, Value>,
    notes: BTreeMap<String, Value>,
    /// `plugin/id/revision` of every note that has been told of.
    announced: BTreeSet<String>,
    primed: bool,
    prime_due: Option<u64>,
    helper_up: bool,
    saver: Value,
}

/// What a change of the helper's topics means for the screens.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Changed {
    /// New notifications, to be told of once.
    pub posted: Vec<Note>,
    pub notes: bool,
    pub cards: bool,
    pub saver: bool,
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn f(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}

impl Hub {
    pub fn new() -> Self {
        Self::default()
    }

    /// The plugin API's icon names as the TV's glyphs.
    pub fn icon(name: &str) -> String {
        lumatv_saver::glyph_for(name)
    }

    /// Takes in an input of the helper's feed. `now` is wall-clock milliseconds.
    pub fn update(&mut self, input: &Input, now: u64) -> Changed {
        let mut out = Changed::default();
        match input {
            Input::Helper { topic, data } if topic == "up" => {
                let up = data.as_bool().unwrap_or(false);
                if up && !self.helper_up {
                    self.prime_due = Some(now + PRIME_MS);
                }
                self.helper_up = up;
            }
            Input::Helper { topic, data } if topic == "plugins" => {
                let list = |k: &str| data.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
                let all: Vec<Value> = list("plugins").into_iter().chain(list("firstParty")).collect();
                let connected: Vec<(String, String)> = all
                    .iter()
                    .filter(|p| p.get("state").and_then(Value::as_str) == Some("connected"))
                    .map(|p| (s(p, "id"), p.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).map_or_else(|| s(p, "id"), str::to_string)))
                    .collect();
                if connected != self.connected {
                    self.connected = connected;
                    out.cards = true;
                    out.notes = true;
                }
            }
            Input::Helper { topic, data } if topic == "saver" => {
                if *data != self.saver {
                    self.saver = data.clone();
                    out.saver = true;
                }
            }
            Input::Helper { topic, data } if topic.starts_with("plugin.") => {
                if let Some(id) = topic.strip_prefix("plugin.").and_then(|t| t.strip_suffix(".cards")) {
                    if self.cards.get(id) != Some(data) {
                        self.cards.insert(id.to_string(), data.clone());
                        out.cards = true;
                        out.saver = true;
                    }
                } else if let Some(id) = topic.strip_prefix("plugin.").and_then(|t| t.strip_suffix(".notifications"))
                    && self.notes.get(id) != Some(data)
                {
                    self.notes.insert(id.to_string(), data.clone());
                    out.notes = true;
                    if self.primed {
                        out.posted = self.announce(now);
                    }
                }
            }
            Input::Tick if self.prime_due.is_some_and(|t| now >= t) => {
                self.prime_due = None;
                self.announced = self.notes().iter().map(Self::key).collect();
                self.primed = true;
            }
            _ => {}
        }
        out
    }

    pub fn due_at(&self) -> Option<u64> {
        self.prime_due
    }

    fn key(n: &Note) -> String {
        format!("{}/{}/{}", n.plugin_id, n.id, n.revision)
    }

    fn name_of(&self, id: &str) -> String {
        self.connected.iter().find(|(i, _)| i == id).map(|(_, n)| n.clone()).unwrap_or_else(|| id.to_string())
    }

    /// Every notification of every connected plugin.
    pub fn notes(&self) -> Vec<Note> {
        let mut out = Vec::new();
        for (pid, _) in &self.connected {
            let Some(doc) = self.notes.get(pid) else { continue };
            for it in doc.get("items").and_then(Value::as_array).into_iter().flatten() {
                let post = it.get("post").cloned().unwrap_or(Value::Null);
                out.push(Note {
                    plugin_id: pid.clone(),
                    plugin: self.name_of(pid),
                    id: s(it, "id"),
                    channel: s(it, "channel"),
                    channel_title: s(it, "channelTitle"),
                    channel_icon: s(it, "channelIcon"),
                    revision: it.get("revision").and_then(Value::as_u64).unwrap_or(0),
                    received_at: f(it, "receivedAt"),
                    expires_at: f(it, "expiresAt"),
                    title: s(&post, "title"),
                    body: s(&post, "body"),
                    actions: post.get("actions").and_then(Value::as_array).cloned().unwrap_or_default(),
                    kind: if post.get("kind").is_some() { s(&post, "kind") } else { s(it, "kind") },
                    level: s(&post, "level"),
                    privacy: s(&post, "privacy"),
                    score: post.get("score").cloned().unwrap_or(Value::Null),
                    importance: s(it, "importance"),
                    channel_kind: s(it, "channelKind"),
                });
            }
        }
        out
    }

    /// Every card of every connected plugin.
    pub fn cards(&self) -> Vec<Card> {
        let mut out = Vec::new();
        for (pid, _) in &self.connected {
            let Some(doc) = self.cards.get(pid) else { continue };
            for it in doc.get("items").and_then(Value::as_array).into_iter().flatten() {
                let card = it.get("card").cloned().unwrap_or(Value::Null);
                out.push(Card {
                    plugin_id: pid.clone(),
                    plugin: self.name_of(pid),
                    id: s(it, "id"),
                    revision: it.get("revision").and_then(Value::as_u64).unwrap_or(0),
                    created_at: f(it, "createdAt"),
                    expires_at: f(it, "expiresAt"),
                    priority: card.get("priority").and_then(Value::as_str).unwrap_or("new").to_string(),
                    content: card.get("content").cloned().unwrap_or(Value::Null),
                    actions: card.get("actions").and_then(Value::as_array).cloned().unwrap_or_default(),
                });
            }
        }
        out
    }

    /// The notifications not told of before and still in date (their end, in seconds, is ahead of `now`).
    fn announce(&mut self, now: u64) -> Vec<Note> {
        let mut fresh = Vec::new();
        for n in self.notes() {
            if self.announced.insert(Self::key(&n)) && (n.expires_at <= 0.0 || n.expires_at * 1000.0 > now as f64) {
                fresh.push(n);
            }
        }
        fresh
    }

    /// The complications the saver reads: what the helper's board holds (`saver` topic, what tools posted) and the cards of the connected
    /// plugins by the contract's rules (`lumatv_saver::from_cards`), the most urgent first, at most the slots the saver has.
    pub fn complications(&self, now: u64) -> Vec<Complication> {
        let mut all = lumatv_saver::from_document(&self.saver, now);
        for (pid, _) in &self.connected {
            if let Some(doc) = self.cards.get(pid) {
                all.extend(lumatv_saver::from_cards(pid, doc, now));
            }
        }
        all.sort_by(|a, b| {
            (a.priority, std::cmp::Reverse(a.updated_at), &a.source, &a.id).cmp(&(b.priority, std::cmp::Reverse(b.updated_at), &b.source, &b.id))
        });
        all.dedup_by(|a, b| a.source == b.source && a.id == b.id);
        all.truncate(lumatv_saver::MAX_SHOWN);
        all
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: u64 = 1_800_000_000_000;

    fn helper(topic: &str, data: Value) -> Input {
        Input::Helper { topic: topic.into(), data }
    }

    fn roster() -> Input {
        helper(
            "plugins",
            json!({"plugins": [{"id": "ha", "name": "Home Assistant", "state": "connected"}, {"id": "off", "state": "waiting"}], "firstParty": [{"id": "weather", "state": "connected"}]}),
        )
    }

    fn note(id: &str, rev: u64, title: &str, expires: u64) -> Value {
        json!({"id": id, "channel": "alerts", "channelTitle": "Alerts", "channelIcon": "base.icons/home", "revision": rev, "receivedAt": 1, "expiresAt": expires,
               "post": {"title": title, "body": "Door sensor is at 5 %", "actions": [{"action": "clear", "label": "Clear"}]}})
    }

    #[test]
    fn a_notification_is_news_once_and_what_was_there_when_the_helper_came_up_is_not() {
        let mut h = Hub::new();
        h.update(&helper("up", json!(true)), NOW);
        h.update(&roster(), NOW);
        let c = h.update(&helper("plugin.ha.notifications", json!({"items": [note("a", 1, "Low battery", 0)]})), NOW + 100);
        assert!(c.notes && c.posted.is_empty(), "not primed yet: it was there when the helper came up");
        assert_eq!(h.due_at(), Some(NOW + PRIME_MS));
        h.update(&Input::Tick, NOW + PRIME_MS);
        assert!(h.due_at().is_none());
        let same = h.update(&helper("plugin.ha.notifications", json!({"items": [note("a", 1, "Low battery", 0), note("b", 1, "Garage open", 0)]})), NOW + 2000);
        assert_eq!(same.posted.len(), 1, "only the new one");
        assert_eq!((same.posted[0].id.as_str(), same.posted[0].plugin.as_str(), same.posted[0].title.as_str()), ("b", "Home Assistant", "Garage open"));
        let rev =
            h.update(&helper("plugin.ha.notifications", json!({"items": [note("a", 2, "Low battery now", 0), note("b", 1, "Garage open", 0)]})), NOW + 3000);
        assert_eq!(rev.posted.len(), 1, "a new revision is news again");
        let old = h.update(
            &helper(
                "plugin.ha.notifications",
                json!({"items": [note("c", 1, "Old", (NOW / 1000) - 10), note("a", 2, "Low battery now", 0), note("b", 1, "Garage open", 0)]}),
            ),
            NOW + 4000,
        );
        assert!(old.posted.is_empty(), "one already past its end is not announced");
    }

    #[test]
    fn only_connected_plugins_are_read_and_a_restart_of_the_helper_primes_again() {
        let mut h = Hub::new();
        h.update(&roster(), NOW);
        h.update(&helper("plugin.off.notifications", json!({"items": [note("x", 1, "Never", 0)]})), NOW);
        assert!(h.notes().is_empty(), "a plugin that is not connected says nothing");
        h.update(&helper("plugin.ha.notifications", json!({"items": [note("a", 1, "One", 0)]})), NOW);
        assert_eq!(h.notes().len(), 1);
        h.update(&helper("up", json!(true)), NOW + 10);
        h.update(&Input::Tick, NOW + 10 + PRIME_MS);
        h.update(&helper("up", json!(false)), NOW + 5000);
        h.update(&helper("up", json!(true)), NOW + 9000);
        assert_eq!(h.due_at(), Some(NOW + 9000 + PRIME_MS), "the helper came back: what is there is marked again");
        let c = h.update(&helper("plugin.ha.notifications", json!({"items": [note("a", 1, "One", 0), note("z", 1, "Fresh", 0)]})), NOW + 9100);
        assert_eq!(c.posted.len(), 1, "primed before: a fresh one is still news");
    }

    #[test]
    fn complications_are_the_boards_and_the_cards_of_connected_plugins_the_most_urgent_first_and_the_firsts_own_places_left_out() {
        let mut h = Hub::new();
        h.update(&roster(), NOW);
        let card = json!({"items": [{"id": "door", "card": {"priority": "running", "content": {"type": "facts", "title": "Front door", "icon": "lock", "facts": [{"label": "s", "value": "Locked"}]}}}]});
        let c = h.update(&helper("plugin.ha.cards", card), NOW);
        assert!(c.cards && c.saver);
        h.update(
            &helper(
                "plugin.weather.cards",
                json!({"items": [{"id": "w", "card": {"content": {"type": "facts", "title": "Weather", "icon": "sun", "facts": [{"value": "60"}]}}}]}),
            ),
            NOW,
        );
        let doc = json!({"items": [{"id": "ntfy1", "source": "ntfy", "glyph": "bell-ring", "text": "Alert", "value": "", "priority": 1, "expiresAt": NOW + 60_000, "tone": "danger", "updatedAt": NOW}]});
        assert!(h.update(&helper("saver", doc), NOW).saver);
        let all = h.complications(NOW + 1);
        assert_eq!(all.iter().map(|c| (c.source.as_str(), c.id.as_str())).collect::<Vec<_>>(), vec![("ntfy", "ntfy1"), ("ha", "door")]);
        assert!(h.complications(NOW + 120_000).iter().all(|c| c.source != "ntfy"), "the board's item ended");
    }
}
