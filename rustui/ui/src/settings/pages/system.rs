//! System (`settings/PageSystem.qml`, `docs/mocks/30-settings-system`): the handful of TV-wide settings a person reaches for, and where the
//! rest of LG's Settings is; five tabs. Keys and categories are the ones LG's own Settings app uses. TV information, the software update,
//! privacy, self-diagnosis and reset are the Support page, the locks' state the Safety page; what is left in LG's app (the account, the
//! apps, the channel lists) is behind the one row that opens it. The OLED care tab holds LG's own panel care and, under a head of its own,
//! LumaTV's (the theme's `oledcare` keys, read by the compositor within two seconds; each is off until its TV check has passed).

use super::util::{atsc3, atsc3_read, panel_hours, panel_reads, panel_shown};
use crate::host::power::SLEEP_LABELS;
use crate::settings::calendar::{self, Field, MONTHS};
use crate::settings::ctx::{Ctx, Spec, lg_launch, toast};
use crate::settings::names;
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Op, PageArg, Pick, Row, ChoiceSpec, patch_of, settings};
use serde_json::{Value, json};
use std::rc::Rc;

pub fn page() -> Page {
    Page::new("system", "System", "settings").rail().tabs(&["General", "Time & power", "OLED care", "Accessibility", "About"]).reads(reads).rows(rows).summary(summary)
}

fn summary(c: &Ctx) -> String {
    c.power.sleep_left()
}

fn reads(_: &Ctx) -> Vec<Read> {
    let mut r = vec![
        Read::lg("", &["localeInfo"]),
        Read::lg(
            "time",
            &["timeZone", "noOperationAutoOff", "autoOff15Min", "autoOff2HourOnTimer", "automaticStandBy", "onTimerEnable", "onTimerHour", "onTimerMinute", "offTimerEnable", "offTimerHour", "offTimerMinute"],
        ),
        Read::lg(
            "option",
            &["quickStartMode", "audioGuidance", "highContrast", "focusEmphasis", "screenMagnification", "adjustments", "emergencyInformationAtsc30", "emergencyInformationLanguageAtsc30", "screenOff"],
        ),
        Read::lg("picture", &["screenShift", "logoLuminanceAdjust", "scheduledPixelCleaning"]),
        Read::lg("other", &["oledCareMode", "oledCareRecommendation"]),
        Read::lg("caption", &["captionEnable"]),
        Read::lg("sound", &["audioDescription"]),
        Read::lg("network", &["deviceName"]),
        Read::Sleep,
        atsc3_read(),
    ];
    r.extend(panel_reads());
    r
}

const LOCALES: [(&str, &str); 4] = [("en-US", "English (US)"), ("en-GB", "English (UK)"), ("es-US", "Spanish (US)"), ("fr-FR", "French")];

fn ui_locale(c: &Ctx) -> String {
    c.lg.get("localeInfo").and_then(|v| v.pointer("/locales/UI")).and_then(Value::as_str).unwrap_or("").to_string()
}

/// The time of a timer as its row shows it: "Off" or "7:30 AM".
fn timer_text(c: &Ctx, which: &str) -> String {
    if c.lg.opt_text(&format!("time.{which}TimerEnable")).as_deref() != Some("on") {
        return "Off".into();
    }
    calendar::clock_text(c.lg.num(&format!("time.{which}TimerHour"), 0.0) as u32, c.lg.num(&format!("time.{which}TimerMinute"), 0.0) as u32)
}

/// The switches of `oledcare` that are on when the theme does not say (the compositor's own defaults, `paneldim::Settings`).
const CARE_ON_BY_DEFAULT: &[&str] = &["panelDim"];

/// One of LumaTV's panel-care switches (the theme's `oledcare`).
fn care(c: &Ctx, key: &str, icon: &str, label: &str, help: &str) -> Row {
    let key = key.to_string();
    let on = c.theme_at("oledcare").and_then(|o| o.get(&key)).and_then(Value::as_bool).unwrap_or(CARE_ON_BY_DEFAULT.contains(&key.as_str()));
    Row::toggle(&format!("care:{key}")).tab(2).icon(icon).label(label).help(help).on(on).act(move |a| match a {
        Arg::Toggle(v) => vec![Op::Patch(patch_of(&format!("oledcare.{key}"), json!(v)))],
        _ => Vec::new(),
    })
}

fn go(page: &str) -> Vec<Op> {
    vec![Op::Open { page: page.into(), arg: PageArg::None }]
}

