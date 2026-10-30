//! Pictures of the TV's inputs for the shell's input picker: the picture an input shows now, and the last one it showed.
//!
//! The shell asks (`POST /v1/previews/capture`, with the input's key and why it asks); this module takes the picture through LG's capture
//! service, as `tools/tv-gate.sh` does (`luna-send ... com.webos.service.capture/executeOneShot`, method `VIDEO`: the video plane alone, so
//! LumaTV's own sheet is never in it), keeps it small and serves it (`GET /v1/previews/{key}`). The shell is not given the right to call the
//! capture service itself (the compositor's list of calls the shell may make does not include it): the one door to the video picture is
//! here, behind the owner's switch.
//!
//! Rules this keeps:
//!
//! - **The owner's switch decides.** `inputs.previews` of theme.json (Settings > Inputs & devices > Show input previews, on by default). Off:
//!   nothing is captured, nothing is served, and what was kept is deleted when the shell says so (`DELETE /v1/previews`).
//! - **Nothing protected is read, and nothing black is kept.** The capture service refuses or blanks what is protected; whatever comes back
//!   as a black picture or as an error is not a picture of the input, and the picture kept before stays (the shell then shows it with its
//!   age, or the input's icon when there is none). Nothing here touches the video path, HDCP or any DRM mechanism.
//! - **Pictures stay on the TV.** One small JPEG per input in `previews/` (the folder 0700, the files 0600), overwritten by the next, and
//!   forgotten after `previews.keepDays`. The raw capture is a file in `/tmp/lumatv-previews/` that is deleted as soon as it is read. Nothing
//!   is uploaded; the route answers on loopback only like every other.
//! - **Cheap.** One capture at a time, not sooner than `previews.minGapMs` after the last, and after three failures in a row none for a
//!   minute (doubling to ten). A picture taken for the live tile (`live`) is kept in memory and goes to the flash at most every 30 s; the
//!   others (`open`, `arrive`, `periodic`, `standby`) are written when taken.
//!
//! | Request | Answer |
//! |---|---|
//! | `GET /v1/previews` | `{previews: {enabled, items: {key: {at, w, h, bytes}}, size, method, stats}}` |
//! | `GET /v1/previews/{key}` | the JPEG (`X-Luma-At`: when it was taken, in epoch milliseconds); 404 `no_picture`, 403 `previews_disabled` |
//! | `POST /v1/previews/capture` `{key, why}` | `{key, at, w, h, bytes, saved, ms}`; failures: 400 `bad_request`, 403 `previews_disabled`, 409 `screen_off` or `no_picture` (black, protected, no signal), 429 `busy` or `too_soon`, 502 `capture_failed`, 503 `backing_off`, 504 `timeout` |
//! | `DELETE /v1/previews[/{key}]` | `{cleared}`: forgets every picture (or one) |

use crate::API_VERSION;
use crate::config::{Config, Paths, PreviewsConfig};
use crate::http::{Request, Response, error_response};
use crate::hub::Hub;
use crate::timeutil::now_ms;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const ROUTES: &[(&str, &str, &str)] = &[
    (
        "GET",
        "/v1/previews",
        "whether input pictures are on and what is kept: per input when its picture was taken and its size, and what the captures cost",
    ),
    (
        "GET",
        "/v1/previews/{key}",
        "the last picture of an input (livetv, hdmi1 ...) as a small JPEG; 404 where there is none",
    ),
    (
        "POST",
        "/v1/previews/capture",
        "takes a picture of the input in front through LG's capture service (video plane only) and keeps it; refused while the owner's switch is off",
    ),
    (
        "DELETE",
        "/v1/previews",
        "forgets every kept picture (or one input's, at /v1/previews/{key})",
    ),
];

/// LG's capture service.
const URI: &str = "luna://com.webos.service.capture/executeOneShot";
/// A live picture goes to the flash at most this often; every other reason writes at once.
const SAVE_EVERY_MS: u64 = 30_000;
/// The largest raw capture read, and the most pixels it may decode to.
const MAX_FILE: u64 = 24 * 1024 * 1024;
const MAX_PIXELS: u64 = 4096 * 2304;
/// After three failures in a row the service is left alone for a minute, then two, four... up to ten.
const FAILS_BEFORE_BACKOFF: u32 = 3;
const BACKOFF_FIRST: Duration = Duration::from_secs(60);
const BACKOFF_MAX: Duration = Duration::from_secs(600);
/// A picture with fewer than this share of its pixels above `BRIGHT` is a black one: protected, no signal, or nothing playing.
const BLANK_SHARE: f32 = 0.005;
const BRIGHT: u8 = 16;
/// The largest stored picture read back from the flash.
const MAX_STORED: u64 = 512 * 1024;

/// The inputs a picture may be kept for, by the key the picker uses: Live TV and the HDMI ports.
pub fn key_ok(key: &str) -> bool {
    key == "livetv"
        || key
            .strip_prefix("hdmi")
            .is_some_and(|d| d.len() == 1 && matches!(d.as_bytes()[0], b'1'..=b'9'))
}

/// Why the shell asks: `live` is the tile of the input in front while the picker is open (kept in memory, flash at most every 30 s); the
/// others are the picker coming up, an input coming to the front, the quiet timer and the screen going off (written at once).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Why {
    Live,
    Open,
    Arrive,
    Periodic,
    Standby,
}

impl Why {
    fn parse(s: &str) -> Option<Why> {
        Some(match s {
            "live" => Why::Live,
            "open" => Why::Open,
            "arrive" => Why::Arrive,
            "periodic" => Why::Periodic,
            "standby" => Why::Standby,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Why::Live => "live",
            Why::Open => "open",
            Why::Arrive => "arrive",
            Why::Periodic => "periodic",
            Why::Standby => "standby",
        }
    }
}

/// The capture service as the module asks it. The real one is `luna-send`; a test stands in a program that writes a picture.
pub trait Camera: Send + Sync {
    /// Asks for the picture of `method` at `width` by `height` as a PNG at `path`; the service's own answer, or why there is none.
    fn shoot(
        &self,
        path: &Path,
        method: &str,
        width: u32,
        height: u32,
        deadline: Duration,
    ) -> Result<Value, Failure>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    /// The program could not be run, or exited badly.
    Run(String),
    /// It did not answer in time.
    Timeout,
}

/// `luna-send -n 1 -w <ms> luna://com.webos.service.capture/executeOneShot '{...}'`, the call the gate makes by hand.
pub struct LunaCamera {
    program: String,
}

impl LunaCamera {
    pub fn new(program: &str) -> Self {
        Self {
            program: program.to_owned(),
        }
    }
}

impl Camera for LunaCamera {
    fn shoot(
        &self,
        path: &Path,
        method: &str,
        width: u32,
        height: u32,
        deadline: Duration,
    ) -> Result<Value, Failure> {
        let payload = json!({
            "path": path.to_string_lossy(),
            "method": method,
            "format": "PNG",
            "width": width,
            "height": height,
        })
        .to_string();
        let mut child = Command::new(&self.program)
            .args([
                "-n",
                "1",
                "-w",
                &deadline.as_millis().to_string(),
                URI,
                &payload,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| Failure::Run(format!("could not run {}: {e}", self.program)))?;
        let end = Instant::now() + deadline + Duration::from_millis(500);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() >= end => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Failure::Timeout);
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => return Err(Failure::Run(e.to_string())),
            }
        }
        // the answer is a line or two of JSON, far under a pipe's buffer, so it waited there
        let mut text = String::new();
        if let Some(mut out) = child.stdout.take() {
            let _ = out.by_ref().take(64 * 1024).read_to_string(&mut text);
        }
        first_object(&text).ok_or_else(|| Failure::Run("luna-send gave no answer".to_owned()))
    }
}

