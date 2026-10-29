//! What the saver says besides the time: the date, the weather, and the rows of its small areas, the slots (complications: a glyph, a label
//! and a value) and the notices (what was kept out of the way while it is up). Pure: the surface draws what this returns.
//!
//! The owner's rules (`/v1/saver/rules`, the `rules` of the helper's `saver` topic) decide what is let in: only a class whose answer is
//! `inside` is a row here (`over` is the cards' and the pop-up's, drawn above the saver; `wake` and `hide` are not for the saver to show). A
//! class the topic does not name has its shipped answer. The weather is the helper's `weather` topic read as the Home card reads it: fresh
//! or nothing, because the saver has no room to say how old a reading is and must not show a stale one as now.

use crate::host::clock::Civil;
use crate::system::plugins::Hub;
use lumatv_saver::Tone;
use lumatv_saver::rules::{self, Choice};
use serde_json::Value;

/// Slots and notices at most (the room decides how many of them fit).
pub const MAX_SLOTS: usize = 4;
pub const MAX_NOTICES: usize = 3;
/// A plugin's note is a recent notice for this long after it arrived.
pub const RECENT_MS: u64 = 3 * 3_600_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Weather {
    /// A Lucide glyph of the module for the sky, or "" when it carries none for it (the words say it).
    pub glyph: &'static str,
    pub temp: String,
    pub cond: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Slot,
    Notice,
}

/// One row of the saver's small areas.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub kind: Kind,
    pub glyph: String,
    /// A slot's label, or what a notice is from.
    pub title: String,
    /// A slot's value, or a notice's words.
    pub text: String,
    pub tone: Tone,
    /// How long ago a notice came, in words ("3 min").
    pub age: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Glance {
    pub date: String,
    pub weather: Option<Weather>,
    pub rows: Vec<Row>,
}

/// The answer of `class` in the topic's rules, else the class's shipped one.
pub fn choice(rules_doc: &Value, class: &str) -> Choice {
    rules_doc.get(class).and_then(Value::as_str).and_then(Choice::parse).unwrap_or_else(|| rules::default_choice(class))
}

/// How long ago, in the fewest words: "now", "7 min", "2 h".
pub fn age_words(ms: u64) -> String {
    let min = ms / 60_000;
    if min < 1 {
        "now".into()
    } else if min < 90 {
        format!("{min} min")
    } else {
        format!("{} h", min / 60)
    }
}

/// The weather row from the helper's `weather` topic (`{state, doc: {display: {temp, cond, nowKind}}}`): none unless the reading is fresh.
pub fn weather_of(topic: &Value) -> Option<Weather> {
    if topic.get("state").and_then(Value::as_str) != Some("fresh") {
        return None;
    }
    let display = topic.pointer("/doc/display")?;
    let text = |k: &str| display.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let temp = text("temp");
    if temp.is_empty() {
        return None;
    }
    // the module carries a sun and a moon; the other skies are told by their words
    let glyph = match text("nowKind").as_str() {
        "sun" => "sun",
        "moon" | "partly-night" => "moon",
        _ => "",
    };
    Some(Weather { glyph, temp, cond: text("cond") })
}

fn tone_of(level: &str) -> Tone {
    match level {
        "critical" => Tone::Danger,
        "warning" => Tone::Warning,
        _ => Tone::Plain,
    }
}

/// The slots: what the board holds and the connected plugins' cards, most urgent first, in the classes the owner lets in.
fn slots(hub: &Hub, rules_doc: &Value, now: u64) -> Vec<Row> {
    hub.complications(now)
        .into_iter()
        .filter(|c| {
            let class = if hub.is_plugin(&c.source) { rules::plugin_source_class(&c.source) } else { "complication".to_string() };
            choice(rules_doc, &class) == Choice::Inside
        })
        .take(MAX_SLOTS)
        .map(|c| Row { kind: Kind::Slot, glyph: c.glyph, title: c.text, text: c.value, tone: c.tone, age: String::new() })
        .collect()
}

