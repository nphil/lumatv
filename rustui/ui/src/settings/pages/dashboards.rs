//! The signed-in profile's dashboard library. The name list is a view, not a
//! switch: switching takes a separate, explicit action.

use crate::home::dashboard;
use crate::settings::ctx::{Ctx, ask};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Op, PageArg, Row, ShellRequest};
use serde_json::{Value, json};
use std::rc::Rc;

const READ: &str = "dashboards.read";
const WRITE: &str = "dashboards.write";
const BOOK: &str = "dashboards.book";
const BUSY: &str = "dashboards.busy";
const WHO: &str = "dashboards.who";
const PROBLEM: &str = "dashboards.problem";

pub fn page() -> Page {
    Page::new("dashboards", "Dashboards", "layout-grid").parent("profiles").back("Profiles")
        .rows(rows).reads(reads).opened(opened).answered(answered)
}

pub fn detail() -> Page {
    Page::new("dashboard", "Dashboard", "layout-grid").parent("profiles").back("Dashboards").hidden_from_search()
        .rows(detail_rows).title_of(detail_title).reads(reads).opened(opened).answered(answered)
}

fn signed_in<'a>(c: &'a Ctx) -> Option<(&'a str, Option<u64>)> {
    c.helper("profiles").and_then(dashboard::identity)
}

fn book_identity(book: &Value) -> Option<(&str, Option<u64>)> {
    Some((book.get("profile")?.as_str()?, book.get("profileCreated").and_then(Value::as_u64)))
}

fn book<'a>(c: &'a Ctx) -> Option<&'a Value> {
    let book = c.local.get(BOOK);
    book_identity(book).filter(|id| Some(*id) == signed_in(c)).map(|_| book)
}

fn reads(c: &Ctx) -> Vec<Read> {
    if signed_in(c).is_some() { vec![Read::Get { name: READ, path: dashboard::PATH }] } else { Vec::new() }
}

fn opened(c: &Ctx) -> Vec<Op> {
    vec![Op::Set(WHO.into(), json!(signed_in(c))), Op::Set(BUSY.into(), json!(false)), Op::Set(PROBLEM.into(), Value::Null)]
}

fn answered(c: &Ctx, call: &str, reply: &Value) -> Vec<Op> {
    let profile = signed_in(c);
    if call == "topic:profiles" {
        if c.local.get(WHO) == &json!(profile) { return Vec::new(); }
        let mut ops = opened(c);
        ops.push(Op::Set(BOOK.into(), Value::Null));
        if c.page_id == "dashboard" {
            ops.push(Op::Shell(ShellRequest::CancelEntry));
            ops.push(Op::Up);
        } else {
            ops.push(Op::Refresh);
        }
        return ops;
    }
    if !matches!(call, READ | WRITE) { return Vec::new(); }
    let Some((profile, created)) = profile else { return Vec::new(); };
    let status = c.docs.get(&format!("{call}.status")).as_u64().unwrap_or_default() as u16;
    let mut ops = Vec::new();
    if call == WRITE {
        ops.push(Op::Set(BUSY.into(), json!(false)));
    }
    if let Some(doc) = dashboard::document(reply, profile, created).filter(|_| status == 200) {
        let names: Vec<_> = dashboard::list(doc).iter().map(|d| json!({"id": dashboard::text(d, "id"), "name": dashboard::text(d, "name")})).collect();
        ops.push(Op::Set(BOOK.into(), json!({"profile": profile, "profileCreated": created, "revision": doc["revision"], "active": doc["active"], "dashboards": names})));
        if call == WRITE {
            ops.push(Op::Shell(ShellRequest::ReloadHome));
            if c.page_id == "dashboard" && c.arg.str("id").is_some_and(|id| dashboard::find(doc, id).is_none()) {
                ops.push(Op::Up);
            }
        }
    } else if status != 200 {
        let problem = if call == READ { "Dashboards aren’t available right now." } else { dashboard::error(status, reply) };
        ops.push(Op::Set(PROBLEM.into(), json!(problem)));
        if call == READ {
            ops.push(Op::Set(BOOK.into(), Value::Null));
        } else if matches!(reply.pointer("/error/code").and_then(Value::as_str), Some("profile_changed" | "stale_revision")) {
            ops.push(Op::Set(BOOK.into(), Value::Null));
            ops.push(Op::Refresh);
        }
    }
    ops
}

fn problem(c: &Ctx) -> Option<Row> {
    c.local.get(PROBLEM).as_str().map(|message| Row::empty(message, "", "info", 0))
}

fn absent(c: &Ctx) -> Vec<Row> {
    if signed_in(c).is_none() {
        return vec![Row::empty("No signed-in profile", "", "user", 0)];
    }
    vec![problem(c).unwrap_or_else(|| Row::empty("Reading dashboards…", "", "layout-grid", 0)),
        Row::action("retry", "Reload").label("Reload dashboards").does(vec![Op::Set(PROBLEM.into(), Value::Null), Op::Refresh])]
}

