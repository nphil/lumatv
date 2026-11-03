//! The refresh mode (`refresh.rs`, `ctl refresh`) on a fake KMS: an output that keeps a `FlipTiming` fed with synthetic page-flip
//! stamps and a divider file in a temp dir, as the KMS output does with the card's. The switch to 120 writes the divider, keys the
//! pacer to 8.33 ms, halves the tick and tells the shell; the measure reads the flips at the period in force; 60 puts everything back.

mod harness;

use harness::Server;
use lumatv_compositor::kms::timing::{FlipTiming, Timing};
use lumatv_compositor::output::{Image, Output, Presented, RenderStats};
use lumatv_compositor::refresh::{self, Divider};
use lumatv_compositor::render::software::Software;
use lumatv_compositor::scene::Plan;
use lumatv_compositor::surface::Surfaces;
use parking_lot::Mutex;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use wayland_server::protocol::wl_buffer::WlBuffer;

/// The software output with the KMS output's refresh bookkeeping.
struct FakeKms {
    inner: Software,
    timing: Arc<Mutex<FlipTiming>>,
    divider: Divider,
    hz: u32,
}

impl Output for FakeKms {
    fn name(&self) -> &'static str {
        "fake-kms"
    }
    fn size(&self) -> (u32, u32) {
        self.inner.size()
    }
    fn query_buffer(&mut self, buffer: &WlBuffer) -> Option<((i32, i32), bool)> {
        self.inner.query_buffer(buffer)
    }
    fn forget_buffer(&mut self, buffer: &WlBuffer) {
        self.inner.forget_buffer(buffer)
    }
    fn render(&mut self, plan: &Plan, surfaces: &Surfaces) -> std::io::Result<RenderStats> {
        self.inner.render(plan, surfaces)
    }
    fn present(&mut self) -> std::io::Result<Presented> {
        self.inner.present()
    }
    fn capture(&mut self) -> std::io::Result<Image> {
        self.inner.capture()
    }
    fn set_refresh(&mut self, hz: u32) -> std::io::Result<u32> {
        self.divider.set(hz)?;
        self.timing.lock().rekey(refresh::period_ns(hz));
        self.hz = hz;
        Ok(hz)
    }
    fn reset_refresh_measure(&mut self) {
        self.timing.lock().measure = Default::default();
    }
    fn refresh_report(&self) -> Option<Value> {
        let t = self.timing.lock();
        let mut r = t.measure.report(t.period_ns());
        r["hz"] = self.hz.into();
        r["divider"] = self.divider.report();
        Some(r)
    }
}

fn divider_file(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("lumatv-refresh-host-{}-{name}", std::process::id()));
    std::fs::write(&p, "2\n").unwrap();
    p
}

fn fake(path: &std::path::Path) -> (Server, Arc<Mutex<FlipTiming>>) {
    let timing = Arc::new(Mutex::new(FlipTiming::new(refresh::period_ns(60))));
    let (t2, p2) = (timing.clone(), path.to_path_buf());
    let server = Server::start_with(Box::new(move || Box::new(FakeKms { inner: Software::new(1920, 1080), timing: t2, divider: Divider::at(&p2), hz: 60 })));
    (server, timing)
}

fn ctl(server: &Server, line: &'static str) -> Value {
    serde_json::from_str(&server.eval(move |s| lumatv_compositor::control::handle(s, line).text())).unwrap()
}

/// `n` flips `period` apart from `from`, each of a frame costing `pipeline`.
fn flips(timing: &Mutex<FlipTiming>, from: u64, period: u64, n: u64, pipeline: u64) {
    let mut t = timing.lock();
    for i in 0..n {
        let at = from + i * period;
        t.landed(at, Some((Timing { slot_ns: at, started_ns: at - pipeline - 500_000, ready_ns: at - 500_000, content_ns: None }, None)));
    }
}