/// The notices, newest first: the helper's lines (what a camera saw) and the plugins' notes whose class is `inside`, recent and not private.
fn notices(hub: &Hub, rules_doc: &Value, now: u64) -> Vec<Row> {
    let mut found: Vec<(u64, Row)> = Vec::new();
    for l in hub.saver_doc().get("lines").and_then(Value::as_array).into_iter().flatten() {
        let text = |k: &str| l.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let at = l.get("at").and_then(Value::as_u64).unwrap_or(0);
        if l.get("expiresAt").and_then(Value::as_u64).unwrap_or(0) <= now || choice(rules_doc, &text("class")) != Choice::Inside {
            continue;
        }
        let glyph = if text("glyph").is_empty() { "bell".to_string() } else { text("glyph") };
        found
            .push((at, Row { kind: Kind::Notice, glyph, title: text("title"), text: text("text"), tone: Tone::Plain, age: age_words(now.saturating_sub(at)) }));
    }
    for n in hub.notes() {
        let class = rules::note_class(&n.plugin_id, rules::card_class(&n.kind, &n.channel, &n.channel_icon));
        let at = (n.received_at * 1000.0) as u64;
        let over = n.expires_at > 0.0 && n.expires_at * 1000.0 <= now as f64;
        if n.importance == "low" || over || now.saturating_sub(at) > RECENT_MS || choice(rules_doc, &class) != Choice::Inside {
            continue;
        }
        let secret = n.privacy == "secret";
        let from = if n.channel_title.is_empty() { n.plugin.clone() } else { n.channel_title.clone() };
        let (title, text) = if secret { (from, String::new()) } else { (if n.title.is_empty() { from } else { n.title.clone() }, n.body.clone()) };
        if title.is_empty() && text.is_empty() {
            continue;
        }
        found.push((
            at,
            Row { kind: Kind::Notice, glyph: Hub::icon(&n.channel_icon), title, text, tone: tone_of(&n.level), age: age_words(now.saturating_sub(at)) },
        ));
    }
    found.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    found.into_iter().take(MAX_NOTICES).map(|(_, r)| r).collect()
}

