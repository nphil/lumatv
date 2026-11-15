//! Xeno's vendor HID configuration interface; gameplay interfaces are never opened.
#![allow(unsafe_code)]
use crate::{http::{Request, Response, error_response}, hub::Hub};
use leadjoy_config::{Command, Controller, Lighting, Request as Query, RunMode, Transport, frame::Report};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fs::{self, File, OpenOptions}, io::{self, Read, Write}, os::{fd::AsRawFd, unix::fs::OpenOptionsExt}, path::{Path, PathBuf}, sync::Arc, time::{Duration, Instant}};
mod settings;

pub const ROUTES: &[(&str, &str, &str)] = &[
    ("GET", "/v1/gaming/leadjoy", "read documented Xeno settings selected by section/profile/sub/side/button; Gaming must be enabled"),
    ("POST", "/v1/gaming/leadjoy", "change one fixed Xeno setting {view,setting,value}, requiring fresh-read preservation and read-back"),
];
// Xeno Plus V54 master descriptor: FF80 usage 1, unnumbered 64-byte input/output reports.
const DESCRIPTOR: &[u8] = &[0x06,0x80,0xff,0x09,0x01,0xa1,0x01,0x09,0x02,0xa1,0x00,0x09,0x03,0x15,0x00,0x26,0xff,0x00,0x75,0x08,0x95,0x40,0x81,0x02,0x09,0x05,0x91,0x02,0xc0,0xc0];
static CONFIG: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Section { #[default] General, Profiles, Buttons, Sticks, Curves, Directions, Triggers, Rumble }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct View { section: Section, profile: u8, sub: u8, side: u8, button: u8, action: u8 }
impl Default for View {
    fn default() -> Self { Self { section: Section::General, profile: 1, sub: 0, side: 0, button: 1, action: 0 } }
}
impl View {
    fn validate(&self) -> Result<(), &'static str> {
        if !(1..=4).contains(&self.profile) || self.sub > 2 || self.side > 1 || self.action > 1 { return Err("invalid profile, configuration, side or action"); }
        if self.section == Section::Buttons && !settings::BUTTONS.iter().any(|(id,_)| *id == i64::from(self.button)) { return Err("this button is not remappable"); }
        if self.section == Section::Directions && !(1..=4).contains(&self.button) { return Err("invalid stick direction"); }
        Ok(())
    }
    fn query(request: &Request) -> Result<Self, &'static str> {
        let mut value = json!(Self::default());
        for (key, text) in &request.query {
            if key == "section" { value[key] = json!(text); }
            else if matches!(key.as_str(), "profile" | "sub" | "side" | "button" | "action") { value[key] = json!(text.parse::<u8>().map_err(|_| "invalid controller selector")?); }
            else { return Err("unknown controller selector"); }
        }
        let view: Self = serde_json::from_value(value).map_err(|_| "unknown controller section")?;
        view.validate()?; Ok(view)
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Setting { view: View, setting: String, value: Value }

fn discover(root: &Path, dev: &Path) -> io::Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(root) { Ok(entries) => entries, Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]), Err(e) => return Err(e) };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry?;
        let device = entry.path().join("device");
        let Ok(uevent) = fs::read_to_string(device.join("uevent")) else { continue };
        let Some(id) = uevent.lines().find_map(|line| line.strip_prefix("HID_ID=")) else { continue };
        let mut fields = id.split(':').map(|field| u32::from_str_radix(field, 16));
        if !matches!((fields.next(), fields.next(), fields.next(), fields.next()),
            (Some(Ok(3)), Some(Ok(0x4131)), Some(Ok(0x3519 | 0x8008)), None)) { continue; }
        if fs::read(device.join("report_descriptor")).is_ok_and(|bytes| bytes == DESCRIPTOR) { found.push(dev.join(entry.file_name())); }
    }
    found.sort();
    Ok(found)
}

