//! The pads' way into the engine: every changed report becomes a `CapturedInput::Gamepad` sample (the engine serialises it into
//! GFN's 38-byte event and sends it at once), the PS button becomes `Guide` (the engine asks the UI for its overlay), and the exit
//! chord a `StopStream` shortcut. While the UI's overlay is up (input paused) the reports go to the UI's own socket instead, as JSON
//! datagrams, so the menu can be driven with the same pad without releasing the grab.
use std::os::{linux::net::SocketAddrExt, unix::net::{SocketAddr, UnixDatagram}};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use lumatv_gfn_pad::{Curve, PadConfig, PadEvent, PadRuntime, PadSink, PadState, RawReport, lab::Timing};

use crate::types::{CapturedInput, CapturedInputQueue, CapturedInputSample, StreamShortcutAction};

struct Bridge {
    queue: Arc<CapturedInputQueue>,
    active: AtomicBool,
    paused: AtomicBool,
    side: Option<(UnixDatagram, PathBuf)>,
    timing: Mutex<[TimedPad; 4]>,
    stats: AtomicBool,
    stats_due: AtomicU64,
}

#[derive(Default)]
struct TimedPad { name: String, timing: Timing, sent_ns: u64 }

impl Bridge {
    fn aside(&self, line: String) {
        if let Some((sock, path)) = &self.side {
            let _ = sock.send_to(line.as_bytes(), path);   /* nobody listening is normal before the UI opens */
        }
    }

    /// The streamer's once-a-second telemetry, for the engine's statistics. The core keeps only the frame rate and bitrate of it, so
    /// the transport's ping, RTP interarrival jitter (ms) and cumulative packet loss (%) go to the engine here, as measured. Only while a
    /// session is live; a field the streamer had no reading for stays null.
    fn stream(&self, telemetry: &serde_json::Value) {
        if self.active.load(Ordering::Acquire) {
            self.aside(stream_line(telemetry));
        }
    }

    /// The stream's own end and who ended it (the server, or the transport once its recovery gave up), for the engine: the core keeps
    /// it only as a status it reports when asked. Sent although the session is no longer live, because the streamer stops its media
    /// before it announces the end.
    fn ended(&self, termination: &serde_json::Value) {
        self.aside(serde_json::json!({"stream":"ended", "source":termination["source"], "reason":termination["reason"], "resumable":termination["resumable"]}).to_string());
    }
}

/// The streamer's `session-timer` as the engine reads it: the seconds left while the far side's idle warning (timer type 4) is on, and
/// null once it is off. Type 4 with `active` false is the transport confirming input was delivered; type 5 is the server's own clear.
/// Other timers are not the idle warning.
fn idle_line(timer: &serde_json::Value) -> Option<String> {
    let kind = timer["timerType"].as_u64().filter(|k| matches!(k, 4 | 5))?;
    let seconds = (kind == 4 && timer["active"] == true).then(|| timer["secondsLeft"].as_u64()).flatten();
    Some(serde_json::json!({"stream":"idle", "seconds":seconds}).to_string())
}

