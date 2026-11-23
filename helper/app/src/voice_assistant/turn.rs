//! What the screen is told of a turn: the `voice.assistant` topic's document.
//!
//! One turn at a time. The TV's own turn (what the voice host shows) and another assistant's in the room (what Home Assistant says
//! of its satellite, see `dot.rs`) go through one model, so the surface draws both the same way. The words live in this document
//! and nowhere else: nothing here logs them.

use serde_json::{Value, json};
use std::time::{Duration, Instant};
use voice_service::Shown;

/// How long a finished turn stays in the topic. The screen keeps its card for as long as its text takes to read; this only has to
/// outlast that, so a turn does not vanish under a reader.
pub const LINGER: Duration = Duration::from_secs(8);
/// A turn nothing ended is dropped this long after it last changed (the screen's own failsafe is 35 s).
const STALE: Duration = Duration::from_secs(40);
/// The most characters of heard or replied words put in the topic (the screen shows fewer).
const MAX_WORDS: usize = 600;

/// Whether the microphone is open, and whether something is being said to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mic {
    Off,
    Listening,
    Turn,
}

impl Mic {
    fn name(self) -> &'static str {
        match self {
            Mic::Off => "off",
            Mic::Listening => "listening",
            Mic::Turn => "turn",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Listening,
    Thinking,
    Replying,
    Done,
    Failed,
}

impl Phase {
    fn name(self) -> &'static str {
        match self {
            Phase::Listening => "listening",
            Phase::Thinking => "thinking",
            Phase::Replying => "replying",
            Phase::Done => "done",
            Phase::Failed => "failed",
        }
    }

    fn ended(self) -> bool {
        matches!(self, Phase::Done | Phase::Failed)
    }
}

/// What Home Assistant says of another assistant's turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Other {
    Listening,
    Thinking,
    Replying,
    Idle,
    Heard(String),
    Reply(String),
    Failed(String),
}

#[derive(Clone, Debug)]
struct Turn {
    id: u64,
    phase: Phase,
    heard: String,
    reply: String,
    error: String,
    ok: Option<bool>,
    spoken: bool,
    /// The assistant that has the turn when it is not this TV.
    label: Option<String>,
    /// When it last changed: a finished turn is dropped `LINGER` after, one that never finished `STALE` after.
    changed: Instant,
}

impl Turn {
    fn limit(&self) -> Duration {
        if self.phase.ended() { LINGER } else { STALE }
    }

    /// Whether this is the TV's own turn, still going.
    fn own_live(&self) -> bool {
        self.label.is_none() && !self.phase.ended()
    }

    fn other_live(&self, label: &str) -> bool {
        self.label.as_deref() == Some(label) && !self.phase.ended()
    }
}

fn words(text: &str) -> String {
    text.trim().chars().take(MAX_WORDS).collect()
}

/// The document of the `voice.assistant` topic, kept up to date from what the voice host and the watcher of other assistants say.
pub struct Overlay {
    enabled: bool,
    mic: Mic,
    turn: Option<Turn>,
    next: u64,
}

