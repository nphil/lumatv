//! The LumaShow bridge's HTTP API, versions 1 and 1.1 (`docs/scrypted-bridge-api.md` in the bridge's repository, and `scrypted-cameras-design.md`
//! 3.3): what the TV asks of it and how it reads the answers.
//!
//! The request builders and the readers are pure (a request is a value, an answer a JSON value or bytes); [`Bridge`] only runs them over the
//! client. The Show's plugin has the same calls in `plugins/scrypted/src/bridge.rs`, written against the plugin SDK's types; the TV has no SDK
//! at level 1, so this is the SDK-free copy of the same requests until the shared crate the design proposes exists.
//!
//! The bridge's times are epoch milliseconds, and so are the TV's. A client ignores the fields it does not know, so answers are read as JSON
//! values and only what is used is looked at. A stream address (`rtsp`) is a capability: it is returned to the caller that needs it and is
//! never put in an error, a log or a topic.

use super::http::{BodyReader, Client, Head, HttpError, Request};
use super::tls::ConnectError;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

/// What the bridge allows: the span of a recordings or events call (48 hours), events and clips per call, the box of a thumbnail.
pub const MAX_SPAN_MS: i64 = 48 * 3_600_000;
pub const MAX_EVENTS: u16 = 500;
pub const MAX_CLIPS: u16 = 100;
/// How long a playback call may wait for the recorder's stream (the bridge's most is 4.5 s).
pub const PLAYBACK_WAIT_MS: u32 = 3_500;
/// The largest JSON answer and the largest picture the bridge sends (256 KiB and 400 KB).
const MAX_JSON: usize = 300 * 1024;
const MAX_JPEG: usize = 500 * 1024;
/// What a stills stream may be silent for at its start (the camera's next key picture): later reads are given less.
pub const STILLS_SILENCE: Duration = Duration::from_secs(10);

/// What the set takes (LG's declared limits; HEVC at 100 Mbit/s ran clean in the GFN probe): the bridge picks the highest stream inside.
pub fn accept() -> Value {
    json!({"codecs": ["h264", "h265"], "max_width": 4096, "max_height": 2304, "max_fps": 60})
}

/// What the full view asks of the bridge at `quality`: [`accept`], turned and trimmed for the stream the camera's listing says that quality is.
/// The decoder's box is 4096x2304 and a portrait picture takes it on its side (`vwin.rs` `Need::fits`: a doorbell's 1920x2560 main stream is as large to the
/// window as 2560x1920), but the bridge compares width with width, so the box is turned for a portrait stream. The codec list is left out when the listing
/// gives the stream no codec: the bridge counts such a stream's fit as unknown and picks a smaller one that does name its codec, which is how the Reolink
/// doorbells' main streams lost to their 640x480 low ones.
pub fn accept_full(listing: Option<&Camera>, quality: &str) -> Value {
    let mut accept = accept();
    let stream = listing.and_then(|c| c.stream(quality).or_else(|| c.native.as_ref().filter(|_| quality == "native")));
    let Some(stream) = stream else { return accept };
    if let Some(limits) = accept.as_object_mut() {
        if stream.height > stream.width && stream.width > 0 {
            let (w, h) = (limits["max_width"].clone(), limits["max_height"].clone());
            limits.insert("max_width".into(), h);
            limits.insert("max_height".into(), w);
        }
        if stream.codec.is_empty() {
            limits.remove("codecs");
        }
    }
    accept
}

/// What the display's sub window takes (`vwin.rs` `limits`: 1920x1088 at 30 for H.264 and H.265): the Cameras full view shown beside an app's video
/// plays inside it, the camera's best stream that fits.
pub fn accept_sub() -> Value {
    json!({"codecs": ["h264", "h265"], "max_width": 1920, "max_height": 1088, "max_fps": 30})
}

/// What the CPU path takes (a software decode into a texture): the camera's low stream, H.264 up to 720p at 30.
pub fn accept_low() -> Value {
    json!({"codecs": ["h264"], "max_width": 1280, "max_height": 720, "max_fps": 30})
}

/// Percent-encodes a value for a query (ids are opaque: never parsed, always encoded).
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(char::from(b))
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// A box the bridge fits a thumbnail inside: 16 to 1280 by 16 to 720.
fn boxed(w: u32, h: u32) -> (u32, u32) {
    (w.clamp(16, 1280), h.clamp(16, 720))
}

// ---- the requests ---------------------------------------------------------------------------------------------------------------

pub fn health() -> Request {
    Request::get("/v1/health")
}

pub fn cameras() -> Request {
    Request::get("/v1/cameras?scope=all")
}

