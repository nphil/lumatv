//! The question AI access asks on the TV (`AiPrompt.qml` of the QML shell, `docs/mocks/05-ai-request-prompt`): an assistant wants to turn the TV
//! off, switch an input or open an app, or the owner has asked for a wider level, a pairing code or the listener. It shows the helper's
//! `ai.prompt` topic and nothing else: who asks, the action in the helper's own words (never the assistant's), the assistant's reason apart and
//! labelled as its own, and how long is left.
//!
//! **Principle 0.** A prompt an AI can reach is not a control, so this screen cannot answer it and nothing in the shell can: it has no button,
//! no focus and no effect. Every key and every click while it is up is taken and does nothing, so it never reaches what is under the prompt;
//! OK and Back are read by the helper itself from the physical remote's input device (`LGE RCU`, `LGE M-RCU*`), and a key that arrives any other
//! way (Home Assistant's `webostv.button`, SSAP, a network key) is not an answer. The model returns no effect for any input but the clock (a
//! test holds it to that). It is over everything, the screensaver and an app's picture included, where the QML's was drawn inside Home only.

use crate::kit::screen::{Effect, Input};
use serde_json::{Value, json};

/// The helper's topic of the open prompt (`helper/app/src/ai/gate.rs`): `null` while there is none.
pub const TOPIC: &str = "ai.prompt";
/// How long a prompt waits for an answer when its document does not say (the helper's `prompt_ttl`).
const DEFAULT_TTL_MS: u64 = 30_000;
/// The countdown's words and bar are read again this often.
const STEP_MS: u64 = 500;

/// One prompt, as the helper words it.
#[derive(Clone, Debug, PartialEq)]
pub struct Prompt {
    pub id: String,
    /// What is asked (`tv_standby`, `tv_launch_app`, `enable_listener`, `pair`, ...): picks the glyph.
    pub kind: String,
    /// The assistant's name, or "" when the owner asked (a wider level, a pairing code).
    pub who: String,
    pub title: String,
    pub detail: String,
    /// The assistant's own reason, in its own words.
    pub reason: String,
    pub shown_at: u64,
    pub expires_at: u64,
}

impl Prompt {
    /// The prompt a topic carries; none for `null` and for anything that is not a document.
    pub fn parse(v: &Value) -> Option<Prompt> {
        let o = v.as_object().filter(|o| !o.is_empty())?;
        let text = |k: &str| o.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
        let who = o.get("assistant").and_then(|a| a.get("name")).and_then(Value::as_str).unwrap_or("").trim().to_string();
        let stamp = |k: &str| o.get(k).and_then(Value::as_u64).unwrap_or(0);
        Some(Prompt { id: text("id"), kind: text("kind"), who, title: text("title"), detail: text("detail"), reason: text("reason"), shown_at: stamp("shownAt"), expires_at: stamp("expiresAt") })
    }

    /// How long the prompt is open in all, in milliseconds.
    pub fn total_ms(&self) -> u64 {
        if self.expires_at > self.shown_at && self.shown_at > 0 { self.expires_at - self.shown_at } else { DEFAULT_TTL_MS }
    }

    /// The glyph that says what is asked.
    pub fn icon(&self) -> &'static str {
        match self.kind.as_str() {
            "tv_standby" => "power",
            "tv_launch_app" => "layout-grid",
            "tv_switch_input" => "cable",
            "enable_listener" => "bot",
            "pair" => "key-round",
            "resume" => "play",
            "raise_level" => "sliders-horizontal",
            "raise_volume" | "raise_limit" => "volume-2",
            _ => "circle-help",
        }
    }

    /// The line over the title: who asks, and that it came through AI access.
    pub fn from_line(&self) -> String {
        if self.who.is_empty() { "AI access".to_string() } else { format!("{} is asking \u{b7} through AI access", self.who) }
    }

    /// The same line without the words that only say how it came, for a header with little room.
    pub fn from_short(&self) -> String {
        if self.who.is_empty() { "AI access".to_string() } else { format!("{} is asking", self.who) }
    }

    /// The label over the assistant's reason, which is the assistant's and not the helper's.
    pub fn reason_label(&self) -> String {
        if self.who.is_empty() { "The assistant\u{2019}s reason, in its own words".to_string() } else { format!("{}\u{2019}s reason, in its own words", self.who) }
    }
}

