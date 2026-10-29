//! Support (`settings/PageSupport.qml`; LG: Support: TV information, Software Update, Screen and Sound Self-diagnosis, Memory Optimizer,
//! Privacy & Terms, Reset to Initial Settings), with App Permissions and Delete My Personal Information under it. Everything LG's TV
//! Information shows is read from the TV's own services, one call each, and shown as LG shows it (no row has a value it was not given:
//! an unanswered one is an en dash). The software update is shown, not run: checking sends the TV's identity to LG and installing it
//! restarts the TV, which a person does in LG's own screen.
//!
//! Block LG system updates is the Homebrew Channel's own `blockUpdates`, set through its service (`getConfiguration`,
//! `setConfiguration`); at each boot, while it is on, the channel maps LG's update servers to this TV in /etc/hosts, so a change applies
//! when the TV restarts, and what is mapped now is the helper's `updateblock` topic. Turning it off asks twice. While updates are
//! blocked, or while it cannot be told, Check for Updates does nothing. Open apps without updating keeps LG's list of apps that must
//! be updated before they open empty (the helper's `openapps` topic, `POST /v1/openapps`), so an app opens as it is; the store still
//! updates by hand. Reset is last and asks twice, with LG's own PIN prompt between (`createPincodePrompt`; this page acts on its yes
//! or no and never sees the digits); the call is LG's `devicereset doReset {resetType}`.

use super::util::{panel_hours, panel_reads, panel_shown};
use crate::settings::ctx::{Ctx, Spec, ask, lg_launch, toast};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Answer, Arg, Ask, Op, PageArg, Row};
use serde_json::{Value, json};

const DASH: &str = "\u{2013}";

pub fn page() -> Page {
    Page::new("support", "Support", "circle-help").parent("system").back("System").hidden_from_search().tabs(&["TV info", "Update", "Privacy", "Tools", "Reset"]).reads(reads).rows(rows).answered(answered)
}

const HBC_GET: &str = "luna://org.webosbrew.hbchannel.service/getConfiguration";
const HBC_SET: &str = "luna://org.webosbrew.hbchannel.service/setConfiguration";

fn reads(_: &Ctx) -> Vec<Read> {
    let mut r = vec![
        Read::Get { name: "crash", path: "/v1/crash" },
        Read::lg("", &["eulaStatus"]),
        Read::lg("support", &["automaticUpdate"]),
        Read::lg("general", &["doNotSellMyPersonalInformation", "lmt"]),
        Read::lg("network", &["deviceName"]),
        Read::lg("option", &["vsn", "livePlus", "livePromotion"]),
        Read::call("device", "luna://com.palm.systemservice/deviceInfo/query", json!({"parameters": ["product_id", "serial_number"]})),
        Read::call("os", "luna://com.palm.systemservice/osInfo/query", json!({"parameters": ["core_os_release", "core_os_release_codename"]})),
        Read::call("storage", "luna://com.webos.service.attachedstoragemanager/getProperties", json!({"deviceId": "INTERNAL_STORAGE"})),
        Read::call("netinfo", "luna://com.webos.service.connectionmanager/getinfo", json!({})),
        Read::call("dolby", "luna://com.webos.service.pqcontroller/getInfo", json!({"key": "dolbySWVersion"})),
        Read::call("keys", "luna://com.webos.service.sm/crypto/checkSeStore", json!({"id": ["esn", "widevine"]})),
        Read::call("update", "luna://com.webos.service.update/getStatus", json!({})),
        Read::call("hbc", HBC_GET, json!({})),
        Read::call("ifa", "luna://com.webos.service.sm/deviceid/getIDs", json!({"idType": ["IFAID"]})),
    ];
    r.extend(panel_reads());
    r
}

fn text(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string().trim_matches('"').to_string(),
    }
}

/// The reading of a call: what it answered, nothing when it failed.
fn doc<'a>(c: &'a Ctx, name: &str) -> &'a Value {
    let d = c.docs.get(name);
    if d.get("returnValue").and_then(Value::as_bool) == Some(false) { &Value::Null } else { d }
}

fn product_id(c: &Ctx) -> String {
    text(doc(c, "device"), "product_id")
}

fn os_version(c: &Ctx) -> String {
    text(doc(c, "os"), "core_os_release")
}

fn webos(c: &Ctx) -> String {
    let v = os_version(c);
    match v.split('.').next().and_then(|m| m.parse::<i64>().ok()) {
        Some(major) if !v.is_empty() => format!("webOS{} / {v}", major + 15),
        _ => String::new(),
    }
}

fn storage(c: &Ctx) -> String {
    let d = doc(c, "storage");
    match (d.get("totalSpace").and_then(Value::as_f64), d.get("freeSpace").and_then(Value::as_f64)) {
        (Some(total), Some(free)) => format!("{:.2}/{:.2}GB", free / 1024.0, total / 1024.0),
        _ => String::new(),
    }
}

fn key_of(c: &Ctx, id: &str) -> String {
    doc(c, "keys")
        .get("keyStatusList")
        .and_then(Value::as_array)
        .and_then(|l| l.iter().find(|k| k.get("isOk").and_then(Value::as_bool) == Some(true) && k.get("id").and_then(Value::as_str) == Some(id) && k.get("identifier").is_some()))
        .map(|k| text(k, "identifier"))
        .unwrap_or_default()
}

fn ad_id(c: &Ctx) -> String {
    if let Some(v) = c.local.get("adid").as_str() {
        return v.to_string();
    }
    doc(c, "ifa").get("idList").and_then(Value::as_array).and_then(|l| l.iter().find(|i| i.get("idType").and_then(Value::as_str) == Some("IFAID"))).map(|i| text(i, "idValue")).unwrap_or_default()
}

fn terms_ok(c: &Ctx) -> bool {
    c.lg.get("eulaStatus").and_then(|e| e.get("generalTermsAllowed")).and_then(Value::as_bool) == Some(true)
}

