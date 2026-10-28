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

fn json(dir: &std::path::Path, words: &[&str]) -> serde_json::Value {
    serde_json::from_str(ctl(dir, words).trim()).unwrap_or_else(|e| panic!("ctl {words:?}: {e}"))
}

/// The binary measures its own wake-ups (`wake`), tunes malloc and says what it locked (`mem`), keeps a trace ring in a file that outlives it
/// (`ring`) and takes the idle tick's period (`idle`).
#[test]
fn the_loop_reports_its_wakeups_its_memory_and_its_ring_and_the_ring_outlives_it() {
    let dir = std::env::temp_dir().join(format!("lumatv-wake-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_lumatv-compositor"))
        .args(["--runtime-dir", dir.to_str().unwrap(), "--renderer", "software", "--seconds", "30"])
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
    std::thread::sleep(Duration::from_millis(1300));

    // The tick's wake-up latency: a 60 Hz timer for 1.3 s is about 80 samples, p50 under the maximum, the line the gate prints.
    let wake = json(&dir, &["wake"]);
    assert_eq!(wake["ok"], true);
    let n = wake["tick"]["n"].as_u64().unwrap();
    assert!((40..=100).contains(&n), "{n} tick wake-ups in 1.3 s: {wake}");
    assert!(wake["tick"]["p50_us"].as_u64().unwrap() <= wake["tick"]["max_us"].as_u64().unwrap());
    assert!(wake["line"].as_str().unwrap().starts_with("tick p50 "), "{wake}");
    assert_eq!(json(&dir, &["wake", "reset"])["ok"], true);
    assert!(json(&dir, &["wake"])["tick"]["n"].as_u64().unwrap() < n, "reset zeroes what was counted");
    // Probe threads of two classes: a time-shared one always runs, a FIFO one only where the process may.
    let started = json(&dir, &["wake", "probe", "start", "other", "2000"]);
    assert_eq!(started["started"][0], "other", "{started}");
    std::thread::sleep(Duration::from_millis(200));
    let stopped = json(&dir, &["wake", "probe", "stop"]);
    assert!(stopped["probes"]["other"]["n"].as_u64().unwrap() >= 20, "{stopped}");
    assert_eq!(json(&dir, &["wake", "probe", "start", "nonsense"])["ok"], false);

    // Memory: malloc takes no mmap in this binary, and the lock line says what happened.
    let mem = json(&dir, &["mem"]);
    assert_eq!(mem["ok"], true);
    assert_eq!(mem["malloc"]["mmapped_blocks"], 0, "tuned: nothing is served by mmap; {mem}");
    assert!(mem["lock"].as_str().is_some_and(|l| l.contains("locked")), "{mem}");
    assert!(mem["rss_kib"].as_u64().unwrap() > 0);

    // The ring has the log and a snapshot a second in.
    let ring = json(&dir, &["ring", "400"]);
    assert_eq!(ring["ok"], true);
    let lines: Vec<&str> = ring["lines"].as_array().unwrap().iter().filter_map(|l| l.as_str()).collect();
    assert!(lines.iter().any(|l| l.contains("listening on")), "the log's lines are in the ring: {lines:?}");
    assert!(lines.iter().any(|l| l.contains(" S ticks ")), "a one-second snapshot: {lines:?}");

    // The idle tick's period, read and set.
    assert_eq!(json(&dir, &["idle"])["idle_tick_ms"], 100);
    assert_eq!(json(&dir, &["idle", "0"])["idle_tick_ms"], 0);

    let _ = ctl(&dir, &["quit"]);
    let _ = child.wait();
    // The file is what a supervisor copies after the process is gone.
    let bytes = std::fs::read(dir.join("lumatv-trace.ring")).expect("the ring file outlives the process");
    let text = lumatv_compositor::ring::unwrap(&bytes).expect("a ring");
    assert!(text.contains("summary:"), "the exit's own line is the last of the log: {}", &text[text.len().saturating_sub(400)..]);
    let _ = std::fs::remove_dir_all(&dir);
}
