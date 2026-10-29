//! The overlay cards' model (`panels/Overlays.qml` of the QML shell): the queue of [`super::cards`] fed from the plugin hub and kept up to date
//! with what is on the screen, what the keys do, and when to look again. Nothing is drawn here; the surface measures a card's height (the queue
//! needs it to know how many fit) and draws what [`Overlay::shown`] says.
//!
//! - **Sources:** a notification the hub says is news (`posted`) is a card; a card at the `ringing` priority is one while it exists; the plugin ending
//!   its record takes the card away. The camera pop-up is the doorbell's own card: this queue only makes room for it and takes in a ring both say.
//! - **Keys:** OK acts (the record's first button, else the drawer opens) and Back dismisses, the top card's, once it has been up `armMs`.
//!   Every other key is the app's.
//! - **Switch:** the Rust shell answers the kind `cards`; with it on the queue runs whatever `overlays.enabled` says (that is the QML's switch).

use super::cards::{self, Config, Ctx, Entry, Offer, Queue};
use super::plugins::{Card, Note};
use crate::kit::screen::Effect;
use serde_json::{Value, json};
use std::collections::HashSet;

/// What the cards ask of the shell besides effects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// OK on a card with no action of its own: the notification drawer.
    OpenDrawer,
}

pub struct Overlay {
    cfg: Config,
    queue: Queue,
    shown: Vec<Entry>,
    armed: bool,
    wake_at: u64,
    dnd: bool,
    camera_up: bool,
    drawer_ready: bool,
    intents: Vec<Intent>,
    /// Something changed that the next refresh must look at.
    dirty: bool,
}

impl Overlay {
    pub fn new(theme_overlays: &Value) -> Self {
        let mut cfg = Config::from_theme(theme_overlays);
        cfg.enabled = true;
        Self { cfg, queue: Queue::new(), shown: Vec::new(), armed: false, wake_at: 0, dnd: false, camera_up: false, drawer_ready: false, intents: Vec::new(), dirty: false }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// The owner's numbers changed (`overlays` in the theme): the queue stays.
    pub fn set_config(&mut self, theme_overlays: &Value) {
        self.cfg = Config::from_theme(theme_overlays);
        self.cfg.enabled = true;
        self.dirty = true;
    }

    /// OK can open the notification drawer here.
    pub fn set_drawer_ready(&mut self, on: bool) {
        if self.drawer_ready != on {
            self.drawer_ready = on;
            self.dirty = true;
        }
    }

    /// Do not disturb: a PIN, an alert, the keyboard, a panel, the viewer, the saver: nothing shows, what waits is kept.
    pub fn set_dnd(&mut self, dnd: bool) {
        if self.dnd != dnd {
            self.dnd = dnd;
            self.dirty = true;
        }
    }

    /// The camera pop-up is up: it has a slot and the top of the column.
    pub fn set_camera(&mut self, up: bool) {
        if self.camera_up != up {
            self.camera_up = up;
            self.dirty = true;
        }
    }

    pub fn drawer_ready(&self) -> bool {
        self.drawer_ready
    }

    pub fn camera_up(&self) -> bool {
        self.camera_up
    }

    /// New notifications of the hub (`Changed::posted`).
    pub fn posted(&mut self, notes: &[Note], now: u64) {
        for n in notes {
            if let Some(e) = cards::from_note(n, &self.cfg, now) {
                self.take(e, now);
            }
        }
    }

    fn take(&mut self, e: Entry, now: u64) {
        self.queue.offer(e, &self.cfg, now);
        self.dirty = true;
    }

    /// The hub's notifications changed: a record the plugin ended (or that expired) takes its card with it.
    pub fn reconcile(&mut self, notes: &[Note]) {
        let present: HashSet<String> = notes.iter().map(|n| format!("n:{}/{}", n.plugin_id, n.id)).collect();
        if self.queue.reconcile("note", &present) {
            self.dirty = true;
        }
    }

    /// The hub's cards changed: those at the `ringing` priority are cards of the queue while they exist.
    pub fn sync_cards(&mut self, hub_cards: &[Card], now: u64) {
        let mut present = HashSet::new();
        for c in hub_cards {
            let Some(e) = cards::from_card(c, &self.cfg, now) else { continue };
            // one that started ringing long ago and was there before this surface is not news
            if c.created_at > 0.0 && now as f64 / 1000.0 - c.created_at > 120.0 && self.queue.find(&e.key).is_none() {
                continue;
            }
            present.insert(e.key.clone());
            if matches!(self.queue.offer(e, &self.cfg, now), Offer::New | Offer::Updated) {
                self.dirty = true;
            }
        }
        if self.queue.reconcile("card", &present) {
            self.dirty = true;
        }
    }

    /// The camera pop-up showed a card of this kind: a doorbell record of the same ring is not a second card.
    pub fn camera_shown(&mut self, kind: &str, now: u64) {
        self.queue.camera_shown(kind, &self.cfg, now);
        self.dirty = true;
    }

    pub fn needs_refresh(&self, now: u64) -> bool {
        self.dirty || (self.wake_at > 0 && now >= self.wake_at)
    }

    /// Looks at the queue again: what shows now, whether the keys are the top card's, when to look next. `top` is where the first card starts
    /// (the placement's place for the column, under the camera's card when that is up), `height_of` a card's height as it is drawn.
    pub fn refresh(&mut self, top: f64, height_of: &mut dyn FnMut(&Entry) -> f64, now: u64) {
        self.dirty = false;
        let mut ctx = Ctx { dnd: self.dnd, camera: self.camera_up, top, height_of };
        let sel = self.queue.select(&mut ctx, &self.cfg, now);
        self.shown = sel.visible.iter().filter_map(|k| self.queue.find(k).cloned()).collect();
        self.armed = self.queue.armed(&sel.visible, &self.cfg, now);
        self.wake_at = sel.wake_at;
    }

    /// The cards on the screen, top first.
    pub fn shown(&self) -> &[Entry] {
        &self.shown
    }

    /// Something waits or shows (the queue is not empty).
    pub fn pending(&self) -> bool {
        !self.queue.list().is_empty()
    }

    pub fn armed(&self) -> bool {
        self.armed && !self.shown.is_empty()
    }

    /// The keys the cards take by name while an app has the focus.
    pub fn takes(&self) -> Vec<&'static str> {
        if self.armed() { vec!["ok", "back"] } else { Vec::new() }
    }

