//! LumaTV's hardware video windows from the shell's side (the compositor's `--vwin idle|apps`, docs/research/hw-planes-result.md): the pictures the screens
//! want on a display video window (the Cameras app's full view, the camera pop-up, picture-in-picture surfaces), kept in step with the compositor's
//! `vwin` topic. Pure, as the screens' models are (`update(Input, now) -> Vec<Effect>`, `due_at()`): the host feeds it the topic and the ticks, asks
//! `want` each time a screen looks, and carries out the effects, which are posts to `/v1/vwin` and nothing else.
//!
//! The screens say all they want at once (`want(&[request, ...])`) and the model posts a claim once for each, a move when only the rectangle changed, and a
//! release for each that is no longer wanted, never the same request twice. The answer is the claim's own line of the topic: granted (with the window's
//! name and whether it is drawing yet), denied or taken back, each with the compositor's reason. The picture layer is a transparent hole either way, so a
//! screen draws the same whatever the grant is; what changes is whether the video behind the hole is the decoder's (granted) or the screen shows its
//! stills (anything else), which is what [`Vwin::grant`] tells it.
//!
//! A picture-in-picture surface is a [`Pip`]: where it sits, the stream it plays and the id it is known by; `Pip::request()` is the claim, and the
//! window it gets is whichever the display has spare (the compositor's `slots`; a camera wall of several is several `Pip`s with ids of their own).

use std::collections::BTreeMap;

use crate::kit::screen::{Effect, Input};
use serde_json::{Value, json};

/// The compositor's channel route for claims.
pub const PATH: &str = "/v1/vwin";
/// The topic the compositor publishes the claims and the windows on.
pub const TOPIC: &str = "vwin";
/// A claim the topic does not list is posted again after this long: the post is not answered, and the compositor may have restarted.
pub const MISSING_MS: u64 = 2_000;

/// What the compositor says about a claim the screens asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Grant {
    /// Nothing is asked for.
    #[default]
    None,
    /// Asked; the topic has not listed the claim yet.
    Asked,
    /// A window is the claim's: `slot` is `main`, `sub` (or `win2`, `win3` where the display has them), `placed` says its window is bound and drawing the hole.
    Granted { slot: String, placed: bool },
    /// Refused: `reason` is one of `off`, `app-video`, `app-front`, `both-scalers`, `no-window`, `too-big`, `standby`, and `text` says it in words.
    Denied { reason: String, text: String },
    /// Granted and then taken back: `higher` (a more important picture), `app-video`, `app-front`, `both-scalers`, `too-big`, `standby`, `no-window`.
    Preempted { reason: String, text: String },
}

impl Grant {
    /// Whether the hardware window carries the picture now.
    pub fn carries(&self) -> bool {
        matches!(self, Grant::Granted { placed: true, .. })
    }

    /// Whether the claim was refused or taken back, and why: the line a screen shows next to its stills.
    pub fn refusal(&self) -> Option<&str> {
        match self {
            Grant::Denied { text, .. } | Grant::Preempted { text, .. } => Some(text),
            _ => None,
        }
    }
}

/// A picture-in-picture surface: a live stream in a rectangle of the 1920x1080 output, over whatever the screen shows, on a spare video window when the
/// compositor has one to give and in stills when it has not (the screen draws both from [`Vwin::grant`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pip {
    pub id: String,
    pub rect: (i32, i32, i32, i32),
    /// `(codec, width, height, fps)` of the stream; without it only the claim's rectangle is checked.
    pub stream: Option<(String, u32, u32, u32)>,
    /// The app id of the video window that will play it (the feeder's `-a`); empty: the first window nobody has claimed.
    pub app: String,
}

/// The margin a corner picture keeps from the screen's edge (the overlay layout's, in output pixels).
pub const PIP_MARGIN: i32 = 48;

impl Pip {
    pub fn new(id: &str, rect: (i32, i32, i32, i32)) -> Self {
        Pip { id: id.into(), rect, stream: None, app: String::new() }
    }

    /// A 16:9 picture `w` wide in the bottom-right corner of the output.
    pub fn corner(id: &str, w: i32) -> Self {
        let h = w * 9 / 16;
        Self::new(id, (1920 - PIP_MARGIN - w, 1080 - PIP_MARGIN - h, w, h))
    }

