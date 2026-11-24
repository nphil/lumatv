//! OpenNOW's standalone streamer entry (native/opennow-streamer/crates/opennow-streamer/src/main.rs at the pinned revision) with
//! three additions: a `controller-rumble` event is applied to the pad here, in the process that holds the pad, before it goes up the
//! pipe; what the engine needs of the events the core does not pass on goes to the engine's side socket; and `input-paused` moves
//! only the pads (`pause_pads_only`); `steam-overlay` types Shift+Tab into the game (`steam_overlay`); the game recorder's requests
//! (`save-clip`, `replay-on`, `replay-off`, `record-start`, `record-stop`) go to the recorder (`recorder_command`). The binary keeps
//! upstream's name because opennow-core looks for `opennow-streamer` beside itself.
use std::io::{self, BufRead, Write};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}, mpsc};
use std::thread;
use std::time::Duration;

use opennow_streamer_core::Engine;
use opennow_streamer_platform::{CapturedInput, MediaRuntime, RECORDER_ACTIONS, controller_rumble, create_runtime, recorder_action, stream_ended, stream_idle, stream_telemetry};
use opennow_streamer_protocol::{Command, error, response};
use serde_json::Value;

fn write_message(message: &Value) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, message)?;
    writeln!(stdout)?;
    stdout.flush()
}

fn apply_rumble(message: &Value) {
    if message["type"] != "controller-rumble" {
        return;
    }
    let slot = message["controllerId"].as_u64().unwrap_or(0) as u8;
    let low = message["lowFrequency"].as_u64().unwrap_or(0) as u16;
    let high = message["highFrequency"].as_u64().unwrap_or(0) as u16;
    let duration = message["durationMs"].as_u64().unwrap_or(0) as u32;
    if let Err(e) = controller_rumble(slot, low, high, duration) {
        eprintln!("rumble slot {slot}: {e}");
    }
}

/// The core keeps only the frame rate and bitrate of the streamer's `telemetry` event, a stream that ended on its own (a `stopped`
/// status naming its `termination`) only as a status it reports when asked, and nothing of the far side's `session-timer` idle
/// countdown, so the engine gets all three from here.
fn forward_to_engine(message: &Value) {
    if message["type"] == "telemetry" {
        stream_telemetry(message);
    } else if message["type"] == "session-timer" {
        stream_idle(message);
    } else if message["type"] == "status" && message["status"] == "stopped" && message["termination"].is_object() {
        stream_ended(&message["termination"]);
    }
}

/// The overlay's `input-paused`, answered here so that it reaches only the pads and the stream keeps running under the overlay.
/// Upstream's handler also pauses the NVST video receivers, which then drop every packet before SRTP sees it, so the rollover counter
/// stops following the sequence numbers. Past 2^15 packets the receiver guesses an index one rollover off and no packet authenticates
/// again (opennow-streamer-transport `ReplayWindow::guess_packet_index`, pinned revision): a 72 s menu, about 47,000 packets of a
/// 4K120 stream, lost the session on 2026-11-20.
fn pause_pads_only(command: &Command, pause_pads: impl FnOnce(bool) -> Result<(), String>) -> Option<Value> {
    if command.kind != "input-paused" {
        return None;
    }
    let Some(paused) = command.paused else {
        return Some(error(Some(&command.id), "missing-paused", "Pause command does not include paused state"));
    };
    Some(match pause_pads(paused) {
        Ok(()) => response(command.id.as_str(), "ok"),
        Err(message) => error(Some(&command.id), "media-host-unavailable", message),
    })
}

/// Shift+Tab, the keys Steam opens its overlay on, as upstream's desktop streamer encodes a typed chord (the official client's Guide
/// button never reaches the host: it sits on bit 0x10000, which the 16-bit button field drops). Each step waits `.0` after the one
/// before: Shift stays down before Tab and Tab stays down for tens of milliseconds, as a typed chord does, because four events
/// pushed in one instant opened the overlay only now and then (2026-11-22), which fits a host that samples keys once a frame.
const OVERLAY_CHORD: [(Duration, u16, u16, bool); 4] = {
    const SHIFT: u16 = 0xa0;
    const TAB: u16 = 0x09;
    const SHIFT_MODIFIER: u16 = 0x01;
    [
        (Duration::ZERO, SHIFT, 0, true),
        (Duration::from_millis(40), TAB, SHIFT_MODIFIER, true),
        (Duration::from_millis(80), TAB, SHIFT_MODIFIER, false),
        (Duration::from_millis(40), SHIFT, 0, false),
    ]
};

fn play_overlay_chord(mut push: impl FnMut(CapturedInput), mut sleep: impl FnMut(Duration)) {
    for (wait, virtual_key, modifiers, pressed) in OVERLAY_CHORD {
        sleep(wait);
        push(CapturedInput::Key { virtual_key, modifiers, pressed });
    }
}

/// The in-stream menu's "Steam overlay". The chord plays on its own thread so the protocol loop keeps answering (a stop or a pause
/// is never held for the chord's 160 ms); one chord at a time, and a second request during it is refused rather than interleaved.
fn steam_overlay(command: &Command, playing: &Arc<AtomicBool>, play: impl FnOnce(Arc<AtomicBool>)) -> Option<Value> {
    if command.kind != "steam-overlay" {
        return None;
    }
    if playing.swap(true, Ordering::AcqRel) {
        return Some(error(Some(&command.id), "steam-overlay-busy", "The Steam overlay keys are still being sent"));
    }
    play(Arc::clone(playing));
    Some(response(command.id.as_str(), "ok"))
}