    pub fn take_intents(&mut self) -> Vec<Intent> {
        std::mem::take(&mut self.intents)
    }

    /// A press of OK or Back: true when it was the top card's (the key is not the app's).
    pub fn key(&mut self, name: &str, now: u64, out: &mut Vec<Effect>) -> bool {
        if !self.armed() {
            return false;
        }
        let Some(top) = self.shown.first().cloned() else { return false };
        match name {
            "back" => {
                self.dismiss(&top, now);
                true
            }
            "ok" => {
                let ok = cards::ok_of(&top, self.drawer_ready);
                self.dismiss(&top, now);
                match ok.action {
                    Some(a) => {
                        let action = a.get("action").and_then(Value::as_str).unwrap_or("").to_string();
                        if !action.is_empty() {
                            let body = a.get("target").and_then(Value::as_str).map_or_else(|| json!({}), |t| json!({"target": t}));
                            out.push(Effect::Post { path: format!("/v1/plugins/{}/actions/{action}", top.plugin_id), body });
                        }
                    }
                    None if self.drawer_ready => self.intents.push(Intent::OpenDrawer),
                    None => {}
                }
                true
            }
            _ => false,
        }
    }

    fn dismiss(&mut self, e: &Entry, now: u64) {
        self.queue.dismiss(&e.key, now);
        self.dirty = true;
    }

    pub fn due_at(&self) -> Option<u64> {
        if self.dirty { Some(0) } else { (self.wake_at > 0).then_some(self.wake_at) }
    }

