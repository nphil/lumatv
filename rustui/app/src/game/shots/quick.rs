//! Quick menu shots, its walk over Home, and its tests.
use super::budget::{Step, Walk, taps};
use super::*;
use crate::game::model::quick::QuickSetting;
use crate::game::screens::{self, paint::Body};

fn press(m: &mut Model, key: Key) -> Vec<Command> {
    m.step(Input::Key { key, down: true });
    m.step(Input::Key { key, down: false })
}
/// Every word the screen draws.
fn texts(m: &Model) -> Vec<String> {
    screens::plan_in(m, ui::lucent::palette::Mode::Dark)
        .nodes
        .iter()
        .filter_map(|n| match &n.body {
            Body::Text { words, .. } => Some(words.iter().map(|w| w.text.clone())),
            _ => None,
        })
        .flatten()
        .collect()
}
/// Home with the Desk PC offline and GeForce NOW's library loaded two minutes ago.
fn home() -> Model {
    let mut m = fixture();
    let offline = serde_json::json!({"id":"main","name":"Desk PC","link":"linked","reach":"offline"});
    event(&mut m, "punktfunk", ipc::Event::Targets { targets: vec![serde_json::from_value(offline).unwrap()] });
    let (gfn, main) = (EngineId::new("gfn").unwrap(), TargetId::new("main").unwrap());
    let entries = m.listing().sources.iter().filter(|s| s.engine == gfn && s.target == main).map(|s| s.entry.clone()).collect();
    let targets = m.integrations.iter().find(|e| e.manifest.id == gfn).unwrap().targets.clone();
    event(&mut m, "gfn", ipc::Event::Targets { targets });
    event(&mut m, "gfn", ipc::Event::Library { target: main, entries, next: None });
    m.step(Input::Tick { now_ms: 120_000, unix_s: 1_800_000_000 });
    m.view.focus = Focus::Hero(0);
    m.rebuild_navigation();
    m
}

pub(super) fn screens(_base: &Model, add: &mut impl FnMut(&str, Model)) {
    let mut m = home();
    press(&mut m, Key::Menu);
    add("home-quick-menu", m);
    let mut m = home();
    (m.prefs.fps, m.prefs.hdr, m.prefs.family, m.prefs.text_percent) = (120, true, "rose-pine".into(), 100);
    m.view.push(Layer::Quick, Focus::SheetRow(2));
    m.view.push(Layer::QuickSettings, Focus::SheetRow(0));
    add("settings-c-quick", m);
}

/// The drawer opening, Quick settings over it, a stepper moved, and both closed: the drawer is one opaque sheet whose scrim lies beside
/// it, so no frame draws a second full-screen layer or captures a backdrop. Over a game's News tab it fits the layer ceiling; over today's
/// Home it cannot until the Home rework brings Home's own 49 layers down.
pub(super) fn walks() -> Vec<Walk> {
    let mut script = taps(&[(Key::Menu, 300)]);
    script.push(Step::Expect("the quick menu open", |m, _| m.view.top() == Some(Layer::Quick)));
    script.extend(taps(&[(Key::Down, 100), (Key::Down, 100), (Key::Ok, 300), (Key::Right, 100), (Key::Down, 100), (Key::Back, 300)]));
    script.push(Step::Expect("back on the menu", |m, _| m.view.top() == Some(Layer::Quick)));
    script.extend(taps(&[(Key::Menu, 300)]));
    script.push(Step::Expect("the menu closed", |m, _| m.view.layers.is_empty()));
    let mut news = home();
    news.view.page = Page::Game;
    news.view.selected = Some(id(&news, "Hades II"));
    news.view.hub_tab = 3;
    news.view.focus = Focus::Tab(3);
    news.rebuild_navigation();
    vec![
        Walk { name: "quick-drawer-over-home".into(), base: home(), script: script.clone() },
        Walk { name: "quick-drawer-over-game-news".into(), base: news, script },
    ]
}

