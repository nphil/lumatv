//! The radio, as LG lets the set have it: `com.webos.service.bluetooth2` over `luna-send`.
//!
//! There is no `hci0` on this set: the controller (`/dev/stpbt`) belongs to `webos-bluetooth-service`, and the only way to hear
//! advertisements is its `le/startScan` (docs/research/ble-result.md has the evidence). The call is a subscription:
//!
//! ```text
//! luna-send -n 1000000 luna://com.webos.service.bluetooth2/le/startScan '{"subscribe":true}'
//! ```
//!
//! Its first reply is `{"subscribed":true,"returnValue":true,"adapterAddress":"..."}`; every later one carries `device` (the one
//! that was heard: new, or with a changed property such as the signal strength) and `devices` (everything known so far). A device
//! is `{"address":"aa:bb:...","name":..,"rssi":-68,"typeOfDevice":"ble","scanRecord":[..bytes..],"manufacturerData":{..},..}`,
//! the same object `device/getStatus` shows for the paired ones on this set. The scan lasts as long as the child lives: ending
//! it (the service logs `StartScan listener dropped`) is how the radio is handed back, so [`LunaBluetooth`] kills the child at
//! the end of every window, on a failure, and when it is dropped.
//!
//! Everything that reads LG's JSON is a plain function here ([`Decoder`], [`advertisement`], [`parse_adapter`], [`audio_busy`]),
//! tested with replies in the shapes LG documents and the set was seen to give; the process side is behind [`Bluetooth`] so the
//! proxy and its duty control are tested with a fake and `LunaBluetooth` with a fake `luna-send`.

use parking_lot::Mutex;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub const SCAN_URI: &str = "luna://com.webos.service.bluetooth2/le/startScan";
pub const ADAPTER_URI: &str = "luna://com.webos.service.bluetooth2/adapter/getStatus";
pub const DEVICES_URI: &str = "luna://com.webos.service.bluetooth2/device/getStatus";

/// The longest raw advertisement ESPHome's proxy carries: an advertisement and its scan response, 31 bytes each.
pub const MAX_DATA: usize = 62;
/// The longest a one-shot read may take.
const ONE_SHOT_DEADLINE: Duration = Duration::from_secs(4);
/// A reply longer than this is not LG's.
const MAX_REPLY: usize = 256 * 1024;
/// Replies the reader holds for the runner before it drops the oldest (a busy room, a runner that is behind).
pub const MAX_QUEUED: usize = 1024;

/// One advertisement as the controller heard it, in what ESPHome's raw advertisement wants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heard {
    /// The address as a number, most significant byte first (`aa:bb:cc:dd:ee:ff` is `0xaabbccddeeff`).
    pub address: u64,
    pub rssi: i32,
    /// 0 public, 1 random.
    pub address_type: u32,
    /// The advertising data, as AD structures.
    pub data: Vec<u8>,
}

/// What LG's service said about the scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The subscription is made; the adapter's address.
    Subscribed(String),
    Heard(Heard),
    /// `returnValue: false`: the code and the words of the service (`errorCode`, `errorText`).
    Failed {
        code: i64,
        text: String,
    },
    /// The service ended the subscription (`subscribed: false`), or the child went away.
    Ended,
}

/// `aa:bb:cc:dd:ee:ff` (either case) as a number.
pub fn parse_address(text: &str) -> Option<u64> {
    let mut parts = text.split(':');
    let mut value: u64 = 0;
    for _ in 0..6 {
        let part = parts.next()?;
        if part.len() != 2 {
            return None;
        }
        value = (value << 8) | u64::from(u8::from_str_radix(part, 16).ok()?);
    }
    parts.next().is_none().then_some(value)
}

/// `AA:BB:CC:DD:EE:FF`.
pub fn address_text(address: u64) -> String {
    let b = address.to_be_bytes();
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        b[2], b[3], b[4], b[5], b[6], b[7]
    )
}

/// LG's reply carries no address type. A random address says so in its top two bits (11 static, 01 resolvable private; 00 is a
/// non-resolvable one, which is indistinguishable from a public one), so those two are reported as random and the rest as public.
/// Home Assistant reads the type only to connect, which this proxy does not offer.
pub fn address_type(address: u64) -> u32 {
    match (address >> 40) as u8 >> 6 {
        0b11 | 0b01 => 1,
        _ => 0,
    }
}