fn rows(c: &Ctx) -> Vec<Row> {
    let mut rows = Vec::new();
    // ---- General
    let ui = ui_locale(c);
    rows.push(
        Row::link("language")
            .tab(0)
            .icon("languages")
            .label("Language & location")
            .help("Menu, audio and keyboard languages, the zip code and the LG services country.")
            .value(&names::name_or(&LOCALES, &ui, ""))
            .enabled(c.lg.has("localeInfo"))
            .does(go("language")),
    );
    rows.push(c.seg(Spec::new("time.noOperationAutoOff").tab(0).icon("power").label("No Operation Auto Off").help("No key for this long turns the TV off.").options(&["Off", "3 h", "6 h", "12 h", "24 h"]).values(&["off", "3", "6", "12", "24"]).lg_shown()));
    rows.push(Row::link("safety").tab(0).icon("shield").label("Safety").help("Which locks are on. A lock changes only with the PIN, in LG's screen.").does(go("safety")));
    rows.push(Row::link("capture").tab(0).icon("camera").label("Screenshots & recording").does(go("capture")));
    rows.push(lg_launch("lg", "Open LG settings", "", "").tab(0).help("LG's full Settings app, themed. For what LumaTV does not cover: the account, apps and channel lists."));

    // ---- Time & power
    let labels: Vec<String> = SLEEP_LABELS.iter().map(|s| (*s).to_string()).collect();
    let mut sleep = Row::stepper("sleep", &labels).tab(1).icon("hourglass").label("Sleep Timer").help("The TV turns itself off after this long.").detail(&c.power.sleep_left()).index(Some(c.power.sleep_index()));
    sleep = sleep.act(|a| match a {
        Arg::Index(i) => vec![Op::Sleep(i)],
        _ => Vec::new(),
    });
    rows.push(sleep);
    let now = calendar::local((c.now / 1000) as i64);
    rows.push(Row::link("clock").tab(1).icon("clock").label("Time & Date").value(&calendar::clock_text(now.hour, now.minute)).does(go("clock")));
    rows.push(Row::link("poweron").tab(1).icon("power").label("Power On Timer").enabled(c.lg.has("time.onTimerEnable")).value(&timer_text(c, "on")).does(go("poweron")));
    rows.push(Row::link("poweroff").tab(1).icon("power").label("Power Off Timer").enabled(c.lg.has("time.offTimerEnable")).value(&timer_text(c, "off")).does(go("poweroff")));
    rows.push(c.toggle(Spec::new("time.autoOff15Min").tab(1).icon("power").label("No Signal Auto Off").help("Switches off after 15 minutes without a picture.").lg_shown()));
    rows.push(c.toggle(Spec::new("time.autoOff2HourOnTimer").tab(1).icon("hourglass").label("Auto Power Off after 4 Hours").help("Turns the TV off after 4 hours without a key press when it was turned on by a timer.").lg_shown()));
    rows.push(c.choice(Spec::new("time.automaticStandBy").tab(1).icon("hourglass").label("Auto Power Off").names(names::AUTOMATIC_STAND_BY)));
    let mut off = Row::action("screenoff", "Turn off").tab(1).icon("monitor").label("Screen Off").help("Turns the picture off and keeps the sound. Any key brings it back.");
    off.enabled = c.lg.usable("option.screenOff");
    off.visible = c.lg.shown("option.screenOff");
    rows.push(off.does(vec![Op::Lg { ds: "option.screenOff".into(), value: json!("on"), app: None }]));
    rows.push(c.toggle(Spec::new("option.quickStartMode").tab(1).icon("zap").label("Quick Start+").help("Wakes in about 2 s from standby. Uses a little power while off.")));

    // ---- OLED care
    let hours = panel_hours(c);
    let mut panel = Row::info("panelhours").tab(2).icon("clock").label("Panel hours").help("How long the panel has been on, as LG counts it. It does not reset.").value(if hours.is_empty() { "Not read" } else { &hours });
    panel.chevron = false;
    panel.visible = panel_shown(c);
    rows.push(panel);
    rows.push(c.toggle(Spec::new("picture.screenShift").tab(2).icon("scan-eye").label("Shift the picture slightly").help("Moves the picture a pixel now and then so the panel wears evenly.")));
    rows.push(c.seg(Spec::new("picture.logoLuminanceAdjust").tab(2).icon("contrast").label("Dim logos and subtitles").help("Lowers the brightness of static logos and subtitles so they do not burn in.").options(&["Off", "Light", "Strong"]).values(&["off", "light", "strong"])));
    rows.push(c.choice(Spec::new("other.oledCareMode").tab(2).icon("shield-check").label("Care Picture Settings").names(names::OLED_CARE).help("A picture set for the eyes or for the room: comfort, theater or kids. It replaces the energy saving step.")));
    let mut pixel = Row::toggle("pixelcleaning").tab(2).icon("sparkles").label("Clean pixels when the TV is turned off").help("The panel runs a cleaning the next time the TV is turned off, and the TV stays off for about an hour.").on(c.lg.is("picture.scheduledPixelCleaning", "on"));
    pixel.detail = c.unavailable("picture.scheduledPixelCleaning", "");
    pixel.enabled = c.lg.usable("picture.scheduledPixelCleaning");
    pixel.visible = c.lg.shown("picture.scheduledPixelCleaning");
    rows.push(pixel.act(|a| match a {
        Arg::Toggle(false) => vec![Op::Lg { ds: "picture.scheduledPixelCleaning".into(), value: json!("off"), app: None }],
        Arg::Toggle(true) => vec![Op::Lg { ds: "picture.scheduledPixelCleaning".into(), value: json!("on"), app: None }, toast("Pixel cleaning runs when the TV is turned off, and takes about an hour")],
        _ => Vec::new(),
    }));
    rows.push(c.toggle(Spec::new("other.oledCareRecommendation").tab(2).icon("lightbulb").label("Suggest picture care settings").help("Suggests a picture setting for eye strain or the room, from how the TV is watched.").lg_shown()));
    rows.push(Row::head("LumaTV panel care", 2));
    rows.push(care(c, "enabled", "shield-check", "Protect LumaTV screens", "Moves LumaTV's own screens a pixel now and then and dims Home when it sits idle."));
    rows.push(care(c, "appDim", "sun-medium", "Dim still apps", "Dims an app a little when its picture has not changed for a few minutes. Video and any key bring it back."));
    rows.push(care(c, "appShift", "waves", "Shift still apps", "Slowly moves an app's picture a few pixels while it sits still. A key or video puts it back."));
    rows.push(care(c, "boxDim", "sun-dim", "Dim bright still areas", "Dims only the bright parts of a picture that stays still, such as menu boxes. Video is never touched."));
    rows.push(care(c, "panelDim", "sun", "Dim with the panel", "The screensaver and an idle Home dim the panel's own light instead of drawing darker. Any key puts it back."));
    rows.push(care(c, "refreshGuard", "rotate-ccw", "Protect panel refresh", "Keeps LG's panel refresh from being cut short when the TV is turned on just after it turns off."));
    let refresh = panel_refresh(c);
    let mut pr = Row::info("panelrefresh").tab(2).icon("history").label("Panel refresh").help("LG refreshes the panel by itself, about 10 minutes after the TV is turned off, each time the panel has run a few hours. Turning the TV on in that time cuts it short.").value(&refresh);
    pr.chevron = false;
    pr.visible = !refresh.is_empty();
    rows.push(pr);

    // ---- Accessibility
    rows.push(Row::head("Vision", 3));
    let guide = c.lg.opt_text("option.audioGuidance").map(|v| names::name_or(names::ONOFF, &v, "")).unwrap_or_default();
    rows.push(Row::link("audioguide").tab(3).icon("audio-lines").label("Audio Guidance").enabled(c.lg.has("option.audioGuidance")).value(&guide).does(go("audioguide")));
    rows.push(c.display("highContrast", Spec::new("").tab(3).icon("contrast").label("Text with High Contrast").help("Shows the menus' text in a stronger colour.")));
    rows.push(c.display("focusEmphasis", Spec::new("").tab(3).icon("scan-eye").label("Focus with High Contrast").help("Marks the selected item more strongly.")));
    rows.push(c.display("blackAndWhite", Spec::new("").tab(3).icon("moon").label("Grayscale").help("Turns every screen black and white.")));
    rows.push(c.display("colorInversion", Spec::new("").tab(3).icon("blend").label("Invert Colors").help("Inverts the colour of every screen.")));
    rows.push(c.display("screenMagnification", Spec::new("").tab(3).icon("search").label("Screen Zoom").help("Zooms the whole screen.")));
    rows.push(Row::head("Hearing", 3));
    let cc = c.lg.opt_text("caption.captionEnable").map(|v| names::name_or(names::ONOFF, &v, "")).unwrap_or_default();
    rows.push(Row::link("captions").tab(3).icon("type").label("Closed Caption").enabled(c.lg.has("caption.captionEnable")).value(&cc).does(go("captions")));
    rows.push(c.toggle(Spec::new("sound.audioDescription").tab(3).icon("audio-lines").label("Audio Description").lg_shown()));
    rows.push(Row::link("audiooptions").tab(3).icon("headphones").label("Audio options").does(go("audiooptions")));
    let mut emergency = Row::link("emergency").tab(3).icon("octagon-alert").label("Emergency Information").help("The emergency alert of NEXTGEN TV, and the language it is spoken in.").does(go("emergency"));
    emergency.visible = atsc3(c) && (c.lg.shown("option.emergencyInformationAtsc30") || c.lg.shown("option.emergencyInformationLanguageAtsc30"));
    rows.push(emergency);

    // ---- About
    let name = if c.lg.has("network.deviceName") { c.lg.opt_text("network.deviceName").unwrap_or_default() } else { "This TV".into() };
    let mut tv = Row::info("tvname").tab(4).icon("tv").label(&name).help("The name on the network and in Home Assistant.");
    tv.chevron = false;
    rows.push(tv);
    let mut helper = Row::info("helper").tab(4).icon("sparkles").label("LumaTV").value(if c.topics.helper_up { "Helper running" } else { "Helper stopped" });
    helper.chevron = false;
    rows.push(helper);
    rows.push(Row::link("about").tab(4).icon("tv").label("About this TV").help("Model, serial number, platform and software version.").does(go("about")));
    rows.push(Row::link("support").tab(4).icon("circle-help").label("Support").help("TV information, software update, privacy, self-diagnosis and reset.").does(go("support")));
    rows
}

