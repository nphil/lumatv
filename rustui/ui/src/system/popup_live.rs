//! The live video of the camera pop-up (`docs/research/camera-popup-result.md`): a window on the display's video plane for the card's picture box, asked of
//! the compositor as a claim of kind `camera-popup`, and the stream the helper plays into it.
//!
//! The order is the point. The claim is made first and the helper is asked for the stream only once the compositor has *granted* a window: the
//! compositor says no where an app has a video on the screen (a second decoder beside it can take the app's picture away), and a decoder that was
//! started before the answer would be one the screen did not allow. The picture box is a hole over the plane only when all of this holds: the helper says
//! the stream plays, the claim is carried by a window that is drawing, videooutputd lists the video on a sink (the proof that the decoder is connected;
//! a hole over a decoder that is not would be black), and `HANDOFF_MS` have passed since the helper said so (the decoder's first picture is on the plane a
//! few frames after it is fed). Until then, and whenever any of it stops holding, the card shows the stills it always had.
//!
//! A refused ask, a stream that fails twice, or one that does not play within `WITHIN_MS` ends the live video for this card (the stills go on); a window
//! that is taken back while the stream plays ends it at once (the helper is told to stop). The shell posts the claim from [`Live::claim`] and tells the
//! model what the compositor made of it with [`Live::set_window`].

use crate::cameras::app::Window;
use crate::kit::screen::Effect;
use serde_json::{Value, json};

/// The id of the pop-up's claim on a spare video window (the compositor's `vwin`).
pub const CLAIM_ID: &str = "camera-popup";
/// The stream the claim says it needs: one that either window takes (the helper asks the bridge for the highest stream inside the sub window's limits,
/// 1920x1088 at 30), so that the claim is not remade when the stream turns out to be another.
const NEED: (&str, u32, u32, u32) = ("h264", 1920, 1088, 30);
/// The stills stay under the hole this long after the helper says the stream plays.
pub const HANDOFF_MS: u64 = 250;
/// A stream asked for that does not play within this long is given up.
pub const WITHIN_MS: u64 = 8_000;
/// A stream that failed is asked for again after this long.
pub const RETRY_MS: u64 = 700;
/// How many times a card asks for its stream.
pub const TRIES: u8 = 2;
/// The tags of the asks this makes of the helper.
pub const TAG_FIRST: u64 = 0x5359_1F00;
pub const TAG_LAST: u64 = 0x5359_1FFF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Nothing asked of the helper: a window is waited for.
    Idle,
    /// The helper was asked at `at`, the `tries`-th time, and has not said that the stream plays.
    Asked { at: u64, tries: u8 },
    /// The helper says it plays, since.
    Playing { since: u64, tries: u8 },
    /// It failed: asked again at `at`.
    Retry { at: u64, tries: u8 },
    /// Given up for this card: stills only.
    Given,
}

/// What `cameras.live` says of the stream, as far as the card reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stream {
    pub camera: String,
    pub mode: String,
    pub state: String,
    /// What the camera says it runs at, and what the feeder took over the last second.
    pub fps: f32,
    pub fed_fps: f32,
    /// What the stream was asked for (`pop-up 41`); empty from a helper that does not say.
    pub purpose: String,
    /// The size of the picture the decoder is fed (the helper follows a change of size in the stream); 0 before it is known.
    pub width: u32,
    pub height: u32,
}

impl Stream {
    pub fn parse(data: &Value) -> Stream {
        let text = |k: &str| data.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let num = |k: &str| data.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let size = |k: &str| data.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
        Stream {
            camera: text("camera"),
            mode: text("mode"),
            state: text("state"),
            fps: num("fps"),
            fed_fps: num("fedFps"),
            purpose: text("purpose"),
            width: size("width"),
            height: size("height"),
        }
    }
}

pub struct Live {
    phase: Phase,
    window: Window,
    slot: String,
    sink: bool,
    stream: Stream,
    rect: Option<(i32, i32, i32, i32)>,
    /// The tag of the ask in flight.
    ask_tag: Option<u64>,
    counter: u64,
    /// The card being served: its camera, and its id for the helper's log.
    camera: String,
    card: Option<u64>,
}

