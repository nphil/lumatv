//! Remote & pointer (`settings/PageRemote.qml`, `docs/mocks/03-settings-remote`) and the app chooser behind its keys (`PageAppKey.qml`).
//! The Magic Remote's pointer is the TV's own switch (`general.remotePointer`): off gives a button-only remote and removes accidental hover
//! moves. Speed and size are the service's own settings. What shows the pointer is LumaTV's own (`remote.pointerShow` in the theme, which the
//! compositor reads): a turn of the wheel by default, so that picking the remote up never shows it, or any movement, as LG's does. Holding Home for LG's Home stays on: it is the escape if LumaTV's Home ever
//! misbehaves, so it is shown, not switchable. Whether the number keys open the dock's tiles is LumaTV's own (`remote.numberKeys` in the
//! theme, default on). Each app key (Netflix, Prime Video, Disney+, LG Channels) opens the chooser and keeps its answer in
//! `remote.appKeys` (`settings/AppKeys.js`): LG's app where it is installed until the owner picks another app or nothing. Voice search is
//! the mic key's: the helper's `base.voice` owner has its switch (off until the owner turns it on here), and the helper's `voice` topic
//! says whether it can listen yet and, when it cannot, why. Which Home Assistant it speaks to is not a setting: it is linked from a
//! command line on the TV (`docs/research/voice-result.md`).

use super::owner::{Field, toggle as owner_toggle};
use crate::home::dock::Dock;
use crate::settings::ctx::{Ctx, Spec};
use crate::settings::names::Table;
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Op, PageArg, Row, patch_of};
use serde_json::{Map, Value, json};

const VOICE: &str = "base.voice";

/// The app keys: id, label, icon, the app LG launches for it (`AppKeys.js`).
pub const KEYS: [(&str, &str, &str, &str); 4] = [
    ("netflix", "Netflix", "clapperboard", "netflix"),
    ("amazon", "Prime Video", "play", "amazon"),
    ("disneyplus", "Disney+", "sparkles", "com.disney.disneyplus-prod"),
    ("lgchannels", "LG Channels", "tv-minimal-play", "com.webos.app.lgchannels"),
];
/// What `remote.pointerShow` says, in the order of the row's options; the first is the default and what any other word means.
const POINTER_SHOW: [&str; 2] = ["wheel", "movement"];
const DIGITS: [&str; 10] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];

/// LG's times for a long press and for the repeat: "Basic", or a number of seconds.
const SECONDS: Table = &[("basic", "Basic"), ("0.5", "0.5 s"), ("1", "1 s"), ("1.5", "1.5 s"), ("2", "2 s"), ("2.5", "2.5 s"), ("3", "3 s"), ("3.5", "3.5 s"), ("4", "4 s"), ("4.5", "4.5 s"), ("5", "5 s")];

pub fn page() -> Page {
    Page::new("remote", "Remote & pointer", "mouse-pointer-2").rail().tabs(&["Pointer", "Keys", "Voice"]).reads(reads).rows(rows).summary(summary)
}

pub fn app_key() -> Page {
    Page::new("appkey", "App key", "layout-grid").parent("remote").back("Remote & pointer").hidden_from_search().reads(app_reads).rows(app_rows).title_of(app_title).opened(app_opened)
}

fn reads(_: &Ctx) -> Vec<Read> {
    vec![Read::lg("general", &["remotePointer"]), Read::lg("option", &["pointerSpeed", "pointerSize", "remoteSlowInput", "longPressInterval", "repeatInterval"]), Read::owner(VOICE), Read::Apps]
}

fn summary(c: &Ctx) -> String {
    if !c.lg.has("general.remotePointer") { String::new() } else if c.lg.is("general.remotePointer", "on") { "Pointer on".into() } else { "Pointer off".into() }
}

// ----------------------------------------------------------------------------------------------------------------- the theme's side

/// What `remote.quickAccess` holds: `{on, slots: {digit: app id}}`.
fn quick(c: &Ctx) -> (bool, Map<String, Value>) {
    let q = c.theme_at("remote.quickAccess");
    (q.and_then(|q| q.get("on")).and_then(Value::as_bool) == Some(true), q.and_then(|q| q.get("slots")).and_then(Value::as_object).cloned().unwrap_or_default())
}

