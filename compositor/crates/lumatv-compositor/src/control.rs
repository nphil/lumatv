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
                "ticks": state.stats.ticks, "drawn": state.stats.frames_drawn, "callbacks": state.stats.callbacks_sent, "callbacksAhead": state.stats.callbacks_ahead,
                "releases": state.stats.releases, "skipped": state.stats.skipped_items,
                "renderUsLast": state.stats.render_us_last, "renderUsMax": state.stats.render_us_max,
                "prepared": state.stats.prepared, "frameImports": state.stats.frame_imports,
            },
            "check": {"ok": state.check.ok, "bad": state.check.bad, "unread": state.check.unread},
            "focus": state.seat.focus.map(|s| s.0),
            "pointer": {"x": state.seat.pointer_position.0, "y": state.seat.pointer_position.1, "visible": state.seat.cursor_visible, "focus": state.seat.pointer_focus.map(|s| s.0), "motions": state.seat.motions, "lastInput": state.seat.last_input},
            "lastCapture": state.last_capture,
            "oledcare": {"enabled": state.scene.care.enabled(), "offset": [state.scene.care.offset().0, state.scene.care.offset().1], "dim": [state.scene.care.dim(0), state.scene.care.dim(1)], "beats": state.scene.care.beats},
        }),
        "windows" => {
            let app_of =
                |s: crate::surface::SurfaceId| state.scene.window_of(s).and_then(|w| state.scene.window(w)).map(|w| w.app_id().to_string()).unwrap_or_default();
            // Per group: its layers, and what is attached (a layer's member, or an anonymous one with its z hint) with the app of its window.
            let groups: Vec<_> = state
                .scene
                .groups
                .iter()
                .map(|g| {
                    let layered = g.layers.iter().filter_map(|(name, l)| l.member.map(|m| json!({"layer": name, "z": l.z, "surface": m.0, "appId": app_of(m)})));
                    let anonymous = g.anonymous.iter().map(|(m, hint)| json!({"layer": null, "zHint": hint, "surface": m.0, "appId": app_of(*m)}));
                    json!({"name": g.name, "owner": g.owner.0, "layers": g.layers.keys().collect::<Vec<_>>(), "members": layered.chain(anonymous).collect::<Vec<_>>()})
                })
                .collect();
            json!({"ok": true, "windows": state.windows_report(), "groups": groups, "video": state.foreign.video_windows()})
        }
        "journal" => json!({"ok": true, "lines": state.journal}),
        "note" => {
            // note TEXT: a marker in the journal, which is how a TV check reads only what happened after it.
            let text = words.collect::<Vec<_>>().join(" ");
            state.journal(format!("note: {text}"));
            json!({"ok": true})
        }
        "ime" => state.ime_report(),
        "transitions" => {
            // transitions [on | off | auto]: the switch over the theme's (for a TV check), then the report
            match words.next() {
                Some("on") => state.motion.forced = Some(true),
                Some("off") => state.motion.forced = Some(false),
                Some("auto") => state.motion.forced = None,
                _ => {}
            }
            state.apply_motion();
            state.transitions_report()
        }
        "keys" => json!({
            "ok": true,
            "table": state.keys.router.table.named().into_iter().map(|(name, code)| json!({"name": name, "code": code})).collect::<Vec<_>>(),
            "appKeys": state.keys.app_keys.chosen().into_iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(),
            "unnamed": state.keys.unnamed,
            "foreground": state.foreground_app(),
            "keyboardUp": state.keyboard_up(),
            "delivered": state.keys.delivered,
            "dropped": state.keys.dropped,
        }),
        "luna" => json!({
            "ok": true,
            "bridge": state.luna.is_connected(),
            "subscriptions": state.luna_state.subscriptions.iter().map(|s| json!({"token": s.token, "service": s.service, "method": s.method, "sender": s.sender, "last": s.last})).collect::<Vec<_>>(),
        }),
        "shell" => {
            // shell [restart | show KIND]: the shell client and its UI channel (`shell.rs`); `show` asks the UI for a panel the way
            // the gear and Input do, `restart` ends the client and starts it again.
            match words.next() {
                Some("restart") => {
                    state.shell_restart();
                    state.shell_report()
                }
                Some("show") => {
                    let kind = words.next().unwrap_or("quick");
                    let taken = state.shell_show(kind, false);
                    json!({"ok": true, "kind": kind, "taken": taken})
                }
                _ => state.shell_report(),
            }
        }
        "switcher" => {
            // switcher [open | close | quitall | activate ID | quit ID]: the app switcher (`switcher.rs`) as the shell would ask for it; without
            // a word, what it holds. A TV check opens it without the remote's keys and reads its cards.
            let now = std::time::Instant::now();
            match words.next() {
                Some(op @ ("open" | "close" | "quitall")) => {
                    state.switcher_op(&json!({"op": op}), now);
                }
                Some(op @ ("activate" | "quit")) => {
                    let id = words.next().unwrap_or("");
                    state.switcher_op(&json!({"op": op, "id": id}), now);
                }
                _ => {}
            }
            state.switcher_report()
        }
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
        // screencap [status | shot | record start | record stop]: screenshots and screen recordings of the composed picture (`screencap`,
        // which the helper's /v1/capture routes forward to); the answers that wait on a frame or a file are given off the loop.
        "screencap" => return state.screencap_command(words.next(), words.next()),
        "key" => {
            // key CODE [press|release|tap]: on seat 0, straight to the focused client.
            use crate::server::seat::{SeatId, send_key};
            let code: Option<u32> = words.next().and_then(|c| c.parse().ok());
            let how = words.next().unwrap_or("tap");
            match code {
                Some(code) => {
                    let sent = match how {
                        "press" => send_key(state, SeatId(0), code, true),
                        "release" => send_key(state, SeatId(0), code, false),
                        _ => send_key(state, SeatId(0), code, true) && send_key(state, SeatId(0), code, false),
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
                    let seat = crate::server::seat::SeatId(0);
                    match how {
                        "press" => state.key_event(seat, code, true, false),
                        "release" => state.key_event(seat, code, false, false),
                        _ => {
                            state.key_event(seat, code, true, false);
                            state.key_event(seat, code, false, false);
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
            let seat = crate::server::seat::SeatId(0);
            match how {
                "press" => state.key_event(seat, crate::input::code::KEY_ENTER, true, false),
                "release" => state.key_event(seat, crate::input::code::KEY_ENTER, false, false),
                _ => {
                    state.key_event(seat, crate::input::code::KEY_ENTER, true, false);
                    state.key_event(seat, crate::input::code::KEY_ENTER, false, false);
                }
            }
            json!({"ok": true, "delivered": state.keys.delivered, "visible": state.seat.cursor_visible})
        }
        // The Mali blob's submission threads' class, for an A/B on the set without a restart: `sched mali fifo [PRIO]` or
        // `sched mali other`.
        "sched" => match (words.next(), words.next()) {
            (Some("mali"), Some("fifo")) => {
                let prio = words.next().and_then(|w| w.parse::<i32>().ok()).filter(|p| (1..=99).contains(p)).unwrap_or(19);
                json!({"ok": true, "threads": crate::sched::mali_threads(Some(prio))})
            }
            (Some("mali"), Some("other")) => json!({"ok": true, "threads": crate::sched::mali_threads(None)}),
            _ => json!({"ok": false, "error": "sched mali fifo [PRIO] | sched mali other"}),
        },
        // When the shell client's frame callbacks go out, for an A/B on the set without a restart: `callbacks shell early` (a frame ahead
        // of the flip, the default) or `callbacks shell landing` (at the flip, as every other client's).
        "callbacks" => match (words.next(), words.next()) {
            (Some("shell"), Some(how @ ("early" | "landing"))) => {
                state.shell_ahead = how == "early";
                json!({"ok": true, "shell": how})
            }
            _ => json!({"ok": false, "error": "callbacks shell early | callbacks shell landing"}),
        },
        // The process's memory as the frame path sees it: resident, pinned, and what malloc holds from the kernel (`mem::report`).
        "mem" => crate::mem::report(),
        // Wake-up latency (`wake`): the loop's distributions as p50/p99/max, zeroed with `wake reset`, and cyclictest-style probe threads
        // of a scheduling class (`wake probe start [fifo20,fifo19,other] [PERIOD_US]`, `wake probe stop` for what they measured).
        "wake" => match (words.next(), words.next()) {
            (None, _) => crate::wake::report(),
            (Some("reset"), _) => {
                crate::wake::reset();
                json!({"ok": true})
            }
            (Some("probe"), Some("start")) => {
                let specs: Vec<crate::wake::Spec> = words.next().unwrap_or("fifo20,fifo19,other").split(',').filter_map(crate::wake::Spec::parse).collect();
                let period_us = words.next().and_then(|w| w.parse::<u64>().ok()).unwrap_or(1000).clamp(200, 100_000);
                if specs.is_empty() {
                    json!({"ok": false, "error": "wake probe start [fifo1-fifo99|other, each with an optional @CPU, comma separated] [PERIOD_US 200-100000]"})
                } else {
                    json!({"ok": true, "started": crate::wake::probe_start(&specs, std::time::Duration::from_micros(period_us)), "period_us": period_us})
                }
            }
            (Some("probe"), Some("stop")) => {
                let done = crate::wake::probe_stop();
                json!({"ok": true, "probes": done.iter().map(|(c, s)| (c.clone(), crate::wake::summary_json(s))).collect::<serde_json::Map<_, _>>()})
            }
            _ => json!({"ok": false, "error": "wake | wake reset | wake probe start [CLASS[@CPU],...] [PERIOD_US] | wake probe stop"}),
        },
        // The last lines of the trace ring (`ring`): what the loop did in its last seconds, the lines the crash bundle keeps.
        "ring" => {
            let n = words.next().and_then(|w| w.parse::<usize>().ok()).unwrap_or(40);
            if crate::ring::is_open() {
                json!({"ok": true, "lines": crate::ring::tail(n)})
            } else {
                json!({"ok": false, "error": "no trace ring (--ring off)"})
            }
        }
        // The tick's period while the screen is at rest, in ms (0: the panel's rate), for an A/B on the set: `idle` reads it, `idle MS` sets it.
        "idle" => {
            if let Some(ms) = words.next().and_then(|w| w.parse::<u64>().ok()) {
                state.config.idle_tick_ns = ms * 1_000_000;
            }
            json!({"ok": true, "idle_tick_ms": state.config.idle_tick_ns / 1_000_000})
        }
        // The refresh the output runs at (`refresh`, docs/research/ui120-result.md): `refresh` reads the mode and the measure since the
        // last switch, `refresh 60|120` switches it (the OSD's vblank divider, the pacer's period, the tick, the shell's `refresh` topic),
        // `refresh reset` starts the measure over at the mode in force (the gate's windows).
        "refresh" => match words.next() {
            None => json!({"ok": true, "hz": state.config.refresh_hz, "tick_ms": state.config.tick_ns as f64 / 1e6, "measure": state.output.refresh_report()}),
            Some(w) => match w.parse::<u32>().ok().filter(|&hz| crate::refresh::supported(hz)).or((w == "reset").then_some(state.config.refresh_hz)) {
                Some(hz) => match state.output.set_refresh(hz) {
                    Ok(hz) => {
                        state.config.refresh_hz = hz;
                        state.config.tick_ns = crate::refresh::period_ns(hz);
                        state.journal(format!("refresh: {hz} Hz (control socket{})", if w == "reset" { ", measure reset" } else { "" }));
                        state.shell_sync_topics();
                        json!({"ok": true, "hz": hz, "tick_ms": state.config.tick_ns as f64 / 1e6, "measure": state.output.refresh_report()})
                    }
                    Err(e) => json!({"ok": false, "error": format!("refresh {hz}: {e}"), "hz": state.config.refresh_hz}),
                },
                None => json!({"ok": false, "error": "refresh [60|120|reset]"}),
            },
        },
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
            json!({"ok": false, "error": format!("unknown command {other}; status, windows, journal, note TEXT, ime, keys, luna, shell [restart|show KIND], capture PATH, screencap [status|shot|record start|record stop], key CODE, remote CODE, pointer X Y, click, sched mali fifo|other, callbacks shell early|landing, mem, wake [reset|probe start|probe stop], ring [N], idle [MS], refresh [60|120], fallback [reason], quit")})
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
