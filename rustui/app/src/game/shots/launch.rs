//! Launch, running, stats, idle and overlay shots.
use super::*;

/// Declares `feature` on `engine`, which a shot's engine does not always have.
fn feature(m: &mut Model, engine: &str, feature: Feature) {
    use crate::game::model::tests::describe;
    let described = m.integrations.iter().any(|e| e.manifest.id.as_str() == engine && e.descriptor.is_some());
    if !described {
        describe(m, engine, vec![], &[feature]);
        return;
    }
    m.integrations.iter_mut().find(|e| e.manifest.id.as_str() == engine).and_then(|e| e.descriptor.as_mut()).unwrap().capabilities.features.insert(feature);
}

pub(super) fn screens(base: &Model, add: &mut impl FnMut(&str, Model)) {
    // The start screen, step by step: GeForce NOW connects, signs in, queues and starts; a sleeping PC wakes first.
    let mut launched = base.clone();
    feature(&mut launched, "gfn", Feature::Queue);
    launched.view.selected = Some(id(&launched, "Cyberpunk 2077"));
    launched.view.page = Page::Game;
    launched.view.chooser_selected = launched.selected_game().unwrap().sources.iter().find(|s| s.engine.as_str() == "gfn").map(|s| s.key());
    launched.view.focus = Focus::Play;
    activate(&mut launched);
    let session = launched.session().unwrap().id;
    add("launch-start-connecting", launched.clone());
    let mut sign_in = launched.clone();
    event(&mut sign_in, "gfn", ipc::Event::Session { session, state: SessionState::SignIn });
    add("launch-start-signin", sign_in);
    event(&mut launched, "gfn", ipc::Event::Session { session, state: SessionState::Queue { position: Some(15), eta_s: Some(120) } });
    let variants = [("", false, 100), ("-light", true, 100), ("-text200", false, 200)];
    for (suffix, light, percent) in variants {
        let mut m = launched.clone();
        m.prefs.light = light;
        m.prefs.text_percent = percent;
        add(&format!("launch-c-strip{suffix}"), m);
    }
    let mut unknown = launched.clone();
    event(&mut unknown, "gfn", ipc::Event::Session { session, state: SessionState::Queue { position: None, eta_s: None } });
    add("launch-start-queue-unknown", unknown);
    let mut dim = launched.clone();
    dim.step(Input::Tick { now_ms: 61_000, unix_s: 1_800_000_061 });
    add("launch-start-dim", dim);
    let mut wake = base.clone();
    feature(&mut wake, "gamestream", Feature::Wake);
    wake.integrations.iter_mut().find(|e| e.manifest.id.as_str() == "gamestream").unwrap().targets.iter_mut().for_each(|t| t.reach = ipc::Reach::Asleep);
    wake.view.selected = Some(id(&wake, "Ori and the Will of the Wisps"));
    wake.view.page = Page::Game;
    wake.view.chooser_selected = wake.selected_game().unwrap().sources.iter().find(|s| s.engine.as_str() == "gamestream").map(|s| s.key());
    wake.view.focus = Focus::Play;
    activate(&mut wake);
    add("launch-start-wake", wake.clone());
    wake.prefs.light = true;
    add("launch-start-wake-light", wake);
    event(&mut launched, "gfn", ipc::Event::Session { session, state: SessionState::Starting });
    add("launch-start-starting", launched.clone());
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
    // The strip and its figures leave after five seconds; the shots below are of the bare stream and of what shows over it.
    launched.step(Input::Tick { now_ms: 6_000, unix_s: 1_800_000_006 });
    add("40-running-transparent", launched.clone());
    // The idle prompt over a game frame (a dark and a bright one), then the Hub after the far side ended the session for it.
    let mut idle = launched.clone();
    event(&mut idle, "gfn", ipc::Event::IdleWarning { session, seconds: Some(59) });
    let mut idle_light = idle.clone();
    idle_light.prefs.light = true;
    add("52-idle-warning", idle.clone());
    add("52-idle-warning-light", idle_light);
    add("52-idle-warning-bright", idle.clone());
    event(&mut idle, "gfn", ipc::Event::Session { session, state: SessionState::Ended { reason: ipc::EndReason::Idle } });
    let mut ended_light = idle.clone();
    ended_light.prefs.light = true;
    add("53-idle-ended", idle);
    add("53-idle-ended-light", ended_light);
    // The stats overlay in four situations, each over a bright and a dark game frame (the file name says which), as a strip and as
    // the full panel, in the dark and light themes.
    for kind in ["healthy","network","host","tv"] {
        for (mode,word) in [(crate::game::stats::Mode::Compact,"compact"),(crate::game::stats::Mode::Full,"full")] {
            let hud=hud_model(&launched,mode,kind);
            for light in [false,true] {
                let mut m=hud.clone();
                m.prefs.light=light;
                add(&format!("48-stats-{word}-{kind}{}",if light {"-light"} else {""}),m);
            }
            add(&format!("48-stats-{word}-{kind}-bright"),hud);
        }
    }
    let mut stale=hud_model(&launched,crate::game::stats::Mode::Full,"healthy");
    stale.step(Input::Tick {now_ms:40_000,unix_s:1_800_000_040});
    add("51-stats-full-stale",stale);
    launched.step(Input::Tick { now_ms: 1_932_000, unix_s: 1_800_001_932 });
    launched.step(Input::Overlay);
    launched.view.stats = ipc::Stats { rtt_ms: Some(18.0), loss_pct: Some(0.0), bitrate_kbps: Some(78_000), ..Default::default() };
    for (suffix, light, percent) in variants {
        let mut m = launched.clone();
        m.prefs.light = light;
        m.prefs.text_percent = percent;
        add(&format!("launch-c-strip-menu{suffix}"), m);
    }
    let mut steam = launched.clone();
    feature(&mut steam, "gfn", Feature::SteamOverlay);
    add("launch-strip-steam", steam);
    let mut clip = launched.clone();
    crate::game::model::tests::recorder_on(&mut clip, "gfn");
    add("launch-strip-recorder", clip.clone());
    let mut recorder = clip.clone();
    recorder.view.focus = Focus::Strip(4);
    activate(&mut recorder);
    event(&mut recorder, "gfn", ipc::Event::Recorder { session, replay: true, recording: true });
    add("launch-recorder", recorder);
    clip.view.focus = Focus::Strip(0);
    activate(&mut clip);
    event(&mut clip, "gfn", ipc::Event::Recorder { session, replay: true, recording: true });
    add("launch-recording", clip.clone());
    event(&mut clip, "gfn", ipc::Event::Clip { session, saved: true, text: "Saved Hades II/2026-01-01 12.00.00.mp4, 5 min 0 s, 2.4 GB".into() });
    add("launch-clip-saved", clip);
    let mut picture = launched.clone();
    picture.view.focus = Focus::Strip(1);
    activate(&mut picture);
    add("launch-picture", picture);
    let mut medals = launched.clone();
    medals.view.focus = Focus::Strip(3);
    activate(&mut medals);
    add("launch-achievements", medals);
    let mut lab_stream = launched.clone();
    lab_stream.integrations.iter_mut().find(|e| e.manifest.id.as_str() == "gfn").unwrap()
        .descriptor.as_mut().unwrap().capabilities.features.insert(Feature::RoundTrip);
    lab_stream.view.page = Page::Settings;
    lab_stream.view.place = Place::Controllers;
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
    launched.view.focus = Focus::Strip(4);
    activate(&mut launched);
    for (suffix, light, percent) in variants {
        let mut m = launched.clone();
        m.prefs.light = light;
        m.prefs.text_percent = percent;
        add(&format!("launch-b-drawer-end{suffix}"), m);
    }
}

