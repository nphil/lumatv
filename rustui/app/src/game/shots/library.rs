//! Library, search and refresh shots.
use super::*;

pub(super) fn screens(base: &Model, add: &mut impl FnMut(&str, Model)) {
    let mut m = base.clone();
    m.view.page = Page::Library;
    m.view.focus = Focus::Grid(2);
    add("07-library", m.clone());
    m.view.push(Layer::Sort, m.view.focus);
    m.view.focus = Focus::SheetRow(0);
    add("07-library-sort", m);
}
pub(super) fn search(base: &Model, add: &mut impl FnMut(&str, Model)) {
    let mut m = base.clone();
    (m.view.page, m.view.search) = (Page::Library, true);
    m.view.query = "Hades".into();
    m.filtered = vec![id(&m, "Hades II")];
    m.view.focus = Focus::Grid(0);
    add("37-search", m);
    let mut m = base.clone();
    (m.view.page, m.view.search) = (Page::Library, true);
    m.view.focus = Focus::Keyboard(0);
    add("38-search-keyboard", m);
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
        let mut library = working.clone();
        library.view.page = Page::Library;
        library.view.focus = Focus::Filter(library.integrations.iter().filter(|e| library.enabled(&e.manifest.id)).count() + 4);
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
            ("72-library-refresh", library),
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
