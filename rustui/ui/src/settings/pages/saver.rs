//! During the screensaver (Home & look › Screensaver): what each kind of pop-up does while the dim saver is up. The helper owns the rules
//! (`GET/POST /v1/saver/rules`, `helper/app/src/saver.rs`: one answer for each class of source, owner only, the plugins' sources
//! listed as they are installed); this page is one calm list of them: a glyph and the source's name, the answer as a small chip, a
//! picker for the other answers, and over the rows a miniature of the dim saver that shows what the focused row would put on it (the
//! option the picker is on, before it is chosen). No sentence: the glyphs of the answers are the key.

use crate::settings::ctx::{Ctx, ask};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, ChoiceSpec, Op, Pick, Row};
use crate::settings::view::{Figure, Legend};
use serde_json::{Value, json};
use std::rc::Rc;

pub fn page() -> Page {
    Page::new("duringsaver", "During the screensaver", "moon").parent("look").back("Home & look").hidden_from_search().reads(reads).rows(rows).figure(figure)
}

fn reads(_: &Ctx) -> Vec<Read> {
    vec![Read::Get { name: "saver", path: "/v1/saver/rules" }]
}

/// The answers in the order they are listed in: how loud each one is, from over the saver to hidden. The glyph and the long words are the
/// page's; a class may offer fewer of them (`choices`), and a new answer the helper adds is listed after these with the helper's words.
const ANSWERS: [(&str, &str, &str, &str); 4] = [
    ("over", "picture-in-picture", "Over", "Show over saver"),
    ("inside", "moon", "Inside", "In the saver area"),
    ("wake", "sun", "Wake", "Wake the screen"),
    ("hide", "eye-off", "Hide", "Hide"),
];

/// The groups the classes are listed in, with the classes of each in order; the plugins' sources (`plugin:<id>`) are the last group.
const GROUPS: [(&str, &[&str]); 3] = [("Cameras", &["camera.popup", "camera.still"]), ("Home", &["doorbell", "ha", "timer", "complication"]), ("Messages", &["ntfy", "toast"])];

fn doc<'a>(c: &'a Ctx) -> Option<&'a Value> {
    let d = c.docs.get("saver");
    d.get("classes").is_some().then_some(d)
}

fn answer_glyph<'a>(d: &'a Value, id: &str) -> &'a str {
    d.get("answers")
        .and_then(Value::as_array)
        .and_then(|l| l.iter().find(|a| a.get("id").and_then(Value::as_str) == Some(id)))
        .and_then(|a| a.get("glyph").and_then(Value::as_str))
        .or_else(|| ANSWERS.iter().find(|a| a.0 == id).map(|a| a.1))
        .unwrap_or("circle")
}

fn answer_short(d: &Value, id: &str) -> String {
    ANSWERS.iter().find(|a| a.0 == id).map(|a| a.2.to_string()).or_else(|| d.get("answers").and_then(Value::as_array).and_then(|l| l.iter().find(|a| a.get("id").and_then(Value::as_str) == Some(id))).and_then(|a| a.get("title").and_then(Value::as_str)).map(str::to_string)).unwrap_or_else(|| id.to_string())
}

fn answer_long(d: &Value, id: &str) -> String {
    ANSWERS.iter().find(|a| a.0 == id).map(|a| a.3.to_string()).unwrap_or_else(|| answer_short(d, id))
}

/// The class's answer now: the live topic's, which follows a change made elsewhere, else the one the document says.
fn choice_of(c: &Ctx, class: &Value) -> String {
    let id = class.get("id").and_then(Value::as_str).unwrap_or("");
    c.helper("saver").and_then(|t| t.pointer(&format!("/rules/{}", id.replace('~', "~0").replace('/', "~1")))).and_then(Value::as_str).or_else(|| class.get("choice").and_then(Value::as_str)).unwrap_or("hide").to_string()
}

fn class_row(c: &Ctx, d: &Value, class: &Value) -> Row {
    let id = class.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    let title = class.get("title").and_then(Value::as_str).unwrap_or(&id).to_string();
    let glyph = class.get("glyph").and_then(Value::as_str).filter(|g| !g.is_empty()).unwrap_or("bell").to_string();
    let choice = choice_of(c, class);
    let offered: Vec<String> = class.get("choices").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let spec = Rc::new(ChoiceSpec {
        title: title.clone(),
        options: offered.iter().map(|o| Pick { icon: answer_glyph(d, o).to_string(), ..Pick::new(o, &answer_long(d, o)) }).collect(),
        current: choice.clone(),
        write: {
            let id = id.clone();
            Box::new(move |v| vec![Op::Send { name: Some("saver".into()), method: "POST", path: "/v1/saver/rules".into(), body: Some(json!({"id": id, "choice": v})) }])
        },
    });
    let widest = ANSWERS.iter().map(|a| a.2).max_by_key(|w| w.chars().count()).unwrap_or("");
    Row::link(&format!("class:{id}")).icon(&glyph).label(&title).value(&answer_short(d, &choice)).value_icon(answer_glyph(d, &choice)).value_min(widest).chevron(false).preview(&glyph, &choice).admin().does(vec![Op::Pick(spec)])
}

