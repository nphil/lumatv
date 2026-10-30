//! Remote & pointer > Buttons (`settings/PageRemote.qml`) and the chooser behind each button (`button`): what a press of a button of the
//! Magic Remote does. The choices are the theme's `remote.buttons` (`{"<button>": "<action>"}`, the compositor applies them before any
//! app sees the key: `compositor/.../buttons.rs` says the words). A button the owner never set is absent and does what it always did.
//! The four app buttons open LG's app until the owner picks something else; every other button is left alone until then. It is what LG
//! Input Hook's disable, replace and launch entries were: it asked for a button's number and a command, this names the button and offers
//! what the TV can do. Running a shell command is not offered. An Input Hook that is still installed sees each key first (it sits in LG's
//! input daemon), which the page says.

use super::remote::ordered;
use super::util::{inputs, inputs_read};
use crate::settings::ctx::{Ctx, ask, toast};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Op, PageArg, Row};
use serde_json::{Map, Value, json};

/// LG Input Hook's app id: its being installed is what the page mentions.
const HOOK: &str = "org.webosbrew.inputhook";

/// The buttons the page lists: id, name, icon, and the app LG's own button opens (empty for a button LG leaves to the app in front).
pub const BUTTONS: [(&str, &str, &str, &str); 12] = [
    ("netflix", "Netflix", "clapperboard", "netflix"),
    ("amazon", "Prime Video", "play", "amazon"),
    ("disneyplus", "Disney+", "sparkles", "com.disney.disneyplus-prod"),
    ("lgchannels", "LG Channels", "tv-minimal-play", "com.webos.app.lgchannels"),
    ("red", "Red", "circle", ""),
    ("green", "Green", "circle", ""),
    ("yellow", "Yellow", "circle", ""),
    ("blue", "Blue", "circle", ""),
    ("chup", "Channel up", "arrow-big-up", ""),
    ("chdown", "Channel down", "arrow-big-up", ""),
    ("list", "List", "list-video", ""),
    ("more", "More (...)", "ellipsis", ""),
];

/// The things a button can do besides an app: the word, the name in the chooser, the icon, what it says the button does.
const GO: [(&str, &str, &str, &str); 6] = [
    ("home", "Go Home", "house", "Goes Home"),
    ("quick", "Open the quick panel", "layout-grid", "Opens the quick panel"),
    ("settings", "Open Settings", "settings", "Opens Settings"),
    ("notifications", "Open notifications", "bell", "Opens notifications"),
    ("mute", "Mute or unmute", "volume-x", "Mutes or unmutes"),
    ("pictureoff", "Turn the picture off", "monitor", "Turns the picture off"),
];

pub fn button() -> Page {
    Page::new("button", "Button", "keyboard")
        .parent("remote")
        .back("Remote & pointer")
        .hidden_from_search()
        .reads(button_reads)
        .rows(button_rows)
        .title_of(button_title)
        .opened(button_opened)
}

pub fn reads() -> Vec<Read> {
    vec![Read::Apps]
}

// ------------------------------------------------------------------------------------------------------------------ the theme's side

/// `remote.buttons` as the owner set it.
fn map(c: &Ctx) -> Map<String, Value> {
    c.theme_at("remote.buttons").and_then(Value::as_object).cloned().unwrap_or_default()
}

fn word<'a>(m: &'a Map<String, Value>, id: &str) -> Option<&'a str> {
    m.get(id).and_then(Value::as_str)
}

fn def(id: &str) -> Option<(&'static str, &'static str, &'static str, &'static str)> {
    BUTTONS.iter().copied().find(|b| b.0 == id)
}

/// A button's name: the list's, or "Button 1234" for a code an import from Input Hook brought that the list lacks.
fn name_of(id: &str) -> String {
    match def(id) {
        Some(b) => b.1.to_string(),
        None => format!("Button {}", id.strip_prefix("code:").unwrap_or(id)),
    }
}

/// The whole `remote.buttons` after `id` was given `choice` (`None` puts it back as it was).
fn chosen(c: &Ctx, id: &str, choice: Option<&str>) -> Value {
    let mut m = map(c);
    m.remove(id);
    if let Some(w) = choice {
        m.insert(id.to_string(), json!(w));
    }
    Value::Object(m)
}

fn is_input(app: &str) -> bool {
    app == "com.webos.app.livetv" || app.starts_with("com.webos.app.hdmi")
}

