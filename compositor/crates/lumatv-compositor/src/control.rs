//! The control socket: a Unix stream socket beside the Wayland one, one text command per line, one JSON line
//! back. `lumatv-compositor ctl <cmd>` is the client. Nothing here listens on the network.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::State;
use crate::capture::write_png;

pub const SOCKET_NAME: &str = "lumatv-control";
/// The longest tail of the trace ring `ctl ring` answers on the loop's own thread (about 150 KB): a longer one goes to a thread that waits for its reader.
const RING_INLINE_LINES: usize = 1500;

pub fn socket_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join(SOCKET_NAME)
}

/// Answers one command line.
pub fn handle(state: &mut State, line: &str) -> Reply {
    let mut words = line.split_whitespace();
    let cmd = words.next().unwrap_or("");
    let reply = match cmd {
        "status" => {
            let now = crate::output::monotonic_ns();
            let mut oledcare = state.scene.care.status(now);
            oledcare["boxDim"] = state.scene.boxdim.report(now, &state.scene.care.settings.boxdim);
            oledcare["panelDim"] = state.panel.status();
            let power = state.luna_report();
            json!({
            "ok": true,
            "power": power,
            "summary": state.summary(),
            "output": state.output.name(),
            "buffers": format!("{:?}", state.buffer_path()),
            "clients": state.clients.values().map(|c| json!({"pid": c.pid, "uid": c.uid, "seconds": c.since.elapsed().as_secs(), "cmd": c.cmd, "bound": c.bound})).collect::<Vec<_>>(),
            "held": state.held(),
            "stats": {
                "requests": state.stats.requests, "commits": state.stats.commits, "buffers": state.stats.buffers_created,
                "ticks": state.stats.ticks, "drawn": state.stats.frames_drawn, "callbacks": state.stats.callbacks_sent, "callbacksAhead": state.stats.callbacks_ahead,
                "callbacksAtCommit": state.stats.callbacks_at_commit, "clientCallbacks": state.client_callbacks.word(),
                "releases": state.stats.releases, "skipped": state.stats.skipped_items,
                "renderUsLast": state.stats.render_us_last, "renderUsMax": state.stats.render_us_max, "passUsMax": state.stats.pass_us_max,
                "prepared": state.stats.prepared, "frameImports": state.stats.frame_imports, "uiDrawn": state.stats.ui_frames,
                "tickUsTotal": state.stats.tick_us_total, "tickPartsUs": state.stats.tick_parts_us, "passes": state.stats.passes, "passUsTotal": state.stats.pass_us_total,
            },
            "check": {"ok": state.check.ok, "bad": state.check.bad, "unread": state.check.unread},
            "focus": state.seat.focus.map(|s| s.0),
            "pointer": {"x": state.seat.pointer_position.0, "y": state.seat.pointer_position.1, "visible": state.seat.cursor_visible, "focus": state.seat.pointer_focus.map(|s| s.0), "motions": state.seat.motions, "lastInput": state.seat.last_input},
            "lastCapture": state.last_capture,
            "oledcare": oledcare,
            "scanout": state.scanout_report(),
            "pageFormat": state.output.page_kind().map(|k| crate::buffer::fourcc::name(k.fourcc())),
            "videoHolesOpaque": state.output.page_kind().map(|k| k.video_holes_opaque()),
            "pointerPlane": state.pointer_report(),
            "im": state.im.as_ref().map(|im| im.report()),
            "vwin": state.vwin_report(),
            "mask": state.scene.mask_report(),
            })
        }
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
            "buttons": state.keys.buttons.chosen().into_iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(),
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
            // Show a native panel or restart the native UI client.
            match words.next() {
                Some("restart") => {
                    state.shell_rust_restart();
                    state.shell_report()
                }
                Some("show") => {
                    // shell show KIND [PAGE]: the page is for a kind that has them (`settings privacy`)
                    let kind = words.next().unwrap_or("quick");
                    let page = words.next();
                    let taken = state.shell_show_page(kind, false, page);
                    json!({"ok": true, "kind": kind, "page": page, "taken": taken})
                }
                None => state.shell_report(),
                Some(_) => json!({"ok": false, "error": "shell [restart | show KIND [PAGE]]"}),
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
            // pointer X Y: as if the Magic Remote moved there (enter/leave/motion by the scene, cursor shown whatever shows it).
            // pointer aim X Y: the remote's pointing as the device reports it: the cursor follows only where it shows (`remote.pointerShow`).
            // pointer [plane|gpu]: where the pointer is drawn, in the frame or by the cursor plane (`pointer`, docs/research/hw-planes-result.md); the
            // report (the switch, whether the plane carries it, its counters) with no word.
            let first = words.next();
            if first.is_none() || first == Some("status") {
                return Reply::Now(json!({"ok": true, "pointer": state.pointer_report()}).to_string());
            }
            if let Some(mode) = first.and_then(crate::pointer::Mode::parse) {
                let done = match mode {
                    // the frame is always there to draw it
                    crate::pointer::Mode::Gpu => {
                        let _ = state.output.set_pointer_mode(mode);
                        Ok(())
                    }
                    crate::pointer::Mode::Plane => state.output.set_pointer_mode(mode),
                };
                return Reply::Now(
                    match done {
                        Ok(()) => {
                            state.config.pointer = mode;
                            state.pointer_sync();
                            state.journal(format!("pointer: {} (control socket)", mode.name()));
                            json!({"ok": true, "pointer": state.pointer_report()})
                        }
                        Err(e) => json!({"ok": false, "error": format!("pointer {}: {e}", mode.name()), "pointer": state.pointer_report()}),
                    }
                    .to_string(),
                );
            }
            let aim = first == Some("aim");
            let x: Option<i32> = if aim { words.next() } else { first }.and_then(|c| c.parse().ok());
            let y: Option<i32> = words.next().and_then(|c| c.parse().ok());
            match (x, y) {
                (Some(x), Some(y)) => {
                    let sent = if aim { crate::server::seat::remote_motion(state, x, y) } else { crate::server::seat::pointer_motion(state, x, y) };
                    json!({"ok": true, "delivered": sent, "focus": state.seat.pointer_focus.map(|s| s.0)})
                }
                _ => json!({"ok": false, "error": "pointer [aim] X Y | pointer [plane|gpu]"}),
            }
        }
        // The scan-out pages as the display and the GPU driver have them (`afbc`, docs/research/hw-planes-result.md): their layout, what was asked for and
        // why the old one stays when the new cannot be made, what the plane and the driver list, and the bandwidth the layout costs and saves.
        // `afbc on|off` and `pagealpha on|off` change the layout while running: the ring is converted one page at a time in the frames that draw into
        // them (needs the memory for one more page; `pages.error` says when it was not there). Same flags as at start (`--afbc`, `--page-alpha`).
        "afbc" | "pagealpha" => {
            let arg = words.next();
            let (mut mode, mut alpha) = (state.config.afbc, state.config.page_alpha);
            let asked = match (cmd, arg) {
                (_, None) => Ok(false),
                ("afbc", Some("on")) => {
                    // AFBC pages carry alpha: the page-alpha switch follows them (and stays on when they go)
                    mode = crate::afbc::Mode::On;
                    alpha = true;
                    Ok(true)
                }
                ("afbc", Some("off")) => {
                    mode = crate::afbc::Mode::Off;
                    Ok(true)
                }
                ("pagealpha", Some("on")) => {
                    alpha = true;
                    Ok(true)
                }
                ("pagealpha", Some("off")) => {
                    alpha = false;
                    Ok(true)
                }
                _ => Err(format!("{cmd} [on|off]")),
            };
            let changed = asked.and_then(|changed| {
                if !changed {
                    return Ok(());
                }
                if !alpha && (mode == crate::afbc::Mode::On || state.config.vwin.mode != crate::vwin::Mode::Off) {
                    return Err("pagealpha off cannot go with afbc on or a video window switch that is on: their holes and pages need alpha".to_string());
                }
                state.output.set_page_kind(crate::afbc::PageKind::wanted(mode, alpha)).map_err(|e| e.to_string())?;
                state.config.afbc = mode;
                state.config.page_alpha = alpha;
                Ok(())
            });
            match (changed, state.output.afbc_report()) {
                (Err(e), _) => json!({"ok": false, "error": e}),
                (Ok(()), Some(r)) => json!({"ok": true, "afbc": r, "mode": state.config.afbc.name(), "pageAlpha": state.config.page_alpha}),
                (Ok(()), None) => json!({"ok": false, "error": "this output has no scan-out pages"}),
            }
        }
        // The hardware video windows (`vwin`): `vwin` reads the allocator and the claims; `vwin off|idle|apps` is the switch; `vwin claim ID KIND X Y W H
        // [APP] [CODEC WxH@FPS]`, `vwin move ID X Y W H`, `vwin release ID|all` ask as the shell would (a TV check places a window without the shell).
        "vwin" => {
            let rest: Vec<&str> = words.collect();
            state.vwin_ctl(&rest)
        }
        // wheel [up|down]: a turn of the Magic Remote's wheel, which shows the pointer where `pointer aim` last put it (`remote.pointerShow`); with
        // a direction it is that way, for the client that scrolls with it.
        "wheel" => {
            let sent = crate::server::seat::wheel_turn(state);
            let way = match words.next() {
                Some("up") => Some(false),
                Some("down") => Some(true),
                _ => None,
            };
            let scrolled = way.map(|down| state.shell_rust_wheel_direction(down));
            let mut reply = json!({"ok": true, "delivered": sent, "visible": state.seat.cursor_visible, "pointer": state.pointer_report()});
            if let Some(scrolled) = scrolled {
                reply["scrolled"] = json!(scrolled);
            }
            reply
        }
        "click" => {
            // click [press|release|tap]: the remote's OK (KEY_ENTER); while the cursor shows, that is BTN_LEFT.
            let how = words.next().unwrap_or("tap");
            let seat = crate::server::seat::SeatId(0);
            let ok = crate::input::code::KEY_ENTER;
            match how {
                "press" => state.key_event(seat, ok, true, false),
                "release" => state.key_event(seat, ok, false, false),
                _ => {
                    state.key_event(seat, ok, true, false);
                    state.key_event(seat, ok, false, false);
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
        // Select when client frame callbacks are delivered.
        "callbacks" => match (words.next(), words.next()) {
            (Some("clients"), Some(how)) if crate::server::state::ClientCallbacks::parse(how).is_some() => {
                state.client_callbacks = crate::server::state::ClientCallbacks::parse(how).unwrap();
                json!({"ok": true, "clients": how})
            }
            _ => json!({"ok": false, "error": "callbacks clients commit|early|landing"}),
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
                if n > RING_INLINE_LINES {
                    // Hundreds of KB do not fit the socket's buffer, and the loop writes without waiting: a thread answers, waiting for the reader.
                    return Reply::Later(Box::new(move || json!({"ok": true, "lines": crate::ring::tail(n)}).to_string()));
                }
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
        "refresh" => {
            let result = match words.next() {
                None => Ok(None),
                Some("reset") => { state.output.reset_refresh_measure(); Ok(None) },
                Some("auto") => state.refresh_auto().map(Some),
                Some(w) => match w.parse::<u32>().ok().filter(|&hz| crate::refresh::supported(hz)) {
                    Some(hz) => state.refresh_apply(hz).map(|hz| { state.refresh_policy.manual = true; Some(hz) }),
                    None => Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "refresh [60|120|auto|reset]")),
                },
            };
            match result {
                Ok(changed) => {
                    if let Some(hz) = changed {
                        state.journal(format!("refresh: {hz} Hz (control socket)"));
                        state.shell_sync_topics();
                    }
                    json!({"ok": true, "hz": state.config.refresh_hz, "mode": if state.refresh_policy.manual { "manual" } else { "auto" }, "default_hz": state.refresh_policy.default_hz, "tick_ms": state.config.tick_ns as f64 / 1e6, "measure": state.output.refresh_report()})
                }
                Err(e) => json!({"ok": false, "error": e.to_string(), "hz": state.config.refresh_hz}),
            }
        },
        // Direct scan-out of a full-screen client buffer (`scanout`, docs/research/scanout-research.md): `scanout` reads the counters (direct and
        // composed frames, the reason of each composed one, the last frame's state, the protocol tap and the off-screen proof), `scanout off|direct`
        // switches it for a TV check (the screen is redrawn through the new path in the next frame), `scanout offset on|off` lets a direct frame
        // carry the OLED care's orbit as the plane's position.
        "scanout" => match words.next() {
            None => json!({"ok": true, "scanout": state.scanout_report()}),
            Some("offset") => match words.next() {
                Some(w @ ("on" | "off")) => match state.set_scanout_offset(w == "on") {
                    Ok(()) => {
                        state.scene.redraw_all();
                        json!({"ok": true, "scanout": state.scanout_report()})
                    }
                    Err(e) => json!({"ok": false, "error": format!("scanout offset {w}: {e}"), "scanout": state.scanout_report()}),
                },
                _ => json!({"ok": false, "error": "scanout offset on|off"}),
            },
            Some(w) => match crate::scanout::Mode::parse(w) {
                Some(mode) => match state.set_scanout(mode) {
                    Ok(()) => {
                        state.scene.redraw_all();
                        json!({"ok": true, "scanout": state.scanout_report()})
                    }
                    Err(e) => json!({"ok": false, "error": format!("scanout {w}: {e}"), "scanout": state.scanout_report()}),
                },
                None => json!({"ok": false, "error": "scanout [off|direct|offset on|off]"}),
            },
        },
        // A test picture of another size on the primary plane for a while (`kms::pageprobe`, docs/research/layer-deep-result.md): `pageprobe`
        // reads its state, `pageprobe WxH SECONDS` starts one (a frame carries it out at once), `pageprobe off` ends it before its time.
        "pageprobe" => match (words.next(), words.next()) {
            (None, _) => json!({"ok": true, "pageprobe": state.output.page_probe_report()}),
            (Some("off"), _) => {
                let was_on = state.output.end_page_probe();
                state.scene.redraw_all();
                json!({"ok": true, "wasOn": was_on, "pageprobe": state.output.page_probe_report()})
            }
            (Some(size), Some(secs)) => {
                let parsed = size.split_once('x').and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?))).zip(secs.parse::<u32>().ok());
                match parsed {
                    Some((size, seconds)) => match state.output.start_page_probe(size, seconds) {
                        Ok(()) => {
                            state.scene.redraw_all();
                            json!({"ok": true, "pageprobe": state.output.page_probe_report()})
                        }
                        Err(e) => json!({"ok": false, "error": format!("pageprobe: {e}"), "pageprobe": state.output.page_probe_report()}),
                    },
                    None => json!({"ok": false, "error": "pageprobe WxH SECONDS | off"}),
                }
            }
            _ => json!({"ok": false, "error": "pageprobe WxH SECONDS | off"}),
        },
        // The panel dim (`paneldim`): its state, or a dim asked for by hand until released (the TV check's quick way to see the whole path without
        // waiting for the saver or an idle Home): `paneldim want 0.8` asks the panel for 0.8 of the user's brightness, `paneldim release` ends it.
        "paneldim" => match (words.next(), words.next()) {
            (None, _) => json!({"ok": true, "panelDim": state.panel.status(), "want": state.panel_want_ctl}),
            (Some("release"), _) => {
                state.panel_want_ctl = None;
                json!({"ok": true, "panelDim": state.panel.status()})
            }
            (Some("want"), Some(v)) => match v.parse::<f32>() {
                Ok(w) if (0.0..=1.0).contains(&w) => {
                    state.panel_want_ctl = Some(w);
                    json!({"ok": true, "panelDim": state.panel.status(), "want": w})
                }
                _ => json!({"ok": false, "error": "paneldim want X (0 to 1)"}),
            },
            _ => json!({"ok": false, "error": "paneldim | paneldim want X | paneldim release"}),
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
            json!({"ok": false, "error": format!("unknown command {other}; status, windows, journal, note TEXT, ime, keys, luna, shell [restart|show KIND], capture PATH, screencap [status|shot|record start|record stop], key CODE, remote CODE, pointer [aim] X Y, wheel, click, sched mali fifo|other, callbacks clients commit|early|landing, mem, wake [reset|probe start|probe stop], ring [N], idle [MS], refresh [60|120], scanout [off|direct|offset on|off], pageprobe [WxH SECONDS|off], pointer [plane|gpu|X Y], afbc [on|off], pagealpha [on|off], vwin [off|idle|apps|claim|move|release], paneldim [want X|release], fallback [reason], quit")})
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
