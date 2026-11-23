//! The voice assistant's overlay model: the helper's `voice.assistant` topic (one turn of the TV's microphone as a Home Assistant voice
//! satellite) turned into what the screen shows: a glow while it listens and thinks, the words as they were heard, and Home Assistant's
//! reply. Everything on it is transient: a turn that nothing has changed for [`FAILSAFE_MS`] is gone, and a finished one stays only as
//! long as its text takes to read. The words and the reply are for the scene alone: nothing here logs them, and [`Assistant::state`]
//! (what the compositor's report carries) holds counts only.

use crate::kit::screen::{Effect, Input, Model};
use serde_json::{Value, json};

pub const TOPIC: &str = "voice.assistant";
const SCHEMA: u64 = 1;
/// The longest a word waits for the next, and the longest the whole reveal of a transcript takes (more words come faster).
pub const WORD_MS: u64 = 75;
pub const REVEAL_MS: u64 = 1_200;
/// A finished turn's card stays this long and 60 ms for each character, at most 6 s, and then fades over [`CARD_FADE_MS`].
pub const CARD_BASE_MS: u64 = 2_500;
pub const CARD_CHAR_MS: u64 = 60;
pub const CARD_MAX_MS: u64 = 6_000;
pub const CARD_FADE_MS: u64 = 400;
/// The glow fades out over this when its phase is over.
pub const GLOW_FADE_MS: u64 = 600;
/// Whatever a turn does, nothing of it is on the screen 35 s after it last changed (the fade starts [`GLOW_FADE_MS`] before).
pub const FAILSAFE_MS: u64 = 35_000;
/// Longer text is cut: the scene shows three lines at most.
const TEXT_CHARS: usize = 600;
/// What a satellite's turn is labelled with when the helper names none.
const SATELLITE: &str = "Voice satellite";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Listening,
    Thinking,
    Replying,
    Done,
    Failed,
}

impl Phase {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "listening" => Self::Listening,
            "thinking" => Self::Thinking,
            "replying" => Self::Replying,
            "done" => Self::Done,
            "failed" => Self::Failed,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::Listening => "listening",
            Self::Thinking => "thinking",
            Self::Replying => "replying",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    fn ends(self) -> bool {
        matches!(self, Self::Done | Self::Failed)
    }
}

/// What the glow does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glow {
    Off,
    Listening,
    Thinking,
}

/// The words as they were heard: `shown` of them are on, the next comes after `step_ms`. `epoch` changes when the text is replaced
/// rather than grown, so the scene starts its reveal again rather than carrying on.
#[derive(Clone, Debug, PartialEq)]
pub struct Heard {
    pub text: String,
    pub epoch: u64,
    pub shown: usize,
    pub step_ms: u32,
}

/// Home Assistant's reply, or the reason the turn failed, verbatim.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub text: String,
    pub ok: bool,
}

/// What the surface draws.
#[derive(Clone, Debug, PartialEq)]
pub struct Show {
    pub turn: u64,
    pub glow: Glow,
    pub heard: Option<Heard>,
    pub card: Option<Card>,
    /// Which device was listening, when it was not the TV: the TV only shows what a voice satellite heard.
    pub label: Option<String>,
    /// The turn is on its way out: everything fades.
    pub leaving: bool,
}

pub type View = Option<Show>;

#[derive(Clone, Debug, PartialEq)]
struct Doc {
    id: u64,
    phase: Phase,
    heard: String,
    reply: String,
    error: String,
    ok: Option<bool>,
    label: Option<String>,
}

/// Single spaces, at most [`TEXT_CHARS`] characters.
fn clean(v: Option<&Value>) -> String {
    let text = v.and_then(Value::as_str).unwrap_or("");
    text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(TEXT_CHARS).collect()
}

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(' ').filter(|w| !w.is_empty())
}

