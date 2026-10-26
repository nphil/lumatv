//! The runtime against the real feeder (gfn/feeder, host build) with the NDL stand-in, and a pad on a pseudo-terminal standing in for
//! an evdev node. Needs `sh gfn/feeder/build.sh` first; without the binaries the test passes vacuously and says so.
use std::ffi::CString;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use opennow_streamer_platform::{
    CapturedInput, EncodedFrame, MediaCodec, MediaColorQuality, MediaFeedback, MediaStreamConfig, MediaVideoCodec, PushOutcome,
    StreamShortcutBindings, controller_rumble, create_runtime,
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().unwrap()
}

fn hevc_unit(key: bool, size: usize) -> Vec<u8> {
    let mut u = vec![0, 0, 0, 1, if key { 19 << 1 } else { 1 << 1 }, 1, 0x80];
    u.resize(size, 0x5a);
    if key {
        let mut sps = vec![0, 0, 0, 1, 33 << 1, 1, 0x01, 0x02];
        sps.extend_from_slice(&u);
        return sps;
    }
    u
}

struct Pty {
    master: i32,
}

/// A pad node: a pseudo-terminal's slave linked into a fake /dev/input, with sysfs facts beside it.
fn fake_pad(tmp: &Path) -> Pty {
    let dev = tmp.join("dev/input");
    let sys = tmp.join("sys/class/input/event7/device");
    fs::create_dir_all(&dev).unwrap();
    fs::create_dir_all(sys.join("capabilities")).unwrap();
    fs::write(sys.join("name"), "Wireless Controller\n").unwrap();
    fs::write(sys.join("uniq"), "1c:a0:b8:4c:7a:99\n").unwrap();
    fs::write(sys.join("capabilities/ev"), "20001b\n").unwrap();                 /* SYN KEY ABS MSC FF */
    let mut key = vec!["0"; 21];
    key[21 - 1 - 4] = "7fdb000000000000";                                      /* word 4: BTN_SOUTH.. (0x130 = 64*4 + 48) */
    fs::write(sys.join("capabilities/key"), key.join(" ") + "\n").unwrap();
    fs::write(sys.join("capabilities/abs"), "3003f\n").unwrap();
    fs::write(sys.join("capabilities/ff"), "1 7030000 0 0\n").unwrap();
    let mut master = 0;
    let mut slave = 0;
    let rc = unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null(), std::ptr::null()) };
    assert_eq!(rc, 0, "openpty");
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        libc::tcgetattr(slave, &mut t);
        libc::cfmakeraw(&mut t);
        libc::tcsetattr(slave, libc::TCSANOW, &t);
        libc::tcgetattr(master, &mut t);
        libc::cfmakeraw(&mut t);
        libc::tcsetattr(master, libc::TCSANOW, &t);
    }
    let name = unsafe { std::ffi::CStr::from_ptr(libc::ptsname(master)) }.to_string_lossy().into_owned();
    symlink(&name, dev.join("event7")).unwrap();
    unsafe { libc::close(slave) };
    Pty { master }
}

fn event(kind: u16, code: u16, value: i32) -> [u8; 24] {
    let mut b = [0u8; 24];
    b[16..18].copy_from_slice(&kind.to_ne_bytes());
    b[18..20].copy_from_slice(&code.to_ne_bytes());
    b[20..24].copy_from_slice(&value.to_ne_bytes());
    b
}

fn write_events(pty: &Pty, events: &[[u8; 24]]) {
    let bytes: Vec<u8> = events.iter().flat_map(|e| e.iter().copied()).collect();
    let n = unsafe { libc::write(pty.master, bytes.as_ptr() as *const libc::c_void, bytes.len()) };
    assert_eq!(n as usize, bytes.len());
}