/// The budget walks: a queued start held for more than two turns of the ring, and the strip leaving, coming back on a key and opening a
/// drawer.
pub(super) fn walks() -> Vec<super::budget::Walk> {
    use super::budget::{Step, Walk};
    let mut start = fixture();
    feature(&mut start, "gfn", Feature::Queue);
    start.view.page = Page::Game;
    start.view.selected = Some(id(&start, "Cyberpunk 2077"));
    start.view.focus = Focus::Play;
    activate(&mut start);
    let session = start.session().unwrap().id;
    event(&mut start, "gfn", ipc::Event::Session { session, state: SessionState::Queue { position: Some(15), eta_s: Some(120) } });
    let mut running = start.clone();
    let stream = serde_json::json!({"codec":"h265","width":3840,"height":2160,"fps":120,"hdr":true,"audio_channels":2});
    event(&mut running, "gfn", ipc::Event::Session { session, state: SessionState::Running { stream: Some(serde_json::from_value(stream).unwrap()) } });
    running.view.stats = ipc::Stats { rtt_ms: Some(18.0), ..Default::default() };
    let waiting = vec![
        Step::Settle,
        Step::Expect("the start screen with its line", |m, _| m.view.page == Page::Launch && m.start_step().is_some()),
        Step::Wait(2_500),
        Step::Expect("the ring to have turned", |m, _| m.now_ms() >= 2_500),
    ];
    let strip = vec![
        Step::Wait(500),
        Step::Expect("the strip with its figures", |m, _| m.strip_visible() && m.figures().len() == 3),
        Step::Wait(5_000),
        Step::Expect("the strip to have left", |m, _| !m.strip_visible()),
        Step::Tap(Key::Left),
        Step::Expect("the first key to bring only the strip back", |m, _| m.strip_visible() && m.view.top().is_none()),
        Step::Tap(Key::Right),
        Step::Tap(Key::Ok),
        Step::Expect("the Picture drawer", |m, _| m.view.top() == Some(crate::game::model::Layer::Picture)),
        Step::Wait(300),
        Step::Tap(Key::Back),
        Step::Wait(5_500),
        Step::Expect("the stream bare again", |m, _| m.stream_bare()),
    ];
    vec![Walk { name: "launch-start".into(), base: start, script: waiting }, Walk { name: "strip-in-out".into(), base: running, script: strip }]
}

