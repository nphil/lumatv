//! Hidden preloads: the window of an app SAM started in the background stays out of the picture until the app is launched.
//!
//! SAM's preload (`launch` with `preload: partial` and `launchHidden`, which the helper's prewarm makes) starts an app with no window on the
//! screen. Marquee and Apple TV ignore the mode and ask for fullscreen as they start, which brought them in front of Home (2026-11-23). A native
//! app is started with its launch parameters as one JSON argument, which names the reason, so a process started as a preload is known when it
//! connects ([`started_as_preload`]) and every window it makes is held (`Window::preloaded`): answered and mapped as any card is, but never raised, so
//! it is in no frame, has no keyboard focus and is under no pointer. The window of an app that asked for fullscreen is exposed until it has drawn
//! its first frame (`sync_exposed`), as a Qt window renders only then, and is covered after it: the app's last frame is the picture that is there
//! when it is launched.
//!
//! The hold ends with the app's launch ([`State::release_preload`]), which raises the window at once so the kept frame opens: SAM's `launch`,
//! `relaunch` and `foreground` life events for the app (those that are no preload's), an explicit raise of the app (`front_app`), and a launch heard
//! while the process was still starting. Home and LumaTV's own apps are never held: the stub's flows (`home_stub`) need their card.

use std::time::{Duration, Instant};

use serde_json::Value;
use wayland_server::Resource;

use crate::State;
use crate::scene::HOME_APP;
use crate::server::webos_shell::fronted;
use crate::window::{ShellState, WindowId};

/// How many apps' last launches are kept.
const KEPT: usize = 16;
/// A launch this long before a preloaded process connected still counts as that process's: SAM's event outruns a process that was still starting.
const WHILE_STARTING: Duration = Duration::from_secs(10);
/// How much of a command line is read for the reason: the launch parameters come after the program's path.
const CMD_READ: usize = 4096;

/// Whether a command line is a preload's: the launch parameters SAM hands a native app carry `"reason":"preload"`.
pub fn is_preload_cmd(cmd: &str) -> bool {
    // arguments may be spaced as JSON is, or quoted as a string inside another
    let compact: String = cmd.chars().filter(|c| !c.is_whitespace() && *c != '\\').collect();
    compact.contains(r#""reason":"preload""#)
}

/// Whether the process `pid` was started as a preload, by the command line it has now.
pub fn started_as_preload(pid: i32) -> bool {
    let mut raw = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
    raw.truncate(CMD_READ);
    is_preload_cmd(&String::from_utf8_lossy(&raw).replace('\0', " "))
}

/// What a life event of SAM's says about the app it names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Life {
    /// A preload: a `preload` event, or one of the others that carries the reason.
    Preload,
    /// The app is launched, relaunched or brought to the foreground: it is meant to be seen.
    Launch,
    Other,
}

/// The app and the kind of a life event (`getAppLifeEvents`). `relaunch` carries the launch parameters, and a preload's say so.
pub fn life(payload: &Value) -> Option<(&str, Life)> {
    let app = payload.get("appId").and_then(Value::as_str).filter(|a| !a.is_empty())?;
    let event = payload.get("event").and_then(Value::as_str)?;
    let reason = |at: &str| payload.pointer(at).and_then(Value::as_str) == Some("preload");
    let says_preload = event == "preload" || reason("/reason") || reason("/parameters/reason") || payload.pointer("/parameters/preload").is_some_and(|p| !p.is_null());
    let kind = match event {
        _ if says_preload => Life::Preload,
        "launch" | "relaunch" | "foreground" => Life::Launch,
        _ => Life::Other,
    };
    Some((app, kind))
}

/// The apps SAM launched last, newest last: a window of a process that was started as a preload and connected after its app's launch is not held.
#[derive(Default)]
pub struct Launches(Vec<(String, Instant)>);

impl Launches {
    pub fn note(&mut self, app: &str, at: Instant) {
        self.0.retain(|(a, _)| a != app);
        if self.0.len() == KEPT {
            self.0.remove(0);
        }
        self.0.push((app.to_string(), at));
    }

    /// Whether `app` was launched at `since`, or in the moments before it ([`WHILE_STARTING`]), or later.
    pub fn since(&self, app: &str, since: Instant) -> bool {
        self.0.iter().any(|(a, at)| a == app && *at + WHILE_STARTING >= since)
    }
}

/// Apps that are never held.
fn exempt(app: &str) -> bool {
    app == HOME_APP || app.starts_with("org.nphil.lumatv")
}

impl State {
    /// `wid` was made by `client`: a process started as a preload has its windows held, while SAM's life events can end the hold.
    pub(crate) fn hold_preload(&mut self, wid: WindowId, client: &wayland_server::Client) {
        let Some(info) = self.clients.get(&client.id()).filter(|c| c.preload) else { return };
        let pid = info.pid;
        if self.scene.window(wid).is_some_and(|w| w.preloaded) {
            return;
        }
        if !self.life_events_open() {
            self.journal(format!("preload: window {} is not held, SAM's life events are not subscribed (pid {pid})", wid.0));
            return;
        }
        if let Some(w) = self.scene.window_mut(wid) {
            w.preloaded = true;
        }
        self.journal(format!("preload: window {} is held, pid {pid} was started as a hidden preload", wid.0));
    }