/// What a press does, in words: the button's own default when the owner set nothing.
fn does(c: &Ctx, id: &str, w: Option<&str>) -> String {
    match w {
        None => match def(id).map(|b| b.3).filter(|a| !a.is_empty()) {
            Some(app) => match c.apps.launchable().into_iter().find(|(i, _)| i == app) {
                Some((_, title)) => format!("Opens {title}"),
                None => "Nothing".into(),
            },
            None => "Not changed".into(),
        },
        Some("nothing") => "Does nothing".into(),
        Some(w) => {
            if let Some(g) = GO.iter().find(|g| g.0 == w) {
                return g.3.into();
            }
            match w.split_once(':') {
                Some(("app", app)) if is_input(app) => format!("Switches to {}", crate::host::inputs::port_name(app)),
                Some(("app", app)) => match c.apps.launchable().into_iter().find(|(i, _)| i == app) {
                    Some((_, title)) => format!("Opens {title}"),
                    None => "App not installed".into(),
                },
                Some(("as", other)) => format!("Acts as {}", name_of(other)),
                _ => "Not changed".into(),
            }
        }
    }
}

fn lower_first(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map(|f| f.to_lowercase().chain(chars).collect()).unwrap_or_default()
}

// --------------------------------------------------------------------------------------------------------------------------- the list

/// The rows of the Buttons tab (`tab`): LG Input Hook's note while it is installed, a row per button with what it does, and the reset.
pub fn rows(c: &Ctx, tab: usize) -> Vec<Row> {
    let m = map(c);
    let mut rows = Vec::new();
    if c.apps.launchable().iter().any(|(id, _)| id == HOOK) {
        rows.push(
            Row::info("inputhook").tab(tab).icon("keyboard").label("LG Input Hook is installed").value("Acts first").help(
                "A button it changes reaches LumaTV already changed. Its own list was copied here once. Remove the app and only the choices below apply.",
            ),
        );
    }
    let custom: Vec<&String> = m.keys().filter(|k| def(k).is_none() && k.starts_with("code:")).collect();
    let listed = BUTTONS.iter().map(|b| (b.0.to_string(), name_of(b.0), b.2)).chain(custom.into_iter().map(|k| (k.clone(), name_of(k), "keyboard")));
    for (id, name, icon) in listed {
        let label = if def(&id).is_some() { format!("{name} button") } else { name };
        rows.push(
            Row::link(&format!("button:{id}"))
                .tab(tab)
                .icon(icon)
                .label(&label)
                .value(&does(c, &id, word(&m, &id)))
                .does(vec![Op::Open { page: "button".into(), arg: PageArg::Json(json!({"button": id})) }]),
        );
    }
    let reset = ask(
        "rotate-ccw",
        "Reset all buttons?",
        "Every button goes back to what it did before.",
        "Reset",
        "Keep",
        true,
        vec![Op::Patch(json!({"remote": {"buttons": {}}})), toast("All buttons reset"), Op::Refresh],
    );
    rows.push(
        Row::action("resetbuttons", "Reset")
            .tab(tab)
            .icon("rotate-ccw")
            .label("Reset all buttons")
            .help("Every button goes back to what it did before.")
            .danger()
            .enabled(!m.is_empty())
            .does(reset),
    );
    rows
}

// ------------------------------------------------------------------------------------------------------------------------ the chooser

fn button_reads(_: &Ctx) -> Vec<Read> {
    vec![Read::Apps, inputs_read()]
}

fn id_arg<'a>(c: &'a Ctx<'_>) -> &'a str {
    c.arg.str("button").unwrap_or("")
}

fn button_title(c: &Ctx) -> String {
    let id = id_arg(c);
    if def(id).is_some() { format!("{} button", name_of(id)) } else { name_of(id) }
}

/// What a pick writes: the new choice, a toast that says it, then back to Remote & pointer.
fn pick(c: &Ctx, choice: Option<&str>) -> Vec<Op> {
    let id = id_arg(c);
    let said = match choice {
        None if def(id).is_some_and(|b| !b.3.is_empty()) => "back to LG's own".to_string(),
        None => "back to normal".to_string(),
        w => lower_first(&does(c, id, w)),
    };
    vec![
        Op::Patch(json!({"remote": {"buttons": chosen(c, id, choice)}})),
        toast(&format!("{}: {said}", button_title(c))),
        Op::Open { page: "remote".into(), arg: PageArg::None },
    ]
}