#[test]
fn each_menu_row_does_what_it_says() {
    let mut m = home();
    let hades = id(&m, "Hades II");
    assert_eq!(m.view.featured, Some(hades));
    press(&mut m, Key::Menu);
    assert_eq!((m.view.top(), m.view.focus), (Some(Layer::Quick), Focus::SheetRow(0)));
    let shown = texts(&m);
    for text in ["Refresh libraries", "Updated 2 min ago", "Desk PC is offline", "Hades II", "Game page", "Pin to Home", "Hide"] {
        assert!(shown.iter().any(|t| t == text), "{text} in {shown:?}");
    }
    // A first quiet tick dims the hints; after it only the age can redraw.
    m.step(Input::Tick { now_ms: 150_000, unix_s: 1_800_000_000 });
    let r = m.revision();
    m.step(Input::Tick { now_ms: 170_000, unix_s: 1_800_000_000 });
    assert_eq!(m.revision(), r, "the age reads the same");
    m.step(Input::Tick { now_ms: 180_000, unix_s: 1_800_000_000 });
    assert!(m.revision() != r && texts(&m).iter().any(|t| t == "Updated 3 min ago"), "a new minute redraws it");

    press(&mut m, Key::Ok);
    assert_eq!(m.view.top(), Some(Layer::Quick), "Refresh keeps the menu open");
    assert!(texts(&m).iter().any(|t| t.starts_with("Nothing to refresh")), "and its row says what it did");

    m.view.focus = Focus::SheetRow(2);
    press(&mut m, Key::Ok);
    assert_eq!(m.view.top(), Some(Layer::QuickSettings));
    press(&mut m, Key::Back);
    assert_eq!((m.view.top(), m.view.focus), (Some(Layer::Quick), Focus::SheetRow(2)), "Back returns to the Settings row");

    m.view.focus = Focus::SheetRow(1);
    press(&mut m, Key::Ok);
    assert_eq!((m.view.page, m.view.place, m.view.top()), (Page::Settings, Place::Accounts, None));
    press(&mut m, Key::Back);
    press(&mut m, Key::Back);
    assert_eq!((m.view.page, m.view.focus), (Page::Home, Focus::Hero(0)), "the way back is the page's own focus");

    press(&mut m, Key::Menu);
    m.view.focus = Focus::SheetRow(3);
    press(&mut m, Key::Ok);
    assert_eq!((m.view.page, m.view.selected, m.view.top()), (Page::Game, Some(hades), None));

    press(&mut m, Key::Menu);
    m.view.focus = Focus::SheetRow(2);
    press(&mut m, Key::Ok);
    press(&mut m, Key::Menu);
    assert!(m.view.layers.is_empty(), "Menu again closes the menu and Quick settings over it");
}

#[test]
fn quick_settings_step_round_and_persist() {
    let mut m = home();
    m.view.focus = Focus::Nav(3);
    press(&mut m, Key::Ok);
    assert_eq!(m.view.top(), Some(Layer::Quick), "the avatar opens the menu");
    m.view.focus = Focus::SheetRow(2);
    press(&mut m, Key::Ok);
    let saved = |m: &Model, key: &str| m.state().profile().ui[key].clone();

    assert_eq!(QuickSetting::Stream.value(&m), "4K, 60 fps");
    press(&mut m, Key::Right);
    assert_eq!((QuickSetting::Stream.value(&m), saved(&m, "fps")), ("4K, 120 fps".into(), serde_json::json!(120)));
    assert!(texts(&m).iter().any(|t| t == "4K, 120 fps"));
    press(&mut m, Key::Right);
    assert_eq!((QuickSetting::Stream.value(&m), saved(&m, "height")), ("720p, 60 fps".into(), serde_json::json!(720)), "the last preset steps round");
    press(&mut m, Key::Left);
    assert_eq!(QuickSetting::Stream.value(&m), "4K, 120 fps");

    m.view.focus = Focus::SheetRow(1);
    press(&mut m, Key::Left);
    assert_eq!(saved(&m, "hdr"), serde_json::json!(true), "a switch flips either way");

    m.step(Input::StatsSetting(crate::game::stats::Overlay { mode: crate::game::stats::Mode::Full, changed: 1 }));
    m.view.focus = Focus::SheetRow(2);
    press(&mut m, Key::Right);
    assert!(!QuickSetting::Stats.on(&m));
    press(&mut m, Key::Right);
    assert_eq!(m.prefs.stats.mode, crate::game::stats::Mode::Full, "on again is the last mode used");

    m.view.focus = Focus::SheetRow(3);
    let first = QuickSetting::Theme.value(&m);
    for _ in 0..lumatv_tokens::themes::game_options().len() {
        press(&mut m, Key::Right);
    }
    assert_eq!(QuickSetting::Theme.value(&m), first);

    m.view.focus = Focus::SheetRow(4);
    press(&mut m, Key::Left);
    assert_eq!((QuickSetting::TextSize.value(&m), saved(&m, "text_percent")), ("200%".into(), serde_json::json!(200)));

    m.view.focus = Focus::SheetAction(1);
    press(&mut m, Key::Ok);
    assert_eq!((m.view.page, m.view.place, m.view.top()), (Page::Settings, Place::ALL[0], None), "All settings opens the first place");
}