const UPDATE_NAMES: [(&str, &str); 10] = [
    ("in progress", "Update in progress"),
    ("ExternalChipInstall", "Installing"),
    ("check update", "Checking for updates"),
    ("pending", "Update paused"),
    ("paused", "Update paused"),
    ("warm reserved", "Update paused"),
    ("schedule detected", "Update reserved"),
    ("failed", "Update failed"),
    ("canceled", "Update canceled"),
    ("completed", "Installed; applies when the TV restarts"),
];

/// A row that only shows a fact the TV gave: an unanswered one is an en dash.
fn info(tab: usize, icon: &str, label: &str, value: &str) -> Row {
    let mut r = Row::info(&format!("i:{label}")).tab(tab).icon(icon).label(label).value(if value.is_empty() { DASH } else { value });
    r.chevron = false;
    r
}

// ---------------------------------------------------------------------------------------------------- the crash notice

fn crash<'a>(c: &'a Ctx<'_>) -> Option<&'a Value> {
    let d = c.docs.get("crash");
    if d.get("ok").and_then(Value::as_bool) == Some(true) { d.get("crash").filter(|c| !c.is_null()) } else { None }
}

fn crash_pending(c: &Ctx) -> bool {
    crash(c).is_some_and(|k| k.get("pending").and_then(Value::as_bool) == Some(true) && k.get("kind").and_then(Value::as_str) != Some("escape"))
}

fn crash_ran(secs: Option<f64>) -> String {
    match secs {
        None => String::new(),
        Some(s) if s >= 120.0 => format!(" after {} minutes", (s / 60.0).round() as i64),
        Some(s) => format!(" after {} s", s as i64),
    }
}

// ---------------------------------------------------------------------------------------------------- the update block

/// The channel's `blockUpdates` and the live state of the hosts block (the helper's `updateblock` topic).
struct Block {
    flag: Option<bool>,
    live: Option<Value>,
    error: String,
}

fn block(c: &Ctx) -> Block {
    let set = c.docs.get("hbcset");
    let get = c.docs.get("hbc");
    // the answer of the last change is the newer one
    let newest = if set.is_object() { set } else { get };
    let failed = |d: &Value| d.is_object() && d.get("returnValue").and_then(Value::as_bool) == Some(false);
    let error = [set, get].into_iter().find(|d| failed(d)).map_or_else(
        String::new,
        |d| match d.get("errorText").and_then(Value::as_str) {
            Some(t) => format!("Homebrew Channel: {t}"),
            None => "Homebrew Channel not answering".into(),
        },
    );
    let flag = newest.get("blockUpdates").and_then(Value::as_bool).or_else(|| get.get("blockUpdates").and_then(Value::as_bool));
    Block { flag, live: c.helper("updateblock").cloned(), error }
}

impl Block {
    fn on(&self) -> bool {
        match self.flag {
            Some(f) => f,
            None => self.live.as_ref().and_then(|l| l.get("flag")).and_then(Value::as_bool) == Some(true),
        }
    }

    fn blocked(&self) -> bool {
        self.flag == Some(true) || self.live.as_ref().and_then(|l| l.get("blocking")).and_then(Value::as_bool) == Some(true)
    }

    fn unknown(&self) -> bool {
        self.flag.is_none() && self.live.is_none()
    }

    fn words(&self) -> String {
        if !self.error.is_empty() {
            return self.error.clone();
        }
        if self.unknown() {
            return "Homebrew Channel not answering".into();
        }
        let Some(live) = &self.live else { return if self.on() { "On".into() } else { "Off".into() } };
        match live.get("state").and_then(Value::as_str) {
            Some("blocked") => "Active; LG updates blocked".into(),
            Some("armed") => "On; active once the TV restarts".into(),
            Some("hosts-only") => "Off; blocked until restart".into(),
            _ => "Off; updates can reach this TV".into(),
        }
    }
}

fn open_words(c: &Ctx) -> String {
    let Some(live) = c.helper("openapps") else { return "Helper not answering".into() };
    let n = live.get("entries").and_then(Value::as_i64).unwrap_or(0);
    match live.get("state").and_then(Value::as_str) {
        Some("holding") => "On; no app is waiting for an update".into(),
        Some("clearing") => "On; clearing LG's list".into(),
        Some("waiting") => "On; clearing once LG's updater is idle".into(),
        Some("unreadable") => "LG's list cannot be read".into(),
        _ if n > 0 => format!("Off; LG lists {n} {} as waiting for an update", if n == 1 { "app" } else { "apps" }),
        _ => "Off".into(),
    }
}

fn two_questions() -> Vec<Op> {
    let second = Ask::yes_no("shield-alert", "Really allow LG system updates?", "The TV can then fetch and install LG's update, and that cannot be undone from here.", "Allow updates", "Keep blocked", true, vec![Op::Call { name: "hbcset".into(), uri: HBC_SET.into(), payload: json!({"blockUpdates": false}), subscribe: false }, Op::RefreshIn(700), toast("LG system updates allowed")]);
    let mut first = Ask::yes_no("shield-alert", "Allow LG system updates?", "LG's update can remove root and LumaTV.", "Continue", "Keep blocked", true, vec![Op::Ask(second)]);
    first.focus = 0;
    vec![Op::Ask(first)]
}

// ---------------------------------------------------------------------------------------------------- the rows

fn go(page: &str) -> Vec<Op> {
    vec![Op::Open { page: page.into(), arg: PageArg::None }]
}

