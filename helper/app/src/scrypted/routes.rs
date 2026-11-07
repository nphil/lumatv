//! The loopback API under `/v1/cameras` (design 3.4): what the shell asks of the cameras. Answers are JSON in the usual envelope
//! (`ok`, `api`, `now`); a picture is a BMP (or the JPEG as it is where no decoder is); times are epoch milliseconds, the bridge's own.
//!
//! | Request | Answer |
//! |---|---|
//! | `GET /v1/cameras` | the `cameras` topic's value |
//! | `GET /v1/cameras/{id}/frame?w=&h=&fps=` | the camera's newest still, never older than 10 s (else `404 no_picture`, or `503 not_connected`); `X-Luma-Camera-Age-Ms`; `fps` (at most 10) is the pace the asker shows the camera at, which its stream keeps while it is the fastest asked (none: the grid's pace) |
//! | `GET /v1/cameras/{id}/recordings?from=&to=`, `/events?from=&to=&count=`, `/clips?from=&to=&count=` | the bridge's own JSON for the span |
//! | `GET /v1/cameras/{id}/thumbnail?at=&w=&h=`, `/clip-thumbnail?clip=&w=&h=` | a picture of a recorded moment or a clip |
//! | `GET /v1/cameras/events/{event}/thumbnail?w=&h=` | the picture of a pushed event |
//! | `POST /v1/cameras/live {camera, quality?, for?, popup?, window?}` | starts live; the state is the `cameras.live` topic; a new camera ends the old session. `quality` is `native` (the camera's own stream), `balanced` (`medium`) or `light` (`low`). `for: "popup"` is a camera pop-up's picture, always the camera's low stream whatever `quality` says (`popup_stream.rs`: inside 1280x720 at up to 30, else inside 1920x1088 at up to 20, the streams the panel keeps 120 Hz with; none: `501 no_calm_stream`, and the shell draws the pop-up as a texture instead), in the feeder's `cameras.popupWindow` window, the decoder opened for Home (`cameras.ndlApp`); `window: "sub"` (the compositor granted the sub window beside an app's video) loads it in the starfish pipeline asking for the sub sink; `window: "texture"` is the CPU path, the low stream (up to 720p at 30, H.264); `popup` is the card's id, for the log. `for: "full"` is the Cameras full view on a video window (`window` `main` by default): the camera's own stream, up to 4K at its own rate; with `window: "sub"` (the view beside an app's video, which keeps the main window) the best stream inside the sub window's limits, 1920x1088 at 30, as are the recordings and clips asked with the same `window` A camera on a battery (`battery` in the camera's entry) is dialled only for the full view the owner opened (`open: true`; no `for` or `for: "full"`): any other call is `409 battery_still_only`, and the shell shows its stills. Its call is waited for up to 45 s, nothing is loaded before its picture's description has come, and `live/stop` hangs the call up at once. |
//! | `POST /v1/cameras/playback {camera, at, rate?}` or `{camera, clip}` | starts a recording or a clip |
//! | `POST /v1/cameras/live/stop` | stops live or playback (the pipeline unloads off this path) |
//! | `POST /v1/cameras/live/pause {paused}` | holds the picture of live or a recording; Play dials the recording from the key picture the held one was decoded from |
//! | `POST /v1/cameras/popup/clear` | the pop-up is dismissed (every reader sees it go) |
//! | `POST /v1/cameras/event {camera, kind}` | a test event through the policy (only with `cameras.testEvents`) |
//!
//! A refusal of a call to the bridge carries its word (`no_recording`, `camera_not_found`, `busy`, `offline`, `timeout`, `not_supported`) and
//! words for the person; a start that is refused leaves the session that is playing as it was.

use super::bridge::{self, Trouble};
use super::events::{Event, plain_id};
use super::policy::{Decision, Held as Why, Phase};
use super::popup_stream;
use super::{Held, Inner, Scrypted};
use crate::API_VERSION;
use crate::http::{Request, Response, error_response};
use crate::images::wanted_size;
use crate::live::{Mode, Plan};
use crate::timeutil::now_ms;
use serde_json::{Map, Value, json};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// Home's app id: the app LG's video path knows as in front while Home is, and so one whose video it connects to its sink.
const HOME_APP: &str = "com.webos.app.home";

