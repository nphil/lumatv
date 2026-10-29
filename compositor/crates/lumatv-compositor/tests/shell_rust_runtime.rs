//! The real `lumatv-compositor` binary (software renderer) started with no Rust shell kinds, then given one at run time with
//! `ctl shell rust +osd`: the channel the client talks to is created then, and the loop must listen to it from then on.
//! A stand-in client does what `lumatv-ui` does first (GET /v1/state with the token) and the compositor must report it connected.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn ctl(dir: &std::path::Path, words: &[&str]) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_lumatv-compositor")).args(["ctl", "--runtime-dir", dir.to_str().unwrap()]).args(words).output().unwrap();
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("ctl {words:?}: {e}: {}", String::from_utf8_lossy(&out.stdout)))
}

#[test]
fn a_kind_enabled_at_run_time_gets_its_client_connection_noticed_by_the_loop() {
    let dir = std::env::temp_dir().join(format!("lumatv-shell-rust-runtime-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let port = {
        let first = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        first.local_addr().unwrap().port().saturating_sub(1).max(20000)
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_lumatv-compositor"))
        .args(["--runtime-dir", dir.to_str().unwrap(), "--data-dir", dir.to_str().unwrap(), "--renderer", "software", "--seconds", "30"])
        .args(["--shell-port", &port.to_string()])
        // a client that exists; the engine is not attached on this renderer, so it is not started and the test plays its part
        .env("LUMATV_UI_BIN", "/bin/true")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let sock = dir.join("lumatv-control");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(sock.exists(), "the control socket");

    assert_eq!(ctl(&dir, &["shell", "rust"])["rust"]["kinds"], serde_json::json!([]), "started with no kinds");
    assert_eq!(ctl(&dir, &["shell", "rust", "+osd"])["ok"], true);

    let token = std::fs::read_to_string(dir.join("lumatv-ui.token")).expect("the token file").trim().to_string();
    let mut client = TcpStream::connect(("127.0.0.1", port + 1)).expect("the channel listens");
    client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    client.write_all(format!("GET /v1/state HTTP/1.1\r\nHost: x\r\nX-Luma-Token: {token}\r\n\r\n").as_bytes()).unwrap();
    let mut buf = [0u8; 256];
    assert!(client.read(&mut buf).unwrap() > 0, "the channel answers");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut connected = false;
    while !connected && Instant::now() < deadline {
        connected = ctl(&dir, &["shell", "rust"])["rust"]["connected"] == true;
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = ctl(&dir, &["quit"]);
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(connected, "the loop noticed the client that connected to a channel created after it started");
}
