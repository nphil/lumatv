//! On Home, a page under Home Assistant (Settings › Home & look › Home Assistant › On Home): what Home Assistant shares with this TV, and which
//! of it Home shows on its Smart home card.
//!
//! **What is shared is decided in Home Assistant**, not here: the LumaShow integration's *LumaTV* entry › *Configure* picks entities by area,
//! device, type or name, and marks which the TV may control. The TV holds no address or token and cannot widen the list; it only chooses
//! among what was shared, and a chosen thing that is no longer shared is said so on its tile. Until the owner chooses, Home shows all of it.
//!
//! Two tabs. *On Home*: where the link stands, the tiles by kind in the order Home has them (Left and Right move one earlier or later among
//! its kind), a row to show everything shared again, and the way to put the card on Home. *Add*: the shared things (from the helper's
//! `GET /v1/onhome/shared`), searched by name and narrowed by room or kind, grouped by kind, each a switch for whether Home shows it.

use crate::home::grid;
use crate::settings::ctx::Ctx;
use crate::settings::page::Page;
use crate::settings::row::{Arg, ChoiceSpec, Op, PageArg, Pick, Row};
use serde_json::{Value, json};
use std::rc::Rc;

const TABS: [&str; 2] = ["On Home", "Add"];
/// Rows of the Add tab's list at a time.
const LIMIT: usize = 30;
const DOC: &str = "onhome:shared";

pub fn page() -> Page {
    Page::new("onhome", "On Home", "house-wifi").parent("look").back("Home Assistant").tabs(&TABS).rows(rows).summary(summary).chip(chip).opened(opened).answered(answered)
}

fn topic<'a>(c: &'a Ctx) -> &'a Value {
    c.helper("onhome").unwrap_or(&Value::Null)
}

/// Where the link stands in a phrase.
pub fn words(c: &Ctx) -> String {
    let t = topic(c);
    let text = |k: &str| t.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    match t.get("state").and_then(Value::as_str) {
        Some("connected") => text("words"),
        Some("empty") => text("words"),
        Some("waiting") => "Waiting for Home Assistant".into(),
        Some("off") => "Not switched on".into(),
        Some("unpaired") => "Not paired".into(),
        Some("problem") => "Can't listen".into(),
        _ => "Starting".into(),
    }
}

fn summary(c: &Ctx) -> String {
    words(c)
}

fn chip(c: &Ctx) -> Option<(String, &'static str)> {
    let tone = match topic(c).get("state").and_then(Value::as_str) {
        Some("connected") => "positive",
        Some("problem") => "danger",
        Some("waiting" | "empty") => "warning",
        _ => "info",
    };
    Some((words(c), tone))
}

