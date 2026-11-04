//! Where Home's weather comes from, on the Home tab of Home & look: the helper's own source (by ZIP code) or a weather entity of Home
//! Assistant (`base.weather` `source`, `entity`, `zip`). The ZIP row is the built-in source's; the entity row is Home Assistant's, and
//! picks from the instance's weather entities by name (`GET /v1/weather/entities`, which the helper answers from the saved link).
//!
//! The helper takes Home Assistant as the source only with an entity chosen, so a choice of Home Assistant with none saved opens the
//! entity row (page state `wxha`) and the source is written with the entity once one is picked, in that order.

use crate::panels::ime::PURPOSE_DIGITS;
use crate::settings::ctx::Ctx;
use crate::settings::row::{ChoiceSpec, Op, Pick, Row, Arg};
use serde_json::{Value, json};
use std::rc::Rc;

const OWNER: &str = "base.weather";
const LIST: &str = "wxents";
const TAB: usize = 1;

/// The page's state: Home Assistant was chosen and no entity is saved yet; the list of entities was asked for.
const CHOSEN: &str = "wxha";
const ASKED: &str = "wxasked";
const BAD_ZIP: &str = "wxzipbad";

fn stored(c: &Ctx, key: &str) -> String {
    c.owner(OWNER).and_then(|o| o.text(key)).unwrap_or_default()
}

/// Home Assistant is the source in force, or has just been chosen.
fn homeassistant(c: &Ctx) -> bool {
    stored(c, "source") == "homeassistant" || c.local.flag(CHOSEN)
}

/// The page's state a fresh visit starts from.
pub fn opened() -> Vec<Op> {
    vec![Op::Set(CHOSEN.into(), json!(false)), Op::Set(ASKED.into(), json!(false))]
}

fn ask() -> Vec<Op> {
    vec![Op::Set(ASKED.into(), json!(true)), Op::Send { name: Some(LIST.into()), method: "GET", path: "/v1/weather/entities".into(), body: None }]
}

/// What the helper's answers make of the page: the list is asked for once the stored source is Home Assistant.
pub fn answered(c: &Ctx, name: &str) -> Vec<Op> {
    if name == format!("owner:{OWNER}") && stored(c, "source") == "homeassistant" && !c.local.flag(ASKED) { ask() } else { Vec::new() }
}

/// The entities as (id, name), or why there are none, in the helper's words.
fn entities(c: &Ctx) -> Result<Vec<(String, String)>, String> {
    let doc = c.docs.get(LIST);
    if doc.is_null() {
        return Err("Loading".into());
    }
    if doc.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(doc.pointer("/error/message").and_then(Value::as_str).unwrap_or("The TV did not answer").to_string());
    }
    let list: Vec<(String, String)> = doc
        .get("entities")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| Some((e.get("id")?.as_str()?.to_string(), e.get("name").and_then(Value::as_str).unwrap_or_default().to_string())))
        .collect();
    if list.is_empty() { Err("Home Assistant has no weather entity".into()) } else { Ok(list) }
}

