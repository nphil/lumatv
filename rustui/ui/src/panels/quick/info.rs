//! Read-only Quick settings measurements. Videooutput reports source metadata, not panel output.
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use serde_json::Value;

use std::hash::{Hash, Hasher};

const PERIOD: Duration = Duration::from_secs(1);
const MAX_AGE: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Display { Measuring, Unavailable, Hz(u64) }

#[derive(Clone, Debug, PartialEq)]
pub struct Video {
    pub context: String,
    pub app: String,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub fps: Option<f64>,
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
    Some(Video { context, app, width, height, fps })
}

#[derive(Clone, Copy)]
struct Published { epoch: u64, at: Instant, value: Display }
enum Command { Active(u64), Inactive }
#[derive(Default)]
pub struct Monitor {
    started: bool, active: bool, epoch: u64,
    tx: Option<mpsc::Sender<Command>>, published: Option<Arc<Mutex<Published>>>,
}
impl Monitor {
    pub fn set_active(&mut self, active: bool) {
        if self.active == active { return; }
        self.active = active;
        self.epoch = self.epoch.wrapping_add(1);
        if active && !self.started {
            self.started = true;
            let (tx, rx) = mpsc::channel();
            let published = Arc::new(Mutex::new(Published { epoch: self.epoch, at: Instant::now(), value: Display::Measuring }));
            let output = Arc::clone(&published);
            if std::thread::Builder::new().name("display-sync".into()).spawn(move || worker(rx, output)).is_ok() {
                self.tx = Some(tx); self.published = Some(published);
            }
        }
        if let Some(tx) = &self.tx { let _ = tx.send(if active { Command::Active(self.epoch) } else { Command::Inactive }); }
    }
    pub fn reset(&mut self) { if self.active { self.set_active(false); self.set_active(true); } }
    pub fn snapshot(&self) -> Display {
        if !self.active { return Display::Unavailable; }
        let Some(p) = &self.published else { return Display::Unavailable; };
        let Ok(p) = p.try_lock() else { return Display::Measuring; };
        if p.epoch != self.epoch { return Display::Measuring; }
        if Instant::now().saturating_duration_since(p.at) > MAX_AGE { return Display::Unavailable; }
        p.value
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
#[derive(Default)]
struct Rate { previous: Option<(Instant, Counter)> }
impl Rate {
    fn sample(&mut self, now: Instant, count: Option<Counter>) -> Display {
        let Some(count) = count else { self.previous = None; return Display::Unavailable; };
        let Some((at, before)) = self.previous.replace((now, count)) else { return Display::Measuring; };
        let elapsed = now.saturating_duration_since(at);
        if elapsed.is_zero() || elapsed > MAX_AGE || count.cpus != before.cpus || count.len != before.len || count.counts[..count.len].iter().zip(&before.counts[..before.len]).any(|(a, b)| a < b) { return Display::Measuring; }
        let delta = count.total - before.total;
        if delta == 0 { return Display::Unavailable; }
        let hz = (delta as f64 / elapsed.as_secs_f64()).round();
        if hz < 1.0 || hz >= u64::MAX as f64 { Display::Unavailable } else { Display::Hz(hz as u64) }
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
fn worker(rx: mpsc::Receiver<Command>, output: Arc<Mutex<Published>>) {
    let mut active = None;
    let mut rate = Rate::default();
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let command = if active.is_some() {
            match rx.recv_timeout(PERIOD) { Ok(c) => Some(c), Err(mpsc::RecvTimeoutError::Timeout) => None, Err(_) => return }
        } else { match rx.recv() { Ok(c) => Some(c), Err(_) => return } };
        match command {
            Some(Command::Active(epoch)) => { active = Some(epoch); rate = Rate::default(); }
            Some(Command::Inactive) => { active = None; continue; }
            None => {}
        }
        if let Some(epoch) = active {
            let count = std::fs::File::open("/proc/interrupts").ok().and_then(|f| read_interrupts(f, &mut buffer));
            let now = Instant::now();
            let value = rate.sample(now, count);
            if let Ok(mut p) = output.lock() { *p = Published { epoch, at: now, value }; }
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
    fn actual_interval_reset_stale_and_disabled_fail_closed() {
        let t = Instant::now(); let mut r = Rate::default();
        let count = |total, cpus| { let mut counts = [0; 64]; counts[0] = total; Some(Counter { total, cpus, counts, len: 1 }) };
        assert_eq!(r.sample(t, count(10, 1)), Display::Measuring);
        assert_eq!(r.sample(t + Duration::from_millis(1500), count(190, 1)), Display::Hz(120));
        assert_eq!(r.sample(t + Duration::from_millis(2500), count(1, 1)), Display::Measuring);
        assert_eq!(r.sample(t + Duration::from_millis(3500), count(121, 2)), Display::Measuring);
        assert_eq!(r.sample(t + Duration::from_millis(7500), count(601, 2)), Display::Measuring);
        assert_eq!(r.sample(t + Duration::from_millis(8500), None), Display::Unavailable);
        let a = parse_count(" CPU0 CPU1\n 42: 100 50 GIC osd_irq\n");
        let b = parse_count(" CPU0 CPU1\n 42: 1 200 GIC osd_irq\n");
        let mut per_cpu = Rate::default();
        assert_eq!(per_cpu.sample(t, a), Display::Measuring);
        assert_eq!(per_cpu.sample(t + PERIOD, b), Display::Measuring);
        let p = Arc::new(Mutex::new(Published { epoch: 1, at: Instant::now() - MAX_AGE - PERIOD, value: Display::Hz(120) }));
        let mut m = Monitor { started: true, active: true, epoch: 1, tx: None, published: Some(p) };
        assert_eq!(m.snapshot(), Display::Unavailable); m.set_active(false); assert_eq!(m.snapshot(), Display::Unavailable);
        m.set_active(true); assert_eq!(m.snapshot(), Display::Measuring);
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
}