    pub fn stream(mut self, codec: &str, w: u32, h: u32, fps: u32) -> Self {
        self.stream = Some((codec.into(), w, h, fps));
        self
    }

    pub fn app(mut self, app: &str) -> Self {
        self.app = app.into();
        self
    }

    /// The claim in the compositor's shape.
    pub fn request(&self) -> Value {
        let (x, y, w, h) = self.rect;
        let mut v = json!({"id": self.id, "kind": "pip", "rect": {"x": x, "y": y, "w": w, "h": h}});
        if let Some((codec, w, h, fps)) = &self.stream {
            v["need"] = json!({"codec": codec, "w": w, "h": h, "fps": fps});
        }
        if !self.app.is_empty() {
            v["app"] = json!(self.app);
        }
        v
    }
}

/// What the topic says about the world: a refused claim is asked again only when this changed.
#[derive(Clone, Debug, PartialEq, Eq)]
struct World {
    mode: String,
    apps: i64,
    /// Apps with a video playing, as against `apps`, which is what the mode counts (an idle element too, in `idle`).
    playing: i64,
    wall: bool,
    free: i64,
}

#[derive(Clone, Debug)]
struct Posted {
    request: Value,
    at_ms: u64,
    grant: Grant,
    /// The world at the time of the post (none before the first topic).
    world: Option<World>,
    /// The sink videooutputd lists the claim's video on (`MAIN`, `SUB0`): the proof that the decoder is connected and the picture can be the plane's.
    sink: Option<String>,
    /// Whether the compositor's line for the claim has a `sink` at all (it has since it reads the sinks; an older one's has not): without it a placed claim is
    /// all there is to go by.
    reports_sink: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Vwin {
    posted: BTreeMap<String, Posted>,
    world: Option<World>,
}

fn id_of(request: &Value) -> Option<&str> {
    request.get("id").and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn claim(request: &Value) -> Effect {
    let mut body = request.clone();
    body["op"] = json!("claim");
    Effect::Channel { path: PATH.into(), body }
}

fn release(id: &str) -> Effect {
    Effect::Channel { path: PATH.into(), body: json!({"op": "release", "id": id}) }
}

impl Vwin {
    pub fn new() -> Self {
        Self::default()
    }

    /// What the compositor made of claim `id` (`None` when it is not asked for).
    pub fn grant(&self, id: &str) -> &Grant {
        const NONE: Grant = Grant::None;
        self.posted.get(id).map_or(&NONE, |p| &p.grant)
    }

    /// The sink the claim's video is on, once the compositor has read it from videooutputd (`None` before, and for a claim that has no video).
    pub fn sink(&self, id: &str) -> Option<&str> {
        self.posted.get(id).and_then(|p| p.sink.as_deref())
    }

    /// Whether the compositor has video windows to give: its topic has come and the mode is not `off`.
    pub fn on(&self) -> bool {
        self.world.as_ref().is_some_and(|w| w.mode != "off")
    }

    /// Whether videooutputd lists the claim's video on a sink (`Some`), or does not yet (`Some(false)`); `None` when the compositor does not say.
    pub fn connected(&self, id: &str) -> Option<bool> {
        self.posted.get(id).filter(|p| p.reports_sink).map(|p| p.sink.is_some())
    }

    /// Whether an app has a video playing on the screen (a decoder attached), as the topic last said it. A compositor that does not say reads as none.
    pub fn app_playing(&self) -> bool {
        self.world.as_ref().is_some_and(|w| w.playing > 0)
    }

    /// The compositor's switch as the topic last said it; `off` until it has said.
    pub fn mode(&self) -> &str {
        self.world.as_ref().map_or("off", |w| w.mode.as_str())
    }

    /// What the shell reports of its claims (`ctl shell` state): the switch, the free windows and each claim's grant; `None` while nothing is asked for.
    pub fn state(&self) -> Option<Value> {
        if self.posted.is_empty() {
            return None;
        }
        let claims: serde_json::Map<String, Value> = self
            .posted
            .iter()
            .map(|(id, p)| {
                let line = match &p.grant {
                    Grant::None => json!({"state": "none"}),
                    Grant::Asked => json!({"state": "asked"}),
                    Grant::Granted { slot, placed } => {
                        let mut line = json!({"state": "granted", "slot": slot, "placed": placed});
                        if let Some(sink) = &p.sink {
                            line["sink"] = json!(sink);
                        }
                        line
                    }
                    Grant::Denied { reason, .. } => json!({"state": "denied", "reason": reason}),
                    Grant::Preempted { reason, .. } => json!({"state": "preempted", "reason": reason}),
                };
                (id.clone(), line)
            })
            .collect();
        Some(json!({"mode": self.mode(), "free": self.free(), "claims": claims}))
    }

    /// How many video windows the topic last said were free.
    pub fn free(&self) -> i64 {
        self.world.as_ref().map_or(0, |w| w.free)
    }

    /// Whether claim `id` is posted (whatever the compositor made of it).
    pub fn asked(&self, id: &str) -> bool {
        self.posted.contains_key(id)
    }

    fn post(&mut self, request: &Value, now: u64) -> Effect {
        let id = id_of(request).unwrap_or_default().to_string();
        self.posted.insert(id, Posted { request: request.clone(), at_ms: now, grant: Grant::Asked, world: self.world.clone(), sink: None, reports_sink: false });
        claim(request)
    }

    /// What the screens want now: the requests in the compositor's shape (`{"id", "kind": "camera-full"|"camera-popup"|"pip", "rect": {x,y,w,h},
    /// "need": {codec,w,h,fps}?, "app"?}`), every one of them, or none. A request without an id is not a request; a claim posted earlier and not in the list
    /// is released.
    pub fn want(&mut self, wanted: &[Value], now: u64) -> Vec<Effect> {
        let mut fx = Vec::new();
        let wanted: Vec<&Value> = wanted.iter().filter(|r| id_of(r).is_some()).collect();
        let gone: Vec<String> = self.posted.keys().filter(|id| !wanted.iter().any(|r| id_of(r) == Some(id.as_str()))).cloned().collect();
        for id in gone {
            self.posted.remove(&id);
            fx.push(release(&id));
        }
        for request in wanted {
            let id = id_of(request).unwrap_or_default().to_string();
            let Some(posted) = self.posted.get_mut(&id) else {
                fx.push(self.post(request, now));
                continue;
            };
            let (rect_moved, need_changed) = (posted.request.get("rect") != request.get("rect"), posted.request.get("need") != request.get("need"));
            let kind_changed = posted.request.get("texture_only") != request.get("texture_only");
            if !rect_moved && !need_changed && !kind_changed {
                continue;
            }
            if kind_changed || !matches!(posted.grant, Grant::Granted { .. }) {
                // a claim that was refused or is not answered yet is asked afresh: another rectangle or stream may fit where this one did not; one that
                // turns into a texture claim (or out of one) is another question, which a move does not ask
                fx.push(self.post(request, now));
                continue;
            }
            let mut body = json!({"op": "move", "id": id, "rect": request["rect"]});
            if need_changed && let Some(need) = request.get("need") {
                body["need"] = need.clone();
            }
            posted.request = request.clone();
            fx.push(Effect::Channel { path: PATH.into(), body });
        }
        fx
    }

    /// When the model next needs a tick: the claims it has posted and not seen listed are asked again then.
    pub fn due_at(&self) -> Option<u64> {
        self.posted.values().filter(|p| p.grant == Grant::Asked).map(|p| p.at_ms + MISSING_MS).min()
    }

    pub fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        match input {
            Input::Topic { topic, data } if topic == TOPIC => self.topic(data, now),
            Input::Tick => self.tick(now),
            _ => Vec::new(),
        }
    }

    fn tick(&mut self, now: u64) -> Vec<Effect> {
        let due: Vec<Value> = self.posted.values().filter(|p| p.grant == Grant::Asked && now >= p.at_ms + MISSING_MS).map(|p| p.request.clone()).collect();
        due.iter().map(|r| self.post(r, now)).collect()
    }

    fn topic(&mut self, data: &Value, now: u64) -> Vec<Effect> {
        let world = World {
            mode: data.get("mode").and_then(Value::as_str).unwrap_or("off").to_string(),
            apps: data.get("apps").and_then(Value::as_i64).unwrap_or(0),
            playing: data.get("appsPlaying").and_then(Value::as_i64).unwrap_or(0),
            wall: data.get("wall").and_then(Value::as_bool).unwrap_or(false),
            free: data.get("free").and_then(Value::as_i64).unwrap_or(0),
        };
        self.world = Some(world.clone());
        let lines = data.get("claims").and_then(Value::as_array);
        let mut again = Vec::new();
        for (id, posted) in self.posted.iter_mut() {
            // the first topic after the post is the world the question was asked in
            let asked_in = posted.world.get_or_insert_with(|| world.clone()).clone();
            // not listed (yet): the tick asks again when the wait is over
            let Some(line) = lines.and_then(|l| l.iter().find(|c| c.get("id").and_then(Value::as_str) == Some(id.as_str()))) else { continue };
            let word = |k: &str| line.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            posted.grant = match line.get("state").and_then(Value::as_str) {
                Some("granted") => Grant::Granted { slot: word("slot"), placed: line.get("placed").and_then(Value::as_bool).unwrap_or(false) },
                Some("denied") => Grant::Denied { reason: word("reason"), text: word("text") },
                Some("preempted") => Grant::Preempted { reason: word("reason"), text: word("text") },
                _ => Grant::Asked,
            };
            posted.sink = line.get("sink").and_then(Value::as_str).map(str::to_string);
            posted.reports_sink = line.get("sink").is_some();
            // A refused claim is asked again when the world changed since it was asked (the switch turned on, an app's video ended, a window freed),
            // and not before: the same answer to the same question is not worth a post.
            let refused = matches!(posted.grant, Grant::Denied { .. } | Grant::Preempted { .. });
            if refused && asked_in != world {
                again.push(posted.request.clone());
            }
        }
        again.iter().map(|r| self.post(r, now)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> Value {
        json!({"id": "cameras-full", "kind": "camera-full", "rect": {"x": 0, "y": 0, "w": 1920, "h": 1080}, "need": {"codec": "h264", "w": 1920, "h": 1080, "fps": 30}})
    }

    fn topic(mode: &str, free: i64, apps: i64, claims: Value) -> Input {
        Input::Topic { topic: TOPIC.into(), data: json!({"mode": mode, "slots": 2, "free": free, "apps": apps, "wall": false, "claims": claims}) }
    }

    fn line(state: &str, reason: &str, text: &str, placed: bool) -> Value {
        json!({"id": "cameras-full", "kind": "camera-full", "state": state, "reason": reason, "text": text, "slot": "main", "placed": placed})
    }

    fn posts(effects: &[Effect]) -> Vec<Value> {
        effects
            .iter()
            .map(|e| match e {
                Effect::Channel { path, body } => {
                    assert_eq!(path, PATH);
                    body.clone()
                }
                other => panic!("only channel posts: {other:?}"),
            })
            .collect()
    }

    fn ops(effects: &[Effect]) -> Vec<String> {
        posts(effects).iter().map(|b| b["op"].as_str().unwrap_or("").to_string()).collect()
    }

    #[test]
    fn a_wanted_picture_is_posted_once_and_its_grant_is_read_from_the_topic() {
        let mut v = Vwin::new();
        assert_eq!((v.grant("cameras-full"), v.asked("cameras-full"), v.mode()), (&Grant::None, false, "off"));
        let first = v.want(&[full()], 1_000);
        let body = posts(&first).remove(0);
        assert_eq!((body["op"].as_str(), body["id"].as_str(), body["kind"].as_str()), (Some("claim"), Some("cameras-full"), Some("camera-full")));
        assert_eq!(body["rect"], full()["rect"], "the request goes as the screen made it");
        assert_eq!(body["need"], full()["need"]);
        assert_eq!(v.grant("cameras-full"), &Grant::Asked);
        assert!(v.want(&[full()], 1_100).is_empty(), "the same wish is not posted again");
        assert_eq!(v.due_at(), Some(1_000 + MISSING_MS));

        // granted, its window not drawing yet; then drawing
        assert!(v.update(&topic("apps", 1, 0, json!([line("granted", "", "", false)])), 1_200).is_empty());
        assert_eq!(v.grant("cameras-full"), &Grant::Granted { slot: "main".into(), placed: false });
        assert!(!v.grant("cameras-full").carries());
        v.update(&topic("apps", 1, 0, json!([line("granted", "", "", true)])), 1_300);
        assert!(v.grant("cameras-full").carries());
        assert_eq!((v.mode(), v.due_at()), ("apps", None));
        assert!(v.want(&[full()], 1_400).is_empty());
    }

    #[test]
    fn a_picture_that_only_moved_is_a_move_and_one_that_was_refused_is_asked_afresh() {
        let mut v = Vwin::new();
        v.want(&[full()], 0);
        v.update(&topic("apps", 1, 0, json!([line("granted", "", "", true)])), 10);
        let mut moved = full();
        moved["rect"] = json!({"x": 100, "y": 50, "w": 1600, "h": 900});
        let sent = posts(&v.want(&[moved.clone()], 20)).remove(0);
        assert_eq!((sent["op"].as_str(), sent["id"].as_str()), (Some("move"), Some("cameras-full")));
        assert_eq!(sent["rect"], moved["rect"]);
        assert!(sent.get("need").is_none(), "the stream did not change");
        assert!(v.want(&[moved.clone()], 30).is_empty());
        // the stream changed too: the move carries it
        moved["need"] = json!({"codec": "h265", "w": 3840, "h": 2160, "fps": 30});
        let sent = posts(&v.want(&[moved.clone()], 40)).remove(0);
        assert_eq!((sent["op"].as_str(), sent["need"].clone()), (Some("move"), moved["need"].clone()));

        // refused: a changed rectangle is a new claim, since another one may fit
        let mut v = Vwin::new();
        v.want(&[full()], 0);
        v.update(&topic("apps", 0, 1, json!([line("denied", "app-video", "an app has a video on the screen", false)])), 10);
        assert_eq!(v.grant("cameras-full"), &Grant::Denied { reason: "app-video".into(), text: "an app has a video on the screen".into() });
        assert_eq!(v.grant("cameras-full").refusal(), Some("an app has a video on the screen"));
        assert_eq!(ops(&v.want(&[moved.clone()], 20)), vec!["claim"]);
    }

    #[test]
    fn a_claim_that_turns_into_a_texture_claim_or_back_is_asked_afresh_even_when_granted() {
        let mut v = Vwin::new();
        let mut want = json!({"id": "camera-popup", "kind": "camera-popup", "rect": {"x": 1316, "y": 200, "w": 520, "h": 292}});
        v.want(&[want.clone()], 0);
        v.update(&topic("idle", 1, 0, json!([json!({"id": "camera-popup", "kind": "camera-popup", "state": "granted", "reason": "", "text": "", "slot": "sub", "placed": true})])), 10);
        assert!(matches!(v.grant("camera-popup"), Grant::Granted { .. }));
        want["texture_only"] = json!(true);
        let sent = posts(&v.want(&[want.clone()], 20)).remove(0);
        assert_eq!((sent["op"].as_str(), sent["texture_only"].clone()), (Some("claim"), json!(true)), "a move would keep the window");
        assert!(v.want(&[want.clone()], 30).is_empty());
        want.as_object_mut().unwrap().remove("texture_only");
        assert_eq!(ops(&v.want(&[want], 40)), vec!["claim"]);
    }

    #[test]
    fn nothing_wanted_releases_what_was_posted_once_and_a_different_picture_releases_the_old_one_first() {
        let mut v = Vwin::new();
        assert!(v.want(&[], 0).is_empty(), "nothing asked, nothing to release");
        v.want(&[full()], 0);
        let sent = posts(&v.want(&[], 10));
        assert_eq!((sent[0]["op"].as_str(), sent[0]["id"].as_str()), (Some("release"), Some("cameras-full")));
        assert_eq!((v.grant("cameras-full"), v.asked("cameras-full")), (&Grant::None, false));
        assert!(v.want(&[], 20).is_empty(), "released once");

        let mut popup = full();
        popup["id"] = json!("camera-popup");
        popup["kind"] = json!("camera-popup");
        v.want(&[full()], 30);
        let sent = v.want(&[popup.clone()], 40);
        assert_eq!(ops(&sent), vec!["release", "claim"]);
        assert_eq!(posts(&sent)[0]["id"], "cameras-full");
        assert_eq!(posts(&sent)[1]["id"], "camera-popup");
        // a request without an id is not one
        assert_eq!(ops(&v.want(&[json!({"kind": "pip"})], 50)), vec!["release"]);
    }

    #[test]
    fn a_refused_claim_is_asked_again_only_when_the_world_changed() {
        let mut v = Vwin::new();
        v.want(&[full()], 0);
        let denied = || json!([line("denied", "off", "hardware video windows are off", false)]);
        // the first topic after the post is the world the question was asked in: no second post for the same answer
        assert!(v.update(&topic("off", 2, 0, denied()), 10).is_empty());
        assert!(v.update(&topic("off", 2, 0, denied()), 20).is_empty());
        // the switch is turned on: ask again
        let sent = v.update(&topic("apps", 2, 0, denied()), 30);
        assert_eq!(ops(&sent), vec!["claim"]);
        assert_eq!(v.grant("cameras-full"), &Grant::Asked);
        // it is granted; an app starts a video and takes the window back: refused, the world changed, so it is asked and refused again
        v.update(&topic("apps", 2, 0, json!([line("granted", "", "", true)])), 40);
        let taken = json!([line("preempted", "app-video", "an app has a video on the screen", false)]);
        let sent = v.update(&topic("idle", 0, 1, taken.clone()), 50);
        assert_eq!(ops(&sent), vec!["claim"]);
        assert!(v.update(&topic("idle", 0, 1, taken), 60).is_empty(), "the same answer to the same question");
    }

    #[test]
    fn a_claim_the_topic_does_not_list_is_posted_again_after_a_wait_and_not_before() {
        let mut v = Vwin::new();
        v.want(&[full()], 5_000);
        // topics of other things arrive: not ours, nothing to read
        assert!(v.update(&topic("apps", 2, 0, json!([])), 5_500).is_empty());
        assert_eq!(v.grant("cameras-full"), &Grant::Asked);
        assert!(v.update(&Input::Tick, 6_999).is_empty());
        let sent = v.update(&Input::Tick, 7_000);
        assert_eq!(ops(&sent), vec!["claim"]);
        assert_eq!(v.due_at(), Some(7_000 + MISSING_MS), "and again after another wait");
        // listed now: no more asking
        v.update(&topic("apps", 1, 0, json!([line("granted", "", "", false)])), 7_100);
        assert!(v.update(&Input::Tick, 20_000).is_empty());
        assert_eq!(v.due_at(), None);
    }

    #[test]
    fn other_topics_inputs_and_a_topic_nobody_asked_about_change_nothing() {
        let mut v = Vwin::new();
        assert!(v.update(&Input::Topic { topic: "audio".into(), data: json!({"volume": 3}) }, 0).is_empty());
        assert!(v.update(&topic("apps", 2, 0, json!([line("granted", "", "", true)])), 10).is_empty(), "no claim was posted: the line is somebody else's");
        assert_eq!((v.grant("cameras-full"), v.mode()), (&Grant::None, "apps"));
        assert!(v.update(&Input::Tick, 20).is_empty());
        assert_eq!(v.due_at(), None);
    }

    fn pip_line(id: &str, state: &str, reason: &str, slot: &str, placed: bool) -> Value {
        json!({"id": id, "kind": "pip", "state": state, "reason": reason, "text": reason, "slot": slot, "placed": placed})
    }

    #[test]
    fn a_picture_in_picture_is_a_claim_of_its_own_with_the_stream_and_the_corner_it_names() {
        let pip = Pip::corner("pip-door", 480).stream("h264", 1280, 720, 15).app("org.nphil.lumatv.cameras");
        assert_eq!(pip.rect, (1392, 762, 480, 270), "16:9, bottom right, 48 px off both edges");
        let r = pip.request();
        assert_eq!((r["kind"].as_str(), r["id"].as_str(), r["app"].as_str()), (Some("pip"), Some("pip-door"), Some("org.nphil.lumatv.cameras")));
        assert_eq!(r["rect"], json!({"x": 1392, "y": 762, "w": 480, "h": 270}));
        assert_eq!(r["need"], json!({"codec": "h264", "w": 1280, "h": 720, "fps": 15}));
        let plain = Pip::new("p", (0, 0, 320, 180)).request();
        assert!(plain.get("need").is_none() && plain.get("app").is_none(), "what is not named is not sent");
    }

    #[test]
    fn several_pictures_are_asked_and_answered_each_by_its_own_id() {
        let (door, yard) = (Pip::new("pip-door", (0, 0, 640, 360)).request(), Pip::new("pip-yard", (640, 0, 640, 360)).request());
        let mut v = Vwin::new();
        let sent = posts(&v.want(&[door.clone(), yard.clone()], 0));
        assert_eq!(sent.iter().map(|b| b["id"].as_str().unwrap()).collect::<Vec<_>>(), vec!["pip-door", "pip-yard"]);
        assert!(v.want(&[door.clone(), yard.clone()], 10).is_empty(), "each is posted once");
        // the compositor gave the first the sub window and had no second one
        let lines = json!([pip_line("pip-door", "granted", "", "sub", true), pip_line("pip-yard", "denied", "no-window", "", false)]);
        v.update(&topic("apps", 0, 1, lines), 20);
        assert_eq!(v.grant("pip-door"), &Grant::Granted { slot: "sub".into(), placed: true });
        assert!(v.grant("pip-door").carries() && !v.grant("pip-yard").carries());
        assert_eq!(v.grant("pip-yard").refusal(), Some("no-window"));
        assert_eq!(v.free(), 0);
        // the screen drops the first: only that one is released, and the free window is the second one's to ask for again
        let sent = posts(&v.want(std::slice::from_ref(&yard), 30));
        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0]["op"].as_str(), sent[0]["id"].as_str()), (Some("release"), Some("pip-door")));
        assert!(!v.asked("pip-door") && v.asked("pip-yard"));
        let freed = json!([pip_line("pip-yard", "denied", "no-window", "", false)]);
        assert_eq!(ops(&v.update(&topic("apps", 1, 1, freed), 40)), vec!["claim"], "a window freed: the refused picture asks again");
        // one moved, the other not: one move
        let mut moved = yard.clone();
        moved["rect"] = json!({"x": 10, "y": 10, "w": 640, "h": 360});
        v.update(&topic("apps", 0, 1, json!([pip_line("pip-yard", "granted", "", "sub", true)])), 50);
        assert_eq!(ops(&v.want(&[moved], 60)), vec!["move"]);
    }