/// "last run 3 h ago, 12 runs" from the helper's panel topic; nothing until LG has answered.
fn panel_refresh(c: &Ctx) -> String {
    let Some(p) = c.helper("panel").filter(|p| p.get("read").and_then(Value::as_bool) == Some(true)) else { return String::new() };
    let hours = p.get("usageHours").and_then(Value::as_f64).unwrap_or(0.0);
    let last = p.get("lastRunHours").and_then(Value::as_f64).unwrap_or(0.0);
    let runs = p.get("offrsCount").and_then(Value::as_i64).unwrap_or(0);
    format!("last run {} h ago, {} {}", (hours - last).round().max(0.0) as i64, runs, if runs == 1 { "run" } else { "runs" })
}

// ------------------------------------------------------------------------------------------------------------ Time & Date

pub fn clock() -> Page {
    Page::new("clock", "Time & Date", "clock").parent("system").back("System").hidden_from_search().reads(clock_reads).rows(clock_rows).summary(clock_summary)
}

fn clock_reads(_: &Ctx) -> Vec<Read> {
    vec![
        Read::lg("time", &["autoClock", "timeZone"]),
        Read::Watch { name: "prefs", uri: "luna://com.palm.systemservice/getPreferences", payload: json!({"keys": ["useNetworkTime", "timeZone"]}) },
        Read::call("zones", "luna://com.palm.systemservice/getPreferenceValues", json!({"key": "timeZone"})),
        Read::call("time", "luna://com.palm.systemservice/time/getEffectiveBroadcastTime", json!({})),
    ]
}