    /// The window `wid` was given its app: Home and LumaTV's own are not held, and one made by a process that was still starting when its app was
    /// launched is not held either.
    pub(crate) fn preload_named(&mut self, wid: WindowId) {
        let Some(w) = self.scene.window(wid).filter(|w| w.preloaded) else { return };
        let app = w.app_id().to_string();
        if app.is_empty() {
            return;
        }
        let since = self.seat.surfaces.get(&w.surface).and_then(|r| r.client()).and_then(|c| self.clients.get(&c.id())).map(|c| c.since);
        if exempt(&app) {
            if let Some(w) = self.scene.window_mut(wid) {
                w.preloaded = false;
            }
        } else if since.is_some_and(|since| self.launches.since(&app, since)) {
            self.release_preload(&app, "launched while the app was starting");
        }
    }

    /// One of SAM's life events: a launch lets the app's windows go, a preload is noted.
    pub(crate) fn preload_life_event(&mut self, payload: &Value) {
        let Some((app, kind)) = life(payload) else { return };
        match kind {
            Life::Preload => self.journal(format!("preload: SAM preloads {app}")),
            Life::Launch => {
                self.launches.note(app, Instant::now());
                self.release_preload(app, "launched");
            }
            Life::Other => {}
        }
    }

    /// The app `app` is launched: its held windows stop being held, the processes that made them are no preloads any more (a window one makes later
    /// is shown), and each window that is mapped or has asked for fullscreen comes to the front now. Returns whether one did.
    pub fn release_preload(&mut self, app: &str, why: &str) -> bool {
        let before = self.scene.foreground(&self.surfaces).map(|w| w.surface);
        let held = self.scene.release_preload(app);
        if held.is_empty() {
            return false;
        }
        let clients: Vec<_> = held
            .iter()
            .filter_map(|id| self.scene.window(*id))
            .filter_map(|w| self.seat.surfaces.get(&w.surface).and_then(|r| r.client()).map(|c| c.id()))
            .collect();
        for id in clients {
            if let Some(c) = self.clients.get_mut(&id) {
                c.preload = false;
            }
        }
        let mut raised = false;
        for id in held {
            let shown = self
                .scene
                .window(id)
                .is_some_and(|w| w.kind.is_card() && !w.in_group && (w.state == ShellState::Fullscreen || self.surfaces.get(w.surface).is_some_and(|s| s.is_mapped())));
            if shown {
                self.scene.resume_snapshot(id, &self.surfaces);
                self.scene.raise(id);
                raised = true;
            }
            self.journal(format!("preload: window {} of {app} released ({why}){}", id.0, if shown { ", raised to the front" } else { "" }));
        }
        if raised {
            fronted(self, before);
        }
        raised
    }

    /// Nothing can end a hold now (SAM's life events are not coming): every held window is let go, and shown if it asked.
    pub(crate) fn release_all_preloads(&mut self, why: &str) {
        let mut apps: Vec<String> = self.scene.windows().filter(|w| w.preloaded).map(|w| w.app_id().to_string()).collect();
        apps.sort_unstable();
        apps.dedup();
        for app in apps {
            self.release_preload(&app, why);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_command_line_names_its_reason() {
        let spaced = r#"/media/developer/apps/usr/palm/applications/com.beb.plxnative/plxnative {"@system_native_app":true, "reason": "preload"}"#;
        assert!(is_preload_cmd(spaced));
        assert!(is_preload_cmd(r#"/usr/bin/app {\"reason\":\"preload\"}"#), "a string inside another");
        assert!(!is_preload_cmd(r#"/usr/bin/app {"reason":"undefined"}"#));
        assert!(!is_preload_cmd("/usr/bin/app --preload"));
    }

    #[test]
    fn a_preloads_life_events_are_not_launches() {
        let kind = |v: Value| life(&v).map(|(_, k)| k);
        assert_eq!(kind(json!({"event": "launch", "appId": "netflix"})), Some(Life::Launch));
        assert_eq!(kind(json!({"event": "relaunch", "appId": "netflix", "parameters": {"query": "x"}})), Some(Life::Launch));
        assert_eq!(kind(json!({"event": "foreground", "appId": "netflix"})), Some(Life::Launch));
        assert_eq!(kind(json!({"event": "preload", "appId": "netflix"})), Some(Life::Preload));
        assert_eq!(kind(json!({"event": "launch", "appId": "netflix", "reason": "preload"})), Some(Life::Preload));
        assert_eq!(kind(json!({"event": "relaunch", "appId": "netflix", "parameters": {"preload": "partial"}})), Some(Life::Preload));
        assert_eq!(kind(json!({"event": "background", "appId": "netflix"})), Some(Life::Other));
        assert_eq!(kind(json!({"event": "launch"})), None, "no app");
    }

    #[test]
    fn a_launch_just_before_the_process_connected_still_counts() {
        let t0 = Instant::now();
        let mut launches = Launches::default();
        launches.note("app", t0);
        assert!(launches.since("app", t0 + Duration::from_secs(5)));
        assert!(!launches.since("app", t0 + Duration::from_secs(11)), "a launch long before is another run's");
        assert!(!launches.since("other", t0));
        for i in 0..KEPT + 3 {
            launches.note(&format!("a{i}"), t0);
        }
        assert!(!launches.since("app", t0), "the oldest are dropped");
        assert_eq!(launches.0.len(), KEPT);
    }
}
