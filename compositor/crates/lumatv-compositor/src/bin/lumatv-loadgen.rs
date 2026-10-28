//! `lumatv-loadgen`: the background a TV has while Home is being used, as synthetic load for the frame gate (LPC 2026: the Xiaomi
//! study measured scrolling with a camera watch, BLE, a plugin sync, an update download and health checks running, not on a quiet
//! device). Four workloads, each on a thread of its own, each of them what the real thing costs the CPU and the kernel's locks rather
//! than a copy of it:
//!
//! - `decode`: a camera-like video decode. Every 33 ms a burst of `--decode-ms` (default 8) of work that reads and writes a 1080p
//!   YUV frame's worth of memory (3 MiB) a few times, the 30 fps of an H.264 camera on the TV's cores and memory bus.
//! - `ws`: Home Assistant's websocket chatter. A loopback TCP pair: a 400-byte JSON-like message every 25 ms, built, sent, read,
//!   scanned and answered, the wake-ups and the socket and softirq work of a busy state stream.
//! - `ble`: the Bluetooth scan stand-in. 20 times a second a 300 us burst and a loopback datagram: many small wake-ups.
//! - `download`: a big download to `--tmp`, at the lowest priority the kernel has (SCHED_IDLE, idle I/O class), over a loopback TCP
//!   stream at about 16 MiB/s: the page cache, the writeback and the filesystem locks, in a directory that is tmpfs on the TV, so the
//!   size is bounded by `--download-mib` (default 64) and by a quarter of what is free, and the file is removed when it ends.
//!
//! `lumatv-loadgen run SECONDS [--only decode,ws,ble,download] [--decode-ms N] [--download-mib N] [--tmp DIR]` runs for SECONDS (or until
//! SIGTERM, which cleans up) and prints one `summary:` line of what each did. `--selftest` is the host's one-second pass.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const FRAME_BYTES: usize = 1920 * 1080 * 3 / 2;
/// The download's rate: a good ADSL line, an update fetched slowly in the background.
const DOWNLOAD_BYTES_PER_SEC: u64 = 16 << 20;
const CHUNK: usize = 1 << 20;

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_term(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

#[derive(Default)]
struct Counters {
    decode_frames: AtomicU64,
    ws_messages: AtomicU64,
    ble_wakes: AtomicU64,
    download_bytes: AtomicU64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Options {
    seconds: u64,
    decode: bool,
    ws: bool,
    ble: bool,
    download: bool,
    decode_ms: u64,
    download_mib: u64,
    tmp: PathBuf,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options { seconds: 0, decode: true, ws: true, ble: true, download: true, decode_ms: 8, download_mib: 64, tmp: PathBuf::from("/tmp") };
    let mut it = args.iter();
    match it.next().map(String::as_str) {
        Some("run") => {}
        Some("--selftest") => {
            o.seconds = 1;
            return Ok(o);
        }
        _ => return Err(USAGE.into()),
    }
    o.seconds = it.next().and_then(|s| s.parse().ok()).ok_or(USAGE)?;
    while let Some(a) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--only" => {
                let v = value()?.clone();
                let wanted: Vec<&str> = v.split(',').collect();
                for w in &wanted {
                    if !["decode", "ws", "ble", "download"].contains(w) {
                        return Err(format!("unknown workload {w}: decode, ws, ble, download"));
                    }
                }
                (o.decode, o.ws, o.ble, o.download) =
                    (wanted.contains(&"decode"), wanted.contains(&"ws"), wanted.contains(&"ble"), wanted.contains(&"download"));
            }
            "--decode-ms" => o.decode_ms = value()?.parse().map_err(|_| "--decode-ms")?,
            "--download-mib" => o.download_mib = value()?.parse().map_err(|_| "--download-mib")?,
            "--tmp" => o.tmp = PathBuf::from(value()?),
            other => return Err(format!("unknown option {other}\n{USAGE}")),
        }
    }
    Ok(o)
}

