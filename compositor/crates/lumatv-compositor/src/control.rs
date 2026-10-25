//! The control socket: a Unix stream socket beside the Wayland one, one text command per line, one JSON line
//! back. `lumatv-compositor ctl <cmd>` is the client. Nothing here listens on the network.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::State;

pub const SOCKET_NAME: &str = "lumatv-control";

pub fn socket_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join(SOCKET_NAME)
}

/// Answers one command line.
pub fn handle(state: &mut State, line: &str) -> String {
    let mut words = line.split_whitespace();
    let cmd = words.next().unwrap_or("");
    let reply = match cmd {
        "status" => json!({
            "ok": true,
            "summary": state.summary(),
            "output": state.output.name(),
            "buffers": format!("{:?}", state.buffer_path()),
            "clients": state.clients.values().map(|c| json!({"pid": c.pid, "uid": c.uid, "seconds": c.since.elapsed().as_secs()})).collect::<Vec<_>>(),
            "stats": {
                "requests": state.stats.requests, "commits": state.stats.commits, "buffers": state.stats.buffers_created,
                "ticks": state.stats.ticks, "drawn": state.stats.frames_drawn, "callbacks": state.stats.callbacks_sent,
                "releases": state.stats.releases, "skipped": state.stats.skipped_items,
                "renderUsLast": state.stats.render_us_last, "renderUsMax": state.stats.render_us_max,
            },
            "check": {"ok": state.check.ok, "bad": state.check.bad, "unread": state.check.unread},
            "focus": state.seat.focus.map(|s| s.0),
            "lastCapture": state.last_capture,
        }),
        "windows" => {
            json!({"ok": true, "windows": state.windows_report(), "groups": state.scene.groups.iter().map(|g| json!({"name": g.name, "owner": g.owner.0, "layers": g.layers.keys().collect::<Vec<_>>()})).collect::<Vec<_>>(), "video": state.foreign.video_windows()})
        }
        "journal" => json!({"ok": true, "lines": state.journal}),
        "capture" => match words.next() {
            Some(path) => match state.capture_to(Path::new(path)) {
                Ok(()) => json!({"ok": true, "path": path}),
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            },
            None => json!({"ok": false, "error": "capture PATH"}),
        },
        "key" => {
            // key CODE [press|release|tap]
            let code: Option<u32> = words.next().and_then(|c| c.parse().ok());
            let how = words.next().unwrap_or("tap");
            match code {
                Some(code) => {
                    let sent = match how {
                        "press" => crate::server::seat::send_key(state, code, true),
                        "release" => crate::server::seat::send_key(state, code, false),
                        _ => crate::server::seat::send_key(state, code, true) && crate::server::seat::send_key(state, code, false),
                    };
                    json!({"ok": true, "delivered": sent})
                }
                None => json!({"ok": false, "error": "key CODE [press|release|tap]"}),
            }
        }
        "remote" => {
            // remote CODE press|release|tap: as if the remote sent it: routing by mask and policy, Home and Back
            // handling, the hold-Home escape (press, wait, release). `key` above bypasses all that.
            let code: Option<u32> = words.next().and_then(|c| c.parse().ok());
            let how = words.next().unwrap_or("tap");
            match code {
                Some(code) => {
                    match how {
                        "press" => state.key_event(code, true, false),
                        "release" => state.key_event(code, false, false),
                        _ => {
                            state.key_event(code, true, false);
                            state.key_event(code, false, false);
                        }
                    }
                    json!({"ok": true, "delivered": state.keys.delivered, "dropped": state.keys.dropped})
                }
                None => json!({"ok": false, "error": "remote CODE [press|release|tap]"}),
            }
        }
        "quit" => {
            state.should_exit = true;
            json!({"ok": true})
        }
        "fallback" => {
            // What the hold-Home escape will send once keys arrive from evdev (C4): the supervisor execs LG's
            // compositor and records the escape.
            let reason = words.collect::<Vec<_>>().join(" ");
            state.luna.send(crate::luna::ToBridge::Fallback { reason: if reason.is_empty() { "control socket".into() } else { reason } });
            json!({"ok": true, "bridge": state.luna.is_connected()})
        }
        "" => json!({"ok": false, "error": "empty command"}),
        other => {
            json!({"ok": false, "error": format!("unknown command {other}; status, windows, journal, capture PATH, key CODE, remote CODE, fallback [reason], quit")})
        }
    };
    reply.to_string()
}

/// The `ctl` subcommand: sends one line and prints the answer.
pub fn client(runtime_dir: &Path, words: &[String]) -> std::io::Result<String> {
    let mut stream = UnixStream::connect(socket_path(runtime_dir))?;
    stream.write_all(format!("{}\n", words.join(" ")).as_bytes())?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    Ok(line)
}