#[test]
fn the_switch_writes_the_divider_keys_the_pacer_halves_the_tick_and_tells_the_shell() {
    let path = divider_file("switch");
    let (server, timing) = fake(&path);
    let p60 = refresh::period_ns(60);
    let p120 = refresh::period_ns(120);
    assert_eq!(server.eval(|s| (s.config.refresh_hz, s.config.tick_ns)), (60, p60));
    let before = ctl(&server, "refresh");
    assert_eq!(before["hz"], 60);
    assert_eq!(before["measure"]["divider"]["value"], 2);

    let on = ctl(&server, "refresh 120");
    assert_eq!(on["ok"], true, "{on}");
    assert_eq!(on["hz"], 120);
    assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "1", "the OSD's vblank divider at 1");
    assert_eq!(server.eval(|s| (s.config.refresh_hz, s.config.tick_ns)), (120, p120), "the fixed tick at the panel's rate");
    assert_eq!(timing.lock().period_ns(), p120, "the pacer keyed to 8.33 ms");
    let topic = server.eval(|s| {
        s.shell_rust_open(0, "test".into()).unwrap();
        s.shell_sync_topics();
        s.shell_rust.topic("refresh").cloned()
    });
    assert_eq!(topic, Some(serde_json::json!({"hz": 120})));
    let journal = server.eval(|s| lumatv_compositor::control::handle(s, "journal").text());
    assert!(journal.contains("refresh: 120 Hz (control socket)"), "{journal}");

    // The measure since the switch: 4 ms frames landing on every 120 Hz refresh, then the shell's 15 ms frames every other one.
    flips(&timing, 10_000_000_000, p120, 13, 4_000_000);
    flips(&timing, 10_000_000_000 + 12 * p120 + 2 * p120, 2 * p120, 6, 15_000_000);
    let r = ctl(&server, "refresh");
    let m = &r["measure"];
    assert_eq!(m["flips"], 19);
    assert_eq!(m["intervals"]["one"], 12);
    assert_eq!(m["intervals"]["two"], 6);
    assert_eq!(m["pipeline_ms"]["max"], 15.0);
    assert_eq!(m["pipeline_ms"]["p50"], 4.25, "the 4 ms frames are the median");
    assert!((m["period_ms"].as_f64().unwrap() - 8.333).abs() < 0.01);

    // Back to 60: the divider as found, the pacer by order (the measured path would read 16.67 ms as a skipped 120 Hz refresh), the tick, the topic.
    let off = ctl(&server, "refresh 60");
    assert_eq!(off["ok"], true, "{off}");
    assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "2");
    assert_eq!(server.eval(|s| (s.config.refresh_hz, s.config.tick_ns)), (60, p60));
    assert_eq!(timing.lock().period_ns(), p60);
    assert_eq!(ctl(&server, "refresh")["measure"]["flips"], 0, "the measure starts over at the switch");
    let topic = server.eval(|s| {
        s.shell_sync_topics();
        s.shell_rust.topic("refresh").cloned()
    });
    assert_eq!(topic, Some(serde_json::json!({"hz": 60})));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_refused_switch_changes_nothing() {
    let path = std::env::temp_dir().join(format!("lumatv-refresh-host-{}-absent", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let (server, timing) = fake(&path);
    let r = ctl(&server, "refresh 120");
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["hz"], 60);
    assert_eq!(server.eval(|s| (s.config.refresh_hz, s.config.tick_ns)), (60, refresh::period_ns(60)));
    assert_eq!(timing.lock().period_ns(), refresh::period_ns(60));
    let r = ctl(&server, "refresh 90");
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(ctl(&server, "refresh")["hz"], 60);
}

#[test]
fn an_output_without_a_panel_takes_the_number_only() {
    let server = Server::start();
    let r = ctl(&server, "refresh 120");
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["measure"], Value::Null);
    assert_eq!(server.eval(|s| s.config.tick_ns), refresh::period_ns(120));
    ctl(&server, "refresh 60");
    assert_eq!(server.eval(|s| s.config.tick_ns), refresh::period_ns(60));
}

