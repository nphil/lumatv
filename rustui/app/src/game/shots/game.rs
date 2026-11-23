//! Game page shots: the Glance page, its sections, the reader, the in-place Steam sign-in, the states the page shows, and the chooser.
use super::budget::{Step, Walk, taps};
use super::*;
use crate::game::model::{facts::Section, game::Entry, tests::linked};

fn open(mut m: Model, title: &str, section: Option<Section>) -> Model {
    let game = id(&m, title);
    m.open_game(game, section);
    m
}

/// Hades II with Steam on for achievements and the community but not signed in, or with its sign-in expired.
fn signed_out(base: &Model, expired: bool) -> Model {
    use lumagame_core::engine::{AuthMethod, EngineKind};
    let mut m = base.clone();
    let manifest = crate::game::model::tests::manifest("steam", "Steam", EngineKind::Account);
    m.integrations.push(Integration { manifest, descriptor: None, schema: SettingsSchema::default(), targets: Vec::new(), error: None });
    crate::game::model::tests::describe(&mut m, "steam", vec![AuthMethod::Qr], &[Feature::Achievements, Feature::Community]);
    if expired {
        event(&mut m, "steam", ipc::Event::Targets { targets: vec![ipc::Target { name: "Nitin".into(), link: ipc::Link::Expired, ..linked("1") }] });
    }
    let game = lumagame_core::ids::ProviderGameId::new("1145350").unwrap();
    event(&mut m, "steam", ipc::Event::Community { game, community: hades_community(1_800_000_000) });
    open(m, "Hades II", Some(Section::Activity))
}

pub(super) fn screens(base: &Model, add: &mut impl FnMut(&str, Model)) {
    let hades = with_hades_community(base.clone());
    let mut states = vec![
        ("game-c-glance", open(hades.clone(), "Hades II", None)),
        ("game-c-glance-community", open(hades.clone(), "Hades II", Some(Section::Community))),
        ("steam-b-in-place", signed_out(base, false)),
        ("steam-b-in-place-stale", signed_out(base, true)),
        ("game-c-no-steam-record", open(hades.clone(), "Cyberpunk 2077", Some(Section::Activity))),
        ("game-c-absent", open(hades.clone(), "Celeste Classic", Some(Section::Community))),
    ];
    let mut big = open(hades.clone(), "Hades II", None);
    big.prefs.text_percent = 200;
    states.push(("game-c-glance-text200", big));
    let mut reader = open(hades.clone(), "Hades II", Some(Section::Activity));
    let news = reader.section_entries(id(&reader, "Hades II"), Section::Activity).iter().position(|e| matches!(e, Entry::News(0))).unwrap();
    reader.view.focus = Focus::Entry(news);
    reader.rebuild_navigation();
    activate(&mut reader);
    states.push(("game-b-split-reader", reader));
    let mut offline = open(base.clone(), "Hades II", None);
    event(&mut offline, "gfn", ipc::Event::Targets { targets: vec![ipc::Target { name: "nitin · Ultimate".into(), reach: ipc::Reach::Offline, ..linked("main") }] });
    states.push(("game-c-offline", offline));
    let mut failed = open(base.clone(), "Hades II", None);
    activate(&mut failed);
    if let Some(session) = failed.session().map(|s| s.id) {
        let state = SessionState::Failed { reason: ipc::FailReason::Engine, message: "The connection was interrupted.".into() };
        event(&mut failed, "gfn", ipc::Event::Session { session, state });
    }
    failed.view.page = Page::Game;
    failed.view.focus = Focus::Play;
    states.push(("game-c-try-again", failed));
    for (name, m) in states {
        let mut light = m.clone();
        light.prefs.light = true;
        add(name, m);
        add(&format!("{name}-light"), light);
    }
    let mut m = base.clone();
    m.view.page = Page::Game;
    m.view.selected = Some(id(&m, "Quake"));
    m.view.push(Layer::Chooser, m.view.focus);
    m.view.chooser_selected = m.selected_game().unwrap().sources.first().map(|s| s.key());
    m.view.focus = Focus::SheetRow(1);
    add("09-chooser", m.clone());
    let mut light = m.clone();
    light.prefs.light = true;
    add("09-chooser-light", light);
    m.prefs.text_percent = 200;
    add("32-chooser-text200", m);
}
pub(super) fn failed(base: &Model, add: &mut impl FnMut(&str, Model)) {
    let mut m = base.clone();
    m.view.page = Page::Game;
    m.view.notice = Some("The connection was interrupted. Check the network, then choose Play to try again.".into());
    add("20-state-failed", m);
}

/// The game page's walks: a section opening over the art and closing (one dip of the art each way), switching sections, and paging the
/// reader.
pub(super) fn walks() -> Vec<Walk> {
    let mut base = open(with_hades_community(fixture()), "Hades II", None);
    base.view.focus = Focus::Card(0);
    let mut open_close = vec![Step::Tap(Key::Ok), Step::Settle];
    open_close.extend([
        Step::Expect("the Activity section open", |model, _| model.view.top() == Some(Layer::Section(Section::Activity))),
        Step::Expect("the art to dip to its dimmed copy once", |_, page| page.art_swaps() == 1),
        Step::Tap(Key::Back),
        Step::Settle,
        Step::Expect("back on the Activity card", |model, _| model.view.focus == Focus::Card(0)),
        Step::Expect("the art to dip back to its own picture", |_, page| page.art_swaps() == 2),
    ]);
    let mut switch = vec![Step::Tap(Key::Ok), Step::Settle];
    switch.extend(taps(&[(Key::Right, 300), (Key::Right, 300), (Key::Left, 300)]));
    switch.push(Step::Expect("on Community after Right, Right, Left", |model, _| model.view.top() == Some(Layer::Section(Section::Community))));
    let mut reader = base.clone();
    reader.view.focus = Focus::Card(0);
    let mut page = vec![Step::Tap(Key::Ok), Step::Settle];
    page.extend(taps(&[(Key::Down, 300), (Key::Down, 300)]));
    page.extend([Step::Tap(Key::Ok), Step::Settle]);
    page.extend(taps(&[(Key::Down, 300), (Key::Down, 300)]));
    page.push(Step::Expect("the long post in the reader", |model, _| model.reader_text().is_some_and(|r| r.title.starts_with("Development update"))));
    page.push(Step::Expect("the reader on its third page", |model, _| model.view.top() == Some(Layer::Reader) && model.view.focus == Focus::ReaderPage(2)));
    vec![
        Walk { name: "game-section-open-close".into(), base: base.clone(), script: open_close },
        Walk { name: "game-section-switch".into(), base, script: switch },
        Walk { name: "game-reader-page".into(), base: reader, script: page },
    ]
}
