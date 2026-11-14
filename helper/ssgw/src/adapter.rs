//! The `com.webos.service.apiadapter` (and `.pub`) methods Home Assistant uses, done here with the calls LG's JavaScript makes.
//! Each reply is what LG's adapter would have answered the gateway; the gateway's own mapping to the client happens in
//! `session`.

use crate::util::{b64_encode, path_join, truthy};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

/// The `sourceId` this gateway's toasts carry; LumaTV's notification layer recognises it.
pub const TOAST_SOURCE_ID: &str = "com.webos.service.secondscreen.gateway";

const GET_VOLUME: &str = "luna://com.webos.service.audio/master/getVolume";
const MUTE_VOLUME: &str = "luna://com.webos.service.audio/master/muteVolume";
const GET_SETTINGS: &str = "luna://com.webos.settingsservice/getSystemSettings";
const SET_SETTINGS: &str = "luna://com.webos.settingsservice/setSystemSettings";
const FACTORY_OPT: &str = "luna://com.webos.service.factorymanager/getFactoryOpt";
const GET_CONFIGS: &str = "luna://com.webos.service.config/getConfigs";
const ALL_INPUTS: &str = "luna://com.webos.service.eim/getAllInputStatus";
const INPUT_STATUS: &str = "luna://com.webos.service.eim/getInputStatus";
const CHANNEL_LIST: &str = "luna://com.webos.service.iepg/getChannelList";
const CHANNEL_LIST_CHANGED: &str = "luna://com.webos.service.iepg/notifyChannelListChanged";
const SIMILAR_CHANNELS: &str = "luna://com.webos.service.iepg/getSimilarChannelList";
const CHANNEL_INFO: &str = "luna://com.webos.service.iepg/getChannelInfo";
const EVENT_LIST: &str = "luna://com.webos.service.iepg/getEventList";
const BIND: &str = "luna://com.webos.service.utp.broadcast/bind";
const CURRENT_CHANNEL: &str = "luna://com.webos.service.utp.broadcast/getCurrentChannel";
const APP_LAUNCH: &str = "luna://com.webos.applicationManager/launch";
const APP_INFO: &str = "luna://com.webos.applicationManager/getAppInfo";
const RUNNING: &str = "luna://com.webos.applicationManager/running";
const CLOSE_BY_APP_ID: &str = "luna://com.webos.applicationManager/closeByAppId";
const CREATE_TOAST: &str = "luna://com.webos.notification/createToast";

const ICON_MAX: usize = 102_400;
const BIND_ERROR: &str = "com.webos.service.utp/bind returns invalid result(response error)";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Status,
    Mute,
    SetMute,
    SetSoundOutput,
    SoundOutput,
    SystemInfo,
    Inputs,
    Channels,
    CurrentChannel,
    ProgramInfo,
    OpenChannel,
    SwitchInput,
    Launch,
    Close,
    Toast,
    AppLaunch,
}

impl Kind {
    /// The adapter method a concrete service and path name, when this gateway does it itself; `sub` is whether the call is a
    /// subscription. Methods that only make sense as requests are left to LG when subscribed.
    pub fn of(service: &str, path: &str, sub: bool) -> Option<Kind> {
        let kind = match (service, path) {
            ("com.webos.service.apiadapter", p) => match p {
                "/audio/getStatus" => Kind::Status,
                "/audio/getMute" => Kind::Mute,
                "/audio/setMute" => Kind::SetMute,
                "/audio/changeSoundOutput" => Kind::SetSoundOutput,
                "/audio/getSoundOutput" => Kind::SoundOutput,
                "/system/getSystemInfo" => Kind::SystemInfo,
                "/tv/getExternalInputList" => Kind::Inputs,
                "/tv/getChannelList" => Kind::Channels,
                "/tv/getCurrentChannel" => Kind::CurrentChannel,
                "/tv/getChannelProgramInfo" => Kind::ProgramInfo,
                "/tv/openChannel" => Kind::OpenChannel,
                "/tv/switchInput" => Kind::SwitchInput,
                "/system_launcher/launch" => Kind::Launch,
                "/system_launcher/close" => Kind::Close,
                "/system_notifications/createToast" => Kind::Toast,
                _ => return None,
            },
            ("com.webos.service.apiadapter.pub", "/application_manager/launch") => Kind::AppLaunch,
            _ => return None,
        };
        let streams = matches!(
            kind,
            Kind::Status | Kind::Mute | Kind::SoundOutput | Kind::Inputs | Kind::Channels | Kind::CurrentChannel | Kind::ProgramInfo
        );
        (streams || !sub).then_some(kind)
    }
}