/// What a stream is for beyond its camera and its mode: set for a camera pop-up's picture, empty for the full view, a recording and a clip.
#[derive(Default)]
struct Usage {
    /// The words the log says it in (`pop-up 41`).
    purpose: String,
    /// The app the feeder's decoder is opened for (empty: the video window's own).
    ndl_app: String,
    /// The window type of the feeder it plays in (empty: the configured one).
    window: String,
    /// The compositor gave the picture a texture, not a window: the feeder decodes it and paints its window (`live::Plan::texture`).
    texture: bool,
    /// The compositor gave the picture the sub window beside an app's video: the starfish pipeline asking for the sub sink (`live::Plan::sub`), opened
    /// for the app in front whatever `ndl_app` says (with Home in front, the main window's load instead: `start_stream_for`).
    sub: bool,
    /// The quality asked for (`native`, `balanced`, `light`), for the log.
    quality: String,
    /// The window the shell was granted for the full view's picture (`main`, `sub`): kept so that Play dials a paused recording again in the same one. Empty: none.
    side: String,
}

/// The app the decoder is opened for on the display's main window (`cameras.ndlApp`): the video window is no app on SAM's foreground list, so it plays
/// for Home, which is in front; `own` is the window's own id. The sub window's pipeline is the app in front's (`start_stream_for`).
fn ndl_app_for(setting: &str) -> String {
    let plain = |id: &str| !id.is_empty() && id.len() <= 100 && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
    match setting {
        "own" => String::new(),
        "home" | "auto" | "" => HOME_APP.to_owned(),
        id if plain(id) => id.to_owned(),
        _ => HOME_APP.to_owned(),
    }
}

/// The bridge's word for a quality: `native` (the camera's own stream), `balanced` (a middle one) or `light` (its low-resolution one); `high`, `medium` and `low`
/// are the same three.
fn quality_of(word: &str) -> Option<&'static str> {
    match word {
        "native" | "high" => Some("native"),
        "balanced" | "medium" => Some("balanced"),
        "light" | "low" => Some("light"),
        _ => None,
    }
}

/// Whether a live call may dial the camera. A camera on a battery starts a call that wakes it, takes up to 45 s to join and drains it, so it is dialled only
/// for its own full view, and only when the owner opened that view (`open`): never for a pop-up, a grid, a tile, a list or a view that turned to it by itself.
fn battery_may_dial(battery: bool, purpose: Option<&str>, opened: bool) -> bool {
    !battery || (opened && matches!(purpose, None | Some("full")))
}

/// The window the body names for the Cameras full view's picture on a video window (`main`, `sub`); empty when it names none.
fn side_of(body: &Map<String, Value>) -> Result<String, Response> {
    match body.get("window").and_then(Value::as_str) {
        None => Ok(String::new()),
        Some(side @ ("main" | "sub")) => Ok(side.to_owned()),
        Some(_) => Err(error_response(400, "bad_request", "window is main or sub")),
    }
}

pub const ROUTES: &[(&str, &str, &str)] = &[
    (
        "GET",
        "/v1/cameras",
        "the cameras topic: the link's state, the cameras the TV shows",
    ),
    (
        "GET",
        "/v1/cameras/{id}/frame",
        "the camera's newest still as a BMP, never older than 10 s: ?w=&h=",
    ),
    (
        "GET",
        "/v1/cameras/{id}/recordings",
        "where something was recorded, the bridge's ranges: ?from=&to=",
    ),
    (
        "GET",
        "/v1/cameras/{id}/events",
        "the span's events, oldest first: ?from=&to=&count=",
    ),
    (
        "GET",
        "/v1/cameras/{id}/clips",
        "the camera's clips, newest first: ?from=&to=&count=",
    ),
    (
        "GET",
        "/v1/cameras/{id}/thumbnail",
        "a picture of a recorded moment: ?at=&w=&h=",
    ),
    (
        "GET",
        "/v1/cameras/{id}/clip-thumbnail",
        "a clip's own picture: ?clip=&w=&h=",
    ),
    (
        "GET",
        "/v1/cameras/events/{event}/thumbnail",
        "the picture of a pushed event: ?w=&h=",
    ),
    (
        "POST",
        "/v1/cameras/live",
        "plays a camera on the video plane: {\"camera\", \"quality\"?: \"native\"|\"balanced\"|\"light\"} or, for a camera pop-up's picture, {\"camera\", \"for\": \"popup\", \"popup\"?, \"window\"?: \"main\"|\"sub\"|\"texture\"}, or, for the Cameras full view on a video window, {\"camera\", \"for\": \"full\", \"window\"?: \"main\"|\"sub\"}",
    ),
    (
        "POST",
        "/v1/cameras/playback",
        "plays a recording or a clip: {\"camera\", \"at\", \"rate\"?} or {\"camera\", \"clip\"}",
    ),
    (
        "POST",
        "/v1/cameras/live/stop",
        "stops the stream (live, playback or clip)",
    ),
    (
        "POST",
        "/v1/cameras/live/pause",
        "holds live or a recording, or plays the recording on: {\"paused\": bool}",
    ),
    ("POST", "/v1/cameras/popup/clear", "dismisses the pop-up"),
    (
        "POST",
        "/v1/cameras/event",
        "a test event through the pop-up policy: {\"camera\", \"kind\"} (needs cameras.testEvents)",
    ),
];

