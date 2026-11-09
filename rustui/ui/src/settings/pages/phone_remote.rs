//! Phone remote, a page of Remote & pointer: the helper's plugin that lets a paired phone drive the TV over the home network
//! (`helper/app/src/phone_remote`). Its switch asks in the plugin's own words before it turns it on (`POST
//! /v1/plugins/phone-remote/switch` with their code); off is one press, ends every phone's session and keeps the pairings. While it is on,
//! the page draws the `phone-remote` topic: the pairing code and its QR code while a window is open, then the paired phones, each with a
//! way to forget it. Pairing asks on the TV first: the helper opens the window after OK on the physical remote, and the code then shows
//! here.

use super::plugin_rows::text;
use crate::host::previews::age_text;
use crate::panels::hub;
use crate::settings::ctx::{Ctx, ask, toast};
use crate::settings::page::Page;
use crate::settings::row::{Arg, Op, Row};
use serde_json::{Value, json};

const ID: &str = "phone-remote";
/// The names the helper's answers come back under: the switch, and a pairing window or a phone changed.
const SWITCH: &str = "phoneremote.switch";
const CHANGE: &str = "phoneremote.change";

pub fn page() -> Page {
    Page::new("phoneremote", "Phone remote", "smartphone").parent("remote").back("Remote & pointer").rows(rows).summary(summary).answered(answered)
}

/// The plugin's entry in the `plugins` topic: none from a helper that is not there, or older than the plugin.
fn entry<'a>(c: &'a Ctx) -> Option<&'a Value> {
    hub::plugin_of(c.helper("plugins")?, ID)
}

fn on(c: &Ctx) -> bool {
    entry(c).is_some_and(|e| text(e, "state") != "off")
}

/// The plugin's own topic, while it is on.
fn state<'a>(c: &'a Ctx) -> Option<&'a Value> {
    c.helper(ID).filter(|_| on(c))
}

