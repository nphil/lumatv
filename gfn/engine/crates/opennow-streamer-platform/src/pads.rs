//! The pads' way into the engine: every changed report becomes a `CapturedInput::Gamepad` sample (the engine serialises it into
//! GFN's 38-byte event and sends it at once), the PS button becomes `Guide` (the engine asks the UI for its overlay), and the exit
//! chord a `StopStream` shortcut. While the UI's overlay is up (input paused) the reports go to the UI's own socket instead, as JSON
//! datagrams, so the menu can be driven with the same pad without releasing the grab.
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use lumatv_gfn_pad::{PadConfig, PadEvent, PadRuntime, PadSink, PadState};

use crate::types::{CapturedInput, CapturedInputQueue, CapturedInputSample, StreamShortcutAction};

struct Bridge {
    queue: Arc<CapturedInputQueue>,
    active: AtomicBool,
    paused: AtomicBool,
    side: Option<(UnixDatagram, PathBuf)>,
}

impl Bridge {
    fn aside(&self, line: String) {
        if let Some((sock, path)) = &self.side {
            let _ = sock.send_to(line.as_bytes(), path);   /* nobody listening is normal before the UI opens */
        }
    }
}

fn gamepad(slot: u8, bitmap: u16, s: PadState) -> CapturedInput {
    CapturedInput::Gamepad {
        controller_id: slot,
        bitmap,
        buttons: s.buttons,
        left_trigger: s.left_trigger,
        right_trigger: s.right_trigger,
        left_stick_x: s.left_x,
        left_stick_y: s.left_y,
        right_stick_x: s.right_x,
        right_stick_y: s.right_y,
    }
}

impl PadSink for Bridge {
    fn pad_event(&self, bitmap: u16, event: PadEvent) {
        match event {
            PadEvent::Connected { slot, name, uniq } => {
                self.aside(format!("{{\"pad\":\"connected\",\"slot\":{slot},\"name\":{},\"uniq\":{},\"bitmap\":{bitmap}}}", json_str(&name), json_str(&uniq)));
                if self.active.load(Ordering::Acquire) {
                    self.queue.push(gamepad(slot, bitmap, PadState::default()));
                }
            }
            PadEvent::Disconnected { slot } => {
                self.aside(format!("{{\"pad\":\"disconnected\",\"slot\":{slot},\"bitmap\":{bitmap}}}"));
                if self.active.load(Ordering::Acquire) {
                    self.queue.release_gamepad(slot, bitmap);
                }
            }
            PadEvent::Report { slot, state, press } => {
                if self.paused.load(Ordering::Acquire) || !self.active.load(Ordering::Acquire) {
                    self.aside(format!(
                        "{{\"pad\":\"report\",\"slot\":{slot},\"buttons\":{},\"lt\":{},\"rt\":{},\"lx\":{},\"ly\":{},\"rx\":{},\"ry\":{}}}",
                        state.buttons, state.left_trigger, state.right_trigger, state.left_x, state.left_y, state.right_x, state.right_y
                    ));
                } else if let Some(id) = press {
                    // The sample's `captured_at` is the key the queue hands back when the engine takes it (latency::Tap::handoff); it is
                    // linked before the push so the engine cannot take the sample first.
                    let at = Instant::now();
                    lumatv_gfn_pad::latency::global().link(id, at);
                    self.queue.push_sample(CapturedInputSample { input: gamepad(slot, bitmap, state), captured_at: at });
                } else {
                    self.queue.push(gamepad(slot, bitmap, state));
                }
            }
            PadEvent::Guide { slot, pressed } => {
                self.aside(format!("{{\"pad\":\"guide\",\"slot\":{slot},\"pressed\":{pressed}}}"));
                if pressed && self.active.load(Ordering::Acquire) {
                    self.queue.push(CapturedInput::Guide);
                }
            }
            PadEvent::ExitChord { slot } => {
                self.aside(format!("{{\"pad\":\"exit\",\"slot\":{slot}}}"));
                if self.active.load(Ordering::Acquire) {
                    self.queue.push(CapturedInput::Shortcut(StreamShortcutAction::StopStream));
                }
            }
        }
    }

