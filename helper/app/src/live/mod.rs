//! The live path: a camera's H.264 (or HEVC) stream on the TV's video plane, through Home Assistant and the GFN feeder
//! (`docs/research/ha-cameras-result.md`, "The live path").
//!
//! The TV never holds a camera's own address or password. It asks Home Assistant's LumaShow integration for the camera's stream
//! (`camera_live.start`); the integration asks Home Assistant's stream integration, which pulls the camera's RTSP itself and serves the
//! stream as low-latency HLS of fragmented MP4 at a tokened address on Home Assistant's own port (`camera_live.started`). This module
//! fetches that playlist from its live edge ([`hls`]), turns each piece into whole access units ([`fmp4`]) and feeds them to a
//! `lumatv-gfn-feeder` it started ([`gf`]), the 32-bit process that owns a window on the video plane and LG's decoder; the shell draws
//! its chrome above the hole. One session at a time: a new camera ends the old one (one decoder; the TV check says whether a second is
//! ever given). Stopping unloads the pipeline and ends the feeder, so nothing runs when nothing is shown.
//!
//! The topic `cameras.live` says where it stands: `{camera, state (idle|asking|starting|playing|failed|stopped), reason, fed, dropped,
//! pieceMs (the latency the stream's part or segment length sets), window, since}`.

pub mod fmp4;
pub mod gf;
pub mod hls;
pub mod net;

use ha_bridge::modules::ModuleOut;
use ha_link::AddressPolicy;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The module of the protocol this speaks with the integration.
pub const MODULE: &str = "camera_live";
/// How long the integration has to answer `start`.
const ANSWER_WITHIN: Duration = Duration::from_secs(12);
/// How long one piece of the stream may take to arrive.
const PIECE_WITHIN: Duration = Duration::from_secs(8);
/// The most one piece may be.
const PIECE_MAX: usize = 16 << 20;
/// How long the feeder has to make its socket and say hello.
const FEEDER_WITHIN: Duration = Duration::from_secs(8);

/// How the feeder is started; `None` leaves the live path off (the pictures' path stays).
#[derive(Clone, Debug)]
pub struct FeederSpec {
    pub program: PathBuf,
    /// The window type the feeder owns (`card`, `overlay`, `screensaver`): the TV check decides.
    pub window: String,
    /// The NDL library to load (`-L`): empty for the feeder's default.
    pub library: String,
    pub extra_args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Where the socket and the stats file go.
    pub run_dir: PathBuf,
}

pub struct Deps {
    pub feeder: Option<FeederSpec>,
    pub policy: AddressPolicy,
}

/// What the shell sees.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    pub camera: String,
    pub state: &'static str,
    pub reason: String,
    pub fed: u64,
    pub dropped: u64,
    pub piece_ms: u64,
    pub window: String,
    pub since: u64,
}

impl View {
    pub fn json(&self) -> Value {
        json!({
            "camera": if self.camera.is_empty() { Value::Null } else { Value::String(self.camera.clone()) },
            "state": self.state,
            "reason": self.reason,
            "fed": self.fed,
            "dropped": self.dropped,
            "pieceMs": self.piece_ms,
            "window": if self.window.is_empty() { Value::Null } else { Value::String(self.window.clone()) },
            "since": self.since,
        })
    }
}

struct Session {
    camera: String,
    op: u64,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

struct Inner {
    deps: Deps,
    out: Mutex<Option<Arc<dyn ModuleOut>>>,
    session: Mutex<Option<Session>>,
    view: Mutex<View>,
    next_op: AtomicU64,
    publish: Box<dyn Fn(Value) + Send + Sync>,
}

/// The one live session, and the door the API and the module use.
#[derive(Clone)]
pub struct Live {
    inner: Arc<Inner>,
}

impl Live {
    pub fn new(deps: Deps, publish: Box<dyn Fn(Value) + Send + Sync>) -> Self {
        let me = Self {
            inner: Arc::new(Inner {
                deps,
                out: Mutex::new(None),
                session: Mutex::new(None),
                view: Mutex::new(View {
                    state: "idle",
                    ..View::default()
                }),
                next_op: AtomicU64::new(1),
                publish,
            }),
        };
        me.publish();
        me
    }

