//! Voice search: the words of someone holding the mic key, as text for the search box.
//!
//! The speech is turned into text by the owner's Home Assistant, through its Assist pipeline (`assist_pipeline/run`, from the
//! speech-to-text stage to the speech-to-text stage), and by nothing else: no LG service, no cloud of LG's. The text goes to the
//! search box and **nothing acts on it**: voice never runs a command, and the helper has no route that would.
//!
//! One request, as the compositor drives it (`POST /v1/voice/...`, answers on the `voice` topic):
//!
//! 1. `listen` (the mic key went down): the microphone's program starts (`voice.capture` in `helper.json`, 16 kHz mono 16-bit PCM
//!    on its standard output) and the topic says `listening`. The audio is kept while Home Assistant is dialled, so the first
//!    words are not lost to the connection.
//! 2. The WebSocket opens, authenticates with the saved token, and `assist_pipeline/run` starts at the `stt` stage and ends there.
//!    Home Assistant names a binary handler; every audio chunk goes to it, and a message of just the handler's id ends the audio.
//! 3. `stop` (the key went up): the microphone stays open [`VoiceConfig::tail_ms`](crate::config::VoiceConfig) longer so the last
//!    word is whole, then closes; the topic says `thinking` from the moment it is closed (so `listening` is exactly as long as the
//!    microphone is open) until Home Assistant answers with `stt-end`. Home Assistant may answer first (its voice-activity
//!    detector ends the speech): the microphone closes then too.
//! 4. The topic says `heard` with the text, or `error` with a code and words.
//!
//! **The microphone is open only between `listen` and the end of the audio**, and never longer than `voice.maxSeconds`, whatever
//! the key does. It is never reachable by AI access: the routes below are on the loopback API, and the MCP server has no tool for
//! them. Whether voice search exists at all is the owner's: the `base.voice` setting (off by default), a saved link to Home
//! Assistant ([`voice_store`], written by the command line) and a chosen microphone program all have to be there.
//!
//! | Request | Answer |
//! |---|---|
//! | `GET /v1/voice` | the state: `{state, session, ready, reason?, text?, error?}` |
//! | `POST /v1/voice/listen` | `{session}`, or `409` with why not: `voice_off`, `voice_unlinked`, `no_microphone`, `link_unreadable`, `busy` |
//! | `POST /v1/voice/stop` | the key went up: finish and answer |
//! | `POST /v1/voice/cancel` | stop at once, nothing is reported |
//! | `POST /v1/voice/refresh` | read the setting and the saved link again (the command line asks after it changes the link) |
//!
//! Nothing here logs the token, the audio or the words.

use crate::API_VERSION;
use crate::config::{FetchConfig, find_ca_bundle};
use crate::ctx::Ctx;
use crate::http::{Request, Response, error_response};
use crate::timeutil::now_ms;
use crate::voice_store::{self, HaLink};
use crate::voice_ws::{Fail, Ha};
use fetch::TlsFetch;
use ha_link::endpoint::AddressPolicy;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The hub topic.
pub const TOPIC: &str = "voice";

/// Method, path and what it does: the discovery document and the route test read this.
pub const ROUTES: &[(&str, &str, &str)] = &[
    (
        "GET",
        "/v1/voice",
        "voice search: its state, whether it is ready and why not, the last words heard",
    ),
    (
        "POST",
        "/v1/voice/listen",
        "the mic key went down: open the microphone and listen; 409 says why not",
    ),
    (
        "POST",
        "/v1/voice/stop",
        "the mic key went up: finish; the words arrive on the voice topic",
    ),
    (
        "POST",
        "/v1/voice/cancel",
        "stop at once and report nothing",
    ),
    (
        "POST",
        "/v1/voice/refresh",
        "read the setting and the saved link again",
    ),
];