/// What the saver says at `now` (wall-clock ms) at `civil` time: the date always; the rest when `glance` is on (`screensaver.glance`).
pub fn build(hub: &Hub, weather_topic: &Value, civil: &Civil, now: u64, glance: bool) -> Glance {
    let mut g = Glance { date: civil.date_line(), ..Glance::default() };
    if !glance {
        return g;
    }
    let rules_doc = hub.saver_doc().get("rules").cloned().unwrap_or(Value::Null);
    g.weather = weather_of(weather_topic);
    g.rows = slots(hub, &rules_doc, now);
    g.rows.extend(notices(hub, &rules_doc, now));
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::clock::civil_at;
    use crate::kit::screen::Input;
    use serde_json::json;

    const NOW: u64 = 1_793_241_312_250;

    fn helper(hub: &mut Hub, topic: &str, data: Value) {
        hub.update(&Input::Helper { topic: topic.into(), data }, NOW);
    }

    fn hub_with(saver: Value) -> Hub {
        let mut hub = Hub::new();
        helper(
            &mut hub,
            "plugins",
            json!({"plugins": [{"id": "photos", "name": "Photos", "state": "connected"}, {"id": "home-assistant", "name": "Home Assistant", "state": "connected"}]}),
        );
        helper(&mut hub, "saver", saver);
        hub
    }

    fn note(id: &str, ago_s: f64, extra: Value) -> Value {
        let mut it = json!({"id": id, "revision": 1, "channel": "ha", "channelTitle": "Home Assistant", "channelIcon": "base.icons/home", "receivedAt": NOW as f64 / 1000.0 - ago_s,
                            "post": {"title": "Low battery", "body": "Door sensor is at 5 %"}});
        if let (Some(m), Some(e)) = (it.as_object_mut(), extra.as_object()) {
            m.extend(e.clone());
        }
        it
    }

    #[test]
    fn the_weather_is_a_row_only_when_it_is_fresh_and_has_a_temperature() {
        let fresh = json!({"state": "fresh", "doc": {"display": {"temp": "61\u{b0}", "cond": "Clear", "nowKind": "moon"}}});
        assert_eq!(weather_of(&fresh), Some(Weather { glyph: "moon", temp: "61\u{b0}".into(), cond: "Clear".into() }));
        assert_eq!(weather_of(&json!({"state": "stale", "doc": fresh["doc"].clone()})), None, "a stale reading is not shown as now");
        assert_eq!(weather_of(&json!({"state": "none", "doc": null})), None);
        assert_eq!(weather_of(&Value::Null), None, "no topic, no row");
        assert_eq!(weather_of(&json!({"state": "fresh", "doc": {"display": {"temp": "", "cond": "Clear"}}})), None);
        let cloudy = json!({"state": "fresh", "doc": {"display": {"temp": "58\u{b0}", "cond": "Cloudy", "nowKind": "cloud"}}});
        assert_eq!(weather_of(&cloudy).unwrap().glyph, "", "the words say the sky when the module has no glyph for it");
    }

    #[test]
    fn ages_are_the_fewest_words() {
        assert_eq!(age_words(10_000), "now");
        assert_eq!(age_words(7 * 60_000), "7 min");
        assert_eq!(age_words(89 * 60_000), "89 min");
        assert_eq!(age_words(3 * 3_600_000 + 10 * 60_000), "3 h");
    }

    #[test]
    fn the_glance_off_is_the_date_alone() {
        let hub = hub_with(json!({"items": [], "rules": {}, "lines": []}));
        let g = build(&hub, &Value::Null, &civil_at(NOW, 0), NOW, false);
        assert_eq!(g, Glance { date: "Thursday, October 29".into(), weather: None, rows: Vec::new() });
    }

    #[test]
    fn a_slot_shows_when_its_class_is_inside_and_a_plugins_own_answer_hides_it() {
        let item = json!({"id": "door", "source": "api", "glyph": "lock", "text": "Front door", "value": "Locked", "priority": 2, "expiresAt": NOW + 60_000, "tone": "plain", "updatedAt": NOW});
        let mut hub = hub_with(json!({"items": [item], "rules": {}, "lines": []}));
        let g = build(&hub, &Value::Null, &civil_at(NOW, 0), NOW, true);
        assert_eq!(g.rows.len(), 1);
        assert_eq!((g.rows[0].glyph.as_str(), g.rows[0].title.as_str(), g.rows[0].text.as_str()), ("lock", "Front door", "Locked"));
        helper(&mut hub, "saver", json!({"items": [item], "rules": {"complication": "hide"}, "lines": []}));
        assert!(build(&hub, &Value::Null, &civil_at(NOW, 0), NOW, true).rows.is_empty(), "hide: it stays off the saver");
        // a plugin's card is a slot of the plugin's own class
        helper(
            &mut hub,
            "plugin.photos.cards",
            json!({"items": [{"id": "c1", "revision": 1, "card": {"priority": "new", "content": {"type": "facts", "title": "Album", "icon": "base.icons/camera", "facts": [{"label": "x", "value": "12 new"}]}}}]}),
        );
        helper(&mut hub, "saver", json!({"items": [], "rules": {}, "lines": []}));
        let g = build(&hub, &Value::Null, &civil_at(NOW, 0), NOW, true);
        assert_eq!((g.rows[0].title.as_str(), g.rows[0].text.as_str()), ("Album", "12 new"));
        helper(&mut hub, "saver", json!({"items": [], "rules": {"plugin:photos": "hide"}, "lines": []}));
        assert!(build(&hub, &Value::Null, &civil_at(NOW, 0), NOW, true).rows.is_empty(), "the plugin's own class says hide");
    }

    #[test]
    fn notices_are_the_helpers_lines_and_the_recent_notes_of_inside_classes_newest_first_and_a_secret_one_says_who() {
        let line = json!({"id": "camera.e1", "class": "camera.still", "glyph": "user", "title": "Driveway", "text": "Person", "at": NOW - 4 * 60_000, "expiresAt": NOW + 60_000});
        let mut hub = hub_with(json!({"items": [], "rules": {}, "lines": [line]}));
        helper(
            &mut hub,
            "plugin.home-assistant.notifications",
            json!({"items": [note("n1", 120.0, json!({})), note("n2", 30.0, json!({"post": {"title": "Alarm", "body": "Armed", "privacy": "secret"}})),
                             note("old", 5.0 * 3600.0, json!({})), note("low", 10.0, json!({"importance": "low"})), note("gone", 10.0, json!({"expiresAt": NOW as f64 / 1000.0 - 1.0}))]}),
        );
        let g = build(&hub, &Value::Null, &civil_at(NOW, 0), NOW, true);
        let titles: Vec<&str> = g.rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(
            titles,
            vec!["Home Assistant", "Low battery", "Driveway"],
            "the secret note is only who it is from, newest first, old and low and ended ones are not here: {g:?}"
        );
        assert_eq!(g.rows[0].text, "", "a private note carries no words");
        assert_eq!(g.rows[1].age, "2 min");
        assert_eq!(g.rows[2].age, "4 min");
        helper(&mut hub, "saver", json!({"items": [], "rules": {"ha": "hide", "camera.still": "hide"}, "lines": [line]}));
        assert!(build(&hub, &Value::Null, &civil_at(NOW, 0), NOW, true).rows.is_empty(), "both classes say hide");
    }

    #[test]
    fn a_line_that_has_ended_is_not_shown_and_the_notices_are_capped() {
        let lines: Vec<Value> = (0..5).map(|i| json!({"id": format!("l{i}"), "class": "camera.still", "glyph": "camera", "title": format!("Cam {i}"), "text": "x", "at": NOW - i * 1000, "expiresAt": if i == 0 { NOW - 1 } else { NOW + 1000 }})).collect();
        let hub = hub_with(json!({"items": [], "rules": {}, "lines": lines}));
        let g = build(&hub, &Value::Null, &civil_at(NOW, 0), NOW, true);
        assert_eq!(g.rows.len(), MAX_NOTICES);
        assert_eq!(g.rows[0].title, "Cam 1", "the ended line is skipped");
    }
}
