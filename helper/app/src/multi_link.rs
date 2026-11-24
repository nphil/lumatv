//! Opt-in Multi Link. Only the trusted helper prepares links; apps opt into source-bound downloads.
//! Private rollback survives a helper crash. Standby restores it without changing any wake/panel setting.

use crate::ctx::Ctx;
use crate::plugins::local::{Local, OptIn, Spec};
use crate::plugins::session::Shared;
use multilink::{Device, FileJournal, Key, Kind, Meter, Result, Transaction, Value};
use multilink::linux::{Linux, uptime_ms};
use serde_json::{Value as Json, json};
use std::fs;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const ID: &str = "multi-link";
pub const TOPIC: &str = "network.multi-link";
const WAKE: &str = "multi-link.wake";
pub const OPT_IN: OptIn = OptIn {
    spec: Spec { id: ID, name: "Multi Link", version: "1", permissions: &["settings"] },
    consent: "Keeps a saved Wi-Fi network connected beside Ethernet while the screen is on. Supported apps can receive downloads over several links while controls stay wired. USB Ethernet is supported too. Temporarily prepares each link and moves private Wi-Fi host routes to the wire; restores those changes when switched off, in standby or when the helper stops. Never keeps a Wi-Fi password or changes wake or screen-care settings. Other apps do not combine links.",
};

struct Lg { ctx: Ctx, shared: Arc<Shared> }
impl Lg {
    fn call(&self, uri: &str, payload: Json) -> Result<Json> {
        let value = crate::luna::once(&self.ctx.config.luna.send, uri, &payload.to_string()).ok_or("LG's network service did not answer")?;
        if value.get("returnValue").and_then(Json::as_bool) != Some(true) { return Err("LG refused the network request".into()); }
        Ok(value)
    }
    fn wifi(&self) -> Result<Json> { self.call("luna://com.webos.service.wifi/getstatus", json!({})) }
    fn sharing(&self) -> Result<bool> {
        let value = self.call("luna://com.webos.service.connectionmanager/getStatus", json!({}))?;
        match value.pointer("/wifiDirect/state").and_then(Json::as_str) {
            Some("disconnected") => Ok(false),
            Some(_) => Ok(true),
            None => Err("Cannot check whether screen sharing owns Wi-Fi".into()),
        }
    }
}

fn wifi_value(key: &Key, status: &Json) -> Result<Value> {
    match key {
        Key::WifiPower => match status.get("status").and_then(Json::as_str) {
            Some("serviceDisabled") => Ok(Value::Number(0)),
            Some("serviceEnabled" | "connectionStateChanged") => Ok(Value::Number(1)),
            _ => Err("Cannot read the saved Wi-Fi power state".into()),
        },
        Key::WifiProfile => {
            match status.pointer("/networkInfo/connectState").and_then(Json::as_str) {
                None | Some("notConnected" | "notAssociated" | "disconnected" | "ipFailed") => return Ok(Value::Number(0)),
                Some("ipConfigured" | "associating" | "associated" | "configuring" | "connecting") => {}
                _ => return Err("Cannot identify the Wi-Fi connection state".into()),
            }
            status.pointer("/networkInfo/profileId").and_then(Json::as_u64).and_then(|n| u32::try_from(n).ok()).filter(|n| *n > 0)
                .map(Value::Number).ok_or_else(|| "Wi-Fi is not using a saved network".into())
        }
        _ => Err("Not a Wi-Fi setting".into()),
    }
}

