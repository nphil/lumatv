//! Fixed Cyclone 2 settings through the helper: the app jail's hidraw nodes do not follow receiver mode changes.
use crate::http::{Request, Response, error_response};
use crate::hub::Hub;
use gamesir_config::{device::{self, Check, Controller, Hidraw, Timing, Transport}, frame::{self, Region, Report}, profile::{Flag, Profile, ReportRate, Side}};
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{io, sync::Arc, time::Duration};

pub const ROUTES: &[(&str, &str, &str)] = &[
    ("GET", "/v1/gaming/gamesir", "discover a Cyclone 2 and read its active profile; Gaming must be enabled"),
    ("POST", "/v1/gaming/gamesir", "save one fixed controller setting {setting,value}, requiring matching read-back"),
];
static CONFIG: Mutex<()> = Mutex::new(());

#[derive(Debug, Deserialize)]
#[serde(tag = "setting", content = "value", rename_all = "kebab-case", deny_unknown_fields)]
enum Change {
    ReportRate(u16),
    RumbleLeft(u8),
    RumbleRight(u8),
    DpadDiagonalLock(bool),
    XinputAbxySwap(bool),
    DeadzoneLeft(u8),
    DeadzoneRight(u8),
}
impl Change {
    fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::ReportRate(250 | 500 | 1000) | Self::RumbleLeft(0..=4) | Self::RumbleRight(0..=4)
            | Self::DpadDiagonalLock(_) | Self::XinputAbxySwap(_) | Self::DeadzoneLeft(0..=20) | Self::DeadzoneRight(0..=20) => Ok(()),
            _ => Err("report rate must be 250, 500 or 1000 Hz; rumble 0-4; dead zone 0-20%"),
        }
    }
    fn apply(&self, profile: &mut Profile) -> Result<(), gamesir_config::Error> {
        match *self {
            Self::ReportRate(rate) => profile.set_report_rate(match rate { 250 => ReportRate::Hz250, 500 => ReportRate::Hz500, _ => ReportRate::Hz1000 }),
            Self::RumbleLeft(left) => profile.set_rumble(left, profile.rumble().1)?,
            Self::RumbleRight(right) => profile.set_rumble(profile.rumble().0, right)?,
            Self::DpadDiagonalLock(on) => profile.set_flag(Flag::DpadDiagonalLock, on),
            Self::XinputAbxySwap(on) => profile.set_flag(Flag::XinputAbxySwap, on),
            Self::DeadzoneLeft(inner) | Self::DeadzoneRight(inner) => {
                let side = if matches!(self, Self::DeadzoneLeft(_)) { Side::Left } else { Side::Right };
                let mut stick = profile.stick(side);
                let mut deadzone = stick.response.deadzone;
                deadzone.inner = inner;
                stick.response = stick.response.with_deadzone(deadzone);
                profile.set_stick(side, &stick)?;
            }
        }
        Ok(())
    }
}

struct Gated<'a> { io: Hidraw, hub: &'a Arc<Hub>, request: &'a Request }
impl Transport for Gated<'_> {
    fn send(&mut self, report: &Report) -> io::Result<()> {
        // Recheck on each command so an off/standby transition cancels a multi-report exchange.
        crate::gaming::check_enabled(self.hub, self.request).map_err(|_| io::Error::new(io::ErrorKind::PermissionDenied, "Gaming was switched off or the TV entered standby"))?;
        self.io.send(report)
    }
    fn recv(&mut self, buf: &mut [u8; frame::REPORT_LEN], timeout: Duration) -> io::Result<Option<usize>> { self.io.recv(buf, timeout) }
}

fn ok(mut value: Value) -> Response {
    value["ok"] = json!(true);
    Response::json(200, &value)
}
fn failed(error: impl std::fmt::Display) -> Response { error_response(502, "controller_failed", &error.to_string()) }