/// The sample rate Home Assistant is told and the microphone program must write.
const SAMPLE_RATE: u32 = 16_000;
/// The audio is read from the microphone this much at a time (100 ms of 16 kHz 16-bit mono).
const CHUNK: usize = 3_200;
/// Chunks kept between the microphone and the connection: a minute and a half, so a slow connection never loses the start.
const QUEUED: usize = 900;
/// How long dialling, authenticating and starting the pipeline may take.
const CONNECT_WITHIN: Duration = Duration::from_secs(8);
/// How long the answer is waited for once the audio has ended.
const ANSWER_WITHIN: Duration = Duration::from_secs(12);
/// The longest a message from Home Assistant is quoted back.
const MAX_WORDS: usize = 120;

/// Why a request to listen was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The `base.voice` setting is off.
    Off,
    /// No Home Assistant link is saved.
    Unlinked,
    /// A link is saved and cannot be read (in words, without the token).
    Unreadable(String),
    /// `voice.capture` names no microphone.
    NoMicrophone,
    /// A request is already running.
    Busy,
}

impl Refused {
    fn code(&self) -> &'static str {
        match self {
            Self::Off => "voice_off",
            Self::Unlinked => "voice_unlinked",
            Self::Unreadable(_) => "link_unreadable",
            Self::NoMicrophone => "no_microphone",
            Self::Busy => "busy",
        }
    }

    fn message(&self) -> String {
        match self {
            Self::Off => "voice search is switched off".into(),
            Self::Unlinked => "no Home Assistant is linked: run lumatv-helper voice-link".into(),
            Self::Unreadable(why) => why.clone(),
            Self::NoMicrophone => "no microphone is chosen: helper.json needs voice.capture".into(),
            Self::Busy => "voice search is already listening".into(),
        }
    }
}

/// What a request came to.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Listening,
    Thinking,
    Heard(String),
    Failed(&'static str, String),
}

struct Inner {
    counter: u64,
    phase: Phase,
    session: u64,
    /// The running request's switches.
    active: Option<Arc<Flags>>,
}

/// What the request's thread is told from outside.
#[derive(Default)]
struct Flags {
    /// The key went up.
    release: AtomicBool,
    /// Stop now and report nothing.
    cancel: AtomicBool,
}

/// Voice search.
pub struct Voice {
    ctx: Ctx,
    policy: AddressPolicy,
    tls: Option<TlsFetch>,
    inner: Mutex<Inner>,
}

impl Voice {
    /// Voice search for the running helper: Home Assistant is dialled only on the home network.
    pub fn new(ctx: &Ctx) -> Arc<Self> {
        Self::with_policy(ctx, AddressPolicy::HOME)
    }

    /// The same with another rule for which addresses may be dialled (a test dials the loopback).
    pub fn with_policy(ctx: &Ctx, policy: AddressPolicy) -> Arc<Self> {
        let voice = Arc::new(Self {
            ctx: ctx.clone(),
            policy,
            tls: tls_for(&ctx.config.fetch),
            inner: Mutex::new(Inner {
                counter: 0,
                phase: Phase::Idle,
                session: 0,
                active: None,
            }),
        });
        voice.publish();
        // A closed switch must not leave the microphone open.
        {
            let weak = Arc::downgrade(&voice);
            ctx.stop.on_stop(move || {
                if let Some(voice) = weak.upgrade() {
                    voice.cancel();
                }
            });
        }
        // A setting that changes is a different `ready` for the surfaces that show the mic hint, and a switch turned off ends a
        // request that is listening.
        {
            let weak = Arc::downgrade(&voice);
            ctx.settings.on_change("base.voice", move || {
                if let Some(voice) = weak.upgrade() {
                    if voice.check().is_err() {
                        voice.cancel();
                    }
                    voice.publish();
                }
            });
        }
        voice
    }

    /// Whether a request can start now, and with what.
    fn check(&self) -> Result<(HaLink, String), Refused> {
        let settings = self.ctx.settings.voice();
        if !settings.enabled {
            return Err(Refused::Off);
        }
        if self.ctx.config.voice.capture.is_empty() {
            return Err(Refused::NoMicrophone);
        }
        match voice_store::load(&self.ctx.paths.data_dir) {
            Ok(Some(link)) => Ok((link, settings.pipeline)),
            Ok(None) => Err(Refused::Unlinked),
            Err(e) => Err(Refused::Unreadable(e.to_string())),
        }
    }