fn devices(s: &Value) -> &[Value] {
    s.get("devices").and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// The value of the row that opens the page.
pub fn summary(c: &Ctx) -> String {
    match state(c) {
        _ if entry(c).is_none() => "Not available".into(),
        None => "Off".into(),
        Some(s) if devices(s).iter().any(|d| d["live"] == true) => "Connected".into(),
        Some(_) => "On".into(),
    }
}

fn answered(_: &Ctx, name: &str, data: &Value) -> Vec<Op> {
    if !matches!(name, SWITCH | CHANGE) || data.get("ok").and_then(Value::as_bool) == Some(true) {
        return Vec::new();
    }
    let why = data.pointer("/error/message").and_then(Value::as_str).unwrap_or("the TV did not answer");
    vec![toast(&format!("Not changed: {why}"))]
}

fn switch(on: bool, code: &str) -> Op {
    let body = if on { json!({"on": true, "approve": code}) } else { json!({"on": false}) };
    Op::Send { name: Some(SWITCH.into()), method: "POST", path: format!("/v1/plugins/{ID}/switch"), body: Some(body) }
}

fn change(method: &'static str, path: &str) -> Op {
    Op::Send { name: Some(CHANGE.into()), method, path: format!("/v1/{ID}{path}"), body: None }
}

/// When a phone that is not connected was last heard from.
fn seen(c: &Ctx, d: &Value) -> String {
    match d.get("last").and_then(Value::as_u64).unwrap_or(0) {
        0 => "Not connected yet".into(),
        _ if c.now == 0 => String::new(),
        at => format!("Last used {}", age_text(c.now, at)),
    }
}

fn rows(c: &Ctx) -> Vec<Row> {
    let Some(e) = entry(c) else {
        return vec![Row::empty("Not available", "The TV's helper is not answering, or does not have the phone remote.", "smartphone", 0)];
    };
    let (consent, code) = (text(e, "consent").to_string(), text(e, "approve").to_string());
    let mut rows = vec![Row::toggle("switch").icon("smartphone").label("Phone remote").help(&consent).on(on(c)).detail(hub::failure(e).unwrap_or_default()).admin().act(move |a| match a {
        Arg::Toggle(true) => ask("smartphone", "Turn on the phone remote?", &consent, "Turn on", "Cancel", false, vec![switch(true, &code)]),
        Arg::Toggle(false) => vec![switch(false, "")],
        _ => Vec::new(),
    })];
    let Some(s) = state(c) else { return rows };
    if let Some(problem) = s.get("problem").and_then(Value::as_str).filter(|p| !p.is_empty()) {
        rows.push(Row::note(problem, 0));
    }
    match s.get("pairing").filter(|p| p.is_object()) {
        Some(p) => {
            let code = text(p, "code");
            rows.push(Row::code("code", text(p, "qr"), code, &format!("Scan the code with the phone, or enter {code} in its remote app. The code works once, for 5 minutes."), 0));
            rows.push(Row::action("closepairing", "Close").icon("x").label("Waiting for the phone").admin().does(vec![change("DELETE", "/pairing")]));
        }
        None => rows.push(Row::action("pair", "Pair").icon("plus").label("Pair a phone").help("The TV asks first: press OK on the remote, and a code for the phone shows here.").admin().does(vec![change("POST", "/pairing")])),
    }
    let list = devices(s);
    rows.push(Row::head("Paired phones", 0));
    if list.is_empty() {
        rows.push(Row::note("No phone is paired yet.", 0));
    }
    for d in list {
        let (id, name) = (text(d, "id"), text(d, "name"));
        let mut r = Row::action(&format!("phone:{id}"), "Forget").icon("smartphone").label(name).admin();
        r.detail = if d["live"] == true { "Connected".into() } else { seen(c, d) };
        let forget = change("DELETE", &format!("/devices/{id}"));
        rows.push(r.does(ask("trash-2", &format!("Forget {name}?"), "It stops controlling the TV at once. To come back, it pairs again with a new code.", "Forget", "Keep", true, vec![forget])));
    }
    if list.len() > 1 {
        let all = ask("ban", "Forget every phone?", "Every phone stops controlling the TV at once and must pair again.", "Forget all", "Keep", true, vec![change("DELETE", "/devices")]);
        rows.push(Row::action("forgetall", "Forget all").icon("ban").label("Forget every phone").danger().admin().does(all));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::World;
    use crate::settings::row::Kind;

    fn world(state: &str, topic: Value) -> World {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        w.topics.helper_up = true;
        w.topics.set_helper("plugins", json!({"plugins": [], "firstParty": [{"id": ID, "state": state, "consent": "Lets paired phones control this TV.", "approve": "c0de"}]}));
        w.topics.set_helper(ID, topic);
        w.now = 10_000_000;
        w
    }

    fn row<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id}"))
    }

    /// What a row does once its question is answered yes.
    fn confirmed(r: &Row, a: Arg) -> Vec<Op> {
        match &(r.act.as_ref().expect("a row with an action"))(a)[..] {
            [Op::Ask(q)] => q.answers.iter().find(|x| !x.then.is_empty()).map(|x| x.then.clone()).unwrap_or_default(),
            ops => ops.to_vec(),
        }
    }

    fn sends(ops: &[Op]) -> Vec<(&'static str, String, Option<Value>)> {
        ops.iter().filter_map(|o| if let Op::Send { method, path, body, .. } = o { Some((*method, path.clone(), body.clone())) } else { None }).collect()
    }

    #[test]
    fn off_it_is_one_switch_that_asks_in_the_plugins_words_and_on_it_shows_the_code_and_the_phones() {
        let off = world("off", json!({"on": false, "pairing": null, "devices": [{"id": "d1", "name": "Old phone", "last": 0, "live": false}]}));
        let (c, r) = (off.ctx(), rows(&off.ctx()));
        assert_eq!((r.len(), summary(&c).as_str()), (1, "Off"), "nothing of the phones while it is off");
        assert_eq!(sends(&confirmed(row(&r, "switch"), Arg::Toggle(true))), [("POST", "/v1/plugins/phone-remote/switch".to_string(), Some(json!({"on": true, "approve": "c0de"})))]);

        let devices = json!([{"id": "d1", "name": "Nitin's iPhone", "last": 9_700_000, "live": false}, {"id": "d2", "name": "iPad", "last": 9_990_000, "live": true}]);
        let on = world("running", json!({"on": true, "problem": null, "pairing": {"code": "ABCD-EFGH", "qr": "lumaremote://pair?c=ABCDEFGH", "expires": 10_300_000}, "devices": devices}));
        let (c, r) = (on.ctx(), rows(&on.ctx()));
        assert_eq!(summary(&c), "Connected");
        let code = row(&r, "code");
        assert_eq!((code.kind, code.text.as_str(), code.label.as_str()), (Kind::Code, "lumaremote://pair?c=ABCDEFGH", "ABCD-EFGH"));
        assert_eq!(sends(&confirmed(row(&r, "closepairing"), Arg::Press)), [("DELETE", "/v1/phone-remote/pairing".to_string(), None)]);
        assert_eq!((row(&r, "phone:d1").detail.as_str(), row(&r, "phone:d2").detail.as_str()), ("Last used 5 min ago", "Connected"));
        assert_eq!(sends(&confirmed(row(&r, "phone:d1"), Arg::Press)), [("DELETE", "/v1/phone-remote/devices/d1".to_string(), None)]);
        assert_eq!(sends(&confirmed(row(&r, "forgetall"), Arg::Press)), [("DELETE", "/v1/phone-remote/devices".to_string(), None)]);

        let closed = world("running", json!({"on": true, "pairing": null, "devices": []}));
        let r = rows(&closed.ctx());
        assert_eq!(sends(&confirmed(row(&r, "pair"), Arg::Press)), [("POST", "/v1/phone-remote/pairing".to_string(), None)]);
        assert!(r.iter().all(|r| r.id != "forgetall" && !r.id.starts_with("phone:")));
    }
}