    pub fn state(&self) -> Value {
        json!({
            "shown": self.shown.iter().map(cards::entry_json).collect::<Vec<_>>(),
            "armed": self.armed(),
            "pending": self.pending(),
            "dnd": self.dnd,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_800_000_000_000;

    fn note(id: &str, kind: &str) -> Note {
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
            actions: vec![json!({"id": "clear", "label": "Clear", "action": "clear"})],
            kind: kind.into(),
            level: String::new(),
            privacy: String::new(),
            score: Value::Null,
            importance: String::new(),
            channel_kind: String::new(),
        }
    }

    fn refresh(o: &mut Overlay, now: u64) {
        o.refresh(134.0, &mut |_| 150.0, now);
    }

    #[test]
    fn a_posted_notification_is_a_card_whose_keys_are_the_apps_for_the_first_600_ms() {
        let mut o = Overlay::new(&json!({}));
        o.posted(&[note("n1", "")], T0);
        assert!(o.needs_refresh(T0));
        refresh(&mut o, T0);
        assert_eq!(o.shown().len(), 1);
        assert!(!o.armed() && o.takes().is_empty(), "a press made as it arrived was meant for the app");
        let mut fx = Vec::new();
        assert!(!o.key("back", T0 + 100, &mut fx));
        assert_eq!(o.due_at(), Some(T0 + 600), "it looks again when the keys become the card's");
        refresh(&mut o, T0 + 600);
        assert!(o.armed());
        assert_eq!(o.takes(), vec!["ok", "back"]);
    }

    #[test]
    fn ok_runs_the_first_action_back_dismisses_and_a_dismissed_record_is_not_shown_again() {
        let mut o = Overlay::new(&json!({}));
        o.posted(&[note("n1", "")], T0);
        refresh(&mut o, T0);
        refresh(&mut o, T0 + 700);
        let mut fx = Vec::new();
        assert!(o.key("ok", T0 + 800, &mut fx));
        assert_eq!(fx, vec![Effect::Post { path: "/v1/plugins/ha/actions/clear".into(), body: json!({}) }]);
        refresh(&mut o, T0 + 800);
        assert!(o.shown().is_empty());
        o.posted(&[note("n1", "")], T0 + 900);
        refresh(&mut o, T0 + 900);
        assert!(o.shown().is_empty(), "the same revision is not news again");
        let mut changed = note("n1", "");
        changed.revision = 2;
        o.posted(&[changed], T0 + 1000);
        refresh(&mut o, T0 + 1000);
        assert_eq!(o.shown().len(), 1, "a changed record is");
        refresh(&mut o, T0 + 1700);
        assert!(o.key("back", T0 + 1800, &mut fx));
        assert!(fx.len() == 1, "back runs nothing");
    }

    #[test]
    fn ok_on_a_card_without_an_action_opens_the_drawer_when_it_can_and_only_dismisses_otherwise() {
        let mut plain = note("n2", "");
        plain.actions.clear();
        let mut o = Overlay::new(&json!({}));
        o.set_drawer_ready(true);
        o.posted(&[plain.clone()], T0);
        refresh(&mut o, T0);
        refresh(&mut o, T0 + 700);
        let mut fx = Vec::new();
        assert!(o.key("ok", T0 + 800, &mut fx));
        assert!(fx.is_empty());
        assert_eq!(o.take_intents(), vec![Intent::OpenDrawer]);
        let mut p = Overlay::new(&json!({}));
        p.posted(&[plain], T0);
        refresh(&mut p, T0);
        refresh(&mut p, T0 + 700);
        assert!(p.key("ok", T0 + 800, &mut fx));
        assert!(p.take_intents().is_empty());
    }

    #[test]
    fn nothing_shows_while_the_host_is_busy_and_what_waits_comes_back() {
        let mut o = Overlay::new(&json!({}));
        o.set_dnd(true);
        o.posted(&[note("n1", "")], T0);
        refresh(&mut o, T0);
        assert!(o.shown().is_empty() && o.pending());
        o.set_dnd(false);
        refresh(&mut o, T0 + 2_000);
        assert_eq!(o.shown().len(), 1);
    }

    #[test]
    fn a_record_the_plugin_ended_takes_its_card_and_a_record_posted_again_is_news() {
        let mut o = Overlay::new(&json!({}));
        o.posted(&[note("n1", "")], T0);
        refresh(&mut o, T0);
        refresh(&mut o, T0 + 700);
        let mut fx = Vec::new();
        o.key("back", T0 + 800, &mut fx);
        o.reconcile(&[]);
        o.posted(&[note("n1", "")], T0 + 900);
        refresh(&mut o, T0 + 900);
        assert_eq!(o.shown().len(), 1, "gone from the hub, so forgotten as dismissed");
        o.reconcile(&[]);
        refresh(&mut o, T0 + 950);
        assert!(o.shown().is_empty() && !o.pending());
    }

    #[test]
    fn a_ringing_card_is_a_card_while_it_exists_and_an_old_one_is_not_news() {
        let ring = |id: &str, created: f64| Card {
            plugin_id: "timers".into(),
            plugin: "Timers".into(),
            id: id.into(),
            revision: 1,
            created_at: created,
            expires_at: 0.0,
            priority: "ringing".into(),
            content: json!({"title": "Pasta", "detail": "Time's up", "kind": "timer", "icon": "timer"}),
            actions: vec![json!({"id": "stop", "label": "Stop", "action": "stop"})],
        };
        let mut o = Overlay::new(&json!({}));
        o.sync_cards(&[ring("t1", T0 as f64 / 1000.0 - 5.0), ring("old", T0 as f64 / 1000.0 - 600.0)], T0);
        refresh(&mut o, T0);
        assert_eq!(o.shown().len(), 1);
        assert_eq!(o.shown()[0].title, "Pasta");
        o.sync_cards(&[], T0 + 1000);
        refresh(&mut o, T0 + 1000);
        assert!(o.shown().is_empty(), "the plugin ended it");
    }

    #[test]
    fn the_camera_popup_has_the_top_and_a_doorbell_it_showed_is_not_a_second_card() {
        let mut o = Overlay::new(&json!({}));
        o.camera_shown("doorbell", T0);
        let mut bell = note("d1", "doorbell");
        bell.channel = "doorbell".into();
        o.posted(&[bell], T0 + 2_000);
        refresh(&mut o, T0 + 2_000);
        assert!(o.shown().is_empty(), "folded into the camera's card");
        let mut seen = Vec::new();
        o.set_camera(true);
        o.posted(&[note("n9", "timer")], T0 + 3_000);
        o.refresh(134.0 + 469.0 + 16.0, &mut |e| {
            seen.push(e.key.clone());
            150.0
        }, T0 + 3_000);
        assert_eq!(o.shown().len(), 1, "one slot beside the window");
        assert!(!seen.is_empty(), "the heights were asked for");
    }
}