impl Overlay {
    /// `first` is the first turn's id; the next ones count up, so an id is never seen twice by a surface that outlives the helper.
    pub fn new(first: u64) -> Self {
        Self {
            enabled: false,
            mic: Mic::Off,
            turn: None,
            next: first,
        }
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.turn = None;
            self.mic = Mic::Off;
        }
    }

    pub fn set_mic(&mut self, mic: Mic) {
        self.mic = mic;
    }

    /// Whether the TV's own turn is going (its words are the voice host's, and it has the screen).
    pub fn own_turn(&self) -> bool {
        self.turn.as_ref().is_some_and(Turn::own_live)
    }

    fn begin(&mut self, label: Option<&str>, phase: Phase, now: Instant) -> &mut Turn {
        self.next += 1;
        self.turn.insert(Turn {
            id: self.next,
            phase,
            heard: String::new(),
            reply: String::new(),
            error: String::new(),
            ok: None,
            spoken: false,
            label: label.map(str::to_owned),
            changed: now,
        })
    }

    /// The TV's own turn that is going, or a new one.
    fn own(&mut self, now: Instant) -> &mut Turn {
        if !self.own_turn() {
            self.begin(None, Phase::Listening, now);
        }
        let turn = self.turn.as_mut().expect("a turn was just begun");
        turn.changed = now;
        turn
    }

    /// What the voice host shows of the TV's own turn. A turn of the TV's takes the screen from another assistant's.
    pub fn shown(&mut self, shown: &Shown, now: Instant) {
        if !self.enabled {
            return;
        }
        match shown {
            Shown::Listening => {
                // One that is listening already is the same turn.
                if !self.turn.as_ref().is_some_and(|t| t.own_live() && t.phase == Phase::Listening) {
                    self.begin(None, Phase::Listening, now);
                }
            }
            Shown::Thinking => self.own(now).phase = Phase::Thinking,
            Shown::Transcript(text) => self.own(now).heard = words(text),
            Shown::Reply { text, ok: true } => {
                let turn = self.own(now);
                turn.reply = words(text);
                turn.ok = Some(true);
                turn.phase = Phase::Replying;
            }
            Shown::Reply { text, ok: false } => {
                let turn = self.own(now);
                turn.error = words(text);
                turn.ok = Some(false);
                turn.phase = Phase::Failed;
            }
            Shown::Speaking => self.own(now).spoken = true,
            Shown::Idle => {
                if self.own_turn() {
                    let turn = self.own(now);
                    turn.phase = Phase::Done;
                }
            }
        }
    }

    /// What Home Assistant says of the turn of the assistant called `label`. The TV's own turn, while it goes, has the screen.
    pub fn other(&mut self, label: &str, event: &Other, now: Instant) {
        if !self.enabled || self.own_turn() {
            return;
        }
        let live = self.turn.as_ref().is_some_and(|t| t.other_live(label));
        match event {
            Other::Idle => {
                if let Some(turn) = self.turn.as_mut().filter(|t| t.other_live(label)) {
                    turn.phase = Phase::Done;
                    turn.changed = now;
                }
                return;
            }
            Other::Listening if live => return,
            _ => {}
        }
        if !live {
            self.begin(Some(label), Phase::Listening, now);
        }
        let turn = self.turn.as_mut().expect("a turn was just begun");
        turn.changed = now;
        match event {
            Other::Listening | Other::Idle => {}
            Other::Thinking => turn.phase = Phase::Thinking,
            Other::Replying => turn.phase = Phase::Replying,
            Other::Heard(text) => turn.heard = words(text),
            Other::Reply(text) => {
                turn.reply = words(text);
                turn.ok = Some(true);
                turn.phase = Phase::Replying;
            }
            Other::Failed(text) => {
                turn.error = words(text);
                turn.ok = Some(false);
                turn.phase = Phase::Failed;
            }
        }
    }

    /// Drops a turn that has been over long enough; whether the document changed.
    pub fn tick(&mut self, now: Instant) -> bool {
        let over = self
            .turn
            .as_ref()
            .is_some_and(|t| now.saturating_duration_since(t.changed) >= t.limit());
        if over {
            self.turn = None;
        }
        over
    }

    pub fn doc(&self) -> Value {
        let mut doc = json!({"schema": 1, "enabled": self.enabled, "mic": self.mic.name(), "turn": null});
        if let Some(turn) = &self.turn {
            doc["turn"] = json!({
                "id": turn.id,
                "phase": turn.phase.name(),
                "heard": turn.heard,
                "reply": turn.reply,
                "ok": turn.ok,
                "spoken": turn.spoken,
                "error": turn.error,
            });
            if let Some(label) = &turn.label {
                doc["source"] = json!("satellite");
                doc["label"] = json!(label);
            }
        }
        doc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlay() -> (Overlay, Instant) {
        let mut overlay = Overlay::new(100);
        overlay.set_enabled(true);
        (overlay, Instant::now())
    }

    fn turn(overlay: &Overlay) -> Value {
        overlay.doc()["turn"].clone()
    }

    #[test]
    fn the_tvs_own_turn_runs_from_listening_to_done_under_one_id() {
        let (mut o, t0) = overlay();
        o.set_mic(Mic::Turn);
        o.shown(&Shown::Listening, t0);
        let id = turn(&o)["id"].clone();
        assert_eq!(turn(&o)["phase"], "listening");
        o.shown(&Shown::Thinking, t0);
        o.shown(&Shown::Transcript("what time is it".into()), t0);
        o.shown(&Shown::Reply { text: "It is five".into(), ok: true }, t0);
        o.shown(&Shown::Speaking, t0);
        let now = turn(&o);
        assert_eq!(now["id"], id);
        assert_eq!(
            (&now["phase"], &now["heard"], &now["reply"], &now["spoken"], &now["ok"]),
            (&json!("replying"), &json!("what time is it"), &json!("It is five"), &json!(true), &json!(true))
        );
        o.shown(&Shown::Idle, t0);
        assert_eq!(turn(&o)["phase"], "done");
        assert_eq!(o.doc()["mic"], "turn");
        assert!(o.doc().get("source").is_none(), "the TV's own turn has no source");
    }

    #[test]
    fn a_listening_that_repeats_is_one_turn_and_the_next_after_the_end_is_new() {
        let (mut o, t0) = overlay();
        o.shown(&Shown::Listening, t0);
        let first = turn(&o)["id"].as_u64().unwrap();
        o.shown(&Shown::Listening, t0);
        assert_eq!(turn(&o)["id"].as_u64().unwrap(), first);
        o.shown(&Shown::Idle, t0);
        o.shown(&Shown::Listening, t0);
        assert!(turn(&o)["id"].as_u64().unwrap() > first);
        assert_eq!(turn(&o)["phase"], "listening");
    }

    #[test]
    fn a_refusal_with_no_turn_before_it_is_a_failed_turn_that_lingers_and_goes() {
        let (mut o, t0) = overlay();
        o.shown(&Shown::Reply { text: "Home Assistant is not connected.".into(), ok: false }, t0);
        let failed = turn(&o);
        assert_eq!((&failed["phase"], &failed["ok"]), (&json!("failed"), &json!(false)));
        assert_eq!(failed["error"], "Home Assistant is not connected.");
        assert!(!o.tick(t0 + LINGER - Duration::from_millis(1)));
        assert!(o.tick(t0 + LINGER));
        assert!(turn(&o).is_null());
    }

    #[test]
    fn a_turn_nothing_ended_is_dropped_after_the_stale_limit() {
        let (mut o, t0) = overlay();
        o.shown(&Shown::Listening, t0);
        assert!(!o.tick(t0 + STALE - Duration::from_millis(1)));
        assert!(o.tick(t0 + STALE));
    }

    #[test]
    fn another_assistants_turn_carries_its_name_and_gives_way_to_the_tvs() {
        let (mut o, t0) = overlay();
        o.other("Echo Dot", &Other::Listening, t0);
        o.other("Echo Dot", &Other::Heard("turn on the lamp".into()), t0);
        o.other("Echo Dot", &Other::Reply("Done".into()), t0);
        let doc = o.doc();
        assert_eq!((&doc["source"], &doc["label"]), (&json!("satellite"), &json!("Echo Dot")));
        assert_eq!(
            (&doc["turn"]["phase"], &doc["turn"]["heard"], &doc["turn"]["reply"]),
            (&json!("replying"), &json!("turn on the lamp"), &json!("Done"))
        );
        o.other("Echo Dot", &Other::Idle, t0);
        assert_eq!(turn(&o)["phase"], "done");

        o.shown(&Shown::Listening, t0);
        assert!(o.doc().get("source").is_none(), "the TV's turn replaced it");
        o.other("Echo Dot", &Other::Listening, t0);
        assert_eq!(turn(&o)["phase"], "listening");
        assert!(o.doc().get("source").is_none(), "another assistant cannot take the screen while the TV's turn goes");
    }

    #[test]
    fn an_assistant_that_failed_stays_failed_when_it_goes_idle() {
        let (mut o, t0) = overlay();
        o.other("Echo Dot", &Other::Thinking, t0);
        o.other("Echo Dot", &Other::Failed("No speech heard".into()), t0);
        o.other("Echo Dot", &Other::Idle, t0);
        assert_eq!(turn(&o)["phase"], "failed");
    }

    #[test]
    fn a_disabled_overlay_has_no_turn_and_takes_none() {
        let mut o = Overlay::new(1);
        o.shown(&Shown::Listening, Instant::now());
        assert_eq!(o.doc(), json!({"schema": 1, "enabled": false, "mic": "off", "turn": null}));
        o.set_enabled(true);
        o.set_mic(Mic::Listening);
        o.shown(&Shown::Listening, Instant::now());
        o.set_enabled(false);
        assert_eq!(o.doc(), json!({"schema": 1, "enabled": false, "mic": "off", "turn": null}));
    }

    #[test]
    fn words_are_trimmed_and_capped() {
        let (mut o, t0) = overlay();
        o.shown(&Shown::Transcript(format!("  {}  ", "a".repeat(MAX_WORDS + 50))), t0);
        assert_eq!(turn(&o)["heard"].as_str().unwrap().len(), MAX_WORDS);
    }
}
