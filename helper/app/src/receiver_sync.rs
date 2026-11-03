//! Tracks LG sound-output notifications across wake and standby.

use crate::receiver::Cmd;
use parking_lot::Mutex;
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, mpsc::Sender};
use std::thread::{self, JoinHandle};

const URI: &str = "luna://com.webos.settingsservice/getSystemSettings";
const PAYLOAD: &str = r#"{"category":"sound","keys":["soundOutput"],"subscribe":true}"#;

#[derive(Clone, Copy, Debug, Default)]
pub struct State {
    pub valid: bool,
    pub preference: Option<&'static str>,
    pub generation: u64,
}

fn accept(state: &mut State, value: &Value) -> bool {
    if value.get("returnValue").and_then(Value::as_bool) != Some(true) {
        state.valid = false;
        return false;
    }
    let Some(output) = value
        .pointer("/settings/soundOutput")
        .and_then(Value::as_str)
    else {
        state.valid = false;
        return false;
    };
    if !state.valid {
        if value.get("subscribed").and_then(Value::as_bool) != Some(true) {
            return false;
        }
        state.valid = true;
        state.preference = arc_preference(output);
        state.generation += 1;
    } else if value.get("store").and_then(Value::as_bool) != Some(false) {
        state.preference = arc_preference(output);
        state.generation += 1;
    }
    true
}
fn arc_preference(output: &str) -> Option<&'static str> {
    match output {
        "external_arc" => Some("external_arc"),
        "tv_speaker_external_arc" => Some("tv_speaker_external_arc"),
        _ => None,
    }
}

pub struct Subscription {
    child: Child,
    reader: Option<JoinHandle<()>>,
    state: Arc<Mutex<State>>,
}

