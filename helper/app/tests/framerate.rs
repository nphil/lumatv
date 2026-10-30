//! The frame-rate watcher against a fake `luna-send` (SAM's foreground app, from a file) and a fake panel: the status files in a
//! temporary directory, the vsync count advanced by a thread at the rate the "display engine" runs, 100 Hz until `1 60` is written.

mod support;

use lumatv_helper::config::{Config, Paths};
use lumatv_helper::control::Stop;
use lumatv_helper::ctx::Ctx;
use lumatv_helper::framerate::{self, Cmd, HOME, Sources};
use lumatv_helper::hub::Hub;
use lumatv_helper::settings::SettingsStore;
use parking_lot::Mutex;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use support::*;

const WAIT: Duration = Duration::from_secs(10);
/// Long enough for a check to have measured and written if it was going to (a measurement is 0.5 s, a step 0.25 s).
const QUIET: Duration = Duration::from_millis(1500);
const SCALER: &str = "[main.p0]\nopen=1\nconnect_src=SRC\nconnect_index=0\nmute=0\nhdr=0\n";

/// Replaces a file in one step, so a reader never sees half of it.
fn put(path: &Path, text: &str) {
    let tmp = path.with_extension("new");
    std::fs::write(&tmp, text).unwrap();
    std::fs::rename(&tmp, path).unwrap();
}

/// The set as the watcher sees it.
struct FakeSet {
    sources: Sources,
    foreground: PathBuf,
    program: PathBuf,
    pid_file: PathBuf,
    hz: Arc<Mutex<f64>>,
    /// Every `1 60` the watcher wrote, with whether a source was connected at that moment.
    writes: Arc<Mutex<Vec<bool>>>,
    done: Arc<AtomicBool>,
    engine: Option<JoinHandle<()>>,
}

impl FakeSet {
    fn new(dir: &Path) -> Self {
        let sources = Sources {
            scaler: dir.join("main.p0"),
            interrupts: dir.join("interrupts"),
            fr_rate: dir.join("fr_rate"),
        };
        let foreground = dir.join("foreground");
        let pid_file = dir.join("luna-pid");
        put(&foreground, "netflix");
        put(&sources.scaler, &SCALER.replace("SRC", "VDEC"));
        put(&sources.interrupts, &interrupts(1_000_000));
        std::fs::write(&sources.fr_rate, "").unwrap();
        // SAM's foreground subscription: the app now, and again whenever the file changes.
        let program = dir.join("luna-send");
        let script = "#!/bin/sh\ncase \"$1 $2\" in\n  \"-n 1000000\") echo $$ > PIDFILE; last=''; while :; do now=$(cat FGFILE); if [ \"$now\" != \"$last\" ]; then printf '{\"subscribed\":true,\"returnValue\":true,\"appId\":\"%s\",\"processId\":\"\",\"windowId\":\"\"}\\n' \"$now\"; last=$now; fi; sleep 0.02; done ;;\n  *) exit 1 ;;\nesac\n"
            .replace("PIDFILE", &pid_file.display().to_string())
            .replace("FGFILE", &foreground.display().to_string());
        std::fs::write(&program, script).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        settle(&program);
        let hz = Arc::new(Mutex::new(100.0));
        let writes = Arc::new(Mutex::new(Vec::new()));
        let done = Arc::new(AtomicBool::new(false));
        // The display engine: the count runs at `hz`; a write of `1 60` is taken (and recorded) and the rate becomes 120 Hz.
        let engine = {
            let (sources, hz, writes, done) =
                (sources.clone(), hz.clone(), writes.clone(), done.clone());
            thread::spawn(move || {
                let mut count = 1_000_000.0_f64;
                let mut last = Instant::now();
                while !done.load(Ordering::SeqCst) {
                    thread::sleep(Duration::from_millis(10));
                    let now = Instant::now();
                    count += *hz.lock() * (now - last).as_secs_f64();
                    last = now;
                    put(&sources.interrupts, &interrupts(count as u64));
                    let request = std::fs::read_to_string(&sources.fr_rate).unwrap_or_default();
                    if !request.is_empty() {
                        assert_eq!(request, "1 60\n");
                        let scaler = std::fs::read_to_string(&sources.scaler).unwrap();
                        writes.lock().push(!scaler.contains("connect_src=NONE"));
                        std::fs::write(&sources.fr_rate, "").unwrap();
                        *hz.lock() = 120.0;
                    }
                }
            })
        };
        Self {
            sources,
            foreground,
            program,
            pid_file,
            hz,
            writes,
            done,
            engine: Some(engine),
        }
    }