    /// The `voice` topic's document as it stands.
    pub fn status(&self) -> Value {
        let (phase, session) = {
            let inner = self.inner.lock();
            (inner.phase.clone(), inner.session)
        };
        let mut doc = json!({"session": session, "ready": true});
        let map = doc.as_object_mut().expect("an object");
        match &phase {
            Phase::Idle => map.insert("state".into(), json!("idle")),
            Phase::Listening => map.insert("state".into(), json!("listening")),
            Phase::Thinking => map.insert("state".into(), json!("thinking")),
            Phase::Heard(text) => {
                map.insert("text".into(), json!(text));
                map.insert("state".into(), json!("heard"))
            }
            Phase::Failed(code, message) => {
                map.insert("error".into(), json!({"code": code, "message": message}));
                map.insert("state".into(), json!("error"))
            }
        };
        if let Err(why) = self.check() {
            // Busy cannot come from `check`: a request that is running still leaves voice search ready.
            map.insert("ready".into(), json!(false));
            map.insert("reason".into(), json!(why.code()));
        }
        doc
    }

    fn publish(&self) {
        self.ctx.hub.set(TOPIC, self.status());
    }

    fn set_phase(&self, session: u64, phase: Phase) {
        {
            let mut inner = self.inner.lock();
            // A request that was replaced or cancelled says nothing more.
            if inner.session != session {
                return;
            }
            inner.phase = phase;
        }
        self.publish();
    }

    /// Starts a request: the mic key went down.
    pub fn listen(self: &Arc<Self>) -> Result<u64, Refused> {
        let (link, pipeline) = self.check()?;
        let flags = Arc::new(Flags::default());
        let session = {
            let mut inner = self.inner.lock();
            if matches!(inner.phase, Phase::Listening | Phase::Thinking) {
                return Err(Refused::Busy);
            }
            inner.counter += 1;
            inner.session = inner.counter;
            inner.phase = Phase::Listening;
            inner.active = Some(flags.clone());
            inner.session
        };
        self.publish();
        let voice = self.clone();
        let spawned = thread::Builder::new()
            .name("voice".into())
            .spawn(move || voice.run(session, &flags, &link, &pipeline));
        if let Err(e) = spawned {
            crate::log_error!("voice: cannot start a thread: {e}");
            self.set_phase(
                session,
                Phase::Failed("internal", "voice search could not start".into()),
            );
        }
        Ok(session)
    }

    /// The mic key went up. True when a request was listening.
    pub fn stop(&self) -> bool {
        let inner = self.inner.lock();
        match (&inner.phase, &inner.active) {
            (Phase::Listening, Some(flags)) => {
                flags.release.store(true, Ordering::SeqCst);
                true
            }
            _ => false,
        }
    }

    /// Ends a request at once and reports nothing. True when one was running.
    pub fn cancel(&self) -> bool {
        let running = {
            let mut inner = self.inner.lock();
            let running = matches!(inner.phase, Phase::Listening | Phase::Thinking);
            if let Some(flags) = inner.active.take() {
                flags.cancel.store(true, Ordering::SeqCst);
            }
            if running {
                inner.phase = Phase::Idle;
            }
            running
        };
        if running {
            self.publish();
        }
        running
    }

    /// Reads the setting and the saved link again.
    pub fn refresh(&self) {
        if self.check().is_err() {
            self.cancel();
        }
        self.publish();
    }

    /// The request's thread.
    fn run(&self, session: u64, flags: &Flags, link: &HaLink, pipeline: &str) {
        let result = self.converse(session, flags, link, pipeline);
        // The request is over whichever way: the microphone program is already gone (the capture drops with `converse`).
        {
            let mut inner = self.inner.lock();
            if inner.session == session {
                inner.active = None;
            }
        }
        match result {
            Ok(text) => {
                crate::log_info!("voice: heard {} characters", text.chars().count());
                self.set_phase(session, Phase::Heard(text));
            }
            Err(Ended::Cancelled) => {}
            Err(Ended::Failed(code, message)) => {
                crate::log_warn!("voice: {code}: {message}");
                self.set_phase(session, Phase::Failed(code, message));
            }
        }
    }

