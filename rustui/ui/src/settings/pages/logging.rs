//! Remote logging settings and the helper's forwarding status.

use crate::settings::ctx::Ctx;
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Op, Row};
use serde_json::{Value, json};
use std::net::IpAddr;
use std::rc::Rc;

const PATH: &str = "/v1/settings/base.logging";

pub fn page() -> Page {
    Page::new("logging", "Logging", "server").parent("system").back("System").reads(reads).opened(opened).answered(answered).rows(rows)
}

fn reads(_: &Ctx) -> Vec<Read> {
    vec![Read::Get { name: "logging", path: "/v1/logging" }]
}

fn opened(_: &Ctx) -> Vec<Op> {
    vec![Op::Set("logging.busy".into(), json!(false)), Op::Set("logging.error".into(), json!("")), Op::Set("logging.available".into(), json!(false)), Op::Set("logging.confirmed".into(), Value::Null)]
}

fn answered(c: &Ctx, name: &str, doc: &Value) -> Vec<Op> {
    match name {
        "logging" => {
            let value = doc.get("value").filter(|v| v.is_object());
            let ok = c.docs.get("logging.status").as_u64() == Some(200) && doc.get("ok").and_then(Value::as_bool) == Some(true) && value.is_some();
            let mut ops = vec![Op::Set("logging.available".into(), json!(ok)), Op::RefreshIn(5000)];
            if let Some(value) = value.filter(|_| ok) {
                ops.push(Op::Set("logging.confirmed".into(), value.clone()));
            }
            ops
        }
        "logging.write" => {
            let ok = c.docs.get("logging.write.status").as_u64() == Some(200) && doc.get("ok").and_then(Value::as_bool) == Some(true);
            let error = if ok { "" } else { "Could not save. Try again." };
            vec![Op::Set("logging.busy".into(), json!(false)), Op::Set("logging.error".into(), json!(error)), Op::Refresh]
        }
        _ => Vec::new(),
    }
}