/// What the surface draws.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub prompt: Prompt,
    /// Whole seconds left, rounded up.
    pub secs_left: u64,
    /// How much of the wait is left, 0 to 1.
    pub fraction: f32,
}

#[derive(Debug, Default)]
pub struct AiPrompt {
    prompt: Option<Prompt>,
    /// The next look at the clock while a prompt is up.
    next: Option<u64>,
}

impl AiPrompt {
    pub fn new() -> Self {
        Self::default()
    }

    /// A prompt is up: it has every key and the pointer.
    pub fn up(&self) -> bool {
        self.prompt.is_some()
    }

    pub fn wants(&self) -> bool {
        self.up()
    }

    /// The prompt's view at `now` (wall-clock milliseconds, the helper's stamps).
    pub fn view(&self, now: u64) -> Option<View> {
        let p = self.prompt.clone()?;
        let left_ms = p.expires_at.saturating_sub(now);
        let total = p.total_ms().max(1);
        Some(View { secs_left: left_ms.div_ceil(1000), fraction: (left_ms as f32 / total as f32).clamp(0.0, 1.0), prompt: p })
    }

    /// What the model does with an input: the helper's topic changes what is shown, the clock moves the countdown, and every other input,
    /// every key and click among them, is taken and does nothing. There is no effect it could return for them.
    pub fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        match input {
            Input::Helper { topic, data } if topic == TOPIC => {
                let next = Prompt::parse(data);
                if next == self.prompt {
                    return Vec::new();
                }
                self.next = next.as_ref().map(|_| now + STEP_MS);
                self.prompt = next;
                return vec![Effect::Redraw];
            }
            Input::Tick if self.prompt.is_some() && self.next.is_some_and(|t| now >= t) => {
                self.next = Some(now + STEP_MS);
                return vec![Effect::Redraw];
            }
            _ => {}
        }
        Vec::new()
    }

    pub fn due_at(&self) -> Option<u64> {
        self.next
    }

    /// What the gate reads of `ctl shell` (`ui.ai`).
    pub fn state(&self) -> Value {
        match &self.prompt {
            Some(p) => json!({"up": true, "id": p.id, "kind": p.kind, "who": p.who}),
            None => json!({"up": false}),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_800_000_000_000;

    fn doc(kind: &str, who: Option<&str>, reason: &str, shown: u64) -> Value {
        json!({"id": "a1b2c3", "kind": kind, "assistant": who.map(|w| json!({"id": "x", "name": w})), "title": "Turn the TV off", "detail": "The TV goes to standby.",
               "reason": reason, "shownAt": shown, "expiresAt": shown + 30_000, "allow": "OK", "deny": "Back"})
    }

    fn topic(v: Value) -> Input {
        Input::Helper { topic: TOPIC.into(), data: v }
    }

    #[test]
    fn the_helpers_topic_is_the_prompt_who_asks_what_is_asked_and_the_reason_apart_and_null_is_none() {
        let mut a = AiPrompt::new();
        assert!(!a.up() && a.view(T0).is_none() && a.due_at().is_none());
        assert_eq!(a.update(&topic(doc("tv_standby", Some("Cody"), "Nitin said good night", T0)), T0), vec![Effect::Redraw]);
        let v = a.view(T0 + 1_200).expect("a prompt");
        assert_eq!((v.prompt.who.as_str(), v.prompt.title.as_str(), v.prompt.reason.as_str()), ("Cody", "Turn the TV off", "Nitin said good night"));
        assert_eq!(v.prompt.icon(), "power");
        assert_eq!(v.prompt.from_line(), "Cody is asking \u{b7} through AI access");
        assert_eq!(v.prompt.reason_label(), "Cody\u{2019}s reason, in its own words");
        assert_eq!((v.secs_left, (v.fraction * 100.0).round() as u32), (29, 96), "28.8 s left of 30");
        // the owner asking (a wider level, a pairing code) has no assistant
        a.update(&topic(doc("pair", None, "", T0 + 5_000)), T0 + 5_000);
        let v = a.view(T0 + 5_000).unwrap();
        assert_eq!((v.prompt.from_line().as_str(), v.prompt.icon()), ("AI access", "key-round"));
        assert_eq!(a.state()["up"], true);
        // the topic going null takes it away
        assert_eq!(a.update(&topic(Value::Null), T0 + 6_000), vec![Effect::Redraw]);
        assert!(!a.up() && a.due_at().is_none());
        assert_eq!(a.state(), json!({"up": false}));
        // the same document again is nothing new, and a document that is not an object is none
        a.update(&topic(doc("resume", Some("Cody"), "", T0)), T0);
        assert!(a.update(&topic(doc("resume", Some("Cody"), "", T0)), T0 + 10).is_empty());
        a.update(&topic(json!("x")), T0 + 20);
        assert!(!a.up());
        a.update(&topic(json!({})), T0 + 30);
        assert!(!a.up(), "an empty document is no prompt");
        assert_eq!(Prompt::parse(&json!({"id": "q"})).map(|p| p.icon()), Some("circle-help"), "a kind it does not know has the generic glyph");
    }

    #[test]
    fn the_countdown_is_read_twice_a_second_while_it_is_up_and_it_never_goes_below_nothing() {
        let mut a = AiPrompt::new();
        a.update(&topic(doc("tv_launch_app", Some("Cody"), "", T0)), T0);
        assert_eq!(a.due_at(), Some(T0 + 500));
        assert!(a.update(&Input::Tick, T0 + 499).is_empty());
        assert_eq!(a.update(&Input::Tick, T0 + 500), vec![Effect::Redraw]);
        assert_eq!(a.due_at(), Some(T0 + 1_000));
        let late = a.view(T0 + 45_000).unwrap();
        assert_eq!((late.secs_left, late.fraction), (0, 0.0), "past its time it reads zero until the helper takes the topic away");
        // a prompt whose document gives no times is thirty seconds
        let bare = Prompt::parse(&json!({"id": "z", "title": "Pair"})).unwrap();
        assert_eq!(bare.total_ms(), 30_000);
    }

    #[test]
    fn nothing_the_remote_or_the_pointer_can_do_answers_it_and_no_input_but_the_clock_and_the_topic_has_any_effect() {
        let mut a = AiPrompt::new();
        a.update(&topic(doc("enable_listener", None, "", T0)), T0);
        let before = a.view(T0).unwrap();
        for name in ["ok", "back", "up", "down", "left", "right", "0", "1", "menu", "search", "any", "volumeup"] {
            for (down, repeat) in [(true, false), (true, true), (false, false)] {
                let fx = a.update(&Input::Key { name: name.into(), down, repeat }, T0 + 400);
                assert!(fx.is_empty(), "{name} down={down} repeat={repeat}: the prompt has no answer to give: {fx:?}");
            }
        }
        for buttons in [0, 1, 2] {
            assert!(a.update(&Input::Pointer { x: 960.0, y: 540.0, buttons, visible: true }, T0 + 400).is_empty(), "a click answers nothing");
        }
        // not Home Assistant's remote keys either, nor a luna answer, a fetched document or a theme patch
        assert!(a.update(&Input::Topic { topic: "syskey".into(), data: json!({"k": "ok", "d": true}) }, T0 + 400).is_empty());
        assert!(a.update(&Input::Luna { tag: 1, payload: json!({"returnValue": true}) }, T0 + 400).is_empty());
        assert!(a.update(&Input::Fetched { tag: 1, status: 200, data: json!({"ok": true}) }, T0 + 400).is_empty());
        assert!(a.update(&Input::Helper { topic: "ai".into(), data: json!({"listener": {"enabled": true}}) }, T0 + 400).is_empty());
        assert_eq!(a.view(T0).unwrap(), before, "still the same prompt, still up");
        assert!(a.up() && a.wants(), "and it still has the keys");
    }
}