fn encode(text: &str) -> String {
    text.bytes().map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// The ask for the Add tab's list, for the filters and the page it is on.
fn ask(q: &str, area: &str, group: &str, offset: usize) -> Op {
    let path = format!("/v1/onhome/shared?q={}&area={}&group={}&offset={offset}&limit={LIMIT}", encode(q), encode(area), encode(group));
    Op::Send { name: Some(DOC.into()), method: "GET", path, body: None }
}

fn opened(_: &Ctx) -> Vec<Op> {
    vec![Op::Set("q".into(), json!("")), Op::Set("area".into(), json!("")), Op::Set("group".into(), json!("")), Op::Set("offset".into(), json!(0)), ask("", "", "", 0)]
}

fn answered(_: &Ctx, name: &str, _: &Value) -> Vec<Op> {
    // the link came up or went, or the shared set changed: the list is asked again for the filters in force
    match name {
        "topic:onhome" => Vec::new(),
        _ => Vec::new(),
    }
}

fn filters(c: &Ctx) -> (String, String, String, usize) {
    (c.local.text("q"), c.local.text("area"), c.local.text("group"), c.local.get("offset").as_u64().unwrap_or(0) as usize)
}

fn tiles(c: &Ctx) -> Vec<Value> {
    topic(c).get("tiles").and_then(Value::as_array).cloned().unwrap_or_default()
}

fn post_pick(body: Value) -> Op {
    Op::Send { name: None, method: "POST", path: "/v1/onhome/picks".into(), body: Some(body) }
}

fn rows(c: &Ctx) -> Vec<Row> {
    let mut rows = Vec::new();
    let state = topic(c).get("state").and_then(Value::as_str).unwrap_or("");
    let t = topic(c);
    let detail = t.get("detail").and_then(Value::as_str).unwrap_or("");
    let mut link = Row::info("link").tab(0).icon("house-wifi").label("Home Assistant").help("The LumaShow integration in Home Assistant shares things with the TV and decides which the TV may control. In Home Assistant: Settings, Devices & services, LumaShow, LumaTV, Configure.").value(&words(c));
    link.detail = detail.to_owned();
    rows.push(link);
    let have = tiles(c);
    if have.is_empty() {
        let (title, line) = match state {
            "off" | "unpaired" | "problem" | "waiting" => (words(c), detail.to_owned()),
            "empty" if t.get("shared").and_then(Value::as_u64).unwrap_or(0) > 0 => ("Nothing on Home".into(), "Add things in the next tab.".into()),
            _ => ("Nothing shared yet".into(), "Share some with the TV in Home Assistant: Settings, Devices & services, LumaShow, LumaTV, Configure.".into()),
        };
        rows.push(Row::empty(&title, &line, "house-wifi", 0));
    }
    let mut last = "";
    for tile in &have {
        let group = tile.get("group").and_then(Value::as_str).unwrap_or("");
        if group != last {
            last = group;
            let title = crate::home::onhome::group_title(group);
            rows.push(Row::head(title, 0));
        }
        let id = tile.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
        let text = |k: &str| tile.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        let mut r = Row::nudge(&format!("t:{id}")).tab(0).icon(&text("icon")).label(&text("name")).value(&text("words")).admin();
        let area = text("area");
        r.detail = if area.is_empty() { String::new() } else { area };
        let up = id.clone();
        r = r.act(move |a| match a {
            Arg::Nudge(d) => vec![post_pick(json!({ if d < 0 { "up" } else { "down" }: up }))],
            _ => Vec::new(),
        });
        rows.push(r);
    }
    if t.get("picked").is_some_and(|p| !p.is_null()) {
        rows.push(Row::action("all", "Show all").tab(0).icon("rotate-ccw").label("Show everything shared").help("Home shows every shared thing again, in Home Assistant's order").admin().does(vec![post_pick(json!({"all": true}))]));
    }
    let on_home = grid::entries(c.layout).iter().any(|e| e.kind == "onhome" && !e.hidden);
    rows.push(Row::link("card").tab(0).icon("layout-grid").label("Smart home card").value(if on_home { "On Home" } else { "Not on Home" }).does(vec![Op::Open { page: "widgets".into(), arg: PageArg::None }]));
    add_rows(c, &mut rows, &have);
    rows
}

fn add_rows(c: &Ctx, rows: &mut Vec<Row>, have: &[Value]) {
    let (q, area, group, offset) = filters(c);
    let doc = c.docs.get(DOC);
    let state = doc.get("state").and_then(Value::as_str).unwrap_or("");
    // search
    let (aarea, agroup) = (area.clone(), group.clone());
    rows.push(
        Row::link("search").tab(1).icon("search").label("Search").value(if q.is_empty() { "All" } else { &q }).does(vec![Op::Entry {
            prompt: "Search shared things".into(),
            text: q.clone(),
            secret: false,
            purpose: crate::panels::ime::PURPOSE_NORMAL,
            then: Rc::new(move |t| {
                let t = t.trim().to_owned();
                vec![Op::Set("q".into(), json!(t)), Op::Set("offset".into(), json!(0)), ask(&t, &aarea, &agroup, 0)]
            }),
        }]),
    );
    // room
    let areas: Vec<Pick> = std::iter::once(Pick::new("", "All rooms"))
        .chain(doc.get("areas").and_then(Value::as_array).into_iter().flatten().map(|a| Pick::new(a["id"].as_str().unwrap_or(""), &format!("{} ({})", a["name"].as_str().unwrap_or(""), a["count"].as_u64().unwrap_or(0)))))
        .collect();
    let area_name = doc.get("areas").and_then(Value::as_array).into_iter().flatten().find(|a| a["id"].as_str() == Some(area.as_str())).and_then(|a| a["name"].as_str()).unwrap_or("All rooms").to_owned();
    let (rq, rgroup, rarea) = (q.clone(), group.clone(), area.clone());
    rows.push(Row::link("area").tab(1).icon("house").label("Room").value(&area_name).does(vec![Op::Pick(Rc::new(ChoiceSpec {
        title: "Room".into(),
        options: areas,
        current: rarea,
        write: Box::new(move |to| vec![Op::Set("area".into(), json!(to)), Op::Set("offset".into(), json!(0)), ask(&rq, to, &rgroup, 0)]),
    }))]));
    // kind
    let groups: Vec<Pick> = std::iter::once(Pick::new("", "All kinds"))
        .chain(doc.get("groups").and_then(Value::as_array).into_iter().flatten().map(|g| Pick::new(g["id"].as_str().unwrap_or(""), &format!("{} ({})", g["title"].as_str().unwrap_or(""), g["count"].as_u64().unwrap_or(0)))))
        .collect();
    let group_name = doc.get("groups").and_then(Value::as_array).into_iter().flatten().find(|g| g["id"].as_str() == Some(group.as_str())).and_then(|g| g["title"].as_str()).unwrap_or("All kinds").to_owned();
    let (kq, karea, kgroup) = (q.clone(), area.clone(), group.clone());
    rows.push(Row::link("kind").tab(1).icon("layout-grid").label("Kind").value(&group_name).does(vec![Op::Pick(Rc::new(ChoiceSpec {
        title: "Kind".into(),
        options: groups,
        current: kgroup,
        write: Box::new(move |to| vec![Op::Set("group".into(), json!(to)), Op::Set("offset".into(), json!(0)), ask(&kq, &karea, to, 0)]),
    }))]));
    // what is shared
    let list = doc.get("rows").and_then(Value::as_array).cloned().unwrap_or_default();
    let shown: Vec<&str> = have.iter().filter_map(|t| t["id"].as_str()).collect();
    let picked = topic(c).get("picked").is_some_and(|p| !p.is_null());
    match state {
        "ready" if list.is_empty() => rows.push(Row::empty("Nothing matches", "Try another word, room or kind.", "search", 1)),
        "ready" => {}
        "unavailable" => rows.push(Row::empty("Not connected", "Home Assistant's LumaShow integration has not connected to the TV.", "house-wifi", 1)),
        _ => rows.push(Row::empty("Loading\u{2026}", "", "house-wifi", 1)),
    }
    let mut last = "";
    for r in &list {
        let text = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        let g = r.get("group").and_then(Value::as_str).unwrap_or("");
        if g != last {
            last = g;
            rows.push(Row::head(crate::home::onhome::group_title(g), 1));
        }
        let id = text("id");
        // with no list of the owner's everything shared is shown; the tiles are what Home draws, so a switch says what Home does
        let on = if picked { shown.contains(&id.as_str()) } else { r.get("shown").and_then(Value::as_bool).unwrap_or(false) };
        let mut row = Row::toggle(&format!("r:{id}")).tab(1).icon(&text("icon")).label(&text("name")).on(on).admin();
        let (area, words) = (text("area"), text("words"));
        row.detail = [area.as_str(), words.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" \u{b7} ");
        let pid = id.clone();
        row = row.act(move |a| match a {
            Arg::Toggle(v) => vec![post_pick(json!({ if v { "show" } else { "hide" }: pid }))],
            _ => Vec::new(),
        });
        rows.push(row);
    }
    let total = doc.get("total").and_then(Value::as_u64).unwrap_or(0) as usize;
    let page = |to: usize| vec![Op::Set("offset".into(), json!(to)), ask(&q, &area, &group, to)];
    if offset > 0 {
        rows.push(Row::action("prev", "Previous").tab(1).icon("chevron-left").label("Previous").does(page(offset.saturating_sub(LIMIT))));
    }
    if doc.get("more").and_then(Value::as_bool) == Some(true) {
        rows.push(Row::action("next", "Next").tab(1).icon("chevron-right").label(&format!("Next \u{b7} {} of {total}", offset + LIMIT.min(total.saturating_sub(offset)))).does(page(offset + LIMIT)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::World;
    use crate::settings::row::Kind;

    fn tile(id: &str, group: &str, name: &str, words: &str) -> Value {
        json!({"id": id, "group": group, "name": name, "words": words, "icon": "lightbulb", "area": "Kitchen", "available": true})
    }

    fn world(topic: Value) -> World {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        w.topics.helper_up = true;
        w.topics.set_helper("onhome", topic);
        w.layout = grid::tidy_layout(&grid::shipped());
        w
    }

    fn row<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id}: {:?}", rows.iter().map(|r| &r.id).collect::<Vec<_>>()))
    }

    fn connected() -> Value {
        json!({"state": "connected", "words": "2 things", "detail": "", "shared": 5, "picked": null,
               "tiles": [tile("light.a", "lights", "Lamp", "On"), tile("light.b", "lights", "Desk", "Off"), tile("sensor.t", "sensors", "Temperature", "21\u{b0}C")]})
    }

    #[test]
    fn the_tiles_are_listed_by_kind_and_move_among_their_kind() {
        let w = world(connected());
        let r = rows(&w.ctx());
        let heads: Vec<&str> = r.iter().filter(|x| x.tab == 0 && x.kind == Kind::Head).map(|x| x.text.as_str()).collect();
        assert_eq!(heads, ["Lights", "Sensors"]);
        let a = row(&r, "t:light.a");
        assert_eq!((a.kind, a.label.as_str(), a.value.as_str(), a.detail.as_str()), (Kind::Nudge, "Lamp", "On", "Kitchen"));
        let down = (a.act.as_ref().unwrap())(Arg::Nudge(1));
        assert!(matches!(&down[..], [Op::Send { path, body: Some(b), .. }] if path == "/v1/onhome/picks" && b == &json!({"down": "light.a"})));
        let up = (a.act.as_ref().unwrap())(Arg::Nudge(-1));
        assert!(matches!(&up[..], [Op::Send { body: Some(b), .. }] if b == &json!({"up": "light.a"})));
        // everything is shown until the owner chooses: no row to put it back
        assert!(r.iter().all(|x| x.id != "all"));
        let mut chosen = connected();
        chosen["picked"] = json!(["light.a"]);
        let w = world(chosen);
        let all = (row(&rows(&w.ctx()), "all").act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&all[..], [Op::Send { body: Some(b), .. }] if b == &json!({"all": true})));
    }

    #[test]
    fn with_no_tile_the_page_says_why_in_the_helpers_words() {
        for (state, title) in [("off", "Not switched on"), ("unpaired", "Not paired"), ("waiting", "Waiting for Home Assistant")] {
            let w = world(json!({"state": state, "detail": "why", "tiles": []}));
            let r = rows(&w.ctx());
            assert!(r.iter().any(|x| x.kind == Kind::Empty && x.id == format!("empty:{title}")), "{state}: {:?}", r.iter().map(|x| (&x.id, &x.text)).collect::<Vec<_>>());
        }
        let w = world(json!({"state": "empty", "shared": 0, "tiles": []}));
        assert!(rows(&w.ctx()).iter().any(|x| x.id == "empty:Nothing shared yet"));
        assert_eq!(chip(&w.ctx()).map(|c| c.1), Some("warning"));
    }

    #[test]
    fn the_add_tab_lists_what_is_shared_by_kind_and_a_switch_shows_or_hides_it() {
        let mut w = world(connected());
        w.docs.set(DOC, json!({"state": "ready", "total": 3, "more": false,
            "areas": [{"id": "kitchen", "name": "Kitchen", "count": 2}], "groups": [{"id": "lights", "title": "Lights", "count": 2}, {"id": "sensors", "title": "Sensors", "count": 1}],
            "rows": [{"id": "light.a", "name": "Lamp", "area": "Kitchen", "group": "lights", "words": "On", "icon": "lightbulb", "shown": true},
                     {"id": "light.c", "name": "Hall", "area": null, "group": "lights", "words": "Off", "icon": "lightbulb", "shown": true},
                     {"id": "sensor.t", "name": "Temperature", "area": "Kitchen", "group": "sensors", "words": "21\u{b0}C", "icon": "activity", "shown": true}]}));
        let r = rows(&w.ctx());
        let lamp = row(&r, "r:light.a");
        assert_eq!((lamp.kind, lamp.tab, lamp.on, lamp.detail.as_str()), (Kind::Toggle, 1, true, "Kitchen \u{b7} On"));
        let off = (lamp.act.as_ref().unwrap())(Arg::Toggle(false));
        assert!(matches!(&off[..], [Op::Send { body: Some(b), .. }] if b == &json!({"hide": "light.a"})));
        let on = (lamp.act.as_ref().unwrap())(Arg::Toggle(true));
        assert!(matches!(&on[..], [Op::Send { body: Some(b), .. }] if b == &json!({"show": "light.a"})));
        let heads: Vec<&str> = r.iter().filter(|x| x.tab == 1 && x.kind == Kind::Head).map(|x| x.text.as_str()).collect();
        assert_eq!(heads, ["Lights", "Sensors"]);
        assert_eq!((row(&r, "area").value.as_str(), row(&r, "kind").value.as_str()), ("All rooms", "All kinds"));
        // with a list of the owner's, a switch says what Home shows (the tiles), not what the picker was last told
        let mut chosen = connected();
        chosen["picked"] = json!(["light.a", "light.b", "sensor.t"]);
        let mut w = world(chosen);
        w.docs.set(DOC, json!({"state": "ready", "total": 1, "more": false, "areas": [], "groups": [], "rows": [{"id": "light.c", "name": "Hall", "group": "lights", "words": "Off", "icon": "lightbulb", "shown": true}]}));
        assert!(!row(&rows(&w.ctx()), "r:light.c").on);
    }

    #[test]
    fn the_filters_ask_the_helper_again_and_the_list_pages() {
        let mut w = world(connected());
        w.local.set("q", json!("lamp room"));
        w.local.set("area", json!("kitchen"));
        w.docs.set(DOC, json!({"state": "ready", "total": 70, "more": true, "areas": [{"id": "kitchen", "name": "Kitchen", "count": 70}], "groups": [], "rows": []}));
        let r = rows(&w.ctx());
        assert_eq!((row(&r, "search").value.as_str(), row(&r, "area").value.as_str()), ("lamp room", "Kitchen"));
        let next = row(&r, "next").act.as_ref().map(|f| f(Arg::Press)).unwrap();
        assert!(matches!(&next[..], [Op::Set(k, v), Op::Send { path, .. }] if k == "offset" && v == &json!(30) && path == "/v1/onhome/shared?q=lamp%20room&area=kitchen&group=&offset=30&limit=30"), "{next:?}");
        assert!(r.iter().all(|x| x.id != "prev"));
        w.local.set("offset", json!(30));
        assert!(rows(&w.ctx()).iter().any(|x| x.id == "prev"));
        // a pick of a room writes it and asks again from the first page
        let Some(Op::Pick(spec)) = row(&r, "area").act.as_ref().map(|f| f(Arg::Press)).unwrap().into_iter().next() else { panic!() };
        assert_eq!(spec.options[0].label, "All rooms");
        let ops = (spec.write)("");
        assert!(matches!(&ops[..], [Op::Set(..), Op::Set(..), Op::Send { path, .. }] if path.contains("area=&") && path.contains("q=lamp%20room")));
    }

    #[test]
    fn the_page_says_whether_the_card_is_on_home_and_opens_the_widgets_page() {
        let w = world(connected());
        let r = rows(&w.ctx());
        assert_eq!(row(&r, "card").value, "Not on Home");
        let ops = (row(&r, "card").act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&ops[..], [Op::Open { page, .. }] if page == "widgets"));
    }
}
