//! The notification drawer (`docs/mocks/13-notifications`, `panels/Drawer.qml` of the QML shell): a panel down the right side that opens
//! from the bell on Home's top bar (and a long press of the gear key), with what has happened grouped by who said it: AI access (what an
//! assistant asked for and what was decided: the helper's audit), each plugin (the broker's notifications, with the plugin's own
//! buttons) and the TV (the [`NoteLog`]: an update ready, what LG's notification service said). One column the D-pad moves down; Right and
//! Left move between an item's buttons; OK runs the focused button (an item without buttons: OK clears it); Back closes. "Clear all"
//! empties the TV's own log and hides what the others said, until they say something new.
//!
//! Nothing in it is stored here: the audit is read each time the drawer opens, the broker's notifications come from the helper's
//! topics and the log is the [`NoteLog`], so a restart shows what the helper and the broker still have. Opening it is what marks
//! everything seen (the bell's badge).

use super::clock;
use super::env::{Env, tags};
use super::hub::{self, Note};
use super::life::Life;
use super::notelog::NoteLog;
use super::{K, Out};
use crate::kit::screen::Effect;
use serde_json::{Value, json};
use std::collections::BTreeSet;

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
}

/// A group header or an item with its number among the items (from 1: the number is the index the focus has).
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Head { source: String, icon: String, count: usize },
    Item { idx: usize, item: Item },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemView {
    pub idx: usize,
    pub icon: String,
    pub title: String,
    pub body: String,
    pub when: String,
    pub buttons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EntryView {
    Head { source: String, icon: String, count: usize },
    Item(ItemView),
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub up: bool,
    pub entries: Vec<EntryView>,
    pub count: usize,
    /// 0 is Clear all, then the items in order.
    pub index: usize,
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
        Self { life: Life::default(), log: NoteLog::new(), index: 1, button: 0, audit: Vec::new(), assistants: Vec::new(), hidden: BTreeSet::new(), hidden_before: 0.0 }
    }

    pub fn notes(env: &Env) -> Vec<Note> {
        hub::notes(&env.topics)
    }

    /// The entries the column draws: a header per group, then its items, each numbered among the items.
    pub fn entries(&self, env: &Env) -> Vec<Entry> {
        let mut groups: Vec<(String, String, Vec<Item>)> = Vec::new();
        let mut add = |source: &str, icon: &str, item: Item| match groups.iter_mut().find(|g| g.0 == source) {
            Some(g) => g.2.push(item),
            None => groups.push((source.to_string(), icon.to_string(), vec![item])),
        };
        for e in &self.audit {
            let at = e.get("at").and_then(Value::as_f64).unwrap_or(0.0);
            let key = format!("audit-{}", at as u64);
            if self.hidden.contains(&key) || at <= self.hidden_before {
                continue;
            }
            let (title, body, actions) = audit_text(e, &self.assistants);
            add("AI access", "bot", Item { id: key, icon: "sparkles".into(), title, body, time: at as u64, actions, own: false });
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
            add(&n.plugin, "plug", Item { id: key, icon: icon.into(), title: n.title.clone(), body: n.body.clone(), time: (n.received_at * 1000.0) as u64, actions, own: false });
        }
        for l in self.log.items() {
            let actions = l.actions.iter().map(|a| Action { label: a.label.clone(), run: Do::Own { id: l.id.clone(), action: a.id.clone() } }).collect();
            add(&l.source, "monitor", Item { id: l.id.clone(), icon: l.icon.clone(), title: l.title.clone(), body: l.body.clone(), time: l.time, actions, own: true });
        }
        let mut out = Vec::new();
        let mut n = 0;
        for (source, icon, items) in groups {
            out.push(Entry::Head { source, icon, count: items.len() });
            for item in items {
                n += 1;
                out.push(Entry::Item { idx: n, item });
            }
        }
        out
    }

    pub fn items(&self, env: &Env) -> Vec<Item> {
        self.entries(env).into_iter().filter_map(|e| if let Entry::Item { item, .. } = e { Some(item) } else { None }).collect()
    }

    // ---- opening

    /// The drawer is wanted: the focus on the first item, everything marked seen, the assistants as they are now, the audit asked for.
    pub fn open(&mut self, env: &Env, now: u64) -> Out {
        self.index = 1;
        self.button = 0;
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

    fn clear_all(&mut self, env: &Env) {
        let items = self.items(env);
        self.log.clear();
        for it in &items {
            self.hidden.insert(it.id.clone());
        }
        self.hidden_before = self.audit.iter().filter_map(|a| a.get("at").and_then(Value::as_f64)).fold(self.hidden_before.max(0.0), f64::max);
        self.index = 0;
    }

    fn dismiss(&mut self, env: &Env, it: &Item) {
        if it.own {
            self.log.dismiss(&it.id);
        } else {
            self.hidden.insert(it.id.clone());
        }
        self.index = self.index.min(self.items(env).len());
        self.button = 0;
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
        let items = self.items(env);
        let it = if self.index > 0 { items.get(self.index - 1).cloned() } else { None };
        match k {
            K::Back => return Out::close(),
            K::Up => {
                self.index = self.index.saturating_sub(1);
                self.button = 0;
            }
            K::Down => {
                self.index = (self.index + 1).min(items.len());
                self.button = 0;
            }
            K::Left => {
                if it.is_some() {
                    self.button = self.button.saturating_sub(1);
                }
            }
            K::Right => {
                if let Some(it) = &it {
                    self.button = (self.button + 1).min(it.actions.len().saturating_sub(1));
                }
            }
            K::Ok => {
                if self.index == 0 {
                    self.clear_all(env);
                } else if let Some(it) = it {
                    if it.actions.is_empty() {
                        self.dismiss(env, &it);
                    } else {
                        let a = it.actions.get(self.button).unwrap_or(&it.actions[0]).clone();
                        return self.run(&a);
                    }
                }
            }
            _ => {}
        }
        Out::default()
    }

    /// A click on "Clear all".
    pub fn click_clear(&mut self, env: &Env) {
        self.clear_all(env);
    }

    /// A click on item `idx` (from 1): an item without buttons is cleared by it, as OK clears it.
    pub fn click_item(&mut self, idx: usize, env: &Env) {
        self.index = idx;
        self.button = 0;
        if let Some(it) = self.items(env).get(idx.wrapping_sub(1)).cloned()
            && it.actions.is_empty()
        {
            self.dismiss(env, &it);
        }
    }

    /// A click on button `b` of item `idx`.
    pub fn click_button(&mut self, idx: usize, b: usize, env: &Env) -> Out {
        self.index = idx;
        self.button = b;
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
        let count = entries.iter().filter(|e| matches!(e, Entry::Item { .. })).count();
        View {
            up: self.life.presented(),
            entries: entries
                .into_iter()
                .map(|e| match e {
                    Entry::Head { source, icon, count } => EntryView::Head { source, icon, count },
                    Entry::Item { idx, item } => EntryView::Item(ItemView { idx, icon: item.icon, title: item.title, body: item.body, when: clock::when(item.time, now), buttons: item.actions.into_iter().map(|a| a.label).collect() }),
                })
                .collect(),
            count,
            index: self.index.min(count),
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

    pub(crate) fn env() -> Env {
        let mut env = Env::new(json!({}));
        let feed = |env: &mut Env, topic: &str, data: Value| {
            env.update(&Input::Helper { topic: topic.into(), data }, 0);
        };
        feed(&mut env, "ai", json!({"listener": {"enabled": true}, "assistants": [{"id": "cody", "name": "Cody", "paused": true}]}));
        feed(&mut env, "plugins", json!({"plugins": [{"id": "lock", "name": "Front door", "state": "connected"}], "firstParty": [{"id": "weather", "name": "Weather", "state": "connected"}]}));
        let secs = (NOW / 1000 - 600) as f64;
        feed(
            &mut env,
            "plugin.lock.notifications",
            json!({"items": [
                {"id": "unlocked", "channel": "events", "revision": 1, "receivedAt": secs, "post": {"title": "Door unlocked", "body": "Someone opened it.", "actions": [{"id": "lock", "label": "Lock it", "action": "lock"}]}},
                {"id": "battery", "channel": "events", "revision": 1, "receivedAt": secs, "post": {"title": "Battery low", "body": "The lock's battery is at 12 %.", "actions": []}}
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
        d.log.from_toast(&json!({"message": "TV update ready\nwebOS 10.2.3"}), &[], NOW - 100_000);
        d.open(env, NOW);
        d.answer(200, &audit());
        d
    }

    fn titles(d: &Drawer, env: &Env) -> Vec<String> {
        d.items(env).into_iter().map(|i| i.title).collect()
    }

    fn go_to(d: &mut Drawer, env: &Env, title: &str) {
        d.index = 1;
        for _ in 0..30 {
            if d.items(env)[d.index - 1].title == title {
                return;
            }
            d.key(K::Down, env);
        }
        panic!("no item titled {title}");
    }

    #[test]
    fn opening_asks_for_the_audit_marks_everything_seen_and_groups_by_who_said_it_newest_first() {
        let env = env();
        let mut d = Drawer::new();
        d.life.show(0);
        d.log.from_toast(&json!({"message": "TV update ready"}), &[], NOW - 100_000);
        assert_eq!(d.log.unread(&Drawer::notes(&env)), 3, "the TV's update and the plugin's two notes are unread before the drawer opens");
        let out = d.open(&env, NOW);
        assert_eq!(out.effects, vec![Effect::Fetch { tag: tags::AUDIT, path: "/v1/ai/audit?limit=12".into() }]);
        assert_eq!(d.log.unread(&Drawer::notes(&env)), 0, "opening is what the bell's count is cleared by");
        d.answer(200, &audit());
        let heads: Vec<String> = d.entries(&env).into_iter().filter_map(|e| if let Entry::Head { source, count, .. } = e { Some(format!("{source}:{count}")) } else { None }).collect();
        assert_eq!(heads, vec!["AI access:2", "Front door:2", "TV:1"]);
        assert_eq!(d.index(), 1);
        assert_eq!(titles(&d, &env)[0], "Cody was denied", "the audit is newest first");
        assert_eq!(d.view(&env, NOW).count, 5);
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
        let it = d.items(&env)[d.index() - 1].clone();
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
        let it = d.items(&env)[d.index() - 1].clone();
        assert!(it.actions.is_empty());
        assert_eq!(it.body, "turn the TV off");
    }

    #[test]
    fn clear_all_empties_the_tvs_log_and_hides_what_the_others_said_and_up_from_the_first_item_reaches_it() {
        let env = env();
        let mut d = open(&env);
        d.index = 1;
        d.key(K::Up, &env);
        assert_eq!(d.index(), 0, "Up from the first item is the Clear all button");
        d.key(K::Ok, &env);
        assert_eq!(d.items(&env).len(), 0);
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
        assert_eq!(d.view(&empty, NOW).count, 0);
    }

    #[test]
    fn a_click_on_an_item_clears_it_when_it_has_no_buttons_and_a_click_on_its_button_runs_it() {
        let env = env();
        let mut d = open(&env);
        let battery = d.items(&env).iter().position(|i| i.title == "Battery low").unwrap() + 1;
        d.click_item(battery, &env);
        assert!(!titles(&d, &env).contains(&"Battery low".to_string()));
        let unlocked = d.items(&env).iter().position(|i| i.title == "Door unlocked").unwrap() + 1;
        let out = d.click_button(unlocked, 0, &env);
        assert!(matches!(&out.effects[..], [Effect::Post { path, .. }] if path == "/v1/plugins/lock/actions/lock"));
    }
}