#[test]
fn a_session_feeds_the_real_feeder_and_pads_reach_the_queue() {
    let root = root();
    let feeder = root.join("gfn/feeder/build/host/lumatv-gfn-feeder");
    let lib_dir = root.join("gfn/feeder/build/host");
    if !feeder.exists() || !lib_dir.join("libNDL_directmedia.so.1").exists() {
        eprintln!("skipped: build the feeder first (sh gfn/feeder/build.sh)");
        return;
    }
    let tmp = std::env::temp_dir().join(format!("gfn-session-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    let ndl_log = tmp.join("ndl.log");
    let pty = fake_pad(&tmp);
    let sock = tmp.join("feed.sock");
    unsafe {
        std::env::set_var("LUMATV_GFN_FEEDER_SOCKET", &sock);
        std::env::set_var("LUMATV_GFN_FEEDER_SPAWN", &feeder);
        std::env::set_var("LUMATV_GFN_FEEDER_ARGS", format!("--once --window-id _Window_Id_42 -L {}", lib_dir.join("libNDL_directmedia.so.1").display()));
        std::env::set_var("LD_LIBRARY_PATH", &lib_dir);
        std::env::set_var("FAKE_NDL_LOG", &ndl_log);
        std::env::set_var("FAKE_NDL_VERBOSE", "1");
        std::env::set_var("LUMATV_GFN_PAD_DEV", tmp.join("dev/input"));
        std::env::set_var("LUMATV_GFN_PAD_SYS", tmp.join("sys/class/input"));
        std::env::set_var("LUMATV_GFN_DROP_THRESHOLD", "2");
    }
    let (host, runtime) = create_runtime().unwrap();
    let (tx, rx) = mpsc::channel();
    let stream = MediaStreamConfig {
        codec: MediaVideoCodec::H265,
        color_quality: MediaColorQuality::TenBit420,
        hdr: true,
        width: 3840,
        height: 2160,
        fps: 120,
        bitrate_bps: 80_000_000,
        cloud_gsync: false,
        shortcuts: StreamShortcutBindings::default(),
    };
    let session = runtime.start_with_backend(tx, stream, "auto").expect("session");
    let sink = session.sink();
    for i in 0..30u32 {
        let key = i % 10 == 0;
        let outcome = sink.push(EncodedFrame {
            mid: "0".to_owned(),
            codec: MediaCodec::H265,
            data: Arc::from(hevc_unit(key, if key { 4000 } else { 300 })),
            frame_index: Some(i),
            timestamp: u64::from(i) * 750,
            clock_rate_hz: 90_000,
            keyframe: key,
            contiguous: true,
            ssrc: Some(7),
        });
        assert_eq!(outcome, PushOutcome::Queued);
        if i % 3 == 0 {
            let outcome = sink.push(EncodedFrame {
                mid: "1".to_owned(),
                codec: MediaCodec::Opus { channels: 2 },
                data: Arc::from(vec![0xfc, 0x01, 0, 0, 0]),
                frame_index: None,
                timestamp: u64::from(i) * 960,
                clock_rate_hz: 48_000,
                keyframe: false,
                contiguous: true,
                ssrc: Some(8),
            });
            assert_eq!(outcome, PushOutcome::Queued);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    /* the pad: a press of Cross with the left stick up, then the PS button */
    write_events(&pty, &[event(1, 0x130, 1), event(3, 1, 0), event(0, 0, 0)]);
    write_events(&pty, &[event(1, 0x13c, 1), event(0, 0, 0), event(1, 0x13c, 0), event(0, 0, 0)]);
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut accepted = Vec::new();
    let mut playing = false;
    let mut timings = None;
    while Instant::now() < deadline && (accepted.len() < 30 || !playing || timings.is_none()) {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(MediaFeedback::VideoFrameAccepted { frame_index, keyframe, .. }) => accepted.push((frame_index, keyframe)),
            Ok(MediaFeedback::PlaybackStarted { backend }) => {
                assert_eq!(backend, "ndl");
                playing = true;
            }
            Ok(MediaFeedback::DecodeTimings(t)) => {
                if t.outputs_total >= 30 {
                    timings = Some(t);
                }
            }
            Ok(MediaFeedback::OutputError { message }) => panic!("output error: {message}"),
            Ok(_) | Err(_) => {}
        }
    }
    assert!(playing, "no PlaybackStarted");
    assert_eq!(accepted.len(), 30, "{accepted:?}");
    assert_eq!(accepted[0], (Some(0), true));
    assert_eq!(accepted[1], (Some(1), false));
    let t = timings.expect("a status with every unit counted");
    assert!(t.last_output_at.is_some());
    assert_eq!(t.epoch, 1);
    let queue = session.captured_input();
    let pad_deadline = Instant::now() + Duration::from_secs(3);
    let mut got = Vec::new();
    while Instant::now() < pad_deadline && got.len() < 3 {
        if let Some(input) = queue.take() {
            got.push(input);
        } else {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let report = got.iter().find(|i| matches!(i, CapturedInput::Gamepad { buttons: 0x1000, .. }));
    assert!(report.is_some(), "the pad's report did not reach the queue: {got:?}");
    if let Some(CapturedInput::Gamepad { controller_id, bitmap, left_stick_y, .. }) = report {
        assert_eq!((*controller_id, *bitmap), (0, 0x0101));
        assert!(*left_stick_y > 30_000, "stick up is positive: {left_stick_y}");
    }
    assert!(got.iter().any(|i| matches!(i, CapturedInput::Guide)), "{got:?}");
    /* rumble reaches the pad's descriptor (a pseudo-terminal refuses the effect upload, which is the answer we expect here) */
    let r = controller_rumble(0, 1000, 2000, 300);
    assert!(r.is_err(), "{r:?}");
    session.stop();
    runtime.shutdown();
    host.run();
    let log = fs::read_to_string(&ndl_log).unwrap_or_default();
    assert!(log.contains("SetWindowId _Window_Id_42"), "{log}");
    assert!(log.contains("Load 3840x2160 type 2 audio OPUS 2 ch 48.0 kHz"), "{log}");
    assert!(log.contains("SetHDRInfo primaries 13250,34500 7500,3000 34000,16000 white 15635,16450 lum 10000000,1 cll 1000,400 vui 16,9,9"), "{log}");
    assert!(log.contains("SetFrameDropThreshold 2"), "{log}");
    assert_eq!(log.matches("Play #").count(), 30, "{log}");
    assert_eq!(log.matches("AudioPlay #").count(), 10, "{log}");
    assert!(log.contains("Unload"), "{log}");
    assert!(!log.contains("VIOLATION"), "{log}");
    let pts: Vec<i64> = log.lines().filter(|l| l.contains(" Play #")).map(|l| l.split(" pts ").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap()).collect();
    assert!(pts.windows(2).all(|w| w[1] > w[0]), "{pts:?}");
    unsafe { libc::close(pty.master) };
    let _ = CString::new("");
    let _ = fs::remove_dir_all(&tmp);
}
