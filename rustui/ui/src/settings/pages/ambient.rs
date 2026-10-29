//! Ambient light (`settings/PageAmbient.qml`, `docs/mocks/30-settings-ambient`): the settings of LumaTV's own lights-around-the-TV feature,
//! kept in the helper's settings layer as the owner `base.ambient` and drawn through the same row builders as the other helper owners.
//! This page only sets; the capture, the mapping and the light output run in the helper (`GET /v1/ambient` says what they are doing).
//! The Sound row is the helper's sound mode: the picture as it is, with the TV's own sound on it (brightness and beats), or the sound
//! alone. The strip's and Hue's addresses are not settings here: they live in the helper's own file. LG has no equivalent on the C4;
//! its "Lighting" settings are for other models and are not shown.

use super::owner::{Field, seg, slider, toggle};
use crate::settings::ctx::Ctx;
use crate::settings::page::{Page, Read};
use crate::settings::row::{Op, Row};
use serde_json::{Value, json};

const OWNER: &str = "base.ambient";
/// While the test pattern runs the page asks the helper again at this rate, to show Stop and to learn when it ended.
const TESTING_MS: u64 = 1500;

pub fn page() -> Page {
    Page::new("ambient", "Ambient light", "lamp").rail().tabs(&["Light", "Layout", "Colour"]).reads(reads).rows(rows).summary(summary).answered(answered)
}

fn reads(_: &Ctx) -> Vec<Read> {
    vec![Read::owner(OWNER), Read::Get { name: "amb", path: "/v1/ambient" }]
}

fn field<'a>(key: &'a str, tab: usize, icon: &'a str, label: &'a str) -> Field<'a> {
    Field::new(OWNER, key, tab, icon, label)
}

fn summary(c: &Ctx) -> String {
    match c.owner(OWNER).filter(|o| o.has("enabled")).and_then(|o| o.get("enabled")) {
        None => "Not available".into(),
        Some(v) if v == &json!(true) => "On".into(),
        Some(_) => "Off".into(),
    }
}

/// What the helper says it is doing (`GET /v1/ambient`), when it answered.
fn amb<'a>(c: &'a Ctx) -> Option<&'a Value> {
    let d = c.docs.get("amb");
    (d.get("ok").and_then(Value::as_bool) == Some(true)).then_some(d)
}

fn testing(c: &Ctx) -> bool {
    amb(c).and_then(|d| d.get("state")).and_then(Value::as_str) == Some("testing")
}

fn sections(c: &Ctx) -> i64 {
    ["top", "right", "bottom", "left"].iter().map(|k| c.owner(OWNER).map_or(0.0, |o| o.num(k, 0.0)).round() as i64).sum()
}

/// The helper's words when it could not run the test pattern, else the problem it reports.
fn problem(c: &Ctx) -> String {
    let said = c.local.text("testProblem");
    if !said.is_empty() {
        return said;
    }
    amb(c).and_then(|d| d.get("problem")).map(|p| p.as_str().map_or_else(|| p.to_string(), str::to_string)).filter(|p| p != "null" && !p.is_empty()).unwrap_or_default()
}

fn answered(_: &Ctx, name: &str, doc: &Value) -> Vec<Op> {
    match name {
        // the helper's answer to the test pattern: its words when it refused (409), then the state again
        "ambtest" => {
            let said = doc.pointer("/error/message").and_then(Value::as_str).unwrap_or("");
            vec![Op::Set("testProblem".into(), json!(said)), Op::Refresh]
        }
        "amb" if doc.get("state").and_then(Value::as_str) == Some("testing") => vec![Op::RefreshIn(TESTING_MS)],
        _ => Vec::new(),
    }
}

