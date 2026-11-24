//! The game recorder in the streamer: when the engine's runtime prefs ask for it, the stream the feeder is given is also kept in a
//! RAM ring (`lumagame-recorder`) and `save-clip` sends the last seconds to the receiver the viewer named. The tee itself sits in
//! `FeederClient::send_media`; this file only builds the recorder from the prefs, owns it for the session and says what became of a
//! clip on the engine's side socket.
use std::path::Path;
use std::sync::{Arc, Mutex};

use lumagame_recorder::{HttpDestination, Recorder, RING_CAP};

struct Active {
    recorder: Arc<Recorder>,
    seconds: u32,
}

static ACTIVE: Mutex<Option<Active>> = Mutex::new(None);

#[derive(Debug, PartialEq, Eq)]
struct Prefs {
    url: String,
    folder: String,
    seconds: u32,
}

/// The `recorder` object of the prefs the engine wrote for this launch; absent unless the viewer turned the recorder on.
fn prefs(bytes: &[u8]) -> Option<Prefs> {
    let v: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let r = v.get("recorder")?;
    Some(Prefs {
        url: r["url"].as_str().filter(|u| !u.is_empty())?.to_owned(),
        folder: r["folder"].as_str().unwrap_or_default().to_owned(),
        seconds: r["seconds"].as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(15),
    })
}

fn line(saved: bool, text: &str) -> String {
    serde_json::json!({"stream": "clip", "saved": saved, "text": text}).to_string()
}

/// Starts the recorder for a stream when the prefs ask for one. A destination that cannot be used is said once on the side socket and
/// the stream plays on without a recorder.
pub(crate) fn arm(prefs_file: Option<&Path>) -> Option<Arc<Recorder>> {
    let p = prefs(&std::fs::read(prefs_file?).ok()?)?;
    let destination = match HttpDestination::new(&p.url, &p.folder) {
        Ok(d) => d,
        Err(why) => {
            crate::runtime::stream_clip(line(false, &why));
            return None;
        }
    };
    let recorder = Arc::new(Recorder::new(RING_CAP, "lumagame", Arc::new(destination), |event| {
        if let Some((saved, text)) = event.sentence() {
            crate::runtime::stream_clip(line(saved, &text));
        }
    }));
    *ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Active { recorder: Arc::clone(&recorder), seconds: p.seconds });
    Some(recorder)
}

/// The stream ended: the recorder goes, with what it held.
pub(crate) fn disarm() {
    *ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The in-stream menu's Save clip. The answer comes later as a `clip` line; a refusal comes at once.
pub fn save_clip() {
    let active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|a| (Arc::clone(&a.recorder), a.seconds));
    let refusal = match active {
        None => Some("The recorder is not running for this stream".to_owned()),
        Some((recorder, seconds)) => recorder.save_last(seconds).err().map(|r| r.to_string()),
    };
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
        assert_eq!(prefs(br#"{"recorder":{"url":"","folder":"x","seconds":9}}"#), None);
        assert_eq!(
            prefs(br#"{"recorder":{"url":"http://nas:8765","folder":"lumagame","seconds":20}}"#),
            Some(Prefs { url: "http://nas:8765".into(), folder: "lumagame".into(), seconds: 20 })
        );
    }
}