fn rows(c: &Ctx) -> Vec<Row> {
    let Some((profile, created)) = signed_in(c) else { return absent(c) };
    let Some(book) = book(c) else { return absent(c) };
    let active = dashboard::text(book, "active");
    let busy = c.local.get(BUSY).as_bool() == Some(true);
    let mut rows: Vec<Row> = problem(c).into_iter().collect();
    for d in dashboard::list(book) {
        let (id, name) = (dashboard::text(d, "id"), dashboard::text(d, "name"));
        let in_use = id == active;
        rows.push(Row::link(&format!("d:{id}")).icon("layout-grid").label(name).value(if in_use { "In use" } else { "" }).selected(in_use).enabled(!busy)
            .does(vec![Op::Open { page: "dashboard".into(), arg: PageArg::Json(json!({"profile": profile, "profileCreated": created, "id": id})) }]));
    }
    rows.push(Row::head("Make it yours", 0));
    for (id, label, icon, replay) in [("new", "Create a dashboard", "plus", false), ("guide", "Run dashboard guide again", "play", true)] {
        rows.push(Row::link(id).icon(icon).label(label).enabled(!busy)
            .does(vec![Op::Shell(ShellRequest::DashboardDraft { profile: profile.into(), created, replay })]));
    }
    rows
}

fn selected<'a>(c: &'a Ctx, book: &'a Value) -> Option<&'a Value> {
    let PageArg::Json(arg) = c.arg else { return None };
    if book_identity(arg) != signed_in(c) { return None; }
    dashboard::find(book, c.arg.str("id")?)
}

fn detail_title(c: &Ctx) -> String {
    book(c).and_then(|b| selected(c, b)).map_or_else(|| "Dashboard".into(), |d| dashboard::text(d, "name").into())
}

fn send(body: Value) -> Vec<Op> {
    vec![Op::Set(BUSY.into(), json!(true)), Op::Set(PROBLEM.into(), Value::Null),
        Op::Send { name: Some(WRITE.into()), method: "POST", path: dashboard::PATH.into(), body: Some(body) }]
}

fn detail_rows(c: &Ctx) -> Vec<Row> {
    let Some(book) = book(c) else { return absent(c) };
    let Some(d) = selected(c, book) else { return vec![Row::empty("Dashboard not available", "", "layout-grid", 0)] };
    let id = dashboard::text(d, "id");
    let in_use = dashboard::text(book, "active") == id;
    let guard = dashboard::Guard::of(book);
    let busy = c.local.get(BUSY).as_bool() == Some(true);
    let mut rows: Vec<Row> = problem(c).into_iter().collect();
    if in_use {
        rows.push(Row::link("edit").icon("move").label("Edit on Home").enabled(!busy).does(vec![Op::Shell(ShellRequest::EditHome)]));
    } else {
        rows.push(Row::action("use", "Use").icon("check").label("Use this dashboard").enabled(!busy).does(send(guard.body("activate", id))));
    }
    let rename = guard.body("save", id);
    rows.push(Row::link("rename").icon("type").label("Rename").enabled(!busy).does(vec![Op::Entry {
        prompt: "Dashboard name".into(), text: dashboard::text(d, "name").into(), secret: false, purpose: crate::panels::ime::PURPOSE_NAME,
        then: Rc::new(move |name| {
            let name = name.trim();
            if !dashboard::name_ok(name) {
                return vec![Op::Set(PROBLEM.into(), json!("Enter a name with 1–32 characters."))];
            }
            let mut body = rename.clone();
            body["name"] = json!(name);
            send(body)
        }),
    }]));
    if !in_use {
        rows.push(Row::action("remove", "Remove").icon("trash-2").label("Remove this dashboard").enabled(!busy).danger()
            .does(ask("trash-2", "Remove this dashboard?", "Only this dashboard will be removed.", "Remove", "Keep it", true, send(guard.body("delete", id)))));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::World;
    use crate::settings::row::Arg;

    #[test]
    fn dashboard_switch_is_explicit_and_bound_to_the_signed_in_profile() {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        w.topics.set_helper("profiles", json!({"ready": true, "active": "ana"}));
        w.local.set(BOOK, json!({"profile":"ana","revision":"r1","active":"home","dashboards":[{"id":"home","name":"Home"},{"id":"news","name":"News"}]}));
        let list = rows(&w.ctx());
        let news = list.iter().find(|r| r.label == "News").unwrap();
        let ops = (news.act.as_ref().unwrap())(Arg::Press);
        let [Op::Open { page, arg }] = &ops[..] else { panic!("the list must not activate") };
        assert_eq!(page, "dashboard");
        w.arg = arg.clone();
        let r = detail_rows(&w.ctx());
        let use_it = r.iter().find(|r| r.id == "use").unwrap();
        let ops = (use_it.act.as_ref().unwrap())(Arg::Press);
        assert!(ops.iter().any(|op| matches!(op, Op::Send { body: Some(b), .. } if b == &json!({"action":"activate","profile":"ana","revision":"r1","id":"news"}))));
        w.topics.set_helper("profiles", json!({"ready": true, "active": "ana", "profiles": [{"id":"ana","profileCreated":24}]}));
        assert!(!rows(&w.ctx()).iter().any(|r| r.label == "News"), "an old book is private even if the new profile reuses the ID");
        w.topics.set_helper("profiles", json!({"ready": true, "active": "owner"}));
        assert!(!rows(&w.ctx()).iter().any(|r| r.label == "News"));
        assert!(!detail_rows(&w.ctx()).iter().any(|r| r.id == "use" || r.id == "remove" || r.id == "rename"));
    }
}