/// The AD structures at the front of `record`, up to [`MAX_DATA`] bytes: the controller's record is zero-padded, and a structure
/// that is cut off (or one that runs over the limit) ends it.
pub fn trim_record(record: &[u8]) -> Vec<u8> {
    let mut at = 0;
    while at < record.len() {
        let length = usize::from(record[at]);
        if length == 0 || at + 1 + length > record.len() || at + 1 + length > MAX_DATA {
            break;
        }
        at += 1 + length;
    }
    record[..at].to_vec()
}

fn bytes_of(value: &Value) -> Option<Vec<u8>> {
    value
        .as_array()?
        .iter()
        .map(|v| v.as_u64().and_then(|n| u8::try_from(n).ok()))
        .collect()
}

/// The advertising data of a device: LG's `scanRecord` when it has one; otherwise rebuilt from what the object does say (the
/// name, and the manufacturer data as `{"id"|"companyId": n, "data": [..]}`), so a stack that fills only the fields still
/// reaches Home Assistant with something it can read.
pub fn advertisement(device: &Value) -> Vec<u8> {
    if let Some(record) = device.get("scanRecord").and_then(bytes_of) {
        let data = trim_record(&record);
        if !data.is_empty() {
            return data;
        }
    }
    let mut data = Vec::new();
    let mut push = |kind: u8, body: &[u8]| {
        if body.len() < 255 && data.len() + 2 + body.len() <= MAX_DATA {
            data.push(u8::try_from(body.len() + 1).unwrap_or(0));
            data.push(kind);
            data.extend_from_slice(body);
        }
    };
    if let Some(name) = device.get("name").and_then(Value::as_str)
        && !name.is_empty()
    {
        push(0x09, name.as_bytes());
    }
    if let Some(m) = device.get("manufacturerData") {
        let company = m
            .get("companyId")
            .or_else(|| m.get("id"))
            .and_then(Value::as_u64)
            .and_then(|n| u16::try_from(n).ok());
        if let (Some(company), Some(body)) = (company, m.get("data").and_then(bytes_of)) {
            let mut payload = company.to_le_bytes().to_vec();
            payload.extend_from_slice(&body);
            push(0xff, &payload);
        }
    }
    data
}

/// A device object as an advertisement heard, or `None` for a classic (BR/EDR) device or one without an address.
pub fn heard(device: &Value) -> Option<Heard> {
    let address = parse_address(device.get("address")?.as_str()?)?;
    if device
        .get("typeOfDevice")
        .and_then(Value::as_str)
        .is_some_and(|t| !t.eq_ignore_ascii_case("ble"))
    {
        return None;
    }
    // 0 is what LG's list shows for a device that was not heard just now: no signal reading, not a 0 dBm one.
    let rssi = match device.get("rssi").and_then(Value::as_i64) {
        Some(0) | None => -127,
        Some(r) => i32::try_from(r.clamp(-127, 20)).unwrap_or(-127),
    };
    Some(Heard {
        address,
        rssi,
        address_type: address_type(address),
        data: advertisement(device),
    })
}

/// Reads the replies of a scan subscription, one JSON value at a time, into events.
///
/// `device` is what changed; `devices` repeats everything known and is used only when a reply has no `device` (and then only
/// for entries that differ from the last time), so a room of fifty devices costs one advertisement per reply, not fifty.
#[derive(Default)]
pub struct Decoder {
    started: bool,
    known: HashMap<u64, (i32, Vec<u8>)>,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, reply: &Value) -> Vec<Event> {
        let mut events = Vec::new();
        if reply.get("returnValue").and_then(Value::as_bool) == Some(false) {
            events.push(Event::Failed {
                code: reply.get("errorCode").and_then(Value::as_i64).unwrap_or(-1),
                text: reply
                    .get("errorText")
                    .and_then(Value::as_str)
                    .unwrap_or("no reason given")
                    .chars()
                    .take(200)
                    .collect(),
            });
            return events;
        }
        let first = !self.started;
        if first {
            self.started = true;
            events.push(Event::Subscribed(
                reply
                    .get("adapterAddress")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
            ));
        }
        if reply.get("subscribed").and_then(Value::as_bool) == Some(false) {
            events.push(Event::Ended);
            return events;
        }
        if let Some(device) = reply.get("device").and_then(heard) {
            self.known
                .insert(device.address, (device.rssi, device.data.clone()));
            events.push(Event::Heard(device));
        } else if !first && let Some(list) = reply.get("devices").and_then(Value::as_array) {
            for device in list.iter().filter_map(heard) {
                let key = (device.rssi, device.data.clone());
                if self.known.get(&device.address) != Some(&key) {
                    self.known.insert(device.address, key);
                    events.push(Event::Heard(device));
                }
            }
        }
        if self.known.len() > 4096 {
            self.known.clear();
        }
        events
    }
}