#[test]
fn foreground_policy_tracks_cards_manual_override_and_commit_callbacks() {
    use harness::Client;
    use lumatv_compositor::server::state::ClientCallbacks;
    use std::time::Duration;
    let path = divider_file("foreground");
    let (mut server, timing) = fake(&path);
    server.eval(|s| { s.client_callbacks = ClientCallbacks::Commit; s.shell_rust_open(0, "test".into()).unwrap(); });
    assert_eq!(ctl(&server, "refresh")["mode"], "auto");
    let mut client = Client::connect(&mut server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "Marquee");
    card.webos.set_property("appId".into(), "com.beb.plxnative".into());
    let buffer = client.shm_buffer(64, 64, 0xff123456);
    client.present(&card.surface, &buffer);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1));
    assert_eq!(ctl(&server, "refresh")["hz"], 120);
    assert_eq!(timing.lock().period_ns(), refresh::period_ns(120));
    assert_eq!(server.eval(|s| s.shell_rust.topic("refresh").cloned()), Some(serde_json::json!({"hz": 120})));
    flips(&timing, 10_000_000_000, refresh::period_ns(120), 3, 1_000_000);
    server.eval(|s| { s.refresh_policy_sync(); s.refresh_policy_sync(); });
    assert_eq!(ctl(&server, "refresh")["measure"]["flips"], 3, "same app does not reset timing");
    assert_eq!(ctl(&server, "refresh 60")["mode"], "manual");
    flips(&timing, 20_000_000_000, refresh::period_ns(60), 3, 1_000_000);
    assert_eq!(ctl(&server, "refresh")["measure"]["flips"], 3);
    assert_eq!(ctl(&server, "refresh reset")["mode"], "manual");
    assert_eq!(ctl(&server, "refresh")["measure"]["flips"], 0);
    assert_eq!(timing.lock().period_ns(), refresh::period_ns(60));
    assert_eq!(ctl(&server, "refresh")["hz"], 60);
    card.webos.set_property("appId".into(), "other".into());
    client.roundtrip();
    card.webos.set_property("appId".into(), "com.beb.plxnative".into());
    client.roundtrip();
    assert_eq!(ctl(&server, "refresh")["hz"], 60);
    client.present(&card.surface, &buffer);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 2));
    assert!(server.eval(|s| s.stats.callbacks_at_commit >= 2));
    assert_eq!(ctl(&server, "refresh auto")["hz"], 120);
    assert_eq!(ctl(&server, "refresh")["mode"], "auto");
    card.webos.set_property("appId".into(), "com.webos.app.home".into());
    client.roundtrip();
    server.eval(|s| s.luna_notify());
    assert_eq!(ctl(&server, "refresh")["hz"], 60);
    card.webos.set_property("appId".into(), "other".into());
    client.roundtrip();
    server.eval(|s| s.luna_notify());
    assert_eq!(ctl(&server, "refresh")["hz"], 60);
    card.webos.set_property("appId".into(), "com.beb.plxnative".into());
    client.roundtrip();
    assert_eq!(ctl(&server, "refresh")["hz"], 120);
    card.surface.destroy();
    client.roundtrip();
    assert_eq!(ctl(&server, "refresh")["hz"], 60);
    drop(server);
    let _ = std::fs::remove_file(path);
}

#[test]
fn shell_property_and_state_requests_reselect_the_foreground_refresh() {
    use harness::Client;
    use std::time::Duration;
    let path = divider_file("eligibility");
    let (mut server, timing) = fake(&path);
    let mut client = Client::connect(&mut server);
    let window = client.card("_WEBOS_WINDOW_TYPE_OVERLAY", "Marquee");
    window.webos.set_property("appId".into(), "com.beb.plxnative".into());
    let buffer = client.shm_buffer(64, 64, 0xff123456);
    client.present(&window.surface, &buffer);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1));
    client.roundtrip();
    assert_eq!(ctl(&server, "refresh")["hz"], 60);
    window.webos.set_property("_WEBOS_WINDOW_TYPE".into(), "_WEBOS_WINDOW_TYPE_CARD".into());
    client.roundtrip();
    assert_eq!(ctl(&server, "refresh")["hz"], 120);
    assert_eq!(timing.lock().period_ns(), refresh::period_ns(120));
    window.webos.set_property("_WEBOS_WINDOW_TYPE".into(), "_WEBOS_WINDOW_TYPE_OVERLAY".into());
    client.roundtrip();
    assert_eq!(ctl(&server, "refresh")["hz"], 60);
    window.webos.set_property("_WEBOS_WINDOW_TYPE".into(), "_WEBOS_WINDOW_TYPE_CARD".into());
    client.roundtrip();
    assert_eq!(ctl(&server, "refresh")["hz"], 120);
    for restored in [2, 0] {
        window.webos.set_state(1);
        client.roundtrip();
        assert_eq!(ctl(&server, "refresh")["hz"], 60);
        window.webos.set_state(restored);
        client.roundtrip();
        assert_eq!(ctl(&server, "refresh")["hz"], 120, "restored state {restored}");
    }
    drop(server);
    let _ = std::fs::remove_file(path);
}