pub fn recordings(camera: &str, from: i64, to: i64) -> Request {
    // the span is the bridge's to limit; a longer one is cut to the newest 48 hours
    let from = from.max(to - MAX_SPAN_MS);
    Request::get(format!(
        "/v1/recordings?camera={}&from={from}&to={to}",
        encode(camera)
    ))
}

pub fn events(camera: &str, from: i64, to: i64, count: u16) -> Request {
    let from = from.max(to - MAX_SPAN_MS);
    Request::get(format!(
        "/v1/events?camera={}&from={from}&to={to}&count={}",
        encode(camera),
        count.clamp(1, MAX_EVENTS)
    ))
}

pub fn clips(camera: &str, from: i64, to: i64, count: u16) -> Request {
    Request::get(format!(
        "/v1/clips?camera={}&from={from}&to={to}&count={}",
        encode(camera),
        count.clamp(1, MAX_CLIPS)
    ))
}

pub fn thumbnail(camera: &str, at: i64, w: u32, h: u32) -> Request {
    let (w, h) = boxed(w, h);
    Request::get(format!(
        "/v1/thumbnail?camera={}&time={at}&w={w}&h={h}",
        encode(camera)
    ))
    .accepting("image/jpeg")
}

pub fn clip_thumbnail(camera: &str, clip: &str, w: u32, h: u32) -> Request {
    let (w, h) = boxed(w, h);
    Request::get(format!(
        "/v1/clip-thumbnail?camera={}&id={}&w={w}&h={h}",
        encode(camera),
        encode(clip)
    ))
    .accepting("image/jpeg")
}

pub fn event_thumbnail(event: &str, w: u32, h: u32) -> Request {
    let (w, h) = boxed(w, h);
    Request::get(format!(
        "/v1/event-thumbnail?id={}&w={w}&h={h}",
        encode(event)
    ))
    .accepting("image/jpeg")
}

pub fn still(camera: &str, w: u32, h: u32) -> Request {
    let (w, h) = boxed(w, h);
    Request::get(format!("/v1/still?camera={}&w={w}&h={h}", encode(camera))).accepting("image/jpeg")
}

/// `GET /v1/stills`: a camera's stills as a multipart stream, `w` 160 to 1280, `fps` 1 to 10.
pub fn stills(camera: &str, w: u32, fps: u32) -> Request {
    Request::get(format!(
        "/v1/stills?camera={}&w={}&fps={}",
        encode(camera),
        w.clamp(160, 1280),
        fps.clamp(1, 10)
    ))
    .accepting("multipart/x-mixed-replace")
}

/// `GET /v1/events/stream`, resuming after `last` when the channel is being reopened. Motion is asked for by name: the bridge does not
/// listen to it unless a client says it wants it.
pub fn events_stream(motion: bool) -> Request {
    let path = if motion {
        "/v1/events/stream?kinds=person,vehicle,animal,package,doorbell,other,motion"
    } else {
        "/v1/events/stream"
    };
    Request::get(path).accepting("text/event-stream")
}

/// `POST /v1/live` for a stream inside `accept`.
pub fn live_within(camera: &str, quality: &str, accept: Value) -> Request {
    Request::post(
        "/v1/live",
        json!({"camera": camera, "quality": quality, "accept": accept}).to_string(),
    )
}

pub fn playback(camera: &str, start: i64, rate: u8) -> Request {
    playback_within(camera, start, rate, accept())
}

/// `POST /v1/playback` for a stream inside `accept`.
pub fn playback_within(camera: &str, start: i64, rate: u8, accept: Value) -> Request {
    Request::post(
        "/v1/playback",
        json!({"camera": camera, "start": start, "rate": rate, "wait_ms": PLAYBACK_WAIT_MS, "accept": accept}).to_string(),
    )
}

pub fn clip_playback(camera: &str, clip: &str) -> Request {
    clip_playback_within(camera, clip, accept())
}

/// `POST /v1/clip-playback` for a stream inside `accept`.
pub fn clip_playback_within(camera: &str, clip: &str, accept: Value) -> Request {
    Request::post(
        "/v1/clip-playback",
        json!({"camera": camera, "id": clip, "wait_ms": PLAYBACK_WAIT_MS, "accept": accept})
            .to_string(),
    )
}

pub fn stop_playback(session: &str) -> Request {
    Request::post("/v1/playback/stop", json!({"session": session}).to_string())
}

// ---- what went wrong ------------------------------------------------------------------------------------------------------------

/// Why a call gave no answer the TV can use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trouble {
    Http(HttpError),
    /// The bridge answered with an error: its status and its machine word.
    Bridge {
        status: u16,
        code: String,
    },
    /// An answer that is not what the API says.
    Unreadable(String),
}

