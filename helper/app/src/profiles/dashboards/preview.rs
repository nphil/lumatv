//! Editor previews use display fields only, never plugin configs, actions, paths or endpoints.

use super::layouts::display_text;
use crate::hub::Hub;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;

static THEME: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../../../payload/theme/theme.json"))
        .expect("shipped theme is JSON")
});

fn is_colour(s: &str) -> bool {
    matches!(s.len(), 7 | 9) && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

fn word(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b' '))
}

fn visual(default: &Value, user: &Value) -> Value {
    match default {
        Value::Object(fields) => Value::Object(fields.iter().map(|(k, v)| (k.clone(), visual(v, &user[k]))).collect()),
        Value::Array(a) => Value::Array(a.iter().enumerate().map(|(i, v)| visual(v, &user[i])).collect()),
        Value::Number(_) if user.as_f64().is_some_and(|n| (-4096.0..=4096.0).contains(&n)) => user.clone(),
        Value::Bool(_) if user.is_boolean() => user.clone(),
        Value::String(s) if user.as_str().is_some_and(|value| {
            if is_colour(s) { is_colour(value) } else { word(value) }
        }) => user.clone(),
        _ => default.clone(),
    }
}

fn one_of<'a>(value: &'a Value, options: &[&str]) -> Option<&'a str> {
    value.as_str().filter(|s| options.contains(s))
}

pub(super) fn theme(user: &Value, tint: &Value) -> Value {
    let fields = ["dark", "light", "radii", "type", "space", "target", "row", "blurRadius", "saturation", "glassStrength", "depth", "focus", "states", "motion", "clock"];
    let mut out: Map<String, Value> = fields.into_iter().map(|k| (k.to_owned(), visual(&THEME[k], &user[k]))).collect();
    let material = one_of(&user["material"], &["auto", "dark", "light"]).unwrap_or("auto");
    let glass = one_of(&user["cardGlass"], &["clear", "frosted", "liquid", "glazed"]).unwrap_or("clear");
    let threshold = user["materialThreshold"].as_f64().filter(|n| (0.0..=1.0).contains(n)).unwrap_or(0.55);
    let mode = match material {
        "auto" if tint["luma"].as_f64().is_some_and(|n| (threshold..=1.0).contains(&n)) => "light",
        "auto" => "dark",
        fixed => fixed,
    };
    out.insert("material".into(), json!(material));
    out.insert("mode".into(), json!(mode));
    out.insert("cardGlass".into(), json!(glass));
    if let Some(accent) = user["accent"].as_str().filter(|s| is_colour(s)) {
        out.insert("accent".into(), json!(accent));
    }
    let mut out = Value::Object(out);
    for mode in ["dark", "light"] {
        for k in ["glassRaised", "edge", "edgeRaised"] {
            if let Some(c) = user[mode][k].as_str().filter(|s| is_colour(s)) {
                out[mode][k] = json!(c);
            }
        }
        if let Some(stops) = user["wallpaperPalette"][mode].as_array().filter(|a| {
            a.len() == 3 && a.iter().all(|v| v.as_str().is_some_and(is_colour))
        }) {
            out["wallpaperPalette"][mode] = json!(stops);
        }
    }
    if let Some(family) = display_text(&user["font"]["family"], 48) {
        out["font"]["family"] = family;
    }
    let bar = &user["layout"]["statusBar"];
    let date = one_of(&bar["date"], &["left", "centre", "right", "hidden", "clock"]).unwrap_or("centre");
    let profile = one_of(&bar["profile"], &["left", "right"]).unwrap_or("right");
    out["layout"]["statusBar"] = json!({"date": date, "profile": profile});
    out["clock"]["style"] = json!(one_of(&user["clock"]["style"], &["analog", "digital"]).unwrap_or("analog"));
    out
}

fn texts(source: &Value, fields: &[(&str, usize)]) -> Value {
    Value::Object(fields.iter().filter_map(|&(k, max)| display_text(&source[k], max).map(|v| (k.to_owned(), v))).collect())
}

