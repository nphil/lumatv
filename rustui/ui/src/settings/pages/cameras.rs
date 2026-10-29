//! Cameras, a page of Home & look (`docs/research/cameras-ui-spec.md` "The Settings page"): the settings page of the helper's Scrypted
//! cameras plugin (`scrypted-cameras`), drawn from the schema it serves (`GET /v1/settings/scrypted-cameras`, `plugin_rows`) and written
//! through the same route. Tabs: the link to Scrypted, which cameras show, the pop-ups, what each camera pops up for, and the video's
//! quality. The Cameras app's Settings button opens it; the page of one camera's kinds hangs under it and is the same list the app's
//! hold-OK options page makes (`cameras::options`): the order picked is kept, the defaults are person and doorbell.
//!
//! The link itself (the bridge's address and token) is the owner's from a shell on the TV (`lumatv-helper scrypted-link`); this page shows
//! the state, the name and verifier to type into the bridge, and can forget the link, which only takes access away.

use super::plugin_rows::{Plugin, ask_all, fact, field_rows, fields, text};
use crate::cameras::data::{Cameras, LinkState, default_kinds};
use crate::cameras::{options, words};
use crate::settings::ctx::{Ctx, ask};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Op, PageArg, Row};
use serde_json::{Value, json};

const ID: &str = options::OWNER;
const PLUGIN: Plugin = Plugin { id: ID, sections: &["link", "show", "popups", "notify", "video"] };
const TABS: [&str; 5] = ["Scrypted", "Cameras", "Pop-ups", "Notify for", "Video"];
const NOTIFY: usize = 3;
const WHERE: &str = "Link Scrypted from a shell on the TV with lumatv-helper scrypted-link.";

pub fn page() -> Page {
    Page::new("cameras", "Cameras", "video")
        .parent("look")
        .back("Home & look")
        .tabs(&TABS)
        .reads(reads)
        .rows(rows)
        .summary(summary)
        .chip(chip)
        .opened(opened)
        .answered(answered)
}

/// One camera's kinds (`options`'s page): hangs under Cameras.
pub fn notify() -> Page {
    Page::new("camnotify", "Camera", "video").parent("look").back("Cameras").hidden_from_search().reads(reads).rows(notify_rows).title_of(notify_title)
}

fn reads(_: &Ctx) -> Vec<Read> {
    vec![Read::owner(ID)]
}

fn opened(_: &Ctx) -> Vec<Op> {
    vec![Op::Set("asked".into(), json!("")), Op::Set("seen".into(), json!(""))]
}

/// What the `cameras` topic says: the link and the cameras shared.
fn cameras(c: &Ctx) -> Cameras {
    c.helper("cameras").map(Cameras::parse).unwrap_or_default()
}

/// The page's state in words and the tone of its chip.
fn state_words(c: &Ctx) -> (String, &'static str) {
    let t = cameras(c);
    match t.state {
        LinkState::Ready => (format!("Connected \u{b7} {}", if t.list.is_empty() { words::NO_CAMERAS.to_string() } else { words::count(t.list.len()) }), "positive"),
        LinkState::Unlinked => ("Not linked".into(), "info"),
        LinkState::Connecting => ("Connecting".into(), "info"),
        LinkState::Offline => (words::link_words("offline").into(), "warning"),
        LinkState::Refused => (words::link_words("refused").into(), "danger"),
        LinkState::Untrusted => (words::link_words("untrusted").into(), "danger"),
    }
}

/// The value of the row of Home & look that opens the page.
pub fn summary(c: &Ctx) -> String {
    let t = cameras(c);
    match t.state {
        LinkState::Ready => words::count(t.list.len()),
        _ => state_words(c).0,
    }
}

fn chip(c: &Ctx) -> Option<(String, &'static str)> {
    Some(state_words(c))
}

fn answered(c: &Ctx, name: &str, _: &Value) -> Vec<Op> {
    match name {
        // the link or the cameras changed: the page's schema may have (a camera has a field of its own)
        "topic:cameras" => {
            let t = cameras(c);
            let key = format!("{}|{}", t.state.name(), t.list.iter().map(|k| k.id.as_str()).collect::<Vec<_>>().join(","));
            if c.local.text("seen") == key { Vec::new() } else { vec![Op::Set("seen".into(), json!(key)), Op::Refresh] }
        }
        "owner:scrypted-cameras" => {
            let Some(o) = c.owner(ID).filter(|o| o.known) else { return Vec::new() };
            let key = format!("{}|{}", cameras(c).state.name(), o.schema.get("revision").and_then(Value::as_i64).unwrap_or(0));
            if c.local.text("asked") == key {
                return Vec::new();
            }
            let mut ops = vec![Op::Set("asked".into(), json!(key))];
            ops.extend(ask_all(PLUGIN, &o.schema));
            ops
        }
        _ => Vec::new(),
    }
}

