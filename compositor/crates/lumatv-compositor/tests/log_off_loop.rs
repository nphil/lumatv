//! Exercises the production datagram producer against a real Unix receiver, including an offline or saturated collector.
use log::{Level, Log, Record};
use lumatv_compositor::{childlog, logsink::{self, Logger}};
use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixDatagram;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

struct CountAlloc;
static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) { ALLOCATIONS.fetch_add(1, Ordering::Relaxed); }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) { unsafe { System.dealloc(ptr, layout) } }
}
#[global_allocator]
static ALLOCATOR: CountAlloc = CountAlloc;

fn record(logger: &Logger, level: Level, args: std::fmt::Arguments<'_>) {
    logger.log(&Record::builder().level(level).args(args).build());
}
fn receive(sink: &UnixDatagram) -> String {
    let mut data = [0; 8192];
    let n = sink.recv(&mut data).unwrap();
    assert!(n <= 4096);
    String::from_utf8(data[..n].to_vec()).expect("whole UTF-8 record")
}

#[test]
fn real_collector_wire_offline_full_utf8_and_readers() {
    assert!(!std::path::Path::new(logsink::SOCKET).exists(), "another collector owns the test socket");
    let logger = Logger::start();
    let missing = Instant::now();
    for _ in 0..1000 { record(&logger, Level::Info, format_args!("offline")); }
    logger.flush();
    let missing_elapsed = missing.elapsed();
    assert!(missing_elapsed < Duration::from_secs(2));

    let sink = UnixDatagram::bind(logsink::SOCKET).unwrap();
    sink.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    for (level, pri, name) in [(Level::Error, 131, "error"), (Level::Warn, 132, "warn"), (Level::Info, 134, "info"), (Level::Debug, 135, "debug"), (Level::Trace, 135, "trace")] {
        record(&logger, level, format_args!("state 42"));
        let line = receive(&sink);
        assert!(line.starts_with(&format!("<{pri}>lumatv-compositor: [srv ")));
        assert!(line.ends_with(&format!("] {name} state 42")));
        assert!(!line.contains('\n'));
    }
    let wide = "é🦀".repeat(2000);
    record(&logger, Level::Warn, format_args!("{wide}"));
    let cut = receive(&sink);
    assert!((4090..=4096).contains(&cut.len()), "{} bytes", cut.len());
    assert!(cut.ends_with('…'));
    record(&logger, Level::Info, format_args!("line\nreturn\r\0tab\tend"));
    assert!(receive(&sink).ends_with("line return  tab end"));

    ALLOCATIONS.store(0, Ordering::Relaxed);
    COUNTING.store(true, Ordering::Relaxed);
    record(&logger, Level::Info, format_args!("steady {}", 42));
    COUNTING.store(false, Ordering::Relaxed);
    assert_eq!(ALLOCATIONS.load(Ordering::Relaxed), 0, "steady-state formatting/sending allocates nothing");
    receive(&sink);

    let full = Instant::now();
    for _ in 0..20000 { record(&logger, Level::Warn, format_args!("full")); }
    logger.flush();
    let full_elapsed = full.elapsed();
    assert!(full_elapsed < Duration::from_secs(2), "a full collector blocked the producer");
    sink.set_nonblocking(true).unwrap();
    let mut buffer = [0; 4096];
    let mut queued = 0;
    while sink.recv(&mut buffer).is_ok() { queued += 1; }
    assert!(queued > 0 && queued < 20000, "a full socket drops records rather than retaining them");
    sink.set_nonblocking(false).unwrap();

    let fd = childlog::pipe("lumatv-shell-rust", "native: ").unwrap();
    assert_ne!(unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) } & libc::O_NONBLOCK, 0);
    let mut output = std::fs::File::from(fd);
    output.write_all(b"first\ninvalid \xff\npartial").unwrap();
    drop(output);
    assert!(receive(&sink).ends_with("native: first"));
    assert!(receive(&sink).ends_with("native: invalid �"));
    assert!(receive(&sink).ends_with("native: partial"));

    drop(sink);
    std::fs::remove_file(logsink::SOCKET).unwrap();
    record(&logger, Level::Info, format_args!("collector stopped"));
    let replacement = UnixDatagram::bind(logsink::SOCKET).unwrap();
    replacement.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    record(&logger, Level::Info, format_args!("collector restarted"));
    assert!(receive(&replacement).ends_with("collector restarted"));
    drop(replacement);
    std::fs::remove_file(logsink::SOCKET).unwrap();

    let directory = std::env::temp_dir().join(format!("lumatv-central-read-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("previous.log"), "LUMATV-LOG-V1\n<134>lumatv-compositor: older\n<134>lumatv-sm: wrong producer\n<134>lumatv-compositor-extra: wrong tag\n<999>lumatv-compositor: invalid priority\n").unwrap();
    std::fs::write(directory.join("current.log"), "LUMATV-LOG-V1\n<132>lumatv-compositor: newer\n<134>lumatv-tool: lumatv-compositor: embedded tag\n").unwrap();
    assert_eq!(logsink::tail_in(&directory, "lumatv-compositor", 10), ["older", "newer"]);
    assert_eq!(logsink::tail_in(&directory, "lumatv-compositor", 1), ["newer"]);
    assert!(logsink::tail_in(&directory, "lumatv-compositor", 0).is_empty());
    std::fs::remove_file(directory.join("previous.log")).unwrap();
    assert_eq!(logsink::tail_in(&directory, "lumatv-compositor", 10), ["newer"]);
    std::fs::remove_dir_all(directory).unwrap();
    println!("actual datagram producer: missing={missing_elapsed:?}, full={full_elapsed:?}, queued={queued}, allocations=0; UTF-8/control caps, restart, native drain and central readers passed");
}