/// What `adapter/getStatus` says about the default adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adapter {
    /// `AA:BB:CC:DD:EE:FF`.
    pub address: String,
    pub powered: bool,
}

pub fn parse_adapter(reply: &Value) -> Option<Adapter> {
    let adapter = reply.get("adapters")?.as_array()?.first()?;
    Some(Adapter {
        address: address_text(parse_address(adapter.get("adapterAddress")?.as_str()?)?),
        powered: adapter.get("powered").and_then(Value::as_bool) == Some(true),
    })
}

/// Whether `device/getStatus` shows a Bluetooth audio or hands-free link up: a device with a connected profile that is A2DP,
/// HFP, headset or audio. Scanning shares the antenna with the stream, so a window is not opened while one is playing.
pub fn audio_busy(reply: &Value) -> bool {
    reply
        .get("devices")
        .and_then(Value::as_array)
        .is_some_and(|devices| {
            devices.iter().any(|device| {
                device
                    .get("connectedProfiles")
                    .and_then(Value::as_array)
                    .is_some_and(|profiles| {
                        profiles.iter().any(|p| {
                            let text = p.to_string().to_ascii_lowercase();
                            ["a2dp", "hfp", "hsp", "headset", "audio", "sco"]
                                .iter()
                                .any(|k| text.contains(k))
                        })
                    })
            })
        })
}

/// A wake-up for the runner, called from the reader's thread when replies are waiting.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// The radio: the one-shot reads, and the scan.
pub trait Bluetooth: Send {
    /// `adapter/getStatus`.
    fn adapter(&mut self) -> Result<Adapter, String>;
    /// Whether a Bluetooth audio link is up (`device/getStatus`).
    fn audio_busy(&mut self) -> Result<bool, String>;
    /// Starts the scan subscription. `always` asks LG's service to report every advertisement, not only a new or changed device.
    fn start_scan(&mut self, always: bool, wake: Wake) -> Result<(), String>;
    /// Ends it. Idempotent; the radio is free when this returns.
    fn stop_scan(&mut self);
    /// Everything the scan said since the last call. An unexpected end of the child shows up here as [`Event::Ended`].
    fn take_events(&mut self) -> Vec<Event>;
    /// How many replies the reader had to drop because the runner was behind, since the last call.
    fn take_dropped(&mut self) -> u64;
}

type Slot = Arc<Mutex<Option<Child>>>;

struct Scan {
    slot: Slot,
    rx: Receiver<Value>,
    reader: Option<JoinHandle<()>>,
    decoder: Decoder,
    dropped: Arc<Mutex<u64>>,
    ended_told: bool,
}

/// The real radio: `luna-send` children.
pub struct LunaBluetooth {
    program: String,
    scan: Option<Scan>,
}

impl LunaBluetooth {
    pub fn new(program: &str) -> Self {
        Self {
            program: program.to_owned(),
            scan: None,
        }
    }

    /// One `luna-send -n 1` and its reply.
    fn call(&self, uri: &str) -> Result<Value, String> {
        let mut child = Command::new(&self.program)
            .args(["-n", "1", uri, "{}"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not run {}: {e}", self.program))?;
        let Some(mut out) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("no output pipe".to_owned());
        };
        let (tx, rx) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("ble-read".to_owned())
            .stack_size(128 * 1024)
            .spawn(move || {
                let mut text = String::new();
                let _ = out
                    .by_ref()
                    .take(MAX_REPLY as u64)
                    .read_to_string(&mut text);
                let _ = tx.send(text);
            });
        if let Err(e) = spawned {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("could not start a reader: {e}"));
        }
        let text = match rx.recv_timeout(ONE_SHOT_DEADLINE) {
            Ok(text) => text,
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("luna-send did not answer".to_owned());
            }
        };
        let _ = child.wait();
        serde_json::Deserializer::from_str(&text)
            .into_iter::<Value>()
            .find_map(Result::ok)
            .ok_or_else(|| "luna-send gave no reply".to_owned())
    }
}

