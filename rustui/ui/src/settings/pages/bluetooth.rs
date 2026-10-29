//! Bluetooth (`settings/PageBluetooth.qml`, `PageBtAudio.qml`, `PageBtHid.qml`, `PageBtDevice.qml`): the devices the TV knows, the ones it can
//! see, and a search. `Kind` says which of LG's two services answers: audio is `btaudiosrc` (speakers and headphones: getPairedDevices,
//! startDiscovery, connect) and hid is `bthidmanager` (keyboards, mice, game pads: getPairedDeviceLists, requestDiscovery, requestConnect).
//! Both are read as LG's own screens read them and shown the same way: registered devices first (OK opens the device), then connectable
//! ones (OK pairs and connects), then Search. Searching stops when the page is left, as LG's does. Connecting and deleting are done at the
//! remote only; nothing of this is offered to an assistant. The page's own state (the devices, the search, the one being connected) is kept
//! in the shell's `bt.*` keys, written by the answers and cleared when the page opens.

use crate::settings::ctx::{Ctx, Spec, ask};
use crate::settings::names;
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, ChoiceSpec, Op, PageArg, Pick, Row};
use serde_json::{Value, json};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Audio,
    Hid,
}

impl Kind {
    fn svc(self) -> &'static str {
        match self {
            Kind::Audio => "luna://com.webos.service.btaudiosrc",
            Kind::Hid => "luna://com.webos.service.bthidmanager",
        }
    }

    fn uri(self, method: &str) -> String {
        format!("{}/{method}", self.svc())
    }

    /// The method that lists the paired devices.
    fn list(self) -> &'static str {
        match self {
            Kind::Audio => "getPairedDevices",
            Kind::Hid => "getPairedDeviceLists",
        }
    }

    /// The names the answers are kept under: the paired list, the state changes, the search, the answer to a connect.
    fn paired(self) -> &'static str {
        match self {
            Kind::Audio => "btpaired",
            Kind::Hid => "hidpaired",
        }
    }

    fn status(self) -> &'static str {
        match self {
            Kind::Audio => "btstatus",
            Kind::Hid => "hidstatus",
        }
    }

    fn found(self) -> &'static str {
        match self {
            Kind::Audio => "btfound",
            Kind::Hid => "hidfound",
        }
    }

    fn connect(self) -> &'static str {
        match self {
            Kind::Audio => "btconnect",
            Kind::Hid => "hidconnect",
        }
    }
}

const AUDIO_LIST: &str = "luna://com.webos.service.btaudiosrc/getPairedDevices";
const HID_LIST: &str = "luna://com.webos.service.bthidmanager/getPairedDeviceLists";

pub fn audio() -> Page {
    Page::new("btaudio", "Bluetooth Speakers", "headphones")
        .parent("sound")
        .back("Sound")
        .reads(audio_reads)
        .rows(audio_rows)
        .answered(audio_answered)
        .opened(opened)
        .closed(audio_closed)
}

pub fn hid() -> Page {
    Page::new("bthid", "Bluetooth Devices", "gamepad-2")
        .parent("inputs")
        .back("Inputs & devices")
        .reads(hid_reads)
        .rows(hid_rows)
        .answered(hid_answered)
        .opened(opened)
        .closed(hid_closed)
}

pub fn audio_device() -> Page {
    Page::new("btdevice", "Bluetooth device", "headphones")
        .parent("sound")
        .back("Bluetooth Speakers")
        .hidden_from_search()
        .reads(audio_device_reads)
        .rows(audio_device_rows)
        .title_of(device_title)
        .answered(audio_device_answered)
        .opened(device_opened)
}

pub fn hid_device() -> Page {
    Page::new("bthiddevice", "Bluetooth device", "gamepad-2")
        .parent("inputs")
        .back("Bluetooth Devices")
        .hidden_from_search()
        .rows(hid_device_rows)
        .title_of(device_title)
        .opened(device_opened)
}

// ------------------------------------------------------------------------------------------------------------------------ state

/// A device of the paired list, as the rows and the device page read it.
#[derive(Clone, Debug, PartialEq)]
pub struct Dev {
    pub address: String,
    pub name: String,
    pub state: String,
    pub connected: bool,
    pub surround: String,
    pub avrcp: bool,
}

impl Dev {
    fn json(&self) -> Value {
        json!({"address": self.address, "name": self.name, "state": self.state, "connected": self.connected, "surround": self.surround, "avrcp": self.avrcp})
    }

    fn of(v: &Value) -> Option<Dev> {
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        Some(Dev {
            address: text("address")?,
            name: text("name").unwrap_or_default(),
            state: text("state").unwrap_or_default(),
            connected: v.get("connected").and_then(Value::as_bool).unwrap_or(false),
            surround: text("surround").unwrap_or_else(|| "none".into()),
            avrcp: v.get("avrcp").and_then(Value::as_bool).unwrap_or(false),
        })
    }
}

