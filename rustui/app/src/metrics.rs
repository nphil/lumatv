//! What the run measures: the interval between frames the client presented, how long each took to issue (and to finish on the GPU,
//! when asked), and what the process costs: CPU, resident memory, the GPU's busy share.
//!
//! The vocabulary is the PLAN's frame gate (`docs/research/perf/gate.py`): a frame is *animated* when the one before it was at most 50 ms
//! ago (it is part of a run that something started: a key's lift, a held key's repeats), and an animated frame that follows its
//! predecessor by more than 20 ms is dropped. Frames at the wallpaper's own pace (100 ms apart) are timer frames and are not judged.

use std::fs;
use std::io::Write;
use std::time::Instant;

#[derive(Clone, Copy, Debug, Default)]
pub struct Frame {
    /// When the frame's swap returned: `CLOCK_MONOTONIC` ns.
    pub at_ns: u64,
    /// Milliseconds since the previous frame's swap returned (0 for the first).
    pub gap_ms: f32,
    pub cpu_us: u32,
    pub gpu_us: u32,
    /// Time inside `eglSwapBuffers` (waiting for the compositor's frame callback), us.
    pub swap_us: u32,
    /// Seconds into the phase the frame was started.
    pub layers: u32,
}

#[derive(Default)]
pub struct Phase {
    pub name: String,
    pub frames: Vec<Frame>,
    pub started_ns: u64,
    pub ended_ns: u64,
    pub cpu_ticks: u64,
    pub clk_tck: u64,
    pub rss_kb_start: u64,
    pub rss_kb_end: u64,
    pub rss_kb_max: u64,
    pub gpu_busy: Option<(u64, u64)>,
    /// The LG compositor's CPU ticks over the phase, when its pid is known.
    pub compositor_ticks: Option<u64>,
    pub keys: u32,
}

pub fn percentile(sorted: &[f32], q: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f32 * q).round() as usize]
}

impl Phase {
    pub fn duration_s(&self) -> f64 {
        (self.ended_ns.saturating_sub(self.started_ns)) as f64 / 1e9
    }

    pub fn summary(&self) -> String {
        let dur = self.duration_s().max(1e-9);
        let n = self.frames.len();
        let mut animated = Vec::new();
        let mut over20_all = 0;
        let mut worst = 0.0f32;
        for (i, f) in self.frames.iter().enumerate() {
            if i == 0 {
                continue;
            }
            if f.gap_ms > 20.0 {
                over20_all += 1;
            }
            worst = worst.max(f.gap_ms);
            if f.gap_ms <= 50.0 {
                animated.push(f);
            }
        }
        let dropped = animated.iter().filter(|f| f.gap_ms > 20.0).count();
        let mut cpu: Vec<f32> = self.frames.iter().map(|f| f.cpu_us as f32 / 1000.0).collect();
        cpu.sort_by(f32::total_cmp);
        let mut cpu_anim: Vec<f32> = animated.iter().map(|f| f.cpu_us as f32 / 1000.0).collect();
        cpu_anim.sort_by(f32::total_cmp);
        let mut gpu: Vec<f32> = self.frames.iter().filter(|f| f.gpu_us > 0).map(|f| f.gpu_us as f32 / 1000.0).collect();
        gpu.sort_by(f32::total_cmp);
        let mut gaps: Vec<f32> = self.frames.iter().skip(1).map(|f| f.gap_ms).collect();
        gaps.sort_by(f32::total_cmp);
        let cpu_pct = if self.clk_tck > 0 { self.cpu_ticks as f64 / self.clk_tck as f64 / dur * 100.0 } else { 0.0 };
        let mut s = format!(
            "{:<8} {:5.1}s  frames {:5}  {:5.1} fps  gap p50 {:5.1} p90 {:5.1} worst {:5.1} ms  >20ms {:3} (animated {:3}, dropped {:3} = {:4.1}%)  render cpu p50 {:4.2} p90 {:4.2} max {:4.2} ms (animated p50 {:4.2} p90 {:4.2})",
            self.name,
            dur,
            n,
            n as f64 / dur,
            percentile(&gaps, 0.5),
            percentile(&gaps, 0.9),
            worst,
            over20_all,
            animated.len(),
            dropped,
            if animated.is_empty() { 0.0 } else { 100.0 * dropped as f64 / animated.len() as f64 },
            percentile(&cpu, 0.5),
            percentile(&cpu, 0.9),
            cpu.last().copied().unwrap_or(0.0),
            percentile(&cpu_anim, 0.5),
            percentile(&cpu_anim, 0.9),
        );
        if !gpu.is_empty() {
            s.push_str(&format!("  gpu-done p50 {:4.2} p90 {:4.2} ms", percentile(&gpu, 0.5), percentile(&gpu, 0.9)));
        }
        s.push_str(&format!("  client cpu {:4.1}% of a core", cpu_pct));
        if let Some(t) = self.compositor_ticks {
            let c = t as f64 / self.clk_tck.max(1) as f64 / dur * 100.0;
            s.push_str(&format!("  LG compositor cpu {:4.1}%", c));
        }
        if let Some((busy, total)) = self.gpu_busy {
            if total > 0 {
                s.push_str(&format!("  gpu busy {:4.1}%", 100.0 * busy as f64 / total as f64));
            }
        }
        s.push_str(&format!("  rss {}→{} kB (max {})  keys {}", self.rss_kb_start, self.rss_kb_end, self.rss_kb_max, self.keys));
        s
    }
}

