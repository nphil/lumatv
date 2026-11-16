//! The recorder's side of the encoder wire (`screencap::encoder`) against real child processes: the C program `venc/` builds for the host
//! (its fake kernel stands in for `/dev/venc`), and small scripts that misbehave the ways a child can: say nothing, say the wrong thing,
//! stop answering. Every wait on the child ends, with the child killed and a reason that names what happened.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use lumatv_compositor::output::{Image, monotonic_ns};
use lumatv_compositor::screencap::encoder::{EncoderFactory, EncoderSpec, VencFactory};
use lumatv_compositor::screencap::record::{Ended, Finished, RecordSpec, Recorder};

const SPEC: EncoderSpec = EncoderSpec { width: 640, height: 360, fps: 30, kbps: 800, gop: 5 };

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lumatv-screencap-venc-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A program that is a shell script.
fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    // A test thread that forks while the file is open for writing holds it busy until its child execs: running it waits that out.
    loop {
        match std::process::Command::new(&p).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => panic!("{}: {e}", p.display()),
            Ok(mut child) => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    p
}

fn factory(binary: PathBuf, _dir: &Path) -> VencFactory {
    VencFactory { binary }
}

fn require_host_venc() -> bool {
    std::env::var_os("LUMATV_REQUIRE_VENC_HOST").is_some()
}

fn host_venc() -> Option<PathBuf> {
    let p = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../venc/build/host/lumatv-venc"));
    if p.is_file() {
        Some(p)
    } else if require_host_venc() {
        panic!("{} is not built; run venc/build.sh --host-only", p.display());
    } else {
        eprintln!("skipped: {} is not built (venc/build.sh --host-only)", p.display());
        None
    }
}

fn picture(v: u8, width: u32, height: u32) -> Image {
    Image { width, height, rgba: [v, v, v, 255].repeat(width as usize * height as usize) }
}

fn spec_for(dir: &Path) -> RecordSpec {
    RecordSpec {
        encoder: SPEC,
        part: dir.join("rec.mp4.part"),
        path: dir.join("rec.mp4"),
        max_bytes: 64 << 20,
        max_seconds: 600,
        min_free: 0,
        start_ns: monotonic_ns(),
        keepalive: Duration::from_secs(30),
        space_check: Duration::from_secs(1),
    }
}