impl Trouble {
    /// The token was refused: the bridge no longer knows this TV.
    pub fn refused(&self) -> bool {
        matches!(self, Self::Bridge { status: 401, .. })
    }

    /// The pinned certificate is not the one the bridge showed.
    pub fn untrusted(&self) -> bool {
        matches!(
            self,
            Self::Http(HttpError::Connect(ConnectError::Untrusted))
        )
    }

    /// The bridge could not be reached or spoken to (as against: reached, and said no, or showed another certificate): nobody answered, the
    /// connection or its handshake ended or failed, or the reply was not HTTP. A bridge that hangs up while the handshake is still running (a
    /// restart, a connection limit) is this too, not a link that is fine.
    pub fn unreachable(&self) -> bool {
        matches!(self, Self::Http(e) if !matches!(e, HttpError::Connect(ConnectError::Untrusted)))
    }

    /// The machine word the TV's own answers carry: the bridge's when it gave one.
    pub fn code(&self) -> String {
        match self {
            Self::Bridge { code, status } if !code.is_empty() => {
                let _ = status;
                code.clone()
            }
            Self::Bridge { status, .. } => format!("http_{status}"),
            Self::Http(HttpError::Connect(ConnectError::Untrusted)) => "untrusted".into(),
            Self::Http(HttpError::Connect(_)) => "unreachable".into(),
            Self::Http(HttpError::Timeout) => "timeout".into(),
            Self::Http(_) => "unreachable".into(),
            Self::Unreadable(_) => "unreadable".into(),
        }
    }

    /// The HTTP status of the TV's own answer to its caller.
    pub fn status(&self) -> u16 {
        match self {
            Self::Bridge { status: 401, .. } | Self::Bridge { status: 403, .. } => 503,
            Self::Bridge { status, .. } if (400..600).contains(status) => *status,
            Self::Http(HttpError::Timeout) => 504,
            Self::Http(_) | Self::Unreadable(_) | Self::Bridge { .. } => 502,
        }
    }

    /// What the person is told: short, in words, and never an address or a token.
    pub fn words(&self) -> String {
        match self {
            Self::Http(HttpError::Connect(ConnectError::Untrusted)) => {
                "The certificate is not the one pinned for Scrypted".into()
            }
            Self::Http(HttpError::Timeout) => "Scrypted is too slow".into(),
            Self::Http(_) => "Scrypted is not answering".into(),
            Self::Unreadable(_) => "Scrypted's answer was not understood".into(),
            Self::Bridge { status, code } => match (*status, code.as_str()) {
                (_, "no_recording") | (_, "clip_not_found") => "No recording at that time".into(),
                (401, _) => "Scrypted refused this TV".into(),
                (403, _) => "Scrypted does not take requests from this TV".into(),
                (_, "camera_not_found") => {
                    "Scrypted does not share that camera with this TV".into()
                }
                (_, "offline") => "The camera is offline".into(),
                (_, "no_calm_stream") => "The camera has no low stream a video window can play without changing the panel's rate".into(),
                (429, _) => "Scrypted is busy, try again".into(),
                (501, _) => "Scrypted cannot do that for this camera".into(),
                (504, _) => "Scrypted's recorder is too slow".into(),
                (502, _) => "Scrypted failed the request".into(),
                (404, _) => "Scrypted has nothing for that".into(),
                _ => "Scrypted could not do that".into(),
            },
        }
    }
}

/// A status and body read as the bridge's JSON: an error status is [`Trouble::Bridge`] with its word.
pub fn json_of(status: u16, body: &[u8]) -> Result<Value, Trouble> {
    if status >= 400 {
        let code = serde_json::from_slice::<Value>(body)
            .ok()
            .and_then(|v| v["error"]["code"].as_str().map(str::to_owned))
            .unwrap_or_default();
        return Err(Trouble::Bridge { status, code });
    }
    serde_json::from_slice(body).map_err(|e| Trouble::Unreadable(e.to_string()))
}

// ---- what is read of the answers ------------------------------------------------------------------------------------------------

/// `GET /v1/health`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Health {
    pub api: u32,
    /// The bridge's own clock, epoch milliseconds.
    pub now: i64,
    pub cameras: u32,
    /// The name this TV's token has in the bridge's list of devices (`None` for the Show's single token).
    pub device: Option<String>,
    pub features: Vec<(String, u32)>,
}

impl Health {
    pub fn has(&self, feature: &str) -> bool {
        self.features
            .iter()
            .any(|(name, version)| name == feature && *version > 0)
    }
}