fn end_child(slot: &Slot) {
    if let Some(mut child) = slot.lock().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Bluetooth for LunaBluetooth {
    fn adapter(&mut self) -> Result<Adapter, String> {
        let reply = self.call(ADAPTER_URI)?;
        if reply.get("returnValue").and_then(Value::as_bool) == Some(false) {
            return Err(format!(
                "adapter/getStatus: {}",
                reply
                    .get("errorText")
                    .and_then(Value::as_str)
                    .unwrap_or("refused")
            ));
        }
        parse_adapter(&reply).ok_or_else(|| "adapter/getStatus listed no adapter".to_owned())
    }

    fn audio_busy(&mut self) -> Result<bool, String> {
        let reply = self.call(DEVICES_URI)?;
        if reply.get("returnValue").and_then(Value::as_bool) == Some(false) {
            return Err("device/getStatus was refused".to_owned());
        }
        Ok(audio_busy(&reply))
    }

    fn start_scan(&mut self, always: bool, wake: Wake) -> Result<(), String> {
        self.stop_scan();
        let payload = if always {
            json!({"subscribe": true, "always": true})
        } else {
            json!({"subscribe": true})
        }
        .to_string();
        let mut child = Command::new(&self.program)
            .args(["-n", "1000000", SCAN_URI, &payload])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not run {}: {e}", self.program))?;
        let Some(out) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("no output pipe".to_owned());
        };
        let (tx, rx) = mpsc::sync_channel::<Value>(MAX_QUEUED);
        let dropped = Arc::new(Mutex::new(0u64));
        let counted = dropped.clone();
        let spawned = thread::Builder::new()
            .name("ble-scan".to_owned())
            .stack_size(128 * 1024)
            .spawn(move || {
                // A reply is one line when luna-send prints it compact; a pretty-printed one is joined until it parses.
                let mut pending = String::new();
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    pending.push_str(&line);
                    pending.push('\n');
                    let parsed = serde_json::from_str::<Value>(&pending);
                    match parsed {
                        Ok(value) => {
                            pending.clear();
                            match tx.try_send(value) {
                                Ok(()) => wake(),
                                Err(mpsc::TrySendError::Full(_)) => *counted.lock() += 1,
                                Err(mpsc::TrySendError::Disconnected(_)) => return,
                            }
                        }
                        Err(e) if e.is_eof() && pending.len() <= MAX_REPLY => {}
                        Err(_) => pending.clear(),
                    }
                }
                // The end of the output is the end of the scan: the runner learns it from the child.
                wake();
            });
        let reader = match spawned {
            Ok(reader) => reader,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("could not start a reader: {e}"));
            }
        };
        let slot: Slot = Arc::new(Mutex::new(Some(child)));
        self.scan = Some(Scan {
            slot,
            rx,
            reader: Some(reader),
            decoder: Decoder::new(),
            dropped,
            ended_told: false,
        });
        Ok(())
    }

    fn stop_scan(&mut self) {
        if let Some(mut scan) = self.scan.take() {
            end_child(&scan.slot);
            if let Some(reader) = scan.reader.take() {
                let _ = reader.join();
            }
        }
    }

    fn take_events(&mut self) -> Vec<Event> {
        let Some(scan) = self.scan.as_mut() else {
            return Vec::new();
        };
        let mut events = Vec::new();
        while let Ok(reply) = scan.rx.try_recv() {
            events.extend(scan.decoder.feed(&reply));
        }
        // A child that went away on its own (the service restarted, luna-send was killed) is the end of the scan; what it said
        // before is above, because the reader sends it all before the end of the output.
        if !scan.ended_told {
            let gone = match scan.slot.lock().as_mut() {
                Some(child) => child.try_wait().ok().flatten().is_some(),
                None => true,
            };
            if gone && scan.reader.as_ref().is_none_or(JoinHandle::is_finished) {
                while let Ok(reply) = scan.rx.try_recv() {
                    events.extend(scan.decoder.feed(&reply));
                }
                scan.ended_told = true;
                events.push(Event::Ended);
            }
        }
        events
    }

    fn take_dropped(&mut self) -> u64 {
        self.scan
            .as_ref()
            .map_or(0, |s| std::mem::take(&mut *s.dropped.lock()))
    }
}

impl Drop for LunaBluetooth {
    fn drop(&mut self) {
        self.stop_scan();
    }
}

#[cfg(test)]
pub mod fake {
    //! A radio that is two lists: what the test says LG heard, and what was asked of it.

    use super::*;
    use std::collections::VecDeque;

