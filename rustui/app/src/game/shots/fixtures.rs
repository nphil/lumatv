//! The shot harness and the shared fixture model.
use super::*;

pub(super) struct Shots {
    pub(super) engine: Engine,
    pub(super) page: GameUi,
    pub(super) events: Receiver<Event>,
    pub(super) now: u64,
    pub(super) shm: tempfile::TempDir,
}
impl Shots {
    pub(super) fn new() -> io::Result<Self> {
        crate::rig::software_gl();
        let shm = tempfile::tempdir()?;
        let gles = Gles::software_pbuffer(c"libEGL.so.1", 1920, 1080)?;
        let (mut engine, end) = Engine::new(
            gles,
            HostConfig {
                width: 1920,
                height: 1080,
                refresh_hz: 60.0,
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
        Ok(Self { engine, page, events, now: now_ns(), shm })
    }
    pub(super) fn frame(&mut self) -> io::Result<()> {
        self.now += 16_666_667;
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
        Ok(())
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
    model.step(Input::Engine { engine: EngineId::new(engine).unwrap(), message: FromEngine { re: None, event } });
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
    let now = 1_800_000_000;
    for (index, (_, title, _, _, _, hours)) in titles.iter().enumerate() {
        let g = games.iter().find(|g| g.title == *title).unwrap();
        if *hours > 0 {
            state.profile_mut().history.record(Played {
                game: g.id,
                source: g.sources[0].key(),
                start: now - 7200 - index as u64 * 7200 - u64::from(*hours) * 3600,
                seconds: hours * 3600,
            });
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
            for (i, name) in ["trailer-1.jpg", "trailer-2.jpg", "shot-1.jpg", "shot-2.jpg", "shot-3.jpg", "shot-4.jpg", "shot-5.jpg"].into_iter().enumerate() {
                let path = assets().join("1145350/media").join(name);
                let url = format!("fixture://media/{name}");
                model.art.insert(url.clone(), path);
                details.media.push(Media {
                    title: if i < 2 { ["Gameplay trailer", "Early access trailer"][i].into() } else { format!("Screenshot {}", i - 1) },
                    image: url,
                    video: (i < 2).then(|| "https://store.steampowered.com/app/1145350".into()),
                });
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
    let continue_titles =
        ["Baldur's Gate 3", "Cyberpunk 2077", "Clair Obscur: Expedition 33", "Stardew Valley", "Forza Horizon 5", "Elden Ring", "It Takes Two"];
    let most = ["Stardew Valley", "Elden Ring", "The Witcher 3: Wild Hunt", "Baldur's Gate 3", "Red Dead Redemption 2", "Rocket League", "Hollow Knight"];
    model.home_rows = vec![
        HomeRow { title: "Continue".into(), items: continue_titles.iter().map(|s| HomeItem::Game(id(&model, s))).collect() },
        HomeRow { title: "Most played".into(), items: most.iter().map(|s| HomeItem::Game(id(&model, s))).collect() },
        HomeRow {
            title: "Genres".into(),
            items: ["RPG", "Action", "Platformer", "Shooter", "Racing"].into_iter().map(|s| HomeItem::Genre(s.into())).collect(),
        },
        HomeRow {
            title: "On this TV".into(),
            items: ["Quake", "DOOM + DOOM II", "Celeste Classic", "OpenTTD", "SuperTuxKart", "Freedoom"]
                .iter()
                .map(|s| HomeItem::Game(id(&model, s)))
                .collect(),
        },
    ];
    model.view.featured = Some(id(&model, "Hades II"));
    model.view.selected = model.view.featured;
    model.view.focus = Focus::Home(0, 1);
    model.rebuild_navigation();
    model
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
/// A cloud game running in `fixture()`, past its first tick.
pub(super) fn playing() -> (Model, lumagame_core::ids::SessionId) {
    let mut m=fixture();
    m.view.page=Page::Hub;m.view.selected=Some(id(&m,"Cyberpunk 2077"));m.view.focus=Focus::HubAction(0);
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
