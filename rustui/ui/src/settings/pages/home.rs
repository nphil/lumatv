//! Home & look (`settings/PageLook.qml`, `docs/mocks/30-settings-home`): the theme's few knobs a person changes from the sofa
//! (`theme.json`, the file `lumatv theme set` and Home's popovers write: a row writes a patch that applies at once and is saved), in four
//! tabs; the rows that are the TV's own settings (Home Settings and AI Recommendation on Home, LG's screen saver on Screensaver, Always
//! Ready on Standby) go through LG's service on LG's keys. The layout of the pop-ups has its own page, the screensaver's sources too.

use crate::settings::ctx::{Ctx, Spec, ask, toast};
use crate::settings::names;
use super::weather_source;
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Op, Row, ShellRequest, patch_of, settings};
use lumatv_tokens::CardGlass;
use serde_json::{Value, json};
use std::rc::Rc;

pub fn page() -> Page {
    Page::new("look", "Home & look", "house").rail().tabs(&["Look", "Home", "Screensaver", "Standby"]).reads(reads).rows(rows).opened(|_| weather_source::opened()).answered(|c, name, _| weather_source::answered(c, name))
}

fn reads(_: &Ctx) -> Vec<Read> {
    vec![
        Read::owner("base.weather"),
        Read::lg("general", &["aiNudge", "aiSettingsNudge", "lifeOnScreenMode", "homeAutoLaunch", "homePromotion", "screenSaverAd"]),
        Read::lg("option", &["broadcastInfoNoti", "lifeOnScreenEnergySaving", "lifeOnScreenNotification", "lifeOnScreenUsingMotionSensor", "screenSaverTimer"]),
        Read::lg("other", &["contentRecommendation", "lgLogoDisplay"]),
    ]
}

/// A switch on one key of the theme, in the row `row` (id, tab, icon, label, help), on where the key reads `default` when it is not written.
fn theme_toggle(c: &Ctx, row: (&str, usize, &str, &str, &str), path: &str, default: bool) -> Row {
    let (id, tab, icon, label, help) = row;
    let path = path.to_string();
    Row::toggle(id).tab(tab).icon(icon).label(label).help(help).on(c.theme_flag(&path, default)).act(move |a| match a {
        Arg::Toggle(v) => vec![Op::Patch(patch_of(&path, json!(v)))],
        _ => Vec::new(),
    })
}

/// A few exclusive options on the theme, each option's patch given by `patch`.
fn theme_seg(id: &str, tab: usize, icon: &str, label: &str, options: &[&str], index: Option<usize>, patch: impl Fn(usize) -> Value + 'static) -> Row {
    Row::seg(id, options).tab(tab).icon(icon).label(label).index(index).act(move |a| match a {
        Arg::Index(i) => vec![Op::Patch(patch(i))],
        _ => Vec::new(),
    })
}

const MATERIALS: [&str; 3] = ["dark", "light", "auto"];
const CLOCKS: [&str; 3] = ["analog", "digital", "off"];
const DEPTHS: [&str; 3] = ["flat", "normal", "deep"];
const CARD_GLASS: [CardGlass; 4] = [CardGlass::Clear, CardGlass::Frosted, CardGlass::Liquid, CardGlass::Glazed];
const STYLES: [&str; 3] = ["clock", "art", "off"];
const ART_EVERY: [u64; 3] = [5, 15, 60];
const STANDBY_HOURS: [u64; 4] = [0, 2, 4, 8];

