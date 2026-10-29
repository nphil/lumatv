//! The overlay system's queue (`Overlays.js` and `panels/Overlays.qml` of the QML shell, `docs/research/overlays-result.md`, the mocks
//! `docs/mocks/overlays`): which of the things that want the person's eye now are on the screen, in what order, for how long. A doorbell, an
//! alarm or a timer that finished, a Home Assistant alert, a score: the plugin broker's records (the notifications and the cards at the
//! `ringing` priority), and the camera pop-up, which is the doorbell's own card: this queue does not draw it, it makes room for it.
//!
//! The rules, one test each (the QML's `tests/overlays.test.js` as it was):
//! - at most `maxVisible` cards (2), the most important class first, and only cards of the most important class that is waiting: while a
//!   doorbell or an alarm is up an alert or a score waits (and goes when it has gone, if it is still in date); a score never shows beside a score;
//! - no card ends below `limit` (810): a second card that would not fit waits; the first always shows;
//! - nothing shows while the host is busy (a PIN, an alert, the keyboard, a panel, the saver): the cards wait and come back if still in date;
//! - a card has a time on the screen (its class's `dwellMs`, counted while it is shown), a record's own end and a time it may wait; a card the
//!   person dismissed is not shown again unless the record changed;
//! - the keys are the person's own until a card has been up `armMs`: a press made as it arrives was meant for the app.
//!
//! Numbers are the theme's (`overlays` in theme.json); [`Config`] is what a theme that says nothing gets, and a value of the wrong kind is not
//! taken. The queue is pure: time is an argument.

use super::plugins::{Card, Note};
use serde_json::{Value, json};
use std::collections::HashMap;

pub use lumatv_saver::rules::{CARD_CLASSES as CLASSES, card_class as class_of};
/// Cards waiting or shown, all classes.
pub const MAX_QUEUE: usize = 24;

#[derive(Clone, Debug, PartialEq)]
pub struct ClassCfg {
    pub priority: u32,
    pub dwell_ms: f64,
    pub wait_ms: f64,
    pub cooldown_ms: f64,
    pub icon: String,
    pub tone: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub enabled: bool,
    pub max_visible: usize,
    pub top: f64,
    pub right: f64,
    pub width: f64,
    pub gap: f64,
    pub limit: f64,
    pub arm_ms: f64,
    pub merge_ms: f64,
    classes: [ClassCfg; 5],
}

fn num(v: Option<&Value>, d: f64, lo: f64, hi: f64) -> f64 {
    v.and_then(Value::as_f64).filter(|n| n.is_finite() && *n >= lo && *n <= hi).unwrap_or(d)
}

fn default_class(name: &str) -> ClassCfg {
    let (priority, dwell, wait, cooldown, icon, tone) = match name {
        "doorbell" => (0, 15_000.0, 20_000.0, 0.0, "bell-ring", "accent"),
        "alarm" => (0, 60_000.0, 120_000.0, 0.0, "alarm-clock", "warning"),
        "timer" => (0, 30_000.0, 60_000.0, 0.0, "timer", "accent"),
        "alert" => (1, 10_000.0, 120_000.0, 0.0, "circle-alert", "accent"),
        _ => (2, 8_000.0, 30_000.0, 30_000.0, "trophy", "accent"),
    };
    ClassCfg { priority, dwell_ms: dwell, wait_ms: wait, cooldown_ms: cooldown, icon: icon.into(), tone: tone.into() }
}

impl Config {
    /// The settings in force: the theme's `overlays` over the defaults, a number that is not one dropped.
    pub fn from_theme(t: &Value) -> Self {
        let classes = CLASSES.map(|name| {
            let d = default_class(name);
            let o = t.get("classes").and_then(|c| c.get(name));
            let get = |k: &str| o.and_then(|o| o.get(k));
            ClassCfg {
                priority: num(get("priority"), f64::from(d.priority), 0.0, 9.0).round() as u32,
                dwell_ms: num(get("dwellMs"), d.dwell_ms, 500.0, 600_000.0),
                wait_ms: num(get("waitMs"), d.wait_ms, 0.0, 3_600_000.0),
                cooldown_ms: num(get("cooldownMs"), d.cooldown_ms, 0.0, 3_600_000.0),
                icon: get("icon").and_then(Value::as_str).filter(|s| !s.is_empty()).map_or(d.icon.clone(), str::to_string),
                tone: get("tone").and_then(Value::as_str).filter(|t| matches!(*t, "warning" | "danger" | "accent")).map_or(d.tone.clone(), str::to_string),
            }
        });
        Config {
            enabled: t.get("enabled").and_then(Value::as_bool) == Some(true) || t.get("rust").and_then(Value::as_bool) == Some(true),
            max_visible: num(t.get("maxVisible"), 2.0, 1.0, 3.0).round() as usize,
            top: num(t.get("top"), 134.0, 0.0, 540.0),
            right: num(t.get("right"), 64.0, 0.0, 480.0),
            width: num(t.get("width"), 560.0, 320.0, 900.0),
            gap: num(t.get("gap"), 16.0, 0.0, 64.0),
            limit: num(t.get("limit"), 810.0, 360.0, 1080.0),
            arm_ms: num(t.get("armMs"), 600.0, 0.0, 5000.0),
            merge_ms: num(t.get("mergeMs"), 20_000.0, 0.0, 120_000.0),
            classes,
        }
    }

    pub fn class(&self, name: &str) -> &ClassCfg {
        &self.classes[CLASSES.iter().position(|c| *c == name).unwrap_or(3)]
    }
}