fn stream_line(t: &serde_json::Value) -> String {
    serde_json::json!({"stream":"telemetry", "fps":t["framesPerSecond"], "bitrate_mbps":t["bitrateMbps"], "ping_ms":t["pingMs"],
        "jitter_ms":t["jitterMs"], "loss_pct":t["packetLossPercent"], "pacing_ms":t["pacingMs"], "host_gap_ms":t["hostGapMs"],
        "arrival_gap_ms":t["arrivalGapMs"]}).to_string()
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
                if let Some(pad) = self.timing.lock().unwrap().get_mut(usize::from(slot)) {
                    *pad = TimedPad { name, ..TimedPad::default() };
                }
                if self.active.load(Ordering::Acquire) {
                    self.queue.push(gamepad(slot, bitmap, PadState::default()));
                }
            }
            PadEvent::Disconnected { slot } => {
                self.aside(format!("{{\"pad\":\"disconnected\",\"slot\":{slot},\"bitmap\":{bitmap}}}"));
                if let Some(pad) = self.timing.lock().unwrap().get_mut(usize::from(slot)) { *pad = TimedPad::default(); }
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

    fn raw(&self, slot: u8, report: &RawReport) {
        let Some((sock, path)) = &self.side else { return };
        if !self.active.load(Ordering::Acquire) { return; }
        if report.read_ns >= self.stats_due.load(Ordering::Relaxed) {
            self.stats_due.store(report.read_ns + 250_000_000, Ordering::Relaxed);
            let mut flag = [0u8];
            while sock.recv(&mut flag).is_ok() {
                let on = flag[0] == 1;
                if self.stats.swap(on, Ordering::AcqRel) && !on {
                    for pad in self.timing.lock().unwrap().iter_mut() { pad.timing = Timing::default(); pad.sent_ns = 0; }
                }
            }
            let _ = sock.send_to(b"stats", path);
        }
        if !self.stats.load(Ordering::Acquire) { return; }
        let mut pads = self.timing.lock().unwrap();
        let Some(pad) = pads.get_mut(usize::from(slot)) else { return };
        pad.timing.push(report.kernel_ns);
        if report.kernel_ns.saturating_sub(pad.sent_ns) < 1_000_000_000 { return; }
        pad.sent_ns = report.kernel_ns;
        let s = pad.timing.summary(report.kernel_ns);
        let line = serde_json::json!({"pad":"timing", "name":pad.name, "reports":s.reports, "rate_hz":s.rate_hz,
            "p99_us":s.p99_us, "bursts":s.bursts.count, "burst_gap_us":s.bursts.gap_us,
            "age_ms":report.read_ns.saturating_sub(report.kernel_ns)/1_000_000}).to_string();
        drop(pads);
        self.aside(line);
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

/// The stick settings the viewer chose, from the app's preferences file that the UI's settings screen writes and `LUMATV_GFN_PREFS`
/// names: `"deadzone"` and `"antiDeadzone"` in percent of stick travel (the core's own range is 0-50) and `"curve"` by name. A key
/// that is missing or unreadable keeps `c`'s value. The file is read when the pads start, once for each stream, so a change made in
/// the settings is in effect for the next stream.
fn apply_prefs(c: &mut PadConfig, path: &std::path::Path) {
    let Some(v) = std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) else { return };
    if let Some(z) = v["deadzone"].as_u64() {
        c.deadzone_percent = z.min(50) as u8;
    }
    if let Some(a) = v["antiDeadzone"].as_u64() {
        c.anti_deadzone_percent = a.min(50) as u8;
    }
    if let Some(curve) = v["curve"].as_str().and_then(Curve::from_name) {
        c.curve = curve;
    }
}

/// The pad crate's configuration: its environment's, with the viewer's stick settings; `LUMATV_GFN_PAD_DEADZONE` fixes the dead zone.
fn pad_config() -> PadConfig {
    let mut c = PadConfig::from_env();
    c.raw = true;
    if let Some(p) = std::env::var_os("LUMATV_GFN_PREFS") {
        let fixed = std::env::var_os("LUMATV_GFN_PAD_DEADZONE").is_some().then_some(c.deadzone_percent);
        apply_prefs(&mut c, std::path::Path::new(&p));
        c.deadzone_percent = fixed.unwrap_or(c.deadzone_percent);
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
        let side = std::env::var_os("LUMATV_GFN_PAD_SIDE").and_then(|p| {
            let address = SocketAddr::from_abstract_name(format!("lumagame-pad-{}", std::process::id())).ok()?;
            let s = UnixDatagram::bind_addr(&address).ok()?;
            s.set_nonblocking(true).ok()?;
            Some((s, PathBuf::from(p)))
        });
        Self {
            bridge: Arc::new(Bridge { queue, active: AtomicBool::new(false), paused: AtomicBool::new(false), side, timing: Mutex::new(std::array::from_fn(|_| TimedPad::default())), stats: AtomicBool::new(false), stats_due: AtomicU64::new(0) }),
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
            self.bridge.stats.store(false, Ordering::Release);
            self.bridge.stats_due.store(0, Ordering::Relaxed);
            for pad in self.bridge.timing.lock().unwrap().iter_mut() { pad.timing = Timing::default(); pad.sent_ns = 0; }
        }
    }

    pub fn stream_telemetry(&self, telemetry: &serde_json::Value) {
        self.bridge.stream(telemetry);
    }

    pub fn stream_ended(&self, termination: &serde_json::Value) {
        self.bridge.ended(termination);
    }

    pub fn stream_idle(&self, timer: &serde_json::Value) {
        if let Some(line) = idle_line(timer) {
            self.bridge.aside(line);
        }
    }

    /// The colour tags of the stream's keyframes (colour.rs), already a wire line. Sent whether or not the session is marked live: a
    /// change is said once, so a drop would leave the engine without the stream's colour for the rest of the session.
    pub fn stream_colour(&self, line: String) {
        self.bridge.aside(line);
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
        let b = Bridge { queue: Arc::clone(&queue), active: AtomicBool::new(false), paused: AtomicBool::new(false), side: None, timing: Mutex::new(std::array::from_fn(|_| TimedPad::default())), stats: AtomicBool::new(false), stats_due: AtomicU64::new(0) };
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
        let side = UnixDatagram::unbound().unwrap();
        side.set_nonblocking(true).unwrap();
        let b = Bridge { queue: Arc::clone(&queue), active: AtomicBool::new(true), paused: AtomicBool::new(false), side: Some((side, path)), timing: Mutex::new(std::array::from_fn(|_| TimedPad::default())), stats: AtomicBool::new(true), stats_due: AtomicU64::new(u64::MAX) };
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
        b.raw(0, &RawReport { kernel_ns: 1_000_000_000, read_ns: 1_000_000_000, ..Default::default() });
        let _ = app.recv(&mut buf).unwrap();
        for ms in [1004, 1008, 1012, 2000] {
            b.raw(0, &RawReport { kernel_ns: ms * 1_000_000, read_ns: ms * 1_000_000, ..Default::default() });
        }
        let n = app.recv(&mut buf).unwrap();
        let timing: serde_json::Value = serde_json::from_slice(&buf[..n]).unwrap();
        assert_eq!(timing["pad"], "timing");
        assert_eq!(timing["rate_hz"], 250.0);
        assert_eq!(timing["p99_us"], 4000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn raw_timing_needs_a_stats_subscription_and_off_clears_it() {
        let (side,app)=UnixDatagram::pair().unwrap();
        side.set_nonblocking(true).unwrap();
        let b=Bridge {queue:Arc::new(CapturedInputQueue::default()),active:AtomicBool::new(true),paused:AtomicBool::new(false),
            side:Some((side,PathBuf::new())),timing:Mutex::new(std::array::from_fn(|_| TimedPad::default())),
            stats:AtomicBool::new(false),stats_due:AtomicU64::new(0)};
        let report=|ns| RawReport {kernel_ns:ns,read_ns:ns,..Default::default()};
        b.raw(0,&report(1));
        assert_eq!(b.timing.lock().unwrap()[0].timing.summary(1).reports,0);
        app.send(&[1]).unwrap();
        b.raw(0,&report(250_000_001));
        b.raw(0,&report(254_000_001));
        assert_eq!(b.timing.lock().unwrap()[0].timing.summary(254_000_001).reports,2);
        app.send(&[0]).unwrap();
        b.raw(0,&report(500_000_001));
        assert_eq!(b.timing.lock().unwrap()[0].timing.summary(500_000_001).reports,0);
    }

    #[test]
    fn stream_telemetry_reaches_the_side_socket_only_while_a_session_is_live() {
        let dir = std::env::temp_dir().join(format!("gfn-stream-side-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pad.sock");
        let app = UnixDatagram::bind(&path).unwrap();
        app.set_nonblocking(true).unwrap();
        let side = UnixDatagram::unbound().unwrap();
        let b = Bridge { queue: Arc::new(CapturedInputQueue::default()), active: AtomicBool::new(false), paused: AtomicBool::new(false), side: Some((side, path)), timing: Mutex::new(std::array::from_fn(|_| TimedPad::default())), stats: AtomicBool::new(false), stats_due: AtomicU64::new(0) };
        let telemetry = serde_json::json!({"type":"telemetry","framesPerSecond":119.5,"bitrateMbps":80.0,"pingMs":7.25,"jitterMs":0.5,"packetLossPercent":null,"decodeTimeMs":0.0,
            "pacingMs":2.5,"hostGapMs":[8.25,9.5],"arrivalGapMs":[4.5,16.0]});
        b.stream(&telemetry);
        let mut buf = [0u8; 512];
        assert!(app.recv(&mut buf).is_err(), "nothing is sent before the session is live");
        b.active.store(true, Ordering::Release);
        b.stream(&telemetry);
        let n = app.recv(&mut buf).unwrap();
        let line: serde_json::Value = serde_json::from_slice(&buf[..n]).unwrap();
        assert_eq!(line, serde_json::json!({"stream":"telemetry","fps":119.5,"bitrate_mbps":80.0,"ping_ms":7.25,"jitter_ms":0.5,"loss_pct":null,
            "pacing_ms":2.5,"host_gap_ms":[8.25,9.5],"arrival_gap_ms":[4.5,16.0]}));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_streams_end_reaches_the_side_socket_after_the_session_stopped() {
        let dir = std::env::temp_dir().join(format!("gfn-stream-end-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pad.sock");
        let app = UnixDatagram::bind(&path).unwrap();
        app.set_nonblocking(true).unwrap();
        let side = UnixDatagram::unbound().unwrap();
        let b = Bridge { queue: Arc::new(CapturedInputQueue::default()), active: AtomicBool::new(false), paused: AtomicBool::new(false), side: Some((side, path)), timing: Mutex::new(std::array::from_fn(|_| TimedPad::default())), stats: AtomicBool::new(false), stats_due: AtomicU64::new(0) };
        b.ended(&serde_json::json!({"source":"nvst-server","code":"server-terminated","wireCode":0x8003_0023_u32,"reason":56,"resumable":false}));
        let mut buf = [0u8; 512];
        let n = app.recv(&mut buf).unwrap();
        let line: serde_json::Value = serde_json::from_slice(&buf[..n]).unwrap();
        assert_eq!(line, serde_json::json!({"stream":"ended","source":"nvst-server","reason":56,"resumable":false}));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_streams_colour_reaches_the_side_socket_without_a_live_session() {
        let dir = std::env::temp_dir().join(format!("gfn-stream-colour-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pad.sock");
        let app = UnixDatagram::bind(&path).unwrap();
        app.set_nonblocking(true).unwrap();
        let side = UnixDatagram::unbound().unwrap();
        let bridge = Bridge { queue: Arc::new(CapturedInputQueue::default()), active: AtomicBool::new(false), paused: AtomicBool::new(false), side: Some((side, path)), timing: Mutex::new(std::array::from_fn(|_| TimedPad::default())), stats: AtomicBool::new(false), stats_due: AtomicU64::new(0) };
        let line = r#"{"stream":"colour","codec":"h265","primaries":9,"transfer":16,"matrix":9,"full_range":false}"#;
        PadBridge { bridge: Arc::new(bridge), runtime: Mutex::new(None) }.stream_colour(line.to_owned());
        let mut buf = [0u8; 512];
        let n = app.recv(&mut buf).unwrap();
        assert_eq!(&buf[..n], line.as_bytes());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_far_sides_idle_countdown_is_told_in_seconds_and_cleared_by_input_or_its_own_clear() {
        let timer = |kind: u64, seconds: u64, active: bool| serde_json::json!({"type":"session-timer","timerType":kind,"secondsLeft":seconds,"active":active});
        let line = |t: serde_json::Value| idle_line(&t).map(|l| serde_json::from_str::<serde_json::Value>(&l).unwrap());
        assert_eq!(line(timer(4, 59, true)), Some(serde_json::json!({"stream":"idle","seconds":59})));
        // Delivered input (type 4, inactive) and the server's own clear (type 5) both end the countdown.
        assert_eq!(line(timer(4, 0, false)), Some(serde_json::json!({"stream":"idle","seconds":null})));
        assert_eq!(line(timer(5, 0, false)), Some(serde_json::json!({"stream":"idle","seconds":null})));
        assert_eq!(line(timer(1, 30, true)), None, "other timers are not the idle warning");
    }

    #[test]
    fn the_stick_settings_are_the_ones_in_the_apps_preferences_file_clamped_to_the_cores_range() {
        let dir = std::env::temp_dir().join(format!("gfn-prefs-sticks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("prefs.json");
        let read = |json: &[u8]| {
            std::fs::write(&file, json).unwrap();
            let mut c = PadConfig::default();
            apply_prefs(&mut c, &file);
            (c.deadzone_percent, c.anti_deadzone_percent, c.curve)
        };
        // what the engine writes (game/crates/lumagame-engine-gfn, `pad_prefs`)
        assert_eq!(read(br#"{"deadzone":12,"antiDeadzone":15,"curve":"precise"}"#), (12, 15, Curve::Precise));
        assert_eq!(read(br#"{"tier":"uhd120","deadzone":0}"#), (0, 0, Curve::Linear), "no dead zone is a choice");
        assert_eq!(read(br#"{"deadzone":90,"antiDeadzone":70,"curve":"wobbly"}"#), (50, 50, Curve::Linear));
        assert_eq!(read(b"{ not json"), (8, 0, Curve::Linear));
        let mut c = PadConfig::default();
        apply_prefs(&mut c, &dir.join("missing.json"));
        assert_eq!(c.deadzone_percent, 8);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