fn rows(c: &Ctx) -> Vec<Row> {
    let mut rows = Vec::new();
    // ---- Look
    rows.push(Row::toggle("tint").tab(0).icon("palette").label("Dynamic tint").help("Panes take a hint of colour from the wallpaper.").on(c.theme_str("accent") == Some("wallpaper")).act(|a| match a {
        Arg::Toggle(v) => vec![Op::Patch(json!({"accent": if v { "wallpaper" } else { "tile" }}))],
        _ => Vec::new(),
    }));
    let material = c.theme_str("material").unwrap_or("auto");
    let mut appearance = theme_seg("material", 0, "moon", "Appearance", &["Dark", "Light", "Auto"], MATERIALS.iter().position(|m| *m == material), |i| json!({"material": MATERIALS[i]}));
    appearance.help = "Auto follows the wallpaper's brightness.".into();
    if material == "auto" {
        appearance.detail = if c.light { "Light now" } else { "Dark now" }.into();
    }
    rows.push(appearance);
    rows.push(Row::link("themes").tab(0).icon("palette").label("Themes").does(vec![Op::Open { page: "themes".into(), arg: Default::default() }]));
    let clock = if c.theme_at("clock.enabled").and_then(Value::as_bool) == Some(false) { "off" } else { c.theme_str("clock.style").unwrap_or("analog") };
    rows.push(theme_seg("clock", 0, "clock", "Clock on Home", &["Analogue", "Digital", "Off"], CLOCKS.iter().position(|m| *m == clock), |i| {
        if i == 2 { json!({"clock": {"enabled": false}}) } else { json!({"clock": {"enabled": true, "style": CLOCKS[i]}}) }
    }));
    rows.push(Row::link("wallpaper").tab(0).icon("wallpaper").label("Wallpaper").does(vec![Op::Shell(ShellRequest::Wallpaper)]));
    let depth = c.theme_str("depth.level").unwrap_or("normal");
    rows.push(theme_seg("depth", 0, "blend", "Glass depth", &["Flat", "Normal", "Deep"], DEPTHS.iter().position(|m| *m == depth), |i| json!({"depth": {"level": DEPTHS[i]}})));
    let glass = CARD_GLASS.iter().copied().find(|style| Some(style.id()) == c.theme_str("cardGlass")).unwrap_or(CardGlass::Clear);
    let labels = CARD_GLASS.map(|style| style.label());
    rows.push(
        theme_seg("cardglass", 0, "layers", "Glass type", &labels, Some(glass.index()), |i| json!({"cardGlass": CARD_GLASS[i].id()}))
            .help("Choose the material used by glass surfaces."),
    );
    let strength = c.theme_num(&format!("glassStrength.{}", glass.id()), 1.0).clamp(0.0, 1.0);
    rows.push(
        Row::slider("glassstrength", 0.0, 100.0, 1.0).tab(0).icon("sliders-vertical").label("Glass strength")
            .help("Adjust the selected glass material. Each material remembers its own strength.")
            .num((strength * 100.0).round()).unit("%")
            .act(move |a| match a {
                Arg::Value(v) => vec![Op::Patch(json!({"glassStrength": {(glass.id()): v.clamp(0.0, 100.0) / 100.0}}))],
                _ => Vec::new(),
            }),
    );
    rows.push(theme_toggle(c, ("transitions", 0, "sparkles", "Smooth transitions", "Apps grow out of their tile and shrink back."), "motion.transitions", false));
    rows.push(popups_row(c));

    // ---- Home
    let shown = c.layout.get("widgets").and_then(Value::as_array).map_or(0, |l| l.iter().filter(|e| e.get("type").and_then(Value::as_str) != Some("dock") && e.get("hidden").and_then(Value::as_bool) != Some(true)).count());
    rows.push(Row::link("widgets").tab(1).icon("layout-grid").label("Home widgets").value(&format!("{shown} shown")).does(vec![Op::Open { page: "widgets".into(), arg: Default::default() }]));
    rows.push(theme_toggle(c, ("twelvehour", 1, "clock", "12-hour clock", ""), "clock.twelveHour", true));
    rows.push(Row::link("homeassistant").tab(1).icon("house-wifi").label("Home Assistant").value(&super::homeassistant::words(c)).does(vec![Op::Open { page: "homeassistant".into(), arg: Default::default() }]));
    rows.push(Row::link("cameras").tab(1).icon("video").label("Cameras").value(&super::cameras::summary(c)).does(vec![Op::Open { page: "cameras".into(), arg: Default::default() }]));
    rows.push(theme_toggle(c, ("greeting", 1, "sunrise", "Greeting", "Good morning, afternoon or evening under the digital clock."), "clock.greeting", true));
    rows.extend(weather_source::rows(c));
    // LG's Home Settings: Power On Screen writes what LG's row writes with it, the input app stays off and the choice is marked the owner's
    let mut power = c.seg(Spec::new("general.homeAutoLaunch").id("poweronscreen").tab(1).icon("power").label("Power On Screen").help("What the TV shows when it is turned on: Home, or the input it was last on.").options(&["Home", "Recent Input"]).values(&["on", "off"]).lg_shown());
    power.act = Some(Rc::new(|a| match a {
        Arg::Index(i) => vec![Op::LgMany { category: "general".into(), settings: settings(&[("homeAutoLaunch", json!(if i == 0 { "on" } else { "off" })), ("selectedInputApp", json!("off")), ("checkHomeAutoLaunchValue", json!("user"))]), app: None }],
        _ => Vec::new(),
    }));
    rows.push(power);
    rows.push(c.toggle(Spec::new("general.homePromotion").tab(1).icon("star").label("Show promotions in LG's Home").help("The sponsored ads in LG's own Home menu.").lg_shown()));
    rows.push(c.toggle(Spec::new("other.contentRecommendation").tab(1).icon("list-video").label("Show content recommendations in LG's Home").help("The content recommendations in LG's own Home menu. Not available in every country.").lg_shown()));
    rows.push(c.toggle(Spec::new("option.broadcastInfoNoti").tab(1).icon("sparkles").label("Show Who.Where.What? suggestions").help("LG's suggestions about the people, places and things in what Live TV is showing.").lg_shown()));
    rows.push(c.toggle(Spec::new("general.aiNudge").tab(1).icon("sparkles").label("Show feature tips on Home").help("LG's tips about features, shown now and then on Home.").lg_shown()));
    rows.push(c.toggle(Spec::new("general.aiSettingsNudge").tab(1).icon("sparkles").label("Show setting tips in LG's Settings").help("LG's tips about settings, shown inside LG's Settings screens.").lg_shown()));

    // ---- Screensaver
    let style = c.theme_str("screensaver.style").unwrap_or("clock").to_string();
    let mut saver = theme_seg("saver", 2, "moon", "Screensaver", &["Clock", "Art", "Off"], STYLES.iter().position(|s| *s == style), |i| json!({"screensaver": {"style": STYLES[i]}}));
    saver.help = "Starts after a while with no key. It moves, so the panel never holds one picture.".into();
    rows.push(saver);
    let on = style != "off";
    let mut dim = theme_seg("dim", 2, "gauge", "Dim over time", &["On", "Off"], Some(usize::from(c.theme_at("screensaver.dim").and_then(Value::as_bool) == Some(false))), |i| json!({"screensaver": {"dim": i == 0}}));
    dim.help = "Dims to 60 % over the first hour, gentle on the panel.".into();
    dim.enabled = on;
    rows.push(dim);
    rows.push(theme_toggle(c, ("glance", 2, "house-wifi", "Weather and home fact", "Under the clock: the temperature and one thing about the house, such as a door locked."), "screensaver.glance", true).visible(style == "clock"));
    let art = c.theme_at("screensaver.art").cloned().unwrap_or(Value::Null);
    let art_patch = {
        let (every, order, caption) = (art.get("everyMin").and_then(Value::as_u64).unwrap_or(15), art.get("order").and_then(Value::as_str).unwrap_or("shuffle").to_string(), art.get("caption").and_then(Value::as_bool) != Some(false));
        Rc::new(move |field: &str, v: Value| {
            let mut a = json!({"everyMin": every, "order": order, "caption": caption});
            a[field] = v;
            json!({"screensaver": {"art": a}})
        })
    };
    let every = art.get("everyMin").and_then(Value::as_u64).unwrap_or(15);
    let ap = art_patch.clone();
    rows.push(theme_seg("artevery", 2, "timer", "Change the art every", &["5 min", "15 min", "1 hour"], Some(if every <= 5 { 0 } else if every <= 15 { 1 } else { 2 }), move |i| ap("everyMin", json!(ART_EVERY[i]))).visible(style == "art"));
    let ap = art_patch.clone();
    rows.push(theme_seg("artorder", 2, "list-video", "Order", &["Shuffle", "In order"], Some(usize::from(art.get("order").and_then(Value::as_str) == Some("sequence"))), move |i| ap("order", json!(if i == 1 { "sequence" } else { "shuffle" }))).visible(style == "art"));
    let ap = art_patch;
    let caption_on = art.get("caption").and_then(Value::as_bool) != Some(false);
    rows.push(Row::toggle("artcaption").tab(2).icon("type").label("Artist and title").on(caption_on).visible(style == "art").act(move |a| match a {
        Arg::Toggle(v) => vec![Op::Patch(ap("caption", json!(v)))],
        _ => Vec::new(),
    }));
    let hours = c.theme_at("screensaver.standbyHours").and_then(Value::as_f64);
    let hi = match hours { None => 2, Some(0.0) => 0, Some(h) if h <= 2.0 => 1, Some(h) if h <= 4.0 => 2, _ => 3 };
    let mut standby = theme_seg("standbyhours", 2, "power", "Then go to standby", &["Never", "2 h", "4 h", "8 h"], Some(hi), |i| json!({"screensaver": {"standbyHours": STANDBY_HOURS[i]}}));
    standby.help = "The panel turns off after this long in the screensaver.".into();
    standby.enabled = on;
    rows.push(standby);
    let mut own = theme_toggle(c, ("own", 2, "moon-star", "LumaTV saver", "LumaTV's clock or art shows when the TV is idle on Home."), "screensaver.own", true);
    own.enabled = on;
    rows.push(own);
    rows.push(Row::link("during").tab(2).icon("bell").label("During the screensaver").does(vec![Op::Open { page: "duringsaver".into(), arg: Default::default() }]));
    // LG's own screen saver: its standby time (LG marks the choice as the owner's with the second key) and its ads
    rows.push(c.choice(Spec::new("option.screenSaverTimer").tab(2).icon("timer").label("Screen Saver Standby Time").help("How long without a key before LG's screen saver starts.").names(SAVER_TIMES).write(|v| {
        vec![Op::LgMany { category: "option".into(), settings: settings(&[("screenSaverTimer", json!(v)), ("checkChangedScreenSaverTimerByUser", json!("user"))]), app: None }]
    })));
    rows.push(c.toggle(Spec::new("general.screenSaverAd").tab(2).icon("star").label("Show ads on LG's screen saver").help("The ads some apps, such as LG Channels, put on LG's screen saver.").lg_shown()));

    // ---- Standby
    rows.push(c.toggle(Spec::new("other.lgLogoDisplay").tab(3).icon("tv").label("LG logo").help("LG's own logo, shown by the TV when it turns off.")));
    let ready = c.lg.has("general.lifeOnScreenMode") && !c.lg.is("general.lifeOnScreenMode", "off");
    let mut always = Row::toggle("alwaysready").tab(3).icon("moon-star").label("Always Ready").help("LG's standby: a wallpaper, the voice and music from a phone while the TV is off. It uses more power than a dark panel.").on(ready).admin();
    always.detail = c.unavailable("general.lifeOnScreenMode", "");
    always.enabled = c.lg.usable("general.lifeOnScreenMode");
    always.visible = c.lg.shown("general.lifeOnScreenMode");
    rows.push(always.act(|a| match a {
        Arg::Toggle(true) => vec![Op::Lg { ds: "general.lifeOnScreenMode".into(), value: json!("allEnabled"), app: None }],
        Arg::Toggle(false) => vec![Op::Lg { ds: "general.lifeOnScreenMode".into(), value: json!("off"), app: None }, toast("Always Ready off: in standby the TV goes dark")],
        _ => Vec::new(),
    }));
    let when_ready = |mut r: Row, ds: &str| {
        r.enabled = ready && c.lg.usable(ds);
        r.detail = if !c.lg.has(ds) { "TV not answering".into() } else if ready { String::new() } else { "Turn Always Ready on".into() };
        r
    };
    rows.push(when_ready(c.seg_tv(Spec::new("option.lifeOnScreenEnergySaving").tab(3).icon("leaf").label("Always Ready energy saving").names(AUTO_OFF).help("Lets Always Ready dim and rest when nobody is near.")), "option.lifeOnScreenEnergySaving"));
    rows.push(when_ready(c.toggle(Spec::new("option.lifeOnScreenNotification").tab(3).icon("bell").label("Always Ready notifications").help("Shows LG's notices on the Always Ready screen.").on_off("true", "false").lg_shown()), "option.lifeOnScreenNotification"));
    rows.push(when_ready(c.toggle(Spec::new("option.lifeOnScreenUsingMotionSensor").tab(3).icon("scan-eye").label("Always Ready motion sensor").help("Wakes the screen when someone walks by.").on_off("true", "false").lg_shown()), "option.lifeOnScreenUsingMotionSensor"));
    rows
}