/// The longest a still's ask waits for the first picture of a stream.
const FIRST_STILL: Duration = Duration::from_millis(700);

fn ok(mut body: Map<String, Value>) -> Response {
    body.insert("ok".into(), Value::Bool(true));
    body.insert("api".into(), json!(API_VERSION));
    body.insert("now".into(), json!(now_ms()));
    Response::json(200, &Value::Object(body))
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => {
            let mut map = Map::new();
            map.insert("value".into(), other);
            map
        }
    }
}

/// A refusal of the bridge, as the TV's own answer.
fn trouble(t: &Trouble) -> Response {
    error_response(t.status(), &t.code(), &t.words())
}

/// A call that starts a stream and was refused: always `409`, with the bridge's word.
fn refused(code: &str, words: &str) -> Response {
    error_response(409, code, words)
}

fn int(request: &Request, name: &str) -> Result<Option<i64>, Response> {
    match request.param(name) {
        None | Some("") => Ok(None),
        Some(text) => text
            .parse::<i64>()
            .map(Some)
            .map_err(|_| error_response(400, "bad_request", &format!("{name} is a whole number"))),
    }
}

fn size(request: &Request, default: (u32, u32)) -> Result<(u32, u32), Response> {
    let (w, h) = wanted_size(request)?;
    Ok((
        if w == 0 { default.0 } else { w },
        if h == 0 { default.1 } else { h },
    ))
}

fn body_object(request: &Request) -> Result<Map<String, Value>, Response> {
    if request.body.iter().all(u8::is_ascii_whitespace) {
        return Ok(Map::new());
    }
    match serde_json::from_slice::<Value>(&request.body) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(error_response(
            400,
            "bad_request",
            "the body must be a JSON object",
        )),
        Err(e) => Err(error_response(
            400,
            "bad_request",
            &format!("the body is not JSON: {e}"),
        )),
    }
}

impl Scrypted {
    /// Handles `/v1/cameras/...`; `rest` is the path after `cameras`.
    pub fn handle(&self, request: &Request, rest: &[&str]) -> Response {
        let inner = &self.inner;
        let (get, post) = (request.method == "GET", request.method == "POST");
        // switched off, the cameras say so and serve nothing else
        if !rest.is_empty() && !inner.shared.switched_on(super::options::OWNER) {
            return error_response(409, "off", "the Cameras plugin is switched off");
        }
        match rest {
            [] if get => ok(object(json!({"cameras": inner.view()}))),
            ["live"] if post => inner.live_route(request),
            ["live", "stop"] if post => inner.stop_route(),
            ["live", "pause"] if post => inner.pause_route(request),
            ["playback"] if post => inner.playback_route(request),
            ["popup", "clear"] if post => {
                inner.clear_popup_if(None);
                ok(object(json!({"cleared": true})))
            }
            ["event"] if post => inner.test_event_route(request),
            ["events", event, "thumbnail"] if get => inner.event_thumbnail_route(request, event),
            [id, "frame"] if get => inner.frame_route(request, id),
            [id, "recordings"] if get => inner.span_route(request, id, "recordings"),
            [id, "events"] if get => inner.span_route(request, id, "events"),
            [id, "clips"] if get => inner.span_route(request, id, "clips"),
            [id, "thumbnail"] if get => inner.thumbnail_route(request, id, false),
            [id, "clip-thumbnail"] if get => inner.thumbnail_route(request, id, true),
            _ => match allowed(rest) {
                Some(allow) => {
                    error_response(405, "method_not_allowed", "that method is not served here")
                        .with_header("Allow", allow)
                }
                None => error_response(404, "not_found", "no such endpoint"),
            },
        }
    }
}

/// The method a path is for, when it is one of ours.
fn allowed(rest: &[&str]) -> Option<&'static str> {
    match rest {
        []
        | ["events", _, "thumbnail"]
        | [
            _,
            "frame" | "recordings" | "events" | "clips" | "thumbnail" | "clip-thumbnail",
        ] => Some("GET"),
        ["live"] | ["live", "stop" | "pause"] | ["playback"] | ["popup", "clear"] | ["event"] => {
            Some("POST")
        }
        _ => None,
    }
}

