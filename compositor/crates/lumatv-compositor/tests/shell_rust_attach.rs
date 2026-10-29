//! The real `lumatv-compositor` binary (feature `ui`, Mesa's software GL) started with no Rust shell kinds, then given one at run time with
//! `ctl shell rust +settings`, the way round 18's fresh session was: the engine attaches while the loop runs. The set's Mali takes 1.3 s
//! to link the engine's programs and the loop that did it answered nothing for that long, so the supervisor's heartbeat (a ping every
//! 500 ms, two unanswered) put LG's compositor back. `LUMATV_COMPILE_DELAY_MS` stands in for the driver's time: each program a renderer
//! links itself takes 30 ms longer, so that the 48 programs are 1.4 s for whoever links them. The loop's longest pass (`ctl status`,
//! `stats.passUsMax`) must stay far under that, the engine must take its programs from the thread that linked them, and the restart of
//! the client, which attaches a new engine, must link nothing.
#![cfg(feature = "ui")]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MESA: &str = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";
const LINK_MS: u64 = 30;
/// What one pass of the loop may take: a frame at 60 Hz is 16.7 ms and this is a loaded host's allowance on top of it, an order under the
/// 1.4 s that linking the programs on the loop's thread takes at the stand-in's speed.
const PASS_BUDGET_US: u64 = 100_000;

fn ctl(dir: &Path, words: &[&str]) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_lumatv-compositor")).args(["ctl", "--runtime-dir", dir.to_str().unwrap()]).args(words).output().unwrap();
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("ctl {words:?}: {e}: {}", String::from_utf8_lossy(&out.stdout)))
}

/// The pid of the Rust client, once it is running.
fn client_pid(dir: &Path) -> Option<u64> {
    ctl(dir, &["shell", "rust"])["rust"]["pid"].as_u64()
}

fn wait_for<T>(what: &str, within: Duration, mut ask: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + within;
    loop {
        if let Some(found) = ask() {
            return found;
        }
        assert!(Instant::now() < deadline, "{what}: not within {within:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn an_engine_attached_while_the_loop_runs_does_not_hold_the_loop_for_its_programs() {
    if !Path::new(MESA).exists() {
        eprintln!("skipped: no Mesa at {MESA}");
        return;
    }
    let dir = std::env::temp_dir().join(format!("lumatv-shell-rust-attach-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A client that stays up and says nothing: the engine is attached and its socket handed over, which is all this looks at.
    let client = dir.join("client.sh");
    std::fs::write(&client, "#!/bin/sh\nexec sleep 60\n").unwrap();
    std::fs::set_permissions(&client, std::fs::Permissions::from_mode(0o755)).unwrap();
    let port = {
        let first = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        first.local_addr().unwrap().port().saturating_sub(1).max(20000)
    };
    let log = dir.join("compositor.log");
    let mut child = Command::new(env!("CARGO_BIN_EXE_lumatv-compositor"))
        .args(["--runtime-dir", dir.to_str().unwrap(), "--data-dir", dir.to_str().unwrap(), "--seconds", "60"])
        .args(["--renderer", "gl", "--driver", MESA, "--egl-platform", "surfaceless", "--buffers", "own", "--output", "640x360"])
        .args(["--shell-port", &port.to_string()])
        .env("LIBGL_ALWAYS_SOFTWARE", "1")
        .env("LUMATV_COMPILE_DELAY_MS", LINK_MS.to_string())
        .env("LUMATV_UI_BIN", &client)
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&log).unwrap())
        .spawn()
        .unwrap();
    let sock = dir.join("lumatv-control");
    wait_for("the control socket", Duration::from_secs(10), || sock.exists().then_some(()));

    assert_eq!(ctl(&dir, &["shell", "rust"])["rust"]["kinds"], serde_json::json!([]), "started with no kinds");
    assert_eq!(ctl(&dir, &["shell", "rust", "+settings"])["ok"], true);
    // The client starts once the programs are linked, 1.4 s on their thread; the loop answers meanwhile.
    let first = wait_for("the client's start", Duration::from_secs(30), || client_pid(&dir));
    assert!(ctl(&dir, &["status"])["ok"] == true, "the loop answers while the engine's programs are linked");
    // A restart attaches a new engine: the old one's programs are in the cache again, and nothing is linked.
    ctl(&dir, &["shell", "restart"]);
    let second = wait_for("the restarted client", Duration::from_secs(30), || client_pid(&dir).filter(|pid| *pid != first));
    assert_ne!(first, second);
    let pass_us = ctl(&dir, &["status"])["stats"]["passUsMax"].as_u64().expect("stats.passUsMax");
    let _ = ctl(&dir, &["quit"]);
    let _ = child.wait();
    let text = std::fs::read_to_string(&log).unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(pass_us < PASS_BUDGET_US, "the loop was busy {pass_us} us in one pass (a pass may take {PASS_BUDGET_US} us):\n{text}");
    let lines = |needle: &str| text.lines().filter(|l| l.contains(needle)).collect::<Vec<_>>();
    // Linked on the thread of their own, and long enough for the stand-in to have been what took the time.
    let linked = lines("programs linked on a thread of its own in");
    assert_eq!(linked.len(), 1, "the programs are linked once for the process:\n{text}");
    let ms: u64 = linked[0].split("thread of its own in ").nth(1).and_then(|s| s.split(' ').next()).and_then(|s| s.parse().ok()).expect("the link's milliseconds");
    assert!(ms >= 40 * LINK_MS, "{ms} ms for the programs: the stand-in's time was not spent");
    // Both engines, the first attach and the restart's, took every program and linked none.
    let built = lines("engine built in");
    assert_eq!(built.len(), 2, "two engines were attached:\n{text}");
    for line in built {
        assert!(line.contains(": 0 programs linked here, "), "an engine linked programs on the loop's thread: {line}");
    }
}
