//! The voice socket's server: one thread that owns the microphone and the speaker, and serves one client (the voice host) over
//! the voice wire (`voice-wire`, the language LumaShow's sound service speaks, so the same host drives it).
//!
//! The microphone is opened when the client asks to listen and closed when it mutes or goes; nothing reads it otherwise, and while
//! it is closed nothing is heard. While it is open the audio goes through the engine and is dropped, except for a turn the client
//! has accepted and, at the client's own request, a [`Tap`] (voice search on the mic key: one reader of the device, so the tap is
//! this stream and not a second `arecord`).

use crate::engine::{Engine, EngineConfig, Out};
use crate::input::{Card, Input};
use crate::level::{Meter, strictness};
use crate::models;
use crate::pcm::{Frames, Pick, RATE, peak_dbfs};
use crate::speech::{Poll, Speaker};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use voice_wire::{Decoder, EndReason, Event, MAX_PCM_BYTES, Request, SpeechEnd, VERSION, encode};

/// How the server hears and what it asks of a wake.
#[derive(Clone, Debug)]
pub struct Config {
    /// Software gain on the microphone, in decibels.
    pub gain_db: f32,
    pub pick: Pick,
    /// The model's cutoff (see [`models::Sensitivity`]).
    pub cutoff: f32,
    pub engine: EngineConfig,
    /// Whether the TV's level is read at all (and, when it is, [`Handle::tv_playing`] says when).
    pub reference: bool,
    /// Exact digital silence for this long calls the microphone silent.
    pub silent_after: Duration,
    /// No byte for this long calls the capture stalled, and it is opened again.
    pub stall_after: Duration,
    /// How often the status is refreshed and the silence and stall checks run.
    pub tick: Duration,
    /// How long after a failed open the microphone is tried again.
    pub retry: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            gain_db: 12.0,
            pick: Pick::Auto,
            cutoff: models::Sensitivity::Normal.cutoff(),
            engine: EngineConfig::default(),
            reference: true,
            silent_after: Duration::from_secs(10),
            stall_after: Duration::from_secs(5),
            tick: Duration::from_secs(1),
            retry: Duration::from_secs(10),
        }
    }
}

/// What the server is doing, for a settings page and a status line (levels, counts and words; never audio).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// The microphone is open.
    pub capturing: bool,
    /// The host is connected.
    pub client: bool,
    pub in_turn: bool,
    /// Another reader (voice search) holds the stream.
    pub tapped: bool,
    pub injecting: bool,
    pub speaking: bool,
    /// The loudest sample of the last second, dBFS (-120: none).
    pub mic_db: f32,
    /// The TV's level, dBFS, while it is read.
    pub tv_db: Option<f32>,
    pub pick: Option<Pick>,
    /// The highest the wake model's window average reached in the last second (0 to 1).
    pub score: f32,
    /// Why the microphone is not open when it should be, in words.
    pub error: Option<String>,
    /// The microphone has given exact silence for [`Config::silent_after`] seconds.
    pub silent: bool,
    pub wakes: u64,
    /// Detections held back by the TV's level.
    pub held_back: u64,
    pub clipped: u64,
}

/// The audio of a [`Handle::tap`]: 16 kHz mono, from the stream the engine reads. While it is held no wake word is listened for.
pub struct Tap {
    rx: Receiver<Vec<i16>>,
}

impl Tap {
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Vec<i16>, mpsc::RecvTimeoutError> {
        self.rx.recv_timeout(timeout)
    }

    pub fn try_recv(&self) -> Result<Vec<i16>, mpsc::TryRecvError> {
        self.rx.try_recv()
    }
}

enum Command {
    Tap(SyncSender<Result<Tap, String>>),
    Inject(Vec<i16>, SyncSender<Result<(), String>>),
}

struct Shared {
    stop: Eventfd,
    wake: Eventfd,
    commands: Mutex<VecDeque<Command>>,
    status: Mutex<Status>,
    tv_playing: AtomicBool,
}

