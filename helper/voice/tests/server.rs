//! The server on its socket, with the real voice-wire client on the other end and a fake card: the microphone opens when it is asked
//! to listen and not before, a recorded wake word makes a turn that goes up only when the client takes it, the microphone is let go
//! at a mute or a lost client, and a card that fails is told once and tried again.

use lumavoice::input::{Card, Input};
use lumavoice::pcm::{Format, Pick, Sample};
use lumavoice::server::{self, Config, Handle};
use lumavoice::speech::{Poll, Speaker};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use voice_wire::{Client, Decoder, EndReason, Event, Request, SpeechEnd, VERSION, encode};
use wake_word::wav;

const OKAY_NABU: &[u8] = include_bytes!("data/okay_nabu_hazel.wav");
const OKAY_NABU_TIME: &[u8] = include_bytes!("data/okay_nabu_time.wav");

const MIC: Format = Format {
    rate: 16_000,
    channels: 2,
    sample: Sample::S24,
};
const REFERENCE: Format = Format {
    rate: 48_000,
    channels: 2,
    sample: Sample::S16,
};

/// One read of the microphone: 20 ms of the card's frames.
const PERIOD: usize = 320;

/// The card: each open hands the server one end of a pipe and keeps the other for the test to write the "room" into.
#[derive(Default)]
struct Rig {
    mics: Mutex<Vec<UnixStream>>,
    references: Mutex<Vec<UnixStream>>,
    mic_opens: AtomicUsize,
    mic_live: Arc<AtomicUsize>,
    reference_opens: AtomicUsize,
    reference_live: Arc<AtomicUsize>,
    /// Opens still to refuse.
    refuse: Mutex<usize>,
}

struct Pipe {
    stream: UnixStream,
    live: Arc<AtomicUsize>,
}

impl Input for Pipe {
    fn fd(&self) -> RawFd {
        self.stream.as_raw_fd()
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.stream.read(buf) {
            Ok(0) => Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => Ok(n),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(0),
            Err(e) => Err(e),
        }
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
    }
}

fn pipe(live: &Arc<AtomicUsize>) -> (Pipe, UnixStream) {
    let (server, test) = UnixStream::pair().unwrap();
    server.set_nonblocking(true).unwrap();
    live.fetch_add(1, Ordering::SeqCst);
    (
        Pipe {
            stream: server,
            live: live.clone(),
        },
        test,
    )
}

impl Card for Rig {
    fn open_mic(&self) -> io::Result<Box<dyn Input>> {
        self.mic_opens.fetch_add(1, Ordering::SeqCst);
        let mut refuse = self.refuse.lock().unwrap();
        if *refuse > 0 {
            *refuse -= 1;
            return Err(io::Error::other("no such device"));
        }
        drop(refuse);
        let (input, test) = pipe(&self.mic_live);
        self.mics.lock().unwrap().push(test);
        Ok(Box::new(input))
    }

    fn mic_format(&self) -> Format {
        MIC
    }

    fn open_reference(&self) -> io::Result<Box<dyn Input>> {
        self.reference_opens.fetch_add(1, Ordering::SeqCst);
        let (input, test) = pipe(&self.reference_live);
        self.references.lock().unwrap().push(test);
        Ok(Box::new(input))
    }

    fn reference_format(&self) -> Format {
        REFERENCE
    }
}

impl Rig {
    /// The test's end of the newest microphone, once the server has opened it.
    fn mic(&self) -> UnixStream {
        self.mics.lock().unwrap().last().expect("the microphone was opened").try_clone().unwrap()
    }
}

/// What the player was asked, and when it ends a reply.
#[derive(Default)]
struct Played {
    begun: Mutex<Vec<(u32, u8)>>,
    bytes: AtomicUsize,
    finished: AtomicBool,
    stopped: AtomicBool,
    refuse: AtomicBool,
}

struct FakeSpeaker(Arc<Played>);