/// The clock as it is shown while it is being set: the draft of the press, else the TV's own time when the page opened, else now.
fn draft(c: &Ctx) -> i64 {
    c.local.get("draft").as_i64().or_else(|| c.docs.field("time", "adjustedUtc").as_i64()).unwrap_or((c.now / 1000) as i64)
}

fn clock_summary(c: &Ctx) -> String {
    let l = calendar::local(draft(c));
    calendar::clock_text(l.hour, l.minute)
}

fn automatic(c: &Ctx) -> bool {
    c.local.get("auto").as_bool().or_else(|| c.docs.field("prefs", "useNetworkTime").as_bool()).unwrap_or(true)
}

/// The zone in force: the pick just made, else the preference.
fn zone(c: &Ctx) -> Value {
    let z = c.local.get("zone");
    if z.is_object() { z.clone() } else { c.docs.field("prefs", "timeZone").clone() }
}

fn zone_name(z: &Value) -> String {
    ["City", "Description", "ZoneID"].iter().find_map(|k| z.get(k).and_then(Value::as_str).filter(|s| !s.is_empty())).unwrap_or("").to_string()
}

fn zones_of_country(c: &Ctx) -> Vec<Value> {
    let cc = zone(c).get("CountryCode").and_then(Value::as_str).unwrap_or("").to_string();
    c.docs.field("zones", "timeZone").as_array().map(|l| l.iter().filter(|z| cc.is_empty() || z.get("CountryCode").and_then(Value::as_str) == Some(cc.as_str())).cloned().collect()).unwrap_or_default()
}