pub fn handle(hub: &Arc<Hub>, request: &Request) -> Response {
    if !matches!(request.method.as_str(), "GET" | "POST") {
        return error_response(405, "method_not_allowed", "that method is not served here").with_header("Allow", "GET, POST");
    }
    if let Err(response) = crate::gaming::check_enabled(hub, request) { return response; }
    let change = if request.method == "POST" {
        let change: Change = match serde_json::from_slice(&request.body) {
            Ok(change) => change,
            Err(error) => return error_response(400, "bad_request", &error.to_string()),
        };
        if let Err(error) = change.validate() { return error_response(400, "bad_request", error); }
        Some(change)
    } else { None };
    let Some(_lock) = CONFIG.try_lock() else { return error_response(409, "busy", "A controller request is already in progress"); };
    let found = match device::discover() { Ok(found) => found, Err(error) => return failed(error) };
    let Some(device) = found.iter().find(|device| device.config && device.model.is_some()) else {
        if let Some(device) = found.first() { return ok(json!({"state":"mode", "device":device.name})); }
        return ok(json!({"state":"missing"}));
    };
    let io = match Hidraw::open(&device.node) { Ok(io) => io, Err(error) => return failed(error) };
    let mut controller = Controller::with_timing(Gated { io, hub, request }, Timing { resend_after: Duration::from_millis(200), max_sends: 2 });
    match profile(&mut controller, &device.name, change.as_ref()) { Ok(value) => ok(value), Err(response) => response }
}

