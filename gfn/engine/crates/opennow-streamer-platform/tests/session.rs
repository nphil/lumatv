//! The runtime against the real feeder (gfn/feeder, host build) with the NDL stand-in, and a pad on a pseudo-terminal standing in for
//! an evdev node. Requires `sh gfn/feeder/build.sh` first. The session answers SDR and the stream's own colour tags decide the TV's
//! HDR mode: PQ on the first keyframe, then a BT.709 one that loads the feeder again to leave HDR, then HLG.
use std::ffi::CString;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use opennow_streamer_platform::{CapturedInput, DecodeTimingsReport, EncodedFrame, MediaCodec, MediaFeedback, PushOutcome, controller_rumble, create_runtime};

mod common;
use common::{SPS_BT709, SPS_HLG, SPS_PQ, hevc_unit};

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

/// What the engine has been told so far.
#[derive(Default)]
struct Told {
    accepted: Vec<(Option<u32>, bool)>,
    playing: bool,
    /// The latest status of the feeder's second.
    timings: Option<DecodeTimingsReport>,
}

impl Told {
    /// Takes feedback until `done` holds, six seconds at most; whether it does.
    fn until(&mut self, rx: &mpsc::Receiver<MediaFeedback>, done: impl Fn(&Told) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(6);
        while !done(self) && Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(MediaFeedback::VideoFrameAccepted { frame_index, keyframe, .. }) => self.accepted.push((frame_index, keyframe)),
                Ok(MediaFeedback::PlaybackStarted { backend }) => {
                    assert_eq!(backend, "ndl");
                    self.playing = true;
                }
                Ok(MediaFeedback::DecodeTimings(t)) => self.timings = Some(t),
                Ok(MediaFeedback::OutputError { message }) => panic!("output error: {message}"),
                Ok(_) | Err(_) => {}
            }
        }
        done(self)
    }
}

#[test]
fn a_session_feeds_the_real_feeder_and_pads_reach_the_queue() {
    let tmp = std::env::temp_dir().join(format!("gfn-session-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    let pty = fake_pad(&tmp);
    let ndl_log = common::feeder_env(&tmp);
    unsafe { std::env::set_var("LUMATV_GFN_DROP_THRESHOLD", "2") };
    let (host, runtime) = create_runtime().unwrap();
    let (tx, rx) = mpsc::channel();
    let stream = common::stream(false);
    let session = runtime.start_with_backend(tx, stream, "auto").expect("session");
    let sink = session.sink();
    let audio = |i: u32| {
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
    };
    let mut told = Told::default();
    // Ten units under each parameter set: PQ, BT.709, HLG. A group is accepted whole before the next begins, so no unit waits for its release
    // when the BT.709 keyframe loads the feeder again. Audio goes in while a pipeline plays: the first group's once the player started, the
    // rest once a status says the pipeline that was loaded again plays.
    for (group, sps) in [SPS_PQ, SPS_BT709, SPS_HLG].into_iter().enumerate() {
        let first = group as u32 * 10;
        for i in first..first + 10 {
            let key = i == first;
            let outcome = sink.push(EncodedFrame {
                mid: "0".to_owned(),
                codec: MediaCodec::H265,
                data: Arc::from(hevc_unit(key, if key { 4000 } else { 300 }, sps)),
                frame_index: Some(i),
                timestamp: u64::from(i) * 750,
                clock_rate_hz: 90_000,
                keyframe: key,
                contiguous: true,
                ssrc: Some(7),
            });
            assert_eq!(outcome, PushOutcome::Queued);
            if i == 0 {
                assert!(told.until(&rx, |t| t.playing), "no PlaybackStarted");
            }
            if group == 0 && i % 3 == 0 {
                audio(i);
            }
        }
        assert!(told.until(&rx, |t| t.accepted.len() == first as usize + 10), "group {group} was not accepted whole: {:?}", told.accepted);
    }
    /* the pad: a press of Cross with the left stick up, then the PS button */
    write_events(&pty, &[event(1, 0x130, 1), event(3, 1, 0), event(0, 0, 0)]);
    write_events(&pty, &[event(1, 0x13c, 1), event(0, 0, 0), event(1, 0x13c, 0), event(0, 0, 0)]);
    // the feeder counts from zero at the reload, so a status of 20 units is one of the pipeline that was loaded again
    let plays_again = |t: &Told| t.timings.is_some_and(|s| s.outputs_total >= 20 && s.last_output_at.is_some());
    assert!(told.until(&rx, plays_again), "no status of the pipeline that was loaded again: {:?}", told.timings);
    for i in (12..30).step_by(3) {
        audio(i);
    }
    assert_eq!(told.accepted.len(), 30, "{:?}", told.accepted);
    assert_eq!(told.accepted[0], (Some(0), true));
    assert_eq!(told.accepted[1], (Some(1), false));
    assert_eq!(told.timings.unwrap().epoch, 1);
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
    let lines: Vec<&str> = log.lines().collect();
    let at = |what: &str| -> Vec<usize> { lines.iter().enumerate().filter(|(_, line)| line.contains(what)).map(|(n, _)| n).collect() };
    let loads = at("Load 3840x2160 type 2 audio OPUS 2 ch 48.0 kHz");
    assert_eq!(loads.len(), 2, "the load of the session, and the one the BT.709 keyframe made to leave HDR: {log}");
    let hdr_info = |vui: &str| format!("SetHDRInfo primaries 13250,34500 7500,3000 34000,16000 white 15635,16450 lum 10000000,1 cll 1000,400 vui {vui}");
    let applied = at("SetHDRInfo");
    assert_eq!(applied.len(), 2, "{log}");
    assert!(lines[applied[0]].contains(&hdr_info("16,9,9")), "PQ puts the TV in HDR10 although the session answered SDR: {log}");
    assert!(lines[applied[1]].contains(&hdr_info("18,9,9")), "HLG puts it in HDR again after the reload, and the BT.709 keyframe sent nothing: {log}");
    assert!(applied[0] < loads[1] && loads[1] < applied[1], "the reload sits between the two: {log}");
    assert!(log.contains("SetFrameDropThreshold 2"), "{log}");
    assert_eq!(log.lines().filter(|line| line.contains(" Play #")).count(), 30, "{log}");
    assert_eq!(log.matches("AudioPlay #").count(), 10, "{log}");
    assert!(log.contains("Unload"), "{log}");
    assert!(!log.contains("VIOLATION"), "{log}");
    // the stand-in numbers pts from zero again with each load
    let mut loaded_units: Vec<Vec<i64>> = Vec::new();
    for line in &lines {
        if line.contains("Load 3840x2160") {
            loaded_units.push(Vec::new());
        } else if line.contains(" Play #") {
            let pts = line.split(" pts ").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap();
            loaded_units.last_mut().expect("a unit after a load").push(pts);
        }
    }
    assert_eq!(loaded_units.iter().map(Vec::len).collect::<Vec<_>>(), [10, 20], "{log}");
    assert!(loaded_units.iter().all(|pts| pts.windows(2).all(|w| w[1] >= w[0]) && pts.last().unwrap() < &1000), "{loaded_units:?}");
    unsafe { libc::close(pty.master) };
    let _ = CString::new("");
    let _ = fs::remove_dir_all(&tmp);
}