fn rows(c: &Ctx) -> Vec<Row> {
    let mut rows = Vec::new();
    // ---- TV info
    let notice = crash_pending(c);
    let mut crashed = info(0, "triangle-alert", "LumaTV recovered from a problem", &crash(c).map(|k| format!("{}: {}", text(k, "at"), text(k, "reason"))).unwrap_or_default());
    crashed.visible = notice;
    rows.push(crashed);
    let mut details = Row::action("crashdetails", "Open").tab(0).icon("file-text").label("Details").help("Why LumaTV's compositor was put out of the way, and when. The log and a trace of its last seconds are saved on the TV.");
    details.visible = notice;
    let body = crash(c).map(|k| format!("{}: {}{}. LG's compositor took over and Home came back. The log and a trace of the last seconds are saved on the TV, in {}.", text(k, "at"), text(k, "reason"), crash_ran(k.get("uptimeSecs").and_then(Value::as_f64)), text(k, "path"))).unwrap_or_default();
    let mut q = Ask::yes_no("triangle-alert", "LumaTV recovered from a problem", &body, "Clear this notice", "Close", false, vec![Op::Send { name: Some("ack".into()), method: "POST", path: "/v1/crash/ack".into(), body: Some(json!({})) }, Op::Refresh]);
    q.answers = vec![Answer { id: "close".into(), label: "Close".into(), danger: false, then: Vec::new() }, q.answers.remove(1)];
    rows.push(details.does(vec![Op::Ask(q)]));
    let product = product_id(c);
    let model = product.split('.').next().unwrap_or("").to_string();
    let name = if c.lg.has("network.deviceName") { c.lg.opt_text("network.deviceName").unwrap_or_default() } else { String::new() };
    rows.push(info(0, "tv", "Device Name", &name));
    rows.push(info(0, "tv", "Model", &model));
    rows.push(info(0, "hash", "Model Code", &product));
    rows.push(info(0, "hash", "Serial Number", &text(doc(c, "device"), "serial_number")));
    rows.push(info(0, "settings", "webOS TV Version", &webos(c)));
    let mut hours = info(0, "clock", "Total Power On Time", &panel_hours(c));
    hours.visible = panel_shown(c);
    rows.push(hours);
    rows.push(info(0, "hard-drive", "Available Internal Storage", &storage(c)));
    let wired = text(doc(c, "netinfo").get("wiredInfo").unwrap_or(&Value::Null), "macAddress");
    let mut wired_row = info(0, "ethernet-port", "MAC Address (Wired)", &wired);
    wired_row.visible = !wired.is_empty();
    rows.push(wired_row);
    rows.push(info(0, "wifi", "MAC Address (Wireless)", &text(doc(c, "netinfo").get("wifiInfo").unwrap_or(&Value::Null), "macAddress")));
    rows.push(info(0, "key-round", "ESN", &key_of(c, "esn")));
    let mut vsn = info(0, "hash", "VSN", &c.lg.opt_text("option.vsn").unwrap_or_default());
    vsn.visible = c.lg.shown("option.vsn");
    rows.push(vsn);
    rows.push(info(0, "shield-check", "Widevine", &key_of(c, "widevine")));
    rows.push(info(0, "film", "Dolby Vision", &text(doc(c, "dolby"), "dolbySWVersion")));

    // ---- Update
    rows.push(info(1, "settings", "Software Version", &webos(c)));
    let state = text(doc(c, "update"), "status");
    let mut status = Row::info("updatestate").tab(1).icon("download").label("Update status").value(&if state.is_empty() { DASH.to_string() } else { crate::settings::names::name_or(&UPDATE_NAMES, &state, "No update running") });
    status.chevron = false;
    rows.push(status);
    let mut auto = c.toggle(Spec::new("support.automaticUpdate").tab(1).icon("refresh-cw").label("Install updates automatically").help("The TV installs software updates by itself, with no notice. Turning it on needs LG's terms, which are accepted in LG's settings."));
    let on = c.lg.is("support.automaticUpdate", "on");
    auto.enabled = c.lg.usable("support.automaticUpdate") && (on || terms_ok(c));
    auto.detail = if !c.lg.has("support.automaticUpdate") { "TV not answering".into() } else if !on && !terms_ok(c) { "Needs LG's terms".into() } else { String::new() };
    rows.push(auto);
    let b = block(c);
    // allowing LG's update can remove root and LumaTV, so this is a status with questions, not a switch; blocking asks nothing
    let mut blocker = Row::link("blockupdates").tab(1).icon("shield-check").label("LG system updates").help("Blocking keeps LG's update servers out of reach, so a surprise update cannot remove root or LumaTV. A change applies when the TV restarts.").value(if b.on() { "Blocked" } else { "Allowed" }).detail(&b.words());
    blocker.enabled = b.flag.is_some();
    let blocked_now = b.on();
    rows.push(blocker.act(move |a| match a {
        Arg::Press if blocked_now => two_questions(),
        Arg::Press => vec![Op::Call { name: "hbcset".into(), uri: HBC_SET.into(), payload: json!({"blockUpdates": true}), subscribe: false }, Op::RefreshIn(700), toast("LG system updates blocked")],
        _ => Vec::new(),
    }));
    let live = c.helper("openapps");
    let mut open = Row::toggle("openapps").tab(1).icon("package").label("Open apps without updating").help("An app opens even when LG says an update is waiting, instead of asking to update first. Updates from the LG Content Store still work by hand.").on(live.and_then(|l| l.get("enabled")).and_then(Value::as_bool) == Some(true)).detail(&open_words(c)).admin();
    open.enabled = live.is_some();
    rows.push(open.act(|a| match a {
        Arg::Toggle(v) => vec![Op::Send { name: None, method: "POST", path: "/v1/openapps".into(), body: Some(json!({"enabled": v})) }],
        _ => Vec::new(),
    }));
    let mut check = lg_launch("checkupdate", "Check for Updates", "SoftwareUpdate", "").tab(1).help("Checking sends the TV's model and version to LG, and an install restarts the TV. Both are done in LG's screen.");
    if b.blocked() || b.unknown() {
        check.enabled = false;
        check.chevron = false;
        check.value = if b.blocked() { "Updates blocked" } else { "Block not readable" }.into();
    }
    rows.push(check);

    // ---- Privacy (the rows of the Privacy page, with the same words)
    rows.push(super::privacy::do_not_sell(c, 2));
    rows.push(super::privacy::limit_tracking(c, 2));
    rows.push(info(2, "hash", "AD ID", &ad_id(c)));
    rows.push(Row::action("resetad", "Reset").tab(2).icon("rotate-ccw").label("Reset AD ID").help("Gives the TV a new advertising ID, and ads are tracked from the new one.").does(ask(
        "rotate-ccw",
        "Reset the AD ID?",
        "The TV gets a new advertising ID.",
        "Reset",
        "Cancel",
        false,
        vec![Op::Act { uri: "luna://com.webos.service.admanager/resetIFA".into(), payload: json!({}) }, Op::Set("adid".into(), json!("")), Op::RefreshIn(700), toast("Advertising ID reset")],
    )));
    rows.push(super::privacy::live_plus(c, 2));
    rows.push(super::privacy::live_promotion(c, 2));
    rows.push(Row::link("permissions").tab(2).icon("shield-check").label("App Permissions").help("Which apps may use the camera, the microphone, nearby devices and files.").does(go("permissions")));
    rows.push(Row::link("deletedata").tab(2).icon("eye-off").label("Delete My Personal Information").help("Deletes the viewing and voice information LG collected from this TV.").does(go("deletedata")));

    // ---- Tools
    rows.push(lg_launch("screentest", "Screen Self-diagnosis", "pictureTest", "com.webos.app.customersupport").tab(3).help("LG's picture test, in LG's Customer Support app."));
    rows.push(lg_launch("soundtest", "Sound Self-diagnosis", "soundTest", "com.webos.app.customersupport").tab(3).help("LG's test tone, in LG's Customer Support app. The TV speaker must be the sound out."));
    let cleared = c.local.flag("recentsCleared");
    rows.push(Row::action("memory", if cleared { "Done" } else { "Run" }).tab(3).icon("cpu").label("Memory Optimizer").help("Clears LG's list of recent apps to free memory.").does(ask(
        "cpu",
        "Clear LG's recent apps?",
        "The list of recent apps is emptied.",
        "Clear",
        "Cancel",
        false,
        vec![Op::Act { uri: "luna://com.webos.service.recent/removeAllRecents".into(), payload: json!({}) }, Op::Set("recentsCleared".into(), json!(true))],
    )));

    // ---- Reset
    rows.push(reset_row("resetall", "Reset All", "user", "Erases the TV's settings, accounts and apps, as new. It asks for the PIN, twice over."));
    rows.push(reset_row("resetkeep", "Reset All Except Network Settings", "userKeepNetwork", "As Reset All, but the Wi-Fi and the wired settings stay."));
    rows
}

