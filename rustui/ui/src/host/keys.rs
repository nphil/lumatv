//! The key socket: the compositor's end of a socket pair it hands this process at its start (`LUMATV_UI_KEYS_FD`), JSON lines,
//! one per message, from the compositor only: `{"key": {"k": "up", "d": true, "r": false, "seat": 0}}` while the shell's stack has
//! the keys (Home drawn means Home has them; `wants` means the system layer has them; `takes` as before for an overlay card's OK
//! and Back) and `{"pointer": {"x": 960.0, "y": 540.0, "buttons": 0, "visible": true}}` samples at the pacer's rate while the pointer
//! is over a shell layer, `visible` false once when it goes. Read on a thread; the loop takes the messages with the rest.

use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender, channel};

#[derive(Clone, Debug, PartialEq)]
pub enum KeyEvent {
    Key {
        name: String,
        down: bool,
        repeat: bool,
        seat: u32,
    },
    Pointer {
        x: f32,
        y: f32,
        buttons: u32,
        visible: bool,
    },
    /// The compositor closed its end.
    Closed,
}

/// Parses one line of the socket.
pub fn parse(line: &str) -> Option<KeyEvent> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    if let Some(k) = v.get("key") {
        return Some(KeyEvent::Key {
            name: k.get("k")?.as_str()?.to_string(),
            down: k.get("d").and_then(Value::as_bool).unwrap_or(true),
            repeat: k.get("r").and_then(Value::as_bool).unwrap_or(false),
            seat: k.get("seat").and_then(Value::as_u64).unwrap_or(0) as u32,
        });
    }
    if let Some(p) = v.get("pointer") {
        return Some(KeyEvent::Pointer {
            x: p.get("x").and_then(Value::as_f64).unwrap_or(0.0) as f32,
            y: p.get("y").and_then(Value::as_f64).unwrap_or(0.0) as f32,
            buttons: p.get("buttons").and_then(Value::as_u64).unwrap_or(0) as u32,
            visible: p.get("visible").and_then(Value::as_bool).unwrap_or(true),
        });
    }
    None
}

/// Reads `fd` (an inherited socket) on a thread.
///
/// # Safety
/// `fd` is an open socket this process owns and nothing else reads.
pub unsafe fn listen(fd: RawFd) -> std::io::Result<Receiver<KeyEvent>> {
    // SAFETY: the caller's.
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    let (tx, rx) = channel();
    std::thread::Builder::new().name("lumatv-ui-keys".into()).spawn(move || read_loop(stream, tx))?;
    Ok(rx)
}

fn read_loop(stream: UnixStream, tx: Sender<KeyEvent>) {
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if let Some(ev) = parse(&line)
            && tx.send(ev).is_err()
        {
            return;
        }
    }
    let _ = tx.send(KeyEvent::Closed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lines_are_keys_and_pointer_samples() {
        assert_eq!(parse(r#"{"key":{"k":"ok","d":true,"r":false,"seat":1}}"#), Some(KeyEvent::Key { name: "ok".into(), down: true, repeat: false, seat: 1 }));
        assert_eq!(parse(r#"{"key":{"k":"back","d":false}}"#), Some(KeyEvent::Key { name: "back".into(), down: false, repeat: false, seat: 0 }));
        assert_eq!(
            parse(r#"{"pointer":{"x":10.5,"y":20,"buttons":1,"visible":true}}"#),
            Some(KeyEvent::Pointer { x: 10.5, y: 20.0, buttons: 1, visible: true })
        );
        assert_eq!(parse("not json"), None);
        assert_eq!(parse(r#"{"other":1}"#), None);
    }
}