impl Default for Live {
    fn default() -> Self {
        Self::new()
    }
}

impl Live {
    pub fn new() -> Self {
        Self {
            phase: Phase::Idle,
            window: Window::Unclaimed,
            slot: String::new(),
            sink: false,
            stream: Stream::default(),
            rect: None,
            ask_tag: None,
            counter: TAG_FIRST,
            camera: String::new(),
            card: None,
        }
    }

    /// What the helper's log and topic call this card's stream (`popup` of the ask's body: `pop-up 41`).
    fn purpose(&self) -> String {
        self.card.map_or_else(|| "pop-up".to_owned(), |id| format!("pop-up {id}"))
    }

    /// Whether `tag` is one of this model's asks.
    pub fn owns(tag: u64) -> bool {
        (TAG_FIRST..=TAG_LAST).contains(&tag)
    }

    fn next_tag(&mut self) -> u64 {
        let t = self.counter;
        self.counter = if t >= TAG_LAST { TAG_FIRST } else { t + 1 };
        t
    }

    /// A card for `camera` came up: nothing is asked or known of any window yet.
    pub fn begin(&mut self, camera: &str, card: Option<u64>) {
        *self = Self { counter: self.counter, rect: self.rect, camera: camera.to_string(), card, ..Self::new() };
    }

    /// The card is gone (or stepped aside): the stream it asked for is stopped, unless the full view is to take it over, and everything is forgotten.
    pub fn end(&mut self, stop: bool) -> Vec<Effect> {
        let had = matches!(self.phase, Phase::Asked { .. } | Phase::Playing { .. } | Phase::Retry { .. });
        let mut fx = Vec::new();
        if stop && had {
            fx.push(stop_post());
        }
        *self = Self { counter: self.counter, rect: self.rect, ..Self::new() };
        fx
    }

    /// The picture box on the screen, in output pixels (the surface measures it).
    pub fn set_rect(&mut self, rect: Option<(i32, i32, i32, i32)>) {
        self.rect = rect;
    }

    pub fn window(&self) -> Window {
        self.window
    }

    /// Given up for this card: the stills are the picture.
    pub fn given(&self) -> bool {
        self.phase == Phase::Given
    }

    /// The size of this card's stream as decoded, once the helper says it: the shape the picture box takes.
    pub fn size(&self) -> Option<(u32, u32)> {
        let s = &self.stream;
        let ours = !self.camera.is_empty() && s.camera == self.camera && s.mode == "live" && (s.purpose.is_empty() || s.purpose == self.purpose());
        (ours && s.width > 0 && s.height > 0).then_some((s.width, s.height))
    }

    /// The claim the shell posts for the card's picture box, while a card that can play live video is up and has not given up. `texture_only`: the camera's
    /// streams would take the panel's rate along with a window's decoder, so the claim asks for the compositor's texture and never a window.
    pub fn claim(&self, capable: bool, texture_only: bool) -> Option<Value> {
        let (x, y, w, h) = self.rect?;
        if !capable || self.camera.is_empty() || self.phase == Phase::Given {
            return None;
        }
        let (codec, nw, nh, fps) = NEED;
        let mut claim = json!({"id": CLAIM_ID, "kind": "camera-popup", "rect": {"x": x, "y": y, "w": w, "h": h}, "need": {"codec": codec, "w": nw, "h": nh, "fps": fps}});
        if texture_only {
            claim["texture_only"] = json!(true);
        }
        Some(claim)
    }

    fn ask(&mut self, now: u64, tries: u8) -> Vec<Effect> {
        let tag = self.next_tag();
        self.ask_tag = Some(tag);
        self.phase = Phase::Asked { at: now, tries };
        // the window the compositor named: `main` and `sub` are the display's, `texture` the compositor's own drawing of the camera's low stream
        let window = match self.slot.as_str() {
            "main" => "main",
            "texture" => "texture",
            _ => "sub",
        };
        let mut body = json!({"camera": self.camera, "for": "popup", "window": window});
        if let Some(card) = self.card {
            body["popup"] = json!(card);
        }
        vec![Effect::Request { tag, method: "POST".into(), path: "/v1/cameras/live".into(), body: Some(body) }]
    }