    #[derive(Default)]
    pub struct State {
        pub adapter: Option<Result<Adapter, String>>,
        pub audio: bool,
        pub scanning: bool,
        pub always: Option<bool>,
        pub starts: u32,
        pub stops: u32,
        pub refuse_start: Option<String>,
        /// The service never answers the call (a hung service).
        pub silent_start: bool,
        pub events: VecDeque<Event>,
        pub wake: Option<Wake>,
        pub dropped: u64,
    }

    #[derive(Clone, Default)]
    pub struct FakeBluetooth {
        pub state: Arc<Mutex<State>>,
    }

    impl FakeBluetooth {
        pub fn new() -> Self {
            let f = Self::default();
            f.state.lock().adapter = Some(Ok(Adapter {
                address: "84:A3:29:B9:4D:43".into(),
                powered: true,
            }));
            f
        }

        /// LG reports an advertisement (only while a scan is on).
        pub fn hear(&self, heard: Heard) {
            self.say(Event::Heard(heard));
        }

        pub fn say(&self, event: Event) {
            let mut s = self.state.lock();
            if s.scanning {
                s.events.push_back(event);
                if let Some(wake) = &s.wake {
                    wake();
                }
            }
        }
    }

    impl Bluetooth for FakeBluetooth {
        fn adapter(&mut self) -> Result<Adapter, String> {
            self.state
                .lock()
                .adapter
                .clone()
                .unwrap_or_else(|| Err("no adapter".into()))
        }

        fn audio_busy(&mut self) -> Result<bool, String> {
            Ok(self.state.lock().audio)
        }

        fn start_scan(&mut self, always: bool, wake: Wake) -> Result<(), String> {
            let mut s = self.state.lock();
            if let Some(why) = s.refuse_start.clone() {
                return Err(why);
            }
            s.scanning = true;
            s.always = Some(always);
            s.starts += 1;
            s.wake = Some(wake);
            // The service answers the call at once, unless a test says it is hung.
            if !s.silent_start {
                s.events
                    .push_back(Event::Subscribed("84:a3:29:b9:4d:43".into()));
            }
            Ok(())
        }

        fn stop_scan(&mut self) {
            let mut s = self.state.lock();
            if s.scanning {
                s.stops += 1;
            }
            s.scanning = false;
            s.events.clear();
        }

        fn take_events(&mut self) -> Vec<Event> {
            self.state.lock().events.drain(..).collect()
        }

