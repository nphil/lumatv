//! The live path: a camera's H.264 (or HEVC) stream on the TV's video plane, through Home Assistant and the GFN feeder
//! (`docs/research/ha-cameras-result.md`, "The live path").
//!
//! The TV never holds a camera's own address or password. It asks Home Assistant's LumaShow integration for the camera's stream
//! (`camera_live.start`, naming the ways it can take it, preferred first); the integration answers with the first it can give
//! (`camera_live.started`):
//!
//! - **relay**: Home Assistant's own go2rtc restreams the camera on its loopback; the integration reads that restream inside Home
//!   Assistant and sends the access units down the paired channel as binary frames ([`relay`]). Each is fed to the feeder the moment its
//!   last chunk is in: no segment, no playlist, the latency is the camera's and the decoder's.
//! - **hls**: Home Assistant's stream integration serves the camera as low-latency HLS of fragmented MP4 at a tokened address on its own
//!   port; this fetches the playlist from its live edge ([`hls`]) and turns each piece into whole access units ([`fmp4`]). Seconds behind;
//!   what remains where Home Assistant's go2rtc is not there.
//!
//! Either way the units go to a `lumatv-gfn-feeder` this starts ([`gf`]), the 32-bit process that owns a window on the video plane and
//! LG's decoder; the shell draws its chrome above the hole. One session at a time: a new camera ends the old one (one decoder; the TV
//! check says whether a second is ever given). Stopping unloads the pipeline and ends the feeder, so nothing runs when nothing is shown.
//!
//! The topic `cameras.live` says where it stands: `{camera, state (idle|asking|starting|playing|failed|stopped), reason, kind (relay|hls),
//! fed, dropped, lost, pieceMs (the latency the stream's part or segment length sets; 0 for the relay), delayMs (the relay's: the most a
//! unit of the last second waited in this helper between arriving and being fed), window, since}`.

pub mod fmp4;
pub mod gf;
pub mod hls;
pub mod net;
pub mod relay;
pub mod sps;

use fmp4::Codec;
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
/// A relay with no unit for this long is over.
const RELAY_QUIET: Duration = Duration::from_secs(8);

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
    /// Ask for the relay before HLS (`cameras.relay`).
    pub relay: bool,
}

/// How the integration gives the stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Hls {
        url: String,
    },
    Relay {
        stream: u16,
        codec: Codec,
        /// Ticks per second of the frames' timestamps.
        clock: u32,
        /// The SDP's parameter sets, Annex B (may be empty: then they come with the key pictures).
        parameter_sets: Vec<u8>,
    },
}

/// What the shell sees.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    pub camera: String,
    pub state: &'static str,
    pub reason: String,
    pub kind: &'static str,
    pub fed: u64,
    pub dropped: u64,
    pub lost: u64,
    pub piece_ms: u64,
    pub delay_ms: u64,
    pub window: String,
    pub since: u64,
}