#[test]
fn legacy_title_relayer_reselects_foreground_refresh() {
    use harness::Client;
    use std::time::Duration;
    let path = divider_file("legacy-title");
    let (mut server, timing) = fake(&path);
    let mut client = Client::connect(&mut server);
    let window = client.card("_WEBOS_WINDOW_TYPE_CARD", "Marquee");
    window.webos.set_property("appId".into(), "com.beb.plxnative".into());
    let buffer = client.shm_buffer(64, 64, 0xff123456);
    client.present(&window.surface, &buffer);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1));
    client.roundtrip();
    assert_eq!(ctl(&server, "refresh")["hz"], 120);
    window.shell_surface.set_title("LumaTV shell: home".into());
    client.roundtrip();
    assert_eq!(ctl(&server, "refresh")["hz"], 60);
    assert_eq!(timing.lock().period_ns(), refresh::period_ns(60));
    window.surface.destroy();
    client.roundtrip();
    assert_eq!(ctl(&server, "refresh")["hz"], 60);
    drop(server);
    let _ = std::fs::remove_file(path);
}

#[test]
fn input_panel_relayer_reselects_foreground_refresh() {
    use harness::Client;
    use std::time::Duration;
    let path = divider_file("input-panel");
    let (mut server, _) = fake(&path);
    let mut client = Client::connect(&mut server);
    let window = client.card("_WEBOS_WINDOW_TYPE_CARD", "Marquee");
    window.webos.set_property("appId".into(), "com.beb.plxnative".into());
    let buffer = client.shm_buffer(64, 64, 0xff123456);
    client.present(&window.surface, &buffer);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1));
    client.roundtrip();
    assert_eq!(ctl(&server, "refresh")["hz"], 120);
    let panel = client.input_panel();
    let _panel_surface = panel.get_input_panel_surface(&window.surface, &client.qh, ());
    client.roundtrip();
    assert_eq!(ctl(&server, "refresh")["hz"], 60);
    drop(server);
    let _ = std::fs::remove_file(path);
}

#[test]
fn refusal_retries_on_a_later_sync_and_auto_failure_preserves_manual() {
    use harness::Client;
    use std::time::Duration;
    let path = std::env::temp_dir().join(format!("lumatv-refresh-host-{}-retry", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let (mut server, timing) = fake(&path);
    server.eval(|s| { s.shell_rust_open(0, "test".into()).unwrap(); s.shell_sync_topics(); });
    let mut client = Client::connect(&mut server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "Marquee");
    card.webos.set_property("appId".into(), "com.beb.plxnative".into());
    let buffer = client.shm_buffer(64, 64, 0xff123456);
    client.present(&card.surface, &buffer);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1));
    assert_eq!(ctl(&server, "refresh")["hz"], 60);
    assert_eq!(timing.lock().period_ns(), refresh::period_ns(60));
    assert_eq!(server.eval(|s| (s.config.tick_ns, s.shell_rust.topic("refresh").cloned())), (refresh::period_ns(60), Some(serde_json::json!({"hz": 60}))));
    std::fs::write(&path, "2\n").unwrap();
    server.eval(|s| s.refresh_policy_sync());
    assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "1");
    assert_eq!(ctl(&server, "refresh")["hz"], 120);
    assert_eq!(ctl(&server, "refresh 60")["mode"], "manual");
    std::fs::remove_file(&path).unwrap();
    assert_eq!(ctl(&server, "refresh auto")["ok"], false);
    let report = ctl(&server, "refresh");
    assert_eq!((report["hz"].as_u64(), report["mode"].as_str()), (Some(60), Some("manual")));
    assert_eq!(ctl(&server, "refresh 120")["ok"], false);
    assert_eq!(ctl(&server, "refresh")["mode"], "manual");
}