/// A reset asks, then LG's PIN prompt (a moment after the first question has closed: a prompt answered at once would open the second
/// question in the instant the first closes, and lose it), then once more; nothing is sent before the second yes.
fn reset_row(id: &str, label: &str, kind: &'static str, help: &str) -> Row {
    Row::action(id, "Reset").tab(4).icon("rotate-ccw").label(label).help(help).danger().does(ask(
        "rotate-ccw",
        &format!("{label}?"),
        "The TV asks for the PIN next, then once more.",
        "Continue",
        "Cancel",
        true,
        vec![
            Op::Set("pending".into(), json!(kind)),
            Op::Later { key: "pin".into(), ms: 150, then: vec![Op::Call { name: "pin".into(), uri: "luna://com.webos.notification/createPincodePrompt".into(), payload: json!({"promptType": "parental"}), subscribe: false }] },
        ],
    ))
}

/// What a page does with the answers of its calls: the PIN prompt's yes opens the last question of a reset; a reset is cleared whatever
/// the PIN said; a change of the block or the ad id is read back.
fn answered(c: &Ctx, name: &str, doc: &Value) -> Vec<Op> {
    match name {
        "pin" => {
            let kind = c.local.get("pending").as_str().unwrap_or("").to_string();
            let mut ops = vec![Op::Set("pending".into(), json!(""))];
            if !kind.is_empty() && doc.get("returnValue").and_then(Value::as_bool) != Some(false) && doc.get("matched").and_then(Value::as_bool) == Some(true) {
                let body = if kind == "user" { "Everything is erased. The TV restarts." } else { "Everything but the network settings is erased. The TV restarts." };
                ops.extend(ask("triangle-alert", "Reset the TV now?", body, "Reset now", "Cancel", true, vec![Op::Act { uri: "luna://com.webos.service.devicereset/doReset".into(), payload: json!({"resetType": kind}) }]));
            }
            ops
        }
        "ifa" => vec![Op::Set("adid".into(), Value::Null)],
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------------------------------- Delete My Personal Information

pub fn delete_data() -> Page {
    Page::new("deletedata", "Delete My Personal Information", "eye-off").parent("support").back("Support").hidden_from_search().reads(delete_reads).rows(delete_rows).answered(delete_answered)
}

fn delete_reads(_: &Ctx) -> Vec<Read> {
    vec![Read::lg("", &["eulaStatus"]), Read::call("net", "luna://com.webos.service.connectionmanager/getStatus", json!({}))]
}

fn voice_terms(c: &Ctx) -> bool {
    c.lg.get("eulaStatus").and_then(|e| e.get("voiceAllowed")).and_then(Value::as_bool) == Some(true)
}

/// Why a delete is not offered: LG's terms not accepted, or the TV not online.
fn why(c: &Ctx, voice: bool) -> &'static str {
    if !terms_ok(c) || (voice && !voice_terms(c)) {
        "Needs LG's terms"
    } else if c.docs.field("net", "isInternetConnectionAvailable").as_bool() != Some(true) {
        "Needs the network"
    } else {
        ""
    }
}

fn delete_rows(c: &Ctx) -> Vec<Row> {
    let row = |id: &str, icon: &str, label: &str, help: &str, voice: bool, done: bool, question: (&str, &str), then: Vec<Op>| {
        let mut r = Row::action(id, if done { "Done" } else { "Delete" }).icon(icon).label(label).help(help).danger();
        let why = why(c, voice);
        r.enabled = why.is_empty();
        r.detail = why.to_string();
        r.does(ask(icon, question.0, question.1, "Delete", "Cancel", true, then))
    };
    vec![
        row(
            "viewing",
            "eye-off",
            "Delete Viewing Information",
            "Deletes the viewing information LG collected from this TV. It may change what LG recommends.",
            false,
            c.local.flag("viewingDone"),
            ("Delete viewing information?", "It may change what LG recommends."),
            vec![
                Op::Call { name: "lgudid".into(), uri: "luna://com.webos.service.sm/deviceid/getIDs".into(), payload: json!({"idType": ["LGUDID"]}), subscribe: false },
                Op::Act { uri: "luna://com.webos.service.userintentmanager/reset".into(), payload: json!({"target": "viewingInfo", "toast": false}) },
                Op::Set("viewingDone".into(), json!(true)),
            ],
        ),
        row(
            "voice",
            "mic",
            "Delete Voice Information",
            "Deletes the voice information LG collected from this TV.",
            true,
            c.local.flag("voiceDone"),
            ("Delete voice information?", "LG's voice service forgets what it kept."),
            vec![
                Op::Act { uri: "luna://lg.thinqai.adapter/deletePrivacyData".into(), payload: json!({}) },
                Op::Act { uri: "luna://com.webos.service.userintentmanager/reset".into(), payload: json!({"target": "voiceInfo", "toast": false}) },
                Op::Set("voiceDone".into(), json!(true)),
            ],
        ),
    ]
}

/// The device id arrives and goes with the viewing delete to the user-context service, as LG's row sends it.
fn delete_answered(_: &Ctx, name: &str, doc: &Value) -> Vec<Op> {
    if name != "lgudid" || doc.get("returnValue").and_then(Value::as_bool) == Some(false) {
        return Vec::new();
    }
    doc.get("idList")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|i| i.get("idType").and_then(Value::as_str) == Some("LGUDID"))
        .map(|i| Op::Act { uri: "luna://com.webos.service.usercontextmanager/reset".into(), payload: json!({"toast": true, "bodyData": {"isDeleteAccountAllData": true, "lgudid": i.get("idValue")}}) })
        .collect()
}

