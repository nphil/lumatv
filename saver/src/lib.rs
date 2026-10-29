//! What the screensaver's slots take, and nothing about how they are drawn (`docs/research/saver-complications.md`). The saver is mostly
//! black; the small things a person may want to see at a glance in the dark are *complications*: a glyph, a short label, a short value, how
//! urgent it is and when it stops being true. Anything may contribute one: a plugin on the LumaShow plugin API (through its cards), the
//! helper's own route for a tool that is no plugin (a bridge from ntfy, a script), an app. The saver knows none of them: it reads this
//! contract and draws what the [`Board`] says.
//!
//! - [`Complication`]: `{id, source, glyph, text, value, priority, expiresAt, tone}` on the wire, `priority` 0 (most urgent) to 9, `expiresAt`
//!   in epoch milliseconds, `glyph` one of the module's Lucide names, `text` at most 28 characters, `value` at most 12: a label and a figure,
//!   never a sentence.
//! - [`parse`]: the one validation of a contribution, with the words of what is wrong.
//! - [`Board`]: what is held (at most 12, four a source), replaced by `(source, id)`, expired by the clock, and what shows (at most six,
//!   the most urgent first, the newest of equals first). An item with no expiry is given one day from when it was last said, so a source
//!   that died cannot leave its mark on the panel for ever.
//! - [`from_card`]: a plugin card of the plugin API (level 1) as a complication, so a plugin that has cards has complications.

pub mod rules;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub const MAX_STORED: usize = 12;
pub const MAX_SHOWN: usize = 6;
pub const PER_SOURCE: usize = 4;
pub const MAX_TEXT: usize = 28;
pub const MAX_VALUE: usize = 12;
pub const MAX_ID: usize = 64;
pub const MAX_SOURCE: usize = 32;
/// An item with no expiry of its own is kept this long after it was last said.
pub const MAX_AGE_MS: u64 = 24 * 60 * 60 * 1000;
pub const DEFAULT_PRIORITY: u8 = 5;
/// The glyph drawn when a contribution names none the module has.
pub const FALLBACK_GLYPH: &str = "circle";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    #[default]
    Plain,
    Info,
    Warning,
    Danger,
}

