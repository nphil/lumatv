//! AI access (`settings/PageAi.qml`, `docs/mocks/04-settings-ai-access`): what the helper's AI module says (the `ai` topic) and the changes
//! the owner makes through `/v1/ai`.
//!
//! Principle 0 shapes this page more tightly than the QML one. Whatever gives an assistant less is done at once: access off, pause, remove,
//! a lower level, a lower volume limit, lock-out. Whatever gives more (access on, a pairing code, a higher level, resuming, a higher limit)
//! is NOT offered here: the helper answers such a request with a prompt that the physical remote approves, and a key on the remote is not
//! proof of a person (a network key can be made to look like one; only the device name tells them apart). The approval that is proof, Face ID
//! on the phone, has no route to this screen yet, so those controls open a plain "not available here" answer instead of asking the helper.
//! Nothing on this page answers a prompt, and there is no route that does. See `docs/research/rust-settings-result.md` for the exposure.

use crate::settings::ctx::{Ctx, ask};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Answer, Ask, Arg, Op, Row};
use serde_json::{Value, json};

const LEVELS: [&str; 3] = ["always", "ask", "never"];
const LEVEL_NAMES: [&str; 3] = ["Always", "Ask", "Never"];
const VOLUME_CEILING: i64 = 98;

pub fn page() -> Page {
    Page::new("ai", "AI access", "bot").rail().tabs(&["Access", "Assistants", "Allowed"]).reads(reads).rows(rows).summary(summary).answered(answered)
}

fn reads(_: &Ctx) -> Vec<Read> {
    vec![Read::Get { name: "aiaudit", path: "/v1/ai/audit?limit=5" }]
}

/// The `ai` topic, while the helper answers.
fn state<'a>(c: &'a Ctx) -> Option<&'a Value> {
    c.topics.helper_up.then(|| c.helper("ai")).flatten()
}

fn summary(c: &Ctx) -> String {
    match state(c) {
        None => "Not running".into(),
        Some(s) if listener_on(s) => "Allowed".into(),
        Some(_) => "Blocked".into(),
    }
}

fn answered(_: &Ctx, name: &str, _: &Value) -> Vec<Op> {
    match name {
        // the limit shown while the change waited is now the helper's own
        "ailimit" => vec![Op::Set("ailimit".into(), Value::Null)],
        _ => Vec::new(),
    }
}

fn listener_on(s: &Value) -> bool {
    s.pointer("/listener/enabled").and_then(Value::as_bool) == Some(true)
}

fn assistants(s: &Value) -> Vec<&Value> {
    s.get("assistants").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default()
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// The answer a control gives when it would give an AI more: no request goes to the helper.
fn needs_phone(what: &str) -> Vec<Op> {
    vec![Op::Ask(Ask {
        icon: "smartphone".into(),
        title: "Not available here".into(),
        body: format!("{what} gives AI more access. That needs Face ID approval on the phone, and the phone's approval is not connected to this screen yet."),
        answers: vec![Answer { id: "ok".into(), label: "Got it".into(), danger: false, then: Vec::new() }],
        focus: 0,
    })]
}

fn send(method: &'static str, path: &str, body: Option<Value>) -> Op {
    Op::Send { name: None, method, path: path.into(), body }
}

fn rank(level: &str) -> usize {
    LEVELS.iter().position(|l| *l == level).map_or(1, |i| 2 - i)
}

/// What an assistants' group is called, a line on what it lets an assistant do, and its glyph.
fn group_words(id: &str, title: &str) -> (String, &'static str, &'static str) {
    match id {
        "state" => ("What's playing".into(), "Title, app, input, volume, on or off", "info"),
        "playback" => ("Playback".into(), "Including mute", "play"),
        "launch" => ("Apps and inputs".into(), "Only apps already installed", "layout-grid"),
        "power" => ("Power".into(), "Never turn it on. The receiver stays as it is.", "power"),
        _ => (title.to_string(), "", "bot"),
    }
}

fn ago(c: &Ctx, at: u64) -> String {
    if at == 0 || c.now == 0 {
        return String::new();
    }
    let s = c.now.saturating_sub(at) / 1000;
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", (s + 30) / 60),
        3600..=86399 => format!("{} h ago", (s + 1800) / 3600),
        _ => "yesterday".into(),
    }
}