    /// What the compositor made of the claim (the shell reads it from the `vwin` topic), and whether videooutputd lists the video on a sink. A window is
    /// what starts the stream; one taken back stops it.
    pub fn set_window(&mut self, window: Window, slot: &str, sink: bool, now: u64) -> Vec<Effect> {
        self.window = window;
        self.sink = sink;
        // the claim was asked again and came back with another kind of window (Home came to the front or went: a texture for a sub window or the reverse):
        // the stream that plays is the old window's and the new one is asked for
        let moved = !slot.is_empty() && !self.slot.is_empty() && self.slot != slot && matches!(window, Window::Waiting | Window::Carried);
        if !slot.is_empty() {
            self.slot = slot.to_string();
        }
        if moved && matches!(self.phase, Phase::Asked { .. } | Phase::Playing { .. } | Phase::Retry { .. }) {
            self.phase = Phase::Idle;
            self.ask_tag = None;
        }
        let mut fx = Vec::new();
        match window {
            Window::Waiting | Window::Carried => {
                if self.phase == Phase::Idle && !self.camera.is_empty() {
                    fx.extend(self.ask(now, 1));
                }
            }
            Window::Refused => {
                if matches!(self.phase, Phase::Asked { .. } | Phase::Playing { .. } | Phase::Retry { .. }) {
                    fx.push(stop_post());
                    self.phase = Phase::Idle;
                    self.ask_tag = None;
                }
            }
            Window::Unclaimed => {}
        }
        fx
    }

    /// The answer to the ask: the helper refused (a camera that is offline, no stream inside the limits, the live path off).
    pub fn answered(&mut self, tag: u64, status: u16, _data: &Value) -> Vec<Effect> {
        if self.ask_tag != Some(tag) {
            return Vec::new();
        }
        self.ask_tag = None;
        if status == 200 {
            return Vec::new();
        }
        self.phase = Phase::Given;
        vec![Effect::Log(format!("camera pop-up: the helper refused the live stream ({status})"))]
    }

    /// `cameras.live`: the stream the helper plays.
    pub fn topic(&mut self, data: &Value, now: u64) -> Vec<Effect> {
        self.stream = Stream::parse(data);
        // The helper's topic keeps the last words of the stream before this card's: a card that replaces another is asked for while the old one's stop is
        // being answered, and its `stopped` (or `playing`) is not this card's stream. A stream asked for something else while this one plays took the
        // session (the full view); while this one is only asked, it is the previous card's and the card waits for its own.
        if !self.stream.purpose.is_empty() && self.stream.purpose != self.purpose() {
            if matches!(self.phase, Phase::Playing { .. }) {
                self.phase = Phase::Given;
            }
            return Vec::new();
        }
        let tries = match self.phase {
            Phase::Asked { tries, .. } | Phase::Playing { tries, .. } => tries,
            _ => return Vec::new(),
        };
        if self.stream.camera != self.camera || self.stream.mode != "live" {
            // another stream took the helper's one session (the full view of another camera): this card's is gone, and it is not the card's to stop
            if !self.stream.camera.is_empty() {
                self.phase = Phase::Given;
            }
            return Vec::new();
        }
        match self.stream.state.as_str() {
            "playing" => {
                if let Phase::Asked { tries, .. } = self.phase {
                    self.phase = Phase::Playing { since: now, tries };
                }
                Vec::new()
            }
            "failed" | "stopped" => {
                if tries < TRIES && self.stream.state == "failed" {
                    self.phase = Phase::Retry { at: now + RETRY_MS, tries };
                    Vec::new()
                } else {
                    self.phase = Phase::Given;
                    vec![stop_post()]
                }
            }
            _ => Vec::new(),
        }
    }