        fn take_dropped(&mut self) -> u64 {
            std::mem::take(&mut self.state.lock().dropped)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ble(address: &str, rssi: i64, record: &[u8]) -> Value {
        json!({
            "address": address, "rssi": rssi, "typeOfDevice": "ble", "name": "x",
            "scanRecord": record, "manufacturerData": {}, "paired": false,
            "connectedProfiles": [], "adapterAddress": "",
        })
    }

    #[test]
    fn addresses_round_trip_in_the_order_they_are_printed() {
        assert_eq!(parse_address("58:01:5f:a3:f7:f0"), Some(0x5801_5fa3_f7f0));
        assert_eq!(address_text(0x5801_5fa3_f7f0), "58:01:5F:A3:F7:F0");
        for bad in [
            "",
            "58:01:5f:a3:f7",
            "58:01:5f:a3:f7:f0:00",
            "5801:5f:a3:f7:f0",
            "zz:01:5f:a3:f7:f0",
        ] {
            assert_eq!(parse_address(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_random_address_says_so_in_its_top_bits() {
        assert_eq!(address_type(0xc801_0203_0405), 1);
        assert_eq!(address_type(0x7801_0203_0405), 1);
        assert_eq!(address_type(0xa4c1_3800_0001), 0);
    }

    #[test]
    fn a_zero_padded_record_is_cut_at_its_last_whole_structure() {
        // flags, a name, then the controller's padding to 62 bytes.
        let mut record = vec![2, 0x01, 0x06, 4, 0x09, b'a', b'b', b'c'];
        record.resize(62, 0);
        assert_eq!(trim_record(&record), &record[..8]);
        // A structure that runs past the end of what there is ends the record before it.
        assert_eq!(trim_record(&[2, 1, 6, 9, 9, 1]), vec![2, 1, 6]);
        assert_eq!(trim_record(&[0, 0, 0]), Vec::<u8>::new());
        // Nothing longer than an advertisement and its scan response.
        let long: Vec<u8> = std::iter::repeat_n(
            [21u8, 0xff].into_iter().chain(std::iter::repeat_n(7u8, 20)),
            4,
        )
        .flatten()
        .collect();
        assert_eq!(trim_record(&long).len(), 44);
    }

    #[test]
    fn a_device_without_a_record_is_rebuilt_from_its_fields() {
        let mut device = ble("aa:bb:cc:dd:ee:ff", -60, &[]);
        device["name"] = json!("LYWSD03MMC");
        device["manufacturerData"] = json!({"id": 0x004c, "data": [2, 21, 9]});
        let data = advertisement(&device);
        let mut expected = vec![11, 0x09];
        expected.extend_from_slice(b"LYWSD03MMC");
        expected.extend_from_slice(&[6, 0xff, 0x4c, 0x00, 2, 21, 9]);
        assert_eq!(data, expected);
        // An empty manufacturer object (what LG sends for none) adds nothing.
        let bare = ble("aa:bb:cc:dd:ee:ff", -60, &[]);
        assert_eq!(advertisement(&bare), vec![2, 0x09, b'x']);
    }

    #[test]
    fn only_a_ble_device_with_a_usable_address_is_heard() {
        let mut classic = ble("aa:bb:cc:dd:ee:ff", -60, &[]);
        classic["typeOfDevice"] = json!("bredr");
        assert_eq!(heard(&classic), None);
        assert_eq!(heard(&json!({"rssi": -50})), None);
        let h = heard(&ble("aa:bb:cc:dd:ee:ff", -61, &[2, 1, 6])).unwrap();
        assert_eq!(
            (h.address, h.rssi, h.data.as_slice()),
            (0xaabb_ccdd_eeff, -61, &[2u8, 1, 6][..])
        );
        // 0 is "not heard just now" in LG's lists; it is not a signal.
        assert_eq!(heard(&ble("aa:bb:cc:dd:ee:ff", 0, &[])).unwrap().rssi, -127);
    }

    #[test]
    fn the_decoder_follows_the_documented_conversation() {
        let mut d = Decoder::new();
        // The reply to the call.
        let first = d.feed(&json!({"subscribed": true, "returnValue": true, "adapterAddress": "84:a3:29:b9:4d:43"}));
        assert_eq!(first, vec![Event::Subscribed("84:a3:29:b9:4d:43".into())]);
        // A new device: `device` and the whole list. Only `device` counts.
        let one = ble("58:01:5f:a3:f7:f0", -68, &[2, 1, 6]);
        let other = ble("42:f9:06:f5:b3:86", -80, &[2, 1, 6]);
        let events = d.feed(&json!({"subscribed": true, "returnValue": true, "device": one, "devices": [one, other]}));
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], Event::Heard(h) if h.address == 0x5801_5fa3_f7f0 && h.rssi == -68)
        );
        // A reply without `device`: only what differs from before is heard.
        let moved = ble("58:01:5f:a3:f7:f0", -70, &[2, 1, 6]);
        let events =
            d.feed(&json!({"subscribed": true, "returnValue": true, "devices": [moved, other]}));
        assert_eq!(
            events.len(),
            2,
            "the moved one and the one never heard from"
        );
        let events =
            d.feed(&json!({"subscribed": true, "returnValue": true, "devices": [moved, other]}));
        assert!(events.is_empty());
        // The end.
        assert_eq!(
            d.feed(&json!({"subscribed": false, "returnValue": true})),
            vec![Event::Ended]
        );
    }

    #[test]
    fn a_refusal_carries_the_codes_of_the_service() {
        let mut d = Decoder::new();
        let events =
            d.feed(&json!({"returnValue": false, "errorCode": 143, "errorText": "Not permitted"}));
        assert_eq!(
            events,
            vec![Event::Failed {
                code: 143,
                text: "Not permitted".into()
            }]
        );
    }

    #[test]
    fn the_first_reply_never_replays_the_stale_list() {
        let mut d = Decoder::new();
        let old = ble("58:01:5f:a3:f7:f0", -68, &[2, 1, 6]);
        let events = d.feed(&json!({"subscribed": true, "returnValue": true, "adapterAddress": "a", "devices": [old]}));
        assert_eq!(events, vec![Event::Subscribed("a".into())]);
    }

    #[test]
    fn the_adapter_and_the_audio_guard_read_what_the_set_returned() {
        // From this set (adapter/getStatus, device/getStatus).
        let status = json!({"adapters":[{"discoveryTimeout":10,"discoverable":true,"powered":true,"name":"Living Room TV","adapterAddress":"84:a3:29:b9:4d:43"}],"returnValue":true,"subscribed":false});
        assert_eq!(
            parse_adapter(&status),
            Some(Adapter {
                address: "84:A3:29:B9:4D:43".into(),
                powered: true
            })
        );
        let off = json!({"adapters":[{"powered":false,"adapterAddress":"84:a3:29:b9:4d:43"}]});
        assert!(!parse_adapter(&off).unwrap().powered);
        assert_eq!(parse_adapter(&json!({"adapters": []})), None);

        let idle = json!({"devices":[{"name":"steamdeck","connectedProfiles":[],"typeOfDevice":"bredr"},{"name":"LGE MR24","connectedProfiles":[],"typeOfDevice":"ble"}]});
        assert!(!audio_busy(&idle));
        let remote_only = json!({"devices":[{"name":"LGE MR24","connectedProfiles":["hid"]}]});
        assert!(!audio_busy(&remote_only));
        let playing =
            json!({"devices":[{"name":"buds","connectedProfiles":[{"mnemonic":"A2DP-sink"}]}]});
        assert!(audio_busy(&playing));
        assert!(audio_busy(
            &json!({"devices":[{"connectedProfiles":["hfp"]}]})
        ));
    }
}

#[cfg(test)]
mod process_tests {
    //! `LunaBluetooth` against a `luna-send` that is a shell script speaking in the shapes LG documents and this set gave.

    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::time::Instant;

    /// Answers the one-shot reads like the set did, and a scan like the documentation shows: the acknowledgement, then one reply
    /// a device (the second pretty-printed over several lines), then it stays until killed. Its arguments and process id are kept.
    const SCRIPT: &str = r#"
[ "$1" = probe ] && exit 0
printf '%s\n' "$*" >> "$DIR/args"
case "$*" in
*adapter/getStatus*)
  echo '{"adapters":[{"powered":true,"adapterAddress":"84:a3:29:b9:4d:43","name":"Living Room TV"}],"returnValue":true,"subscribed":false}' ;;
*device/getStatus*)
  echo "$AUDIO" ;;
*le/startScan*)
  echo $$ > "$DIR/pid"
  echo '{"subscribed":true,"adapterAddress":"84:a3:29:b9:4d:43","returnValue":true}'
  echo '{"device":{"address":"58:01:5f:a3:f7:f0","name":"Tag","rssi":-68,"typeOfDevice":"ble","scanRecord":[2,1,6,3,3,24,24,0,0,0],"manufacturerData":{}},"returnValue":true,"subscribed":true,"devices":[]}'
  printf '{\n  "device": {\n    "address": "42:f9:06:f5:b3:86",\n    "rssi": -80,\n    "typeOfDevice": "ble",\n    "scanRecord": [2, 1, 6]\n  },\n  "returnValue": true,\n  "subscribed": true\n}\n'
  [ -n "$ENDS" ] && exit 0
  exec sleep 30 ;;
esac
"#;

