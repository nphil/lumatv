//! Keeps webOS from preloading the apps the owner switched off.
//!
//! webOS 10 forgets its preload policy at every switch-on, so each time the screen goes from off to on (and once at start if it
//! is on) the helper reads `<data>/preload.json`, `{"off": ["youtube.leanback.v4", ...]}`, written by `lumatv preload on` and
//! deleted by `off`, and tells the preload manager to leave each listed app alone:
//! `luna-send -n 1 luna://com.webos.service.preloadmanager/setPreloadPolicy '{"application":{"id":"<id>","isEnabled":false}}'`.
//! A missing file does nothing. Each call has five seconds; a failure is logged and nothing else (no topic reports it).

use crate::config::Paths;
use crate::control::Stop;
use crate::ctx::Ctx;
use crate::hub::Hub;
use crate::screen;
use serde::Deserialize;
use serde_json::json;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const URI: &str = "luna://com.webos.service.preloadmanager/setPreloadPolicy";
const DEADLINE: Duration = Duration::from_secs(5);
const MAX_IDS: usize = 32;
const MAX_ID_BYTES: usize = 128;
const MAX_FILE_BYTES: u64 = 16 * 1024;
/// How long the watcher sleeps when the screen does not change (it wakes on the stop and on a screen event anyway).
const LONGEST_SLEEP: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    off: Vec<String>,
}

