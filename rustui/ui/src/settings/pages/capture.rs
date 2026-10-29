//! Screenshots & recording (`settings/PageCapture.qml`, `settings/Capture.js`, `docs/research/capture-result.md`): a picture of what LumaTV
//! composes, or a recording of it, saved on the TV. The compositor does the work, the helper's `/v1/capture` routes are the way in and
//! `capture.enabled` in the helper's config is the switch: while it is off the rows are dimmed and the page says so. Video is never in a
//! capture (a window for it is black), and a recording shows a red dot in the corner of the screen for as long as it runs. The page asks
//! the helper for the state when it opens and each second while a recording runs; nothing else polls.

use crate::settings::ctx::Ctx;
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Op, Row};
use serde_json::{Value, json};

pub fn page() -> Page {
    Page::new("capture", "Screenshots & recording", "camera").parent("system").back("System").hidden_from_search().reads(reads).rows(rows).summary(summary).answered(answered).opened(opened)
}

fn reads(_: &Ctx) -> Vec<Read> {
    vec![Read::Get { name: "capture", path: "/v1/capture" }]
}

fn opened(_: &Ctx) -> Vec<Op> {
    vec![Op::Set("said".into(), json!("")), Op::Set("waiting".into(), json!(false))]
}

/// What GET /v1/capture said, as the page keeps it: whether the helper answered at all, whether capture is on, whether the compositor
/// answered, the recording as it runs, what the folder holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub answered: bool,
    pub enabled: bool,
    pub available: bool,
    pub recording: bool,
    pub seconds: u64,
    pub count: u64,
    pub bytes: u64,
    pub encoder: bool,
}

pub fn state(doc: &Value) -> State {
    let status = doc.get("status").and_then(Value::as_u64);
    let c = if status.is_some_and(|s| s != 200) { None } else { doc.get("capture").filter(|c| c.is_object()) };
    let Some(c) = c else { return State::default() };
    if c.get("enabled").and_then(Value::as_bool) != Some(true) {
        return State { answered: true, ..State::default() };
    }
    let files = c.get("captures").cloned().unwrap_or(Value::Null);
    State {
        answered: true,
        enabled: true,
        available: c.get("available").and_then(Value::as_bool) == Some(true),
        recording: c.get("recording").and_then(Value::as_bool) == Some(true),
        seconds: c.get("recorder").and_then(|r| r.get("seconds")).and_then(Value::as_f64).unwrap_or(0.0).floor() as u64,
        count: files.get("count").and_then(Value::as_u64).unwrap_or(0),
        bytes: files.get("bytes").and_then(Value::as_u64).unwrap_or(0),
        encoder: c.get("encoder").and_then(|e| e.get("available")).and_then(Value::as_bool) == Some(true),
    }
}

/// What the page last read: the document of the helper, with the status of its answer kept beside it.
fn read(c: &Ctx) -> State {
    let mut d = c.docs.get("capture").clone();
    if let (Some(o), Some(s)) = (d.as_object_mut(), c.docs.get("capture.status").as_u64()) {
        o.insert("status".into(), json!(s));
    }
    state(&d)
}

/// "812 KB", "3.4 MB", "1.2 GB": what the files take.
pub fn size(bytes: u64) -> String {
    let b = bytes as f64;
    if b < 1e6 {
        format!("{} KB", (b / 1e3).round() as u64)
    } else if b < 1e9 {
        format!("{} MB", (b / 1e5).round() / 10.0)
    } else {
        format!("{} GB", (b / 1e8).round() / 10.0)
    }
}

