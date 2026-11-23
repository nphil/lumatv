//! Home shots and the carousel motion run.
use super::*;

pub(super) fn screens(base: &Model, add: &mut impl FnMut(&str, Model)) {
    add("01-home", base.clone());
    let mut m = base.clone();
    m.prefs.light = true;
    add("01-home-light", m);
    let mut m = base.clone();
    m.view.focus = Focus::Hero(0);
    add("01-home-play-focus", m.clone());
    m.prefs.light = true;
    add("01-home-play-focus-light", m);
    let mut m = base.clone();
    m.view.focus = Focus::Hero(1);
    add("01-home-details-focus", m);
    for (name, row, item) in [("02-home-most-played", 1, 2), ("03-home-genres", 2, 1), ("04-home-native-plate", 3, 2)] {
        let mut m = base.clone();
        m.view.home_row = row;
        m.view.focus = Focus::Home(row, item);
        add(name, m);
    }
    for (name, density) in [("05-home-cinematic", Density::Cinematic), ("06-home-compact", Density::Compact)] {
        let mut m = base.clone();
        m.prefs.hero = density;
        m.prefs.covers = density;
        add(name, m);
    }
}
pub(super) fn text200(base: &Model, add: &mut impl FnMut(&str, Model)) {
    for (name, page) in [("30-home-text200", Page::Home), ("31-hub-text200", Page::Game)] {
        let mut m = base.clone();
        m.prefs.text_percent = 200;
        m.view.page = page;
        m.view.focus = if page == Page::Game { Focus::Play } else { Focus::Home(0, 1) };
        add(name, m);
    }
}

#[test]
fn carousel_motion_frames() -> io::Result<()> {
    if std::env::var_os("GAME_MOTION").is_none() {
        return Ok(());
    }
    let scratch = std::env::var_os("CODY_SCRATCH").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let out = scratch.join("lumagame-motion");
    std::fs::create_dir_all(&out)?;
    let mut m = fixture();
    m.prefs.reduce_motion = std::env::var_os("GAME_MOTION_REDUCED").is_some();
    let mode = std::env::var("GAME_MOTION").unwrap_or_default();
    if mode == "grid" {
        m.view.page = Page::Library;
        m.view.focus = Focus::Grid(0);
    } else {
        m.view.focus = Focus::Home(0, 0);
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
