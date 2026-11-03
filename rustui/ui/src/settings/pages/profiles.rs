//! Profiles: who the TV's people are (`/v1/profiles`, the helper's `profiles` topic: `helper/app/src/profiles.rs`, the design in
//! `docs/research/profiles-design.md`). One list of the people with their pictures, the owner's way to add one, and the two options of
//! turning the TV on; a person's own page has the name, the colour, the picture, the PIN, the text size and the way to remove them.
//! The helper enforces who may do what (403 `owner_only`); the rows are locked here too, with the owner's name, so nothing is offered
//! that would only be refused.

use crate::settings::ctx::{Ctx, ask};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Card, ChoiceSpec, Face, Op, PageArg, Pick, Row};
use serde_json::{Value, json};
use std::rc::Rc;

/// The profile's colours (`profiles.rs` `COLORS`): Rose Pine's accents, by the names a person would use.
const COLOURS: [(&str, &str); 8] = [
    ("#A79BF5", "Lavender"),
    ("#EB6F92", "Rose"),
    ("#F6C177", "Gold"),
    ("#9CCFD8", "Foam"),
    ("#3E8FB0", "Pine"),
    ("#EA9A97", "Blush"),
    ("#C4A7E7", "Iris"),
    ("#908CAA", "Stone"),
];

/// The text sizes a person can take (`type.scale` of the theme).
const SIZES: [(f64, &str); 3] = [(1.0, "100 %"), (1.5, "150 %"), (2.0, "200 %")];

pub fn page() -> Page {
    Page::new("profiles", "Profiles", "user").rail().rows(rows).reads(reads).summary(summary)
}

pub fn person() -> Page {
    Page::new("profile", "Profile", "user").parent("profiles").back("Profiles").hidden_from_search().rows(person_rows).reads(reads).title_of(person_title).answered(answered)
}

pub fn avatars() -> Page {
    Page::new("avatars", "Picture", "image").parent("profiles").back("Profile").hidden_from_search().rows(avatar_rows).reads(avatar_reads)
}

fn reads(_: &Ctx) -> Vec<Read> {
    Vec::new()
}

fn avatar_reads(_: &Ctx) -> Vec<Read> {
    vec![Read::Get { name: "gallery", path: "/v1/profiles/gallery" }]
}

/// The profiles topic, once the helper has said it is ready.
fn topic<'a>(c: &'a Ctx) -> Option<&'a Value> {
    c.helper("profiles").filter(|t| t.get("ready").and_then(Value::as_bool) != Some(false))
}

fn list<'a>(c: &'a Ctx) -> Vec<&'a Value> {
    topic(c).and_then(|t| t.get("profiles")).and_then(Value::as_array).map(|l| l.iter().collect()).unwrap_or_default()
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn face_of(p: &Value) -> Face {
    Face { file: p.get("avatarFile").and_then(Value::as_str).map(str::to_string), letter: text(p, "initial").to_string(), colour: text(p, "color").to_string() }
}

fn summary(c: &Ctx) -> String {
    list(c).iter().find(|p| p.get("active").and_then(Value::as_bool) == Some(true)).map(|p| text(p, "name").to_string()).unwrap_or_default()
}

