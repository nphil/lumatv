//! Home shots, its walks and its tests, and the carousel motion run.
use super::budget::{Step, Walk, taps};
use super::*;
use crate::game::model::{home::HomeCard, nav::Hints};

fn press(m: &mut Model, key: Key) -> Vec<Command> {
    m.step(Input::Key { key, down: true });
    m.step(Input::Key { key, down: false })
}
fn tile(m: &Model) -> Option<GameId> {
    match m.view.focus {
        Focus::Home(Spot::Tile(i)) => m.home_games.get(i).copied(),
        _ => None,
    }
}
/// The fixture with Steam linked for Hades II and the game pages on, so the featured game has every card; back on Home with Hades II featured.
fn home() -> Model {
    let mut m = with_hades_community(fixture());
    m.prefs.game_pages = true;
    m.view.page = Page::Home;
    m.view.pane = None;
    m.view.featured = Some(id(&m, "Hades II"));
    m.view.focus = m.home_focus();
    m.rebuild_navigation();
    m
}
fn down(base: &Model) -> Model {
    let mut m = base.clone();
    press(&mut m, Key::Down);
    m
}

pub(super) fn screens(_base: &Model, add: &mut impl FnMut(&str, Model)) {
    let base = &home();
    add("home-a-spotlight", base.clone());
    let mut m = base.clone();
    m.prefs.light = true;
    add("home-a-spotlight-light", m);
    add("home-a-spotlight-down", down(base));
    let mut m = base.clone();
    let quake = m.home_games.iter().position(|g| *g == id(&m, "Quake")).expect("Quake, which Steam does not know, is on the row");
    for _ in 0..quake {
        press(&mut m, Key::Right);
    }
    add("home-a-spotlight-native", down(&m));
}
pub(super) fn text200(base: &Model, add: &mut impl FnMut(&str, Model)) {
    for (name, page) in [("home-a-spotlight-text200", Page::Home), ("31-hub-text200", Page::Game)] {
        let mut m = base.clone();
        m.prefs.text_percent = 200;
        m.view.page = page;
        m.view.focus = if page == Page::Game { Focus::Play } else { m.home_focus() };
        add(name, m);
    }
}

/// Right along the row with the art resting on each game, and Down to the cards and back.
pub(super) fn walks() -> Vec<Walk> {
    let mut right = taps(&[(Key::Right, 400), (Key::Right, 400), (Key::Right, 120), (Key::Right, 120)]);
    right.push(Step::Settle);
    right.push(Step::Expect("the stage four tiles on", |m, _| matches!(m.view.focus, Focus::Home(Spot::Tile(i)) if i >= 4 && m.view.featured == m.home_games.get(i).copied())));
    let mut down = taps(&[(Key::Down, 300)]);
    down.push(Step::Expect("on the first card", |m, _| matches!(m.view.focus, Focus::Home(Spot::Card(_)))));
    down.extend(taps(&[(Key::Right, 200), (Key::Up, 300)]));
    down.push(Step::Expect("back on the tile", |m, _| m.view.focus == m.home_focus()));
    vec![Walk { name: "home-right-walk".into(), base: home(), script: right }, Walk { name: "home-down-up".into(), base: home(), script: down }]
}

#[test]
fn right_moves_one_tile_and_the_stage_follows_it() {
    let mut m = home();
    let first = tile(&m).unwrap();
    assert_eq!(m.view.featured, Some(first));
    press(&mut m, Key::Right);
    let next = tile(&m).unwrap();
    assert_ne!(next, first);
    assert_eq!(m.view.featured, Some(next), "the stage and the fetch ahead follow the focused tile");
}

#[test]
fn down_shows_only_present_cards_and_up_returns_to_the_same_tile() {
    let mut m = home();
    let at = m.view.focus;
    let cards = m.home_cards();
    assert!(cards.len() > 1);
    press(&mut m, Key::Down);
    assert_eq!(m.view.focus, Focus::Home(Spot::Card(cards[0])));
    assert_eq!(m.hints(), Hints(vec![(Key::Up, "Games"), (Key::Menu, "Menu")]));
    for _ in 1..cards.len() {
        press(&mut m, Key::Right);
    }
    assert_eq!(m.view.focus, Focus::Home(Spot::Card(*cards.last().unwrap())));
    press(&mut m, Key::Up);
    assert_eq!(m.view.focus, at);
}

