//! LumaTV's hardware video windows from the shell's side (the compositor's `--vwin idle|apps`, docs/research/hw-planes-result.md): the one picture
//! the screens want on a display video window (the Cameras app's full view; later the camera pop-up and a picture-in-picture surface), kept in step
//! with the compositor's `vwin` topic. Pure, as the screens' models are (`update(Input, now) -> Vec<Effect>`, `due_at()`): the host feeds it the topic
//! and the ticks, asks `want` each time a screen looks, and carries out the effects, which are posts to `/v1/vwin` and nothing else.
//!
//! A screen says what it wants (`want(Some(request))`) and the model posts a claim once, a move when only the rectangle changed, and a release when
//! nothing is wanted any more, never the same request twice. The answer is the claim's own line of the topic: granted (with the window's name and
//! whether it is drawing yet), denied or taken back, each with the compositor's reason. The picture layer is a transparent hole either way, so a
//! screen draws the same whatever the grant is; what changes is whether the video behind the hole is the decoder's (granted) or the screen shows its
//! stills (anything else), which is what [`Vwin::grant`] tells it.

use crate::kit::screen::{Effect, Input};
use serde_json::{Value, json};

/// The compositor's channel route for claims.
pub const PATH: &str = "/v1/vwin";
/// The topic the compositor publishes the claims and the windows on.
pub const TOPIC: &str = "vwin";
/// A claim the topic does not list is posted again after this long: the post is not answered, and the compositor may have restarted.
pub const MISSING_MS: u64 = 2_000;

/// What the compositor says about the claim the screens asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Grant {
    /// Nothing is asked for.
    #[default]
    None,
    /// Asked; the topic has not listed the claim yet.
    Asked,
    /// A window is the claim's: `slot` is `main` or `sub`, `placed` says its window is bound and drawing the hole.
    Granted { slot: String, placed: bool },
    /// Refused: `reason` is one of `off`, `app-video`, `both-scalers`, `no-window`, `too-big`, `standby`, and `text` says it in words.
    Denied { reason: String, text: String },
    /// Granted and then taken back: `higher` (a more important picture), `app-video`, `both-scalers`, `too-big`, `standby`, `no-window`.
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

/// What the topic says about the world: a refused claim is asked again only when this changed.
#[derive(Clone, Debug, PartialEq, Eq)]
struct World {
    mode: String,
    apps: i64,
    wall: bool,
    free: i64,
}

#[derive(Clone, Debug)]
struct Posted {
    request: Value,
    at_ms: u64,
    /// The world at the time of the post (none before the first topic).
    world: Option<World>,
}

#[derive(Clone, Debug, Default)]
pub struct Vwin {
    posted: Option<Posted>,
    grant: Grant,
    world: Option<World>,
}

impl Vwin {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn grant(&self) -> &Grant {
        &self.grant
    }

    /// The compositor's switch as the topic last said it; `off` until it has said.
    pub fn mode(&self) -> &str {
        self.world.as_ref().map_or("off", |w| w.mode.as_str())
    }

    /// Whether a claim is posted (whatever the compositor made of it).
    pub fn asked(&self) -> bool {
        self.posted.is_some()
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

    fn post(&mut self, request: &Value, now: u64) -> Effect {
        self.grant = Grant::Asked;
        self.posted = Some(Posted { request: request.clone(), at_ms: now, world: self.world.clone() });
        Self::claim(request)
    }

    /// What the screens want now: `Some(request)` in the compositor's shape (`{"id", "kind": "camera-full"|"camera-popup"|"pip", "rect": {x,y,w,h},
    /// "need": {codec,w,h,fps}?, "app"?}`), or `None` for nothing. A request without an id is not a request.
    pub fn want(&mut self, wanted: Option<&Value>, now: u64) -> Vec<Effect> {
        let Some(request) = wanted.filter(|r| Self::id_of(r).is_some()) else {
            return match self.posted.take() {
                Some(p) => {
                    self.grant = Grant::None;
                    vec![Self::release(Self::id_of(&p.request).unwrap_or_default())]
                }
                None => Vec::new(),
            };
        };
        let id = Self::id_of(request).unwrap_or_default().to_string();
        let Some(posted) = &self.posted else { return vec![self.post(request, now)] };
        let old = Self::id_of(&posted.request).unwrap_or_default().to_string();
        if old != id {
            let release = Self::release(&old);
            return vec![release, self.post(request, now)];
        }
        let (rect_moved, need_changed) = (posted.request.get("rect") != request.get("rect"), posted.request.get("need") != request.get("need"));
        if !rect_moved && !need_changed {
            return Vec::new();
        }
        if !matches!(self.grant, Grant::Granted { .. }) {
            // a claim that was refused or is not answered yet is asked afresh: another rectangle or stream may fit where this one did not
            return vec![self.post(request, now)];
        }
        let mut body = json!({"op": "move", "id": id, "rect": request["rect"]});
        if need_changed && let Some(need) = request.get("need") {
            body["need"] = need.clone();
        }
        if let Some(p) = &mut self.posted {
            p.request = request.clone();
        }
        vec![Effect::Channel { path: PATH.into(), body }]
    }

    /// When the model next needs a tick: the claim it has posted and not seen listed is asked again then.
    pub fn due_at(&self) -> Option<u64> {
        match (&self.grant, &self.posted) {
            (Grant::Asked, Some(p)) => Some(p.at_ms + MISSING_MS),
            _ => None,
        }
    }

    pub fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        match input {
            Input::Topic { topic, data } if topic == TOPIC => self.topic(data, now),
            Input::Tick => self.tick(now),
            _ => Vec::new(),
        }
    }