// ---------------------------------------------------------------------------------------------------- App Permissions

pub fn permissions() -> Page {
    Page::new("permissions", "App Permissions", "shield-check").parent("support").back("Support").hidden_from_search().tabs(&["Camera", "Microphone", "Nearby Devices", "Access File"]).reads(permission_reads).rows(permission_rows).answered(permission_answered)
}

const PPM: &str = "luna://com.webos.service.ppm";

fn permission_reads(_: &Ctx) -> Vec<Read> {
    vec![
        Read::lg("option", &["cameraResourcePermission", "nearbyDevicePermission", "fileapiResourcePermission"]),
        // LG's own call: the TV refuses an empty request (errorCode 8006, read on the set 2026-10-26)
        Read::Watch { name: "perm", uri: "luna://com.webos.service.ppm/queryPermissions", payload: json!({}) },
        Read::Apps,
    ]
}

/// Per tab: the resource LG's tables name, what the question calls it, and where the switch is written (a setting, or a `ppm` privacy group).
struct Kind {
    resource: &'static str,
    noun: &'static str,
    icon: &'static str,
    ds: Option<&'static str>,
    group: Option<&'static str>,
    visible_ds: Option<&'static str>,
}

const KINDS: [Kind; 4] = [
    Kind { resource: "camera", noun: "camera", icon: "camera", ds: Some("option.cameraResourcePermission"), group: None, visible_ds: None },
    Kind { resource: "mic", noun: "microphone", icon: "mic", ds: None, group: Some("microphone.privacy"), visible_ds: None },
    Kind { resource: "ble", noun: "Nearby Devices", icon: "radio", ds: Some("option.nearbyDevicePermission"), group: None, visible_ds: None },
    Kind { resource: "usbFileAccess", noun: "file", icon: "hard-drive", ds: None, group: Some("usbFileAccess.privacy"), visible_ds: Some("option.fileapiResourcePermission") },
];

struct Entry {
    app: String,
    title: String,
    on: bool,
}

/// The microphone's and the files' are the permission service's (`queryPermissions` lists them, the answer's table keyed by group).
fn from_ppm(c: &Ctx, group: &str, answered: bool) -> Vec<Entry> {
    // the pick just made shows at once: the page's own table, until the service's answer replaces it
    let table = c.local.get(&format!("ppm:{group}"));
    if let (Some(t), false) = (table.as_object(), answered) {
        return t.iter().map(|(app, on)| Entry { app: app.clone(), title: String::new(), on: on.as_bool() == Some(true) }).collect();
    }
    c.docs
        .field("perm", "privacyPermissions")
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let state = e.get("permissionTable").and_then(|t| t.get(group)).and_then(Value::as_str)?;
            let app = text(e, "appId");
            let on = match table.get(&app).and_then(Value::as_bool) {
                Some(local) => local,
                None => state == "allow",
            };
            Some(Entry { app, title: String::new(), on })
        })
        .collect()
}

/// LG's tables: `[{appId, appTitle, permissions: [{resource, permission}]}]`, the entry that names the resource kept.
fn from_settings(c: &Ctx, ds: &str, resource: &str) -> Vec<Entry> {
    c.lg
        .get(ds)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let p = t.get("permissions").and_then(Value::as_array)?.iter().find(|p| p.get("resource").and_then(Value::as_str) == Some(resource))?;
            Some(Entry { app: text(t, "appId"), title: text(t, "appTitle"), on: p.get("permission").and_then(Value::as_bool) == Some(true) })
        })
        .collect()
}

fn entries(c: &Ctx, i: usize) -> Vec<Entry> {
    let k = &KINDS[i];
    if let Some(group) = k.group {
        return if k.visible_ds.is_none_or(|d| c.lg.shown(d)) { from_ppm(c, group, false) } else { Vec::new() };
    }
    match k.ds {
        Some(ds) if c.lg.shown(ds) => from_settings(c, ds, k.resource),
        _ => Vec::new(),
    }
}

fn app_title(c: &Ctx, app: &str, fallback: &str) -> String {
    c.apps.by_id(app).map(|a| a.title.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| if fallback.is_empty() { app.to_string() } else { fallback.to_string() })
}