#[test]
fn ok_on_a_tile_plays_and_ok_on_a_card_opens_the_game() {
    let mut m = home();
    let game = tile(&m).unwrap();
    let out = press(&mut m, Key::Ok);
    assert!(!out.is_empty() || m.view.page != Page::Home || !m.view.layers.is_empty(), "OK plays the focused game");

    let mut m = home();
    press(&mut m, Key::Down);
    press(&mut m, Key::Ok);
    assert_eq!((m.view.page, m.view.selected), (Page::Game, Some(game)));
}

#[test]
fn a_game_steam_does_not_know_has_no_steam_cards() {
    let mut m = home();
    let quake = m.home_games.iter().position(|g| *g == id(&m, "Quake")).expect("Quake, which Steam does not know, is on the row");
    for _ in 0..quake {
        press(&mut m, Key::Right);
    }
    assert!(m.home_cards().iter().all(|c| *c == HomeCard::LastPlayed));
}

#[test]
fn carousel_motion_frames() -> io::Result<()> {
    if std::env::var_os("GAME_MOTION").is_none() {
        return Ok(());
    }
    let scratch = std::env::var_os("CODY_SCRATCH").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let out = scratch.join("lumagame-motion");
    std::fs::create_dir_all(&out)?;
    let mut m = home();
    m.prefs.reduce_motion = std::env::var_os("GAME_MOTION_REDUCED").is_some();
    let mode = std::env::var("GAME_MOTION").unwrap_or_default();
    if mode == "grid" {
        m.view.page = Page::Library;
        m.view.focus = Focus::Grid(0);
    }
    m.rebuild_navigation();
    let mut shots = Shots::new()?;
    shots.shot(&m)?;
    if mode == "down" {
        assert!(shots.page.has_stage(), "the Home backdrop must be loaded before scrolling");
    }
    let key = if mode == "right" || mode == "1" { Key::Right } else { Key::Down };
    let mut peak = (0, 0);
    for press in 0..8 {
        m.step(Input::Key { key, down: true });
        m.step(Input::Key { key, down: false });
        for tick in 0..10 {
            shots.page.show(&m)?;
            shots.frame()?;
            if mode == "down" {
                assert!(shots.page.has_stage(), "the Home backdrop disappeared while scrolling");
            }
            let (_, groups) = shots.page.retained_counts();
            let nodes = shots.page.ui.scene.live_layers();
            peak = (peak.0.max(nodes), peak.1.max(groups));
            assert!(!shots.page.ui.scene.take_overflow(), "carousel exceeded the layer limit");
            assert!(nodes <= ui::limits::MAX_DRAWABLE_NODES && groups <= 9, "unbounded carousel: {nodes} layers, {groups} scroll groups");
            png(&out.join(format!("p{press}-f{tick:02}.png")), 1920, 1080, &shots.engine.read_page(0))?;
        }
    }
    if mode == "grid" {
        let mut rapid = fixture();
        rapid.prefs.reduce_motion = m.prefs.reduce_motion;
        rapid.view.page = Page::Library;
        rapid.view.focus = Focus::Grid(0);
        rapid.rebuild_navigation();
        shots.shot(&rapid)?;
        for _ in 0..24 {
            rapid.step(Input::Key { key: Key::Down, down: true });
            shots.page.show(&rapid)?;
            shots.frame()?;
            let (_, groups) = shots.page.retained_counts();
            let nodes = shots.page.ui.scene.live_layers();
            peak = (peak.0.max(nodes), peak.1.max(groups));
            assert!(!shots.page.ui.scene.take_overflow());
            assert!(nodes <= ui::limits::MAX_DRAWABLE_NODES && groups <= 9);
            assert!(matches!(rapid.view.focus, Focus::Grid(_)), "focus left the grid during repeated navigation");
        }
    }
    eprintln!("{mode}: peak {} layers, {} scroll groups", peak.0, peak.1);
    Ok(())
}