/// `QA.switched`: the whole object with the switch turned.
fn quick_switched(c: &Ctx, on: bool) -> Value {
    json!({"on": on, "slots": quick(c).1})
}

/// `QA.chosen`: the whole object after `digit` was given `choice` ("default" puts the dock tile back, an app id sets it).
fn quick_chosen(c: &Ctx, digit: &str, choice: &str) -> Value {
    let (on, mut slots) = quick(c);
    slots.remove(digit);
    if choice != "default" {
        slots.insert(digit.to_string(), json!(choice));
    }
    json!({"on": on, "slots": slots})
}

/// `AK.config`: the app keys the owner set.
fn key_cfg(c: &Ctx) -> Map<String, Value> {
    c.theme_at("remote.appKeys").and_then(Value::as_object).cloned().unwrap_or_default()
}

/// `AK.chosen`: the whole `appKeys` after the owner picked `choice` for key `id` ("default" removes the entry, "" is nothing).
fn keys_chosen(c: &Ctx, id: &str, choice: &str) -> Value {
    let mut cfg = key_cfg(c);
    cfg.remove(id);
    if choice != "default" {
        cfg.insert(id.to_string(), json!(choice));
    }
    Value::Object(cfg)
}

/// What an app key does now (`AK.target`): the app's id, whether it is installed and its title; `set` is whether the owner chose.
struct KeyTarget {
    set: bool,
    app_id: String,
    installed: Option<String>,
}

fn key_target(c: &Ctx, id: &str) -> KeyTarget {
    let cfg = key_cfg(c);
    let set = cfg.get(id);
    let app_id = match set {
        None => KEYS.iter().find(|k| k.0 == id).map_or("", |k| k.3).to_string(),
        Some(v) => crate::host::sio::js::string(v),
    };
    let installed = (!app_id.is_empty()).then(|| c.apps.launchable().into_iter().find(|(i, _)| *i == app_id).map(|(_, t)| t)).flatten();
    KeyTarget { set: set.is_some(), app_id, installed }
}

/// The dock's tile ids in order, as Home arranges them.
fn dock_ids(c: &Ctx) -> Vec<String> {
    Dock::tile_ids(c.apps.points(), c.theme)
}

/// A digit's dock tile: 1 is the first, 9 the ninth, 0 the tenth.
fn tile_of(digit: &str) -> usize {
    if digit == "0" { 9 } else { digit.parse::<usize>().unwrap_or(1).saturating_sub(1) }
}

fn title_of_app(c: &Ctx, id: &str) -> String {
    c.apps.by_id(id).map_or_else(|| id.to_string(), |a| a.title.clone())
}

/// What a number key opens now (`QA.target`): the title, and whether it is the owner's slot, else a dock tile, else nothing.
fn number_target(c: &Ctx, digit: &str) -> (&'static str, String) {
    if let Some(id) = quick(c).1.get(digit).and_then(Value::as_str).filter(|s| !s.is_empty()) {
        return ("slot", title_of_app(c, id));
    }
    match dock_ids(c).get(tile_of(digit)) {
        Some(id) => ("dock", title_of_app(c, id)),
        None => ("none", String::new()),
    }
}

// --------------------------------------------------------------------------------------------------------------------------- rows

/// The row's option in force: Movement where the theme says so, else the wheel.
fn pointer_show_index(c: &Ctx) -> usize {
    usize::from(c.theme_str("remote.pointerShow") == Some(POINTER_SHOW[1]))
}

fn remote_toggle(c: &Ctx, id: &str, icon: &str, label: &str, help: &str, key: &str, default: bool) -> Row {
    let key = format!("remote.{key}");
    Row::toggle(id).tab(1).icon(icon).label(label).help(help).on(c.theme_flag(&key, default)).act(move |a| match a {
        Arg::Toggle(v) => vec![Op::Patch(patch_of(&key, json!(v)))],
        _ => Vec::new(),
    })
}

