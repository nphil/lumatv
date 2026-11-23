//! Settings, controller and stream-preference shots.
use super::*;

pub(super) fn screens(base: &Model, add: &mut impl FnMut(&str, Model)) {
    for (name, place, pane, focus, light, scale) in [
        ("settings-a-rail", Place::Accounts, None, Focus::Rail(0), false, 100),
        ("settings-a-rail-light", Place::Accounts, None, Focus::Rail(0), true, 100),
        ("settings-a-rail-text200", Place::Accounts, None, Focus::Rail(0), false, 200),
        ("settings-a-picture", Place::Picture, None, Focus::Setting(3), false, 100),
        ("settings-a-picture-limited", Place::Picture, None, Focus::Setting(3), false, 100),
        ("settings-a-picture-wifi", Place::Picture, None, Focus::Setting(3), false, 100),
        ("settings-a-picture-text200-limited", Place::Picture, None, Focus::Setting(3), false, 200),
        ("settings-a-controllers", Place::Controllers, None, Focus::Setting(0), false, 100),
        ("settings-a-appearance", Place::Appearance, None, Focus::Setting(1), false, 100),
        ("settings-a-theme", Place::Appearance, Some(Pane::Theme), Focus::Theme(3), false, 100),
        ("settings-a-system", Place::System, None, Focus::Setting(0), false, 100),
        ("settings-a-diagnostics", Place::System, Some(Pane::Diagnostics), Focus::Setting(1), false, 100),
        ("settings-a-diagnostics-light", Place::System, Some(Pane::Diagnostics), Focus::Setting(1), true, 100),
    ] {
        let mut m = base.clone();
        m.view.page = Page::Settings;
        (m.view.place, m.view.pane, m.view.focus) = (place, pane, focus);
        m.prefs.light = light;
        m.prefs.text_percent = scale;
        m.prefs.fps = 120;
        m.prefs.hdr = true;
        // A bitrate set higher than the TV's 100 Mbit/s port carries, and the TV on Wi-Fi.
        if name.ends_with("-limited") {
            m.uplink = Some(Uplink { interface: "eth0".into(), medium: Medium::Wired, mbit: Some(100) });
            m.prefs.bitrate_mbps = 150;
        }
        if name.ends_with("-wifi") {
            m.uplink = Some(Uplink { interface: "wlan0".into(), medium: Medium::Wifi, mbit: None });
        }
        m.diagnostics.level = crate::game::diagnostics::Level::Debug;
        add(name, m);
    }
    let mut secret = base.clone();
    secret.view.page = Page::Settings;
    (secret.view.place, secret.view.pane) = (Place::System, Some(Pane::Diagnostics));
    secret.view.push(Layer::EditText, secret.view.focus);
    secret.view.edit_field = Some(4);
    secret.view.address = "fixture-token".into();
    secret.view.focus = Focus::Keyboard(0);
    add("settings-a-diagnostics-token", secret);
    let mut long_diagnostics = base.clone();
    long_diagnostics.view.page = Page::Settings;
    (long_diagnostics.view.place, long_diagnostics.view.pane) = (Place::System, Some(Pane::Diagnostics));
    long_diagnostics.prefs.text_percent = 200;
    long_diagnostics.diagnostics.level = crate::game::diagnostics::Level::Trace;
    long_diagnostics.diagnostics.server = "https://long-server-name.example.invalid/a/long/path/for/remote/diagnostics".into();
    long_diagnostics.diagnostics.topic = "long_diagnostics_topic_name_for_a_large_text_settings_screen_123".into();
    long_diagnostics.view.focus = Focus::Setting(6);
    long_diagnostics.rebuild_navigation();
    add("settings-a-diagnostics-text200", long_diagnostics);
}
/// The rail stepping through all five places, then into Appearance's theme pane and back out.
pub(super) fn walks() -> Vec<super::budget::Walk> {
    use super::budget::{Step, Walk, taps};
    let mut base = fixture();
    base.open_settings(Place::Accounts);
    base.rebuild_navigation();
    let mut script = taps(&[(Key::Down, 150), (Key::Down, 150), (Key::Down, 150), (Key::Down, 150)]);
    script.push(Step::Expect("the rail on System", |model, _| model.view.place == Place::System));
    script.extend(taps(&[(Key::Up, 150), (Key::Right, 150), (Key::Ok, 300)]));
    script.push(Step::Expect("the theme pane open", |model, _| model.view.pane == Some(Pane::Theme)));
    script.extend(taps(&[(Key::Right, 150), (Key::Back, 300), (Key::Back, 300)]));
    script.push(Step::Expect("focus back on the rail", |model, _| model.view.focus == Focus::Rail(3)));
    vec![Walk { name: "settings-rail-walk".into(), base, script }]
}
pub(super) fn families(base: &Model, add: &mut impl FnMut(&str, Model)) {
    for (name, family, light, page) in [
        ("34-home-nord-light", "nord", true, Page::Home),
        ("35-hub-solarized", "solarized", false, Page::Game),
        ("36-settings-gruvbox-light", "gruvbox", true, Page::Settings),
    ] {
        let mut m = base.clone();
        m.prefs.family = family.into();
        m.prefs.light = light;
        m.view.page = page;
        add(name, m);
    }
}
pub(super) fn controllers(base: &Model, add: &mut impl FnMut(&str, Model)) {
    let mut lab = base.clone();
    lab.prefs.reduce_motion = true;
    lab.view.page = Page::Settings;
    lab.view.place = Place::Controllers;
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
        (m.view.page, m.view.place, m.view.pane) = (Page::Settings, Place::Controllers, Some(Pane::GameSir));
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
        (m.view.page, m.view.place, m.view.pane) = (Page::Settings, Place::Controllers, Some(Pane::Leadjoy));
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
    for (section, row) in [("profiles", 4), ("buttons", 8), ("macros", 7), ("sticks", 16), ("curves", 6), ("directions", 6), ("triggers", 11), ("motion", 23), ("gestures", 6), ("rumble", 9)] {
        let mut m = base.clone();
        m.prefs.reduce_motion = true;
        (m.view.page, m.view.pane) = (Page::Settings, Some(crate::game::model::settings::Pane::Leadjoy));
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
    for (fixture, key) in [("keyboard","key-0"),("mouse","key-2"),("motion-long","trigger-key-47"),("sticks-missing","anti-shake"),("profiles-sub","sub-key-0")] {
        let mut m = base.clone();
        m.prefs.reduce_motion = true;
        (m.view.page, m.view.pane) = (Page::Settings, Some(crate::game::model::settings::Pane::Leadjoy));
        let state = leadjoy_fixture(fixture);
        let LeadjoyState::Ready(profile) = &state else { unreachable!() };
        let focus = profile.schema.fields().position(|field| field.key == key).unwrap() + 1;
        m.step(Input::Leadjoy(Ok(state)));
        m.view.focus = Focus::Setting(focus);
        add(&format!("47-leadjoy-{fixture}-dark"),m.clone());
        m.prefs.light = true;
        add(&format!("47-leadjoy-{fixture}-light"),m);
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
    let commands = m.step(Input::Engine { engine: id.clone(), message: Box::new(event.clone()) });
    assert!(commands.iter().any(|c| matches!(c, Command::Send { request: ipc::Request::Settings { values }, .. }
        if values.get("bitrate-mbps") == Some(&lumagame_core::settings::Value::Int(150)))));
    let commands = m.step(Input::Engine { engine: id, message: Box::new(event) });
    assert!(!commands.iter().any(|c| matches!(c, Command::Send { request: ipc::Request::Settings { .. }, .. })));
}

#[test]
fn a_slow_wired_port_holds_the_engines_bitrate_below_the_viewers_choice() {
    let mut m = fixture();
    let id = EngineId::new("gfn").unwrap();
    m.integrations.iter_mut().find(|e| e.manifest.id == id).unwrap().schema = bitrate_schema();
    m.prefs.bitrate_mbps = 150;
    let commands = m.step(Input::Uplink(wired(100)));
    assert_eq!(sent_bitrates(&commands), [Value::Int(85)]);
    assert_eq!(m.prefs.bitrate_mbps, 150, "the slider keeps what the viewer chose");
    assert_eq!(m.state().engines[&id].settings["bitrate-mbps"], Value::Int(85), "an engine that restarts starts at the cap");
    assert!(m.step(Input::Uplink(wired(100))).is_empty(), "the same reading says nothing new");
    assert_eq!(sent_bitrates(&m.step(Input::Uplink(wired(1_000)))), [Value::Int(150)], "a faster port lifts the cap by itself");
    assert_eq!(sent_bitrates(&m.step(Input::Uplink(wired(100)))), [Value::Int(85)]);
    assert_eq!(sent_bitrates(&m.step(Input::Uplink(None))), [Value::Int(150)], "no uplink, no cap");
}

#[test]
fn a_backgrounded_app_does_not_talk_to_engines_the_host_has_stopped() {
    let mut m = fixture();
    let id = EngineId::new("gfn").unwrap();
    m.integrations.iter_mut().find(|e| e.manifest.id == id).unwrap().schema = bitrate_schema();
    m.prefs.bitrate_mbps = 150;
    m.step(Input::Foreground(false));
    assert!(sent_bitrates(&m.step(Input::Uplink(wired(100)))).is_empty());
    assert_eq!(m.state().engines[&id].settings["bitrate-mbps"], Value::Int(85), "it still starts at the cap when the app comes back");
}

#[test]
fn an_engine_that_becomes_ready_after_the_uplink_was_read_starts_at_the_cap() {
    let mut m = fixture();
    let id = EngineId::new("gfn").unwrap();
    m.prefs.bitrate_mbps = 150;
    m.step(Input::Uplink(wired(100)));
    let descriptor = m.integrations.iter_mut().find(|e| e.manifest.id == id).unwrap().descriptor.take().unwrap();
    let ready = FromEngine { re: None, event: ipc::Event::Ready { descriptor, schema: bitrate_schema() } };
    assert_eq!(sent_bitrates(&m.step(Input::Engine { engine: id, message: Box::new(ready) })), [Value::Int(85)]);
}
