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
enum Section { #[default] General, Profiles, Buttons, Macros, Sticks, Curves, Directions, Triggers, Motion, Gestures, Rumble }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct View { section: Section, profile: u8, sub: u8, side: u8, button: u8, action: u8, #[serde(default)] step: u16, #[serde(default = "first_page")] page: u16 }
fn first_page() -> u16 { 1 }
impl Default for View {
    fn default() -> Self { Self { section: Section::General, profile: 1, sub: 0, side: 0, button: 1, action: 0, step: 1, page: 1 } }
}
impl View {
    fn validate(&self) -> Result<(), &'static str> {
        if !(1..=4).contains(&self.profile) || self.sub > 2 || self.side > 1 || self.action > 1 || self.page == 0 { return Err("invalid profile, configuration, side, action or page"); }
        if self.section == Section::Buttons && !settings::BUTTONS.iter().any(|(id,_)| *id == i64::from(self.button)) { return Err("this button is not remappable"); }
        if self.section == Section::Directions && !(1..=4).contains(&self.button) { return Err("invalid stick direction"); }
        if self.section == Section::Gestures && !(1..=6).contains(&self.button) { return Err("invalid motion gesture"); }
        if self.section == Section::Macros && self.button == 0 { return Err("invalid macro"); }
        Ok(())
    }
    fn query(request: &Request) -> Result<Self, &'static str> {
        let mut value = json!(Self::default());
        for (key, text) in &request.query {
            if key == "section" { value[key] = json!(text); }
            else if matches!(key.as_str(), "profile" | "sub" | "side" | "button" | "action") { value[key] = json!(text.parse::<u8>().map_err(|_| "invalid controller selector")?); }
            else if matches!(key.as_str(), "step" | "page") { value[key] = json!(text.parse::<u16>().map_err(|_| "invalid controller selector")?); }
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
    let mut settings = settings::Settings::read(controller, &mut view)?;
    if let Some(setting) = setting {
        if view != *selected { return Err(error_response(409, "changed", "The controller's macros or gestures changed. Refresh before editing.")); }
        let field = settings.fields.iter().find(|field| field.key == setting.setting)
            .ok_or_else(|| error_response(400, "bad_request", "this setting is not editable in the selected section"))?;
        let next = field.apply(&setting.value, &settings.groups[field.group].value).map_err(|error| error_response(400, "bad_request", error))?;
        let (key, index) = (field.key.clone(), field.group);
        let (changes, written) = settings.changes(field, next, &view)?;
        let mut read_back = None;
        for (i, change) in changes.iter().enumerate() {
            read_back = controller.apply(change, RunMode::Apply).map_err(|error| match error {
                // Only a macro edit has a second write, made after its delete.
                _ if i > 0 => error_response(502, "rewrite_failed", "The controller removed the macro to rewrite it but did not confirm the new version. Refresh before editing."),
                leadjoy_config::Error::ReadBack => error_response(409, "readback_mismatch", "The controller did not keep the change. Refresh before changing another setting."),
                // A mode switch re-enumerates the controller, which can drop the read-back.
                leadjoy_config::Error::Unverified(_) if key == "mode" => error_response(502, "unverified", "The controller did not answer after the mode request. Refresh to confirm its mode."),
                leadjoy_config::Error::Unverified(_) => error_response(502, "unverified", "The controller did not answer the check. Refresh to see what it kept."),
                other => failed(other),
            })?.read_back;
        }
        // A written macro goes to the end of the list, and a name listed twice would play the older macro.
        if let Some(name) = written {
            let found: Vec<usize> = settings::macros(controller)?.iter().enumerate().filter(|(_, (n, _))| *n == name).map(|(i, _)| i).collect();
            let [i] = found[..] else { return Err(error_response(409, "readback_mismatch", "The controller does not list the macro exactly once. Refresh before changing another setting.")) };
            view.button = u8::try_from(i + 1).unwrap_or(u8::MAX);
        }
        match key.as_str() {
            "macro-new" | "delete-macro" => view.step = 1,
            "step-insert" => view.step = view.step.saturating_add(1),
            _ => {}
        }
        // A changed target or key set can change which fields exist.
        if view.section == Section::Macros || key.starts_with("sub-key-") || matches!(key.as_str(), "target" | "macro-playback" | "response") { settings = settings::Settings::read(controller, &mut view)?; }
        else { settings.groups[index].value = read_back.ok_or_else(|| failed("controller write has no read-back"))?.result; }
    }
    let (schema, values) = settings.display(&view, count)?;
    Ok(json!({"state":"ready", "device":name, "firmware":firmware, "view":view, "verified":setting.is_some(), "schema":schema, "values":values}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use leadjoy_config::frame::Decoder;
    use std::collections::{BTreeMap, VecDeque};
    struct Fake { decoder: Decoder, replies: VecDeque<Report>, values: BTreeMap<u8,Value>, macros: Vec<Value>, product: &'static str, stuck: bool, wrong_side: bool, reconnect: bool, gone: bool, writes: usize, last_sub: Option<Value> }
    impl Fake {
        fn new() -> Self {
            let action = json!({"t":0,"v":[0,0,0,0],"n":"","r":0,"sp":10,"mr":0,"ms":0,"ri":0});
            let binding = json!({"a":1,"dn":action,"up":action});
            let gesture = |e: u8| json!({"e":e,"dn":action,"up":action});
            Self { decoder: Decoder::default(), replies: VecDeque::new(), product:"Xeno", stuck:false, wrong_side:false, reconnect:false, gone:false, writes:0, last_sub:None,
                values: BTreeMap::from([
                    (30,json!(0)),(42,json!({"m":0,"l":1})),(44,json!(500)),(46,json!(0)),(48,json!({"std":0,"ljod":1,"rjod":2,"ddl":0})),(50,json!(1)),
                    (51,json!({"b":1,"d2":{"a":0,"n":"Default","cfg":[{"c":2,"ko":0,"at":1,"k":[25,26],"future":"sub2"},{"c":0,"ko":1,"at":0,"k":[24]},{"c":1,"ko":1,"at":0,"k":[24],"future":"sub1"}]}})),
                    (60,json!({"b":1,"c":0,"d1":[{"k":1,"a":1,"dn":action,"up":{"t":1,"v":[3,0,0,0],"n":"Retained macro","r":1,"sp":5,"mr":4,"ms":17,"ri":300}}]})),
                    (70,json!({"b":1,"c":0,"d":0,"d1":{"co":[0,0],"ci":1,"rjc":19,"mm":0,"mse":50,"mo":0,"mx":0,"ry":0,"asf":0,"ct":1,"cv":[0,0,25,25,50,50,75,75,100,100],"rt":0,"rc":50,"cpl":50,"cpu":50,"cpr":50,"cpd":50,"tkl":binding,"tku":binding,"tkr":binding,"tkd":binding,"future":[7,9]}})),
                    (90,json!({"b":1,"c":0,"d1":{"mol":0,"dzl":[0,100],"vtl":50,"mor":2,"dzr":[5,95],"vtr":40}})),
                    (100,json!({"b":1,"c":0,"d1":{"sj":3,"mlj":0,"tm":2,"tk":[9,10],"kao":0,"cv":[0,0,100,100],"rt":0,"rc":50,"se":60,"hvr":50,"had":0,"hsr":0,"hrr":1,"vr":0,"st":1,"fs":0,
                        "ec":[{"et":0,"acc":30,"ag":20,"a":2},{"et":1,"acc":35,"ag":25,"a":2},{"et":2,"acc":40,"ag":30,"a":0}],"el":[gesture(0),gesture(1),gesture(4),gesture(5)]}})),
                    (120,json!({"b":1,"c":0,"d1":{"enl":1,"grl":[0,100],"trl":[10,90],"enr":1,"grr":[5,80],"trr":[20,75]}})),
                    (190,json!({"brt":80,"rgb":[10,0,30]})),
                ]),
                macros: vec![
                    json!({"n":"Burst","s":2,"t":0,"d":[0,1,40,80,1,0,4,0,50,1],"k":[0,0]}),
                    json!({"n":"Combo","s":1,"t":1,"d":[3,0,0,0,0]}),
                ],
            }
        }
    }
    impl Transport for Fake {
        fn send(&mut self, report: &Report, _: Duration) -> io::Result<()> {
            if let Some(payload) = self.decoder.push(report).unwrap() {
                let query = leadjoy_config::decode(&payload).unwrap();
                let command = query["c"].as_u64().unwrap() as u8;
                if command == 42 && self.gone { return Err(io::Error::new(io::ErrorKind::NotConnected, "controller disconnected")); }
                let write = match command { 31=>Some((30,"s")),45=>Some((44,"s")),47=>Some((46,"s")),49=>Some((48,"s")),61=>Some((60,"w")),71=>Some((70,"w")),91=>Some((90,"w")),101=>Some((100,"w")),121=>Some((120,"w")),190 if query["a"] == 1=>Some((190,"s")),_=>None };
                let mut error = 0;
                let result = if let Some((read,arg)) = write {
                    self.writes += 1; if !self.stuck { self.values.insert(read,query[arg].clone()); } None
                } else if matches!(command,43|52|53|54|172|173) {
                    self.writes += 1;
                    if command == 54 { self.last_sub = Some(query["s"].clone()); }
                    if !self.stuck {
                        match command {
                            43 => { self.values.get_mut(&42).unwrap()["m"] = query["s"]["m"].clone(); self.gone = self.reconnect; }
                            54 => {
                                let entries = self.values.get_mut(&51).unwrap()["d2"]["cfg"].as_array_mut().unwrap();
                                if let Some(entry) = entries.iter_mut().find(|entry| entry["c"] == query["s"]["c"]) {
                                    for key in ["ko","at"] { if let Some(value) = query["s"].get(key) { entry[key] = value.clone(); } }
                                    if let Some(keys) = query["s"]["k"].as_array() {
                                        let order = [1,2,28,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,3,29,30,31,32];
                                        entry["k"] = json!(order.into_iter().filter(|id| keys.iter().any(|key| key.as_i64() == Some(i64::from(*id)))).collect::<Vec<_>>());
                                    }
                                }
                            }
                            // As on V54, c172 appends even under a name already in use, and c171 answers with the first match.
                            172 => self.macros.push(query["w"].clone()),
                            173 => { if let Some(i) = self.macros.iter().position(|m| m["n"] == query["n"]) { self.macros.remove(i); } }
                            _ => {
                                let summary = self.values.get_mut(&51).unwrap();
                                let version = if summary.get("d2").is_some() { "d2" } else { "d1" };
                                if command == 52 { summary[version]["a"] = json!(1); } else { summary[version]["n"] = query["s"]["n"].clone(); }
                            }
                        }
                    }
                    None
                } else {
                    match command {
                        12=>Some(json!("V54")),17=>Some(json!({"m":self.product,"s":0})),
                        170=>Some(json!(self.macros.iter().map(|m| json!({"n":m["n"],"k":m.get("k").cloned().unwrap_or(json!([0,0])),"s":m["s"]})).collect::<Vec<_>>())),
                        171=>{ let found = self.macros.iter().find(|m| m["n"] == query["n"]).cloned(); if found.is_none() { error = 1; } found }
                        _=> {
                            let mut value = self.values.get(&command).unwrap_or_else(|| panic!("unexpected command {query}")).clone();
                            for key in ["b","c","d"] { if let Some(selector) = query["g"].get(key) { value[key] = selector.clone(); } }
                            if command == 60 { value["d1"][0]["k"] = query["g"]["k"][0].clone(); }
                            if command == 70 && self.wrong_side { value["d"] = json!(3); }
                            Some(value)
                        }
                    }
                };
                let mut reply = json!({"c":command,"e":error}); if let Some(result) = result { reply["r"] = result; }
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
            (Section::General,"left-directions",json!("2"),48,"/ljod",json!(2)),
            (Section::General,"mode",json!("3"),42,"/m",json!(3)),
            (Section::Profiles,"profile-name",json!("Racing"),51,"/d2/n",json!("Racing")),
            (Section::Profiles,"profile-activate",Value::Null,51,"/d2/a",json!(1)),
            (Section::Buttons,"key-0",json!("5"),60,"/d1/0/dn/v/0",json!(5)),
            (Section::Buttons,"turbo",json!("2"),60,"/d1/0/dn/r",json!(2)),
            (Section::Buttons,"target",json!("1"),60,"/d1/0/dn",json!({"t":1,"v":[0,0,0,0],"n":"Burst","r":1,"sp":0,"mr":1,"ms":2,"ri":0})),
            (Section::Sticks,"anti-shake",json!(-3),70,"/d1/asf",json!(-3)),
            (Section::Sticks,"invert-x",json!(true),70,"/d1/mx",json!(1)),
            (Section::Directions,"key-0",json!("7"),70,"/d1/tkl/dn/v/0",json!(7)),
            (Section::Triggers,"mode-r",json!("1"),90,"/d1/mor",json!(1)),
            (Section::Rumble,"range-l-1",json!(75),120,"/d1/grl/1",json!(75)),
            (Section::Motion,"sensitivity",json!(80),100,"/d1/se",json!(80)),
            (Section::Motion,"trigger-key-1",json!("8"),100,"/d1/tk/1",json!(8)),
            (Section::Motion,"angle-2",json!(45),100,"/d1/ec/2/ag",json!(45)),
            (Section::Gestures,"key-0",json!("4"),100,"/d1/el/0/dn/v/0",json!(4)),
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
            change(Section::Buttons,"key-0",json!("255")),change(Section::General,"mode",json!("5")),
            change(Section::Motion,"sensitivity",json!(101)),change(Section::Gestures,"key-0",json!("255"))] {
            let mut controller = Controller::new(Fake::new(),Duration::from_secs(1));
            assert_eq!(profile(&mut controller,&setting.view,Some(&setting)).unwrap_err().status,400);
            assert_eq!(controller.into_transport().writes,0);
        }
        assert!(serde_json::from_value::<Setting>(json!({"view":View::default(),"setting":"logo-red","value":0,"path":"/dev/hidraw0"})).is_err());
        // A mode switch re-enumerates the controller; the acknowledged change is reported as unconfirmed, not as done.
        let mut fake = Fake::new(); fake.reconnect = true;
        let mut controller = Controller::new(fake,Duration::from_secs(1));
        let setting = change(Section::General,"mode",json!("2"));
        let error = profile(&mut controller,&setting.view,Some(&setting)).unwrap_err();
        assert_eq!(error.status,502);
        assert!(String::from_utf8_lossy(&error.body).contains("\"unverified\""));
        assert_eq!(controller.into_transport().values[&42],json!({"m":2,"l":1}));
    }
    #[test]
    fn macros_are_created_reshaped_assigned_and_deleted_through_read_back() {
        let post = |controller: &mut Controller<Fake>, view: &View, setting: &str, value: Value| profile(controller,view,Some(&Setting { view:view.clone(), setting:setting.into(), value }));
        let next = |reply: &Value| serde_json::from_value::<View>(reply["view"].clone()).unwrap();
        let mut controller = Controller::new(Fake::new(),Duration::from_secs(1));
        // An edit deletes the macro and writes it again, which moves it to the end of the list.
        let reply = post(&mut controller,&View { section:Section::Macros, step:2, ..View::default() },"step-duration",json!(120)).unwrap();
        let mut view = next(&reply);
        assert_eq!((view.button,view.step,reply["values"]["step-duration"].clone()),(2,2,json!(120)));
        view.step = 1;
        let reply = post(&mut controller,&view,"step-insert",Value::Null).unwrap();
        assert_eq!(next(&reply).step,2);
        view.step = 3;
        let reply = post(&mut controller,&view,"step-remove",Value::Null).unwrap();
        view = next(&reply); assert_eq!(view.step,2);
        for name in [" Burst ".to_owned(),"a/b".into(),"x".repeat(33),"é".repeat(17)] { assert_eq!(post(&mut controller,&view,"macro-new",json!(name)).unwrap_err().status,400,"{name}"); }
        let reply = post(&mut controller,&view,"macro-new",json!("Dash ")).unwrap();
        assert_eq!((next(&reply).button,reply["values"]["step-duration"].clone()),(3,json!(100)));
        // Advanced macros remain untouched, including deletion.
        view = View { section:Section::Macros, button:1, ..View::default() };
        let shown = profile(&mut controller,&view,None).unwrap();
        assert!(shown["values"].get("step-key").is_none());
        assert_eq!(schema_field(&shown,"macro-unavailable")["type"],"note");
        assert_eq!(post(&mut controller,&view,"step-insert",Value::Null).unwrap_err().status,400);
        assert_eq!(post(&mut controller,&view,"delete-macro",Value::Null).unwrap_err().status,400);
        view.step = 9;
        assert_eq!(post(&mut controller,&view,"step-duration",json!(10)).unwrap_err().status,409);
        // The release action plays a macro the controller no longer has; choosing another carries its step count.
        let buttons = View { section:Section::Buttons, action:1, ..View::default() };
        assert_eq!(profile(&mut controller,&buttons,None).unwrap()["values"]["macro"],"Retained macro");
        assert_eq!(post(&mut controller,&buttons,"macro",json!("Dash")).unwrap()["values"]["macro-interval"],300);
        // The repeat interval is offered only while the macro repeats.
        let press = View { section:Section::Buttons, ..View::default() };
        assert!(post(&mut controller,&press,"target",json!("1")).unwrap()["values"].get("macro-interval").is_none());
        assert_eq!(post(&mut controller,&press,"macro-playback",json!("3")).unwrap()["values"]["macro-interval"],0);
        let fake = controller.into_transport();
        assert_eq!(fake.writes,10);
        assert_eq!(fake.values[&60]["d1"][0]["up"],json!({"t":1,"v":[3,0,0,0],"n":"Dash","r":1,"sp":5,"mr":4,"ms":1,"ri":300}));
        assert_eq!(fake.macros,[json!({"n":"Combo","s":1,"t":1,"d":[3,0,0,0,0]}),json!({"n":"Burst","s":2,"t":0,"d":[0,1,40,80,1,0,1,40,80,1]}),json!({"n":"Dash","s":1,"t":0,"d":[0,1,0,100,1]})]);
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
    fn schema_field<'a>(reply: &'a Value, key: &str) -> &'a Value {
        reply["schema"]["sections"][0]["fields"].as_array().unwrap().iter().find(|field| field["key"] == key).unwrap()
    }

    #[test]
    fn action_targets_reset_incompatible_slots_and_preserve_the_complete_object() {
        for (section, command, path) in [
            (Section::Buttons,60,"/d1/0/dn"), (Section::Directions,70,"/d1/tkl/dn"), (Section::Gestures,100,"/d1/el/0/dn"),
        ] {
            let mut fake = Fake::new();
            let action = fake.values.get_mut(&command).unwrap().pointer_mut(path).unwrap();
            action["t"] = json!(2); action["v"] = json!([1,2,3,4]); action["future"] = json!({"retained":[9,8]});
            let mut expected = fake.values.clone();
            let mut controller = Controller::new(fake,Duration::from_secs(1));
            let post = |controller: &mut Controller<Fake>, key, value| {
                let setting = change(section,key,value);
                profile(controller,&setting.view,Some(&setting)).unwrap()
            };
            let keyboard = post(&mut controller,"target",json!("3"));
            assert_eq!(keyboard["values"]["key-0"],"0");
            assert_eq!(post(&mut controller,"key-0",json!("4"))["values"]["key-0"],"4");
            let mouse = post(&mut controller,"target",json!("4"));
            assert_eq!(mouse["values"]["key-0"],"0");
            assert_eq!(post(&mut controller,"key-1",json!("5"))["values"]["key-1"],"5");
            let gamepad = post(&mut controller,"target",json!("2"));
            assert_eq!(gamepad["values"]["key-1"],"0");
            expected.get_mut(&command).unwrap().pointer_mut(path).unwrap()["v"] = json!([0,0,0,0]);
            let fake = controller.into_transport();
            assert_eq!(fake.writes,5);
            assert_eq!(fake.values,expected,"{section:?}");
        }
    }

    #[test]
    fn unreported_or_invalid_fields_are_notes_and_cannot_write() {
        for (target, setting, value) in [(3,"key-0",json!("40")),(3,"key-0",json!("255")),(4,"key-0",json!("6")),(2,"target",json!("5"))] {
            let mut fake = Fake::new(); fake.values.get_mut(&60).unwrap()["d1"][0]["dn"]["t"] = json!(target);
            let mut controller = Controller::new(fake,Duration::from_secs(1));
            let setting = change(Section::Buttons,setting,value);
            assert_eq!(profile(&mut controller,&setting.view,Some(&setting)).unwrap_err().status,400);
            assert_eq!(controller.into_transport().writes,0);
        }
        let mut fake = Fake::new();
        let stick = fake.values.get_mut(&70).unwrap()["d1"].as_object_mut().unwrap();
        stick.remove("mse"); stick.insert("mom".into(),json!(0)); stick.insert("mx".into(),json!("invalid"));
        let mut controller = Controller::new(fake,Duration::from_secs(1));
        let view = View { section:Section::Sticks, ..View::default() };
        let shown = profile(&mut controller,&view,None).unwrap();
        for key in ["release-cut","mouse-sensitivity","mouse-mode","invert-x"] {
            assert_eq!(schema_field(&shown,key)["type"],"note","{key}");
            let setting = Setting { view:view.clone(), setting:key.into(), value:json!(true) };
            assert_eq!(profile(&mut controller,&view,Some(&setting)).unwrap_err().status,400,"{key}");
        }
        let connection = change(Section::General,"connection-type",json!(1));
        assert_eq!(profile(&mut controller,&connection.view,Some(&connection)).unwrap_err().status,400);
        assert_eq!(controller.into_transport().writes,0);
        let mut fake = Fake::new();
        fake.values.get_mut(&70).unwrap()["d1"]["mse"] = json!("not a number");
        let mut controller = Controller::new(fake,Duration::from_secs(1));
        let setting = change(Section::Sticks,"mouse-sensitivity",json!(50));
        let shown = profile(&mut controller,&setting.view,None).unwrap();
        assert_eq!(schema_field(&shown,"mouse-sensitivity")["type"],"note");
        assert_eq!(profile(&mut controller,&setting.view,Some(&setting)).unwrap_err().status,400);
        assert_eq!(controller.into_transport().writes,0);
    }

    #[test]
    fn repeated_gestures_are_not_ambiguous_edit_targets() {
        let mut fake = Fake::new();
        let events = fake.values.get_mut(&100).unwrap()["d1"]["el"].as_array_mut().unwrap();
        events.push(events[0].clone());
        let original = fake.values.clone();
        let mut controller = Controller::new(fake,Duration::from_secs(1));
        let view = View { section:Section::Gestures, ..View::default() };
        let shown = profile(&mut controller,&view,None).unwrap();
        assert_eq!(shown["view"]["button"],2);
        assert_eq!(schema_field(&shown,"other-gestures")["type"],"note");
        let invalid = Setting { view:view.clone(), setting:"target".into(), value:json!("3") };
        assert_eq!(profile(&mut controller,&view,Some(&invalid)).unwrap_err().status,409);
        let fake = controller.into_transport();
        assert_eq!(fake.writes,0); assert_eq!(fake.values,original);
    }

    #[test]
    fn stick_aliases_and_reported_boolean_fields_keep_the_fresh_spelling() {
        for (key, first, second, value, wire) in [
            ("mouse-mode","mo","mom",json!("1"),json!(1)),
            ("invert-x","mx","xr",json!(true),json!(1)),
            ("invert-y","ry","yr",json!(true),json!(1)),
            ("release-cut","rjc","rjc",json!(true),json!(1)),
            ("mouse-sensitivity","mse","mse",json!(75),json!(75)),
        ] {
            for spelling in [first,second] {
                let mut fake = Fake::new();
                let stick = fake.values.get_mut(&70).unwrap()["d1"].as_object_mut().unwrap();
                let previous = stick.remove(first).unwrap();
                stick.insert(spelling.into(),if first == "rjc" { json!(0) } else { previous });
                let mut expected = fake.values.clone();
                expected.get_mut(&70).unwrap()["d1"][spelling] = wire.clone();
                let mut controller = Controller::new(fake,Duration::from_secs(1));
                let setting = change(Section::Sticks,key,value.clone());
                assert_eq!(profile(&mut controller,&setting.view,Some(&setting)).unwrap()["verified"],true);
                let fake = controller.into_transport();
                assert_eq!(fake.writes,1); assert_eq!(fake.values,expected,"{key}/{spelling}");
            }
        }
    }

    #[test]
    fn all_reported_motion_keys_are_paged_without_resizing() {
        let mut fake = Fake::new();
        fake.values.get_mut(&100).unwrap()["d1"]["tk"] = json!(vec![1; 61]);
        fake.values.get_mut(&100).unwrap()["d1"]["tk"][60] = json!(28);
        let mut expected = fake.values.clone();
        let mut controller = Controller::new(fake,Duration::from_secs(1));
        let first = View { section:Section::Motion, ..View::default() };
        let page = profile(&mut controller,&first,None).unwrap();
        assert_eq!(schema_field(&page,"view-page")["max"],3);
        assert!(page["values"].get("trigger-key-23").is_some());
        assert!(page["values"].get("trigger-key-24").is_none());
        let absent = Setting { view:first, setting:"trigger-key-60".into(), value:json!("2") };
        assert_eq!(profile(&mut controller,&absent.view,Some(&absent)).unwrap_err().status,400);
        let last = View { section:Section::Motion, page:3, ..View::default() };
        let shown = profile(&mut controller,&last,None).unwrap();
        assert_eq!(shown["values"]["trigger-key-60"],"28");
        assert!(schema_field(&shown,"trigger-key-60")["options"].as_array().unwrap().iter().any(|item| item["id"] == "28" && item["label"].as_str().unwrap().contains("retained")));
        let invalid = Setting { view:last.clone(), setting:"trigger-key-60".into(), value:json!("29") };
        assert_eq!(profile(&mut controller,&last,Some(&invalid)).unwrap_err().status,400);
        let setting = Setting { view:last, setting:"trigger-key-60".into(), value:json!("2") };
        let result = profile(&mut controller,&setting.view,Some(&setting)).unwrap();
        assert_eq!(result["values"]["trigger-key-60"],"2");
        expected.get_mut(&100).unwrap()["d1"]["tk"][60] = json!(2);
        let fake = controller.into_transport();
        assert_eq!(fake.writes,1); assert_eq!(fake.values,expected);
    }

    #[test]
    fn keyboard_choice_windows_wrap_at_the_global_ends() {
        let mut fake = Fake::new();
        fake.values.get_mut(&60).unwrap()["d1"][0]["dn"]["t"] = json!(3);
        fake.values.get_mut(&60).unwrap()["d1"][0]["dn"]["v"] = json!([0,227,4,0]);
        let mut controller = Controller::new(fake,Duration::from_secs(1));
        let shown = profile(&mut controller,&View { section:Section::Buttons, ..View::default() },None).unwrap();
        for (key, current, previous, next) in [("key-0","0","227","53"),("key-1","227","226","0")] {
            let options = schema_field(&shown,key)["options"].as_array().unwrap();
            assert!(options.len() <= 32);
            let at = options.iter().position(|option| option["id"] == current).unwrap();
            assert_eq!(options[at - 1]["id"],previous);
            assert_eq!(options[at + 1]["id"],next);
        }
    }

    #[test]
    fn unknown_curve_scales_are_not_writable_and_survive_other_changes() {
        let mut fake = Fake::new();
        fake.values.get_mut(&70).unwrap()["d1"]["co"] = json!([-32767,32767]);
        fake.values.get_mut(&70).unwrap()["d1"]["cv"] = json!([0,0,65535,65535]);
        let mut expected = fake.values.clone();
        let mut controller = Controller::new(fake,Duration::from_secs(1));
        for setting in [change(Section::Curves,"curve-1",json!(20)),change(Section::Sticks,"centre-x",json!(20))] {
            assert_eq!(profile(&mut controller,&setting.view,Some(&setting)).unwrap_err().status,400);
        }
        let inactive = change(Section::Curves,"curvature",json!(35));
        assert_eq!(profile(&mut controller,&inactive.view,Some(&inactive)).unwrap_err().status,400);
        let response = change(Section::Curves,"response",json!("1"));
        assert_eq!(profile(&mut controller,&response.view,Some(&response)).unwrap()["verified"],true);
        assert_eq!(profile(&mut controller,&inactive.view,Some(&inactive)).unwrap()["verified"],true);
        expected.get_mut(&70).unwrap()["d1"]["rt"] = json!(1);
        expected.get_mut(&70).unwrap()["d1"]["rc"] = json!(35);
        let setting = change(Section::Sticks,"mouse-sensitivity",json!(75));
        assert_eq!(profile(&mut controller,&setting.view,Some(&setting)).unwrap()["verified"],true);
        expected.get_mut(&70).unwrap()["d1"]["mse"] = json!(75);
        let fake = controller.into_transport();
        assert_eq!(fake.writes,3); assert_eq!(fake.values,expected);
    }

    #[test]
    fn stick_smooth_response_requires_a_reported_smooth_mode() {
        for (mode,response) in [(0,1),(2,1),(1,9)] {
            let mut fake = Fake::new();
            fake.values.get_mut(&70).unwrap()["d1"]["ct"] = json!(mode);
            fake.values.get_mut(&70).unwrap()["d1"]["rt"] = json!(response);
            let mut controller = Controller::new(fake,Duration::from_secs(1));
            let view = View { section:Section::Curves, ..View::default() };
            let shown = profile(&mut controller,&view,None).unwrap();
            for key in ["response","curvature"] {
                assert_eq!(schema_field(&shown,key)["type"],"note");
                let setting = Setting { view:view.clone(), setting:key.into(), value:json!("1") };
                assert_eq!(profile(&mut controller,&view,Some(&setting)).unwrap_err().status,400);
            }
            assert_eq!(controller.into_transport().writes,0);
        }
    }

    #[test]
    fn subconfig_uses_the_reported_selector_and_retains_every_other_field() {
        for (sub,index) in [(1,2),(2,0)] {
            let fake = Fake::new(); let mut expected = fake.values.clone();
            let mut controller = Controller::new(fake,Duration::from_secs(1));
            let view = View { section:Section::Profiles, sub, ..View::default() };
            let shown = profile(&mut controller,&view,None).unwrap();
            assert!(shown["values"].get("sub-keep").is_some());
            assert!(shown["values"].get("sub-logic").is_none());
            let setting = Setting { view, setting:"sub-activation".into(), value:json!(if sub == 1 { "1" } else { "0" }) };
            assert_eq!(profile(&mut controller,&setting.view,Some(&setting)).unwrap()["verified"],true);
            expected.get_mut(&51).unwrap()["d2"]["cfg"][index]["at"] = json!(u8::from(sub == 1));
            let keep = Setting { view:setting.view, setting:"sub-keep".into(), value:json!(sub == 2) };
            assert_eq!(profile(&mut controller,&keep.view,Some(&keep)).unwrap()["verified"],true);
            expected.get_mut(&51).unwrap()["d2"]["cfg"][index]["ko"] = json!(u8::from(sub == 2));
            let fake = controller.into_transport();
            assert_eq!(fake.writes,2); assert_eq!(fake.values,expected);
            let mut sent = expected[&51]["d2"]["cfg"][index].clone();
            sent["b"] = json!(1);
            assert_eq!(fake.last_sub.unwrap(),sent);
            assert!(sent.get("kao").is_none());
        }
    }

    #[test]
    fn reported_subconfig_key_logic_is_preserved_but_not_editable() {
        let mut fake = Fake::new();
        fake.values.get_mut(&51).unwrap()["d2"]["cfg"][2]["kao"] = json!(1);
        let mut expected = fake.values.clone();
        let mut controller = Controller::new(fake,Duration::from_secs(1));
        let view = View { section:Section::Profiles, sub:1, ..View::default() };
        let bad = Setting { view:view.clone(), setting:"sub-logic".into(), value:json!("0") };
        assert_eq!(profile(&mut controller,&view,Some(&bad)).unwrap_err().status,400);
        let setting = Setting { view, setting:"sub-activation".into(), value:json!("1") };
        assert_eq!(profile(&mut controller,&setting.view,Some(&setting)).unwrap()["verified"],true);
        expected.get_mut(&51).unwrap()["d2"]["cfg"][2]["at"] = json!(1);
        let fake = controller.into_transport();
        assert_eq!(fake.writes,1); assert_eq!(fake.values,expected);
        assert_eq!(fake.last_sub.unwrap()["kao"],1);
    }

    #[test]
    fn subconfig_keys_preserve_unknowns_and_follow_the_reported_set() {
        let mut fake = Fake::new();
        fake.values.get_mut(&51).unwrap()["d2"]["cfg"][2]["k"] = json!([28,24]);
        let mut expected = fake.values.clone();
        let mut controller = Controller::new(fake,Duration::from_secs(1));
        let view = View { section:Section::Profiles, sub:1, ..View::default() };
        let shown = profile(&mut controller,&view,None).unwrap();
        assert_eq!(shown["values"]["sub-key-0"],"28");
        assert!(!schema_field(&shown,"sub-key-1")["options"].as_array().unwrap().iter().any(|item| item["id"] == "28"));
        let bad = Setting { view:view.clone(), setting:"sub-key-1".into(), value:json!("28") };
        assert_eq!(profile(&mut controller,&view,Some(&bad)).unwrap_err().status,400);
        let setting = Setting { view, setting:"sub-key-1".into(), value:json!("4") };
        assert_eq!(profile(&mut controller,&setting.view,Some(&setting)).unwrap()["verified"],true);
        expected.get_mut(&51).unwrap()["d2"]["cfg"][2]["k"] = json!([28,4]);
        let fake = controller.into_transport();
        assert_eq!(fake.writes,1); assert_eq!(fake.values,expected);
        assert_eq!(fake.last_sub.unwrap()["k"],json!([28,4]));

        let mut controller = Controller::new(Fake::new(),Duration::from_secs(1));
        let view = View { section:Section::Profiles, sub:1, ..View::default() };
        let post = |controller: &mut Controller<Fake>, key: &str, value: &str| {
            profile(controller,&view,Some(&Setting { view:view.clone(), setting:key.into(), value:json!(value) })).unwrap()
        };
        assert_eq!(post(&mut controller,"sub-key-1","1")["values"]["sub-key-0"],"1", "firmware may reorder keys");
        assert_eq!(post(&mut controller,"sub-key-0","0")["values"]["sub-key-0"],"24");
        assert_eq!(post(&mut controller,"sub-key-0","0")["values"]["sub-key-0"],"0");
        let fake = controller.into_transport();
        assert_eq!(fake.values[&51]["d2"]["cfg"][2]["k"],json!([]));
        assert_eq!(fake.writes,3);
    }

    #[test]
    fn malformed_or_ambiguous_subconfigs_never_write() {
        for case in 0..3 {
            let mut fake = Fake::new();
            let entries = fake.values.get_mut(&51).unwrap()["d2"]["cfg"].as_array_mut().unwrap();
            match case {
                0 => { entries[2].as_object_mut().unwrap().remove("at"); }
                1 => { let duplicate = entries[2].clone(); entries.push(duplicate); }
                _ => entries[2]["k"] = json!([24,24]),
            }
            let mut controller = Controller::new(fake,Duration::from_secs(1));
            let view = View { section:Section::Profiles, sub:1, ..View::default() };
            let shown = profile(&mut controller,&view,None).unwrap();
            assert_eq!(schema_field(&shown,"sub-activation-unavailable")["type"],"note");
            let setting = Setting { view, setting:"sub-keep".into(), value:json!(true) };
            assert_eq!(profile(&mut controller,&setting.view,Some(&setting)).unwrap_err().status,400);
            assert_eq!(controller.into_transport().writes,0);
        }
    }
    #[test]
    fn native_ui_fixtures_match_the_helper_contract() {
        let mut states = serde_json::Map::new();
        for section in [Section::General,Section::Profiles,Section::Buttons,Section::Macros,Section::Sticks,Section::Curves,Section::Directions,Section::Triggers,Section::Motion,Section::Gestures,Section::Rumble] {
            let mut controller = Controller::new(Fake::new(),Duration::from_secs(1));
            let value = profile(&mut controller,&View { section, ..View::default() },None).unwrap();
            states.insert(serde_json::to_value(section).unwrap().as_str().unwrap().into(),value);
        }
        for (name, section, page) in [("keyboard",Section::Buttons,1),("mouse",Section::Buttons,1),("motion-long",Section::Motion,2),("sticks-missing",Section::Sticks,1),("profiles-sub",Section::Profiles,1)] {
            let mut fake = Fake::new();
            match name {
                "keyboard" => {
                    fake.values.get_mut(&60).unwrap()["d1"][0]["dn"]["t"] = json!(3);
                    fake.values.get_mut(&60).unwrap()["d1"][0]["dn"]["v"] = json!([0,227,4,0]);
                }
                "mouse" => {
                    fake.values.get_mut(&60).unwrap()["d1"][0]["dn"]["t"] = json!(4);
                    fake.values.get_mut(&60).unwrap()["d1"][0]["dn"]["v"] = json!([1,2,5,0]);
                }
                "motion-long" => fake.values.get_mut(&100).unwrap()["d1"]["tk"] = json!(vec![1; 61]),
                "sticks-missing" => {
                    let stick = fake.values.get_mut(&70).unwrap()["d1"].as_object_mut().unwrap();
                    stick.remove("mse"); stick.remove("mo"); stick.insert("xr".into(),json!(1));
                }
                "profiles-sub" => {},
                _ => unreachable!(),
            }
            let mut controller = Controller::new(fake,Duration::from_secs(1));
            let sub = u8::from(name == "profiles-sub");
            let value = profile(&mut controller,&View { section, page, sub, ..View::default() },None).unwrap();
            states.insert(name.into(),value);
        }
        let generated = Value::Object(states);
        if let Some(path) = std::env::var_os("LEADJOY_FIXTURE_OUTPUT") { fs::write(path,serde_json::to_string_pretty(&generated).unwrap() + "\n").unwrap(); }
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