impl Device for Lg {
    fn allowed(&self) -> bool { self.shared.switched_on(ID) && confirmed_screen(&self.ctx) && !self.ctx.stop.is_set() }
    fn read(&self, key: &Key) -> Result<Value> {
        if key.radio() { wifi_value(key, &self.wifi()?) } else { Linux.read(key) }
    }
    fn write(&self, key: &Key, value: &Value) -> Result<()> {
        if !key.radio() { return Linux.write(key, value); }
        if self.sharing()? { return Err("Screen sharing owns Wi-Fi; restoration waits until it ends".into()); }
        match (key, value) {
            (Key::WifiPower, Value::Number(n @ 0..=1)) => {
                self.call("luna://com.webos.service.wifi/setstate", json!({"state": if *n == 1 { "enabled" } else { "disabled" }}))?;
            }
            (Key::WifiProfile, Value::Number(0)) => { self.call("luna://com.webos.service.wifi/disconnect", json!({}))?; }
            (Key::WifiProfile, Value::Number(profile)) => { self.call("luna://com.webos.service.wifi/connect", json!({"profileId": profile}))?; }
            _ => return Err("Invalid Wi-Fi state".into()),
        }
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            if self.read(key)? == *value { return Ok(()); }
            if Instant::now() >= deadline { break; }
            if matches!(key, Key::WifiProfile) && matches!(value, Value::Number(n) if *n > 0) && !self.allowed() { break; }
            thread::sleep(Duration::from_millis(100));
        }
        // A connection request is asynchronous. Cancel an unconfirmed join before releasing its journal entry.
        if matches!(key, Key::WifiProfile) && matches!(value, Value::Number(n) if *n != 0) && !self.sharing()? {
            self.call("luna://com.webos.service.wifi/disconnect", json!({}))?;
        }
        Err("LG did not confirm the network change".into())
    }
}

type Changes = Transaction<Lg, FileJournal>;

fn saved_profile(value: &Json) -> Result<u32> {
    let profiles = value.get("profileList").and_then(Json::as_array).ok_or("LG did not list saved Wi-Fi networks")?;
    if profiles.len() != 1 { return Err("Choose a Wi-Fi network in LG settings first".into()); }
    profiles[0].pointer("/wifiProfile/profileId").and_then(Json::as_u64).and_then(|n| u32::try_from(n).ok()).filter(|n| *n > 0)
        .ok_or_else(|| "The saved Wi-Fi network has no profile number".into())
}

fn join_saved(changes: &mut Changes) -> Result<()> {
    if changes.device().sharing()? { return Err("Screen sharing has priority over extra Wi-Fi downloads".into()); }
    let wifi = changes.device().wifi()?;
    if matches!(wifi_value(&Key::WifiProfile, &wifi)?, Value::Number(n) if n > 0) { return Ok(()); }
    changes.set(Key::WifiPower, Value::Number(1))?;
    let profiles = changes.device().call("luna://com.webos.service.wifi/getprofilelist", json!({}))?;
    let profile = saved_profile(&profiles)?;
    changes.set(Key::WifiProfile, Value::Number(profile))
}

fn register() -> Json {
    let field = |key: &str, label: &str, help: &str| json!({"key":key,"label":label,"help":help,
        "kind":{"type":"read-only","value":{"capability":format!("plugin.{ID}.{key}")}},"apply":"on-commit"});
    json!({"title":OPT_IN.spec.name,"settings":{"format":1,"owner":ID,"revision":1,"title":OPT_IN.spec.name,
        "sections":[{"id":"links","title":"Connections","fields":[
            field("status","Status","Only apps that support Multi Link can combine connections. IPv4 only; each app checks its server before using a second link."),
            field("ethernet","Built-in Ethernet","Observed download speed now and the highest sample this session; not a speed-test estimate."),
            field("usb","USB Ethernet","A supported gigabit adapter is the simplest way past the built-in 100 Mbps port. Plug it in and let LG obtain its address."),
            field("wifi","Wi-Fi","Uses the network already saved by LG. Weak signal or screen sharing removes Wi-Fi from new downloads."),
            field("bulk","Downloads","Supported apps try the fastest healthy link first and add a second only when needed. They leave room on the wire for controls."),
            field("controls","Remote, voice and app controls","LG's wired default route is kept. Private Wi-Fi host routes using the same gateway are temporarily moved to it. IPv6 and special public routes are not changed.")]}]}})
}

