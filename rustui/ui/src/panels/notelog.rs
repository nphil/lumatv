//! What the TV itself has told the person since it started (`kit/NoteLog.qml` of the QML shell): an update that is ready, a backup that
//! finished, what LG's notification service raised as a toast, the answer to an AI access question. The notification drawer shows this
//! beside what the AI access audit and the plugins say. It lives in memory (a restart is a clean slate: the helper keeps the audit and
//! the broker its own notifications). [`NoteLog::post`] is the one way in, so a surface that has something to say does not know the drawer.
//!
//! LG's toasts reach the log through a subscription of its own (toasts are subscribe-only on this firmware: there is no backlog, so the
//! log only has what arrived while it existed). A toast that a plugin's notification was raised as is not logged again: the drawer already
//! lists it under the plugin. `unread` is what the status bar's bell counts: the log's entries and the plugins' notifications that arrived
//! after the drawer was last open.

use super::hub::Note;
use crate::kit::screen::{Effect, Input};
use serde_json::{Value, json};

pub const FEED_URI: &str = "luna://com.webos.notification/getToastNotification";
/// The tag of this log's own subscription (the toast kind has another).
pub const FEED_TAG: u64 = 0x5100_0100;
/// How many entries are kept.
pub const KEPT: usize = 50;

#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub source: String,
    pub icon: String,
    pub title: String,
    pub body: String,
    /// Milliseconds since the epoch.
    pub time: u64,
    pub actions: Vec<Button>,
}

#[derive(Clone, Debug, Default)]
pub struct NoteLog {
    items: Vec<Item>,
    serial: u64,
    seen_at: u64,
}

impl NoteLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe(&self) -> Vec<Effect> {
        vec![Effect::Luna { tag: FEED_TAG, uri: FEED_URI.into(), payload: json!({"subscribe": true}), subscribe: true }]
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// What the bell's badge counts.
    pub fn unread(&self, notes: &[Note]) -> usize {
        self.items.iter().filter(|i| i.time > self.seen_at).count() + notes.iter().filter(|n| (n.received_at * 1000.0) as u64 > self.seen_at).count()
    }

    pub fn mark_seen(&mut self, now: u64) {
        self.seen_at = now;
    }

    /// An entry of the TV's own: `source` "TV" and the info icon unless given; newest first, at most [`KEPT`].
    pub fn post(&mut self, n: Item) {
        self.serial += 1;
        let mut item = n;
        item.id = format!("tv-{}", self.serial);
        if item.source.is_empty() {
            item.source = "TV".into();
        }
        if item.icon.is_empty() {
            item.icon = "info".into();
        }
        self.items.insert(0, item);
        self.items.truncate(KEPT);
    }