/// "0:07", "12:30": a recording's length so far.
pub fn clock(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// "3 files, 5.2 MB".
pub fn files(st: &State) -> String {
    if !st.enabled { String::new() } else { format!("{} {}, {}", st.count, if st.count == 1 { "file" } else { "files" }, size(st.bytes)) }
}

/// The reason, when the body did not come: the helper's own statuses; the 409 group (no_indicator, busy, not_recording) is told from the
/// state read after the command. "" is none of them.
fn why(kind: &str, status: u64, st: &State) -> &'static str {
    match status {
        403 => "capture_disabled",
        501 => "no_encoder",
        503 => "compositor_unavailable",
        504 => "timeout",
        507 => "disk_full",
        0 | 409 => {
            if !st.answered {
                "helper"
            } else if !st.enabled {
                "capture_disabled"
            } else if kind == "stop" {
                "not_recording"
            } else if kind == "start" && st.recording {
                "busy"
            } else {
                "no_indicator"
            }
        }
        _ => "",
    }
}

/// The words for an answer to a command: `kind` is "shot", "start" or "stop".
pub fn said(kind: &str, status: u64, doc: &Value, st: &State) -> String {
    if (200..300).contains(&status) && doc.get("ok").and_then(Value::as_bool) == Some(true) {
        return match kind {
            "shot" => "Screenshot saved",
            "start" => "Recording",
            _ => "Recording saved",
        }
        .into();
    }
    let given = doc.get("error").and_then(|e| e.get("code")).and_then(Value::as_str).map(str::to_string);
    let code = given.unwrap_or_else(|| why(kind, status, st).to_string());
    match code.as_str() {
        "helper" => "The TV's helper did not answer",
        "capture_disabled" => "Capture is off",
        "no_indicator" => {
            if kind == "shot" {
                "The shell is not on screen"
            } else {
                "The recording dot could not be shown"
            }
        }
        "busy" => "Already recording",
        "not_recording" => "Not recording",
        "no_encoder" => "Recording is not set up on this TV",
        "disk_full" => "Not enough space on the TV",
        "compositor_unavailable" => "Needs LumaTV's own compositor",
        "timeout" => "That took too long",
        _ => {
            if kind == "shot" {
                "The screenshot failed"
            } else {
                "The recording failed"
            }
        }
    }
    .into()
}

fn summary(c: &Ctx) -> String {
    let st = read(c);
    if st.recording { format!("Recording {}", clock(st.seconds)) } else { String::new() }
}

/// A command, then the state again, then what happened in words.
fn run(kind: &'static str, path: &'static str) -> Vec<Op> {
    vec![Op::Set("busy".into(), json!(true)), Op::Send { name: Some(format!("capture.{kind}")), method: "POST", path: path.into(), body: Some(json!({})) }]
}

fn rows(c: &Ctx) -> Vec<Row> {
    let st = read(c);
    let usable = st.enabled && st.available;
    let busy = c.local.flag("busy");
    let waiting = c.local.flag("waiting");
    let mut rows = Vec::new();
    let mut off = Row::note("Screen capture is off on this TV.", 0);
    off.visible = !st.enabled;
    rows.push(off);
    let said = c.local.text("said");
    let mut note = Row::note(&said, 0);
    note.id = "said".into();
    note.visible = !said.is_empty();
    rows.push(note);
    let mut shot = Row::action("shot", "Capture").icon("camera").label("Take a screenshot").help("Saves a picture of the screen on the TV. Video stays black in it.");
    shot.detail = if st.enabled && !st.available { "Needs LumaTV's own compositor".into() } else { String::new() };
    shot.enabled = usable && !busy && !waiting;
    rows.push(shot.does(run("shot", "/v1/capture/screenshot")));
    let mut later = Row::action("later", if waiting { "Waiting" } else { "Start" }).icon("camera").label("Screenshot in 5 seconds").help("Waits five seconds first, so you can go to the screen you want.");
    later.enabled = usable && !busy && !waiting;
    rows.push(later.does(vec![Op::Set("waiting".into(), json!(true)), Op::Later { key: "capture-later".into(), ms: 5000, then: [vec![Op::Set("waiting".into(), json!(false))], run("shot", "/v1/capture/screenshot")].concat() }]));
    let mut record = Row::toggle("record").icon("video").label("Screen recording").help("Records what LumaTV shows into a file on the TV. A red dot is on the screen while it records. Video stays black in it.").on(st.recording);
    record.detail = if st.recording { clock(st.seconds) } else if usable && !st.encoder { "Not set up on this TV".into() } else { String::new() };
    record.enabled = (usable && st.encoder && !busy) || st.recording;
    rows.push(record.act(|a| match a {
        Arg::Toggle(true) => run("start", "/v1/capture/record/start"),
        Arg::Toggle(false) => run("stop", "/v1/capture/record/stop"),
        _ => Vec::new(),
    }));
    let mut saved = Row::info("files").icon("hard-drive").label("Saved on the TV").help("They are kept on the TV. When there is no more room for a new one, the oldest go first.").value(&files(&st));
    saved.chevron = false;
    saved.visible = st.enabled;
    rows.push(saved);
    rows
}