    #[test]
    fn the_tick_asks_again_only_for_the_claims_that_were_not_answered() {
        let (a, b) = (Pip::new("a", (0, 0, 320, 180)).request(), Pip::new("b", (320, 0, 320, 180)).request());
        let mut v = Vwin::new();
        v.want(&[a.clone(), b.clone()], 1_000);
        v.update(&topic("apps", 1, 0, json!([pip_line("a", "granted", "", "sub", false)])), 1_100);
        assert_eq!(v.due_at(), Some(1_000 + MISSING_MS), "b is still unanswered");
        let sent = posts(&v.update(&Input::Tick, 3_000));
        assert_eq!(sent.iter().map(|x| x["id"].as_str().unwrap()).collect::<Vec<_>>(), vec!["b"]);
    }

    #[test]
    fn the_state_names_each_claim_s_grant_and_is_absent_while_nothing_is_asked() {
        let mut v = Vwin::new();
        assert_eq!(v.state(), None);
        let door = Pip::new("pip-door", (0, 0, 640, 360)).request();
        v.want(std::slice::from_ref(&door), 0);
        assert_eq!(v.state().unwrap()["claims"]["pip-door"], json!({"state": "asked"}));
        v.update(&topic("apps", 1, 1, json!([pip_line("pip-door", "granted", "", "sub", true)])), 10);
        assert_eq!(v.state().unwrap(), json!({"mode": "apps", "free": 1, "claims": {"pip-door": {"state": "granted", "slot": "sub", "placed": true}}}));
        v.update(&topic("apps", 1, 1, json!([pip_line("pip-door", "preempted", "higher", "", false)])), 20);
        assert_eq!(v.state().unwrap()["claims"]["pip-door"], json!({"state": "preempted", "reason": "higher"}));
    }

    #[test]
    fn an_app_playing_a_video_is_read_from_the_topic_and_an_older_compositor_says_none() {
        let mut v = Vwin::new();
        assert!(!v.app_playing(), "no topic yet");
        v.update(&topic("apps", 1, 1, json!([])), 0);
        assert!(!v.app_playing(), "an idle element counts for the mode and plays nothing");
        let playing = Input::Topic { topic: TOPIC.into(), data: json!({"mode": "apps", "slots": 2, "free": 1, "apps": 1, "appsPlaying": 1, "wall": false, "claims": []}) };
        v.update(&playing, 10);
        assert!(v.app_playing());
    }
}