/// The first JSON object in `luna-send`'s output (one object per reply, compact or pretty-printed).
fn first_object(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    serde_json::Deserializer::from_str(&text[start..])
        .into_iter::<Value>()
        .next()?
        .ok()
}

/// A picture as 8-bit RGB.
struct Rgb {
    w: u32,
    h: u32,
    px: Vec<u8>,
}

/// Decodes a PNG (any colour type, alpha dropped) into RGB.
fn decode_png(bytes: &[u8]) -> Result<Rgb, String> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| format!("png: {e}"))?;
    let (w, h) = reader.info().size();
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(format!("a picture of {w} by {h}"));
    }
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("png: {e}"))?;
    let n = (info.width * info.height) as usize;
    let px: Vec<u8> = match info.color_type {
        png::ColorType::Rgb => buf[..n * 3].to_vec(),
        png::ColorType::Rgba => buf[..n * 4]
            .chunks_exact(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect(),
        png::ColorType::Grayscale => buf[..n].iter().flat_map(|&g| [g, g, g]).collect(),
        png::ColorType::GrayscaleAlpha => buf[..n * 2]
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0]])
            .collect(),
        png::ColorType::Indexed => return Err("png: an indexed picture was not expanded".into()),
    };
    Ok(Rgb {
        w: info.width,
        h: info.height,
        px,
    })
}

/// How much of a picture is lit: the share of its pixels brighter than `BRIGHT` (a luma of 16 in 255).
fn lit_share(p: &Rgb) -> f32 {
    let lit =
        p.px.chunks_exact(3)
            .filter(|c| {
                (u32::from(c[0]) * 54 + u32::from(c[1]) * 183 + u32::from(c[2]) * 19) >> 8
                    > u32::from(BRIGHT)
            })
            .count();
    lit as f32 / (p.px.len() / 3).max(1) as f32
}

/// A picture with nothing lit in it is not a picture of the input.
fn is_blank(p: &Rgb) -> bool {
    lit_share(p) < BLANK_SHARE
}

/// `p` shrunk to fit inside `w` by `h`, the shape kept; the mean of the source under each pixel. Never made bigger.
fn shrink(p: Rgb, w: u32, h: u32) -> Rgb {
    let scale = (w as f32 / p.w as f32).min(h as f32 / p.h as f32);
    if scale >= 1.0 {
        return p;
    }
    let (tw, th) = (
        ((p.w as f32 * scale).round() as u32).max(1),
        ((p.h as f32 * scale).round() as u32).max(1),
    );
    let mut out = vec![0u8; (tw * th * 3) as usize];
    let (sx, sy) = (p.w as f32 / tw as f32, p.h as f32 / th as f32);
    for y in 0..th {
        let (y0, y1) = (y as f32 * sy, ((y + 1) as f32 * sy).min(p.h as f32));
        for x in 0..tw {
            let (x0, x1) = (x as f32 * sx, ((x + 1) as f32 * sx).min(p.w as f32));
            let mut acc = [0f32; 3];
            let mut weight = 0.0;
            let mut row = y0.floor();
            while row < y1 {
                let wy = (row + 1.0).min(y1) - row.max(y0);
                let mut col = x0.floor();
                while col < x1 {
                    let wx = (col + 1.0).min(x1) - col.max(x0);
                    let i =
                        ((row as u32).min(p.h - 1) * p.w + (col as u32).min(p.w - 1)) as usize * 3;
                    for c in 0..3 {
                        acc[c] += f32::from(p.px[i + c]) * wx * wy;
                    }
                    weight += wx * wy;
                    col += 1.0;
                }
                row += 1.0;
            }
            let o = ((y * tw + x) * 3) as usize;
            for c in 0..3 {
                out[o + c] = (acc[c] / weight.max(f32::EPSILON)).round() as u8;
            }
        }
    }
    Rgb {
        w: tw,
        h: th,
        px: out,
    }
}

fn encode_jpeg(p: &Rgb, quality: u8) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(32 * 1024);
    let mut encoder = jpeg_encoder::Encoder::new(&mut out, quality);
    encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_2);
    encoder
        .encode(&p.px, p.w as u16, p.h as u16, jpeg_encoder::ColorType::Rgb)
        .map_err(|e| format!("jpeg: {e}"))?;
    Ok(out)
}

/// One kept picture: the JPEG, when it was taken (epoch milliseconds) and its size.
#[derive(Clone)]
struct Shot {
    at: u64,
    w: u32,
    h: u32,
    jpeg: Arc<Vec<u8>>,
}

/// What the captures have cost, for a TV check to read.
#[derive(Default, Clone)]
struct Stats {
    taken: u64,
    kept: u64,
    blank: u64,
    failed: u64,
    ms_total: u64,
    ms_max: u64,
    last: Option<Value>,
}

#[derive(Default)]
struct State {
    loaded: bool,
    shots: BTreeMap<String, Shot>,
    /// When each key's picture last went to the flash.
    saved_at: BTreeMap<String, u64>,
    busy: bool,
    last_start: Option<Instant>,
    failures: u32,
    backoff: Duration,
    backoff_until: Option<Instant>,
    stats: Stats,
}

pub struct Previews {
    config: PreviewsConfig,
    dir: PathBuf,
    theme: PathBuf,
    scratch: PathBuf,
    hub: Arc<Hub>,
    camera: Box<dyn Camera>,
    state: Mutex<State>,
}

/// Frees `busy` however a capture ends.
struct Busy<'a>(&'a Mutex<State>);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.lock().busy = false;
    }
}