impl Tone {
    pub fn parse(s: &str) -> Option<Tone> {
        Some(match s {
            "" | "plain" | "none" => Tone::Plain,
            "info" | "accent" => Tone::Info,
            "warning" => Tone::Warning,
            "danger" | "critical" => Tone::Danger,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Complication {
    pub id: String,
    pub source: String,
    pub glyph: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub value: String,
    pub priority: u8,
    /// Epoch milliseconds; the board sets one when a contribution has none.
    pub expires_at: u64,
    pub tone: Tone,
    /// When it was last said (epoch ms): the newest of equal priority shows first.
    pub updated_at: u64,
}

/// A name of a Lucide glyph: lower case, digits and hyphens.
pub fn glyph_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn clip(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('\u{2026}');
        t
    }
}

/// A contribution as a complication: `{id, glyph?, text?, value?, priority?, expiresAt?, ttlS?, tone?}` from `source`, said at `now` (epoch ms).
/// `Err` says what is wrong. A label or value that is too long is cut with an ellipsis, not refused; a glyph that is not a name is refused; a
/// contribution that has no glyph, no text and no value says nothing.
pub fn parse(v: &Value, source: &str, now: u64) -> Result<Complication, String> {
    let o = v.as_object().ok_or("a complication is a JSON object")?;
    let text_of = |k: &str| o.get(k).and_then(Value::as_str).unwrap_or("");
    let id = text_of("id").trim();
    if id.is_empty() || id.chars().count() > MAX_ID {
        return Err(format!("id is 1 to {MAX_ID} characters"));
    }
    let source = source.trim();
    if source.is_empty() || source.chars().count() > MAX_SOURCE {
        return Err(format!("source is 1 to {MAX_SOURCE} characters"));
    }
    let glyph = text_of("glyph").trim();
    if !glyph.is_empty() && !glyph_ok(glyph) {
        return Err(
            "glyph is the name of one of the TV's icons (lower case, digits, hyphens)".into(),
        );
    }
    let text = clip(text_of("text"), MAX_TEXT);
    let value = clip(text_of("value"), MAX_VALUE);
    if glyph.is_empty() && text.is_empty() && value.is_empty() {
        return Err("a complication needs a glyph, a text or a value".into());
    }
    let priority = match o.get("priority") {
        None | Some(Value::Null) => DEFAULT_PRIORITY,
        Some(p) => match p.as_u64() {
            Some(n) if n <= 9 => n as u8,
            _ => return Err("priority is 0 (most urgent) to 9".into()),
        },
    };
    let tone = match o.get("tone") {
        None | Some(Value::Null) => Tone::Plain,
        Some(t) => t
            .as_str()
            .and_then(Tone::parse)
            .ok_or("tone is info, warning or danger")?,
    };
    let expires_at = match (
        o.get("expiresAt").and_then(Value::as_u64),
        o.get("ttlS").and_then(Value::as_f64),
    ) {
        (Some(at), _) => at,
        (None, Some(s)) if s > 0.0 => now.saturating_add((s * 1000.0) as u64),
        _ => 0,
    };
    let expires_at = if expires_at == 0 {
        now + MAX_AGE_MS
    } else {
        expires_at.min(now + MAX_AGE_MS)
    };
    Ok(Complication {
        id: id.to_string(),
        source: source.to_string(),
        glyph: if glyph.is_empty() {
            FALLBACK_GLYPH.to_string()
        } else {
            glyph.to_string()
        },
        text,
        value,
        priority,
        expires_at,
        tone,
        updated_at: now,
    })
}

/// What is held and what shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Board {
    items: Vec<Complication>,
}

impl Board {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Takes `c` in: it replaces the item of the same `(source, id)`; past four of one source the one that shows last of theirs goes, and past
    /// twelve in all the one that shows last of all. Returns how many items the board lost to make room.
    pub fn put(&mut self, c: Complication) -> usize {
        self.items
            .retain(|o| !(o.source == c.source && o.id == c.id));
        self.items.push(c.clone());
        let mut lost = 0;
        while self.items.iter().filter(|o| o.source == c.source).count() > PER_SOURCE {
            let worst = self.last_of(|o| o.source == c.source);
            self.items.remove(worst);
            lost += 1;
        }
        while self.items.len() > MAX_STORED {
            let worst = self.last_of(|_| true);
            self.items.remove(worst);
            lost += 1;
        }
        lost
    }

    /// The index of the item that shows last among those `of` accepts: the least urgent, the oldest of equals.
    fn last_of(&self, of: impl Fn(&Complication) -> bool) -> usize {
        let mut best: Option<usize> = None;
        for (i, c) in self.items.iter().enumerate().filter(|(_, c)| of(c)) {
            best = Some(match best {
                None => i,
                Some(b) => {
                    let o = &self.items[b];
                    if (c.priority, std::cmp::Reverse(c.updated_at))
                        > (o.priority, std::cmp::Reverse(o.updated_at))
                    {
                        i
                    } else {
                        b
                    }
                }
            });
        }
        best.unwrap_or(0)
    }

    /// Withdraws one item; whether it was there.
    pub fn remove(&mut self, source: &str, id: &str) -> bool {
        let n = self.items.len();
        self.items.retain(|o| !(o.source == source && o.id == id));
        n != self.items.len()
    }

    /// Withdraws everything one source said.
    pub fn remove_source(&mut self, source: &str) -> usize {
        let n = self.items.len();
        self.items.retain(|o| o.source != source);
        n - self.items.len()
    }

    /// Drops what has expired by `now`; how many went.
    pub fn expire(&mut self, now: u64) -> usize {
        let n = self.items.len();
        self.items.retain(|o| o.expires_at > now);
        n - self.items.len()
    }

    /// When the next item expires, if any is held.
    pub fn next_expiry(&self) -> Option<u64> {
        self.items.iter().map(|c| c.expires_at).min()
    }

    /// What shows at `now`: not expired, the most urgent first and the newest of equals first, at most [`MAX_SHOWN`].
    pub fn shown(&self, now: u64) -> Vec<&Complication> {
        let mut v: Vec<&Complication> = self.items.iter().filter(|c| c.expires_at > now).collect();
        v.sort_by(|a, b| {
            (
                a.priority,
                std::cmp::Reverse(a.updated_at),
                &a.source,
                &a.id,
            )
                .cmp(&(
                    b.priority,
                    std::cmp::Reverse(b.updated_at),
                    &b.source,
                    &b.id,
                ))
        });
        v.truncate(MAX_SHOWN);
        v
    }

    /// The topic's document: `{items: [...]}` in the order they show, all held (not cut to six: the reader decides how many it has room for).
    pub fn document(&self, now: u64) -> Value {
        let mut v: Vec<&Complication> = self.items.iter().filter(|c| c.expires_at > now).collect();
        v.sort_by(|a, b| {
            (
                a.priority,
                std::cmp::Reverse(a.updated_at),
                &a.source,
                &a.id,
            )
                .cmp(&(
                    b.priority,
                    std::cmp::Reverse(b.updated_at),
                    &b.source,
                    &b.id,
                ))
        });
        json!({"items": v})
    }
}

/// The items of a topic document (`{items: [...]}`), each as a complication where it is whole; one that is not is left out.
pub fn from_document(doc: &Value, now: u64) -> Vec<Complication> {
    doc.get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| serde_json::from_value::<Complication>(v.clone()).ok())
        .filter(|c| glyph_ok(&c.glyph) && c.expires_at > now && c.priority <= 9)
        .collect()
}

/// The plugin API's icon names as the TV's glyphs (`kit/PluginHub.qml` `icon()`): the closest the TV carries, else the plug.
pub fn glyph_for(name: &str) -> String {
    let n = name.rsplit('/').next().unwrap_or("").to_lowercase();
    let mapped = match n.as_str() {
        "message" | "notification" | "doorbell" => "bell",
        "home" => "house",
        "light" => "lightbulb",
        "lamp" => "lamp",
        "lock" => "lock",
        "camera" => "camera",
        "music" => "audio-lines",
        "speaker" => "boom-box",
        "tv" => "monitor",
        "sensor" => "activity",
        "clock" => "clock",
        "timer" => "hourglass",
        "info" => "info",
        "home-assistant" => "house-wifi",
        "warning" => "circle-alert",
        "check" => "circle-check",
        "wifi" => "wifi",
        "mic" => "mic",
        "heart" => "heart-pulse",
        "leaf" => "leaf",
        "moon" => "moon",
        "sun" => "sun",
        "alarm" | "alarms" | "alarm-clock" => "alarm-clock",
        "score" | "scores" | "sports" | "trophy" => "trophy",
        "newspaper" | "news" => "scroll-text",
        "cloud" | "weather" => "sun-dim",
        "bot" => "bot",
        "download" => "download",
        other if glyph_ok(other) && CARRIED.contains(&other) => return other.to_string(),
        _ => "plug",
    };
    mapped.to_string()
}

/// The names the TV already has (`icons/lucide/index.json`), drawn as they are.
const CARRIED: &[&str] = &[
    "activity",
    "airplay",
    "antenna",
    "aperture",
    "arrow-big-up",
    "arrow-left",
    "audio-lines",
    "audio-waveform",
    "badge-check",
    "ban",
    "bell",
    "bell-off",
    "blend",
    "boom-box",
    "bot",
    "brain",
    "cable",
    "camera",
    "cast",
    "check",
    "chevron-down",
    "chevron-left",
    "chevron-right",
    "circle",
    "circle-alert",
    "circle-check",
    "circle-help",
    "circle-x",
    "clapperboard",
    "clipboard-list",
    "clock",
    "cog",
    "contrast",
    "corner-down-left",
    "cpu",
    "delete",
    "disc-3",
    "download",
    "drama",
    "ellipsis",
    "ethernet-port",
    "external-link",
    "eye",
    "eye-off",
    "fast-forward",
    "file-text",
    "film",
    "gamepad-2",
    "gauge",
    "gem",
    "globe",
    "globe-2",
    "hard-drive",
    "hash",
    "headphones",
    "heart-pulse",
    "history",
    "hourglass",
    "house",
    "house-wifi",
    "image",
    "info",
    "key-round",
    "keyboard",
    "lamp",
    "languages",
    "layout-grid",
    "leaf",
    "lightbulb",
    "list-checks",
    "list-video",
    "lock",
    "lock-keyhole",
    "log-out",
    "message-square-warning",
    "mic",
    "minus",
    "monitor",
    "monitor-cog",
    "monitor-speaker",
    "moon",
    "moon-star",
    "mountain",
    "mouse-pointer-2",
    "music",
    "network",
    "octagon-alert",
    "package",
    "paintbrush",
    "palette",
    "pause",
    "picture-in-picture",
    "play",
    "plug",
    "plus",
    "popcorn",
    "power",
    "projector",
    "radio",
    "rectangle-horizontal",
    "refresh-cw",
    "rotate-ccw",
    "router",
    "satellite-dish",
    "scan-eye",
    "scroll-text",
    "search",
    "server",
    "settings",
    "shield",
    "shield-alert",
    "shield-check",
    "shield-off",
    "signal",
    "skip-forward",
    "sliders-horizontal",
    "sliders-vertical",
    "smartphone",
    "space",
    "sparkles",
    "speaker",
    "square",
    "square-arrow-out-up-right",
    "square-check",
    "star",
    "sun",
    "sun-dim",
    "sun-medium",
    "sunrise",
    "text-cursor-input",
    "theater",
    "timer",
    "toggle-right",
    "trash-2",
    "triangle-alert",
    "tv",
    "tv-minimal",
    "tv-minimal-play",
    "type",
    "undo-2",
    "unplug",
    "usb",
    "user",
    "volume-1",
    "volume-2",
    "volume-x",
    "wallpaper",
    "wand-sparkles",
    "waves",
    "wifi",
    "wrench",
    "x",
    "zap",
    "alarm-clock",
    "trophy",
    "car",
    "paw-print",
    "video",
    "bell-ring",
];

/// The plugins whose cards have a place of their own on Home and are not complications (the helper's weather, news and clock).
pub const FIRST_PARTY_OWN: &[&str] = &["weather", "news", "clock"];

/// A card of the plugin API (an item of `plugin.<id>.cards`: `{id, revision, createdAt, expiresAt, card: {size, priority, content, ...}}`)
/// as a complication of `plugin_id`: the card's icon is the glyph, its title the label, its first fact's value (an event's detail, a media's
/// subtitle) the value; a card that is ringing is the most urgent, one that is running next, a new one after; the card's own expiry (seconds)
/// is the complication's. `None` for the plugins that draw their own place on Home and for a card with nothing to show.
pub fn from_card(plugin_id: &str, item: &Value, now: u64) -> Option<Complication> {
    if FIRST_PARTY_OWN.contains(&plugin_id) {
        return None;
    }
    let card = item.get("card").unwrap_or(item);
    let content = card.get("content")?;
    let title = content.get("title").and_then(Value::as_str).unwrap_or("");
    let value = match content.get("type").and_then(Value::as_str) {
        Some("facts") => content
            .get("facts")
            .and_then(Value::as_array)
            .and_then(|f| f.first())
            .and_then(|f| f.get("value"))
            .and_then(Value::as_str)
            .unwrap_or(""),
        Some("event") => content.get("detail").and_then(Value::as_str).unwrap_or(""),
        Some("media") => content
            .get("subtitle")
            .and_then(Value::as_str)
            .unwrap_or(""),
        _ => "",
    };
    let tone = match content.get("role").and_then(Value::as_str) {
        Some("danger") => Tone::Danger,
        Some("warning") => Tone::Warning,
        Some("accent") | Some("info") => Tone::Info,
        _ => Tone::Plain,
    };
    let priority = match card.get("priority").and_then(Value::as_str) {
        Some("ringing") => 1,
        Some("running") => 3,
        Some("new") => 4,
        _ => DEFAULT_PRIORITY,
    };
    let id = item
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    if title.trim().is_empty() && value.trim().is_empty() {
        return None;
    }
    let mut doc = Map::new();
    doc.insert("id".into(), json!(id));
    doc.insert(
        "glyph".into(),
        json!(glyph_for(
            content.get("icon").and_then(Value::as_str).unwrap_or("")
        )),
    );
    doc.insert("text".into(), json!(title));
    doc.insert("value".into(), json!(value));
    doc.insert("priority".into(), json!(priority));
    doc.insert(
        "tone".into(),
        json!(match tone {
            Tone::Plain => "plain",
            Tone::Info => "info",
            Tone::Warning => "warning",
            Tone::Danger => "danger",
        }),
    );
    if let Some(secs) = item
        .get("expiresAt")
        .and_then(Value::as_f64)
        .filter(|s| *s > 0.0)
    {
        doc.insert("expiresAt".into(), json!((secs * 1000.0) as u64));
    }
    parse(&Value::Object(doc), plugin_id, now).ok()
}

/// Every card of a plugin's `plugin.<id>.cards` document (`{items: [...]}`) as complications.
pub fn from_cards(plugin_id: &str, doc: &Value, now: u64) -> Vec<Complication> {
    doc.get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|i| from_card(plugin_id, i, now))
        .take(PER_SOURCE)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000_000;