    /// Time passed: a retry is due, or an ask has waited too long.
    pub fn tick(&mut self, now: u64) -> Vec<Effect> {
        match self.phase {
            Phase::Retry { at, tries } if now >= at && matches!(self.window, Window::Waiting | Window::Carried) => self.ask(now, tries + 1),
            Phase::Asked { at, .. } if now >= at + WITHIN_MS => {
                self.phase = Phase::Given;
                self.ask_tag = None;
                vec![stop_post(), Effect::Log("camera pop-up: the live stream did not play in time; the stills go on".into())]
            }
            _ => Vec::new(),
        }
    }

    /// The picture box is a hole over the video plane: every condition of the module's header holds.
    pub fn video(&self, now: u64) -> bool {
        // a texture window is drawn by the compositor itself: there is no sink of videooutputd's to wait for
        matches!(self.phase, Phase::Playing { since, .. } if now >= since + HANDOFF_MS) && self.window == Window::Carried && (self.sink || self.slot == "texture")
    }

    /// When the model next needs a tick.
    pub fn due_at(&self, now: u64) -> Option<u64> {
        match self.phase {
            Phase::Retry { at, .. } => Some(at),
            Phase::Asked { at, .. } => Some(at + WITHIN_MS),
            Phase::Playing { since, .. } if now < since + HANDOFF_MS => Some(since + HANDOFF_MS),
            _ => None,
        }
    }

    /// For `ctl shell` and the TV check: where the live video stands.
    pub fn state(&self, now: u64) -> Value {
        let phase = match self.phase {
            Phase::Idle => "idle",
            Phase::Asked { .. } => "asked",
            Phase::Playing { .. } => "playing",
            Phase::Retry { .. } => "retry",
            Phase::Given => "given",
        };
        let window = match self.window {
            Window::Unclaimed => "unclaimed",
            Window::Waiting => "waiting",
            Window::Carried => "carried",
            Window::Refused => "refused",
        };
        json!({"phase": phase, "window": window, "slot": self.slot, "sink": self.sink, "video": self.video(now), "stream": self.stream.state, "fps": self.stream.fps, "fedFps": self.stream.fed_fps})
    }
}

