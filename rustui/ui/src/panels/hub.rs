//! What the plugin broker puts on the quick panel and in the notification drawer (`kit/PluginHub.qml` of the QML shell): the helper
//! publishes the roster (`plugins`: the plugins the owner switched on, and the helper's own as `firstParty`) and, per connected
//! plugin, its tiles and notifications (`plugin.<id>.tiles`, `plugin.<id>.notifications`). Every contribution has been validated by
//! the helper, so nothing here checks a plugin's words; it only reads them, and a tap is `POST /v1/plugins/<id>/actions/<action>`.
//! The plugin's icon names are its own (`base.icons/lamp`): the closest of the icons this TV carries, or a plug.

use serde_json::Value;
use std::collections::BTreeMap;

pub type Topics = BTreeMap<String, Value>;

#[derive(Clone, Debug, PartialEq)]
pub struct Tile {
    pub plugin_id: String,
    pub plugin: String,
    pub id: String,
    pub label: String,
    pub icon: String,
    /// `on`, `off`, `observed`, `unknown`, `unavailable`.
    pub state: String,
    pub on: bool,
    pub reason: String,
    pub action: String,
    pub target: Option<String>,
    pub hint: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub id: String,
    pub label: String,
    pub action: String,
    pub target: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub plugin_id: String,
    pub plugin: String,
    pub id: String,
    pub revision: i64,
    pub channel_icon: String,
    pub received_at: f64,
    pub title: String,
    pub body: String,
    pub actions: Vec<Action>,
}

const MAPPED: &[(&str, &str)] = &[
    ("message", "bell"),
    ("notification", "bell"),
    ("home", "house"),
    ("light", "lightbulb"),
    ("lamp", "lamp"),
    ("lock", "lock"),
    ("camera", "camera"),
    ("music", "audio-lines"),
    ("speaker", "boom-box"),
    ("tv", "monitor"),
    ("sensor", "activity"),
    ("clock", "clock"),
    ("timer", "hourglass"),
    ("info", "info"),
    ("home-assistant", "house-wifi"),
    ("doorbell", "bell"),
    ("warning", "circle-alert"),
    ("check", "circle-check"),
    ("wifi", "wifi"),
    ("mic", "mic"),
    ("heart", "heart-pulse"),
    ("leaf", "leaf"),
    ("moon", "moon"),
    ("sun", "sun"),
    ("alarm", "alarm-clock"),
    ("alarms", "alarm-clock"),
    ("alarm-clock", "alarm-clock"),
    ("score", "trophy"),
    ("scores", "trophy"),
    ("sports", "trophy"),
    ("trophy", "trophy"),
    ("newspaper", "scroll-text"),
    ("news", "scroll-text"),
    ("cloud", "sun-dim"),
    ("weather", "sun-dim"),
    ("bot", "bot"),
    ("download", "download"),
];