/// The paired devices out of the service's answer; `None` for an answer that says it failed (the list stays as it was).
fn parse_paired(kind: Kind, doc: &Value) -> Option<Vec<Dev>> {
    if doc.get("returnValue").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let mut out = Vec::new();
    match kind {
        Kind::Audio => {
            for d in doc.get("devices").and_then(Value::as_array).into_iter().flatten() {
                if d.get("display").and_then(Value::as_bool) == Some(false) {
                    continue;
                }
                let state = d.get("state").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("disconnected").to_string();
                out.push(Dev {
                    address: d.get("address").and_then(Value::as_str).unwrap_or("").to_string(),
                    name: d.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                    connected: state == "connected",
                    state,
                    surround: d.get("surround").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("none").to_string(),
                    avrcp: d.get("avrcpSupported").and_then(Value::as_bool) == Some(true) && d.get("avrcpConnected").and_then(Value::as_bool) == Some(true),
                });
            }
        }
        Kind::Hid => {
            // the service answers one entry for each of its slots: an empty one is "Not Paired" with the address "N/A"
            let mut slots: Vec<(&String, &Value)> = doc.get("deviceLists").and_then(Value::as_object).map(|m| m.iter().collect()).unwrap_or_default();
            slots.sort_by(|a, b| a.0.cmp(b.0));
            for (_, e) in slots {
                let addr = e.get("deviceAddr").and_then(Value::as_str).unwrap_or("");
                if e.get("isPaired").and_then(Value::as_bool) == Some(false) || addr.is_empty() || addr == "N/A" {
                    continue;
                }
                let up = e.get("isConnected").and_then(Value::as_bool).unwrap_or(false);
                out.push(Dev {
                    address: addr.to_string(),
                    name: e.get("deviceName").and_then(Value::as_str).unwrap_or("").to_string(),
                    state: if up { "connected" } else { "disconnected" }.into(),
                    connected: up,
                    surround: "none".into(),
                    avrcp: false,
                });
            }
        }
    }
    Some(out)
}

fn paired_key(kind: Kind) -> &'static str {
    match kind {
        Kind::Audio => "bt.paired.audio",
        Kind::Hid => "bt.paired.hid",
    }
}

fn paired(c: &Ctx, kind: Kind) -> Vec<Dev> {
    c.local.get(paired_key(kind)).as_array().map(|l| l.iter().filter_map(Dev::of).collect()).unwrap_or_default()
}

fn found(c: &Ctx) -> Vec<(String, String)> {
    c.local
        .get("bt.found")
        .as_array()
        .map(|l| {
            l.iter().filter_map(|d| Some((d.get("address")?.as_str()?.to_string(), d.get("name").and_then(Value::as_str).unwrap_or("").to_string()))).collect()
        })
        .unwrap_or_default()
}

/// The address being connected, until the list says it is.
fn busy(c: &Ctx, kind: Kind) -> String {
    let b = c.local.text("bt.busy");
    if !b.is_empty() && paired(c, kind).iter().any(|d| d.address == b && d.connected) { String::new() } else { b }
}

fn searching(c: &Ctx) -> bool {
    c.local.flag("bt.searching")
}

/// LG's Bluetooth Speaker Volume (0 to 10) and Sound Balance (0 left to 10 right, 5 the middle): -1 until the service says.
fn level(c: &Ctx, key: &str) -> f64 {
    c.local.get(key).as_f64().unwrap_or(-1.0)
}

/// LG's Sound Balance is for a pair of the same speaker (two connected with the same model name, which is what comes before "(" or "_")
/// that both answer over AVRCP; the TV's name for each is "Model (L)" or "Model_L".
pub fn paired_speakers(devs: &[Dev]) -> bool {
    let mut seen: Vec<&str> = Vec::new();
    for d in devs.iter().filter(|d| d.connected && d.avrcp) {
        let model = if d.name.contains('(') { d.name.split('(').next() } else { d.name.split('_').next() }.unwrap_or("");
        if seen.contains(&model) {
            return true;
        }
        seen.push(model);
    }
    false
}

fn balance_text(v: f64) -> String {
    if v < 0.0 {
        "\u{2013}".into()
    } else if v < 5.0 {
        format!("Left {}", (5.0 - v) as i64)
    } else if v > 5.0 {
        format!("Right {}", (v - 5.0) as i64)
    } else {
        "0".into()
    }
}

fn state_text(d: &Dev, busy: &str) -> String {
    let pos = if d.state == "connected" && (d.surround == "left" || d.surround == "right") {
        names::name(names::SPEAKER_POSITION, &d.surround)
    } else {
        String::new()
    };
    let t = if busy == d.address {
        "Connecting"
    } else {
        match d.state.as_str() {
            "connected" => "Connected",
            "connecting" => "Connecting",
            "disconnecting" => "Disconnecting",
            _ => "",
        }
    };
    if !pos.is_empty() && t == "Connected" { format!("{t} \u{b7} {pos}") } else { t.to_string() }
}

// -------------------------------------------------------------------------------------------------------------------- the list

fn reads_of(kind: Kind) -> Vec<Read> {
    let mut r = vec![Read::lg("sound", &["deviceCntOfBtGeneral", "bluetoothMode", "hearingAid"])];
    match kind {
        Kind::Audio => {
            r.push(Read::Watch { name: "btpaired", uri: AUDIO_LIST, payload: json!({}) });
            r.push(Read::Watch { name: "btstatus", uri: "luna://com.webos.service.btaudiosrc/getDeviceStatus", payload: json!({}) });
            r.push(Read::Watch { name: "btbalance", uri: "luna://com.webos.service.btaudiosrc/surround/getVolumeBalancing".into(), payload: json!({}) });
        }
        Kind::Hid => {
            r.push(Read::Call { name: "hidpaired", uri: HID_LIST, payload: json!({}) });
            r.push(Read::Watch { name: "hidstatus", uri: "luna://com.webos.service.bthidmanager/subscribeDevicesStatus", payload: json!({}) });
        }
    }
    r
}

fn audio_reads(_: &Ctx) -> Vec<Read> {
    reads_of(Kind::Audio)
}

fn hid_reads(_: &Ctx) -> Vec<Read> {
    reads_of(Kind::Hid)
}

/// The page is brought up: nothing found, nothing searching, nothing being connected (a search and a subscription are for the page that is up).
fn opened(_: &Ctx) -> Vec<Op> {
    vec![Op::Set("bt.found".into(), json!([])), Op::Set("bt.searching".into(), json!(false)), Op::Set("bt.busy".into(), json!(""))]
}

fn closed_of(kind: Kind, c: &Ctx) -> Vec<Op> {
    let mut ops = Vec::new();
    if searching(c) && kind == Kind::Hid {
        ops.push(Op::Act { uri: kind.uri("cancelDiscovery"), payload: json!({}) });
    }
    for name in [kind.paired(), kind.status(), kind.found(), "btbalance"] {
        ops.push(Op::Close(name.into()));
    }
    ops.push(Op::Set("bt.searching".into(), json!(false)));
    ops
}