fn plain(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_BYTES
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The app ids the file lists, or why they cannot be read. A missing file is an empty list.
pub fn read_list(path: &Path) -> Result<Vec<String>, String> {
    let mut bytes = Vec::new();
    match std::fs::File::open(path) {
        Ok(file) => {
            file.take(MAX_FILE_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    }
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("over 16 KiB".to_owned());
    }
    let file: File = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if file.off.len() > MAX_IDS {
        return Err(format!("more than {MAX_IDS} apps"));
    }
    if let Some(bad) = file.off.iter().find(|id| !plain(id)) {
        return Err(format!("{bad:?} is not an app id"));
    }
    Ok(file.off)
}

/// Runs `luna-send` once with `payload`, killing it after the deadline.
fn call(program: &str, payload: &str) -> Result<(), String> {
    let mut child = Command::new(program)
        .args(["-n", "1", URI, payload])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not run {program}: {e}"))?;
    let end = Instant::now() + DEADLINE;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("luna-send exited with {status}")),
            Ok(None) if Instant::now() >= end => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("luna-send did not answer in 5 s".to_owned());
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// Applies the file's list once. Returns the ids the preload manager was told about.
pub fn apply(paths: &Paths, program: &str) -> Vec<String> {
    let ids = match read_list(&paths.preload_file()) {
        Ok(ids) => ids,
        Err(why) => {
            crate::log_warn!("preload: preload.json: {why}");
            return Vec::new();
        }
    };
    let mut told = Vec::new();
    for id in ids {
        if id == "com.webos.app.lgchannels" && paths.data_dir.join("lgchannels.json").exists() {
            continue;
        }
        let payload = json!({"application": {"id": id, "isEnabled": false}}).to_string();
        match call(program, &payload) {
            Ok(()) => told.push(id),
            Err(why) => crate::log_warn!("preload: {id}: {why}"),
        }
    }
    if !told.is_empty() {
        crate::log_info!("preload: {} apps kept from preloading", told.len());
    }
    told
}

/// Calls `on_switch_on` once when the screen is on at the start and each time it goes from off to on.
pub fn watch(hub: &Hub, stop: &Stop, mut on_switch_on: impl FnMut()) {
    let topics = [screen::TOPIC.to_owned()];
    let boot = hub.boot();
    let mut seq = hub.events_after(None, None, None).seq;
    let mut was_on = false;
    loop {
        let on = screen::is_on(hub);
        if on && !was_on {
            on_switch_on();
        }
        was_on = on;
        if stop.is_set() {
            return;
        }
        let batch = hub.wait_events(Some(seq), Some(boot), Some(&topics), LONGEST_SLEEP);
        seq = batch.seq;
        if stop.is_set() {
            return;
        }
    }
}

/// Starts the watcher thread.
pub fn spawn(ctx: &Ctx, channels: std::sync::Arc<crate::lgchannels::Channels>) -> std::io::Result<JoinHandle<()>> {
    let ctx = ctx.clone();
    thread::Builder::new()
        .name("preload".to_owned())
        .stack_size(128 * 1024)
        .spawn(move || {
            let program = ctx.config.luna.send.clone();
            channels.reconcile();
            watch(&ctx.hub, &ctx.stop, || {
                channels.with_preload_lock(|| apply(&ctx.paths, &program));
                channels.reconcile();
            });
        })
}
fn preload_service_connected(value: &serde_json::Value) -> Option<bool> {
    value.get("connected").and_then(|value| value.as_bool())
}
/// Reapply the managed setting when the preload manager reconnects while the TV is awake.
pub fn spawn_service_watch(ctx: &Ctx, channels: std::sync::Arc<crate::lgchannels::Channels>) -> std::io::Result<JoinHandle<()>> {
    use std::sync::{Arc, Mutex, mpsc};
    let program = ctx.config.luna.send.clone();
    let stop = ctx.stop.clone();
    let slot = Arc::new(Mutex::new(None::<std::process::Child>));
    stop.on_stop({
        let slot = slot.clone();
        move || {
            if let Some(mut child) = slot.lock().unwrap_or_else(|e| e.into_inner()).take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    });
    thread::Builder::new().name("preload-service".into()).stack_size(128 * 1024).spawn(move || {
        while !stop.is_set() {
            let child = Command::new(&program)
                .args(["-n", "1000000", "luna://com.webos.service.bus/signal/registerServerStatus", r#"{"serviceName":"com.webos.service.preloadmanager","subscribe":true}"#])
                .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
            if let Ok(mut child) = child {
                if let Some(out) = child.stdout.take() {
                    let (tx, rx) = mpsc::sync_channel(16);
                    let reader = thread::Builder::new().name("preload-status".into()).stack_size(128 * 1024).spawn(move || {
                        let mut pending = String::new();
                        for line in BufReader::new(out.take(1024 * 1024)).lines().map_while(Result::ok) {
                            pending.push_str(&line);
                            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&pending) {
                                pending.clear();
                                if let Some(connected) = preload_service_connected(&value) {
                                    if tx.send(connected).is_err() { break; }
                                }
                            } else if pending.len() > 8192 {
                                pending.clear();
                            }
                        }
                    });
                    if let Ok(reader) = reader {
                        *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
                        let mut received = false;
                        let first_deadline = Instant::now() + Duration::from_secs(10);
                        while !stop.is_set() {
                            match rx.recv_timeout(Duration::from_secs(1)) {
                                Ok(connected) => { received = true; if connected { channels.reconcile(); } }
                                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                                Err(mpsc::RecvTimeoutError::Timeout) if !received && Instant::now() >= first_deadline => break,
                                Err(mpsc::RecvTimeoutError::Timeout) => (),
                            }
                        }
                        drop(rx);
                        if let Some(mut child) = slot.lock().unwrap_or_else(|e| e.into_inner()).take() {
                            let _ = child.kill();
                            let _ = child.wait();
                        }
                        let _ = reader.join();
                    } else {
                        let _ = child.kill();
                        let _ = child.wait();
                    }
                } else {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
            if stop.sleep(Duration::from_secs(5)) { break; }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn preload_service_events_distinguish_reconnects() {
        assert_eq!(preload_service_connected(&json!({"connected": true})), Some(true));
        assert_eq!(preload_service_connected(&json!({"connected": false})), Some(false));
        assert_eq!(preload_service_connected(&json!({"returnValue": false})), None);
    }

    #[test]
    fn the_list_is_read_strictly_and_a_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preload.json");
        assert_eq!(read_list(&path).unwrap(), Vec::<String>::new());
        std::fs::write(
            &path,
            r#"{"off":["youtube.leanback.v4","com.webos.app.browser"]}"#,
        )
        .unwrap();
        assert_eq!(
            read_list(&path).unwrap(),
            ["youtube.leanback.v4", "com.webos.app.browser"]
        );
        for bad in [
            "{",
            r#"{"off":["a b"]}"#,
            r#"{"off":["\"}, {\"x"]}"#,
            r#"{"off":[""]}"#,
            r#"{"off":[],"on":[]}"#,
            r#"{"on":[]}"#,
        ] {
            std::fs::write(&path, bad).unwrap();
            assert!(read_list(&path).is_err(), "{bad}");
        }
        let many: Vec<String> = (0..=MAX_IDS).map(|n| format!("app{n}")).collect();
        std::fs::write(&path, json!({"off": many}).to_string()).unwrap();
        assert!(read_list(&path).is_err());
    }

    /// Runs a script that was just written until the system lets it run (exec of a file another thread's fork still holds open
    /// for writing fails with "text file busy").
    fn settle(program: &str) {
        for _ in 0..300 {
            match Command::new(program)
                .args(["probe"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
            {
                Err(e) if e.raw_os_error() == Some(26) => thread::sleep(Duration::from_millis(10)),
                _ => return,
            }
        }
    }

    fn fake_luna(dir: &Path, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("luna-send");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.display().to_string()
    }

    #[test]
    fn each_listed_app_is_told_once_and_a_failure_does_not_stop_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("calls");
        let program = fake_luna(
            dir.path(),
            &format!(
                "[ \"$1\" = probe ] && exit 0\nprintf '%s\\n' \"$*\" >> {}\ncase \"$4\" in *bad.app*) exit 1;; esac",
                log.display()
            ),
        );
        settle(&program);
        let paths = Paths::new(dir.path());
        std::fs::write(
            paths.preload_file(),
            r#"{"off":["youtube.leanback.v4","bad.app","com.webos.app.browser"]}"#,
        )
        .unwrap();
        let told = apply(&paths, &program);
        assert_eq!(told, ["youtube.leanback.v4", "com.webos.app.browser"]);
        let calls = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = calls.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            format!(
                "-n 1 {URI} {{\"application\":{{\"id\":\"youtube.leanback.v4\",\"isEnabled\":false}}}}"
            )
        );
        // No file: nothing is called.
        std::fs::remove_file(paths.preload_file()).unwrap();
        assert!(apply(&paths, &program).is_empty());
        assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 3);
    }

    #[test]
    fn a_luna_send_that_hangs_is_killed_at_the_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let program = fake_luna(dir.path(), "[ \"$1\" = probe ] && exit 0; exec sleep 30");
        settle(&program);
        let started = Instant::now();
        let result = call(&program, "{}");
        assert!(result.unwrap_err().contains("did not answer"));
        assert!(started.elapsed() < Duration::from_secs(8));
    }

    #[test]
    fn it_runs_at_start_with_the_screen_on_and_at_each_switch_on_and_not_otherwise() {
        let hub = Arc::new(Hub::new(1));
        let stop = Stop::new();
        let count = Arc::new(AtomicUsize::new(0));
        let watcher = {
            let (hub, stop, count) = (hub.clone(), stop.clone(), count.clone());
            thread::spawn(move || {
                watch(&hub, &stop, || {
                    count.fetch_add(1, Ordering::SeqCst);
                });
            })
        };
        let wait_for = |n: usize| {
            let end = Instant::now() + Duration::from_secs(5);
            while Instant::now() < end && count.load(Ordering::SeqCst) != n {
                thread::sleep(Duration::from_millis(5));
            }
            thread::sleep(Duration::from_millis(60));
            assert_eq!(count.load(Ordering::SeqCst), n);
        };
        thread::sleep(Duration::from_millis(50));
        assert_eq!(count.load(Ordering::SeqCst), 0, "unknown is off");
        hub.set("screen", json!({"on": true, "state": "Active"}));
        wait_for(1);
        hub.set("screen", json!({"on": true, "state": "Screen Saver"}));
        wait_for(1);
        hub.set("screen", json!({"on": false, "state": "Active Standby"}));
        wait_for(1);
        hub.set("screen", json!({"on": true, "state": "Active"}));
        wait_for(2);
        stop.request();
        hub.close();
        watcher.join().unwrap();
    }

    #[test]
    fn a_screen_that_is_already_on_when_the_watcher_starts_counts_as_a_start() {
        let hub = Arc::new(Hub::new(1));
        hub.set("screen", json!({"on": true, "state": "Active"}));
        let stop = Stop::new();
        let count = Arc::new(AtomicUsize::new(0));
        let watcher = {
            let (hub, stop, count) = (hub.clone(), stop.clone(), count.clone());
            thread::spawn(move || {
                watch(&hub, &stop, || {
                    count.fetch_add(1, Ordering::SeqCst);
                });
            })
        };
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end && count.load(Ordering::SeqCst) == 0 {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
        stop.request();
        hub.close();
        watcher.join().unwrap();
    }
}