/// The names the TV already has icons for (`icons/lucide`), drawn as they are.
const CARRIED: &[&str] = &[
    "activity", "airplay", "antenna", "aperture", "arrow-big-up", "arrow-left", "audio-lines", "audio-waveform", "badge-check", "ban", "bell", "bell-off", "blend", "boom-box", "bot", "brain", "cable", "camera", "cast", "check",
    "chevron-down", "chevron-left", "chevron-right", "circle", "circle-alert", "circle-check", "circle-help", "circle-x", "clapperboard", "clipboard-list", "clock", "cog", "contrast", "corner-down-left", "cpu", "delete", "disc-3",
    "download", "drama", "ellipsis", "ethernet-port", "external-link", "eye", "eye-off", "fast-forward", "file-text", "film", "gamepad-2", "gauge", "gem", "globe", "globe-2", "hard-drive", "hash", "headphones", "heart-pulse",
    "history", "hourglass", "house", "house-wifi", "image", "info", "key-round", "keyboard", "lamp", "languages", "layout-grid", "leaf", "lightbulb", "list-checks", "list-video", "lock", "lock-keyhole", "log-out",
    "message-square-warning", "mic", "minus", "monitor", "monitor-cog", "monitor-speaker", "moon", "moon-star", "mountain", "mouse-pointer-2", "music", "network", "octagon-alert", "package", "paintbrush", "palette", "pause",
    "picture-in-picture", "play", "plug", "plus", "popcorn", "power", "projector", "radio", "rectangle-horizontal", "refresh-cw", "rotate-ccw", "router", "satellite-dish", "scan-eye", "scroll-text", "search", "server",
    "settings", "shield", "shield-alert", "shield-check", "shield-off", "signal", "skip-forward", "sliders-horizontal", "sliders-vertical", "smartphone", "space", "sparkles", "speaker", "square", "square-arrow-out-up-right",
    "square-check", "star", "sun", "sun-dim", "sun-medium", "sunrise", "text-cursor-input", "theater", "timer", "toggle-right", "trash-2", "triangle-alert", "tv", "tv-minimal", "tv-minimal-play", "type", "undo-2", "unplug",
    "usb", "user", "volume-1", "volume-2", "volume-x", "wallpaper", "wand-sparkles", "waves", "wifi", "wrench", "x", "zap", "alarm-clock", "trophy",
];

/// A plugin's icon name as one of the TV's: the part after the last `/`, mapped, or carried, or a plug.
pub fn icon(name: &str) -> &'static str {
    let n = name.rsplit('/').next().unwrap_or("").to_lowercase();
    if let Some((_, to)) = MAPPED.iter().find(|(from, _)| *from == n) {
        return to;
    }
    CARRIED.iter().find(|c| **c == n).copied().unwrap_or("plug")
}

fn text(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// The plugins that are connected: the owner's, then the helper's own.
fn connected(topics: &Topics) -> Vec<(String, String)> {
    let roster = topics.get("plugins").cloned().unwrap_or(Value::Null);
    let mut out = Vec::new();
    for key in ["plugins", "firstParty"] {
        for p in roster.get(key).and_then(Value::as_array).into_iter().flatten() {
            if p.get("state").and_then(Value::as_str) == Some("connected") {
                let id = text(p, "id");
                let name = p.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).map_or_else(|| id.clone(), str::to_string);
                out.push((id, name));
            }
        }
    }
    out
}

fn items(topics: &Topics, plugin: &str, kind: &str) -> Vec<Value> {
    topics.get(&format!("plugin.{plugin}.{kind}")).and_then(|t| t.get("items")).and_then(Value::as_array).cloned().unwrap_or_default()
}

pub fn tiles(topics: &Topics) -> Vec<Tile> {
    let mut out = Vec::new();
    for (pid, name) in connected(topics) {
        for it in items(topics, &pid, "tiles") {
            let state = it.get("state").cloned().unwrap_or(Value::Null);
            let kind = text(&state, "type");
            let kind = if kind.is_empty() { "unknown".to_string() } else { kind };
            let on = kind == "on" || (kind == "observed" && state.get("on").and_then(Value::as_bool) == Some(true));
            out.push(Tile {
                plugin_id: pid.clone(),
                plugin: name.clone(),
                id: text(&it, "id"),
                label: text(&it, "label"),
                icon: icon(&text(&it, "icon")).to_string(),
                state: kind,
                on,
                reason: text(&state, "reason"),
                action: text(&it, "action"),
                target: it.get("target").and_then(Value::as_str).map(str::to_string),
                hint: text(&it, "hint"),
            });
        }
    }
    out
}

/// The plugin of the roster (`plugins`: the owner's, then the helper's own as `firstParty`) with this id, connected or not.
pub fn plugin_of<'a>(roster: &'a Value, id: &str) -> Option<&'a Value> {
    let list = |k: &str| roster.get(k).and_then(Value::as_array).into_iter().flatten();
    list("plugins").chain(list("firstParty")).find(|p| p.get("id").and_then(Value::as_str) == Some(id))
}