fn rows(c: &Ctx) -> Vec<Row> {
    let mut rows = Vec::new();
    let Some(t) = topic(c) else {
        let words = if c.topics.helper_up { "Reading the profiles" } else { "The helper is not answering" };
        rows.push(Row::empty("Not available", words, "user", 0));
        return rows;
    };
    let who = c.who();
    let people = list(c);
    for p in &people {
        let id = text(p, "id").to_string();
        let active = p.get("active").and_then(Value::as_bool) == Some(true);
        let owner = p.get("owner").and_then(Value::as_bool) == Some(true);
        let locked = p.get("locked").and_then(Value::as_bool) == Some(true);
        let mut row = Row::link(&format!("p:{id}")).face(face_of(p)).label(text(p, "name")).selected(active);
        row.detail = [if owner { "Owner" } else { "" }, if locked { "PIN" } else { "" }].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" \u{b7} ");
        rows.push(row.does(vec![Op::Open { page: "profile".into(), arg: PageArg::Json(json!({"id": id})) }]));
    }
    let max = t.get("max").and_then(Value::as_u64).unwrap_or(6) as usize;
    let full = people.len() >= max;
    let colour = COLOURS[people.len() % COLOURS.len()].0;
    let mut add = Row::action("add", "Add").icon("plus").label("Add profile").enabled(!full).admin();
    if full {
        add.detail = format!("{max} profiles at most");
    }
    rows.push(add.does(vec![Op::Entry {
        prompt: "Name".into(),
        text: String::new(),
        secret: false,
        purpose: crate::panels::ime::PURPOSE_NAME,
        then: Rc::new(move |name| {
            let name = name.trim().to_string();
            if name.is_empty() {
                return Vec::new();
            }
            vec![Op::Send { name: None, method: "POST", path: "/v1/profiles".into(), body: Some(json!({"name": name, "color": colour, "avatar": {"kind": "initial"}})) }]
        }),
    }]));
    // when the TV turns on
    rows.push(Row::head("When the TV turns on", 0));
    let wake = t.get("pickerOnWake").and_then(Value::as_bool) == Some(true);
    rows.push(
        Row::toggle("pickerOnWake")
            .icon("user")
            .label("Ask who is watching")
            .on(wake)
            .admin()
            .act(|a| match a {
                Arg::Toggle(v) => vec![Op::Send { name: None, method: "POST", path: "/v1/profiles/options".into(), body: Some(json!({"pickerOnWake": v})) }],
                _ => Vec::new(),
            }),
    );
    let start = text(t, "startWith").to_string();
    let mut options = vec![Pick::new("last", "Last used")];
    options.extend(people.iter().map(|p| Pick::new(text(p, "id"), text(p, "name"))));
    let name = options.iter().find(|o| o.value == start).map_or("Last used", |o| o.label.as_str()).to_string();
    let current = if start.is_empty() { "last".to_string() } else { start };
    rows.push(Row::link("startWith").icon("power").label("Start with").value(&name).admin().does(vec![Op::Pick(Rc::new(ChoiceSpec {
        title: "Start with".into(),
        options,
        current,
        write: Box::new(|v| vec![Op::Send { name: None, method: "POST", path: "/v1/profiles/options".into(), body: Some(json!({"startWith": v})) }]),
    }))]));
    let _ = who;
    rows
}

/// The profile a person's page is about.
fn subject<'a>(c: &'a Ctx) -> Option<&'a Value> {
    let id = c.arg.str("id")?;
    list(c).into_iter().find(|p| text(p, "id") == id)
}

fn person_title(c: &Ctx) -> String {
    subject(c).map(|p| text(p, "name").to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| "Profile".into())
}