    fn c(source: &str, id: &str, priority: u8, at: u64) -> Complication {
        parse(
            &json!({"id": id, "glyph": "bell", "text": id, "priority": priority}),
            source,
            at,
        )
        .unwrap()
    }

    #[test]
    fn a_contribution_is_a_glyph_a_label_and_a_value_with_a_priority_and_an_expiry() {
        let v = parse(&json!({"id": "door", "glyph": "lock", "text": "Front door", "value": "Locked", "priority": 2, "tone": "warning", "ttlS": 600}), "home-assistant", NOW).unwrap();
        assert_eq!(
            (
                v.id.as_str(),
                v.source.as_str(),
                v.glyph.as_str(),
                v.text.as_str(),
                v.value.as_str()
            ),
            ("door", "home-assistant", "lock", "Front door", "Locked")
        );
        assert_eq!(
            (v.priority, v.tone, v.expires_at, v.updated_at),
            (2, Tone::Warning, NOW + 600_000, NOW)
        );
        let wire = serde_json::to_value(&v).unwrap();
        assert_eq!(wire["expiresAt"], NOW + 600_000, "camelCase on the wire");
        assert_eq!(wire["tone"], "warning");
        assert_eq!(serde_json::from_value::<Complication>(wire).unwrap(), v);
    }