const USAGE: &str = "lumatv-loadgen run SECONDS [--only decode,ws,ble,download] [--decode-ms N] [--download-mib N] [--tmp DIR] | --selftest";

/// Sleeps until `at`, in slices that notice a stop.
fn sleep_until(at: Instant) {
    while !STOP.load(Ordering::Relaxed) {
        let now = Instant::now();
        if now >= at {
            return;
        }
        std::thread::sleep((at - now).min(Duration::from_millis(50)));
    }
}

/// Runs `body` every `period` until stopped or `end`, without drifting.
fn every(period: Duration, end: Instant, mut body: impl FnMut()) {
    let mut next = Instant::now() + period;
    while !STOP.load(Ordering::Relaxed) && Instant::now() < end {
        sleep_until(next);
        if STOP.load(Ordering::Relaxed) {
            return;
        }
        body();
        next += period;
        let now = Instant::now();
        if next < now {
            next = now;
        }
    }
}

fn spin_work(buf: &mut [u8], budget: Duration) -> u64 {
    let t0 = Instant::now();
    let mut sum = 0u64;
    let mut seed = buf[0] as u64 | 1;
    while t0.elapsed() < budget {
        // A pass over the frame: read every 64th byte (a cache line), write one back: memory-bound, as a decoder's loop filter is.
        for chunk in buf.chunks_mut(4096) {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            chunk[(seed >> 52) as usize % chunk.len()] = seed as u8;
            sum = sum.wrapping_add(chunk[0] as u64 + chunk[chunk.len() / 2] as u64);
            if t0.elapsed() >= budget {
                break;
            }
        }
    }
    sum
}

fn decode(o: &Options, c: Arc<Counters>, end: Instant) {
    let mut frame = vec![0x80u8; FRAME_BYTES];
    let budget = Duration::from_millis(o.decode_ms);
    every(Duration::from_millis(33), end, || {
        std::hint::black_box(spin_work(&mut frame, budget));
        c.decode_frames.fetch_add(1, Ordering::Relaxed);
    });
}

fn message(n: u64) -> String {
    format!(
        "{{\"id\":{n},\"type\":\"event\",\"event\":{{\"event_type\":\"state_changed\",\"data\":{{\"entity_id\":\"sensor.room_{}\",\"new_state\":{{\"state\":\"{}\",\"attributes\":{{\"unit_of_measurement\":\"W\",\"friendly_name\":\"Room {}\"}}}}}}}}}}\n",
        n % 40,
        n * 7 % 1000,
        n % 40
    )
}

fn ws(c: Arc<Counters>, end: Instant) -> std::io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let echo = std::thread::spawn(move || {
        let Ok((mut s, _)) = listener.accept() else { return };
        let _ = s.set_read_timeout(Some(Duration::from_millis(200)));
        let mut buf = [0u8; 2048];
        while !STOP.load(Ordering::Relaxed) {
            match s.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    // The reply: the text scanned for its entity, as a client would, and a short acknowledgement sent back.
                    let found = buf[..n].windows(9).filter(|w| *w == b"entity_id").count();
                    if s.write_all(format!("{{\"ack\":{found}}}\n").as_bytes()).is_err() {
                        break;
                    }
                }
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(_) => break,
            }
        }
    });
    let mut s = TcpStream::connect(addr)?;
    s.set_nodelay(true)?;
    s.set_read_timeout(Some(Duration::from_millis(200)))?;
    let mut n = 0u64;
    let mut buf = [0u8; 256];
    every(Duration::from_millis(25), end, || {
        n += 1;
        if s.write_all(message(n).as_bytes()).is_ok() && s.read(&mut buf).is_ok() {
            c.ws_messages.fetch_add(1, Ordering::Relaxed);
        }
    });
    drop(s);
    let _ = echo.join();
    Ok(())
}

