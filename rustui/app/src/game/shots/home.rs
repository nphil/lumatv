//! Home shots, its walks and its tests, and the carousel motion run.
use super::budget::{Step, Walk, taps};
use super::*;
use crate::game::model::nav::Hints;

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
pub(super) fn screens(_base: &Model, add: &mut impl FnMut(&str, Model)) {
    let base = &home();
    add("home-a-spotlight", base.clone());
    let mut m = base.clone();
    m.prefs.light = true;
    add("home-a-spotlight-light", m);
    let mut m = base.clone();
    let quake = m.home_games.iter().position(|g| *g == id(&m, "Quake")).expect("Quake, which Steam does not know, is on the row");
    for _ in 0..quake {
        press(&mut m, Key::Right);
    }
    add("home-a-spotlight-native", m);
    let mut m = base.clone();
    press(&mut m, Key::Down);
    add("home-more-game-page", m);
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

/// Right along the row with the art resting on each game.
pub(super) fn walks() -> Vec<Walk> {
    let mut right = taps(&[(Key::Right, 400), (Key::Right, 400), (Key::Right, 120), (Key::Right, 120)]);
    right.push(Step::Settle);
    right.push(Step::Expect("the stage four tiles on", |m, _| matches!(m.view.focus, Focus::Home(Spot::Tile(i)) if i >= 4 && m.view.featured == m.home_games.get(i).copied())));
    // Down opens the game's page, and Back from it is Home on the same tile.
    let mut page = taps(&[(Key::Right, 300), (Key::Down, 300)]);
    page.push(Step::Expect("on the game's page", |m, _| m.view.page == Page::Game));
    page.extend(taps(&[(Key::Back, 300)]));
    page.push(Step::Expect("back on the tile", |m, _| m.view.page == Page::Home && m.view.focus == m.home_focus()));
    vec![Walk { name: "home-right-walk".into(), base: home(), script: right }, Walk { name: "home-down-up".into(), base: home(), script: page }]
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
fn down_opens_the_games_page_and_back_is_home_on_the_same_tile() {
    let mut m = home();
    let (at, game) = (m.view.focus, tile(&m).unwrap());
    assert_eq!(m.hints(), Hints(vec![(Key::Down, "More"), (Key::Menu, "Menu")]));
    press(&mut m, Key::Down);
    assert_eq!((m.view.page, m.view.selected, m.view.focus), (Page::Game, Some(game), Focus::Play));
    press(&mut m, Key::Back);
    assert_eq!((m.view.page, m.view.focus), (Page::Home, at));
}

#[test]
fn ok_on_a_tile_plays() {
    let mut m = home();
    let out = press(&mut m, Key::Ok);
    assert!(!out.is_empty() || m.view.page != Page::Home || !m.view.layers.is_empty(), "OK plays the focused game");
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
    let mut clock = 1_000;
    // Frames a tick apart: the stage art waits on the model's clock, and a picture decodes on its own thread.
    let mut rest = |shots: &mut Shots, m: &mut Model, frames: u32| -> io::Result<()> {
        for _ in 0..frames {
            clock += 17;
            m.step(Input::Tick { now_ms: clock, unix_s: 100 });
            shots.page.show(m)?;
            shots.frame()?;
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    };
    rest(&mut shots, &mut m, 60)?;
    let key = if mode == "right" || mode == "1" { Key::Right } else { Key::Down };
    if key == Key::Right {
        // The next game's banner is decoded before the first press, so the run shows the morph and not the wait for the art.
        press(&mut m, Key::Right);
        rest(&mut shots, &mut m, 60)?;
        press(&mut m, Key::Left);
        rest(&mut shots, &mut m, 60)?;
    }
    let mut peak = (0, 0);
    for press in 0..8 {
        m.step(Input::Key { key, down: true });
        m.step(Input::Key { key, down: false });
        for tick in 0..22 {
            rest(&mut shots, &mut m, 1)?;
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