impl Speaker for FakeSpeaker {
    fn begin(&mut self, rate: u32, percent: u8) -> Result<(), String> {
        if self.0.refuse.load(Ordering::SeqCst) {
            return Err("no player".to_owned());
        }
        self.0.begun.lock().unwrap().push((rate, percent));
        self.0.finished.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn push(&mut self, pcm: &[u8]) {
        self.0.bytes.fetch_add(pcm.len(), Ordering::SeqCst);
    }

    fn finish(&mut self) {
        self.0.finished.store(true, Ordering::SeqCst);
    }

    fn stop(&mut self) {
        self.0.stopped.store(true, Ordering::SeqCst);
    }

    fn poll(&mut self) -> Poll {
        if self.0.finished.swap(false, Ordering::SeqCst) {
            Poll::Ended(SpeechEnd::Played)
        } else {
            Poll::Idle
        }
    }
}

struct Fixture {
    rig: Arc<Rig>,
    played: Arc<Played>,
    handle: Option<Handle>,
    path: PathBuf,
    _dir: tempfile::TempDir,
}

fn config() -> Config {
    Config {
        gain_db: 0.0,
        pick: Pick::Channel(0),
        tick: Duration::from_millis(10),
        retry: Duration::from_millis(40),
        stall_after: Duration::from_secs(30),
        silent_after: Duration::from_secs(30),
        ..Config::default()
    }
}

fn fixture(config: Config) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("voice.sock");
    let rig = Arc::new(Rig::default());
    let played = Arc::new(Played::default());
    let handle = server::start(&path, rig.clone(), Box::new(FakeSpeaker(played.clone())), config).unwrap();
    Fixture {
        rig,
        played,
        handle: Some(handle),
        path,
        _dir: dir,
    }
}

impl Fixture {
    fn handle(&self) -> &Handle {
        self.handle.as_ref().unwrap()
    }
}

/// A client past its hello.
fn client(path: &Path) -> (Client, Vec<Event>) {
    let mut client = Client::connect(path).unwrap();
    let mut seen = Vec::new();
    until(&mut client, &mut seen, |s| s.contains(&Event::Ready { rate: 16_000 }));
    (client, seen)
}

fn listen(client: &mut Client, seen: &mut Vec<Event>, word: &str) {
    client.send(&Request::Listen { words: vec![word.to_owned()] }).unwrap();
    until(client, seen, |s| s.iter().any(|e| matches!(e, Event::Listening { .. })));
}

/// Waits, without sleeping on a timer, until what the client has been told satisfies `done`.
fn until(client: &mut Client, seen: &mut Vec<Event>, done: impl Fn(&[Event]) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        seen.extend(client.poll().unwrap());
        if done(seen) {
            return;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "timed out; the client saw {seen:?}");
        let mut fd = libc::pollfd {
            fd: client.raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd.
        unsafe { libc::poll(&mut fd, 1, left.as_millis().min(1_000) as i32) };
    }
}

/// Waits for a condition on the server's side, which nothing tells the test about.
fn eventually(what: &str, cond: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(1));
    }
}

fn clip(bytes: &[u8]) -> Vec<i16> {
    wav::read(bytes).unwrap().samples
}

fn silence(seconds: f32) -> Vec<i16> {
    vec![0; (seconds * 16_000.0) as usize]
}

/// 16-bit samples as the card gives them: 24 bits in the low three bytes of four, both microphones the same.
fn s24_stereo(samples: &[i16]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|&s| {
            let word = (i32::from(s) << 8).to_le_bytes();
            [word, word].concat()
        })
        .collect()
}

/// The room, into the microphone, a period at a time and faster than it would happen.
fn speak(mic: UnixStream, samples: Vec<i16>) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut mic = mic;
        for period in samples.chunks(PERIOD) {
            if mic.write_all(&s24_stereo(period)).is_err() {
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
    })
}