    fn converse(
        &self,
        session: u64,
        flags: &Flags,
        link: &HaLink,
        pipeline: &str,
    ) -> Result<String, Ended> {
        let config = &self.ctx.config.voice;
        let started = Instant::now();
        let hard_stop = started + Duration::from_secs(config.max_seconds);
        let tail = Duration::from_millis(config.tail_ms);

        let mut capture = Capture::start(&config.capture).map_err(|e| {
            crate::log_warn!("voice: the microphone program did not start: {e}");
            Ended::Failed("microphone", "The microphone could not be opened".into())
        })?;

        let connect_by = Instant::now() + CONNECT_WITHIN;
        let mut ha = Ha::connect(link, self.policy, self.tls.as_ref(), connect_by)
            .map_err(Ended::from_fail)?;
        if flags.cancel.load(Ordering::SeqCst) {
            return Err(Ended::Cancelled);
        }
        let mut run = json!({
            "id": 1,
            "type": "assist_pipeline/run",
            "start_stage": "stt",
            "end_stage": "stt",
            "input": {"sample_rate": SAMPLE_RATE},
            "timeout": config.max_seconds + 15,
        });
        if !pipeline.is_empty() {
            run["pipeline"] = json!(pipeline);
        }
        ha.send_json(&run, connect_by).map_err(Ended::from_fail)?;
        let handler = await_run_start(&mut ha, flags, connect_by)?;

        let mut sent_bytes: usize = 0;
        let mut released_at: Option<Instant> = None;
        let mut audio_ended: Option<Instant> = None;
        loop {
            if flags.cancel.load(Ordering::SeqCst) {
                return Err(Ended::Cancelled);
            }
            let now = Instant::now();
            if released_at.is_none() && (flags.release.load(Ordering::SeqCst) || now >= hard_stop) {
                released_at = Some(now);
            }
            // The audio
            if audio_ended.is_none() {
                let tail_over = released_at.is_some_and(|at| now >= at + tail);
                let step = if tail_over {
                    // The key is up and the tail is over: close the microphone and send what it had.
                    capture.stop();
                    Step::Closed
                } else {
                    capture.next(Duration::from_millis(20))
                };
                match step {
                    Step::Chunk(bytes) => {
                        sent_bytes += bytes.len();
                        ha.send_audio(handler, &bytes, now + CONNECT_WITHIN)
                            .map_err(Ended::from_fail)?;
                    }
                    Step::Idle => {}
                    Step::Closed => {
                        // Whatever the microphone still held goes first.
                        while let Step::Chunk(bytes) = capture.next(Duration::ZERO) {
                            sent_bytes += bytes.len();
                            ha.send_audio(handler, &bytes, now + CONNECT_WITHIN)
                                .map_err(Ended::from_fail)?;
                        }
                        if sent_bytes == 0 {
                            return Err(Ended::Failed(
                                "microphone",
                                "The microphone gave no sound".into(),
                            ));
                        }
                        ha.send_audio(handler, &[], now + CONNECT_WITHIN)
                            .map_err(Ended::from_fail)?;
                        audio_ended = Some(Instant::now());
                        // The microphone is closed: only now is it waiting for Home Assistant, so a screen that shows the
                        // microphone's state can trust `listening` for exactly as long as it is open.
                        self.set_phase(session, Phase::Thinking);
                    }
                }
            } else if audio_ended.is_some_and(|at| at.elapsed() > ANSWER_WITHIN) {
                return Err(Ended::Failed(
                    "timeout",
                    "Home Assistant did not answer in time".into(),
                ));
            }
            // What Home Assistant says: with the audio going, only what is there; after it, wait for it a little at a time.
            let wait = if audio_ended.is_some() {
                Duration::from_millis(50)
            } else {
                Duration::ZERO
            };
            while let Some(message) = ha.recv(wait).map_err(Ended::from_fail)? {
                if let Some(text) = read_message(&message)? {
                    capture.stop();
                    ha.close();
                    return Ok(text);
                }
                if wait > Duration::ZERO {
                    break;
                }
            }
        }
    }
}

