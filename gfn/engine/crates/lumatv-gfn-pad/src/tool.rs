//! The shell side of the latency measurement (`lumatv-gfn-latency`): read what the engine's tap wrote, optionally tap the HID layer
//! at the same time, print the histogram. Everything is on `CLOCK_MONOTONIC`, the clock of the evdev stamps the engine records, so
//! a second process can line its own readings up with them.
//!
//! The pad is grabbed while a stream is in front, so no second reader sees its events; the engine's `latency.tsv` is the record of
//! them. The HID tap needs no grab: `/sys/kernel/debug/hid/<device>/events` (root only) prints every input report the USB driver
//! hands to the HID layer, before `hid-sony` turns it into evdev events. The moment a report with a newly pressed button reaches
//! this process is the "USB arrival"; the engine's evdev stamp of the same press minus that is what the kernel took to deliver it.
use crate::latency::{BUCKETS_US, Stat, histogram, mono_ns};
use std::io::Write;
use std::os::unix::io::RawFd;
use std::path::Path;
use std::time::Duration;

/// One line of `latency.tsv`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub slot: u8,
    pub buttons: u16,
    pub kernel_ns: u64,
    pub wake_us: u32,
    pub pushed_us: u32,
    pub handoff_us: u32,
    pub send_us: Option<u32>,
}

pub fn parse_tsv(text: &str) -> Vec<Row> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() != 8 {
                return None;
            }
            Some(Row {
                id: f[0].parse().ok()?,
                slot: f[1].parse().ok()?,
                buttons: f[2].parse().ok()?,
                kernel_ns: f[3].parse().ok()?,
                wake_us: f[4].parse().ok()?,
                pushed_us: f[5].parse().ok()?,
                handoff_us: f[6].parse().ok()?,
                send_us: if f[7] == "-" { None } else { Some(f[7].parse().ok()?) },
            })
        })
        .collect()
}

/// The bytes of an input report from a line of the HID debug `events` file (`report (size 64) (numbered) = 01 7f ...`).
pub fn parse_hid_line(line: &str) -> Option<Vec<u8>> {
    let rest = line.trim_start().strip_prefix("report (size")?;
    let (_, hex) = rest.split_once('=')?;
    hex.split_whitespace().map(|b| u8::from_str_radix(b, 16).ok()).collect()
}

/// The buttons of a DualShock 4 input report 1 (USB, 64 bytes) as one mask: any d-pad direction, the four face buttons, the 8 of byte 6,
/// the PS and touchpad buttons. The receiver presents the stock DS4 v2 descriptor (docs/research/gfn-input-result.md).
pub fn ds4_buttons(report: &[u8]) -> Option<u32> {
    if report.len() < 8 || report[0] != 0x01 {
        return None;
    }
    let (b5, b6, b7) = (report[5], report[6], report[7]);
    let dpad = u32::from(b5 & 0x0f < 8);
    Some(dpad | u32::from(b5 >> 4) << 1 | u32::from(b6) << 5 | u32::from(b7 & 3) << 13)
}

/// A report that put a button down, and when it reached this process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub arrival_ns: u64,
}

/// Turns the HID debug lines into press edges.
#[derive(Default)]
pub struct EdgeFinder {
    prev: u32,
    pending: String,
    pub edges: Vec<Edge>,
}

impl EdgeFinder {
    /// `chunk` was read at `arrival_ns`; every complete line in it is one report.
    pub fn feed(&mut self, chunk: &str, arrival_ns: u64) {
        self.pending.push_str(chunk);
        while let Some(i) = self.pending.find('\n') {
            let line: String = self.pending.drain(..=i).collect();
            if let Some(mask) = parse_hid_line(&line).and_then(|r| ds4_buttons(&r)) {
                if mask & !self.prev != 0 {
                    self.edges.push(Edge { arrival_ns });
                }
                self.prev = mask;
            }
        }
    }
}

fn poll_in(fd: RawFd, ms: i32) -> bool {
    let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    // SAFETY: one valid pollfd.
    unsafe { libc::poll(&mut p, 1, ms) > 0 && p.revents & libc::POLLIN != 0 }
}