    /// Whether the live path can run at all (a feeder is configured).
    pub fn available(&self) -> bool {
        self.inner.deps.feeder.is_some()
    }

    pub fn view(&self) -> View {
        self.inner.view.lock().clone()
    }

    fn publish(&self) {
        let view = self.inner.view.lock().clone();
        (self.inner.publish)(view.json());
    }

    fn set(&self, f: impl FnOnce(&mut View)) {
        {
            let mut view = self.inner.view.lock();
            f(&mut view);
        }
        self.publish();
    }

    /// The module's session with the integration began or ended.
    pub fn module_session(&self, up: bool, out: Arc<dyn ModuleOut>) {
        *self.inner.out.lock() = if up { Some(out) } else { None };
        if !up {
            self.stop("the LumaShow integration disconnected");
        }
    }

    /// `POST /v1/cameras/live {camera}`: asks the integration for the stream and plays it when it answers. `Err` says why not, now.
    pub fn start(&self, camera: &str) -> Result<(), String> {
        if !crate::cameras::plain_id(camera) {
            return Err("a camera is camera.<name>".to_owned());
        }
        if !self.available() {
            return Err("the live path is off (cameras.live)".to_owned());
        }
        let Some(out) = self.inner.out.lock().clone() else {
            return Err("Home Assistant's LumaShow integration is not connected".to_owned());
        };
        self.end_session();
        let op = self.inner.next_op.fetch_add(1, Ordering::SeqCst);
        let stop = Arc::new(AtomicBool::new(false));
        *self.inner.session.lock() = Some(Session {
            camera: camera.to_owned(),
            op,
            stop: stop.clone(),
            thread: None,
        });
        self.set(|v| {
            *v = View {
                camera: camera.to_owned(),
                state: "asking",
                since: crate::timeutil::now_ms(),
                ..View::default()
            }
        });
        if !out.send(
            "start",
            json!({"op": op, "camera": camera, "format": "hls"}),
        ) {
            self.set(|v| {
                v.state = "failed";
                v.reason = "the integration is not connected".to_owned();
            });
            return Err("the integration is not connected".to_owned());
        }
        // the answer has a deadline: a session still asking then is failed
        let me = self.clone();
        thread::Builder::new()
            .name("cameras-live-wait".to_owned())
            .stack_size(64 * 1024)
            .spawn(move || {
                thread::sleep(ANSWER_WITHIN);
                let asking = {
                    let session = me.inner.session.lock();
                    session.as_ref().is_some_and(|s| {
                        s.op == op && s.thread.is_none() && !stop.load(Ordering::SeqCst)
                    })
                };
                if asking {
                    me.end_session();
                    me.set(|v| {
                        v.state = "failed";
                        v.reason = "Home Assistant did not answer with a stream".to_owned();
                    });
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// The integration's answer to `start`.
    pub fn started(&self, op: u64, camera: &str, url: &str, kind: &str) {
        let (matches, stop) = {
            let session = self.inner.session.lock();
            match session.as_ref() {
                Some(s) if s.op == op && s.camera == camera && s.thread.is_none() => {
                    (true, s.stop.clone())
                }
                _ => (false, Arc::new(AtomicBool::new(true))),
            }
        };
        if !matches {
            return;
        }
        if kind != "hls" {
            self.set(|v| {
                v.state = "failed";
                v.reason = format!("a {kind} stream is not one this TV plays yet");
            });
            return;
        }
        let Some(spec) = self.inner.deps.feeder.clone() else {
            return;
        };
        let url = url.to_owned();
        let me = self.clone();
        let policy = self.inner.deps.policy;
        let thread = thread::Builder::new()
            .name("cameras-live".to_owned())
            .stack_size(512 * 1024)
            .spawn(move || {
                let outcome = run(&me, &spec, &url, policy, &stop);
                if let Err(why) = outcome
                    && !stop.load(Ordering::SeqCst)
                {
                    crate::log_warn!("cameras: live: {why}");
                    me.set(|v| {
                        v.state = "failed";
                        v.reason = why;
                    });
                }
            });
        match thread {
            Ok(handle) => {
                if let Some(s) = self.inner.session.lock().as_mut() {
                    s.thread = Some(handle);
                }
                self.set(|v| v.state = "starting");
            }
            Err(e) => self.set(|v| {
                v.state = "failed";
                v.reason = e.to_string();
            }),
        }
    }

    /// The integration could not give the stream.
    pub fn failed(&self, op: u64, code: &str, message: &str) {
        let matches = self
            .inner
            .session
            .lock()
            .as_ref()
            .is_some_and(|s| s.op == op);
        if matches {
            self.end_session();
            self.set(|v| {
                v.state = "failed";
                v.reason = if message.is_empty() {
                    code.to_owned()
                } else {
                    format!("{code}: {message}")
                };
            });
        }
    }

    /// `POST /v1/cameras/live/stop`, the viewer closing, the module going.
    pub fn stop(&self, why: &str) {
        let had = self.end_session();
        if had {
            self.set(|v| {
                v.state = "stopped";
                v.reason = why.to_owned();
                v.window = String::new();
            });
        }
    }

    fn end_session(&self) -> bool {
        let session = self.inner.session.lock().take();
        let Some(session) = session else {
            return false;
        };
        session.stop.store(true, Ordering::SeqCst);
        if let Some(out) = self.inner.out.lock().clone() {
            out.send("stop", json!({"camera": session.camera}));
        }
        if let Some(thread) = session.thread {
            let _ = thread.join();
        }
        true
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(session) = self.session.get_mut().take() {
            session.stop.store(true, Ordering::SeqCst);
        }
    }
}

/// A feeder process and its socket; the process is ended when this goes.
struct Feeder {
    child: Child,
    socket: PathBuf,
    client: gf::Client,
}

impl Feeder {
    fn start(spec: &FeederSpec) -> Result<(Self, String), String> {
        std::fs::create_dir_all(&spec.run_dir)
            .map_err(|e| format!("{}: {e}", spec.run_dir.display()))?;
        let socket = spec
            .run_dir
            .join(format!("feeder-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let mut command = Command::new(&spec.program);
        command
            .arg("--serve")
            .arg(&socket)
            .arg("-w")
            .arg(&spec.window)
            .arg("--once")
            .arg("--stats")
            .arg(spec.run_dir.join("feeder-stats.txt"));
        if !spec.library.is_empty() {
            command.arg("-L").arg(&spec.library);
        }
        command.args(&spec.extra_args);
        for (k, v) in &spec.env {
            command.env(k, v);
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = command
            .spawn()
            .map_err(|e| format!("cannot start the feeder {}: {e}", spec.program.display()))?;
        let (client, hello) = match gf::Client::connect(&socket, FEEDER_WITHIN) {
            Ok(pair) => pair,
            Err(e) => {
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("the feeder did not answer on its socket: {e}"));
            }
        };
        let window = match hello {
            gf::Message::Hello {
                library, window_id, ..
            } if library => window_id,
            gf::Message::Hello { .. } => {
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                return Err("the feeder has no decoder library".to_owned());
            }
            _ => String::new(),
        };
        Ok((
            Self {
                child,
                socket,
                client,
            },
            window,
        ))
    }
}

impl Drop for Feeder {
    fn drop(&mut self) {
        let _ = self.client.send_unload();
        // the feeder ends by itself after an unload with --once; a moment, then it is ended
        let deadline = Instant::now() + Duration::from_millis(1500);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
                _ => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break;
                }
            }
        }
        let _ = std::fs::remove_file(&self.socket);
    }
}

/// The pipeline, from the stream's address to the feeder, until `stop`.
fn run(
    live: &Live,
    spec: &FeederSpec,
    url: &str,
    policy: AddressPolicy,
    stop: &AtomicBool,
) -> Result<(), String> {
    let fetch = |u: &str| -> Result<Vec<u8>, String> {
        let got = net::get(u, policy, PIECE_WITHIN, PIECE_MAX)?;
        if got.status != 200 {
            return Err(format!("{u}: HTTP {}", got.status));
        }
        Ok(got.body)
    };
    // the playlist: a master that names the media playlist, or the media playlist itself
    let first = String::from_utf8_lossy(&fetch(url)?).into_owned();
    let (playlist_url, mut playlist) = match hls::parse_media(&first) {
        Some(p) => (url.to_owned(), p),
        None => {
            let variant = hls::parse_master(&first).ok_or("the address is not a playlist")?;
            let variant_url = hls::resolve(url, &variant);
            let text = String::from_utf8_lossy(&fetch(&variant_url)?).into_owned();
            let p = hls::parse_media(&text).ok_or("the variant is not a media playlist")?;
            (variant_url, p)
        }
    };
    let init_uri = playlist
        .init_uri
        .clone()
        .ok_or("the playlist names no initialisation segment")?;
    let init = fmp4::parse_init(&fetch(&hls::resolve(&playlist_url, &init_uri))?)
        .map_err(|e| e.to_string())?;
    if stop.load(Ordering::SeqCst) {
        return Ok(());
    }
    let (mut feeder, window) = Feeder::start(spec)?;
    let piece_ms = (playlist.part_target.unwrap_or(playlist.target_duration) * 1000.0) as u64;
    live.set(|v| {
        v.window = window.clone();
        v.piece_ms = piece_ms;
    });
    // the first piece tells the rate; the load waits for it
    let mut loaded = false;
    let mut pieces = playlist.live_edge();
    let mut at = pieces.last().map(hls::Playlist::position_after);
    let mut base_pts: Option<i64> = None;
    let mut sent: u64 = 0;
    let fed = Arc::new(AtomicU64::new(0));
    let dropped = Arc::new(AtomicU64::new(0));
    let playing = Arc::new(AtomicBool::new(false));
    let reader = {
        let mut stream = feeder.client.try_clone().map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .ok();
        let (fed, dropped, playing, live) =
            (fed.clone(), dropped.clone(), playing.clone(), live.clone());
        let stop_flag = Arc::new(AtomicBool::new(false));
        let flag = stop_flag.clone();
        let handle = thread::Builder::new()
            .name("cameras-live-feeder".to_owned())
            .stack_size(128 * 1024)
            .spawn(move || {
                let mut last_publish = Instant::now();
                while !flag.load(Ordering::SeqCst) {
                    match gf::read_frame(&mut stream) {
                        Ok((kind, payload)) => match gf::decode(kind, &payload) {
                            gf::Message::Accept(_) => {
                                fed.fetch_add(1, Ordering::Relaxed);
                                if !playing.swap(true, Ordering::SeqCst) {
                                    live.set(|v| v.state = "playing");
                                }
                            }
                            gf::Message::Drop { .. } => {
                                dropped.fetch_add(1, Ordering::Relaxed);
                            }
                            gf::Message::Loaded { ok: false, text } => {
                                live.set(|v| {
                                    v.state = "failed";
                                    v.reason = format!("the pipeline refused the load: {text}");
                                });
                            }
                            gf::Message::Error { code, text } => {
                                live.set(|v| {
                                    v.state = "failed";
                                    v.reason = format!("the feeder failed ({code}): {text}");
                                });
                            }
                            _ => {}
                        },
                        Err(e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                || e.kind() == std::io::ErrorKind::TimedOut => {}
                        Err(_) => break,
                    }
                    if last_publish.elapsed() >= Duration::from_secs(1) {
                        last_publish = Instant::now();
                        let (f, d) = (fed.load(Ordering::Relaxed), dropped.load(Ordering::Relaxed));
                        live.set(|v| {
                            v.fed = f;
                            v.dropped = d;
                        });
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        (handle, stop_flag)
    };
    let outcome = (|| -> Result<(), String> {
        loop {
            for piece in pieces.drain(..) {
                if stop.load(Ordering::SeqCst) {
                    return Ok(());
                }
                let bytes = fetch(&hls::resolve(&playlist_url, &piece.uri))?;
                let units = fmp4::parse_segment(&init, &bytes).map_err(|e| e.to_string())?;
                if !loaded && !units.is_empty() {
                    let span = units.last().map_or(0, |u| u.dts) - units[0].dts;
                    let fps = if span > 0 && units.len() > 1 {
                        ((units.len() as f64 - 1.0) * f64::from(init.timescale) / span as f64)
                            .round() as u32
                    } else {
                        0
                    };
                    let codec = match init.codec {
                        fmp4::Codec::H264 => gf::CODEC_H264,
                        fmp4::Codec::H265 => gf::CODEC_H265,
                    };
                    feeder
                        .client
                        .send_load(codec, init.width, init.height, fps)
                        .map_err(|e| e.to_string())?;
                    feeder.client.send_state(true).map_err(|e| e.to_string())?;
                    loaded = true;
                }
                // the feeder is given the decode time: it rises with every unit, which the decoder's pipeline insists on; a stream with
                // B pictures is shown a reorder delay later, which the pipeline does itself
                for unit in units {
                    let base = *base_pts.get_or_insert(unit.dts);
                    let pts_ns = (unit.dts - base).max(0) as i128 * 1_000_000_000
                        / i128::from(init.timescale);
                    feeder
                        .client
                        .send_video(pts_ns as i64, unit.key, &unit.data)
                        .map_err(|e| format!("the feeder went: {e}"))?;
                    sent += 1;
                }
                at = Some(hls::Playlist::position_after(&piece));
            }
            if playlist.ended || stop.load(Ordering::SeqCst) {
                return Ok(());
            }
            let position = at.unwrap_or(hls::Position {
                msn: playlist.media_sequence,
                part: None,
            });
            let reload = playlist.next_reload(position);
            let reload_url = match &reload {
                hls::Reload::Block { .. } => {
                    let sep = if playlist_url.contains('?') { '&' } else { '?' };
                    format!("{playlist_url}{sep}{}", reload.query())
                }
                hls::Reload::After(pause) => {
                    let end = Instant::now() + *pause;
                    while Instant::now() < end {
                        if stop.load(Ordering::SeqCst) {
                            return Ok(());
                        }
                        thread::sleep(Duration::from_millis(25));
                    }
                    playlist_url.clone()
                }
            };
            let text = String::from_utf8_lossy(&fetch(&reload_url)?).into_owned();
            playlist = hls::parse_media(&text).ok_or("the playlist went away")?;
            pieces = playlist.after(position);
        }
    })();
    // a stream that ended by itself: the feeder's last answers are given a moment to arrive before the count is final
    if outcome.is_ok() && !stop.load(Ordering::SeqCst) {
        let until = Instant::now() + Duration::from_secs(2);
        while fed.load(Ordering::Relaxed) + dropped.load(Ordering::Relaxed) < sent
            && Instant::now() < until
        {
            thread::sleep(Duration::from_millis(20));
        }
    }
    reader.1.store(true, Ordering::SeqCst);
    let _ = reader.0.join();
    let (f, d) = (fed.load(Ordering::Relaxed), dropped.load(Ordering::Relaxed));
    drop(feeder);
    live.set(|v| {
        v.fed = f;
        v.dropped = d;
        if outcome.is_ok() && !stop.load(Ordering::SeqCst) {
            v.state = "stopped";
            v.reason = "the stream ended".to_owned();
        }
    });
    outcome
}

/// The default feeder on the set: the GFN feeder staged beside the helper.
pub fn default_spec(data_dir: &Path, window: &str, library: &str) -> FeederSpec {
    FeederSpec {
        program: data_dir.join("bin/lumatv-gfn-feeder"),
        window: window.to_owned(),
        library: library.to_owned(),
        extra_args: Vec::new(),
        env: Vec::new(),
        run_dir: std::env::temp_dir().join("lumatv-cameras"),
    }
}
