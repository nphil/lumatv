//! The notification drawer (`docs/mocks/13-notifications`, `panels/Drawer.qml` of the QML shell): a panel down the right side, the quick
//! panel's width and place, that opens from the bell on Home's top bar (and a long press of the gear key), with what has happened grouped
//! the way Android does: by who said it (AI access: what an assistant asked for and what was decided, the helper's audit; each plugin:
//! the broker's notifications, with the plugin's own buttons; the TV: the [`NoteLog`], such as an update ready; Do Not Disturb: the alerts it
//! and the snoozes held, the helper's `dnd` topic, forgotten there once the last of them is cleared here).
//! A group is as new as its newest notification and the groups are ordered by it; inside a group the newest comes first.
//! Notifications that say exactly the same (title, text and buttons) are one entry with a count. A group of one entry is that entry
//! under a label; a group of several is a header with a focus of its own (OK opens or folds the group, Right reaches its Clear) and its
//! newest entry, the others behind OK. One column the D-pad moves down; Right and Left move between an item's buttons; OK runs the
//! focused button (an item without buttons: OK clears it); Back closes. "Clear all" empties the TV's own log and hides what the others
//! said, until they say something new.
//!
//! Nothing in it is stored here: the audit is read each time the drawer opens, the broker's notifications come from the helper's
//! topics and the log is the [`NoteLog`], so a restart shows what the helper and the broker still have. Opening it is what marks
//! everything seen (the bell's badge), and what folds every group.

use super::clock;
use super::env::{Env, tags};
use super::hub::{self, Note};
use super::life::{LINGER_MS, Life, SPRING_MS};
use super::notelog::NoteLog;
use super::{K, Out};
use crate::kit::screen::Effect;
use serde_json::{Value, json};
use std::cmp::Reverse;
use std::collections::BTreeSet;

/// The id prefix of what Do Not Disturb held.
const MISSED: &str = "dnd-";

/// What a button of an item does.
#[derive(Clone, Debug, PartialEq)]
pub enum Do {
    /// Resume a paused assistant (the helper then asks on the TV, and only the physical remote answers).
    Resume { assistant: String },
    Plugin { plugin: String, action: String, target: Option<String> },
    /// A button of an entry of the TV's own log: told to whoever posted it.
    Own { id: String, action: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub label: String,
    pub run: Do,
}

/// One notification an entry stands for.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub id: String,
    /// An entry of the TV's own log.
    pub own: bool,
}

/// What the person sees as one notification: the newest of those that said exactly the same, and the others it stands for.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub icon: String,
    pub title: String,
    pub body: String,
    /// Milliseconds since the epoch (0: none).
    pub time: u64,
    pub actions: Vec<Action>,
    /// An entry of the TV's own log.
    pub own: bool,
    /// Every notification this entry stands for, the newest (this one) first.
    pub members: Vec<Member>,
}

impl Item {
    fn new(id: String, icon: &str, title: String, body: String, time: u64, actions: Vec<Action>, own: bool) -> Self {
        let members = vec![Member { id: id.clone(), own }];
        Self { id, icon: icon.into(), title, body, time, actions, own, members }
    }

    /// Whether `other` says what this does: the same title, text and button labels.
    fn says_the_same(&self, other: &Item) -> bool {
        self.title == other.title && self.body == other.body && self.actions.iter().map(|a| &a.label).eq(other.actions.iter().map(|a| &a.label))
    }
}

/// Everything one source said.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub source: String,
    pub icon: String,
    /// Newest first; notifications that said the same are one.
    pub items: Vec<Item>,
}

impl Group {
    pub fn newest(&self) -> u64 {
        self.items.first().map_or(0, |i| i.time)
    }

    /// How many notifications the group holds, those that said the same counted each.
    pub fn count(&self) -> usize {
        self.items.iter().map(|i| i.members.len()).sum()
    }

    /// More than one entry: a header the focus visits, and the entries after the newest behind it.
    pub fn stacked(&self) -> bool {
        self.items.len() > 1
    }
}

/// A group header or an item. What the focus visits is numbered from 1 (the number is the index the focus has): a stacked group's header
/// and each item shown; the label of a group of one entry is not visited (`idx` 0).
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Head { idx: usize, source: String, icon: String, count: usize, expanded: bool },
    Item { idx: usize, item: Item },
}