fn rows(source: &Value, cap: usize) -> impl Iterator<Item = &Value> {
    source.as_array().into_iter().flatten().take(cap)
}

fn state(source: &Value) -> Value {
    source["state"].as_str().filter(|s| word(s)).map_or(Value::Null, |s| json!(s))
}

pub(super) fn widgets(hub: &Hub) -> Value {
    hub.inspect_topics(|topics| {
        let mut out = Map::new();
        if let Some(display) = topics.get(crate::homefeeds::WEATHER_TOPIC).and_then(|w| w.pointer("/doc/display")).filter(|v| v.is_object()) {
            let mut weather = texts(display, &[("place", 80), ("temp", 24), ("cond", 80), ("stats", 120)]);
            if let Some(kind) = display["nowKind"].as_str().filter(|s| word(s)) {
                weather["kind"] = json!(kind);
            }
            let days: Vec<Value> = rows(&display["days"], 7)
                .map(|d| texts(d, &[("day", 16), ("lo", 24), ("hi", 24), ("kind", 24)]))
                .collect();
            weather["days"] = json!(days);
            out.insert("weather".into(), weather);
        }
        if let Some(items) = topics.get(crate::homefeeds::NEWS_TOPIC).and_then(|n| n.pointer("/doc/items")) {
            let items: Vec<Value> = rows(items, 12).filter_map(|story| {
                let mut data = texts(story, &[("title", 160), ("summary", 320), ("tag", 32), ("src", 64)]);
                data.get("title")?;
                if let Some(published) = story["pubMs"].as_u64() {
                    data["pubMs"] = json!(published);
                }
                Some(data)
            }).collect();
            out.insert("news".into(), json!({"items": items}));
        }
        if let Some(source) = topics.get(crate::scrypted::TOPIC) {
            let cameras: Vec<Value> = rows(&source["cameras"], 16).filter_map(|camera| {
                let mut data = texts(camera, &[("name", 64), ("room", 64)]);
                data.get("name")?;
                if let Some(online) = camera["online"].as_bool() {
                    data["online"] = json!(online);
                }
                Some(data)
            }).collect();
            out.insert("cameras".into(), json!({"state": state(source), "cameras": cameras}));
        }
        if let Some(source) = topics.get(crate::onhome::TOPIC) {
            let tiles: Vec<Value> = rows(&source["tiles"], 24).filter_map(|tile| {
                let mut data = texts(tile, &[("name", 80), ("words", 120), ("icon", 32), ("tone", 24), ("size", 24), ("group", 24)]);
                data.get("name")?;
                if let Some(available) = tile["available"].as_bool() {
                    data["available"] = json!(available);
                }
                Some(data)
            }).collect();
            out.insert("onhome".into(), json!({"state": state(source), "tiles": tiles}));
        }
        let link = topics.get("ha.link");
        let cards = topics.get("plugin.home-assistant.cards");
        if link.is_some() || cards.is_some() {
            let items: Vec<Value> = rows(cards.map_or(&Value::Null, |v| &v["items"]), 8).filter_map(|item| {
                let c = item.pointer("/card/content")?;
                let mut data = texts(c, &[("title", 80), ("role", 24)]);
                data.get("title")?;
                if let Some(icon) = c["icon"].as_str().and_then(|s| s.rsplit('/').next()).filter(|s| word(s)) {
                    data["icon"] = json!(icon);
                }
                if let Some(fact) = c.pointer("/facts/0") {
                    let label = fact["label"].as_str().unwrap_or("");
                    let value = fact["value"].as_str().unwrap_or("");
                    let detail = json!(format!("{label} {value}").trim());
                    if let Some(detail) = display_text(&detail, 96) {
                        data["detail"] = detail;
                    }
                }
                Some(data)
            }).collect();
            out.insert("homeassistant".into(), json!({"state": link.map_or(Value::Null, state), "items": items}));
        }
        Value::Object(out)
    })
}