/// A side of a score.
#[derive(Clone, Debug, PartialEq)]
pub struct Side {
    pub name: String,
    pub abbr: String,
    pub score: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Score {
    pub league: String,
    pub status: String,
    pub live: bool,
    pub home: Side,
    pub away: Side,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub key: String,
    /// `note`, `card` or `door` (made by hand: no record of the hub stands behind it).
    pub src: &'static str,
    pub plugin_id: String,
    pub id: String,
    pub revision: u64,
    pub cls: &'static str,
    pub priority: u32,
    pub icon: String,
    pub icon_name: String,
    pub tone: String,
    pub title: String,
    pub body: String,
    pub source: String,
    pub actions: Vec<Value>,
    pub score: Option<Score>,
    pub arrived_at: u64,
    /// Milliseconds, 0 for none.
    pub expires_at: u64,
    pub dwell_ms: u64,
    pub wait_ms: u64,
    pub shown_at: u64,
    pub shown_ms: u64,
    pub arm_at: u64,
    pub born_at: u64,
    pub hidden_at: u64,
    pub visible: bool,
    seq_no: u64,
}

fn tone_of(level: &str, role: &str, default: &str) -> String {
    match (level, role) {
        ("critical", _) | (_, "danger") => "danger".into(),
        ("warning", _) | (_, "warning") => "warning".into(),
        _ => default.to_string(),
    }
}

fn secs(v: f64) -> u64 {
    if v > 0.0 { (v * 1000.0) as u64 } else { 0 }
}

/// The structured score of the proposal (`post.score`): two sides with a name and a score, a competition, a clock. Anything else is the title.
fn clean_score(s: &Value) -> Option<Score> {
    if !s.is_object() {
        return None;
    }
    let side = |x: &Value| -> Option<Side> {
        let score = match x.get("score") {
            Some(Value::Number(n)) => n.to_string(),
            Some(Value::String(s)) => s.clone(),
            _ => return None,
        };
        let text = |k: &str| x.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        Some(Side { name: if text("name").is_empty() { text("abbr") } else { text("name") }, abbr: text("abbr"), score })
    };
    let (home, away) = (side(s.get("home")?)?, side(s.get("away")?)?);
    let text = |k: &str| s.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    Some(Score { league: text("league"), status: text("status"), live: s.get("live").and_then(Value::as_bool) == Some(true), home, away })
}

fn blank(key: String, src: &'static str, cls: &'static str, cfg: &Config) -> Entry {
    let spec = cfg.class(cls);
    Entry {
        key,
        src,
        plugin_id: String::new(),
        id: String::new(),
        revision: 0,
        cls,
        priority: spec.priority,
        icon: spec.icon.clone(),
        icon_name: String::new(),
        tone: spec.tone.clone(),
        title: String::new(),
        body: String::new(),
        source: String::new(),
        actions: Vec::new(),
        score: None,
        arrived_at: 0,
        expires_at: 0,
        dwell_ms: spec.dwell_ms as u64,
        wait_ms: spec.wait_ms as u64,
        shown_at: 0,
        shown_ms: 0,
        arm_at: 0,
        born_at: 0,
        hidden_at: 0,
        visible: false,
        seq_no: 0,
    }
}

/// A notification record as an entry; `None` when it is not one for the screen (a record of `low` importance is the drawer's, and so is one
/// with nothing to say).
pub fn from_note(n: &Note, cfg: &Config, now: u64) -> Option<Entry> {
    if n.importance == "low" || (n.title.is_empty() && n.body.is_empty()) {
        return None;
    }
    let cls = class_of(&n.kind, &n.channel, &n.channel_icon);
    let mut e = blank(format!("n:{}/{}", n.plugin_id, n.id), "note", cls, cfg);
    let secret = n.privacy == "secret";
    e.plugin_id = n.plugin_id.clone();
    e.id = n.id.clone();
    e.revision = n.revision;
    e.icon_name = n.channel_icon.clone();
    e.tone = tone_of(&n.level, "", &e.tone);
    e.title = if secret { if n.channel_title.is_empty() { n.title.clone() } else { n.channel_title.clone() } } else { n.title.clone() };
    e.body = if secret { String::new() } else { n.body.clone() };
    e.source = if n.channel_title.is_empty() { n.plugin.clone() } else { n.channel_title.clone() };
    e.actions = n.actions.iter().take(2).cloned().collect();
    e.score = if cls == "score" { clean_score(&n.score) } else { None };
    e.arrived_at = now;
    e.expires_at = secs(n.expires_at);
    if n.level == "critical" {
        e.dwell_ms *= 2;
    }
    Some(e)
}

/// A card at the `ringing` priority: something is ringing now. It is up while the card exists; the plugin takes it away.
pub fn from_card(c: &Card, cfg: &Config, now: u64) -> Option<Entry> {
    if c.priority != "ringing" {
        return None;
    }
    let k = &c.content;
    let kind = c.content.get("kind").and_then(Value::as_str).unwrap_or("");
    let mut cls = class_of(kind, &c.id, k.get("icon").and_then(Value::as_str).unwrap_or(""));
    if cls == "alert" {
        cls = "alarm"; // ringing says it is not a mere message
    }
    let text = |key: &str| k.get(key).and_then(Value::as_str).unwrap_or("");
    let detail = [text("detail"), text("subtitle")]
        .into_iter()
        .find(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| k.get("facts").and_then(Value::as_array).and_then(|f| f.first()).and_then(|f| f.get("value")).and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    let mut e = blank(format!("c:{}/{}", c.plugin_id, c.id), "card", cls, cfg);
    e.plugin_id = c.plugin_id.clone();
    e.id = c.id.clone();
    e.revision = c.revision;
    e.icon_name = text("icon").to_string();
    e.tone = tone_of("", text("role"), &e.tone);
    e.title = if text("title").is_empty() { c.plugin.clone() } else { text("title").to_string() };
    e.body = detail;
    e.source = c.plugin.clone();
    e.actions = c.actions.iter().take(2).cloned().collect();
    e.arrived_at = now;
    e.expires_at = secs(c.expires_at);
    Some(e)
}

/// What `offer` did with an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer {
    New,
    Updated,
    Dismissed,
    Cooldown,
    Merged,
}

#[derive(Clone, Debug, Default)]
pub struct Queue {
    list: Vec<Entry>,
    dismissed: HashMap<String, u64>,
    last_shown: HashMap<String, u64>,
    camera: Option<(String, u64)>,
    seq: u64,
}

/// What the screen is in when the queue is asked what shows.
pub struct Ctx<'a> {
    /// The host is busy: nothing shows.
    pub dnd: bool,
    /// The camera pop-up is up: it takes a slot and the top of the column.
    pub camera: bool,
    /// Where the first card of the queue starts: the camera's bottom and the gap, else `cfg.top`.
    pub top: f64,
    pub height_of: &'a mut dyn FnMut(&Entry) -> f64,
}