fn person_rows(c: &Ctx) -> Vec<Row> {
    let Some(p) = subject(c) else { return vec![Row::empty("No such profile", "It was removed.", "user", 0)] };
    let who = c.who();
    let id = text(p, "id").to_string();
    let active = p.get("active").and_then(Value::as_bool) == Some(true);
    let owner = p.get("owner").and_then(Value::as_bool) == Some(true);
    let locked = p.get("locked").and_then(Value::as_bool) == Some(true);
    // a person's name, colour and picture are their own or the owner's; the PIN is set by its owner or the owner
    let mine = who.can_admin || active;
    let path = format!("/v1/profiles/{id}");
    let mut rows = Vec::new();
    rows.push(face_row(p, &path, mine, &who.owner));
    let name_ops = {
        let path = path.clone();
        vec![Op::Entry {
            prompt: "Name".into(),
            text: text(p, "name").into(),
            secret: false,
            purpose: crate::panels::ime::PURPOSE_NAME,
            then: Rc::new(move |name| {
                let name = name.trim().to_string();
                if name.is_empty() { Vec::new() } else { vec![Op::Send { name: None, method: "POST", path: path.clone(), body: Some(json!({"name": name})) }] }
            }),
        }]
    };
    rows.push(Row::link("name").icon("type").label("Name").value(text(p, "name")).enabled(mine).does(name_ops));
    let colour = text(p, "color").to_string();
    let colour_name = COLOURS.iter().find(|(h, _)| h.eq_ignore_ascii_case(&colour)).map_or("Custom", |(_, n)| n).to_string();
    let colour_ops = {
        let path = path.clone();
        vec![Op::Pick(Rc::new(ChoiceSpec {
            title: "Colour".into(),
            options: COLOURS.iter().map(|(h, n)| Pick { swatch: (*h).into(), ..Pick::new(h, n) }).collect(),
            current: COLOURS.iter().find(|(h, _)| h.eq_ignore_ascii_case(&colour)).map(|(h, _)| (*h).to_string()).unwrap_or_default(),
            write: Box::new(move |v| vec![Op::Send { name: None, method: "POST", path: path.clone(), body: Some(json!({"color": v})) }]),
        }))]
    };
    let mut colour_row = Row::link("color").label("Colour").value(&colour_name).enabled(mine).does(colour_ops);
    colour_row.face = Some(Face { file: None, letter: String::new(), colour: colour.clone() });
    rows.push(colour_row);
    rows.push(Row::link("avatar").icon("image").label("Picture").enabled(mine).does(vec![Op::Open { page: "avatars".into(), arg: PageArg::Json(json!({"id": id})) }]));
    // the PIN
    let pin_path = format!("{path}/pin");
    let pin_ops = if locked {
        if who.can_admin {
            ask("lock", "Remove the PIN?", "", "Remove", "Keep", false, vec![Op::Send { name: None, method: "POST", path: pin_path.clone(), body: Some(json!({"clear": true})) }])
        } else {
            let pin_path = pin_path.clone();
            vec![Op::Entry {
                prompt: "Current PIN".into(),
                text: String::new(),
                secret: true,
                purpose: crate::panels::ime::PURPOSE_DIGITS,
                then: Rc::new(move |current| vec![Op::Send { name: None, method: "POST", path: pin_path.clone(), body: Some(json!({"clear": true, "current": current})) }]),
            }]
        }
    } else {
        let pin_path = pin_path.clone();
        vec![Op::Entry {
            prompt: "New PIN".into(),
            text: String::new(),
            secret: true,
            purpose: crate::panels::ime::PURPOSE_DIGITS,
            then: Rc::new(move |pin| {
                if pin.len() == 4 && pin.bytes().all(|b| b.is_ascii_digit()) {
                    vec![Op::Send { name: None, method: "POST", path: pin_path.clone(), body: Some(json!({"pin": pin})) }]
                } else {
                    Vec::new()
                }
            }),
        }]
    };
    rows.push(Row::link("pin").icon("lock").label("PIN").value(if locked { "On" } else { "Off" }).enabled(mine).does(pin_ops));
    // the text size belongs to the person using the TV
    if active {
        let scale = c.theme_num("type.scale", 1.0);
        let at = SIZES.iter().position(|(s, _)| (s - scale).abs() < 0.01);
        let labels: Vec<&str> = SIZES.iter().map(|(_, l)| *l).collect();
        let mut size = Row::seg("scale", &labels).icon("search").label("Text size").index(at);
        size.act = Some(Rc::new(|a| match a {
            Arg::Index(i) => vec![Op::Patch(json!({"type": {"scale": SIZES[i.min(SIZES.len() - 1)].0}}))],
            _ => Vec::new(),
        }));
        rows.push(size);
    } else {
        let name = text(p, "name").to_string();
        rows.push(
            Row::action("switch", "Switch")
                .icon("user")
                .label(&format!("Switch to {name}"))
                .does(vec![Op::Send { name: None, method: "POST", path: "/v1/profiles/switch".into(), body: Some(json!({"id": id})) }]),
        );
    }
    // removing a person is the owner's, never the owner's own
    if !owner {
        let name = text(p, "name").to_string();
        let remove = ask(
            "trash-2",
            &format!("Remove {name}?"),
            "Their layout, look and picture are deleted.",
            "Remove",
            "Keep",
            true,
            vec![Op::Send { name: None, method: "DELETE", path: path.clone(), body: None }, Op::Up],
        );
        rows.push(Row::action("remove", "Remove").icon("trash-2").label("Remove profile").danger().admin().does(remove));
    }
    rows
}

