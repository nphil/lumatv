//! Native UI startup creates a channel the compositor loop listens to.
//! A stand-in client fetches state with the token and must be reported connected.
#![cfg(feature = "ui")]

use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn ctl(dir: &std::path::Path, words: &[&str]) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_lumatv-compositor")).args(["ctl", "--runtime-dir", dir.to_str().unwrap()]).args(words).output().unwrap();
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("ctl {words:?}: {e}: {}", String::from_utf8_lossy(&out.stdout)))
}

#[test]
fn native_startup_channel_notices_its_client_connection() {
    let dir = std::env::temp_dir().join(format!("lumatv-shell-rust-runtime-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mesa = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";
    if !Path::new(mesa).exists() { return; }
    let client_bin = dir.join("client.sh");
    std::fs::write(&client_bin, "#!/bin/sh\nexec sleep 60\n").unwrap();
    std::fs::set_permissions(&client_bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let port = {
        let first = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        first.local_addr().unwrap().port().saturating_sub(1).max(20000)
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_lumatv-compositor"))
        .args(["--runtime-dir", dir.to_str().unwrap(), "--data-dir", dir.to_str().unwrap(), "--renderer", "gl", "--driver", mesa, "--egl-platform", "surfaceless", "--buffers", "own", "--output", "640x360", "--seconds", "60"])
        .args(["--shell-port", &port.to_string(), "--shell-rust", "osd"])
        .env("LIBGL_ALWAYS_SOFTWARE", "1")
        .env("LUMATV_UI_BIN", &client_bin)
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

    assert_eq!(ctl(&dir, &["shell"])["kinds"], serde_json::json!(["osd"]));
    assert_eq!(ctl(&dir, &["shell"])["ready"], false, "a process without a first frame is not ready");

    let token = std::fs::read_to_string(dir.join("lumatv-ui.token")).expect("the token file").trim().to_string();
    let mut client = TcpStream::connect(("127.0.0.1", port + 1)).expect("the channel listens");
    client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    client.write_all(format!("GET /v1/state HTTP/1.1\r\nHost: x\r\nX-Luma-Token: {token}\r\n\r\n").as_bytes()).unwrap();
    let mut buf = [0u8; 256];
    assert!(client.read(&mut buf).unwrap() > 0, "the channel answers");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut connected = false;
    while !connected && Instant::now() < deadline {
        connected = ctl(&dir, &["shell"])["connected"] == true;
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = ctl(&dir, &["quit"]);
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(connected, "the loop noticed the client on the startup channel");
}