fn button_rows(c: &Ctx) -> Vec<Row> {
    let id = id_arg(c);
    let m = map(c);
    let now = word(&m, id);
    let choose = |c: &Ctx, w: Option<&str>| {
        let ops = pick(c, w);
        move |_: Arg| ops.clone()
    };
    let mut rows = Vec::new();
    let lg = def(id).map(|b| b.3).filter(|a| !a.is_empty());
    let (default_label, default_value) = match lg {
        Some(_) => ("LG's default", does(c, id, None)),
        None => ("Not changed", "As it always did".to_string()),
    };
    rows.push(Row::link("default").icon("rotate-ccw").label(default_label).chevron(false).value(&default_value).selected(now.is_none()).act(choose(c, None)));
    rows.push(Row::link("nothing").icon("ban").label("Does nothing").chevron(false).selected(now == Some("nothing")).act(choose(c, Some("nothing"))));
    rows.push(Row::head("Go to", 0));
    for (w, label, icon, _) in GO {
        rows.push(Row::link(w).icon(icon).label(label).chevron(false).selected(now == Some(w)).act(choose(c, Some(w))));
    }
    rows.push(Row::head("Switch input", 0));
    let live = "app:com.webos.app.livetv";
    rows.push(Row::link("input:live").icon("tv").label("Live TV").chevron(false).selected(now == Some(live)).act(choose(c, Some(live))));
    for e in inputs(c).list() {
        let w = format!("app:{}", e.app_id);
        let label = if e.port.is_empty() { e.app_id.clone() } else { e.port.clone() };
        rows.push(
            Row::link(&format!("input:{}", e.app_id))
                .icon("cable")
                .label(&label)
                .chevron(false)
                .value(&e.name)
                .selected(now == Some(w.as_str()))
                .act(choose(c, Some(&w))),
        );
    }
    rows.push(Row::head("Act as another button", 0));
    for (other, label, icon, _) in BUTTONS.iter().filter(|b| b.0 != id) {
        let w = format!("as:{other}");
        rows.push(Row::link(&format!("as:{other}")).icon(icon).label(label).chevron(false).selected(now == Some(w.as_str())).act(choose(c, Some(&w))));
    }
    rows.push(Row::head("Open an app", 0));
    for (app, title) in ordered(c) {
        let w = format!("app:{app}");
        rows.push(Row::link(&format!("app:{app}")).icon("layout-grid").label(&title).chevron(false).selected(now == Some(w.as_str())).act(choose(c, Some(&w))));
    }
    rows
}