/// The verifier is 64 digits: in groups of eight it is read and typed across, and a line breaks between groups, not inside one.
fn grouped(v: &str) -> String {
    v.as_bytes().chunks(8).map(|g| String::from_utf8_lossy(g).into_owned()).collect::<Vec<_>>().join(" ")
}

fn rows(c: &Ctx) -> Vec<Row> {
    let (words, _) = state_words(c);
    let linked = cameras(c).state != LinkState::Unlinked;
    let mut rows = vec![Row::info("link").tab(0).icon("video").label("Connection").help("Linked from a shell on the TV with lumatv-helper scrypted-link").value(&words)];
    let tell = "Type this name and verifier into the bridge's Devices in Scrypted. The verifier is a hash; the token never leaves the TV.";
    if linked {
        if let Some(d) = fact(c, PLUGIN, "device").filter(|d| !d.is_empty()) {
            rows.push(Row::info("device").tab(0).icon("tv-minimal").label("Device name").help(tell).value(&d));
        }
        if let Some(v) = fact(c, PLUGIN, "verifier").filter(|v| !v.is_empty()) {
            let mut r = Row::info("verifier").tab(0).icon("hash").label("Verifier").help(tell);
            r.detail = grouped(&v);
            rows.push(r);
        }
        rows.push(Row::action("forget", "Forget").tab(0).icon("unplug").label("Forget link").help("Deletes this TV's token for Scrypted").danger().admin().does(ask(
            "unplug",
            "Forget Scrypted?",
            "The TV forgets its token for Scrypted and shows no cameras until it is linked again. Nothing is changed in Scrypted.",
            "Forget",
            "Cancel",
            true,
            vec![Op::Send { name: None, method: "POST", path: format!("/v1/plugins/{ID}/actions/forget"), body: Some(json!({})) }],
        )));
    }
    let o = c.owner(ID).filter(|o| o.known);
    let Some(o) = o.filter(|_| linked) else {
        let (title, line) = if linked { ("Not available yet", "The page is set once the plugin is running.") } else { ("Not linked", WHERE) };
        rows.extend((1..TABS.len()).map(|t| Row::empty(title, line, "unplug", t)));
        return rows;
    };
    for (tab, f) in fields(PLUGIN, &o.schema) {
        if tab == 0 {
            continue;
        }
        let key = text(f, "key");
        if tab == NOTIFY {
            if let Some(camera) = key.strip_prefix("notify.") {
                rows.push(notify_row(o.get(key), camera, text(f, "label")));
            }
            continue;
        }
        rows.extend(field_rows(c, PLUGIN, tab, f, ""));
    }
    for t in 1..TABS.len() {
        if !rows.iter().any(|r| r.tab == t) {
            rows.push(if t == NOTIFY { Row::empty("No cameras", "Cameras appear here once Scrypted shares some.", "video", t) } else { Row::empty("Nothing to set", "The plugin has no settings for this.", "video", t) });
        }
    }
    rows
}

/// The kinds a camera pops up for: the owner's pick, else the defaults.
fn kinds_of(pick: Option<&Value>) -> Vec<String> {
    match pick {
        Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        _ => default_kinds(),
    }
}

fn notify_row(pick: Option<&Value>, camera: &str, name: &str) -> Row {
    Row::link(&format!("notify:{camera}")).tab(NOTIFY).icon("video").label(name).value(&words::notify_value(&kinds_of(pick))).does(vec![Op::Open { page: "camnotify".into(), arg: PageArg::Json(json!({"camera": camera})) }])
}

fn camera_of<'a>(c: &'a Ctx) -> Option<&'a str> {
    c.arg.str("camera")
}

/// The camera's name: the label of its field in the schema.
fn name_of(c: &Ctx, camera: &str) -> Option<String> {
    let schema = &c.owner(ID)?.schema;
    fields(PLUGIN, schema).into_iter().find(|(_, f)| text(f, "key").strip_prefix("notify.") == Some(camera)).map(|(_, f)| text(f, "label").to_string())
}

fn notify_title(c: &Ctx) -> String {
    camera_of(c).map(|cam| name_of(c, cam).unwrap_or_else(|| cam.to_string())).unwrap_or_else(|| "Camera".into())
}