/// The person at the top of their page: their picture, large, with the owner's mark.
fn face_row(p: &Value, _path: &str, _mine: bool, _owner: &str) -> Row {
    let mut r = Row::info("who").face(face_of(p)).label(text(p, "name"));
    if p.get("owner").and_then(Value::as_bool) == Some(true) {
        r.detail = "Owner".into();
    }
    r
}

fn answered(_: &Ctx, _: &str, _: &Value) -> Vec<Op> {
    Vec::new()
}

fn avatar_rows(c: &Ctx) -> Vec<Row> {
    let Some(p) = subject(c) else { return vec![Row::empty("No such profile", "It was removed.", "user", 0)] };
    let id = text(p, "id").to_string();
    let path = format!("/v1/profiles/{id}");
    let gallery: Vec<&Value> = c.docs.get("gallery").get("gallery").and_then(Value::as_array).map(|l| l.iter().collect()).unwrap_or_default();
    let current = p.pointer("/avatar/id").and_then(Value::as_str).map(str::to_string);
    let kind = p.pointer("/avatar/kind").and_then(Value::as_str).unwrap_or("initial").to_string();
    let mut cards = vec![Card { title: "Initial".into(), icon: String::new(), sub: String::new(), face: Some(Face { file: None, letter: text(p, "initial").into(), colour: text(p, "color").into() }) }];
    let mut ids: Vec<Option<String>> = vec![None];
    for g in &gallery {
        let gid = text(g, "id").to_string();
        let set = text(g, "set");
        let n = gid.rsplit('-').next().unwrap_or("");
        let title = format!("{} {}", if set == "critters" { "Critter" } else { "Person" }, n.trim_start_matches('0'));
        cards.push(Card { title, icon: String::new(), sub: String::new(), face: Some(Face { file: g.get("file").and_then(Value::as_str).map(str::to_string), letter: String::new(), colour: text(p, "color").into() }) });
        ids.push(Some(gid));
    }
    let chosen = if kind == "gallery" { ids.iter().position(|i| *i == current) } else if kind == "initial" { Some(0) } else { None };
    let mut row = Row::cards("pick", cards).index(chosen).enabled(c.who().can_admin || p.get("active").and_then(Value::as_bool) == Some(true));
    row.cols = 4;
    row.act = Some(Rc::new(move |a| match a {
        Arg::Index(i) => {
            let avatar = match ids.get(i).cloned().flatten() {
                Some(g) => json!({"kind": "gallery", "id": g}),
                None => json!({"kind": "initial"}),
            };
            vec![Op::Send { name: None, method: "POST", path: path.clone(), body: Some(json!({"avatar": avatar})) }]
        }
        _ => Vec::new(),
    }));
    vec![row]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::World;
    use crate::settings::row::Kind;

    fn topic_doc(can_admin: bool, active: &str) -> Value {
        json!({"ready": true, "active": active, "owner": "nitin", "canAdmin": can_admin, "pickerOnWake": false, "startWith": "last", "max": 6, "profiles": [
            {"id": "nitin", "name": "Nitin", "initial": "N", "color": "#A79BF5", "avatar": {"kind": "initial"}, "avatarFile": null, "owner": true, "active": active == "nitin", "locked": false},
            {"id": "isabel", "name": "Isabel", "initial": "I", "color": "#EB6F92", "avatar": {"kind": "gallery", "id": "people-03"}, "avatarFile": "/x/people-03.png", "owner": false, "active": active == "isabel", "locked": true}]})
    }

    fn world(can_admin: bool, active: &str) -> World {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        w.topics.set_helper("profiles", topic_doc(can_admin, active));
        w
    }

    #[test]
    fn the_people_are_a_list_with_their_faces_the_active_one_marked_and_the_owner_can_add_one() {
        let w = world(true, "nitin");
        let r = rows(&w.ctx());
        assert_eq!(r[0].label, "Nitin");
        assert!(r[0].selected && r[0].face.is_some() && r[0].detail == "Owner");
        assert_eq!((r[1].label.as_str(), r[1].detail.as_str(), r[1].selected), ("Isabel", "PIN", false));
        assert_eq!(r[1].face.as_ref().unwrap().file.as_deref(), Some("/x/people-03.png"));
        let add = r.iter().find(|x| x.id == "add").unwrap();
        assert!(add.enabled && add.admin);
        let ops = (add.act.as_ref().unwrap())(Arg::Press);
        let [Op::Entry { prompt, then, .. }] = &ops[..] else { panic!("{ops:?}") };
        assert_eq!(prompt, "Name");
        let sent = then("  Ana ".into());
        let [Op::Send { path, body, .. }] = &sent[..] else { panic!() };
        assert_eq!(path, "/v1/profiles");
        assert_eq!(body.as_ref().unwrap(), &json!({"name": "Ana", "color": "#F6C177", "avatar": {"kind": "initial"}}), "the next colour of the eight");
        assert!(then("   ".into()).is_empty(), "a name of nothing is not sent");
    }

    #[test]
    fn the_options_of_turning_the_tv_on_are_the_owners() {
        let w = world(false, "isabel");
        let r = rows(&w.ctx());
        let wake = r.iter().find(|x| x.id == "pickerOnWake").unwrap();
        assert!(wake.admin && !wake.on);
        let ops = (wake.act.as_ref().unwrap())(Arg::Toggle(true));
        assert!(matches!(&ops[..], [Op::Send { path, body, .. }] if path == "/v1/profiles/options" && body.as_ref().unwrap() == &json!({"pickerOnWake": true})));
        let start = r.iter().find(|x| x.id == "startWith").unwrap();
        assert_eq!(start.value, "Last used");
        let ops = (start.act.as_ref().unwrap())(Arg::Press);
        let [Op::Pick(spec)] = &ops[..] else { panic!() };
        assert_eq!(spec.options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>(), vec!["Last used", "Nitin", "Isabel"]);
    }

    #[test]
    fn a_persons_page_has_their_own_rows_and_the_owner_alone_removes_them() {
        let mut w = world(true, "nitin");
        w.arg = PageArg::Json(json!({"id": "isabel"}));
        let r = person_rows(&w.ctx());
        assert_eq!(r.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["who", "name", "color", "avatar", "pin", "switch", "remove"]);
        assert_eq!(person_title(&w.ctx()), "Isabel");
        let sw = r.iter().find(|x| x.id == "switch").unwrap();
        assert_eq!(sw.label, "Switch to Isabel");
        let rm = r.iter().find(|x| x.id == "remove").unwrap();
        assert!(rm.danger && rm.admin);
        let ops = (rm.act.as_ref().unwrap())(Arg::Press);
        let [Op::Ask(a)] = &ops[..] else { panic!() };
        assert_eq!(a.title, "Remove Isabel?");
        assert!(a.answers[1].danger && a.answers[0].then.is_empty(), "the safe answer first, the doing one in the danger style");
        assert!(matches!(&a.answers[1].then[..], [Op::Send { method: "DELETE", path, .. }, Op::Up] if path == "/v1/profiles/isabel"));
        // the owner's own page has no way to remove it
        w.arg = PageArg::Json(json!({"id": "nitin"}));
        assert!(!person_rows(&w.ctx()).iter().any(|x| x.id == "remove"));
    }

    #[test]
    fn the_text_size_is_the_active_persons_and_a_pick_patches_the_theme() {
        let mut w = world(true, "nitin");
        w.arg = PageArg::Json(json!({"id": "nitin"}));
        w.theme = json!({"type": {"scale": 1.5}});
        let r = person_rows(&w.ctx());
        let size = r.iter().find(|x| x.id == "scale").unwrap();
        assert_eq!((size.kind, size.index), (Kind::Seg, Some(1)));
        let ops = (size.act.as_ref().unwrap())(Arg::Index(2));
        assert!(matches!(&ops[..], [Op::Patch(p)] if p == &json!({"type": {"scale": 2.0}})));
        w.arg = PageArg::Json(json!({"id": "isabel"}));
        assert!(!person_rows(&w.ctx()).iter().any(|x| x.id == "scale"), "someone else's size is theirs to set when they are using the TV");
    }

    #[test]
    fn a_pin_is_set_by_its_owner_or_the_owner_and_removing_it_asks_the_current_one_of_a_person() {
        let mut w = world(false, "isabel");
        w.arg = PageArg::Json(json!({"id": "isabel"}));
        let r = person_rows(&w.ctx());
        let pin = r.iter().find(|x| x.id == "pin").unwrap();
        assert!(pin.enabled && pin.value == "On");
        let ops = (pin.act.as_ref().unwrap())(Arg::Press);
        let [Op::Entry { prompt, secret, then, .. }] = &ops[..] else { panic!() };
        assert_eq!((prompt.as_str(), *secret), ("Current PIN", true));
        assert!(matches!(&then("1234".into())[..], [Op::Send { body, .. }] if body.as_ref().unwrap() == &json!({"clear": true, "current": "1234"})));
        // not the owner and not their own page: locked
        w.arg = PageArg::Json(json!({"id": "nitin"}));
        assert!(!person_rows(&w.ctx()).iter().find(|x| x.id == "pin").unwrap().enabled);
        // the owner clears another's after one question
        let mut w = world(true, "nitin");
        w.arg = PageArg::Json(json!({"id": "isabel"}));
        let ops = (person_rows(&w.ctx()).iter().find(|x| x.id == "pin").unwrap().act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&ops[..], [Op::Ask(_)]));
        // a new PIN is four digits
        let mut w = world(true, "nitin");
        w.arg = PageArg::Json(json!({"id": "nitin"}));
        let ops = (person_rows(&w.ctx()).iter().find(|x| x.id == "pin").unwrap().act.as_ref().unwrap())(Arg::Press);
        let [Op::Entry { then, .. }] = &ops[..] else { panic!() };
        assert!(then("12a4".into()).is_empty() && then("123".into()).is_empty());
        assert!(matches!(&then("4321".into())[..], [Op::Send { body, .. }] if body.as_ref().unwrap() == &json!({"pin": "4321"})));
    }

    #[test]
    fn the_picture_page_offers_the_initial_and_the_gallery_and_marks_the_one_in_force() {
        let mut w = world(true, "nitin");
        w.arg = PageArg::Json(json!({"id": "isabel"}));
        w.docs.set("gallery", json!({"ok": true, "gallery": [{"id": "people-01", "set": "people", "file": "/x/people-01.png"}, {"id": "people-03", "set": "people", "file": "/x/people-03.png"}, {"id": "critters-02", "set": "critters", "file": "/x/critters-02.png"}]}));
        let r = avatar_rows(&w.ctx());
        assert_eq!(r.len(), 1);
        let cards = &r[0];
        assert_eq!(cards.cards.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(), vec!["Initial", "Person 1", "Person 3", "Critter 2"]);
        assert_eq!(cards.index, Some(2), "Isabel has people-03");
        let ops = (cards.act.as_ref().unwrap())(Arg::Index(3));
        assert!(matches!(&ops[..], [Op::Send { path, body, .. }] if path == "/v1/profiles/isabel" && body.as_ref().unwrap() == &json!({"avatar": {"kind": "gallery", "id": "critters-02"}})));
        let ops = (cards.act.as_ref().unwrap())(Arg::Index(0));
        assert!(matches!(&ops[..], [Op::Send { body, .. }] if body.as_ref().unwrap() == &json!({"avatar": {"kind": "initial"}})));
    }

    #[test]
    fn without_the_topic_the_page_says_so() {
        let w = World::new(crate::host::sio::Sio::new(0));
        let r = rows(&w.ctx());
        assert_eq!((r.len(), r[0].kind), (1, Kind::Empty));
        assert_eq!(summary(&w.ctx()), "");
    }
}