#[test]
fn closing_waits_for_session_end_and_records_the_final_seconds() {
    let mut m = fixture();
    let game = id(&m, "Cyberpunk 2077");
    let before = m.state().profile().history.total(game).seconds;
    m.view.page = Page::Game;
    m.view.selected = Some(game);
    m.view.focus = Focus::Play;
    activate(&mut m);
    let session = m.session().unwrap().id;
    event(&mut m, "gfn", ipc::Event::Session { session, state: SessionState::Starting });
    event(&mut m, "gfn", ipc::Event::Session { session, state: SessionState::Running { stream: None } });
    m.step(Input::Tick { now_ms: 17_000, unix_s: 1_800_000_017 });
    let commands = m.step(Input::Close);
    assert!(!commands.iter().any(|c| matches!(c, Command::Exit)));
    assert!(commands.iter().any(|c| matches!(c, Command::Send { request: ipc::Request::Stop { end_game: false, .. }, .. })));
    let commands = m.step(Input::Engine { engine: EngineId::new("gfn").unwrap(), message: Box::new(FromEngine { re: None, event: ipc::Event::Session { session, state: SessionState::Ended { reason: ipc::EndReason::Stopped } } }) });
    assert!(commands.iter().any(|c| matches!(c, Command::Exit)));
    assert_eq!(m.state().profile().history.total(game).seconds, before + 17);
}

