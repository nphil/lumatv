//! Voice search end to end: the real service, a fake microphone (a shell script) and a fake Home Assistant that speaks the
//! WebSocket API's `assist_pipeline/run` over real TCP.
//!
//! What these hold voice search to: off until the owner's setting, a saved link and a chosen microphone are all there; the
//! microphone is open only while a request runs and is gone (process and children) when it ends, whichever way; the audio reaches
//! Home Assistant as 16 kHz PCM through the handler it named, with nothing but the handler's id to end it; the token is sent to
//! the saved address in the auth message and appears nowhere else; the words come back as text and nothing acts on them.

mod support;

use ha_link::ws;
use lumatv_helper::config::Config;
use lumatv_helper::voice_store;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use support::*;

const WAIT: Duration = Duration::from_secs(10);
const TOKEN: &str = "eyJhbGciOiJIUzI1NiJ9.test-payload.test-signature";
/// Half a second of a 16 kHz 16-bit mono ramp, so a byte that moved or went missing shows.
const AUDIO_BYTES: usize = 16_000;

// ------------------------------------------------------------------------------------------------------------- fake Home Assistant

#[derive(Clone)]
enum Answer {
    /// Speech ends with the audio: `stt-end` with these words.
    Words(&'static str),
    /// Speech ends with the audio: nothing was recognised.
    NothingHeard,
    /// Speech is over after this many bytes (the voice-activity detector), before the audio ends.
    Early(usize, &'static str),
    /// Never answers.
    Silent,
    /// The pipeline asked for does not exist.
    UnknownPipeline,
}

#[derive(Default)]
struct Seen {
    connections: usize,
    auths: Vec<String>,
    runs: Vec<Value>,
    audio: Vec<u8>,
    /// The first byte of every binary message, which must be the handler's id.
    handler_bytes: Vec<u8>,
    /// Binary messages that were just the id: the end of the audio.
    ends: usize,
    /// Connections that were closed by the client.
    closed: usize,
}

struct Shared {
    token: String,
    answer: Mutex<Answer>,
    seen: Mutex<Seen>,
    stop: AtomicBool,
}

struct FakeHa {
    addr: SocketAddr,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl FakeHa {
    fn start(token: &str, answer: Answer) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let shared = Arc::new(Shared {
            token: token.to_owned(),
            answer: Mutex::new(answer),
            seen: Mutex::new(Seen::default()),
            stop: AtomicBool::new(false),
        });
        let accept = {
            let shared = shared.clone();
            thread::spawn(move || {
                let mut children = Vec::new();
                while !shared.stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let shared = shared.clone();
                            children.push(thread::spawn(move || serve(stream, &shared)));
                        }
                        Err(_) => thread::sleep(Duration::from_millis(5)),
                    }
                }
                for child in children {
                    let _ = child.join();
                }
            })
        };
        Self {
            addr,
            shared,
            thread: Some(accept),
        }
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn seen<T>(&self, read: impl FnOnce(&Seen) -> T) -> T {
        read(&self.shared.seen.lock())
    }
}