fn stop_post() -> Effect {
    Effect::Post { path: "/v1/cameras/live/stop".into(), body: json!({}) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_800_000_000_000;
    const RECT: (i32, i32, i32, i32) = (1316, 200, 520, 292);

    fn card() -> Live {
        let mut l = Live::new();
        l.set_rect(Some(RECT));
        l.begin("275", Some(41));
        l
    }

    fn asks(fx: &[Effect]) -> Vec<(u64, Value)> {
        fx.iter()
            .filter_map(|e| match e {
                Effect::Request { tag, method, path, body } if method == "POST" && path == "/v1/cameras/live" => Some((*tag, body.clone().unwrap())),
                _ => None,
            })
            .collect()
    }

    fn stops(fx: &[Effect]) -> usize {
        fx.iter().filter(|e| matches!(e, Effect::Post { path, .. } if path == "/v1/cameras/live/stop")).count()
    }

    fn stream(state: &str) -> Value {
        json!({"camera": "275", "mode": "live", "state": state, "fps": 15, "fedFps": 14.8})
    }

    #[test]
    fn the_claim_is_the_picture_box_for_a_card_that_can_play_live_and_nobody_else() {
        let l = card();
        let c = l.claim(true, false).unwrap();
        assert_eq!(c["id"], "camera-popup");
        assert_eq!(c["kind"], "camera-popup");
        assert_eq!(c["rect"], json!({"x": 1316, "y": 200, "w": 520, "h": 292}));
        assert_eq!(c["need"], json!({"codec": "h264", "w": 1920, "h": 1088, "fps": 30}), "a stream either window takes");
        assert!(l.claim(false, false).is_none(), "a record that says no live video claims nothing");
        assert!(Live::new().claim(true, false).is_none(), "no card, no claim");
        let mut unmeasured = Live::new();
        unmeasured.begin("275", None);
        assert!(unmeasured.claim(true, false).is_none(), "the picture box is not known yet");
    }

    #[test]
    fn a_card_whose_camera_has_no_calm_stream_claims_a_texture_and_no_window() {
        let l = card();
        assert_eq!(l.claim(true, true).unwrap()["texture_only"], true);
        assert!(l.claim(true, false).unwrap().get("texture_only").is_none(), "a window claim is as it was");
        assert!(l.claim(false, true).is_none(), "and a card that cannot play live claims nothing either way");
    }

    #[test]
    fn the_stream_is_asked_for_only_once_a_window_is_granted_and_says_which() {
        let mut l = card();
        assert!(l.set_window(Window::Unclaimed, "", false, T0).is_empty(), "asked, not answered");
        assert!(l.set_window(Window::Refused, "", false, T0 + 10).is_empty(), "refused: nothing was started, nothing to stop");
        let fx = l.set_window(Window::Waiting, "sub", false, T0 + 20);
        let asked = asks(&fx);
        assert_eq!(asked.len(), 1, "{fx:?}");
        assert_eq!(asked[0].1, json!({"camera": "275", "for": "popup", "window": "sub", "popup": 41}));
        // the same grant read again does not ask twice
        assert!(asks(&l.set_window(Window::Carried, "sub", false, T0 + 30)).is_empty());
        let mut main = card();
        assert_eq!(asks(&main.set_window(Window::Carried, "main", true, T0))[0].1["window"], "main");
    }

    #[test]
    fn a_texture_grant_asks_for_the_low_stream_by_name_and_its_hole_needs_no_sink() {
        let mut l = card();
        let fx = l.set_window(Window::Waiting, "texture", false, T0);
        assert_eq!(asks(&fx)[0].1, json!({"camera": "275", "for": "popup", "window": "texture", "popup": 41}), "the helper picks the camera's low stream for it");
        l.topic(&stream("playing"), T0 + 400);
        l.set_window(Window::Carried, "texture", false, T0 + 500);
        assert!(!l.video(T0 + 400 + HANDOFF_MS - 1), "the handoff is counted from the stream's first picture");
        assert!(l.video(T0 + 400 + HANDOFF_MS), "the compositor paints it itself: placed is all there is to wait for");
        // the same stream under a hardware window with no sink is still no hole
        let mut h = card();
        h.set_window(Window::Waiting, "main", false, T0);
        h.topic(&stream("playing"), T0 + 400);
        h.set_window(Window::Carried, "main", false, T0 + 500);
        assert!(!h.video(T0 + 10_000));
    }

    #[test]
    fn a_claim_that_comes_back_with_another_kind_of_window_has_its_stream_asked_for_again() {
        let mut l = card();
        asks(&l.set_window(Window::Carried, "sub", true, T0));
        l.topic(&stream("playing"), T0 + 400);
        let fx = l.set_window(Window::Carried, "texture", false, T0 + 900);
        assert_eq!(asks(&fx)[0].1["window"], "texture", "the texture's stream replaces the sub window's");
        assert!(matches!(l.phase, Phase::Asked { .. }));
        assert!(asks(&l.set_window(Window::Carried, "texture", false, T0 + 1_000)).is_empty(), "once");
    }

    #[test]
    fn the_hole_opens_only_with_a_playing_stream_a_carried_claim_a_sink_and_the_handoff_passed() {
        let mut l = card();
        l.set_window(Window::Waiting, "main", false, T0);
        assert!(!l.video(T0 + 10_000), "asked, not playing");
        l.topic(&stream("starting"), T0 + 100);
        assert!(!l.video(T0 + 10_000));
        l.topic(&stream("playing"), T0 + 500);
        assert!(!l.video(T0 + 10_000), "the window is not drawing yet");
        l.set_window(Window::Carried, "main", false, T0 + 600);
        assert!(!l.video(T0 + 10_000), "no sink lists the video: a hole over a decoder that is not connected is black");
        l.set_window(Window::Carried, "main", true, T0 + 700);
        assert!(!l.video(T0 + 500 + HANDOFF_MS - 1), "the stills stay under the hole for the handoff");
        assert!(l.video(T0 + 500 + HANDOFF_MS));
        assert_eq!(l.due_at(T0 + 600), Some(T0 + 500 + HANDOFF_MS));
        assert_eq!(l.due_at(T0 + 10_000), None);
        // the window taken back: the stream is stopped and the hole is gone
        let fx = l.set_window(Window::Refused, "main", false, T0 + 5_000);
        assert_eq!(stops(&fx), 1);
        assert!(!l.video(T0 + 6_000));
    }

    #[test]
    fn a_stream_that_fails_is_asked_for_again_once_and_then_given_up() {
        let mut l = card();
        let tag = asks(&l.set_window(Window::Carried, "main", true, T0))[0].0;
        assert!(l.answered(tag, 200, &json!({})).is_empty());
        let fx = l.topic(&stream("failed"), T0 + 1_000);
        assert!(fx.is_empty() && !l.given());
        assert_eq!(l.due_at(T0 + 1_000), Some(T0 + 1_000 + RETRY_MS));
        assert!(l.tick(T0 + 1_000 + RETRY_MS - 1).is_empty(), "not yet");
        let again = asks(&l.tick(T0 + 1_000 + RETRY_MS));
        assert_eq!(again.len(), 1, "asked again");
        let fx = l.topic(&stream("failed"), T0 + 3_000);
        assert!(l.given() && stops(&fx) == 1, "the second failure ends it, and the helper is told");
        assert!(l.claim(true, false).is_none(), "and the window is let go");
    }

    #[test]
    fn a_refused_ask_a_stream_that_never_plays_and_another_camera_taking_the_session_give_it_up() {
        let mut l = card();
        let tag = asks(&l.set_window(Window::Carried, "main", true, T0))[0].0;
        let fx = l.answered(tag, 409, &json!({"error": {"code": "offline", "message": "The camera is offline"}}));
        assert!(l.given());
        assert!(matches!(&fx[0], Effect::Log(s) if s.contains("409") && !s.contains("offline")), "{fx:?}");

        let mut slow = card();
        slow.set_window(Window::Carried, "main", true, T0);
        assert!(slow.tick(T0 + WITHIN_MS - 1).is_empty());
        let fx = slow.tick(T0 + WITHIN_MS);
        assert!(slow.given() && stops(&fx) == 1, "{fx:?}");

        let mut taken = card();
        taken.set_window(Window::Carried, "main", true, T0);
        taken.topic(&stream("playing"), T0 + 400);
        let other = json!({"camera": "273", "mode": "live", "state": "starting"});
        let fx = taken.topic(&other, T0 + 900);
        assert!(taken.given() && stops(&fx) == 0, "the helper's one session is another's now: not the card's to stop");
    }

    #[test]
    fn ending_the_card_stops_what_it_asked_for_except_when_the_full_view_takes_it_over() {
        let mut l = card();
        l.set_window(Window::Carried, "main", true, T0);
        l.topic(&stream("playing"), T0 + 400);
        let fx = l.end(true);
        assert_eq!(stops(&fx), 1);
        assert!(l.claim(true, false).is_none(), "nothing is served any more");
        let mut over = card();
        over.set_window(Window::Carried, "main", true, T0);
        assert_eq!(stops(&over.end(false)), 0, "OK opens the full view of the same camera: its own ask finds the stream there");
        // a card that asked nothing has nothing to stop
        assert_eq!(stops(&card().end(true)), 0);
    }

    #[test]
    fn a_stream_of_the_card_s_camera_in_another_mode_is_not_its_stream() {
        let mut l = card();
        l.set_window(Window::Carried, "main", true, T0);
        l.topic(&json!({"camera": "275", "mode": "playback", "state": "playing"}), T0 + 300);
        assert!(l.given(), "a recording plays in the helper's one session now");
        assert!(!l.video(T0 + 10_000));
    }

    #[test]
    fn the_state_says_where_it_stands_for_the_tv_check() {
        let mut l = card();
        l.set_window(Window::Carried, "main", true, T0);
        l.topic(&stream("playing"), T0 + 400);
        let s = l.state(T0 + 800);
        assert_eq!(s["phase"], "playing");
        assert_eq!((s["window"].as_str(), s["sink"].as_bool(), s["video"].as_bool()), (Some("carried"), Some(true), Some(true)));
        assert_eq!(s["fedFps"].as_f64().map(|f| (f * 10.0).round() / 10.0), Some(14.8));
    }
}
