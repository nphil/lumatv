//! The helper's own plugins: Home's weather, news and clock, and the ambient light tile, speaking the plugin API through the
//! broker's sessions (`local.rs`).
//!
//! What they publish is what any Luma device can draw of the same plugin: a fact card of the weather (the temperature first,
//! then the conditions, today's high and low, and the details), an event card with the top headline, a fact card with the time
//! and the date, a quick tile that switches the ambient light. The TV's Home keeps drawing its own richer weather and news cards
//! from the helper's `weather` and `news` topics (a card of four facts cannot carry a forecast), and the plugin cards are what
//! the contract allows: they appear wherever plugin cards are drawn (the Notification Centre's neighbours, the cards sheet, another
//! device).
//!
//! **Saved first.** The plugins read the topics the feeds publish (`weather`, `news`), and the feeds put the saved documents on
//! the hub before the screen is known and before any network, so a card exists from the start of the helper from what was saved
//! and is replaced by each fetch. A reading the feeds would not show (past `expiresAt`) is not a card either, and a card expires
//! on the session's clock when its reading does.
//!
//! **No work while the screen is off.** The clock card is the only thing that moves by itself, once a minute, and only while the
//! screen is on; every other change is an event on a topic.

use super::local::{Handler, Local, Spec};
use super::session::Shared;
use crate::ctx::Ctx;
use crate::homefeeds;
use crate::screen;
use crate::timeutil::now_ms;
use plugin_api::wire::ToPlugin;
use serde_json::{Value as Json, json};
use settings_schema::{SettingChange, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub const WEATHER: Spec = Spec {
    id: "weather",
    name: "Weather",
    version: "1",
    permissions: &["cards"],
};
pub const NEWS: Spec = Spec {
    id: "news",
    name: "News",
    version: "1",
    permissions: &["cards"],
};
pub const CLOCK: Spec = Spec {
    id: "clock",
    name: "Clock",
    version: "1",
    permissions: &["cards"],
};
pub const AMBIENT: Spec = Spec {
    id: "ambient",
    name: "Ambient light",
    version: "1",
    permissions: &["tiles"],
};

/// The topics the plugins follow.
const TOPICS: [&str; 4] = ["weather", "news", screen::TOPIC, crate::ambient::TOPIC];
/// The longest the thread sleeps, so that a stop, a deadline and a clock that jumped are all noticed.
const LONGEST_SLEEP: Duration = Duration::from_secs(30);
/// The settings owner the ambient tile switches.
const AMBIENT_OWNER: &str = "base.ambient";

// -- the words ---------------------------------------------------------------------------------------------------------------

/// `text` cut to `max` characters, with an ellipsis where it was cut (the contract counts characters, not bytes).
pub fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.truncate(out.trim_end().len());
    out.push('\u{2026}');
    out
}

/// The card's seconds to live: until the reading's `expiresAt` (Unix milliseconds), at least one; `None` when it is over.
fn lives(doc: &Json, now_s: i64) -> Option<u32> {
    let expires = i64::try_from(doc.get("expiresAt")?.as_u64()? / 1000).ok()?;
    let left = expires - now_s;
    (left >= 1).then(|| u32::try_from(left).unwrap_or(u32::MAX))
}

fn text<'a>(value: &'a Json, key: &str) -> &'a str {
    value.get(key).and_then(Json::as_str).unwrap_or("")
}