impl Drop for FakeHa {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn text_frame(value: &Value) -> Vec<u8> {
    let payload = value.to_string().into_bytes();
    let mut out = vec![0x81];
    match payload.len() {
        n if n < 126 => out.push(n as u8),
        n => {
            out.push(126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
    }
    out.extend_from_slice(&payload);
    out
}

/// One masked client frame from `buf`, removed from it.
fn take_frame(buf: &mut Vec<u8>) -> Option<(u8, Vec<u8>)> {
    if buf.len() < 2 {
        return None;
    }
    let opcode = buf[0] & 0x0f;
    assert!(buf[1] & 0x80 != 0, "a client frame must be masked");
    let (len, at) = match buf[1] & 0x7f {
        126 if buf.len() >= 4 => (usize::from(u16::from_be_bytes([buf[2], buf[3]])), 4),
        126 => return None,
        127 if buf.len() >= 10 => (
            u64::from_be_bytes(buf[2..10].try_into().unwrap()) as usize,
            10,
        ),
        127 => return None,
        n => (usize::from(n), 2),
    };
    if buf.len() < at + 4 + len {
        return None;
    }
    let mask: [u8; 4] = buf[at..at + 4].try_into().unwrap();
    let payload: Vec<u8> = buf[at + 4..at + 4 + len]
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ mask[i % 4])
        .collect();
    buf.drain(..at + 4 + len);
    Some((opcode, payload))
}

fn serve(mut stream: TcpStream, shared: &Shared) {
    stream
        .set_read_timeout(Some(Duration::from_millis(20)))
        .unwrap();
    shared.seen.lock().connections += 1;
    let mut buf = Vec::new();
    // The upgrade.
    let key = loop {
        if shared.stop.load(Ordering::SeqCst) {
            return;
        }
        let mut chunk = [0u8; 2048];
        match stream.read(&mut chunk) {
            Ok(0) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => {}
        }
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).into_owned();
            buf.drain(..end + 4);
            break head
                .lines()
                .find_map(|l| l.strip_prefix("Sec-WebSocket-Key: "))
                .expect("a key")
                .to_owned();
        }
    };
    let accept = ws::accept_key(&key);
    stream
        .write_all(
            format!(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
            )
            .as_bytes(),
        )
        .unwrap();
    let send = |stream: &mut TcpStream, value: Value| {
        let _ = stream.write_all(&text_frame(&value));
    };
    send(
        &mut stream,
        json!({"type": "auth_required", "ha_version": "2026.10.0"}),
    );
    let mut authed = false;
    let mut run_id = 0;
    let mut answered = false;
    loop {
        if shared.stop.load(Ordering::SeqCst) {
            return;
        }
        while let Some((opcode, payload)) = take_frame(&mut buf) {
            match opcode {
                0x8 => {
                    shared.seen.lock().closed += 1;
                    return;
                }
                0x1 => {
                    let message: Value = serde_json::from_slice(&payload).unwrap();
                    if !authed {
                        let token = message["access_token"].as_str().unwrap_or("").to_owned();
                        shared.seen.lock().auths.push(token.clone());
                        if message["type"] == "auth" && token == shared.token {
                            authed = true;
                            send(
                                &mut stream,
                                json!({"type": "auth_ok", "ha_version": "2026.10.0"}),
                            );
                        } else {
                            send(
                                &mut stream,
                                json!({"type": "auth_invalid", "message": "Invalid access token or password"}),
                            );
                            return;
                        }
                    } else if message["type"] == "assist_pipeline/run" {
                        run_id = message["id"].as_u64().unwrap();
                        shared.seen.lock().runs.push(message.clone());
                        if matches!(*shared.answer.lock(), Answer::UnknownPipeline) {
                            send(
                                &mut stream,
                                json!({"id": run_id, "type": "result", "success": false,
                                "error": {"code": "not_found", "message": "Pipeline not found"}}),
                            );
                            continue;
                        }
                        send(
                            &mut stream,
                            json!({"id": run_id, "type": "result", "success": true, "result": null}),
                        );
                        send(
                            &mut stream,
                            json!({"id": run_id, "type": "event", "event": {"type": "run-start",
                            "data": {"pipeline": "p", "language": "en", "runner_data": {"stt_binary_handler_id": 1, "timeout": 300}}}}),
                        );
                        send(
                            &mut stream,
                            json!({"id": run_id, "type": "event", "event": {"type": "stt-start", "data": {"engine": "stt.test"}}}),
                        );
                    }
                }
                0x2 => {
                    let mut seen = shared.seen.lock();
                    seen.handler_bytes.push(payload[0]);
                    seen.audio.extend_from_slice(&payload[1..]);
                    let total = seen.audio.len();
                    let ended = payload.len() == 1;
                    if ended {
                        seen.ends += 1;
                    }
                    drop(seen);
                    let answer = shared.answer.lock().clone();
                    let words = match (&answer, ended, total) {
                        (Answer::Words(w), true, _) => Some(Ok(*w)),
                        (Answer::NothingHeard, true, _) => Some(Err(())),
                        (Answer::Early(after, w), _, total) if total >= *after => Some(Ok(*w)),
                        _ => None,
                    };
                    if let (Some(result), false) = (words, answered) {
                        answered = true;
                        match result {
                            Ok(words) => {
                                send(
                                    &mut stream,
                                    json!({"id": run_id, "type": "event", "event": {"type": "stt-end",
                                    "data": {"stt_output": {"text": words}}}}),
                                );
                                send(
                                    &mut stream,
                                    json!({"id": run_id, "type": "event", "event": {"type": "run-end", "data": null}}),
                                );
                            }
                            Err(()) => send(
                                &mut stream,
                                json!({"id": run_id, "type": "event", "event": {"type": "error",
                                "data": {"code": "stt-no-text-recognized", "message": "No text recognized"}}}),
                            ),
                        }
                    }
                }
                0x9 => {}
                _ => {}
            }
        }
        let mut chunk = [0u8; 8192];
        match stream.read(&mut chunk) {
            Ok(0) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => {}
        }
    }
}

