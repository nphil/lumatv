//! Plex, a page of Inputs & devices: the TV's Plex account (`helper/app/src/plex.rs`), the one sign-in everything of Plex on the TV uses
//! (Guide data's Plex listings). Signing in is a code, never typing: the page asks the helper for one (`POST
//! /v1/plex/signin`) and shows it with a QR code of Plex's link page for it; a phone confirms it, the helper, asking after it, signs the TV
//! in, and the page follows the `plex.account` topic. Signed in, it says who, the server the TV uses (a choice when the account has several
//! on the home network) and what uses it; Sign out ends the TV's sign-in at plex.tv. No token ever reaches the page.

use super::util::go;
use crate::settings::ctx::{Ctx, ask, toast};
use crate::settings::page::Page;
use crate::settings::row::{ChoiceSpec, Op, Pick, Row};
use serde_json::{Value, json};
use std::rc::Rc;

/// The helper's topic of the account.
const TOPIC: &str = "plex.account";

pub fn page() -> Page {
    Page::new("plex", "Plex", "tv-minimal-play")
        .parent("inputs")
        .back("Inputs & devices")
        .rows(rows)
        .summary(summary)
        .chip(chip)
        .opened(opened)
        .answered(answered)
}

fn account<'a>(c: &'a Ctx) -> &'a Value {
    c.helper(TOPIC).unwrap_or(&Value::Null)
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn send(name: &str, path: &str, body: Value) -> Op {
    Op::Send { name: Some(name.into()), method: "POST", path: path.into(), body: Some(body) }
}

/// The name and the address of a server of the account.
fn server_words(s: &Value) -> String {
    format!("{} · {}", text(s, "name"), text(s, "address"))
}

/// The value of the row that opens the page.
pub fn summary(c: &Ctx) -> String {
    let a = account(c);
    match text(a, "state") {
        "signed-in" => format!("Signed in as {}", text(a, "user")),
        "code" => "Signing in".into(),
        "" => "Not available".into(),
        _ => "Not signed in".into(),
    }
}

fn chip(c: &Ctx) -> Option<(String, &'static str)> {
    let a = account(c);
    let tone = match text(a, "state") {
        _ if a.get("problem").is_some() => "warning",
        "signed-in" if a["server"].is_object() => "positive",
        "signed-in" => "warning",
        _ => "info",
    };
    Some((summary(c), tone))
}

/// Signed in, the account's name and servers are read again (the helper does so at most once a minute).
fn opened(c: &Ctx) -> Vec<Op> {
    if text(account(c), "state") == "signed-in" { vec![send("plex.refresh", "/v1/plex/refresh", json!({}))] } else { Vec::new() }
}

fn answered(_: &Ctx, name: &str, data: &Value) -> Vec<Op> {
    if !name.starts_with("plex.") || data.get("ok").and_then(Value::as_bool) == Some(true) {
        return Vec::new();
    }
    let why = data.pointer("/error/message").and_then(Value::as_str).unwrap_or("the TV did not answer");
    vec![toast(&format!("Not done: {why}"))]
}

fn rows(c: &Ctx) -> Vec<Row> {
    let a = account(c);
    let mut rows = Vec::new();
    match text(a, "state") {
        "" => {
            return vec![Row::empty("Not available", "The TV's helper is not answering, or is older than the Plex sign-in.", "tv-minimal-play", 0)];
        }
        "code" => {
            let code = text(a, "code");
            rows.push(Row::code(
                "code",
                text(a, "url"),
                code,
                &format!("Scan the code with a phone, or open plex.tv/link and enter {code}. Then allow LumaTV. The code works for 15 minutes."),
                0,
            ));
            rows.push(Row::action("cancel", "Cancel").icon("x").label("Waiting for the phone").does(vec![send("plex.cancel", "/v1/plex/cancel", json!({}))]));
        }
        "signed-in" => {
            rows.push(Row::info("user").icon("user").label(&format!("Signed in as {}", text(a, "user"))));
            rows.push(server_row(a));
        }
        _ => {
            rows.push(
                Row::action("signin", "Sign in")
                    .icon("smartphone")
                    .label("Sign in to Plex")
                    .help("Shows a code to scan with a phone. Nothing is typed on the TV.")
                    .admin()
                    .does(vec![send("plex.signin", "/v1/plex/signin", json!({}))]),
            );
        }
    }
    if let Some(problem) = a.get("problem").and_then(Value::as_str) {
        rows.push(Row::note(problem, 0));
    }
    rows.push(Row::head("Used by", 0));
    rows.push(Row::link("guidedata").icon("tv-minimal").label("Guide data").help("Its Plex listings come from this sign-in's server.").does(go("guidedata")));
    if text(a, "state") == "signed-in" {
        rows.push(Row::action("signout", "Sign out").icon("log-out").label("Sign out of Plex").danger().admin().does(ask(
            "log-out",
            "Sign out of Plex?",
            "The TV's sign-in is ended at plex.tv and deleted here. Guide data's Plex listings stop until the TV signs in again.",
            "Sign out",
            "Cancel",
            true,
            vec![send("plex.signout", "/v1/plex/signout", json!({}))],
        )));
    }
    rows
}