/// The weather card from the `weather` topic: `None` while there is no reading the feeds would show.
pub fn weather_card(topic: &Json, now_s: i64) -> Option<Json> {
    let doc = topic.get("doc").filter(|d| d.is_object())?;
    let display = doc.get("display")?;
    let live = lives(doc, now_s)?;
    let now = text(display, "temp");
    if now.is_empty() {
        return None;
    }
    let mut facts = vec![json!({"label": "Now", "value": clip(now, 64)})];
    let cond = text(display, "cond");
    if !cond.is_empty() {
        facts.push(json!({"label": "Conditions", "value": clip(cond, 64)}));
    }
    if let Some(today) = display.get("days").and_then(|d| d.get(0)) {
        let (hi, lo) = (text(today, "hi"), text(today, "lo"));
        if !hi.is_empty() && !lo.is_empty() {
            facts.push(json!({"label": "Today", "value": clip(&format!("{hi} / {lo}"), 64)}));
        }
    }
    let stats = text(display, "stats");
    if !stats.is_empty() {
        facts.push(json!({"label": "Details", "value": clip(stats, 64)}));
    }
    let kind = text(display, "nowKind");
    let icon = match kind {
        "moon" | "partly-night" => "moon",
        "sun" | "partly" => "sun",
        _ => "cloud",
    };
    let role = if kind == "storm" { "warning" } else { "info" };
    let place = text(display, "place");
    let title = if place.is_empty() {
        "Weather".to_owned()
    } else {
        format!("Weather \u{b7} {place}")
    };
    Some(json!({
        "t": "card.put", "id": "weather", "size": "half", "priority": "new", "expires_in_secs": live,
        "content": {"type": "facts", "title": clip(&title, 128), "icon": format!("base.icons/{icon}"), "role": role, "facts": facts},
        "open": {"view": "main"},
        "actions": [{"action": "refresh", "label": "Refresh"}]
    }))
}

/// The news card from the `news` topic: the top story, as an event.
pub fn news_card(topic: &Json, now_s: i64) -> Option<Json> {
    let doc = topic.get("doc").filter(|d| d.is_object())?;
    let live = lives(doc, now_s)?;
    let top = doc.get("items")?.get(0)?;
    let title = text(top, "title");
    if title.is_empty() {
        return None;
    }
    let mut content = json!({
        "type": "event", "title": clip(title, 128), "icon": "base.icons/newspaper", "role": "info"
    });
    let source = text(top, "src");
    if !source.is_empty() {
        content["detail"] = json!(clip(source, 256));
    }
    if let Some(published) = top.get("pubMs").and_then(Json::as_u64) {
        content["at"] = json!(published / 1000);
    }
    Some(json!({
        "t": "card.put", "id": "top", "size": "full", "priority": "new", "expires_in_secs": live,
        "content": content,
        "open": {"view": "main"},
        "actions": [{"action": "refresh", "label": "Refresh"}]
    }))
}

/// A moment in the TV's own time zone, to the minute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    /// 1 to 12.
    pub month: u32,
    pub day: u32,
    /// 0 is Sunday.
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
}

/// `unix_s` in the time zone the system has (musl reads `TZ` or `/etc/localtime`).
pub fn local_time(unix_s: i64) -> Option<LocalTime> {
    let t = libc::time_t::try_from(unix_s).ok()?;
    // SAFETY: `tm` is ours to fill; localtime_r writes through the pointer it is given and keeps nothing.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are valid for the call.
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return None;
    }
    Some(LocalTime {
        year: tm.tm_year + 1900,
        month: u32::try_from(tm.tm_mon).ok()? + 1,
        day: u32::try_from(tm.tm_mday).ok()?,
        weekday: u32::try_from(tm.tm_wday).ok()?,
        hour: u32::try_from(tm.tm_hour).ok()?,
        minute: u32::try_from(tm.tm_min).ok()?,
    })
}

const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// `9:56 PM` or `21:56`, and `Thursday, October 22`.
pub fn clock_words(t: LocalTime, twelve_hour: bool) -> (String, String) {
    let time = if twelve_hour {
        let hour = match t.hour % 12 {
            0 => 12,
            h => h,
        };
        format!(
            "{hour}:{:02} {}",
            t.minute,
            if t.hour < 12 { "AM" } else { "PM" }
        )
    } else {
        format!("{:02}:{:02}", t.hour, t.minute)
    };
    let date = format!(
        "{}, {} {}",
        DAYS[(t.weekday % 7) as usize],
        MONTHS[((t.month.max(1) - 1) % 12) as usize],
        t.day
    );
    (time, date)
}

/// The clock card. It lives two minutes, so that a clock whose thread has stopped is not left telling a wrong time.
pub fn clock_card(t: LocalTime, twelve_hour: bool) -> Json {
    let (time, date) = clock_words(t, twelve_hour);
    json!({
        "t": "card.put", "id": "time", "size": "half", "priority": "new", "expires_in_secs": 120,
        "content": {"type": "facts", "title": "Clock", "icon": "base.icons/clock", "role": "neutral",
                    "facts": [{"label": "Time", "value": time}, {"label": "Date", "value": date}]},
        "open": {"view": "main"}
    })
}