// ----------------------------------------------------------------------------------------------------------------- fake microphone

/// A microphone program: a shell script that records its pid, writes `AUDIO_BYTES` of a ramp and then stays open (silent) like a
/// microphone does, until it is killed.
struct FakeMic {
    argv: Vec<String>,
    pid_file: PathBuf,
    ran_file: PathBuf,
}

impl FakeMic {
    fn new(dir: &Path) -> Self {
        let pcm = dir.join("ramp.pcm");
        let bytes: Vec<u8> = (0..AUDIO_BYTES).map(|i| (i % 251) as u8).collect();
        std::fs::write(&pcm, bytes).unwrap();
        let pid_file = dir.join("mic.pid");
        let ran_file = dir.join("mic.ran");
        let script = format!(
            "echo $$ > {pid}; echo ran >> {ran}; cat {pcm}; sleep 600 & wait",
            pid = pid_file.display(),
            ran = ran_file.display(),
            pcm = pcm.display()
        );
        Self {
            argv: vec!["sh".into(), "-c".into(), script],
            pid_file,
            ran_file,
        }
    }

    fn ramp() -> Vec<u8> {
        (0..AUDIO_BYTES).map(|i| (i % 251) as u8).collect()
    }

    fn started(&self) -> usize {
        std::fs::read_to_string(&self.ran_file)
            .map(|t| t.lines().count())
            .unwrap_or(0)
    }

    fn pid(&self) -> Option<i32> {
        std::fs::read_to_string(&self.pid_file)
            .ok()
            .and_then(|t| t.trim().parse().ok())
    }

    /// Whether the microphone program (the shell) and its `sleep` are all gone.
    fn gone(&self) -> bool {
        let Some(pid) = self.pid() else { return true };
        // A process that is gone, or one that is a zombie nobody collected yet, is not open.
        let alive = |pid: i32| {
            std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| !s.contains(") Z "))
        };
        if alive(pid) {
            return false;
        }
        // The group the shell led: nothing of it is left.
        !std::fs::read_dir("/proc")
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().to_str().and_then(|n| n.parse::<i32>().ok()))
            .any(|other| {
                std::fs::read_to_string(format!("/proc/{other}/stat")).is_ok_and(|s| {
                    let after = s.rsplit(") ").next().unwrap_or("");
                    let mut f = after.split_whitespace();
                    let (state, _ppid, pgrp) = (f.next(), f.next(), f.next());
                    pgrp == Some(&pid.to_string()) && state != Some("Z")
                })
            })
    }
}

// --------------------------------------------------------------------------------------------------------------------------- rig

struct VoiceRig {
    rig: Rig,
    mic: FakeMic,
}

fn config(mic: &FakeMic, extra: Value) -> Config {
    let mut voice = json!({"capture": mic.argv, "maxSeconds": 20, "tailMs": 100});
    if let Value::Object(more) = extra {
        for (k, v) in more {
            voice[k] = v;
        }
    }
    test_config(json!({"voice": voice}))
}