/// The server in use; with several on the home network, a choice of them.
fn server_row(a: &Value) -> Row {
    let servers: Vec<&Value> = a["servers"].as_array().into_iter().flatten().collect();
    let current = text(&a["server"], "id").to_string();
    let base = Row::link("server").icon("server").label("Server");
    match (a["server"].is_object(), servers.len()) {
        (false, _) => base.value("None on the home network").chevron(false),
        (true, 0 | 1) => base.value(&server_words(&a["server"])).chevron(false),
        (true, _) => {
            let options = servers.iter().map(|s| Pick::new(text(s, "id"), &server_words(s))).collect();
            base.value(&server_words(&a["server"])).admin().does(vec![Op::Pick(Rc::new(ChoiceSpec {
                title: "Plex server".into(),
                options,
                current: current.clone(),
                write: Box::new(move |id| if id == current { Vec::new() } else { vec![send("plex.server", "/v1/plex/server", json!({"id": id}))] }),
            }))])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::World;
    use crate::settings::row::{Arg, Kind};

    fn world(account: Value) -> World {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        w.topics.helper_up = true;
        w.topics.set_helper(TOPIC, account);
        w
    }

    fn row<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id} in {rows:?}"))
    }

    fn sends(r: &Row) -> Vec<String> {
        (r.act.as_ref().unwrap())(Arg::Press).iter().filter_map(|o| if let Op::Send { path, .. } = o { Some(path.clone()) } else { None }).collect()
    }

    #[test]
    fn signed_out_one_press_asks_for_a_code_and_the_code_is_shown_with_its_qr_and_the_link_page() {
        let w = world(json!({"state": "signed-out"}));
        let r = rows(&w.ctx());
        assert_eq!(sends(row(&r, "signin")), ["/v1/plex/signin"]);
        assert_eq!(summary(&w.ctx()), "Not signed in");

        let w = world(json!({"state": "code", "code": "WBVQ", "url": "https://plex.tv/link/?pin=WBVQ", "expiresAt": 1}));
        let r = rows(&w.ctx());
        let code = row(&r, "code");
        assert_eq!((code.kind, code.text.as_str(), code.label.as_str()), (Kind::Code, "https://plex.tv/link/?pin=WBVQ", "WBVQ"));
        assert!(code.detail.contains("plex.tv/link") && !code.focusable());
        assert_eq!(sends(row(&r, "cancel")), ["/v1/plex/cancel"]);
    }

    #[test]
    fn signed_in_it_says_who_offers_the_other_servers_and_signing_out_asks_first() {
        let beast = json!({"id": "beast", "name": "BeastNAS", "address": "192.168.1.69:32400"});
        let den = json!({"id": "den", "name": "Den", "address": "192.168.1.80:32400"});
        let w = world(json!({"state": "signed-in", "user": "nitin", "server": beast, "servers": [beast, den]}));
        let c = w.ctx();
        let r = rows(&c);
        assert_eq!(row(&r, "user").label, "Signed in as nitin");
        assert_eq!(chip(&c), Some(("Signed in as nitin".into(), "positive")));
        assert_eq!(opened(&c).len(), 1, "the account is read again");
        let server = row(&r, "server");
        assert_eq!(server.value, "BeastNAS · 192.168.1.69:32400");
        let [Op::Pick(pick)] = &(server.act.as_ref().unwrap())(Arg::Press)[..] else { panic!("a choice") };
        assert!(matches!(&(pick.write)("den")[..], [Op::Send { path, body: Some(b), .. }] if path == "/v1/plex/server" && b == &json!({"id": "den"})));
        assert!((pick.write)("beast").is_empty());
        let [Op::Ask(question)] = &(row(&r, "signout").act.as_ref().unwrap())(Arg::Press)[..] else { panic!("asks first") };
        assert!(matches!(&question.answers[1].then[..], [Op::Send { path, .. }] if path == "/v1/plex/signout"));
    }
}