    fn tick(&mut self, now: u64) -> Vec<Effect> {
        match (&self.grant, &self.posted) {
            (Grant::Asked, Some(p)) if now >= p.at_ms + MISSING_MS => {
                let request = p.request.clone();
                vec![self.post(&request, now)]
            }
            _ => Vec::new(),
        }
    }

    fn topic(&mut self, data: &Value, now: u64) -> Vec<Effect> {
        let world = World {
            mode: data.get("mode").and_then(Value::as_str).unwrap_or("off").to_string(),
            apps: data.get("apps").and_then(Value::as_i64).unwrap_or(0),
            wall: data.get("wall").and_then(Value::as_bool).unwrap_or(false),
            free: data.get("free").and_then(Value::as_i64).unwrap_or(0),
        };
        self.world = Some(world.clone());
        let Some(posted) = &mut self.posted else { return Vec::new() };
        // the first topic after the post is the world the question was asked in
        let asked_in = posted.world.get_or_insert_with(|| world.clone()).clone();
        let id = Self::id_of(&posted.request).unwrap_or_default();
        let mine = data.get("claims").and_then(Value::as_array).and_then(|l| l.iter().find(|c| c.get("id").and_then(Value::as_str) == Some(id)));
        let Some(line) = mine else {
            // not listed (yet): the tick asks again when the wait is over
            return Vec::new();
        };
        let word = |k: &str| line.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        self.grant = match line.get("state").and_then(Value::as_str) {
            Some("granted") => Grant::Granted { slot: word("slot"), placed: line.get("placed").and_then(Value::as_bool).unwrap_or(false) },
            Some("denied") => Grant::Denied { reason: word("reason"), text: word("text") },
            Some("preempted") => Grant::Preempted { reason: word("reason"), text: word("text") },
            _ => Grant::Asked,
        };
        // A refused claim is asked again when the world changed since it was asked (the switch turned on, an app's video ended, a window freed), and
        // not before: the same answer to the same question is not worth a post.
        let refused = matches!(self.grant, Grant::Denied { .. } | Grant::Preempted { .. });
        if refused && asked_in != world {
            let request = posted.request.clone();
            return vec![self.post(&request, now)];
        }
        Vec::new()
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
        assert_eq!((v.grant(), v.asked(), v.mode()), (&Grant::None, false, "off"));
        let first = v.want(Some(&full()), 1_000);
        let body = posts(&first).remove(0);
        assert_eq!((body["op"].as_str(), body["id"].as_str(), body["kind"].as_str()), (Some("claim"), Some("cameras-full"), Some("camera-full")));
        assert_eq!(body["rect"], full()["rect"], "the request goes as the screen made it");
        assert_eq!(body["need"], full()["need"]);
        assert_eq!(v.grant(), &Grant::Asked);
        assert!(v.want(Some(&full()), 1_100).is_empty(), "the same wish is not posted again");
        assert_eq!(v.due_at(), Some(1_000 + MISSING_MS));

        // granted, its window not drawing yet; then drawing
        assert!(v.update(&topic("apps", 1, 0, json!([line("granted", "", "", false)])), 1_200).is_empty());
        assert_eq!(v.grant(), &Grant::Granted { slot: "main".into(), placed: false });
        assert!(!v.grant().carries());
        v.update(&topic("apps", 1, 0, json!([line("granted", "", "", true)])), 1_300);
        assert!(v.grant().carries());
        assert_eq!((v.mode(), v.due_at()), ("apps", None));
        assert!(v.want(Some(&full()), 1_400).is_empty());
    }