/// Whether Home's clock writes 12-hour time: `clock.twelveHour` of the owner's `theme.json` (Home's own default is yes).
pub fn twelve_hour(theme_json: &str) -> bool {
    serde_json::from_str::<Json>(theme_json)
        .ok()
        .and_then(|t| t.get("clock")?.get("twelveHour")?.as_bool())
        .unwrap_or(true)
}

/// The ambient light tile from the `ambient` topic: `None` (no tile) while no light is set up to drive.
pub fn ambient_tile(topic: &Json) -> Option<Json> {
    if !topic
        .get("available")
        .and_then(Json::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let enabled = topic
        .get("enabled")
        .and_then(Json::as_bool)
        .unwrap_or(false);
    let level = topic
        .get("brightness")
        .and_then(Json::as_i64)
        .unwrap_or(100);
    let hint = if !enabled {
        "Off".to_owned()
    } else {
        match (text(topic, "state"), text(topic, "mode")) {
            ("testing", _) => "Test pattern".to_owned(),
            ("waiting", _) => "Waiting for the picture".to_owned(),
            (_, "colour") => format!("Still colour \u{b7} {level}%"),
            (_, "picture-off-home") => format!("Follows the picture, off on Home \u{b7} {level}%"),
            _ => format!("Follows the picture \u{b7} {level}%"),
        }
    };
    Some(json!({
        "t": "tile.put", "id": "ambient", "label": "Ambient light", "icon": "base.icons/lamp",
        "state": {"type": if enabled { "on" } else { "off" }}, "action": "toggle", "hint": hint
    }))
}

// -- the thread --------------------------------------------------------------------------------------------------------------

/// Puts `card` for `local` when it differs from what was put last, or takes the card away when there is none: a card that has not
/// changed is not put again (each put raises its revision, and an idle TV would otherwise churn its topics).
struct Put {
    last: BTreeMap<&'static str, Option<Json>>,
}

impl Put {
    fn new() -> Self {
        Self {
            last: BTreeMap::new(),
        }
    }

    /// `kind` is `card` or `tile`: what is taken away with `{kind}.remove`.
    fn set(&mut self, local: &Local, kind: &'static str, id: &'static str, message: Option<Json>) {
        let key: &'static str = match (kind, id) {
            ("card", "weather") => "card:weather",
            ("card", "top") => "card:top",
            ("card", "time") => "card:time",
            _ => "tile:ambient",
        };
        if self.last.get(key) == Some(&message) {
            return;
        }
        let said = match &message {
            Some(message) => local.say(message.clone()),
            None if self.last.get(key).is_some_and(Option::is_some) => {
                local.say(json!({"t": format!("{kind}.remove"), "id": id}))
            }
            None => Ok(()),
        };
        match said {
            Ok(()) => {
                self.last.insert(key, message);
            }
            Err(e) => crate::log_warn!("plugins: {e}"),
        }
    }
}

/// What the thread holds.
struct Plugins {
    weather: Option<Arc<Local>>,
    news: Option<Arc<Local>>,
    clock: Option<Arc<Local>>,
    ambient: Option<Arc<Local>>,
}

impl Plugins {
    fn all(&self) -> impl Iterator<Item = &Arc<Local>> {
        [&self.weather, &self.news, &self.clock, &self.ambient]
            .into_iter()
            .flatten()
    }
}

fn start(
    shared: &Arc<Shared>,
    spec: &Spec,
    register: Json,
    handler: Handler,
) -> Option<Arc<Local>> {
    match Local::start(shared, spec, register, handler) {
        Ok(local) => Some(local),
        Err(e) => {
            crate::log_warn!("plugins: first-party plugin not started: {e}");
            None
        }
    }
}

/// Handles `refresh` by asking the feeds to fetch now, and refuses anything else in words.
fn refresh_handler(feeds: Sender<homefeeds::Cmd>) -> Handler {
    Arc::new(move |local, message| {
        if let ToPlugin::ActionInvoke { op, action, .. } = message {
            let result = if action == "refresh" {
                feeds
                    .send(homefeeds::Cmd::Refresh)
                    .map_err(|_| "the feeds are not running")
            } else {
                Err("this plugin has no such action")
            };
            if let Err(e) = local.answer(op, result) {
                crate::log_warn!("plugins: {e}");
            }
        }
    })
}

