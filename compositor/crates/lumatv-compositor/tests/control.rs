//! The control socket on the real `lumatv-compositor` binary (software renderer): a client that connects and sends nothing,
//! or half a line, holds nothing up; `capture PATH` answers once the PNG is written, off the loop, and the loop keeps
//! ticking meanwhile.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn ctl(dir: &std::path::Path, words: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_lumatv-compositor")).args(["ctl", "--runtime-dir", dir.to_str().unwrap()]).args(words).output().unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn ticks(dir: &std::path::Path) -> u64 {
    let status: serde_json::Value = serde_json::from_str(ctl(dir, &["status"]).trim()).expect("status json");
    status["stats"]["ticks"].as_u64().unwrap()
}

#[test]
fn control_clients_never_hold_the_loop_and_a_capture_is_written_off_it() {
    let dir = std::env::temp_dir().join(format!("lumatv-control-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_lumatv-compositor"))
        .args(["--runtime-dir", dir.to_str().unwrap(), "--renderer", "software", "--seconds", "20"])
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

    // A client that says nothing, and one that sends half a line: the loop goes on ticking for everyone else.
    let _silent = UnixStream::connect(&sock).unwrap();
    let mut half = UnixStream::connect(&sock).unwrap();
    half.write_all(b"stat").unwrap();
    let before = ticks(&dir);
    std::thread::sleep(Duration::from_millis(100));
    let after = ticks(&dir);
    assert!(after > before, "ticks {before} -> {after} with a silent and a half-line client connected");
    // The half line completes and is answered on that same connection.
    half.write_all(b"us\n").unwrap();
    let mut line = String::new();
    half.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    BufReader::new(&half).read_line(&mut line).unwrap();
    assert!(line.contains("\"ok\":true") && line.contains("\"summary\""), "{line}");

    // A capture: the reply comes once the file is there, and the loop ticked while it was being written.
    let png = dir.join("shot.png");
    let mut client = UnixStream::connect(&sock).unwrap();
    client.write_all(format!("capture {}\n", png.display()).as_bytes()).unwrap();
    client.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut reply = String::new();
    BufReader::new(&client).read_line(&mut reply).unwrap();
    assert!(reply.contains("\"ok\":true"), "{reply}");
    let mut header = [0u8; 8];
    std::fs::File::open(&png).unwrap().read_exact(&mut header).unwrap();
    assert_eq!(&header, b"\x89PNG\r\n\x1a\n");
    assert!(ticks(&dir) > after);

    let _ = ctl(&dir, &["quit"]);
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}