fn clock_rows(c: &Ctx) -> Vec<Row> {
    let auto = automatic(c);
    let mut rows = vec![Row::toggle("auto").icon("clock").label("Set Automatically").on(auto).act(|a| match a {
        Arg::Toggle(v) => vec![
            Op::Set("auto".into(), json!(v)),
            Op::Lg { ds: "time.autoClock".into(), value: json!(if v { "on" } else { "off" }), app: None },
            Op::Act { uri: "luna://com.palm.systemservice/setPreferences".into(), payload: json!({"useNetworkTime": v}) },
        ],
        _ => Vec::new(),
    })];
    // the zone row opens the picker on the TV's zones for the country of the one in force
    let zones = zones_of_country(c);
    let current = zone(c).get("ZoneID").and_then(Value::as_str).unwrap_or("").to_string();
    let options: Vec<Pick> = zones.iter().map(|z| Pick::new(z.get("ZoneID").and_then(Value::as_str).unwrap_or(""), &zone_name(z))).collect();
    let by_id: Vec<(String, Value)> = zones.iter().map(|z| (z.get("ZoneID").and_then(Value::as_str).unwrap_or("").to_string(), z.clone())).collect();
    rows.push(Row::link("zone").icon("globe").label("Time Zone").value(&zone_name(&zone(c))).enabled(!options.is_empty()).does(vec![Op::Pick(Rc::new(ChoiceSpec {
        title: "Time Zone".into(),
        options,
        current,
        write: Box::new(move |id| match by_id.iter().find(|(i, _)| i == id) {
            Some((_, z)) => vec![
                Op::Act { uri: "luna://com.palm.systemservice/setPreferences".into(), payload: json!({"timeZone": z}) },
                Op::LgMany { category: "time".into(), settings: settings(&[("timeZone", z.clone())]), app: None },
                Op::Set("zone".into(), z.clone()),
            ],
            None => Vec::new(),
        }),
    }))]));
    let l = calendar::local(draft(c));
    let fields: [(Field, &str, &str, String); 5] = [
        (Field::Hour, "Hour", "clock", calendar::hour_text(l.hour)),
        (Field::Minute, "Minute", "clock", format!("{:02}", l.minute)),
        (Field::Year, "Year", "history", l.year.to_string()),
        (Field::Month, "Month", "history", MONTHS[l.month as usize].to_string()),
        (Field::Day, "Day", "history", l.day.to_string()),
    ];
    let base = draft(c);
    for (field, label, icon, text) in fields {
        let mut r = Row::nudge(&format!("{field:?}").to_lowercase()).icon(icon).label(label).value(&text).enabled(!auto);
        if auto {
            r.detail = "Set Automatically is on".into();
        }
        rows.push(r.act(move |a| match a {
            Arg::Nudge(d) => {
                let next = calendar::step(base, field, d);
                vec![
                    Op::Set("draft".into(), json!(next)),
                    // sent a second after the last press, as LG's is, so holding a key does not send every step
                    Op::Later { key: "settime".into(), ms: 1000, then: vec![Op::Act { uri: "luna://com.palm.systemservice/time/setSystemTime".into(), payload: json!({"utc": next}) }] },
                ]
            }
            _ => Vec::new(),
        }));
    }
    rows
}

// ------------------------------------------------------------------------------------------------------------ the timers

pub fn timer_on() -> Page {
    Page::new("poweron", "Power On Timer", "power").parent("system").back("System").hidden_from_search().tabs(&["Time", "Days"]).reads(|_| timer_reads("on")).rows(|c| timer_rows(c, "on"))
}

pub fn timer_off() -> Page {
    Page::new("poweroff", "Power Off Timer", "power").parent("system").back("System").hidden_from_search().tabs(&["Time", "Days"]).reads(|_| timer_reads("off")).rows(|c| timer_rows(c, "off"))
}

fn timer_reads(which: &'static str) -> Vec<Read> {
    let mut keys: Vec<&'static str> = match which {
        "on" => vec!["onTimerEnable", "onTimerHour", "onTimerMinute", "onTimerWeekday", "onTimerSource", "onTimerVolume"],
        _ => vec!["offTimerEnable", "offTimerHour", "offTimerMinute", "offTimerWeekday"],
    };
    keys.shrink_to_fit();
    vec![Read::lg("time", &keys)]
}

const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