#[test]
fn all_stats_controls_share_one_setting_and_off_stops_frame_capture() {
    use crate::game::stats::{Mode,Overlay};
    let mut m=fixture();
    m.view.page=Page::Game;m.view.selected=Some(id(&m,"Cyberpunk 2077"));m.view.focus=Focus::Play;
    activate(&mut m);
    let session=m.session().unwrap().id;
    event(&mut m,"gfn",ipc::Event::Session {session,state:SessionState::Running {stream:None}});
    m.step(Input::Tick {now_ms:6_000,unix_s:0});
    assert!(!m.hud_visible() && !m.wants_frames());
    m.step(Input::StatsSetting(Overlay {mode:Mode::Compact,changed:1}));
    assert!(m.hud_visible() && m.wants_frames());
    m.step(Input::Overlay);
    m.view.focus=Focus::Strip(1);activate(&mut m);
    m.view.focus=Focus::SheetRow(2);activate(&mut m);
    assert_eq!(m.prefs.stats.mode,Mode::Off);
    m.view.clear_layers();m.view.page=Page::Settings;m.view.place=Place::Picture;m.view.focus=Focus::Setting(8);
    m.step(Input::Key {key:Key::Right,down:true});m.step(Input::Key {key:Key::Right,down:false});
    assert_eq!(m.prefs.stats.mode,Mode::Compact);
    let changed=m.prefs.stats.changed;
    m.step(Input::StatsSetting(Overlay {mode:Mode::Full,changed:changed-1}));
    assert_eq!(m.prefs.stats.mode,Mode::Compact,"an older helper reply cannot undo a local edit");
    let out=m.step(Input::StatsSetting(Overlay {mode:Mode::Off,changed:changed+1}));
    assert!(out.iter().any(|c| matches!(c,Command::Send {request:ipc::Request::Stats {on:false,..},..})));
    m.view.page=Page::Running;m.step(Input::Tick {now_ms:20_000,unix_s:0});
    assert!(!m.hud_visible() && !m.wants_frames());
}

#[test]
fn compact_stats_reach_the_screen_in_a_running_game_at_both_text_sizes() {
    use crate::game::stats::{Mode,Overlay};
    let mut m=fixture();
    m.view.page=Page::Game;m.view.selected=Some(id(&m,"Cyberpunk 2077"));m.view.focus=Focus::Play;
    activate(&mut m);
    let session=m.session().unwrap().id;
    event(&mut m,"gfn",ipc::Event::Session {session,state:SessionState::Running {stream:None}});
    for (mode,changed) in [(Mode::Compact,1),(Mode::Full,2)] {
        m.step(Input::StatsSetting(Overlay {mode,changed}));
        for percent in [100,200] {
            m.prefs.text_percent=percent;
            let planned=crate::game::screens::planned(&m);
            let live=|n: u64| planned.iter().any(|(id,f)| *id==crate::game::screens::HUD.start+n && f.w>0.0 && f.h>0.0);
            assert!(live(0) && live(1),"{mode:?} at {percent}% drew no HUD layers: {planned:?}");
        }
    }
}

#[test]
fn the_stats_hud_dims_and_steps_inside_the_safe_area_in_a_running_game() {
    use crate::game::stats::{DIM_AFTER_MS,DIM_TO,Mode,Overlay,SHIFT_EVERY_MS};
    let mut m=fixture();
    m.view.page=Page::Game;m.view.selected=Some(id(&m,"Cyberpunk 2077"));m.view.focus=Focus::Play;
    activate(&mut m);
    let session=m.session().unwrap().id;
    event(&mut m,"gfn",ipc::Event::Session {session,state:SessionState::Running {stream:None}});
    for (mode,changed) in [(Mode::Compact,1),(Mode::Full,2)] {
        m.step(Input::StatsSetting(Overlay {mode,changed}));
        let hud=|m: &Model| crate::game::screens::planned_with_opacity(m).into_iter().find(|(id,..)| *id==crate::game::screens::HUD.start).unwrap();
        m.step(Input::Tick {now_ms:1_000,unix_s:0});
        let (_,corner,bright)=hud(&m);
        assert_eq!(bright,1.0,"{mode:?} starts at the strength it always had");
        let revision=m.revision();
        m.step(Input::Tick {now_ms:1_000+DIM_AFTER_MS,unix_s:0});
        assert_ne!(m.revision(),revision,"{mode:?} is drawn again when it dims");
        let (_,dimmed,opacity)=hud(&m);
        assert_eq!((opacity,dimmed),(DIM_TO,corner),"{mode:?} dims in place");
        m.step(Input::Tick {now_ms:1_000+SHIFT_EVERY_MS,unix_s:0});
        let (_,moved,opacity)=hud(&m);
        assert_eq!(opacity,DIM_TO);
        assert!(moved.x<corner.x && moved.y>corner.y && corner.x-moved.x<=12.0 && moved.y-corner.y<=12.0,"{mode:?} steps left and down: {corner:?} to {moved:?}");
    }
}