struct Hid<'a> { file: File, hub: &'a Arc<Hub>, request: &'a Request }
impl Hid<'_> {
    fn wait(&self, events: i16, timeout: Duration) -> io::Result<bool> {
        let end = Instant::now() + timeout;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() { return Ok(false); }
            let mut fd = libc::pollfd { fd: self.file.as_raw_fd(), events, revents: 0 };
            // The file owns this descriptor throughout poll; there is exactly one initialized entry.
            let count = unsafe { libc::poll(&mut fd, 1, left.as_millis().saturating_add(1).min(i32::MAX as u128) as i32) };
            if count < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted { continue; }
                return Err(error);
            }
            if fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 { return Err(io::Error::new(io::ErrorKind::NotConnected, "controller disconnected")); }
            return Ok(count > 0 && fd.revents & events != 0);
        }
    }
}
impl Transport for Hid<'_> {
    fn send(&mut self, report: &Report, timeout: Duration) -> io::Result<()> {
        crate::gaming::check_enabled(self.hub, self.request).map_err(|_| io::Error::new(io::ErrorKind::PermissionDenied, "Gaming was switched off or the TV entered standby"))?;
        if !self.wait(libc::POLLOUT, timeout)? { return Err(io::Error::new(io::ErrorKind::TimedOut, "controller write timed out")); }
        crate::gaming::check_enabled(self.hub, self.request).map_err(|_| io::Error::new(io::ErrorKind::PermissionDenied, "Gaming was switched off or the TV entered standby"))?;
        let mut bytes = [0; 65];
        bytes[1..].copy_from_slice(report);
        if self.file.write(&bytes)? != bytes.len() { return Err(io::Error::new(io::ErrorKind::WriteZero, "short HID report write")); }
        Ok(())
    }
    fn recv(&mut self, report: &mut Report, timeout: Duration) -> io::Result<Option<usize>> {
        if !self.wait(libc::POLLIN, timeout)? { return Ok(None); }
        crate::gaming::check_enabled(self.hub, self.request).map_err(|_| io::Error::new(io::ErrorKind::PermissionDenied, "Gaming was switched off or the TV entered standby"))?;
        let mut bytes = [0; 65];
        let count = self.file.read(&mut bytes)?;
        let data = match count { 64 => &bytes[..64], 65 if bytes[0] == 0 => &bytes[1..], _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "unexpected vendor HID report length or id")) };
        report.copy_from_slice(data);
        Ok(Some(report.len()))
    }
}

fn failed(error: impl std::fmt::Display) -> Response { error_response(502, "controller_failed", &error.to_string()) }
fn ok(mut value: Value) -> Response { value["ok"] = json!(true); Response::json(200, &value) }
pub fn handle(hub: &Arc<Hub>, request: &Request) -> Response {
    if !matches!(request.method.as_str(), "GET" | "POST") { return error_response(405, "method_not_allowed", "that method is not served here").with_header("Allow", "GET, POST"); }
    if let Err(response) = crate::gaming::check_enabled(hub, request) { return response; }
    let setting = if request.method == "POST" {
        let setting: Setting = match serde_json::from_slice(&request.body) { Ok(value) => value, Err(error) => return error_response(400, "bad_request", &error.to_string()) };
        if let Err(error) = setting.view.validate() { return error_response(400, "bad_request", error); }
        Some(setting)
    } else { None };
    let view = match &setting { Some(setting) => setting.view.clone(), None => match View::query(request) {
        Ok(view) => view, Err(error) => return error_response(400, "bad_request", error),
    }};
    let Some(_lock) = CONFIG.try_lock() else { return error_response(409, "busy", "A controller request is already in progress"); };
    let devices = match discover(Path::new("/sys/class/hidraw"), Path::new("/dev")) { Ok(devices) => devices, Err(error) => return failed(error) };
    let Some(path) = devices.first() else { return ok(json!({"state":"missing"})); };
    let file = match OpenOptions::new().read(true).write(true).custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC).open(path) { Ok(file) => file, Err(error) => return failed(error) };
    let mut controller = Controller::new(Hid { file, hub, request }, Duration::from_secs(2));
    match profile(&mut controller, &view, setting.as_ref()) { Ok(value) => ok(value), Err(error) => error }
}

