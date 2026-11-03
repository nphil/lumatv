//! In-memory TV notifications shown beside the AI audit and plugin notifications in the drawer.
//! Transient LG toasts are displayed separately and do not enter notification history.

use super::hub::Note;
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
        log.post(Item { id: String::new(), source: String::new(), icon: String::new(), title: "One".into(), body: String::new(), time: 1000, actions: Vec::new() });
        let note = |at: f64| Note { plugin_id: "p".into(), plugin: "P".into(), id: "n".into(), revision: 1, channel_icon: String::new(), received_at: at, title: "T".into(), body: String::new(), actions: Vec::new() };
        assert_eq!(log.unread(&[note(5.0)]), 2);
        log.mark_seen(6000);
        assert_eq!(log.unread(&[note(5.0)]), 0);
        log.post(Item { id: String::new(), source: String::new(), icon: String::new(), title: "Two".into(), body: String::new(), time: 7000, actions: Vec::new() });
        assert_eq!(log.unread(&[note(5.0), note(8.0)]), 2);
    }
}