#[test]
fn the_idle_countdown_runs_on_the_model_clock_and_only_the_far_sides_clear_dismisses_it() {
    let (mut m,session)=playing();
    assert_eq!(m.idle_left(),None);
    event(&mut m,"gfn",ipc::Event::IdleWarning {session,seconds:Some(59)});
    assert_eq!((m.idle_left(),m.wants_frames()),(Some(59),true));
    let revision=m.revision();
    m.step(Input::Tick {now_ms:1500,unix_s:0});
    assert_eq!(m.revision(),revision,"no repaint inside a second");
    m.step(Input::Tick {now_ms:2000,unix_s:0});
    assert_ne!(m.revision(),revision);
    assert_eq!(m.idle_left(),Some(58));
    m.step(Input::Tick {now_ms:90_000,unix_s:0});
    assert_eq!(m.idle_left(),Some(0),"the countdown stops at zero");
    // A press is not the answer: the far side's clear is, and until it arrives the prompt stays.
    m.step(Input::Key {key:Key::Ok,down:true});
    assert!(m.idle_left().is_some());
    event(&mut m,"gfn",ipc::Event::IdleWarning {session,seconds:None});
    m.step(Input::Tick {now_ms:100_000,unix_s:0});
    assert_eq!((m.idle_left(),m.wants_frames()),(None,false));
}

#[test]
fn a_key_tells_a_cloud_engine_at_most_once_a_second_during_the_countdown_and_once_in_30_seconds_otherwise() {
    let (mut m,session)=playing();
    let press=|m:&mut Model,key| activity(&m.step(Input::Key {key,down:true}));
    // 1000 ms on the model's clock: the first press of a session with no warning is sent, held keys are not.
    assert_eq!(press(&mut m,Key::Ok),1,"the remote's key never reaches the stream, so the engine is asked to send one");
    assert_eq!(press(&mut m,Key::Down),0,"held keys do not flood the link");
    m.step(Input::Tick {now_ms:20_000,unix_s:0});
    assert_eq!(press(&mut m,Key::Ok),0,"no warning: once in 30 s");
    m.step(Input::Tick {now_ms:31_100,unix_s:0});
    assert_eq!(press(&mut m,Key::Ok),1);
    // The countdown asks for more: one a second, so the prompt clears at once.
    event(&mut m,"gfn",ipc::Event::IdleWarning {session,seconds:Some(30)});
    assert_eq!(press(&mut m,Key::Ok),0,"a second has not passed");
    m.step(Input::Tick {now_ms:32_200,unix_s:0});
    assert_eq!(press(&mut m,Key::Ok),1);
    // A key release is not a press.
    m.step(Input::Tick {now_ms:35_000,unix_s:0});
    assert_eq!(activity(&m.step(Input::Key {key:Key::Ok,down:false})),0);
    // The menu's pad keys count as well.
    assert_eq!(activity(&m.step(Input::PadKey {slot:0,key:Key::Down,down:true})),1);
}