fn notify_rows(c: &Ctx) -> Vec<Row> {
    let Some(camera) = camera_of(c) else { return vec![Row::empty("No such camera", "It is no longer shared.", "video", 0)] };
    let (Some(key), Some(o)) = (options::field_key(camera), c.owner(ID)) else { return vec![Row::empty("No such camera", "It is no longer shared.", "video", 0)] };
    if name_of(c, camera).is_none() && o.known {
        return vec![Row::empty("No such camera", "It is no longer shared.", "video", 0)];
    }
    let now = kinds_of(o.get(&key));
    let mut rows = vec![Row::head(options::GROUP, 0)];
    for kind in words::KINDS {
        let (key, on, now) = (key.clone(), now.iter().any(|k| k == kind), now.clone());
        rows.push(Row::toggle(kind).tab(0).icon(words::kind_icon(kind)).label(words::kind_name(kind)).on(on).enabled(o.has(&key)).admin().act(move |a| match a {
            Arg::Toggle(v) => {
                if v == on {
                    return Vec::new();
                }
                let mut next: Vec<String> = now.iter().filter(|k| *k != kind).cloned().collect();
                if v {
                    next.push(kind.to_string());
                }
                vec![Op::Owner { owner: ID.into(), key: key.clone(), value: json!(next) }]
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

    fn schema() -> Value {
        json!({"revision": 1, "sections": [
            {"id": "link", "title": "Scrypted", "fields": [{"key": "link.status", "label": "Connection", "kind": {"type": "read-only", "value": {"capability": "plugin.scrypted-cameras.link"}}}]},
            {"id": "show", "title": "Cameras", "fields": [{"key": "show", "label": "Show cameras", "help": "Empty shows every camera.", "kind": {"type": "ordered-set", "options": {"provider": "cameras"}, "max_items": 128}}]},
            {"id": "popups", "title": "Pop-ups", "fields": [
                {"key": "popups.on", "label": "Event pop-ups", "kind": {"type": "bool"}},
                {"key": "popups.seconds", "label": "Stay for", "kind": {"type": "integer", "min": 5, "max": 60, "step": 1, "unit": "s"}}]},
            {"id": "notify", "title": "Notify for", "fields": [
                {"key": "notify.270", "label": "Front door", "kind": {"type": "ordered-set", "options": {"provider": "kinds"}, "max_items": 8}},
                {"key": "notify.271", "label": "Garage", "kind": {"type": "ordered-set", "options": {"provider": "kinds"}, "max_items": 8}}]},
            {"id": "video", "title": "Video", "fields": [
                {"key": "video.live", "label": "Live quality", "kind": {"type": "choice", "options": {"inline": [{"id": "native", "label": "Full"}, {"id": "balanced", "label": "Medium"}, {"id": "light", "label": "Low"}]}}},
                {"key": "video.grid", "label": "Grid", "kind": {"type": "choice", "options": {"inline": [{"id": "light", "label": "Light"}, {"id": "balanced", "label": "Balanced"}, {"id": "smooth", "label": "Smooth"}]}}}]}
        ]})
    }

    fn world(state: &str) -> World {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        w.topics.helper_up = true;
        w.topics.set_helper("cameras", json!({"state": state, "cameras": [{"id": "270", "name": "Front door"}, {"id": "271", "name": "Garage"}]}));
        let mut o = crate::settings::src::Owner::new(ID);
        o.take(&json!({"ok": true, "version": 3, "schema": schema(), "effective": {
            "popups.on": {"value": true}, "popups.seconds": {"value": 12}, "video.live": {"value": "native"}, "video.grid": {"value": "balanced"},
            "notify.270": {"value": ["person", "vehicle"]}, "notify.271": {"value": []}}}));
        w.owners.insert(ID.into(), o);
        w
    }

    fn row<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id}"))
    }

    #[test]
    fn the_connection_says_the_state_and_the_cameras_and_the_page_has_nothing_more_while_unlinked() {
        let w = world("ready");
        assert_eq!(chip(&w.ctx()), Some(("Connected \u{b7} 2 cameras".into(), "positive")));
        assert_eq!(summary(&w.ctx()), "2 cameras");
        let r = rows(&w.ctx());
        assert_eq!(row(&r, "link").value, "Connected \u{b7} 2 cameras");
        let w = world("unlinked");
        let r = rows(&w.ctx());
        assert_eq!(chip(&w.ctx()), Some(("Not linked".into(), "info")));
        assert!(r.iter().filter(|r| r.tab == 0).count() == 1 && r.iter().filter(|r| r.kind == Kind::Empty).count() == 4);
        assert!(r.iter().any(|r| r.text.contains("scrypted-link")), "says how to link");
        let refused = world("refused");
        assert_eq!(chip(&refused.ctx()), Some(("Scrypted refused this TV".into(), "danger")));
    }

    #[test]
    fn the_name_and_verifier_are_shown_in_groups_and_forgetting_asks_first() {
        let mut w = world("ready");
        w.topics.set_helper("plugin.scrypted-cameras.facts", json!({"facts": {"device": "Living room TV", "verifier": "0123456789abcdef".repeat(4)}}));
        let r = rows(&w.ctx());
        assert_eq!(row(&r, "device").value, "Living room TV");
        assert_eq!(row(&r, "verifier").detail.split(' ').map(str::len).collect::<Vec<_>>(), vec![8; 8], "eight groups of eight, none cut");
        let forget = row(&r, "forget");
        assert!(forget.danger);
        let ops = (forget.act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&ops[..], [Op::Ask(a)] if a.title == "Forget Scrypted?" && matches!(&a.answers[1].then[..], [Op::Send { path, .. }] if path == "/v1/plugins/scrypted-cameras/actions/forget")));
    }

    #[test]
    fn each_tab_draws_its_fields_the_notify_tab_one_row_a_camera_with_the_kinds_as_its_value() {
        let mut w = world("ready");
        w.docs.set("choices:scrypted-cameras:cameras", json!({"page": {"state": "ready", "rows": [{"id": "270", "label": "Front door", "detail": "Porch"}], "more": false}}));
        let r = rows(&w.ctx());
        assert!(row(&r, "show:270").tab == 1 && row(&r, "show:270").detail == "Porch");
        assert!(row(&r, "popups.on").on && row(&r, "popups.seconds").value_text.as_deref() == Some("12 s"));
        let (front, garage) = (row(&r, "notify:270"), row(&r, "notify:271"));
        assert_eq!((front.value.as_str(), garage.value.as_str(), front.label.as_str()), ("Person, Vehicle", "Off", "Front door"));
        assert!(matches!(&(front.act.as_ref().unwrap())(Arg::Press)[..], [Op::Open { page, arg }] if page == "camnotify" && arg.str("camera") == Some("270")));
        // two or three answers are segments, as the spec has them
        let (live, grid) = (row(&r, "video.live"), row(&r, "video.grid"));
        assert_eq!((live.kind, live.index, grid.index), (Kind::Seg, Some(0), Some(1)));
        assert!(matches!(&(live.act.as_ref().unwrap())(Arg::Index(2))[..], [Op::Owner { key, value, .. }] if key == "video.live" && value == &json!("light")));
    }

    #[test]
    fn a_camera_page_has_a_switch_for_each_kind_in_the_order_picked_and_defaults_for_a_camera_with_no_pick() {
        let mut w = world("ready");
        w.arg = PageArg::Json(json!({"camera": "270"}));
        assert_eq!(notify_title(&w.ctx()), "Front door");
        let r = notify_rows(&w.ctx());
        assert_eq!(r.iter().filter(|r| r.kind == Kind::Toggle).count(), 6);
        assert!(row(&r, "person").on && row(&r, "vehicle").on && !row(&r, "doorbell").on);
        let add = (row(&r, "doorbell").act.as_ref().unwrap())(Arg::Toggle(true));
        assert!(matches!(&add[..], [Op::Owner { owner, key, value }] if owner == ID && key == "notify.270" && value == &json!(["person", "vehicle", "doorbell"])));
        let off = (row(&r, "person").act.as_ref().unwrap())(Arg::Toggle(false));
        assert!(matches!(&off[..], [Op::Owner { value, .. }] if value == &json!(["vehicle"])));
        // a camera whose pick the owner never made has the defaults
        let mut w = world("ready");
        w.owners.get_mut(ID).unwrap().take(&json!({"ok": true, "version": 4, "schema": schema(), "effective": {}}));
        w.arg = PageArg::Json(json!({"camera": "271"}));
        let r = notify_rows(&w.ctx());
        assert!(row(&r, "person").on && row(&r, "doorbell").on && !row(&r, "motion").on);
        // a camera that is gone
        w.arg = PageArg::Json(json!({"camera": "999"}));
        assert!(notify_rows(&w.ctx())[0].kind == Kind::Empty);
    }

    #[test]
    fn the_lists_are_asked_again_when_the_link_or_the_schema_changes_and_the_cameras_topic_refreshes_the_page() {
        let mut w = world("ready");
        let first = answered(&w.ctx(), "owner:scrypted-cameras", &Value::Null);
        let asked: Vec<&str> = first.iter().filter_map(|o| if let Op::Send { path, .. } = o { path.rsplit('/').next() } else { None }).collect();
        assert_eq!(asked, ["cameras?limit=64", "kinds?limit=64"]);
        w.local.set("asked", json!("ready|1"));
        assert!(answered(&w.ctx(), "owner:scrypted-cameras", &Value::Null).is_empty());
        let first = answered(&w.ctx(), "topic:cameras", &Value::Null);
        assert!(matches!(&first[..], [Op::Set(..), Op::Refresh]));
        w.local.set("seen", json!("ready|270,271"));
        assert!(answered(&w.ctx(), "topic:cameras", &Value::Null).is_empty(), "a topic that says the same changes nothing");
    }
}