fn ble(c: Arc<Counters>, end: Instant) -> std::io::Result<()> {
    let a = UdpSocket::bind("127.0.0.1:0")?;
    let b = UdpSocket::bind("127.0.0.1:0")?;
    a.connect(b.local_addr()?)?;
    b.set_nonblocking(true)?;
    let mut scratch = vec![0u8; 4096];
    let mut buf = [0u8; 64];
    every(Duration::from_millis(50), end, || {
        std::hint::black_box(spin_work(&mut scratch, Duration::from_micros(300)));
        let _ = a.send(&[0xbe; 31]);
        let _ = b.recv(&mut buf);
        c.ble_wakes.fetch_add(1, Ordering::Relaxed);
    });
    Ok(())
}

/// The lowest priority there is: SCHED_IDLE for the thread, the idle I/O class for its disk work.
fn be_idle() {
    let param = libc::sched_param { sched_priority: 0 };
    // SAFETY: a stack sched_param; pid 0 is the calling thread.
    unsafe { libc::sched_setscheduler(0, libc::SCHED_IDLE, &param) };
    // ioprio_set(IOPRIO_WHO_PROCESS, this thread, IOPRIO_CLASS_IDLE << 13).
    // SAFETY: a raw syscall with plain integers.
    unsafe { libc::syscall(libc::SYS_ioprio_set, 1, 0, 3 << 13) };
}

/// The most the download may write: the asked size, and a quarter of what `dir`'s filesystem has free.
fn download_cap(dir: &Path, asked_mib: u64) -> u64 {
    let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes()) else { return 0 };
    // SAFETY: a stack statvfs for the kernel to fill.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: c is a NUL-terminated path.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return 0;
    }
    let free = st.f_bavail as u64 * st.f_frsize as u64;
    (asked_mib << 20).min(free / 4)
}

fn download(o: &Options, c: Arc<Counters>, end: Instant) -> std::io::Result<()> {
    let total = download_cap(&o.tmp, o.download_mib);
    if total == 0 {
        return Err(std::io::Error::other(format!("{} has no room for a download", o.tmp.display())));
    }
    let path = o.tmp.join(format!("lumatv-loadgen-{}.part", std::process::id()));
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let file_path = path.clone();
    let cc = c.clone();
    let receiver = std::thread::spawn(move || -> std::io::Result<()> {
        be_idle();
        let (mut s, _) = listener.accept()?;
        let mut f = std::fs::File::create(&file_path)?;
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = s.read(&mut buf)?;
            if n == 0 {
                return Ok(());
            }
            f.write_all(&buf[..n])?;
            cc.download_bytes.fetch_add(n as u64, Ordering::Relaxed);
        }
    });
    let sender = std::thread::spawn(move || -> std::io::Result<()> {
        be_idle();
        let mut s = TcpStream::connect(addr)?;
        let chunk = vec![0x5au8; CHUNK];
        let mut sent = 0u64;
        let t0 = Instant::now();
        while sent < total && !STOP.load(Ordering::Relaxed) && Instant::now() < end {
            let n = CHUNK.min((total - sent) as usize);
            s.write_all(&chunk[..n])?;
            sent += n as u64;
            // Held to the line's rate: this much by now, no more.
            let due = Duration::from_secs_f64(sent as f64 / DOWNLOAD_BYTES_PER_SEC as f64);
            if let Some(wait) = due.checked_sub(t0.elapsed()) {
                std::thread::sleep(wait.min(Duration::from_millis(500)));
            }
        }
        Ok(())
    });
    let sent = sender.join().map_err(|_| std::io::Error::other("download sender panicked"))?;
    let got = receiver.join().map_err(|_| std::io::Error::other("download receiver panicked"))?;
    let _ = std::fs::remove_file(&path);
    sent.and(got)
}

