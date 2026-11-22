//! Read-only Quick settings measurements. Videooutput reports source metadata, not panel output.
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use serde_json::Value;

use std::hash::{Hash, Hasher};

const PERIOD: Duration = Duration::from_secs(1);

/// The display's refresh as last read. There is no "measuring" state: the last reading stays until the next one replaces it, and
/// `Unavailable` (nothing is shown) is only what the kernel's counter says when it has no rate to give.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Display { Unavailable, Hz(u64) }

#[derive(Clone, Debug, PartialEq)]
pub struct Video {
    pub context: String,
    pub app: String,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub fps: Option<f64>,
    /// `videoInfo.hdrType` as videooutputd names it (`none`, `HDR10`, `HLG`, `DolbyVision`, ...).
    pub hdr: Option<String>,
}

/// Only one connected MAIN is unambiguous. Unknown HDR is intentionally not inferred.
pub fn video(doc: &Value) -> Option<Video> {
    if doc.get("returnValue").and_then(Value::as_bool) == Some(false) { return None; }
    let mut main = doc.get("video")?.as_array()?.iter().filter(|v| {
        v.get("sink").and_then(Value::as_str) == Some("MAIN") && v.get("connected").and_then(Value::as_bool) == Some(true)
    });
    let v = main.next()?;
    if main.next().is_some() { return None; }
    let context = v.get("context").and_then(Value::as_str).filter(|s| !s.is_empty())?.to_owned();
    let app = v.get("appId").and_then(Value::as_str).filter(|s| !s.is_empty())?.to_owned();
    let width = v.get("width").and_then(Value::as_u64).filter(|n| *n > 0);
    let height = v.get("height").and_then(Value::as_u64).filter(|n| *n > 0);
    let fps = v.get("frameRate").and_then(Value::as_f64).filter(|n| n.is_finite() && *n > 0.0);
    let hdr = v.pointer("/videoInfo/hdrType").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
    Some(Video { context, app, width, height, fps, hdr })
}

/// utp.extinputs' `getVideoInfo` for an input's pipeline (`videoInfo.video`), what LG's Game Optimizer reads: the signal's size and range.
/// The pipeline is videooutputd's context for the input's picture; a broadcast that is not an input's answers `returnValue: false`.
pub fn input_video(doc: &Value, context: &str, app: &str) -> Option<Video> {
    if doc.get("returnValue").and_then(Value::as_bool) == Some(false) { return None; }
    let v = doc.pointer("/videoInfo/video")?;
    let width = v.get("width").and_then(Value::as_u64).filter(|n| *n > 0);
    let height = v.get("height").and_then(Value::as_u64).filter(|n| *n > 0);
    let hdr = v.get("hdrType").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
    Some(Video { context: context.to_owned(), app: app.to_owned(), width, height, fps: None, hdr })
}

/// utp.extinputs' `getVRRInfo` (`vrrInfo`), the Game Optimizer's frame rate and VRR state for an input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vrr { pub fps: Option<f64>, pub on: bool }

pub fn input_vrr(doc: &Value) -> Option<Vrr> {
    if doc.get("returnValue").and_then(Value::as_bool) == Some(false) { return None; }
    let v = doc.get("vrrInfo")?;
    let fps = v.get("frameRate").and_then(Value::as_f64).filter(|n| n.is_finite() && *n > 0.0);
    // the Game Optimizer's own default is "off"
    let on = v.get("vrrType").and_then(Value::as_str).is_some_and(|t| !t.is_empty() && !t.eq_ignore_ascii_case("off"));
    Some(Vrr { fps, on })
}

