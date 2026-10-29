//! Network (`settings/PageNetwork.qml`, mocks `30-settings-network`), Join Network (`PageWifi.qml`) and IP Settings (`PageIp.qml`).
//!
//! Network shows what the connection manager says about the links and the one network setting that matters to a house with Home Assistant,
//! waking the TV over the network. Joining another network and a fixed address are network entrances, so they are done at the remote only:
//! the typed text comes from the on-screen keyboard (the password as bullets), a question follows (the TV may leave the network it is on),
//! and the password goes straight to `com.webos.service.wifi/connect` and is kept nowhere. LG's Network app is not in the Settings bundle:
//! the method names are webOS's own Wi-Fi service and connection manager, and have not been run on this TV. IP Settings holds a draft of the
//! addresses; nothing changes until Apply, and Apply asks first because wrong addresses cut the TV off from the network.

use super::util::{go, net};
use crate::settings::ctx::{Ctx, Spec, ask};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Op, Row};
use serde_json::{Value, json};
use std::rc::Rc;

pub fn page() -> Page {
    Page::new("network", "Network", "wifi").rail().tabs(&["Wi-Fi", "Wired", "Details"]).reads(reads).rows(rows).summary(summary)
}

pub fn wifi() -> Page {
    Page::new("wifi", "Join Network", "wifi").parent("network").back("Network").reads(no_reads).rows(wifi_rows).answered(wifi_answered).opened(scan)
}

pub fn ip() -> Page {
    Page::new("ipsettings", "IP Settings", "router").parent("network").back("Network").reads(no_reads).rows(ip_rows).answered(ip_answered).opened(ip_opened)
}

fn no_reads(_: &Ctx) -> Vec<Read> {
    Vec::new()
}

fn summary(c: &Ctx) -> String {
    let n = net(c);
    if n.wired {
        "Wired".into()
    } else if n.wifi {
        let strength = n.strength();
        format!("{}{}", if n.ssid.is_empty() { "Wi-Fi" } else { &n.ssid }, if strength.is_empty() { String::new() } else { format!(" \u{b7} {strength}") })
    } else {
        "Not connected".into()
    }
}

fn reads(_: &Ctx) -> Vec<Read> {
    vec![Read::lg("network", &["wolwowlOnOff", "deviceName", "ipv6Enable", "bleAdvertisingOnOff"])]
}