/// What the helper says is missing for voice search, in words for the owner (the row's help); empty when it can listen.
fn voice_needs(c: &Ctx) -> &'static str {
    match c.helper("voice") {
        None => "The TV's helper does not answer for voice search.",
        Some(t) => match t.get("reason").and_then(Value::as_str).unwrap_or("") {
            "voice_unlinked" => "Home Assistant is not linked yet. Link it from the TV's command line (see the voice notes), then it listens.",
            "no_microphone" => "No microphone is set in the helper's configuration.",
            "link_unreadable" => "The saved Home Assistant link cannot be read. Link it again.",
            _ => "",
        },
    }
}

/// The same in two words (the row's value).
fn voice_status(c: &Ctx) -> &'static str {
    match c.helper("voice") {
        None => "Not answering",
        Some(t) => match t.get("reason").and_then(Value::as_str).unwrap_or("") {
            "voice_unlinked" => "Not linked",
            "no_microphone" => "No microphone",
            "link_unreadable" => "Link unreadable",
            _ => "",
        },
    }
}

fn rows(c: &Ctx) -> Vec<Row> {
    let mut rows = vec![
        c.toggle(Spec::new("general.remotePointer").tab(0).icon("mouse-pointer-2").label("Magic Remote pointer").help("Use the pointer on screen. Turn off if you only use a button remote.")),
        Row::seg("pointershow", &["Wheel turn", "Movement"])
            .tab(0)
            .icon("mouse-pointer-2")
            .label("Show pointer")
            .help("Wheel turn: the pointer appears when the wheel is turned, and moving the remote never shows it. Movement: any movement shows it, as on LG's own. A direction key hides it either way.")
            .index(Some(pointer_show_index(c)))
            .act(|a| match a {
                Arg::Index(i) => POINTER_SHOW.get(i).map(|v| vec![Op::Patch(patch_of("remote.pointerShow", json!(v)))]).unwrap_or_default(),
                _ => Vec::new(),
            }),
        c.seg(Spec::new("option.pointerSpeed").tab(0).icon("gauge").label("Pointer speed").options(&["Slow", "Normal", "Fast"]).values(&["slow", "normal", "fast"])),
        c.seg(Spec::new("option.pointerSize").tab(0).icon("scan-eye").label("Pointer size").options(&["Small", "Medium", "Large"]).values(&["small", "medium", "large"])),
        Row::info("holdhome").tab(1).icon("house").label("Hold Home for LG's Home").value("Always on").help("Press and hold Home for 650 ms to open LG's own Home if LumaTV misbehaves."),
        remote_toggle(c, "numberkeys", "hash", "Number keys open dock tiles", "1 opens the first tile on the dock, 2 the second, and so on.", "numberKeys", true),
        remote_toggle(c, "gearhold", "bell", "Hold the gear key for notifications", "Hold Settings for a moment to open the notifications. Off gives the hold back to LG's Settings.", "gearHold", true),
        remote_toggle(c, "quickpanel", "layout-grid", "Quick panel on gear", "", "quickPanel", true),
        remote_toggle(c, "inputpicker", "cable", "Input picker on Input key", "", "inputPicker", true),
        remote_toggle(c, "quickpaneledit", "layout-grid", "Quick panel tiles: choices and editing", "OK on Picture mode or Sleep timer opens its choices beside the panel, and a held OK on a tile moves, removes or adds tiles. Off leaves OK and a held OK as they were.", "quickPanelEdit", false),
        remote_toggle(c, "appswitcher", "layout-grid", "App switcher on a second Home press", "Pressing Home again, from an app within a moment or on Home itself, shows the apps that are open as live pictures, to switch to or quit. The first press still goes Home at once. Off leaves Home as it was.", "appSwitcher", false),
        remote_toggle(c, "recents", "history", "Recent apps on Back", "Back on Home shows the same switcher, to switch to an app or quit it. Off leaves Back to LG's own.", "recents", false),
    ];
    let (on, _) = quick(c);
    let mut qa = Row::toggle("quickaccess").tab(1).icon("hash").label("Quick Access for the number keys").help("A number key says what it opened, and one with nothing behind it asks what to give it. Off leaves the number keys as the dock's first ten tiles.").on(on);
    let switched = |v| quick_switched(c, v);
    let (to_on, to_off) = (switched(true), switched(false));
    qa = qa.act(move |a| match a {
        Arg::Toggle(v) => vec![Op::Patch(json!({"remote": {"quickAccess": if v { to_on.clone() } else { to_off.clone() }}}))],
        _ => Vec::new(),
    });
    rows.push(qa);
    rows.push(Row::head("App keys", 1));
    for (id, label, icon, _) in KEYS {
        let t = key_target(c, id);
        let value = t.installed.clone().unwrap_or_else(|| "Nothing".into());
        rows.push(Row::link(&format!("key:{id}")).tab(1).icon(icon).label(&format!("{label} key")).value(&value).does(vec![Op::Open { page: "appkey".into(), arg: PageArg::Json(json!({"key": id})) }]));
    }
    if on {
        rows.push(Row::head("Number keys", 1));
        for d in DIGITS {
            let (kind, title) = number_target(c, d);
            let value = if kind == "none" { "Nothing yet".to_string() } else { title };
            rows.push(Row::link(&format!("number:{d}")).tab(1).icon("hash").label(&format!("Number key {d}")).value(&value).does(vec![Op::Open { page: "appkey".into(), arg: PageArg::Json(json!({"digit": d})) }]));
        }
    }
    rows.push(c.toggle(Spec::new("option.remoteSlowInput").tab(1).icon("timer").label("Extend Pressing Time on Remote").help("For a slower hand: a key must be held longer to count as a long press, and repeats come slower.")));
    let slow_on = c.lg.is("option.remoteSlowInput", "on");
    for (ds, icon, label, help) in [
        ("option.longPressInterval", "hourglass", "Activate time for long press", "How long a key is held before it counts as a long press."),
        ("option.repeatInterval", "fast-forward", "Interval time for repeated actions", "How long between the repeats while a key is held."),
    ] {
        let mut r = c.choice(Spec::new(ds).tab(1).icon(icon).label(label).help(help).names(SECONDS));
        r.enabled = slow_on && c.lg.usable(ds);
        r.detail = if !c.lg.has(ds) { "TV not answering" } else if slow_on { "" } else { "Turn Extend Pressing Time on" }.into();
        rows.push(r);
    }
    rows.push(owner_toggle(c, Field::new(VOICE, "enabled", 2, "mic", "Search by voice").help("Hold the mic key and speak; the words go into search. The mic is open only while held.")));
    let voice_on = c.owner(VOICE).is_some_and(|o| o.has("enabled") && o.get("enabled") == Some(&json!(true)));
    let needs = voice_needs(c);
    if voice_on && !needs.is_empty() {
        rows.push(Row::info("voicestatus").tab(2).icon("mic").label("Voice search").value(voice_status(c)).help(needs));
    }
    rows
}