fn run(o: &Options) -> String {
    let counters = Arc::new(Counters::default());
    let end = Instant::now() + Duration::from_secs(o.seconds);
    let mut threads: Vec<(&str, JoinHandle<Option<String>>)> = Vec::new();
    let mut spawn = |name: &'static str, f: Box<dyn FnOnce() -> Option<String> + Send>| {
        threads.push((name, std::thread::Builder::new().name(format!("loadgen-{name}")).spawn(f).expect("a thread")));
    };
    if o.decode {
        let (o2, c) = (o.clone(), counters.clone());
        spawn(
            "decode",
            Box::new(move || {
                decode(&o2, c, end);
                None
            }),
        );
    }
    if o.ws {
        let c = counters.clone();
        spawn("ws", Box::new(move || ws(c, end).err().map(|e| e.to_string())));
    }
    if o.ble {
        let c = counters.clone();
        spawn("ble", Box::new(move || ble(c, end).err().map(|e| e.to_string())));
    }
    if o.download {
        let (o2, c) = (o.clone(), counters.clone());
        spawn("download", Box::new(move || download(&o2, c, end).err().map(|e| e.to_string())));
    }
    let mut problems = Vec::new();
    for (name, t) in threads {
        if let Some(e) = t.join().unwrap_or_else(|_| Some("panicked".into())) {
            problems.push(format!("{name}: {e}"));
        }
    }
    format!(
        "summary: decode {} frames, ws {} messages, ble {} wake-ups, download {} MiB{}",
        counters.decode_frames.load(Ordering::Relaxed),
        counters.ws_messages.load(Ordering::Relaxed),
        counters.ble_wakes.load(Ordering::Relaxed),
        counters.download_bytes.load(Ordering::Relaxed) >> 20,
        if problems.is_empty() { String::new() } else { format!("; problems: {}", problems.join("; ")) }
    )
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let o = match parse(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    // SAFETY: a handler that only stores to an atomic.
    unsafe {
        libc::signal(libc::SIGTERM, on_term as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_term as *const () as libc::sighandler_t);
    }
    println!("{}", run(&o));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn the_command_line_picks_workloads_and_sizes() {
        let o = parse(&args(&["run", "30"])).unwrap();
        assert_eq!((o.seconds, o.decode, o.ws, o.ble, o.download, o.decode_ms, o.download_mib), (30, true, true, true, true, 8, 64));
        let o = parse(&args(&["run", "5", "--only", "ws,ble", "--decode-ms", "12", "--download-mib", "8", "--tmp", "/var/tmp"])).unwrap();
        assert_eq!((o.decode, o.ws, o.ble, o.download, o.decode_ms, o.download_mib, o.tmp.to_str()), (false, true, true, false, 12, 8, Some("/var/tmp")));
        assert!(parse(&args(&["run", "5", "--only", "gpu"])).unwrap_err().contains("unknown workload gpu"));
        assert!(parse(&args(&["run"])).is_err() && parse(&args(&["go", "5"])).is_err() && parse(&args(&["run", "5", "--what"])).is_err());
    }

    #[test]
    fn the_download_never_asks_for_more_than_a_quarter_of_what_is_free() {
        let dir = std::env::temp_dir();
        let cap = download_cap(&dir, 1 << 30);
        assert!(cap > 0 && cap < (1u64 << 30) << 20, "a petabyte is not asked for: {cap}");
        assert_eq!(download_cap(&dir, 3), 3 << 20, "a small ask is kept");
        assert_eq!(download_cap(Path::new("/nonexistent/place"), 3), 0);
    }

    #[test]
    fn a_short_run_does_some_of_each_and_leaves_no_file_behind() {
        let dir = std::env::temp_dir().join(format!("lumatv-loadgen-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let o = Options { seconds: 1, decode: true, ws: true, ble: true, download: true, decode_ms: 4, download_mib: 4, tmp: dir.clone() };
        STOP.store(false, Ordering::SeqCst);
        let line = run(&o);
        let n = |what: &str| -> u64 {
            let rest = line.split(what).next().unwrap();
            rest.split_whitespace().last().unwrap().parse().unwrap()
        };
        assert!(n(" frames") >= 20, "{line}");
        assert!(n(" messages") >= 20, "{line}");
        assert!(n(" wake-ups") >= 10, "{line}");
        assert!(!line.contains("problems"), "{line}");
        assert!(line.contains("download 4 MiB"), "the 4 MiB of the download, in the second: {line}");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "the download's file is removed");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