/// Switches the ambient light on or off: flips `enabled` of `base.ambient`, the way the Settings page does.
fn ambient_handler(ctx: &Ctx) -> Handler {
    let settings = ctx.settings.clone();
    Arc::new(move |local, message| {
        if let ToPlugin::ActionInvoke { op, action, .. } = message {
            let result = if action == "toggle" {
                toggle_ambient(&settings)
            } else {
                Err("this plugin has no such action".to_owned())
            };
            if let Err(e) = local.answer(op, result.as_ref().map(|_| ()).map_err(String::as_str)) {
                crate::log_warn!("plugins: {e}");
            }
        }
    })
}

fn toggle_ambient(settings: &crate::settings::SettingsStore) -> Result<(), String> {
    let enabled = settings
        .effective(AMBIENT_OWNER)
        .and_then(|values| match values.get("enabled") {
            Some(Value::Bool(b)) => Some(*b),
            _ => None,
        })
        .ok_or_else(|| "the ambient light has no settings".to_owned())?;
    let change = SettingChange {
        key: "enabled"
            .parse()
            .map_err(|_| "no such setting".to_owned())?,
        value: Value::Bool(!enabled),
    };
    settings
        .apply(AMBIENT_OWNER, vec![change], None)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Starts the plugins and the thread that keeps their cards and tile current. `shared` is the broker's (`Plugins::shared`).
pub fn spawn(
    ctx: &Ctx,
    shared: Arc<Shared>,
    feeds: Sender<homefeeds::Cmd>,
) -> std::io::Result<JoinHandle<()>> {
    let ctx = ctx.clone();
    thread::Builder::new()
        .name("plugins-1p".to_owned())
        .stack_size(512 * 1024)
        .spawn(move || run(&ctx, &shared, feeds))
}

fn run(ctx: &Ctx, shared: &Arc<Shared>, feeds: Sender<homefeeds::Cmd>) {
    let view = json!({"id": "main", "title": "", "presentation": "sheet"});
    let one_view = |title: &str| {
        let mut v = view.clone();
        v["title"] = json!(title);
        json!([v])
    };
    let refresh = json!([{"key": "refresh", "label": "Refresh"}]);
    let plugins = Plugins {
        weather: start(
            shared,
            &WEATHER,
            json!({"title": "Weather", "actions": refresh, "views": one_view("Weather")}),
            refresh_handler(feeds.clone()),
        ),
        news: start(
            shared,
            &NEWS,
            json!({"title": "News", "actions": refresh, "views": one_view("News")}),
            refresh_handler(feeds),
        ),
        clock: start(
            shared,
            &CLOCK,
            json!({"title": "Clock", "views": one_view("Clock")}),
            Arc::new(|_, _| {}),
        ),
        ambient: start(
            shared,
            &AMBIENT,
            json!({"title": "Ambient light", "actions": [{"key": "toggle", "label": "Switch the ambient light"}]}),
            ambient_handler(ctx),
        ),
    };
    let hub = &ctx.hub;
    let theme_file = ctx.paths.data_dir.join("theme").join("theme.json");
    let topics: Vec<String> = TOPICS.iter().map(|t| (*t).to_owned()).collect();
    let mut put = Put::new();
    let mut after = hub.events_after(None, None, None).seq;
    let mut changed: Vec<String> = topics.clone();
    let mut clock_minute = u64::MAX;
    loop {
        if ctx.stop.is_set() {
            break;
        }
        let now = now_ms();
        let now_s = i64::try_from(now / 1000).unwrap_or(i64::MAX);
        let on = screen::is_on(hub);
        let wants = |topic: &str| changed.iter().any(|t| t == topic);
        if let Some(local) = &plugins.weather
            && wants("weather")
        {
            let card = hub.get("weather").and_then(|t| weather_card(&t, now_s));
            put.set(local, "card", "weather", card);
        }
        if let Some(local) = &plugins.news
            && wants("news")
        {
            let card = hub.get("news").and_then(|t| news_card(&t, now_s));
            put.set(local, "card", "top", card);
        }
        if let Some(local) = &plugins.ambient
            && wants(crate::ambient::TOPIC)
        {
            let tile = hub
                .get(crate::ambient::TOPIC)
                .and_then(|t| ambient_tile(&t));
            put.set(local, "tile", "ambient", tile);
        }
        // The clock is only kept current while the screen is on, and a card that was put before it went off simply runs out.
        if let Some(local) = &plugins.clock
            && on
            && (wants(screen::TOPIC) || now / 60_000 != clock_minute)
        {
            clock_minute = now / 60_000;
            let twelve = std::fs::read_to_string(&theme_file).map_or(true, |t| twelve_hour(&t));
            let card = local_time(now_s).map(|t| clock_card(t, twelve));
            put.set(local, "card", "time", card);
        }
        for local in plugins.all() {
            local.tick();
        }
        // Sleep until something is due: the next minute (screen on), a card or a question running out, or a long poll's worth.
        let mut wait = LONGEST_SLEEP;
        if on && plugins.clock.is_some() {
            wait = wait.min(Duration::from_millis(60_000 - now % 60_000 + 20));
        }
        for deadline in plugins.all().filter_map(|l| l.next_deadline_ms()) {
            wait = wait.min(Duration::from_millis(deadline.saturating_sub(now_ms()) + 5));
        }
        let batch = hub.wait_events(
            Some(after),
            None,
            Some(&topics),
            wait.max(Duration::from_millis(50)),
        );
        after = batch.seq;
        changed = if batch.reset {
            topics.clone()
        } else {
            batch.events.iter().map(|e| e.topic.clone()).collect()
        };
        if !on && screen::is_on(hub) {
            // The screen came on while this slept: the clock is due at once.
            clock_minute = u64::MAX;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weather_topic(expires_at: u64) -> Json {
        json!({"doc": {"schema": 1, "fetchedAt": 1, "staleAt": 2, "expiresAt": expires_at,
            "display": {"place": "Atlanta", "temp": "72\u{b0}", "cond": "Partly cloudy",
                        "stats": "Feels like 70\u{b0} \u{b7} 62% humidity \u{b7} 8 mph", "nowKind": "partly",
                        "days": [{"day": "Today", "kind": "sun", "hi": "81\u{b0}", "lo": "62\u{b0}"},
                                 {"day": "Fri", "kind": "rain", "hi": "78\u{b0}", "lo": "60\u{b0}"}]}},
            "status": {"failures": 0, "error": null, "lastAttemptAt": 1}})
    }

    #[test]
    fn the_weather_card_is_the_temperature_then_the_rest_and_runs_out_with_the_reading() {
        let card = weather_card(&weather_topic(1_800_021_600_000), 1_800_000_000).unwrap();
        assert_eq!(card["id"], "weather");
        assert_eq!(card["expires_in_secs"], 21_600);
        assert_eq!(card["content"]["title"], "Weather \u{b7} Atlanta");
        assert_eq!(card["content"]["icon"], "base.icons/sun");
        let values: Vec<&str> = card["content"]["facts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["value"].as_str().unwrap())
            .collect();
        assert_eq!(
            values,
            [
                "72\u{b0}",
                "Partly cloudy",
                "81\u{b0} / 62\u{b0}",
                "Feels like 70\u{b0} \u{b7} 62% humidity \u{b7} 8 mph"
            ]
        );
        assert_eq!(card["actions"][0]["action"], "refresh");
        // A reading the feeds would no longer show is no card, and neither is an empty topic.
        assert_eq!(
            weather_card(&weather_topic(1_800_000_000_000), 1_800_000_000),
            None
        );
        assert_eq!(weather_card(&json!({"doc": null, "status": {}}), 0), None);
        assert_eq!(weather_card(&json!({}), 0), None);
    }

    #[test]
    fn the_news_card_is_the_top_story_with_its_source_and_time() {
        let topic = json!({"doc": {"expiresAt": 1_800_021_600_000_u64, "items": [
            {"title": "Newborn twins fight for survival as conflict fuels Yemen's hunger crisis", "src": "BBC News",
             "pubMs": 1_799_960_400_000_u64, "summary": "x", "link": "https://x", "img": "", "tag": "WORLD", "pub": ""},
            {"title": "Second", "src": "BBC News"}]}, "status": {}});
        let card = news_card(&topic, 1_800_000_000).unwrap();
        assert_eq!(card["id"], "top");
        assert_eq!(card["size"], "full");
        assert_eq!(card["content"]["type"], "event");
        assert_eq!(card["content"]["detail"], "BBC News");
        assert_eq!(card["content"]["at"], 1_799_960_400);
        assert_eq!(
            news_card(
                &json!({"doc": {"expiresAt": 1_800_021_600_000_u64, "items": []}}),
                1_800_000_000
            ),
            None
        );
        // A headline longer than the contract allows is cut by characters, not bytes.
        let long = "\u{e9}".repeat(200);
        let cut = news_card(&json!({"doc": {"expiresAt": 1_800_021_600_000_u64, "items": [{"title": long, "src": "S"}]}}), 1_800_000_000).unwrap();
        assert_eq!(
            cut["content"]["title"].as_str().unwrap().chars().count(),
            128
        );
        assert!(
            cut["content"]["title"]
                .as_str()
                .unwrap()
                .ends_with('\u{2026}')
        );
    }

    #[test]
    fn the_clock_says_the_time_and_the_date_in_the_homes_hours() {
        let t = LocalTime {
            year: 2026,
            month: 10,
            day: 22,
            weekday: 4,
            hour: 21,
            minute: 56,
        };
        assert_eq!(
            clock_words(t, true),
            ("9:56 PM".to_owned(), "Thursday, October 22".to_owned())
        );
        assert_eq!(clock_words(t, false).0, "21:56");
        let midnight = LocalTime {
            hour: 0,
            minute: 5,
            ..t
        };
        assert_eq!(clock_words(midnight, true).0, "12:05 AM");
        assert_eq!(
            clock_words(
                LocalTime {
                    hour: 12,
                    minute: 0,
                    ..t
                },
                true
            )
            .0,
            "12:00 PM"
        );
        let card = clock_card(t, true);
        assert_eq!(card["content"]["facts"][0]["value"], "9:56 PM");
        assert_eq!(card["expires_in_secs"], 120);
        assert!(
            twelve_hour("{}")
                && twelve_hour("not json")
                && !twelve_hour(r#"{"clock":{"twelveHour":false}}"#)
        );
    }

    #[test]
    fn the_system_has_a_local_time() {
        let t = local_time(1_800_000_000).unwrap();
        assert!((1..=12).contains(&t.month) && t.hour < 24 && t.minute < 60 && t.weekday < 7);
        assert_eq!(t.year, 2027);
    }

    #[test]
    fn the_ambient_tile_exists_only_with_a_light_to_drive_and_says_what_it_does() {
        assert_eq!(
            ambient_tile(&json!({"enabled": true, "available": false})),
            None
        );
        let on = ambient_tile(&json!({"enabled": true, "available": true, "mode": "picture", "state": "running", "brightness": 70})).unwrap();
        assert_eq!(on["state"], json!({"type": "on"}));
        assert_eq!(on["hint"], "Follows the picture \u{b7} 70%");
        assert_eq!(on["action"], "toggle");
        let off = ambient_tile(&json!({"enabled": false, "available": true, "mode": "picture", "state": "off", "brightness": 70})).unwrap();
        assert_eq!(off["state"], json!({"type": "off"}));
        assert_eq!(off["hint"], "Off");
        let waiting = ambient_tile(
            &json!({"enabled": true, "available": true, "state": "waiting", "brightness": 40}),
        )
        .unwrap();
        assert_eq!(waiting["hint"], "Waiting for the picture");
        let still = ambient_tile(&json!({"enabled": true, "available": true, "mode": "colour", "state": "running", "brightness": 40})).unwrap();
        assert_eq!(still["hint"], "Still colour \u{b7} 40%");
    }

    #[test]
    fn clip_counts_characters() {
        assert_eq!(clip("short", 10), "short");
        assert_eq!(clip("abcdefghij", 10), "abcdefghij");
        assert_eq!(clip("abcdefghijk", 10), "abcdefghi\u{2026}");
        assert_eq!(clip("  padded  ", 10), "padded");
        assert_eq!(clip("ab cd ef gh ij", 7), "ab cd\u{2026}");
    }
}