    fn app(&self, id: &str) {
        put(&self.foreground, id);
    }

    fn window(&self, src: &str) {
        put(&self.sources.scaler, &SCALER.replace("SRC", src));
    }

    fn rate(&self, hz: f64) {
        *self.hz.lock() = hz;
    }

    fn writes(&self) -> Vec<bool> {
        self.writes.lock().clone()
    }
}

impl Drop for FakeSet {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        if let Some(engine) = self.engine.take() {
            let _ = engine.join();
        }
    }
}

fn interrupts(count: u64) -> String {
    format!(
        "           CPU0       CPU1       CPU2       CPU3\n104: {count:>10}          0          0          0     GICv2 104 Edge      osd_irq\n175:    4821871          0          0          0     GICv2 207 Level     DE2VSYNC\n"
    )
}

fn ctx(dir: &Path, luna: &Path, extra: serde_json::Value) -> Ctx {
    let mut config = json!({"luna": {"send": luna}});
    if let (Some(base), Some(more)) = (config.as_object_mut(), extra.as_object()) {
        base.extend(more.clone());
    }
    let hub = Arc::new(Hub::new(1));
    hub.set("screen", json!({"on": true}));
    Ctx {
        config: Arc::new(Config::parse(&config.to_string()).unwrap()),
        paths: Paths::new(dir),
        settings: Arc::new(SettingsStore::new(dir.join("settings"), hub.clone())),
        hub,
        stop: Stop::new(),
    }
}

#[test]
fn the_rate_is_set_back_on_home_only_with_no_window_and_never_inside_an_app() {
    let dir = tempfile::tempdir().unwrap();
    let set = FakeSet::new(dir.path());
    let ctx = ctx(dir.path(), &set.program, json!({}));
    let (tx, rx) = mpsc::channel();
    let handle = framerate::spawn_with(&ctx, set.sources.clone(), tx.clone(), rx)
        .unwrap()
        .expect("a thread");

    // A 50 fps video in an app, then its window closes with the app still in front (round 11's YouTube): left to LG.
    thread::sleep(QUIET);
    set.window("NONE");
    thread::sleep(QUIET);
    assert!(set.writes().is_empty(), "nothing is written inside an app");

    // Home in front, no window, 100 Hz: set back once.
    set.app(HOME);
    assert!(
        wait_until(WAIT, || set.writes().len() == 1),
        "{:?}",
        set.writes()
    );
    thread::sleep(QUIET);
    assert_eq!(
        set.writes(),
        [false],
        "written once, with no source connected"
    );

    // Home again while the app's window is still open (a backgrounded player): nothing until it closes.
    set.app("youtube.leanback.v4");
    set.window("VDEC");
    set.rate(100.0);
    thread::sleep(Duration::from_millis(300));
    set.app(HOME);
    thread::sleep(QUIET);
    assert_eq!(
        set.writes().len(),
        1,
        "nothing is written while a source is connected"
    );
    set.window("NONE");
    assert!(
        wait_until(WAIT, || set.writes().len() == 2),
        "{:?}",
        set.writes()
    );
    assert_eq!(set.writes(), [false, false]);

    // With the screen off nothing is looked at; the screen coming on with Home in front is a check.
    ctx.hub.set("screen", json!({"on": false}));
    tx.send(Cmd::Wake).unwrap();
    set.app("netflix");
    set.rate(100.0);
    thread::sleep(Duration::from_millis(300));
    set.app(HOME);
    thread::sleep(QUIET);
    assert_eq!(set.writes().len(), 2, "nothing while the screen is off");
    ctx.hub.set("screen", json!({"on": true}));
    tx.send(Cmd::Wake).unwrap();
    assert!(
        wait_until(WAIT, || set.writes().len() == 3),
        "{:?}",
        set.writes()
    );

    // Stopping ends the thread and the subscription's child.
    let pid = std::fs::read_to_string(&set.pid_file)
        .unwrap()
        .trim()
        .to_owned();
    ctx.stop.request();
    tx.send(Cmd::Stop).unwrap();
    let stopped = Instant::now();
    handle.join().unwrap();
    assert!(stopped.elapsed() < Duration::from_secs(5));
    assert!(
        wait_until(WAIT, || !Path::new(&format!("/proc/{pid}")).exists()),
        "the luna-send child is gone"
    );
}

