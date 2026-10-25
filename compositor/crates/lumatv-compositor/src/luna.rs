//! The compositor's end of luna: the contract with the 32-bit luna bridge (`docs/research/compositor-c2-result.md`
//! §5) and what the compositor says over it.
//!
//! A 64-bit process cannot load LG's 32-bit `libluna-service2`, so the bridge is a separate program linked
//! against it, at the executable path the hub's role files expect. Messages cross a Unix stream socket as JSON
//! lines. C2 has no bridge running (LG's compositor owns every bus name); the [`Null`] bridge records what would
//! have been sent, so a TV session shows the calls an app provoked.

use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A luna call that arrived at one of the compositor's bus names.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Inbound {
    pub token: u64,
    /// The bus name it was addressed to (`com.webos.surfacemanager`, `com.webos.service.recent`, …).
    pub service: String,
    /// The method without the leading slash (`getForegroundAppInfo`).
    pub method: String,
    pub payload: Value,
    pub subscribe: bool,
    /// The caller's bus name, for the per-caller behaviour LG's QML has.
    pub sender: String,
    /// `true` when a subscriber went away: the token is dead.
    #[serde(default)]
    pub cancelled: bool,
}

/// A reply to an [`Inbound`]; a subscription sends many with the same token.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Reply {
    pub token: u64,
    pub payload: Value,
}

/// A call the compositor makes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Outbound {
    pub token: u64,
    /// `luna://com.webos.service.videooutput/display/setDisplayWindow`
    pub uri: String,
    pub payload: Value,
    pub subscribe: bool,
}

/// The answer to an [`Outbound`] (one per call, many per subscription).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub token: u64,
    pub payload: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FromBridge {
    Inbound(Inbound),
    Event(Event),
    /// The bridge's heartbeat; the compositor answers every one (C4's watchdog).
    Ping { seq: u64 },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToBridge {
    Reply(Reply),
    Outbound(Outbound),
    Pong { seq: u64 },
}

/// The bus names the compositor owns through the bridge.
pub const BUS_NAMES: &[&str] = &[
    "com.webos.surfacemanager",
    "com.webos.surfacemanager.home",
    "com.webos.surfacemanager.input",
    "com.webos.surfacemanager.systemui",
    "com.webos.surfacemanager.extended",
    "com.webos.surfacemanager.hcap",
    "com.webos.surfacemanager.screenSaver",
    "com.webos.surfacemanager.windowext",
    "com.webos.service.recent",
    "com.webos.service.systemUIManager",
    "com.webos.service.sysuicompmgr",
    "com.webos.coverUI",
    "com.webos.keyfilter.magicnumber",
];

pub trait Bridge {
    fn send(&mut self, msg: ToBridge);
    /// Messages that arrived since the last call.
    fn poll(&mut self) -> Vec<FromBridge>;
    fn is_connected(&self) -> bool;
    /// The outbound calls kept for inspection (the null bridge keeps the last 256).
    fn sent(&self) -> Vec<(String, Value)> {
        Vec::new()
    }
    /// Everything sent, for tests.
    fn history(&self) -> Vec<ToBridge> {
        Vec::new()
    }
}

/// No bridge: outbound calls are kept (bounded) for the control socket to show.
#[derive(Default)]
pub struct Null {
    pub sent: VecDeque<ToBridge>,
}

impl Bridge for Null {
    fn send(&mut self, msg: ToBridge) {
        if let ToBridge::Outbound(o) = &msg {
            log::info!("luna (no bridge): {} {}", o.uri, o.payload);
        }
        if self.sent.len() >= 256 {
            self.sent.pop_front();
        }
        self.sent.push_back(msg);
    }

    fn poll(&mut self) -> Vec<FromBridge> {
        Vec::new()
    }

    fn is_connected(&self) -> bool {
        false
    }

    fn sent(&self) -> Vec<(String, Value)> {
        self.sent.iter().filter_map(|m| match m { ToBridge::Outbound(o) => Some((o.uri.clone(), o.payload.clone())), _ => None }).collect()
    }

    fn history(&self) -> Vec<ToBridge> {
        self.sent.iter().cloned().collect()
    }
}

/// A connected bridge over a Unix stream socket, JSON lines both ways.
pub struct Pipe {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    line: String,
}

impl Pipe {
    pub fn connect(path: &str) -> io::Result<Pipe> {
        let stream = UnixStream::connect(path)?;
        stream.set_nonblocking(true)?;
        Ok(Pipe { reader: BufReader::new(stream.try_clone()?), writer: stream, line: String::new() })
    }
}

impl Bridge for Pipe {
    fn send(&mut self, msg: ToBridge) {
        let mut s = serde_json::to_string(&msg).unwrap_or_default();
        s.push('\n');
        if let Err(e) = self.writer.write_all(s.as_bytes()) {
            log::warn!("luna bridge write: {e}");
        }
    }

    fn poll(&mut self) -> Vec<FromBridge> {
        let mut out = Vec::new();
        loop {
            self.line.clear();
            match self.reader.read_line(&mut self.line) {
                Ok(0) => break,
                Ok(_) => match serde_json::from_str::<FromBridge>(self.line.trim_end()) {
                    Ok(m) => out.push(m),
                    Err(e) => log::warn!("luna bridge: bad line {:?}: {e}", self.line),
                },
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    log::warn!("luna bridge read: {e}");
                    break;
                }
            }
        }
        out
    }

    fn is_connected(&self) -> bool {
        true
    }
}