fn audio_closed(c: &Ctx) -> Vec<Op> {
    closed_of(Kind::Audio, c)
}

fn hid_closed(c: &Ctx) -> Vec<Op> {
    closed_of(Kind::Hid, c)
}

fn search_ops(kind: Kind) -> Vec<Op> {
    let mut ops = vec![Op::Set("bt.found".into(), json!([])), Op::Set("bt.searching".into(), json!(true))];
    match kind {
        Kind::Audio => ops.push(Op::Call { name: kind.found().into(), uri: kind.uri("startDiscovery"), payload: json!({}), subscribe: true }),
        Kind::Hid => {
            ops.push(Op::Call { name: kind.found().into(), uri: kind.uri("subscribeDiscovery"), payload: json!({}), subscribe: true });
            ops.push(Op::Call { name: "hidgo".into(), uri: kind.uri("requestDiscovery"), payload: json!({}), subscribe: false });
        }
    }
    ops
}

fn stop_ops(kind: Kind) -> Vec<Op> {
    let mut ops = vec![Op::Set("bt.searching".into(), json!(false)), Op::Close(kind.found().into())];
    if kind == Kind::Hid {
        ops.push(Op::Act { uri: kind.uri("cancelDiscovery"), payload: json!({}) });
    }
    ops
}

fn connect_ops(kind: Kind, address: &str, name: &str, found: &[(String, String)]) -> Vec<Op> {
    let payload = match kind {
        Kind::Audio => json!({"address": address}),
        Kind::Hid => json!({"name": name, "address": address}),
    };
    let method = if kind == Kind::Audio { "connect" } else { "requestConnect" };
    let rest: Vec<Value> = found.iter().filter(|(a, _)| a != address).map(|(a, n)| json!({"address": a, "name": n})).collect();
    vec![
        Op::Set("bt.busy".into(), json!(address)),
        Op::Call { name: kind.connect().into(), uri: kind.uri(method), payload, subscribe: false },
        Op::Set("bt.found".into(), Value::Array(rest)),
    ]
}

fn reread(kind: Kind) -> Op {
    Op::Call { name: kind.paired().into(), uri: kind.uri(kind.list()), payload: json!({}), subscribe: false }
}

/// What the page does with an answer of the service.
fn answered_of(kind: Kind, c: &Ctx, name: &str, doc: &Value) -> Vec<Op> {
    if doc.get("returnValue").and_then(Value::as_bool) == Some(false) {
        return Vec::new();
    }
    if name == kind.paired() {
        let Some(list) = parse_paired(kind, doc) else { return Vec::new() };
        let mut ops = vec![Op::Set(paired_key(kind).into(), Value::Array(list.iter().map(Dev::json).collect()))];
        let b = c.local.text("bt.busy");
        if list.iter().any(|d| d.address == b && d.connected) {
            ops.push(Op::Set("bt.busy".into(), json!("")));
        }
        ops
    } else if name == kind.status() || name == kind.connect() {
        // a change of state (or the answer to a connect): read the list again; a connection that failed or ended is no longer pending
        let mut ops = Vec::new();
        if name == kind.status() && matches!(doc.get("status").and_then(Value::as_str), Some("fail" | "disconnected")) {
            ops.push(Op::Set("bt.busy".into(), json!("")));
        }
        ops.push(reread(kind));
        ops
    } else if name == "btbalance" && kind == Kind::Audio {
        let mut ops = Vec::new();
        if let Some(v) = doc.get("tvspeaker_bluetooth").and_then(crate::host::sio::js::float) {
            ops.push(Op::Set("bt.volume".into(), json!(v)));
        }
        if let Some(v) = doc.get("bluetooth_left_right").and_then(crate::host::sio::js::float) {
            ops.push(Op::Set("bt.balance".into(), json!(v)));
        }
        ops
    } else if name == kind.found() {
        let dev = match kind {
            Kind::Audio => {
                if doc.get("state").and_then(Value::as_str) == Some("stopped") {
                    return vec![Op::Set("bt.searching".into(), json!(false))];
                }
                if doc.get("state").and_then(Value::as_str) == Some("started") { doc.get("device") } else { None }
            }
            Kind::Hid => match doc.get("status").and_then(Value::as_str) {
                Some("endDiscovery") => return vec![Op::Set("bt.searching".into(), json!(false))],
                Some("startDiscovery") => return vec![Op::Set("bt.searching".into(), json!(true))],
                _ => doc.get("address").is_some().then_some(doc),
            },
        };
        let Some(address) = dev.and_then(|d| d.get("address")).and_then(Value::as_str).filter(|a| !a.is_empty()) else { return Vec::new() };
        let mut list = found(c);
        if paired(c, kind).iter().any(|d| d.address == address) || list.iter().any(|(a, _)| a == address) {
            return Vec::new();
        }
        let name = dev.and_then(|d| d.get("name")).and_then(Value::as_str).filter(|n| !n.is_empty()).unwrap_or(address);
        list.push((address.to_string(), name.to_string()));
        vec![Op::Set("bt.found".into(), Value::Array(list.iter().map(|(a, n)| json!({"address": a, "name": n})).collect()))]
    } else {
        Vec::new()
    }
}

fn audio_answered(c: &Ctx, name: &str, doc: &Value) -> Vec<Op> {
    answered_of(Kind::Audio, c, name, doc)
}

fn hid_answered(c: &Ctx, name: &str, doc: &Value) -> Vec<Op> {
    answered_of(Kind::Hid, c, name, doc)
}

