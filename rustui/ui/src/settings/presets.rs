//! The helper's picture presets and slots (topic `picture`, `helper/app/src/picture.rs`), as Settings › Picture and Quick Settings list
//! them after LG's own modes.

use crate::settings::names;
use serde_json::{Value, json};

pub const TOPIC: &str = "picture";
/// A preset's value in a mode list, apart from LG's mode values.
pub const PREFIX: &str = "preset:";
pub const POST: &str = "/v1/picture/preset";

#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub id: String,
    pub name: String,
    /// The LG mode it is written into.
    pub mode: String,
    /// Whether it is the last written into that mode.
    pub applied: bool,
}

impl Preset {
    pub fn value(&self) -> String {
        format!("{PREFIX}{}", self.id)
    }

    pub fn icon(&self) -> &'static str {
        match self.id.as_str() {
            "lumaVivid" => "sparkles",
            "rtingsRich" => "palette",
            _ => "scan-eye",
        }
    }

    pub fn body(&self) -> Value {
        json!({"id": self.id})
    }
}

/// Every preset the helper offers.
pub fn all(topic: Option<&Value>) -> Vec<Preset> {
    topic
        .and_then(|t| t.get("presets"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_owned);
            Some(Preset { id: s("id")?, name: s("name")?, mode: s("mode")?, applied: p.get("applied").and_then(Value::as_bool) == Some(true) })
        })
        .collect()
}

/// The presets a mode list offers: only while the apps' SDR picture is in force, the one place the helper writes them.
pub fn offered(topic: Option<&Value>) -> Vec<Preset> {
    let context = topic.and_then(|t| t.get("context"));
    let s = |k: &str| context.and_then(|c| c.get(k)).and_then(Value::as_str).unwrap_or("");
    if s("input") == "default" && s("range").eq_ignore_ascii_case("sdr") { all(topic) } else { Vec::new() }
}

/// The preset in force: the last written into the mode in force.
pub fn in_force<'a>(presets: &'a [Preset], mode: &str) -> Option<&'a Preset> {
    presets.iter().find(|p| p.applied && p.mode == mode)
}

/// A mode's name, or the name of the preset last written into it.
pub fn mode_name(presets: &[Preset], mode: &str) -> String {
    in_force(presets, mode).map_or_else(|| names::picture_mode(mode).to_string(), |p| p.name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topic(input: &str) -> Value {
        json!({"context": {"mode": "eco", "input": input, "range": "sdr"}, "presets": [
            {"id": "rtings", "name": "RTINGS Accurate", "mode": "expert2", "applied": false},
            {"id": "lumaVivid", "name": "LumaTV Vivid", "mode": "eco", "applied": true}
        ]})
    }

    #[test]
    fn presets_are_offered_for_the_apps_sdr_picture_only() {
        let apps = topic("default");
        let offered = offered(Some(&apps));
        assert_eq!(offered.iter().map(Preset::value).collect::<Vec<_>>(), ["preset:rtings", "preset:lumaVivid"]);
        assert_eq!(in_force(&offered, "eco").map(|p| p.id.as_str()), Some("lumaVivid"));
        assert_eq!(mode_name(&offered, "eco"), "LumaTV Vivid");
        assert!(super::offered(Some(&topic("hdmi2"))).is_empty());
        assert!(super::offered(None).is_empty());
    }
}