    #[test]
    fn what_is_wrong_is_said_and_what_is_long_is_cut_not_refused() {
        let bad = |v: Value| parse(&v, "x", NOW).unwrap_err();
        assert!(bad(json!("a string")).contains("object"));
        assert!(bad(json!({"glyph": "bell", "text": "x"})).contains("id"));
        assert!(bad(json!({"id": "a"})).contains("glyph, a text or a value"));
        assert!(bad(json!({"id": "a", "glyph": "Bell!"})).contains("icons"));
        assert!(bad(json!({"id": "a", "text": "x", "priority": 10})).contains("priority"));
        assert!(bad(json!({"id": "a", "text": "x", "tone": "pink"})).contains("tone"));
        assert!(
            parse(&json!({"id": "a", "text": "x"}), "", NOW).is_err(),
            "a source is needed"
        );
        let long = parse(&json!({"id": "a", "text": "A label that is certainly longer than twenty-eight characters", "value": "1234567890123456"}), "x", NOW).unwrap();
        assert_eq!(long.text.chars().count(), MAX_TEXT);
        assert!(long.text.ends_with('\u{2026}'));
        assert_eq!(long.value.chars().count(), MAX_VALUE);
        let glyphless = parse(&json!({"id": "a", "text": "Back door"}), "x", NOW).unwrap();
        assert_eq!(glyphless.glyph, FALLBACK_GLYPH);
    }