impl Inner {
    /// The link is not good: a picture or a span cannot be had, and the state says why.
    fn not_connected(&self) -> Response {
        match self.bridge() {
            Ok(_) => error_response(503, "not_connected", "Scrypted is not connected"),
            Err((status, reason)) => {
                let words = if reason.is_empty() {
                    format!("Scrypted is {}", status.word())
                } else {
                    reason
                };
                error_response(503, "not_connected", &words)
            }
        }
    }

    fn frame_route(&self, request: &Request, id: &str) -> Response {
        let (w, h) = match wanted_size(request) {
            Ok(size) => size,
            Err(r) => return r,
        };
        // the pictures a second the asker shows (`fps`, at most 10, the bridge's own limit): Home's tiles 1, a pop-up with no video more than a tile
        // of the grid, which names none and gets the grid's pace
        let fps = match int(request, "fps") {
            Ok(n) => u32::try_from(n.unwrap_or(0).clamp(0, 10)).unwrap_or(0),
            Err(r) => return r,
        };
        if !plain_id(id) {
            return error_response(400, "bad_request", "a camera is the id Scrypted gave it");
        }
        if self.camera(id).is_none() {
            return error_response(
                404,
                "no_such_camera",
                "Scrypted shares no such camera with this TV",
            );
        }
        if self.bridge().is_err() {
            return self.not_connected();
        }
        let picture = match self
            .stills
            .ask(id, if w == 0 { 480 } else { w }, fps, FIRST_STILL)
        {
            Ok(Some(picture)) => picture,
            Ok(None) => {
                return error_response(404, "no_picture", "no picture of this camera just now");
            }
            Err(_) => {
                return error_response(
                    503,
                    "busy",
                    "Scrypted is serving as many cameras as it can",
                );
            }
        };
        let age = picture.age().as_millis() as u64;
        self.images
            .serve_jpeg((*picture.jpeg).clone(), w, h)
            .with_header("Cache-Control", "no-store")
            .with_header("X-Luma-Camera-Age-Ms", age.to_string())
    }

    /// `recordings`, `events` and `clips`: the bridge's JSON for the span, unchanged.
    fn span_route(&self, request: &Request, id: &str, what: &str) -> Response {
        if self.camera(id).is_none() {
            return error_response(
                404,
                "no_such_camera",
                "Scrypted shares no such camera with this TV",
            );
        }
        let Ok(bridge) = self.bridge() else {
            return self.not_connected();
        };
        let (from, to, count) = match (
            int(request, "from"),
            int(request, "to"),
            int(request, "count"),
        ) {
            (Ok(from), Ok(to), Ok(count)) => (from, to, count),
            (Err(r), ..) | (_, Err(r), _) | (.., Err(r)) => return r,
        };
        let now = now_ms() as i64 + self.skew_ms();
        let to = to.unwrap_or(now);
        let default_span = if what == "clips" {
            24 * 3_600_000
        } else {
            3_600_000
        };
        let from = from.unwrap_or(to - default_span);
        if from >= to {
            return error_response(400, "bad_request", "from is before to");
        }
        let count = u16::try_from(count.unwrap_or(0).clamp(0, 1000)).unwrap_or(0);
        let request = match what {
            "recordings" => bridge::recordings(id, from, to),
            "events" => bridge::events(id, from, to, if count == 0 { 200 } else { count }),
            _ => bridge::clips(id, from, to, if count == 0 { 50 } else { count }),
        };
        match bridge.relay(&request) {
            Ok(doc) => ok(object(doc)),
            Err(t) => trouble(&t),
        }
    }

    /// A picture of a recorded moment or of a clip, decoded like a still.
    fn thumbnail_route(&self, request: &Request, id: &str, clip: bool) -> Response {
        if self.camera(id).is_none() {
            return error_response(
                404,
                "no_such_camera",
                "Scrypted shares no such camera with this TV",
            );
        }
        let Ok(bridge) = self.bridge() else {
            return self.not_connected();
        };
        let (w, h) = match size(request, (256, 144)) {
            Ok(size) => size,
            Err(r) => return r,
        };
        let asked = if clip {
            match request
                .param("clip")
                .filter(|c| !c.is_empty() && c.len() <= 400)
            {
                Some(clip) => bridge::clip_thumbnail(id, clip, w, h),
                None => return error_response(400, "bad_request", "clip is the clip's id"),
            }
        } else {
            match int(request, "at") {
                Ok(Some(at)) => bridge::thumbnail(id, at, w, h),
                Ok(None) => {
                    return error_response(
                        400,
                        "bad_request",
                        "at is the moment, in epoch milliseconds",
                    );
                }
                Err(r) => return r,
            }
        };
        match bridge.picture(&asked) {
            Ok(jpeg) => self
                .images
                .serve_jpeg(jpeg, w, h)
                .with_header("Cache-Control", "no-store"),
            Err(t) => trouble(&t),
        }
    }