fn confirmed_screen(ctx: &Ctx) -> bool {
    ctx.hub.get(crate::screen::TOPIC).is_some_and(|s| s.get("on") == Some(&Json::Bool(true)) && s.get("source").and_then(Json::as_str) != Some("assumed"))
}

fn inactive(ctx: &Ctx, enabled: bool, reason: &str, generation: u64) {
    ctx.hub.set_ignoring(TOPIC, json!({"schema":1,"boot":ctx.hub.boot(),"enabled":enabled,"active":false,
        "generation":generation,"reason":reason,"sampledUptimeMs":uptime_ms().unwrap_or(0),"validForMs":0,
        "controlInterface":null,"candidates":[],"rates":[],"ipv4Only":true,"sourceProbeRequired":true}), &["sampledUptimeMs"]);
}

fn page_facts(page: &Option<Arc<Local>>, status: &Json, reason: &str) {
    let Some(page) = page else { return };
    let links = status.get("candidates").and_then(Json::as_array);
    let rates = status.get("rates").and_then(Json::as_array);
    let line = |kind: &str| -> String {
        let Some(link) = links.and_then(|list| list.iter().find(|l| l["kind"] == kind)) else { return "Not available for downloads".into() };
        let interface = link["interface"].as_str().unwrap_or("");
        let Some(rate) = rates.and_then(|list| list.iter().find(|r| r["interface"] == interface)) else { return format!("{} — waiting for a traffic sample", link["address"].as_str().unwrap_or("")) };
        format!("{} — {:.1} Mbps now, {:.1} peak", link["address"].as_str().unwrap_or(""), rate["rxMbps"].as_f64().unwrap_or(0.0), rate["peakRxMbps"].as_f64().unwrap_or(0.0))
    };
    let bulk = links.filter(|l| !l.is_empty()).map(|l| l.iter().filter_map(|v| v["interface"].as_str()).collect::<Vec<_>>().join(" + ")).unwrap_or_else(|| "Normal wired connection".into());
    let controls = status["controlInterface"].as_str().unwrap_or("Normal wired connection");
    let _ = page.say(json!({"t":"facts.put","facts":{"status":reason,"ethernet":line("ethernet"),"usb":line("usb-ethernet"),
        "wifi":line("wifi"),"bulk":format!("{bulk}; supported apps only"),"controls":controls}}));
}

fn handler(local: &Local, message: plugin_api::wire::ToPlugin) {
    match message {
        plugin_api::wire::ToPlugin::SettingsApply { txn, values, .. } => {
            let _ = local.say(json!({"t":"settings.result","txn":txn,"outcome":{"type":"applied","values":values}}));
        }
        plugin_api::wire::ToPlugin::ActionInvoke { op, .. } => { let _ = local.answer(op, Err("This page only reports connection status")); }
        _ => {}
    }
}

pub fn spawn(ctx: &Ctx, shared: Arc<Shared>) -> std::io::Result<JoinHandle<()>> {
    let hub = ctx.hub.clone();
    shared.offer(&OPT_IN, Arc::new(move || { hub.emit(WAKE, "switch", json!({})); }));
    let ctx = ctx.clone();
    let guard_shared = shared.clone();
    crate::plugins::guard::spawn("multi-link", 256 * 1024, guard_shared, &[ID], ctx.stop.clone(), move |_| run(&ctx, &shared))
}