fn timer_rows(c: &Ctx, which: &str) -> Vec<Row> {
    let pre = format!("time.{which}Timer");
    let mut rows = vec![c.toggle(Spec::new(&format!("{pre}Enable")).tab(0).icon("power").label("Timer"))];
    let nudge = |ds: String, label: &str, max: i64, text: fn(i64) -> String| {
        let mut r = Row::nudge(&ds).tab(0).icon("clock").label(label);
        r.visible = c.lg.shown(&ds);
        r.enabled = c.lg.usable(&ds);
        r.value = if c.lg.has(&ds) { text(c.lg.num(&ds, 0.0) as i64) } else { String::new() };
        let cur = c.lg.num(&ds, 0.0) as i64;
        r.act(move |a| match a {
            Arg::Nudge(d) => vec![Op::Lg { ds: ds.clone(), value: json!((cur + i64::from(d)).rem_euclid(max)), app: None }],
            _ => Vec::new(),
        })
    };
    rows.push(nudge(format!("{pre}Hour"), "Hour", 24, |h| calendar::hour_text(h as u32)));
    rows.push(nudge(format!("{pre}Minute"), "Minute", 60, |m| format!("{m:02}")));
    if which == "on" {
        let mut input = c.choice(Spec::new("time.onTimerSource").tab(0).icon("cable").label("Input").names(names::TIMER_INPUT).values(&["hdmi1", "hdmi2", "hdmi3", "hdmi4"]));
        input.visible = c.lg.shown("time.onTimerSource");
        rows.push(input);
        let mut vol = c.slider(Spec::new("time.onTimerVolume").tab(0).icon("volume-2").label("Volume").range(0.0, 100.0, 1.0));
        vol.visible = c.lg.shown("time.onTimerVolume");
        rows.push(vol);
    }
    let mask = c.lg.num(&format!("{pre}Weekday"), 0.0).max(0.0) as i64;
    for (i, day) in DAYS.iter().enumerate() {
        let ds = format!("{pre}Weekday");
        let mut r = Row::toggle(&format!("day{i}")).tab(1).icon("clock").label(day).on(mask & (1 << i) != 0);
        r.enabled = c.lg.usable(&ds);
        rows.push(r.act(move |a| match a {
            Arg::Toggle(v) => vec![Op::Lg { ds: ds.clone(), value: json!(if v { mask | (1 << i) } else { mask & !(1 << i) }), app: None }],
            _ => Vec::new(),
        }));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::{World, lg_with};

    fn world() -> World {
        let mut w = World::new(lg_with(
            &[
                ("time", json!({"settings": {"noOperationAutoOff": "off", "onTimerEnable": "on", "onTimerHour": 18, "onTimerMinute": 5, "offTimerEnable": "off", "autoOff15Min": "on", "autoOff2HourOnTimer": "off", "automaticStandBy": "4", "onTimerWeekday": 5, "onTimerSource": "hdmi2", "onTimerVolume": 20}})),
                ("option", json!({"settings": {"quickStartMode": "on", "audioGuidance": "off", "screenOff": "off", "highContrast": "off", "focusEmphasis": "off", "screenMagnification": "off", "adjustments": {"blackAndWhite": "on", "colorInversion": "off"}}})),
                ("picture", json!({"settings": {"screenShift": "on", "logoLuminanceAdjust": "light", "scheduledPixelCleaning": "off"}})),
                ("other", json!({"settings": {"oledCareMode": "off", "oledCareRecommendation": "on"}})),
                ("caption", json!({"settings": {"captionEnable": "on"}})),
                ("sound", json!({"settings": {"audioDescription": "off"}})),
                ("network", json!({"settings": {"deviceName": "Living room"}})),
                ("", json!({"settings": {"localeInfo": {"locales": {"UI": "en-US", "AUD": "en-US"}, "keyboards": ["en"]}}})),
            ],
            &[],
        ));
        w.theme = json!({"oledcare": {"enabled": true, "appDim": false}});
        w.now = 1_760_000_000_000;
        w
    }

    fn by_id<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id} in {:?}", rows.iter().map(|r| &r.id).collect::<Vec<_>>()))
    }

    #[test]
    fn the_general_tab_reads_the_tvs_language_and_opens_its_children() {
        let w = world();
        let r = rows(&w.ctx());
        assert_eq!(by_id(&r, "language").value, "English (US)");
        assert!(by_id(&r, "language").enabled);
        assert!(matches!(&(by_id(&r, "language").act.as_ref().unwrap())(Arg::Press)[..], [Op::Open { page, .. }] if page == "language"));
        assert_eq!(by_id(&r, "lg").label, "Open LG settings");
        assert!(matches!(&(by_id(&r, "lg").act.as_ref().unwrap())(Arg::Press)[..], [Op::Launch { app, .. }] if app == "com.palm.app.settings"));
        assert_eq!(by_id(&r, "time.noOperationAutoOff").index, Some(0));
    }

    #[test]
    fn the_sleep_timer_is_a_stepper_on_lgs_list_and_the_timers_say_when() {
        let mut w = world();
        let r = rows(&w.ctx());
        let sleep = by_id(&r, "sleep");
        assert_eq!((sleep.index, sleep.options.len()), (Some(0), 9));
        assert_eq!(sleep.detail, "");
        assert!(matches!(&(sleep.act.as_ref().unwrap())(Arg::Index(4))[..], [Op::Sleep(4)]));
        assert_eq!(by_id(&r, "poweron").value, "6:05 PM");
        assert_eq!(by_id(&r, "poweroff").value, "Off");
        w.power.set(3, 0);
        assert_eq!(summary(&w.ctx()), "Off in 30 min");
    }

    #[test]
    fn oled_care_has_lgs_own_rows_then_lumatvs_under_a_head_each_writing_one_key_of_the_theme() {
        let mut w = world();
        let r = rows(&w.ctx());
        let tab2: Vec<&str> = r.iter().filter(|x| x.tab == 2).map(|x| x.id.as_str()).collect();
        assert_eq!(tab2, vec!["panelhours", "picture.screenShift", "picture.logoLuminanceAdjust", "other.oledCareMode", "pixelcleaning", "other.oledCareRecommendation", "head:LumaTV panel care", "care:enabled", "care:appDim", "care:appShift", "care:boxDim", "care:panelDim", "care:refreshGuard", "panelrefresh"]);
        assert!(by_id(&r, "care:enabled").on && !by_id(&r, "care:appDim").on && !by_id(&r, "care:appShift").on, "an absent key is off");
        assert!(by_id(&r, "care:panelDim").on, "but the panel dim is on while the theme says nothing: the compositor's own default");
        let ops = (by_id(&r, "care:panelDim").act.as_ref().unwrap())(Arg::Toggle(false));
        assert!(matches!(&ops[..], [Op::Patch(p)] if p == &json!({"oledcare": {"panelDim": false}})));
        let ops = (by_id(&r, "care:appShift").act.as_ref().unwrap())(Arg::Toggle(true));
        assert!(matches!(&ops[..], [Op::Patch(p)] if p == &json!({"oledcare": {"appShift": true}})));
        assert_eq!(by_id(&r, "panelhours").value, "Not read");
        assert!(!by_id(&r, "panelrefresh").visible, "nothing until LG has answered");
        w.docs.set("panel", json!({"panelUsageTime": 600}));
        w.topics.set_helper("panel", json!({"read": true, "usageHours": 120.0, "lastRunHours": 117.0, "offrsCount": 12}));
        let r = rows(&w.ctx());
        assert_eq!(by_id(&r, "panelhours").value, "100 h");
        assert_eq!(by_id(&r, "panelrefresh").value, "last run 3 h ago, 12 runs");
        assert!(by_id(&r, "panelrefresh").visible);
    }

    #[test]
    fn pixel_cleaning_flips_in_place_and_says_what_it_will_do() {
        let w = world();
        let r = rows(&w.ctx());
        let pixel = by_id(&r, "pixelcleaning");
        let on = (pixel.act.as_ref().unwrap())(Arg::Toggle(true));
        assert!(matches!(&on[..], [Op::Lg { value, .. }, Op::Act { .. }] if value == "on"), "{on:?}");
        assert_eq!(pixel.label, "Clean pixels when the TV is turned off");
        assert!(matches!(&(pixel.act.as_ref().unwrap())(Arg::Toggle(false))[..], [Op::Lg { value, .. }] if value == "off"));
    }

    #[test]
    fn accessibility_has_the_five_display_switches_that_exclude_each_other_and_the_nextgen_row_waits_for_the_tuner() {
        let mut w = world();
        let r = rows(&w.ctx());
        assert!(by_id(&r, "option.adjustments.blackAndWhite").on, "Grayscale reads the adjustments' own key");
        assert!(!by_id(&r, "option.highContrast").on);
        assert!(!by_id(&r, "emergency").visible, "no ATSC 3.0 tuner on this TV");
        w.docs.set("atsc3", json!({"configs": {"system.supportATSC3tuner": "true"}}));
        w.lg.learn("option", &json!({"settings": {"emergencyInformationAtsc30": "on"}}));
        w.lg.learn_desc("option", &json!({"results": [{"key": "emergencyInformationAtsc30", "ui": {"visible": true, "active": true}}]}));
        assert!(by_id(&rows(&w.ctx()), "emergency").visible);
        assert_eq!(by_id(&r, "audioguide").value, "Off");
        assert_eq!(by_id(&r, "captions").value, "On");
    }

    #[test]
    fn the_about_tab_names_the_tv_and_the_helper() {
        let mut w = world();
        let r = rows(&w.ctx());
        assert_eq!(by_id(&r, "tvname").label, "Living room");
        assert_eq!(by_id(&r, "helper").value, "Helper stopped");
        w.topics.helper_up = true;
        assert_eq!(by_id(&rows(&w.ctx()), "helper").value, "Helper running");
        assert!(matches!(&(by_id(&r, "support").act.as_ref().unwrap())(Arg::Press)[..], [Op::Open { page, .. }] if page == "support"));
    }

    #[test]
    fn time_and_date_sets_the_clock_a_second_after_the_last_press_and_only_while_the_tv_is_not_setting_it_by_itself() {
        let mut w = world();
        w.docs.set("time", json!({"adjustedUtc": 1_760_000_000_i64}));
        w.docs.set("prefs", json!({"useNetworkTime": true, "timeZone": {"ZoneID": "America/New_York", "City": "New York", "CountryCode": "US"}}));
        w.docs.set("zones", json!({"timeZone": [{"ZoneID": "America/New_York", "City": "New York", "CountryCode": "US"}, {"ZoneID": "America/Chicago", "City": "Chicago", "CountryCode": "US"}, {"ZoneID": "Europe/Paris", "City": "Paris", "CountryCode": "FR"}]}));
        let r = clock_rows(&w.ctx());
        assert!(by_id(&r, "auto").on);
        assert!(!by_id(&r, "hour").enabled && by_id(&r, "hour").detail == "Set Automatically is on");
        assert_eq!(by_id(&r, "zone").value, "New York");
        let ops = (by_id(&r, "zone").act.as_ref().unwrap())(Arg::Press);
        let [Op::Pick(spec)] = &ops[..] else { panic!() };
        assert_eq!(spec.options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>(), vec!["New York", "Chicago"], "the zones of the country in force");
        assert_eq!(spec.current, "America/New_York");
        let w2 = (spec.write)("America/Chicago");
        assert!(matches!(&w2[0], Op::Act { payload, .. } if payload["timeZone"]["City"] == "Chicago"));
        // turning it off sets the manual state at once, in LG's two places
        let off = (by_id(&r, "auto").act.as_ref().unwrap())(Arg::Toggle(false));
        assert!(matches!(&off[..], [Op::Set(k, v), Op::Lg { ds, value, .. }, Op::Act { payload, .. }] if k == "auto" && v == &json!(false) && ds == "time.autoClock" && value == "off" && payload["useNetworkTime"] == false));
        w.local.set("auto", json!(false));
        let r = clock_rows(&w.ctx());
        assert!(by_id(&r, "minute").enabled);
        let ops = (by_id(&r, "minute").act.as_ref().unwrap())(Arg::Nudge(1));
        let [Op::Set(k, Value::Number(n)), Op::Later { key, ms, then }] = &ops[..] else { panic!("{ops:?}") };
        assert_eq!((k.as_str(), key.as_str(), *ms), ("draft", "settime", 1000));
        let sent = n.as_i64().unwrap();
        assert!(matches!(&then[..], [Op::Act { uri, payload }] if uri.ends_with("/time/setSystemTime") && payload["utc"] == sent));
        assert_eq!(calendar::local(sent).minute, (calendar::local(1_760_000_000).minute + 1) % 60, "one minute on the field");
    }

    #[test]
    fn a_timer_page_has_lgs_keys_for_its_side_and_a_toggle_for_each_day_of_the_weekday_mask() {
        let w = world();
        let on = timer_rows(&w.ctx(), "on");
        assert_eq!(by_id(&on, "time.onTimerHour").value, "6 PM");
        assert_eq!(by_id(&on, "time.onTimerMinute").value, "05");
        assert!(on.iter().any(|r| r.id == "time.onTimerSource") && on.iter().any(|r| r.id == "time.onTimerVolume"));
        let days: Vec<(bool, &str)> = on.iter().filter(|r| r.tab == 1).map(|r| (r.on, r.label.as_str())).collect();
        assert_eq!(days, vec![(true, "Sunday"), (false, "Monday"), (true, "Tuesday"), (false, "Wednesday"), (false, "Thursday"), (false, "Friday"), (false, "Saturday")], "5 is Sunday and Tuesday");
        let ops = (by_id(&on, "day1").act.as_ref().unwrap())(Arg::Toggle(true));
        assert!(matches!(&ops[..], [Op::Lg { ds, value, .. }] if ds == "time.onTimerWeekday" && value == &json!(7)));
        let ops = (by_id(&on, "day0").act.as_ref().unwrap())(Arg::Toggle(false));
        assert!(matches!(&ops[..], [Op::Lg { value, .. }] if value == &json!(4)));
        let ops = (by_id(&on, "time.onTimerHour").act.as_ref().unwrap())(Arg::Nudge(1));
        assert!(matches!(&ops[..], [Op::Lg { value, .. }] if value == &json!(19)));
        let off = timer_rows(&w.ctx(), "off");
        assert!(!off.iter().any(|r| r.id == "time.onTimerSource") && by_id(&off, "time.offTimerEnable").label == "Timer");
    }
}