const SAVER_TIMES: names::Table = &[("3", "3 minutes"), ("10", "10 minutes"), ("20", "20 minutes"), ("30", "30 minutes")];
const AUTO_OFF: names::Table = &[("auto", "Auto"), ("off", "Off")];

/// Where the pop-ups sit (`placement`): the volume bar, the mute mark, the camera window, the cards and the toasts, placed by the owner on
/// a picture of the screen. The row says how many are moved; OK opens the page of them.
fn popups_row(c: &Ctx) -> Row {
    let reg = lumatv_placement::Registry::shipped();
    let user = reg.user_from_json(c.theme_at("layout.overlays").unwrap_or(&Value::Null));
    let moved = user.customised();
    Row::link("popups").tab(0).icon("layout-grid").label("Layout of pop-ups").value(if moved == 0 { "Default".to_string() } else { format!("{moved} moved") }.as_str()).does(vec![Op::Open { page: "popups".into(), arg: Default::default() }])
}

pub fn popups() -> Page {
    Page::new("popups", "Layout of pop-ups", "layout-grid").parent("look").back("Home & look").hidden_from_search().rows(popup_rows)
}

/// One row for each item the registry places, valued with where it sits; "Edit in preview" opens the editor (kind `layout`) on a picture of
/// the screen; "Reset all" puts every item back where it was.
fn popup_rows(c: &Ctx) -> Vec<Row> {
    let reg = lumatv_placement::Registry::shipped();
    let user = reg.user_from_json(c.theme_at("layout.overlays").unwrap_or(&Value::Null));
    let mut rows = vec![Row::action("edit", "Edit").icon("layout-grid").label("Edit in preview").does(vec![Op::Shell(ShellRequest::Layout { item: None })])];
    for it in reg.items() {
        let id = it.id.to_string();
        rows.push(Row::link(&format!("item:{id}")).icon(it.icon).label(it.title).value(&reg.words(it.id, &user)).does(vec![Op::Shell(ShellRequest::Layout { item: Some(id) })]));
    }
    let any = user.customised() > 0;
    let mut reset = Row::action("reset", "Reset").icon("rotate-ccw").label("Reset all").danger().enabled(any);
    if !any {
        reset.detail = "Nothing is moved".into();
    }
    rows.push(reset.does(ask("rotate-ccw", "Put every pop-up back?", "They return to where they were.", "Reset", "Cancel", true, vec![Op::Patch(json!({"layout": {"overlays": {}}}))])));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::{World, lg_with};

    fn world() -> World {
        let mut w = World::new(lg_with(
            &[
                ("general", json!({"settings": {"homeAutoLaunch": "on", "homePromotion": "off", "lifeOnScreenMode": "off", "screenSaverAd": "off", "aiNudge": "off", "aiSettingsNudge": "off"}})),
                ("option", json!({"settings": {"screenSaverTimer": "10", "lifeOnScreenEnergySaving": "auto", "lifeOnScreenNotification": "true", "lifeOnScreenUsingMotionSensor": "true", "broadcastInfoNoti": "off"}, "errorKey": []})),
                ("other", json!({"settings": {"contentRecommendation": "off", "lgLogoDisplay": "on"}})),
            ],
            &[("option", json!({"results": [{"key": "screenSaverTimer", "values": {"arrayExt": [{"value": "3"}, {"value": "10"}, {"value": "20"}, {"value": "30"}]}}]}))],
        ));
        w.theme = json!({"material": "auto", "clock": {"style": "digital", "enabled": true, "twelveHour": true}, "screensaver": {"style": "art", "art": {"everyMin": 5, "order": "sequence", "caption": true}, "standbyHours": 4}, "logo": {"on": 2, "off": 3}, "depth": {"level": "deep"}});
        w.layout = json!({"widgets": [{"type": "clock"}, {"type": "weather", "hidden": true}, {"type": "dock"}, {"type": "news"}]});
        w
    }

    fn by_id<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id}"))
    }

    fn patch_of_row(r: &Row, a: Arg) -> Value {
        match &(r.act.as_ref().unwrap())(a)[..] {
            [Op::Patch(p)] => p.clone(),
            other => panic!("one patch: {other:?}"),
        }
    }

    #[test]
    fn the_look_tab_reads_the_theme_and_a_row_writes_a_patch_of_one_key() {
        let w = world();
        let r = rows(&w.ctx());
        let tint = by_id(&r, "tint");
        assert!(!tint.on);
        assert_eq!(patch_of_row(tint, Arg::Toggle(true)), json!({"accent": "wallpaper"}));
        let look = by_id(&r, "material");
        assert_eq!((look.index, look.detail.as_str()), (Some(2), "Dark now"), "Auto says what it resolved to");
        assert_eq!(patch_of_row(look, Arg::Index(1)), json!({"material": "light"}));
        let clock = by_id(&r, "clock");
        assert_eq!(clock.index, Some(1));
        assert_eq!(patch_of_row(clock, Arg::Index(2)), json!({"clock": {"enabled": false}}));
        assert_eq!(patch_of_row(clock, Arg::Index(0)), json!({"clock": {"enabled": true, "style": "analog"}}));
        assert!(matches!(&(by_id(&r, "wallpaper").act.as_ref().unwrap())(Arg::Press)[..], [Op::Shell(ShellRequest::Wallpaper)]));
        assert_eq!(by_id(&r, "depth").index, Some(2));
        assert_eq!(patch_of_row(by_id(&r, "transitions"), Arg::Toggle(true)), json!({"motion": {"transitions": true}}));
    }

    #[test]
    fn card_materials_keep_independent_strengths() {
        let mut w = world();
        w.theme["glassStrength"] = json!({"clear": 0.2, "frosted": 0.4, "liquid": 0.6, "glazed": 0.8});
        for (i, style) in CARD_GLASS.iter().copied().enumerate() {
            w.theme["cardGlass"] = json!(style.id());
            let r = rows(&w.ctx());
            let choice = by_id(&r, "cardglass");
            assert_eq!(choice.index, Some(i));
            assert_eq!(patch_of_row(choice, Arg::Index(i)), json!({"cardGlass": style.id()}));
            let slider = by_id(&r, "glassstrength");
            assert_eq!(slider.num, (i + 1) as f64 * 20.0);
            let patch = patch_of_row(slider, Arg::Value(35.0));
            assert_eq!(patch, json!({"glassStrength": {(style.id()): 0.35}}));
            lumatv_tokens::merge(&mut w.theme, &patch);
        }
        assert_eq!(w.theme["glassStrength"], json!({"clear": 0.35, "frosted": 0.35, "liquid": 0.35, "glazed": 0.35}));
        w.theme["glassStrength"] = json!({});
        assert_eq!(by_id(&rows(&w.ctx()), "glassstrength").num, 100.0);
    }

    #[test]
    fn the_home_tab_counts_the_widgets_shown_and_writes_lgs_power_on_screen_with_its_two_marks() {
        let w = world();
        let r = rows(&w.ctx());
        assert_eq!(by_id(&r, "widgets").value, "2 shown", "the dock and a hidden card are not counted");
        let power = by_id(&r, "poweronscreen");
        assert_eq!(power.index, Some(0));
        let ops = (power.act.as_ref().unwrap())(Arg::Index(1));
        let [Op::LgMany { category, settings, .. }] = &ops[..] else { panic!() };
        assert_eq!(category, "general");
        assert_eq!((settings["homeAutoLaunch"].as_str(), settings["selectedInputApp"].as_str(), settings["checkHomeAutoLaunchValue"].as_str()), (Some("off"), Some("off"), Some("user")));
        assert!(by_id(&r, "general.homePromotion").visible && by_id(&r, "general.homePromotion").tab == 1);
    }

    #[test]
    fn the_screensaver_tab_shows_the_art_rows_only_with_the_art_style_and_an_art_option_carries_the_whole_block() {
        let mut w = world();
        let r = rows(&w.ctx());
        assert!(by_id(&r, "artevery").visible && !by_id(&r, "glance").visible);
        assert_eq!(by_id(&r, "artevery").index, Some(0));
        assert_eq!(patch_of_row(by_id(&r, "artevery"), Arg::Index(2)), json!({"screensaver": {"art": {"everyMin": 60, "order": "sequence", "caption": true}}}));
        assert_eq!(patch_of_row(by_id(&r, "artorder"), Arg::Index(0)), json!({"screensaver": {"art": {"everyMin": 5, "order": "shuffle", "caption": true}}}));
        assert_eq!(patch_of_row(by_id(&r, "artcaption"), Arg::Toggle(false)), json!({"screensaver": {"art": {"everyMin": 5, "order": "sequence", "caption": false}}}));
        assert_eq!(by_id(&r, "standbyhours").index, Some(2));
        w.theme["screensaver"]["style"] = json!("off");
        let r = rows(&w.ctx());
        assert!(!by_id(&r, "dim").enabled && !by_id(&r, "standbyhours").enabled && !by_id(&r, "own").enabled, "nothing to dim or stand by with no saver");
        assert_eq!(by_id(&r, "during").label, "During the screensaver");
        // LG's saver time goes with the mark that says the owner chose it
        let timer = by_id(&r, "option.screenSaverTimer");
        assert_eq!(timer.value, "10 minutes");
        let ops = (timer.act.as_ref().unwrap())(Arg::Press);
        let [Op::Pick(spec)] = &ops[..] else { panic!() };
        let written = (spec.write)("20");
        let [Op::LgMany { settings, .. }] = &written[..] else { panic!() };
        assert_eq!((settings["screenSaverTimer"].as_str(), settings["checkChangedScreenSaverTimerByUser"].as_str()), (Some("20"), Some("user")));
    }

    #[test]
    fn the_standby_tab_has_always_ready_whose_rows_wait_for_it_and_whose_off_flips_in_place() {
        let w = world();
        let r = rows(&w.ctx());
        let always = by_id(&r, "alwaysready");
        assert!(!always.on && always.admin);
        let on = (always.act.as_ref().unwrap())(Arg::Toggle(true));
        assert!(matches!(&on[..], [Op::Lg { value, .. }] if value == "allEnabled"));
        let off = (always.act.as_ref().unwrap())(Arg::Toggle(false));
        assert!(matches!(&off[0], Op::Lg { value, .. } if value == "off") && !off.iter().any(|o| matches!(o, Op::Ask(_))), "a switch opens no question");
        assert!(off.iter().any(|o| matches!(o, Op::Act { uri, .. } if uri.ends_with("/createToast"))), "it says what it did");
        let sensor = by_id(&r, "option.lifeOnScreenUsingMotionSensor");
        assert!(!sensor.enabled && sensor.detail == "Turn Always Ready on");
    }

    #[test]
    fn lgs_logo_is_the_only_logo_control_and_writes_lgs_key() {
        let w = world();
        let r = rows(&w.ctx());
        let lg = by_id(&r, "other.lgLogoDisplay");
        assert_eq!((lg.tab, lg.label.as_str(), lg.on), (3, "LG logo", true));
        assert!(lg.help.contains("LG's own"), "it says whose logo it is: {}", lg.help);
        assert!(matches!(&(lg.act.as_ref().unwrap())(Arg::Toggle(false))[..], [Op::Lg { ds, value, .. }] if ds == "other.lgLogoDisplay" && value == "off"));
        assert_eq!(r.iter().filter(|x| x.label.to_lowercase().contains("logo")).count(), 1, "only LG's logo control remains");
    }

    #[test]
    fn the_layout_of_the_popups_is_a_row_with_the_count_of_what_is_moved_and_a_page_with_a_row_for_each_item() {
        let mut w = world();
        let r = rows(&w.ctx());
        assert_eq!((by_id(&r, "popups").label.as_str(), by_id(&r, "popups").value.as_str()), ("Layout of pop-ups", "Default"));
        w.theme["layout"] = json!({"overlays": {"volume": {"anchor": "left", "dx": 0, "dy": 0.5}}});
        assert_eq!(by_id(&rows(&w.ctx()), "popups").value, "1 moved");
        let p = popup_rows(&w.ctx());
        assert_eq!(p[0].label, "Edit in preview");
        assert!(matches!(&(p[0].act.as_ref().unwrap())(Arg::Press)[..], [Op::Shell(ShellRequest::Layout { item: None })]));
        let ids: Vec<&str> = p.iter().filter(|x| x.id.starts_with("item:")).map(|x| x.id.as_str()).collect();
        assert_eq!(ids, vec!["item:volume", "item:mute", "item:popup", "item:cards", "item:toast"]);
        let volume = by_id(&p, "item:volume");
        assert!(volume.value.contains("moved") || volume.value.contains("Left"), "{}", volume.value);
        assert!(matches!(&(volume.act.as_ref().unwrap())(Arg::Press)[..], [Op::Shell(ShellRequest::Layout { item: Some(i) })] if i == "volume"));
        let reset = by_id(&p, "reset");
        assert!(reset.danger && reset.enabled);
        let ops = (reset.act.as_ref().unwrap())(Arg::Press);
        let [Op::Ask(a)] = &ops[..] else { panic!() };
        assert!(matches!(&a.answers[1].then[..], [Op::Patch(p)] if p == &json!({"layout": {"overlays": {}}})));
        w.theme["layout"] = json!({});
        assert!(!by_id(&popup_rows(&w.ctx()), "reset").enabled, "nothing moved: nothing to reset");
    }
}