#[derive(Debug, Default)]
pub struct Selection {
    pub visible: Vec<String>,
    /// When to look again (ms), 0 for never.
    pub wake_at: u64,
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn list(&self) -> &[Entry] {
        &self.list
    }

    pub fn find(&self, key: &str) -> Option<&Entry> {
        self.list.iter().find(|e| e.key == key)
    }

    fn find_mut(&mut self, key: &str) -> Option<&mut Entry> {
        self.list.iter_mut().find(|e| e.key == key)
    }

    fn remove(&mut self, key: &str) -> Option<Entry> {
        let i = self.list.iter().position(|e| e.key == key)?;
        Some(self.list.remove(i))
    }

    /// Takes an entry in: queued, changed in place, or why it was not taken.
    pub fn offer(&mut self, mut e: Entry, cfg: &Config, now: u64) -> Offer {
        let mine = self.find(&e.key).is_some();
        let seen = self.dismissed.get(&e.key).copied();
        if let Some(s) = seen
            && e.revision <= s
            && !mine
        {
            return Offer::Dismissed;
        }
        if e.cls == "doorbell"
            && let Some((kind, at)) = &self.camera
            && kind == "doorbell"
            && now.saturating_sub(*at) < cfg.merge_ms as u64
        {
            // it is the camera's card now; the record staying in the drawer is not a second ring
            self.dismissed.insert(e.key.clone(), e.revision);
            return Offer::Merged;
        }
        if mine {
            let m = self.find_mut(&e.key).expect("found above");
            let changed = m.revision != e.revision || m.title != e.title || m.body != e.body;
            m.revision = e.revision;
            m.title = e.title;
            m.body = e.body;
            m.source = e.source;
            m.actions = e.actions;
            m.tone = e.tone;
            m.score = e.score;
            m.expires_at = e.expires_at;
            m.icon_name = e.icon_name;
            // a score that moved is news again: it has its time on the screen afresh, though never more than three times over
            if changed && m.visible && m.cls == "score" && now.saturating_sub(m.born_at) < m.dwell_ms * 3 {
                m.shown_ms = 0;
                m.shown_at = now;
            }
            return Offer::Updated;
        }
        let spec = cfg.class(e.cls);
        if spec.cooldown_ms > 0.0
            && let Some(last) = self.last_shown.get(&e.key)
            && (now.saturating_sub(*last) as f64) < spec.cooldown_ms
        {
            return Offer::Cooldown;
        }
        if seen.is_some() {
            self.dismissed.remove(&e.key);
        }
        e.arrived_at = now;
        self.seq += 1;
        e.seq_no = self.seq;
        let key = e.key.clone();
        self.list.push(e);
        // a plugin that floods the queue does not grow it for ever: past the cap the oldest of the least important class that is not on
        // the screen goes
        while self.list.len() > MAX_QUEUE {
            let mut drop: Option<usize> = None;
            for (i, c) in self.list.iter().enumerate() {
                if c.visible || c.key == key {
                    continue;
                }
                drop = Some(match drop {
                    None => i,
                    Some(d) => {
                        let o = &self.list[d];
                        if c.priority > o.priority || (c.priority == o.priority && c.seq_no < o.seq_no) { i } else { d }
                    }
                });
            }
            match drop {
                Some(i) => {
                    self.list.remove(i);
                }
                None => break,
            }
        }
        Offer::New
    }

    /// A record the plugin ended (or that expired): its card goes, and a later record of the same id is news.
    pub fn withdraw(&mut self, key: &str) -> bool {
        let had = self.remove(key).is_some();
        self.dismissed.remove(key);
        had
    }

    /// The person dismissed it (Back, or OK after its action ran): gone, and not back for this revision.
    pub fn dismiss(&mut self, key: &str, now: u64) -> bool {
        let Some(e) = self.remove(key) else { return false };
        self.dismissed.insert(e.key.clone(), e.revision);
        if e.visible {
            self.last_shown.insert(e.key, now);
        }
        true
    }

    /// The camera's pop-up showed a card of this kind: a doorbell record of the same ring is not a second card, and one already queued or shown is
    /// taken back into the camera's card. Returns how many entries went.
    pub fn camera_shown(&mut self, kind: &str, cfg: &Config, now: u64) -> usize {
        self.camera = Some((kind.to_string(), now));
        if kind != "doorbell" {
            return 0;
        }
        let before = self.list.len();
        self.list.retain(|e| !(e.cls == "doorbell" && e.src == "note" && now.saturating_sub(e.arrived_at) < cfg.merge_ms as u64));
        before - self.list.len()
    }