    pub fn dismiss(&mut self, id: &str) {
        self.items.retain(|i| i.id != id);
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// LG's own toast. The first line of the message is the title, the rest the subtitle (LG writes a line break as `<br>` or a newline).
    /// A toast that says what a plugin's notification of the last half minute said is not logged again, nor is one LG delivered twice in
    /// a row within five seconds.
    pub fn from_toast(&mut self, m: &Value, notes: &[Note], now: u64) -> bool {
        let raw = m.get("message").and_then(Value::as_str).unwrap_or("");
        let lowered = raw.replace("<br>", "\n").replace("<br/>", "\n").replace("<br />", "\n");
        let text = lowered.trim().to_string();
        if text.is_empty() {
            return false;
        }
        let said_by_a_plugin = notes.iter().any(|n| {
            let said = if n.body.is_empty() { n.title.clone() } else { format!("{}\n{}", n.title, n.body) };
            said == text && now.saturating_sub((n.received_at * 1000.0) as u64) < 30_000
        });
        if said_by_a_plugin {
            return false;
        }
        let mut lines = text.split('\n');
        let title = lines.next().unwrap_or("").to_string();
        let body = lines.collect::<Vec<_>>().join(" ");
        if let Some(last) = self.items.first()
            && last.title == title
            && last.body == body
            && now.saturating_sub(last.time) < 5000
        {
            return false;
        }
        self.post(Item { id: String::new(), source: String::new(), icon: String::new(), title, body, time: now, actions: Vec::new() });
        true
    }

    /// Takes a toast of the feed. Returns whether the log changed.
    pub fn update(&mut self, input: &Input, notes: &[Note], now: u64) -> bool {
        match input {
            Input::Luna { tag, payload } if *tag == FEED_TAG => payload.get("message").is_some() && self.from_toast(payload, notes, now),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toast(msg: &str) -> Value {
        json!({"sourceId": "lg", "message": msg})
    }

    #[test]
    fn a_toast_is_an_entry_whose_first_line_is_the_title_and_the_rest_the_subtitle() {
        let mut log = NoteLog::new();
        assert!(log.from_toast(&toast("Remote battery low<br>Replace it soon\nOr charge it"), &[], 1000));
        let i = &log.items()[0];
        assert_eq!((i.title.as_str(), i.body.as_str(), i.source.as_str(), i.icon.as_str(), i.id.as_str()), ("Remote battery low", "Replace it soon Or charge it", "TV", "info", "tv-1"));
        assert!(!log.from_toast(&toast("   "), &[], 2000), "an empty toast is nothing");
    }

    #[test]
    fn a_toast_delivered_twice_in_a_row_is_one_entry_and_newest_comes_first() {
        let mut log = NoteLog::new();
        log.from_toast(&toast("Update ready"), &[], 1000);
        assert!(!log.from_toast(&toast("Update ready"), &[], 3000), "within five seconds of itself");
        assert!(log.from_toast(&toast("Update ready"), &[], 7000), "later it is news again");
        log.from_toast(&toast("Backup done"), &[], 8000);
        assert_eq!(log.items().iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), vec!["Backup done", "Update ready", "Update ready"]);
    }

    #[test]
    fn a_toast_a_plugin_notification_was_raised_as_is_not_logged_again() {
        let mut log = NoteLog::new();
        let note = Note {
            plugin_id: "lock".into(),
            plugin: "Front door".into(),
            id: "n1".into(),
            revision: 1,
            channel_icon: String::new(),
            received_at: 100.0,
            title: "Door".into(),
            body: "Opened".into(),
            actions: Vec::new(),
        };
        assert!(!log.from_toast(&toast("Door\nOpened"), std::slice::from_ref(&note), 100_000 + 5000));
        assert!(log.from_toast(&toast("Door\nOpened"), std::slice::from_ref(&note), 100_000 + 31_000), "after half a minute it is a toast of its own");
    }

    #[test]
    fn the_log_keeps_fifty_and_dismiss_and_clear_empty_it() {
        let mut log = NoteLog::new();
        for i in 0..60 {
            log.post(Item { id: String::new(), source: "TV".into(), icon: "download".into(), title: format!("t{i}"), body: String::new(), time: i, actions: Vec::new() });
        }
        assert_eq!(log.items().len(), KEPT);
        assert_eq!(log.items()[0].title, "t59");
        let id = log.items()[3].id.clone();
        log.dismiss(&id);
        assert_eq!(log.items().len(), KEPT - 1);
        log.clear();
        assert!(log.items().is_empty());
    }

    #[test]
    fn unread_counts_what_came_after_the_drawer_was_last_open_in_the_log_and_among_the_plugins() {
        let mut log = NoteLog::new();
        log.from_toast(&toast("One"), &[], 1000);
        let note = |at: f64| Note { plugin_id: "p".into(), plugin: "P".into(), id: "n".into(), revision: 1, channel_icon: String::new(), received_at: at, title: "T".into(), body: String::new(), actions: Vec::new() };
        assert_eq!(log.unread(&[note(5.0)]), 2);
        log.mark_seen(6000);
        assert_eq!(log.unread(&[note(5.0)]), 0);
        log.from_toast(&toast("Two"), &[], 7000);
        assert_eq!(log.unread(&[note(5.0), note(8.0)]), 2);
    }

    #[test]
    fn only_the_logs_own_feed_is_taken() {
        let mut log = NoteLog::new();
        assert!(!log.update(&Input::Luna { tag: 1, payload: toast("x") }, &[], 0));
        assert!(log.update(&Input::Luna { tag: FEED_TAG, payload: toast("x") }, &[], 0));
        assert!(matches!(log.subscribe().as_slice(), [Effect::Luna { subscribe: true, .. }]));
    }
}
