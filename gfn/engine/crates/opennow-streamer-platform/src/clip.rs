//! The game recorder in the streamer: when the engine's runtime prefs ask for it, the stream the feeder is given is also kept in a
//! RAM ring (`lumagame-recorder`), sent on to the receiver while long replay or a recording is on, and saved as a clip on request.
//! The tee itself sits in `FeederClient::send_media`; this file only builds the recorder from the prefs, owns it for the session,
//! passes it the in-stream menu's requests and says what became of them on the engine's side socket.
use std::path::Path;
use std::sync::{Arc, Mutex};

use lumagame_recorder::{Event, HttpDestination, KeyframeRequest, Plan, Recorder, RING_CAP};

struct Active {
    recorder: Arc<Recorder>,
    seconds: u32,
    title: String,
}

static ACTIVE: Mutex<Option<Active>> = Mutex::new(None);

/// The control commands that are the recorder's; the engine sends them as `streamer.control` actions of the same names.
pub const RECORDER_ACTIONS: [&str; 5] = ["save-clip", "replay-on", "replay-off", "record-start", "record-stop"];

#[derive(Debug, PartialEq, Eq)]
struct Prefs {
    url: String,
    /// The length of a clip from the ring.
    seconds: u32,
    /// The game's name, which the receiver files under.
    title: String,
    plan: Plan,
}

/// The `recorder` object of the prefs the engine wrote for this launch; absent unless the viewer turned the recorder on.
fn prefs(bytes: &[u8]) -> Option<Prefs> {
    let v: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let r = v.get("recorder")?;
    let number = |key: &str, default: u32| r[key].as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(default);
    Some(Prefs {
        url: r["url"].as_str().filter(|u| !u.is_empty())?.to_owned(),
        seconds: number("seconds", 15),
        title: r["title"].as_str().unwrap_or_default().to_owned(),
        plan: Plan {
            replay: r["replay"].as_bool().unwrap_or(false),
            reach_s: number("reach", 300),
            mbps: number("mbps", 0),
            record_max_s: number("recordMax", 7200),
            keyframe_s: number("keyframes", 5),
        },
    })
}

fn line(saved: bool, text: &str) -> String {
    serde_json::json!({"stream": "clip", "saved": saved, "text": text}).to_string()
}

fn state_line(replay: bool, recording: bool, ready: bool) -> String {
    serde_json::json!({"stream": "recorder", "replay": replay, "recording": recording, "ready": ready}).to_string()
}

/// Starts the recorder for a stream when the prefs ask for one; `keyframes` asks the game server for a keyframe. A destination that
/// cannot be used is said once on the side socket and the stream plays on without a recorder.
pub(crate) fn arm(prefs_file: Option<&Path>, keyframes: KeyframeRequest) -> Option<Arc<Recorder>> {
    let p = prefs(&std::fs::read(prefs_file?).ok()?)?;
    let destination = match HttpDestination::new(&p.url) {
        Ok(d) => d,
        Err(why) => {
            crate::runtime::stream_clip(line(false, &why));
            return None;
        }
    };
    crate::runtime::stream_clip(state_line(p.plan.replay, false, p.plan.keyframe_s == 0));
    let recorder = Arc::new(Recorder::new(RING_CAP, Arc::new(destination), p.plan, keyframes, |event| {
        crate::runtime::stream_clip(match (event.sentence(), event) {
            (Some((saved, text)), _) => line(saved, &text),
            (None, Event::State { replay, recording, ready }) => state_line(replay, recording, ready),
            (None, _) => return,
        });
    }));
    *ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Active { recorder: Arc::clone(&recorder), seconds: p.seconds, title: p.title });
    Some(recorder)
}

/// The stream ended: the recorder goes, with what it held. The receiver closes a recording when its stream ends.
pub(crate) fn disarm() {
    *ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The picture is in trouble (a frame was lost or the decoder is recovering): the recorder asks the game server for nothing for a few
/// seconds. Called from the video path, so it never waits for the lock.
pub(crate) fn trouble() {
    if let Ok(active) = ACTIVE.try_lock() {
        if let Some(a) = active.as_ref() {
            a.recorder.trouble();
        }
    }
}

/// One of the in-stream menu's recorder requests ([`RECORDER_ACTIONS`]). The answer comes later as `clip` and `recorder` lines; a
/// refusal comes at once.
pub fn recorder_action(action: &str) {
    let guard = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    let refusal = match guard.as_ref() {
        None => Some("The recorder is not running for this stream".to_owned()),
        Some(a) => match action {
            "save-clip" => a.recorder.save_clip(&a.title, a.seconds).err().map(|r| r.to_string()),
            "replay-on" | "replay-off" => {
                a.recorder.set_replay(action == "replay-on");
                None
            }
            "record-start" => {
                a.recorder.start_recording(&a.title);
                None
            }
            _ => {
                a.recorder.stop_recording();
                None
            }
        },
    };
    drop(guard);
    if let Some(text) = refusal {
        crate::runtime::stream_clip(line(false, &text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_recorder_starts_only_from_a_recorder_object_with_an_address() {
        assert_eq!(prefs(br#"{"deadzone":8}"#), None);
        assert_eq!(prefs(br#"{"recorder":{"url":"","seconds":9}}"#), None);
        assert_eq!(
            prefs(br#"{"recorder":{"url":"http://nas:8765","seconds":20,"replay":true,"reach":900,"mbps":65,"recordMax":3600,"title":"Hades II"}}"#),
            Some(Prefs {
                url: "http://nas:8765".into(),
                seconds: 20,
                title: "Hades II".into(),
                plan: Plan { replay: true, reach_s: 900, mbps: 65, record_max_s: 3600, keyframe_s: 5 }
            })
        );
        assert_eq!(prefs(br#"{"recorder":{"url":"http://nas:8765","keyframes":0}}"#).unwrap().plan.keyframe_s, 0);
        let said: serde_json::Value = serde_json::from_str(&state_line(true, false, false)).unwrap();
        assert_eq!(said, serde_json::json!({"stream": "recorder", "replay": true, "recording": false, "ready": false}));
    }
}