fn rows(c: &Ctx) -> Vec<Row> {
    let Some(d) = doc(c) else {
        let words = if c.topics.helper_up { "Reading the rules" } else { "The helper is not answering" };
        return vec![Row::empty("Not available", words, "moon", 0)];
    };
    let classes: Vec<&Value> = d.get("classes").and_then(Value::as_array).map(|l| l.iter().collect()).unwrap_or_default();
    let by_id = |id: &str| classes.iter().copied().find(|k| k.get("id").and_then(Value::as_str) == Some(id));
    let mut rows = Vec::new();
    let mut listed: Vec<&str> = Vec::new();
    for (group, ids) in GROUPS {
        let members: Vec<&Value> = ids.iter().filter_map(|id| by_id(id)).collect();
        if members.is_empty() {
            continue;
        }
        rows.push(Row::head(group, 0));
        for k in members {
            listed.push(k.get("id").and_then(Value::as_str).unwrap_or(""));
            rows.push(class_row(c, d, k));
        }
    }
    // the plugins' sources, and any class the helper adds that this page does not know yet, are the last group
    let rest: Vec<&Value> = classes.iter().copied().filter(|k| !listed.contains(&k.get("id").and_then(Value::as_str).unwrap_or(""))).collect();
    if !rest.is_empty() {
        rows.push(Row::head("Plugins", 0));
        for k in rest {
            rows.push(class_row(c, d, k));
        }
    }
    let moved = classes.iter().any(|k| choice_of(c, k) != k.get("default").and_then(Value::as_str).unwrap_or(""));
    let mut reset = Row::action("reset", "Reset").icon("rotate-ccw").label("Reset all").danger().enabled(moved).admin();
    if !moved {
        reset.detail = "Nothing is changed".into();
    }
    rows.push(reset.does(ask("rotate-ccw", "Put every answer back?", "Each source returns to its first answer.", "Reset", "Cancel", true, vec![Op::Send { name: Some("saver".into()), method: "POST", path: "/v1/saver/rules".into(), body: Some(json!({"reset": true})) }])));
    let _ = Arg::Press;
    rows
}