    /// The picture of a pushed event (the pop-up asks for it the moment the card shows): the bridge keeps it for 30 minutes, the TV the last few.
    fn event_thumbnail_route(&self, request: &Request, event: &str) -> Response {
        if !plain_id(event) {
            return error_response(400, "bad_request", "an event is the id the bridge gave it");
        }
        let Ok(bridge) = self.bridge() else {
            return self.not_connected();
        };
        let (w, h) = match size(request, (520, 292)) {
            Ok(size) => size,
            Err(r) => return r,
        };
        let key = format!("{event}:{w}x{h}");
        let cached = self
            .thumbs
            .lock()
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.clone());
        let jpeg = match cached {
            Some(jpeg) => (*jpeg).clone(),
            None => match bridge.picture(&bridge::event_thumbnail(event, w, h)) {
                Ok(jpeg) => {
                    let mut thumbs = self.thumbs.lock();
                    thumbs.push_back((key, Arc::new(jpeg.clone())));
                    while thumbs.len() > 16 {
                        thumbs.pop_front();
                    }
                    jpeg
                }
                Err(t) => return trouble(&t),
            },
        };
        self.images.serve_jpeg(jpeg, w, h)
    }

    // ---- the streams -----------------------------------------------------------------------------------------------------------------

    /// A stream of the Cameras full view on a video window of the display (`side`: the window the shell was granted, `main` or `sub`; empty: the picture is not a
    /// video window's, and the stream is as it was before there were any): the decoder opened for Home, in the feeder's video window. Live, a recording and a clip
    /// all play there, so that a seek reuses the pipeline and the picture stays in the one claim.
    fn full_usage(&self, side: &str, purpose: &str, quality: &str) -> Usage {
        if side.is_empty() {
            return Usage::default();
        }
        Usage {
            purpose: purpose.to_owned(),
            ndl_app: ndl_app_for(&self.config.ndl_app),
            window: self.config.popup_window.clone(),
            texture: false,
            sub: side == "sub",
            quality: quality.to_owned(),
            side: side.to_owned(),
        }
    }

    fn camera_for(&self, body: &Map<String, Value>) -> Result<String, Response> {
        let id = body.get("camera").and_then(Value::as_str).unwrap_or("");
        if !plain_id(id) {
            return Err(error_response(
                400,
                "bad_request",
                "camera is the id Scrypted gave it",
            ));
        }
        if self.camera(id).is_none() {
            return Err(error_response(
                404,
                "no_such_camera",
                "Scrypted shares no such camera with this TV",
            ));
        }
        Ok(id.to_owned())
    }

    /// A start that is nobody's pop-up: the full view's live, a recording, a clip.
    fn start_stream(
        self: &Arc<Self>,
        camera: &str,
        mode: Mode,
        ask: impl FnOnce(&bridge::Bridge) -> Result<bridge::Dial, Trouble>,
    ) -> Response {
        self.start_stream_for(camera, mode, Usage::default(), ask)
    }

    /// What every start has in common: the live path is on, the link is good, and the call to the bridge is made before anything is changed, so
    /// that a refusal leaves the session that plays as it was. The last of two quick calls is the one that plays.
    fn start_stream_for(
        self: &Arc<Self>,
        camera: &str,
        mode: Mode,
        mut usage: Usage,
        ask: impl FnOnce(&bridge::Bridge) -> Result<bridge::Dial, Trouble>,
    ) -> Response {
        if !self.live.available() {
            return refused(
                "live_off",
                "Live video is switched off on this TV (cameras.live)",
            );
        }
        let Ok(bridge) = self.bridge() else {
            return refused("not_ready", "Scrypted is not connected");
        };
        // the sub window's pipeline is opened for the app in front: SAM lists it, so the media server and videooutputd connect the video, and LG's AV
        // connector keeps the sub sink the load asks for. Home's media pipeline it puts on the main window whatever the load asks (Home's own
        // picture-in-picture), so with Home in front the picture takes the main window as it would there (docs/research/multiview-result.md)
        if usage.sub {
            match self.deps.front_app.as_ref().and_then(|front| front()) {
                Some(app) if app != HOME_APP => usage.ndl_app = app,
                front => {
                    crate::log_info!(
                        "cameras: the sub window was granted with {} in front: the picture plays on the main window",
                        front.as_deref().unwrap_or("no app")
                    );
                    usage.sub = false;
                }
            }
        }
        let ticket = self.ticket.fetch_add(1, Ordering::SeqCst) + 1;
        self.live.asking(camera, mode, &usage.purpose);
        let asked_at = std::time::Instant::now();
        let dial = match ask(&bridge) {
            Ok(dial) => dial,
            Err(t) => {
                self.live.asked_in_vain(camera, &t.words());
                if !usage.purpose.is_empty() {
                    crate::log_info!(
                        "cameras: live for {} on camera {camera} refused: {}",
                        usage.purpose,
                        t.words()
                    );
                }
                return refused(&t.code(), &t.words());
            }
        };
        let bridge_ms = asked_at.elapsed().as_millis();
        // a camera on a battery: its live view is a call that wakes it, and the call is hung up when the view ends or is cancelled
        let battery = mode == Mode::Live && self.camera(camera).is_some_and(|c| c.battery);
        let end_session = |dial: &bridge::Dial| -> Option<Box<dyn FnOnce() + Send>> {
            ((mode != Mode::Live || battery) && !dial.session.is_empty()).then(|| {
                let (bridge, session) = (bridge.clone(), dial.session.clone());
                Box::new(move || {
                    let _ = bridge.stop_playback(&session);
                }) as Box<dyn FnOnce() + Send>
            })
        };
        if self.ticket.load(Ordering::SeqCst) != ticket {
            // another call came after this one: its stream is the one that plays, and this one's playback is given back
            if let Some(end) = end_session(&dial) {
                end();
            }
            return refused("superseded", "Another stream was asked for meanwhile");
        }
        if battery {
            crate::log_info!("cameras: camera {camera} runs on a battery: its first picture is waited for up to 45 s");
        }
        if !usage.purpose.is_empty() {
            // the stream's address is a capability and is not here: what the bridge says it is, where it will play, and how long the bridge took to answer
            crate::log_info!(
                "cameras: live for {} on camera {camera}: {} {}x{} at {} a second declared{}, quality {}, window {}, decoder opened for {}, the bridge answered in {bridge_ms} ms",
                usage.purpose,
                dial.codec,
                dial.width,
                dial.height,
                dial.fps,
                if dial.prebuffered { ", prebuffered" } else { ", not prebuffered" },
                if usage.quality.is_empty() { "as set" } else { &usage.quality },
                if usage.window.is_empty() { "as configured" } else { &usage.window },
                if usage.ndl_app.is_empty() { "the window's own app" } else { &usage.ndl_app },
            );
        }
        let plan = Plan {
            camera: camera.to_owned(),
            mode,
            url: dial.rtsp.clone(),
            codec: dial.codec.clone(),
            width: dial.width,
            height: dial.height,
            fps: dial.fps,
            started_at: dial.started_at,
            rate: dial.rate,
            ndl_app: usage.ndl_app,
            texture: usage.texture,
            sub: usage.sub,
            purpose: usage.purpose,
            window: usage.window,
            battery,
            on_end: end_session(&dial),
        };
        self.state.lock().side.clone_from(&usage.side);
        self.set_held(None);
        match self.live.start(plan) {
            Ok(()) => ok(object(json!({"live": self.live.view().json()}))),
            Err(why) => refused("not_now", &why),
        }
    }

    fn live_route(self: &Arc<Self>, request: &Request) -> Response {
        let body = match body_object(request) {
            Ok(body) => body,
            Err(r) => return r,
        };
        let camera = match self.camera_for(&body) {
            Ok(camera) => camera,
            Err(r) => return r,
        };
        let quality = match body.get("quality").and_then(Value::as_str) {
            None => self.options().live,
            Some(word) => match quality_of(word) {
                Some(quality) => quality.to_owned(),
                None => return error_response(400, "bad_request", "quality is native, balanced or light"),
            },
        };
        let purpose = body.get("for").and_then(Value::as_str);
        let opened = body.get("open").and_then(Value::as_bool) == Some(true);
        if self.camera(&camera).is_some_and(|c| !battery_may_dial(c.battery, purpose, opened)) {
            return refused(
                "battery_still_only",
                "This camera runs on a battery: it shows its latest picture, and goes live only when its full view is opened",
            );
        }
        match purpose {
            None => {
                let listing = self.camera(&camera);
                self.start_stream(&camera, Mode::Live, |bridge| bridge.live_full(&camera, &quality, listing.as_ref()))
            }
            Some("popup") => {
                if !self.config.popup_live {
                    return refused(
                        "live_off",
                        "Live video in the camera pop-up is switched off on this TV (cameras.popupLive)",
                    );
                }
                let window = body.get("window").and_then(Value::as_str).unwrap_or("main");
                if !["main", "sub", "texture"].contains(&window) {
                    return error_response(400, "bad_request", "window is main, sub or texture");
                }
                let purpose = match body.get("popup").and_then(Value::as_u64) {
                    Some(id) => format!("pop-up {id}"),
                    None => "pop-up".to_owned(),
                };
                // a pop-up is always the camera's low stream, whatever quality the owner chose for the full view. The texture path is decoded on the CPU, so
                // the bridge's low stream inside what it takes is all it needs. A window's decoder sets the panel's rate (1080p at 25 takes 120 Hz to 100 Hz), so
                // its stream must be a calm one (`popup_stream`), judged by what was read from the stream where the bridge gives no size; a camera without one
                // is refused, and the shell draws that pop-up as a texture
                let texture = window == "texture";
                let quality = "light".to_owned();
                let learned = self.camera(&camera).and_then(|c| c.learned);
                let usage = Usage {
                    purpose,
                    ndl_app: ndl_app_for(&self.config.ndl_app),
                    window: self.config.popup_window.clone(),
                    texture,
                    sub: window == "sub",
                    quality: quality.clone(),
                    side: String::new(),
                };
                self.start_stream_for(&camera, Mode::Live, usage, |bridge| {
                    if texture {
                        bridge.live_low(&camera, &quality)
                    } else {
                        popup_stream::ask_window(&camera, learned.as_ref(), |step| bridge.live_within(&camera, &quality, step.accept())).map(|(dial, _)| dial)
                    }
                })
            }
            Some("full") => {
                let window = match side_of(&body) {
                    Ok(side) if side.is_empty() => "main".to_owned(),
                    Ok(side) => side,
                    Err(r) => return r,
                };
                let usage = self.full_usage(&window, "full view", &quality);
                // beside an app's video the view is on the sub window, which takes less than the camera's own stream may be
                let sub = window == "sub";
                let listing = self.camera(&camera);
                self.start_stream_for(&camera, Mode::Live, usage, |bridge| {
                    if sub {
                        bridge.live_sub(&camera, &quality)
                    } else {
                        bridge.live_full(&camera, &quality, listing.as_ref())
                    }
                })
            }
            Some(_) => error_response(400, "bad_request", "for is popup or full"),
        }
    }

    fn playback_route(self: &Arc<Self>, request: &Request) -> Response {
        let body = match body_object(request) {
            Ok(body) => body,
            Err(r) => return r,
        };
        let camera = match self.camera_for(&body) {
            Ok(camera) => camera,
            Err(r) => return r,
        };
        let rate = body
            .get("rate")
            .map_or(Some(1), |r| r.as_u64().and_then(|r| u8::try_from(r).ok()));
        let Some(rate) = rate.filter(|r| matches!(r, 1 | 2 | 4)) else {
            return error_response(400, "bad_request", "rate is 1, 2 or 4");
        };
        // the full view on a video window of the display asks for its recordings and clips there too (`window`), so that a seek reuses the pipeline and the picture
        // stays in the one claim; without it they play as they did before there were any
        let side = match side_of(&body) {
            Ok(side) => side,
            Err(r) => return r,
        };
        if let Some(clip) = body.get("clip").and_then(Value::as_str) {
            if clip.is_empty() || clip.len() > 400 {
                return error_response(400, "bad_request", "clip is the clip's id");
            }
            return self.start_stream_for(&camera, Mode::Clip, self.full_usage(&side, "full view clip", ""), |bridge| {
                bridge.clip_playback(&camera, clip, side == "sub")
            });
        }
        let Some(at) = body.get("at").and_then(Value::as_i64) else {
            return error_response(
                400,
                "bad_request",
                "at is the moment, in epoch milliseconds, or give a clip",
            );
        };
        self.start_stream_for(&camera, Mode::Playback, self.full_usage(&side, "full view recording", ""), |bridge| {
            bridge.playback(&camera, at, rate, side == "sub")
        })
    }

    fn stop_route(&self) -> Response {
        self.ticket.fetch_add(1, Ordering::SeqCst);
        self.set_held(None);
        self.live.stop("the viewer closed");
        ok(object(json!({"live": self.live.view().json()})))
    }

    fn pause_route(self: &Arc<Self>, request: &Request) -> Response {
        let body = match body_object(request) {
            Ok(body) => body,
            Err(r) => return r,
        };
        let Some(paused) = body.get("paused").and_then(Value::as_bool) else {
            return error_response(400, "bad_request", "paused is true or false");
        };
        if paused {
            let Some((camera, _)) = self.live.current() else {
                return refused("not_playback", "Nothing is playing");
            };
            let (mode, from) = match self.live.pause() {
                Ok(held) => held,
                Err(why) => return refused("not_playback", &why),
            };
            let side = self.state.lock().side.clone();
            // held live plays on as the recording of that moment; its pictures are dated by this TV's clock, the recording by the bridge's
            let (mode, at) = if mode == Mode::Live { (Mode::Playback, from + self.skew_ms()) } else { (mode, from) };
            self.set_held(Some(Held {
                camera,
                mode,
                at,
                rate: self.live.view().rate,
                side,
            }));
            return ok(object(json!({"live": self.live.view().json()})));
        }
        // Play: the recording is dialled again from the key picture the held one was decoded from
        let Some(held) = self.held() else {
            return refused("not_paused", "Nothing is paused");
        };
        let camera = held.camera.clone();
        let response = self.start_stream_for(&camera, held.mode, self.full_usage(&held.side, "full view recording", ""), |bridge| {
            bridge.playback(&held.camera, held.at, held.rate, held.side == "sub")
        });
        if response.status == 200 {
            // the mode is the one it was (a clip stays a clip in the view)
            self.set_held(None);
        }
        response
    }

    fn test_event_route(self: &Arc<Self>, request: &Request) -> Response {
        if !self.config.test_events {
            return error_response(
                403,
                "test_events_off",
                "test events are off (cameras.testEvents)",
            );
        }
        let body = match body_object(request) {
            Ok(body) => body,
            Err(r) => return r,
        };
        let camera = match self.camera_for(&body) {
            Ok(camera) => camera,
            Err(r) => return r,
        };
        let kind = body.get("kind").and_then(Value::as_str).unwrap_or("");
        if !super::policy::KINDS.contains(&kind) {
            return error_response(
                400,
                "bad_request",
                "kind is person, vehicle, animal, package, doorbell or motion",
            );
        }
        let name = self.camera(&camera).map(|c| c.name).unwrap_or_default();
        let seq = self.popup_seq();
        let event = Event {
            id: format!("t{seq}"),
            seq: None,
            camera,
            name,
            kind: kind.to_owned(),
            label: body
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars()
                .take(64)
                .collect(),
            classes: vec![kind.to_owned()],
            score: Some(1.0),
            at: now_ms() as i64 + self.skew_ms(),
            phase: Phase::New,
            thumbnail: false,
            recording: false,
        };
        match self.on_event(event, true) {
            Some(Decision::Pop { lasts_ms }) => {
                ok(object(json!({"popup": true, "lastsMs": lasts_ms})))
            }
            Some(Decision::Update) => ok(object(json!({"popup": false, "held": "update"}))),
            Some(Decision::Held(why)) => ok(object(json!({"popup": false, "held": match why {
                Why::Off => "off",
                Why::Kind => "kind",
                Why::End => "end",
                Why::Cooldown => "cooldown",
                Why::Old => "old",
            }}))),
            None => error_response(
                404,
                "no_such_camera",
                "Scrypted shares no such camera with this TV",
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quality_is_one_of_three_and_the_plain_words_are_the_same_three() {
        assert_eq!(
            ["native", "high", "balanced", "medium", "light", "low"].map(quality_of),
            [Some("native"), Some("native"), Some("balanced"), Some("balanced"), Some("light"), Some("light")]
        );
        assert_eq!(quality_of("ultra"), None);
    }

    #[test]
    fn the_decoder_is_opened_for_home_in_either_window_unless_the_setting_names_an_app() {
        assert_eq!(ndl_app_for("auto"), HOME_APP);
        assert_eq!(ndl_app_for(""), HOME_APP);
        assert_eq!(ndl_app_for("own"), "");
        assert_eq!(ndl_app_for("home"), HOME_APP);
        assert_eq!(ndl_app_for("org.example.app"), "org.example.app");
        assert_eq!(ndl_app_for("bad id!"), HOME_APP);
    }
}
