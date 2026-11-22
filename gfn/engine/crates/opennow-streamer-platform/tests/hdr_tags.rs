//! A stream whose own colour tags say SDR never puts the TV in HDR mode, whatever the session answered: the real feeder is loaded with
//! HDR armed and no metadata, and BT.709 keyframes leave the NDL stand-in without a `SetHDRInfo`. Its own file because the runtime is
//! process-global (tests/session.rs has the HDR10 and HLG streams). Requires `sh gfn/feeder/build.sh` first.
use std::fs;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use opennow_streamer_platform::{EncodedFrame, MediaCodec, MediaFeedback, PushOutcome, create_runtime};

mod common;

#[test]
fn bt709_tags_keep_the_tv_in_sdr_when_the_session_answered_hdr() {
    let tmp = std::env::temp_dir().join(format!("gfn-sdr-tags-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(tmp.join("dev/input")).unwrap();
    fs::create_dir_all(tmp.join("sys/class/input")).unwrap();
    let ndl_log = common::feeder_env(&tmp);
    let (host, runtime) = create_runtime().unwrap();
    let (tx, rx) = mpsc::channel();
    let session = runtime.start_with_backend(tx, common::stream(true), "auto").expect("session");
    let sink = session.sink();
    for i in 0..12u32 {
        let key = i % 10 == 0;
        let outcome = sink.push(EncodedFrame {
            mid: "0".to_owned(),
            codec: MediaCodec::H265,
            data: Arc::from(common::hevc_unit(key, if key { 4000 } else { 300 }, common::SPS_BT709)),
            frame_index: Some(i),
            timestamp: u64::from(i) * 750,
            clock_rate_hz: 90_000,
            keyframe: key,
            contiguous: true,
            ssrc: Some(7),
        });
        assert_eq!(outcome, PushOutcome::Queued);
    }
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut accepted = 0;
    while accepted < 12 && Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(MediaFeedback::VideoFrameAccepted { .. }) => accepted += 1,
            Ok(MediaFeedback::OutputError { message }) => panic!("output error: {message}"),
            Ok(_) | Err(_) => {}
        }
    }
    assert_eq!(accepted, 12, "every unit reached the player");
    session.stop();
    runtime.shutdown();
    host.run();
    let log = fs::read_to_string(&ndl_log).unwrap_or_default();
    assert!(log.contains("Load 3840x2160 type 2"), "{log}");
    assert_eq!(log.lines().filter(|line| line.contains(" Play #")).count(), 12, "{log}");
    assert!(!log.contains("SetHDRInfo"), "SDR tags must not enter HDR mode: {log}");
    let _ = fs::remove_dir_all(&tmp);
}