fn rows(c: &Ctx) -> Vec<Row> {
    let n = net(c);
    let mut rows = Vec::new();

    // ---- Wi-Fi
    let name = if n.wifi && !n.ssid.is_empty() { n.ssid.as_str() } else { "Wi-Fi" };
    rows.push(Row::info("wifistate").tab(0).icon("wifi").label(name).value(if n.wifi { if n.strength().is_empty() { "Connected" } else { n.strength() } } else { "Not connected" }));
    rows.push(Row::info("ipaddr").tab(0).icon("router").label("IP address").value(if n.ip.is_empty() { "None" } else { &n.ip }));
    rows.push(Row::link("join").tab(0).icon("wifi").label("Join Network").does(go("wifi")));
    rows.push(Row::link("ip").tab(0).icon("router").label("IP Settings").does(go("ipsettings")));
    rows.push(c.toggle(Spec::new("network.wolwowlOnOff").tab(0).on_off("true", "false").icon("power").label("Wake from the network").help("Lets Home Assistant turn the TV on (Wake-on-LAN).")));

    // ---- Wired
    rows.push(Row::info("wired").tab(1).icon("ethernet-port").label("Wired connection").value(if n.wired { "Connected" } else { "Unplugged" }));
    rows.push(Row::info("ipaddr2").tab(1).icon("router").label("IP address").value(if n.ip.is_empty() { "None" } else { &n.ip }).visible(n.wired));
    rows.push(Row::link("ip2").tab(1).icon("router").label("IP Settings").does(go("ipsettings")));

    // ---- Details
    let signal = if !n.wifi {
        "No link".to_string()
    } else if !n.dbm.is_empty() {
        format!("{} dBm", n.dbm)
    } else if !n.strength().is_empty() {
        n.strength().to_string()
    } else {
        "Not reported".to_string()
    };
    rows.push(Row::info("signal").tab(2).icon("signal").label("Signal").value(&signal));
    rows.push(Row::info("band").tab(2).icon("radio").label("Band").value(&n.band).visible(n.wifi && !n.band.is_empty()));
    rows.push(
        Row::info("method")
            .tab(2)
            .icon("globe")
            .label("Address")
            .value(match n.method.as_str() {
                "dhcp" => "Automatic",
                "manual" => "Manual",
                _ => "",
            })
            .visible(!n.method.is_empty())
            .help("Automatic means the router hands it out (DHCP)."),
    );
    let mut tv = Row::link("tvname").tab(2).icon("tv").label("TV Name").help("The name on the network and in Home Assistant.").value(&c.lg.opt_text("network.deviceName").unwrap_or_default()).admin();
    tv.enabled = c.lg.usable("network.deviceName");
    tv.visible = c.lg.shown("network.deviceName");
    let current = c.lg.opt_text("network.deviceName").unwrap_or_default();
    rows.push(tv.does(vec![Op::Entry {
        prompt: "TV name".into(),
        text: current,
        secret: false,
        then: Rc::new(|t| if t.trim().is_empty() { Vec::new() } else { vec![Op::Lg { ds: "network.deviceName".into(), value: json!(t.trim()), app: None }] }),
    }]));
    rows.push(c.toggle(Spec::new("network.ipv6Enable").tab(2).on_off("true", "false").icon("globe").label("IPv6").lg_shown()));
    rows.push(c.toggle(Spec::new("network.bleAdvertisingOnOff").tab(2).icon("radio").label("TV On With Mobile").help("Lets a phone turn the TV on over Bluetooth. Uses a little power while off.").lg_shown()));
    rows
}

// ------------------------------------------------------------------------------------------------------------------- Join Network

const WIFI: &str = "luna://com.webos.service.wifi";

fn scan(_: &Ctx) -> Vec<Op> {
    vec![Op::Set("wifi.scanning".into(), json!(true)), Op::Set("wifi.status".into(), json!("")), Op::Call { name: "wifiscan".into(), uri: format!("{WIFI}/findnetworks"), payload: json!({}), subscribe: false }]
}

/// A network the scan found is secured when it offers any security type but none or open.
fn secured(n: &Value) -> bool {
    n.get("availableSecurityTypes").and_then(Value::as_array).is_some_and(|t| t.iter().any(|v| v != "none" && v != "open"))
}

fn wifi_answered(_: &Ctx, name: &str, doc: &Value) -> Vec<Op> {
    let ok = doc.get("returnValue").and_then(Value::as_bool) != Some(false) && !doc.is_null();
    match name {
        "wifiscan" => {
            let mut ops = vec![Op::Set("wifi.scanning".into(), json!(false))];
            if !ok {
                ops.push(Op::Set("wifi.nets".into(), json!([])));
                ops.push(Op::Set("wifi.status".into(), json!("No answer from the Wi-Fi")));
                return ops;
            }
            let found: Vec<Value> = doc
                .get("foundNetworks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|e| {
                    let n = e.get("networkInfo").unwrap_or(e);
                    let ssid = n.get("ssid").and_then(Value::as_str).filter(|s| !s.is_empty())?;
                    Some(json!({"ssid": ssid, "secured": secured(n), "bars": n.get("signalBars").and_then(Value::as_i64).unwrap_or(0)}))
                })
                .collect();
            ops.push(Op::Set("wifi.nets".into(), Value::Array(found)));
            ops
        }
        "wifijoin" => vec![Op::Set("wifi.status".into(), json!(if ok { "Joined" } else { "Could not join" }))],
        _ => Vec::new(),
    }
}

fn bars_text(b: i64) -> &'static str {
    match b {
        3.. => "Strong",
        2 => "Good",
        1 => "Weak",
        _ => "",
    }
}