#[test]
fn the_idle_prompt_is_planned_below_the_stats_strip_for_the_countdown_only() {
    use crate::game::stats::{Mode,Overlay};
    let (mut m,session)=playing();
    let layers=|m:&Model| crate::game::screens::planned(m).into_iter().filter(|(id,_)| (90..92).contains(id)).collect::<Vec<_>>();
    assert!(layers(&m).is_empty(),"nothing is drawn without a warning");
    event(&mut m,"gfn",ipc::Event::IdleWarning {session,seconds:Some(59)});
    for percent in [100,200] {
        m.prefs.text_percent=percent;
        m.step(Input::StatsSetting(Overlay {mode:Mode::Off,changed:1}));
        let prompt=layers(&m);
        assert_eq!(prompt.len(),2,"{percent}%: {prompt:?}");
        assert!(prompt.iter().all(|(_,f)| f.w>0.0 && f.x>=0.0 && f.x+f.w<=1920.0 && f.y+f.h<=1080.0),"{percent}%: {prompt:?}");
        m.step(Input::StatsSetting(Overlay {mode:Mode::Compact,changed:2}));
        let hud=crate::game::screens::planned(&m).into_iter().find(|(id,_)| *id==crate::game::screens::HUD.start).unwrap().1;
        assert!(layers(&m).iter().all(|(_,f)| f.y>=hud.y+hud.h),"{percent}%: the prompt does not cover the strip");
    }
    event(&mut m,"gfn",ipc::Event::IdleWarning {session,seconds:None});
    assert!(layers(&m).is_empty(),"gone once the far side clears it");
}

#[test]
fn an_idle_end_returns_to_the_hub_with_a_calm_notice_and_the_game_launches_again_at_once() {
    let (mut m,session)=playing();
    event(&mut m,"gfn",ipc::Event::IdleWarning {session,seconds:Some(5)});
    event(&mut m,"gfn",ipc::Event::Session {session,state:SessionState::Ended {reason:ipc::EndReason::Idle}});
    assert_eq!(m.view.page,Page::Game);
    assert_eq!(m.view.notice.as_deref(),Some("GeForce NOW ended the session after inactivity."));
    assert!(!m.session_active() && m.idle_left().is_none() && m.view.idle.is_none());
    assert!(crate::game::screens::planned(&m).iter().all(|(id,_)| !(90..92).contains(id)),"no prompt left behind");
    m.view.focus=Focus::Play;
    let commands={m.step(Input::Key {key:Key::Ok,down:true});m.step(Input::Key {key:Key::Ok,down:false})};
    assert!(commands.iter().any(|c| matches!(c,Command::Send {request:ipc::Request::Launch {..},..})),"{commands:?}");
    assert!(m.session_active());
}

#[test]
fn controller_sampling_follows_the_visible_hud_not_menu_statistics() {
    use crate::game::stats::{Mode,Overlay};
    let mut m=fixture();
    m.view.page=Page::Game;m.view.selected=Some(id(&m,"Cyberpunk 2077"));m.view.focus=Focus::Play;
    activate(&mut m);
    let session=m.session().unwrap().id;
    event(&mut m,"gfn",ipc::Event::Session {session,state:SessionState::Running {stream:None}});
    let flags=|commands:Vec<Command>| commands.into_iter().find_map(|c| match c {
        Command::Send {request:ipc::Request::Stats {on,pad_timing,..},..}=>Some((on,pad_timing)),
        _=>None,
    }).expect("a visibility change updates both measurement flags");
    assert_eq!(flags(m.step(Input::Overlay)),(true,false));
    for (mode,changed) in [(Mode::Compact,1),(Mode::Full,2)] {
        // The strip leaves the HUD showing, so its readings go on; a drawer covers it, and nothing is read for a HUD that is not there.
        assert_eq!(flags(m.step(Input::StatsSetting(Overlay {mode,changed}))),(true,true));
        m.view.focus=Focus::Strip(1);
        let mut opened=m.step(Input::Key {key:Key::Ok,down:true});
        opened.extend(m.step(Input::Key {key:Key::Ok,down:false}));
        assert_eq!(flags(opened),(true,false));
        let mut closed=m.step(Input::Key {key:Key::Back,down:true});
        closed.extend(m.step(Input::Key {key:Key::Back,down:false}));
        assert_eq!(flags(closed),(true,true));
    }
    assert_eq!(flags(m.step(Input::StatsSetting(Overlay {mode:Mode::Off,changed:3}))),(true,false));
    assert_eq!(flags(m.step(Input::StatsSetting(Overlay {mode:Mode::Compact,changed:4}))),(true,true));
    assert_eq!(flags(m.step(Input::Foreground(false))),(false,false));
}
