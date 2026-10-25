//! The channel between a 64-bit app and the 32-bit program SAM starts for it.
//!
//! The luna-service2 library is 32-bit and the hub gives a bus name to an executable by its path, so the registration with SAM
//! (`registerApp`, then SAM's `relaunch` and `close`) is made by a small 32-bit launcher (`stub/lumatv-card.c`) that starts the app
//! and hands SAM's events on over a socket pair: one JSON object a line, in both directions, on the descriptor the launcher names
//! (`--channel-fd`). An app without a launcher (a host run) has no channel and no SAM events.

use serde_json::{Value, json};
use std::io::{self, Read, Write};
use std::os::fd::{FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;

/// What SAM said, as the launcher passes it on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToApp {
    /// SAM launched the app again while it runs (`relaunch`): `parameters` is the raw JSON object of the launch parameters.
    Relaunch { reason: String, parameters: String },
    /// SAM asks the app to end (`close`): `reason` is `undefined`, `memoryReclaim`, `powerOff`, ...
    Close { reason: String },
    /// The launcher's own news: whether SAM accepted the registration. The app runs either way.
    Sam { registered: bool },
}

/// What the app tells the launcher, for its log: nothing here changes what the launcher does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FromApp {
    /// The first frame is on its way to the compositor.
    Ready,
    /// The compositor's word on the window (`wl_webos_shell_surface.state`): 0 default, 1 minimized, 2 maximized, 3 fullscreen.
    State(u32),
    /// The page the app shows.
    Page(String),
    /// The app is ending, and why.
    Exit(String),
}

pub fn state_name(state: u32) -> &'static str {
    match state {
        1 => "minimized",
        2 => "maximized",
        3 => "fullscreen",
        _ => "default",
    }
}

impl FromApp {
    pub fn to_line(&self) -> String {
        let v = match self {
            FromApp::Ready => json!({"kind": "ready"}),
            FromApp::State(s) => json!({"kind": "state", "state": state_name(*s)}),
            FromApp::Page(p) => json!({"kind": "page", "page": p}),
            FromApp::Exit(why) => json!({"kind": "exit", "why": why}),
        };
        let mut line = v.to_string();
        line.push('\n');
        line
    }
}

impl ToApp {
    /// One line of the launcher's. `None` for a line that is not one of its messages.
    pub fn parse(line: &str) -> Option<ToApp> {
        let v: Value = serde_json::from_str(line.trim()).ok()?;
        let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        match v.get("kind")?.as_str()? {
            "relaunch" => {
                Some(ToApp::Relaunch { reason: text("reason"), parameters: v.get("parameters").map(Value::to_string).unwrap_or_else(|| "{}".into()) })
            }
            "close" => Some(ToApp::Close { reason: text("reason") }),
            "sam" => Some(ToApp::Sam { registered: v.get("registered").and_then(Value::as_bool).unwrap_or(false) }),
            _ => None,
        }
    }
}

pub struct Channel {
    stream: UnixStream,
    buf: Vec<u8>,
}

impl Channel {
    /// Takes over descriptor `fd`, which the launcher made and the app inherited.
    ///
    /// # Safety
    /// `fd` must be an open socket this process owns and nothing else closes.
    pub unsafe fn from_fd(fd: RawFd) -> io::Result<Channel> {
        // SAFETY: the caller's promise.
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        let stream = UnixStream::from(owned);
        stream.set_nonblocking(true)?;
        Ok(Channel { stream, buf: Vec::new() })
    }

    pub fn from_stream(stream: UnixStream) -> io::Result<Channel> {
        stream.set_nonblocking(true)?;
        Ok(Channel { stream, buf: Vec::new() })
    }

    pub fn fd(&self) -> RawFd {
        std::os::fd::AsRawFd::as_raw_fd(&self.stream)
    }

    /// Every message that has arrived. `Err(UnexpectedEof)` when the launcher is gone: an app that outlives its launcher has no one
    /// to answer to and ends.
    pub fn read(&mut self) -> io::Result<Vec<ToApp>> {
        let mut chunk = [0u8; 4096];
        let mut eof = false;
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    eof = true;
                    break;
                }
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        let mut out = Vec::new();
        while let Some(at) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=at).collect();
            if let Some(msg) = ToApp::parse(&String::from_utf8_lossy(&line)) {
                out.push(msg);
            }
        }
        if eof && out.is_empty() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the launcher closed the channel"));
        }
        Ok(out)
    }

    /// One message; a launcher that does not read it in time (a full socket) loses it rather than holding the app up.
    pub fn send(&mut self, msg: &FromApp) {
        let line = msg.to_line();
        let _ = self.stream.write_all(line.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_sam_says_is_understood() {
        assert_eq!(
            ToApp::parse(r#"{"kind":"relaunch","reason":"undefined","parameters":{"launchType":"homeKey"}}"#),
            Some(ToApp::Relaunch { reason: "undefined".into(), parameters: r#"{"launchType":"homeKey"}"#.into() })
        );
        assert_eq!(ToApp::parse(r#"{"kind":"relaunch"}"#), Some(ToApp::Relaunch { reason: String::new(), parameters: "{}".into() }));
        assert_eq!(ToApp::parse(r#"{"kind":"close","reason":"memoryReclaim"}"#), Some(ToApp::Close { reason: "memoryReclaim".into() }));
        assert_eq!(ToApp::parse(r#"{"kind":"sam","registered":true}"#), Some(ToApp::Sam { registered: true }));
        assert_eq!(ToApp::parse(r#"{"kind":"nonsense"}"#), None);
        assert_eq!(ToApp::parse("not json"), None);
    }

    #[test]
    fn what_the_app_says_is_one_line_each() {
        assert_eq!(FromApp::Ready.to_line(), "{\"kind\":\"ready\"}\n");
        assert_eq!(FromApp::State(1).to_line(), "{\"kind\":\"state\",\"state\":\"minimized\"}\n");
        assert_eq!(FromApp::Page("keys".into()).to_line(), "{\"kind\":\"page\",\"page\":\"keys\"}\n");
        assert_eq!(FromApp::Exit("Back at the root".into()).to_line(), "{\"kind\":\"exit\",\"why\":\"Back at the root\"}\n");
    }

    #[test]
    fn the_channel_reads_lines_and_notices_the_end() {
        let (a, b) = UnixStream::pair().unwrap();
        let mut app = Channel::from_stream(b).unwrap();
        let mut launcher = a;
        launcher.write_all(b"{\"kind\":\"close\",\"reason\":\"undefined\"}\n{\"kind\":\"sam\",\"regis").unwrap();
        assert_eq!(app.read().unwrap(), vec![ToApp::Close { reason: "undefined".into() }]);
        launcher.write_all(b"tered\":false}\n").unwrap();
        assert_eq!(app.read().unwrap(), vec![ToApp::Sam { registered: false }]);
        assert!(app.read().unwrap().is_empty());
        app.send(&FromApp::Ready);
        let mut got = [0u8; 64];
        launcher.set_nonblocking(true).unwrap();
        let n = launcher.read(&mut got).unwrap();
        assert_eq!(&got[..n], b"{\"kind\":\"ready\"}\n");
        drop(launcher);
        assert_eq!(app.read().unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }
}