#[test]
fn the_watcher_is_off_when_the_config_says_so_or_the_set_has_no_lever() {
    let dir = tempfile::tempdir().unwrap();
    let set = FakeSet::new(dir.path());
    let off = ctx(
        dir.path(),
        &set.program,
        json!({"frameRate": {"returnOnHome": false}}),
    );
    let (tx, rx) = mpsc::channel();
    assert!(
        framerate::spawn_with(&off, set.sources.clone(), tx, rx)
            .unwrap()
            .is_none()
    );

    let on = ctx(dir.path(), &set.program, json!({}));
    let mut sources = set.sources.clone();
    sources.fr_rate = dir.path().join("no-such-file");
    let (tx, rx) = mpsc::channel();
    assert!(
        framerate::spawn_with(&on, sources, tx, rx)
            .unwrap()
            .is_none()
    );
    assert!(
        Config::parse(r#"{"frameRate": {"returnOnHome": "yes"}}"#).is_err(),
        "a bad value stops the start"
    );
}

#[test]
fn a_camera_stream_over_home_is_put_back_while_it_plays_and_its_end_is_a_check_too() {
    let dir = tempfile::tempdir().unwrap();
    let set = FakeSet::new(dir.path());
    let ctx = ctx(dir.path(), &set.program, json!({}));
    let (tx, rx) = mpsc::channel();
    let handle = framerate::spawn_with(&ctx, set.sources.clone(), tx.clone(), rx)
        .unwrap()
        .expect("a thread");

    // Home in front with the panel already at 120 Hz: the arrival check finds nothing to do.
    set.window("NONE");
    set.rate(120.0);
    set.app(HOME);
    thread::sleep(QUIET);
    assert!(set.writes().is_empty());

    // A pop-up's 25 fps stream re-times the panel at its first frame; Home never leaves the front. Once it plays, the rate is written with the
    // source still connected (the firmware took no request without one on the set).
    set.window("VDEC");
    set.rate(100.0);
    ctx.hub.set("cameras.live", json!({"state": "playing"}));
    assert!(wait_until(WAIT, || set.writes() == [true]), "{:?}", set.writes());
    thread::sleep(QUIET);
    assert_eq!(set.writes(), [true], "written once");

    // The stream ends and its window closes at 120 Hz: nothing is left to do.
    set.window("NONE");
    ctx.hub.set("cameras.live", json!({"state": "idle"}));
    thread::sleep(QUIET);
    assert_eq!(set.writes(), [true]);

    // A stream that ended with the panel left at 100 Hz (its start was not seen) is put back once its window is closed.
    set.rate(100.0);
    ctx.hub.set("cameras.live", json!({"state": "asking"}));
    ctx.hub.set("cameras.live", json!({"state": "failed"}));
    assert!(wait_until(WAIT, || set.writes() == [true, false]), "{:?}", set.writes());

    ctx.stop.request();
    tx.send(Cmd::Stop).unwrap();
    handle.join().unwrap();
}

#[test]
fn a_view_is_ended_when_it_is_idle_or_failed() {
    assert!(framerate::live_ended(&json!({"state": "idle"})));
    assert!(framerate::live_ended(&json!({"state": "failed"})));
    assert!(!framerate::live_ended(&json!({"state": "playing"})));
    assert!(!framerate::live_ended(&json!({"state": "asking"})));
    assert!(!framerate::live_ended(&json!({})));
}