#[test]
fn configured_startup_refresh_remains_automatic_fallback() {
    use lumatv_compositor::{config::Config, luna, server::state::State};
    let display = wayland_server::Display::<State>::new().unwrap();
    let config = Config::parse(["--refresh", "120"].map(String::from)).unwrap();
    let mut state = State::new(display.handle(), config, Box::new(Software::new(64, 64)), Box::new(luna::Null::default()));
    assert_eq!(state.refresh_policy.default_hz, 120);
    assert!(!state.refresh_policy.manual);
    state.shell_sync_topics();
    assert_eq!(state.config.refresh_hz, 120);
    state.refresh_apply(60).unwrap();
    state.refresh_policy.manual = true;
    assert_eq!(state.refresh_auto().unwrap(), 120);
}

#[test]
fn interactive_ui_and_marquee_share_one_automatic_vote() {
    use harness::Client;
    use lumatv_compositor::shell::Msg;
    use serde_json::json;
    use std::time::Duration;

    let path = divider_file("interactive");
    let (mut server, timing) = fake(&path);
    let mut client = Client::connect(&mut server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "Other");
    card.webos.set_property("appId".into(), "other".into());
    let buffer = client.shm_buffer(64, 64, 0xff123456);
    client.present(&card.surface, &buffer);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1));

    let report = |state| Msg::Ui { can: None, wants: None, live: None, state };
    server.eval(move |s| { s.shell_rust_handle(report(Some(json!({"interactiveVisible": true})))); });
    assert_eq!(ctl(&server, "refresh")["hz"], 60, "no engine");
    server.eval(|s| { s.shell_rust.engine_connected = true; s.shell_rust.connected = false; s.refresh_policy_sync(); });
    assert_eq!(ctl(&server, "refresh")["hz"], 60, "no HTTP connection");
    server.eval(|s| { s.shell_rust.connected = true; s.luna_state.power.rendering = false; s.refresh_policy_sync(); });
    assert_eq!(ctl(&server, "refresh")["hz"], 60, "screen off");
    server.eval(|s| { s.luna_state.power.rendering = true; s.refresh_policy_sync(); });
    assert_eq!(ctl(&server, "refresh")["hz"], 120, "overlay over another app");

    flips(&timing, 10_000_000_000, refresh::period_ns(120), 3, 1_000_000);
    card.webos.set_property("appId".into(), "com.beb.plxnative".into());
    client.roundtrip();
    server.eval(move |s| { s.shell_rust_handle(report(Some(json!({"interactiveVisible": false})))); });
    assert_eq!(ctl(&server, "refresh")["measure"]["flips"], 3, "overlapping votes do not rekey");
    assert_eq!(ctl(&server, "refresh 60")["mode"], "manual");
    server.eval(move |s| { s.shell_rust_handle(report(Some(json!({"interactiveVisible": true})))); });
    card.webos.set_property("appId".into(), "other".into());
    client.roundtrip();
    assert_eq!(ctl(&server, "refresh")["hz"], 60, "manual remains sticky");
    assert_eq!(ctl(&server, "refresh reset")["mode"], "manual");
    assert_eq!(ctl(&server, "refresh auto")["hz"], 120, "auto resumes current UI vote");
    server.eval(move |s| { s.shell_rust_handle(report(Some(json!({"interactiveVisible": "true"})))); });
    assert_eq!(ctl(&server, "refresh")["hz"], 60, "wrong type is false");
    server.eval(move |s| { s.shell_rust_handle(report(Some(json!({"interactiveVisible": true})))); });
    server.eval(move |s| { s.shell_rust_handle(report(None)); });
    assert_eq!(ctl(&server, "refresh")["hz"], 60, "missing state closes the vote");
    server.eval(move |s| { s.shell_rust_handle(report(Some(json!({"interactiveVisible": true})))); });
    server.eval(move |s| { s.shell_rust_handle(report(Some(json!({})))); });
    assert_eq!(ctl(&server, "refresh")["hz"], 60, "missing field closes the vote");
    drop(server);
    let _ = std::fs::remove_file(path);
}