fn list_rows(c: &Ctx, kind: Kind) -> Vec<Row> {
    let devs = paired(c, kind);
    let new = found(c);
    let busy = busy(c, kind);
    let icon = if kind == Kind::Audio { "headphones" } else { "gamepad-2" };
    let mut rows = Vec::new();
    if kind == Kind::Audio {
        let connected = devs.iter().any(|d| d.connected);
        let aid = c.lg.is("sound.hearingAid", "bluetooth");
        rows.push(c.choice(Spec::new("sound.deviceCntOfBtGeneral").icon("headphones").label("Speaker Connection").names(names::DEVICE_CNT)));
        // LG's Bluetooth + TV Speaker rows: the playback option waits for a connected speaker (and is not for a hearing aid), the volume is for a connected one
        const MODES: names::Table = &[("surroundMode", "Surround Effect"), ("speakerMode", "Simultaneous Sound Out")];
        let mut mode = c.choice(
            Spec::new("sound.bluetoothMode")
                .icon("speaker")
                .label("Playback Option")
                .names(MODES)
                .help("How the TV's speakers and the Bluetooth speaker play together."),
        );
        mode.enabled = c.lg.usable("sound.bluetoothMode") && connected && !aid;
        mode.detail = if !c.lg.has("sound.bluetoothMode") {
            "TV not answering".into()
        } else if !connected {
            "No speaker connected".into()
        } else if aid {
            "Hearing aid in use".into()
        } else {
            String::new()
        };
        rows.push(mode);
        if connected && !aid {
            let v = level(c, "bt.volume");
            let mut r = Row::slider("btvolume", 0.0, 10.0, 1.0)
                .icon("volume-2")
                .label("Bluetooth Speaker Volume")
                .help("How loud the Bluetooth speaker plays next to the TV's speakers.")
                .admin();
            r.num = v.max(0.0);
            r.value_text = Some(if v < 0.0 { "\u{2013}".into() } else { format!("{}", v as i64) });
            r.enabled = v >= 0.0;
            rows.push(r.act(|a| match a {
                Arg::Value(v) => vec![Op::Set("bt.volume".into(), json!(v)), Op::Act { uri: SURROUND.into(), payload: json!({"tvspeaker_bluetooth": v}) }],
                _ => Vec::new(),
            }));
        }
        if paired_speakers(&devs) {
            let v = level(c, "bt.balance");
            let mut r = Row::slider("btbalance", 0.0, 10.0, 1.0)
                .icon("audio-lines")
                .label("Sound Balance")
                .help("Which of the two matching speakers plays louder.")
                .admin();
            r.num = if v < 0.0 { 5.0 } else { v };
            r.value_text = Some(balance_text(v));
            r.enabled = v >= 0.0;
            rows.push(r.act(|a| match a {
                Arg::Value(v) => vec![Op::Set("bt.balance".into(), json!(v)), Op::Act { uri: SURROUND.into(), payload: json!({"bluetooth_left_right": v}) }],
                _ => Vec::new(),
            }));
        }
    }
    rows.push(Row::head(&format!("Registered Devices ({})", devs.len()), 0));
    if devs.is_empty() {
        rows.push(Row::note("There are no paired devices.", 0));
    }
    for d in &devs {
        let arg = PageArg::Json(
            json!({"kind": if kind == Kind::Audio { "audio" } else { "hid" }, "address": d.address, "name": d.name, "connected": d.connected, "surround": d.surround}),
        );
        let page = if kind == Kind::Audio { "btdevice" } else { "bthiddevice" };
        rows.push(
            Row::link(&format!("dev:{}", d.address)).icon(icon).label(&d.name).value(&state_text(d, &busy)).does(vec![Op::Open { page: page.into(), arg }]),
        );
    }
    rows.push(Row::head(&format!("Connectable Devices ({})", new.len()), 0));
    for (address, name) in &new {
        rows.push(
            Row::link(&format!("found:{address}"))
                .icon(icon)
                .label(name)
                .value(if busy == *address { "Connecting" } else { "" })
                .chevron(false)
                .admin()
                .does(connect_ops(kind, address, name, &new)),
        );
    }
    let on = searching(c);
    let mut search = Row::action("search", if on { "Stop" } else { "Search" }).icon("search").label(if on { "Searching" } else { "Search" }).admin();
    search.act = Some(Rc::new(move |_| if on { stop_ops(kind) } else { search_ops(kind) }));
    rows.push(search);
    rows
}

const SURROUND: &str = "luna://com.webos.service.btaudiosrc/surround/setVolumeBalancing";

fn audio_rows(c: &Ctx) -> Vec<Row> {
    list_rows(c, Kind::Audio)
}

fn hid_rows(c: &Ctx) -> Vec<Row> {
    list_rows(c, Kind::Hid)
}

// ------------------------------------------------------------------------------------------------------------------ one device

/// One Bluetooth device (LG: the device's own screen under Registered Devices): connect or disconnect it, or delete it from the paired list.
/// Opened with `{kind, address, name, connected, surround}`; the actions name the device by its address. Disconnecting and deleting ask first;
/// the page goes back to the list. A connected speaker has LG's Position row (`btaudiosrc setChannel {address, from, to}`): a chooser of the
/// two positions that go with the one it has (Left and Right, otherwise Front and Rear), live only while the sound out is the Bluetooth speaker
/// with the TV's. LG asks before Left and Right swap, since the speaker that holds the other one takes this one's place.
fn arg<'a>(c: &'a Ctx<'_>, key: &str) -> Option<&'a Value> {
    match c.arg {
        PageArg::Json(v) => v.get(key),
        PageArg::None => None,
    }
}

fn device_title(c: &Ctx) -> String {
    c.arg.str("name").unwrap_or("Bluetooth device").to_string()
}

fn device_opened(c: &Ctx) -> Vec<Op> {
    vec![
        Op::Set("bt.position".into(), json!(c.arg.str("surround").unwrap_or("none"))),
        Op::Set("bt.connected".into(), json!(arg(c, "connected").and_then(Value::as_bool) == Some(true))),
    ]
}