// ---------------------------------------------------------------------------------------------------------------- the app chooser

fn app_reads(_: &Ctx) -> Vec<Read> {
    vec![Read::Apps]
}

fn digit<'a>(c: &'a Ctx<'_>) -> Option<&'a str> {
    c.arg.str("digit").filter(|d| !d.is_empty())
}

fn key_def(c: &Ctx) -> Option<(&'static str, &'static str, &'static str, &'static str)> {
    KEYS.iter().copied().find(|k| Some(k.0) == c.arg.str("key"))
}

fn app_title(c: &Ctx) -> String {
    match (digit(c), key_def(c)) {
        (Some(d), _) => format!("Number key {d}"),
        (None, Some(k)) => format!("{} key", k.1),
        _ => "App key".into(),
    }
}

/// The apps the TV launches, the dock's in its order, then the rest by name (`Apps.list`).
fn ordered(c: &Ctx) -> Vec<(String, String)> {
    let dock = dock_ids(c);
    let mut apps: Vec<(String, String)> = c.apps.list().iter().map(|a| (a.id.clone(), a.title.clone())).collect();
    apps.sort_by_key(|(id, title)| match dock.iter().position(|d| d == id) {
        Some(i) => (0, i, String::new()),
        None => (1, 0, title.to_lowercase()),
    });
    apps
}