pub enum Act {
    Call(u32, &'static str, Value),
    Sub(u32, &'static str, Value),
    Cancel(u32),
    /// A reply, as LG's adapter would send it to the gateway.
    Emit(Value),
    /// Nothing more comes from this operation.
    Finish,
}

/// Apps launched through this gateway, as LG's launch manager keeps them: the session id is the app id in base64.
#[derive(Default)]
pub struct Launches {
    sessions: HashMap<String, (String, HashSet<String>)>,
}

impl Launches {
    fn add(&mut self, app_id: &str, client: &str) -> String {
        let id = b64_encode(app_id.as_bytes());
        let entry = self.sessions.entry(id.clone()).or_insert_with(|| (app_id.to_owned(), HashSet::new()));
        entry.1.insert(client.to_owned());
        id
    }

    /// The app of the session a close names, when that client has one here.
    pub fn find(&self, payload: &Value, client: &str) -> Option<String> {
        if let Some(sid) = payload.get("sessionId").filter(|v| truthy(Some(v))).and_then(Value::as_str)
            && let Some((app, clients)) = self.sessions.get(sid)
            && clients.contains(client)
        {
            return Some(app.clone());
        }
        let app = payload.get("id").filter(|v| truthy(Some(v))).and_then(Value::as_str)?;
        self.sessions.values().find(|(a, clients)| a == app && clients.contains(client)).map(|(a, _)| a.clone())
    }

    fn remove_app(&mut self, app_id: &str) {
        self.sessions.retain(|_, (a, _)| a != app_id);
    }
}

/// Toast icons, written where the notification service can read them; the newest few are kept.
pub struct Icons {
    dir: PathBuf,
    seq: u64,
    files: VecDeque<PathBuf>,
}

const ICONS_KEPT: usize = 8;

impl Icons {
    pub fn new(dir: PathBuf) -> Icons {
        Icons { dir, seq: 0, files: VecDeque::new() }
    }

    fn write(&mut self, ext: Option<&str>, data: &[u8]) -> std::io::Result<String> {
        std::fs::create_dir_all(&self.dir)?;
        self.seq += 1;
        let name = match ext {
            Some(e) => format!("{}.{e}", self.seq),
            None => self.seq.to_string(),
        };
        let path = self.dir.join(name);
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o644).open(&path)?;
        f.write_all(data)?;
        self.files.push_back(path.clone());
        while self.files.len() > ICONS_KEPT {
            if let Some(old) = self.files.pop_front() {
                let _ = std::fs::remove_file(old);
            }
        }
        Ok(path.display().to_string())
    }
}

/// What an operation may touch besides its own calls.
pub struct Env<'a> {
    /// The client's stable id (from its key), for launch sessions.
    pub client: &'a str,
    pub launches: &'a mut Launches,
    pub icons: &'a mut Icons,
}

fn failed(v: &Value) -> bool {
    v.get("returnValue") == Some(&Value::Bool(false))
}

/// `respond(promise)` on success: `_.extend({returnValue: true}, result)`.
fn resolved(result: Option<&Value>) -> Value {
    let mut out = Map::new();
    out.insert("returnValue".into(), Value::Bool(true));
    if let Some(Value::Object(m)) = result {
        for (k, v) in m {
            out.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out)
}

/// `respond(promise)` on failure with a plain value.
fn rejected(err: &Value) -> Value {
    let mut out = json!({"returnValue": false});
    if let Value::Object(m) = err {
        let code = m.get("errorCode").filter(|c| truthy(Some(c)));
        if let Some(c) = code {
            out["errorCode"] = c.clone();
        }
        if let Some(t) = m.get("errorText").filter(|t| truthy(Some(t))) {
            out["errorText"] = t.clone();
        } else if let Some(c) = code {
            match c.as_f64() {
                Some(403.0) => out["errorText"] = json!("access denied"),
                Some(404.0) => out["errorText"] = json!("no such service or method"),
                _ => {}
            }
        }
    }
    out
}

/// `respond(promise)` when the JavaScript threw.
fn internal_error() -> Value {
    json!({"returnValue": false, "errorCode": -1000, "errorText": "internal error"})
}

fn error_obj(text: &str) -> Value {
    json!({"returnValue": false, "errorCode": -1000, "errorText": text})
}

fn with_sub(mut payload: Value, sub: bool) -> Value {
    if sub {
        payload["subscribe"] = Value::Bool(true);
    }
    payload
}

/// `addVolumeStatus`: copies `volumeStatus.muteStatus` and `.volume` up when both are present.
fn add_volume_status(v: &Value) -> Value {
    let mut out = v.clone();
    if let Some(vs) = v.get("volumeStatus").filter(|s| truthy(Some(s))).and_then(Value::as_object)
        && let (Some(m), Some(vol)) = (vs.get("muteStatus"), vs.get("volume"))
        && let Value::Object(o) = &mut out
    {
        o.insert("mute".into(), m.clone());
        o.insert("volume".into(), vol.clone());
    }
    out
}

/// `/([0-9]+_){5,6}[0-9]+/` anywhere in the string: six or more digit runs joined by single underscores.
fn valid_channel_id(s: &str) -> bool {
    // counts the digit groups of the current `digits(_digits)*` run
    let (mut groups, mut in_digits, mut after_sep) = (0, false, false);
    for c in s.chars() {
        if c.is_ascii_digit() {
            if !in_digits {
                groups = if after_sep { groups + 1 } else { 1 };
            }
            (in_digits, after_sep) = (true, false);
        } else {
            after_sep = c == '_' && in_digits;
            if !after_sep {
                groups = 0;
            }
            in_digits = false;
        }
        if groups >= 6 {
            return true;
        }
    }
    false
}

/// The `$`-patterns of `String.prototype.replace` with a string pattern, applied to the first match.
fn js_replace_first(s: &str, pat: &str, with: &str) -> String {
    let Some(at) = s.find(pat) else { return s.to_owned() };
    let (before, after) = (&s[..at], &s[at + pat.len()..]);
    let mut rep = String::new();
    let mut chars = with.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            rep.push(c);
            continue;
        }
        match chars.peek() {
            Some('$') => {
                chars.next();
                rep.push('$');
            }
            Some('&') => {
                chars.next();
                rep.push_str(pat);
            }
            Some('`') => {
                chars.next();
                rep.push_str(before);
            }
            Some('\'') => {
                chars.next();
                rep.push_str(after);
            }
            _ => rep.push('$'),
        }
    }
    format!("{before}{rep}{after}")
}

/// `String(v)` for the values a payload carries.
fn js_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// LG's `findExtension`: the text after the last dot, the whole string when there is none; nothing when it ends in a dot.
fn find_extension(v: &Value) -> Option<&str> {
    let s = v.as_str()?;
    match s.rfind('.') {
        _ if s.is_empty() => None,
        Some(i) if i == s.len() - 1 => None,
        Some(i) => Some(&s[i + 1..]),
        None => Some(s),
    }
}