/// The in-stream menu's recorder requests: the game recorder takes the request and the outcome, or a refusal, goes to the engine as
/// `clip` and `recorder` lines on the side socket, so the command itself is answered at once.
fn recorder_command(command: &Command, act: impl FnOnce(&str)) -> Option<Value> {
    if !RECORDER_ACTIONS.contains(&command.kind.as_str()) {
        return None;
    }
    act(&command.kind);
    Some(response(command.id.as_str(), "ok"))
}

fn run_protocol(media_runtime: MediaRuntime) -> io::Result<()> {
    let (event_tx, event_rx) = mpsc::channel::<Value>();
    let event_writer = thread::spawn(move || {
        while let Ok(message) = event_rx.recv() {
            apply_rumble(&message);
            forward_to_engine(&message);
            if write_message(&message).is_err() {
                break;
            }
        }
    });
    let pads = media_runtime.clone();
    let keys = pads.captured_input();
    let playing = Arc::new(AtomicBool::new(false));
    let mut engine = Engine::with_media_runtime(event_tx, media_runtime);

    for line in io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let command: Command = match serde_json::from_str(&line) {
            Ok(command) => command,
            Err(parse_error) => {
                write_message(&error(None, "invalid-command", parse_error.to_string()))?;
                continue;
            }
        };
        if let Some(reply) = pause_pads_only(&command, |paused| pads.set_paused(paused)) {
            write_message(&reply)?;
            continue;
        }
        let play = |done: Arc<AtomicBool>| {
            let keys = Arc::clone(&keys);
            thread::spawn(move || {
                play_overlay_chord(|key| keys.push(key), thread::sleep);
                done.store(false, Ordering::Release);
            });
        };
        if let Some(reply) = steam_overlay(&command, &playing, play) {
            write_message(&reply)?;
            continue;
        }
        if let Some(reply) = recorder_command(&command, recorder_action) {
            write_message(&reply)?;
            continue;
        }
        let (responses, keep_running) = engine.handle(command);
        for response in responses {
            write_message(&response)?;
        }
        if !keep_running {
            break;
        }
    }

    drop(engine);
    let _ = event_writer.join();
    Ok(())
}

fn main() -> io::Result<()> {
    if let Some(path) = std::env::var_os("OPENNOW_STREAMER_LOG_FILE") {
        let _ = opennow_streamer_protocol::log::set_log_file(&path.to_string_lossy());
    }
    let (host, media_runtime) = create_runtime().map_err(io::Error::other)?;
    let shutdown_runtime = media_runtime.clone();
    let protocol = thread::Builder::new()
        .name("opennow-protocol".to_owned())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_protocol(media_runtime)));
            shutdown_runtime.shutdown();
            result
        })?;
    host.run();
    match protocol.join() {
        Ok(Ok(result)) => result,
        Ok(Err(payload)) => std::panic::resume_unwind(payload),
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(json: &str) -> Command {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn input_paused_moves_only_the_pads_and_never_reaches_the_engine() {
        let mut pads = Vec::new();
        let reply = pause_pads_only(&command(r#"{"id":"input-paused","type":"input-paused","paused":true}"#), |paused| {
            pads.push(paused);
            Ok(())
        });
        assert_eq!(reply, Some(response("input-paused", "ok")));
        assert_eq!(pads, [true]);
        assert_eq!(pause_pads_only(&command(r#"{"id":"stop","type":"stop"}"#), |_| unreachable!()), None);
    }

    #[test]
    fn the_overlay_chord_holds_shift_then_tab_with_a_hold_on_a_fake_clock() {
        let clock = std::cell::Cell::new(Duration::ZERO);
        let mut seen = Vec::new();
        play_overlay_chord(|key| seen.push((clock.get(), key)), |wait| clock.set(clock.get() + wait));
        let at = |ms, virtual_key, modifiers, pressed| (Duration::from_millis(ms), CapturedInput::Key { virtual_key, modifiers, pressed });
        assert_eq!(seen, [at(0, 0xa0, 0, true), at(40, 0x09, 1, true), at(120, 0x09, 1, false), at(160, 0xa0, 0, false)]);
    }

    #[test]
    fn a_recorder_request_goes_to_the_recorder_and_is_answered_at_once() {
        let mut asked = Vec::new();
        for kind in RECORDER_ACTIONS {
            let reply = recorder_command(&command(&format!(r#"{{"id":"runtime-control","type":"{kind}"}}"#)), |action| asked.push(action.to_owned()));
            assert_eq!(reply, Some(response("runtime-control", "ok")));
        }
        assert_eq!(asked, RECORDER_ACTIONS);
        assert_eq!(recorder_command(&command(r#"{"id":"stop","type":"stop"}"#), |_| unreachable!()), None);
    }

    #[test]
    fn one_chord_plays_at_a_time_and_other_commands_pass() {
        let playing = Arc::new(AtomicBool::new(false));
        let overlay = command(r#"{"id":"runtime-control","type":"steam-overlay"}"#);
        let mut started = 0;
        let reply = steam_overlay(&overlay, &playing, |_| started += 1);
        assert_eq!(reply, Some(response("runtime-control", "ok")));
        let again = steam_overlay(&overlay, &playing, |_| started += 1).unwrap();
        assert_eq!(again["code"], "steam-overlay-busy");
        assert_eq!(started, 1);
        playing.store(false, Ordering::Release);
        assert!(steam_overlay(&overlay, &playing, |_| started += 1).is_some());
        assert_eq!(started, 2);
        assert_eq!(steam_overlay(&command(r#"{"id":"stop","type":"stop"}"#), &playing, |_| unreachable!()), None);
    }
}