fn wait_finished(r: &Recorder, limit: Duration) -> Finished {
    let t = Instant::now();
    loop {
        if let Some(f) = r.snapshot().finished {
            return f;
        }
        assert!(t.elapsed() < limit, "the recording did not end");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn send_frame(rec: &Recorder, i: u64) -> Result<(), String> {
    let began = Instant::now();
    let (width, height) = (rec.spec().encoder.width, rec.spec().encoder.height);
    loop {
        // The spare vector comes back from the pipeline: the same bytes are drawn into again.
        let mut image = picture((i * 12) as u8, width, height);
        image.rgba = rec.spare().into_iter().chain(image.rgba).take(width as usize * height as usize * 4).collect();
        if rec.send(image, i * 33_333) {
            return Ok(());
        }
        let state = rec.snapshot();
        if let Some(finished) = &state.finished {
            return Err(format!("recording ended before frame {i}: {finished:?}"));
        }
        if began.elapsed() >= Duration::from_secs(10) {
            return Err(format!("timed out sending frame {i}: {state:?}"));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn fail_recording(rec: Recorder, reason: String) -> ! {
    if !rec.is_gone() {
        // Dropping joins the recorder, so let a stuck pipeline be cleaned up off the test thread.
        let _ = std::thread::spawn(move || drop(rec));
    }
    panic!("{reason}");
}

fn boxes(file: &[u8]) -> Vec<String> {
    let (mut v, mut i) = (Vec::new(), 0);
    while i + 8 <= file.len() {
        let size = u32::from_be_bytes(file[i..i + 4].try_into().unwrap()) as usize;
        v.push(String::from_utf8_lossy(&file[i + 4..i + 8]).into_owned());
        assert!(size >= 8 && i + size <= file.len());
        i += size;
    }
    v
}

#[test]
fn the_real_program_records_a_whole_file_through_the_wire() {
    let Some(venc) = host_venc() else { return };
    let d = dir("real");
    let _cleanup = Cleanup(d.clone());
    let mut spec = spec_for(&d);
    spec.encoder.height = 480; // The host fake kernel emits a fixed 640x480 SPS.
    let rec = Recorder::start(spec, Arc::new(factory(venc, &d))).unwrap();
    for i in 0..20u64 {
        if let Err(error) = send_frame(&rec, i) {
            fail_recording(rec, error);
        }
    }
    let began = Instant::now();
    loop {
        let state = rec.snapshot();
        if state.samples == 20 {
            break;
        }
        if state.finished.is_some() || began.elapsed() >= Duration::from_secs(10) {
            fail_recording(rec, format!("recording did not produce 20 samples: {state:?}"));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    rec.stop(20 * 33_333);
    let f = wait_finished(&rec, Duration::from_secs(10));
    assert_eq!(f.how, Ended::Stopped, "{f:?}");
    assert_eq!(f.samples, 20);
    let file = std::fs::read(&f.path).unwrap();
    let b = boxes(&file);
    // Keyframes every 5: four fragments.
    assert_eq!((&b[..2], b.iter().filter(|x| *x == "moof").count()), (&["ftyp".to_string(), "moov".to_string()][..], 4), "{b:?}");
    let out = match std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-count_packets",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name,width,height,nb_read_packets",
            "-of",
            "default=nw=1",
        ])
        .arg(&f.path)
        .output()
    {
        Ok(out) => out,
        Err(e) if !require_host_venc() && e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipped packet probe: ffprobe is not installed");
            return;
        }
        Err(e) => panic!("ffprobe is required to check the recorded packets: {e}"),
    };
    assert!(out.status.success(), "ffprobe: {}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    for expected in ["codec_name=h264", "width=640", "height=480", "nb_read_packets=20"] {
        assert!(text.lines().any(|line| line == expected), "expected {expected}, got:\n{text}");
    }
}

#[test]
fn retrying_a_frame_reports_the_finished_recordings_error() {
    let d = dir("send-dies");
    let _cleanup = Cleanup(d.clone());
    let venc = script(&d, "venc", "exit 2");
    let mut rec = Recorder::start(spec_for(&d), Arc::new(factory(venc, &d))).unwrap();
    let finished = wait_finished(&rec, Duration::from_secs(10));
    rec.join();
    assert!(matches!(&finished.how, Ended::Failed(_)), "{finished:?}");
    let error = send_frame(&rec, 0).expect_err("a finished recorder cannot accept a frame");
    assert!(error.contains(&format!("{finished:?}")), "{error}");
}

#[test]
fn a_program_that_dies_before_it_is_ready_is_an_error() {
    let d = dir("dies");
    // Its stderr goes to the collector, so the error names only that the encoder closed its output.
    let f = factory(script(&d, "venc", "exit 2"), &d);
    assert!(f.available());
    let e = f.open(&SPEC).err().expect("an error");
    assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof, "{e}");
    assert!(e.to_string().contains("closed its output"), "{e}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_program_that_is_ready_for_another_size_is_refused() {
    let d = dir("size");
    // "LRDY", version 1, 320 x 240
    let f = factory(script(&d, "venc", r"printf 'LRDY\001\000\000\000\100\001\000\000\360\000\000\000\000\000\000\000\000\000\000\000'; sleep 5"), &d);
    let e = f.open(&SPEC).err().expect("an error");
    assert!(e.to_string().contains("320x240"), "{e}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_program_that_stops_answering_costs_a_killed_child_and_a_recording_that_ends_whole() {
    let d = dir("hang");
    // Ready, then never an answer: one frame is read and the script sleeps.
    let venc = script(&d, "venc", r"printf 'LRDY\001\000\000\000\200\002\000\000\150\001\000\000\000\000\000\000\000\000\000\000'; sleep 60");
    let rec = Recorder::start(spec_for(&d), Arc::new(factory(venc, &d))).unwrap();
    assert!(rec.send(picture(90, SPEC.width, SPEC.height), 0));
    let began = Instant::now();
    let f = wait_finished(&rec, Duration::from_secs(10));
    assert!(began.elapsed() > Duration::from_secs(2) && began.elapsed() < Duration::from_secs(8), "the deadline is 3 s: {:?}", began.elapsed());
    match &f.how {
        Ended::Failed(why) => assert!(why.contains("did not answer in time"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(f.samples, 0);
    assert!(!d.join("rec.mp4").exists() && !d.join("rec.mp4.part").exists(), "nothing was recorded, so nothing is left");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_program_that_answers_with_nonsense_is_cut_off() {
    let d = dir("nonsense");
    let venc = script(
        &d,
        "venc",
        r"printf 'LRDY\001\000\000\000\200\002\000\000\150\001\000\000\000\000\000\000\000\000\000\000'; head -c 345624 >/dev/null; printf 'XXXX\000\000\000\000\000\000\000\000\000\000\000\000\000\000\000\000\000\000\000\000'; sleep 60",
    );
    let rec = Recorder::start(spec_for(&d), Arc::new(factory(venc, &d))).unwrap();
    assert!(rec.send(picture(10, SPEC.width, SPEC.height), 0));
    let f = wait_finished(&rec, Duration::from_secs(10));
    assert!(matches!(&f.how, Ended::Failed(why) if why.contains("answered")), "{f:?}");
    let _ = std::fs::remove_dir_all(&d);
}