    #[test]
    fn a_picture_that_only_moved_is_a_move_and_one_that_was_refused_is_asked_afresh() {
        let mut v = Vwin::new();
        v.want(Some(&full()), 0);
        v.update(&topic("apps", 1, 0, json!([line("granted", "", "", true)])), 10);
        let mut moved = full();
        moved["rect"] = json!({"x": 100, "y": 50, "w": 1600, "h": 900});
        let sent = posts(&v.want(Some(&moved), 20)).remove(0);
        assert_eq!((sent["op"].as_str(), sent["id"].as_str()), (Some("move"), Some("cameras-full")));
        assert_eq!(sent["rect"], moved["rect"]);
        assert!(sent.get("need").is_none(), "the stream did not change");
        assert!(v.want(Some(&moved), 30).is_empty());
        // the stream changed too: the move carries it
        moved["need"] = json!({"codec": "h265", "w": 3840, "h": 2160, "fps": 30});
        let sent = posts(&v.want(Some(&moved), 40)).remove(0);
        assert_eq!((sent["op"].as_str(), sent["need"].clone()), (Some("move"), moved["need"].clone()));

        // refused: a changed rectangle is a new claim, since another one may fit
        let mut v = Vwin::new();
        v.want(Some(&full()), 0);
        v.update(&topic("apps", 0, 1, json!([line("denied", "app-video", "an app has a video on the screen", false)])), 10);
        assert_eq!(v.grant(), &Grant::Denied { reason: "app-video".into(), text: "an app has a video on the screen".into() });
        assert_eq!(v.grant().refusal(), Some("an app has a video on the screen"));
        assert_eq!(ops(&v.want(Some(&moved), 20)), vec!["claim"]);
    }

    #[test]
    fn nothing_wanted_releases_what_was_posted_once_and_a_different_picture_releases_the_old_one_first() {
        let mut v = Vwin::new();
        assert!(v.want(None, 0).is_empty(), "nothing asked, nothing to release");
        v.want(Some(&full()), 0);
        let sent = posts(&v.want(None, 10));
        assert_eq!((sent[0]["op"].as_str(), sent[0]["id"].as_str()), (Some("release"), Some("cameras-full")));
        assert_eq!((v.grant(), v.asked()), (&Grant::None, false));
        assert!(v.want(None, 20).is_empty(), "released once");

        let mut popup = full();
        popup["id"] = json!("camera-popup");
        popup["kind"] = json!("camera-popup");
        v.want(Some(&full()), 30);
        let sent = v.want(Some(&popup), 40);
        assert_eq!(ops(&sent), vec!["release", "claim"]);
        assert_eq!(posts(&sent)[0]["id"], "cameras-full");
        assert_eq!(posts(&sent)[1]["id"], "camera-popup");
        // a request without an id is not one
        assert_eq!(ops(&v.want(Some(&json!({"kind": "pip"})), 50)), vec!["release"]);
    }

    #[test]
    fn a_refused_claim_is_asked_again_only_when_the_world_changed() {
        let mut v = Vwin::new();
        v.want(Some(&full()), 0);
        let denied = || json!([line("denied", "off", "hardware video windows are off", false)]);
        // the first topic after the post is the world the question was asked in: no second post for the same answer
        assert!(v.update(&topic("off", 2, 0, denied()), 10).is_empty());
        assert!(v.update(&topic("off", 2, 0, denied()), 20).is_empty());
        // the switch is turned on: ask again
        let sent = v.update(&topic("apps", 2, 0, denied()), 30);
        assert_eq!(ops(&sent), vec!["claim"]);
        assert_eq!(v.grant(), &Grant::Asked);
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
        v.want(Some(&full()), 5_000);
        // topics of other things arrive: not ours, nothing to read
        assert!(v.update(&topic("apps", 2, 0, json!([])), 5_500).is_empty());
        assert_eq!(v.grant(), &Grant::Asked);
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
        assert_eq!((v.grant(), v.mode()), (&Grant::None, "apps"));
        assert!(v.update(&Input::Tick, 20).is_empty());
        assert_eq!(v.due_at(), None);
    }
}