pub fn rows(c: &Ctx) -> Vec<Row> {
    let known = c.owner(OWNER).is_some_and(|o| o.has("source") && o.has("entity"));
    let on_ha = homeassistant(c);
    let entity = stored(c, "entity");
    let tv = |r: Row| {
        let mut r = r.tab(TAB).enabled(known).admin();
        if !known {
            r.detail = "TV not answering".into();
        }
        r
    };

    let source = {
        let entity = entity.clone();
        tv(Row::seg("wxsource", &["Built-in", "Home Assistant"]).icon("sun").label("Weather source").help("Where Home's weather card, the weather view and the screensaver get the weather."))
            .index(Some(usize::from(on_ha)))
            .act(move |a| match a {
                Arg::Index(0) => vec![Op::Owner { owner: OWNER.into(), key: "source".into(), value: json!("builtin") }, Op::Set(CHOSEN.into(), json!(false))],
                Arg::Index(1) if entity.is_empty() => {
                    let mut ops = vec![Op::Set(CHOSEN.into(), json!(true))];
                    ops.extend(ask());
                    ops
                }
                Arg::Index(1) => vec![Op::Owner { owner: OWNER.into(), key: "source".into(), value: json!("homeassistant") }, Op::Set(CHOSEN.into(), json!(false))],
                _ => Vec::new(),
            })
    };

    let listed = entities(c);
    let name_of = |id: &str| listed.as_ref().ok().and_then(|l| l.iter().find(|(i, _)| i == id)).map(|(_, n)| n.clone());
    let mut picker = tv(Row::link("wxentity").icon("house-wifi").label("Weather entity").help("The weather entity of Home Assistant to use. Needs the Home Assistant link set up on this TV."))
        .visible(on_ha)
        .value(&if entity.is_empty() { "Choose".to_string() } else { name_of(&entity).unwrap_or_else(|| entity.clone()) });
    if known {
        picker.enabled = listed.is_ok();
        if let Err(words) = &listed {
            picker.detail = words.clone();
        }
    }
    if let Ok(list) = listed {
        let options: Vec<Pick> = list.iter().map(|(id, name)| Pick::new(id, if name.is_empty() { id } else { name })).collect();
        picker = picker.does(vec![Op::Pick(Rc::new(ChoiceSpec {
            title: "Weather entity".into(),
            options,
            current: entity.clone(),
            write: Box::new(move |id| {
                vec![
                    Op::Owner { owner: OWNER.into(), key: "entity".into(), value: json!(id) },
                    Op::Owner { owner: OWNER.into(), key: "source".into(), value: json!("homeassistant") },
                    Op::Set(CHOSEN.into(), json!(false)),
                ]
            }),
        }))]);
    }

    let zip = stored(c, "zip");
    let mut place = tv(Row::link("wxzip").icon("hash").label("ZIP code").help("Five digits, typed on the keyboard. The U.S. place the built-in weather is for."))
        .visible(!on_ha)
        .value(&zip);
    if known && c.local.flag(BAD_ZIP) {
        place.detail = "Needs 5 digits".into();
    }
    let place = place.does(vec![
        Op::Set(BAD_ZIP.into(), json!(false)),
        Op::Entry {
            prompt: "ZIP code".into(),
            text: zip,
            secret: false,
            purpose: PURPOSE_DIGITS,
            then: Rc::new(|t| {
                let z = t.trim().to_string();
                if z.len() == 5 && z.bytes().all(|b| b.is_ascii_digit()) {
                    vec![Op::Owner { owner: OWNER.into(), key: "zip".into(), value: json!(z) }]
                } else {
                    vec![Op::Set(BAD_ZIP.into(), json!(true))]
                }
            }),
        },
    ]);
    vec![source, picker, place]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::World;
    use crate::settings::pages::owner::fixtures::{doc, world_with};

    fn world(source: &str, entity: &str) -> World {
        world_with(OWNER, &doc(&[("source", "choice", json!(source)), ("entity", "text", json!(entity)), ("zip", "text", json!("30311")), ("units", "choice", json!("fahrenheit"))]))
    }

    fn row<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id}"))
    }

    fn owner_writes(ops: &[Op]) -> Vec<(String, Value)> {
        ops.iter().filter_map(|o| if let Op::Owner { key, value, .. } = o { Some((key.clone(), value.clone())) } else { None }).collect()
    }

    fn entities_doc(w: &mut World) {
        w.docs.set(LIST, json!({"ok": true, "entities": [{"id": "weather.home", "name": "Forecast Home"}, {"id": "weather.pirate_weather", "name": "Pirate Weather"}]}));
    }

    #[test]
    fn the_built_in_source_shows_the_zip_row_and_not_the_entity_row() {
        let w = world("builtin", "");
        let r = rows(&w.ctx());
        assert_eq!(row(&r, "wxsource").index, Some(0));
        assert!(row(&r, "wxzip").visible && !row(&r, "wxentity").visible);
        assert_eq!(row(&r, "wxzip").value, "30311");
        assert!(r.iter().all(|x| x.tab == TAB && x.admin));
        let ops = (row(&r, "wxzip").act.as_ref().unwrap())(Arg::Press);
        let Some(Op::Entry { then, purpose, .. }) = ops.iter().find(|o| matches!(o, Op::Entry { .. })) else { panic!("{ops:?}") };
        assert_eq!(*purpose, PURPOSE_DIGITS);
        assert_eq!(owner_writes(&then("10001".into())), vec![("zip".to_string(), json!("10001"))]);
        assert!(matches!(&then("1000".into())[..], [Op::Set(k, v)] if k == BAD_ZIP && v == &json!(true)), "five digits or nothing is written");
    }

    #[test]
    fn choosing_home_assistant_with_no_entity_opens_the_picker_and_writes_nothing_until_one_is_picked() {
        let mut w = world("builtin", "");
        let r = rows(&w.ctx());
        let ops = (row(&r, "wxsource").act.as_ref().unwrap())(Arg::Index(1));
        assert!(owner_writes(&ops).is_empty(), "the helper refuses the source without an entity");
        assert!(ops.iter().any(|o| matches!(o, Op::Send { path, .. } if path == "/v1/weather/entities")));
        w.local.set(CHOSEN, json!(true));
        entities_doc(&mut w);
        let r = rows(&w.ctx());
        assert_eq!(row(&r, "wxsource").index, Some(1));
        assert!(row(&r, "wxentity").visible && !row(&r, "wxzip").visible);
        assert_eq!(row(&r, "wxentity").value, "Choose");
        let ops = (row(&r, "wxentity").act.as_ref().unwrap())(Arg::Press);
        let [Op::Pick(spec)] = &ops[..] else { panic!("{ops:?}") };
        assert_eq!(spec.options.iter().map(|p| (p.value.as_str(), p.label.as_str())).collect::<Vec<_>>(), [("weather.home", "Forecast Home"), ("weather.pirate_weather", "Pirate Weather")]);
        // the entity goes first, then the source: each is valid on its own
        let written = (spec.write)("weather.pirate_weather");
        assert_eq!(owner_writes(&written), vec![("entity".to_string(), json!("weather.pirate_weather")), ("source".to_string(), json!("homeassistant"))]);
    }

    #[test]
    fn with_an_entity_saved_the_source_switches_in_one_write_and_back() {
        let mut w = world("builtin", "weather.pirate_weather");
        let r = rows(&w.ctx());
        assert_eq!(owner_writes(&(row(&r, "wxsource").act.as_ref().unwrap())(Arg::Index(1))), vec![("source".to_string(), json!("homeassistant"))]);
        w = world("homeassistant", "weather.pirate_weather");
        entities_doc(&mut w);
        let r = rows(&w.ctx());
        assert_eq!(row(&r, "wxsource").index, Some(1));
        assert_eq!(row(&r, "wxentity").value, "Pirate Weather", "the entity by its name");
        assert_eq!(owner_writes(&(row(&r, "wxsource").act.as_ref().unwrap())(Arg::Index(0))), vec![("source".to_string(), json!("builtin"))]);
    }

    #[test]
    fn the_entity_row_says_why_it_cannot_pick_in_the_helpers_words() {
        let mut w = world("homeassistant", "weather.pirate_weather");
        assert_eq!(row(&rows(&w.ctx()), "wxentity").detail, "Loading");
        w.docs.set(LIST, json!({"ok": false, "error": {"code": "no_link", "message": "Weather from Home Assistant needs the Home Assistant link"}}));
        let r = rows(&w.ctx());
        let e = row(&r, "wxentity");
        assert!(!e.enabled && e.detail.contains("Home Assistant link"));
        assert_eq!(e.value, "weather.pirate_weather", "the saved entity is still named");
        let unknown = World::new(crate::settings::ctx::fixtures::lg_with(&[], &[]));
        let r = rows(&unknown.ctx());
        assert!(row(&r, "wxsource").detail == "TV not answering" && !row(&r, "wxsource").enabled);
    }

    #[test]
    fn the_list_is_asked_for_once_when_the_stored_source_is_home_assistant() {
        let mut w = world("homeassistant", "weather.pirate_weather");
        assert!(matches!(&answered(&w.ctx(), "owner:base.weather")[..], [Op::Set(..), Op::Send { .. }]));
        w.local.set(ASKED, json!(true));
        assert!(answered(&w.ctx(), "owner:base.weather").is_empty());
        assert!(answered(&world("builtin", "").ctx(), "owner:base.weather").is_empty());
    }
}