    fn install(dir: &Path, env: &str) -> String {
        let path = dir.join("luna-send");
        let body = format!("#!/bin/sh\nDIR={}\n{env}\n{SCRIPT}", dir.display());
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A script just written may be "text file busy" for a moment while another test forks.
        for _ in 0..200 {
            match Command::new(&path).arg("probe").status() {
                Err(e) if e.raw_os_error() == Some(26) => thread::sleep(Duration::from_millis(10)),
                _ => break,
            }
        }
        path.display().to_string()
    }

    fn wait_for(bt: &mut LunaBluetooth, mut done: impl FnMut(&[Event]) -> bool) -> Vec<Event> {
        let mut events = Vec::new();
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            events.extend(bt.take_events());
            if done(&events) {
                return events;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("never: {events:?}");
    }

    fn alive(pid: u32) -> bool {
        Path::new(&format!("/proc/{pid}")).exists()
            && std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .is_ok_and(|s| !s.contains(") Z "))
    }

    #[test]
    fn the_one_shot_reads_say_what_the_set_said() {
        let dir = tempfile::tempdir().unwrap();
        let mut bt = LunaBluetooth::new(&install(
            dir.path(),
            "AUDIO='{\"devices\":[{\"connectedProfiles\":[]}],\"returnValue\":true}'",
        ));
        assert_eq!(
            bt.adapter().unwrap(),
            Adapter {
                address: "84:A3:29:B9:4D:43".into(),
                powered: true
            }
        );
        assert_eq!(bt.audio_busy(), Ok(false));
        let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
        assert!(args.contains("-n 1 luna://com.webos.service.bluetooth2/adapter/getStatus {}"));
        assert!(args.contains("-n 1 luna://com.webos.service.bluetooth2/device/getStatus {}"));
        let mut busy = LunaBluetooth::new(&install(
            dir.path(),
            "AUDIO='{\"devices\":[{\"connectedProfiles\":[\"A2DP-sink\"]}],\"returnValue\":true}'",
        ));
        assert_eq!(busy.audio_busy(), Ok(true));
    }

    #[test]
    fn a_scan_is_one_subscription_whose_replies_come_back_as_advertisements_and_ending_it_kills_the_child()
     {
        let dir = tempfile::tempdir().unwrap();
        let mut bt = LunaBluetooth::new(&install(dir.path(), "AUDIO='{}'"));
        bt.start_scan(false, Arc::new(|| {})).unwrap();
        let events = wait_for(&mut bt, |e| {
            e.iter().filter(|e| matches!(e, Event::Heard(_))).count() == 2
        });
        assert_eq!(events[0], Event::Subscribed("84:a3:29:b9:4d:43".into()));
        let Event::Heard(first) = &events[1] else {
            panic!("{events:?}")
        };
        assert_eq!(
            (first.address, first.rssi, first.data.as_slice()),
            (0x5801_5fa3_f7f0, -68, &[2u8, 1, 6, 3, 3, 24, 24][..])
        );
        // The reply that luna-send pretty-printed over several lines is read as one.
        let Event::Heard(second) = &events[2] else {
            panic!("{events:?}")
        };
        assert_eq!((second.address, second.rssi), (0x42f9_06f5_b386, -80));
        let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
        assert_eq!(
            args.trim(),
            "-n 1000000 luna://com.webos.service.bluetooth2/le/startScan {\"subscribe\":true}"
        );
        let pid: u32 = std::fs::read_to_string(dir.path().join("pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(alive(pid));
        bt.stop_scan();
        assert!(
            !alive(pid),
            "the radio is handed back when the scan is ended"
        );
        assert!(bt.take_events().is_empty());
        bt.stop_scan();
    }

    #[test]
    fn the_always_option_is_asked_only_when_chosen() {
        let dir = tempfile::tempdir().unwrap();
        let mut bt = LunaBluetooth::new(&install(dir.path(), "AUDIO='{}'"));
        bt.start_scan(true, Arc::new(|| {})).unwrap();
        wait_for(&mut bt, |e| !e.is_empty());
        bt.stop_scan();
        let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
        assert!(
            args.contains("{\"always\":true,\"subscribe\":true}")
                || args.contains("{\"subscribe\":true,\"always\":true}"),
            "{args}"
        );
    }

    #[test]
    fn a_child_that_goes_away_is_reported_as_the_end_after_what_it_said() {
        let dir = tempfile::tempdir().unwrap();
        let mut bt = LunaBluetooth::new(&install(dir.path(), "AUDIO='{}'\nENDS=1"));
        bt.start_scan(false, Arc::new(|| {})).unwrap();
        let events = wait_for(&mut bt, |e| e.contains(&Event::Ended));
        let heard = events
            .iter()
            .filter(|e| matches!(e, Event::Heard(_)))
            .count();
        assert_eq!(
            heard, 2,
            "everything said before the end is delivered: {events:?}"
        );
        assert_eq!(events.last(), Some(&Event::Ended));
        assert!(bt.take_events().is_empty(), "the end is told once");
    }

    #[test]
    fn dropping_the_radio_ends_a_scan_that_is_on() {
        let dir = tempfile::tempdir().unwrap();
        let mut bt = LunaBluetooth::new(&install(dir.path(), "AUDIO='{}'"));
        bt.start_scan(false, Arc::new(|| {})).unwrap();
        wait_for(&mut bt, |e| !e.is_empty());
        let pid: u32 = std::fs::read_to_string(dir.path().join("pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        drop(bt);
        assert!(!alive(pid));
    }

    #[test]
    fn a_program_that_is_not_there_is_an_error_not_a_panic() {
        let mut bt = LunaBluetooth::new("/nonexistent/luna-send");
        assert!(bt.adapter().is_err());
        assert!(bt.audio_busy().is_err());
        assert!(bt.start_scan(false, Arc::new(|| {})).is_err());
        assert!(bt.take_events().is_empty());
    }
}