fn ok(mut body: Value) -> Response {
    if let Some(map) = body.as_object_mut() {
        map.insert("ok".into(), Value::Bool(true));
        map.insert("api".into(), json!(API_VERSION));
        map.insert("now".into(), json!(now_ms()));
    }
    Response::json(200, &body)
}

/// Writes `bytes` to `path` as a file only its owner reads: a temporary beside it, then a rename.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(bytes)?;
    }
    fs::rename(&tmp, path)
}

impl Previews {
    pub fn new(config: &Config, paths: &Paths, hub: Arc<Hub>) -> Self {
        Self::with_camera(
            config,
            paths,
            hub,
            Box::new(LunaCamera::new(&config.luna.send)),
            std::env::temp_dir().join("lumatv-previews"),
        )
    }

    /// The module with another camera (a test's) and another scratch folder for the raw captures.
    pub fn with_camera(
        config: &Config,
        paths: &Paths,
        hub: Arc<Hub>,
        camera: Box<dyn Camera>,
        scratch: PathBuf,
    ) -> Self {
        Self {
            config: config.previews.clone(),
            dir: paths.previews_dir(),
            theme: paths.theme_file(),
            scratch,
            hub,
            camera,
            state: Mutex::new(State::default()),
        }
    }

    /// The owner's switch: `inputs.previews` of theme.json, on unless it says `false`.
    fn enabled(&self) -> bool {
        fs::read(&self.theme)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v.pointer("/inputs/previews").and_then(Value::as_bool))
            != Some(false)
    }

    fn index_file(&self) -> PathBuf {
        self.dir.join("index.json")
    }

    fn private_dir(&self, dir: &Path) -> std::io::Result<()> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }

    /// Reads what an earlier run kept, once: the index, and each picture it dates that is still whole and not too old.
    fn load(&self, state: &mut State) {
        if state.loaded {
            return;
        }
        state.loaded = true;
        let Some(doc) = fs::read(self.index_file())
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        else {
            return;
        };
        let oldest = now_ms().saturating_sub(u64::from(self.config.keep_days) * 86_400_000);
        let mut dropped = false;
        for (key, item) in doc
            .get("items")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let file = self.dir.join(format!("{key}.jpg"));
            let at = item.get("at").and_then(Value::as_u64).unwrap_or(0);
            let bytes = fs::metadata(&file)
                .ok()
                .filter(|m| m.is_file() && m.len() <= MAX_STORED)
                .and_then(|_| fs::read(&file).ok());
            match bytes.as_deref().and_then(crate::images::jpeg_size) {
                Some((w, h)) if key_ok(key) && at >= oldest => {
                    state.saved_at.insert(key.clone(), at);
                    state.shots.insert(
                        key.clone(),
                        Shot {
                            at,
                            w,
                            h,
                            jpeg: Arc::new(bytes.unwrap_or_default()),
                        },
                    );
                }
                _ => {
                    let _ = fs::remove_file(&file);
                    dropped = true;
                }
            }
        }
        if dropped {
            self.write_index(state);
        }
    }

    fn write_index(&self, state: &State) {
        let items: serde_json::Map<String, Value> = state
            .shots
            .iter()
            .filter(|(k, _)| state.saved_at.contains_key(*k))
            .map(|(k, s)| {
                let at = state.saved_at.get(k).copied().unwrap_or(s.at);
                (k.clone(), json!({"at": at, "w": s.w, "h": s.h}))
            })
            .collect();
        let doc = json!({"v": 1, "items": items});
        if let Err(e) = self
            .private_dir(&self.dir)
            .and_then(|()| write_private(&self.index_file(), doc.to_string().as_bytes()))
        {
            crate::log_warn!("previews: the index could not be written: {e}");
        }
    }

    fn save(&self, state: &mut State, key: &str, shot: &Shot) {
        let wrote = self
            .private_dir(&self.dir)
            .and_then(|()| write_private(&self.dir.join(format!("{key}.jpg")), &shot.jpeg));
        match wrote {
            Ok(()) => {
                state.saved_at.insert(key.to_owned(), shot.at);
                self.write_index(state);
            }
            Err(e) => crate::log_warn!("previews: {key} could not be written: {e}"),
        }
    }

    fn clear(&self, key: Option<&str>) -> usize {
        let mut state = self.state.lock();
        self.load(&mut state);
        let keys: Vec<String> = match key {
            Some(k) => vec![k.to_owned()],
            None => state.shots.keys().cloned().collect(),
        };
        let mut n = 0;
        for k in &keys {
            n += usize::from(state.shots.remove(k).is_some());
            state.saved_at.remove(k);
            let _ = fs::remove_file(self.dir.join(format!("{k}.jpg")));
        }
        if key.is_none() {
            // pictures that an earlier run left and this one never loaded go too
            if let Ok(list) = fs::read_dir(&self.dir) {
                for e in list.flatten() {
                    let name = e.file_name();
                    if name.to_string_lossy().ends_with(".jpg") {
                        let _ = fs::remove_file(e.path());
                    }
                }
            }
        }
        if self.dir.exists() {
            self.write_index(&state);
        }
        n
    }

    pub fn handle(&self, request: &Request, rest: &[&str]) -> Response {
        match (request.method.as_str(), rest) {
            ("GET", []) => self.status(),
            ("POST", ["capture"]) => self.capture(request),
            ("GET", [key]) if *key != "capture" => self.picture(key),
            ("DELETE", []) => ok(json!({"cleared": self.clear(None)})),
            ("DELETE", [key]) if key_ok(key) => ok(json!({"cleared": self.clear(Some(key))})),
            (_, []) => error_response(405, "method_not_allowed", "that method is not served here")
                .with_header("Allow", "GET, DELETE"),
            (_, ["capture"]) => {
                error_response(405, "method_not_allowed", "that method is not served here")
                    .with_header("Allow", "POST")
            }
            (_, [key]) if key_ok(key) => {
                error_response(405, "method_not_allowed", "that method is not served here")
                    .with_header("Allow", "GET, DELETE")
            }
            _ => error_response(404, "not_found", "no such endpoint"),
        }
    }

    fn status(&self) -> Response {
        let enabled = self.enabled();
        let mut state = self.state.lock();
        self.load(&mut state);
        let items: serde_json::Map<String, Value> = state
            .shots
            .iter()
            .map(|(k, s)| {
                (
                    k.clone(),
                    json!({"at": s.at, "w": s.w, "h": s.h, "bytes": s.jpeg.len()}),
                )
            })
            .collect();
        let s = &state.stats;
        ok(json!({"previews": {
            "enabled": enabled,
            "items": if enabled { Value::Object(items) } else { json!({}) },
            "size": [self.config.width, self.config.height],
            "method": self.config.method,
            "stats": {
                "taken": s.taken,
                "kept": s.kept,
                "blank": s.blank,
                "failed": s.failed,
                "msAvg": s.ms_total.checked_div(s.taken + s.blank + s.failed).unwrap_or(0),
                "msMax": s.ms_max,
                "last": s.last,
            },
        }}))
    }

    fn picture(&self, key: &str) -> Response {
        if !key_ok(key) {
            return error_response(404, "no_picture", "there is no such input");
        }
        if !self.enabled() {
            return error_response(
                403,
                "previews_disabled",
                "input previews are off (Settings > Inputs & devices)",
            );
        }
        let mut state = self.state.lock();
        self.load(&mut state);
        match state.shots.get(key) {
            Some(shot) => {
                let mut r = Response {
                    status: 200,
                    headers: Vec::new(),
                    body: shot.jpeg.as_ref().clone(),
                };
                r = r
                    .with_header("Content-Type", "image/jpeg")
                    .with_header("Cache-Control", "no-store")
                    .with_header("X-Luma-At", shot.at.to_string());
                r
            }
            None => error_response(404, "no_picture", "no picture of this input has been kept"),
        }
    }

    fn capture(&self, request: &Request) -> Response {
        let doc: Value = match serde_json::from_slice(&request.body) {
            Ok(v) => v,
            Err(_) => return error_response(400, "bad_request", "the body is JSON: {key, why}"),
        };
        let key = doc.get("key").and_then(Value::as_str).unwrap_or("");
        let why = doc.get("why").and_then(Value::as_str).and_then(Why::parse);
        let (true, Some(why)) = (key_ok(key), why) else {
            return error_response(
                400,
                "bad_request",
                "key is livetv or hdmi1 to hdmi9; why is live, open, arrive, periodic or standby",
            );
        };
        if !self.enabled() {
            return error_response(
                403,
                "previews_disabled",
                "input previews are off (Settings > Inputs & devices)",
            );
        }
        if why != Why::Standby && !crate::screen::is_on(&self.hub) {
            return error_response(409, "screen_off", "the screen is off");
        }
        {
            let mut state = self.state.lock();
            let now = Instant::now();
            if state.busy {
                return error_response(429, "busy", "a picture is being taken");
            }
            if let Some(until) = state.backoff_until.filter(|u| *u > now) {
                return error_response(
                    503,
                    "backing_off",
                    &format!(
                        "the capture service failed {} times in a row; not asked again for {} s",
                        state.failures,
                        (until - now).as_secs() + 1
                    ),
                );
            }
            if state.last_start.is_some_and(|t| {
                now.duration_since(t) < Duration::from_millis(self.config.min_gap_ms)
            }) {
                return error_response(429, "too_soon", "the last picture was taken a moment ago");
            }
            state.busy = true;
            state.last_start = Some(now);
        }
        let _busy = Busy(&self.state);
        let started = Instant::now();
        let outcome = self.take(key);
        let ms = started.elapsed().as_millis() as u64;
        let mut state = self.state.lock();
        self.load(&mut state);
        let stats = &mut state.stats;
        stats.ms_total += ms;
        stats.ms_max = stats.ms_max.max(ms);
        match outcome {
            Ok(Taken::Picture(rgb)) => {
                let (w, h) = (rgb.w, rgb.h);
                let jpeg = match encode_jpeg(&rgb, self.config.quality) {
                    Ok(j) => j,
                    Err(e) => {
                        state.stats.failed += 1;
                        state.stats.last = Some(
                            json!({"key": key, "why": why.name(), "code": "capture_failed", "ms": ms}),
                        );
                        crate::log_warn!("previews: {key}: {e}");
                        return error_response(502, "capture_failed", &e);
                    }
                };
                state.failures = 0;
                state.backoff = Duration::ZERO;
                state.backoff_until = None;
                let shot = Shot {
                    at: now_ms(),
                    w,
                    h,
                    jpeg: Arc::new(jpeg),
                };
                let due = state
                    .saved_at
                    .get(key)
                    .is_none_or(|t| shot.at.saturating_sub(*t) >= SAVE_EVERY_MS);
                let saved = why != Why::Live || due;
                state.shots.insert(key.to_owned(), shot.clone());
                if saved {
                    self.save(&mut state, key, &shot);
                    state.stats.kept += 1;
                }
                state.stats.taken += 1;
                state.stats.last = Some(
                    json!({"key": key, "why": why.name(), "ok": true, "ms": ms, "bytes": shot.jpeg.len()}),
                );
                ok(
                    json!({"key": key, "at": shot.at, "w": w, "h": h, "bytes": shot.jpeg.len(), "saved": saved, "ms": ms}),
                )
            }
            Ok(Taken::Blank) => {
                // the service answered, with nothing in it: a good answer, not a failure of the service
                state.failures = 0;
                state.stats.blank += 1;
                state.stats.last =
                    Some(json!({"key": key, "why": why.name(), "code": "no_picture", "ms": ms}));
                error_response(
                    409,
                    "no_picture",
                    "the picture is black: nothing playing, no signal, or a picture that is protected",
                )
            }
            Ok(Taken::Refused(text)) => {
                state.failures = 0;
                state.stats.blank += 1;
                state.stats.last = Some(
                    json!({"key": key, "why": why.name(), "code": "no_picture", "ms": ms, "text": text}),
                );
                error_response(
                    409,
                    "no_picture",
                    &format!("LG's capture service gave no picture: {text}"),
                )
            }
            Err((code, status, text)) => {
                state.stats.failed += 1;
                state.stats.last = Some(
                    json!({"key": key, "why": why.name(), "code": code, "ms": ms, "text": text}),
                );
                state.failures += 1;
                if state.failures >= FAILS_BEFORE_BACKOFF {
                    state.backoff = (state.backoff * 2).clamp(BACKOFF_FIRST, BACKOFF_MAX);
                    state.backoff_until = Some(Instant::now() + state.backoff);
                    crate::log_warn!(
                        "previews: {} failures in a row ({text}); the capture service is left alone for {} s",
                        state.failures,
                        state.backoff.as_secs()
                    );
                }
                error_response(status, code, &text)
            }
        }
    }

    /// One capture through the camera: the raw file read and deleted, decoded, checked for black, shrunk to the size kept.
    fn take(&self, key: &str) -> Result<Taken, (&'static str, u16, String)> {
        self.private_dir(&self.scratch).map_err(|e| {
            (
                "capture_failed",
                502,
                format!("{}: {e}", self.scratch.display()),
            )
        })?;
        let path = self.scratch.join(format!("{key}.png"));
        let _ = fs::remove_file(&path);
        let reply = self.camera.shoot(
            &path,
            &self.config.method,
            self.config.width,
            self.config.height,
            Duration::from_millis(self.config.deadline_ms),
        );
        let read = |p: &Path| -> Result<Vec<u8>, String> {
            let meta = fs::metadata(p).map_err(|e| format!("{}: {e}", p.display()))?;
            if !meta.is_file() || meta.len() > MAX_FILE {
                return Err("the capture is not a file this reads".to_owned());
            }
            fs::read(p).map_err(|e| e.to_string())
        };
        let reply = match reply {
            Ok(r) => r,
            Err(Failure::Timeout) => {
                let _ = fs::remove_file(&path);
                return Err((
                    "timeout",
                    504,
                    "LG's capture service did not answer in time".to_owned(),
                ));
            }
            Err(Failure::Run(e)) => {
                let _ = fs::remove_file(&path);
                return Err(("capture_failed", 502, e));
            }
        };
        if reply.get("returnValue").and_then(Value::as_bool) == Some(false) {
            let _ = fs::remove_file(&path);
            let text = reply
                .get("errorText")
                .and_then(Value::as_str)
                .unwrap_or("refused")
                .to_owned();
            // a refusal for no signal or for protection is the answer for those pictures; anything else is a fault
            return if text.contains("REFUSED") || text.contains("NO_SIGNAL") {
                Ok(Taken::Refused(text))
            } else {
                Err(("capture_failed", 502, text))
            };
        }
        let bytes = read(&path);
        let _ = fs::remove_file(&path);
        let bytes = bytes.map_err(|e| ("capture_failed", 502, e))?;
        let rgb = decode_png(&bytes).map_err(|e| ("capture_failed", 502, e))?;
        if is_blank(&rgb) {
            return Ok(Taken::Blank);
        }
        Ok(Taken::Picture(shrink(
            rgb,
            self.config.width,
            self.config.height,
        )))
    }
}