/// An assistant's state line: paused, else when it was last heard from.
fn last_for(c: &Ctx, a: &Value) -> String {
    if a.get("paused").and_then(Value::as_bool) == Some(true) {
        return "Paused".into();
    }
    let said = |e: &Value| e.pointer("/assistant/id").and_then(Value::as_str) == a.get("id").and_then(Value::as_str);
    let at = c.docs.field("aiaudit", "entries").as_array().and_then(|l| l.iter().rev().find(|e| said(e)).and_then(|e| e.get("at")).and_then(Value::as_u64)).or_else(|| a.get("lastSeenAt").and_then(Value::as_u64)).unwrap_or(0);
    let said = ago(c, at);
    if said.is_empty() { "Active".into() } else { said }
}

fn outcome(o: &str) -> &str {
    match o {
        "ok" => "Allowed",
        "denied" => "Denied",
        "expired" => "Expired",
        "never" => "Blocked",
        "paused" => "Paused",
        other => other,
    }
}

fn not_running(tab: usize) -> Row {
    Row::empty("The helper is not running", "Try again in a moment.", "bot", tab)
}

fn rows(c: &Ctx) -> Vec<Row> {
    let Some(s) = state(c) else { return (0..3).map(not_running).collect() };
    let mut rows = access(c, s);
    rows.extend(assistant_rows(c, s));
    rows.extend(allowed(c, s));
    rows
}

// ------------------------------------------------------------------------------------------------------------------------- Access

fn access(c: &Ctx, s: &Value) -> Vec<Row> {
    let on = listener_on(s);
    let mut rows = vec![
        Row::seg("listener", &["Blocked", "Home network only"])
            .tab(0)
            .icon("bot")
            .label("Allow AI assistants")
            .help("Off by default. It reaches the TV from the home network only. AI can never change these settings. Turning it on needs the phone's approval, which is not connected here yet.")
            .index(Some(usize::from(on)))
            .admin()
            .act(move |a| match a {
                Arg::Index(0) if on => vec![send("POST", "/v1/ai/listener", Some(json!({"enabled": false})))],
                Arg::Index(1) if !on => needs_phone("Turning access on"),
                _ => Vec::new(),
            }),
    ];
    if let Some(e) = s.pointer("/listener/error").and_then(Value::as_str).filter(|e| !e.is_empty() && on) {
        rows.push(Row::note(e, 0));
    }

    let now = s.pointer("/limits/maxVolume").and_then(Value::as_i64).unwrap_or(VOLUME_CEILING);
    let shown = c.local.get("ailimit").as_i64().unwrap_or(now);
    rows.push(
        Row::slider("limit", 0.0, VOLUME_CEILING as f64, 2.0)
            .tab(0)
            .icon("volume-2")
            .label("Volume limit")
            .help("The loudest an assistant may set the receiver. Lower is done at once; higher needs the phone's approval, which is not connected here yet.")
            .num(shown as f64)
            .value_text(&shown.to_string())
            .admin()
            .act(move |a| match a {
                Arg::Value(v) if v.round() as i64 > now => needs_phone("A higher volume limit"),
                Arg::Value(v) => {
                    let v = v.round() as i64;
                    vec![Op::Set("ailimit".into(), json!(v)), Op::Later { key: "ailimit".into(), ms: 400, then: vec![Op::Send { name: Some("ailimit".into()), method: "POST", path: "/v1/ai/limits".into(), body: Some(json!({"maxVolume": v})) }] }]
                }
                _ => Vec::new(),
            }),
    );

    match s.get("pairing").filter(|p| !p.is_null()) {
        Some(p) => {
            let mut r = Row::action("pair", "Close").icon("plus").label(&format!("Pairing code {}", text(p, "code"))).admin().does(vec![send("DELETE", "/v1/ai/pairing", None)]);
            let left = p.get("attemptsLeft").and_then(Value::as_i64).unwrap_or(0);
            r.detail = format!("{left} {} left", if left == 1 { "try" } else { "tries" });
            rows.push(r);
        }
        None => {
            let mut r = Row::link("pair").tab(0).icon("plus").label("Pair a new assistant").help("A code shows after the phone's approval, which is not connected here yet. Give it to the assistant once.").chevron(false).admin().does(needs_phone("A pairing code"));
            r.detail = if on { "Needs the phone".into() } else { "Turn access on first".into() };
            r.enabled = on;
            rows.push(r);
        }
    }

    rows.push(
        Row::action("lock", "Lock out")
            .tab(0)
            .icon("ban")
            .label("Lock out all AI")
            .help("Removes every assistant and turns access off, at once.")
            .danger()
            .admin()
            .does(ask("ban", "Lock out all AI?", "Every assistant is removed and access is turned off.", "Lock out", "Cancel", true, vec![send("POST", "/v1/ai/lockout", Some(json!({})))])),
    );
    rows
}

