//! The shot harness and the shared fixture model.
use super::*;
use lumatv_render::host::FrameCost;

pub(super) struct Shots {
    pub(super) engine: Engine,
    pub(super) page: GameUi,
    pub(super) events: Receiver<Event>,
    pub(super) now: u64,
    /// What one frame takes at the panel's refresh rate, in nanoseconds.
    pub(super) frame_ns: u64,
    pub(super) shm: tempfile::TempDir,
}
impl Shots {
    pub(super) fn new() -> io::Result<Self> {
        Self::at(60.0)
    }
    /// A harness whose panel refreshes `refresh_hz` times a second: the shots draw at 60, the budget walks at the 120 Hz the TV is driven at.
    pub(super) fn at(refresh_hz: f32) -> io::Result<Self> {
        crate::rig::software_gl();
        write_avatar()?;
        let shm = tempfile::tempdir()?;
        let gles = Gles::software_pbuffer(c"libEGL.so.1", 1920, 1080)?;
        let (mut engine, end) = Engine::new(
            gles,
            HostConfig {
                width: 1920,
                height: 1080,
                refresh_hz,
                shm_dir: shm.path().to_owned(),
                direct: false,
                gpu_sync: false,
                idle_trim: false,
                pages: 0,
                reference: false,
                blit: false,
                external: false,
                system_z: i32::MAX,
            },
        )?;
        engine.set_transparent(true);
        let (scene, _, events) = ui::Scene::over_with(end, SceneOptions { shm_dir: Some(shm.path().to_owned()), ..Default::default() })?;
        engine.finish_handshake()?;
        let mut ui = Ui::new(scene);
        ui.enable_glyph_text().map_err(io::Error::other)?;
        let mut land = |ui: &mut Ui| -> io::Result<()> {
            let end = Instant::now() + Duration::from_secs(20);
            loop {
                while let Ok(event) = events.try_recv() {
                    ui.scene.observe(&event);
                }
                if let Some(atlas) = ui.text_atlas() {
                    atlas.pump();
                }
                if ui.scene.has_pending() {
                    let _ = ui.try_commit()?;
                }
                engine.pump()?;
                engine.apply_waiting(now_ns());
                engine.tidy();
                if ui.glyph_text_settled() && !ui.scene.has_pending() && !engine.tidy_more() {
                    return Ok(());
                }
                if Instant::now() > end {
                    return Err(io::Error::other("glyph atlas did not settle"));
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        let page = GameUi::build(ui, &crate::rig::assets(), &mut land)?;
        Ok(Self { engine, page, events, now: now_ns(), frame_ns: (1e9 / f64::from(refresh_hz)).round() as u64, shm })
    }
    /// Draws the next frame, a refresh after the last, and returns what it cost.
    pub(super) fn frame(&mut self) -> io::Result<FrameCost> {
        self.now += self.frame_ns;
        while let Ok(event) = self.events.try_recv() {
            self.page.ui.scene.observe(&event);
        }
        if let Some(atlas) = self.page.ui.text_atlas() {
            atlas.pump();
        }
        if self.page.ui.scene.has_pending() {
            let _ = self.page.ui.try_commit()?;
        }
        if self.engine.pump()?.closed {
            return Err(io::Error::other("renderer closed"));
        }
        let cost = self.engine.draw(self.now);
        self.engine.frame_done(self.now, &cost);
        self.engine.tidy();
        Ok(cost)
    }
    pub(super) fn shot(&mut self, model: &Model) -> io::Result<Vec<u8>> {
        self.page.invalidate();
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            self.page.show(model)?;
            self.frame()?;
            if !self.page.art_busy() {
                break;
            }
            if Instant::now() > end {
                return Err(io::Error::other("artwork or glyph atlas did not settle"));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        for _ in 0..20 {
            self.frame()?;
            self.page.show(model)?;
        }
        let _ = &self.shm;
        Ok(self.engine.read_page(0))
    }
}
pub(super) fn event(model: &mut Model, engine: &str, event: ipc::Event) {
    model.step(Input::Engine { engine: EngineId::new(engine).unwrap(), message: Box::new(FromEngine { re: None, event }) });
}
pub(super) fn activate(model: &mut Model) {
    model.step(Input::Key { key: Key::Ok, down: true });
    model.step(Input::Key { key: Key::Ok, down: false });
}
pub(super) fn id(model: &Model, title: &str) -> GameId {
    model.games().iter().find(|g| g.title == title).unwrap().id
}
pub(super) fn assets() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/lumagame/mock/assets")
}
/// The mock's clock: every time in the fixtures counts back from it.
const NOW: u64 = 1_800_000_000;
pub(super) fn fixture() -> Model {
    let info = [
        ("gfn", "GeForce NOW", "cloud", "nitin · Ultimate", "device-code"),
        ("gamestream", "Gaming PC", "host", "BEAST-PC", "show-pin"),
        ("punktfunk", "Desk PC", "host", "Desk-PC", "enter-pin"),
        ("native", "This TV", "native", "Installed", "approval"),
    ];
    let manifests = info
        .iter()
        .map(|(id, name, kind, _, _)| {
            serde_json::from_value::<Manifest>(serde_json::json!({"id":id,"name":name,"summary":"Screenshot fixture","kind":kind,"protocol":1,"license":"MIT"}))
                .unwrap()
        })
        .collect::<Vec<_>>();
    let titles = [
        (1145350, "Hades II", "Roguelike", 2025, "gfn", 31),
        (1086940, "Baldur's Gate 3", "RPG", 2023, "gfn", 142),
        (1091500, "Cyberpunk 2077", "RPG", 2020, "gfn", 64),
        (1903340, "Clair Obscur: Expedition 33", "RPG", 2025, "gamestream", 48),
        (413150, "Stardew Valley", "Farming", 2016, "gamestream", 212),
        (1551360, "Forza Horizon 5", "Racing", 2021, "gfn", 37),
        (1245620, "Elden Ring", "Action RPG", 2022, "gamestream", 188),
        (292030, "The Witcher 3: Wild Hunt", "RPG", 2015, "gfn", 171),
        (1174180, "Red Dead Redemption 2", "Open world", 2019, "gfn", 96),
        (367520, "Hollow Knight", "Metroidvania", 2017, "gamestream", 77),
        (620, "Portal 2", "Puzzle", 2011, "punktfunk", 22),
        (1426210, "It Takes Two", "Co-op", 2021, "gamestream", 19),
        (1057090, "Ori and the Will of the Wisps", "Platformer", 2020, "gamestream", 14),
        (504230, "Celeste", "Platformer", 2018, "punktfunk", 26),
        (632470, "Disco Elysium", "RPG", 2019, "gfn", 41),
        (2358720, "Black Myth: Wukong", "Action", 2024, "gfn", 29),
        (2379780, "Balatro", "Card game", 2024, "punktfunk", 58),
        (553420, "Tunic", "Adventure", 2022, "gamestream", 12),
        (1868140, "Dave the Diver", "Adventure", 2023, "punktfunk", 33),
        (2246340, "Monster Hunter Wilds", "Action", 2025, "gfn", 52),
        (252950, "Rocket League", "Sports", 2015, "gfn", 90),
        (870780, "Control", "Action", 2019, "gfn", 21),
        (2310, "Quake", "Shooter", 1996, "native", 9),
        (2280, "DOOM + DOOM II", "Shooter", 1993, "native", 15),
        (1536610, "OpenTTD", "Strategy", 2004, "native", 40),
        (0, "Celeste Classic", "Platformer", 2015, "native", 0),
        (0, "SuperTuxKart", "Racing", 2007, "native", 0),
        (0, "Freedoom", "Shooter", 2003, "native", 0),
    ];
    let mut listing = Listing::default();
    let mut state = State::default();
    state.profile_mut().name = "Nitin".into();
    state.profile_mut().user.preferred_engine = Some(EngineId::new("gfn").unwrap());
    for (engine, _, _, _, _) in info {
        let engine = EngineId::new(engine).unwrap();
        state.engines.entry(engine.clone()).or_default().enabled = true;
        let entries=titles.iter().filter(|t|t.4==engine.as_str()||t.0==2310||t.0==1145350).map(|(steam,title,_,_,_,_)|serde_json::from_value(serde_json::json!({"id":if *steam>0{steam.to_string()}else{title.to_string()},"title":title,"external":if *steam>0{vec![format!("steam:{steam}")]}else{vec![]},"store":if engine.as_str()=="native"{"native"}else{"steam"},"store_account":"nitin","readiness":"ready"})).unwrap()).collect();
        listing.replace(&engine, &TargetId::new("main").unwrap(), entries);
    }
    let enabled = state.enabled_engines();
    let (games, merges) = catalog::build(&mut state.identity, listing.enabled(&enabled));
    state.apply_merges(&merges);
    let now = NOW;
    for (index, (_, title, _, _, _, hours)) in titles.iter().enumerate() {
        let g = games.iter().find(|g| g.title == *title).unwrap();
        if *hours > 0 {
            let mut start = now - 7200 - index as u64 * 7200 - u64::from(*hours) * 3600;
            // Hades II ends on a 72 minute session, which the game page lists apart from the hours before it.
            let sessions = if *title == "Hades II" { vec![hours * 3600 - 4320, 4320] } else { vec![hours * 3600] };
            for seconds in sessions {
                state.profile_mut().history.record(Played { game: g.id, source: g.sources[0].key(), start, seconds });
                start += u64::from(seconds);
            }
        }
    }
    let mut model = Model::new(state, listing, manifests);
    model.step(Input::Tick { now_ms: 0, unix_s: now });
    for (engine, _, kind, name, auth) in info {
        let descriptor:Descriptor=serde_json::from_value(serde_json::json!({"id":engine,"version":"fixture","capabilities":{"kind":kind,"targets":if kind=="cloud"{"account"}else if kind=="native"{"device"}else{"host"},"auth":[auth],"presentation":if kind=="native"{"app"}else{"feeder"},"features":["stats","overlay-keys","end-game","manual-host","wake"]}})).unwrap();
        event(&mut model, engine, ipc::Event::Ready { descriptor, schema: Default::default() });
        event(
            &mut model,
            engine,
            ipc::Event::Targets {
                targets: vec![serde_json::from_value(serde_json::json!({"id":"main","name":name,"link":"linked","reach":"online"})).unwrap()],
            },
        );
    }
    let keys = model.games().iter().flat_map(|g| g.sources.iter().map(|s| s.key())).collect::<Vec<_>>();
    for key in keys {
        event(&mut model, key.engine.as_str(), ipc::Event::Readiness { target: key.target, game: key.id, readiness: Readiness::Ready });
    }
    for (steam, title, genre, year, _, _) in titles {
        let game = id(&model, title);
        let mut details =
            GameDetails { year: Some(year), genres: vec![genre.into()], players: Some("1 player".into()), cloud_saves: Some(steam > 0), ..Default::default() };
        if title == "Hades II" {
            details.description = "Battle beyond the Underworld as Melinoë, Princess of the Underworld, against the Titan of Time.".into();
            details.capabilities = vec!["4K HDR".into(), "Full controller support".into(), "Cloud saves".into()];
            let mut hub = lumagame_meta::Hub {
                release: Some("30 Sep, 2025".into()),
                review: Some(lumagame_meta::hub::Review { label: "Overwhelmingly Positive".into(), percent: Some(97), count: 48_210 }),
                metacritic: Some(93),
                deck: Some(lumagame_meta::hub::Deck::Verified),
                developers: vec!["Supergiant Games".into()],
                publishers: vec!["Supergiant Games".into()],
                categories: ["Single-player", "Steam Achievements", "Full controller support", "Steam Cloud", "In-App Purchases", "Remote Play on TV"].map(Into::into).to_vec(),
                dlc: 0,
                credits: vec![lumagame_meta::hub::STEAM_CREDIT.into(), lumagame_meta::hub::PCGW_CREDIT.into()],
                facts: vec![
                    lumagame_meta::hub::Fact { label: "Ultrawide".into(), value: "Yes".into() },
                    lumagame_meta::hub::Fact { label: "HDR".into(), value: "No".into() },
                    lumagame_meta::hub::Fact { label: "Adaptive triggers".into(), value: "Yes".into() },
                ],
                news: vec![
                    lumagame_meta::hub::News { title: "Patch 1.2: new boons and fixes".into(), author: "Supergiant".into(), date: 1_760_000_000, url: "https://store.steampowered.com/news/1".into(), summary: "Adds a set of boons, rebalances several weapons and fixes the co-op menu focus. Saves carry over.".into() },
                    lumagame_meta::hub::News { title: "Anniversary sale".into(), author: String::new(), date: 1_758_000_000, url: "https://store.steampowered.com/news/2".into(), summary: String::new() },
                ],
                ..Default::default()
            };
            hub.achievements.total = 40;
            hub.achievements.rarest = Some(12);
            for (i, (name, permille)) in [("First Descent", Some(870)), ("Titan Slayer", Some(310)), ("The Last Word", None)].into_iter().enumerate() {
                let url = format!("fixture://achievement/{i}");
                model.art.insert(url.clone(), assets().join("1145350/media/shot-1.jpg"));
                hub.achievements.shown.push(lumagame_meta::hub::Achievement { api: None, name: name.into(), icon: url, permille });
            }
            details.hub = hub;
            for name in ["trailer-1.jpg", "trailer-2.jpg", "shot-1.jpg", "shot-2.jpg", "shot-3.jpg", "shot-4.jpg", "shot-5.jpg"] {
                model.art.insert(format!("fixture://media/{name}"), assets().join("1145350/media").join(name));
            }
        }
        model.details.insert(game, details);
        if steam > 0 {
            for (kind, name) in
                [(catalog::ArtKind::Cover, "library_600x900_2x.jpg"), (catalog::ArtKind::Hero, "library_hero.jpg"), (catalog::ArtKind::Logo, "logo.png")]
            {
                if let Some(url) = model.art_url(game, kind) {
                    model.art.insert(url, assets().join(steam.to_string()).join(name));
                }
            }
        }
    }
    // The mock's row: the last played first, then the rest of the recent and most played, and a native game that Steam does not know.
    let row = [
        "Hades II",
        "Baldur's Gate 3",
        "Cyberpunk 2077",
        "Clair Obscur: Expedition 33",
        "Balatro",
        "Forza Horizon 5",
        "Elden Ring",
        "It Takes Two",
        "Stardew Valley",
        "The Witcher 3: Wild Hunt",
        "Red Dead Redemption 2",
        "Quake",
    ];
    model.home_games = row.iter().map(|s| id(&model, s)).collect();
    model.view.featured = Some(id(&model, "Hades II"));
    model.view.selected = model.view.featured;
    model.view.focus = model.home_focus();
    model.rebuild_navigation();
    model
}
/// The achievements the game page names for Hades II: Steam's id, the title, how many in a thousand players have it, and how long
/// before [`NOW`] the owner got it. The first four are the latest unlocked, the last three the ones to go for next.
const HADES_ACHIEVEMENTS: [(&str, &str, u16, Option<u64>); 7] = [
    ("ACH_DEPTHS", "Witch of the Depths", 610, Some(9_000)),
    ("ACH_MOTE", "So Mote It Be", 340, Some(10_800)),
    ("ACH_SOAK", "Soothing Soak", 720, Some(4 * 86_400)),
    ("ACH_WOODS", "Witch of the Woods", 880, Some(64 * 86_400)),
    ("ACH_PLAINS", "Witch of the Plains", 470, None),
    ("ACH_NIGHT", "The Arms of Night", 290, None),
    ("ACH_CHRONOS", "Death to Chronos", 90, None),
];
/// What Steam's community says about Hades II in the game page's mock, with the news and the discussions dated back from `now`.
pub(super) fn hades_community(now: u64) -> lumagame_core::community::Community {
    use lumagame_core::community::{Absence, Community, Guide, NewsItem, Review, Reviews, Section, Shot, Topic, Trailer};
    let day = 86_400;
    let review = |text: &str, recommended, hours: u64, helpful| Review { text: text.into(), recommended, helpful, funny: 0, playtime_s: hours * 3600, at: now - 20 * day };
    let news = |title: &str, text: &str, days: u64| NewsItem {
        title: title.into(),
        text: text.into(),
        author: Some("Supergiant Games".into()),
        source: None,
        at: now - days * day,
    };
    let topic = |id: &str, title: &str, author: &str, replies, ago: u64| Topic {
        id: id.into(),
        title: title.into(),
        author: author.into(),
        replies,
        last_post: Some(now - ago),
        pinned: false,
        locked: false,
        preview: String::new(),
    };
    let guide = |id: &str, title: &str, summary: &str, author: &str| Guide {
        id: id.into(),
        title: title.into(),
        summary: summary.into(),
        author: author.into(),
        image: None,
    };
    let shot = |n: u32| Shot { thumb: format!("fixture://media/shot-{n}.jpg"), full: format!("fixture://media/shot-{n}.jpg") };
    let trailer = |n: u32, title: &str| Trailer { title: title.into(), thumb: format!("fixture://media/trailer-{n}.jpg"), hls: Some(format!("https://fixture.invalid/trailer-{n}.m3u8")), dash: None };
    let reviews = Reviews {
        label: Some("Overwhelmingly Positive".into()),
        percent: Some(96),
        positive: 77_956,
        negative: 3_248,
        top: vec![
            review("Every run teaches something new, and the boons keep each one different.", true, 54, 1_204),
            review("Beautiful and tight. The co-op menu is fiddly, but nothing else gets in the way.", true, 22, 618),
            review("The late game leans on luck more than skill, which wore thin after a while.", false, 71, 240),
        ],
    };
    Community {
        reviews: Section::Present { value: reviews },
        news: Section::list(vec![
            news("Patch 1.2: new boons and fixes", "Adds a set of boons, rebalances several weapons and fixes the co-op menu focus. Saves carry over.", 112),
            news("Summer sale", "Hades II is on sale for the next week.", 171),
            // Long enough for three pages, so the reader walk pages through it.
            news(
                "Development update: the road to 1.0",
                &"A look at what is next for the Underworld, from new bosses to controller improvements. The team shares the plan for the \
                  coming patches, the changes to the late game that players asked for, and how saves carry over between them. "
                    .repeat(9),
                213,
            ),
        ]),
        players: Section::Present { value: 8_412 },
        rarity: Section::Present { value: HADES_ACHIEVEMENTS.iter().map(|(api, _, permille, _)| ((*api).to_owned(), *permille)).collect() },
        store_screenshots: Section::list((1..=5).map(shot).collect()),
        trailers: Section::list(vec![trailer(1, "Gameplay trailer"), trailer(2, "Early access trailer")]),
        hub_screenshots: Section::absent(Absence::Empty),
        hub_artwork: Section::absent(Absence::Empty),
        hub_videos: Section::absent(Absence::Empty),
        guides: Section::list(vec![
            guide("2900000001", "Beginner's guide to the Underworld", "Weapons, aspects and the first boss, in the order to unlock them.", "PylosFan"),
            guide("2900000002", "Every keepsake and where it comes from", "What each keepsake does and how to get it.", "Melinoe_Main"),
        ]),
        discussions: Section::list(vec![
            topic("1", "Which weapon should I unlock first?", "PylosFan", 48, 10_800),
            topic("2", "Is the co-op menu focus fixed in 1.2?", "Melinoe_Main", 112, 86_400),
            topic("3", "Fastest route to the Oceanus boss", "TheDeepEnd", 19, 5 * 86_400),
        ]),
    }
}
/// The fixture with Steam signed in and Hades II carrying the game page's mock: Steam's playing time and progress, and its community
/// page. [`fixture`] on its own stays as it is, so the shots that do not show these keep their pixels.
pub(super) fn with_hades_community(mut model: Model) -> Model {
    use crate::game::model::tests::{describe, entry, linked, manifest};
    use lumagame_core::{
        activity::Activity,
        engine::{AuthMethod, EngineKind, Feature},
        ids::ProviderGameId,
        ipc::{Role, Unlocked},
    };
    const ACCOUNT: &str = "76561198000000000";
    const APP: &str = "1145350";
    let steam = EngineId::new("steam").unwrap();
    if !model.integrations.iter().any(|e| e.manifest.id == steam) {
        let at = model.integrations.iter().rposition(|e| e.manifest.kind == EngineKind::Host).map_or(0, |i| i + 1);
        let manifest = manifest("steam", "Steam", EngineKind::Account);
        model.integrations.insert(at, Integration { manifest, descriptor: None, schema: SettingsSchema::default(), targets: Vec::new(), error: None });
    }
    describe(&mut model, "steam", vec![AuthMethod::Qr], &[Feature::Achievements, Feature::Community]);
    let target = TargetId::new(ACCOUNT).unwrap();
    event(&mut model, "steam", ipc::Event::Targets { targets: vec![ipc::Target { name: "Nitin".into(), avatar: Some(AVATAR_URL.into()), ..linked(ACCOUNT) }] });
    model.art.insert(AVATAR_URL.into(), avatar_file());
    // Steam lists the game as a record only: its recent hours, beside this TV's own history of it.
    let record = entry(APP, "Hades II", Role::Record, &["steam:1145350"], Activity { recent_s: Some(4 * 3600), ..Activity::default() });
    event(&mut model, "steam", ipc::Event::Library { target: target.clone(), entries: vec![record], next: None });
    let game = ProviderGameId::new(APP).unwrap();
    let unlocked = HADES_ACHIEVEMENTS
        .iter()
        .filter_map(|(api, _, _, ago)| ago.map(|ago| Unlocked { name: (*api).into(), at: Some(NOW - ago) }))
        .chain((0..14).map(|n| Unlocked { name: format!("ACH_MORE_{n}"), at: None }))
        .collect();
    event(&mut model, "steam", ipc::Event::Achievements { target, game: game.clone(), unlocked });
    let hades = id(&model, "Hades II");
    let details = model.details.get_mut(&hades).unwrap();
    details.developer = Some("Supergiant Games".into());
    details.hub.achievements.total = 50;
    details.hub.achievements.shown = HADES_ACHIEVEMENTS
        .iter()
        .enumerate()
        .map(|(i, (api, name, _, _))| {
            let icon = format!("fixture://achievement/{i}");
            model.art.insert(icon.clone(), assets().join("1145350/media/shot-1.jpg"));
            lumagame_meta::hub::Achievement { api: Some((*api).into()), name: (*name).into(), icon, permille: None }
        })
        .collect();
    event(&mut model, "steam", ipc::Event::Community { game, community: hades_community(NOW) });
    model
}
#[test]
fn hades_carries_the_game_page_mock() {
    use crate::game::model::facts::{Fact, Start};
    fn count<T>(fact: Fact<&[T]>) -> Option<usize> {
        if let Fact::Present(items) = fact { Some(items.len()) } else { None }
    }
    let model = with_hades_community(fixture());
    let facts = model.facts(id(&model, "Hades II"));
    let Fact::Present(play) = &facts.activity.play_time else { panic!("no play time") };
    assert_eq!((play.seconds, play.recent), (31 * 3600, Some(4 * 3600)));
    let Fact::Present(last) = &facts.activity.last_session else { panic!("no last session") };
    assert_eq!(last.seconds, Some(72 * 60));
    let Fact::Present(achievements) = &facts.activity.achievements else { panic!("no achievements") };
    assert_eq!((achievements.unlocked, achievements.total, achievements.latest.len(), achievements.next.len()), (18, Some(50), 4, 3));
    let Fact::Present(reviews) = facts.community.reviews else { panic!("no reviews") };
    assert_eq!((reviews.percent, reviews.positive + reviews.negative), (Some(96), 81_204));
    assert_eq!(facts.activity.players, Fact::Present(8_412));
    let counts = [count(facts.activity.news), count(facts.community.discussions), count(facts.community.guides), count(facts.media.store_screenshots), count(facts.media.trailers)];
    assert_eq!(counts, [Some(3), Some(3), Some(2), Some(5), Some(2)]);
    assert_eq!(facts.start, Start::Ready);
}
/// A running stream whose overlay has been up for half a minute with the readings of `kind`: a healthy one, or one where the
/// network, the host or the TV is at fault.
pub(super) fn hud_model(launched: &Model, mode: crate::game::stats::Mode, kind: &str) -> Model {
    let mut m = launched.clone();
    let session = m.session().unwrap().id;
    event(&mut m, "gfn", ipc::Event::Session { session, state: SessionState::Running { stream: Some(ipc::StreamInfo {
        codec: lumagame_core::engine::Codec::H265, width: 3840, height: 2160, fps: 120, hdr: true, audio_channels: 6, sdr_reason: None, vui: None }) } });
    m.step(Input::Tick { now_ms: 1000, unix_s: 1_800_000_001 });
    m.step(Input::StatsSetting(crate::game::stats::Overlay { mode, changed: 1 }));
    let pad = |gap: u32, bursts: u32| ipc::PadStats { name: "DualShock 4 · Bluetooth".into(), reports: 250, rate_hz: 250.0, p99_us: gap, bursts, burst_gap_us: 0, age_ms: 0 };
    let mut stats = ipc::Stats { rtt_ms: Some(11.3), jitter_ms: Some(0.8), loss_pct: Some(0.0), bitrate_kbps: Some(74_200), fps: Some(119.9), host_ms: Some(5.2),
        client_ms: Some(1.1), pacing_ms: Some(2.4), host_gap_ms: Some([8.3, 9.1]), arrival_gap_ms: Some([8.3, 9.4]), pad: Some(pad(4900, 0)), ..Default::default() };
    let mut display = serde_json::json!({"window":{"width":3840,"height":2160,"hdrType":2,"lowDelay":true},
        "refresh":{"hz":120.0,"fewest":12},"firmware":{"sysVeri10":{"videoDelayUs":15816}}});
    // The gap of each frame in µs for the sample `i`, and the drops that grow over the last eight seconds.
    let mut gap: fn(usize) -> i64 = |i| if i % 400 == 41 { 16_800 } else { 8000 + (i % 10) as i64 * 70 };
    let mut drops = [0u64; 5];
    match kind {
        "network" => {
            stats.rtt_ms = Some(58.0); stats.jitter_ms = Some(11.5); stats.loss_pct = Some(1.4); stats.bitrate_kbps = Some(31_000);
            stats.fps = Some(112.0); stats.arrival_gap_ms = Some([5.2, 19.0]); stats.dropped = Some(8);
            gap = |i| if i % 7 == 3 { 17_500 + (i % 5) as i64 * 400 } else { 4_200 + (i % 11) as i64 * 300 };
            drops = [0, 0, 0, 8, 0];
        }
        "host" => {
            stats.host_ms = Some(7.4); stats.fps = Some(101.0); stats.host_gap_ms = Some([8.4, 17.8]); stats.arrival_gap_ms = Some([8.4, 17.9]);
            gap = |i| if i % 5 == 2 { 17_000 + (i % 3) as i64 * 500 } else { 8_400 };
        }
        "tv" => {
            stats.client_ms = Some(6.5); stats.pacing_ms = Some(12.0); stats.socket_drops = Some(12); stats.dropped = Some(20);
            display["refresh"]["fewest"] = serde_json::json!(9);
            gap = |i| if i % 9 == 4 { 21_000 } else { 8_300 + (i % 10) as i64 * 60 };
            drops = [14, 6, 0, 0, 0];
        }
        _ => {}
    }
    let mut fed = 30_000;
    for step in 0..440u64 {
        let now = 1000 + step * 250;
        // Readings wander a little so that the graphs have a shape.
        let mut moved = stats.clone();
        let by = stats.jitter_ms.unwrap_or(0.0) * 0.4;
        moved.jitter_ms = stats.jitter_ms.map(|v| (v + by * ((step % 7) as f32 - 3.0) / 3.0).max(0.0));
        moved.bitrate_kbps = stats.bitrate_kbps.map(|k| (k as f32 * (1.0 + 0.05 * ((step % 5) as f32 - 2.0) / 2.0)) as u32);
        if kind == "network" && step % 3 == 0 { moved.loss_pct = Some(0.0); }
        event(&mut m, "gfn", ipc::Event::Stats { session, stats: moved });
        let grown = step.saturating_sub(400) * 1000 / 39;
        let dropped = drops.map(|d| d * grown.min(1000) / 1000);
        fed += 30;
        m.step(Input::Frames(crate::lifecycle::Frames { fed, accepted: fed - dropped.iter().sum::<u64>(), dropped, omitted: 0,
            samples: (0..30).map(|i| [gap(step as usize * 30 + i), 400 + (i % 10) as i64 * 20, 1, 0]).collect() }));
        m.step(Input::Display(display.clone()));
        m.step(Input::Tick { now_ms: now, unix_s: 1_800_000_001 + now / 1000 });
    }
    m
}

pub(super) fn png(path: &Path, w: u32, h: u32, bytes: &[u8]) -> io::Result<()> {
    let mut encoder = png::Encoder::new(io::BufWriter::new(std::fs::File::create(path)?), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().and_then(|mut w| w.write_image_data(bytes)).map_err(io::Error::other)
}

const AVATAR_URL: &str = "fixture://avatar";
fn avatar_file() -> PathBuf {
    std::env::var_os("CODY_SCRATCH").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("lumagame-fixture-avatar.png")
}
/// The Steam profile picture of the shots: an abstract gradient with a ring, nobody's photo, 64 px like Steam's medium picture.
fn write_avatar() -> io::Result<()> {
    let mut rgba = Vec::with_capacity(64 * 64 * 4);
    for y in 0..64 {
        for x in 0..64 {
            let t = (x + y) as f32 / 126.0;
            let ring = ((x as f32 - 31.5).hypot(y as f32 - 31.5) - 17.0).abs() < 3.0;
            let (r, g, b) = if ring { (205.0, 215.0, 225.0) } else { (40.0 + 120.0 * t, 150.0 - 80.0 * t, 175.0 + 40.0 * t) };
            rgba.extend([r as u8, g as u8, b as u8, 255]);
        }
    }
    png(&avatar_file(), 64, 64, &rgba)
}
/// A cloud game running in `fixture()`, past its first tick.
pub(super) fn playing() -> (Model, lumagame_core::ids::SessionId) {
    let mut m=fixture();
    m.view.page=Page::Game;m.view.selected=Some(id(&m,"Cyberpunk 2077"));m.view.focus=Focus::Play;
    activate(&mut m);
    let session=m.session().unwrap().id;
    event(&mut m,"gfn",ipc::Event::Session {session,state:SessionState::Running {stream:None}});
    m.step(Input::Tick {now_ms:1000,unix_s:0});
    (m,session)
}
pub(super) fn activity(commands:&[Command]) -> usize {
    commands.iter().filter(|c| matches!(c,Command::Send {request:ipc::Request::Activity {..},..})).count()
}

pub(super) fn bitrate_schema() -> SettingsSchema {
    serde_json::from_value(serde_json::json!({"sections":[{"title":"Stream","fields":[
        {"key":"bitrate-mbps","label":"Bitrate","type":"number","min":5,"max":150,"step":5,"default":80}
    ]}]})).unwrap()
}
pub(super) fn sent_bitrates(commands: &[Command]) -> Vec<Value> {
    commands.iter().filter_map(|c| match c {
        Command::Send { request: ipc::Request::Settings { values }, .. } => values.get("bitrate-mbps").cloned(),
        _ => None,
    }).collect()
}
pub(super) fn wired(mbit: u32) -> Option<Uplink> {
    Some(Uplink { interface: "eth0".into(), medium: Medium::Wired, mbit: Some(mbit) })
}