/// Request payload keys this gateway never passes on to the bus.
fn strip_private(mut payload: Value) -> Value {
    if let Value::Object(m) = &mut payload {
        m.retain(|k, _| !k.starts_with('_'));
    }
    payload
}

pub struct Op {
    kind: Kind,
    sub: bool,
    payload: Value,
    first: bool,
    next_tag: u32,
    /// The tag of the call each step waits on; replies to any other tag are stale.
    main: u32,
    bind: u32,
    current: u32,
    info: u32,
    events: u32,
    launch: u32,
    /// For the channel list: the fetches in flight and whether each answers the first notification.
    fetches: HashMap<u32, bool>,
    prev_mute: Option<Value>,
    model: (Option<Value>, Option<Value>),
    subs: HashSet<u32>,
    channel_id: Value,
    record: Value,
    app_id: String,
}

impl Op {
    pub fn start(kind: Kind, payload: Value, sub: bool, env: &mut Env) -> (Op, Vec<Act>) {
        let mut op = Op {
            kind,
            sub,
            payload,
            first: true,
            next_tag: 1,
            main: 0,
            bind: 0,
            current: 0,
            info: 0,
            events: 0,
            launch: 0,
            fetches: HashMap::new(),
            prev_mute: None,
            model: (None, None),
            subs: HashSet::new(),
            channel_id: Value::Null,
            record: Value::Null,
            app_id: String::new(),
        };
        let mut acts = Vec::new();
        op.begin(env, &mut acts);
        (op, acts)
    }

    fn tag(&mut self) -> u32 {
        let t = self.next_tag;
        self.next_tag += 1;
        t
    }

    fn call(&mut self, acts: &mut Vec<Act>, uri: &'static str, payload: Value) -> u32 {
        let t = self.tag();
        acts.push(Act::Call(t, uri, payload));
        t
    }

    /// `chainRequest`: a subscription when the operation is one, else a call.
    fn request(&mut self, acts: &mut Vec<Act>, uri: &'static str, payload: Value) -> u32 {
        if self.sub {
            return self.call_sub(acts, uri, with_sub(payload, true));
        }
        self.call(acts, uri, payload)
    }

    fn done(acts: &mut Vec<Act>, reply: Value) {
        acts.push(Act::Emit(reply));
        acts.push(Act::Finish);
    }