// ------------------------------------------------------------------------------------------------------------------------ Assistants

fn assistant_rows(c: &Ctx, s: &Value) -> Vec<Row> {
    let list = assistants(s);
    let mut rows = Vec::new();
    if list.is_empty() {
        rows.push(Row::empty("No assistants paired", "Pairing needs the phone's approval, which is not connected here yet.", "bot", 1));
    }
    for a in &list {
        let (id, name) = (text(a, "id").to_string(), text(a, "name").to_string());
        let paused = a.get("paused").and_then(Value::as_bool) == Some(true);
        let mut r = Row::seg(&format!("assistant:{id}"), &[if paused { "Resume" } else { "Pause" }, "Remove"]).tab(1).icon("bot").label(&name).admin();
        r.detail = last_for(c, a);
        rows.push(r.act(move |arg| match arg {
            Arg::Index(0) if paused => needs_phone("Resuming an assistant"),
            Arg::Index(0) => vec![send("POST", &format!("/v1/ai/assistants/{id}/pause"), Some(json!({})))],
            Arg::Index(1) => ask("trash-2", &format!("Remove {name}?"), "It is unpaired at once and must be paired again to come back.", "Remove", "Keep", true, vec![send("DELETE", &format!("/v1/ai/assistants/{id}"), None)]),
            _ => Vec::new(),
        }));
    }
    let entries: Vec<&Value> = c.docs.field("aiaudit", "entries").as_array().map(|l| l.iter().rev().collect()).unwrap_or_default();
    if !entries.is_empty() {
        rows.push(Row::head("Recent activity", 1));
    }
    for (i, e) in entries.iter().enumerate() {
        let who = e.pointer("/assistant/name").and_then(Value::as_str).map(|n| format!("{n} \u{b7} ")).unwrap_or_default();
        let what = e.get("message").and_then(Value::as_str).filter(|m| !m.is_empty()).or_else(|| e.get("tool").and_then(Value::as_str)).unwrap_or("");
        let mut r = Row::info(&format!("audit:{i}")).tab(1).label(&format!("{who}{what}")).value(outcome(text(e, "outcome")));
        r.detail = ago(c, e.get("at").and_then(Value::as_u64).unwrap_or(0));
        rows.push(r);
    }
    rows
}

// --------------------------------------------------------------------------------------------------------------------------- Allowed