/// The question, then the join; the password lives in this closure until the join is sent.
fn join(ssid: &str, pass: &str) -> Vec<Op> {
    let mut payload = json!({"ssid": ssid});
    if !pass.is_empty() {
        payload["security"] = json!({"securityType": "psk", "simpleSecurity": {"passKey": pass}});
    }
    ask(
        "wifi",
        &format!("Join {ssid}?"),
        "",
        "Join",
        "Cancel",
        false,
        vec![Op::Set("wifi.status".into(), json!(format!("Joining {ssid}"))), Op::Call { name: "wifijoin".into(), uri: format!("{WIFI}/connect"), payload, subscribe: false }],
    )
}

fn password(ssid: &str, then: impl Fn(&str, &str) -> Vec<Op> + 'static) -> Op {
    let ssid = ssid.to_string();
    Op::Entry { prompt: format!("Password for {ssid}"), text: String::new(), secret: true, then: Rc::new(move |text| then(&ssid, &text)) }
}

fn wifi_rows(c: &Ctx) -> Vec<Row> {
    let n = net(c);
    let scanning = c.local.flag("wifi.scanning");
    let status = c.local.text("wifi.status");
    let mut rows = Vec::new();
    if scanning || !status.is_empty() {
        rows.push(Row::note(if scanning { "Looking for networks" } else { &status }, 0));
    }
    for e in c.local.get("wifi.nets").as_array().into_iter().flatten() {
        let ssid = e.get("ssid").and_then(Value::as_str).unwrap_or("").to_string();
        let secure = e.get("secured").and_then(Value::as_bool) == Some(true);
        let bars = e.get("bars").and_then(Value::as_i64).unwrap_or(0);
        let value = format!("{}{}", bars_text(bars), if secure { format!("{}Secured", if bars > 0 { " \u{b7} " } else { "" }) } else { String::new() });
        let ops = if secure { vec![password(&ssid, |s, p| if p.is_empty() { Vec::new() } else { join(s, p) })] } else { join(&ssid, "") };
        rows.push(Row::link(&format!("net:{ssid}")).icon("wifi").label(&ssid).value(&value).chevron(false).selected(n.wifi && n.ssid == ssid).admin().does(ops));
    }
    rows.push(Row::link("hidden").icon("eye-off").label("Other Network").admin().does(vec![Op::Entry {
        prompt: "Network name".into(),
        text: String::new(),
        secret: false,
        then: Rc::new(|ssid| if ssid.is_empty() { Vec::new() } else { vec![password(&ssid, join)] }),
    }]));
    let mut again = Row::action("rescan", "Search").icon("refresh-cw").label("Search Again").admin().does(scan(c));
    again.enabled = !scanning;
    rows.push(again);
    rows
}

// ------------------------------------------------------------------------------------------------------------------- IP Settings

const CM: &str = "luna://com.webos.service.connectionmanager";

fn ip_opened(_: &Ctx) -> Vec<Op> {
    vec![Op::Set("ip.status".into(), json!("")), Op::Call { name: "ipstatus".into(), uri: format!("{CM}/getStatus"), payload: json!({}), subscribe: false }]
}

/// The link that is up and its addresses, as the connection manager gives them.
fn ip_answered(_: &Ctx, name: &str, doc: &Value) -> Vec<Op> {
    let ok = doc.get("returnValue").and_then(Value::as_bool) != Some(false) && !doc.is_null();
    match name {
        "ipstatus" => {
            if !ok {
                return vec![Op::Set("ip.link".into(), json!(""))];
            }
            let up = |k: &str| doc.get(k).filter(|l| l.get("state").and_then(Value::as_str) == Some("connected"));
            let (link, l) = match (up("wired"), up("wifi")) {
                (Some(w), _) => ("wired", Some(w)),
                (None, Some(f)) => ("wifi", Some(f)),
                _ => ("", None),
            };
            let text = |v: Option<&Value>, keys: &[&str]| keys.iter().find_map(|k| v.and_then(|v| v.get(*k)).and_then(Value::as_str).filter(|s| !s.is_empty())).unwrap_or("").to_string();
            vec![
                Op::Set("ip.link".into(), json!(link)),
                Op::Set("ip.ssid".into(), json!(text(doc.get("wifi"), &["ssid"]))),
                Op::Set("ip.manual".into(), json!(l.and_then(|l| l.get("method")).and_then(Value::as_str) == Some("manual"))),
                Op::Set("ip.ip".into(), json!(text(l, &["ipAddress"]))),
                Op::Set("ip.mask".into(), json!(text(l, &["netmask", "subnetMask"]))),
                Op::Set("ip.gateway".into(), json!(text(l, &["gateway"]))),
                Op::Set("ip.dns".into(), json!(text(l, &["dns1"]))),
            ]
        }
        "ipapply" => vec![Op::Set("ip.status".into(), json!(if ok { "Applied" } else { "Not applied" }))],
        _ => Vec::new(),
    }
}