fn rows(c: &Ctx) -> Vec<Row> {
    let lamp = |key, icon, label, help| field(key, 0, icon, label).help(help);
    let mut rows = vec![
        toggle(c, lamp("enabled", "lamp", "Ambient light", "Lights around the TV follow what is on the screen.")),
        slider(c, lamp("brightness", "sun", "Brightness", "How bright the strip goes. It dims with the room at night."), 0.0, 100.0, 5.0, "%"),
        seg(c, lamp("mode", "sliders-horizontal", "Mode", "Follows the picture, shows a still colour, or goes off on Home and menus."), &["Picture", "Colour", "Off on Home"], &["picture", "colour", "picture-off-home"]),
        seg(c, lamp("sound", "audio-waveform", "Sound", "Adds the TV's own sound: its loudness and its beats. Nothing is saved."), &["Picture", "Picture + sound", "Sound"], &["picture", "picture-sound", "sound"]),
    ];
    let said = problem(c);
    if !said.is_empty() {
        rows.push(Row::note(&said, 0));
    }
    let known = amb(c).and_then(|d| d.get("strip")).and_then(Value::as_bool) == Some(true);
    let lit = |k: &str| c.owner(OWNER).is_some_and(|o| o.num(k, 0.0) > 0.0);
    rows.push(Row::head("Strip", 0));
    rows.push(Row::figure("strip", &format!("{} sections", sections(c)), [lit("top"), lit("bottom"), lit("left"), lit("right")], 0));
    let mut test = Row::action("test", if testing(c) { "Stop" } else { "Run" }).icon("cable").label("Test pattern").help("Lights each section in turn so the layout can be checked.").enabled(known);
    test.detail = if amb(c).is_none() { "Not answering" } else if known { "" } else { "No strip set up" }.into();
    rows.push(test.admin().does(vec![Op::Send { name: Some("ambtest".into()), method: "POST", path: "/v1/ambient/test".into(), body: Some(json!({"run": !testing(c)})) }]));

    // ---- Layout
    rows.push(slider(c, field("top", 1, "arrow-big-up", "Top").help("Sections along the top edge, left to right. The edges run clockwise from the top left."), 0.0, 120.0, 1.0, ""));
    rows.push(slider(c, field("right", 1, "chevron-right", "Right"), 0.0, 120.0, 1.0, ""));
    rows.push(slider(c, field("bottom", 1, "chevron-down", "Bottom"), 0.0, 120.0, 1.0, ""));
    rows.push(slider(c, field("left", 1, "arrow-left", "Left"), 0.0, 120.0, 1.0, ""));
    rows.push(slider(c, field("delay_ms", 1, "timer", "Delay").help("The lights follow the picture this late, to line up with the sound."), 0.0, 2000.0, 10.0, " ms"));

    // ---- Colour
    rows.push(seg(c, field("still_colour", 2, "palette", "Still colour").help("The colour in Colour mode."), &["Warm", "Cool", "Rose", "Iris", "Foam"], &["warm", "cool", "rose", "iris", "foam"]));
    rows.push(slider(c, field("saturation", 2, "blend", "Saturation").help("How strong the colours are next to the picture."), 50.0, 150.0, 5.0, "%"));
    rows.push(toggle(c, field("dark_hold", 2, "moon", "Glow in dark scenes").help("A very dim glow instead of off, which a lamp can flicker at.")));
    rows.push(toggle(c, field("skip_bars", 2, "film", "Skip black bars").help("Ignores the bars of letterboxed and pillarboxed films.")));
    rows.push(toggle(c, field("game_edges", 2, "gamepad-2", "Edges for games").help("Uses the edge-weighted capture on a game input.")));
    rows
}

#[cfg(test)]
mod tests {
    use super::super::owner::fixtures::{doc, world_with};
    use super::*;
    use crate::settings::row::{Arg, Kind};

    fn owner_doc() -> Value {
        doc(&[
            ("enabled", "bool", json!(true)),
            ("brightness", "integer", json!(70)),
            ("mode", "choice", json!("picture")),
            ("sound", "choice", json!("picture")),
            ("top", "integer", json!(18)),
            ("right", "integer", json!(11)),
            ("bottom", "integer", json!(18)),
            ("left", "integer", json!(0)),
            ("delay_ms", "integer", json!(40)),
            ("still_colour", "choice", json!("warm")),
            ("saturation", "integer", json!(100)),
            ("dark_hold", "bool", json!(false)),
            ("skip_bars", "bool", json!(true)),
            ("game_edges", "bool", json!(false)),
        ])
    }