/// The row in force: LG's default, nothing, or an app.
fn now(c: &Ctx) -> (&'static str, String) {
    if let Some(d) = digit(c) {
        return match number_target(c, d) {
            ("slot", _) => ("app", quick(c).1.get(d).and_then(Value::as_str).unwrap_or("").to_string()),
            _ => ("default", String::new()),
        };
    }
    let id = c.arg.str("key").unwrap_or("");
    let t = key_target(c, id);
    (if !t.set { "default" } else if t.app_id.is_empty() { "nothing" } else { "app" }, t.app_id)
}

/// What a pick writes: the new setting, then back to Remote & pointer.
fn pick(c: &Ctx, choice: &str) -> Vec<Op> {
    let patch = match (digit(c), key_def(c)) {
        (Some(d), _) => json!({"remote": {"quickAccess": quick_chosen(c, d, choice)}}),
        (None, Some(k)) => json!({"remote": {"appKeys": keys_chosen(c, k.0, choice)}}),
        _ => return Vec::new(),
    };
    vec![Op::Patch(patch), Op::Open { page: "remote".into(), arg: PageArg::None }]
}

fn app_rows(c: &Ctx) -> Vec<Row> {
    let (kind, app) = now(c);
    let mut rows = Vec::new();
    let choice = |c: &Ctx, v: &str| {
        let ops = pick(c, v);
        move |_: Arg| ops.clone()
    };
    if let Some(d) = digit(c) {
        let tile = dock_ids(c).get(tile_of(d)).map(|id| title_of_app(c, id));
        rows.push(Row::link("default").icon("rotate-ccw").label("The dock's tile").chevron(false).value(&tile.unwrap_or_else(|| "None on the dock".into())).selected(kind == "default").act(choice(c, "default")));
    } else {
        let installed = key_def(c).and_then(|k| c.apps.launchable().into_iter().find(|(id, _)| id == k.3).map(|(_, t)| t));
        rows.push(Row::link("default").icon("rotate-ccw").label("LG's default").chevron(false).value(&installed.unwrap_or_else(|| "Not installed".into())).selected(kind == "default").act(choice(c, "default")));
        rows.push(Row::link("nothing").icon("ban").label("Does nothing").chevron(false).selected(kind == "nothing").act(choice(c, "")));
    }
    rows.push(Row::head("Open an app", 0));
    for (id, title) in ordered(c) {
        rows.push(Row::link(&format!("app:{id}")).icon("layout-grid").label(&title).chevron(false).selected(kind == "app" && app == id).act(choice(c, &id)));
    }
    rows
}