    /// The app's stats line shows the press latency: the send's figures when the preload gave them, else the handoff's (microseconds).
    fn latency(&self, summary: &lumatv_gfn_pad::latency::Summary) {
        let (stage, s) = match summary.send {
            Some(s) => ("send", s),
            None => ("handoff", summary.handoff),
        };
        self.aside(format!(
            "{{\"pad\":\"latency\",\"stage\":\"{stage}\",\"n\":{},\"p50\":{},\"p90\":{},\"p99\":{},\"max\":{}}}",
            summary.samples, s.p50, s.p90, s.p99, s.max
        ));
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The dead zone the viewer chose, in percent of stick travel: the `"deadzone"` of the app's preferences file, which the UI's settings
/// screen writes and `LUMATV_GFN_PREFS` names. It is read when the pads start, once for each stream, so a change made in the settings
/// is in effect for the next stream. The core's own range is 0-50.
fn prefs_deadzone(path: &std::path::Path) -> Option<u8> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    Some(v["deadzone"].as_u64()?.min(50) as u8)
}

/// The pad crate's configuration: its environment's, with the viewer's dead zone unless `LUMATV_GFN_PAD_DEADZONE` fixes one.
fn pad_config() -> PadConfig {
    let mut c = PadConfig::from_env();
    if std::env::var_os("LUMATV_GFN_PAD_DEADZONE").is_none() {
        if let Some(z) = std::env::var_os("LUMATV_GFN_PREFS").and_then(|p| prefs_deadzone(std::path::Path::new(&p))) {
            c.deadzone_percent = z;
        }
    }
    c
}

/// The pads as the runtime sees them: started once, told when a session is live and when the overlay has the input.
pub struct PadBridge {
    bridge: Arc<Bridge>,
    runtime: Mutex<Option<PadRuntime>>,
}

impl PadBridge {
    pub fn new(queue: Arc<CapturedInputQueue>) -> Self {
        let side = std::env::var_os("LUMATV_GFN_PAD_SIDE").and_then(|p| UnixDatagram::unbound().ok().map(|s| (s, PathBuf::from(p))));
        Self {
            bridge: Arc::new(Bridge { queue, active: AtomicBool::new(false), paused: AtomicBool::new(false), side }),
            runtime: Mutex::new(None),
        }
    }

    pub fn start(&self) {
        let mut rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        if rt.is_none() {
            let sink: Arc<dyn PadSink> = Arc::clone(&self.bridge) as Arc<dyn PadSink>;
            *rt = Some(PadRuntime::start(pad_config(), sink));
        }
    }

    pub fn set_active(&self, active: bool) {
        self.bridge.active.store(active, Ordering::Release);
        if !active {
            self.bridge.queue.clear();
        }
    }

    pub fn set_paused(&self, paused: bool) {
        self.bridge.paused.store(paused, Ordering::Release);
    }

    pub fn set_grab(&self, grab: bool) {
        if let Some(rt) = self.runtime.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            rt.set_grab(grab);
        }
    }