    #[test]
    fn the_rows_of_the_three_tabs_read_the_owner_and_the_strip_figure_lights_the_edges_that_have_sections() {
        let mut w = world_with(OWNER, &owner_doc());
        w.docs.set("amb", json!({"ok": true, "state": "running", "strip": true, "sections": 47}));
        let c = w.ctx();
        assert_eq!(summary(&c), "On");
        let r = rows(&c);
        let tab = |t: usize| r.iter().filter(|x| x.tab == t).map(|x| x.id.as_str()).collect::<Vec<_>>();
        assert_eq!(tab(0), vec!["enabled", "brightness", "mode", "sound", "head:Strip", "strip", "test"]);
        assert_eq!(tab(1), vec!["top", "right", "bottom", "left", "delay_ms"]);
        assert_eq!(tab(2), vec!["still_colour", "saturation", "dark_hold", "skip_bars", "game_edges"]);
        let fig = r.iter().find(|x| x.kind == Kind::Figure).unwrap();
        assert_eq!((fig.text.as_str(), fig.zones), ("47 sections", [true, true, false, true]), "top, bottom, left, right: the left edge has none");
        let test = r.iter().find(|x| x.id == "test").unwrap();
        assert!(test.enabled && test.button == "Run" && test.detail.is_empty());
        let ops = (test.act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&ops[..], [Op::Send { name: Some(n), method: "POST", path, body: Some(b) }] if n == "ambtest" && path == "/v1/ambient/test" && b == &json!({"run": true})));
        let mode = r.iter().find(|x| x.id == "mode").unwrap();
        assert_eq!(mode.index, Some(0));
        assert!(matches!(&(mode.act.as_ref().unwrap())(Arg::Index(2))[..], [Op::Owner { key, value, .. }] if key == "mode" && value == &json!("picture-off-home")));
        let delay = r.iter().find(|x| x.id == "delay_ms").unwrap();
        assert_eq!((delay.value_text.as_deref(), delay.max, delay.step), (Some("40 ms"), 2000.0, 10.0));
    }

    #[test]
    fn the_test_pattern_waits_for_a_strip_runs_as_stop_while_it_runs_and_a_refusal_is_said_in_a_note() {
        let mut w = world_with(OWNER, &owner_doc());
        let c = w.ctx();
        let t = rows(&c).into_iter().find(|x| x.id == "test").unwrap();
        assert!(!t.enabled && t.detail == "Not answering", "the helper did not answer");
        w.docs.set("amb", json!({"ok": true, "state": "idle", "strip": false}));
        let t = rows(&w.ctx()).into_iter().find(|x| x.id == "test").unwrap();
        assert!(!t.enabled && t.detail == "No strip set up");
        w.docs.set("amb", json!({"ok": true, "state": "testing", "strip": true}));
        let t = rows(&w.ctx()).into_iter().find(|x| x.id == "test").unwrap();
        assert!(t.enabled && t.button == "Stop");
        assert!(matches!(&(t.act.as_ref().unwrap())(Arg::Press)[..], [Op::Send { body: Some(b), .. }] if b == &json!({"run": false})));
        assert!(matches!(&answered(&w.ctx(), "amb", &json!({"state": "testing"}))[..], [Op::RefreshIn(1500)]), "asks again while it runs");
        let ops = answered(&w.ctx(), "ambtest", &json!({"ok": false, "error": {"message": "The strip is busy"}}));
        assert!(matches!(&ops[..], [Op::Set(k, v), Op::Refresh] if k == "testProblem" && v == "The strip is busy"));
        w.local.set("testProblem", json!("The strip is busy"));
        let r = rows(&w.ctx());
        assert_eq!(r.iter().find(|x| x.kind == Kind::Note).map(|x| x.text.as_str()), Some("The strip is busy"));
    }

    #[test]
    fn without_the_helper_the_rail_says_not_available_and_the_rows_are_dimmed() {
        let w = crate::settings::ctx::fixtures::World::new(crate::host::sio::Sio::new(0));
        let c = w.ctx();
        assert_eq!(summary(&c), "Not available");
        assert!(rows(&c).iter().filter(|r| r.kind == Kind::Toggle).all(|r| !r.enabled));
    }
}