    #[test]
    fn nothing_stays_for_ever_an_item_with_no_end_is_given_a_day_and_none_is_kept_longer() {
        assert_eq!(
            parse(&json!({"id": "a", "text": "x"}), "s", NOW)
                .unwrap()
                .expires_at,
            NOW + MAX_AGE_MS
        );
        assert_eq!(
            parse(
                &json!({"id": "a", "text": "x", "expiresAt": NOW + 10 * MAX_AGE_MS}),
                "s",
                NOW
            )
            .unwrap()
            .expires_at,
            NOW + MAX_AGE_MS
        );
        assert_eq!(
            parse(
                &json!({"id": "a", "text": "x", "expiresAt": NOW + 5000}),
                "s",
                NOW
            )
            .unwrap()
            .expires_at,
            NOW + 5000
        );
    }

    #[test]
    fn the_board_replaces_by_source_and_id_caps_a_source_at_four_and_all_at_twelve() {
        let mut b = Board::new();
        b.put(c("a", "x", 5, NOW));
        let mut again = c("a", "x", 3, NOW + 10);
        again.text = "changed".into();
        b.put(again);
        assert_eq!(b.len(), 1);
        assert_eq!(b.shown(NOW + 20)[0].text, "changed");
        for i in 0..6 {
            b.put(c("loud", &format!("i{i}"), 5, NOW + i as u64));
        }
        assert_eq!(
            b.items.iter().filter(|o| o.source == "loud").count(),
            PER_SOURCE,
            "one source may hold four"
        );
        assert!(
            b.items
                .iter()
                .all(|o| o.source != "loud" || o.id != "i0" && o.id != "i1"),
            "the oldest of equal priority went first"
        );
        for s in 0..6 {
            for i in 0..3 {
                b.put(c(
                    &format!("s{s}"),
                    &format!("i{i}"),
                    5,
                    NOW + 100 + s as u64 * 10 + i as u64,
                ));
            }
        }
        assert!(b.len() <= MAX_STORED);
        let mut urgent = Board::new();
        for i in 0..12 {
            urgent.put(c(&format!("s{i}"), "x", 5, NOW + i));
        }
        urgent.put(c("late", "x", 0, NOW + 100));
        assert_eq!(urgent.len(), MAX_STORED);
        assert!(
            urgent.shown(NOW + 200).iter().any(|c| c.source == "late"),
            "the urgent one is kept and the oldest of the rest went"
        );
        assert!(!urgent.items.iter().any(|c| c.source == "s0"));
    }