/// Where Home Assistant stands, in one word (`PluginHub.haState`): `off` (not switched on), `inert` (switched on but this TV will not run
/// it), `waiting` (its program is not up), then what the helper's lease (`ha.link`, `Null` before the helper has said) says: `unlinked`,
/// `refused`, `offline`, `connecting` or `connected`.
pub fn ha_state(roster: &Value, link: &Value) -> String {
    let Some(p) = plugin_of(roster, HA) else { return "off".into() };
    match p.get("state").and_then(Value::as_str) {
        Some("inert") => return "inert".into(),
        Some("connected") => {}
        _ => return "waiting".into(),
    }
    match link.get("state").and_then(Value::as_str) {
        Some("auth_failed") => "refused".into(),
        None | Some("off" | "unconfigured") => "unlinked".into(),
        Some(s) => s.into(),
    }
}

/// A short phrase for a row that opens the page (`PluginHub.haWords`).
pub fn ha_words(roster: &Value, link: &Value) -> &'static str {
    let stuck = || plugin_of(roster, HA).and_then(|p| p.get("reason")).is_some_and(|r| !r.is_null());
    match ha_state(roster, link).as_str() {
        "off" => "Not switched on",
        "inert" => "Cannot run",
        "waiting" if stuck() => "Not running",
        "waiting" => "Starting",
        "unlinked" => "Not linked",
        "refused" => "Token refused",
        "offline" => "Offline",
        "connecting" => "Connecting",
        _ => "Connected",
    }
}

/// What the At-home card says when no plugin has a card (`PluginHub.laneNone`): a name, the state under it, and the tone of its glyph
/// (`danger`, `warning`, or `muted`).
pub struct Lane {
    pub title: &'static str,
    pub sub: &'static str,
    pub role: &'static str,
}

pub fn lane_none(roster: &Value, link: &Value) -> Lane {
    let state = ha_state(roster, link);
    let stuck = state == "waiting" && plugin_of(roster, HA).and_then(|p| p.get("reason")).is_some_and(|r| !r.is_null());
    let role = if state == "refused" {
        "danger"
    } else if matches!(state.as_str(), "inert" | "offline") || stuck {
        "warning"
    } else {
        "muted"
    };
    Lane { title: "Home Assistant", sub: if state == "connected" { "Nothing to show" } else { ha_words(roster, link) }, role }
}

/// The plugins whose cards Home draws in full in their own widgets.
const HOME_EXCLUDED: [&str; 3] = ["weather", "news", "clock"];

/// What the At-home card draws as chips (`PluginHub.homeCards`): the cards of every connected plugin but those Home draws elsewhere, the
/// most urgent first (`ringing`, `running`, `new`, any other), each as the helper published it with the plugin's id as `pluginId`.
pub fn home_cards(topics: &Topics) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for (pid, _) in connected(topics) {
        if HOME_EXCLUDED.contains(&pid.as_str()) {
            continue;
        }
        for mut it in items(topics, &pid, "cards") {
            if let Some(o) = it.as_object_mut() {
                o.insert("pluginId".into(), Value::String(pid.clone()));
            }
            out.push(it);
        }
    }
    let rank = |c: &Value| match c.pointer("/card/priority").and_then(Value::as_str).unwrap_or("new") {
        "ringing" => 0,
        "running" => 1,
        "new" => 2,
        _ => 3,
    };
    out.sort_by_key(rank);
    out
}

/// The Home Assistant plugin's id.
pub const HA: &str = "home-assistant";

/// What a tile says when its plugin cannot be reached or may not act.
pub fn unavailable_text(reason: &str) -> &'static str {
    match reason {
        "offline" => "Offline",
        "not_permitted" => "Not allowed",
        "no_capability" => "Not on this TV",
        _ => "Not available",
    }
}