// DNS names or bare IP literals, with no scheme, port, whitespace or brackets.
fn valid_host(host: &str) -> bool {
    if host.parse::<IpAddr>().is_ok() {
        return true;
    }
    !host.is_empty() && host.len() <= 253 && host.split('.').all(|label| {
        (1..=63).contains(&label.len())
            && label.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
            && label.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
            && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

fn write(key: &str, kind: &str, value: Value) -> Vec<Op> {
    vec![
        Op::Set("logging.error".into(), json!("")),
        Op::Set("logging.busy".into(), json!(true)),
        Op::Send { name: Some("logging.write".into()), method: "POST", path: PATH.into(), body: Some(json!({"changes": [{"key": key, "value": {"type": kind, "value": value}}]})) },
    ]
}

fn invalid(message: &str) -> Vec<Op> {
    vec![Op::Set("logging.error".into(), json!(message))]
}

fn rows(c: &Ctx) -> Vec<Row> {
    let state = c.local.get("logging.confirmed");
    let available = c.local.flag("logging.available");
    let ready = available;
    let busy = c.local.flag("logging.busy");
    let enabled = state.get("enabled").and_then(Value::as_bool).unwrap_or(false);
    let host = state.pointer("/server/host").and_then(Value::as_str).unwrap_or("");
    let port = state.pointer("/server/port").and_then(Value::as_u64).unwrap_or(514);
    let detail = state.get("error").and_then(Value::as_str).unwrap_or("");
    let status = if !available { "Unavailable" } else if !enabled { "Disabled" } else if state.get("active").and_then(Value::as_bool) == Some(true) { "Forwarding (UDP)" } else { "Unavailable" };
    let error = c.local.text("logging.error");
    let host_for_toggle = host.to_string();
    let toggle = Row::toggle("enabled").icon("server").label("Remote logging").on(enabled).enabled(ready).admin().act(move |arg| match arg {
        _ if busy => Vec::new(),
        Arg::Toggle(true) if !valid_host(&host_for_toggle) => invalid("Enter a valid server address first."),
        Arg::Toggle(on) => write("enabled", "bool", json!(on)),
        _ => Vec::new(),
    });
    let shown_host = if host.is_empty() { "Not set".to_string() } else if host.chars().count() > 15 { format!("{}…", host.chars().take(15).collect::<String>()) } else { host.to_string() };
    let host_value = host.to_string();
    let address_entry = Op::Entry { purpose: crate::panels::ime::PURPOSE_URL, prompt: "Server address".into(), text: host_value, secret: false,
    then: Rc::new(move |text| {
        if (text.is_empty() && !enabled) || valid_host(&text) { write("host", "text", json!(text)) } else { invalid("Enter a valid server address.") }
    }), };
    let address = Row::link("host").icon("globe").label("Server address").value(&shown_host).enabled(ready).admin().act(move |_| if busy { Vec::new() } else { vec![address_entry.clone()] });
    let port_entry = Op::Entry { purpose: crate::panels::ime::PURPOSE_DIGITS, prompt: "Port".into(), text: port.to_string(), secret: false,
    then: Rc::new(|text| {
        if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
            if let Ok(port) = text.parse::<u16>() {
                if port != 0 { return write("port", "integer", json!(port)); }
            }
        }
        invalid("Enter a port from 1 to 65535.")
    }), };
    let port_row = Row::link("port").icon("network").label("Port").value(&port.to_string()).enabled(ready).admin().act(move |_| if busy { Vec::new() } else { vec![port_entry.clone()] });
    vec![
        Row::note(&error, 0).visible(!error.is_empty()),
        toggle,
        address,
        port_row,
        Row::info("status").label("Status").value(status).detail(detail).help("UDP forwarding does not confirm delivery."),
        Row::info("buffer").label("RAM buffer limit").value("1 MiB"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::World;

    #[test]
    fn input_validation_and_refused_writes_keep_confirmed_values() {
        assert!(valid_host("logs.example.org") && valid_host("192.0.2.1") && valid_host("2001:db8::1"));
        assert!(!valid_host("") && !valid_host("https://logs") && !valid_host("-bad.example") && !valid_host("host:514"));
        let mut w = World::new(crate::host::sio::Sio::new(0));
        w.local.set("logging.available", json!(true));
        w.local.set("logging.confirmed", json!({"enabled": false, "active": false, "server": {"host": "old.example", "port": 514}}));
        let r = rows(&w.ctx());
        let host = r.iter().find(|r| r.id == "host").unwrap();
        let [Op::Entry { then, .. }] = &(host.act.as_ref().unwrap())(Arg::Press)[..] else { panic!() };
        assert!(matches!(&then("bad host".into())[..], [Op::Set(_, _)]));
        assert!(matches!(&then("new.example".into())[..], [Op::Set(..), Op::Set(..), Op::Send { body: Some(body), .. }] if body["changes"][0]["value"]["value"] == "new.example"));
        w.docs.set("logging.write.status", json!(500));
        let ops = answered(&w.ctx(), "logging.write", &json!({"ok": false}));
        assert!(ops.iter().all(|op| !matches!(op, Op::Set(key, _) if key == "logging.confirmed")));
        assert_eq!(rows(&w.ctx()).iter().find(|r| r.id == "host").unwrap().value, "old.example");
    }

    #[test]
    fn editing_a_full_dns_name_retains_the_whole_value_and_source_warnings_do_not_hide_forwarding() {
        let host = ["a".repeat(63), "b".repeat(63), "c".repeat(63), "d".repeat(61)].join(".");
        let mut w = World::new(crate::host::sio::Sio::new(0));
        w.local.set("logging.available", json!(true));
        w.local.set("logging.confirmed", json!({"enabled": true, "active": true, "error": "One source unavailable", "server": {"host": host, "port": 514}}));
        let r = rows(&w.ctx());
        let address = r.iter().find(|r| r.id == "host").unwrap();
        let [Op::Entry { text, then, .. }] = &(address.act.as_ref().unwrap())(Arg::Press)[..] else { panic!() };
        assert_eq!(text, &host);
        assert!(matches!(&then(text.clone())[..], [Op::Set(..), Op::Set(..), Op::Send { body: Some(body), .. }] if body["changes"][0]["value"]["value"] == host));
        let status = r.iter().find(|r| r.id == "status").unwrap();
        assert_eq!(status.value, "Forwarding (UDP)");
        assert_eq!(status.detail, "One source unavailable");
    }
}