fn profile<T: Transport>(controller: &mut Controller<T>, selected: &View, setting: Option<&Setting>) -> Result<Value, Response> {
    selected.validate().map_err(|error| error_response(400, "bad_request", error))?;
    let version = controller.read(&Query::query(Command::DeviceVersion)).map_err(failed)?;
    let firmware = version.result.as_str().ok_or_else(|| failed("controller returned an invalid firmware version"))?;
    let product = controller.read(&Query::query(Command::ProductName)).map_err(failed)?;
    let name = product.result.get("m").and_then(Value::as_str).ok_or_else(|| failed("controller returned an invalid product name"))?;
    if name != "Xeno" { return Ok(json!({"state":"unsupported", "device":name, "firmware":firmware})); }
    if selected.section != Section::General {
        let lighting: Lighting = serde_json::from_value(controller.read(&Query::lighting()).map_err(failed)?.result).map_err(failed)?;
        if lighting.brightness > 100 { return Err(failed("controller returned an invalid logo brightness")); }
    }
    let count = controller.read(&Query::query(Command::ProfileCount)).map_err(failed)?.result.as_u64()
        .filter(|n| (1..=4).contains(n)).ok_or_else(|| failed("controller returned an unsupported profile count"))? as u8;
    let mut view = selected.clone();
    if view.profile > count {
        if setting.is_some() { return Err(error_response(400, "bad_request", "the controller does not have this profile")); }
        view.profile = 1;
    }
    let mut settings = settings::Settings::read(controller, &view)?;
    if let Some(setting) = setting {
        let field = settings.fields.iter().find(|field| field.key == setting.setting)
            .ok_or_else(|| error_response(400, "bad_request", "this setting is not editable in the selected section"))?;
        let group = &settings.groups[field.group];
        let next = field.apply(&setting.value, &group.value).map_err(|error| error_response(400, "bad_request", error))?;
        let change = group.change(next, &field.key, &view)?;
        let execution = controller.apply(&change, RunMode::Apply).map_err(|error| match error {
            leadjoy_config::Error::ReadBack => error_response(409, "readback_mismatch", "The controller did not keep the change. Refresh before changing another setting."),
            other => failed(other),
        })?;
        settings.groups[field.group].value = execution.read_back.ok_or_else(|| failed("controller write has no read-back"))?.result;
    }
    let (schema, values) = settings.display(&view, count)?;
    Ok(json!({"state":"ready", "device":name, "firmware":firmware, "view":view, "verified":setting.is_some(), "schema":schema, "values":values}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use leadjoy_config::frame::Decoder;
    use std::collections::{BTreeMap, VecDeque};
    struct Fake { decoder: Decoder, replies: VecDeque<Report>, values: BTreeMap<u8,Value>, product: &'static str, stuck: bool, wrong_side: bool, writes: usize }
    impl Fake {
        fn new() -> Self {
            let action = json!({"t":0,"v":[0,0,0,0],"n":"","r":0,"sp":10,"mr":0,"ms":0,"ri":0});
            let binding = json!({"a":1,"dn":action,"up":action});
            Self { decoder: Decoder::default(), replies: VecDeque::new(), product:"Xeno", stuck:false, wrong_side:false, writes:0,
                values: BTreeMap::from([
                    (30,json!(0)),(44,json!(500)),(46,json!(0)),(48,json!({"std":0,"ljod":3,"rjod":2,"ddl":0})),(50,json!(1)),
                    (51,json!({"b":1,"d2":{"a":0,"n":"Default","cfg":[{"c":0,"ko":1,"at":2,"k":[24]}]}})),
                    (60,json!({"b":1,"c":0,"d1":[{"k":1,"a":1,"dn":action,"up":{"t":1,"v":[3,0,0,0],"n":"Retained macro","r":1,"sp":5,"mr":4,"ms":17,"ri":300}}]})),
                    (70,json!({"b":1,"c":0,"d":0,"d1":{"co":[0,0],"ci":1,"rjc":19,"mm":0,"mse":50,"mo":0,"mx":0,"ry":0,"asf":0,"ct":1,"cv":[0,0,25,25,50,50,75,75,100,100],"rt":0,"rc":50,"cpl":50,"cpu":50,"cpr":50,"cpd":50,"tkl":binding,"tku":binding,"tkr":binding,"tkd":binding,"future":[7,9]}})),
                    (90,json!({"b":1,"c":0,"d1":{"mol":0,"dzl":[0,100],"vtl":50,"mor":2,"dzr":[5,95],"vtr":40}})),
                    (120,json!({"b":1,"c":0,"d1":{"enl":1,"grl":[0,100],"trl":[10,90],"enr":1,"grr":[5,80],"trr":[20,75]}})),
                    (190,json!({"brt":80,"rgb":[10,0,30]})),
                ])
            }
        }
    }
    impl Transport for Fake {
        fn send(&mut self, report: &Report, _: Duration) -> io::Result<()> {
            if let Some(payload) = self.decoder.push(report).unwrap() {
                let query = leadjoy_config::decode(&payload).unwrap();
                let command = query["c"].as_u64().unwrap() as u8;
                let write = match command { 31=>Some((30,"s")),45=>Some((44,"s")),47=>Some((46,"s")),49=>Some((48,"s")),61=>Some((60,"w")),71=>Some((70,"w")),91=>Some((90,"w")),121=>Some((120,"w")),190 if query["a"] == 1=>Some((190,"s")),_=>None };
                let result = if let Some((read,arg)) = write {
                    self.writes += 1; if !self.stuck { self.values.insert(read,query[arg].clone()); } None
                } else if matches!(command,52|53) {
                    self.writes += 1;
                    if !self.stuck {
                        let summary = self.values.get_mut(&51).unwrap();
                        let version = if summary.get("d2").is_some() { "d2" } else { "d1" };
                        if command == 52 { summary[version]["a"] = json!(1); } else { summary[version]["n"] = query["s"]["n"].clone(); }
                    }
                    None
                } else {
                    Some(match command {
                        12=>json!("V54"),17=>json!({"m":self.product,"s":0}),
                        _=> {
                            let mut value = self.values.get(&command).unwrap_or_else(|| panic!("unexpected command {query}")).clone();
                            for key in ["b","c","d"] { if let Some(selector) = query["g"].get(key) { value[key] = selector.clone(); } }
                            if command == 60 { value["d1"][0]["k"] = query["g"]["k"][0].clone(); }
                            if command == 70 && self.wrong_side { value["d"] = json!(3); }
                            value
                        }
                    })
                };
                let mut reply = json!({"c":command,"e":0}); if let Some(result) = result { reply["r"] = result; }
                self.replies.extend(leadjoy_config::encode(&reply).unwrap());
            }
            Ok(())
        }
        fn recv(&mut self, report: &mut Report, _: Duration) -> io::Result<Option<usize>> { *report = self.replies.pop_front().unwrap(); Ok(Some(report.len())) }
    }
    fn change(section: Section, setting: &str, value: Value) -> Setting { Setting { view: View { section, ..View::default() }, setting:setting.into(), value } }
    #[test]
    fn documented_writes_preserve_complete_fresh_objects_and_read_back() {
        for (section,key,value,command,path,wire) in [
            (Section::General,"logo-brightness",json!(65),190,"/brt",json!(65)),
            (Section::General,"logo-red",json!(15),190,"/rgb/0",json!(15)),
            (Section::General,"logo-green",json!(16),190,"/rgb/1",json!(16)),
            (Section::General,"logo-blue",json!(17),190,"/rgb/2",json!(17)),
            (Section::General,"polling",json!("1000"),44,"",json!(1000)),
            (Section::General,"precision",json!("128"),46,"",json!(128)),
            (Section::General,"dual-output",json!(true),30,"",json!(1)),
            (Section::General,"dpad-lock",json!(true),48,"/ddl",json!(1)),
            (Section::Profiles,"profile-name",json!("Racing"),51,"/d2/n",json!("Racing")),
            (Section::Profiles,"profile-activate",Value::Null,51,"/d2/a",json!(1)),
            (Section::Buttons,"key-0",json!("5"),60,"/d1/0/dn/v/0",json!(5)),
            (Section::Buttons,"turbo",json!("2"),60,"/d1/0/dn/r",json!(2)),
            (Section::Sticks,"anti-shake",json!(-3),70,"/d1/asf",json!(-3)),
            (Section::Sticks,"invert-x",json!(true),70,"/d1/mx",json!(1)),
            (Section::Curves,"curve-9",json!(90),70,"/d1/cv/9",json!(90)),
            (Section::Directions,"key-0",json!("7"),70,"/d1/tkl/dn/v/0",json!(7)),
            (Section::Triggers,"mode-r",json!("1"),90,"/d1/mor",json!(1)),
            (Section::Rumble,"range-l-1",json!(75),120,"/d1/grl/1",json!(75)),
        ] {
            let fake = Fake::new(); let mut expected = fake.values.clone();
            *expected.get_mut(&command).unwrap().pointer_mut(path).unwrap() = wire;
            let mut controller = Controller::new(fake,Duration::from_secs(1)); let setting = change(section,key,value);
            let actual = profile(&mut controller,&setting.view,Some(&setting)).unwrap();
            assert_eq!(actual["verified"],true,"{key}");
            let fake = controller.into_transport(); assert_eq!(fake.writes,1,"{key}"); assert_eq!(fake.values,expected,"{key}");
        }
    }
    #[test]
    fn mismatches_unsupported_devices_and_invalid_values_never_claim_success() {
        let setting = change(Section::General,"logo-red",json!(255));
        let mut fake = Fake::new(); fake.stuck = true;
        let mut controller = Controller::new(fake,Duration::from_secs(1));
        assert_eq!(profile(&mut controller,&setting.view,Some(&setting)).unwrap_err().status,409);
        let mut fake = Fake::new(); fake.product = "Other";
        let mut controller = Controller::new(fake,Duration::from_secs(1));
        assert_eq!(profile(&mut controller,&setting.view,Some(&setting)).unwrap()["state"],"unsupported");
        assert_eq!(controller.into_transport().writes,0);
        for setting in [change(Section::General,"logo-brightness",json!(101)),change(Section::General,"logo-red",json!(256)),
            change(Section::General,"firmware",json!(0)),change(Section::Sticks,"anti-shake",json!(-11)),
            change(Section::Buttons,"key-0",json!("255"))] {
            let mut controller = Controller::new(Fake::new(),Duration::from_secs(1));
            assert_eq!(profile(&mut controller,&setting.view,Some(&setting)).unwrap_err().status,400);
            assert_eq!(controller.into_transport().writes,0);
        }
        assert!(serde_json::from_value::<Setting>(json!({"view":View::default(),"setting":"logo-red","value":0,"path":"/dev/hidraw0"})).is_err());
    }
    #[test]
    fn selectors_profile_versions_and_unknown_action_payloads_are_preserved() {
        let mut controller = Controller::new(Fake::new(),Duration::from_secs(1));
        let view = View { section:Section::Profiles, profile:2, ..View::default() };
        assert_eq!(profile(&mut controller,&view,None).unwrap()["view"]["profile"],1);
        let setting = Setting { view, setting:"profile-name".into(), value:json!("Extra") };
        assert_eq!(profile(&mut controller,&setting.view,Some(&setting)).unwrap_err().status,400);
        let mut fake = Fake::new(); let summary = fake.values.get_mut(&51).unwrap();
        summary["d1"] = summary.as_object_mut().unwrap().remove("d2").unwrap();
        let mut controller = Controller::new(fake,Duration::from_secs(1));
        let setting = change(Section::Profiles,"profile-name",json!("Old format"));
        assert_eq!(profile(&mut controller,&setting.view,Some(&setting)).unwrap()["values"]["profile-name"],"Old format");
        let mut fake = Fake::new(); fake.wrong_side = true;
        let mut controller = Controller::new(fake,Duration::from_secs(1));
        let setting = change(Section::Sticks,"invert-x",json!(true));
        assert_eq!(profile(&mut controller,&setting.view,Some(&setting)).unwrap_err().status,502);
        assert_eq!(controller.into_transport().writes,0);
        let mut controller = Controller::new(Fake::new(),Duration::from_secs(1));
        let mut setting = change(Section::Buttons,"key-0",json!("5")); setting.view.action = 1;
        assert_eq!(profile(&mut controller,&setting.view,Some(&setting)).unwrap_err().status,400);
        assert_eq!(controller.into_transport().writes,0);
    }
    #[test]
    fn native_ui_fixtures_match_the_helper_contract() {
        let mut states = serde_json::Map::new();
        for section in [Section::General,Section::Profiles,Section::Buttons,Section::Sticks,Section::Curves,Section::Directions,Section::Triggers,Section::Rumble] {
            let mut controller = Controller::new(Fake::new(),Duration::from_secs(1));
            let value = profile(&mut controller,&View { section, ..View::default() },None).unwrap();
            states.insert(serde_json::to_value(section).unwrap().as_str().unwrap().into(),value);
        }
        let generated = Value::Object(states);
        if let Some(path) = std::env::var_os("LEADJOY_FIXTURE_OUTPUT") { fs::write(path,serde_json::to_vec(&generated).unwrap()).unwrap(); }
        else {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rustui/app/src/game/leadjoy-fixtures.json");
            assert_eq!(serde_json::from_slice::<Value>(&fs::read(path).unwrap()).unwrap(),generated);
        }
    }
    #[test]
    fn discovery_requires_both_usb_identity_and_vendor_descriptor() {
        let dir = tempfile::tempdir().unwrap();
        for (i, id, descriptor) in [(0,"0003:00004131:00003519",DESCRIPTOR), (1,"0003:00004131:00008008",DESCRIPTOR),
            (2,"0003:00004131:00003519",&[][..]), (3,"0003:0000054c:00000ce6",DESCRIPTOR)] {
            let device = dir.path().join(format!("hidraw{i}/device")); fs::create_dir_all(&device).unwrap();
            fs::write(device.join("uevent"), format!("HID_ID={id}\n")).unwrap(); fs::write(device.join("report_descriptor"), descriptor).unwrap();
        }
        assert_eq!(discover(dir.path(), Path::new("/dev")).unwrap(), [PathBuf::from("/dev/hidraw0"),PathBuf::from("/dev/hidraw1")]);
    }
}