/// The turn of a document, `None` for anything that is not one the overlay draws: no topic, another schema, switched off, no turn, or a
/// turn that does not read. Never panics.
fn parse(data: &Value) -> Option<Doc> {
    if data.get("schema")?.as_u64()? != SCHEMA || !data.get("enabled")?.as_bool()? {
        return None;
    }
    let turn = data.get("turn")?.as_object()?;
    let id = turn.get("id")?.as_f64().filter(|n| n.is_finite() && (0.0..9.0e15).contains(n) && n.fract() == 0.0)? as u64;
    let phase = Phase::parse(turn.get("phase")?.as_str()?)?;
    let field = |name: &str| data.get(name).or_else(|| turn.get(name));
    let label = (field("source").and_then(Value::as_str) == Some("satellite")).then(|| {
        let label = clean(field("label"));
        if label.is_empty() { SATELLITE.to_owned() } else { label }
    });
    Some(Doc {
        id,
        phase,
        heard: clean(turn.get("heard")),
        reply: clean(turn.get("reply")),
        error: clean(turn.get("error")),
        ok: turn.get("ok").and_then(Value::as_bool),
        label,
    })
}

#[derive(Clone, Debug)]
struct Turn {
    doc: Doc,
    /// The last time the turn changed: the failsafe counts from it.
    changed: u64,
    /// When the turn reached a phase that ends it, or changed after that: the card's dwell counts from here.
    rest: Option<u64>,
    epoch: u64,
    shown: usize,
    step: u64,
    next: Option<u64>,
}

impl Turn {
    /// The text of the card: the failure's reason when it failed with one, else the reply.
    fn card_text(&self) -> Option<&str> {
        let d = &self.doc;
        let text = if d.phase == Phase::Failed && !d.error.is_empty() { &d.error } else { &d.reply };
        (matches!(d.phase, Phase::Replying | Phase::Done | Phase::Failed) && !text.is_empty()).then_some(text.as_str())
    }

    /// How long a finished turn stays and then fades: by the characters of what it shows; nothing shown, nothing stays.
    fn dwell(&self) -> (u64, u64) {
        let shown = self.card_text().or_else(|| Some(self.doc.heard.as_str()).filter(|h| !h.is_empty()));
        shown.map_or((0, 0), |text| {
            let stays = (CARD_BASE_MS + CARD_CHAR_MS * text.chars().count() as u64).min(CARD_MAX_MS);
            (stays, stays + CARD_FADE_MS)
        })
    }

    fn leave_at(&self) -> u64 {
        let failsafe = (self.changed + FAILSAFE_MS).saturating_sub(GLOW_FADE_MS);
        self.rest.map_or(failsafe, |r| (r + self.dwell().0).min(failsafe))
    }

    fn gone_at(&self) -> u64 {
        let failsafe = self.changed + FAILSAFE_MS;
        self.rest.map_or(failsafe, |r| (r + self.dwell().1).min(failsafe))
    }

    /// Sets the pace for the words that are not on yet, so that they are all on within [`REVEAL_MS`].
    fn pace(&mut self, now: u64) {
        let pending = words(&self.doc.heard).count().saturating_sub(self.shown) as u64;
        self.step = (REVEAL_MS / pending.max(1)).clamp(1, WORD_MS);
        self.next = (pending > 0).then(|| self.next.unwrap_or(now));
    }

    /// Puts on the words that are due; whether any came.
    fn advance(&mut self, now: u64) -> bool {
        let total = words(&self.doc.heard).count();
        let mut moved = false;
        while let Some(at) = self.next.filter(|at| *at <= now) {
            if self.shown < total {
                self.shown += 1;
                moved = true;
            }
            self.next = (self.shown < total).then_some(at + self.step);
        }
        moved
    }

    fn revise(&mut self, doc: Doc, now: u64, epochs: &mut u64) {
        if doc.heard != self.doc.heard {
            let (old, new) = (words(&self.doc.heard).count(), words(&doc.heard).count());
            let grown = old > 0 && new >= old && words(&doc.heard).take(old).eq(words(&self.doc.heard));
            if !grown {
                *epochs += 1;
                self.epoch = *epochs;
                self.shown = 0;
                self.next = None;
            }
            self.doc.heard = doc.heard.clone();
            self.pace(now);
        }
        self.rest = doc.phase.ends().then_some(now);
        self.doc = doc;
        self.changed = now;
        self.advance(now);
    }
}