    fn shown_for(e: &Entry, now: u64) -> u64 {
        e.shown_ms + if e.visible { now.saturating_sub(e.shown_at) } else { 0 }
    }

    fn hide(e: &mut Entry, now: u64) {
        if !e.visible {
            return;
        }
        e.shown_ms += now.saturating_sub(e.shown_at);
        e.shown_at = 0;
        e.hidden_at = now;
        e.visible = false;
    }

    /// What is on the screen now, top first, and when to look again.
    pub fn select(&mut self, ctx: &mut Ctx, cfg: &Config, now: u64) -> Selection {
        self.last_shown.retain(|_, t| now.saturating_sub(*t) <= 3_600_000);
        // the end of what a card may be
        let mut i = self.list.len();
        while i > 0 {
            i -= 1;
            let e = &self.list[i];
            let over = e.expires_at > 0 && now >= e.expires_at;
            let used = Self::shown_for(e, now) >= e.dwell_ms;
            let stale =
                !e.visible && e.src != "card" && e.wait_ms > 0 && now.saturating_sub(if e.hidden_at > 0 { e.hidden_at } else { e.arrived_at }) >= e.wait_ms;
            if over || used || stale {
                let gone = self.list.remove(i);
                if gone.visible {
                    self.last_shown.insert(gone.key.clone(), now);
                }
                // a card that had its time is done with: this revision is not news again
                if used {
                    self.dismissed.insert(gone.key, gone.revision);
                }
            }
        }
        let mut chosen: Vec<String> = Vec::new();
        if !ctx.dnd {
            let mut order: Vec<usize> = (0..self.list.len()).collect();
            order.sort_by(|&a, &b| {
                let (x, y) = (&self.list[a], &self.list[b]);
                (x.priority, !x.visible, x.seq_no).cmp(&(y.priority, !y.visible, y.seq_no))
            });
            let best = if ctx.camera { 0 } else { order.first().map_or(0, |&i| self.list[i].priority) };
            let slots = cfg.max_visible.saturating_sub(usize::from(ctx.camera));
            let mut y = if ctx.camera { ctx.top } else { cfg.top };
            let mut have_score = false;
            for &i in &order {
                if chosen.len() >= slots {
                    break;
                }
                let e = &self.list[i];
                if e.priority > best {
                    break;
                }
                if e.cls == "score" && have_score {
                    continue;
                }
                let h = (ctx.height_of)(e);
                // the first card of an empty screen always shows (a doorbell is not left unseen for its size); any other must end above the limit
                if (!chosen.is_empty() || ctx.camera) && y + h > cfg.limit {
                    continue;
                }
                chosen.push(e.key.clone());
                y += h + cfg.gap;
                if e.cls == "score" {
                    have_score = true;
                }
            }
        }
        for e in &mut self.list {
            if chosen.contains(&e.key) {
                if !e.visible {
                    e.visible = true;
                    e.shown_at = now;
                    e.arm_at = now;
                    if e.born_at == 0 {
                        e.born_at = now;
                    }
                    self.last_shown.insert(e.key.clone(), now);
                }
            } else {
                Self::hide(e, now);
            }
        }
        let mut wake = 0u64;
        let mut soon = |t: u64| {
            if t > now && (wake == 0 || t < wake) {
                wake = t;
            }
        };
        for e in &self.list {
            if e.expires_at > 0 {
                soon(e.expires_at);
            }
            if e.visible {
                soon(e.shown_at + e.dwell_ms.saturating_sub(e.shown_ms));
                soon(e.arm_at + cfg.arm_ms as u64);
            } else if e.src != "card" && e.wait_ms > 0 {
                soon((if e.hidden_at > 0 { e.hidden_at } else { e.arrived_at }) + e.wait_ms);
            }
        }
        // the order on the screen is the order chosen
        let mut visible = chosen;
        visible.retain(|k| self.list.iter().any(|e| &e.key == k));
        Selection { visible, wake_at: wake }
    }

    /// Whether the keys are the top card's: it has been up for `armMs` (a press made as it arrived was the app's).
    pub fn armed(&self, visible: &[String], cfg: &Config, now: u64) -> bool {
        visible.first().and_then(|k| self.find(k)).is_some_and(|e| e.visible && now.saturating_sub(e.arm_at) as f64 >= cfg.arm_ms)
    }
}

/// What OK does on a card, in one word, and the button it runs: the record's first action, else the drawer (when it can be opened here).
#[derive(Clone, Debug, PartialEq)]
pub struct Ok {
    pub label: String,
    pub action: Option<Value>,
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() > max { text.chars().take(max - 1).collect::<String>() + "\u{2026}" } else { text.to_string() }
}

pub fn ok_of(e: &Entry, drawer_ready: bool) -> Ok {
    if let Some(a) = e.actions.first() {
        let label = a.get("label").and_then(Value::as_str).filter(|l| !l.is_empty()).unwrap_or("OK");
        return Ok { label: clip(label, 24), action: Some(a.clone()) };
    }
    Ok { label: if drawer_ready { "Open".into() } else { "Dismiss".into() }, action: None }
}

/// The hint's two keys: OK where it does something Back does not.
pub fn keys_of(e: &Entry, drawer_ready: bool) -> Vec<(&'static str, String)> {
    let ok = ok_of(e, drawer_ready);
    if ok.label == "Dismiss" { vec![("OK", "Dismiss".into())] } else { vec![("OK", ok.label), ("Back", "Dismiss".into())] }
}