/// The focus starts on the row in force.
fn app_opened(c: &Ctx) -> Vec<Op> {
    app_rows(c).into_iter().find(|r| r.selected).map(|r| vec![Op::Focus(r.id)]).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::super::owner::fixtures::{doc, world_with};
    use super::*;
    use crate::settings::ctx::fixtures::{World, lg_with};
    use crate::settings::row::Arg;

    fn world() -> World {
        let lg = lg_with(&[("general", json!({"settings": {"remotePointer": "on"}})), ("option", json!({"settings": {"pointerSpeed": "normal", "pointerSize": "medium", "remoteSlowInput": "off", "longPressInterval": "basic", "repeatInterval": "basic"}}))], &[]);
        let mut w = World::new(lg);
        w.apps.update(&crate::kit::screen::Input::Luna {
            tag: 0,
            payload: json!({"launchPoints": [
                {"id": "netflix", "title": "Netflix", "launchPointId": "netflix_default"},
                {"id": "com.webos.app.lgchannels", "title": "LG Channels", "systemApp": true, "hidden": true},
                {"id": "youtube.leanback.v4", "title": "YouTube"},
                {"id": "plex", "title": "Plex"}]}),
        });
        w
    }

    fn row<'a>(r: &'a [Row], id: &str) -> &'a Row {
        r.iter().find(|x| x.id == id).unwrap_or_else(|| panic!("no row {id}: {:?}", r.iter().map(|x| &x.id).collect::<Vec<_>>()))
    }

    #[test]
    fn the_pointer_tab_is_the_tvs_own_keys_and_the_rail_says_whether_the_pointer_is_on() {
        let w = world();
        let c = w.ctx();
        assert_eq!(summary(&c), "Pointer on");
        let r = rows(&c);
        let tab0: Vec<&str> = r.iter().filter(|x| x.tab == 0).map(|x| x.id.as_str()).collect();
        assert_eq!(tab0, vec!["general.remotePointer", "pointershow", "option.pointerSpeed", "option.pointerSize"]);
        assert!(row(&r, "general.remotePointer").on);
        assert_eq!(row(&r, "option.pointerSpeed").index, Some(1));
        let hold = row(&r, "holdhome");
        assert!(hold.info && !hold.chevron && hold.value == "Always on" && hold.focusable(), "shown, not switchable, and its help can be reached");
    }

    #[test]
    fn show_pointer_is_the_wheel_until_the_theme_says_movement_and_writes_the_one_key() {
        let mut w = world();
        let r = rows(&w.ctx());
        let show = row(&r, "pointershow");
        assert_eq!((show.index, show.tab), (Some(0), 0), "the wheel is the default");
        let write = |r: &Row, i| match &(r.act.as_ref().unwrap())(Arg::Index(i))[..] {
            [Op::Patch(p)] => p.clone(),
            other => panic!("one patch: {}", other.len()),
        };
        assert_eq!(write(show, 1), json!({"remote": {"pointerShow": "movement"}}));
        assert_eq!(write(show, 0), json!({"remote": {"pointerShow": "wheel"}}));
        assert!((show.act.as_ref().unwrap())(Arg::Index(2)).is_empty(), "no third option");
        w.theme = json!({"remote": {"pointerShow": "movement"}});
        assert_eq!(row(&rows(&w.ctx()), "pointershow").index, Some(1));
        w.theme = json!({"remote": {"pointerShow": "tilt"}});
        assert_eq!(row(&rows(&w.ctx()), "pointershow").index, Some(0), "a word the compositor does not know is the default");
    }

    #[test]
    fn the_keys_tab_switches_theme_keys_and_lists_what_each_app_key_opens_now() {
        let mut w = world();
        w.theme = json!({"remote": {"gearHold": false, "appKeys": {"netflix": "plex", "amazon": ""}}});
        let r = rows(&w.ctx());
        assert!(!row(&r, "gearhold").on && row(&r, "numberkeys").on && !row(&r, "appswitcher").on, "each has the default the QML page reads");
        assert!(matches!(&(row(&r, "gearhold").act.as_ref().unwrap())(Arg::Toggle(true))[..], [Op::Patch(p)] if p == &json!({"remote": {"gearHold": true}})));
        assert_eq!(row(&r, "key:netflix").value, "Plex", "the owner's choice, by the app's title");
        assert_eq!(row(&r, "key:amazon").value, "Nothing", "chosen: nothing");
        assert_eq!(row(&r, "key:disneyplus").value, "Nothing", "LG's own app is not installed here");
        assert_eq!(row(&r, "key:lgchannels").value, "LG Channels", "a hidden system app still counts as installed");
        let ops = (row(&r, "key:netflix").act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&ops[..], [Op::Open { page, arg }] if page == "appkey" && arg.str("key") == Some("netflix")));
        assert!(!r.iter().any(|x| x.id.starts_with("number:")), "the number keys are listed while Quick Access is on");
    }

    #[test]
    fn quick_access_lists_the_number_keys_with_the_dock_tile_or_the_owners_slot_and_keeps_the_slots_when_switched() {
        let mut w = world();
        w.theme = json!({"remote": {"quickAccess": {"on": true, "slots": {"2": "plex"}}}});
        let r = rows(&w.ctx());
        assert!(row(&r, "quickaccess").on);
        assert_eq!(r.iter().filter(|x| x.id.starts_with("number:")).count(), 10);
        assert_eq!(row(&r, "number:2").value, "Plex");
        assert_ne!(row(&r, "number:1").value, "Nothing yet", "the first dock tile");
        assert_eq!(row(&r, "number:9").value, "Nothing yet", "no ninth tile");
        let ops = (row(&r, "quickaccess").act.as_ref().unwrap())(Arg::Toggle(false));
        assert!(matches!(&ops[..], [Op::Patch(p)] if p == &json!({"remote": {"quickAccess": {"on": false, "slots": {"2": "plex"}}}})));
    }

    #[test]
    fn extended_pressing_time_gates_the_two_times_and_the_voice_row_says_what_is_missing() {
        let w = world();
        let r = rows(&w.ctx());
        let long = row(&r, "option.longPressInterval");
        assert!(!long.enabled && long.detail == "Turn Extend Pressing Time on" && long.value == "Basic");
        let lg = lg_with(&[("option", json!({"settings": {"remoteSlowInput": "on", "longPressInterval": "2", "repeatInterval": "basic"}}))], &[]);
        let mut w = World::new(lg);
        w.owners = world_with(VOICE, &doc(&[("enabled", "bool", json!(true))])).owners;
        assert!(row(&rows(&w.ctx()), "option.longPressInterval").enabled);
        assert_eq!(row(&rows(&w.ctx()), "option.longPressInterval").value, "2 s");
        let r = rows(&w.ctx());
        let status = row(&r, "voicestatus");
        assert_eq!((status.value.as_str(), status.tab), ("Not answering", 2), "the helper has no voice topic");
        w.topics.set_helper("voice", json!({"reason": "voice_unlinked"}));
        let r = rows(&w.ctx());
        assert_eq!(row(&r, "voicestatus").value, "Not linked");
        assert!(row(&r, "voicestatus").help.starts_with("Home Assistant is not linked"));
        w.topics.set_helper("voice", json!({"reason": ""}));
        assert!(!rows(&w.ctx()).iter().any(|x| x.id == "voicestatus"), "it can listen: nothing to say");
    }

    #[test]
    fn the_chooser_offers_lgs_default_nothing_and_the_apps_and_a_pick_writes_one_entry_and_goes_back() {
        let mut w = world();
        w.theme = json!({"remote": {"appKeys": {"netflix": "plex"}}});
        w.arg = PageArg::Json(json!({"key": "netflix"}));
        let c = w.ctx();
        assert_eq!(app_title(&c), "Netflix key");
        let r = app_rows(&c);
        assert_eq!(r.iter().take(3).map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["default", "nothing", "head:Open an app"]);
        assert_eq!(row(&r, "default").value, "Netflix");
        assert!(row(&r, "app:plex").selected && !row(&r, "default").selected);
        assert!(matches!(&app_opened(&c)[..], [Op::Focus(id)] if id == "app:plex"), "the focus starts on the row in force");
        let ops = (row(&r, "app:youtube.leanback.v4").act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&ops[..], [Op::Patch(p), Op::Open { page, .. }] if p == &json!({"remote": {"appKeys": {"netflix": "youtube.leanback.v4"}}}) && page == "remote"));
        let ops = (row(&r, "default").act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&ops[0], Op::Patch(p) if p == &json!({"remote": {"appKeys": {}}})), "LG's default removes the entry");
        let ops = (row(&r, "nothing").act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&ops[0], Op::Patch(p) if p == &json!({"remote": {"appKeys": {"netflix": ""}}})));
    }

    #[test]
    fn the_chooser_of_a_number_key_starts_with_the_docks_tile_and_has_no_does_nothing() {
        let mut w = world();
        w.arg = PageArg::Json(json!({"digit": "3"}));
        let c = w.ctx();
        assert_eq!(app_title(&c), "Number key 3");
        let r = app_rows(&c);
        assert_eq!(r.iter().take(2).map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["default", "head:Open an app"]);
        assert!(row(&r, "default").selected && row(&r, "default").label == "The dock's tile");
        let ops = (row(&r, "app:plex").act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&ops[0], Op::Patch(p) if p == &json!({"remote": {"quickAccess": {"on": false, "slots": {"3": "plex"}}}})));
    }
}