/// Video placement, as LG's compositor calls videooutputd (`StarfishViewStateController.qml`).
pub mod video {
    use serde_json::{Value, json};

    use crate::Rect;

    fn rect(r: &Rect) -> Value {
        json!({"x": r.x, "y": r.y, "width": r.w, "height": r.h})
    }

    pub fn set_display_window(context: &str, source: Option<&Rect>, destination: &Rect, app_output: Option<&Rect>) -> (String, Value) {
        let mut p = json!({"displayOutput": rect(destination), "context": context, "fullScreen": false});
        if let Some(s) = source {
            p["sourceInput"] = rect(s);
        }
        if let Some(a) = app_output.filter(|a| a.w != 0 && a.h != 0) {
            p["appOutput"] = rect(a);
        }
        ("luna://com.webos.service.videooutput/display/setDisplayWindow".into(), p)
    }

    pub fn set_crop_region(context: &str, original: &Rect, source: &Rect, destination: &Rect, app_output: Option<&Rect>) -> (String, Value) {
        let mut p = json!({"originalInput": rect(original), "sourceInput": rect(source), "displayOutput": rect(destination), "context": context, "fullScreen": false});
        if let Some(a) = app_output.filter(|a| a.w != 0 && a.h != 0) {
            p["appOutput"] = rect(a);
        }
        ("luna://com.webos.service.videooutput/display/setDisplayWindow".into(), p)
    }

    pub fn set_property(context: &str, name: &str, value: &str) -> (String, Value) {
        match name {
            "mute" => ("luna://com.webos.service.avmutemanager/setAvMute".into(), json!({"contextId": context, "muteType": "videoMute", "muteOn": value == "on"})),
            "registerMute" => (
                format!("luna://com.webos.service.avmutemanager/{}", if value == "on" { "register" } else { "unregister" }),
                json!({"contextId": context}),
            ),
            "zoomHPosition" | "zoomVPosition" | "zoomHRatio" | "zoomVRatio" => {
                let v: i64 = value.parse().unwrap_or(0);
                ("luna://com.webos.service.videooutput/display/setDisplayWindowProperty".into(), json!({"context": context, "zoomData": {name: v}}))
            }
            "autoMenuControl" => ("luna://com.webos.service.videooutput/display/setDisplayWindowProperty".into(), json!({"context": context, name: value == "on"})),
            _ => ("luna://com.webos.service.videooutput/display/setDisplayWindowProperty".into(), json!({"context": context, name: value})),
        }
    }

    pub fn set_compositing(context: &str, z_order: i32) -> (String, Value) {
        ("luna://com.webos.service.videooutput/display/setCompositing".into(), json!({"composeOrder": {"context": context, "zOrder": z_order}}))
    }
}

/// The tvpower handshake (`StarfishPowerService.qml`), as the calls the compositor will make through the bridge.
pub mod power {
    use serde_json::{Value, json};

    pub const CLIENT_NAME: &str = "com.webos.surfacemanager";
    pub const SERVICE: &str = "com.webos.service.tvpower";

    pub fn register_calls() -> Vec<(String, Value)> {
        ["registerPreparePowerOff", "registerPreparePowerOn", "registerPowerOffLogoRequest"]
            .iter()
            .map(|m| (format!("luna://{SERVICE}/power2/{m}"), json!({"clientName": CLIENT_NAME, "subscribe": true})))
            .chain(std::iter::once((format!("luna://{SERVICE}/power2/getPowerState"), json!({"subscribe": true}))))
            .collect()
    }

    pub fn response(round: &str, timestamp: &Value, ack: bool) -> (String, Value) {
        (format!("luna://{SERVICE}/power2/response{round}"), json!({"clientName": CLIENT_NAME, "timestamp": timestamp, "ack": ack}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rect;

    #[test]
    fn placement_payloads_match_lg() {
        let (uri, p) = video::set_display_window("_ctx", None, &Rect::new(160, 0, 960, 540), None);
        assert!(uri.ends_with("/display/setDisplayWindow"));
        assert_eq!(p["displayOutput"]["width"], 960);
        assert_eq!(p["fullScreen"], false);
        let (uri, p) = video::set_property("_ctx", "mute", "on");
        assert!(uri.contains("avmutemanager/setAvMute"));
        assert_eq!(p["muteOn"], true);
        let (_, p) = video::set_property("_ctx", "fastMode", "on");
        assert_eq!(p["fastMode"], "on");
    }

    #[test]
    fn messages_round_trip_as_json_lines() {
        let m = ToBridge::Outbound(Outbound { token: 7, uri: "luna://x/y".into(), payload: serde_json::json!({"a": 1}), subscribe: false });
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains("\"kind\":\"outbound\""));
        let back: FromBridge = serde_json::from_str(r#"{"kind":"inbound","token":1,"service":"com.webos.surfacemanager","method":"getForegroundAppInfo","payload":{"subscribe":true},"subscribe":true,"sender":"com.webos.applicationManager"}"#).unwrap();
        match back {
            FromBridge::Inbound(i) => assert_eq!(i.method, "getForegroundAppInfo"),
            _ => panic!(),
        }
        assert_eq!(power::register_calls().len(), 4);
    }
}