/// The saver in miniature (its face as the theme has it) and the key to the answers' glyphs; the model lights the answer of the focused row.
fn figure(c: &Ctx) -> Option<Figure> {
    let d = doc(c)?;
    let style = c.theme_str("screensaver.style").unwrap_or("clock");
    let mut ids: Vec<String> = ANSWERS.iter().map(|a| a.0.to_string()).collect();
    for a in d.get("answers").and_then(Value::as_array).into_iter().flatten() {
        if let Some(id) = a.get("id").and_then(Value::as_str)
            && !ids.iter().any(|i| i == id)
        {
            ids.push(id.to_string());
        }
    }
    Some(Figure {
        face: style.to_string(),
        dim: c.theme_at("screensaver.dim").and_then(Value::as_bool) != Some(false),
        shows: None,
        legend: ids.iter().map(|id| Legend { id: id.clone(), icon: answer_glyph(d, id).to_string(), word: answer_long(d, id), lit: false }).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::World;
    use crate::settings::row::Kind;

    fn rules() -> Value {
        json!({"answers": [{"id": "over", "glyph": "picture-in-picture", "title": "Over"}, {"id": "inside", "glyph": "moon", "title": "Inside"}, {"id": "wake", "glyph": "sun", "title": "Wake"}, {"id": "hide", "glyph": "eye-off", "title": "Hide"}],
            "classes": [
                {"id": "doorbell", "title": "Doorbell", "glyph": "bell-ring", "choice": "wake", "default": "wake", "choices": ["over", "inside", "wake", "hide"]},
                {"id": "timer", "title": "Timers", "glyph": "timer", "choice": "wake", "default": "wake", "choices": ["over", "inside", "wake", "hide"]},
                {"id": "camera.popup", "title": "Camera video", "glyph": "video", "choice": "hide", "default": "hide", "choices": ["over", "wake", "hide"]},
                {"id": "camera.still", "title": "Camera events", "glyph": "camera", "choice": "inside", "default": "inside", "choices": ["over", "inside", "wake", "hide"]},
                {"id": "ha", "title": "Home Assistant", "glyph": "house-wifi", "choice": "inside", "default": "inside", "choices": ["over", "inside", "wake", "hide"]},
                {"id": "ntfy", "title": "ntfy", "glyph": "bell", "choice": "inside", "default": "inside", "choices": ["over", "inside", "wake", "hide"]},
                {"id": "toast", "title": "System notices", "glyph": "message-square-warning", "choice": "hide", "default": "hide", "choices": ["over", "inside", "wake", "hide"]},
                {"id": "complication", "title": "Saver items", "glyph": "sparkles", "choice": "inside", "default": "inside", "choices": ["inside", "hide"]},
                {"id": "plugin:chores", "title": "Chores", "glyph": "plug", "choice": "inside", "default": "inside", "choices": ["over", "inside", "wake", "hide"]}]})
    }

    fn world() -> World {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        w.docs.set("saver", rules());
        w.theme = json!({"screensaver": {"style": "clock"}});
        w
    }

    #[test]
    fn the_rows_are_one_calm_list_in_groups_with_a_chip_for_the_answer_and_the_plugins_last() {
        let w = world();
        let r = rows(&w.ctx());
        let heads: Vec<&str> = r.iter().filter(|x| x.kind == Kind::Head).map(|x| x.text.as_str()).collect();
        assert_eq!(heads, vec!["Cameras", "Home", "Messages", "Plugins"]);
        let ids: Vec<&str> = r.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "head:Cameras", "class:camera.popup", "class:camera.still", "head:Home", "class:doorbell", "class:ha", "class:timer", "class:complication", "head:Messages", "class:ntfy", "class:toast", "head:Plugins", "class:plugin:chores", "reset"
            ]
        );
        let bell = r.iter().find(|x| x.id == "class:doorbell").unwrap();
        assert_eq!((bell.label.as_str(), bell.icon.as_str(), bell.value.as_str(), bell.value_icon.as_str()), ("Doorbell", "bell-ring", "Wake", "sun"));
        assert!(!bell.chevron && bell.admin && bell.detail.is_empty() && bell.help.is_empty(), "a chip and a glyph, no sentence");
        let video = r.iter().find(|x| x.id == "class:camera.popup").unwrap();
        assert_eq!((video.value.as_str(), video.value_icon.as_str()), ("Hide", "eye-off"));
        assert!(!r.last().unwrap().enabled, "everything is as shipped: nothing to reset");
    }

    #[test]
    fn a_row_opens_the_picker_with_the_answers_the_class_offers_in_the_long_words_and_a_pick_posts_the_rule() {
        let w = world();
        let r = rows(&w.ctx());
        let video = r.iter().find(|x| x.id == "class:camera.popup").unwrap();
        let ops = (video.act.as_ref().unwrap())(Arg::Press);
        let [Op::Pick(spec)] = &ops[..] else { panic!("{ops:?}") };
        assert_eq!(spec.title, "Camera video");
        assert_eq!(spec.options.iter().map(|o| (o.value.as_str(), o.label.as_str(), o.icon.as_str())).collect::<Vec<_>>(), vec![("over", "Show over saver", "picture-in-picture"), ("wake", "Wake the screen", "sun"), ("hide", "Hide", "eye-off")], "no 'inside' for the live video");
        assert_eq!(spec.current, "hide");
        let sent = (spec.write)("over");
        assert!(matches!(&sent[..], [Op::Send { name: Some(n), method: "POST", path, body: Some(b) }] if n == "saver" && path == "/v1/saver/rules" && b == &json!({"id": "camera.popup", "choice": "over"})));
    }

    #[test]
    fn the_live_topic_wins_over_the_document_and_a_changed_rule_enables_the_reset() {
        let mut w = world();
        w.topics.set_helper("saver", json!({"rules": {"doorbell": "over", "plugin:chores": "hide"}}));
        let r = rows(&w.ctx());
        let bell = r.iter().find(|x| x.id == "class:doorbell").unwrap();
        assert_eq!((bell.value.as_str(), bell.value_icon.as_str()), ("Over", "picture-in-picture"));
        assert_eq!(r.iter().find(|x| x.id == "class:plugin:chores").unwrap().value, "Hide");
        let reset = r.last().unwrap();
        assert!(reset.enabled && reset.danger);
        let ops = (reset.act.as_ref().unwrap())(Arg::Press);
        let [Op::Ask(a)] = &ops[..] else { panic!() };
        assert!(matches!(&a.answers[1].then[..], [Op::Send { body: Some(b), .. }] if b == &json!({"reset": true})));
    }

    #[test]
    fn rows_carry_what_they_would_put_on_the_picture_and_the_picture_has_the_legend_and_the_savers_face() {
        let mut w = world();
        let r = rows(&w.ctx());
        let still = r.iter().find(|x| x.id == "class:camera.still").unwrap();
        assert_eq!(still.preview.as_ref().map(|p| (p.icon.as_str(), p.choice.as_str())), Some(("camera", "inside")));
        w.theme = json!({"screensaver": {"style": "art", "dim": false}});
        let f = figure(&w.ctx()).unwrap();
        assert_eq!((f.face.as_str(), f.dim), ("art", false));
        assert_eq!(f.legend.iter().map(|l| (l.id.as_str(), l.word.as_str())).collect::<Vec<_>>(), vec![("over", "Show over saver"), ("inside", "In the saver area"), ("wake", "Wake the screen"), ("hide", "Hide")]);
    }

    #[test]
    fn without_the_rules_the_page_says_so_and_has_no_picture() {
        let w = World::new(crate::host::sio::Sio::new(0));
        let r = rows(&w.ctx());
        assert_eq!((r.len(), r[0].kind), (1, Kind::Empty));
        assert!(figure(&w.ctx()).is_none());
    }
}