/// How a request ended other than with words.
enum Ended {
    Cancelled,
    Failed(&'static str, String),
}

impl Ended {
    fn from_fail(fail: Fail) -> Self {
        match fail {
            Fail::AuthInvalid => Self::Failed(
                "auth",
                "Home Assistant refused the saved token: link it again".into(),
            ),
            Fail::Cannot(why) => Self::Failed("unreachable", why),
            Fail::Lost(why) => Self::Failed("lost", why),
        }
    }
}

/// Waits for `run-start` and returns the id of the binary handler the audio goes to.
fn await_run_start(ha: &mut Ha, flags: &Flags, by: Instant) -> Result<u8, Ended> {
    loop {
        if flags.cancel.load(Ordering::SeqCst) {
            return Err(Ended::Cancelled);
        }
        let left = by.saturating_duration_since(Instant::now());
        let Some(message) = ha
            .recv(left.min(Duration::from_millis(100)))
            .map_err(Ended::from_fail)?
        else {
            if left.is_zero() {
                return Err(Ended::Failed(
                    "timeout",
                    "Home Assistant did not start listening in time".into(),
                ));
            }
            continue;
        };
        if message["type"] == "event" && message["event"]["type"] == "run-start" {
            let id = message["event"]["data"]["runner_data"]["stt_binary_handler_id"]
                .as_u64()
                .and_then(|id| u8::try_from(id).ok())
                .ok_or_else(|| {
                    Ended::Failed(
                        "pipeline",
                        "Home Assistant gave no place for the audio".into(),
                    )
                })?;
            return Ok(id);
        }
        // An error before the start (an unknown pipeline) ends it; anything else is not the start yet.
        read_message(&message)?;
    }
}

/// What one message from Home Assistant means: `Ok(Some(text))` is the words, `Ok(None)` is nothing yet, `Err` is the end.
fn read_message(message: &Value) -> Result<Option<String>, Ended> {
    match message["type"].as_str() {
        Some("result") if message["success"] == false => {
            let code = message["error"]["code"].as_str().unwrap_or("error");
            let words = words_of(&message["error"]["message"]);
            Err(Ended::Failed(
                "pipeline",
                if words.is_empty() {
                    format!("Home Assistant refused: {code}")
                } else {
                    words
                },
            ))
        }
        Some("event") => {
            let event = &message["event"];
            match event["type"].as_str() {
                Some("stt-end") => {
                    let text = event["data"]["stt_output"]["text"]
                        .as_str()
                        .unwrap_or("")
                        .trim();
                    if text.is_empty() {
                        Err(Ended::Failed("nothing_heard", "Nothing was heard".into()))
                    } else {
                        Ok(Some(text.chars().take(200).collect()))
                    }
                }
                Some("error") => {
                    let code = event["data"]["code"].as_str().unwrap_or("");
                    if code == "stt-no-text-recognized" {
                        Err(Ended::Failed("nothing_heard", "Nothing was heard".into()))
                    } else {
                        let words = words_of(&event["data"]["message"]);
                        Err(Ended::Failed(
                            "stt",
                            if words.is_empty() {
                                "Home Assistant could not turn the speech into text".into()
                            } else {
                                words
                            },
                        ))
                    }
                }
                // The run ended and no words came with it.
                Some("run-end") => Err(Ended::Failed(
                    "stt",
                    "Home Assistant ended without any words".into(),
                )),
                _ => Ok(None),
            }
        }
        _ => Ok(None),
    }
}

/// A message of Home Assistant's, bounded and cleaned, for the screen.
fn words_of(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or("")
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_WORDS)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The TLS child a secure Home Assistant address is dialled through: the system's `openssl s_client`, as the weather's HTTPS is.
fn tls_for(config: &FetchConfig) -> Option<TlsFetch> {
    let bundle = find_ca_bundle(config)?;
    let program =
        crate::sys::which(&config.openssl).unwrap_or_else(|| config.openssl.clone().into());
    let tls = TlsFetch::new(program, bundle);
    if crate::sys::is_root()
        && let Some(user) = &config.user
        && let Some((uid, gid)) = crate::sys::lookup_user(user)
    {
        return Some(tls.unprivileged(uid, gid));
    }
    Some(tls)
}

// -------------------------------------------------------------------------------------------------------------- the microphone

/// What the microphone had.
enum Step {
    Chunk(Vec<u8>),
    /// Nothing yet.
    Idle,
    /// The program has ended (or was stopped) and everything it said was taken.
    Closed,
}

/// The microphone program and what it has written. It is killed, with everything it started, when this is stopped or dropped.
struct Capture {
    child: Child,
    audio: Receiver<Vec<u8>>,
    reader: Option<JoinHandle<()>>,
}

impl Capture {
    fn start(argv: &[String]) -> std::io::Result<Self> {
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| std::io::Error::other("no microphone program"))?;
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            // Its own process group, so stopping it stops whatever it started too.
            .process_group(0)
            .spawn()?;
        let Some(mut out) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::other(
                "the microphone program has no output",
            ));
        };
        let (tx, audio) = mpsc::sync_channel(QUEUED);
        let reader = thread::Builder::new()
            .name("voice-mic".into())
            .spawn(move || {
                let mut buf = vec![0u8; CHUNK];
                loop {
                    match out.read(&mut buf) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => {
                            if tx.send(buf[..n].to_vec()).is_err() {
                                return;
                            }
                        }
                    }
                }
            })?;
        Ok(Self {
            child,
            audio,
            reader: Some(reader),
        })
    }

    /// The next chunk, waiting up to `wait`.
    fn next(&mut self, wait: Duration) -> Step {
        let got = if wait.is_zero() {
            self.audio.try_recv().map_err(|e| match e {
                mpsc::TryRecvError::Empty => RecvTimeoutError::Timeout,
                mpsc::TryRecvError::Disconnected => RecvTimeoutError::Disconnected,
            })
        } else {
            self.audio.recv_timeout(wait)
        };
        match got {
            Ok(bytes) => Step::Chunk(bytes),
            Err(RecvTimeoutError::Timeout) => Step::Idle,
            Err(RecvTimeoutError::Disconnected) => Step::Closed,
        }
    }

    /// Closes the microphone: the program and its group are killed and collected. What it had already written stays to be read.
    fn stop(&mut self) {
        if self.reader.is_none() {
            return;
        }
        // SAFETY: a signal to the process group this child leads; the pid is the child's own, which is not yet collected.
        unsafe {
            libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop();
    }
}