/// One switch changes: a setting's table is written whole with the one flag changed; a privacy group goes to the permission service.
fn set_permission(c: &Ctx, i: usize, app: &str, on: bool) -> Vec<Op> {
    let k = &KINDS[i];
    if let Some(ds) = k.ds {
        let mut table = c.lg.get(ds).cloned().unwrap_or_else(|| json!([]));
        for t in table.as_array_mut().into_iter().flatten().filter(|t| t.get("appId").and_then(Value::as_str) == Some(app)) {
            for p in t.get_mut("permissions").and_then(Value::as_array_mut).into_iter().flatten().filter(|p| p.get("resource").and_then(Value::as_str) == Some(k.resource)) {
                p["permission"] = json!(on);
            }
        }
        let key = ds.split_once('.').map_or(ds, |(_, k)| k);
        return vec![Op::LgMany { category: "option".into(), settings: crate::settings::row::settings(&[(key, table)]), app: None }];
    }
    let group = k.group.unwrap_or("");
    let mut table = c.local.get(&format!("ppm:{group}")).as_object().cloned().unwrap_or_default();
    for e in entries(c, i) {
        table.entry(e.app).or_insert(json!(e.on));
    }
    table.insert(app.to_string(), json!(on));
    vec![
        Op::Act { uri: format!("{PPM}/setPermission"), payload: json!({"appId": app, "privacyGroup": group, "status": if on { "allow" } else { "deny" }}) },
        Op::Set(format!("ppm:{group}"), Value::Object(table)),
        Op::RefreshIn(600),
    ]
}

fn permission_rows(c: &Ctx) -> Vec<Row> {
    let mut rows = Vec::new();
    for (i, k) in KINDS.iter().enumerate() {
        let list = entries(c, i);
        if list.is_empty() {
            rows.push(Row::empty("No app has asked", "Apps that ask for access are listed here, each with a switch.", k.icon, i));
        }
        for e in list {
            let title = app_title(c, &e.app, &e.title);
            let app = e.app.clone();
            let (on_ops, off_ops) = (set_permission(c, i, &app, true), set_permission(c, i, &app, false));
            let mut r = Row::toggle(&format!("app:{i}:{}", e.app)).tab(i).icon(k.icon).label(&title).on(e.on);
            r.detail = String::new();
            let noun = k.noun;
            rows.push(r.act(move |a| match a {
                Arg::Toggle(false) => [off_ops.clone(), vec![toast(&format!("{title}: {noun} access blocked"))]].concat(),
                Arg::Toggle(true) => [on_ops.clone(), vec![toast(&format!("{title}: {noun} access allowed"))]].concat(),
                _ => Vec::new(),
            }));
        }
    }
    rows
}