pub fn notes(topics: &Topics) -> Vec<Note> {
    let mut out = Vec::new();
    for (pid, name) in connected(topics) {
        for it in items(topics, &pid, "notifications") {
            let post = it.get("post").cloned().unwrap_or(Value::Null);
            let actions = post
                .get("actions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|a| Action { id: text(a, "id"), label: text(a, "label"), action: text(a, "action"), target: a.get("target").and_then(Value::as_str).map(str::to_string) })
                .collect();
            out.push(Note {
                plugin_id: pid.clone(),
                plugin: name.clone(),
                id: text(&it, "id"),
                revision: it.get("revision").and_then(Value::as_i64).unwrap_or(0),
                channel_icon: text(&it, "channelIcon"),
                received_at: it.get("receivedAt").and_then(Value::as_f64).unwrap_or(0.0),
                title: text(&post, "title"),
                body: text(&post, "body"),
                actions,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn topics() -> Topics {
        let mut t = Topics::new();
        t.insert("plugins".into(), json!({"plugins": [{"id": "ambient", "name": "Ambient light", "state": "connected"}, {"id": "lock", "name": "Front door", "state": "connected"}, {"id": "off", "state": "inert"}]}));
        t.insert("plugin.ambient.tiles".into(), json!({"items": [{"id": "ambient", "label": "Ambient light", "icon": "base.icons/lamp", "state": {"type": "on"}, "action": "toggle", "hint": "Follows the picture"}]}));
        t.insert(
            "plugin.lock.tiles".into(),
            json!({"items": [
                {"id": "door", "label": "Front door", "icon": "base.icons/lock", "state": {"type": "unavailable", "reason": "offline"}, "action": "lock"},
                {"id": "scene", "label": "Movie scene", "icon": "base.icons/home", "state": {"type": "unknown"}, "action": "run", "target": "movie"}
            ]}),
        );
        t.insert("plugin.off.tiles".into(), json!({"items": [{"id": "x", "label": "Not shown"}]}));
        t
    }

    #[test]
    fn the_tiles_of_the_connected_plugins_in_the_rosters_order_with_their_states() {
        let t = tiles(&topics());
        assert_eq!(t.iter().map(|t| (t.plugin_id.as_str(), t.id.as_str())).collect::<Vec<_>>(), vec![("ambient", "ambient"), ("lock", "door"), ("lock", "scene")], "a plugin that is not connected has none");
        assert_eq!((t[0].state.as_str(), t[0].on, t[0].icon.as_str(), t[0].hint.as_str()), ("on", true, "lamp", "Follows the picture"));
        assert_eq!((t[1].state.as_str(), t[1].reason.as_str()), ("unavailable", "offline"));
        assert_eq!(unavailable_text(&t[1].reason), "Offline");
        assert_eq!((t[2].state.as_str(), t[2].target.as_deref(), t[2].icon.as_str()), ("unknown", Some("movie"), "house"));
    }

    #[test]
    fn an_icon_is_the_closest_the_tv_carries_or_a_plug() {
        assert_eq!(icon("base.icons/message"), "bell");
        assert_eq!(icon("hourglass"), "hourglass");
        assert_eq!(icon("base.icons/never-heard-of-it"), "plug");
        assert_eq!(icon(""), "plug");
    }

    #[test]
    fn the_notes_carry_their_buttons() {
        let mut t = topics();
        t.insert(
            "plugin.lock.notifications".into(),
            json!({"items": [{"id": "n1", "revision": 2, "receivedAt": 1700000000.0, "channelIcon": "base.icons/doorbell", "post": {"title": "Door", "body": "Opened", "actions": [{"id": "ack", "label": "Got it", "action": "ack"}]}}]}),
        );
        let n = notes(&t);
        assert_eq!(n.len(), 1);
        assert_eq!((n[0].title.as_str(), n[0].body.as_str(), n[0].revision, n[0].plugin.as_str()), ("Door", "Opened", 2, "Front door"));
        assert_eq!(n[0].actions, vec![Action { id: "ack".into(), label: "Got it".into(), action: "ack".into(), target: None }]);
    }
}