// ------------------------------------------------------------------------------------------------------------------- the routes

/// Serves `/v1/voice/...`.
pub fn handle(voice: &Arc<Voice>, request: &Request, rest: &[&str]) -> Response {
    let get = request.method == "GET";
    let post = request.method == "POST";
    match rest {
        [] if get => ok_response(json!({"voice": voice.status()})),
        ["listen"] if post => match voice.listen() {
            Ok(session) => ok_response(json!({"session": session})),
            Err(why) => error_response(409, why.code(), &why.message()),
        },
        ["stop"] if post => ok_response(json!({"stopped": voice.stop()})),
        ["cancel"] if post => ok_response(json!({"cancelled": voice.cancel()})),
        ["refresh"] if post => {
            voice.refresh();
            ok_response(json!({"voice": voice.status()}))
        }
        [] | ["listen" | "stop" | "cancel" | "refresh"] => {
            error_response(405, "method_not_allowed", "that method is not served here")
                .with_header("Allow", if rest.is_empty() { "GET" } else { "POST" })
        }
        _ => error_response(404, "not_found", "no such endpoint"),
    }
}

/// A `200` with the API's envelope around `body`.
fn ok_response(mut body: Value) -> Response {
    if let Some(map) = body.as_object_mut() {
        map.insert("ok".into(), Value::Bool(true));
        map.insert("api".into(), json!(API_VERSION));
        map.insert("now".into(), json!(now_ms()));
    }
    Response::json(200, &body)
}
