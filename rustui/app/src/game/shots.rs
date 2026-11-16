//! Explicit `GAME_SHOTS=all` visual acceptance run. Illustrative catalog/history mirrors the HTML mocks, never production data.
use super::{model::*, screens::GameUi};
use super::helper::{GameSirState, LeadjoyState, gamesir_fixture, leadjoy_fixture};
use compositor_protocol::Event;
use lumagame_core::{
    catalog::{self, Listing},
    engine::{Descriptor, Feature, Manifest},
    history::Played,
    ids::{EngineId, GameId, TargetId},
    ipc::{self, AuthState, FromEngine, Readiness, SessionState},
    state::State,
};
use lumatv_render::{
    Gles,
    host::{Engine, HostConfig},
    now_ns,
};
use std::{
    io,
    path::{Path, PathBuf},
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};
use ui::{SceneOptions, Ui};

struct Shots {
    engine: Engine,
    page: GameUi,
    events: Receiver<Event>,
    now: u64,
    shm: tempfile::TempDir,
}
impl Shots {
    fn new() -> io::Result<Self> {
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
    fn frame(&mut self) -> io::Result<()> {
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
    fn shot(&mut self, model: &Model) -> io::Result<Vec<u8>> {
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
        for _ in 0..14 {
            self.frame()?;
            self.page.show(model)?;
        }
        let _ = &self.shm;
        Ok(self.engine.read_page(0))
    }
}
fn event(model: &mut Model, engine: &str, event: ipc::Event) {
    model.step(Input::Engine { engine: EngineId::new(engine).unwrap(), message: FromEngine { re: None, event } });
}
fn activate(model: &mut Model) {
    model.step(Input::Key { key: Key::Ok, down: true });
    model.step(Input::Key { key: Key::Ok, down: false });
}
fn id(model: &Model, title: &str) -> GameId {
    model.games().iter().find(|g| g.title == title).unwrap().id
}
fn assets() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/lumagame/mock/assets")
}
fn fixture() -> Model {
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
            details.achievements = Some("18 of 40".into());
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
fn screens() -> Vec<(String, Model)> {
    let base = fixture();
    let mut out: Vec<(String, Model)> = Vec::new();
    let mut add = |name: &str, mut model: Model| {
        model.rebuild_navigation();
        out.push((name.into(), model));
    };
    add("01-home", base.clone());
    let mut m = base.clone();
    m.prefs.light = true;
    add("01-home-light", m);
    let mut m = base.clone();
    m.view.focus = Focus::Hero(0);
    add("01-home-play-focus", m);
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
    let mut m = base.clone();
    m.view.page = Page::Library;
    m.view.focus = Focus::Grid(2);
    add("07-library", m.clone());
    m.view.sheet = Some(Sheet::Sort);
    m.view.focus = Focus::SheetRow(0);
    add("07-library-sort", m);
    for (name, tab, light) in
        [("08-hub", 0, false), ("08-hub-light", 0, true), ("08-hub-ways", 1, false), ("08-hub-media", 2, false), ("08-hub-media-light", 2, true)]
    {
        let mut m = base.clone();
        m.view.page = Page::Hub;
        m.view.hub_tab = tab;
        m.prefs.light = light;
        m.view.focus = if tab == 1 {
            Focus::Way(0)
        } else if tab == 2 {
            Focus::Media(0)
        } else {
            Focus::HubAction(0)
        };
        add(name, m);
    }
    let mut m = base.clone();
    m.view.page = Page::Hub;
    m.view.selected = Some(id(&m, "Quake"));
    m.view.sheet = Some(Sheet::Chooser);
    m.view.chooser_selected = m.selected_game().unwrap().sources.first().map(|s| s.key());
    m.view.focus = Focus::SheetRow(1);
    add("09-chooser", m.clone());
    m.prefs.text_percent = 200;
    add("32-chooser-text200", m);
    for (name, section, light, scale) in [
        ("10-integrations", Section::Integrations, false, 100),
        ("13-settings-appearance", Section::Appearance, false, 100),
        ("13-settings-appearance-light", Section::Appearance, true, 100),
        ("14-settings-streaming", Section::Streaming, false, 100),
        ("25-settings-controllers", Section::Controllers, false, 100),
        ("26-settings-about", Section::About, false, 100),
        ("33-settings-text200", Section::Streaming, false, 200),
    ] {
        let mut m = base.clone();
        m.view.page = Page::Settings;
        m.view.section = section;
        m.prefs.light = light;
        m.prefs.text_percent = scale;
        m.prefs.fps = 120;
        m.prefs.hdr = true;
        m.view.focus = if section == Section::Appearance {
            Focus::Theme(3)
        } else if section == Section::Integrations {
            Focus::Integration(1)
        } else if section == Section::Controllers {
            Focus::Setting(0)
        } else {
            Focus::Setting(3)
        };
        add(name, m);
    }
    let mut m = base.clone();
    m.view.page = Page::Settings;
    m.view.sheet = Some(Sheet::Theme);
    m.view.focus = Focus::Theme(3);
    add("27-theme-picker", m);
    for (name, state, engine) in [
        ("11-pairing", AuthState::ShowPin { pin: "7249".into() }, "gamestream"),
        ("12-signin", AuthState::DeviceCode { url: "https://nvidia.com/link".into(), code: "ABCD-EFGH".into(), qr: None, expires_s: 420 }, "gfn"),
        ("28-enter-pin", AuthState::EnterPin { digits: 4 }, "punktfunk"),
        ("29-approval", AuthState::Approval { hint: Some("Allow LumaGame in the Punktfunk request on Desk-PC.".into()) }, "punktfunk"),
    ] {
        let mut m = base.clone();
        m.view.page = Page::Settings;
        m.view.section = Section::Integrations;
        event(&mut m, engine, ipc::Event::Auth { state });
        add(name, m);
    }
    let mut launched = base.clone();
    launched.view.selected = Some(id(&launched, "Cyberpunk 2077"));
    launched.view.page = Page::Hub;
    launched.view.chooser_selected = launched.selected_game().unwrap().sources.iter().find(|s| s.engine.as_str() == "gfn").map(|s| s.key());
    launched.view.focus = Focus::HubAction(0);
    activate(&mut launched);
    add("17-state-checking", launched.clone());
    let session = launched.session().unwrap().id;
    event(&mut launched, "gfn", ipc::Event::Session { session, state: SessionState::Queue { position: Some(4), eta_s: Some(120) } });
    add("18-state-queue", launched.clone());
    event(&mut launched, "gfn", ipc::Event::Session { session, state: SessionState::Starting });
    add("19-state-starting", launched.clone());
    event(
        &mut launched,
        "gfn",
        ipc::Event::Session {
            session,
            state: SessionState::Running {
                stream: Some(
                    serde_json::from_value(serde_json::json!({"codec":"h265","width":3840,"height":2160,"fps":120,"hdr":true,"audio_channels":2})).unwrap(),
                ),
            },
        },
    );
    add("40-running-transparent", launched.clone());
    let mut hud = launched.clone();
    hud.step(Input::Tick {now_ms:1000,unix_s:1_800_000_001});
    hud.step(Input::StatsSetting(super::stats::Overlay {mode:super::stats::Mode::Full,changed:1}));
    event(&mut hud,"gfn",ipc::Event::Stats {session,stats:ipc::Stats {
        rtt_ms:Some(11.3),jitter_ms:Some(0.8),loss_pct:Some(0.0),bitrate_kbps:Some(74200),fps:Some(119.9),
        pad:Some(ipc::PadStats {name:"DualShock 4 · Bluetooth".into(),reports:250,rate_hz:250.0,p99_us:4900,bursts:0,burst_gap_us:0,age_ms:0}),
        ..Default::default()
    }});
    hud.step(Input::Frames(crate::lifecycle::Frames {fed:31204,accepted:31204,dropped:[0;5],omitted:0,
        samples:(0..240).map(|i| [if i==41 {16800} else {8000+i%10*70},400+i%10*20,1,0]).collect()}));
    hud.step(Input::Display(serde_json::json!({"window":{"width":3840,"height":2160,"hdrType":2,"lowDelay":true},
        "refresh":{"hz":120.0,"fewest":12},"firmware":{"sysVeri10":{"videoDelayUs":15816}}})));
    hud.step(Input::Tick {now_ms:1250,unix_s:1_800_000_001});
    for (name,mode) in [("48-stats-compact-bright",super::stats::Mode::Compact),("49-stats-full-bright",super::stats::Mode::Full)] {
        hud.prefs.stats.mode=mode;
        add(name,hud.clone());
    }
    let mut rough=hud.clone();
    rough.telemetry.frames.dropped=[29,0,8,0,0];
    rough.telemetry.frames.fed=18433;
    rough.telemetry.frames.omitted=4;
    rough.telemetry.stream.loss_pct=Some(1.4);
    rough.telemetry.stream.host_ms=Some(5.2);
    rough.telemetry.stream.pad=Some(ipc::PadStats {name:"Xbox Wireless · Bluetooth".into(),reports:250,rate_hz:125.0,p99_us:22400,bursts:6,burst_gap_us:22000,age_ms:0});
    add("50-stats-full-dark",rough.clone());
    rough.prefs.light=true;
    add("50-stats-full-dark-light",rough);
    let mut stale=hud.clone();
    stale.step(Input::Tick {now_ms:4000,unix_s:1_800_000_004});
    add("51-stats-stale-dark",stale);
    launched.step(Input::Tick { now_ms: 1_932_000, unix_s: 1_800_001_932 });
    launched.step(Input::Overlay);
    launched.view.stats = ipc::Stats { rtt_ms: Some(8.0), loss_pct: Some(0.0), bitrate_kbps: Some(78_000), ..Default::default() };
    add("15-overlay", launched.clone());
    let mut lab_stream = launched.clone();
    lab_stream.integrations.iter_mut().find(|e| e.manifest.id.as_str() == "gfn").unwrap()
        .descriptor.as_mut().unwrap().capabilities.features.insert(Feature::RoundTrip);
    lab_stream.view.page = Page::Settings;
    lab_stream.view.section = Section::Controllers;
    lab_stream.view.focus = Focus::Setting(0);
    activate(&mut lab_stream);
    activate(&mut lab_stream);
    event(&mut lab_stream, "gfn", ipc::Event::RoundTrip { session, trip: ipc::RoundTrip {
        presses: Some(14), press_p50_ms: Some(0.42), press_p99_ms: Some(1.9), network_ms: Some(21.5), host_ms: Some(6.0), ..Default::default()
    } });
    lab_stream.prefs.reduce_motion = true;
    add("45-controller-lab-stream", lab_stream.clone());
    lab_stream.view.focus = Focus::Lab(12);
    add("45-controller-lab-stream-bottom", lab_stream);
    launched.view.focus = Focus::Overlay(3);
    activate(&mut launched);
    add("16-overlay-end-confirm", launched);
    let mut m = base.clone();
    m.view.page = Page::Hub;
    m.view.notice = Some("The connection was interrupted. Check the network, then choose Play to try again.".into());
    add("20-state-failed", m);
    let mut m = base.clone();
    m.view.page = Page::Integration;
    m.view.selected_engine = Some(EngineId::new("gamestream").unwrap());
    if let Some(e) = m.integrations.iter_mut().find(|e| e.manifest.id.as_str() == "gamestream") {
        e.targets[0].reach = ipc::Reach::Offline;
    }
    m.view.focus = Focus::Setting(0);
    add("21-state-offline-pc", m);
    let mut m = Model::new(State::default(), Listing::default(), base.integrations.iter().map(|e| e.manifest.clone()).collect());
    m.view.focus = Focus::FirstRun(0);
    add("22-state-first-run", m);
    for (name, page) in [("30-home-text200", Page::Home), ("31-hub-text200", Page::Hub)] {
        let mut m = base.clone();
        m.prefs.text_percent = 200;
        m.view.page = page;
        m.view.focus = if page == Page::Hub { Focus::HubAction(0) } else { Focus::Home(0, 1) };
        add(name, m);
    }
    let mut m = base.clone();
    m.view.page = Page::Search;
    m.view.query = "Hades".into();
    m.filtered = vec![id(&m, "Hades II")];
    m.view.focus = Focus::Grid(0);
    add("37-search", m);
    let mut m = base.clone();
    m.view.page = Page::Search;
    m.view.focus = Focus::Keyboard(0);
    add("38-search-keyboard", m);
    let mut m = base.clone();
    m.view.page = Page::Media;
    m.view.media = 2;
    add("39-media-image", m);
    for (name, family, light, page) in [
        ("34-home-nord-light", "nord", true, Page::Home),
        ("35-hub-solarized", "solarized", false, Page::Hub),
        ("36-settings-gruvbox-light", "gruvbox", true, Page::Settings),
    ] {
        let mut m = base.clone();
        m.prefs.family = family.into();
        m.prefs.light = light;
        m.view.page = page;
        add(name, m);
    }
    let mut m = base.clone();
    m.view.page = Page::Integration;
    m.view.selected_engine = Some(EngineId::new("gamestream").unwrap());
    m.view.sheet = Some(Sheet::AddHost);
    m.view.focus = Focus::Keyboard(0);
    add("41-add-pc", m);
    let mut m = base.clone();
    m.view.page = Page::Integration;
    m.view.selected_engine = Some(EngineId::new("native").unwrap());
    m.view.sheet = Some(Sheet::EditText);
    m.view.focus = Focus::Keyboard(0);
    m.view.address = "/media/games/quake".into();
    add("42-edit-path", m);
    let mut lab = base.clone();
    lab.prefs.reduce_motion = true;
    lab.view.page = Page::Settings;
    lab.view.section = Section::Controllers;
    lab.view.focus = Focus::Setting(0);
    activate(&mut lab);
    let reports = (0..32).map(|i| (0, lumatv_gfn_pad::RawReport {
        kernel_ns: 1_000_000_000 + i * 2_000_000, read_ns: 1_000_080_000 + i * 2_000_000,
        left: (0.16, -0.12), right: (0.02, 0.01), left_trigger: 158, right_trigger: 28, buttons: lumatv_gfn_pad::button::A
    })).collect();
    lab.step(Input::Lab { now_ns: 1_400_000_000, frame: 120, reports, names: [Some("GameSir Cyclone 2".into()), None, None, None] });
    add("43-controller-lab", lab.clone());
    let mut light = lab.clone();
    light.prefs.light = true;
    add("43-controller-lab-light", light);
    for (name, focus) in [("43-controller-sticks", Focus::Lab(5)), ("43-controller-buttons", Focus::Lab(6)),
        ("43-controller-timing", Focus::Lab(7)), ("43-controller-shaping", Focus::Setting(2))] {
        let mut view = lab.clone();
        view.view.focus = focus;
        add(name, view);
    }
    let mut captured = lab.clone();
    captured.view.focus = Focus::Lab(0);
    activate(&mut captured);
    add("43-controller-captured", captured);
    lab.view.focus = Focus::Lab(3);
    activate(&mut lab);
    lab.step(Input::Lab { now_ns: 1_500_000_000, frame: 121, reports: vec![(0, lumatv_gfn_pad::RawReport {
        kernel_ns: 1_499_000_000, buttons: lumatv_gfn_pad::button::X, ..Default::default()
    })], names: [Some("GameSir Cyclone 2".into()), None, None, None] });
    add("44-controller-flash", lab);
    for (name, state) in [
        ("46-gamesir-off", GameSirState::Off), ("46-gamesir-missing", GameSirState::Missing),
        ("46-gamesir-ds4", GameSirState::Mode { device: "Wireless Controller".into() }), ("46-gamesir-ready", gamesir_fixture()),
    ] {
        let mut m = base.clone();
        m.prefs.reduce_motion = true;
        m.view.page = Page::GameSir;
        m.view.focus = Focus::Setting(0);
        m.step(Input::GameSir(Ok(state)));
        add(name, m.clone());
        if name == "46-gamesir-ready" {
            m.prefs.light = true;
            add("46-gamesir-ready-light", m.clone());
            m.prefs.light = false;
            m.view.focus = Focus::Setting(7);
            add("46-gamesir-bottom", m.clone());
            m.gamesir.pending = true;
            add("46-gamesir-pending", m.clone());
            m.step(Input::GameSir(Err("The controller did not keep the change. Refresh before changing another setting.".into())));
            add("46-gamesir-error", m);
        }
    }
    for (name, state) in [("47-leadjoy-off", LeadjoyState::Off), ("47-leadjoy-missing", LeadjoyState::Missing),
        ("47-leadjoy-unsupported", LeadjoyState::Unsupported { device: "Other".into(), firmware: "1".into() }), ("47-leadjoy-ready", leadjoy_fixture("general"))] {
        let mut m = base.clone();
        m.prefs.reduce_motion = true;
        m.view.page = Page::Leadjoy;
        m.view.focus = Focus::Setting(0);
        m.step(Input::Leadjoy(Ok(state)));
        add(name, m.clone());
        if name == "47-leadjoy-ready" {
            m.prefs.light = true;
            add("47-leadjoy-light", m.clone());
            m.prefs.light = false;
            m.view.focus = Focus::Setting(4);
            m.leadjoy.pending = true;
            add("47-leadjoy-pending", m.clone());
            m.step(Input::Leadjoy(Err("Read-back did not match. Refresh before changing another setting.".into())));
            add("47-leadjoy-error", m);
        }
    }
    for (section, row) in [("profiles", 3), ("buttons", 8), ("macros", 7), ("sticks", 16), ("curves", 16), ("directions", 6), ("triggers", 11), ("motion", 24), ("gestures", 6), ("rumble", 9)] {
        let mut m = base.clone();
        m.prefs.reduce_motion = true;
        m.view.page = Page::Leadjoy;
        m.step(Input::Leadjoy(Ok(leadjoy_fixture(section))));
        m.view.focus = Focus::Setting(row);
        add(&format!("47-leadjoy-{section}"), m.clone());
        if section == "profiles" {
            activate(&mut m);
            add("47-leadjoy-rename", m);
        } else if section == "macros" {
            m.view.focus = Focus::Setting(11);
            activate(&mut m);
            add("47-leadjoy-delete", m);
        }
    }
    let large = out
        .iter()
        .filter(|(name, _)| {
            [
                "48-stats-compact-bright", "49-stats-full-bright", "50-stats-full-dark", "50-stats-full-dark-light", "51-stats-stale-dark",
                "14-settings-streaming", "15-overlay",
                "11-pairing",
                "12-signin",
                "28-enter-pin",
                "29-approval",
                "16-overlay-end-confirm",
                "22-state-first-run",
                "38-search-keyboard",
                "41-add-pc",
                "42-edit-path",
                "13-settings-appearance",
                "25-settings-controllers",
                "26-settings-about",
                "43-controller-lab",
                "43-controller-lab-light",
                "44-controller-flash",
                "45-controller-lab-stream",
                "46-gamesir-ready",
                "46-gamesir-ready-light",
                "46-gamesir-ds4",
                "43-controller-sticks", "43-controller-buttons", "43-controller-timing", "43-controller-shaping", "43-controller-captured",
                "45-controller-lab-stream-bottom", "46-gamesir-bottom", "46-gamesir-pending", "46-gamesir-error",
                "47-leadjoy-ready", "47-leadjoy-light", "47-leadjoy-pending", "47-leadjoy-error", "47-leadjoy-buttons", "47-leadjoy-sticks",
                "47-leadjoy-rename", "47-leadjoy-macros", "47-leadjoy-motion", "47-leadjoy-delete",
            ]
            .contains(&name.as_str())
        })
        .map(|(name, m)| {
            let mut m = m.clone();
            m.prefs.text_percent = 200;
            if m.view.page == Page::Settings && m.view.sheet.is_none() {
                m.view.focus = Focus::Setting(if m.view.section == Section::Appearance { 5 } else { 0 });
            }
            m.rebuild_navigation();
            (format!("{name}-text200"), m)
        })
        .collect::<Vec<_>>();
    out.extend(large);
    out
}
fn png(path: &Path, w: u32, h: u32, bytes: &[u8]) -> io::Result<()> {
    let mut encoder = png::Encoder::new(io::BufWriter::new(std::fs::File::create(path)?), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().and_then(|mut w| w.write_image_data(bytes)).map_err(io::Error::other)
}
#[test]
fn every_native_screen() -> io::Result<()> {
    let selection=std::env::var("GAME_SHOTS").unwrap_or_default();
    if selection.is_empty() { return Ok(()); }
    let scratch = std::env::var_os("CODY_SCRATCH").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let out = scratch.join("lumagame-native-shots");
    std::fs::create_dir_all(&out)?;
    let mocks = scratch.join("lumagame-shots");
    let mut shots = Shots::new()?;
    for (name, model) in screens() {
        if selection!="all" && !selection.split(',').any(|s| name.contains(s)) {continue;}
        let mut pixels = shots.shot(&model)?;
        if name.contains("-stats-") {assert!(!shots.page.show(&model)?, "unchanged HUD repainted");}
        if name.contains("-stats-") {
            let app=if name.contains("bright") {1551360} else {1091500};
            let background=lumagame_meta::decode_image(&std::fs::read(assets().join(format!("{app}/frame.jpg")))?,
                lumagame_meta::ImageSpec {width:1920,height:1080,contain:false,trim_alpha:false}).map_err(io::Error::other)?;
            for (fg,bg) in pixels.chunks_exact_mut(4).zip(background.rgba.chunks_exact(4)) {
                let alpha=u32::from(fg[3]);
                for i in 0..3 {fg[i]=(u32::from(fg[i])+u32::from(bg[i])*(255-alpha)/255).min(255) as u8;}
                fg[3]=255;
            }
        }
        png(&out.join(format!("{name}.png")), 1920, 1080, &pixels)?;
        let mock = mocks.join(format!("{name}.png"));
        if mock.exists() {
            let reference =
                lumagame_meta::decode_image(&std::fs::read(mock)?, lumagame_meta::ImageSpec { width: 1920, height: 1080, contain: false, trim_alpha: false })
                    .map_err(io::Error::other)?;
            let mut side = vec![0; 3840 * 1080 * 4];
            for y in 0..1080 {
                side[y * 3840 * 4..(y * 3840 + 1920) * 4].copy_from_slice(&reference.rgba[y * 1920 * 4..(y + 1) * 1920 * 4]);
                side[(y * 3840 + 1920) * 4..(y + 1) * 3840 * 4].copy_from_slice(&pixels[y * 1920 * 4..(y + 1) * 1920 * 4]);
            }
            png(&out.join(format!("{name}-compare.png")), 3840, 1080, &side)?;
        }
        println!("{}", out.join(format!("{name}.png")).display());
    }
    Ok(())
}

#[test]
fn native_text_fields_activate_on_release_and_validate_before_saving() {
    use lumagame_core::settings::{Field, FieldKind, Section, SettingsSchema};
    let mut m = fixture();
    let native = EngineId::new("native").unwrap();
    m.integrations.iter_mut().find(|e| e.manifest.id == native).unwrap().schema = SettingsSchema {
        sections: vec![Section {
            title: "Games".into(),
            fields: vec![Field { key: "path".into(), label: "Game folder".into(), help: None, kind: FieldKind::Text { max_chars: 3, default: String::new() } }],
        }],
    };
    m.view.page = Page::Integration;
    m.view.selected_engine = Some(native);
    m.view.focus = Focus::EngineField(0);
    m.rebuild_navigation();
    m.step(Input::Key { key: Key::Right, down: true });
    assert_eq!(m.view.sheet, None);
    m.step(Input::Key { key: Key::Ok, down: true });
    assert_eq!(m.view.sheet, None);
    m.step(Input::Key { key: Key::Ok, down: false });
    assert_eq!(m.view.sheet, Some(Sheet::EditText));
    m.step(Input::Text("abcd".into()));
    m.view.focus = Focus::SheetAction(0);
    activate(&mut m);
    assert_eq!(m.view.sheet, Some(Sheet::EditText));
    assert_eq!(m.view.notice.as_deref(), Some("Use at most 3 characters."));
    m.view.address = "abc".into();
    activate(&mut m);
    assert_eq!(m.view.sheet, None);
    assert_eq!(m.view.focus, Focus::EngineField(0));
}

#[test]
fn closing_waits_for_session_end_and_records_the_final_seconds() {
    let mut m = fixture();
    let game = id(&m, "Cyberpunk 2077");
    let before = m.state().profile().history.total(game).seconds;
    m.view.page = Page::Hub;
    m.view.selected = Some(game);
    m.view.focus = Focus::HubAction(0);
    activate(&mut m);
    let session = m.session().unwrap().id;
    event(&mut m, "gfn", ipc::Event::Session { session, state: SessionState::Starting });
    event(&mut m, "gfn", ipc::Event::Session { session, state: SessionState::Running { stream: None } });
    m.step(Input::Tick { now_ms: 17_000, unix_s: 1_800_000_017 });
    let commands = m.step(Input::Close);
    assert!(!commands.iter().any(|c| matches!(c, Command::Exit)));
    assert!(commands.iter().any(|c| matches!(c, Command::Send { request: ipc::Request::Stop { end_game: false, .. }, .. })));
    let commands = m.step(Input::Engine {
        engine: EngineId::new("gfn").unwrap(),
        message: FromEngine { re: None, event: ipc::Event::Session { session, state: SessionState::Ended { reason: ipc::EndReason::Stopped } } },
    });
    assert!(commands.iter().any(|c| matches!(c, Command::Exit)));
    assert_eq!(m.state().profile().history.total(game).seconds, before + 17);
}

#[test]
fn remote_back_opens_the_overlay_while_a_game_runs() {
    let mut m = fixture();
    m.view.page = Page::Hub;
    m.view.selected = Some(id(&m, "Cyberpunk 2077"));
    m.view.focus = Focus::HubAction(0);
    activate(&mut m);
    let session = m.session().unwrap().id;
    event(&mut m, "gfn", ipc::Event::Session { session, state: SessionState::Running { stream: None } });
    assert_eq!(m.view.page, Page::Running);
    m.step(Input::Key { key: Key::Back, down: true });
    m.step(Input::Key { key: Key::Back, down: false });
    assert_eq!(m.view.page, Page::Overlay);
}

#[test]
fn all_stats_controls_share_one_setting_and_off_stops_frame_capture() {
    use super::stats::{Mode,Overlay};
    let mut m=fixture();
    m.view.page=Page::Hub;m.view.selected=Some(id(&m,"Cyberpunk 2077"));m.view.focus=Focus::HubAction(0);
    activate(&mut m);
    let session=m.session().unwrap().id;
    event(&mut m,"gfn",ipc::Event::Session {session,state:SessionState::Running {stream:None}});
    assert!(!m.hud_visible() && !m.wants_frames());
    m.step(Input::StatsSetting(Overlay {mode:Mode::Compact,changed:1}));
    assert!(m.hud_visible() && m.wants_frames());
    m.step(Input::Overlay);
    m.step(Input::Key {key:Key::Menu,down:true});
    m.step(Input::Key {key:Key::Menu,down:false});
    assert_eq!(m.prefs.stats.mode,Mode::Full);
    m.view.focus=Focus::Overlay(2);activate(&mut m);
    assert_eq!(m.prefs.stats.mode,Mode::Off);
    m.view.page=Page::Settings;m.view.section=Section::Streaming;m.view.focus=Focus::Setting(6);
    m.step(Input::Key {key:Key::Right,down:true});m.step(Input::Key {key:Key::Right,down:false});
    assert_eq!(m.prefs.stats.mode,Mode::Compact);
    let changed=m.prefs.stats.changed;
    m.step(Input::StatsSetting(Overlay {mode:Mode::Full,changed:changed-1}));
    assert_eq!(m.prefs.stats.mode,Mode::Compact,"an older helper reply cannot undo a local edit");
    let out=m.step(Input::StatsSetting(Overlay {mode:Mode::Off,changed:changed+1}));
    assert!(out.iter().any(|c| matches!(c,Command::Send {request:ipc::Request::Stats {on:false,..},..})));
    m.view.page=Page::Running;
    assert!(!m.hud_visible() && !m.wants_frames());
}

#[test]
fn controller_sampling_follows_the_visible_hud_not_menu_statistics() {
    use super::stats::{Mode,Overlay};
    let mut m=fixture();
    m.view.page=Page::Hub;m.view.selected=Some(id(&m,"Cyberpunk 2077"));m.view.focus=Focus::HubAction(0);
    activate(&mut m);
    let session=m.session().unwrap().id;
    event(&mut m,"gfn",ipc::Event::Session {session,state:SessionState::Running {stream:None}});
    let flags=|commands:Vec<Command>| commands.into_iter().find_map(|c| match c {
        Command::Send {request:ipc::Request::Stats {on,pad_timing,..},..}=>Some((on,pad_timing)),
        _=>None,
    }).expect("a visibility change updates both measurement flags");
    assert_eq!(flags(m.step(Input::Overlay)),(true,false));
    for (mode,changed) in [(Mode::Compact,1),(Mode::Full,2)] {
        assert_eq!(flags(m.step(Input::StatsSetting(Overlay {mode,changed}))),(true,false));
        let mut commands=m.step(Input::Key {key:Key::Back,down:true});
        commands.extend(m.step(Input::Key {key:Key::Back,down:false}));
        assert_eq!(flags(commands),(true,true));
        assert_eq!(flags(m.step(Input::Overlay)),(true,false));
    }
    assert_eq!(flags(m.step(Input::StatsSetting(Overlay {mode:Mode::Off,changed:3}))),(true,false));
    assert_eq!(flags(m.step(Input::StatsSetting(Overlay {mode:Mode::Compact,changed:4}))),(true,false));
    assert_eq!(flags(m.step(Input::Foreground(false))),(false,false));
}

#[test]
fn large_text_stacks_actions_and_down_follows_the_visible_column() {
    let mut m = fixture();
    m.prefs.text_percent = 200;
    for (page, first, second) in [
        (Page::Home, Focus::Hero(0), Focus::Hero(1)),
        (Page::Hub, Focus::HubAction(0), Focus::HubAction(1)),
    ] {
        m.view.page = page;
        m.view.focus = first;
        m.rebuild_navigation();
        m.step(Input::Key { key: Key::Down, down: true });
        assert_eq!(m.view.focus, second);
    }
}

#[test]
fn engine_ready_applies_stream_preferences_once_without_schema_feedback() {
    let mut m = fixture();
    let id = EngineId::new("gfn").unwrap();
    let engine = m.integrations.iter_mut().find(|e| e.manifest.id == id).unwrap();
    let descriptor = engine.descriptor.take().unwrap();
    let schema = serde_json::from_value(serde_json::json!({"sections":[{"title":"Stream","fields":[
        {"key":"bitrate-mbps","label":"Bitrate","type":"number","min":5,"max":150,"step":5,"default":80}
    ]}]})).unwrap();
    let event = FromEngine { re: None, event: ipc::Event::Ready { descriptor, schema } };
    m.prefs.bitrate_mbps = 150;
    let commands = m.step(Input::Engine { engine: id.clone(), message: event.clone() });
    assert!(commands.iter().any(|c| matches!(c, Command::Send { request: ipc::Request::Settings { values }, .. }
        if values.get("bitrate-mbps") == Some(&lumagame_core::settings::Value::Int(150)))));
    let commands = m.step(Input::Engine { engine: id, message: event });
    assert!(!commands.iter().any(|c| matches!(c, Command::Send { request: ipc::Request::Settings { .. }, .. })));
}