fn audio_device_reads(_: &Ctx) -> Vec<Read> {
    vec![Read::lg("sound", &["soundOutput"]), Read::Call { name: "btdpaired", uri: AUDIO_LIST, payload: json!({}) }]
}

fn audio_device_answered(c: &Ctx, name: &str, doc: &Value) -> Vec<Op> {
    if doc.get("returnValue").and_then(Value::as_bool) == Some(false) {
        return Vec::new();
    }
    match name {
        // as LG's panel does, once the service has answered
        "btchannel" => vec![Op::Call { name: "btdpaired".into(), uri: Kind::Audio.uri("getPairedDevices"), payload: json!({}), subscribe: false }],
        "btdpaired" => {
            let address = c.arg.str("address").unwrap_or("");
            let devs = parse_paired(Kind::Audio, doc).unwrap_or_default();
            devs.iter()
                .find(|d| d.address == address)
                .map(|d| vec![Op::Set("bt.position".into(), json!(d.surround)), Op::Set("bt.connected".into(), json!(d.connected))])
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

fn positions(position: &str) -> [&'static str; 2] {
    if position == "left" || position == "right" { ["left", "right"] } else { ["front", "rear"] }
}

fn device_rows_of(c: &Ctx, kind: Kind) -> Vec<Row> {
    let address = c.arg.str("address").unwrap_or("").to_string();
    let name = c.arg.str("name").unwrap_or("").to_string();
    let connected = c.local.flag("bt.connected");
    let position = c.local.get("bt.position").as_str().unwrap_or("none").to_string();
    let with_tv = c.lg.is("sound.soundOutput", "tv_speaker_bluetooth");
    let (disconnect, delete) = match kind {
        Kind::Audio => (("disconnect", json!({"address": address})), ("removePairedDevice", json!({"address": address}))),
        Kind::Hid => (("requestDisconnect", json!({"deviceAddr": address})), ("requestDelete", json!({"deviceAddr": address}))),
    };
    let act = |(method, payload): (&str, Value)| Op::Act { uri: kind.uri(method), payload };
    let mut rows = Vec::new();

    let mut toggle = Row::link("toggle").icon("plug").label(if connected { "Disconnect" } else { "Connect" }).chevron(false).admin();
    toggle.act = Some(Rc::new({
        let (name, address) = (name.clone(), address.clone());
        let off = vec![act(disconnect.clone()), Op::Up];
        move |_| {
            if address.is_empty() {
                Vec::new()
            } else if connected {
                ask("unplug", &format!("Disconnect {name}?"), "", "Disconnect", "Keep it", false, off.clone())
            } else {
                let payload = if kind == Kind::Audio { json!({"address": address}) } else { json!({"name": name, "address": address}) };
                vec![Op::Act { uri: kind.uri(if kind == Kind::Audio { "connect" } else { "requestConnect" }), payload }, Op::Up]
            }
        }
    }));
    rows.push(toggle);

    if kind == Kind::Audio {
        let mut pos = Row::link("position").icon("speaker").label("Position").value(&names::name(names::SPEAKER_POSITION, &position)).admin();
        pos.visible = connected;
        pos.enabled = with_tv;
        pos.detail = if with_tv { String::new() } else { "Needs TV + Bluetooth".into() };
        let options: Vec<Pick> = positions(&position).iter().map(|v| Pick::new(v, &names::name(names::SPEAKER_POSITION, v))).collect();
        let (from, addr) = (position.clone(), address.clone());
        pos.act = Some(Rc::new(move |_| {
            let (from, addr) = (from.clone(), addr.clone());
            vec![Op::Pick(Rc::new(ChoiceSpec {
                title: "Position".into(),
                options: options.clone(),
                current: from.clone(),
                write: Box::new(move |to| {
                    if to == from {
                        return Vec::new();
                    }
                    let go = Op::Call {
                        name: "btchannel".into(),
                        uri: Kind::Audio.uri("setChannel"),
                        payload: json!({"address": addr, "from": from, "to": to}),
                        subscribe: false,
                    };
                    if (from == "left" && to == "right") || (from == "right" && to == "left") {
                        ask("speaker", "Swap Left and Right?", "The other speaker takes this position.", "Swap", "Keep it", false, vec![go])
                    } else {
                        vec![go]
                    }
                }),
            }))]
        }));
        rows.push(pos);
    }
    let delete_ops = ask("trash-2", &format!("Delete {name}?"), "It is removed from the paired list.", "Delete", "Keep it", true, vec![act(delete), Op::Up]);
    rows.push(Row::action("delete", "Delete").icon("trash-2").label("Delete Device").danger().admin().does(delete_ops));
    rows
}

fn audio_device_rows(c: &Ctx) -> Vec<Row> {
    device_rows_of(c, Kind::Audio)
}

fn hid_device_rows(c: &Ctx) -> Vec<Row> {
    device_rows_of(c, Kind::Hid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::{World, lg_with};
    use crate::settings::row::Kind as RowKind;

    fn by_id<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id} in {:?}", rows.iter().map(|r| &r.id).collect::<Vec<_>>()))
    }

    fn audio_doc() -> Value {
        json!({"returnValue": true, "devices": [
            {"address": "AA", "name": "Soundbar (L)", "state": "connected", "surround": "left", "avrcpSupported": true, "avrcpConnected": true},
            {"address": "BB", "name": "Soundbar (R)", "state": "connected", "surround": "right", "avrcpSupported": true, "avrcpConnected": true},
            {"address": "CC", "name": "Headset", "state": "disconnected"},
            {"address": "DD", "name": "Hidden", "display": false}]})
    }

    /// Carries out the `Set`s of an answer as the model does.
    fn settle(w: &mut World, ops: Vec<Op>) -> Vec<Op> {
        let mut rest = Vec::new();
        for op in ops {
            match op {
                Op::Set(k, v) => w.local.set(&k, v),
                other => rest.push(other),
            }
        }
        rest
    }

    #[test]
    fn the_paired_list_is_read_as_each_service_answers_it_and_an_empty_slot_is_left_out() {
        let devs = parse_paired(Kind::Audio, &audio_doc()).unwrap();
        assert_eq!(devs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["Soundbar (L)", "Soundbar (R)", "Headset"], "one the TV hides is not listed");
        assert!(devs[0].connected && devs[0].avrcp && !devs[2].connected && devs[2].surround == "none");
        assert!(paired_speakers(&devs), "two of one model that both answer over AVRCP");
        assert!(!paired_speakers(&devs[1..]));
        let hid = json!({"deviceLists": {"b": {"deviceAddr": "N/A", "deviceName": "Not Paired", "isPaired": false}, "a": {"deviceAddr": "11:22", "deviceName": "Keys", "isPaired": true, "isConnected": true}, "c": {"deviceAddr": "33:44", "deviceName": "Pad", "isConnected": false}}});
        let devs = parse_paired(Kind::Hid, &hid).unwrap();
        assert_eq!(devs.iter().map(|d| (d.name.as_str(), d.connected)).collect::<Vec<_>>(), [("Keys", true), ("Pad", false)], "sorted by slot, empty ones out");
        assert!(parse_paired(Kind::Audio, &json!({"returnValue": false})).is_none());
    }

    #[test]
    fn the_list_shows_the_registered_the_connectable_and_a_search_that_toggles() {
        let mut w = World::new(lg_with(
            &[("sound", json!({"settings": {"deviceCntOfBtGeneral": "singleConnection", "bluetoothMode": "surroundMode", "hearingAid": "off"}}))],
            &[],
        ));
        let r = audio_rows(&w.ctx());
        assert_eq!(by_id(&r, "head:Registered Devices (0)").text, "Registered Devices (0)");
        assert!(by_id(&r, "note:There are no paired devices.").visible);
        assert_eq!(by_id(&r, "sound.bluetoothMode").detail, "No speaker connected");
        assert!(!by_id(&r, "sound.bluetoothMode").enabled && r.iter().all(|x| x.id != "btvolume"));
        let ops = audio_answered(&w.ctx(), "btpaired", &audio_doc());
        settle(&mut w, ops);
        w.local.set("bt.volume", json!(4));
        w.local.set("bt.balance", json!(3));
        let r = audio_rows(&w.ctx());
        assert_eq!(by_id(&r, "dev:AA").value, "Connected \u{b7} Left");
        assert_eq!(by_id(&r, "dev:CC").value, "");
        assert!(by_id(&r, "sound.bluetoothMode").enabled && by_id(&r, "sound.bluetoothMode").detail.is_empty());
        assert_eq!((by_id(&r, "btvolume").num, by_id(&r, "btvolume").value_text.as_deref()), (4.0, Some("4")));
        assert_eq!(by_id(&r, "btbalance").value_text.as_deref(), Some("Left 2"));
        let vol = (by_id(&r, "btvolume").act.as_ref().unwrap())(Arg::Value(7.0));
        assert!(
            matches!(&vol[..], [Op::Set(k, _), Op::Act { uri, payload }] if k == "bt.volume" && uri.ends_with("/surround/setVolumeBalancing") && payload == &json!({"tvspeaker_bluetooth": 7.0}))
        );
        let open = (by_id(&r, "dev:CC").act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&open[..], [Op::Open { page, arg }] if page == "btdevice" && arg.str("address") == Some("CC") && arg.str("name") == Some("Headset")));
        // the search
        let on = (by_id(&r, "search").act.as_ref().unwrap())(Arg::Press);
        assert!(
            matches!(&on[..], [Op::Set(..), Op::Set(..), Op::Call { name, uri, subscribe: true, .. }] if name == "btfound" && uri.ends_with("/startDiscovery"))
        );
        w.local.set("bt.searching", json!(true));
        let r = audio_rows(&w.ctx());
        assert_eq!((by_id(&r, "search").label.as_str(), by_id(&r, "search").button.as_str()), ("Searching", "Stop"));
        let off = (by_id(&r, "search").act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&off[..], [Op::Set(..), Op::Close(n)] if n == "btfound"), "a Bluetooth audio search is stopped by letting the call go");
    }

    #[test]
    fn a_hearing_aid_takes_the_speaker_volume_row_away_and_says_why() {
        let mut w = World::new(lg_with(&[("sound", json!({"settings": {"bluetoothMode": "speakerMode", "hearingAid": "bluetooth"}}))], &[]));
        let ops = audio_answered(&w.ctx(), "btpaired", &audio_doc());
        settle(&mut w, ops);
        let r = audio_rows(&w.ctx());
        assert!(
            r.iter().all(|x| x.id != "btvolume")
                && by_id(&r, "sound.bluetoothMode").detail == "Hearing aid in use"
                && !by_id(&r, "sound.bluetoothMode").enabled
        );
    }

    #[test]
    fn found_devices_are_added_once_and_a_paired_one_is_not_found_again() {
        let mut w = World::new(lg_with(&[], &[]));
        let ops = audio_answered(&w.ctx(), "btpaired", &audio_doc());
        settle(&mut w, ops);
        w.local.set("bt.found", json!([]));
        let seen = |w: &mut World, doc: Value| {
            let ops = audio_answered(&w.ctx(), "btfound", &doc);
            settle(w, ops);
        };
        seen(&mut w, json!({"state": "started", "device": {"address": "EE", "name": "Speaker"}}));
        seen(&mut w, json!({"state": "started", "device": {"address": "EE", "name": "Speaker again"}}));
        seen(&mut w, json!({"state": "started", "device": {"address": "CC", "name": "Headset"}}));
        seen(&mut w, json!({"state": "started", "device": {"address": "FF"}}));
        assert_eq!(
            found(&w.ctx()),
            [("EE".to_string(), "Speaker".to_string()), ("FF".to_string(), "FF".to_string())],
            "a device without a name is its address"
        );
        w.local.set("bt.searching", json!(true));
        seen(&mut w, json!({"state": "stopped"}));
        assert!(!searching(&w.ctx()));
        // OK on one pairs and connects it, and it leaves the list
        let r = audio_rows(&w.ctx());
        assert_eq!(by_id(&r, "head:Connectable Devices (2)").tab, 0);
        let ops = (by_id(&r, "found:EE").act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&ops[..], [Op::Set(b, addr), Op::Call { name, uri, payload, .. }, Op::Set(f, left)]
            if b == "bt.busy" && addr == "EE" && name == "btconnect" && uri.ends_with("/connect") && payload == &json!({"address": "EE"}) && f == "bt.found" && left.as_array().unwrap().len() == 1));
        let ops = settle(&mut w, ops);
        assert!(matches!(&ops[..], [Op::Call { .. }]));
        assert_eq!(by_id(&audio_rows(&w.ctx()), "found:FF").value, "");
    }

    #[test]
    fn hid_search_asks_the_manager_and_its_end_says_so() {
        let mut w = World::new(lg_with(&[], &[]));
        let r = hid_rows(&w.ctx());
        let ops = (by_id(&r, "search").act.as_ref().unwrap())(Arg::Press);
        assert!(
            matches!(&ops[2..], [Op::Call { uri: a, subscribe: true, .. }, Op::Call { uri: b, subscribe: false, .. }] if a.ends_with("/subscribeDiscovery") && b.ends_with("/requestDiscovery"))
        );
        let ops = hid_answered(&w.ctx(), "hidfound", &json!({"status": "startDiscovery"}));
        settle(&mut w, ops);
        assert!(searching(&w.ctx()));
        let ops = hid_answered(&w.ctx(), "hidfound", &json!({"address": "11:22", "name": "Keys"}));
        settle(&mut w, ops);
        assert_eq!(found(&w.ctx()).len(), 1);
        let connect = (by_id(&hid_rows(&w.ctx()), "found:11:22").act.as_ref().unwrap())(Arg::Press);
        assert!(
            matches!(&connect[1], Op::Call { uri, payload, .. } if uri.ends_with("/requestConnect") && payload == &json!({"name": "Keys", "address": "11:22"}))
        );
        // leaving the page while it searches cancels the discovery and lets go of the calls
        let left = hid_closed(&w.ctx());
        assert!(matches!(&left[0], Op::Act { uri, .. } if uri.ends_with("/cancelDiscovery")));
        assert!(left.iter().any(|o| matches!(o, Op::Close(n) if n == "hidfound")) && left.iter().any(|o| matches!(o, Op::Close(n) if n == "hidstatus")));
        let ops = hid_answered(&w.ctx(), "hidfound", &json!({"status": "endDiscovery"}));
        settle(&mut w, ops);
        assert!(!searching(&w.ctx()));
        assert!(!matches!(hid_closed(&w.ctx())[0], Op::Act { .. }), "nothing to cancel once it ended");
    }

    #[test]
    fn a_change_of_state_reads_the_list_again_and_a_failed_connection_is_no_longer_pending() {
        let mut w = World::new(lg_with(&[], &[]));
        w.local.set("bt.busy", json!("CC"));
        let ops = audio_answered(&w.ctx(), "btstatus", &json!({"status": "fail"}));
        assert!(
            matches!(&ops[..], [Op::Set(k, v), Op::Call { name, uri, subscribe: false, .. }] if k == "bt.busy" && v == "" && name == "btpaired" && uri.ends_with("/getPairedDevices"))
        );
        let ops = hid_answered(&w.ctx(), "hidconnect", &json!({"returnValue": true}));
        assert!(matches!(&ops[..], [Op::Call { uri, .. }] if uri.ends_with("/getPairedDeviceLists")));
        assert!(audio_answered(&w.ctx(), "btstatus", &json!({"returnValue": false})).is_empty());
        // pending shows "Connecting" until the list says it is connected
        w.local.set("bt.busy", json!("CC"));
        let ops = audio_answered(&w.ctx(), "btpaired", &audio_doc());
        settle(&mut w, ops);
        assert_eq!(by_id(&audio_rows(&w.ctx()), "dev:CC").value, "Connecting");
        let mut doc = audio_doc();
        doc["devices"][2]["state"] = json!("connected");
        let ops = audio_answered(&w.ctx(), "btpaired", &doc);
        settle(&mut w, ops);
        assert_eq!(by_id(&audio_rows(&w.ctx()), "dev:CC").value, "Connected");
        assert_eq!(w.local.text("bt.busy"), "");
    }

    fn device_world(connected: bool, out: &str, position: &str) -> World {
        let mut w = World::new(lg_with(&[("sound", json!({"settings": {"soundOutput": out}}))], &[]));
        w.arg = PageArg::Json(json!({"kind": "audio", "address": "AA", "name": "Soundbar (L)", "connected": connected, "surround": position}));
        let ops = device_opened(&w.ctx());
        settle(&mut w, ops);
        w
    }

    #[test]
    fn a_device_connects_at_once_and_disconnects_and_deletes_after_a_question_then_goes_back() {
        let w = device_world(false, "tv_speaker", "none");
        assert_eq!(device_title(&w.ctx()), "Soundbar (L)");
        let r = audio_device_rows(&w.ctx());
        assert_eq!(by_id(&r, "toggle").label, "Connect");
        assert!(!by_id(&r, "position").visible);
        let ops = (by_id(&r, "toggle").act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&ops[..], [Op::Act { uri, payload }, Op::Up] if uri.ends_with("/connect") && payload == &json!({"address": "AA"})));
        let w = device_world(true, "tv_speaker_bluetooth", "left");
        let r = audio_device_rows(&w.ctx());
        assert_eq!(by_id(&r, "toggle").label, "Disconnect");
        let ops = (by_id(&r, "toggle").act.as_ref().unwrap())(Arg::Press);
        let [Op::Ask(a)] = &ops[..] else { panic!("{ops:?}") };
        assert_eq!((a.title.as_str(), a.answers[0].label.as_str(), a.answers[1].label.as_str()), ("Disconnect Soundbar (L)?", "Keep it", "Disconnect"));
        assert!(matches!(&a.answers[1].then[..], [Op::Act { uri, .. }, Op::Up] if uri.ends_with("/disconnect")));
        let ops = (by_id(&r, "delete").act.as_ref().unwrap())(Arg::Press);
        let [Op::Ask(a)] = &ops[..] else { panic!("{ops:?}") };
        assert!(
            a.answers[1].danger
                && matches!(&a.answers[1].then[..], [Op::Act { uri, payload }, Op::Up] if uri.ends_with("/removePairedDevice") && payload == &json!({"address": "AA"}))
        );
        assert_eq!(by_id(&r, "delete").kind, RowKind::Action);
        // the hid manager names the device by deviceAddr
        let mut h = device_world(true, "tv_speaker", "none");
        h.arg = PageArg::Json(json!({"kind": "hid", "address": "11:22", "name": "Keys", "connected": true}));
        let r = hid_device_rows(&h.ctx());
        assert!(r.iter().all(|x| x.id != "position"));
        let ops = (by_id(&r, "delete").act.as_ref().unwrap())(Arg::Press);
        let [Op::Ask(a)] = &ops[..] else { panic!() };
        assert!(
            matches!(&a.answers[1].then[..], [Op::Act { uri, payload }, Op::Up] if uri.ends_with("/requestDelete") && payload == &json!({"deviceAddr": "11:22"}))
        );
    }

    #[test]
    fn the_position_is_live_with_the_tv_and_the_speaker_and_a_swap_asks_first() {
        let w = device_world(true, "tv_speaker", "left");
        let r = audio_device_rows(&w.ctx());
        let p = by_id(&r, "position");
        assert!(p.visible && !p.enabled && p.detail == "Needs TV + Bluetooth" && p.value == "Left");
        let w = device_world(true, "tv_speaker_bluetooth", "left");
        let r = audio_device_rows(&w.ctx());
        assert!(by_id(&r, "position").enabled);
        let ops = (by_id(&r, "position").act.as_ref().unwrap())(Arg::Press);
        let [Op::Pick(spec)] = &ops[..] else { panic!("{ops:?}") };
        assert_eq!(spec.options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>(), ["Left", "Right"]);
        assert!((spec.write)("left").is_empty(), "the same position is nothing");
        let swap = (spec.write)("right");
        let [Op::Ask(a)] = &swap[..] else { panic!("{swap:?}") };
        assert!(
            matches!(&a.answers[1].then[..], [Op::Call { uri, payload, .. }] if uri.ends_with("/setChannel") && payload == &json!({"address": "AA", "from": "left", "to": "right"}))
        );
        let w = device_world(true, "tv_speaker_bluetooth", "front");
        let ops = (by_id(&audio_device_rows(&w.ctx()), "position").act.as_ref().unwrap())(Arg::Press);
        let [Op::Pick(spec)] = &ops[..] else { panic!() };
        assert_eq!(spec.options.iter().map(|o| o.value.as_str()).collect::<Vec<_>>(), ["front", "rear"]);
        assert!(matches!(&(spec.write)("rear")[..], [Op::Call { .. }]), "front and rear swap without asking");
    }

    #[test]
    fn the_device_page_follows_the_services_answer_for_its_position_and_its_connection() {
        let mut w = device_world(false, "tv_speaker_bluetooth", "none");
        let ops = audio_device_answered(&w.ctx(), "btdpaired", &audio_doc());
        settle(&mut w, ops);
        assert_eq!((w.local.text("bt.position"), w.local.flag("bt.connected")), ("left".to_string(), true));
        let ops = audio_device_answered(&w.ctx(), "btchannel", &json!({"returnValue": true}));
        assert!(matches!(&ops[..], [Op::Call { name, .. }] if name == "btdpaired"));
    }

    #[test]
    fn in_the_model_the_page_subscribes_follows_the_answers_and_lets_go_when_it_is_left() {
        use crate::kit::screen::{Effect, Input, Model as _};
        let mut m = crate::settings::model::Model::new(crate::settings::pages::all(), json!({}), json!({}));
        let fx = m.open(Some("btaudio"), 1000);
        let subscribed: Vec<(u64, String)> = fx
            .iter()
            .filter_map(|e| match e {
                Effect::Luna { tag, uri, subscribe: true, .. } => Some((*tag, uri.clone())),
                _ => None,
            })
            .collect();
        let list = subscribed.iter().find(|(_, u)| u.ends_with("btaudiosrc/getPairedDevices")).expect("the paired list is subscribed").0;
        assert!(
            subscribed.iter().any(|(_, u)| u.ends_with("btaudiosrc/getDeviceStatus"))
                && subscribed.iter().any(|(_, u)| u.ends_with("surround/getVolumeBalancing"))
        );
        m.update(&Input::Luna { tag: list, payload: audio_doc() }, 1100);
        let page = m.view().page.expect("the page is in front");
        assert_eq!((page.title.as_str(), page.back.as_deref()), ("Bluetooth Speakers", Some("Sound")));
        assert!(page.rows.iter().any(|r| r.id == "dev:AA" && r.value == "Connected \u{b7} Left"));
        let gone = m.update(&Input::Key { name: "back".into(), down: true, repeat: false }, 2000);
        assert!(gone.iter().any(|e| matches!(e, Effect::Cancel { tag } if *tag == list)), "leaving the page lets the subscription go: {gone:?}");
        assert_eq!(m.current(), "sound");
    }
}
