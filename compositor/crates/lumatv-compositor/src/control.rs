//! The control socket: a Unix stream socket beside the Wayland one, one text command per line, one JSON line
//! back. `lumatv-compositor ctl <cmd>` is the client. Nothing here listens on the network.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::State;
use crate::capture::write_png;

pub const SOCKET_NAME: &str = "lumatv-control";

pub fn socket_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join(SOCKET_NAME)
}

/// Answers one command line.
pub fn handle(state: &mut State, line: &str) -> Reply {
    let mut words = line.split_whitespace();
    let cmd = words.next().unwrap_or("");
    let reply = match cmd {
        "status" => json!({
            "ok": true,
            "summary": state.summary(),
            "output": state.output.name(),
            "buffers": format!("{:?}", state.buffer_path()),
            "clients": state.clients.values().map(|c| json!({"pid": c.pid, "uid": c.uid, "seconds": c.since.elapsed().as_secs(), "cmd": c.cmd, "bound": c.bound})).collect::<Vec<_>>(),
            "stats": {
                "requests": state.stats.requests, "commits": state.stats.commits, "buffers": state.stats.buffers_created,
                "ticks": state.stats.ticks, "drawn": state.stats.frames_drawn, "callbacks": state.stats.callbacks_sent,
                "releases": state.stats.releases, "skipped": state.stats.skipped_items,
                "renderUsLast": state.stats.render_us_last, "renderUsMax": state.stats.render_us_max,
            },
            "check": {"ok": state.check.ok, "bad": state.check.bad, "unread": state.check.unread},
            "focus": state.seat.focus.map(|s| s.0),
            "pointer": {"x": state.seat.pointer_position.0, "y": state.seat.pointer_position.1, "visible": state.seat.cursor_visible, "focus": state.seat.pointer_focus.map(|s| s.0), "motions": state.seat.motions, "lastInput": state.seat.last_input},
            "lastCapture": state.last_capture,
        }),
        "windows" => {
            json!({"ok": true, "windows": state.windows_report(), "groups": state.scene.groups.iter().map(|g| json!({"name": g.name, "owner": g.owner.0, "layers": g.layers.keys().collect::<Vec<_>>()})).collect::<Vec<_>>(), "video": state.foreign.video_windows()})
        }
        "journal" => json!({"ok": true, "lines": state.journal}),
        "luna" => json!({
            "ok": true,
            "bridge": state.luna.is_connected(),
            "subscriptions": state.luna_state.subscriptions.iter().map(|s| json!({"token": s.token, "service": s.service, "method": s.method, "sender": s.sender, "last": s.last})).collect::<Vec<_>>(),
        }),
        "capture" => match words.next() {
            Some(path) => match state.capture_image() {
                Ok(image) => {
                    let path = path.to_string();
                    return Reply::Later(Box::new(move || match write_png(Path::new(&path), &image) {
                        Ok(()) => json!({"ok": true, "path": path}).to_string(),
                        Err(e) => json!({"ok": false, "error": e.to_string()}).to_string(),
                    }));
                }
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
        "pointer" => {
            // pointer X Y: as if the Magic Remote moved there (enter/leave/motion by the scene, cursor shown).
            let x: Option<i32> = words.next().and_then(|c| c.parse().ok());
            let y: Option<i32> = words.next().and_then(|c| c.parse().ok());
            match (x, y) {
                (Some(x), Some(y)) => {
                    let sent = crate::server::seat::pointer_motion(state, x, y);
                    json!({"ok": true, "delivered": sent, "focus": state.seat.pointer_focus.map(|s| s.0)})
                }
                _ => json!({"ok": false, "error": "pointer X Y"}),
            }
        }
        "click" => {
            // click [press|release|tap]: OK through the key routing while the cursor shows, i.e. BTN_LEFT.
            let how = words.next().unwrap_or("tap");
            match how {
                "press" => state.key_event(crate::input::code::KEY_ENTER, true, false),
                "release" => state.key_event(crate::input::code::KEY_ENTER, false, false),
                _ => {
                    state.key_event(crate::input::code::KEY_ENTER, true, false);
                    state.key_event(crate::input::code::KEY_ENTER, false, false);
                }
            }
            json!({"ok": true, "delivered": state.keys.delivered, "visible": state.seat.cursor_visible})
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
            json!({"ok": false, "error": format!("unknown command {other}; status, windows, journal, luna, capture PATH, key CODE, remote CODE, pointer X Y, click, fallback [reason], quit")})
        }
    };
    Reply::Now(reply.to_string())
}

/// How a command is answered: at once, or by a job that runs on a thread of its own with the client's socket (a PNG
/// of the output takes a few hundred milliseconds to encode on the set, which the loop cannot spend).
pub enum Reply {
    Now(String),
    Later(Box<dyn FnOnce() -> String + Send + 'static>),
}

impl Reply {
    /// The reply's text, running a deferred job here (tests; the loop never does).
    pub fn text(self) -> String {
        match self {
            Reply::Now(s) => s,
            Reply::Later(job) => job(),
        }
    }
}

/// The `ctl` subcommand: sends one line and prints the answer.
pub fn client(runtime_dir: &Path, words: &[String]) -> std::io::Result<String> {
    let mut stream = UnixStream::connect(socket_path(runtime_dir))?;
    stream.write_all(format!("{}\n", words.join(" ")).as_bytes())?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    Ok(line)
}