impl Subscription {
    pub fn start(program: &str, wake: Sender<Cmd>) -> std::io::Result<Self> {
        let mut child = Command::new(program)
            .args(["-n", "1000000", URI, PAYLOAD])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let Some(out) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::other("no subscription output"));
        };
        let state = Arc::new(Mutex::new(State::default()));
        let shared = state.clone();
        let reader = match thread::Builder::new()
            .name("receiver-output".into())
            .stack_size(128 * 1024)
            .spawn(move || {
                let mut pending = String::new();
                for line in BufReader::new(out).lines() {
                    let Ok(line) = line else { break };
                    pending.push_str(&line);
                    pending.push('\n');
                    match serde_json::from_str::<Value>(&pending) {
                        Ok(value) => {
                            pending.clear();
                            if !accept(&mut shared.lock(), &value) {
                                break;
                            }
                            let _ = wake.send(Cmd::Wake);
                        }
                        Err(e) if e.is_eof() && pending.len() <= 64 * 1024 => {}
                        Err(_) => break,
                    }
                }
                shared.lock().valid = false;
                let _ = wake.send(Cmd::Wake);
            }) {
            Ok(reader) => reader,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        Ok(Self {
            child,
            reader: Some(reader),
            state,
        })
    }

    pub fn state(&self) -> State {
        *self.state.lock()
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteResult {
    NotAttempted,
    Unconfirmed,
    Confirmed,
}

fn native_reply(
    source: &crate::refreshguard::LunaSource,
    uri: &str,
    payload: &str,
) -> Option<Value> {
    source
        .call(uri, payload)
        .ok()
        .and_then(|reply| serde_json::from_str::<Value>(&reply).ok())
        .filter(|reply| reply.get("returnValue").and_then(Value::as_bool) == Some(true))
}

fn output_is(
    source: &crate::refreshguard::LunaSource,
    output: &str,
    current: &dyn Fn() -> bool,
) -> Option<bool> {
    if !current() {
        return None;
    }
    let audio = native_reply(
        source,
        "luna://com.webos.service.audio/getSoundOutput",
        "{}",
    )?;
    if !current() {
        return None;
    }
    if audio.get("soundOutput")?.as_str()? != output {
        return Some(false);
    }
    let volume = native_reply(
        source,
        "luna://com.webos.service.audio/master/getVolume",
        "{}",
    )?;
    current().then(|| {
        volume
            .pointer("/volumeStatus/soundOutput")
            .and_then(Value::as_str)
            == Some(output)
    })
}

pub fn tv_speaker_active(program: &str, current: &dyn Fn() -> bool) -> Option<bool> {
    output_is(
        &crate::refreshguard::LunaSource::new(program),
        "tv_speaker",
        current,
    )
}

pub fn set_temporary(program: &str, output: &str, current: &dyn Fn() -> bool) -> RouteResult {
    if !current() {
        return RouteResult::NotAttempted;
    }
    let source = crate::refreshguard::LunaSource::new(program);
    if output != "tv_speaker" {
        // A previous restore may already have applied despite a lost reply.
        // Confirm that route instead of restarting native ARC/CEC routing.
        match output_is(&source, output, current) {
            Some(true) => return RouteResult::Confirmed,
            None => return RouteResult::NotAttempted,
            Some(false) => {}
        }
    }
    if !current() {
        return RouteResult::NotAttempted;
    }
    // This native route changes current audio and volume routing without writing
    // the saved soundOutput preference. Even a lost reply may have applied it.
    let payload = serde_json::json!({"soundOut":output}).to_string();
    if native_reply(
        &source,
        "luna://com.webos.service.audio/soundSettings/setSoundOut",
        &payload,
    )
    .is_none()
    {
        return RouteResult::Unconfirmed;
    }
    if output_is(&source, output, current) == Some(true) {
        RouteResult::Confirmed
    } else {
        RouteResult::Unconfirmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn temporary_route_requires_native_audio_and_volume_confirmation_and_stays_cancelable() {
        use std::os::unix::fs::PermissionsExt;
        for (audio, volume, cancel, expected) in [
            ("tv_speaker", "tv_speaker", false, true),
            ("external_arc", "tv_speaker", false, false),
            ("tv_speaker", "external_arc", false, false),
            ("tv_speaker", "tv_speaker", true, false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let script = dir.path().join("luna-send");
            let cancelled = dir.path().join("cancelled");
            let calls = dir.path().join("calls");
            let body = format!(
                r##"#!/bin/sh
printf '%s\n' "$3" >> '{calls}'
case "$3" in
  */soundSettings/setSoundOut) {cancel_command}; echo '{{"returnValue":true}}' ;;
  */getSoundOutput) echo '{{"returnValue":true,"soundOutput":"{audio}","settingsSoundOutputs":["{audio}"]}}' ;;
  */master/getVolume) echo '{{"returnValue":true,"volumeStatus":{{"soundOutput":"{volume}"}}}}' ;;
  *) exit 1 ;;
esac
"##,
                calls = calls.display(),
                cancel_command = if cancel {
                    format!("touch '{}'", cancelled.display())
                } else {
                    ":".into()
                }
            );
            std::fs::write(&script, body).unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(
                set_temporary(script.to_str().unwrap(), "tv_speaker", &|| !cancelled
                    .exists()),
                if expected {
                    RouteResult::Confirmed
                } else {
                    RouteResult::Unconfirmed
                }
            );
            let records = std::fs::read_to_string(calls).unwrap();
            assert!(!records.contains("setSystemSettings"));
            if cancel {
                assert_eq!(records.lines().count(), 1);
            }
        }
    }

    #[test]
    fn temporary_notifications_preserve_preference_and_same_value_saved_write_revokes_it() {
        let mut state = State::default();
        assert!(accept(
            &mut state,
            &json!({"returnValue":true,"subscribed":true,"settings":{"soundOutput":"external_arc"}})
        ));
        let generation = state.generation;
        assert!(accept(
            &mut state,
            &json!({"returnValue":true,"store":false,"settings":{"soundOutput":"tv_speaker"}})
        ));
        assert_eq!(state.preference, Some("external_arc"));
        assert_eq!(state.generation, generation);
        assert!(accept(
            &mut state,
            &json!({"returnValue":true,"settings":{"soundOutput":"tv_speaker"}})
        ));
        assert_eq!(state.preference, None);
        assert_ne!(state.generation, generation);
        assert!(!accept(&mut state, &json!({"returnValue":false})));
        assert!(!state.valid);
    }
    #[test]
    fn malformed_subscription_invalidates_the_live_state() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::Duration;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("luna-send");
        std::fs::write(
            &script,
            r##"#!/bin/sh
echo '{"returnValue":true,"subscribed":true,"settings":{"soundOutput":"external_arc"}}'
sleep 0.2
echo 'not-json'
exec sleep 60
"##,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let sub = Subscription::start(script.to_str().unwrap(), tx).unwrap();
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(sub.state().valid);
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(!sub.state().valid);
    }
}