    fn begin(&mut self, env: &mut Env, acts: &mut Vec<Act>) {
        let p = self.payload.clone();
        match self.kind {
            Kind::Status | Kind::Mute => {
                let sub = self.sub;
                self.main = self.request(acts, GET_VOLUME, json!({"subscribe": sub}));
            }
            Kind::SetMute => {
                self.main = self.call(acts, MUTE_VOLUME, json!({"mute": truthy(p.get("mute"))}));
            }
            Kind::SetSoundOutput => {
                if !truthy(p.get("output")) {
                    return Op::done(acts, rejected(&error_obj("missing \"output\" parameter")));
                }
                let body = json!({"category": "sound", "settings": {"soundOutput": p["output"]}});
                self.main = self.call(acts, SET_SETTINGS, body);
            }
            Kind::SoundOutput => {
                self.main = self.request(acts, GET_SETTINGS, json!({"category": "sound", "keys": ["soundOutput"]}));
            }
            Kind::SystemInfo => {
                self.main = self.call(acts, FACTORY_OPT, json!({"keys": ["modelName", "serialNumber"]}));
            }
            Kind::Inputs => {
                self.main = self.request(acts, ALL_INPUTS, json!({}));
            }
            Kind::Channels => {
                if self.sub {
                    self.main = self.call_sub(acts, CHANNEL_LIST_CHANGED, json!({"subscribe": true}));
                } else {
                    self.fetch_channels(acts, false);
                }
            }
            Kind::CurrentChannel => self.bind = self.call(acts, BIND, json!({})),
            Kind::ProgramInfo => {
                if truthy(p.get("channelId")) {
                    self.on_channel_id(p["channelId"].clone(), acts);
                } else {
                    self.bind = self.call(acts, BIND, json!({}));
                }
            }
            Kind::OpenChannel => {
                let id = p.get("channelId").filter(|v| truthy(Some(v)));
                if let Some(id) = id
                    && !id.as_str().is_some_and(valid_channel_id)
                {
                    return Op::done(acts, error_obj("invalid channel id"));
                }
                if !truthy(p.get("channelNumber")) && id.is_none() {
                    return Op::done(acts, error_obj("requires channelNumber or channelId"));
                }
                match id {
                    Some(id) => self.launch_channel(id.clone(), acts),
                    None => self.bind = self.call(acts, BIND, json!({})),
                }
            }
            Kind::SwitchInput => match p.get("inputId").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                Some(id) => self.main = self.call(acts, INPUT_STATUS, json!({"id": id})),
                None => Op::done(acts, error_obj("expected inputId")),
            },
            Kind::Launch => self.begin_launch(acts),
            Kind::Close => {
                self.app_id = env.launches.find(&p, env.client).unwrap_or_default();
                self.main = self.call(acts, RUNNING, json!({}));
            }
            Kind::Toast => self.begin_toast(env, acts),
            Kind::AppLaunch => {
                self.main = self.call(acts, APP_LAUNCH, strip_private(p));
            }
        }
    }

    fn call_sub(&mut self, acts: &mut Vec<Act>, uri: &'static str, payload: Value) -> u32 {
        let t = self.tag();
        self.subs.insert(t);
        acts.push(Act::Sub(t, uri, payload));
        t
    }

    fn fetch_channels(&mut self, acts: &mut Vec<Act>, first: bool) {
        let mode = self.payload.get("channelMode").filter(|m| truthy(Some(m))).cloned().unwrap_or(json!(""));
        let body = json!({"type": 2, "sort": 0, "channelGroup": ["All"], "channelMode": mode});
        let t = self.call(acts, CHANNEL_LIST, body);
        self.fetches.insert(t, first);
    }

    fn launch_channel(&mut self, channel_id: Value, acts: &mut Vec<Act>) {
        let body = json!({"id": "com.webos.app.livetv", "params": {"id": "channelChange", "channelId": channel_id}});
        self.launch = self.call(acts, APP_LAUNCH, body);
    }

    fn begin_launch(&mut self, acts: &mut Vec<Act>) {
        let mut p = strip_private(self.payload.clone());
        if let Value::Object(m) = &mut p {
            m.remove("subscribe");
            let mut params = match m.remove("params") {
                Some(Value::Object(o)) => o,
                _ => Map::new(),
            };
            params.remove("windowType");
            m.insert("params".into(), Value::Object(params));
        }
        self.payload = p;
        let content = self.payload.get("contentId").filter(|c| truthy(Some(c)));
        if content.is_some_and(|c| js_string(c).chars().any(|ch| !matches!(ch, '"' | '\'' | '\\'))) {
            match self.payload.get("id").filter(|v| truthy(Some(v))).cloned() {
                Some(id) => self.main = self.call(acts, APP_INFO, json!({"id": id})),
                None => Op::done(acts, json!({"returnValue": false})),
            }
        } else {
            let body = self.payload.clone();
            self.launch = self.call(acts, APP_LAUNCH, body);
        }
    }

    fn begin_toast(&mut self, env: &mut Env, acts: &mut Vec<Act>) {
        let p = &self.payload;
        if !truthy(p.get("message")) {
            return Op::done(acts, rejected(&error_obj("missing \"message\" parameter")));
        }
        let mut icon = None;
        if let (Some(ext), Some(data)) = (p.get("iconExtension").filter(|e| truthy(Some(e))), p.get("iconData").and_then(Value::as_str))
            && !data.is_empty()
        {
            let size = crate::util::b64_byte_length(data);
            if size > ICON_MAX {
                let text = format!("{size} bytes icon size greater than maximum allowed {ICON_MAX} bytes");
                return Op::done(acts, rejected(&error_obj(&text)));
            }
            // an extension LG's file cache would take but that is no safe file name is left off, as LG leaves off one it
            // cannot find
            let ext = find_extension(ext).filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
            match env.icons.write(ext, &crate::util::b64_decode_lenient(data)) {
                Ok(path) => icon = Some(path),
                Err(e) => {
                    crate::warn!("could not write a toast icon: {e}");
                    return Op::done(acts, internal_error());
                }
            }
        }
        let on_click = p.get("onClick").filter(|o| truthy(Some(o)));
        if let Some(oc) = on_click {
            if !(oc.get("appId").is_some_and(Value::is_string) || oc.get("target").is_some_and(Value::is_string)) {
                return Op::done(acts, rejected(&error_obj("onClick missing appID or target property")));
            }
            if oc.get("params").is_some_and(|v| truthy(Some(v)) && !v.is_object() && !v.is_array()) {
                return Op::done(acts, rejected(&error_obj("onClick params property is not an object")));
            }
        }
        let mut body = json!({"sourceId": TOAST_SOURCE_ID, "message": p["message"]});
        match on_click {
            Some(oc) => body["onclick"] = oc.clone(),
            None => body["noaction"] = Value::Bool(true),
        }
        if let Some(path) = icon {
            body["iconUrl"] = Value::String(path);
        }
        self.main = self.call(acts, CREATE_TOAST, body);
    }

    fn on_channel_id(&mut self, id: Value, acts: &mut Vec<Act>) {
        self.channel_id = id.clone();
        if self.info != 0 {
            acts.push(Act::Cancel(self.info));
        }
        self.info = self.request(acts, CHANNEL_INFO, json!({"channelId": id, "type": 2}));
    }

    /// A reply to one of this operation's calls.
    pub fn on_reply(&mut self, tag: u32, v: &Value, env: &mut Env) -> Vec<Act> {
        let mut acts = Vec::new();
        self.reply(tag, Some(v), env, &mut acts);
        acts
    }

    /// One of this operation's calls ended without a reply: a call counts as failed with an empty error, a subscription as
    /// silent.
    pub fn on_end(&mut self, tag: u32, env: &mut Env) -> Vec<Act> {
        let mut acts = Vec::new();
        if !self.subs.remove(&tag) {
            self.reply(tag, None, env, &mut acts);
        }
        acts
    }

    fn reply(&mut self, tag: u32, v: Option<&Value>, env: &mut Env, acts: &mut Vec<Act>) {
        let empty = json!({});
        let ok = v.is_some_and(|v| !failed(v));
        let v = v.unwrap_or(&empty);
        match self.kind {
            Kind::Status | Kind::Mute if tag == self.main => {
                if !ok {
                    // LG's adapter has no handler for this failure: the client hears nothing more
                    acts.push(Act::Cancel(tag));
                    return;
                }
                let status = add_volume_status(v);
                if self.kind == Kind::Status {
                    if self.sub {
                        acts.push(Act::Emit(status));
                    } else {
                        Op::done(acts, resolved(Some(&status)));
                    }
                    return;
                }
                let mute = status.get("mute").cloned();
                if !self.sub {
                    let mut r = Map::new();
                    if let Some(m) = mute {
                        r.insert("mute".into(), m);
                    }
                    return Op::done(acts, resolved(Some(&Value::Object(r))));
                }
                let changed = self.prev_mute.as_ref().is_some_and(|p| truthy(Some(p)) != truthy(mute.as_ref()));
                if self.first || changed {
                    let mut r = json!({"returnValue": true});
                    if let Some(m) = &mute {
                        r["mute"] = m.clone();
                    }
                    if self.first {
                        r["subscribed"] = Value::Bool(true);
                        self.first = false;
                    }
                    self.prev_mute = mute;
                    acts.push(Act::Emit(r));
                }
            }
            Kind::SetMute | Kind::SetSoundOutput | Kind::Toast | Kind::AppLaunch if tag == self.main => {
                Op::done(acts, if ok { resolved(Some(v)) } else { rejected(v) });
            }
            Kind::SoundOutput if tag == self.main => {
                if !ok {
                    acts.push(Act::Cancel(tag));
                    return;
                }
                let Some(settings) = v.get("settings").filter(|s| !s.is_null()) else { return };
                let mut r = Map::new();
                if let Some(o) = settings.get("soundOutput") {
                    r.insert("soundOutput".into(), o.clone());
                }
                if self.sub {
                    r.insert("subscribed".into(), Value::Bool(true));
                    acts.push(Act::Emit(Value::Object(r)));
                } else {
                    Op::done(acts, resolved(Some(&Value::Object(r))));
                }
            }
            Kind::SystemInfo if tag == self.main => {
                if !ok {
                    return Op::done(acts, rejected(v));
                }
                self.model = (v.get("modelName").cloned(), v.get("serialNumber").cloned());
                let names = ["tv.rmm.dvrReady", "tv.model.sysType", "tv.conti.supportProgramMode"];
                self.current = self.call(acts, GET_CONFIGS, json!({"configNames": names}));
            }
            Kind::SystemInfo if tag == self.current => {
                if !ok {
                    return Op::done(acts, rejected(v));
                }
                let Some(configs) = v.get("configs").filter(|c| !c.is_null()) else {
                    return Op::done(acts, internal_error());
                };
                let mut r = json!({"returnValue": true, "features": {"dvr": configs.get("tv.rmm.dvrReady") == Some(&Value::Bool(true))}});
                for (key, from) in [("receiverType", configs.get("tv.model.sysType")), ("programMode", configs.get("tv.conti.supportProgramMode"))] {
                    if let Some(x) = from {
                        r[key] = x.clone();
                    }
                }
                for (key, x) in [("modelName", &self.model.0), ("serialNumber", &self.model.1)] {
                    if let Some(x) = x {
                        r[key] = x.clone();
                    }
                }
                Op::done(acts, resolved(Some(&r)));
            }
            Kind::Inputs if tag == self.main => {
                if !ok {
                    return Op::done(acts, rejected(v));
                }
                let Some(devices) = v.get("devices").and_then(Value::as_array) else {
                    if !self.sub {
                        Op::done(acts, internal_error());
                    }
                    return;
                };
                let devices: Vec<Value> = devices.iter().map(adapt_device).collect();
                let mut r = json!({"devices": devices});
                if self.sub {
                    if self.first {
                        r["subscribed"] = Value::Bool(true);
                        self.first = false;
                    }
                    acts.push(Act::Emit(r));
                } else {
                    Op::done(acts, resolved(Some(&r)));
                }
            }
            Kind::Channels if tag == self.main => {
                if !ok {
                    return Op::done(acts, rejected(v));
                }
                let changed = truthy(v.get("changed")) && v.get("dataSource").and_then(Value::as_f64) == Some(0.0);
                if changed || self.first {
                    let first = self.first;
                    self.first = false;
                    self.fetch_channels(acts, first);
                }
            }
            Kind::Channels if self.fetches.contains_key(&tag) => {
                let first = self.fetches.remove(&tag).unwrap_or(false);
                if !ok {
                    return Op::done(acts, rejected(v));
                }
                if !self.sub {
                    return Op::done(acts, resolved(Some(v)));
                }
                let mut r = v.clone();
                if first && let Value::Object(m) = &mut r {
                    m.insert("subscribed".into(), Value::Bool(true));
                }
                acts.push(Act::Emit(r));
            }
            Kind::CurrentChannel | Kind::ProgramInfo | Kind::OpenChannel if tag == self.bind => {
                if !(ok && truthy(v.get("returnValue")) && truthy(v.get("binded"))) {
                    let err = json!({"returnValue": false, "errorCode": -1000, "errorText": BIND_ERROR, "response": v});
                    // getCurrentChannel's handler returns the error rather than throwing it
                    let reply = if self.kind == Kind::CurrentChannel { resolved(Some(&err)) } else { rejected(&err) };
                    return Op::done(acts, reply);
                }
                let mut body = Map::new();
                if let Some(b) = v.get("broadcastId") {
                    body.insert("broadcastId".into(), b.clone());
                }
                match self.kind {
                    Kind::CurrentChannel => {
                        body.insert("subscribe".into(), Value::Bool(self.sub));
                        self.current = self.request(acts, CURRENT_CHANNEL, Value::Object(body));
                    }
                    Kind::ProgramInfo => self.current = self.request(acts, CURRENT_CHANNEL, Value::Object(body)),
                    _ => {
                        let number = self.payload.get("channelNumber").filter(|n| truthy(Some(n))).cloned().unwrap_or(json!(""));
                        body.insert("channelNumber".into(), number);
                        body.insert("channelId".into(), json!("0_0_0_0_0_0_0"));
                        self.current = self.call(acts, SIMILAR_CHANNELS, Value::Object(body));
                    }
                }
            }
            Kind::CurrentChannel if tag == self.current => {
                if !ok {
                    return Op::done(acts, resolved(Some(v)));
                }
                let channel = v.get("channel").filter(|c| !c.is_null());
                if !self.sub {
                    return Op::done(acts, resolved(channel));
                }
                if self.first {
                    self.first = false;
                    let mut r = json!({"subscribed": true});
                    if let Some(Value::Object(m)) = channel {
                        for (k, x) in m {
                            r[k] = x.clone();
                        }
                    }
                    acts.push(Act::Emit(r));
                } else if let Some(c) = channel.filter(|c| truthy(Some(c))) {
                    acts.push(Act::Emit(c.clone()));
                }
            }
            Kind::ProgramInfo if tag == self.current => {
                if !ok {
                    return Op::done(acts, rejected(v));
                }
                match v.get("channel").filter(|c| !c.is_null()) {
                    Some(c) => self.on_channel_id(c.get("channelId").cloned().unwrap_or(Value::Null), acts),
                    None if !self.sub => Op::done(acts, internal_error()),
                    None => {}
                }
            }
            Kind::ProgramInfo if tag == self.info => {
                if !ok {
                    return Op::done(acts, rejected(v));
                }
                let found = v.get("channelList").and_then(Value::as_array).and_then(|list| {
                    list.iter().find(|c| c.get("channelId").is_some_and(|id| *id == self.channel_id)).cloned()
                });
                let Some(record) = found else {
                    return Op::done(acts, rejected(&error_obj("no such channel")));
                };
                self.record = record;
                let (y, m, d, h) = crate::util::local_now();
                let two = |n: i64| {
                    let s = format!("00{n}");
                    s[s.len() - 2..].to_owned()
                };
                let start = format!("{y},{},{},{},00,00", two(m), two(d + 1), two(h - 1));
                let end = format!("{y},{},{},{},00,00", two(m), two(d + 1), two(h + 2));
                if self.events != 0 {
                    acts.push(Act::Cancel(self.events));
                }
                let body = json!({"channelId": [self.channel_id], "type": 2, "localStartTime": start, "localEndTime": end});
                self.events = self.request(acts, EVENT_LIST, body);
            }
            Kind::ProgramInfo if tag == self.events => {
                if !ok {
                    return Op::done(acts, rejected(v));
                }
                let Some(list) = v.get("programList").and_then(Value::as_array) else { return };
                if list.iter().any(Value::is_null) {
                    return;
                }
                let programs: Vec<Value> = list.iter().map(|p| self.adapt_program(p)).collect();
                let mut r = json!({"channel": self.record, "programList": programs});
                if self.sub {
                    if self.first {
                        r["subscribed"] = Value::Bool(true);
                        self.first = false;
                    }
                    acts.push(Act::Emit(r));
                } else {
                    Op::done(acts, resolved(Some(&r)));
                }
            }
            Kind::OpenChannel if tag == self.current => {
                if !ok {
                    return Op::done(acts, rejected(v));
                }
                let id = v.get("channelList").and_then(Value::as_array).and_then(|l| l.first()).and_then(|c| c.get("channelId"));
                match id.filter(|i| truthy(Some(i))) {
                    Some(id) => self.launch_channel(id.clone(), acts),
                    None => Op::done(acts, rejected(&error_obj("no such channel"))),
                }
            }
            Kind::OpenChannel | Kind::SwitchInput if tag == self.launch => {
                Op::done(acts, if ok { resolved(Some(&json!({"returnValue": true}))) } else { rejected(v) });
            }
            Kind::SwitchInput if tag == self.main => match v.get("appId").filter(|a| ok && truthy(Some(a))) {
                Some(app) => self.launch = self.call(acts, APP_LAUNCH, json!({"id": app})),
                None => Op::done(acts, error_obj("no such input")),
            },
            Kind::Launch if tag == self.main => {
                if !ok {
                    return Op::done(acts, rejected(v));
                }
                let Some(info) = v.get("appInfo").filter(|i| !i.is_null()) else {
                    return Op::done(acts, internal_error());
                };
                if let Some(dl) = info.get("deeplinkingParams").filter(|d| truthy(Some(d))) {
                    let Some(dl) = dl.as_str() else { return Op::done(acts, internal_error()) };
                    let content = js_string(&self.payload["contentId"]);
                    let Ok(parsed) = serde_json::from_str::<Value>(&js_replace_first(dl, "$CONTENTID", &content)) else {
                        return Op::done(acts, internal_error());
                    };
                    if let (Value::Object(extra), Some(Value::Object(params))) = (parsed, self.payload.get_mut("params")) {
                        params.extend(extra);
                    }
                }
                let body = self.payload.clone();
                self.launch = self.call(acts, APP_LAUNCH, body);
            }
            Kind::Launch if tag == self.launch => {
                if !ok {
                    return Op::done(acts, json!({"returnValue": false}));
                }
                let Some(app) = self.payload.get("id").and_then(Value::as_str) else {
                    return Op::done(acts, internal_error());
                };
                let session = env.launches.add(app, env.client);
                Op::done(acts, resolved(Some(&json!({"id": app, "returnValue": true, "sessionId": session}))));
            }
            Kind::Close if tag == self.main => {
                if !ok {
                    return Op::done(acts, rejected(v));
                }
                let found = v.get("running").and_then(Value::as_array).and_then(|r| {
                    r.iter().find(|a| a.get("id").and_then(Value::as_str) == Some(self.app_id.as_str())).cloned()
                });
                match found {
                    Some(app) => self.launch = self.call(acts, CLOSE_BY_APP_ID, json!({"id": app["id"]})),
                    None => {
                        // LG's launch manager forgets a session once its app stops running
                        env.launches.remove_app(&self.app_id);
                        Op::done(acts, rejected(&json!({"errorCode": 403})));
                    }
                }
            }
            Kind::Close if tag == self.launch => {
                if !ok {
                    return Op::done(acts, rejected(v));
                }
                env.launches.remove_app(&self.app_id);
                let mut picked = Map::new();
                for k in ["returnValue", "errorCode", "errorText"] {
                    if let Some(x) = v.get(k) {
                        picked.insert(k.into(), x.clone());
                    }
                }
                Op::done(acts, resolved(Some(&Value::Object(picked))));
            }
            _ => {}
        }
    }

    fn adapt_program(&self, p: &Value) -> Value {
        let mut out = Map::new();
        out.insert("channelId".into(), self.channel_id.clone());
        for k in ["duration", "endTime", "eventId", "localEndTime", "localStartTime", "genre", "programId", "programName", "rating"] {
            if let Some(x) = p.get(k) {
                out.insert(k.into(), x.clone());
            }
        }
        if let Some(s) = self.record.get("signalChannelId") {
            out.insert("signalChannelId".into(), s.clone());
        }
        if let Some(s) = p.get("startTime") {
            out.insert("startTime".into(), s.clone());
        }
        out.insert("tableId".into(), json!(0));
        Value::Object(out)
    }
}