enum Taken {
    Picture(Rgb),
    Blank,
    Refused(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// What the fake camera does on its next shot.
    #[derive(Clone)]
    enum Next {
        Picture(u32, u32),
        Black,
        Refused(&'static str),
        Error(&'static str),
        Hang,
    }

    struct Fake {
        next: Arc<Mutex<Next>>,
        shots: Arc<AtomicUsize>,
        seen: Arc<Mutex<Vec<(String, u32, u32)>>>,
    }

    fn png_of(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        let mut data = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&f(x, y));
            }
        }
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            wr.write_image_data(&data).unwrap();
        }
        out
    }

    impl Camera for Fake {
        fn shoot(
            &self,
            path: &Path,
            method: &str,
            width: u32,
            height: u32,
            _deadline: Duration,
        ) -> Result<Value, Failure> {
            self.shots.fetch_add(1, Ordering::SeqCst);
            self.seen.lock().push((method.to_owned(), width, height));
            match self.next.lock().clone() {
                Next::Picture(w, h) => {
                    // a sunset-ish gradient so the picture has something lit in it
                    let png = png_of(w, h, |x, y| [(x * 255 / w) as u8, (y * 200 / h) as u8, 90]);
                    fs::write(path, png).unwrap();
                    Ok(
                        json!({"returnValue": true, "capturedWidth": w, "capturedHeight": h, "writtenBytes": 1}),
                    )
                }
                Next::Black => {
                    fs::write(path, png_of(width, height, |_, _| [0, 0, 0])).unwrap();
                    Ok(
                        json!({"returnValue": true, "capturedWidth": width, "capturedHeight": height, "writtenBytes": 1}),
                    )
                }
                Next::Refused(why) => {
                    Ok(json!({"returnValue": false, "errorCode": 3, "errorText": why}))
                }
                Next::Error(why) => Err(Failure::Run(why.to_owned())),
                Next::Hang => Err(Failure::Timeout),
            }
        }
    }

    struct Rig {
        dir: tempfile::TempDir,
        previews: Previews,
        next: Arc<Mutex<Next>>,
        shots: Arc<AtomicUsize>,
        seen: Arc<Mutex<Vec<(String, u32, u32)>>>,
        hub: Arc<Hub>,
    }

    fn config(min_gap_ms: u64) -> Config {
        let mut c = Config::default();
        c.previews.min_gap_ms = min_gap_ms;
        c
    }

    fn rig_with(dir: tempfile::TempDir, cfg: &Config) -> Rig {
        let hub = Arc::new(Hub::new(1));
        hub.set("screen", json!({"on": true, "state": "Active"}));
        let next = Arc::new(Mutex::new(Next::Picture(480, 270)));
        let shots = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let camera = Fake {
            next: next.clone(),
            shots: shots.clone(),
            seen: seen.clone(),
        };
        let previews = Previews::with_camera(
            cfg,
            &Paths::new(dir.path()),
            hub.clone(),
            Box::new(camera),
            dir.path().join("scratch"),
        );
        Rig {
            dir,
            previews,
            next,
            shots,
            seen,
            hub,
        }
    }

    fn rig() -> Rig {
        rig_with(tempfile::tempdir().unwrap(), &config(0))
    }

    fn request(method: &str, path: &str, body: &str) -> Request {
        Request {
            method: method.into(),
            path: path.into(),
            query: Vec::new(),
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
            peer: "127.0.0.1".parse().unwrap(),
        }
    }

    fn call(r: &Rig, method: &str, rest: &[&str], body: &str) -> Response {
        r.previews
            .handle(&request(method, "/v1/previews", body), rest)
    }

    fn json_of(r: &Response) -> Value {
        serde_json::from_slice(&r.body).unwrap()
    }

    fn code(r: &Response) -> String {
        json_of(r)["error"]["code"]
            .as_str()
            .unwrap_or("")
            .to_owned()
    }

    fn shoot(r: &Rig, key: &str, why: &str) -> Response {
        call(
            r,
            "POST",
            &["capture"],
            &format!(r#"{{"key":"{key}","why":"{why}"}}"#),
        )
    }

    fn theme(r: &Rig, text: &str) {
        let f = r.dir.path().join("theme").join("theme.json");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(f, text).unwrap();
    }

    fn mode(p: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn only_live_tv_and_the_hdmi_ports_have_pictures() {
        for k in ["livetv", "hdmi1", "hdmi4", "hdmi9"] {
            assert!(key_ok(k), "{k}");
        }
        for k in [
            "",
            "hdmi",
            "hdmi0",
            "hdmi10",
            "HDMI1",
            "../hdmi1",
            "netflix",
            "livetv ",
            "hdmi1.jpg",
        ] {
            assert!(!key_ok(k), "{k:?}");
        }
    }

    #[test]
    fn a_capture_is_asked_of_the_video_plane_at_the_size_kept_and_comes_back_as_a_small_jpeg() {
        let r = rig();
        let a = shoot(&r, "hdmi2", "open");
        assert_eq!(a.status, 200, "{}", String::from_utf8_lossy(&a.body));
        let b = json_of(&a);
        assert_eq!(
            (
                b["key"].as_str(),
                b["w"].as_u64(),
                b["h"].as_u64(),
                b["saved"].as_bool()
            ),
            (Some("hdmi2"), Some(480), Some(270), Some(true))
        );
        assert_eq!(*r.seen.lock(), vec![("VIDEO".to_owned(), 480, 270)]);
        let got = call(&r, "GET", &["hdmi2"], "");
        assert_eq!(got.status, 200);
        assert!(
            got.headers
                .iter()
                .any(|(n, v)| *n == "Content-Type" && v == "image/jpeg")
        );
        assert!(
            got.headers
                .iter()
                .any(|(n, v)| *n == "X-Luma-At" && v.parse::<u64>().ok() == b["at"].as_u64())
        );
        assert!(got.body.len() < 60 * 1024, "{} bytes", got.body.len());
        let decoded = jpeg_decoder::Decoder::new(Cursor::new(&got.body))
            .decode()
            .unwrap();
        assert_eq!(decoded.len(), 480 * 270 * 3);
        // the picture is the gradient: red grows to the right
        let px = |x: usize, y: usize| &decoded[(y * 480 + x) * 3..(y * 480 + x) * 3 + 3];
        assert!(
            px(450, 135)[0] > px(30, 135)[0] + 150,
            "{:?} {:?}",
            px(450, 135),
            px(30, 135)
        );
    }

    #[test]
    fn the_raw_capture_is_deleted_as_soon_as_it_is_read() {
        let r = rig();
        assert_eq!(shoot(&r, "hdmi1", "open").status, 200);
        let left: Vec<_> = fs::read_dir(r.dir.path().join("scratch"))
            .unwrap()
            .flatten()
            .collect();
        assert!(left.is_empty(), "{left:?}");
        *r.next.lock() = Next::Black;
        assert_eq!(shoot(&r, "hdmi1", "open").status, 409);
        *r.next.lock() = Next::Error("x");
        assert_eq!(shoot(&r, "hdmi1", "open").status, 502);
        assert!(
            fs::read_dir(r.dir.path().join("scratch"))
                .unwrap()
                .next()
                .is_none(),
            "nothing is left after a black capture or a failure"
        );
    }

    #[test]
    fn a_black_or_refused_picture_is_no_picture_and_the_one_kept_stays() {
        let r = rig();
        let first = json_of(&shoot(&r, "hdmi3", "open"));
        *r.next.lock() = Next::Black;
        let black = shoot(&r, "hdmi3", "periodic");
        assert_eq!((black.status, code(&black).as_str()), (409, "no_picture"));
        *r.next.lock() = Next::Refused("REFUSED_NO_SIGNAL");
        let refused = shoot(&r, "hdmi3", "periodic");
        assert_eq!(
            (refused.status, code(&refused).as_str()),
            (409, "no_picture")
        );
        let kept = call(&r, "GET", &["hdmi3"], "");
        assert_eq!(kept.status, 200);
        let at = kept
            .headers
            .iter()
            .find(|(n, _)| *n == "X-Luma-At")
            .unwrap()
            .1
            .clone();
        assert_eq!(at, first["at"].to_string(), "still the first picture");
        // a refusal is an answer, not a fault: the service is not backed off from however many come
        for _ in 0..6 {
            assert_eq!(shoot(&r, "hdmi3", "periodic").status, 409);
        }
        *r.next.lock() = Next::Picture(480, 270);
        assert_eq!(shoot(&r, "hdmi3", "periodic").status, 200);
    }

    #[test]
    fn a_picture_with_a_few_lit_pixels_is_a_picture_and_one_with_none_is_not() {
        let dark = Rgb {
            w: 100,
            h: 100,
            px: (0..100 * 100)
                .flat_map(|i| {
                    if i % 50 == 0 {
                        [200, 200, 200]
                    } else {
                        [2, 2, 2]
                    }
                })
                .collect(),
        };
        assert!(!is_blank(&dark), "2 % of the pixels are lit: a night scene");
        let black = Rgb {
            w: 100,
            h: 100,
            px: vec![3; 100 * 100 * 3],
        };
        assert!(is_blank(&black));
        let one = Rgb {
            w: 100,
            h: 100,
            px: (0..100 * 100)
                .flat_map(|i| if i == 7 { [255, 255, 255] } else { [0, 0, 0] })
                .collect(),
        };
        assert!(is_blank(&one), "one pixel in ten thousand is not a picture");
    }

    #[test]
    fn the_owners_switch_decides_and_turning_it_off_leaves_nothing_to_serve_or_take() {
        let r = rig();
        assert_eq!(shoot(&r, "hdmi1", "open").status, 200);
        theme(&r, r#"{"inputs": {"previews": false}}"#);
        let n = r.shots.load(Ordering::SeqCst);
        let off = shoot(&r, "hdmi1", "open");
        assert_eq!(
            (off.status, code(&off).as_str()),
            (403, "previews_disabled")
        );
        assert_eq!(
            r.shots.load(Ordering::SeqCst),
            n,
            "the capture service was not asked"
        );
        assert_eq!(
            call(&r, "GET", &["hdmi1"], "").status,
            403,
            "and the picture is not served while it is off"
        );
        let status = json_of(&call(&r, "GET", &[], ""));
        assert_eq!(status["previews"]["enabled"], false);
        assert_eq!(status["previews"]["items"], json!({}));
        // deleting is always allowed
        assert_eq!(json_of(&call(&r, "DELETE", &[], ""))["cleared"], 1);
        assert!(!r.dir.path().join("previews").join("hdmi1.jpg").exists());
        // a theme without the key, or on, is on
        theme(&r, r#"{"inputs": {"hdmi1": "always"}}"#);
        assert_eq!(shoot(&r, "hdmi1", "open").status, 200);
        theme(&r, r#"{"inputs": {"previews": true}}"#);
        assert_eq!(shoot(&r, "hdmi1", "open").status, 200);
    }

    #[test]
    fn pictures_are_kept_for_the_owner_alone_and_one_per_input_is_overwritten() {
        let r = rig();
        assert_eq!(shoot(&r, "hdmi1", "open").status, 200);
        assert_eq!(shoot(&r, "livetv", "arrive").status, 200);
        let dir = r.dir.path().join("previews");
        assert_eq!(mode(&dir), 0o700);
        for f in ["hdmi1.jpg", "livetv.jpg", "index.json"] {
            assert_eq!(mode(&dir.join(f)), 0o600, "{f}");
        }
        let before = fs::read(dir.join("hdmi1.jpg")).unwrap();
        *r.next.lock() = Next::Picture(480, 270);
        assert_eq!(shoot(&r, "hdmi1", "periodic").status, 200);
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names.len(),
            3,
            "{names:?}: one file per input and the index, no more"
        );
        assert!(!before.is_empty());
        assert_eq!(mode(&r.dir.path().join("scratch")), 0o700);
    }

    #[test]
    fn what_was_kept_is_there_after_a_restart_and_what_is_too_old_is_forgotten() {
        let r = rig();
        assert_eq!(shoot(&r, "hdmi2", "open").status, 200);
        assert_eq!(shoot(&r, "hdmi4", "open").status, 200);
        let at2 = json_of(&call(&r, "GET", &[], ""))["previews"]["items"]["hdmi2"]["at"]
            .as_u64()
            .unwrap();
        let dir = r.dir.path().to_path_buf();
        // age hdmi4 beyond the limit in the index
        let idx = dir.join("previews").join("index.json");
        let mut doc: Value = serde_json::from_slice(&fs::read(&idx).unwrap()).unwrap();
        doc["items"]["hdmi4"]["at"] = json!(now_ms() - 31 * 86_400_000);
        fs::write(&idx, doc.to_string()).unwrap();
        let again = {
            let hub = Arc::new(Hub::new(2));
            Previews::with_camera(
                &Config::default(),
                &Paths::new(&dir),
                hub,
                Box::new(Fake {
                    next: r.next.clone(),
                    shots: r.shots.clone(),
                    seen: r.seen.clone(),
                }),
                dir.join("scratch"),
            )
        };
        let status = json_of(&again.handle(&request("GET", "/v1/previews", ""), &[]));
        let items = &status["previews"]["items"];
        assert_eq!(items["hdmi2"]["at"], at2);
        assert!(items.get("hdmi4").is_none(), "{items}");
        assert!(
            !dir.join("previews").join("hdmi4.jpg").exists(),
            "its file went with it"
        );
        assert_eq!(
            again
                .handle(&request("GET", "/v1/previews/hdmi2", ""), &["hdmi2"])
                .status,
            200
        );
    }

    #[test]
    fn a_picture_that_is_not_a_jpeg_on_the_flash_is_not_served() {
        let r = rig();
        let dir = r.dir.path().join("previews");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("hdmi1.jpg"), b"not a picture").unwrap();
        fs::write(
            dir.join("index.json"),
            json!({"v": 1, "items": {"hdmi1": {"at": now_ms()}, "../x": {"at": now_ms()}}})
                .to_string(),
        )
        .unwrap();
        assert_eq!(call(&r, "GET", &["hdmi1"], "").status, 404);
        assert!(!dir.join("hdmi1.jpg").exists());
    }

    #[test]
    fn a_live_picture_is_kept_in_memory_and_goes_to_the_flash_at_most_every_30_seconds() {
        let r = rig();
        let first = json_of(&shoot(&r, "hdmi2", "live"));
        assert_eq!(
            first["saved"], true,
            "the first one has nothing before it on the flash"
        );
        let second = json_of(&shoot(&r, "hdmi2", "live"));
        assert_eq!(second["saved"], false);
        let on_flash: Value =
            serde_json::from_slice(&fs::read(r.dir.path().join("previews/index.json")).unwrap())
                .unwrap();
        assert_eq!(
            on_flash["items"]["hdmi2"]["at"], first["at"],
            "the flash still has the first"
        );
        let served = call(&r, "GET", &["hdmi2"], "");
        assert_eq!(
            served
                .headers
                .iter()
                .find(|(n, _)| *n == "X-Luma-At")
                .unwrap()
                .1,
            second["at"].to_string(),
            "the memory has the newest"
        );
        // the other reasons write at once
        let open = json_of(&shoot(&r, "hdmi2", "open"));
        assert_eq!(open["saved"], true);
        let on_flash: Value =
            serde_json::from_slice(&fs::read(r.dir.path().join("previews/index.json")).unwrap())
                .unwrap();
        assert_eq!(on_flash["items"]["hdmi2"]["at"], open["at"]);
    }

    #[test]
    fn captures_come_one_at_a_time_and_not_faster_than_the_gap() {
        let r = rig_with(tempfile::tempdir().unwrap(), &config(60_000));
        assert_eq!(shoot(&r, "hdmi1", "live").status, 200);
        let soon = shoot(&r, "hdmi1", "live");
        assert_eq!((soon.status, code(&soon).as_str()), (429, "too_soon"));
        assert_eq!(r.shots.load(Ordering::SeqCst), 1);
        r.previews.state.lock().busy = true;
        assert_eq!(code(&shoot(&r, "hdmi1", "live")), "busy");
    }

    #[test]
    fn a_service_that_fails_three_times_in_a_row_is_left_alone() {
        let r = rig();
        *r.next.lock() = Next::Error("luna-send gave no answer");
        for _ in 0..3 {
            let f = shoot(&r, "hdmi1", "periodic");
            assert_eq!((f.status, code(&f).as_str()), (502, "capture_failed"));
        }
        let n = r.shots.load(Ordering::SeqCst);
        let backed = shoot(&r, "hdmi1", "periodic");
        assert_eq!(
            (backed.status, code(&backed).as_str()),
            (503, "backing_off")
        );
        assert_eq!(r.shots.load(Ordering::SeqCst), n, "not asked");
        let t = r.previews.state.lock().backoff_until.unwrap();
        assert!(t > Instant::now() + Duration::from_secs(50));
        // after the pause it is tried again, and a good picture ends the backing off
        r.previews.state.lock().backoff_until = Some(Instant::now() - Duration::from_secs(1));
        *r.next.lock() = Next::Picture(480, 270);
        assert_eq!(shoot(&r, "hdmi1", "periodic").status, 200);
        assert_eq!(r.previews.state.lock().failures, 0);
        // a timeout counts like any failure
        *r.next.lock() = Next::Hang;
        let t = shoot(&r, "hdmi1", "periodic");
        assert_eq!((t.status, code(&t).as_str()), (504, "timeout"));
    }

    #[test]
    fn nothing_is_captured_with_the_screen_off_except_the_last_one_as_it_goes_off() {
        let r = rig();
        r.hub.set("screen", json!({"on": false}));
        let off = shoot(&r, "hdmi1", "periodic");
        assert_eq!((off.status, code(&off).as_str()), (409, "screen_off"));
        assert_eq!(r.shots.load(Ordering::SeqCst), 0);
        assert_eq!(
            shoot(&r, "hdmi1", "standby").status,
            200,
            "the standby picture is taken as the screen goes"
        );
    }

    #[test]
    fn a_picture_bigger_than_asked_is_shrunk_to_fit_and_a_smaller_one_is_not_enlarged() {
        let r = rig();
        *r.next.lock() = Next::Picture(1920, 1080);
        let b = json_of(&shoot(&r, "hdmi1", "open"));
        assert_eq!((b["w"].as_u64(), b["h"].as_u64()), (Some(480), Some(270)));
        *r.next.lock() = Next::Picture(320, 180);
        let b = json_of(&shoot(&r, "hdmi1", "open"));
        assert_eq!((b["w"].as_u64(), b["h"].as_u64()), (Some(320), Some(180)));
        // a 4:3 source keeps its shape inside the box
        *r.next.lock() = Next::Picture(1440, 1080);
        let b = json_of(&shoot(&r, "hdmi1", "open"));
        assert_eq!((b["w"].as_u64(), b["h"].as_u64()), (Some(360), Some(270)));
    }

    #[test]
    fn bad_requests_are_told_apart_and_nothing_is_captured_for_them() {
        let r = rig();
        for body in [
            "",
            "x",
            "{}",
            r#"{"key":"hdmi1"}"#,
            r#"{"why":"open"}"#,
            r#"{"key":"../hdmi1","why":"open"}"#,
            r#"{"key":"netflix","why":"open"}"#,
            r#"{"key":"hdmi1","why":"whenever"}"#,
        ] {
            let a = call(&r, "POST", &["capture"], body);
            assert_eq!(
                (a.status, code(&a).as_str()),
                (400, "bad_request"),
                "{body:?}"
            );
        }
        assert_eq!(r.shots.load(Ordering::SeqCst), 0);
        assert_eq!(call(&r, "GET", &["capture"], "").status, 405);
        assert_eq!(call(&r, "POST", &[], "").status, 405);
        assert_eq!(call(&r, "GET", &["hdmi1", "x"], "").status, 404);
        assert_eq!(call(&r, "GET", &["nope"], "").status, 404);
        assert_eq!(call(&r, "DELETE", &["nope"], "").status, 404);
    }

    #[test]
    fn deleting_clears_one_input_or_all_of_them_with_their_files() {
        let r = rig();
        for k in ["hdmi1", "hdmi2", "livetv"] {
            assert_eq!(shoot(&r, k, "open").status, 200);
        }
        assert_eq!(json_of(&call(&r, "DELETE", &["hdmi1"], ""))["cleared"], 1);
        assert_eq!(call(&r, "GET", &["hdmi1"], "").status, 404);
        assert_eq!(call(&r, "GET", &["hdmi2"], "").status, 200);
        let dir = r.dir.path().join("previews");
        assert!(!dir.join("hdmi1.jpg").exists() && dir.join("hdmi2.jpg").exists());
        // a file an earlier run left that this one never loaded is gone too
        fs::write(dir.join("hdmi3.jpg"), b"x").unwrap();
        assert_eq!(json_of(&call(&r, "DELETE", &[], ""))["cleared"], 2);
        let left: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, vec!["index.json".to_owned()]);
        let status = json_of(&call(&r, "GET", &[], ""));
        assert_eq!(status["previews"]["items"], json!({}));
    }

    #[test]
    fn the_status_says_what_the_captures_cost_and_what_the_last_one_did() {
        let r = rig();
        assert_eq!(shoot(&r, "hdmi1", "open").status, 200);
        *r.next.lock() = Next::Black;
        assert_eq!(shoot(&r, "hdmi1", "periodic").status, 409);
        let s = &json_of(&call(&r, "GET", &[], ""))["previews"];
        assert_eq!(
            (
                s["stats"]["taken"].as_u64(),
                s["stats"]["blank"].as_u64(),
                s["stats"]["failed"].as_u64()
            ),
            (Some(1), Some(1), Some(0))
        );
        assert_eq!(s["stats"]["last"]["code"], "no_picture");
        assert_eq!(s["size"], json!([480, 270]));
        assert_eq!(s["method"], "VIDEO");
        assert_eq!(s["items"]["hdmi1"]["w"], 480);
    }

    #[test]
    fn luna_sends_answer_is_the_first_object_in_its_output() {
        assert_eq!(
            first_object("{\"returnValue\":true,\"writtenBytes\":5}\n"),
            Some(json!({"returnValue": true, "writtenBytes": 5}))
        );
        assert_eq!(
            first_object(
                "noise\n{\n  \"returnValue\": false,\n  \"errorText\": \"x\"\n}\n{\"b\":1}"
            ),
            Some(json!({"returnValue": false, "errorText": "x"}))
        );
        assert_eq!(first_object("no json here"), None);
        assert_eq!(first_object(""), None);
    }

    #[test]
    fn the_luna_camera_runs_luna_send_with_the_capture_call_and_kills_one_that_hangs() {
        let dir = tempfile::tempdir().unwrap();
        let args = dir.path().join("args");
        let script = dir.path().join("luna-send");
        fs::write(&script, format!("#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > {}\necho '{{\"returnValue\":true,\"writtenBytes\":9}}'\n", args.display())).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let cam = LunaCamera::new(script.to_str().unwrap());
        let reply = cam
            .shoot(
                Path::new("/tmp/x.png"),
                "VIDEO",
                480,
                270,
                Duration::from_secs(2),
            )
            .unwrap();
        assert_eq!(reply["writtenBytes"], 9);
        let got: Vec<String> = fs::read_to_string(&args)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect();
        assert_eq!(&got[..4], ["-n", "1", "-w", "2000"]);
        assert_eq!(got[4], URI);
        let payload: Value = serde_json::from_str(&got[5]).unwrap();
        assert_eq!(
            payload,
            json!({"path": "/tmp/x.png", "method": "VIDEO", "format": "PNG", "width": 480, "height": 270})
        );
        // one that never answers is killed at the deadline
        fs::write(&script, "#!/bin/sh\nsleep 30\n").unwrap();
        let started = Instant::now();
        let hung = cam.shoot(
            Path::new("/tmp/x.png"),
            "VIDEO",
            480,
            270,
            Duration::from_millis(300),
        );
        assert_eq!(hung, Err(Failure::Timeout));
        assert!(started.elapsed() < Duration::from_secs(5));
        // and a program that is not there is a failure in words
        let gone = LunaCamera::new("/nonexistent/luna-send").shoot(
            Path::new("/tmp/x.png"),
            "VIDEO",
            480,
            270,
            Duration::from_secs(1),
        );
        assert!(matches!(gone, Err(Failure::Run(m)) if m.contains("could not run")));
    }
}