    #[test]
    fn what_shows_is_the_most_urgent_first_then_the_newest_and_never_more_than_six_or_what_has_ended()
     {
        let mut b = Board::new();
        b.put(c("a", "calm", 5, NOW));
        b.put(c("a", "newer-calm", 5, NOW + 50));
        b.put(c("b", "urgent", 1, NOW + 10));
        let mut short = c("b", "short", 0, NOW + 20);
        short.expires_at = NOW + 1000;
        b.put(short);
        let ids = |now| {
            b.shown(now)
                .iter()
                .map(|c| c.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(NOW + 60), vec!["short", "urgent", "newer-calm", "calm"]);
        assert_eq!(
            ids(NOW + 1000),
            vec!["urgent", "newer-calm", "calm"],
            "ended at its time"
        );
        assert_eq!(b.next_expiry(), Some(NOW + 1000));
        assert_eq!(b.expire(NOW + 1000), 1);
        for i in 0..4 {
            b.put(c("c", &format!("c{i}"), 4, NOW + 100 + i));
        }
        assert_eq!(b.shown(NOW + 200).len(), MAX_SHOWN);
        assert!(b.remove("a", "calm"));
        assert!(!b.remove("a", "calm"));
        assert_eq!(b.remove_source("c"), 4);
        let doc = b.document(NOW + 200);
        let back = from_document(&doc, NOW + 200);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].id, "urgent");
    }

    #[test]
    fn a_plugin_card_is_a_complication_by_the_rules_and_the_helpers_own_places_are_not() {
        let facts = json!({"id": "door", "revision": 3, "createdAt": 1, "expiresAt": (NOW / 1000) + 90, "card": {"size": "half", "priority": "running",
            "content": {"type": "facts", "title": "Front door", "icon": "base.icons/lock", "role": "warning", "facts": [{"label": "State", "value": "Locked"}]}}});
        let v = from_card("home-assistant", &facts, NOW).unwrap();
        assert_eq!(
            (
                v.glyph.as_str(),
                v.text.as_str(),
                v.value.as_str(),
                v.priority,
                v.tone
            ),
            ("lock", "Front door", "Locked", 3, Tone::Warning)
        );
        assert_eq!(
            v.expires_at,
            NOW + 90_000,
            "the card's end, in milliseconds"
        );
        let event = json!({"id": "ring", "card": {"priority": "ringing", "content": {"type": "event", "title": "Doorbell", "detail": "Front", "icon": "doorbell"}}});
        let r = from_card("ring-plugin", &event, NOW).unwrap();
        assert_eq!(
            (r.priority, r.glyph.as_str(), r.value.as_str()),
            (1, "bell", "Front")
        );
        let media = json!({"id": "m", "card": {"priority": "new", "content": {"type": "media", "title": "Kitchen", "subtitle": "Jazz", "icon": "speaker"}}});
        assert_eq!(from_card("sonos", &media, NOW).unwrap().value, "Jazz");
        assert!(
            from_card("weather", &facts, NOW).is_none(),
            "the weather has its own place"
        );
        assert!(
            from_card(
                "x",
                &json!({"id": "n", "card": {"content": {"type": "facts"}}}),
                NOW
            )
            .is_none(),
            "nothing to show"
        );
        let doc = json!({"items": [facts, event, media]});
        assert_eq!(from_cards("p", &doc, NOW).len(), 3);
    }

    #[test]
    fn the_plugin_apis_icon_names_become_the_tvs_glyphs() {
        assert_eq!(glyph_for("base.icons/message"), "bell");
        assert_eq!(glyph_for("home-assistant"), "house-wifi");
        assert_eq!(glyph_for("lock"), "lock");
        assert_eq!(glyph_for("Alarm"), "alarm-clock");
        assert_eq!(
            glyph_for("paw-print"),
            "paw-print",
            "a name the TV has is drawn as it is"
        );
        assert_eq!(glyph_for("no-such-icon"), "plug");
        assert_eq!(glyph_for(""), "plug");
    }
}
