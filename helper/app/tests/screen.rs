//! The screen watcher against a fake `luna-send` (a shell script that says what the power state is).

mod support;

use lumatv_helper::config::{Config, Paths};
use lumatv_helper::control::Stop;
use lumatv_helper::ctx::Ctx;
use lumatv_helper::hub::Hub;
use lumatv_helper::screen;
use lumatv_helper::settings::SettingsStore;
use lumatv_helper::sys;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use support::*;

const WAIT: Duration = Duration::from_secs(10);

fn ctx(dir: &Path, luna: &str, poll_ms: u64) -> Ctx {
    let config = Config::parse(
        &json!({"screen": {"source": "luna", "pollMs": poll_ms}, "luna": {"send": luna}})
            .to_string(),
    )
    .unwrap();
    let hub = Arc::new(Hub::new(1));
    Ctx {
        config: Arc::new(config),
        paths: Paths::new(dir),
        settings: Arc::new(SettingsStore::new(dir.join("settings"), hub.clone())),
        hub,
        stop: Stop::new(),
    }
}

fn topic(ctx: &Ctx) -> Option<Value> {
    ctx.hub.get(screen::TOPIC)
}

fn wait_screen(ctx: &Ctx, check: impl Fn(&Value) -> bool) -> Option<Value> {
    let mut last = None;
    wait_until(WAIT, || {
        last = topic(ctx);
        last.as_ref().is_some_and(&check)
    })
    .then_some(last)
    .flatten()
}

#[test]
fn a_luna_send_that_cannot_subscribe_is_polled() {
    let dir = tempfile::tempdir().unwrap();
    let luna = FakeLuna::new(dir.path(), LunaMode::PollOnly, "Active");
    let ctx = ctx(dir.path(), luna.program.to_str().unwrap(), 30);
    let changes = Arc::new(AtomicUsize::new(0));
    let notify = {
        let changes = changes.clone();
        Arc::new(move || {
            changes.fetch_add(1, Ordering::SeqCst);
        })
    };
    let handle = screen::spawn(&ctx, notify).unwrap().expect("a thread");
    let on =
        wait_screen(&ctx, |v| v["on"] == true && v["source"] == "luna-poll").expect("on by poll");
    assert_eq!(on["state"], "Active");
    assert_eq!(on["error"], Value::Null);
    let since = on["since"].as_u64().unwrap();

    luna.set_state("Active Standby");
    let off = wait_screen(&ctx, |v| v["on"] == false).expect("off");
    assert_eq!(off["state"], "Active Standby");
    assert!(off["since"].as_u64().unwrap() >= since);
    luna.set_state("Screen Saver");
    let saver = wait_screen(&ctx, |v| v["state"] == "Screen Saver").expect("screen saver");
    assert_eq!(saver["on"], true, "the panel is lit during the saver");
    assert_eq!(changes.load(Ordering::SeqCst), 3);

    ctx.stop.request();
    handle.join().unwrap();
}

#[test]
fn a_subscription_pushes_changes_without_polling() {
    let dir = tempfile::tempdir().unwrap();
    let luna = FakeLuna::new(dir.path(), LunaMode::Subscribes, "Active");
    // A poll period of a minute: anything seen within the test came through the subscription.
    let ctx = ctx(dir.path(), luna.program.to_str().unwrap(), 60_000);
    let handle = screen::spawn(&ctx, Arc::new(|| {}))
        .unwrap()
        .expect("a thread");
    wait_screen(&ctx, |v| v["on"] == true && v["source"] == "luna-subscribe")
        .expect("on by subscription");
    luna.set_state("Active Standby");
    let off = wait_screen(&ctx, |v| v["on"] == false).expect("off, pushed");
    assert_eq!(off["source"], "luna-subscribe");
    luna.set_state("Active");
    wait_screen(&ctx, |v| v["on"] == true).expect("on again, pushed");
    ctx.stop.request();
    handle.join().unwrap();
}

#[test]
fn stopping_ends_the_subscription_child() {
    let dir = tempfile::tempdir().unwrap();
    let luna = FakeLuna::new(dir.path(), LunaMode::Subscribes, "Active");
    let ctx = ctx(dir.path(), luna.program.to_str().unwrap(), 60_000);
    let handle = screen::spawn(&ctx, Arc::new(|| {}))
        .unwrap()
        .expect("a thread");
    wait_screen(&ctx, |v| v["source"] == "luna-subscribe").expect("subscribed");
    let pid: u32 = std::fs::read_to_string(&luna.pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(sys::pid_alive(pid));
    ctx.stop.request();
    handle.join().unwrap();
    assert!(
        wait_until(Duration::from_secs(3), || !sys::pid_alive(pid)),
        "the luna-send child was left running"
    );
}

#[test]
fn a_silent_subscription_is_dropped_for_the_poll() {
    let dir = tempfile::tempdir().unwrap();
    let luna = FakeLuna::new(dir.path(), LunaMode::SilentSubscription, "Active");
    let ctx = ctx(dir.path(), luna.program.to_str().unwrap(), 30);
    let handle = screen::spawn(&ctx, Arc::new(|| {}))
        .unwrap()
        .expect("a thread");
    // The subscription gets four seconds to say something; then the poll takes over.
    let on = wait_screen(&ctx, |v| v["on"] == true).expect("on by poll");
    assert_eq!(on["source"], "luna-poll");
    let pid: u32 = std::fs::read_to_string(&luna.pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        wait_until(Duration::from_secs(3), || !sys::pid_alive(pid)),
        "the silent child was left running"
    );
    luna.set_state("Active Standby");
    wait_screen(&ctx, |v| v["on"] == false).expect("off by poll");
    ctx.stop.request();
    handle.join().unwrap();
}

#[test]
fn when_the_power_state_cannot_be_read_the_screen_is_assumed_on_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ctx(dir.path(), "/nonexistent/luna-send", 20);
    let handle = screen::spawn(&ctx, Arc::new(|| {}))
        .unwrap()
        .expect("a thread");
    let assumed = wait_screen(&ctx, |v| v["source"] == "assumed").expect("assumed");
    assert_eq!(assumed["on"], true);
    assert!(
        assumed["error"].as_str().unwrap().contains("luna-send"),
        "{assumed}"
    );
    assert!(screen::is_on(&ctx.hub));
    ctx.stop.request();
    handle.join().unwrap();
}

#[test]
fn a_garbled_answer_is_not_a_reading() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("luna-send");
    std::fs::write(
        &script,
        "#!/bin/sh\nif [ \"$2\" != 1 ]; then exit 0; fi\necho 'Unable to connect to the bus'\n",
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let ctx = ctx(dir.path(), script.to_str().unwrap(), 20);
    let handle = screen::spawn(&ctx, Arc::new(|| {}))
        .unwrap()
        .expect("a thread");
    let assumed = wait_screen(&ctx, |v| v["source"] == "assumed").expect("assumed");
    assert!(
        assumed["error"]
            .as_str()
            .unwrap()
            .contains("no power state")
    );
    ctx.stop.request();
    handle.join().unwrap();
}

#[test]
fn the_config_source_is_always_on_and_runs_no_thread() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(dir.path(), "luna-send", 30);
    ctx.config = Arc::new(Config::parse(r#"{"screen":{"source":"on"}}"#).unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let notify = {
        let calls = calls.clone();
        Arc::new(move || {
            calls.fetch_add(1, Ordering::SeqCst);
        })
    };
    assert!(screen::spawn(&ctx, notify).unwrap().is_none());
    let value = topic(&ctx).unwrap();
    assert_eq!(
        (value["on"].clone(), value["source"].clone()),
        (json!(true), json!("config"))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