/// Four numbers of at most three digits, each up to 255.
pub fn valid(a: &str) -> bool {
    let parts: Vec<&str> = a.split('.').collect();
    parts.len() == 4 && parts.iter().all(|p| (1..=3).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()) && p.parse::<u32>().is_ok_and(|n| n <= 255))
}

struct Draft {
    link: String,
    ssid: String,
    manual: bool,
    ip: String,
    mask: String,
    gateway: String,
    dns: String,
}

impl Draft {
    fn of(c: &Ctx) -> Draft {
        Draft {
            link: c.local.text("ip.link"),
            ssid: c.local.text("ip.ssid"),
            manual: c.local.flag("ip.manual"),
            ip: c.local.text("ip.ip"),
            mask: c.local.text("ip.mask"),
            gateway: c.local.text("ip.gateway"),
            dns: c.local.text("ip.dns"),
        }
    }

    fn ready(&self) -> bool {
        !self.link.is_empty() && (!self.manual || (valid(&self.ip) && valid(&self.mask) && valid(&self.gateway) && valid(&self.dns)))
    }

    /// `setipv4` for the link that is up, then `setdns` for a fixed address.
    fn apply(&self) -> Vec<Op> {
        let wifi = self.link == "wifi";
        let mut p = json!({"method": if self.manual { "manual" } else { "dhcp" }});
        if wifi {
            p["ssid"] = json!(self.ssid);
        }
        if self.manual {
            p["address"] = json!(self.ip);
            p["netmask"] = json!(self.mask);
            p["gateway"] = json!(self.gateway);
        }
        let mut ops = vec![Op::Call { name: "ipapply".into(), uri: format!("{CM}/setipv4"), payload: p, subscribe: false }];
        if self.manual {
            let mut d = json!({"dns": [self.dns]});
            if wifi {
                d["ssid"] = json!(self.ssid);
            }
            ops.push(Op::Call { name: "ipdns".into(), uri: format!("{CM}/setdns"), payload: d, subscribe: false });
        }
        ops.push(Op::Set("ip.status".into(), json!("Applying")));
        ops
    }
}

fn field(d: &Draft, id: &'static str, icon: &str, label: &'static str, value: &str) -> Row {
    let mut r = Row::link(id).icon(icon).label(label).value(value).admin();
    r.enabled = d.manual && !d.link.is_empty();
    r.detail = if d.manual {
        if valid(value) || value.is_empty() { String::new() } else { "Not an address".into() }
    } else {
        "Set to Manual".into()
    };
    r.does(vec![Op::Entry { prompt: label.into(), text: value.to_string(), secret: false, then: Rc::new(move |t| vec![Op::Set(format!("ip.{id}"), json!(t.trim()))]) }])
}