    pub fn rumble(&self, slot: u8, low: u16, high: u16, duration_ms: u32) -> Result<(), String> {
        match self.runtime.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            Some(rt) => rt.rumble(slot, low, high, duration_ms).map_err(|e| e.to_string()),
            None => Err("pads not started".to_owned()),
        }
    }

    pub fn stop(&self) {
        if let Some(rt) = self.runtime.lock().unwrap_or_else(|e| e.into_inner()).take() {
            rt.stop();
        }
    }

    pub fn runtime(&self) -> Option<PadRuntime> {
        self.runtime.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_reach_the_queue_only_while_active_and_not_paused() {
        let queue = Arc::new(CapturedInputQueue::default());
        let b = Bridge { queue: Arc::clone(&queue), active: AtomicBool::new(false), paused: AtomicBool::new(false), side: None };
        let s = PadState { buttons: 0x1000, ..Default::default() };
        b.pad_event(0x0101, PadEvent::Report { slot: 0, state: s, press: None });
        assert!(queue.take().is_none());
        b.active.store(true, Ordering::Release);
        b.pad_event(0x0101, PadEvent::Report { slot: 0, state: s, press: None });
        assert!(matches!(queue.take(), Some(CapturedInput::Gamepad { controller_id: 0, bitmap: 0x0101, buttons: 0x1000, .. })));
        b.paused.store(true, Ordering::Release);
        b.pad_event(0x0101, PadEvent::Report { slot: 0, state: s, press: None });
        assert!(queue.take().is_none());
        b.paused.store(false, Ordering::Release);
        b.pad_event(0x0101, PadEvent::Guide { slot: 0, pressed: true });
        assert!(matches!(queue.take(), Some(CapturedInput::Guide)));
        b.pad_event(0x0101, PadEvent::ExitChord { slot: 0 });
        assert!(matches!(queue.take(), Some(CapturedInput::Shortcut(StreamShortcutAction::StopStream))));
        b.pad_event(0x0000, PadEvent::Disconnected { slot: 0 });
        assert!(matches!(queue.take(), Some(CapturedInput::Gamepad { bitmap: 0, buttons: 0, .. })));
        assert_eq!(json_str("a\"b"), "\"a\\\"b\"");
    }

    #[test]
    fn a_press_is_followed_from_the_sink_to_the_engines_take_and_its_figures_go_to_the_apps_side_socket() {
        use lumatv_gfn_pad::latency::{self, Summary};
        let dir = std::env::temp_dir().join(format!("gfn-pad-side-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pad.sock");
        let app = UnixDatagram::bind(&path).unwrap();
        let queue = Arc::new(CapturedInputQueue::default());
        let b = Bridge { queue: Arc::clone(&queue), active: AtomicBool::new(true), paused: AtomicBool::new(false), side: Some((UnixDatagram::unbound().unwrap(), path)) };
        let s = PadState { buttons: 0x1000, ..Default::default() };
        let tap = latency::global();
        let now = latency::mono_ns();
        let id = tap.begin(0, 0x1000, now - 250_000, now - 150_000);
        b.pad_event(0x0101, PadEvent::Report { slot: 0, state: s, press: Some(id) });
        tap.pushed(id, latency::mono_ns());
        // a sample that is not a press passes without leaving a trace
        b.pad_event(0x0101, PadEvent::Report { slot: 0, state: PadState::default(), press: None });
        let taken = queue.take_sample().expect("the press");
        assert!(matches!(taken.input, CapturedInput::Gamepad { buttons: 0x1000, .. }));
        assert!(queue.take_sample().is_some());
        let done = tap.take_finished(latency::mono_ns());
        let press = done.iter().find(|d| d.id == id).expect("the press finished at the take");
        assert!(press.handoff_us >= 250 && press.handoff_us < 1_000_000 && press.wake_us == 100, "{press:?}");
        let summary: Summary = tap.summary();
        b.latency(&summary);
        let mut buf = [0u8; 256];
        let n = app.recv(&mut buf).unwrap();
        let line = std::str::from_utf8(&buf[..n]).unwrap();
        assert!(line.starts_with("{\"pad\":\"latency\",\"stage\":\"handoff\",\"n\":"), "{line}");
        assert!(line.contains("\"p99\":") && line.contains("\"max\":"), "{line}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_dead_zone_is_the_one_in_the_apps_preferences_file_clamped_to_the_cores_range() {
        let dir = std::env::temp_dir().join(format!("gfn-prefs-deadzone-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("prefs.json");
        // what the UI writes (rustui/app/src/gfn/prefs.rs, `Prefs::to_json`)
        std::fs::write(&file, br#"{"tier":"uhd120","hdr":true,"bitrateMbps":80,"deadzone":12,"showStats":false}"#).unwrap();
        assert_eq!(prefs_deadzone(&file), Some(12));
        std::fs::write(&file, br#"{"deadzone":0}"#).unwrap();
        assert_eq!(prefs_deadzone(&file), Some(0), "no dead zone is a choice");
        std::fs::write(&file, br#"{"deadzone":90}"#).unwrap();
        assert_eq!(prefs_deadzone(&file), Some(50));
        std::fs::write(&file, br#"{"tier":"uhd120"}"#).unwrap();
        assert_eq!(prefs_deadzone(&file), None);
        std::fs::write(&file, b"{ not json").unwrap();
        assert_eq!(prefs_deadzone(&file), None);
        assert_eq!(prefs_deadzone(&dir.join("missing.json")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