/// `onGetExternalInputListRequest`'s per-device rewrite.
fn adapt_device(d: &Value) -> Value {
    let mut d = d.clone();
    if let Value::Object(m) = &mut d {
        if let Some(active) = m.remove("activate") {
            m.insert("connected".into(), active);
            match m.remove("chosen") {
                Some(c) => {
                    m.insert("favorite".into(), c);
                }
                None => {
                    m.remove("favorite");
                }
            }
        }
        if let (Some(Value::String(icon)), Some(Value::String(prefix))) = (m.get("icon"), m.get("iconPrefix"))
            && !icon.is_empty()
            && !prefix.is_empty()
        {
            let joined = path_join(prefix, icon);
            m.insert("icon".into(), Value::String(joined));
            m.remove("iconPrefix");
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        launches: Launches,
        icons: Icons,
        _dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Fixture {
            let dir = tempfile::tempdir().unwrap();
            Fixture { launches: Launches::default(), icons: Icons::new(dir.path().join("icons")), _dir: dir }
        }

        fn env(&mut self) -> Env<'_> {
            Env { client: "c1", launches: &mut self.launches, icons: &mut self.icons }
        }
    }

    fn emitted(acts: &[Act]) -> Vec<Value> {
        acts.iter().filter_map(|a| if let Act::Emit(v) = a { Some(v.clone()) } else { None }).collect()
    }

    fn calls(acts: &[Act]) -> Vec<(u32, &'static str, Value, bool)> {
        acts.iter()
            .filter_map(|a| match a {
                Act::Call(t, u, p) => Some((*t, *u, p.clone(), false)),
                Act::Sub(t, u, p) => Some((*t, *u, p.clone(), true)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn mute_subscription_reports_first_and_changes_only() {
        let mut f = Fixture::new();
        let (mut op, acts) = Op::start(Kind::Mute, json!({}), true, &mut f.env());
        let (tag, uri, body, sub) = calls(&acts).remove(0);
        assert_eq!((uri, sub, body), (GET_VOLUME, true, json!({"subscribe": true})));
        let vol = |m: bool| json!({"returnValue": true, "volumeStatus": {"muteStatus": m, "volume": 5}});
        assert_eq!(emitted(&op.on_reply(tag, &vol(false), &mut f.env())), [json!({"mute": false, "returnValue": true, "subscribed": true})]);
        assert!(emitted(&op.on_reply(tag, &vol(false), &mut f.env())).is_empty());
        assert_eq!(emitted(&op.on_reply(tag, &vol(true), &mut f.env())), [json!({"mute": true, "returnValue": true})]);
    }

    #[test]
    fn status_request_adds_volume_and_mute() {
        let mut f = Fixture::new();
        let (mut op, acts) = Op::start(Kind::Status, json!({}), false, &mut f.env());
        let (tag, _, body, sub) = calls(&acts).remove(0);
        assert_eq!((body, sub), (json!({"subscribe": false}), false));
        let reply = json!({"returnValue": true, "volumeStatus": {"muteStatus": false, "volume": 9}});
        let acts = op.on_reply(tag, &reply, &mut f.env());
        assert_eq!(emitted(&acts)[0]["volume"], 9);
        assert!(matches!(acts.last(), Some(Act::Finish)));
    }

    #[test]
    fn current_channel_errors_resolve_with_the_error() {
        let mut f = Fixture::new();
        let (mut op, acts) = Op::start(Kind::CurrentChannel, json!({}), true, &mut f.env());
        let (bind, uri, _, _) = calls(&acts).remove(0);
        assert_eq!(uri, BIND);
        let none = json!({"errorCode": -101, "returnValue": false, "errorText": "There is no active broadcast to bind"});
        let out = emitted(&op.on_reply(bind, &none, &mut f.env()));
        assert_eq!(out[0]["returnValue"], false);
        assert_eq!(out[0]["errorText"], BIND_ERROR);
        assert_eq!(out[0]["response"]["errorCode"], -101);

        let (mut op, acts) = Op::start(Kind::CurrentChannel, json!({}), true, &mut f.env());
        let (bind, _, _, _) = calls(&acts).remove(0);
        let acts = op.on_reply(bind, &json!({"returnValue": true, "binded": true, "broadcastId": "b1"}), &mut f.env());
        let (cur, uri, body, sub) = calls(&acts).remove(0);
        assert_eq!((uri, body, sub), (CURRENT_CHANNEL, json!({"broadcastId": "b1", "subscribe": true}), true));
        let ch = json!({"returnValue": true, "channel": {"channelId": "1_2_3_4_5_6", "channelName": "One"}});
        assert_eq!(emitted(&op.on_reply(cur, &ch, &mut f.env())), [json!({"subscribed": true, "channelId": "1_2_3_4_5_6", "channelName": "One"})]);
        assert_eq!(emitted(&op.on_reply(cur, &ch, &mut f.env())), [ch["channel"].clone()]);
    }

    #[test]
    fn program_info_follows_the_channel_and_rebuilds_programs() {
        let mut f = Fixture::new();
        let (mut op, acts) = Op::start(Kind::ProgramInfo, json!({"channelId": "7_1_2_3_4_5"}), false, &mut f.env());
        let (info, uri, body, _) = calls(&acts).remove(0);
        assert_eq!((uri, body), (CHANNEL_INFO, json!({"channelId": "7_1_2_3_4_5", "type": 2})));
        let list = json!({"returnValue": true, "channelList": [{"channelId": "7_1_2_3_4_5", "signalChannelId": "s7", "_id": "x"}]});
        let acts = op.on_reply(info, &list, &mut f.env());
        let (events, uri, body, _) = calls(&acts).remove(0);
        assert_eq!((uri, &body["channelId"]), (EVENT_LIST, &json!(["7_1_2_3_4_5"])));
        assert_eq!(body["localStartTime"].as_str().unwrap().split(',').count(), 6);
        let progs = json!({"returnValue": true, "programList": [{"programName": "News", "_id": "p", "startTime": "s"}]});
        let out = emitted(&op.on_reply(events, &progs, &mut f.env()));
        assert_eq!(
            out[0]["programList"],
            json!([{"channelId": "7_1_2_3_4_5", "programName": "News", "signalChannelId": "s7", "startTime": "s", "tableId": 0}])
        );
        assert_eq!(out[0]["channel"]["_id"], "x");
    }

    #[test]
    fn launch_records_a_session_that_close_finds() {
        let mut f = Fixture::new();
        let (mut op, acts) = Op::start(Kind::Launch, json!({"id": "netflix", "params": {"windowType": "x", "a": 1}, "_clientKey": "k"}), false, &mut f.env());
        let (launch, uri, body, _) = calls(&acts).remove(0);
        assert_eq!((uri, body), (APP_LAUNCH, json!({"id": "netflix", "params": {"a": 1}})));
        let out = emitted(&op.on_reply(launch, &json!({"returnValue": true}), &mut f.env()));
        assert_eq!(out[0], json!({"id": "netflix", "returnValue": true, "sessionId": "bmV0ZmxpeA=="}));
        assert_eq!(f.launches.find(&json!({"sessionId": "bmV0ZmxpeA=="}), "c1").as_deref(), Some("netflix"));
        assert!(f.launches.find(&json!({"id": "netflix"}), "c2").is_none());

        let (mut op, acts) = Op::start(Kind::Close, json!({"id": "netflix"}), false, &mut f.env());
        let (running, _, _, _) = calls(&acts).remove(0);
        let out = emitted(&op.on_reply(running, &json!({"returnValue": true, "running": []}), &mut f.env()));
        assert_eq!(out[0], json!({"returnValue": false, "errorCode": 403, "errorText": "access denied"}));
        assert!(f.launches.find(&json!({"id": "netflix"}), "c1").is_none());
    }

    #[test]
    fn launch_merges_deep_link_params() {
        let mut f = Fixture::new();
        let (mut op, acts) = Op::start(Kind::Launch, json!({"id": "yt", "contentId": "abc"}), false, &mut f.env());
        let (info, uri, _, _) = calls(&acts).remove(0);
        assert_eq!(uri, APP_INFO);
        let reply = json!({"returnValue": true, "appInfo": {"deeplinkingParams": "{\"contentTarget\":\"$CONTENTID\"}"}});
        let acts = op.on_reply(info, &reply, &mut f.env());
        assert_eq!(calls(&acts)[0].2["params"], json!({"contentTarget": "abc"}));
    }

    #[test]
    fn toasts_check_their_input_and_carry_the_gateway_source() {
        let mut f = Fixture::new();
        let (_, acts) = Op::start(Kind::Toast, json!({"message": ""}), false, &mut f.env());
        assert_eq!(emitted(&acts)[0]["errorText"], "missing \"message\" parameter");
        let big = "A".repeat(136_540);
        let (_, acts) = Op::start(Kind::Toast, json!({"message": "m", "iconData": big, "iconExtension": "png"}), false, &mut f.env());
        assert_eq!(emitted(&acts)[0]["errorText"], "102405 bytes icon size greater than maximum allowed 102400 bytes");
        let (_, acts) = Op::start(Kind::Toast, json!({"message": "m", "onClick": {"params": 1}}), false, &mut f.env());
        assert_eq!(emitted(&acts)[0]["errorText"], "onClick missing appID or target property");
        let (_, acts) = Op::start(Kind::Toast, json!({"message": "hi", "iconData": "iVBORw==", "iconExtension": ".png"}), false, &mut f.env());
        let body = &calls(&acts)[0].2;
        assert_eq!((&body["sourceId"], &body["noaction"]), (&json!(TOAST_SOURCE_ID), &json!(true)));
        let path = body["iconUrl"].as_str().unwrap();
        assert!(path.ends_with("/1.png"));
        assert_eq!(std::fs::read(path).unwrap(), b"\x89PNG");
    }

    #[test]
    fn channel_ids_and_inputs_follow_lg_rules() {
        assert!(valid_channel_id("3_15_2_2_1_1_1"));
        assert!(valid_channel_id("x1_2_3_4_5_6y"));
        assert!(!valid_channel_id("1_2_3_4_5"));
        assert!(!valid_channel_id("1_2__3_4_5_6"));
        let dev = adapt_device(&json!({"activate": true, "chosen": false, "icon": "hdmi.png", "iconPrefix": "/usr/icons/"}));
        assert_eq!(dev, json!({"connected": true, "favorite": false, "icon": "/usr/icons/hdmi.png"}));
        assert_eq!(js_replace_first("{\"a\":\"$CONTENTID\"}", "$CONTENTID", "x$&y"), "{\"a\":\"x$CONTENTIDy\"}");
        assert_eq!(Kind::of("com.webos.service.apiadapter", "/system_launcher/launch", true), None);
        assert_eq!(Kind::of("com.webos.service.apiadapter", "/audio/getStatus", true), Some(Kind::Status));
    }
}
