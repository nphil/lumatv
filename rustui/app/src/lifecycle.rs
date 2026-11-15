//! The channel between a 64-bit app and the 32-bit program SAM starts for it.
//!
//! The luna-service2 library is 32-bit and the hub gives a bus name to an executable by its path, so the registration with SAM
//! (`registerApp`, then SAM's `relaunch` and `close`) is made by a small 32-bit launcher (`stub/lumatv-card.c`) that starts the app
//! and hands SAM's events on over a socket pair: one JSON object a line, in both directions, on the descriptor the launcher names
//! (`--channel-fd`). An app without a launcher (a host run) has no channel and no SAM events.

use lumagame_core::ipc::{LineBuffer, MAX_LINE_BYTES};
use serde::Deserialize;
use std::collections::VecDeque;
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
    /// A launch through the feeder's registered LS2 handle.
    Launched { seq: u64, app_id: String, success: bool, message: String },
    Foreground { app_id: String },
    Apps { seq: u64, apps: Vec<InstalledApp>, success: bool, message: String },
}

/// Commands and window reports sent to the launcher.
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
    Launch { seq: u64, app_id: String, parameters: Value },
    Apps { seq: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct InstalledApp {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub icon: Option<String>,
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
            FromApp::Launch { seq, app_id, parameters } => json!({"kind":"launch","seq":seq,"app_id":app_id,"parameters":parameters}),
            FromApp::Apps { seq } => json!({"kind":"apps","seq":seq}),
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
            "launched" => Some(ToApp::Launched { seq: v.get("seq")?.as_u64()?, app_id: text("app_id"), success: v.get("success")?.as_bool()?, message: text("message") }),
            "foreground" => Some(ToApp::Foreground { app_id: text("app_id") }),
            "apps" => Some(ToApp::Apps { seq: v.get("seq")?.as_u64()?, apps: serde_json::from_value(v.get("apps").cloned().unwrap_or_else(|| json!([]))).ok()?, success: v.get("success")?.as_bool()?, message: text("message") }),
            _ => None,
        }
    }
}

pub struct Channel {
    stream: UnixStream,
    buf: LineBuffer,
    outgoing: VecDeque<Vec<u8>>,
    offset: usize,
    queued: usize,
}

impl Channel {
    /// Takes over descriptor `fd`, which the launcher made and the app inherited.
    ///
    /// # Safety
    /// `fd` must be an open socket this process owns and nothing else closes.
    pub unsafe fn from_fd(fd: RawFd) -> io::Result<Channel> {
        // SAFETY: the caller's promise.
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        Self::from_stream(UnixStream::from(owned))
    }

    pub fn from_stream(stream: UnixStream) -> io::Result<Channel> {
        stream.set_nonblocking(true)?;
        let fd = std::os::fd::AsRawFd::as_raw_fd(&stream);
        // The inherited lifecycle socket must not keep the feeder alive in an engine child.
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 { return Err(io::Error::last_os_error()); }
        Ok(Channel { stream, buf: LineBuffer::default(), outgoing: VecDeque::new(), offset: 0, queued: 0 })
    }

    pub fn fd(&self) -> RawFd {
        std::os::fd::AsRawFd::as_raw_fd(&self.stream)
    }

    /// Every message that has arrived. `Err(UnexpectedEof)` when the launcher is gone: an app that outlives its launcher has no one
    /// to answer to and ends.
    pub fn read(&mut self) -> io::Result<Vec<ToApp>> {
        self.flush()?;
        let mut out = Vec::new();
        let mut chunk = [0u8; 4096];
        let mut eof = false;
        for _ in 0..256 {
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    eof = true;
                    break;
                }
                Ok(n) => self.buf.push(&chunk[..n], |line| out.extend(ToApp::parse(line))),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        if eof && out.is_empty() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the launcher closed the channel"));
        }
        Ok(out)
    }

    pub fn send(&mut self, msg: &FromApp) {
        let _ = self.try_send(msg);
    }

    pub fn try_send(&mut self, msg: &FromApp) -> io::Result<()> {
        let line = msg.to_line().into_bytes();
        if self.queued + line.len() > MAX_LINE_BYTES {
            return Err(io::Error::new(io::ErrorKind::WouldBlock, "the launcher's outgoing queue is full"));
        }
        self.queued += line.len();
        self.outgoing.push_back(line);
        self.flush()
    }

    pub fn pending(&self) -> bool { !self.outgoing.is_empty() }

    pub fn flush(&mut self) -> io::Result<()> {
        while let Some(line) = self.outgoing.front() {
            match self.stream.write(&line[self.offset..]) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "the launcher took no bytes")),
                Ok(n) => {
                    self.offset += n;
                    self.queued -= n;
                    if self.offset == line.len() { self.outgoing.pop_front(); self.offset = 0; }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
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
    fn native_game_lines_keep_the_request_identity() {
        assert_eq!(ToApp::parse(r#"{"kind":"launched","seq":7,"app_id":"game","success":false,"message":"not installed"}"#),
            Some(ToApp::Launched { seq: 7, app_id: "game".into(), success: false, message: "not installed".into() }));
        assert_eq!(ToApp::parse(r#"{"kind":"foreground","app_id":"game"}"#), Some(ToApp::Foreground { app_id: "game".into() }));
        let apps = ToApp::parse(r#"{"kind":"apps","seq":8,"success":true,"apps":[{"id":"game","title":"Game"}]}"#).unwrap();
        assert!(matches!(apps, ToApp::Apps { seq: 8, success: true, apps, .. } if apps.len() == 1 && apps[0].icon.is_none()));
        let launch: Value = serde_json::from_str(&FromApp::Launch { seq: 9, app_id: "game".into(), parameters: json!({"a":1}) }.to_line()).unwrap();
        assert_eq!(launch["parameters"]["a"], 1);
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