/// Starts the service with voice search ready: the setting on, the link saved to `ha`, a microphone chosen.
fn ready(ha: &FakeHa, extra: Value) -> VoiceRig {
    let mut rig = Rig::new();
    receiver_off(&rig.paths);
    let mic = FakeMic::new(rig.dir.path());
    seed_settings(
        &rig.paths,
        "base.voice",
        json!({"enabled": {"type": "bool", "value": true}}),
    );
    voice_store::save(&rig.paths.data_dir, &ha.url(), TOKEN).unwrap();
    let config = config(&mic, extra);
    rig.start_voice(config, Script::new());
    VoiceRig { rig, mic }
}

impl VoiceRig {
    fn listen(&self) -> Reply {
        post(self.rig.addr, "/v1/voice/listen", "{}")
    }

    fn stop(&self) {
        assert_eq!(post(self.rig.addr, "/v1/voice/stop", "{}").status, 200);
    }

    fn state(&self, want: &str) -> Option<Value> {
        self.rig.wait_topic("voice", WAIT, |v| v["state"] == want)
    }
}

// ---------------------------------------------------------------------------------------------------------------------------- tests

#[test]
fn the_words_come_back_as_text_and_the_audio_went_through_the_handler() {
    let ha = FakeHa::start(TOKEN, Answer::Words("play the news"));
    let voice = ready(&ha, json!({}));
    let reply = voice.listen();
    assert_eq!(
        reply.status,
        200,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    let session = reply.json()["session"].as_u64().unwrap();
    let listening = voice.state("listening").expect("listening");
    assert_eq!(listening["session"], session);
    assert_eq!(listening["ready"], true);

    // The microphone's audio reaches Home Assistant while the key is still down.
    assert!(wait_until(WAIT, || ha.seen(|s| s.audio.len() >= AUDIO_BYTES)));
    voice.stop();
    let heard = voice.state("heard").expect("heard");
    assert_eq!(heard["text"], "play the news");
    assert_eq!(heard["session"], session);

    ha.seen(|s| {
        assert_eq!(s.auths, [TOKEN]);
        assert_eq!(s.runs.len(), 1);
        let run = &s.runs[0];
        assert_eq!(run["type"], "assist_pipeline/run");
        assert_eq!(run["start_stage"], "stt");
        assert_eq!(run["end_stage"], "stt");
        assert_eq!(run["input"]["sample_rate"], 16_000);
        assert_eq!(
            run["pipeline"], "01m3xamd8xzkdv52syapw0cmf2",
            "the Local (Kokoro + Whisper) pipeline until the owner picks another"
        );
        assert_eq!(s.audio, FakeMic::ramp(), "every byte, in order, once");
        assert!(
            s.handler_bytes.iter().all(|b| *b == 1),
            "the handler Home Assistant named"
        );
        assert_eq!(s.ends, 1, "one empty message ends the audio");
    });
    // The microphone is closed with the request.
    assert!(
        wait_until(WAIT, || voice.mic.gone()),
        "the microphone program is still running"
    );
    assert_eq!(voice.mic.started(), 1);
}

#[test]
fn the_token_goes_to_the_saved_address_and_to_nothing_else() {
    let ha = FakeHa::start(TOKEN, Answer::Words("hello"));
    let voice = ready(&ha, json!({}));
    voice.listen();
    voice.state("listening").unwrap();
    voice.stop();
    voice.state("heard").unwrap();

    // Not in a topic, an event, a settings document, or an answer of the API.
    let mut everything = String::new();
    for path in [
        "/v1/state",
        "/v1/voice",
        "/v1/settings/base.voice",
        "/v1/settings",
        "/v1",
    ] {
        everything.push_str(&String::from_utf8_lossy(&get(voice.rig.addr, path).body));
    }
    let events = get(voice.rig.addr, "/v1/events?after=0&timeout=0");
    everything.push_str(&String::from_utf8_lossy(&events.body));
    assert!(!everything.contains(TOKEN) && !everything.contains("test-signature"));
    assert!(
        !everything.contains(&ha.url()),
        "the address of Home Assistant is not published either"
    );

    // The file is the only place, private.
    let file = voice_store::file(&voice.rig.paths.data_dir);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    // No settings route can change where the audio goes: the owner has no such field, and a try is refused.
    let tried = post(
        voice.rig.addr,
        "/v1/settings/base.voice",
        r#"{"changes":[{"key":"url","value":{"type":"text","value":"http://192.168.1.99"}}]}"#,
    );
    assert_eq!(tried.status, 422);
    let log = std::fs::read_to_string(voice.rig.dir.path().join("log-helper")).unwrap_or_default();
    assert!(
        !log.contains(TOKEN) && !log.contains("play the news") && !log.contains("hello"),
        "the log holds a secret or the words"
    );
}

#[test]
fn nothing_is_open_until_the_owner_has_switched_it_on_and_linked_and_a_microphone_is_named() {
    let ha = FakeHa::start(TOKEN, Answer::Words("x"));

    // Off (the default): no microphone program, no connection.
    let mut rig = Rig::new();
    receiver_off(&rig.paths);
    let mic = FakeMic::new(rig.dir.path());
    voice_store::save(&rig.paths.data_dir, &ha.url(), TOKEN).unwrap();
    rig.start_voice(config(&mic, json!({})), Script::new());
    let state = rig.topic("voice").unwrap();
    assert_eq!(
        (state["ready"].clone(), state["reason"].clone()),
        (json!(false), json!("voice_off"))
    );
    let refused = post(rig.addr, "/v1/voice/listen", "{}");
    assert_eq!(refused.status, 409);
    assert_eq!(refused.json()["error"]["code"], "voice_off");
    thread::sleep(Duration::from_millis(100));
    assert_eq!(mic.started(), 0);
    assert_eq!(ha.seen(|s| s.connections), 0);

    // Switched on through the settings: ready.
    let on = post(
        rig.addr,
        "/v1/settings/base.voice",
        r#"{"changes":[{"key":"enabled","value":{"type":"bool","value":true}}]}"#,
    );
    assert_eq!(on.status, 200);
    assert!(
        rig.wait_topic("voice", WAIT, |v| v["ready"] == true)
            .is_some()
    );
    rig.stop();

    // On but not linked.
    let mut rig = Rig::new();
    receiver_off(&rig.paths);
    let mic = FakeMic::new(rig.dir.path());
    seed_settings(
        &rig.paths,
        "base.voice",
        json!({"enabled": {"type": "bool", "value": true}}),
    );
    rig.start_voice(config(&mic, json!({})), Script::new());
    let refused = post(rig.addr, "/v1/voice/listen", "{}");
    assert_eq!(
        (refused.status, refused.json()["error"]["code"].clone()),
        (409, json!("voice_unlinked"))
    );
    // Linking while the helper runs (what `voice-link` does) and telling it makes it ready.
    voice_store::save(&rig.paths.data_dir, &ha.url(), TOKEN).unwrap();
    assert_eq!(post(rig.addr, "/v1/voice/refresh", "{}").status, 200);
    assert!(
        rig.wait_topic("voice", WAIT, |v| v["ready"] == true)
            .is_some()
    );
    rig.stop();

    // On and linked, but the owner emptied voice.capture: no microphone.
    let mut rig = Rig::new();
    receiver_off(&rig.paths);
    seed_settings(
        &rig.paths,
        "base.voice",
        json!({"enabled": {"type": "bool", "value": true}}),
    );
    voice_store::save(&rig.paths.data_dir, &ha.url(), TOKEN).unwrap();
    rig.start_voice(
        test_config(json!({"voice": {"capture": []}})),
        Script::new(),
    );
    let refused = post(rig.addr, "/v1/voice/listen", "{}");
    assert_eq!(
        (refused.status, refused.json()["error"]["code"].clone()),
        (409, json!("no_microphone"))
    );
    assert_eq!(ha.seen(|s| s.connections), 0);
}

#[test]
fn a_request_is_one_at_a_time_and_cancel_closes_the_microphone_and_the_connection() {
    let ha = FakeHa::start(TOKEN, Answer::Silent);
    let voice = ready(&ha, json!({}));
    assert_eq!(voice.listen().status, 200);
    voice.state("listening").unwrap();
    let again = voice.listen();
    assert_eq!(
        (again.status, again.json()["error"]["code"].clone()),
        (409, json!("busy"))
    );
    assert!(wait_until(WAIT, || ha.seen(|s| s.audio.len() >= AUDIO_BYTES)));

    let cancelled = post(voice.rig.addr, "/v1/voice/cancel", "{}");
    assert_eq!(cancelled.json()["cancelled"], true);
    let idle = voice.state("idle").expect("idle");
    assert!(idle.get("text").is_none());
    assert!(
        wait_until(WAIT, || voice.mic.gone()),
        "the microphone is still open after cancel"
    );
    // A new request can start, and nothing was reported for the cancelled one.
    assert_eq!(voice.listen().status, 200);
    voice.state("listening").unwrap();
    post(voice.rig.addr, "/v1/voice/cancel", "{}");
    assert!(wait_until(WAIT, || voice.mic.gone()));
}

#[test]
fn a_silent_home_assistant_ends_in_an_error_and_a_closed_microphone() {
    let ha = FakeHa::start(TOKEN, Answer::Silent);
    let voice = ready(&ha, json!({}));
    voice.listen();
    assert!(wait_until(WAIT, || ha.seen(|s| s.audio.len() >= AUDIO_BYTES)));
    voice.stop();
    // The microphone closes when the key is up, long before Home Assistant's answer is given up on.
    assert!(wait_until(WAIT, || voice.mic.gone()));
    let thinking = voice.state("thinking").expect("thinking");
    assert!(thinking.get("text").is_none());
    assert!(wait_until(Duration::from_secs(1), || ha.seen(|s| s.ends == 1)));
}

#[test]
fn nothing_heard_is_said_in_words_not_as_empty_text() {
    let ha = FakeHa::start(TOKEN, Answer::NothingHeard);
    let voice = ready(&ha, json!({}));
    voice.listen();
    assert!(wait_until(WAIT, || ha.seen(|s| s.audio.len() >= AUDIO_BYTES)));
    voice.stop();
    let failed = voice.state("error").expect("error");
    assert_eq!(failed["error"]["code"], "nothing_heard");
    assert!(failed.get("text").is_none());
    assert!(wait_until(WAIT, || voice.mic.gone()));
}

#[test]
fn home_assistant_ending_the_speech_first_closes_the_microphone_without_the_key() {
    let ha = FakeHa::start(TOKEN, Answer::Early(8_000, "open settings"));
    let voice = ready(&ha, json!({}));
    voice.listen();
    // No stop: the voice-activity detector is what ended it.
    let heard = voice.state("heard").expect("heard");
    assert_eq!(heard["text"], "open settings");
    assert!(wait_until(WAIT, || voice.mic.gone()));
    assert_eq!(
        ha.seen(|s| s.ends),
        0,
        "the audio was not ended: Home Assistant had already answered"
    );
}

#[test]
fn the_key_never_released_still_closes_the_microphone_at_the_longest_time() {
    let ha = FakeHa::start(TOKEN, Answer::Words("still here"));
    let voice = ready(&ha, json!({"maxSeconds": 1}));
    voice.listen();
    // No stop at all.
    let heard = voice
        .rig
        .wait_topic("voice", WAIT, |v| v["state"] == "heard")
        .expect("heard by itself");
    assert_eq!(heard["text"], "still here");
    assert!(wait_until(WAIT, || voice.mic.gone()));
}

#[test]
fn a_refused_token_is_said_in_words_and_the_microphone_closes() {
    let ha = FakeHa::start("another-token", Answer::Words("x"));
    let voice = ready(&ha, json!({}));
    voice.listen();
    let failed = voice.state("error").expect("error");
    assert_eq!(failed["error"]["code"], "auth");
    assert!(
        failed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("token")
    );
    assert!(!failed.to_string().contains(TOKEN));
    assert!(wait_until(WAIT, || voice.mic.gone()));
}

#[test]
fn an_unreachable_home_assistant_and_a_missing_microphone_are_errors_not_hangs() {
    // Nothing listens at the saved address.
    let closed = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    let mut rig = Rig::new();
    receiver_off(&rig.paths);
    let mic = FakeMic::new(rig.dir.path());
    seed_settings(
        &rig.paths,
        "base.voice",
        json!({"enabled": {"type": "bool", "value": true}}),
    );
    voice_store::save(&rig.paths.data_dir, &format!("http://{closed}"), TOKEN).unwrap();
    rig.start_voice(config(&mic, json!({})), Script::new());
    post(rig.addr, "/v1/voice/listen", "{}");
    let failed = rig
        .wait_topic("voice", WAIT, |v| v["state"] == "error")
        .expect("error");
    assert_eq!(failed["error"]["code"], "unreachable");
    assert!(wait_until(WAIT, || mic.gone()));
    rig.stop();

    // A microphone program that does not exist.
    let ha = FakeHa::start(TOKEN, Answer::Words("x"));
    let mut rig = Rig::new();
    receiver_off(&rig.paths);
    seed_settings(
        &rig.paths,
        "base.voice",
        json!({"enabled": {"type": "bool", "value": true}}),
    );
    voice_store::save(&rig.paths.data_dir, &ha.url(), TOKEN).unwrap();
    rig.start_voice(
        test_config(json!({"voice": {"capture": ["/nonexistent/arecord"]}})),
        Script::new(),
    );
    post(rig.addr, "/v1/voice/listen", "{}");
    let failed = rig
        .wait_topic("voice", WAIT, |v| v["state"] == "error")
        .expect("error");
    assert_eq!(failed["error"]["code"], "microphone");
    assert_eq!(
        ha.seen(|s| s.connections),
        0,
        "nothing is dialled without a microphone"
    );
}

#[test]
fn the_token_is_never_dialled_to_an_address_outside_the_home_network() {
    // The service as it runs on the TV: the loopback is not the home network, so the token is not sent.
    let ha = FakeHa::start(TOKEN, Answer::Words("x"));
    let mut rig = Rig::new();
    receiver_off(&rig.paths);
    let mic = FakeMic::new(rig.dir.path());
    seed_settings(
        &rig.paths,
        "base.voice",
        json!({"enabled": {"type": "bool", "value": true}}),
    );
    voice_store::save(&rig.paths.data_dir, &ha.url(), TOKEN).unwrap();
    rig.start(config(&mic, json!({})), Script::new());
    post(rig.addr, "/v1/voice/listen", "{}");
    let failed = rig
        .wait_topic("voice", WAIT, |v| v["state"] == "error")
        .expect("error");
    assert_eq!(failed["error"]["code"], "unreachable");
    assert_eq!(ha.seen(|s| (s.connections, s.auths.len())), (0, 0));
    assert!(wait_until(WAIT, || mic.gone()));
}

#[test]
fn a_chosen_pipeline_is_asked_for_and_an_unknown_one_is_reported() {
    let ha = FakeHa::start(TOKEN, Answer::Words("x"));
    let voice = ready(&ha, json!({}));
    let set = post(
        voice.rig.addr,
        "/v1/settings/base.voice",
        r#"{"changes":[{"key":"pipeline","value":{"type":"text","value":"01m3yddanfdmg4epdmbmaht8kb"}}]}"#,
    );
    assert_eq!(set.status, 200, "{}", String::from_utf8_lossy(&set.body));
    // A pipeline id is a word, not a path or a second message.
    let bad = post(
        voice.rig.addr,
        "/v1/settings/base.voice",
        r#"{"changes":[{"key":"pipeline","value":{"type":"text","value":"a\"b"}}]}"#,
    );
    assert_eq!(bad.status, 422);
    voice.listen();
    assert!(wait_until(WAIT, || ha.seen(|s| !s.runs.is_empty())));
    assert_eq!(
        ha.seen(|s| s.runs[0]["pipeline"].clone()),
        json!("01m3yddanfdmg4epdmbmaht8kb")
    );
    post(voice.rig.addr, "/v1/voice/cancel", "{}");
    assert!(wait_until(WAIT, || voice.mic.gone()));

    // Emptied by the owner: Home Assistant's preferred pipeline (no `pipeline` in the request).
    let empty = post(
        voice.rig.addr,
        "/v1/settings/base.voice",
        r#"{"changes":[{"key":"pipeline","value":{"type":"text","value":""}}]}"#,
    );
    assert_eq!(empty.status, 200);
    voice.listen();
    assert!(wait_until(WAIT, || ha.seen(|s| s.runs.len() == 2)));
    assert!(ha.seen(|s| s.runs[1].get("pipeline").is_none()));
    post(voice.rig.addr, "/v1/voice/cancel", "{}");
    assert!(wait_until(WAIT, || voice.mic.gone()));

    *ha.shared.answer.lock() = Answer::UnknownPipeline;
    voice.listen();
    let failed = voice.state("error").expect("error");
    assert_eq!(failed["error"]["code"], "pipeline");
    assert!(
        failed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Pipeline not found")
    );
    assert!(wait_until(WAIT, || voice.mic.gone()));
}

#[test]
fn switching_voice_search_off_while_it_listens_closes_the_microphone() {
    let ha = FakeHa::start(TOKEN, Answer::Silent);
    let voice = ready(&ha, json!({}));
    voice.listen();
    assert!(wait_until(WAIT, || ha.seen(|s| s.audio.len() >= AUDIO_BYTES)));
    let off = post(
        voice.rig.addr,
        "/v1/settings/base.voice",
        r#"{"changes":[{"key":"enabled","value":{"type":"bool","value":false}}]}"#,
    );
    assert_eq!(off.status, 200);
    assert!(
        wait_until(WAIT, || voice.mic.gone()),
        "the microphone is still open after the switch went off"
    );
    let state = voice
        .rig
        .wait_topic("voice", WAIT, |v| {
            v["state"] == "idle" && v["ready"] == false
        })
        .expect("idle and not ready");
    assert_eq!(state["reason"], "voice_off");
}

#[test]
fn stopping_the_helper_closes_the_microphone() {
    let ha = FakeHa::start(TOKEN, Answer::Silent);
    let mut voice = ready(&ha, json!({}));
    voice.listen();
    assert!(wait_until(WAIT, || ha.seen(|s| s.audio.len() >= AUDIO_BYTES)));
    voice.rig.stop();
    assert!(
        wait_until(WAIT, || voice.mic.gone()),
        "the microphone outlived the helper's stop"
    );
}

#[test]
fn the_voice_routes_are_listed_and_ai_access_has_no_tool_that_reaches_the_microphone() {
    let ha = FakeHa::start(TOKEN, Answer::Silent);
    let voice = ready(&ha, json!({}));
    let routes = get(voice.rig.addr, "/v1").json();
    let paths: Vec<&str> = routes["endpoints"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["path"].as_str().unwrap())
        .collect();
    for path in [
        "/v1/voice",
        "/v1/voice/listen",
        "/v1/voice/stop",
        "/v1/voice/cancel",
        "/v1/voice/refresh",
    ] {
        assert!(paths.contains(&path), "{path}");
    }
    // The assistants' catalogue (what the LAN door serves) has nothing to do with voice, listening or the microphone.
    for tool in lumatv_helper::ai::tools::catalogue() {
        let text = format!("{} {} {}", tool.name, tool.title, tool.description).to_lowercase();
        for word in ["voice", "microphone", "listen", "speech", "audio"] {
            assert!(!text.contains(word), "tool {} mentions {word}", tool.name);
        }
    }
    assert_eq!(get(voice.rig.addr, "/v1/voice/listen").status, 405);
    assert_eq!(post(voice.rig.addr, "/v1/voice", "{}").status, 405);
    assert_eq!(get(voice.rig.addr, "/v1/voice/nope").status, 404);
}