/// Read the HID events file until `until_ns`; the press edges, stamped when `poll` woke for them.
pub fn collect_hid(path: &Path, until_ns: u64) -> std::io::Result<Vec<Edge>> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path)?;
    let fd = std::os::unix::io::AsRawFd::as_raw_fd(&file);
    let mut finder = EdgeFinder::default();
    let mut buf = [0u8; 4096];
    while mono_ns() < until_ns {
        if !poll_in(fd, 50) {
            continue;
        }
        let arrival = mono_ns();
        // SAFETY: a valid buffer; the descriptor is open for the length of this function.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n > 0 {
            finder.feed(&String::from_utf8_lossy(&buf[..n as usize]), arrival);
        } else {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    Ok(finder.edges)
}

/// For each press, the microseconds from the last HID edge before it to its evdev stamp. An edge may arrive a little after the stamp
/// (the stamp is taken inside the same interrupt, before this process has woken), so up to 2 ms after counts, as a negative number.
pub fn usb_to_evdev_us(rows: &[Row], edges: &[Edge]) -> Vec<i64> {
    const BEFORE_NS: i64 = 20_000_000;
    const AFTER_NS: i64 = 2_000_000;
    rows.iter()
        .filter_map(|r| {
            let k = r.kernel_ns as i64;
            edges.iter().map(|e| k - e.arrival_ns as i64).filter(|d| (-AFTER_NS..=BEFORE_NS).contains(d)).min_by_key(|d| d.abs()).map(|d| d / 1000)
        })
        .collect()
}

fn ms(us: u32) -> String {
    format!("{:.2}", f64::from(us) / 1000.0)
}

fn stat_line(name: &str, values: &[u32]) -> String {
    let s = Stat::of(values);
    format!("{name:<28}{:>8}{:>8}{:>8}{:>8}\n", ms(s.p50), ms(s.p90), ms(s.p99), ms(s.max))
}

