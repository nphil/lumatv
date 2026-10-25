//! The bridge protocol end to end: the real `lumatv-compositor` binary connected to a fake supervisor over the
//! JSON-line socket (what `sm/lumatv-sm.c` speaks), answering pings and luna calls and sending the fallback.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn read_line_with(reader: &mut BufReader<std::os::unix::net::UnixStream>, timeout: Duration, pred: impl Fn(&str) -> bool) -> Option<String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => return None,
            Ok(_) => {
                if pred(&line) {
                    return Some(line);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => return None,
        }
    }
    None
}

#[test]
fn compositor_speaks_the_bridge_protocol() {
    let dir = std::env::temp_dir().join(format!("lumatv-bridge-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("bridge.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_lumatv-compositor"))
        .args(["--runtime-dir", dir.to_str().unwrap(), "--renderer", "software", "--seconds", "20"])
        .env("LUMATV_LUNA_BRIDGE", &sock)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    listener.set_nonblocking(false).unwrap();
    let (stream, _) = listener.accept().unwrap();
    stream.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    writer.write_all(b"{\"kind\":\"ping\",\"seq\":7}\n").unwrap();
    let pong = read_line_with(&mut reader, Duration::from_secs(3), |l| l.contains("\"pong\"")).expect("a pong");
    assert!(pong.contains("\"seq\":7"), "{pong}");

    writer
        .write_all(b"{\"kind\":\"inbound\",\"token\":11,\"service\":\"com.webos.surfacemanager\",\"method\":\"getForegroundAppInfo\",\"payload\":{\"subscribe\":true},\"subscribe\":true,\"sender\":\"com.webos.applicationManager\"}\n")
        .unwrap();
    let reply = read_line_with(&mut reader, Duration::from_secs(3), |l| l.contains("\"reply\"")).expect("a reply");
    assert!(
        reply.contains("\"token\":11")
            && reply.contains("\"returnValue\":true")
            && reply.contains("\"subscribed\":true")
            && reply.contains("\"foregroundAppInfo\":[]"),
        "{reply}"
    );

    writer.write_all(b"{\"kind\":\"inbound\",\"token\":12,\"service\":\"com.webos.coverUI\",\"method\":\"getStatus\",\"payload\":{},\"subscribe\":false,\"sender\":\"com.webos.bootManager\"}\n").unwrap();
    let reply = read_line_with(&mut reader, Duration::from_secs(3), |l| l.contains("\"token\":12")).expect("a reply");
    assert!(reply.contains("\"status\":\"closed\""), "{reply}");

    // The hold-Home escape, as the control socket sends it until keys come from evdev.
    let ctl = Command::new(env!("CARGO_BIN_EXE_lumatv-compositor")).args(["ctl", "--runtime-dir", dir.to_str().unwrap(), "fallback", "test"]).output().unwrap();
    assert!(String::from_utf8_lossy(&ctl.stdout).contains("\"ok\":true"), "{:?}", ctl);
    let fb = read_line_with(&mut reader, Duration::from_secs(3), |l| l.contains("\"fallback\"")).expect("a fallback");
    assert!(fb.contains("\"reason\":\"test\""), "{fb}");

    let _ = Command::new(env!("CARGO_BIN_EXE_lumatv-compositor")).args(["ctl", "--runtime-dir", dir.to_str().unwrap(), "quit"]).output();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}