fn allowed(c: &Ctx, s: &Value) -> Vec<Row> {
    let list = assistants(s);
    if list.is_empty() {
        return vec![Row::empty("No assistants paired", "What an assistant may do is set once it is paired.", "bot", 2)];
    }
    let chosen = c.local.text("aiwho");
    let at = list.iter().position(|a| text(a, "id") == chosen).unwrap_or(0);
    let who = list[at];
    let mut rows = Vec::new();
    if list.len() > 1 {
        let names: Vec<&str> = list.iter().map(|a| text(a, "name")).collect();
        let ids: Vec<String> = list.iter().map(|a| text(a, "id").to_string()).collect();
        rows.push(Row::seg("who", &names).tab(2).icon("bot").label("Assistant").index(Some(at)).act(move |a| match a {
            Arg::Index(i) => ids.get(i).map(|id| vec![Op::Set("aiwho".into(), json!(id))]).unwrap_or_default(),
            _ => Vec::new(),
        }));
    }
    let levels = who.get("levels").cloned().unwrap_or_else(|| json!({}));
    for g in s.get("groups").and_then(Value::as_array).into_iter().flatten() {
        let gid = text(g, "id").to_string();
        let (label, help, glyph) = group_words(&gid, text(g, "title"));
        let now = levels.get(&gid).and_then(Value::as_str).unwrap_or("ask").to_string();
        let (assistant, levels) = (text(who, "id").to_string(), levels.clone());
        let r = Row::seg(&format!("level:{gid}"), &LEVEL_NAMES).tab(2).icon(glyph).label(&label).help(help).index(LEVELS.iter().position(|l| *l == now)).admin();
        rows.push(r.act(move |a| match a {
            Arg::Index(i) if i < LEVELS.len() && rank(LEVELS[i]) > rank(&now) => needs_phone("A wider level"),
            Arg::Index(i) if i < LEVELS.len() && LEVELS[i] != now => {
                let mut next = levels.clone();
                next[&gid] = json!(LEVELS[i]);
                vec![send("POST", &format!("/v1/ai/assistants/{assistant}/levels"), Some(json!({"levels": next})))]
            }
            _ => Vec::new(),
        }));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::World;
    use crate::settings::row::Kind;

    fn world(ai: Value) -> World {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        w.topics.helper_up = true;
        w.topics.set_helper("ai", ai);
        w.now = 10_000_000;
        w
    }

    fn doc() -> Value {
        json!({
            "listener": {"enabled": true},
            "pairing": null,
            "assistants": [
                {"id": "a1", "name": "Claude", "paused": false, "levels": {"state": "always", "playback": "ask", "launch": "never", "power": "never"}, "lastSeenAt": 9_700_000},
                {"id": "a2", "name": "Voice", "paused": true, "levels": {"state": "ask", "playback": "ask", "launch": "ask", "power": "ask"}}
            ],
            "limits": {"maxVolume": 60},
            "groups": [{"id": "state", "title": "State"}, {"id": "playback", "title": "Playback"}, {"id": "launch", "title": "Launch"}, {"id": "power", "title": "Power"}]
        })
    }

    fn row<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id}"))
    }

    fn ops(r: &Row, a: Arg) -> Vec<Op> {
        (r.act.as_ref().expect("a row with an action"))(a)
    }

    /// Whether an answer only explains: a dialog with one answer that does nothing, and no request to the helper.
    fn explains(ops: &[Op]) -> bool {
        matches!(ops, [Op::Ask(a)] if a.answers.len() == 1 && a.answers[0].then.is_empty() && a.title == "Not available here")
    }

    fn reaches_helper(ops: &[Op]) -> bool {
        ops.iter().any(|o| matches!(o, Op::Send { .. }) || matches!(o, Op::Later { then, .. } if then.iter().any(|t| matches!(t, Op::Send { .. }))))
    }

    #[test]
    fn what_gives_an_assistant_less_goes_to_the_helper_and_what_gives_more_is_explained_as_unavailable() {
        let w = world(doc());
        let c = w.ctx();
        let r = rows(&c);
        // access: off goes at once, on does not leave the TV's screen
        let l = row(&r, "listener");
        assert_eq!(l.index, Some(1));
        assert!(matches!(&ops(l, Arg::Index(0))[..], [Op::Send { method: "POST", path, body: Some(b), .. }] if path == "/v1/ai/listener" && b == &json!({"enabled": false})));
        let off = world(json!({"listener": {"enabled": false}, "assistants": [], "limits": {"maxVolume": 60}, "groups": []}));
        let off_rows = rows(&off.ctx());
        assert!(explains(&ops(row(&off_rows, "listener"), Arg::Index(1))), "on is explained, not asked of the helper");
        assert!(!row(&off_rows, "pair").enabled, "pairing waits for access");
        // the volume limit: lower writes after a quiet moment and shows at once, higher is explained
        let limit = row(&r, "limit");
        assert_eq!((limit.num, limit.value_text.as_deref()), (60.0, Some("60")));
        let lower = ops(limit, Arg::Value(58.0));
        assert!(reaches_helper(&lower) && matches!(&lower[0], Op::Set(k, v) if k == "ailimit" && v == &json!(58)));
        assert!(explains(&ops(limit, Arg::Value(62.0))));
        // levels: Always -> Ask lowers, Never -> Ask widens
        let state = row(&r, "level:state");
        assert_eq!(state.index, Some(0));
        let lowered = ops(state, Arg::Index(2));
        assert!(matches!(&lowered[..], [Op::Send { path, body: Some(b), .. }] if path == "/v1/ai/assistants/a1/levels" && b["levels"]["state"] == json!("never") && b["levels"]["playback"] == json!("ask")));
        assert!(explains(&ops(row(&r, "level:launch"), Arg::Index(1))), "never -> ask is more");
        assert!(ops(state, Arg::Index(0)).is_empty(), "the level already in force changes nothing");
        // assistants: pause goes, resume is explained, remove asks first
        assert!(matches!(&ops(row(&r, "assistant:a1"), Arg::Index(0))[..], [Op::Send { path, .. }] if path == "/v1/ai/assistants/a1/pause"));
        let resume = ops(row(&r, "assistant:a2"), Arg::Index(0));
        assert!(explains(&resume) && !reaches_helper(&resume));
        let remove = ops(row(&r, "assistant:a1"), Arg::Index(1));
        assert!(matches!(&remove[..], [Op::Ask(a)] if a.answers[1].danger && matches!(&a.answers[1].then[..], [Op::Send { method: "DELETE", path, .. }] if path == "/v1/ai/assistants/a1")));
        let lock = ops(row(&r, "lock"), Arg::Press);
        assert!(matches!(&lock[..], [Op::Ask(a)] if matches!(&a.answers[1].then[..], [Op::Send { path, .. }] if path == "/v1/ai/lockout")));
    }

    #[test]
    fn an_open_pairing_window_shows_its_code_and_closes_but_nothing_here_opens_one() {
        let mut d = doc();
        d["pairing"] = json!({"code": "482913", "attemptsLeft": 3});
        let w = world(d);
        let r = rows(&w.ctx());
        let p = row(&r, "pair");
        assert_eq!((p.kind, p.label.as_str(), p.detail.as_str()), (Kind::Action, "Pairing code 482913", "3 tries left"));
        assert!(matches!(&ops(p, Arg::Press)[..], [Op::Send { method: "DELETE", path, .. }] if path == "/v1/ai/pairing"));
        let none = world(doc());
        let r = rows(&none.ctx());
        assert!(explains(&ops(row(&r, "pair"), Arg::Press)));
    }

    #[test]
    fn the_assistants_tab_says_when_each_was_heard_and_lists_the_recent_calls_newest_first() {
        let mut w = world(doc());
        w.docs.set("aiaudit", json!({"ok": true, "entries": [{"at": 9_000_000, "tool": "tv.play", "outcome": "denied", "assistant": {"id": "a1", "name": "Claude"}}, {"at": 9_990_000, "message": "Set volume", "outcome": "ok", "assistant": {"id": "a1", "name": "Claude"}}]}));
        let r = rows(&w.ctx());
        assert_eq!(row(&r, "assistant:a1").detail, "just now");
        assert_eq!(row(&r, "assistant:a2").detail, "Paused");
        let first = row(&r, "audit:0");
        assert_eq!((first.label.as_str(), first.value.as_str(), first.detail.as_str()), ("Claude \u{b7} Set volume", "Allowed", "just now"));
        assert_eq!(row(&r, "audit:1").value, "Denied");
    }

    #[test]
    fn the_allowed_tab_follows_the_assistant_chosen_and_the_page_says_when_the_helper_is_down() {
        let mut w = world(doc());
        w.local.set("aiwho", json!("a2"));
        let r = rows(&w.ctx());
        assert_eq!(row(&r, "who").index, Some(1));
        assert_eq!(row(&r, "level:state").index, Some(1), "Voice's levels, not Claude's");
        assert_eq!(summary(&w.ctx()), "Allowed");
        w.topics.helper_up = false;
        let r = rows(&w.ctx());
        assert!(r.iter().all(|r| r.kind == Kind::Empty) && r.len() == 3);
        assert_eq!(summary(&w.ctx()), "Not running");
    }
}
