//! Shared fixtures for the slices' model tests.
use super::*;

pub(crate) fn key(m: &mut Model, key: Key) -> Vec<Command> {
    m.step(Input::Key { key, down: true });
    m.step(Input::Key { key, down: false })
}
/// Sunshine's pairing sheet once its search has found no PC.
pub(crate) fn no_pc_found() -> Model {
    let mut m = Model::new(State::default(), Listing::default(), vec![manifest("gamestream", "Sunshine", EngineKind::Host)]);
    describe(&mut m, "gamestream", vec![lumagame_core::engine::AuthMethod::ShowPin], &[Feature::ManualHost]);
    m.begin_link(EngineId::new("gamestream").unwrap(), None, &mut Vec::new());
    m.step(Input::Tick { now_ms: PC_SEARCH_MS, unix_s: 0 });
    m
}
/// The launch screen of a game called `title` (with artwork when `art`) as it opens: nothing is known of the launch yet.
pub(crate) fn launching(title: &str, art: bool) -> Model {
    let mut m = Model::new(State::default(), Listing::default(), vec![manifest("gfn", "GeForce NOW", EngineKind::Cloud)]);
    describe(&mut m, "gfn", vec![], &[Feature::EndGame]);
    let source: SourceKey = "gfn|main|1".parse().unwrap();
    let link = |name: &str| art.then(|| name.to_string());
    let entry = ipc::LibraryEntry {
        id: source.id.clone(),
        title: title.into(),
        role: Default::default(),
        external: vec![],
        store: None,
        store_account: None,
        art: ipc::Art { hero: link("hero"), logo: link("logo"), ..Default::default() },
        readiness: Default::default(),
        resumable: false,
        activity: Default::default(),
    };
    m.games.push(Game {
        id: GameId(1),
        role: Default::default(),
        title: title.into(),
        sources: vec![catalog::Source { engine: source.engine.clone(), target: source.target.clone(), entry }],
        reports: Vec::new(),
    });
    if art {
        m.art.extend(["hero", "logo"].map(|name| (name.to_string(), PathBuf::from(name))));
    }
    m.start(GameId(1), source, false, &mut Vec::new());
    m.rebuild_navigation();
    m
}
/// What the engine says of the launch.
pub(crate) fn report(m: &mut Model, state: SessionState) {
    let session = m.session().expect("a launch").id;
    m.step(Input::Engine { engine: EngineId::new("gfn").unwrap(), message: FromEngine { re: None, event: ipc::Event::Session { session, state } } });
}
/// The Steam integration as the screenshots show it.
#[derive(Clone, Copy)]
pub(crate) enum SteamShot {
    SignedOut,
    Qr,
    /// An account linked, Steam's records and GeForce NOW's favourites merged into the shelves.
    SignedIn,
}
/// The screenshot fixture with the Steam integration added to its Integrations page.
pub(crate) fn with_steam(mut m: Model, shot: SteamShot) -> Model {
    use lumagame_core::activity::Activity;
    let at = m.integrations.iter().rposition(|e| e.manifest.kind == EngineKind::Host).map_or(0, |i| i + 1);
    let integration = Integration { manifest: manifest("steam", "Steam", EngineKind::Account), descriptor: None, schema: SettingsSchema::default(), targets: Vec::new(), error: None };
    m.integrations.insert(at, integration);
    m.view.page = Page::Accounts;
    m.view.selected_engine = Some(EngineId::new("steam").unwrap());
    m.view.focus = Focus::Setting(0);
    describe(&mut m, "steam", vec![lumagame_core::engine::AuthMethod::Qr], &[]);
    m.view.focus = Focus::Setting(1);
    match shot {
        SteamShot::SignedOut => {}
        SteamShot::Qr => {
            let state = AuthState::Qr { url: "https://s.team/q/1/11860205094928508090".into(), expires_s: 252 };
            engine_says(&mut m, "steam", ipc::Event::Auth { state });
        }
        SteamShot::SignedIn => {
            let (now, day, account) = (m.unix_s, 86_400, "76561198000000000");
            let gfn = EngineId::new("gfn").unwrap();
            let mut entries: Vec<ipc::LibraryEntry> = m.listing.sources.iter().filter(|s| s.engine == gfn).map(|s| s.entry.clone()).collect();
            for e in &mut entries {
                let favourite = ["Baldur's Gate 3", "Cyberpunk 2077", "Disco Elysium"].contains(&e.title.as_str());
                e.activity = Activity { favourite, ..Activity::default() };
            }
            engine_says(&mut m, "gfn", ipc::Event::Library { target: TargetId::new("main").unwrap(), entries, next: None });
            let records = [
                ("1245620", 1_800, 412, &["Action", "RPG"][..], &["Souls-like", "Open World", "Difficult"][..]),
                ("1086940", 3 * day, 160, &["RPG", "Strategy"][..], &["Turn-Based", "Story Rich", "Co-op"][..]),
                ("367520", 3 * day, 81, &["Action", "Adventure"][..], &["Metroidvania", "Hand-drawn"][..]),
                ("2379780", day, 64, &["Strategy"][..], &["Card Game", "Roguelike"][..]),
                ("504230", 12 * day, 31, &["Platformer"][..], &["Precision Platformer", "Pixel Graphics"][..]),
                ("620", 40 * day, 9, &["Puzzle"][..], &["Co-op", "First-Person"][..]),
                ("632470", 20 * day, 38, &["RPG"][..], &["Story Rich", "Detective"][..]),
            ];
            let records = records
                .into_iter()
                .map(|(appid, ago, hours, genres, tags)| {
                    let words = |w: &[&str]| w.iter().map(|s| s.to_string()).collect();
                    let activity = Activity { last_played: Some(now - ago), playtime_s: Some(hours * 3600), genres: words(genres), tags: words(tags), ..Activity::default() };
                    entry(appid, appid, ipc::Role::Record, &[&format!("steam:{appid}")], activity)
                })
                .collect();
            let mut target = linked(account);
            target.name = "Nitin".into();
            engine_says(&mut m, "steam", ipc::Event::Targets { targets: vec![target] });
            engine_says(&mut m, "steam", ipc::Event::Library { target: TargetId::new(account).unwrap(), entries: records, next: None });
            m.view.page = Page::Accounts;
            m.view.focus = Focus::Setting(1);
        }
    }
    m
}
pub(crate) fn manifest(id: &str, name: &str, kind: EngineKind) -> Manifest {
    Manifest { id: EngineId::new(id).unwrap(), name: name.into(), summary: String::new(), kind, protocol: 1, exec: None, args: Vec::new(), license: "MIT".into(), source: None }
}
pub(crate) fn describe(m: &mut Model, id: &str, auth: Vec<lumagame_core::engine::AuthMethod>, features: &[Feature]) {
    let id = EngineId::new(id).unwrap();
    m.state.engines.entry(id.clone()).or_default().enabled = true;
    let e = m.integrations.iter_mut().find(|e| e.manifest.id == id).unwrap();
    e.descriptor = Some(Descriptor {
        id,
        version: "1".into(),
        capabilities: lumagame_core::engine::Capabilities {
            kind: e.manifest.kind,
            targets: lumagame_core::engine::TargetKind::Host,
            auth,
            presentation: Presentation::Feeder,
            features: features.iter().copied().collect(),
            video: None,
        },
    });
}
pub(crate) fn entry(id: &str, title: &str, role: ipc::Role, external: &[&str], activity: lumagame_core::activity::Activity) -> ipc::LibraryEntry {
    ipc::LibraryEntry {
        id: lumagame_core::ids::ProviderGameId::new(id).unwrap(),
        title: title.into(),
        role,
        external: external.iter().map(|x| x.parse().unwrap()).collect(),
        store: None,
        store_account: None,
        art: ipc::Art::default(),
        readiness: Readiness::Ready,
        resumable: false,
        activity,
    }
}
pub(crate) fn linked(id: &str) -> Target {
    Target { id: TargetId::new(id).unwrap(), name: id.into(), detail: None, link: ipc::Link::Linked, reach: ipc::Reach::Online }
}
pub(crate) fn engine_says(m: &mut Model, engine: &str, event: ipc::Event) -> Vec<Command> {
    m.step(Input::Engine { engine: EngineId::new(engine).unwrap(), message: FromEngine { re: None, event } })
}
pub(crate) fn id_of(m: &Model, title: &str) -> GameId {
    m.games().iter().find(|g| g.title == title).unwrap().id
}
pub(crate) fn refreshing_steam() -> (Model, TargetId) {
    use lumagame_core::activity::Activity;
    let mut m = Model::new(State::default(), Listing::default(), vec![manifest("steam", "Steam", EngineKind::Account)]);
    describe(&mut m, "steam", vec![lumagame_core::engine::AuthMethod::Qr], &[]);
    let account = TargetId::new("76561198000000000").unwrap();
    engine_says(&mut m, "steam", ipc::Event::Targets { targets: vec![linked("76561198000000000")] });
    let records = ["Beta", "Gamma", "Delta"].iter().enumerate().map(|(i, t)| entry(&format!("{i}"), t, ipc::Role::Game, &[&format!("steam:{i}")], Activity::default())).collect();
    engine_says(&mut m, "steam", ipc::Event::Library { target: account.clone(), entries: records, next: None });
    (m, account)
}
pub(crate) fn sent(commands: &[Command]) -> Vec<&Request> {
    commands.iter().filter_map(|c| if let Command::Send { request, .. } = c { Some(request) } else { None }).collect()
}