/// The overlay's model.
#[derive(Clone, Debug)]
pub struct Assistant {
    turn: Option<Turn>,
    /// The turn that last ended: its late or repeated documents are not a new turn.
    ended: Option<u64>,
    /// No document has come since the start or since the helper came back: the first is what the topic held before, not a turn that is
    /// happening, and a finished turn in it is not drawn again.
    fresh: bool,
    leaving: bool,
    epochs: u64,
    dirty: bool,
}

impl Default for Assistant {
    fn default() -> Self {
        Self { turn: None, ended: None, fresh: true, leaving: false, epochs: 0, dirty: false }
    }
}

impl Assistant {
    pub fn new() -> Self {
        Self::default()
    }

    fn take(&mut self, doc: Doc, now: u64) {
        if std::mem::take(&mut self.fresh) && doc.phase.ends() {
            self.ended = Some(doc.id);
            return;
        }
        if self.ended == Some(doc.id) && doc.phase != Phase::Listening {
            return;
        }
        match self.turn.as_mut().filter(|t| t.doc.id == doc.id) {
            Some(t) if t.doc == doc => return,
            Some(t) => t.revise(doc, now, &mut self.epochs),
            None => {
                let mut t = Turn { rest: doc.phase.ends().then_some(now), doc, changed: now, epoch: 0, shown: 0, step: WORD_MS, next: None };
                self.epochs += 1;
                t.epoch = self.epochs;
                t.pace(now);
                t.advance(now);
                self.turn = Some(t);
            }
        }
        self.dirty = true;
    }

    fn clear(&mut self) {
        self.dirty |= self.turn.take().is_some();
    }

    /// Brings the turn to `now`: the words that are due, the start of its leaving, its end.
    fn tick(&mut self, now: u64) {
        let Some(t) = self.turn.as_mut() else {
            self.leaving = false;
            return;
        };
        self.dirty |= t.advance(now);
        if now >= t.gone_at() {
            self.ended = Some(t.doc.id);
            self.turn = None;
            self.leaving = false;
            self.dirty = true;
            return;
        }
        let leaving = now >= t.leave_at();
        self.dirty |= leaving != self.leaving;
        self.leaving = leaving;
    }

    /// The state for the compositor's report: counts and names, never the words.
    pub fn state(&self) -> Value {
        match &self.turn {
            None => json!({"turn": null}),
            Some(t) => json!({
                "turn": t.doc.id,
                "phase": t.doc.phase.name(),
                "words": words(&t.doc.heard).count(),
                "shown": t.shown,
                "card": t.card_text().is_some(),
                "leaving": self.leaving,
            }),
        }
    }
}

impl Model for Assistant {
    type View = View;

    fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        self.tick(now);
        match input {
            Input::Helper { topic, data } if topic == TOPIC => match parse(data) {
                Some(doc) => self.take(doc, now),
                None => {
                    self.fresh = false;
                    self.clear();
                }
            },
            // the helper is gone: what it last said is over
            Input::Helper { topic, data } if topic == "up" && data == &json!(false) => {
                self.fresh = true;
                self.clear();
            }
            _ => {}
        }
        self.tick(now);
        if std::mem::take(&mut self.dirty) { vec![Effect::Redraw] } else { Vec::new() }
    }

    fn view(&self) -> View {
        let t = self.turn.as_ref()?;
        let d = &t.doc;
        let glow = match d.phase {
            _ if self.leaving => Glow::Off,
            Phase::Listening => Glow::Listening,
            Phase::Thinking => Glow::Thinking,
            _ => Glow::Off,
        };
        Some(Show {
            turn: d.id,
            glow,
            heard: (!d.heard.is_empty()).then(|| Heard { text: d.heard.clone(), epoch: t.epoch, shown: t.shown, step_ms: t.step as u32 }),
            card: t.card_text().map(|text| Card { text: text.to_owned(), ok: d.phase != Phase::Failed && d.ok != Some(false) }),
            label: d.label.clone(),
            leaving: self.leaving,
        })
    }

    fn due_at(&self) -> Option<u64> {
        let t = self.turn.as_ref()?;
        let due = if self.leaving { t.gone_at() } else { t.leave_at() };
        Some(t.next.map_or(due, |next| next.min(due)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(id: u64, phase: &str, heard: &str, reply: &str) -> Value {
        json!({"schema": 1, "enabled": true, "mic": "turn",
            "turn": {"id": id, "phase": phase, "heard": (!heard.is_empty()).then_some(heard), "reply": (!reply.is_empty()).then_some(reply),
                     "ok": null, "spoken": false, "error": null}})
    }

    fn feed(m: &mut Assistant, data: Value, now: u64) -> bool {
        m.update(&Input::Helper { topic: TOPIC.into(), data }, now).contains(&Effect::Redraw)
    }

    fn tick(m: &mut Assistant, now: u64) -> bool {
        m.update(&Input::Tick, now).contains(&Effect::Redraw)
    }

    /// A model that has seen the topic once already, as a running one has.
    fn running() -> Assistant {
        let mut m = Assistant::new();
        feed(&mut m, json!(null), 0);
        m
    }

    fn shown(m: &Assistant) -> usize {
        m.view().and_then(|v| v.heard).map_or(0, |h| h.shown)
    }

    #[test]
    fn a_document_that_is_not_a_turn_draws_nothing() {
        let bad = [
            json!(null), json!(7), json!("turn"), json!([]), json!({}),
            json!({"schema": 2, "enabled": true, "turn": {"id": 1, "phase": "listening"}}),
            json!({"schema": 1, "enabled": false, "turn": {"id": 1, "phase": "listening"}}),
            json!({"schema": 1, "enabled": "yes", "turn": {"id": 1, "phase": "listening"}}),
            json!({"schema": 1, "enabled": true, "mic": "listening", "turn": null}),
            json!({"schema": 1, "enabled": true, "turn": []}),
            json!({"schema": 1, "enabled": true, "turn": {"id": "1", "phase": "listening"}}),
            json!({"schema": 1, "enabled": true, "turn": {"id": -1, "phase": "listening"}}),
            json!({"schema": 1, "enabled": true, "turn": {"id": 1.5, "phase": "listening"}}),
            json!({"schema": 1, "enabled": true, "turn": {"id": 1, "phase": "dancing"}}),
            json!({"schema": 1, "enabled": true, "turn": {"id": 1, "phase": 4}}),
        ];
        for data in bad {
            let mut m = running();
            feed(&mut m, data.clone(), 10);
            assert_eq!(m.view(), None, "{data}");
        }
        let odd = json!({"schema": 1, "enabled": true, "turn": {"id": 3, "phase": "replying", "heard": 4, "reply": ["x"], "error": {}, "ok": "no"}});
        let mut m = running();
        feed(&mut m, odd, 10);
        assert_eq!(m.view().map(|v| (v.heard, v.card)), Some((None, None)));
    }

    #[test]
    fn the_glow_follows_the_phase_and_a_later_document_may_skip_phases() {
        let mut m = running();
        let glow = |m: &Assistant| m.view().map(|v| v.glow);
        feed(&mut m, doc(1, "listening", "", ""), 0);
        assert_eq!(glow(&m), Some(Glow::Listening));
        feed(&mut m, doc(1, "thinking", "turn on the lamp", ""), 100);
        assert_eq!(glow(&m), Some(Glow::Thinking));
        feed(&mut m, doc(1, "done", "turn on the lamp", "Okay, the lamp is on."), 900);
        let v = m.view().unwrap();
        assert_eq!((v.glow, v.card), (Glow::Off, Some(Card { text: "Okay, the lamp is on.".into(), ok: true })));
        feed(&mut m, doc(2, "failed", "", ""), 1000);
        assert_eq!(m.view(), None, "a turn with nothing to show is not drawn");
    }

    #[test]
    fn the_cards_tone_and_text_come_from_the_failure_when_there_is_one() {
        let mut m = running();
        let mut failed = doc(1, "failed", "lights", "ignored");
        failed["turn"]["error"] = json!("Home Assistant did not answer");
        feed(&mut m, failed, 0);
        assert_eq!(m.view().and_then(|v| v.card), Some(Card { text: "Home Assistant did not answer".into(), ok: false }));
        let mut sorry = doc(2, "done", "", "Sorry, I did not understand");
        sorry["turn"]["ok"] = json!(false);
        feed(&mut m, sorry, 5);
        assert_eq!(m.view().and_then(|v| v.card).map(|c| c.ok), Some(false));
    }

    #[test]
    fn a_repeated_document_changes_nothing_and_extends_nothing() {
        let mut m = running();
        assert!(feed(&mut m, doc(1, "listening", "", ""), 1000));
        let due = m.due_at();
        assert!(!feed(&mut m, doc(1, "listening", "", ""), 20_000));
        assert_eq!(m.due_at(), due);
        assert_eq!(due, Some(1000 + FAILSAFE_MS - GLOW_FADE_MS));
    }

    #[test]
    fn words_come_one_after_another_and_all_of_them_within_the_cap() {
        let mut m = running();
        feed(&mut m, doc(1, "thinking", "one two three four five six seven eight nine ten", ""), 1000);
        assert_eq!((shown(&m), m.due_at()), (1, Some(1000 + WORD_MS)));
        assert!(tick(&mut m, 1000 + WORD_MS));
        assert_eq!(shown(&m), 2);
        tick(&mut m, 1000 + 9 * WORD_MS);
        assert_eq!(shown(&m), 10);
        assert!(!tick(&mut m, 1000 + 10 * WORD_MS), "nothing is left to come");

        let long = vec!["word"; 40].join(" ");
        let mut m = running();
        feed(&mut m, doc(2, "thinking", &long, ""), 0);
        let step = m.view().and_then(|v| v.heard).map(|h| u64::from(h.step_ms)).unwrap();
        assert_eq!(step, REVEAL_MS / 40);
        tick(&mut m, 39 * step);
        assert_eq!(shown(&m), 40);
        assert!(40 * step <= REVEAL_MS);
    }

    #[test]
    fn words_that_follow_the_ones_heard_are_added_and_others_replace_them() {
        let mut m = running();
        feed(&mut m, doc(1, "thinking", "turn on the", ""), 0);
        tick(&mut m, 200);
        let epoch = m.view().and_then(|v| v.heard).map(|h| h.epoch).unwrap();
        feed(&mut m, doc(1, "thinking", "turn on the lights", ""), 1000);
        let h = m.view().and_then(|v| v.heard).unwrap();
        assert_eq!((h.epoch, h.shown), (epoch, 4), "only the new word comes");
        feed(&mut m, doc(1, "thinking", "switch off", ""), 2000);
        let h = m.view().and_then(|v| v.heard).unwrap();
        assert_eq!((h.epoch > epoch, h.shown, h.text.as_str()), (true, 1, "switch off"));
    }

    #[test]
    fn a_finished_card_stays_for_its_length_and_then_goes() {
        let reply = "Okay, the lamp is on.";
        let stays = CARD_BASE_MS + CARD_CHAR_MS * reply.chars().count() as u64;
        let mut m = running();
        feed(&mut m, doc(1, "done", "", reply), 1000);
        assert_eq!(m.due_at(), Some(1000 + stays));
        assert!(!tick(&mut m, 1000 + stays - 1));
        assert!(tick(&mut m, 1000 + stays));
        assert_eq!(m.view().map(|v| v.leaving), Some(true));
        assert_eq!(m.due_at(), Some(1000 + stays + CARD_FADE_MS));
        assert!(tick(&mut m, 1000 + stays + CARD_FADE_MS));
        assert_eq!(m.view(), None);

        let mut m = running();
        feed(&mut m, doc(2, "done", "", &"long ".repeat(60)), 0);
        assert_eq!(m.due_at(), Some(CARD_MAX_MS));
    }

    #[test]
    fn a_finished_turn_with_only_words_keeps_them_for_their_length_and_one_with_nothing_goes_at_once() {
        let mut m = running();
        feed(&mut m, doc(1, "done", "lights", ""), 0);
        assert_eq!(m.view().map(|v| (v.heard.is_some(), v.card)), Some((true, None)));
        assert_eq!(m.due_at(), Some(CARD_BASE_MS + 6 * CARD_CHAR_MS));
        feed(&mut m, doc(2, "failed", "", ""), 100);
        assert_eq!(m.view(), None);
    }

    #[test]
    fn nothing_stays_past_the_failsafe_however_the_turn_stalls() {
        let mut m = running();
        feed(&mut m, doc(1, "listening", "", ""), 0);
        assert!(!tick(&mut m, FAILSAFE_MS - GLOW_FADE_MS - 1));
        assert!(tick(&mut m, FAILSAFE_MS - GLOW_FADE_MS));
        let v = m.view().unwrap();
        assert_eq!((v.leaving, v.glow), (true, Glow::Off));
        assert!(tick(&mut m, FAILSAFE_MS));
        assert_eq!(m.view(), None);
        // a change is the last word: the time counts from it
        let mut m = running();
        feed(&mut m, doc(2, "listening", "", ""), 0);
        feed(&mut m, doc(2, "thinking", "", ""), 10_000);
        tick(&mut m, FAILSAFE_MS);
        assert!(m.view().is_some());
        tick(&mut m, 10_000 + FAILSAFE_MS);
        assert_eq!(m.view(), None);
    }

    #[test]
    fn a_turn_the_process_was_not_there_for_does_not_come_back() {
        let mut m = Assistant::new();
        assert!(!feed(&mut m, doc(7, "done", "lights", "Okay"), 0), "the topic's old value on start");
        assert_eq!(m.view(), None);
        feed(&mut m, doc(8, "listening", "", ""), 50);
        assert!(m.view().is_some());
        feed(&mut m, doc(8, "done", "", "Okay"), 60);
        tick(&mut m, 60 + CARD_BASE_MS + 4 * CARD_CHAR_MS + CARD_FADE_MS);
        assert_eq!(m.view(), None);
        assert!(!feed(&mut m, doc(8, "done", "", "Okay"), 20_000), "a late repeat of the turn that ended");
        assert_eq!(m.view(), None);
        // a turn that is in progress when the process starts is shown
        let mut m = Assistant::new();
        feed(&mut m, doc(9, "thinking", "lights", ""), 0);
        assert!(m.view().is_some());
    }

    #[test]
    fn the_helper_going_away_clears_the_turn() {
        let mut m = running();
        feed(&mut m, doc(1, "listening", "", ""), 0);
        assert!(m.update(&Input::Helper { topic: "up".into(), data: json!(false) }, 5).contains(&Effect::Redraw));
        assert_eq!(m.view(), None);
        assert!(!feed(&mut m, doc(1, "done", "", "Okay"), 10), "what it held when it came back is old");
    }

    #[test]
    fn a_satellites_turn_names_the_device_and_a_tv_turn_names_none() {
        let mut m = running();
        feed(&mut m, doc(1, "listening", "", ""), 0);
        assert_eq!(m.view().and_then(|v| v.label), None);
        let mut sat = doc(1, "listening", "", "");
        sat["source"] = json!("satellite");
        sat["label"] = json!("Living Room Echo Dot");
        feed(&mut m, sat.clone(), 10);
        assert_eq!(m.view().and_then(|v| v.label).as_deref(), Some("Living Room Echo Dot"));
        sat["label"] = json!(null);
        feed(&mut m, sat, 20);
        assert_eq!(m.view().and_then(|v| v.label).as_deref(), Some(SATELLITE));
    }

    #[test]
    fn what_the_report_carries_holds_no_words() {
        let mut m = running();
        feed(&mut m, doc(1, "replying", "secret phrase", "private answer"), 0);
        let state = m.state().to_string();
        assert!(!state.contains("secret") && !state.contains("private"), "{state}");
        assert_eq!(m.state()["words"], 2);
    }
}