pub fn health_of(v: &Value) -> Result<Health, Trouble> {
    let api = v["api"]
        .as_u64()
        .ok_or_else(|| Trouble::Unreadable("no api".into()))?;
    Ok(Health {
        api: u32::try_from(api).unwrap_or(u32::MAX),
        now: v["now"].as_i64().unwrap_or(0),
        cameras: v["cameras"]
            .as_u64()
            .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX)),
        device: v["device"].as_str().map(str::to_owned),
        features: v["features"]
            .as_object()
            .map(|o| {
                o.iter()
                    .map(|(k, n)| {
                        (
                            k.clone(),
                            n.as_u64().map_or(0, |n| u32::try_from(n).unwrap_or(0)),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// One stream of a camera, in the bridge's words.
#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    /// `native`, `balanced` or `light`.
    pub quality: String,
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub prebuffered: bool,
}

/// One camera of `GET /v1/cameras?scope=all`.
#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    pub id: String,
    pub name: String,
    pub room: Option<String>,
    /// `camera` or `doorbell`.
    pub kind: String,
    pub online: bool,
    pub features: Vec<String>,
    /// The oldest moment the bridge found a recording at, epoch milliseconds.
    pub earliest: Option<i64>,
    pub native: Option<Stream>,
    pub streams: Vec<Stream>,
}

impl Camera {
    pub fn has(&self, feature: &str) -> bool {
        self.features.iter().any(|f| f == feature)
    }

    /// The stream the full view plays at the given quality, if the bridge listed one.
    pub fn stream(&self, quality: &str) -> Option<&Stream> {
        self.streams.iter().find(|s| s.quality == quality)
    }
}

/// A stream's description; any member of it may be absent (the bridge leaves out what Scrypted did not say), and `null` is no description.
fn stream_of(quality: &str, v: &Value) -> Option<Stream> {
    v.as_object()?;
    Some(Stream {
        quality: quality.to_owned(),
        codec: v["codec"].as_str().unwrap_or("").to_ascii_lowercase(),
        width: v["width"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0),
        height: v["height"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0),
        fps: v["fps"].as_f64().unwrap_or(0.0),
        prebuffered: v["prebuffered"].as_bool().unwrap_or(false),
    })
}

pub fn cameras_of(v: &Value) -> Result<Vec<Camera>, Trouble> {
    let list = v["cameras"]
        .as_array()
        .ok_or_else(|| Trouble::Unreadable("no cameras".into()))?;
    Ok(list
        .iter()
        .filter_map(|c| {
            let id = c["id"].as_str()?.to_owned();
            let streams: Vec<Stream> = c["streams"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| stream_of(s["quality"].as_str().unwrap_or(""), s))
                        .filter(|s| !s.quality.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            let native = stream_of("native", &c["native"])
                .or_else(|| streams.iter().find(|s| s.quality == "native").cloned());
            Some(Camera {
                name: c["name"]
                    .as_str()
                    .filter(|n| !n.is_empty())
                    .unwrap_or(&id)
                    .to_owned(),
                room: c["room"]
                    .as_str()
                    .filter(|r| !r.is_empty())
                    .map(str::to_owned),
                kind: if c["type"].as_str() == Some("doorbell") {
                    "doorbell".into()
                } else {
                    "camera".into()
                },
                online: c["online"].as_bool().unwrap_or(true),
                features: c["features"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|f| f.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default(),
                earliest: c["earliest"].as_i64(),
                native,
                streams,
                id,
            })
        })
        .collect())
}

/// What a live or playback call gave: where to dial and what the stream is.
#[derive(Clone, PartialEq, Eq)]
pub struct Dial {
    /// The stream's address: a capability, never shown.
    pub rtsp: String,
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Playback: the moment of the recording the first picture belongs to, epoch milliseconds (0 for live).
    pub started_at: i64,
    /// Playback: the bridge's name for the session, for its stop call.
    pub session: String,
    pub rate: u8,
    pub prebuffered: bool,
}

impl std::fmt::Debug for Dial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Dial({} {}x{}, address hidden)",
            self.codec, self.width, self.height
        )
    }
}

pub fn dial_of(v: &Value) -> Result<Dial, Trouble> {
    let rtsp = v["rtsp"]
        .as_str()
        .filter(|r| r.starts_with("rtsp://"))
        .ok_or_else(|| Trouble::Unreadable("no stream address".into()))?
        .to_owned();
    let video = &v["video"];
    Ok(Dial {
        rtsp,
        codec: video["codec"].as_str().unwrap_or("").to_ascii_lowercase(),
        width: video["width"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0),
        height: video["height"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0),
        fps: video["fps"].as_f64().map_or(0, |f| f.round() as u32),
        started_at: v["started_at"].as_i64().unwrap_or(0),
        session: v["session"].as_str().unwrap_or_default().to_owned(),
        rate: v["rate"]
            .as_u64()
            .and_then(|r| u8::try_from(r).ok())
            .filter(|r| *r > 0)
            .unwrap_or(1),
        prebuffered: v["prebuffered"].as_bool().unwrap_or(false),
    })
}

// ---- running them ---------------------------------------------------------------------------------------------------------------

/// The typed calls over one client.
#[derive(Clone)]
pub struct Bridge {
    client: Arc<Client>,
}

impl Bridge {
    pub fn new(client: Arc<Client>) -> Self {
        Self { client }
    }

    pub fn client(&self) -> &Arc<Client> {
        &self.client
    }

    fn json(&self, request: &Request) -> Result<Value, Trouble> {
        let reply = self.client.call(request, MAX_JSON).map_err(Trouble::Http)?;
        json_of(reply.head.status, &reply.body)
    }

    /// An answer that is a picture: a `2xx` is its bytes.
    fn jpeg(&self, request: &Request) -> Result<Vec<u8>, Trouble> {
        let reply = self.client.call(request, MAX_JPEG).map_err(Trouble::Http)?;
        if reply.head.status >= 400 {
            return Err(json_of(reply.head.status, &reply.body)
                .err()
                .unwrap_or(Trouble::Bridge {
                    status: reply.head.status,
                    code: String::new(),
                }));
        }
        Ok(reply.body)
    }

    pub fn health(&self) -> Result<Health, Trouble> {
        health_of(&self.json(&health())?)
    }

    pub fn cameras(&self) -> Result<Vec<Camera>, Trouble> {
        cameras_of(&self.json(&cameras())?)
    }

    /// The full view's live stream: `quality`'s, inside what the decoder takes for the stream the camera's listing says that is ([`accept_full`]).
    pub fn live_full(&self, camera: &str, quality: &str, listing: Option<&Camera>) -> Result<Dial, Trouble> {
        self.live_within(camera, quality, accept_full(listing, quality))
    }

    /// A stream inside the limits `accept` names (a pop-up's window asks for each calm step in turn, `popup_stream`).
    pub fn live_within(&self, camera: &str, quality: &str, accept: Value) -> Result<Dial, Trouble> {
        dial_of(&self.json(&live_within(camera, quality, accept))?)
    }

    /// A stream for the CPU path: the camera's own low stream, inside [`accept_low`].
    pub fn live_low(&self, camera: &str, quality: &str) -> Result<Dial, Trouble> {
        dial_of(&self.json(&live_within(camera, quality, accept_low()))?)
    }

    /// A live stream for the display's sub window: the best the camera has inside [`accept_sub`], `quality` first and the lower ones when the camera has
    /// no stream that small at that quality (the bridge's `501`).
    pub fn live_sub(&self, camera: &str, quality: &str) -> Result<Dial, Trouble> {
        let ladder: &[&str] = match quality {
            "native" => &["native", "balanced", "light"],
            "balanced" => &["balanced", "light"],
            _ => &["light"],
        };
        let mut last = Trouble::Bridge { status: 501, code: String::new() };
        for q in ladder {
            match self.live_within(camera, q, accept_sub()) {
                Err(t @ Trouble::Bridge { status: 501, .. }) => last = t,
                other => return other,
            }
        }
        Err(last)
    }

    /// A recording for the main window (`sub` false: up to what the decoder takes) or for the sub window (inside [`accept_sub`]).
    pub fn playback(&self, camera: &str, start: i64, rate: u8, sub: bool) -> Result<Dial, Trouble> {
        let accept = if sub { accept_sub() } else { accept() };
        dial_of(&self.json(&playback_within(camera, start, rate, accept))?)
    }

    pub fn clip_playback(&self, camera: &str, clip: &str, sub: bool) -> Result<Dial, Trouble> {
        let accept = if sub { accept_sub() } else { accept() };
        dial_of(&self.json(&clip_playback_within(camera, clip, accept))?)
    }

    pub fn stop_playback(&self, session: &str) -> Result<(), Trouble> {
        self.json(&stop_playback(session)).map(|_| ())
    }

    /// The bridge's JSON for `recordings`, `events` and `clips`, as it is: the shell reads the bridge's own shape.
    pub fn relay(&self, request: &Request) -> Result<Value, Trouble> {
        self.json(request)
    }

    pub fn picture(&self, request: &Request) -> Result<Vec<u8>, Trouble> {
        self.jpeg(request)
    }

    /// Opens the push channel. A status other than 200 is the bridge's refusal.
    pub fn open_events(
        &self,
        motion: bool,
        last_id: Option<&str>,
        within: Duration,
    ) -> Result<BodyReader, Trouble> {
        let mut request = events_stream(motion);
        if let Some(id) = last_id {
            request.last_event_id = Some(id.to_owned());
        }
        let (head, body) = self.client.open(&request, within).map_err(Trouble::Http)?;
        Self::checked(head, body)
    }

    /// Opens a camera's stills stream; the body is the multipart stream and `head` names its boundary.
    pub fn open_stills(
        &self,
        camera: &str,
        w: u32,
        fps: u32,
    ) -> Result<(Head, BodyReader), Trouble> {
        let (head, body) = self
            .client
            .open(&stills(camera, w, fps), STILLS_SILENCE)
            .map_err(Trouble::Http)?;
        if head.status != 200 {
            return Err(Self::refusal(&head, body));
        }
        Ok((head, body))
    }

    fn checked(head: Head, body: BodyReader) -> Result<BodyReader, Trouble> {
        if head.status != 200 {
            return Err(Self::refusal(&head, body));
        }
        Ok(body)
    }

    fn refusal(head: &Head, mut body: BodyReader) -> Trouble {
        use std::io::Read;
        let mut text = Vec::new();
        let _ = (&mut body).take(8192).read_to_end(&mut text);
        json_of(head.status, &text)
            .err()
            .unwrap_or(Trouble::Bridge {
                status: head.status,
                code: String::new(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_path_has_its_own_limits_the_cpu_path_the_low_stream_and_the_full_view_the_decoders() {
        let low = accept_low();
        assert_eq!(
            (low["codecs"].clone(), low["max_width"].clone(), low["max_height"].clone(), low["max_fps"].clone()),
            (json!(["h264"]), json!(1280), json!(720), json!(30))
        );
        // a window's pop-up is limited by `popup_stream::Step`, the full view by what the decoder takes
        assert_eq!(accept()["max_width"], 4096, "the full view is up to what the decoder takes");
        // beside an app's video the full view is on the sub window, whose limits a live stream, a recording and a clip all keep to
        let sub = accept_sub();
        assert_eq!((sub["max_width"].clone(), sub["max_height"].clone(), sub["max_fps"].clone()), (json!(1920), json!(1088), json!(30)));
        for request in [live_within("273", "native", sub.clone()), playback_within("273", 5, 1, sub.clone()), clip_playback_within("273", "c1", sub.clone())] {
            let body: Value = serde_json::from_str(request.json.as_deref().unwrap()).unwrap();
            assert_eq!(body["accept"], sub, "{}", request.path);
        }
        assert_eq!(serde_json::from_str::<Value>(playback("273", 5, 1).json.as_deref().unwrap()).unwrap()["accept"], accept(), "the main window's are as they were");
        let body: Value = serde_json::from_str(live_within("273", "light", accept_low()).json.as_deref().unwrap()).unwrap();
        assert_eq!((body["camera"].as_str(), body["quality"].as_str(), body["accept"]["max_height"].clone()), (Some("273"), Some("light"), json!(720)));
    }

    #[test]
    fn requests_encode_every_id_and_keep_to_the_bridges_limits() {
        assert_eq!(encode("a b/c?d=é"), "a%20b%2Fc%3Fd%3D%C3%A9");
        assert_eq!(
            recordings("27 3", 0, 100 * 3_600_000).path,
            format!(
                "/v1/recordings?camera=27%203&from={}&to={}",
                52 * 3_600_000,
                100 * 3_600_000
            )
        );
        assert!(events("1", 0, 10, 9999).path.ends_with("count=500"));
        assert!(clips("1", 0, 10, 0).path.ends_with("count=1"));
        assert_eq!(
            thumbnail("1", 5, 1, 9999).path,
            "/v1/thumbnail?camera=1&time=5&w=16&h=720"
        );
        assert_eq!(
            clip_thumbnail("1", "c1.W/x", 320, 180).path,
            "/v1/clip-thumbnail?camera=1&id=c1.W%2Fx&w=320&h=180"
        );
        assert_eq!(
            event_thumbnail("e4021", 560, 315).path,
            "/v1/event-thumbnail?id=e4021&w=560&h=315"
        );
        assert_eq!(
            stills("273", 50, 99).path,
            "/v1/stills?camera=273&w=160&fps=10"
        );
        assert_eq!(
            still("273", 480, 270).path,
            "/v1/still?camera=273&w=480&h=270"
        );
        assert_eq!(cameras().path, "/v1/cameras?scope=all");
        assert!(
            events_stream(true).path.contains("kinds=")
                && events_stream(true).path.ends_with("motion")
        );
        assert_eq!(events_stream(false).path, "/v1/events/stream");
        let live = live_within("273", "native", accept());
        assert_eq!((live.method, live.path.as_str()), ("POST", "/v1/live"));
        let body: Value = serde_json::from_str(live.json.as_deref().unwrap()).unwrap();
        assert_eq!(body["camera"], "273");
        assert_eq!(body["accept"]["max_width"], 4096);
        assert_eq!(body["accept"]["codecs"], json!(["h264", "h265"]));
        let play: Value =
            serde_json::from_str(playback("273", 99, 2).json.as_deref().unwrap()).unwrap();
        assert_eq!(
            (
                play["start"].as_i64(),
                play["rate"].as_u64(),
                play["wait_ms"].as_u64()
            ),
            (Some(99), Some(2), Some(3500))
        );
        assert_eq!(play["accept"]["max_fps"], 60);
        let clip: Value =
            serde_json::from_str(clip_playback("273", "c1.x").json.as_deref().unwrap()).unwrap();
        assert_eq!(clip["id"], "c1.x");
    }

    #[test]
    fn the_cameras_are_read_with_their_streams_and_a_v1_list_still_reads() {
        let v = json!({"cameras": [
            {"id": "273", "name": "Front Door", "room": "Entry", "type": "doorbell", "online": true,
             "features": ["live", "stills", "recordings", "events"], "earliest": 1760500000000i64,
             "native": {"codec": "H265", "width": 3840, "height": 2160, "fps": 15},
             "streams": [{"quality": "native", "codec": "h265", "width": 3840, "height": 2160, "fps": 15, "prebuffered": true},
                         {"quality": "light", "codec": "h264", "width": 640, "height": 360, "fps": 10, "prebuffered": false}]},
            {"id": "274", "features": ["recordings"]},
            {"name": "no id"}
        ]});
        let cameras = cameras_of(&v).unwrap();
        assert_eq!(cameras.len(), 2);
        let door = &cameras[0];
        assert_eq!(
            (
                door.name.as_str(),
                door.room.as_deref(),
                door.kind.as_str(),
                door.online
            ),
            ("Front Door", Some("Entry"), "doorbell", true)
        );
        assert!(door.has("live") && !door.has("clips"));
        assert_eq!(
            door.native
                .as_ref()
                .map(|n| (n.codec.as_str(), n.width, n.height)),
            Some(("h265", 3840, 2160))
        );
        assert!(
            door.stream("native").unwrap().prebuffered
                && !door.stream("light").unwrap().prebuffered
        );
        assert_eq!(door.stream("balanced"), None);
        let bare = &cameras[1];
        assert_eq!(
            (
                bare.name.as_str(),
                bare.kind.as_str(),
                bare.online,
                bare.room.as_deref()
            ),
            ("274", "camera", true, None)
        );
        assert!(bare.native.is_none() && bare.streams.is_empty());
        assert!(cameras_of(&json!({"nope": 1})).is_err());
    }

    #[test]
    fn health_and_the_dials_are_read_and_a_stream_address_is_never_shown() {
        let h = health_of(
            &json!({"ok": true, "api": 1, "now": 1792609200000i64, "cameras": 3, "device": "LumaTV",
                                  "features": {"live": 1, "stills": 1, "devices": 0}}),
        )
        .unwrap();
        assert_eq!(
            (h.api, h.now, h.cameras, h.device.as_deref()),
            (1, 1792609200000, 3, Some("LumaTV"))
        );
        assert!(h.has("live") && !h.has("devices") && !h.has("events"));
        assert!(health_of(&json!({"ok": true})).is_err());
        let live = dial_of(&json!({"rtsp": "rtsp://192.168.1.69:34135/9c1f0e5a2b7d4c36", "camera": "273", "quality": "native",
                                   "video": {"codec": "H265", "width": 3840, "height": 2160, "fps": 14.99}, "prebuffered": true})).unwrap();
        assert_eq!(
            (
                live.codec.as_str(),
                live.width,
                live.height,
                live.fps,
                live.prebuffered
            ),
            ("h265", 3840, 2160, 15, true)
        );
        assert!(!format!("{live:?}").contains("9c1f0e5a"), "{live:?}");
        let play = dial_of(&json!({"session": "s1", "rtsp": "rtsp://192.168.1.69:34567/lumashow/abc", "started_at": 1792605903000i64, "rate": 2,
                                   "video": {"codec": "h264", "width": 1280, "height": 720}})).unwrap();
        assert_eq!(
            (play.session.as_str(), play.started_at, play.rate, play.fps),
            ("s1", 1792605903000, 2, 0)
        );
        assert!(dial_of(&json!({"video": {}})).is_err());
        assert!(
            dial_of(&json!({"rtsp": "http://x/y"})).is_err(),
            "only an RTSP address is dialled"
        );
    }

    #[test]
    fn a_refusal_becomes_the_bridges_word_and_words_for_the_person_without_an_address() {
        let body = br#"{"error": {"code": "no_recording", "message": "nothing there"}}"#;
        let t = json_of(404, body).unwrap_err();
        assert_eq!(
            t,
            Trouble::Bridge {
                status: 404,
                code: "no_recording".into()
            }
        );
        assert_eq!(
            (t.status(), t.code().as_str(), t.words().as_str()),
            (404, "no_recording", "No recording at that time")
        );
        let busy = json_of(429, br#"{"error": {"code": "busy"}}"#).unwrap_err();
        assert_eq!(
            (busy.status(), busy.words().as_str()),
            (429, "Scrypted is busy, try again")
        );
        let offline = json_of(409, br#"{"error": {"code": "offline"}}"#).unwrap_err();
        assert_eq!(
            (offline.status(), offline.words().as_str()),
            (409, "The camera is offline")
        );
        let refused = json_of(401, b"").unwrap_err();
        assert!(refused.refused());
        assert_eq!(
            (
                refused.status(),
                refused.words().as_str(),
                refused.code().as_str()
            ),
            (503, "Scrypted refused this TV", "http_401")
        );
        let late = Trouble::Http(HttpError::Timeout);
        assert_eq!(
            (late.status(), late.code().as_str(), late.unreachable()),
            (504, "timeout", true)
        );
        let untrusted = Trouble::Http(HttpError::Connect(ConnectError::Untrusted));
        assert!(untrusted.untrusted() && !untrusted.unreachable());
        // a hung-up handshake and a reply that is not HTTP are the bridge not answering, a certificate that is not the pinned one is not
        assert!(Trouble::Http(HttpError::Connect(ConnectError::Tls("eof".into()))).unreachable());
        assert!(Trouble::Http(HttpError::Protocol("no".into())).unreachable());
        assert!(!busy.unreachable() && !refused.unreachable());
        assert!(json_of(200, b"not json").is_err());
        assert_eq!(json_of(200, br#"{"a": 1}"#).unwrap()["a"], 1);
        for t in [t, busy, offline, refused, late, untrusted] {
            let shown = format!("{} {}", t.words(), t.code());
            assert!(
                !shown.contains("rtsp://") && !shown.contains("192.168"),
                "{shown}"
            );
        }
    }

    fn listed(streams: &[(&str, &str, u32, u32)]) -> Camera {
        let streams: Vec<Stream> = streams
            .iter()
            .map(|(quality, codec, width, height)| Stream { quality: (*quality).into(), codec: (*codec).into(), width: *width, height: *height, fps: 0.0, prebuffered: true })
            .collect();
        Camera {
            id: "104".into(),
            name: "Front Door".into(),
            room: None,
            kind: "doorbell".into(),
            online: true,
            features: vec!["live".into()],
            earliest: None,
            native: streams.first().cloned(),
            streams,
        }
    }

    #[test]
    fn the_full_view_asks_for_the_main_stream_with_the_decoders_box_turned_for_a_portrait_one_and_no_codec_list_for_one_that_names_none() {
        // the Reolink doorbell as Scrypted lists it: a portrait main stream that names no codec, low streams declared landscape
        let door = listed(&[("native", "", 1920, 2560), ("balanced", "", 896, 672), ("light", "", 640, 480)]);
        let main = accept_full(Some(&door), "native");
        assert_eq!((main["max_width"].clone(), main["max_height"].clone(), main["max_fps"].clone()), (json!(2304), json!(4096), json!(60)));
        assert!(main.get("codecs").is_none(), "{main}");
        let body: Value = serde_json::from_str(live_within("104", "native", main).json.as_deref().unwrap()).unwrap();
        assert_eq!((body["camera"].as_str(), body["quality"].as_str(), body["accept"]["max_height"].clone()), (Some("104"), Some("native"), json!(4096)));
        // the low stream is landscape: the box stays as it is, and the codec list still goes, as the listing names none for it either
        let light = accept_full(Some(&door), "light");
        assert_eq!((light["max_width"].clone(), light["max_height"].clone()), (json!(4096), json!(2304)));
        assert!(light.get("codecs").is_none());
        // a camera whose main stream names its codec and is landscape is asked as it always was
        let wide = listed(&[("native", "h264", 2688, 1520), ("balanced", "h264", 1280, 720)]);
        assert_eq!(accept_full(Some(&wide), "native"), accept());
        // nothing listed for the camera or the quality: nothing to turn or trim
        assert_eq!(accept_full(None, "native"), accept());
        assert_eq!(accept_full(Some(&wide), "light"), accept());
        // a size the listing does not know is no reason to turn the box
        let bare = listed(&[("native", "", 0, 0)]);
        assert_eq!(accept_full(Some(&bare), "native")["max_width"], json!(4096));
    }
}