/// A command's answer waits for the state to be read again, and is told in words then (a 409 is told from that state); a recording that
/// runs is read each second.
fn answered(c: &Ctx, name: &str, doc: &Value) -> Vec<Op> {
    let mut ops = Vec::new();
    if let Some(kind) = name.strip_prefix("capture.").filter(|k| matches!(*k, "shot" | "start" | "stop")) {
        let status = c.docs.get(&format!("{name}.status")).as_u64().unwrap_or(0);
        ops.push(Op::Set("busy".into(), json!(false)));
        ops.push(Op::Set("after".into(), json!({"kind": kind, "status": status, "doc": doc})));
        ops.push(Op::Refresh);
    }
    if name == "capture" {
        let st = state_of(doc, c);
        if let Some(after) = c.local.get("after").as_object() {
            let kind = after.get("kind").and_then(Value::as_str).unwrap_or("shot");
            let status = after.get("status").and_then(Value::as_u64).unwrap_or(0);
            ops.push(Op::Set("said".into(), json!(said(kind, status, after.get("doc").unwrap_or(&Value::Null), &st))));
            ops.push(Op::Set("after".into(), Value::Null));
        }
        if st.recording {
            ops.push(Op::RefreshIn(1000));
        }
    }
    ops
}

fn state_of(doc: &Value, c: &Ctx) -> State {
    let mut d = doc.clone();
    if let (Some(o), Some(s)) = (d.as_object_mut(), c.docs.get("capture.status").as_u64()) {
        o.insert("status".into(), json!(s));
    }
    state(&d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::World;

    fn by_id<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id}"))
    }

    fn world(doc: Value) -> World {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        w.docs.set("capture", doc);
        w.docs.set("capture.status", json!(200));
        w
    }

    #[test]
    fn the_state_is_what_the_helper_said_and_off_until_capture_is_turned_on() {
        assert_eq!(state(&json!({"capture": {"enabled": false}})), State { answered: true, ..State::default() });
        assert_eq!(state(&Value::Null), State::default(), "the helper did not answer");
        let on = state(&json!({"capture": {"enabled": true, "available": true, "recording": true, "recorder": {"seconds": 7.9}, "captures": {"count": 3, "bytes": 5_234_000}, "encoder": {"available": true}}}));
        assert_eq!((on.enabled, on.available, on.recording, on.seconds, on.count, on.encoder), (true, true, true, 7, 3, true));
        assert_eq!(files(&on), "3 files, 5.2 MB");
        assert_eq!(clock(7), "0:07");
        assert_eq!(clock(750), "12:30");
        assert_eq!(size(812_000), "812 KB");
        assert_eq!(size(1_234_000_000), "1.2 GB");
    }

    #[test]
    fn the_words_for_an_answer_come_from_its_code_or_from_the_state_read_after_it() {
        let st = State { answered: true, enabled: true, available: true, ..State::default() };
        assert_eq!(said("shot", 200, &json!({"ok": true}), &st), "Screenshot saved");
        assert_eq!(said("stop", 200, &json!({"ok": true}), &st), "Recording saved");
        assert_eq!(said("shot", 507, &json!({}), &st), "Not enough space on the TV");
        assert_eq!(said("shot", 409, &json!({"error": {"code": "busy"}}), &st), "Already recording");
        assert_eq!(said("stop", 0, &Value::Null, &st), "Not recording", "a 409 the workstation host hands over as status 0 is told from the state");
        assert_eq!(said("start", 409, &Value::Null, &State { recording: true, ..st.clone() }), "Already recording");
        assert_eq!(said("shot", 0, &Value::Null, &State::default()), "The TV's helper did not answer");
        assert_eq!(said("start", 418, &Value::Null, &st), "The recording failed");
    }

    #[test]
    fn rows_are_dimmed_while_capture_is_off_and_a_recording_can_always_be_stopped() {
        let w = world(json!({"capture": {"enabled": false}}));
        let r = rows(&w.ctx());
        assert!(by_id(&r, "note:Screen capture is off on this TV.").visible);
        assert!(!by_id(&r, "shot").enabled && !by_id(&r, "later").enabled && !by_id(&r, "record").enabled);
        assert!(!by_id(&r, "files").visible);
        let w = world(json!({"capture": {"enabled": true, "available": true, "recording": true, "recorder": {"seconds": 65}, "encoder": {"available": false}, "captures": {"count": 1, "bytes": 1000}}}));
        let r = rows(&w.ctx());
        assert!(by_id(&r, "record").on && by_id(&r, "record").enabled && by_id(&r, "record").detail == "1:05");
        assert_eq!(summary(&w.ctx()), "Recording 1:05");
        assert!(matches!(&(by_id(&r, "record").act.as_ref().unwrap())(Arg::Toggle(false))[..], [Op::Set(..), Op::Send { path, .. }] if path == "/v1/capture/record/stop"));
        // a recording that runs is read again in a second
        let ops = answered(&w.ctx(), "capture", w.docs.get("capture"));
        assert!(matches!(&ops[..], [Op::RefreshIn(1000)]));
    }

    #[test]
    fn a_screenshot_is_a_command_and_the_one_in_five_seconds_waits_then_takes_it() {
        let w = world(json!({"capture": {"enabled": true, "available": true, "encoder": {"available": true}, "captures": {"count": 0, "bytes": 0}}}));
        let r = rows(&w.ctx());
        assert!(by_id(&r, "shot").enabled && by_id(&r, "record").enabled);
        let ops = (by_id(&r, "shot").act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&ops[..], [Op::Set(k, _), Op::Send { name: Some(n), path, .. }] if k == "busy" && n == "capture.shot" && path == "/v1/capture/screenshot"));
        let ops = (by_id(&r, "later").act.as_ref().unwrap())(Arg::Press);
        let [Op::Set(..), Op::Later { ms, then, .. }] = &ops[..] else { panic!() };
        assert_eq!(*ms, 5000);
        assert!(matches!(&then[..], [Op::Set(..), Op::Set(..), Op::Send { .. }]));
        // the answer waits for the state to be read again, then it is told in words
        let mut w = w;
        w.docs.set("capture.shot.status", json!(507));
        let ops = answered(&w.ctx(), "capture.shot", &json!({"ok": false}));
        assert!(matches!(&ops[..], [Op::Set(..), Op::Set(k, _), Op::Refresh] if k == "after"));
        w.local.set("after", json!({"kind": "shot", "status": 507, "doc": {"ok": false}}));
        let ops = answered(&w.ctx(), "capture", w.docs.get("capture"));
        assert!(matches!(&ops[..], [Op::Set(k, v), Op::Set(k2, Value::Null)] if k == "said" && v == &json!("Not enough space on the TV") && k2 == "after"));
        w.local.set("waiting", json!(true));
        assert!(!by_id(&rows(&w.ctx()), "later").enabled && by_id(&rows(&w.ctx()), "later").button == "Waiting");
    }
}