/// The service's answer replaces what the page showed at once.
fn permission_answered(_: &Ctx, name: &str, _: &Value) -> Vec<Op> {
    if name == "perm" { vec![Op::Set("ppm:microphone.privacy".into(), Value::Null), Op::Set("ppm:usbFileAccess.privacy".into(), Value::Null)] } else { Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::{World, lg_with};

    fn by_id<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id} in {:?}", rows.iter().map(|r| &r.id).collect::<Vec<_>>()))
    }

    fn world() -> World {
        let mut w = World::new(lg_with(
            &[
                ("", json!({"settings": {"eulaStatus": {"generalTermsAllowed": true, "voiceAllowed": false}}})),
                ("support", json!({"settings": {"automaticUpdate": "off"}})),
                ("general", json!({"settings": {"doNotSellMyPersonalInformation": "off", "lmt": "off"}})),
                ("network", json!({"settings": {"deviceName": "Living room"}})),
                ("option", json!({"settings": {"vsn": "V1", "livePlus": "off", "livePromotion": "off"}})),
            ],
            &[],
        ));
        w.docs.set("device", json!({"returnValue": true, "product_id": "OLED65G2PUA.AUSQLJR", "serial_number": "S123"}));
        w.docs.set("os", json!({"returnValue": true, "core_os_release": "7.1.2"}));
        w.docs.set("storage", json!({"returnValue": true, "totalSpace": 6144.0, "freeSpace": 2048.0}));
        w.docs.set("netinfo", json!({"returnValue": true, "wiredInfo": {"macAddress": ""}, "wifiInfo": {"macAddress": "aa:bb"}, "isInternetConnectionAvailable": true}));
        w.docs.set("keys", json!({"keyStatusList": [{"id": "esn", "isOk": true, "identifier": "E-1"}, {"id": "widevine", "isOk": false, "identifier": "W-1"}]}));
        w.docs.set("update", json!({"status": "paused"}));
        w
    }

    #[test]
    fn the_tv_info_shows_what_the_tv_said_and_an_en_dash_for_what_it_did_not() {
        let w = world();
        let r = rows(&w.ctx());
        assert_eq!(by_id(&r, "i:Model").value, "OLED65G2PUA");
        assert_eq!(by_id(&r, "i:Model Code").value, "OLED65G2PUA.AUSQLJR");
        assert_eq!(by_id(&r, "i:webOS TV Version").value, "webOS22 / 7.1.2");
        assert_eq!(by_id(&r, "i:Available Internal Storage").value, "2.00/6.00GB");
        assert_eq!(by_id(&r, "i:ESN").value, "E-1");
        assert_eq!(by_id(&r, "i:Widevine").value, DASH, "a key that is not ok is not shown");
        assert_eq!(by_id(&r, "i:Dolby Vision").value, DASH);
        assert!(!by_id(&r, "i:MAC Address (Wired)").visible && by_id(&r, "i:MAC Address (Wireless)").value == "aa:bb");
        assert!(!by_id(&r, "i:LumaTV recovered from a problem").visible);
        assert_eq!(by_id(&r, "updatestate").value, "Update paused");
    }

    #[test]
    fn a_crash_the_owner_has_not_read_is_on_top_with_details_and_a_way_to_clear_it() {
        let mut w = world();
        w.docs.set("crash", json!({"ok": true, "crash": {"pending": true, "kind": "hang", "at": "Oct 29 03:00", "reason": "drew nothing", "uptimeSecs": 90, "path": "/media/crash/1"}}));
        let r = rows(&w.ctx());
        assert!(by_id(&r, "i:LumaTV recovered from a problem").visible && by_id(&r, "crashdetails").visible);
        assert_eq!(by_id(&r, "i:LumaTV recovered from a problem").value, "Oct 29 03:00: drew nothing");
        let ops = (by_id(&r, "crashdetails").act.as_ref().unwrap())(Arg::Press);
        let [Op::Ask(a)] = &ops[..] else { panic!() };
        assert!(a.body.contains("after 90 s") && a.body.contains("/media/crash/1"));
        assert_eq!(a.answers.iter().map(|x| x.label.as_str()).collect::<Vec<_>>(), vec!["Close", "Clear this notice"]);
        w.docs.set("crash", json!({"ok": true, "crash": {"pending": true, "kind": "escape"}}));
        assert!(!by_id(&rows(&w.ctx()), "crashdetails").visible, "an escape the owner made is no notice");
    }

    #[test]
    fn the_update_block_is_a_status_that_asks_twice_to_come_off_and_check_for_updates_waits_for_it() {
        let mut w = world();
        let r = rows(&w.ctx());
        let blocker = by_id(&r, "blockupdates");
        assert!(!blocker.enabled && blocker.detail == "Homebrew Channel not answering");
        assert!(!by_id(&r, "checkupdate").enabled && by_id(&r, "checkupdate").value == "Block not readable");
        w.docs.set("hbc", json!({"returnValue": true, "blockUpdates": true}));
        w.topics.set_helper("updateblock", json!({"flag": true, "blocking": true, "state": "blocked"}));
        let r = rows(&w.ctx());
        let blocker = by_id(&r, "blockupdates");
        assert!(blocker.kind == crate::settings::row::Kind::Value && blocker.chevron, "risky when it comes off, so a status with a question, not a switch");
        assert!((blocker.value.as_str(), blocker.enabled, blocker.detail.as_str()) == ("Blocked", true, "Active; LG updates blocked"));
        assert!(!by_id(&r, "checkupdate").enabled && by_id(&r, "checkupdate").value == "Updates blocked");
        // coming off: Keep blocked first, then a second question; only the second yes writes
        let ops = (blocker.act.as_ref().unwrap())(Arg::Press);
        let [Op::Ask(first)] = &ops[..] else { panic!() };
        assert_eq!(first.answers[0].label, "Keep blocked");
        assert!(first.answers[0].then.is_empty());
        let [Op::Ask(second)] = &first.answers[1].then[..] else { panic!("a second question") };
        assert_eq!((second.title.as_str(), second.answers[1].label.as_str()), ("Really allow LG system updates?", "Allow updates"));
        assert!(matches!(&second.answers[1].then[0], Op::Call { name, payload, .. } if name == "hbcset" && payload["blockUpdates"] == false));
        // an error from the channel is the words
        w.docs.set("hbcset", json!({"returnValue": false, "errorText": "denied"}));
        assert_eq!(by_id(&rows(&w.ctx()), "blockupdates").detail, "Homebrew Channel: denied");
        // not blocked and readable: check for updates opens LG's screen
        let mut w = world();
        w.docs.set("hbc", json!({"returnValue": true, "blockUpdates": false}));
        w.topics.set_helper("updateblock", json!({"flag": false, "blocking": false, "state": "open"}));
        let r = rows(&w.ctx());
        assert!(by_id(&r, "checkupdate").enabled);
        assert!(matches!(&(by_id(&r, "checkupdate").act.as_ref().unwrap())(Arg::Press)[..], [Op::Launch { params, .. }] if params["target"] == "SoftwareUpdate"));
        let blocker = by_id(&r, "blockupdates");
        assert_eq!((blocker.value.as_str(), blocker.detail.as_str()), ("Allowed", "Off; updates can reach this TV"));
        // blocking is safe: it writes at once, with a toast and no question
        let ops = (blocker.act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&ops[0], Op::Call { payload, .. } if payload["blockUpdates"] == true) && !ops.iter().any(|o| matches!(o, Op::Ask(_))), "{ops:?}");
    }

    #[test]
    fn opening_apps_without_updating_is_the_owners_switch_on_the_helper() {
        let mut w = world();
        let r = rows(&w.ctx());
        let open = by_id(&r, "openapps");
        assert!(open.admin && !open.enabled && open.detail == "Helper not answering");
        w.topics.set_helper("openapps", json!({"enabled": false, "state": "off", "entries": 2}));
        let r = rows(&w.ctx());
        let open = by_id(&r, "openapps");
        assert!(open.enabled && !open.on);
        assert_eq!(open.detail, "Off; LG lists 2 apps as waiting for an update");
        let ops = (open.act.as_ref().unwrap())(Arg::Toggle(true));
        assert!(matches!(&ops[..], [Op::Send { method: "POST", path, body: Some(b), .. }] if path == "/v1/openapps" && b == &json!({"enabled": true})));
        w.topics.set_helper("openapps", json!({"enabled": true, "state": "holding", "entries": 0}));
        assert_eq!(by_id(&rows(&w.ctx()), "openapps").detail, "On; no app is waiting for an update");
    }

    #[test]
    fn auto_update_needs_lgs_terms_to_turn_on_and_the_ad_rows_flip_in_place_but_a_new_ad_id_asks() {
        let mut w = world();
        w.lg.learn("", &json!({"settings": {"eulaStatus": {"generalTermsAllowed": false}}}));
        let r = rows(&w.ctx());
        assert!(!by_id(&r, "support.automaticUpdate").enabled && by_id(&r, "support.automaticUpdate").detail == "Needs LG's terms");
        let dns = by_id(&r, "general.doNotSellMyPersonalInformation");
        assert_eq!(dns.label, "Ask LG not to sell my data");
        assert!(matches!(&(dns.act.as_ref().unwrap())(Arg::Toggle(true))[..], [Op::Act { .. }, Op::RefreshIn(_), Op::Act { .. }]), "no question, then a toast");
        assert!(matches!(&(by_id(&r, "resetad").act.as_ref().unwrap())(Arg::Press)[..], [Op::Ask(_)]));
        let promo = by_id(&r, "option.livePromotion");
        assert!(!promo.enabled && promo.detail == "Allow content recognition first", "it follows Live Plus");
        assert!(matches!(&(by_id(&r, "option.livePlus").act.as_ref().unwrap())(Arg::Toggle(true))[..], [Op::Lg { .. }, Op::Act { .. }]));
        assert!(matches!(&(by_id(&r, "option.livePlus").act.as_ref().unwrap())(Arg::Toggle(false))[..], [Op::Lg { .. }, Op::Act { .. }]));
    }

    #[test]
    fn a_reset_asks_then_the_pin_then_once_more_and_sends_only_after_the_second_yes() {
        let mut w = world();
        let r = rows(&w.ctx());
        let reset = by_id(&r, "resetkeep");
        let ops = (reset.act.as_ref().unwrap())(Arg::Press);
        let [Op::Ask(first)] = &ops[..] else { panic!() };
        assert!(first.answers[0].then.is_empty() && first.answers[1].danger);
        let [Op::Set(k, v), Op::Later { ms, then, .. }] = &first.answers[1].then[..] else { panic!() };
        assert_eq!((k.as_str(), v), ("pending", &json!("userKeepNetwork")));
        assert_eq!(*ms, 150);
        assert!(matches!(&then[..], [Op::Call { name, payload, .. }] if name == "pin" && payload["promptType"] == "parental"));
        // the PIN's answer: a wrong or cancelled one does nothing; a matched one asks the last question
        w.local.set("pending", json!("userKeepNetwork"));
        assert!(answered(&w.ctx(), "pin", &json!({"returnValue": true, "matched": false})).iter().all(|o| matches!(o, Op::Set(..))));
        let ops = answered(&w.ctx(), "pin", &json!({"returnValue": true, "matched": true}));
        let [Op::Set(..), Op::Ask(last)] = &ops[..] else { panic!("{ops:?}") };
        assert_eq!(last.title, "Reset the TV now?");
        assert!(last.body.starts_with("Everything but the network settings"));
        assert!(matches!(&last.answers[1].then[..], [Op::Act { uri, payload }] if uri.ends_with("devicereset/doReset") && payload["resetType"] == "userKeepNetwork"));
        w.local.set("pending", json!(""));
        assert!(answered(&w.ctx(), "pin", &json!({"returnValue": true, "matched": true})).iter().all(|o| matches!(o, Op::Set(..))), "no reset was asked for");
    }

    #[test]
    fn deleting_personal_information_needs_the_terms_and_the_network_and_the_viewing_one_sends_the_device_id_along() {
        let mut w = world();
        w.docs.set("net", json!({"isInternetConnectionAvailable": false}));
        let r = delete_rows(&w.ctx());
        assert!(!by_id(&r, "viewing").enabled && by_id(&r, "viewing").detail == "Needs the network");
        assert_eq!(by_id(&r, "voice").detail, "Needs LG's terms", "the voice terms are not accepted");
        w.docs.set("net", json!({"isInternetConnectionAvailable": true}));
        let r = delete_rows(&w.ctx());
        assert!(by_id(&r, "viewing").enabled);
        let ops = (by_id(&r, "viewing").act.as_ref().unwrap())(Arg::Press);
        let [Op::Ask(a)] = &ops[..] else { panic!() };
        assert!(a.answers[1].danger && matches!(&a.answers[1].then[0], Op::Call { name, .. } if name == "lgudid"));
        let sent = delete_answered(&w.ctx(), "lgudid", &json!({"returnValue": true, "idList": [{"idType": "LGUDID", "idValue": "U-1"}, {"idType": "IFAID", "idValue": "x"}]}));
        assert!(matches!(&sent[..], [Op::Act { uri, payload }] if uri.ends_with("usercontextmanager/reset") && payload["bodyData"]["lgudid"] == "U-1"));
    }

    #[test]
    fn permissions_are_a_tab_each_with_a_switch_per_app_that_flips_in_place() {
        let mut w = world();
        w.lg.learn("option", &json!({"settings": {"cameraResourcePermission": [{"appId": "cam.app", "appTitle": "Cam", "permissions": [{"resource": "camera", "permission": false}]}, {"appId": "other", "permissions": [{"resource": "mic", "permission": true}]}]}}));
        w.lg.learn_desc("option", &json!({"results": [{"key": "cameraResourcePermission", "ui": {"visible": true}}]}));
        w.docs.set("perm", json!({"returnValue": true, "privacyPermissions": [{"appId": "mic.app", "permissionTable": {"microphone.privacy": "allow"}}]}));
        let r = permission_rows(&w.ctx());
        assert_eq!(by_id(&r, "app:0:cam.app").label, "Cam");
        assert!(!by_id(&r, "app:0:cam.app").on);
        assert!(by_id(&r, "app:1:mic.app").on && by_id(&r, "app:1:mic.app").label == "mic.app");
        assert!(r.iter().any(|x| x.tab == 2 && x.kind == crate::settings::row::Kind::Empty), "no app asked for the nearby devices");
        let ops = (by_id(&r, "app:0:cam.app").act.as_ref().unwrap())(Arg::Toggle(true));
        let [Op::LgMany { category, settings, .. }, Op::Act { payload, .. }] = &ops[..] else { panic!("{ops:?}") };
        assert_eq!(payload["message"], "Cam: camera access allowed");
        assert_eq!(category, "option");
        assert_eq!(settings["cameraResourcePermission"][0]["permissions"][0]["permission"], true);
        assert_eq!(settings["cameraResourcePermission"][1]["permissions"][0]["permission"], true, "the others are as they were");
        // off is written at once, through the permission service for the microphone
        let ops = (by_id(&r, "app:1:mic.app").act.as_ref().unwrap())(Arg::Toggle(false));
        assert!(matches!(&ops[0], Op::Act { uri, payload } if uri.ends_with("ppm/setPermission") && payload["status"] == "deny" && payload["privacyGroup"] == "microphone.privacy"));
        assert!(matches!(&ops[1], Op::Set(k, _) if k == "ppm:microphone.privacy"));
    }
}