/// The running server.
pub struct Handle {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Handle {
    pub fn status(&self) -> Status {
        self.shared.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Says whether the TV is playing sound, so that its level is worth reading (the reference stream is opened only then).
    pub fn tv_playing(&self, playing: bool) {
        self.shared
            .tv_playing
            .store(playing, Ordering::Relaxed);
    }

    fn ask<T>(&self, make: impl FnOnce(SyncSender<Result<T, String>>) -> Command) -> Result<T, String> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.shared
            .commands
            .lock()
            .map_err(|_| "the voice server is gone".to_owned())?
            .push_back(make(tx));
        self.shared.wake.signal();
        rx.recv_timeout(Duration::from_secs(2))
            .map_err(|_| "the voice server did not answer".to_owned())?
    }

    /// The microphone's audio for one other reader, or why there is none (it is not open, a turn is open, or another reader has it).
    pub fn tap(&self) -> Result<Tap, String> {
        self.ask(Command::Tap)
    }

    /// Replaces the microphone's audio with this clip (16 kHz mono) at the pace the microphone runs, then silence for the end of
    /// the turn: what a self-check does so that no speaker is needed. Refused unless the microphone is open and idle.
    pub fn inject(&self, clip: Vec<i16>) -> Result<(), String> {
        self.ask(|tx| Command::Inject(clip, tx))
    }

    /// Stops the server: the microphone and the speaker are released before this returns.
    pub fn stop(mut self) {
        self.shared.stop.signal();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.shared.stop.signal();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Binds `socket` and runs the server.
pub fn start(
    socket: &Path,
    card: Arc<dyn Card>,
    speaker: Box<dyn Speaker>,
    config: Config,
) -> io::Result<Handle> {
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    listener.set_nonblocking(true)?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    }
    let shared = Arc::new(Shared {
        stop: Eventfd::new()?,
        wake: Eventfd::new()?,
        commands: Mutex::new(VecDeque::new()),
        status: Mutex::new(Status {
            mic_db: -120.0,
            ..Status::default()
        }),
        tv_playing: AtomicBool::new(false),
    });
    let server = Server {
        shared: shared.clone(),
        socket: socket.to_owned(),
        listener,
        card,
        speaker,
        config,
        client: None,
        wanted: None,
        capture: None,
        reference: None,
        injection: None,
        tap: None,
        retry_at: None,
        failure: None,
        wakes: 0,
        speaking: false,
    };
    let thread = thread::Builder::new()
        .name("voice-server".to_owned())
        .stack_size(512 * 1024)
        .spawn(move || server.run())?;
    Ok(Handle {
        shared,
        thread: Some(thread),
    })
}

struct Eventfd(OwnedFd);

impl Eventfd {
    fn new() -> io::Result<Self> {
        // SAFETY: a new descriptor, owned from here on.
        let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `fd` is a valid descriptor nothing else owns.
        Ok(Self(unsafe { OwnedFd::from_raw_fd(fd) }))
    }

    fn signal(&self) {
        let one = 1u64.to_ne_bytes();
        // SAFETY: eight bytes to an eventfd.
        unsafe { libc::write(self.0.as_raw_fd(), one.as_ptr().cast(), 8) };
    }

    fn drain(&self) {
        let mut buf = [0u8; 8];
        // SAFETY: eight bytes from an eventfd.
        unsafe { libc::read(self.0.as_raw_fd(), buf.as_mut_ptr().cast(), 8) };
    }