enum Command { Active, Inactive }
/// The reader's thread and what it last read. A reading outlives the panel being closed and opened again, so the first frame of a new
/// opening already shows the refresh until the new interval has a reading of its own.
#[derive(Default)]
pub struct Monitor {
    started: bool, active: bool,
    tx: Option<mpsc::Sender<Command>>, published: Option<Arc<Mutex<Display>>>,
}
impl Monitor {
    pub fn set_active(&mut self, active: bool) {
        if self.active == active { return; }
        self.active = active;
        if active && !self.started {
            self.started = true;
            let (tx, rx) = mpsc::channel();
            let published = Arc::new(Mutex::new(Display::Unavailable));
            let output = Arc::clone(&published);
            if std::thread::Builder::new().name("display-sync".into()).spawn(move || worker(rx, output)).is_ok() {
                self.tx = Some(tx); self.published = Some(published);
            }
        }
        if let Some(tx) = &self.tx { let _ = tx.send(if active { Command::Active } else { Command::Inactive }); }
    }
    pub fn snapshot(&self) -> Display {
        // the reader holds the lock only to store a number, so waiting for it is not waiting for the reader's work
        self.published.as_ref().map_or(Display::Unavailable, |p| *p.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Counter { total: u64, cpus: u64, counts: [u64; 64], len: usize }
fn parse_count(text: &str) -> Option<Counter> {
    let mut lines = text.lines();
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    let mut cpus = 0;
    for field in lines.next()?.split_whitespace() {
        if !field.strip_prefix("CPU").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) { return None; }
        field.hash(&mut hash);
        cpus += 1;
    }
    if cpus == 0 || cpus > 64 { return None; }
    let signature = hash.finish();
    let mut found = None;
    for line in lines {
        let mut fields = line.split_whitespace();
        let Some(id) = fields.next() else { continue; };
        if !id.ends_with(':') || line.split_whitespace().next_back() != Some("osd_irq") { continue; }
        if found.is_some() { return None; }
        let mut total = 0u64;
        let mut counts = [0; 64];
        for count in &mut counts[..cpus] {
            *count = fields.next()?.parse::<u64>().ok()?;
            total = total.checked_add(*count)?;
        }
        found = Some(Counter { total, cpus: signature, counts, len: cpus });
    }
    found
}
/// The rate of the display's interrupt between one reading and the next. The interval is whatever really passed between the two reads
/// (a reader that was slow to be scheduled gives a longer interval, not a lost reading), and a counter that went backwards starts the
/// interval again.
#[derive(Default)]
struct Rate { previous: Option<(Instant, Counter)> }
impl Rate {
    /// `None` while there is no interval yet (the reading just taken is its start): what is shown is left as it is.
    fn sample(&mut self, now: Instant, count: Option<Counter>) -> Option<Display> {
        let Some(count) = count else { self.previous = None; return Some(Display::Unavailable); };
        let (at, before) = self.previous.replace((now, count))?;
        let elapsed = now.saturating_duration_since(at);
        if elapsed.is_zero() || count.cpus != before.cpus || count.len != before.len || count.counts[..count.len].iter().zip(&before.counts[..before.len]).any(|(a, b)| a < b) { return None; }
        let delta = count.total - before.total;
        if delta == 0 { return Some(Display::Unavailable); }
        let hz = (delta as f64 / elapsed.as_secs_f64()).round();
        Some(if hz < 1.0 || hz >= u64::MAX as f64 { Display::Unavailable } else { Display::Hz(hz as u64) })
    }
}
fn read_interrupts(mut reader: impl std::io::Read, buffer: &mut [u8]) -> Option<Counter> {
    let mut used = 0;
    loop {
        if used == buffer.len() { return None; }
        match reader.read(&mut buffer[used..]) {
            Ok(0) => return parse_count(std::str::from_utf8(&buffer[..used]).ok()?),
            Ok(n) => used += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        }
    }
}
fn worker(rx: mpsc::Receiver<Command>, output: Arc<Mutex<Display>>) {
    crate::host::priority::background();
    let mut active = false;
    let mut rate = Rate::default();
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let command = if active {
            match rx.recv_timeout(PERIOD) { Ok(c) => Some(c), Err(mpsc::RecvTimeoutError::Timeout) => None, Err(_) => return }
        } else { match rx.recv() { Ok(c) => Some(c), Err(_) => return } };
        match command {
            // a new opening starts a new interval: the closed time between two readings is not an interval of the display
            Some(Command::Active) => { active = true; rate = Rate::default(); }
            Some(Command::Inactive) => { active = false; continue; }
            None => {}
        }
        if active {
            let count = std::fs::File::open("/proc/interrupts").ok().and_then(|f| read_interrupts(f, &mut buffer));
            if let Some(value) = rate.sample(Instant::now(), count) {
                *output.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = value;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn only_exact_irq_and_all_cpu_columns_count() {
        assert_eq!(parse_count(" CPU0 CPU1\n 42: 10 20 GIC osd_irq\n").map(|v| v.total), Some(30));
        assert_eq!(parse_count(" CPU0\n 42: 10 GIC osd_irq_extra\n"), None);
        assert_eq!(parse_count(" CPU0 CPU1\n 42: 10 x GIC osd_irq\n"), None);
        assert_eq!(parse_count(" CPU0\n 42: 10 GIC osd_irq\n 43: 2 GIC osd_irq\n"), None);
        let mut b = [0; 12]; assert_eq!(read_interrupts(&b" CPU0\n 42: 2 GIC osd_irq\n"[..], &mut b), None);
    }
    #[test]
    fn a_reading_is_the_real_interval_and_never_goes_back_to_measuring() {
        let t = Instant::now(); let mut r = Rate::default();
        let count = |total, cpus| { let mut counts = [0; 64]; counts[0] = total; Some(Counter { total, cpus, counts, len: 1 }) };
        assert_eq!(r.sample(t, count(10, 1)), None, "the first read only starts the interval");
        assert_eq!(r.sample(t + Duration::from_millis(1500), count(190, 1)), Some(Display::Hz(120)));
        // a reader that ran late measures the longer interval it really had: 120 Hz over 6 s, not a lost reading
        assert_eq!(r.sample(t + Duration::from_millis(7500), count(910, 1)), Some(Display::Hz(120)));
        // a counter that went backwards or changed shape starts again and leaves the last reading alone
        assert_eq!(r.sample(t + Duration::from_millis(8500), count(1, 1)), None);
        assert_eq!(r.sample(t + Duration::from_millis(9500), count(121, 2)), None);
        assert_eq!(r.sample(t + Duration::from_millis(10500), count(241, 2)), Some(Display::Hz(120)));
        assert_eq!(r.sample(t + Duration::from_millis(11500), count(241, 2)), Some(Display::Unavailable), "a counter that does not move has no rate");
        assert_eq!(r.sample(t + Duration::from_millis(12500), None), Some(Display::Unavailable), "no counter, nothing to show");
        let a = parse_count(" CPU0 CPU1\n 42: 100 50 GIC osd_irq\n");
        let b = parse_count(" CPU0 CPU1\n 42: 1 200 GIC osd_irq\n");
        let mut per_cpu = Rate::default();
        assert_eq!(per_cpu.sample(t, a), None);
        assert_eq!(per_cpu.sample(t + PERIOD, b), None, "one CPU's counter going back is a different counter");
    }
    #[test]
    fn a_known_refresh_survives_the_panel_closing_and_opening_and_nothing_is_shown_before_one() {
        let shown = Arc::new(Mutex::new(Display::Hz(120)));
        let mut m = Monitor { started: true, active: true, tx: None, published: Some(shown) };
        assert_eq!(m.snapshot(), Display::Hz(120));
        m.set_active(false); assert_eq!(m.snapshot(), Display::Hz(120));
        m.set_active(true); assert_eq!(m.snapshot(), Display::Hz(120), "the last reading until the new interval has one");
        assert_eq!(Monitor::default().snapshot(), Display::Unavailable, "no reading yet: no placeholder");
    }
    #[test]
    fn unique_connected_main_only_and_hdr_is_not_assumed() {
        let main = json!({"sink":"MAIN","connected":true,"context":"ctx","appId":"app","width":1920,"height":1080,"frameRate":24,"displayOutput":{"width":3840},"hdr":true});
        let one = video(&json!({"video":[main.clone(),{"sink":"SUB0","connected":true}]})).unwrap();
        assert_eq!((one.width, one.height, one.fps), (Some(1920), Some(1080), Some(24.0)));
        assert_eq!(video(&json!({"video":[main.clone(),main]})), None);
        assert_eq!(video(&json!({"video":[{"sink":"MAIN","connected":false}]})), None);
        assert_eq!(video(&json!({"video":[{"sink":"MAIN","connected":true}]})), None);
        let placement_only = json!({"video":[{"sink":"MAIN","connected":true,"context":"ctx","appId":"app","width":0,"height":0,"frameRate":0,"displayOutput":{"width":3840,"height":2160},"hdr":true}]});
        let unreported = video(&placement_only).unwrap();
        assert_eq!((unreported.width, unreported.height, unreported.fps), (None, None, None));
    }
    #[test]
    fn an_inputs_signal_is_read_as_the_game_optimizer_reads_it() {
        let v = input_video(&json!({"videoInfo":{"video":{"width":3840,"height":2160,"hdrType":"HDR10"}}}), "pipe", "com.webos.app.hdmi2").unwrap();
        assert_eq!((v.context.as_str(), v.width, v.height, v.hdr.as_deref()), ("pipe", Some(3840), Some(2160), Some("HDR10")));
        assert_eq!(input_video(&json!({"returnValue":false,"errorCode":-106}), "pipe", "app"), None);
        assert_eq!(input_vrr(&json!({"vrrInfo":{"frameRate":119.88,"vrrType":"Gsync"}})), Some(Vrr { fps: Some(119.88), on: true }));
        assert_eq!(input_vrr(&json!({"vrrInfo":{"frameRate":60,"vrrType":"off"}})), Some(Vrr { fps: Some(60.0), on: false }));
        assert_eq!(input_vrr(&json!({"returnValue":false})), None);
    }
}
