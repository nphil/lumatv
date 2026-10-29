//! The panel dim through the real binary and a stand-in supervisor on the bridge socket: the settings service is answered by this test, the
//! compositor dims the panel when `ctl paneldim want` asks, and the value comes back on SIGTERM; after SIGKILL it is the next start that puts it
//! back, from the journal.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{Value, json};

struct Tv {
    value: u32,
    dimension: Value,
    writes: Vec<u32>,
}

/// Accepts the compositor's connections on `sock` and answers its calls to the settings service from one shared value.
fn serve(sock: &Path, tv: Arc<Mutex<Tv>>) {
    let listener = UnixListener::bind(sock).unwrap();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let tv = tv.clone();
            thread::spawn(move || answer(stream, tv));
        }
    });
}

fn answer(stream: UnixStream, tv: Arc<Mutex<Tv>>) {
    let mut writer = stream.try_clone().unwrap();
    for line in BufReader::new(stream).lines().map_while(Result::ok) {
        let Ok(m) = serde_json::from_str::<Value>(&line) else { continue };
        if m["kind"] != "outbound" {
            continue;
        }
        let uri = m["uri"].as_str().unwrap_or("");
        let reply = if uri.ends_with("/getSystemSettings") {
            let t = tv.lock();
            json!({"returnValue": true, "category": "picture", "settings": {"backlight": t.value.to_string()}, "dimension": t.dimension})
        } else if uri.ends_with("/setSystemSettings") {
            let v: u32 = m["payload"]["settings"]["backlight"].as_str().unwrap().parse().unwrap();
            let mut t = tv.lock();
            t.value = v;
            t.writes.push(v);
            json!({"returnValue": true})
        } else {
            continue;
        };
        let line = json!({"kind": "event", "token": m["token"], "payload": reply});
        if writeln!(writer, "{line}").is_err() {
            return;
        }
    }
}

fn spawn(dir: &Path, sock: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_lumatv-compositor"))
        .args(["--runtime-dir", dir.to_str().unwrap(), "--data-dir", dir.to_str().unwrap(), "--renderer", "software", "--seconds", "120"])
        .env("LUMATV_LUNA_BRIDGE", sock)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn ctl(dir: &Path, words: &[&str]) -> Value {
    for _ in 0..100 {
        let out = Command::new(env!("CARGO_BIN_EXE_lumatv-compositor")).args(["ctl", "--runtime-dir", dir.to_str().unwrap()]).args(words).output().unwrap();
        if let Ok(v) = serde_json::from_slice(&out.stdout) {
            return v;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("the control socket did not answer {words:?}");
}

fn until(what: &str, secs: u64, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if ok() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("not within {secs} s: {what}");
}

fn setup(name: &str) -> (PathBuf, PathBuf, Arc<Mutex<Tv>>) {
    let dir = std::env::temp_dir().join(format!("lumatv-paneldim-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("theme")).unwrap();
    // a one-second ramp step keeps the test short
    std::fs::write(dir.join("theme/theme.json"), r#"{"oledcare": {"panelDimStepSec": 1}}"#).unwrap();
    let sock = dir.join("bridge.sock");
    let tv =
        Arc::new(Mutex::new(Tv { value: 80, dimension: json!({"pictureMode": "filmMaker", "dynamicRange": "sdr", "input": "hdmi2"}), writes: Vec::new() }));
    serve(&sock, tv.clone());
    (dir, sock, tv)
}

fn kill(child: &Child, sig: i32) {
    // SAFETY: a signal to a child this test started.
    unsafe { libc::kill(child.id() as i32, sig) };
}

#[test]
fn a_sigterm_puts_the_panel_back_before_the_process_ends() {
    let (dir, sock, tv) = setup("term");
    let mut child = spawn(&dir, &sock);
    let reply = ctl(&dir, &["paneldim", "want", "0.9"]);
    assert_eq!(reply["ok"], true, "{reply}");
    until("the panel dims to 72", 30, || tv.lock().value == 72);
    assert!(dir.join("paneldim.json").exists(), "the journal stands while it is dim");
    let status = ctl(&dir, &["status"]);
    assert_eq!(status["oledcare"]["panelDim"]["dimmed"], true, "{status}");
    kill(&child, libc::SIGTERM);
    until("the process ends", 10, || child.try_wait().ok().flatten().is_some());
    assert_eq!(tv.lock().value, 80, "the user's value is back");
    assert!(!dir.join("paneldim.json").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn after_a_sigkill_the_next_start_puts_the_panel_back_from_the_journal() {
    let (dir, sock, tv) = setup("kill");
    let mut child = spawn(&dir, &sock);
    ctl(&dir, &["paneldim", "want", "0.9"]);
    until("the panel dims to 72", 30, || tv.lock().value == 72);
    kill(&child, libc::SIGKILL);
    let _ = child.wait();
    assert_eq!(tv.lock().value, 72, "nothing put it back: the process was killed");
    assert!(dir.join("paneldim.json").exists());
    let mut next = spawn(&dir, &sock);
    until("the next start puts it back", 30, || tv.lock().value == 80);
    until("and the journal goes", 5, || !dir.join("paneldim.json").exists());
    let status = ctl(&dir, &["status"]);
    assert_eq!(status["oledcare"]["panelDim"]["dimmed"], false, "{status}");
    kill(&next, libc::SIGTERM);
    let _ = next.wait();
    let _ = std::fs::remove_dir_all(&dir);
}