impl View {
    pub fn json(&self) -> Value {
        json!({
            "camera": if self.camera.is_empty() { Value::Null } else { Value::String(self.camera.clone()) },
            "state": self.state,
            "reason": self.reason,
            "kind": if self.kind.is_empty() { Value::Null } else { Value::String(self.kind.to_owned()) },
            "fed": self.fed,
            "dropped": self.dropped,
            "lost": self.lost,
            "pieceMs": self.piece_ms,
            "delayMs": self.delay_ms,
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

/// The relay's receiving end, on the bridge's thread.
struct RelayIn {
    assembler: relay::Assembler,
    queue: relay::Queue,
}

struct Inner {
    deps: Deps,
    out: Mutex<Option<Arc<dyn ModuleOut>>>,
    session: Mutex<Option<Session>>,
    relay: Mutex<Option<RelayIn>>,
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
                relay: Mutex::new(None),
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
        let formats: Vec<&str> = if self.inner.deps.relay {
            vec!["relay", "hls"]
        } else {
            vec!["hls"]
        };
        // `format` is the older spelling, for an integration that reads only that
        if !out.send(
            "start",
            json!({"op": op, "camera": camera, "formats": formats, "format": "hls"}),
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
    pub fn started(&self, op: u64, camera: &str, source: Source) {
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
        let Some(spec) = self.inner.deps.feeder.clone() else {
            return;
        };
        let me = self.clone();
        let policy = self.inner.deps.policy;
        let kind = match &source {
            Source::Hls { .. } => "hls",
            Source::Relay { .. } => "relay",
        };
        let taker = match &source {
            Source::Relay { stream, .. } => {
                let (queue, taker) = relay::Queue::new();
                *self.inner.relay.lock() = Some(RelayIn {
                    assembler: relay::Assembler::new(*stream),
                    queue,
                });
                Some(taker)
            }
            Source::Hls { .. } => None,
        };
        let thread = thread::Builder::new()
            .name("cameras-live".to_owned())
            .stack_size(512 * 1024)
            .spawn(move || {
                let outcome = match source {
                    Source::Hls { url } => run_hls(&me, &spec, &url, policy, &stop),
                    Source::Relay {
                        codec,
                        clock,
                        parameter_sets,
                        ..
                    } => run_relay(
                        &me,
                        &spec,
                        taker.expect("a relay has its queue"),
                        codec,
                        clock,
                        &parameter_sets,
                        &stop,
                    ),
                };
                *me.inner.relay.lock() = None;
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
                self.set(|v| {
                    v.state = "starting";
                    v.kind = kind;
                });
            }
            Err(e) => self.set(|v| {
                v.state = "failed";
                v.reason = e.to_string();
            }),
        }
    }

    /// A binary frame of the relay's type: a chunk of an access unit. On the bridge's thread: puts the unit together and queues it.
    pub fn binary(&self, payload: &[u8]) {
        let mut relay = self.inner.relay.lock();
        if let Some(r) = relay.as_mut()
            && let Some(unit) = r.assembler.push(payload)
        {
            r.queue.give(unit);
        }
    }

    /// The integration says the relay ended on its own.
    pub fn ended(&self, stream: u64, reason: &str) {
        let ours = self
            .inner
            .relay
            .lock()
            .as_ref()
            .is_some_and(|r| u64::from(r.assembler.stream) == stream);
        if ours {
            self.stop(&format!("the stream ended: {reason}"));
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

/// A started feeder with its answers read: what both ways of getting the stream feed.
struct Player {
    feeder: Feeder,
    reader: Option<(JoinHandle<()>, Arc<AtomicBool>)>,
    fed: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    loaded: bool,
    sent: u64,
}

impl Player {
    /// Starts the feeder and the thread that reads its answers (counts, the first picture, failures).
    fn open(live: &Live, spec: &FeederSpec) -> Result<Self, String> {
        let (feeder, window) = Feeder::start(spec)?;
        live.set(|v| v.window = window);
        let fed = Arc::new(AtomicU64::new(0));
        let dropped = Arc::new(AtomicU64::new(0));
        let mut stream = feeder.client.try_clone().map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .ok();
        let stop_flag = Arc::new(AtomicBool::new(false));
        let handle = {
            let (fed, dropped, live, flag) = (
                fed.clone(),
                dropped.clone(),
                live.clone(),
                stop_flag.clone(),
            );
            thread::Builder::new()
                .name("cameras-live-feeder".to_owned())
                .stack_size(128 * 1024)
                .spawn(move || {
                    let mut playing = false;
                    let mut last_publish = Instant::now();
                    while !flag.load(Ordering::SeqCst) {
                        match gf::read_frame(&mut stream) {
                            Ok((kind, payload)) => match gf::decode(kind, &payload) {
                                gf::Message::Accept(_) => {
                                    fed.fetch_add(1, Ordering::Relaxed);
                                    if !playing {
                                        playing = true;
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
                            let (f, d) =
                                (fed.load(Ordering::Relaxed), dropped.load(Ordering::Relaxed));
                            live.set(|v| {
                                v.fed = f;
                                v.dropped = d;
                            });
                        }
                    }
                })
                .map_err(|e| e.to_string())?
        };
        Ok(Self {
            feeder,
            reader: Some((handle, stop_flag)),
            fed,
            dropped,
            loaded: false,
            sent: 0,
        })
    }

    fn load(&mut self, codec: Codec, width: u32, height: u32, fps: u32) -> Result<(), String> {
        let codec = match codec {
            Codec::H264 => gf::CODEC_H264,
            Codec::H265 => gf::CODEC_H265,
        };
        self.feeder
            .client
            .send_load(codec, width, height, fps)
            .map_err(|e| e.to_string())?;
        self.feeder
            .client
            .send_state(true)
            .map_err(|e| e.to_string())?;
        self.loaded = true;
        Ok(())
    }

    fn push(&mut self, pts_ns: i64, key: bool, data: &[u8]) -> Result<(), String> {
        self.feeder
            .client
            .send_video(pts_ns, key, data)
            .map_err(|e| format!("the feeder went: {e}"))?;
        self.sent += 1;
        Ok(())
    }

    /// Ends the feeder and writes the final counts; a stream that ended by itself is given a moment for its last answers.
    fn finish(mut self, live: &Live, outcome: &Result<(), String>, stop: &AtomicBool, why: &str) {
        if outcome.is_ok() && !stop.load(Ordering::SeqCst) {
            let until = Instant::now() + Duration::from_secs(2);
            while self.fed.load(Ordering::Relaxed) + self.dropped.load(Ordering::Relaxed)
                < self.sent
                && Instant::now() < until
            {
                thread::sleep(Duration::from_millis(20));
            }
        }
        if let Some((handle, flag)) = self.reader.take() {
            flag.store(true, Ordering::SeqCst);
            let _ = handle.join();
        }
        let (f, d) = (
            self.fed.load(Ordering::Relaxed),
            self.dropped.load(Ordering::Relaxed),
        );
        drop(self.feeder);
        live.set(|v| {
            v.fed = f;
            v.dropped = d;
            if outcome.is_ok() && !stop.load(Ordering::SeqCst) {
                v.state = "stopped";
                v.reason = why.to_owned();
            }
        });
    }
}

/// The HLS pipeline, from the stream's address to the feeder, until `stop`.
fn run_hls(
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
    let mut player = Player::open(live, spec)?;
    let piece_ms = (playlist.part_target.unwrap_or(playlist.target_duration) * 1000.0) as u64;
    live.set(|v| v.piece_ms = piece_ms);
    // the first piece tells the rate; the load waits for it
    let mut pieces = playlist.live_edge();
    let mut at = pieces.last().map(hls::Playlist::position_after);
    let mut base_pts: Option<i64> = None;
    let outcome = (|| -> Result<(), String> {
        loop {
            for piece in pieces.drain(..) {
                if stop.load(Ordering::SeqCst) {
                    return Ok(());
                }
                let bytes = fetch(&hls::resolve(&playlist_url, &piece.uri))?;
                let units = fmp4::parse_segment(&init, &bytes).map_err(|e| e.to_string())?;
                if !player.loaded && !units.is_empty() {
                    let span = units.last().map_or(0, |u| u.dts) - units[0].dts;
                    let fps = if span > 0 && units.len() > 1 {
                        ((units.len() as f64 - 1.0) * f64::from(init.timescale) / span as f64)
                            .round() as u32
                    } else {
                        0
                    };
                    player.load(init.codec, init.width, init.height, fps)?;
                }
                // the feeder is given the decode time: it rises with every unit, which the decoder's pipeline insists on; a stream with
                // B pictures is shown a reorder delay later, which the pipeline does itself
                for unit in units {
                    let base = *base_pts.get_or_insert(unit.dts);
                    let pts_ns = (unit.dts - base).max(0) as i128 * 1_000_000_000
                        / i128::from(init.timescale);
                    player.push(pts_ns as i64, unit.key, &unit.data)?;
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
    player.finish(live, &outcome, stop, "the stream ended");
    outcome
}

/// The relay pipeline: units from the queue to the feeder as they come, until `stop`, the queue going, or quiet.
fn run_relay(
    live: &Live,
    spec: &FeederSpec,
    taker: relay::Taker,
    codec: Codec,
    clock: u32,
    parameter_sets: &[u8],
    stop: &AtomicBool,
) -> Result<(), String> {
    let clock = if clock == 0 { 90_000 } else { clock };
    let mut player = Player::open(live, spec)?;
    live.set(|v| v.piece_ms = 0);
    let mut last_unit = Instant::now();
    let mut last_timestamp: Option<u32> = None;
    let mut extended: i64 = 0;
    let mut last_pts: i64 = -1;
    let mut delay_max = Duration::ZERO;
    let mut last_publish = Instant::now();
    let publish = |delay_max: &mut Duration| {
        let (lost, dropped) = {
            let relay = live.inner.relay.lock();
            relay.as_ref().map_or((0, 0), |r| {
                (r.assembler.lost, r.queue.dropped.load(Ordering::Relaxed))
            })
        };
        let delay_ms = delay_max.as_millis() as u64;
        *delay_max = Duration::ZERO;
        // `lost`: what never reached the feeder (a chunk missed on the way, a unit the queue let go); `dropped` stays the pipeline's
        live.set(|v| {
            v.lost = lost + dropped;
            v.delay_ms = delay_ms;
        });
    };
    let outcome = (|| -> Result<(), String> {
        loop {
            if stop.load(Ordering::SeqCst) {
                return Ok(());
            }
            if last_publish.elapsed() >= Duration::from_secs(1) {
                last_publish = Instant::now();
                publish(&mut delay_max);
            }
            let unit = match taker.take(Duration::from_millis(100)) {
                relay::Taken::Unit(unit) => unit,
                relay::Taken::Nothing => {
                    if last_unit.elapsed() >= RELAY_QUIET {
                        return Err("the relay went quiet".to_owned());
                    }
                    continue;
                }
                relay::Taken::Gone => return Ok(()),
            };
            last_unit = Instant::now();
            if !player.loaded {
                if !unit.key {
                    continue;
                }
                let mut data = unit.data;
                let has_sets = sps::dimensions(codec, &data).is_some();
                if !has_sets && !parameter_sets.is_empty() {
                    let mut with = parameter_sets.to_vec();
                    with.extend_from_slice(&data);
                    data = with;
                }
                let (width, height) = sps::dimensions(codec, &data)
                    .ok_or("the stream's parameter sets name no picture size")?;
                player.load(codec, width, height, 0)?;
                last_timestamp = Some(unit.timestamp);
                let delay = unit.arrived.elapsed();
                delay_max = delay_max.max(delay);
                player.push(0, true, &data)?;
                continue;
            }
            // the RTP timestamp, unwrapped; the pipeline insists on rising times, so one that goes back is placed just after the last
            let previous = last_timestamp
                .replace(unit.timestamp)
                .unwrap_or(unit.timestamp);
            extended += i64::from(unit.timestamp.wrapping_sub(previous) as i32);
            let mut pts_ns = (i128::from(extended) * 1_000_000_000 / i128::from(clock)) as i64;
            if pts_ns <= last_pts {
                pts_ns = last_pts + 1_000_000;
            }
            last_pts = pts_ns;
            let delay = unit.arrived.elapsed();
            delay_max = delay_max.max(delay);
            player.push(pts_ns, unit.key, &unit.data)?;
        }
    })();
    player.finish(live, &outcome, stop, "the stream ended");
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