/// What the focus can be on below Clear all.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Head { source: String, expanded: bool },
    Item(Item),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemView {
    pub idx: usize,
    pub icon: String,
    pub title: String,
    pub body: String,
    pub when: String,
    /// How many notifications said exactly this.
    pub count: usize,
    pub buttons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EntryView {
    Head { idx: usize, source: String, icon: String, count: usize, expanded: bool },
    Item(ItemView),
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub up: bool,
    pub entries: Vec<EntryView>,
    /// How many notifications there are, those that said the same counted each.
    pub count: usize,
    /// How many rows the focus visits below Clear all.
    pub rows: usize,
    /// 0 is Clear all, then the rows in order.
    pub index: usize,
    /// Of an item, its buttons in order; of a header, 0 is the header and 1 its Clear.
    pub button: usize,
    pub over_video: bool,
}

pub struct Drawer {
    pub life: Life,
    pub log: NoteLog,
    index: usize,
    button: usize,
    audit: Vec<Value>,
    assistants: Vec<Value>,
    hidden: BTreeSet<String>,
    /// "Clear all" hides every audit line up to this time (audit lines have no id).
    hidden_before: f64,
    /// The stacked groups that are open, by source.
    expanded: BTreeSet<String>,
}

impl Default for Drawer {
    fn default() -> Self {
        Self::new()
    }
}

/// What an audit line says, and the button it offers: a denial pauses the assistant, and a paused one can be resumed from here.
fn audit_text(e: &Value, assistants: &[Value]) -> (String, String, Vec<Action>) {
    let who = e.pointer("/assistant/name").and_then(Value::as_str).unwrap_or("An assistant").to_string();
    let outcome = e.get("outcome").and_then(Value::as_str).unwrap_or("");
    let said = match outcome {
        "ok" => "allowed".to_string(),
        "denied" => "was denied".into(),
        "expired" => "was not answered".into(),
        "never" => "was blocked".into(),
        "paused" => "was paused".into(),
        other => other.to_string(),
    };
    let what = e.get("message").and_then(Value::as_str).filter(|s| !s.is_empty()).or_else(|| e.get("tool").and_then(Value::as_str)).unwrap_or("").to_string();
    let paused = if outcome == "denied" {
        let id = e.pointer("/assistant/id").and_then(Value::as_str);
        assistants.iter().find(|a| id.is_some() && a.get("id").and_then(Value::as_str) == id && a.get("paused").and_then(Value::as_bool) == Some(true))
    } else {
        None
    };
    let title = if outcome == "ok" { format!("{who}: {what}") } else { format!("{who} {said}") };
    let body = if outcome == "ok" {
        String::new()
    } else if paused.is_some() {
        format!("It asked to {what}; its requests are paused until you resume them.")
    } else {
        what
    };
    let actions = paused
        .map(|p| {
            let name = p.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            let id = p.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            vec![Action { label: format!("Resume {name}"), run: Do::Resume { assistant: id } }]
        })
        .unwrap_or_default();
    (title, body, actions)
}

impl Drawer {
    pub fn new() -> Self {
        Self { life: Life::new(SPRING_MS, SPRING_MS, LINGER_MS), log: NoteLog::new(), index: 1, button: 0, audit: Vec::new(), assistants: Vec::new(), hidden: BTreeSet::new(), hidden_before: 0.0, expanded: BTreeSet::new() }
    }

    pub fn notes(env: &Env) -> Vec<Note> {
        hub::notes(&env.topics)
    }

    /// The groups, newest first, each with its entries newest first and what said the same folded into one.
    pub fn groups(&self, env: &Env) -> Vec<Group> {
        let mut groups: Vec<Group> = Vec::new();
        let mut add = |source: &str, icon: &str, item: Item| match groups.iter_mut().find(|g| g.source == source) {
            Some(g) => g.items.push(item),
            None => groups.push(Group { source: source.to_string(), icon: icon.to_string(), items: vec![item] }),
        };
        for e in &self.audit {
            let at = e.get("at").and_then(Value::as_f64).unwrap_or(0.0);
            let key = format!("audit-{}", at as u64);
            if self.hidden.contains(&key) || at <= self.hidden_before {
                continue;
            }
            let (title, body, actions) = audit_text(e, &self.assistants);
            add("AI access", "bot", Item::new(key, "sparkles", title, body, at as u64, actions, false));
        }
        for n in Self::notes(env) {
            let key = format!("plugin-{}-{}-{}", n.plugin_id, n.id, n.revision);
            if self.hidden.contains(&key) {
                continue;
            }
            let icon = if n.channel_icon.is_empty() { "bell" } else { hub::icon(&n.channel_icon) };
            let actions = n
                .actions
                .iter()
                .map(|a| Action { label: a.label.clone(), run: Do::Plugin { plugin: n.plugin_id.clone(), action: if a.action.is_empty() { a.id.clone() } else { a.action.clone() }, target: a.target.clone() } })
                .collect();
            add(&n.plugin, "plug", Item::new(key, icon, n.title.clone(), n.body.clone(), (n.received_at * 1000.0) as u64, actions, false));
        }
        let dnd = crate::dnd::Dnd::from_json(env.topic(crate::dnd::TOPIC).unwrap_or(&Value::Null));
        for m in &dnd.missed {
            let key = format!("{MISSED}{}-{}", m.at, m.title);
            if self.hidden.contains(&key) {
                continue;
            }
            let icon = if m.kind == crate::dnd::CAMERA { "camera" } else { "bell" };
            add(crate::dnd::MISSED_GROUP, crate::dnd::GLYPH, Item::new(key, icon, m.title.clone(), m.body.clone(), m.at, Vec::new(), false));
        }
        for l in self.log.items() {
            let actions = l.actions.iter().map(|a| Action { label: a.label.clone(), run: Do::Own { id: l.id.clone(), action: a.id.clone() } }).collect();
            add(&l.source, "monitor", Item::new(l.id.clone(), &l.icon, l.title.clone(), l.body.clone(), l.time, actions, true));
        }
        for g in &mut groups {
            // newest first (a stable sort: what has no time keeps its order), then what said the same folds into the newest of it
            g.items.sort_by_key(|i| Reverse(i.time));
            let mut folded: Vec<Item> = Vec::new();
            for it in std::mem::take(&mut g.items) {
                match folded.iter_mut().find(|f| f.says_the_same(&it)) {
                    Some(f) => f.members.extend(it.members),
                    None => folded.push(it),
                }
            }
            g.items = folded;
        }
        groups.sort_by_key(|g| Reverse(g.newest()));
        groups
    }

    /// The entries the column draws: a header per group, then its newest item, or all of them when the group is open.
    pub fn entries(&self, env: &Env) -> Vec<Entry> {
        let mut out = Vec::new();
        let mut n = 0;
        for g in self.groups(env) {
            let expanded = g.stacked() && self.expanded.contains(&g.source);
            let idx = if g.stacked() {
                n += 1;
                n
            } else {
                0
            };
            let shown = if expanded { g.items.len() } else { 1 };
            out.push(Entry::Head { idx, source: g.source.clone(), icon: g.icon.clone(), count: g.count(), expanded });
            for item in g.items.into_iter().take(shown) {
                n += 1;
                out.push(Entry::Item { idx: n, item });
            }
        }
        out
    }

    /// What the focus visits below Clear all, in order.
    pub fn rows(&self, env: &Env) -> Vec<Row> {
        self.entries(env)
            .into_iter()
            .filter_map(|e| match e {
                Entry::Head { idx, source, expanded, .. } if idx > 0 => Some(Row::Head { source, expanded }),
                Entry::Head { .. } => None,
                Entry::Item { item, .. } => Some(Row::Item(item)),
            })
            .collect()
    }

    /// The items that show.
    pub fn items(&self, env: &Env) -> Vec<Item> {
        self.entries(env).into_iter().filter_map(|e| if let Entry::Item { item, .. } = e { Some(item) } else { None }).collect()
    }

    // ---- opening

    /// The drawer is wanted: the focus on the first row, every group folded, everything marked seen, the assistants as they are now, the
    /// audit asked for.
    pub fn open(&mut self, env: &Env, now: u64) -> Out {
        self.index = 1;
        self.button = 0;
        self.expanded.clear();
        self.log.mark_seen(now);
        self.assistants = env.topic("ai").and_then(|a| a.get("assistants")).and_then(Value::as_array).cloned().unwrap_or_default();
        Out::effects(vec![Effect::Fetch { tag: tags::AUDIT, path: "/v1/ai/audit?limit=12".into() }])
    }

    /// The audit came: newest first (the helper's list is oldest first).
    pub fn answer(&mut self, status: u16, data: &Value) -> bool {
        if status != 200 {
            return false;
        }
        let Some(entries) = data.get("entries").and_then(Value::as_array) else { return false };
        let audit: Vec<Value> = entries.iter().rev().cloned().collect();
        let changed = audit != self.audit;
        self.audit = audit;
        changed
    }

    // ---- what the keys and the pointer do

    /// Takes the notifications of `items` out of the drawer: the TV's own from its log, the others hidden by id. When the last of what Do Not
    /// Disturb held goes, the helper forgets the list too.
    fn remove(&mut self, env: &Env, items: &[Item]) -> Vec<Effect> {
        let mut held = false;
        for m in items.iter().flat_map(|i| &i.members) {
            held |= m.id.starts_with(MISSED);
            if m.own {
                self.log.dismiss(&m.id);
            } else {
                self.hidden.insert(m.id.clone());
            }
        }
        if held && !self.groups(env).iter().any(|g| g.source == crate::dnd::MISSED_GROUP) {
            return vec![Effect::Request { tag: tags::DND, method: "DELETE".into(), path: "/v1/dnd/missed".into(), body: None }];
        }
        Vec::new()
    }

    fn clear_all(&mut self, env: &Env) -> Vec<Effect> {
        let all: Vec<Item> = self.groups(env).into_iter().flat_map(|g| g.items).collect();
        self.log.clear();
        let effects = self.remove(env, &all);
        self.hidden_before = self.audit.iter().filter_map(|a| a.get("at").and_then(Value::as_f64)).fold(self.hidden_before.max(0.0), f64::max);
        self.expanded.clear();
        self.index = 0;
        self.button = 0;
        effects
    }

    /// Clears the group `source`: its notifications go, the others stay.
    fn clear_group(&mut self, env: &Env, source: &str) -> Vec<Effect> {
        let effects = match self.groups(env).into_iter().find(|g| g.source == source) {
            Some(g) => self.remove(env, &g.items),
            None => Vec::new(),
        };
        self.expanded.remove(source);
        self.settle(env);
        effects
    }

    fn dismiss(&mut self, env: &Env, it: &Item) -> Vec<Effect> {
        let effects = self.remove(env, std::slice::from_ref(it));
        self.settle(env);
        effects
    }

    /// After something went: the focus stays on a row, and on the first of its buttons.
    fn settle(&mut self, env: &Env) {
        self.index = self.index.min(self.rows(env).len());
        self.button = 0;
    }

    fn toggle(&mut self, source: &str) {
        if !self.expanded.remove(source) {
            self.expanded.insert(source.to_string());
        }
    }

    fn run(&mut self, a: &Action) -> Out {
        match &a.run {
            Do::Resume { assistant } => Out::effects(vec![Effect::Post { path: format!("/v1/ai/assistants/{assistant}/resume"), body: json!({}) }]),
            Do::Plugin { plugin, action, target } => {
                let body = target.as_ref().map_or_else(|| json!({}), |t| json!({"target": t}));
                Out::effects(vec![Effect::Post { path: format!("/v1/plugins/{plugin}/actions/{action}"), body }])
            }
            Do::Own { id, action } => {
                let mut out = Out::default();
                out.events.push(("notelog.action".into(), json!({"id": id, "action": action})));
                out
            }
        }
    }

    pub fn key(&mut self, k: K, env: &Env) -> Out {
        let rows = self.rows(env);
        let row = if self.index > 0 { rows.get(self.index - 1).cloned() } else { None };
        match k {
            K::Back => return Out::close(),
            K::Up => {
                self.index = self.index.saturating_sub(1);
                self.button = 0;
            }
            K::Down => {
                self.index = (self.index + 1).min(rows.len());
                self.button = 0;
            }
            K::Left => {
                if row.is_some() {
                    self.button = self.button.saturating_sub(1);
                }
            }
            K::Right => match &row {
                Some(Row::Head { .. }) => self.button = 1,
                Some(Row::Item(it)) => self.button = (self.button + 1).min(it.actions.len().saturating_sub(1)),
                None => {}
            },
            K::Ok => match row {
                None => {
                    if self.index == 0 {
                        return Out::effects(self.clear_all(env));
                    }
                }
                Some(Row::Head { source, .. }) => {
                    if self.button == 0 {
                        self.toggle(&source);
                    } else {
                        return Out::effects(self.clear_group(env, &source));
                    }
                }
                Some(Row::Item(it)) => {
                    if it.actions.is_empty() {
                        return Out::effects(self.dismiss(env, &it));
                    }
                    let a = it.actions.get(self.button).unwrap_or(&it.actions[0]).clone();
                    return self.run(&a);
                }
            },
            _ => {}
        }
        Out::default()
    }

    /// A click on "Clear all".
    pub fn click_clear(&mut self, env: &Env) -> Out {
        Out::effects(self.clear_all(env))
    }

    /// A click on item `idx` (from 1): an item without buttons is cleared by it, as OK clears it.
    pub fn click_item(&mut self, idx: usize, env: &Env) -> Out {
        self.index = idx;
        self.button = 0;
        if let Some(Row::Item(it)) = self.rows(env).get(idx.wrapping_sub(1)).cloned()
            && it.actions.is_empty()
        {
            return Out::effects(self.dismiss(env, &it));
        }
        Out::default()
    }

    /// A click on button `b` of item `idx`.
    pub fn click_button(&mut self, idx: usize, b: usize, env: &Env) -> Out {
        self.index = idx;
        self.button = b;
        self.key(K::Ok, env)
    }

    /// A click on the header `idx` (from 1) opens or folds its group.
    pub fn click_head(&mut self, idx: usize, env: &Env) {
        self.index = idx;
        self.button = 0;
        self.key(K::Ok, env);
    }

    /// A click on the Clear of the header `idx`.
    pub fn click_head_clear(&mut self, idx: usize, env: &Env) -> Out {
        self.index = idx;
        self.button = 1;
        self.key(K::Ok, env)
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn button(&self) -> usize {
        self.button
    }

    pub fn view(&self, env: &Env, now: u64) -> View {
        let entries = self.entries(env);
        let rows = self.rows(env).len();
        let count = self.groups(env).iter().map(Group::count).sum();
        View {
            up: self.life.presented(),
            entries: entries
                .into_iter()
                .map(|e| match e {
                    Entry::Head { idx, source, icon, count, expanded } => EntryView::Head { idx, source, icon, count, expanded },
                    Entry::Item { idx, item } => EntryView::Item(ItemView {
                        idx,
                        icon: item.icon,
                        title: item.title,
                        body: item.body,
                        when: clock::when(item.time, now),
                        count: item.members.len(),
                        buttons: item.actions.into_iter().map(|a| a.label).collect(),
                    }),
                })
                .collect(),
            count,
            rows,
            index: self.index.min(rows),
            button: self.button,
            over_video: !env.fg.home,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::kit::screen::Input;
    use crate::panels::notelog;

    pub(crate) const NOW: u64 = 1_761_000_000_000;
    const SECS: f64 = (NOW / 1000 - 600) as f64;

    fn feed(env: &mut Env, topic: &str, data: Value) {
        env.update(&Input::Helper { topic: topic.into(), data }, 0);
    }

    fn note(id: &str, body: &str, at: f64) -> Value {
        json!({"id": id, "channel": "events", "revision": 1, "receivedAt": at, "post": {"title": "Battery low", "body": body, "actions": []}})
    }

    pub(crate) fn env() -> Env {
        let mut env = Env::new(json!({}));
        feed(&mut env, "ai", json!({"listener": {"enabled": true}, "assistants": [{"id": "cody", "name": "Cody", "paused": true}]}));
        feed(&mut env, "plugins", json!({"plugins": [{"id": "lock", "name": "Front door", "state": "connected"}], "firstParty": [{"id": "weather", "name": "Weather", "state": "connected"}]}));
        feed(
            &mut env,
            "plugin.lock.notifications",
            json!({"items": [
                {"id": "unlocked", "channel": "events", "revision": 1, "receivedAt": SECS, "post": {"title": "Door unlocked", "body": "Someone opened it.", "actions": [{"id": "lock", "label": "Lock it", "action": "lock"}]}},
                note("battery", "The lock's battery is at 12 %.", SECS)
            ]}),
        );
        env
    }

    fn audit() -> Value {
        json!({"entries": [
            {"assistant": {"id": "cody", "name": "Cody"}, "tool": "tv_get_now_playing", "message": "what is playing", "outcome": "ok", "at": NOW - 420_000},
            {"assistant": {"id": "cody", "name": "Cody"}, "tool": "tv_standby", "message": "turn the TV off", "outcome": "denied", "at": NOW - 300_000}
        ]})
    }

    pub(crate) fn open(env: &Env) -> Drawer {
        let mut d = Drawer::new();
        d.life.show(0);
        d.log.post(notelog::Item { id: String::new(), source: String::new(), icon: String::new(), title: "TV update ready".into(), body: "webOS 10.2.3".into(), time: NOW - 100_000, actions: Vec::new() });
        d.open(env, NOW);
        d.answer(200, &audit());
        d
    }

    /// The titles of the items that show.
    fn titles(d: &Drawer, env: &Env) -> Vec<String> {
        d.items(env).into_iter().map(|i| i.title).collect()
    }

    /// "source:count" of each group, in order.
    fn heads(d: &Drawer, env: &Env) -> Vec<String> {
        d.entries(env).into_iter().filter_map(|e| if let Entry::Head { source, count, .. } = e { Some(format!("{source}:{count}")) } else { None }).collect()
    }

    /// Opens the group the item is in when it is behind its header, and puts the focus on the item.
    fn go_to(d: &mut Drawer, env: &Env, title: &str) {
        for g in d.groups(env) {
            if g.stacked() && g.items.iter().any(|i| i.title == title) {
                d.expanded.insert(g.source);
            }
        }
        let at = d.rows(env).iter().position(|r| matches!(r, Row::Item(i) if i.title == title)).unwrap_or_else(|| panic!("no item titled {title}"));
        d.index = at + 1;
        d.button = 0;
    }

    /// Puts the focus on the header of `source`.
    fn go_to_head(d: &mut Drawer, env: &Env, source: &str) {
        let at = d.rows(env).iter().position(|r| matches!(r, Row::Head { source: s, .. } if s == source)).unwrap_or_else(|| panic!("no header of {source}"));
        d.index = at + 1;
        d.button = 0;
    }

    #[test]
    fn opening_asks_for_the_audit_marks_everything_seen_and_groups_by_who_said_it_the_newest_group_first() {
        let env = env();
        let mut d = Drawer::new();
        d.life.show(0);
        d.log.post(notelog::Item { id: String::new(), source: String::new(), icon: String::new(), title: "TV update ready".into(), body: String::new(), time: NOW - 100_000, actions: Vec::new() });
        assert_eq!(d.log.unread(&Drawer::notes(&env)), 3, "the TV's update and the plugin's two notes are unread before the drawer opens");
        let out = d.open(&env, NOW);
        assert_eq!(out.effects, vec![Effect::Fetch { tag: tags::AUDIT, path: "/v1/ai/audit?limit=12".into() }]);
        assert_eq!(d.log.unread(&Drawer::notes(&env)), 0, "opening is what the bell's count is cleared by");
        d.answer(200, &audit());
        assert_eq!(heads(&d, &env), vec!["TV:1", "AI access:2", "Front door:2"], "the TV spoke 100 s ago, the assistant 300 s, the lock 600 s");
        assert_eq!(d.index(), 1);
        assert_eq!(titles(&d, &env), vec!["TV update ready", "Cody was denied", "Door unlocked"], "each group shows its newest, the audit newest first");
        assert_eq!(d.view(&env, NOW).count, 5);
    }

    #[test]
    fn a_group_of_one_has_a_label_the_focus_does_not_visit_and_a_group_of_several_has_a_header_it_does() {
        let env = env();
        let d = open(&env);
        let e = d.entries(&env);
        assert!(matches!(&e[0], Entry::Head { idx: 0, source, .. } if source == "TV"), "{e:?}");
        assert!(matches!(&e[1], Entry::Item { idx: 1, .. }));
        assert!(matches!(&e[2], Entry::Head { idx: 2, source, count: 2, expanded: false, .. } if source == "AI access"));
        assert!(matches!(&e[3], Entry::Item { idx: 3, .. }));
        assert_eq!(d.view(&env, NOW).rows, 5, "the TV's item, the two headers and the two newest items");
    }

    #[test]
    fn ok_on_a_header_opens_its_group_and_again_folds_it_and_opening_the_drawer_folds_every_group() {
        let env = env();
        let mut d = open(&env);
        go_to_head(&mut d, &env, "AI access");
        assert_eq!(titles(&d, &env).len(), 3);
        d.key(K::Ok, &env);
        assert_eq!(titles(&d, &env), vec!["TV update ready", "Cody was denied", "Cody: what is playing", "Door unlocked"], "the rest of the group, newest first");
        assert!(matches!(d.entries(&env)[2], Entry::Head { expanded: true, .. }));
        assert_eq!(d.index(), 2, "the focus stays on the header");
        d.key(K::Down, &env);
        d.key(K::Down, &env);
        assert_eq!(d.index(), 4, "the opened group's second item is a row of its own");
        d.key(K::Up, &env);
        d.key(K::Up, &env);
        d.key(K::Ok, &env);
        assert_eq!(titles(&d, &env).len(), 3, "OK again folds it");
        d.key(K::Ok, &env);
        d.open(&env, NOW);
        assert_eq!(titles(&d, &env).len(), 3, "opening folds every group");
        assert_eq!(d.index(), 1);
    }

    #[test]
    fn right_on_a_header_reaches_its_clear_and_ok_there_clears_that_group_only() {
        let env = env();
        let mut d = open(&env);
        go_to_head(&mut d, &env, "AI access");
        d.key(K::Right, &env);
        assert_eq!(d.button(), 1);
        d.key(K::Left, &env);
        assert_eq!(d.button(), 0);
        d.key(K::Right, &env);
        d.key(K::Ok, &env);
        assert_eq!(heads(&d, &env), vec!["TV:1", "Front door:2"], "the assistant's group is gone, the others stay");
        assert_eq!((d.index(), d.button()), (2, 0), "the focus rests on the row that took its place");
        d.answer(200, &audit());
        assert_eq!(heads(&d, &env), vec!["TV:1", "Front door:2"], "the audit read again does not bring it back");
        let mut newer = audit();
        newer["entries"].as_array_mut().unwrap().push(json!({"assistant": {"id": "cody", "name": "Cody"}, "tool": "x", "message": "dim the lights", "outcome": "ok", "at": NOW + 1000}));
        d.answer(200, &newer);
        assert_eq!(heads(&d, &env)[0], "AI access:1", "what it says after is shown");
    }

    #[test]
    fn clearing_the_lock_group_hides_its_notes_and_the_tvs_item_is_cleared_from_its_log() {
        let env = env();
        let mut d = open(&env);
        d.key(K::Ok, &env);
        assert!(d.log.items().is_empty(), "OK on the TV's item cleared it from the log");
        go_to_head(&mut d, &env, "Front door");
        d.key(K::Right, &env);
        d.key(K::Ok, &env);
        assert_eq!(heads(&d, &env), vec!["AI access:2"]);
    }

    #[test]
    fn notifications_that_say_exactly_the_same_are_one_entry_with_a_count_and_the_newest_time() {
        let mut env = env();
        feed(
            &mut env,
            "plugin.lock.notifications",
            json!({"items": [
                note("b1", "The lock's battery is at 12 %.", SECS),
                note("b2", "The lock's battery is at 12 %.", SECS + 30.0),
                note("b3", "The lock's battery is at 12 %.", SECS + 60.0),
                note("b4", "The lock's battery is at 11 %.", SECS + 20.0),
                {"id": "unlocked", "channel": "events", "revision": 1, "receivedAt": SECS + 10.0, "post": {"title": "Door unlocked", "body": "Someone opened it.", "actions": []}}
            ]}),
        );
        let mut d = open(&env);
        go_to_head(&mut d, &env, "Front door");
        d.key(K::Ok, &env);
        let door = d.groups(&env).into_iter().find(|g| g.source == "Front door").unwrap();
        assert_eq!(door.items.len(), 3, "three sayings, five notifications");
        assert_eq!(door.count(), 5);
        let first = &door.items[0];
        assert_eq!((first.body.as_str(), first.members.len()), ("The lock's battery is at 12 %.", 3));
        assert_eq!(first.time, (SECS as u64 + 60) * 1000, "the entry is as new as the newest of them");
        assert_eq!(door.items[1].body, "The lock's battery is at 11 %.", "a different text is another entry");
        assert_eq!(door.items[2].title, "Door unlocked", "another title is another entry");
        let counts: Vec<usize> = d.view(&env, NOW).entries.iter().filter_map(|e| if let EntryView::Item(i) = e { Some(i.count) } else { None }).collect();
        assert_eq!(counts, vec![1, 1, 3, 1, 1], "the view carries each entry's count: TV, AI access, then the lock's three");
        go_to(&mut d, &env, "Battery low");
        d.key(K::Ok, &env);
        let door = d.groups(&env).into_iter().find(|g| g.source == "Front door").unwrap();
        assert_eq!(door.count(), 2, "OK on the entry cleared all three");
        assert!(door.items.iter().all(|i| i.members.len() == 1 && i.body != "The lock's battery is at 12 %."));
    }

    #[test]
    fn the_same_words_from_different_sources_are_not_folded_together() {
        let mut env = env();
        feed(&mut env, "plugin.lock.notifications", json!({"items": [note("b1", "Low.", SECS)]}));
        feed(&mut env, "plugins", json!({"plugins": [{"id": "lock", "name": "Front door", "state": "connected"}, {"id": "gate", "name": "Gate", "state": "connected"}]}));
        feed(&mut env, "plugin.gate.notifications", json!({"items": [note("g1", "Low.", SECS + 5.0)]}));
        let d = open(&env);
        assert_eq!(heads(&d, &env), vec!["TV:1", "AI access:2", "Gate:1", "Front door:1"]);
    }

    #[test]
    fn the_tvs_own_repeats_fold_too_and_clearing_the_entry_empties_every_one_from_the_log() {
        let env = Env::new(json!({}));
        let mut d = Drawer::new();
        d.life.show(0);
        let post = |d: &mut Drawer, title: &str, time: u64| d.log.post(notelog::Item { id: String::new(), source: "TV".into(), icon: "info".into(), title: title.into(), body: String::new(), time, actions: Vec::new() });
        for n in 0..3 {
            post(&mut d, "Backup finished", NOW + n * 1000);
        }
        post(&mut d, "Update ready", NOW + 500);
        d.open(&env, NOW + 10_000);
        let g = &d.groups(&env)[0];
        assert_eq!((g.items.len(), g.count()), (2, 4));
        assert_eq!(g.items[0].title, "Backup finished");
        assert_eq!(g.items[0].members.len(), 3);
        d.key(K::Ok, &env);
        assert_eq!(d.log.items().len(), 4, "OK on the header only opens it");
        d.key(K::Down, &env);
        d.key(K::Ok, &env);
        assert_eq!(d.log.items().len(), 1, "the three that said the same go with it");
        assert_eq!(d.log.items()[0].title, "Update ready");
    }

    #[test]
    fn what_is_newer_moves_its_group_to_the_top_and_inside_a_group_the_newest_is_first() {
        let env = env();
        let mut d = open(&env);
        let mut newer = audit();
        newer["entries"].as_array_mut().unwrap().push(json!({"assistant": {"id": "cody", "name": "Cody"}, "tool": "x", "message": "dim the lights", "outcome": "ok", "at": NOW + 1000}));
        d.answer(200, &newer);
        assert_eq!(heads(&d, &env), vec!["AI access:3", "TV:1", "Front door:2"]);
        assert_eq!(titles(&d, &env)[0], "Cody: dim the lights");
        let times: Vec<u64> = d.groups(&env)[0].items.iter().map(|i| i.time).collect();
        assert!(times.windows(2).all(|w| w[0] >= w[1]), "{times:?}");
    }

    #[test]
    fn a_plugins_button_is_a_post_to_its_action_route_and_an_item_without_buttons_is_cleared_by_ok() {
        let env = env();
        let mut d = open(&env);
        go_to(&mut d, &env, "Door unlocked");
        let out = d.key(K::Ok, &env);
        assert_eq!(out.effects, vec![Effect::Post { path: "/v1/plugins/lock/actions/lock".into(), body: json!({}) }]);
        go_to(&mut d, &env, "Battery low");
        let before = d.items(&env).len();
        d.key(K::Ok, &env);
        assert_eq!(d.items(&env).len(), before - 1);
        assert!(!titles(&d, &env).contains(&"Battery low".to_string()));
    }

    #[test]
    fn a_paused_assistants_denial_has_one_button_that_resumes_it() {
        let env = env();
        let mut d = open(&env);
        go_to(&mut d, &env, "Cody was denied");
        d.key(K::Right, &env);
        assert_eq!(d.button(), 0, "one button: Right stays on it");
        let Some(Row::Item(it)) = d.rows(&env).get(d.index() - 1).cloned() else { panic!("an item") };
        assert_eq!(it.actions[0].label, "Resume Cody");
        assert_eq!(it.body, "It asked to turn the TV off; its requests are paused until you resume them.");
        let out = d.key(K::Ok, &env);
        assert_eq!(out.effects, vec![Effect::Post { path: "/v1/ai/assistants/cody/resume".into(), body: json!({}) }]);
    }

    #[test]
    fn an_assistant_that_is_not_paused_has_nothing_to_resume() {
        let mut env = env();
        env.update(&Input::Helper { topic: "ai".into(), data: json!({"assistants": [{"id": "cody", "name": "Cody", "paused": false}]}) }, 1);
        let mut d = open(&env);
        go_to(&mut d, &env, "Cody was denied");
        let Some(Row::Item(it)) = d.rows(&env).get(d.index() - 1).cloned() else { panic!("an item") };
        assert!(it.actions.is_empty());
        assert_eq!(it.body, "turn the TV off");
    }

    #[test]
    fn clear_all_empties_the_tvs_log_and_hides_what_the_others_said_and_up_from_the_first_row_reaches_it() {
        let env = env();
        let mut d = open(&env);
        d.index = 1;
        d.key(K::Up, &env);
        assert_eq!(d.index(), 0, "Up from the first row is the Clear all button");
        d.key(K::Ok, &env);
        assert_eq!(d.items(&env).len(), 0);
        assert_eq!(d.groups(&env).len(), 0);
        assert!(d.log.items().is_empty(), "the TV's own log is empty too");
        assert_eq!(d.view(&env, NOW).count, 0);
        // something newer than the clearing shows again
        let mut newer = audit();
        newer["entries"].as_array_mut().unwrap().push(json!({"assistant": {"id": "cody", "name": "Cody"}, "tool": "x", "message": "dim the lights", "outcome": "ok", "at": NOW + 1000}));
        d.answer(200, &newer);
        assert_eq!(titles(&d, &env), vec!["Cody: dim the lights"]);
    }

    #[test]
    fn back_closes_and_the_keys_stop_at_the_ends_with_the_button_back_on_the_first() {
        let env = env();
        let mut d = open(&env);
        for _ in 0..20 {
            d.key(K::Down, &env);
        }
        assert_eq!(d.index(), 5);
        d.key(K::Right, &env);
        d.key(K::Up, &env);
        assert_eq!(d.button(), 0, "a move up or down puts the button back on the first");
        assert!(d.key(K::Back, &env).close);
    }

    #[test]
    fn the_tvs_own_buttons_are_told_to_whoever_posted_them() {
        let env = Env::new(json!({}));
        let mut d = Drawer::new();
        d.life.show(0);
        d.log.post(notelog::Item {
            id: String::new(),
            source: "TV".into(),
            icon: "download".into(),
            title: "Update ready".into(),
            body: String::new(),
            time: NOW,
            actions: vec![notelog::Button { id: "install".into(), label: "Install".into() }],
        });
        d.open(&env, NOW);
        let out = d.key(K::Ok, &env);
        assert_eq!(out.events, vec![("notelog.action".to_string(), json!({"id": "tv-1", "action": "install"}))]);
        assert!(out.effects.is_empty());
    }

    #[test]
    fn the_view_says_when_in_words_and_the_empty_drawer_has_nothing_to_focus() {
        let env = env();
        let d = open(&env);
        let v = d.view(&env, NOW);
        let EntryView::Item(first) = v.entries.iter().find(|e| matches!(e, EntryView::Item(_))).unwrap() else { panic!() };
        assert!(!first.when.is_empty());
        let empty = Env::new(json!({}));
        let mut d = Drawer::new();
        d.life.show(0);
        d.open(&empty, NOW);
        let out = d.key(K::Ok, &empty);
        assert!(out.effects.is_empty() && !out.close, "OK on nothing does nothing");
        let v = d.view(&empty, NOW);
        assert_eq!((v.count, v.rows), (0, 0));
    }

    #[test]
    fn a_click_on_an_item_clears_it_when_it_has_no_buttons_and_a_click_on_its_button_runs_it() {
        let env = env();
        let mut d = open(&env);
        go_to(&mut d, &env, "Battery low");
        let battery = d.index();
        d.click_item(battery, &env);
        assert!(!titles(&d, &env).contains(&"Battery low".to_string()));
        go_to(&mut d, &env, "Door unlocked");
        let unlocked = d.index();
        let out = d.click_button(unlocked, 0, &env);
        assert!(matches!(&out.effects[..], [Effect::Post { path, .. }] if path == "/v1/plugins/lock/actions/lock"));
    }

    #[test]
    fn a_click_on_a_header_opens_or_folds_its_group_and_a_click_on_its_clear_clears_it() {
        let env = env();
        let mut d = open(&env);
        go_to_head(&mut d, &env, "AI access");
        let head = d.index();
        d.click_head(head, &env);
        assert_eq!(titles(&d, &env).len(), 4);
        d.click_head(head, &env);
        assert_eq!(titles(&d, &env).len(), 3);
        d.click_head_clear(head, &env);
        assert_eq!(heads(&d, &env), vec!["TV:1", "Front door:2"]);
    }

    #[test]
    fn what_do_not_disturb_held_is_a_group_of_its_own_and_clearing_the_last_of_it_forgets_it_in_the_helper() {
        let mut env = env();
        feed(
            &mut env,
            crate::dnd::TOPIC,
            json!({"on": false, "missed": [
                {"kind": "camera", "source": "275", "title": "Person at Driveway", "body": "", "at": NOW - 60_000, "by": "dnd"},
                {"kind": "toast", "source": "ha", "title": "Washer", "body": "Done", "at": NOW - 120_000, "by": "dnd"}
            ]}),
        );
        let mut d = open(&env);
        let group = d.groups(&env).into_iter().find(|g| g.source == crate::dnd::MISSED_GROUP).expect("the group");
        assert_eq!((group.icon.as_str(), group.items[0].icon.as_str(), group.items[1].icon.as_str()), (crate::dnd::GLYPH, "camera", "bell"));
        go_to(&mut d, &env, "Washer");
        assert!(d.key(K::Ok, &env).effects.is_empty(), "one of two goes from the drawer only");
        go_to(&mut d, &env, "Person at Driveway");
        let out = d.key(K::Ok, &env);
        assert_eq!(out.effects, vec![Effect::Request { tag: tags::DND, method: "DELETE".into(), path: "/v1/dnd/missed".into(), body: None }]);
        assert!(d.groups(&env).iter().all(|g| g.source != crate::dnd::MISSED_GROUP));
    }
}