fn audio_samples(seen: &[Event]) -> usize {
    seen.iter()
        .map(|e| match e {
            Event::Audio { pcm } => pcm.len() / 2,
            _ => 0,
        })
        .sum()
}

fn failures(seen: &[Event]) -> Vec<&str> {
    seen.iter()
        .filter_map(|e| match e {
            Event::Failed { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn nothing_is_opened_until_the_client_asks_to_listen() {
    let fx = fixture(config());
    let (mut c, mut seen) = client(&fx.path);
    assert_eq!(fx.rig.mic_opens.load(Ordering::SeqCst), 0, "a hello opens nothing");
    listen(&mut c, &mut seen, "okay_nabu");
    assert_eq!(fx.rig.mic_opens.load(Ordering::SeqCst), 1);
    assert!(seen.contains(&Event::Listening { words: vec!["okay_nabu".to_owned()] }));
    eventually("the status", || fx.handle().status().capturing);
    assert!(fx.handle().status().client);
}

#[test]
fn a_mute_and_a_lost_client_both_let_the_microphone_go() {
    let fx = fixture(config());
    let (mut c, mut seen) = client(&fx.path);
    listen(&mut c, &mut seen, "okay_nabu");
    assert_eq!(fx.rig.mic_live.load(Ordering::SeqCst), 1);
    c.send(&Request::Mute).unwrap();
    until(&mut c, &mut seen, |s| s.contains(&Event::Muted));
    assert_eq!(fx.rig.mic_live.load(Ordering::SeqCst), 0, "released before the mute is answered");

    seen.clear();
    listen(&mut c, &mut seen, "hey_jarvis");
    assert_eq!(fx.rig.mic_live.load(Ordering::SeqCst), 1);
    drop(c);
    eventually("the microphone to be released", || fx.rig.mic_live.load(Ordering::SeqCst) == 0);
    assert_eq!(fx.rig.mic_opens.load(Ordering::SeqCst), 2);
}

#[test]
fn a_change_of_word_keeps_the_stream_open() {
    let fx = fixture(config());
    let (mut c, mut seen) = client(&fx.path);
    listen(&mut c, &mut seen, "okay_nabu");
    seen.clear();
    listen(&mut c, &mut seen, "hey_jarvis");
    assert_eq!(seen, [Event::Listening { words: vec!["hey_jarvis".to_owned()] }]);
    assert_eq!(fx.rig.mic_opens.load(Ordering::SeqCst), 1, "the device is not closed and reopened for a word");
}

#[test]
fn a_word_the_engine_does_not_carry_is_refused_in_words() {
    let fx = fixture(config());
    let (mut c, mut seen) = client(&fx.path);
    c.send(&Request::Listen { words: vec!["alexa".to_owned()] }).unwrap();
    until(&mut c, &mut seen, |s| !failures(s).is_empty());
    assert_eq!(failures(&seen), ["there is no wake word alexa"]);
    c.send(&Request::Listen { words: Vec::new() }).unwrap();
    until(&mut c, &mut seen, |s| failures(s).len() == 2);
    assert_eq!(fx.rig.mic_opens.load(Ordering::SeqCst), 0);
}

#[test]
fn a_wake_word_makes_a_turn_that_goes_up_when_the_client_takes_it() {
    let fx = fixture(config());
    let (mut c, mut seen) = client(&fx.path);
    listen(&mut c, &mut seen, "okay_nabu");
    let mut room = silence(0.3);
    room.extend(clip(OKAY_NABU));
    room.extend(silence(2.0));
    let feeder = speak(fx.rig.mic(), room);
    until(&mut c, &mut seen, |s| s.iter().any(|e| matches!(e, Event::Wake { .. })));
    let Some(Event::Wake { word, score, .. }) = seen.iter().find(|e| matches!(e, Event::Wake { .. })) else {
        unreachable!()
    };
    assert_eq!(word, "okay_nabu");
    assert!(*score >= 0.97);
    assert_eq!(audio_samples(&seen), 0, "no sample left before the client took the turn");

    c.send(&Request::Accept).unwrap();
    until(&mut c, &mut seen, |s| audio_samples(s) >= 16_000);
    feeder.join().unwrap();
    c.send(&Request::End).unwrap();
    until(&mut c, &mut seen, |s| s.contains(&Event::TurnEnded { reason: EndReason::Client }));
    eventually("the turn to be over", || !fx.handle().status().in_turn);
    assert_eq!(fx.handle().status().wakes, 1);
}

#[test]
fn a_wake_nobody_takes_ends_and_no_audio_left() {
    let fx = fixture(config());
    let (mut c, mut seen) = client(&fx.path);
    listen(&mut c, &mut seen, "okay_nabu");
    let mut room = silence(0.3);
    room.extend(clip(OKAY_NABU));
    room.extend(silence(4.0));
    let feeder = speak(fx.rig.mic(), room);
    until(&mut c, &mut seen, |s| s.contains(&Event::TurnEnded { reason: EndReason::NoAck }));
    feeder.join().unwrap();
    assert_eq!(audio_samples(&seen), 0);
}

#[test]
fn a_mute_in_a_turn_ends_it_first() {
    let fx = fixture(config());
    let (mut c, mut seen) = client(&fx.path);
    listen(&mut c, &mut seen, "okay_nabu");
    let mut room = silence(0.3);
    room.extend(clip(OKAY_NABU));
    let feeder = speak(fx.rig.mic(), room);
    until(&mut c, &mut seen, |s| s.iter().any(|e| matches!(e, Event::Wake { .. })));
    seen.clear();
    c.send(&Request::Mute).unwrap();
    until(&mut c, &mut seen, |s| s.contains(&Event::Muted));
    assert_eq!(seen, [Event::TurnEnded { reason: EndReason::Muted }, Event::Muted]);
    let _ = feeder.join();
}

#[test]
fn an_injected_clip_wakes_the_engine_in_place_of_the_microphone() {
    let fx = fixture(config());
    let (mut c, mut seen) = client(&fx.path);
    assert!(fx.handle().inject(silence(0.1)).is_err(), "no microphone is open to take the place of");
    listen(&mut c, &mut seen, "okay_nabu");
    // The room is silent: the card gives zeros for as long as the test lasts.
    let feeder = speak(fx.rig.mic(), silence(8.0));
    fx.handle().inject(clip(OKAY_NABU_TIME)).unwrap();
    assert!(fx.handle().inject(silence(0.1)).is_err(), "one at a time");
    until(&mut c, &mut seen, |s| s.iter().any(|e| matches!(e, Event::Wake { .. })));
    c.send(&Request::Accept).unwrap();
    until(&mut c, &mut seen, |s| audio_samples(s) >= 16_000);
    c.send(&Request::End).unwrap();
    until(&mut c, &mut seen, |s| s.contains(&Event::TurnEnded { reason: EndReason::Client }));
    drop(c);
    drop(fx);
    let _ = feeder.join();
}

#[test]
fn a_tap_takes_the_stream_and_the_wake_word_is_not_listened_for_while_it_is_held() {
    let fx = fixture(config());
    let (mut c, mut seen) = client(&fx.path);
    assert!(fx.handle().tap().is_err(), "no microphone is open");
    listen(&mut c, &mut seen, "okay_nabu");
    let tap = fx.handle().tap().unwrap();
    assert!(fx.handle().tap().is_err(), "one reader besides the engine");
    let mut room = silence(0.3);
    room.extend(clip(OKAY_NABU));
    room.extend(silence(0.5));
    let total = room.len();
    let feeder = speak(fx.rig.mic(), room);
    let mut heard = 0;
    while heard < total - 6_000 {
        heard += tap.recv_timeout(Duration::from_secs(5)).expect("the tap gets the stream").len();
    }
    feeder.join().unwrap();
    seen.extend(c.poll().unwrap());
    assert!(
        !seen.iter().any(|e| matches!(e, Event::Wake { .. })),
        "the word said while another reader has the microphone wakes nothing"
    );
    drop(tap);
    let mut again = clip(OKAY_NABU);
    again.extend(silence(0.5));
    let feeder = speak(fx.rig.mic(), again);
    until(&mut c, &mut seen, |s| s.iter().any(|e| matches!(e, Event::Wake { .. })));
    feeder.join().unwrap();
}

#[test]
fn a_failed_open_is_told_once_and_tried_again() {
    let fx = fixture(config());
    *fx.rig.refuse.lock().unwrap() = 2;
    let (mut c, mut seen) = client(&fx.path);
    c.send(&Request::Listen { words: vec!["okay_nabu".to_owned()] }).unwrap();
    until(&mut c, &mut seen, |s| s.iter().any(|e| matches!(e, Event::Listening { .. })));
    assert_eq!(fx.rig.mic_opens.load(Ordering::SeqCst), 3, "refused twice, then opened");
    let told = failures(&seen);
    assert_eq!(told.len(), 1, "the same words are not told again: {told:?}");
    assert!(told[0].contains("did not open"), "{}", told[0]);
}

#[test]
fn a_capture_that_gives_no_sound_is_called_stalled_and_opened_again() {
    let fx = fixture(Config {
        stall_after: Duration::from_millis(80),
        ..config()
    });
    let (mut c, mut seen) = client(&fx.path);
    listen(&mut c, &mut seen, "okay_nabu");
    seen.clear();
    until(&mut c, &mut seen, |s| s.iter().any(|e| matches!(e, Event::Listening { .. })));
    assert_eq!(failures(&seen), ["the microphone stopped giving sound"]);
    assert_eq!(fx.rig.mic_opens.load(Ordering::SeqCst), 2);
}

#[test]
fn a_microphone_that_ends_is_a_lost_turn_and_a_failure() {
    let fx = fixture(config());
    let (mut c, mut seen) = client(&fx.path);
    listen(&mut c, &mut seen, "okay_nabu");
    let mut room = silence(0.3);
    room.extend(clip(OKAY_NABU));
    let feeder = speak(fx.rig.mic(), room);
    until(&mut c, &mut seen, |s| s.iter().any(|e| matches!(e, Event::Wake { .. })));
    feeder.join().unwrap();
    fx.rig.mics.lock().unwrap().clear();
    until(&mut c, &mut seen, |s| {
        s.contains(&Event::TurnEnded { reason: EndReason::Lost }) && !failures(s).is_empty()
    });
    assert!(failures(&seen)[0].contains("the microphone stopped"), "{seen:?}");
}

#[test]
fn exact_silence_for_long_enough_is_reported() {
    let fx = fixture(Config {
        silent_after: Duration::from_millis(30),
        ..config()
    });
    let (mut c, mut seen) = client(&fx.path);
    listen(&mut c, &mut seen, "okay_nabu");
    let feeder = speak(fx.rig.mic(), silence(1.0));
    eventually("the silence to be noticed", || fx.handle().status().silent);
    feeder.join().unwrap();
    let mut loud = vec![1_000i16; 16_000];
    loud[0] = 3_000;
    speak(fx.rig.mic(), loud).join().unwrap();
    eventually("the sound to be noticed", || {
        let s = fx.handle().status();
        !s.silent && s.mic_db > -40.0
    });
}

#[test]
fn the_tvs_level_is_read_only_while_it_plays_and_the_microphone_is_open() {
    let fx = fixture(config());
    let (mut c, mut seen) = client(&fx.path);
    fx.handle().tv_playing(true);
    assert_eq!(fx.rig.reference_live.load(Ordering::SeqCst), 0, "no microphone, no reference");
    listen(&mut c, &mut seen, "okay_nabu");
    eventually("the reference to open", || fx.rig.reference_live.load(Ordering::SeqCst) == 1);
    let mut mix = fx.rig.references.lock().unwrap()[0].try_clone().unwrap();
    // Half-scale 16-bit stereo: -6 dBFS.
    let loud: Vec<u8> = (0..4_800).flat_map(|_| [0x00u8, 0x40, 0x00, 0x40]).collect();
    mix.write_all(&loud).unwrap();
    eventually("a level", || fx.handle().status().tv_db.is_some_and(|db| db > -12.0));
    fx.handle().tv_playing(false);
    eventually("the reference to close", || fx.rig.reference_live.load(Ordering::SeqCst) == 0);
    eventually("the level to go", || fx.handle().status().tv_db.is_none());
}

#[test]
fn a_reply_is_played_at_the_asked_rate_and_volume_and_its_end_is_told() {
    let fx = fixture(config());
    let (mut c, mut seen) = client(&fx.path);
    c.send(&Request::Speak { rate: 24_000, percent: 40 }).unwrap();
    c.send(&Request::Pcm { pcm: vec![0; 480] }).unwrap();
    c.send(&Request::Pcm { pcm: vec![0; 480] }).unwrap();
    c.send(&Request::SpeakEnd).unwrap();
    until(&mut c, &mut seen, |s| s.contains(&Event::SpeechDone { end: SpeechEnd::Played }));
    assert_eq!(*fx.played.begun.lock().unwrap(), [(24_000, 40)]);
    assert_eq!(fx.played.bytes.load(Ordering::SeqCst), 960);
}

#[test]
fn a_reply_that_is_stopped_is_cut_and_one_that_cannot_start_is_refused() {
    let fx = fixture(config());
    let (mut c, mut seen) = client(&fx.path);
    c.send(&Request::Speak { rate: 16_000, percent: 60 }).unwrap();
    c.send(&Request::StopSpeech).unwrap();
    until(&mut c, &mut seen, |s| s.contains(&Event::SpeechDone { end: SpeechEnd::Cut }));
    assert!(fx.played.stopped.load(Ordering::SeqCst));
    fx.played.refuse.store(true, Ordering::SeqCst);
    c.send(&Request::Speak { rate: 16_000, percent: 60 }).unwrap();
    until(&mut c, &mut seen, |s| {
        s.contains(&Event::SpeechDone {
            end: SpeechEnd::Refused {
                why: "no player".to_owned(),
            },
        })
    });
}

/// A connection with no hello of its own: what the server says to whoever comes.
fn raw(path: &Path) -> UnixStream {
    let stream = UnixStream::connect(path).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    stream
}

fn first_event(stream: &mut UnixStream) -> Event {
    let mut decoder = Decoder::default();
    let mut buf = [0u8; 4096];
    loop {
        if let Some(event) = decoder.take::<Event>().unwrap() {
            return event;
        }
        let n = stream.read(&mut buf).unwrap();
        assert!(n > 0, "the server closed without a word");
        decoder.push(&buf[..n]);
    }
}

#[test]
fn one_client_at_a_time_and_only_the_wire_version_it_speaks() {
    let fx = fixture(config());
    let (_first, _) = client(&fx.path);
    let Event::Failed { text } = first_event(&mut raw(&fx.path)) else {
        panic!("a second client is refused in words")
    };
    assert_eq!(text, "another client has the microphone");

    let fx = fixture(config());
    let mut old = raw(&fx.path);
    old.write_all(&encode(&Request::Hello { version: VERSION + 1 }).unwrap()).unwrap();
    let Event::Failed { text } = first_event(&mut old) else {
        panic!("a hello of another version is refused in words")
    };
    assert!(text.contains("version"), "{text}");
}

#[test]
fn the_socket_is_the_owners_alone_and_goes_with_the_server() {
    use std::os::unix::fs::PermissionsExt;
    let mut fx = fixture(config());
    let mode = std::fs::metadata(&fx.path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    fx.handle.take().unwrap().stop();
    assert!(!fx.path.exists());
}