    fn fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

/// The client's connection: reads in pieces, writes whole messages.
struct Client {
    stream: UnixStream,
    decoder: Decoder,
}

impl Client {
    fn send(&mut self, event: &Event) -> io::Result<()> {
        let frame = encode(event)?;
        let until = Instant::now() + Duration::from_millis(500);
        let mut written = 0;
        while written < frame.len() {
            match self.stream.write(&frame[written..]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => written += n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock && Instant::now() < until => {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// What has arrived; `Err` when the connection is closed or not to be trusted.
    fn read(&mut self) -> io::Result<Vec<Request>> {
        let mut buf = [0u8; 8192];
        loop {
            match self.stream.read(&mut buf) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => self.decoder.push(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        let mut requests = Vec::new();
        while let Some(r) = self.decoder.take::<Request>()? {
            requests.push(r);
        }
        Ok(requests)
    }
}

/// The open microphone and what is made of it.
struct Capture {
    input: Box<dyn Input>,
    frames: Frames,
    engine: Engine,
    /// The last time a byte arrived.
    fed: Instant,
    /// Since when the stream has been exact silence.
    silent_since: Option<Instant>,
    /// Samples still to drop at the start.
    settle: usize,
}

struct Reference {
    input: Box<dyn Input>,
    meter: Meter,
}

/// A clip being played into the engine in place of the microphone.
struct Injection {
    clip: Vec<i16>,
    at: usize,
    /// Silence after the clip, in samples: the end of what was said, for whoever listens for it.
    tail: usize,
}

/// Seconds of silence after an injected clip: longer than a speech service waits to call the end of a sentence.
const INJECT_TAIL_SECS: usize = 4;
/// Samples dropped at the start of a capture: the TV's microphone opens with a 40 ms pop at full scale (measured), which no wake
/// word or noise estimate should learn from.
const SETTLE: usize = RATE as usize / 4;

struct Server {
    shared: Arc<Shared>,
    socket: PathBuf,
    listener: UnixListener,
    card: Arc<dyn Card>,
    speaker: Box<dyn Speaker>,
    config: Config,
    client: Option<Client>,
    /// The words the client asked to hear.
    wanted: Option<Vec<String>>,
    capture: Option<Capture>,
    reference: Option<Reference>,
    injection: Option<Injection>,
    tap: Option<SyncSender<Vec<i16>>>,
    retry_at: Option<Instant>,
    /// Why the microphone is not open, until it is.
    failure: Option<String>,
    wakes: u64,
    /// A reply is being played (until the speaker says it ended).
    speaking: bool,
}

fn poll_fds(fds: &mut [libc::pollfd], timeout: Duration) {
    // SAFETY: a valid array of pollfd structures.
    unsafe {
        libc::poll(
            fds.as_mut_ptr(),
            fds.len() as libc::nfds_t,
            timeout.as_millis().min(i32::MAX as u128) as i32,
        );
    }
}

fn readable(fd: RawFd) -> libc::pollfd {
    libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    }
}

fn bytes_of(pcm: &[i16]) -> Vec<u8> {
    pcm.iter().flat_map(|s| s.to_le_bytes()).collect()
}

impl Server {
    fn run(mut self) {
        let mut next_tick = Instant::now() + self.config.tick;
        let mut scratch = vec![0u8; 8192];
        loop {
            // Descriptors, in a fixed order: stop, wake, listener, then the optional ones whose indices are noted.
            let mut fds = vec![
                readable(self.shared.stop.fd()),
                readable(self.shared.wake.fd()),
                readable(self.listener.as_raw_fd()),
            ];
            let client_at = self.client.as_ref().map(|c| {
                fds.push(readable(c.stream.as_raw_fd()));
                fds.len() - 1
            });
            let mic_at = self.capture.as_ref().map(|c| {
                fds.push(readable(c.input.fd()));
                fds.len() - 1
            });
            let reference_at = self.reference.as_ref().map(|r| {
                fds.push(readable(r.input.fd()));
                fds.len() - 1
            });
            let now = Instant::now();
            let mut timeout = next_tick.saturating_duration_since(now);
            if self.speaking {
                timeout = timeout.min(Duration::from_millis(20));
            }
            if let Some(at) = self.retry_at {
                timeout = timeout.min(at.saturating_duration_since(now));
            }
            poll_fds(&mut fds, timeout);
            let hit = |i: usize| fds[i].revents != 0;
            if hit(0) {
                break;
            }
            if hit(1) {
                self.shared.wake.drain();
                self.commands();
            }
            if hit(2) {
                self.accept();
            }
            if client_at.is_some_and(hit) {
                self.client_ready();
            }
            if mic_at.is_some_and(hit) {
                self.mic_ready(&mut scratch);
            }
            if reference_at.is_some_and(hit) {
                self.reference_ready(&mut scratch);
            }
            self.speaker_tick();
            if self.retry_at.is_some_and(|at| Instant::now() >= at) {
                self.retry_at = None;
                self.open_capture();
            }
            if Instant::now() >= next_tick {
                next_tick = Instant::now() + self.config.tick;
                self.tick();
            }
        }
        self.speaker.stop();
        self.capture = None;
        self.reference = None;
        let _ = std::fs::remove_file(&self.socket);
    }

    fn send(&mut self, event: Event) {
        if let Some(client) = self.client.as_mut()
            && client.send(&event).is_err()
        {
            self.client_gone();
        }
    }

    fn accept(&mut self) {
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    let _ = stream.set_nonblocking(true);
                    let mut incoming = Client {
                        stream,
                        decoder: Decoder::default(),
                    };
                    if self.client.is_some() {
                        let _ = incoming.send(&Event::Failed {
                            text: "another client has the microphone".to_owned(),
                        });
                        continue;
                    }
                    self.client = Some(incoming);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
    }

    /// The client is gone: nothing listens, nothing speaks, the microphone is closed.
    fn client_gone(&mut self) {
        self.client = None;
        self.mute();
    }

    fn mute(&mut self) {
        self.wanted = None;
        self.retry_at = None;
        self.failure = None;
        self.speaker.stop();
        self.speaking = false;
        self.injection = None;
        self.capture = None;
        self.reference = None;
    }

    fn client_ready(&mut self) {
        let requests = match self.client.as_mut().map(Client::read) {
            Some(Ok(requests)) => requests,
            Some(Err(_)) => {
                self.client_gone();
                return;
            }
            None => return,
        };
        for request in requests {
            self.request(request);
            if self.client.is_none() {
                break;
            }
        }
    }

    fn request(&mut self, request: Request) {
        match request {
            Request::Hello { version } => {
                if version == VERSION {
                    self.send(Event::Ready { rate: RATE });
                } else {
                    self.send(Event::Failed {
                        text: format!("the voice wire is version {VERSION}, not {version}"),
                    });
                    self.client = None;
                }
            }
            Request::Listen { words } => self.listen(words),
            Request::Mute => {
                let was_in_turn = self.capture.as_ref().is_some_and(|c| c.engine.in_turn());
                self.mute();
                if was_in_turn {
                    self.send(Event::TurnEnded {
                        reason: EndReason::Muted,
                    });
                }
                self.send(Event::Muted);
            }
            Request::Accept => {
                let mut out = Vec::new();
                if let Some(c) = self.capture.as_mut() {
                    c.engine.accept(&mut out);
                }
                self.emit(out);
            }
            Request::Open => {
                if let Some(c) = self.capture.as_mut() {
                    c.engine.open();
                }
            }
            Request::End => {
                let mut out = Vec::new();
                if let Some(c) = self.capture.as_mut() {
                    c.engine.end(EndReason::Client, &mut out);
                }
                self.emit(out);
            }
            Request::Speak { rate, percent } => match self.speaker.begin(rate, percent) {
                Ok(()) => self.speaking = true,
                Err(why) => self.send(Event::SpeechDone {
                    end: SpeechEnd::Refused { why },
                }),
            },
            Request::Pcm { pcm } => {
                if pcm.len() <= MAX_PCM_BYTES {
                    self.speaker.push(&pcm);
                }
            }
            Request::SpeakEnd => self.speaker.finish(),
            Request::StopSpeech => {
                if self.speaking {
                    self.speaker.stop();
                    self.speaking = false;
                    self.send(Event::SpeechDone { end: SpeechEnd::Cut });
                }
            }
        }
    }

    fn listen(&mut self, words: Vec<String>) {
        if words.is_empty() {
            self.send(Event::Failed {
                text: "no wake word was asked for".to_owned(),
            });
            return;
        }
        for id in &words {
            if models::find(id).is_none() {
                self.send(Event::Failed {
                    text: format!("there is no wake word {id}"),
                });
                return;
            }
        }
        self.wanted = Some(words);
        // New words replace the engine, never the stream: the device is not closed and reopened for a change of word.
        self.failure = None;
        self.retry_at = None;
        if self.capture.is_some() {
            match self.engine() {
                Ok(engine) => {
                    if let Some(c) = self.capture.as_mut() {
                        c.engine = engine;
                    }
                    self.listening();
                }
                Err(text) => {
                    self.capture = None;
                    self.failed(text);
                }
            }
        } else {
            self.open_capture();
        }
    }

    fn listening(&mut self) {
        let words = self.wanted.clone().unwrap_or_default();
        self.send(Event::Listening { words });
    }

    fn engine(&self) -> Result<Engine, String> {
        let mut words = Vec::new();
        for id in self.wanted.as_deref().unwrap_or_default() {
            words.push(models::load(id, self.config.cutoff)?);
        }
        Engine::new(self.config.engine.clone(), words)
    }

    /// Opens the microphone for the words asked. A failure is told to the client once, and tried again after [`RETRY`].
    fn open_capture(&mut self) {
        if self.wanted.is_none() || self.capture.is_some() {
            return;
        }
        let opened = (|| {
            let engine = self.engine()?;
            let frames = Frames::new(self.card.mic_format(), self.config.gain_db, self.config.pick)
                .map_err(|e| e.to_string())?;
            let input = self
                .card
                .open_mic()
                .map_err(|e| format!("the microphone did not open: {e}"))?;
            Ok::<_, String>(Capture {
                input,
                frames,
                engine,
                fed: Instant::now(),
                silent_since: None,
                settle: SETTLE,
            })
        })();
        match opened {
            Ok(capture) => {
                self.capture = Some(capture);
                self.failure = None;
                self.retry_at = None;
                self.listening();
            }
            Err(text) => self.failed(text),
        }
    }

    fn failed(&mut self, text: String) {
        let first = self.failure.as_deref() != Some(text.as_str());
        self.failure = Some(text.clone());
        self.retry_at = Some(Instant::now() + self.config.retry);
        if first {
            self.send(Event::Failed { text });
        }
    }

    fn emit(&mut self, out: Vec<Out>) {
        for o in out {
            match o {
                Out::Wake { word, score, level } => {
                    self.wakes += 1;
                    self.send(Event::Wake { word, score, level });
                }
                Out::Audio(pcm) => self.send(Event::Audio { pcm: bytes_of(&pcm) }),
                Out::Ended(reason) => self.send(Event::TurnEnded { reason }),
            }
        }
    }

    fn commands(&mut self) {
        loop {
            let command = self.shared.commands.lock().ok().and_then(|mut c| c.pop_front());
            let Some(command) = command else { break };
            match command {
                Command::Tap(reply) => {
                    let answer = self.new_tap();
                    let _ = reply.send(answer);
                }
                Command::Inject(clip, reply) => {
                    let answer = self.start_injection(clip);
                    let _ = reply.send(answer);
                }
            }
        }
    }

    fn new_tap(&mut self) -> Result<Tap, String> {
        let Some(capture) = self.capture.as_ref() else {
            return Err("the assistant's microphone is not open".to_owned());
        };
        if capture.engine.in_turn() {
            return Err("the assistant is listening to someone".to_owned());
        }
        if self.tap.is_some() {
            return Err("the microphone is already being read".to_owned());
        }
        let (tx, rx) = mpsc::sync_channel(64);
        self.tap = Some(tx);
        Ok(Tap { rx })
    }

    fn start_injection(&mut self, clip: Vec<i16>) -> Result<(), String> {
        let Some(capture) = self.capture.as_ref() else {
            return Err("the microphone is not open".to_owned());
        };
        if capture.engine.in_turn() || self.injection.is_some() || self.tap.is_some() {
            return Err("the microphone is busy".to_owned());
        }
        self.injection = Some(Injection {
            clip,
            at: 0,
            tail: INJECT_TAIL_SECS * RATE as usize,
        });
        Ok(())
    }

    fn mic_ready(&mut self, scratch: &mut [u8]) {
        let mut mono: Vec<i16> = Vec::new();
        let mut failed = None;
        if let Some(c) = self.capture.as_mut() {
            // A few reads at most per wake-up, so that a stream that is far ahead cannot starve the socket.
            for _ in 0..4 {
                match c.input.read(scratch) {
                    Ok(0) => break,
                    Ok(n) => {
                        c.fed = Instant::now();
                        c.frames.push(&scratch[..n], &mut mono);
                        if n < scratch.len() {
                            break;
                        }
                    }
                    Err(e) => {
                        failed = Some(format!("the microphone stopped: {e}"));
                        break;
                    }
                }
            }
        }
        if let Some(text) = failed {
            self.capture_lost(text);
            return;
        }
        if let Some(c) = self.capture.as_mut() {
            let skip = c.settle.min(mono.len());
            mono.drain(..skip);
            c.settle -= skip;
        }
        if mono.is_empty() {
            return;
        }
        self.feed(mono);
    }

    /// One stretch of microphone audio: the engine, the tap, and what comes of them.
    fn feed(&mut self, mut mono: Vec<i16>) {
        if let Some(c) = self.capture.as_mut() {
            let exact_silence = mono.iter().all(|&s| s == 0);
            if exact_silence {
                c.silent_since.get_or_insert_with(Instant::now);
            } else {
                c.silent_since = None;
            }
        }
        // An injected clip takes the place of the microphone's audio, sample for sample, so that it runs at the microphone's pace.
        if let Some(inj) = self.injection.as_mut() {
            for slot in mono.iter_mut() {
                *slot = if inj.at < inj.clip.len() {
                    inj.clip[inj.at]
                } else {
                    0
                };
                inj.at += 1;
            }
            if inj.at >= inj.clip.len() + inj.tail {
                self.injection = None;
            }
        }
        if let Some(tap) = self.tap.as_ref() {
            match tap.try_send(mono.clone()) {
                Err(TrySendError::Disconnected(_)) => self.tap = None,
                Ok(()) | Err(TrySendError::Full(_)) => {}
            }
        }
        let deaf = self.tap.is_some();
        let mut out = Vec::new();
        if let Some(c) = self.capture.as_mut() {
            if let Err(e) = c.engine.push(&mono, deaf, &mut out) {
                let text = format!("the wake word model failed: {e}");
                self.capture_lost(text);
                return;
            }
        }
        self.emit(out);
    }

    fn capture_lost(&mut self, text: String) {
        let was_in_turn = self.capture.as_ref().is_some_and(|c| c.engine.in_turn());
        self.capture = None;
        self.injection = None;
        self.tap = None;
        if was_in_turn {
            self.send(Event::TurnEnded {
                reason: EndReason::Lost,
            });
        }
        self.failed(text);
    }

    fn reference_ready(&mut self, scratch: &mut [u8]) {
        let mut lost = false;
        if let Some(r) = self.reference.as_mut() {
            for _ in 0..4 {
                match r.input.read(scratch) {
                    Ok(0) => break,
                    Ok(n) => {
                        r.meter.push(&scratch[..n]);
                        if n < scratch.len() {
                            break;
                        }
                    }
                    Err(_) => {
                        lost = true;
                        break;
                    }
                }
            }
        }
        if lost {
            // The level is a help, not a need: without the stream the cutoff is the plain one.
            self.reference = None;
            if let Some(c) = self.capture.as_mut() {
                c.engine.set_strictness(0.0);
            }
        } else if let (Some(r), Some(c)) = (self.reference.as_ref(), self.capture.as_mut()) {
            c.engine.set_strictness(strictness(r.meter.db()));
        }
    }

    fn speaker_tick(&mut self) {
        if let Poll::Ended(end) = self.speaker.poll() {
            self.speaking = false;
            self.send(Event::SpeechDone { end });
        }
    }

    /// Once a second: the status, the reference stream coming and going with the TV's sound, and the stream's watchdogs.
    fn tick(&mut self) {
        let playing = self.shared.tv_playing.load(Ordering::Relaxed);
        let want_reference = self.config.reference && self.capture.is_some() && playing;
        if want_reference && self.reference.is_none() {
            self.reference = self.card.open_reference().ok().map(|input| Reference {
                input,
                meter: Meter::new(self.card.reference_format()),
            });
        } else if !want_reference && self.reference.is_some() {
            self.reference = None;
            if let Some(c) = self.capture.as_mut() {
                c.engine.set_strictness(0.0);
            }
        }
        let stalled = self
            .capture
            .as_ref()
            .is_some_and(|c| c.fed.elapsed() > self.config.stall_after);
        if stalled {
            self.capture_lost("the microphone stopped giving sound".to_owned());
        }
        let silent_for = self.config.silent_after;
        let mut status = Status {
            client: self.client.is_some(),
            tapped: self.tap.is_some(),
            injecting: self.injection.is_some(),
            speaking: self.speaking,
            mic_db: -120.0,
            error: self.failure.clone(),
            wakes: self.wakes,
            ..Status::default()
        };
        if let Some(c) = self.capture.as_mut() {
            status.capturing = true;
            status.in_turn = c.engine.in_turn();
            status.mic_db = peak_dbfs(&[c.engine.take_peak()]);
            status.score = c.engine.take_score_peak();
            status.pick = Some(c.frames.pick());
            status.held_back = c.engine.refused();
            status.clipped = c.frames.clipped();
            status.silent = c.silent_since.is_some_and(|t| t.elapsed() >= silent_for);
        }
        status.tv_db = self.reference.as_ref().map(|r| r.meter.db());
        if let Ok(mut slot) = self.shared.status.lock() {
            *slot = status;
        }
    }
}