/// `utime + stime` of `pid` in clock ticks, and its resident set in kB.
pub fn proc_cpu(pid: &str) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &stat[stat.rfind(')')? + 2..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    Some(f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?)
}

pub fn rss_kb() -> u64 {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("VmRSS:").and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok()))
        })
        .unwrap_or(0)
}

pub fn clk_tck() -> u64 {
    // SAFETY: a plain sysconf query.
    let t = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if t > 0 { t as u64 } else { 100 }
}

/// The pid of the process named `name` (LG's compositor is `surface-manager`).
pub fn pid_of(name: &str) -> Option<String> {
    for entry in fs::read_dir("/proc").ok()?.flatten() {
        let pid = entry.file_name().to_string_lossy().into_owned();
        if !pid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|c| c.trim() == name) {
            return Some(pid);
        }
    }
    None
}

/// The GPU's cumulative busy and total time, in the unit the driver counts (`/sys/kernel/debug/mali0/dvfs_utilization`, which
/// `capture.sh` samples): `gpu_busy`, `gpu_total`.
pub fn gpu_counters() -> Option<(u64, u64)> {
    let text = fs::read_to_string("/sys/kernel/debug/mali0/dvfs_utilization").ok()?;
    let (mut busy, mut idle) = (None, None);
    for token in text.split_whitespace() {
        let _ = token;
    }
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match (it.next(), it.next()) {
            (Some("busy_time:"), Some(v)) | (Some("gpu_busy:"), Some(v)) => busy = v.parse::<u64>().ok(),
            (Some("idle_time:"), Some(v)) | (Some("gpu_idle:"), Some(v)) => idle = v.parse::<u64>().ok(),
            _ => {}
        }
    }
    match (busy, idle) {
        (Some(b), Some(i)) => Some((b, b + i)),
        _ => None,
    }
}

/// The one-line-a-second log the harness reads: time, frames this second, worst gap, cpu, rss.
pub struct Log {
    out: Option<fs::File>,
    began: Instant,
}

impl Log {
    pub fn open(path: Option<&str>) -> Self {
        Self {
            out: path.and_then(|p| fs::File::create(p).ok()),
            began: Instant::now(),
        }
    }

    pub fn line(&mut self, text: &str) {
        let t = self.began.elapsed().as_secs_f64();
        let line = format!("[{t:8.3}] {text}");
        println!("{line}");
        if let Some(f) = &mut self.out {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// The ftrace marker `capture.sh` and `analyse.py` cut windows at (`P:<name>`), when tracing is armed on the set.
pub struct Marker(Option<fs::File>);

impl Marker {
    pub fn open() -> Self {
        Self(fs::OpenOptions::new().write(true).open("/sys/kernel/debug/tracing/trace_marker").ok())
    }

    pub fn write(&mut self, text: &str) {
        if let Some(f) = &mut self.0 {
            let _ = f.write_all(text.as_bytes());
        }
    }
}
