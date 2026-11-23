//! The Media section and the viewer: Hades II's store pictures and trailers, Steam's community pictures, artwork and videos, and the
//! section loading, empty and unreachable.
use super::budget::{Step, Walk, taps};
use super::*;
use crate::game::model::facts::Section;
use crate::game::model::game::media::Group;
use lumagame_core::community::{Absence, HubItem, Section as Part};

/// The fixtures' clock.
const NOW: u64 = 1_800_000_000;

/// Hades II's page with `section` open.
fn open(mut m: Model, section: Section) -> Model {
    let game = id(&m, "Hades II");
    m.open_game(game, Some(section));
    m
}

/// Hades II with Steam's community pictures, artwork and videos beside the store's.
fn with_hub(base: &Model) -> Model {
    let mut m = with_hades_community(base.clone());
    let item = |n: usize, title: &str, author: &str, youtube: Option<&str>| HubItem {
        id: (1000 + n).to_string(),
        title: title.into(),
        author: author.into(),
        image: format!("fixture://media/shot-{}.jpg", n % 5 + 1),
        youtube: youtube.map(str::to_owned),
        likes: 0,
        comments: 0,
    };
    let mut community = hades_community(NOW);
    community.hub_screenshots = Part::list((0..24).map(|n| item(n, "", "Melinoë", None)).collect());
    community.hub_artwork = Part::list((0..3).map(|n| item(n + 2, "Hecate", "inkwitch", None)).collect());
    community.hub_videos = Part::list(vec![item(4, "Speedrun, any %", "zag", Some("dQw4w9WgXcQ")), item(1, "First clear", "nyx", Some("9bZkp7q19f0"))]);
    event(&mut m, "steam", ipc::Event::Community { game: lumagame_core::ids::ProviderGameId::new("1145350").unwrap(), community });
    m
}

pub(super) fn screens(base: &Model, add: &mut impl FnMut(&str, Model)) {
    let hades = with_hades_community(base.clone());
    let media = open(hades.clone(), Section::Media);
    let mut states = vec![("game-b-split-media", media.clone())];
    let mut trailers = media.clone();
    trailers.view.media.group = Group::Trailers;
    trailers.view.focus = Focus::Media(1);
    states.push(("game-b-split-media-trailers", trailers));
    let mut hub = open(with_hub(base), Section::Media);
    hub.view.media.group = Group::Artwork;
    hub.view.focus = Focus::MediaFilter(3);
    states.push(("game-b-split-media-hub", hub));
    let mut loading = media.clone();
    // An answer made under the other age setting is not shown: the section waits for the next.
    loading.prefs.mature_pages = true;
    states.push(("game-b-split-media-loading", loading));
    let mut error = hades.clone();
    let mut unreachable = hades_community(NOW);
    unreachable.store_screenshots = Part::absent(Absence::Unreachable);
    unreachable.trailers = Part::absent(Absence::Unreachable);
    event(&mut error, "steam", ipc::Event::Community { game: lumagame_core::ids::ProviderGameId::new("1145350").unwrap(), community: unreachable });
    states.push(("game-b-split-media-error", open(error, Section::Media)));
    let mut none = hades.clone();
    let game = id(&none, "Celeste Classic");
    none.open_game(game, Some(Section::Media));
    states.push(("game-b-split-media-none", none));
    let mut viewer = open(with_hub(base), Section::Media);
    viewer.view.media.group = Group::Community;
    viewer.view.focus = Focus::Media(2);
    viewer.rebuild_navigation();
    activate(&mut viewer);
    states.push(("game-viewer", viewer.clone()));
    let mut hidden = viewer;
    hidden.step(Input::Tick { now_ms: hidden.now_ms() + 3_000, unix_s: NOW });
    states.push(("game-viewer-chrome-hidden", hidden));
    for (name, m) in states {
        let mut light = m.clone();
        light.prefs.light = true;
        add(name, m);
        add(&format!("{name}-light"), light);
    }
    let mut big = media;
    big.prefs.text_percent = 200;
    add("game-b-split-media-text200", big);
    let mut big = open(with_hub(base), Section::Media);
    big.view.media.group = Group::Artwork;
    big.view.focus = Focus::MediaFilter(3);
    big.prefs.text_percent = 200;
    add("game-b-split-media-hub-text200", big);
}

/// Stepping through the store screenshots in the viewer and back: each step one dip of the one art layer.
pub(super) fn walks() -> Vec<Walk> {
    let base = open(with_hades_community(fixture()), Section::Media);
    let mut script = vec![Step::Tap(Key::Ok), Step::Settle];
    script.push(Step::Expect("the viewer open on the first screenshot", |m, _| m.view.top() == Some(Layer::Viewer) && m.view.media.at == 0));
    script.extend(taps(&[(Key::Right, 400), (Key::Right, 400), (Key::Right, 400), (Key::Right, 400), (Key::Right, 400)]));
    script.push(Step::Settle);
    script.push(Step::Expect("the last screenshot holds", |m, _| m.view.media.at == 4));
    script.extend([Step::Tap(Key::Back), Step::Settle]);
    script.push(Step::Expect("back on the section, at the last screenshot", |m, _| {
        m.view.top() == Some(Layer::Section(Section::Media)) && m.view.focus == Focus::Media(4)
    }));
    vec![Walk { name: "media-viewer-step".into(), base, script }]
}