fn ip_rows(c: &Ctx) -> Vec<Row> {
    let d = Draft::of(c);
    let status = c.local.text("ip.status");
    let mut rows = Vec::new();
    rows.push(Row::note(if d.link.is_empty() { "No network link" } else { &status }, 0).visible(d.link.is_empty() || !status.is_empty()));
    let mut method = Row::seg("method", &["Automatic", "Manual"]).icon("router").label("IP Address Setting").index(Some(usize::from(d.manual))).admin();
    method.enabled = !d.link.is_empty();
    rows.push(method.act(|a| match a {
        Arg::Index(i) => vec![Op::Set("ip.manual".into(), json!(i == 1))],
        _ => Vec::new(),
    }));
    rows.push(field(&d, "ip", "globe", "IP Address", &d.ip));
    rows.push(field(&d, "mask", "network", "Subnet Mask", &d.mask));
    rows.push(field(&d, "gateway", "router", "Gateway", &d.gateway));
    rows.push(field(&d, "dns", "server", "DNS Server", &d.dns));
    let ready = d.ready();
    let mut apply = Row::action("apply", "Apply").icon("check").label("Apply").admin();
    apply.enabled = ready;
    apply.detail = if !d.link.is_empty() && !ready { "Check the addresses".into() } else { String::new() };
    rows.push(apply.does(ask("router", "Change network settings?", "The TV may lose its connection if they are wrong.", "Apply", "Cancel", false, d.apply())));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::{World, lg_with};

    fn by_id<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id} in {:?}", rows.iter().map(|r| &r.id).collect::<Vec<_>>()))
    }

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

    fn on_wifi() -> World {
        let mut w = World::new(lg_with(&[("network", json!({"settings": {"wolwowlOnOff": "true", "deviceName": "Living room", "ipv6Enable": "false"}}))], &[]));
        w.docs.set("net", json!({"returnValue": true, "wifi": {"state": "connected", "ssid": "Nate's Home", "signalLevel": -50, "ipAddress": "192.168.1.9", "channelFrequency": 5180, "method": "dhcp"}, "wired": {"state": "disconnected"}}));
        w
    }

    #[test]
    fn the_network_page_says_which_link_is_up_and_how_strong() {
        let w = on_wifi();
        assert_eq!(summary(&w.ctx()), "Nate's Home \u{b7} Strong");
        let r = rows(&w.ctx());
        assert_eq!((by_id(&r, "wifistate").label.as_str(), by_id(&r, "wifistate").value.as_str()), ("Nate's Home", "Strong"));
        assert_eq!(by_id(&r, "ipaddr").value, "192.168.1.9");
        assert_eq!(by_id(&r, "wired").value, "Unplugged");
        assert!(!by_id(&r, "ipaddr2").visible, "no address to show for a cable that is out");
        assert_eq!((by_id(&r, "signal").value.as_str(), by_id(&r, "band").value.as_str(), by_id(&r, "method").value.as_str()), ("-50 dBm", "5 GHz", "Automatic"));
        assert!(by_id(&r, "band").visible && by_id(&r, "method").visible);
        assert!(by_id(&r, "network.wolwowlOnOff").on, "LG's true is on");
        assert!(!by_id(&r, "network.ipv6Enable").on);
        let ops = (by_id(&r, "network.wolwowlOnOff").act.as_ref().unwrap())(Arg::Toggle(false));
        assert!(matches!(&ops[..], [Op::Lg { value, .. }] if value == "false"));
        let j = (by_id(&r, "join").act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&j[..], [Op::Open { page, .. }] if page == "wifi"));
        // a cable
        let mut w = on_wifi();
        w.docs.set("net", json!({"returnValue": true, "wired": {"state": "connected", "ipAddress": "192.168.1.20", "method": "manual"}, "wifi": {"state": "disconnected"}}));
        assert_eq!(summary(&w.ctx()), "Wired");
        let r = rows(&w.ctx());
        assert!(by_id(&r, "ipaddr2").visible && by_id(&r, "ipaddr2").value == "192.168.1.20" && by_id(&r, "wired").value == "Connected");
        assert_eq!((by_id(&r, "signal").value.as_str(), by_id(&r, "method").value.as_str(), by_id(&r, "wifistate").value.as_str()), ("No link", "Manual", "Not connected"));
        assert!(!by_id(&r, "band").visible);
        let w = World::new(lg_with(&[], &[]));
        assert_eq!(summary(&w.ctx()), "Not connected");
        assert_eq!(by_id(&rows(&w.ctx()), "ipaddr").value, "None");
    }

    #[test]
    fn the_tv_name_is_typed_and_a_blank_one_is_not_written() {
        let w = on_wifi();
        let r = rows(&w.ctx());
        assert_eq!(by_id(&r, "tvname").value, "Living room");
        let ops = (by_id(&r, "tvname").act.as_ref().unwrap())(Arg::Press);
        let [Op::Entry { prompt, text, secret, then }] = &ops[..] else { panic!("{ops:?}") };
        assert_eq!((prompt.as_str(), text.as_str(), *secret), ("TV name", "Living room", false));
        assert!(then("   ".into()).is_empty());
        assert!(matches!(&then(" Den ".into())[..], [Op::Lg { ds, value, .. }] if ds == "network.deviceName" && value == "Den"));
    }

    fn scan_doc() -> Value {
        json!({"returnValue": true, "foundNetworks": [
            {"networkInfo": {"ssid": "Nate's Home", "availableSecurityTypes": ["psk"], "signalBars": 3}},
            {"networkInfo": {"ssid": "Cafe", "availableSecurityTypes": ["none"], "signalBars": 1}},
            {"networkInfo": {"ssid": "", "signalBars": 2}},
            {"ssid": "Flat", "availableSecurityTypes": ["open", "psk"]}]})
    }

    #[test]
    fn the_scan_lists_networks_with_their_strength_and_whether_they_are_secured() {
        let mut w = on_wifi();
        let ops = scan(&w.ctx());
        let rest = settle(&mut w, ops);
        assert!(matches!(&rest[..], [Op::Call { name, uri, .. }] if name == "wifiscan" && uri.ends_with("/findnetworks")));
        assert_eq!(by_id(&wifi_rows(&w.ctx()), "note:Looking for networks").tab, 0);
        assert!(!by_id(&wifi_rows(&w.ctx()), "rescan").enabled);
        let ops = wifi_answered(&w.ctx(), "wifiscan", &scan_doc());
        settle(&mut w, ops);
        let r = wifi_rows(&w.ctx());
        assert_eq!(r.iter().filter(|x| x.id.starts_with("net:")).count(), 3, "a network with no name is not listed");
        assert_eq!(by_id(&r, "net:Nate's Home").value, "Strong \u{b7} Secured");
        assert!(by_id(&r, "net:Nate's Home").selected && !by_id(&r, "net:Cafe").selected, "the one the TV is on is marked");
        assert_eq!((by_id(&r, "net:Cafe").value.as_str(), by_id(&r, "net:Flat").value.as_str()), ("Weak", "Secured"));
        assert!(by_id(&r, "rescan").enabled);
        // a failed scan says so
        let ops = wifi_answered(&w.ctx(), "wifiscan", &json!({"returnValue": false}));
        settle(&mut w, ops);
        let r = wifi_rows(&w.ctx());
        assert_eq!(by_id(&r, "note:No answer from the Wi-Fi").text, "No answer from the Wi-Fi");
        assert!(r.iter().all(|x| !x.id.starts_with("net:")));
    }

    #[test]
    fn joining_asks_for_the_password_then_a_question_and_the_password_goes_only_to_the_wifi_service() {
        let mut w = on_wifi();
        let ops = wifi_answered(&w.ctx(), "wifiscan", &scan_doc());
        settle(&mut w, ops);
        let r = wifi_rows(&w.ctx());
        let ops = (by_id(&r, "net:Nate's Home").act.as_ref().unwrap())(Arg::Press);
        let [Op::Entry { prompt, secret, then, .. }] = &ops[..] else { panic!("{ops:?}") };
        assert_eq!((prompt.as_str(), *secret), ("Password for Nate's Home", true));
        assert!(then(String::new()).is_empty(), "no password, no join");
        let q = then("hunter2".into());
        let [Op::Ask(a)] = &q[..] else { panic!("{q:?}") };
        assert_eq!((a.title.as_str(), a.answers[0].label.as_str(), a.answers[1].label.as_str()), ("Join Nate's Home?", "Cancel", "Join"));
        assert!(matches!(&a.answers[1].then[..], [Op::Set(k, s), Op::Call { name, uri, payload, .. }]
            if k == "wifi.status" && s == "Joining Nate's Home" && name == "wifijoin" && uri.ends_with("wifi/connect")
                && payload == &json!({"ssid": "Nate's Home", "security": {"securityType": "psk", "simpleSecurity": {"passKey": "hunter2"}}})));
        assert!(!format!("{:?}", a.answers[1].then).contains("hunter2"), "a debug print of the join never shows the password");
        // an open network joins after the question alone
        let ops = (by_id(&r, "net:Cafe").act.as_ref().unwrap())(Arg::Press);
        let [Op::Ask(a)] = &ops[..] else { panic!("{ops:?}") };
        assert!(matches!(&a.answers[1].then[..], [_, Op::Call { payload, .. }] if payload == &json!({"ssid": "Cafe"})));
        // the answer
        let done = wifi_answered(&w.ctx(), "wifijoin", &json!({"returnValue": true}));
        assert!(matches!(&done[..], [Op::Set(_, s)] if s == "Joined"));
        let no = wifi_answered(&w.ctx(), "wifijoin", &json!({"returnValue": false}));
        assert!(matches!(&no[..], [Op::Set(_, s)] if s == "Could not join"));
    }

    #[test]
    fn another_network_asks_for_its_name_then_its_password() {
        let w = on_wifi();
        let r = wifi_rows(&w.ctx());
        let ops = (by_id(&r, "hidden").act.as_ref().unwrap())(Arg::Press);
        let [Op::Entry { prompt, then, .. }] = &ops[..] else { panic!("{ops:?}") };
        assert_eq!(prompt, "Network name");
        assert!(then(String::new()).is_empty());
        let next = then("Garage".into());
        let [Op::Entry { prompt, secret, then, .. }] = &next[..] else { panic!("{next:?}") };
        assert_eq!((prompt.as_str(), *secret), ("Password for Garage", true));
        let q = then("pw".into());
        assert!(matches!(&q[..], [Op::Ask(a)] if a.title == "Join Garage?"));
        let open = then(String::new());
        let [Op::Ask(a)] = &open[..] else { panic!() };
        assert!(matches!(&a.answers[1].then[..], [_, Op::Call { payload, .. }] if payload == &json!({"ssid": "Garage"})), "an empty password is an open network");
    }

    fn status_doc(method: &str) -> Value {
        json!({"returnValue": true, "wifi": {"state": "connected", "ssid": "Nate's Home", "method": method, "ipAddress": "192.168.1.9", "netmask": "255.255.255.0", "gateway": "192.168.1.1", "dns1": "192.168.1.1"}, "wired": {"state": "disconnected"}})
    }

    #[test]
    fn ip_settings_read_the_link_that_is_up_and_a_fixed_address_needs_four_good_ones() {
        let mut w = on_wifi();
        let ops = ip_opened(&w.ctx());
        let rest = settle(&mut w, ops);
        assert!(matches!(&rest[..], [Op::Call { name, uri, .. }] if name == "ipstatus" && uri.ends_with("connectionmanager/getStatus")));
        let r = ip_rows(&w.ctx());
        assert!(by_id(&r, "note:No network link").visible && !by_id(&r, "method").enabled, "nothing is known until the manager answers");
        let ops = ip_answered(&w.ctx(), "ipstatus", &status_doc("dhcp"));
        settle(&mut w, ops);
        let r = ip_rows(&w.ctx());
        assert!(!by_id(&r, "note:").visible);
        assert_eq!((by_id(&r, "method").index, by_id(&r, "ip").value.as_str(), by_id(&r, "mask").value.as_str()), (Some(0), "192.168.1.9", "255.255.255.0"));
        assert!(!by_id(&r, "ip").enabled && by_id(&r, "ip").detail == "Set to Manual");
        assert!(by_id(&r, "apply").enabled, "automatic needs no address");
        let ops = (by_id(&r, "method").act.as_ref().unwrap())(Arg::Index(1));
        settle(&mut w, ops);
        let r = ip_rows(&w.ctx());
        assert!(by_id(&r, "ip").enabled && by_id(&r, "ip").detail.is_empty() && by_id(&r, "apply").enabled);
        // a wrong address says so and holds Apply back
        let edit = (by_id(&r, "gateway").act.as_ref().unwrap())(Arg::Press);
        let [Op::Entry { prompt, text, then, .. }] = &edit[..] else { panic!("{edit:?}") };
        assert_eq!((prompt.as_str(), text.as_str()), ("Gateway", "192.168.1.1"));
        let ops = then(" 192.168.1.300 ".into());
        assert!(matches!(&ops[..], [Op::Set(k, v)] if k == "ip.gateway" && v == "192.168.1.300"));
        settle(&mut w, ops);
        let r = ip_rows(&w.ctx());
        assert_eq!(by_id(&r, "gateway").detail, "Not an address");
        assert!(!by_id(&r, "apply").enabled && by_id(&r, "apply").detail == "Check the addresses");
    }

    #[test]
    fn apply_asks_first_and_sends_the_method_the_addresses_and_the_dns_for_the_link() {
        let mut w = on_wifi();
        let ops = ip_answered(&w.ctx(), "ipstatus", &status_doc("manual"));
        settle(&mut w, ops);
        let r = ip_rows(&w.ctx());
        let ops = (by_id(&r, "apply").act.as_ref().unwrap())(Arg::Press);
        let [Op::Ask(a)] = &ops[..] else { panic!("{ops:?}") };
        assert_eq!(a.title, "Change network settings?");
        let then = &a.answers[1].then;
        assert!(matches!(&then[..], [Op::Call { uri: u1, payload: p1, .. }, Op::Call { uri: u2, payload: p2, .. }, Op::Set(k, v)]
            if u1.ends_with("/setipv4") && p1 == &json!({"method": "manual", "ssid": "Nate's Home", "address": "192.168.1.9", "netmask": "255.255.255.0", "gateway": "192.168.1.1"})
                && u2.ends_with("/setdns") && p2 == &json!({"dns": ["192.168.1.1"], "ssid": "Nate's Home"}) && k == "ip.status" && v == "Applying"));
        // automatic: the method alone, no dns
        let mut w = on_wifi();
        let ops = ip_answered(&w.ctx(), "ipstatus", &status_doc("dhcp"));
        settle(&mut w, ops);
        let ops = (by_id(&ip_rows(&w.ctx()), "apply").act.as_ref().unwrap())(Arg::Press);
        let [Op::Ask(a)] = &ops[..] else { panic!() };
        assert!(matches!(&a.answers[1].then[..], [Op::Call { payload, .. }, Op::Set(..)] if payload == &json!({"method": "dhcp", "ssid": "Nate's Home"})));
        let done = ip_answered(&w.ctx(), "ipapply", &json!({"returnValue": true}));
        assert!(matches!(&done[..], [Op::Set(_, s)] if s == "Applied"));
        settle(&mut w, done);
        assert_eq!(by_id(&ip_rows(&w.ctx()), "note:Applied").text, "Applied");
        // a cable: no ssid
        let mut wired = status_doc("dhcp");
        wired["wired"] = json!({"state": "connected", "method": "dhcp", "ipAddress": "10.0.0.2"});
        let ops = ip_answered(&w.ctx(), "ipstatus", &wired);
        settle(&mut w, ops);
        assert_eq!((w.local.text("ip.link"), w.local.text("ip.ip")), ("wired".to_string(), "10.0.0.2".to_string()));
        let ops = (by_id(&ip_rows(&w.ctx()), "apply").act.as_ref().unwrap())(Arg::Press);
        let [Op::Ask(a)] = &ops[..] else { panic!() };
        assert!(matches!(&a.answers[1].then[..], [Op::Call { payload, .. }, Op::Set(..)] if payload == &json!({"method": "dhcp"})));
    }

    #[test]
    fn an_address_is_four_numbers_up_to_255() {
        for good in ["0.0.0.0", "192.168.1.1", "255.255.255.255"] {
            assert!(valid(good), "{good}");
        }
        for bad in ["", "1.2.3", "1.2.3.4.5", "256.1.1.1", "1.2.3.a", "1..2.3", "1.2.3.1000", " 1.2.3.4", "-1.2.3.4", "+1.2.3.4"] {
            assert!(!valid(bad), "{bad}");
        }
    }
}