/// The report: percentiles per stage and the handoff's histogram.
pub fn render(rows: &[Row], usb: Option<&[i64]>, seconds: Option<f64>) -> String {
    let mut out = String::new();
    if rows.is_empty() {
        return "no presses recorded: was the stream in front with the pad grabbed, and the engine given LUMATV_GFN_LATENCY_DIR?\n".to_owned();
    }
    let window = seconds.map_or(String::new(), |s| format!(" in {s:.0} s"));
    out.push_str(&format!("{} presses{window}; times in ms after the kernel's evdev time stamp\n", rows.len()));
    out.push_str(&format!("{:<28}{:>8}{:>8}{:>8}{:>8}\n", "stage", "p50", "p90", "p99", "max"));
    if let Some(u) = usb {
        if u.is_empty() {
            out.push_str("USB arrival: no HID report matched a press (the debugfs file, or the pad's report layout)\n");
        } else {
            // Signed: negative means this process woke after the evdev stamp was taken. Shown as the spread, not as percentiles.
            let mut v = u.to_vec();
            v.sort_unstable();
            out.push_str(&format!(
                "{:<28}{:>8}{:>8}{:>8}{:>8}   ({} of {} matched)\n",
                "USB arrival to evdev stamp",
                format!("{:.2}", v[v.len() / 2] as f64 / 1000.0),
                format!("{:.2}", v[(v.len() * 9).div_ceil(10).clamp(1, v.len()) - 1] as f64 / 1000.0),
                format!("{:.2}", v[(v.len() * 99).div_ceil(100).clamp(1, v.len()) - 1] as f64 / 1000.0),
                format!("{:.2}", v[v.len() - 1] as f64 / 1000.0),
                v.len(),
                rows.len()
            ));
        }
    }
    let col = |f: fn(&Row) -> u32| rows.iter().map(f).collect::<Vec<u32>>();
    out.push_str(&stat_line("evdev stamp to reader woke", &col(|r| r.wake_us)));
    out.push_str(&stat_line("evdev stamp to queued", &col(|r| r.pushed_us)));
    let handoff = col(|r| r.handoff_us);
    out.push_str(&stat_line("evdev stamp to handoff", &handoff));
    let sends: Vec<u32> = rows.iter().filter_map(|r| r.send_us).collect();
    if sends.is_empty() {
        out.push_str("evdev stamp to send return   (no send stamps: the engine ran without the sendtap preload)\n");
    } else {
        out.push_str(&stat_line("evdev stamp to send return", &sends));
    }
    out.push_str("handoff histogram\n");
    let h = histogram(&handoff);
    let peak = h.iter().copied().max().unwrap_or(1).max(1);
    for (i, n) in h.iter().enumerate() {
        let label = match i {
            0 => format!("< {} ms", ms(BUCKETS_US[0])),
            6 => format!(">= {} ms", ms(BUCKETS_US[5])),
            _ => format!("{}-{} ms", ms(BUCKETS_US[i - 1]), ms(BUCKETS_US[i])),
        };
        out.push_str(&format!("  {label:<16}{n:>5}  {}\n", "#".repeat(n * 40 / peak)));
    }
    out
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn dir(args: &[String]) -> Option<std::path::PathBuf> {
    arg_value(args, "--dir").or_else(|| std::env::var("LUMATV_GFN_LATENCY_DIR").ok()).map(Into::into)
}

const USAGE: &str = "usage: lumatv-gfn-latency report [--dir DIR] [--last N]
       lumatv-gfn-latency run [--dir DIR] [--seconds 30] [--hid /sys/kernel/debug/hid/<device>/events]
       lumatv-gfn-latency hid FILE [--seconds 10]
DIR holds latency.tsv (the engine writes it while LUMATV_GFN_LATENCY_DIR is set; the app sets it to its data directory)";

/// The command line; returns the exit code. `out` is where the report goes.
pub fn main(args: &[String], out: &mut dyn Write) -> i32 {
    let usage = |out: &mut dyn Write| {
        let _ = writeln!(out, "{USAGE}");
        2
    };
    let secs = |a: &[String], default: u64| arg_value(a, "--seconds").and_then(|s| s.parse().ok()).unwrap_or(default);
    match args.first().map(String::as_str) {
        Some("report") => {
            let Some(dir) = dir(args) else { return usage(out) };
            let text = std::fs::read_to_string(dir.join("latency.tsv")).unwrap_or_default();
            let mut rows = parse_tsv(&text);
            if let Some(n) = arg_value(args, "--last").and_then(|n| n.parse::<usize>().ok()) {
                rows = rows.split_off(rows.len().saturating_sub(n));
            }
            let _ = write!(out, "{}", render(&rows, None, None));
            0
        }
        Some("run") => {
            let Some(dir) = dir(args) else { return usage(out) };
            let seconds = secs(args, 30);
            let start = mono_ns();
            let until = start + seconds * 1_000_000_000;
            let _ = writeln!(out, "press buttons for {seconds} s ...");
            let hid_edges = arg_value(args, "--hid").map(|path| {
                let path = std::path::PathBuf::from(path);
                std::thread::spawn(move || collect_hid(&path, until + 200_000_000))
            });
            std::thread::sleep(Duration::from_nanos(until.saturating_sub(mono_ns())));
            // The engine publishes once a second and a press needs 30 ms to be complete.
            std::thread::sleep(Duration::from_millis(1300));
            let edges = match hid_edges.map(|h| h.join()) {
                Some(Ok(Ok(e))) => Some(e),
                Some(Ok(Err(e))) => {
                    let _ = writeln!(out, "cannot read the HID events file: {e} (it needs root)");
                    Some(Vec::new())
                }
                Some(Err(_)) => Some(Vec::new()),
                None => None,
            };
            let text = std::fs::read_to_string(dir.join("latency.tsv")).unwrap_or_default();
            let rows: Vec<Row> = parse_tsv(&text).into_iter().filter(|r| r.kernel_ns >= start && r.kernel_ns <= until).collect();
            let usb = edges.map(|e| usb_to_evdev_us(&rows, &e));
            let _ = write!(out, "{}", render(&rows, usb.as_deref(), Some(seconds as f64)));
            0
        }
        Some("hid") => {
            let Some(file) = args.get(1) else { return usage(out) };
            let seconds = secs(args, 10);
            match collect_hid(Path::new(file), mono_ns() + seconds * 1_000_000_000) {
                Ok(edges) => {
                    for e in &edges {
                        let _ = writeln!(out, "press edge at {} ns", e.arrival_ns);
                    }
                    let _ = writeln!(out, "{} press edges in {seconds} s", edges.len());
                    0
                }
                Err(e) => {
                    let _ = writeln!(out, "cannot read {file}: {e}");
                    1
                }
            }
        }
        _ => usage(out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u64, kernel_ns: u64, handoff: u32, send: Option<u32>) -> Row {
        Row { id, slot: 0, buttons: 0x1000, kernel_ns, wake_us: handoff / 3, pushed_us: handoff / 2, handoff_us: handoff, send_us: send }
    }

    fn report(b5: u8, b6: u8, b7: u8) -> String {
        let mut r = [0u8; 64];
        r[0] = 1;
        r[1..5].copy_from_slice(&[0x80; 4]);
        (r[5], r[6], r[7]) = (b5, b6, b7);
        format!("report (size 64) (numbered) = {}\n", r.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "))
    }

    #[test]
    fn tsv_lines_round_trip_and_junk_is_skipped() {
        let d =
            crate::latency::Done { id: 7, slot: 1, buttons: 0x2000, kernel_ns: 123_456_789, wake_us: 80, pushed_us: 95, handoff_us: 310, send_us: Some(420) };
        let text = format!("{}\n{}\nnot a row\n1\t2\n", crate::latency::Done::TSV_HEADER, d.tsv());
        let rows = parse_tsv(&text);
        assert_eq!(
            rows,
            vec![Row { id: 7, slot: 1, buttons: 0x2000, kernel_ns: 123_456_789, wake_us: 80, pushed_us: 95, handoff_us: 310, send_us: Some(420) }]
        );
        let none = crate::latency::Done { send_us: None, ..d };
        assert_eq!(parse_tsv(&none.tsv())[0].send_us, None);
    }

    #[test]
    fn ds4_reports_give_press_edges_only_for_buttons_going_down() {
        let neutral = report(0x08, 0, 0);
        assert_eq!(ds4_buttons(&parse_hid_line(&neutral).unwrap()), Some(0));
        let mut f = EdgeFinder::default();
        f.feed(&neutral, 1_000);
        f.feed(&report(0x28, 0, 0), 2_000); // cross down
        f.feed(&report(0x28, 0, 0), 3_000); // held, a stick moved: no edge
        f.feed(&report(0x08, 0, 0), 4_000); // released: no edge
        f.feed(&report(0x08, 0x02, 0), 5_000); // R1
        f.feed(&report(0x00, 0x02, 0), 6_000); // d-pad up while R1 is held
        f.feed(&report(0x08, 0, 0x01), 7_000); // PS button
        assert_eq!(f.edges.iter().map(|e| e.arrival_ns).collect::<Vec<_>>(), [2_000, 5_000, 6_000, 7_000]);
        // lines split across reads, and lines that are not reports
        let mut g = EdgeFinder::default();
        let line = report(0x48, 0, 0);
        let (a, b) = line.split_at(30);
        g.feed("Report ID 1\n", 1);
        g.feed(a, 2);
        assert!(g.edges.is_empty());
        g.feed(b, 3);
        assert_eq!(g.edges, vec![Edge { arrival_ns: 3 }]);
        assert_eq!(ds4_buttons(&[2, 0, 0, 0, 0, 0, 0, 0]), None);
    }

    #[test]
    fn edges_are_matched_to_presses_by_time() {
        let rows = [row(1, 10_000_000_000, 300, None), row(2, 11_000_000_000, 300, None), row(3, 12_000_000_000, 300, None)];
        let edges = [
            Edge { arrival_ns: 10_000_000_000 - 120_000 },    // 120 us before the stamp
            Edge { arrival_ns: 11_000_000_000 + 40_000 },     // woke 40 us after the stamp
            Edge { arrival_ns: 12_000_000_000 - 90_000_000 }, // too early to be this press's
        ];
        assert_eq!(usb_to_evdev_us(&rows, &edges), vec![120, -40]);
    }

    #[test]
    fn the_report_has_percentiles_a_histogram_and_says_when_a_stage_is_missing() {
        let rows: Vec<Row> = (1..=100).map(|i| row(i, i * 1_000_000, (i * 10) as u32, None)).collect();
        let text = render(&rows, None, Some(30.0));
        assert!(text.starts_with("100 presses in 30 s;"), "{text}");
        let handoff = text.lines().find(|l| l.starts_with("evdev stamp to handoff")).unwrap();
        assert_eq!(handoff.split_whitespace().skip(4).collect::<Vec<_>>(), ["0.50", "0.90", "0.99", "1.00"], "{text}");
        assert!(text.contains("no send stamps"), "{text}");
        assert!(text.contains("< 0.25 ms"), "{text}");
        assert!(render(&[], None, None).starts_with("no presses recorded"));
        let with_send: Vec<Row> = rows.iter().map(|r| Row { send_us: Some(r.handoff_us + 100), ..*r }).collect();
        assert!(render(&with_send, Some(&[100, 50, -20]), None).contains("(3 of 100 matched)"));
    }

    #[test]
    fn the_hid_tap_reads_a_fifo_like_the_debugfs_file_and_stamps_arrivals() {
        let dir = std::env::temp_dir().join(format!("gfn-hid-tap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fifo = dir.join("events");
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let keep_open = std::fs::OpenOptions::new().read(true).write(true).open(&fifo).unwrap();
        let until = mono_ns() + 600_000_000;
        let path = fifo.clone();
        let reader = std::thread::spawn(move || collect_hid(&path, until).unwrap());
        std::thread::sleep(Duration::from_millis(100));
        let before = mono_ns();
        (&keep_open).write_all(report(0x08, 0, 0).as_bytes()).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let press_at = mono_ns();
        (&keep_open).write_all(report(0x28, 0, 0).as_bytes()).unwrap();
        let edges = reader.join().unwrap();
        assert_eq!(edges.len(), 1);
        assert!(edges[0].arrival_ns >= press_at && edges[0].arrival_ns < press_at + 50_000_000, "{} vs {press_at} ({before})", edges[0].arrival_ns);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_collects_the_window_and_prints_the_report() {
        let dir = std::env::temp_dir().join(format!("gfn-latency-run-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // two presses inside the window (written as the engine would, a moment after), one before it
        let now = mono_ns();
        let early =
            crate::latency::Done { id: 1, slot: 0, buttons: 1, kernel_ns: now - 5_000_000_000, wake_us: 10, pushed_us: 20, handoff_us: 30, send_us: None };
        let mk = move |id, at: u64| crate::latency::Done { id, kernel_ns: at, handoff_us: 400, ..early };
        let writer_dir = dir.clone();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            let text = format!("{}\n{}\n{}\n{}\n", crate::latency::Done::TSV_HEADER, early.tsv(), mk(2, mono_ns()).tsv(), mk(3, mono_ns()).tsv());
            std::fs::write(writer_dir.join("latency.tsv"), text).unwrap();
        });
        let mut out = Vec::new();
        let args: Vec<String> = ["run", "--dir", dir.to_str().unwrap(), "--seconds", "1"].iter().map(|s| s.to_string()).collect();
        assert_eq!(main(&args, &mut out), 0);
        writer.join().unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("2 presses in 1 s"), "{text}");
        let mut out = Vec::new();
        let args: Vec<String> = ["report", "--dir", dir.to_str().unwrap(), "--last", "1"].iter().map(|s| s.to_string()).collect();
        assert_eq!(main(&args, &mut out), 0);
        assert!(String::from_utf8(out).unwrap().starts_with("1 presses;"));
        assert_eq!(main(&["nope".to_owned()], &mut Vec::new()), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