fn run(ctx: &Ctx, shared: &Arc<Shared>) {
    let open = || {
        let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").map_err(|e| e.to_string())?;
        Transaction::open(Lg { ctx: ctx.clone(), shared: shared.clone() }, FileJournal::new(ctx.paths.data_dir.join(ID)), boot.trim().into())
    };
    let mut changes = match open() {
        Ok(changes) => changes,
        Err(e) => {
            let reason = format!("Wired only: {e}");
            let topics = [WAKE.to_string()];
            let mut seq = ctx.hub.events_after(None, None, None).seq;
            let mut page = None;
            while !ctx.stop.is_set() {
                let enabled = shared.switched_on(ID);
                if enabled && page.is_none() { page = Local::start(shared, &OPT_IN.spec, register(), Arc::new(handler)).ok(); }
                if !enabled { page = None; }
                inactive(ctx, enabled, &reason, 0);
                if let Some(status) = ctx.hub.get(TOPIC) { page_facts(&page, &status, &reason); }
                seq = ctx.hub.wait_events(Some(seq), None, Some(&topics), Duration::from_secs(30)).seq;
            }
            return;
        }
    };
    let mut page = None;
    let mut meter = Meter::default();
    let mut generation = 0;
    let mut identity = Json::Null;
    let mut next_join = 0;
    let mut wifi_allowed = false;
    let mut wifi_reason = String::new();
    let topics = [WAKE.to_string(), crate::screen::TOPIC.to_string()];
    let mut sequence = ctx.hub.events_after(None, None, None).seq;
    let mut recovery = changes.restore().err();
    loop {
        let enabled = shared.switched_on(ID);
        if enabled && page.is_none() {
            match Local::start(shared, &OPT_IN.spec, register(), Arc::new(handler)) {
                Ok(registered) => page = Some(registered),
                Err(e) => recovery = Some(e),
            }
        } else if !enabled { page = None; }
        let awake = confirmed_screen(ctx);
        let active = enabled && awake && page.is_some() && !ctx.stop.is_set();
        let mut reason = if !enabled { "Off".to_string() } else { "Paused while the screen is off".to_string() };
        if !active || recovery.is_some() {
            generation += u64::from(identity != Json::Null);
            identity = Json::Null;
            inactive(ctx, enabled, &reason, generation);
            recovery = changes.restore().err();
            next_join = 0;
            meter = Meter::default();
            if let Some(e) = &recovery { reason = format!("Restoration pending: {e}"); inactive(ctx, enabled, &reason, generation); }
        } else {
            match Linux.snapshot().and_then(|mut snapshot| {
                // Joining never happens until Ethernet already owns the default route.
                multilink::plan(&snapshot, false)?;
                let now = uptime_ms()?;
                if now >= next_join {
                    wifi_reason = match join_saved(&mut changes) {
                        Ok(()) => { wifi_allowed = true; String::new() },
                        Err(e) => { wifi_allowed = false; e },
                    };
                    next_join = uptime_ms()?.saturating_add(15_000);
                    snapshot = Linux.snapshot()?;
                }
                if !changes.device().allowed() { return Err("Network preparation is no longer enabled".into()); }
                let now = uptime_ms()?;
                let mut selected = multilink::plan(&snapshot, wifi_allowed)?;
                changes.network(&selected.settings)?;
                let rates = meter.sample(now, &snapshot.links);
                selected.candidates.sort_by(|a, b| {
                    let speed = |l: &multilink::Link| if l.kind == Kind::Wifi {
                        rates.iter().find(|r| r.interface == l.interface).map_or(0.0, |r| r.peak_rx_mbps)
                    } else { f64::from(l.nominal_mbps.unwrap_or(0)) };
                    speed(b).total_cmp(&speed(a))
                });
                let identities = json!(selected.candidates.iter().map(|l| (&l.interface, l.address, l.prefix, l.carrier_changes)).collect::<Vec<_>>());
                if identities != identity { generation += 1; identity = identities; }
                reason = if selected.candidates.len() > 1 { "Ready for supported apps".into() }
                    else if wifi_reason.is_empty() { "Wired only; waiting for another healthy link".into() }
                    else { format!("Wired only: {wifi_reason}") };
                ctx.hub.set_ignoring(TOPIC, json!({"schema":1,"boot":ctx.hub.boot(),"enabled":true,"active":true,
                    "generation":generation,"sampledUptimeMs":now,"validForMs":multilink::VALID_FOR_MS,"reason":reason,
                    "controlInterface":selected.control,"candidates":selected.candidates,"rates":rates,
                    "ipv4Only":true,"sourceProbeRequired":true}), &["sampledUptimeMs"]);
                Ok(())
            }) {
                Ok(()) => {},
                Err(error) => {
                    generation += u64::from(identity != Json::Null);
                    identity = Json::Null;
                    reason = format!("Wired only: {error}");
                    inactive(ctx, enabled, &reason, generation);
                    recovery = changes.restore().err();
                    next_join = 0;
                }
            }
        }
        if let Some(status) = ctx.hub.get(TOPIC) { page_facts(&page, &status, &reason); }
        if ctx.stop.is_set() { break; }
        let wait = if active && recovery.is_none() { Duration::from_millis(500) } else { Duration::from_secs(30) };
        sequence = ctx.hub.wait_events(Some(sequence), None, Some(&topics), wait).seq;
    }
    let reason = changes.restore().err().map_or_else(|| "Helper stopped".into(), |e| format!("Restoration pending: {e}"));
    inactive(ctx, false, &reason, generation + 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_link_uses_only_one_existing_profile_and_never_a_password() {
        let list = json!({"profileList":[{"wifiProfile":{"profileId":777,"ssid":"saved"}}]});
        assert_eq!(saved_profile(&list).unwrap(), 777);
        assert!(saved_profile(&json!({"profileList":[]})).is_err());
        assert!(saved_profile(&json!({"profileList":[{},{}]})).is_err());
        let status = json!({"status":"connectionStateChanged","networkInfo":{"profileId":777,"connectState":"ipConfigured"}});
        assert_eq!(wifi_value(&Key::WifiProfile, &status).unwrap(), Value::Number(777));
        assert_eq!(wifi_value(&Key::WifiPower, &status).unwrap(), Value::Number(1));
        assert_eq!(wifi_value(&Key::WifiPower, &json!({"status":"serviceDisabled"})).unwrap(), Value::Number(0));
        assert!(wifi_value(&Key::WifiPower, &json!({})).is_err());
        assert_eq!(wifi_value(&Key::WifiProfile, &json!({"networkInfo":{"profileId":777,"connectState":"notConnected"}})).unwrap(), Value::Number(0));
        assert_eq!(wifi_value(&Key::WifiProfile, &json!({"networkInfo":{"profileId":777,"connectState":"associating"}})).unwrap(), Value::Number(777));
        assert!(wifi_value(&Key::WifiProfile, &json!({"networkInfo":{"profileId":777,"connectState":"unknown"}})).is_err());
    }

    #[test]
    fn multi_link_page_uses_the_existing_schema_renderer_and_read_only_facts() {
        let register: plugin_api::wire::Register = serde_json::from_value(register()).unwrap();
        assert!(register.settings.is_some());
        let spec = super::register();
        let fields = spec["settings"]["sections"][0]["fields"].as_array().unwrap();
        assert_eq!(fields.len(), 6);
        assert!(fields.iter().all(|f| f["kind"]["type"] == "read-only"));
        assert_eq!(OPT_IN.spec.permissions, &["settings"]);
    }
}

/// Core ARP flux guard: runs whether or not Multi Link is on, since Multi Link's own
/// restore writes the original (0) values back when it switches off.
pub fn spawn_arp_guard(ctx: &Ctx) -> std::io::Result<JoinHandle<()>> {
    let stop = ctx.stop.clone();
    thread::Builder::new().name("arp-guard".into()).stack_size(128 * 1024).spawn(move || {
        let mut last = String::new();
        while !stop.is_set() {
            let note = match multilink::arp::apply() { Ok(0) => String::new(), Ok(n) => format!("applied {n} ARP values"), Err(e) => e };
            if !note.is_empty() && note != last { eprintln!("arp-guard: {note}"); }
            last = note;
            for _ in 0..10 { if stop.is_set() { return; } thread::sleep(Duration::from_millis(500)); }
        }
    })
}
