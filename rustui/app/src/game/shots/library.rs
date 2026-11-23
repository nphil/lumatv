//! Library shots, its walks, and the refresh shots.
use super::budget::{Step, Walk, taps};
use super::*;

/// The library on its first cover.
fn library(base: &Model) -> Model {
    let mut m = base.clone();
    m.view.page = Page::Library;
    m.view.focus = Focus::Grid(0);
    m.rebuild_filter();
    m.rebuild_navigation();
    m
}

pub(super) fn screens(base: &Model, add: &mut impl FnMut(&str, Model)) {
    let m = library(base);
    add("library-b-panel", m.clone());
    let mut light = m.clone();
    light.prefs.light = true;
    add("library-b-panel-light", light);
    let mut large = m.clone();
    large.prefs.text_percent = 200;
    add("library-b-panel-text200", large);
    let mut refine = m.clone();
    refine.view.push(Layer::PlaysOn, Focus::SheetRow(0));
    add("library-b-panel-plays-on", refine);
    let mut search = m.clone();
    search.view.search = true;
    search.view.query = "HA".into();
    search.view.focus = Focus::Keyboard(7);
    search.rebuild_filter();
    add("library-b-panel-search", search.clone());
    search.view.query = "ZZQ".into();
    search.rebuild_filter();
    add("library-no-results", search);
    let mut empty = Model::new(lumagame_core::state::State::default(), Listing::default(), vec![]);
    empty.view.page = Page::Library;
    empty.view.focus = Focus::SheetAction(0);
    add("library-empty", empty);
}

/// The grid scrolled two rows down and back, and focus into the panel and out again: covers stream in at most one per frame and the
/// panel's tint glass is never captured.
pub(super) fn walks() -> Vec<Walk> {
    let mut down = taps(&[(Key::Down, 120), (Key::Down, 120), (Key::Down, 120), (Key::Up, 120), (Key::Up, 120), (Key::Up, 300)]);
    down.push(Step::Expect("back on the first cover", |m, _| m.view.focus == Focus::Grid(0)));
    let mut panel = taps(&[(Key::Left, 200), (Key::Down, 120), (Key::Ok, 300)]);
    panel.push(Step::Expect("Favourites shown", |m, _| m.view.filter.show == crate::game::model::library::Show::Favourites));
    panel.extend(taps(&[(Key::Up, 120), (Key::Ok, 300), (Key::Right, 300)]));
    panel.push(Step::Expect("back in the grid", |m, _| matches!(m.view.focus, Focus::Grid(_))));
    vec![
        Walk { name: "library-grid-scroll".into(), base: library(&fixture()), script: down },
        Walk { name: "library-panel-in-out".into(), base: library(&fixture()), script: panel },
    ]
}
/// The refresh states over Home, the library and the game page, in both themes at 100% and 200% text.
pub(super) fn refresh_screens(base: &Model) -> Vec<(String, Model)> {
    let mut out = Vec::new();
    for (suffix, light, text) in [("dark100", false, 100u16), ("light100", true, 100), ("dark200", false, 200), ("light200", true, 200)] {
        let mut base = base.clone();
        base.prefs.light = light;
        base.prefs.text_percent = text;
        base.integrations.iter_mut().find(|e| e.manifest.id.as_str() == "gfn").unwrap().descriptor.as_mut().unwrap().capabilities.features.insert(Feature::StoreSync);
        let gfn = EngineId::new("gfn").unwrap();
        let mut working = base.clone();
        working.view.focus = Focus::Hero(2);
        working.refresh_libraries(Some(gfn.clone()), &mut Vec::new());
        let mut failed = working.clone();
        event(&mut failed, "gfn", ipc::Event::Error { code: ipc::ErrorCode::Network, message: "offline".into() });
        let mut done = working.clone();
        let mut entries: Vec<_> = done.games().iter().flat_map(|g| g.sources.iter()).filter(|s| s.engine == gfn).map(|s| s.entry.clone()).collect();
        let mut fresh = entries[0].clone();
        fresh.id = lumagame_core::ids::ProviderGameId::new("silksong").unwrap();
        fresh.title = "Hollow Knight: Silksong".into();
        fresh.external.clear();
        entries.push(fresh);
        event(&mut done, "gfn", ipc::Event::Library { target: TargetId::new("main").unwrap(), entries, next: None });
        let hub = |title: &str, tab: usize| {
            let mut m = base.clone();
            m.view.page = Page::Game;
            m.view.selected = Some(id(&m, title));
            m.view.hub_tab = tab;
            m.view.focus = if tab == 0 && text == 100 { Focus::HubAction(0) } else if tab == 2 || tab == 3 { Focus::Item(0) } else { Focus::Tab(tab) };
            m
        };
        for (name, m) in [
            ("70-refresh-working", working),
            ("71-refresh-failed", failed),
            ("71-refresh-result", done),
            ("73-hub-steam", hub("Hades II", 0)),
            ("74-hub-achievements", hub("Hades II", 2)),
            ("75-hub-news", hub("Hades II", 3)),
            ("76-hub-details", hub("Hades II", 5)),
            ("77-hub-gfn-only", hub("Cyberpunk 2077", 0)),
            ("78-hub-native", hub("Celeste Classic", 0)),
        ] {
            let mut m = m;
            m.rebuild_navigation();
            out.push((format!("{name}-{suffix}"), m));
        }
    }
    out
}