/// The focus starts on the choice in force.
fn button_opened(c: &Ctx) -> Vec<Op> {
    button_rows(c).into_iter().find(|r| r.selected).map(|r| vec![Op::Focus(r.id)]).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::{World, lg_with};

    fn world() -> World {
        let mut w = World::new(lg_with(&[], &[]));
        w.apps.update(&crate::kit::screen::Input::Luna {
            tag: 0,
            payload: json!({"launchPoints": [
                {"id": "netflix", "title": "Netflix", "launchPointId": "netflix_default"},
                {"id": "com.webos.app.lgchannels", "title": "LG Channels", "systemApp": true, "hidden": true},
                {"id": "plex", "title": "Plex"}]}),
        });
        w
    }

    fn row<'a>(r: &'a [Row], id: &str) -> &'a Row {
        r.iter().find(|x| x.id == id).unwrap_or_else(|| panic!("no row {id}: {:?}", r.iter().map(|x| &x.id).collect::<Vec<_>>()))
    }

    fn press(r: &Row) -> Vec<Op> {
        (r.act.as_ref().expect("a row that does something"))(Arg::Press)
    }

    #[test]
    fn each_button_says_what_it_does_now_in_words() {
        let mut w = world();
        w.theme = json!({"remote": {"buttons": {"netflix": "app:plex", "amazon": "nothing", "red": "quick", "green": "as:blue", "blue": "app:com.webos.app.hdmi2", "list": "home"}}});
        let r = rows(&w.ctx(), 2);
        let value = |id: &str| row(&r, &format!("button:{id}")).value.clone();
        assert_eq!(value("netflix"), "Opens Plex");
        assert_eq!(value("amazon"), "Does nothing");
        assert_eq!(value("disneyplus"), "Nothing", "LG's own app is not installed here");
        assert_eq!(value("lgchannels"), "Opens LG Channels", "a hidden system app still counts as installed");
        assert_eq!(value("red"), "Opens the quick panel");
        assert_eq!(value("green"), "Acts as Blue");
        assert_eq!(value("blue"), "Switches to HDMI 2");
        assert_eq!(value("list"), "Goes Home");
        w.theme = json!({"remote": {"buttons": {"red": "app:com.gone.app"}}});
        assert_eq!(row(&rows(&w.ctx(), 2), "button:red").value, "App not installed");
        assert_eq!(value("yellow"), "Not changed");
        assert_eq!(row(&r, "button:netflix").label, "Netflix button");
        assert!(matches!(&press(row(&r, "button:red"))[..], [Op::Open { page, arg }] if page == "button" && arg.str("button") == Some("red")));
    }

    #[test]
    fn a_code_the_list_lacks_is_a_row_of_its_own_and_can_be_put_back() {
        let mut w = world();
        w.theme = json!({"remote": {"buttons": {"code:1234": "nothing"}}});
        let r = rows(&w.ctx(), 2);
        assert_eq!((row(&r, "button:code:1234").label.as_str(), row(&r, "button:code:1234").value.as_str()), ("Button 1234", "Does nothing"));
        w.arg = PageArg::Json(json!({"button": "code:1234"}));
        let c = w.ctx();
        assert_eq!(button_title(&c), "Button 1234");
        let ops = press(row(&button_rows(&c), "default"));
        assert!(matches!(&ops[0], Op::Patch(p) if p == &json!({"remote": {"buttons": {}}})));
    }

    #[test]
    fn input_hook_is_mentioned_only_while_it_is_installed() {
        let mut w = world();
        assert!(!rows(&w.ctx(), 2).iter().any(|x| x.id == "inputhook"));
        w.apps.update(&crate::kit::screen::Input::Luna { tag: 0, payload: json!({"launchPoints": [{"id": HOOK, "title": "LG Input Hook"}]}) });
        let r = rows(&w.ctx(), 2);
        let note = row(&r, "inputhook");
        assert!(note.info && note.label == "LG Input Hook is installed" && note.help.contains("Remove the app"));
        assert_eq!(r.first().map(|x| x.id.as_str()), Some("inputhook"), "above the buttons");
    }

    #[test]
    fn reset_asks_first_and_is_dimmed_with_nothing_to_reset() {
        let mut w = world();
        assert!(!row(&rows(&w.ctx(), 2), "resetbuttons").enabled);
        w.theme = json!({"remote": {"buttons": {"red": "home"}}});
        let r = rows(&w.ctx(), 2);
        let reset = row(&r, "resetbuttons");
        assert!(reset.enabled && reset.danger);
        let Op::Ask(q) = &press(reset)[0] else { panic!("a question first") };
        assert_eq!(q.answers.len(), 2);
        assert!(q.answers[1].danger && q.answers[1].then.iter().any(|o| matches!(o, Op::Patch(p) if p == &json!({"remote": {"buttons": {}}}))));
    }

    #[test]
    fn the_chooser_offers_each_action_and_a_pick_writes_one_entry_says_so_and_goes_back() {
        let mut w = world();
        w.theme = json!({"remote": {"buttons": {"netflix": "app:plex", "red": "mute"}}});
        w.arg = PageArg::Json(json!({"button": "netflix"}));
        let c = w.ctx();
        assert_eq!(button_title(&c), "Netflix button");
        let r = button_rows(&c);
        let order: Vec<&str> = r.iter().filter(|x| x.id.starts_with("head:")).map(|x| x.id.as_str()).collect();
        assert_eq!(order, ["head:Go to", "head:Switch input", "head:Act as another button", "head:Open an app"]);
        assert_eq!(row(&r, "default").label, "LG's default");
        assert_eq!(row(&r, "default").value, "Opens Netflix");
        assert!(row(&r, "app:plex").selected && !row(&r, "default").selected);
        assert!(!r.iter().any(|x| x.id == "as:netflix"), "a button does not act as itself");
        assert!(matches!(&button_opened(&c)[..], [Op::Focus(id)] if id == "app:plex"));
        let ops = press(row(&r, "quick"));
        assert!(matches!(&ops[0], Op::Patch(p) if p == &json!({"remote": {"buttons": {"netflix": "quick", "red": "mute"}}})));
        assert!(matches!(&ops[1], Op::Act { payload, .. } if payload["message"] == "Netflix button: opens the quick panel"));
        assert!(matches!(&ops[2], Op::Open { page, .. } if page == "remote"));
        let ops = press(row(&r, "default"));
        assert!(matches!(&ops[0], Op::Patch(p) if p == &json!({"remote": {"buttons": {"red": "mute"}}})), "LG's default removes the entry");
        assert!(matches!(&ops[1], Op::Act { payload, .. } if payload["message"] == "Netflix button: back to LG's own"));
        let ops = press(row(&r, "as:red"));
        assert!(matches!(&ops[0], Op::Patch(p) if p["remote"]["buttons"]["netflix"] == "as:red"));
    }

    #[test]
    fn the_chooser_of_a_button_without_an_app_of_its_own_starts_with_not_changed() {
        let mut w = world();
        w.arg = PageArg::Json(json!({"button": "red"}));
        let r = button_rows(&w.ctx());
        assert!(row(&r, "default").selected && row(&r, "default").label == "Not changed");
        assert!(press(row(&r, "default")).iter().any(|o| matches!(o, Op::Act { payload, .. } if payload["message"] == "Red button: back to normal")));
        assert!(row(&r, "input:live").label == "Live TV");
    }
}