#[test]
fn pin_and_hide_round_trip() {
    let mut m = home();
    m.view.page = Page::Library;
    let celeste = id(&m, "Celeste");
    let at = m.filtered.iter().position(|g| *g == celeste).unwrap();
    m.view.focus = Focus::Grid(at);
    m.rebuild_navigation();
    press(&mut m, Key::Menu);
    assert_eq!(m.view.top(), Some(Layer::Quick), "Menu in Library opens the quick menu, not the sort");
    let pinned = |m: &Model| m.home_rows.iter().any(|r| r.title == "Favourites" && r.items.contains(&HomeItem::Game(celeste)));

    m.view.focus = Focus::SheetRow(4);
    press(&mut m, Key::Ok);
    assert!(pinned(&m) && m.view.top() == Some(Layer::Quick));
    assert!(texts(&m).iter().any(|t| t == "Unpin from Home"));
    press(&mut m, Key::Ok);
    assert!(!pinned(&m));

    m.view.focus = Focus::SheetRow(5);
    press(&mut m, Key::Ok);
    assert!(m.view.layers.is_empty() && !m.filtered.contains(&celeste));
    assert_eq!(m.view.focus, Focus::Grid(at.min(m.filtered.len() - 1)));
    assert!(!m.home_rows.iter().any(|r| r.items.contains(&HomeItem::Game(celeste))));
    m.set_hidden(celeste, false, &mut Vec::new());
    assert!(m.filtered.contains(&celeste));

    let mut m = home();
    let hades = id(&m, "Hades II");
    m.view.focus = Focus::Hero(1);
    press(&mut m, Key::Ok);
    assert_eq!(m.view.page, Page::Game);
    press(&mut m, Key::Menu);
    m.view.focus = Focus::SheetRow(5);
    press(&mut m, Key::Ok);
    assert_eq!(m.view.page, Page::Home, "hiding the open game leaves its page");
    assert!(m.view.featured.is_some_and(|g| g != hades), "and Home features another");
}

#[test]
fn menu_over_the_strip_does_nothing() {
    let mut m = fixture();
    m.view.page = Page::Game;
    m.view.selected = Some(id(&m, "Cyberpunk 2077"));
    m.view.focus = Focus::HubAction(0);
    activate(&mut m);
    let session = m.session().unwrap().id;
    event(&mut m, "gfn", ipc::Event::Session { session, state: SessionState::Running { stream: None } });
    m.step(Input::Overlay);
    assert!(m.strip_visible() && !m.menu_allowed());
    let stats = m.prefs.stats.mode;
    press(&mut m, Key::Menu);
    assert_eq!((m.view.top(), m.prefs.stats.mode), (None, stats));
}

#[test]
fn choosing_a_mode_in_quick_settings_mid_game_starts_and_stops_sampling() {
    use crate::game::stats::{Mode,Overlay};
    let mut m=fixture();
    m.view.page=Page::Game;m.view.selected=Some(id(&m,"Cyberpunk 2077"));m.view.focus=Focus::Play;
    activate(&mut m);
    let session=m.session().unwrap().id;
    event(&mut m,"gfn",ipc::Event::Session {session,state:SessionState::Running {stream:None}});
    m.step(Input::Tick {now_ms:6_000,unix_s:0});
    let flags=|commands:Vec<Command>| commands.into_iter().find_map(|c| match c {
        Command::Send {request:ipc::Request::Stats {on,pad_timing,..},..}=>Some((on,pad_timing)),
        _=>None,
    });
    assert_eq!(m.view.page,Page::Running);
    assert_eq!(flags(m.step(Input::StatsSetting(Overlay {mode:Mode::Full,changed:1}))),Some((true,true)),"the shared setting alone must ask the engine for stream and controller reports");
    assert_eq!(flags(m.step(Input::StatsSetting(Overlay {mode:Mode::Off,changed:2}))),Some((false,false)));
}