fn profile<T: Transport>(controller: &mut Controller<T>, name: &str, change: Option<&Change>) -> Result<Value, Response> {
    let firmware = controller.firmware_version().map_err(failed)?;
    let slot = controller.current_profile().map_err(failed)?;
    let region = Region::profile(slot).ok_or_else(|| failed("Controller returned an invalid profile slot"))?;
    let before = controller.read(region).map_err(failed)?;
    let mut profile = Profile::from_bytes(&before).map_err(failed)?;
    if let Some(change) = change {
        change.validate().map_err(|error| error_response(400, "bad_request", error))?;
        change.apply(&mut profile).map_err(|error| error_response(400, "bad_request", &error.to_string()))?;
        let written = controller.write(region, &before, profile.as_bytes(), false).map_err(failed)?;
        if let Some(Check::Mismatch { offset, wanted, found }) = written.check {
            return Err(Response::json(409, &json!({"ok":false, "error":{"code":"readback_mismatch", "message":format!("The controller did not keep the change (byte {offset}: wanted {wanted}, read {found}). Refresh before changing another setting.")}, "check":{"state":"mismatch", "offset":offset, "wanted":wanted, "found":found}})));
        }
    }
    let (left, right) = profile.rumble();
    Ok(json!({"state":"ready", "device":name, "firmware":firmware.controller, "receiver":firmware.receiver, "profile":slot,
        "profile_name":profile.name(), "verified":change.is_some(), "check":if change.is_some() { json!({"state":"match"}) } else { Value::Null },
        "values":{"report_rate":profile.report_rate().map(ReportRate::hz), "rumble_left":left, "rumble_right":right,
        "dpad_diagonal_lock":profile.flag(Flag::DpadDiagonalLock), "xinput_abxy_swap":profile.flag(Flag::XinputAbxySwap),
        "deadzone_left":profile.stick(Side::Left).response.deadzone.inner, "deadzone_right":profile.stick(Side::Right).response.deadzone.inner}}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use frame::{REPLY_ID, cmd, reply};
    use std::collections::VecDeque;

    struct Fake { bytes: [u8; frame::PROFILE_LEN], replies: VecDeque<Vec<u8>>, sent: Vec<Report>, stuck: bool }
    impl Fake {
        fn new(stuck: bool) -> Self { Self { bytes: [0; frame::PROFILE_LEN], replies: VecDeque::new(), sent: Vec::new(), stuck } }
    }
    impl Transport for Fake {
        fn send(&mut self, report: &Report) -> io::Result<()> {
            self.sent.push(*report);
            let offset = usize::from(u16::from_be_bytes([report[3], report[4]]));
            let len = usize::from(report[5]);
            let response = match report[1] {
                cmd::HEARTBEAT => return Ok(()),
                cmd::READ_FIRMWARE_VERSION => { let mut response = vec![0; 17]; response[..2].copy_from_slice(&[REPLY_ID, reply::FIRMWARE_VERSION]); response[4..7].copy_from_slice(b"123"); response },
                cmd::READ_CURRENT_PROFILE => vec![REPLY_ID, reply::CURRENT_PROFILE, 1],
                cmd::READ => [&[REPLY_ID, reply::DATA], &report[2..6], &self.bytes[offset..offset + len]].concat(),
                cmd::WRITE => {
                    if !self.stuck { self.bytes[offset..offset + len].copy_from_slice(&report[6..6 + len]); }
                    vec![REPLY_ID, reply::ACK, 0]
                }
                _ => panic!("unexpected controller command"),
            };
            self.replies.push_back(response);
            Ok(())
        }
        fn recv(&mut self, buf: &mut [u8; frame::REPORT_LEN], _: Duration) -> io::Result<Option<usize>> {
            let response = self.replies.pop_front().expect("every test command has an immediate reply");
            buf[..response.len()].copy_from_slice(&response);
            Ok(Some(response.len()))
        }
    }

    #[test]
    fn fixed_settings_read_back_and_preserve_unrelated_bytes() {
        for change in [Change::ReportRate(1000), Change::RumbleLeft(4), Change::RumbleRight(3), Change::DpadDiagonalLock(true),
            Change::XinputAbxySwap(true), Change::DeadzoneLeft(8), Change::DeadzoneRight(9)] {
            let mut fake = Fake::new(false);
            fake.bytes[679] = 0xa5;
            let mut controller = Controller::new(fake);
            let answer = profile(&mut controller, "Cyclone 2", Some(&change)).unwrap();
            assert_eq!(answer["verified"], true);
            assert_eq!(answer["check"]["state"], "match");
            let fake = controller.into_inner();
            let write = fake.sent.iter().rposition(|r| r[1] == cmd::WRITE).unwrap();
            assert!(fake.sent[write + 1..].iter().any(|r| r[1] == cmd::READ));
            assert_eq!(fake.bytes[679], 0xa5);
        }
    }

    #[test]
    fn failed_readback_never_claims_a_saved_setting() {
        let mut controller = Controller::new(Fake::new(true));
        let error = profile(&mut controller, "Cyclone 2", Some(&Change::ReportRate(1000))).unwrap_err();
        assert_eq!(error.status, 409);
    }

    #[test]
    fn changes_accept_only_fixed_fields_and_ranges() {
        for body in [r#"{"setting":"firmware","value":1}"#, r#"{"setting":"report-rate","value":1000,"node":"/dev/hidraw0"}"#,
            r#"{"setting":"dpad-diagonal-lock","value":1}"#] { assert!(serde_json::from_str::<Change>(body).is_err()); }
        assert!(Change::ReportRate(8000).validate().is_err());
        assert!(Change::DeadzoneLeft(21).validate().is_err());
        assert!(Change::RumbleRight(5).validate().is_err());
    }

    #[test]
    fn disabled_plugin_refuses_before_discovery() {
        let hub = Arc::new(Hub::new(0));
        let request = Request { method: "GET".into(), path: "/v1/gaming/gamesir".into(), query: vec![], headers: vec![], body: vec![], peer: "127.0.0.1".parse().unwrap() };
        assert_eq!(handle(&hub, &request).status, 403);
        let request = Request { path: "/v1/gaming/leadjoy".into(), ..request };
        assert_eq!(crate::leadjoy::handle(&hub, &request).status, 403);
    }
}