/// The wire form of an entry for the gate's state.
pub fn entry_json(e: &Entry) -> Value {
    json!({"key": e.key, "class": e.cls, "title": e.title, "visible": e.visible})
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_000_000;

    fn cfg() -> Config {
        Config::from_theme(&json!({"enabled": true}))
    }

    fn note(id: &str) -> Note {
        Note {
            plugin_id: "ha".into(),
            plugin: "Home Assistant".into(),
            id: id.into(),
            channel: "ha".into(),
            channel_title: "Home Assistant".into(),
            channel_icon: "base.icons/home".into(),
            revision: 1,
            received_at: 1.0,
            expires_at: 0.0,
            title: "Low battery".into(),
            body: "Door sensor is at 5 %".into(),
            actions: Vec::new(),
            kind: String::new(),
            level: String::new(),
            privacy: String::new(),
            score: Value::Null,
            importance: String::new(),
            channel_kind: String::new(),
        }
    }

    struct Rig {
        q: Queue,
        t: u64,
        cfg: Config,
        h: f64,
    }

    impl Rig {
        fn new() -> Self {
            Rig { q: Queue::new(), t: T0, cfg: cfg(), h: 150.0 }
        }
        fn take(&mut self, n: &Note) -> Offer {
            let e = from_note(n, &self.cfg, self.t).expect("an entry");
            self.q.offer(e, &self.cfg, self.t)
        }
        fn ctx(&mut self, dnd: bool, camera: bool, top: f64) -> Selection {
            let h = self.h;
            let mut f = move |_: &Entry| h;
            let mut ctx = Ctx { dnd, camera, top, height_of: &mut f };
            self.q.select(&mut ctx, &self.cfg, self.t)
        }
        fn now(&mut self) -> Selection {
            self.ctx(false, false, 134.0)
        }
        fn ids(&mut self) -> Vec<String> {
            self.now().visible.iter().map(|k| k.split('/').next_back().unwrap_or("").to_string()).collect()
        }
    }

    fn with<R>(mut n: Note, f: impl FnOnce(&mut Note) -> R) -> Note {
        f(&mut n);
        n
    }

    #[test]
    fn a_theme_that_says_nothing_gets_the_defaults_and_a_number_of_the_wrong_kind_is_not_taken() {
        let c = Config::from_theme(&json!({}));
        assert!(!c.enabled);
        assert_eq!((c.max_visible, c.limit), (2, 810.0));
        assert_eq!((c.class("doorbell").priority, c.class("score").priority), (0, 2));
        let c2 = Config::from_theme(
            &json!({"enabled": true, "maxVisible": "many", "limit": 9999, "gap": 8, "classes": {"score": {"dwellMs": 5000, "tone": "pink"}, "alert": {"priority": 1.4}}}),
        );
        assert!(c2.enabled);
        assert_eq!((c2.max_visible, c2.limit, c2.gap), (2, 810.0, 8.0));
        assert_eq!((c2.class("score").dwell_ms, c2.class("score").tone.as_str()), (5000.0, "accent"));
        assert_eq!(c2.class("alert").priority, 1);
        assert!(!Config::from_theme(&json!({"enabled": "yes"})).enabled);
        assert!(Config::from_theme(&json!({"rust": true})).enabled, "the Rust shell's own switch while the QML cards are its fallback");
    }

    #[test]
    fn the_defaults_are_the_shipped_themes_numbers_and_the_shipped_theme_has_the_switch_off() {
        let theme: Value = serde_json::from_str(include_str!("../../../../payload/theme/theme.json")).unwrap();
        assert_eq!(theme["overlays"]["enabled"], false);
        assert_eq!(Config::from_theme(&theme["overlays"]), Config::from_theme(&json!({})));
    }

    #[test]
    fn a_records_class_is_told_from_its_channel_and_icon_an_explicit_kind_over_both_an_alert_otherwise() {
        assert_eq!(class_of("", "doorbell", "base.icons/doorbell"), "doorbell");
        assert_eq!(class_of("", "ha", "base.icons/home"), "alert");
        assert_eq!(class_of("", "x", "base.icons/timer"), "timer");
        assert_eq!(class_of("", "alarms", ""), "alarm");
        assert_eq!(class_of("", "scores", ""), "score");
        assert_eq!(class_of("", "nba", "base.icons/trophy"), "score");
        assert_eq!(class_of("alert", "doorbell", ""), "alert");
        assert_eq!(class_of("timer", "ha", ""), "timer");
        assert_eq!(class_of("nonsense", "ha", ""), "alert");
    }

    #[test]
    fn a_notification_becomes_an_entry_low_importance_and_an_empty_one_are_the_drawers_a_secret_one_shows_no_words_of_its_own() {
        let c = cfg();
        let e = from_note(&note("a"), &c, T0).unwrap();
        assert_eq!((e.key.as_str(), e.cls, e.priority, e.title.as_str(), e.source.as_str()), ("n:ha/a", "alert", 1, "Low battery", "Home Assistant"));
        assert!(from_note(&with(note("b"), |n| n.importance = "low".into()), &c, T0).is_none());
        assert!(from_note(&with(note("c"), |n| (n.title.clear(), n.body.clear())), &c, T0).is_none());
        let s = from_note(&with(note("d"), |n| (n.privacy = "secret".into(), n.title = "Code 1234".into(), n.body = "do not read".into())), &c, T0).unwrap();
        assert_eq!((s.title.as_str(), s.body.as_str()), ("Home Assistant", ""));
        let w = from_note(&with(note("w"), |n| n.level = "warning".into()), &c, T0).unwrap();
        let k = from_note(&with(note("k"), |n| n.level = "critical".into()), &c, T0).unwrap();
        assert_eq!((w.tone.as_str(), k.tone.as_str()), ("warning", "danger"));
        assert_eq!(k.dwell_ms, 2 * e.dwell_ms, "a critical one stays twice as long");
        let timed = from_note(&with(note("z"), |n| n.expires_at = 1_000_030.0), &c, T0).unwrap();
        assert_eq!(timed.expires_at, 1_000_030_000, "the record's end is in seconds, an entry's in milliseconds");
    }

    #[test]
    fn a_score_reads_its_two_sides_or_is_the_title_and_the_body_of_a_card() {
        let c = cfg();
        let sc = json!({"league": "NBA", "status": "Q4 2:31", "live": true, "home": {"name": "Celtics", "abbr": "BOS", "score": 99}, "away": {"name": "Lakers", "abbr": "LAL", "score": 102}});
        let e =
            from_note(&with(note("g"), |n| (n.channel = "scores".into(), n.channel_icon = "base.icons/trophy".into(), n.score = sc.clone())), &c, T0).unwrap();
        let s = e.score.unwrap();
        assert_eq!((e.cls, s.home.name.as_str(), s.away.score.as_str(), s.live), ("score", "Celtics", "102", true));
        let half = json!({"home": {"name": "A"}, "away": {"name": "B", "score": 1}});
        assert!(from_note(&with(note("h"), |n| (n.channel = "scores".into(), n.score = half.clone())), &c, T0).unwrap().score.is_none());
        assert!(from_note(&with(note("i"), |n| n.score = sc.clone()), &c, T0).unwrap().score.is_none(), "only a score card has one");
    }

    #[test]
    fn only_a_card_at_the_ringing_priority_is_an_entry_and_one_that_rings_and_is_no_timer_or_doorbell_is_an_alarm() {
        let c = cfg();
        let card = Card {
            plugin_id: "t".into(),
            plugin: "Timers".into(),
            id: "pasta".into(),
            revision: 1,
            created_at: 0.0,
            expires_at: 0.0,
            priority: "ringing".into(),
            content: json!({"type": "event", "title": "Pasta", "detail": "Time's up", "icon": "base.icons/timer", "role": "accent"}),
            actions: vec![json!({"action": "stop", "label": "Stop"})],
        };
        let e = from_card(&card, &c, T0).unwrap();
        assert_eq!((e.key.as_str(), e.cls, e.title.as_str(), e.body.as_str()), ("c:t/pasta", "timer", "Pasta", "Time's up"));
        assert!(from_card(&Card { priority: "running".into(), ..card.clone() }, &c, T0).is_none());
        let ring = Card { content: json!({"type": "event", "title": "Ring", "icon": "base.icons/message"}), ..card };
        assert_eq!(from_card(&ring, &c, T0).unwrap().cls, "alarm");
    }

    #[test]
    fn an_entry_is_taken_once_the_same_record_again_is_an_update_and_what_the_person_dismissed_is_not_shown_again_until_it_changes() {
        let mut r = Rig::new();
        assert_eq!(r.take(&note("a")), Offer::New);
        assert_eq!(r.take(&note("a")), Offer::Updated);
        assert_eq!(r.q.list().len(), 1);
        assert_eq!(r.take(&with(note("a"), |n| (n.revision = 2, n.body = "Door sensor is at 4 %".into()))), Offer::Updated);
        assert_eq!(r.q.list()[0].body, "Door sensor is at 4 %");
        r.now();
        assert!(r.q.dismiss("n:ha/a", r.t));
        assert_eq!(r.q.list().len(), 0);
        assert_eq!(r.take(&with(note("a"), |n| n.revision = 2)), Offer::Dismissed);
        assert_eq!(r.take(&with(note("a"), |n| n.revision = 3)), Offer::New);
        // a record the plugin ended and posted again is news whatever its revision
        r.q.dismiss("n:ha/a", r.t);
        assert!(!r.q.withdraw("n:ha/a"));
        assert_eq!(r.take(&with(note("a"), |n| n.revision = 1)), Offer::New);
    }

    fn door(id: &str) -> Note {
        with(note(id), |n| {
            (n.channel = "doorbell".into(), n.channel_icon = "base.icons/doorbell".into(), n.title = "Doorbell".into(), n.body = "Front door".into())
        })
    }

    #[test]
    fn the_most_important_class_first_and_a_lower_class_waits_while_a_higher_one_is_up() {
        let mut r = Rig::new();
        r.take(&with(note("score"), |n| (n.channel = "scores".into(), n.title = "Lakers 102, Celtics 99".into())));
        r.take(&note("alert"));
        r.take(&door("door"));
        assert_eq!(r.ids(), vec!["door"]);
        r.q.dismiss("n:ha/door", r.t);
        assert_eq!(r.ids(), vec!["alert"]);
        r.q.dismiss("n:ha/alert", r.t);
        assert_eq!(r.ids(), vec!["score"]);
    }

    #[test]
    fn two_cards_at_most_of_the_same_class_a_doorbell_and_an_alarm_are_one_class_and_a_third_waits() {
        let mut r = Rig::new();
        r.take(&with(door("d"), |n| n.title = "Doorbell".into()));
        r.take(&with(note("t"), |n| (n.channel = "timers".into(), n.title = "Pasta".into())));
        r.take(&with(note("a"), |n| (n.channel = "alarms".into(), n.title = "Weekday".into())));
        assert_eq!(r.ids(), vec!["d", "t"]);
        r.q.dismiss("n:ha/d", r.t);
        assert_eq!(r.ids(), vec!["t", "a"]);
        r.take(&note("x1"));
        r.take(&note("x2"));
        assert_eq!(r.ids(), vec!["t", "a"]);
    }

    #[test]
    fn two_alerts_show_together_and_a_second_score_does_not_show_beside_a_score() {
        let mut r = Rig::new();
        for id in ["a1", "a2", "a3"] {
            r.take(&note(id));
        }
        assert_eq!(r.ids(), vec!["a1", "a2"]);
        for id in ["a1", "a2", "a3"] {
            r.q.dismiss(&format!("n:ha/{id}"), r.t);
        }
        r.take(&with(note("s1"), |n| n.channel = "scores".into()));
        r.take(&with(note("s2"), |n| n.channel = "scores".into()));
        assert_eq!(r.ids(), vec!["s1"]);
    }

    #[test]
    fn a_card_that_would_end_below_the_limit_waits_and_the_first_card_always_shows() {
        let mut r = Rig::new();
        r.h = 400.0;
        r.take(&note("a1"));
        r.take(&note("a2"));
        // 134 + 400 fits; the second would end at 134 + 400 + 16 + 400 = 950 > 810
        assert_eq!(r.ids(), vec!["a1"]);
        r.q.dismiss("n:ha/a1", r.t);
        assert_eq!(r.ids(), vec!["a2"]);
        r.h = 900.0;
        assert_eq!(r.ids(), vec!["a2"], "taller than the zone: it still shows, it is alone");
    }

    #[test]
    fn the_cameras_popup_takes_a_slot_and_the_top_of_the_column_a_second_card_goes_under_it_if_it_fits_else_waits() {
        let mut r = Rig::new();
        r.take(&with(note("t"), |n| (n.channel = "timers".into(), n.title = "Pasta".into())));
        let keys = |s: Selection| s.visible.iter().map(|k| k.split('/').next_back().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(keys(r.ctx(false, true, 134.0 + 448.0 + 16.0)), vec!["t"], "598 + 150 = 748 <= 810");
        assert!(keys(r.ctx(false, true, 134.0 + 626.0 + 16.0)).is_empty(), "776 + 150 = 926 > 810: it waits");
        // an alert waits under a camera card: the camera is the most important class
        r.take(&note("al"));
        r.q.dismiss("n:ha/t", r.t);
        assert!(keys(r.ctx(false, true, 134.0 + 448.0 + 16.0)).is_empty());
        // with the camera's card gone the alert shows at the top
        r.t += 1;
        assert_eq!(keys(r.ctx(false, false, 134.0)), vec!["al"]);
    }

    #[test]
    fn a_doorbell_the_camera_shows_is_not_a_second_card_one_queued_is_taken_in_one_that_comes_after_is_merged() {
        let mut r = Rig::new();
        r.take(&door("ring"));
        assert_eq!(r.ids(), vec!["ring"]);
        r.t += 400;
        let c = r.cfg.clone();
        assert_eq!(r.q.camera_shown("doorbell", &c, r.t), 1);
        assert!(r.ids().is_empty());
        // the record staying in the drawer does not come back as a card
        assert_eq!(r.take(&door("ring")), Offer::Merged);
        r.t += c.merge_ms as u64 + 1000;
        assert_eq!(r.take(&door("ring")), Offer::Dismissed);
        // a ring that comes after the camera's card
        r.q.camera_shown("doorbell", &c, r.t);
        assert_eq!(r.take(&door("ring2")), Offer::Merged);
        // another kind of camera event is nothing to a doorbell record
        let mut r = Rig::new();
        let c = r.cfg.clone();
        r.q.camera_shown("person", &c, r.t);
        assert_eq!(r.take(&door("ring3")), Offer::New);
    }

    #[test]
    fn nothing_shows_while_the_host_is_busy_the_cards_wait_keep_what_time_they_had_and_come_back() {
        let mut r = Rig::new();
        r.take(&note("a"));
        assert_eq!(r.ids(), vec!["a"]);
        r.t += 4000;
        assert!(r.ctx(true, false, 134.0).visible.is_empty());
        assert!(!r.q.list()[0].visible);
        assert_eq!(r.q.list()[0].shown_ms, 4000);
        r.t += 30_000;
        let sel = r.now();
        assert_eq!(sel.visible.len(), 1);
        // 4 s of its 10 were seen: first the keys become its (armMs), then 6 s from now it is done
        assert_eq!(sel.wake_at, r.t + r.cfg.arm_ms as u64);
        let back = r.t;
        r.t += r.cfg.arm_ms as u64;
        assert_eq!(r.now().wake_at, back + 6000);
        r.t = back + 6000;
        assert!(r.now().visible.is_empty());
        assert_eq!(r.q.list().len(), 0);
    }

    #[test]
    fn a_card_is_up_for_its_classs_time_counted_while_it_is_shown_and_is_not_news_again_for_that_revision() {
        let mut r = Rig::new();
        let s = with(note("s"), |n| (n.channel = "scores".into(), n.title = "Lakers 102, Celtics 99".into()));
        r.take(&s);
        let sel = r.now();
        assert_eq!(sel.visible.len(), 1);
        assert_eq!(sel.wake_at, r.t + r.cfg.arm_ms as u64);
        r.t += 8000;
        assert!(r.now().visible.is_empty());
        assert_eq!(r.take(&s), Offer::Dismissed);
    }

    #[test]
    fn a_waiting_card_is_dropped_when_it_has_waited_longer_than_its_class_allows_and_a_records_own_end_ends_it() {
        let mut r = Rig::new();
        r.take(&with(note("s"), |n| (n.channel = "scores".into(), n.title = "Final".into())));
        r.ctx(true, false, 134.0);
        r.t += r.cfg.class("score").wait_ms as u64;
        assert!(r.now().visible.is_empty());
        assert_eq!(r.q.list().len(), 0);
        r.take(&with(note("e"), |n| n.expires_at = (r.t + 3000) as f64 / 1000.0));
        assert_eq!(r.ids(), vec!["e"]);
        r.t += 3000;
        assert!(r.ids().is_empty());
    }

    #[test]
    fn a_higher_class_that_arrives_takes_the_screen_and_the_card_it_displaced_comes_back_with_the_time_it_had_left() {
        let mut r = Rig::new();
        r.take(&note("a"));
        r.now();
        r.t += 3000;
        r.take(&with(door("d"), |n| n.title = "Doorbell".into()));
        assert_eq!(r.ids(), vec!["d"]);
        assert_eq!(r.q.find("n:ha/a").unwrap().shown_ms, 3000);
        r.q.dismiss("n:ha/d", r.t);
        r.t += 1000;
        let back = r.t;
        assert_eq!(r.ids(), vec!["a"]);
        r.t += r.cfg.arm_ms as u64;
        assert_eq!(r.now().wake_at, back + 7000);
    }

    #[test]
    fn a_score_that_moves_is_news_again_while_it_is_up_for_no_more_than_three_times_its_time() {
        let mut r = Rig::new();
        let s = |title: &str, rev: u64| with(note("s"), |n| (n.channel = "scores".into(), n.title = title.into(), n.revision = rev));
        r.take(&s("A 1, B 0", 1));
        r.now();
        r.t += 6000;
        assert_eq!(r.take(&s("A 1, B 1", 2)), Offer::Updated);
        let sel = r.now();
        assert_eq!(sel.visible.len(), 1);
        assert_eq!(sel.wake_at, r.t + 8000, "the keys were its long ago: only the end of its time is ahead");
        assert_eq!(r.q.list()[0].title, "A 1, B 1");
        // far into its life it does not stretch for ever
        r.t += 8000 * 3;
        r.take(&s("A 2, B 1", 3));
        assert!(r.q.list().first().is_none_or(|e| e.shown_at < r.t), "its time was not started afresh");
    }

    #[test]
    fn a_score_again_too_soon_after_the_last_is_not_shown() {
        let mut r = Rig::new();
        let s = |title: &str, rev: u64| with(note("s"), |n| (n.channel = "scores".into(), n.title = title.into(), n.revision = rev));
        r.take(&s("A 1, B 0", 1));
        r.now();
        r.t += 8000;
        r.now();
        r.t += 1000;
        assert_eq!(r.take(&s("A 2, B 0", 2)), Offer::Cooldown);
        r.t += r.cfg.class("score").cooldown_ms as u64;
        assert_eq!(r.take(&s("A 3, B 0", 3)), Offer::New);
    }

    #[test]
    fn the_keys_are_the_cards_only_after_it_has_been_up_for_arm_ms() {
        let mut r = Rig::new();
        r.take(&note("a"));
        let sel = r.now();
        let arm = r.cfg.arm_ms as u64;
        let c = r.cfg.clone();
        assert!(!r.q.armed(&sel.visible, &c, r.t));
        assert!(!r.q.armed(&sel.visible, &c, r.t + arm - 1));
        assert!(r.q.armed(&sel.visible, &c, r.t + arm));
        assert!(!r.q.armed(&[], &c, r.t));
    }

    #[test]
    fn what_ok_does_is_the_records_first_action_else_the_drawer_else_it_dismisses_and_a_long_word_is_cut() {
        let c = cfg();
        let with_action =
            from_note(&with(note("a"), |n| n.actions = vec![json!({"action": "clear", "label": "Clear"}), json!({"action": "x", "label": "More"})]), &c, T0)
                .unwrap();
        assert_eq!(ok_of(&with_action, true).label, "Clear");
        assert_eq!(keys_of(&with_action, false), vec![("OK", "Clear".to_string()), ("Back", "Dismiss".to_string())]);
        let plain = from_note(&note("b"), &c, T0).unwrap();
        assert_eq!(ok_of(&plain, true), Ok { label: "Open".into(), action: None });
        assert_eq!(keys_of(&plain, true), vec![("OK", "Open".to_string()), ("Back", "Dismiss".to_string())]);
        assert_eq!(keys_of(&plain, false), vec![("OK", "Dismiss".to_string())]);
        let long =
            from_note(&with(note("c"), |n| n.actions = vec![json!({"action": "x", "label": "Turn off every light in the house now please"})]), &c, T0).unwrap();
        let l = ok_of(&long, true).label;
        assert_eq!(l.chars().count(), 24);
        assert!(l.ends_with('\u{2026}'));
    }

    #[test]
    fn a_withdrawn_records_card_goes_at_once_and_a_plugin_that_floods_the_queue_does_not_grow_it_for_ever() {
        let mut r = Rig::new();
        r.take(&note("a"));
        assert_eq!(r.ids(), vec!["a"]);
        assert!(r.q.withdraw("n:ha/a"));
        assert!(r.ids().is_empty());
        let mut r = Rig::new();
        r.take(&with(door("first"), |n| n.title = "Doorbell".into()));
        r.now();
        for i in 0..40 {
            r.take(&with(note(&format!("s{i}")), |n| (n.channel = "scores".into(), n.title = format!("Game {i}"))));
        }
        r.take(&with(note("last"), |n| n.title = "Alert".into()));
        assert_eq!(r.q.list().len(), MAX_QUEUE);
        assert!(r.q.find("n:ha/first").is_some(), "what is shown stays");
        assert!(r.q.find("n:ha/last").is_some(), "so does the newest");
        assert!(r.q.find("n:ha/s0").is_none(), "the oldest score is the one that went");
        assert_eq!(r.q.list().iter().filter(|e| e.cls == "score").count(), MAX_QUEUE - 2);
    }
}
