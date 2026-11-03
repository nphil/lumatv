//! Screenshots and screen recordings (`screencap`) on the in-process compositor with the software renderer: off until the switch, never without
//! the shell's indicator, a PNG of what was composed with the video's hole black, a recording through a fake encoder into a playable
//! fragmented MP4, and every way a recording ends. The hardware encoder itself is `venc/test.sh` and the TV check.

mod harness;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{Value, json};

use harness::{Client, Server};
use lumatv_compositor::Rect;
use lumatv_compositor::control::{self, Reply};
use lumatv_compositor::scene::Hole;
use lumatv_compositor::screencap::encoder::{Encoder, EncoderFactory, EncoderSpec, Unit};
use lumatv_compositor::shell::Msg;

const WAIT: Duration = Duration::from_secs(5);

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lumatv-screencap-it-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn enable(data: &Path, extra: Value) {
    let mut capture = json!({"enabled": true, "record": {"height": 360}});
    if let (Some(c), Some(e)) = (capture.as_object_mut(), extra.as_object()) {
        c.extend(e.clone());
    }
    std::fs::write(data.join("helper.json"), json!({"capture": capture}).to_string()).unwrap();
}

/// What the fake encoder saw: the mean luma of each frame it was given.
#[derive(Default)]
struct Seen {
    lumas: Vec<u8>,
    spec: Option<EncoderSpec>,
    closed: bool,
}

struct FakeFactory(Arc<Mutex<Seen>>);

struct Fake {
    seen: Arc<Mutex<Seen>>,
    spec: EncoderSpec,
    n: u64,
}

impl EncoderFactory for FakeFactory {
    fn open(&self, spec: &EncoderSpec) -> std::io::Result<Box<dyn Encoder>> {
        self.0.lock().spec = Some(*spec);
        Ok(Box::new(Fake { seen: self.0.clone(), spec: *spec, n: 0 }))
    }

    fn describe(&self) -> String {
        "fake".into()
    }

    fn available(&self) -> bool {
        true
    }
}

impl Encoder for Fake {
    fn encode(&mut self, nv12: &[u8], pts_us: u64, key: bool) -> std::io::Result<Option<Unit>> {
        let luma = &nv12[..(self.spec.width * self.spec.height) as usize];
        let mean = (luma.iter().map(|&v| v as u64).sum::<u64>() / luma.len() as u64) as u8;
        self.seen.lock().lumas.push(mean);
        let is_key = key || self.n % self.spec.gop as u64 == 0;
        let mut data = Vec::new();
        if is_key {
            data.extend_from_slice(&[0, 0, 0, 1, 0x67, 100, 0, 31, 0xAC, 0xD9, 0, 0, 0, 1, 0x68, 0xEB, 0xE3, 0xCB]);
        }
        data.extend_from_slice(&[0, 0, 1, if is_key { 0x65 } else { 0x41 }, mean]);
        data.extend((0..300).map(|i| (i % 200 + 1) as u8));
        self.n += 1;
        Ok(Some(Unit { data, key: is_key, pts_us }))
    }

    fn close(self: Box<Self>) -> String {
        self.seen.lock().closed = true;
        format!("fake closed after {} frames", self.n)
    }
}

fn prepare(server: &Server, data: &Path, with_fake: bool) -> Arc<Mutex<Seen>> {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (data, s2) = (data.to_path_buf(), seen.clone());
    server.eval(move |s| {
        s.config.data_dir = data;
        if with_fake {
            s.screencap.set_encoder(Arc::new(FakeFactory(s2)));
        }
    });
    seen
}

fn shell(server: &Server, connected: bool, dot: bool) {
    server.eval(move |s| {
        if connected {
            s.shell_rust.kinds = vec!["capture".into()];
            s.shell_rust_open(0, "test".into()).unwrap();
            s.shell_rust_handle(Msg::Connected);
        }
        s.shell_rust_handle(Msg::Ui { can: Some(vec!["capture".into()]), wants: None, live: None, state: Some(json!({"capture": {"dot": dot}})) });
    });
}

fn start(server: &Server, line: &'static str) -> Reply {
    server.eval(move |s| control::handle(s, line))
}

fn answer(reply: Reply) -> Value {
    serde_json::from_str(&reply.text()).expect("a JSON answer")
}

fn ask(server: &Server, line: &'static str) -> Value {
    answer(start(server, line))
}

fn wait_for(limit: Duration, mut f: impl FnMut() -> bool) -> bool {
    let t = Instant::now();
    while t.elapsed() < limit {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// A client with a full-screen card of one colour; returns it with its window's surface for holes.
fn card(server: &mut Server, argb: u32) -> (Client, harness::Window) {
    let mut client = Client::connect(server);
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "Test");
    let buf = client.shm_buffer(1920, 1080, argb);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1));
    (client, win)
}

fn decode(path: &Path) -> (png::OutputInfo, Vec<u8>) {
    let mut reader = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap())).read_info().unwrap();
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    (info, buf)
}

fn top_level(file: &[u8]) -> Vec<String> {
    let (mut v, mut i) = (Vec::new(), 0);
    while i + 8 <= file.len() {
        let size = u32::from_be_bytes(file[i..i + 4].try_into().unwrap()) as usize;
        v.push(String::from_utf8_lossy(&file[i + 4..i + 8]).into_owned());
        assert!(size >= 8 && i + size <= file.len(), "a box runs past the end");
        i += size;
    }
    v
}

#[test]
fn capture_is_off_until_the_switch_and_nothing_is_written_while_it_is_off() {
    let mut server = Server::start();
    let data = dir("off");
    prepare(&server, &data, true);
    shell(&server, true, true);
    let _screen = card(&mut server, 0xffff_0000);
    for line in ["screencap shot", "screencap record start"] {
        let a = ask(&server, line);
        assert_eq!((a["ok"].clone(), a["code"].clone()), (json!(false), json!("disabled")), "{line}: {a}");
    }
    let status = ask(&server, "screencap status");
    assert_eq!(status["enabled"], false);
    assert_eq!(status["recording"], false);
    assert!(!data.join("captures").exists(), "no folder is made for a capture that was refused");
    // helper.json says false: the same.
    std::fs::write(data.join("helper.json"), r#"{"capture": {"enabled": false}}"#).unwrap();
    assert_eq!(ask(&server, "screencap shot")["code"], "disabled");
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn nothing_is_captured_while_there_is_nothing_on_the_screen_to_say_so() {
    let mut server = Server::start();
    let data = dir("noshell");
    enable(&data, json!({}));
    prepare(&server, &data, true);
    let _screen = card(&mut server, 0xffff_0000);
    for line in ["screencap shot", "screencap record start"] {
        let a = ask(&server, line);
        assert_eq!(a["code"], "no-indicator", "{line}: {a}");
    }
    assert!(!data.join("captures").exists());
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn a_screenshot_is_a_png_of_the_composed_picture_and_the_hole_for_video_is_black() {
    let mut server = Server::start();
    let data = dir("shot");
    enable(&data, json!({}));
    prepare(&server, &data, true);
    shell(&server, true, false);
    // A card that leaves a window for video, as a player's does: transparent in a rectangle (a hole for the video plane is under it), a
    // half-transparent white strip across its lower edge (premultiplied: 50 % white over black is grey 128), red everywhere else.
    let mut client = Client::connect(&mut server);
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "Test");
    let buf = client.shm_pattern(1920, 1080, |x, y| match (x, y) {
        (600..1320, 300..700) => 0x0000_0000,
        (_, 1000..1080) => 0x8080_8080,
        _ => 0xffff_0000,
    });
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1));
    server.eval(move |s| {
        let surface = s.scene.windows().find(|w| w.title == "Test").unwrap().surface;
        s.scene.holes.push(Hole { surface, rect: Rect::new(600, 300, 720, 400) });
        s.scene.redraw_all();
    });

    let a = ask(&server, "screencap shot");
    assert_eq!(a["ok"], true, "{a}");
    let name = a["name"].as_str().unwrap();
    assert!(name.starts_with("shot-") && name.ends_with(".png"), "{name}");
    let path = data.join("captures").join(name);
    assert_eq!(a["path"], path.display().to_string());
    assert_eq!((a["width"].clone(), a["height"].clone()), (json!(1920), json!(1080)));
    assert_eq!(a["bytes"].as_u64().unwrap(), std::fs::metadata(&path).unwrap().len());
    assert!(!data.join("captures").join(format!("{name}.part")).exists(), "no part is left");

    let (info, px) = decode(&path);
    assert_eq!((info.width, info.height, info.color_type, info.bit_depth), (1920, 1080, png::ColorType::Rgb, png::BitDepth::Eight));
    let at = |x: usize, y: usize| [px[(y * 1920 + x) * 3], px[(y * 1920 + x) * 3 + 1], px[(y * 1920 + x) * 3 + 2]];
    assert_eq!(at(5, 5), [255, 0, 0], "the card");
    assert_eq!(at(1900, 900), [255, 0, 0]);
    assert_eq!(at(960, 500), [0, 0, 0], "video is black: the hole is never transparent and never anything of the video");
    assert_eq!(at(600, 300), [0, 0, 0]);
    assert_eq!(at(1319, 699), [0, 0, 0]);
    assert_eq!(at(599, 300), [255, 0, 0], "the hole's edge");
    assert_eq!(at(960, 1040), [128, 128, 128], "premultiplied pixels are composited over black");

    // The shell is told: a counter its toast follows, and what the last capture was.
    let topic = server.eval(|s| s.shell_rust.topic("capture").cloned()).expect("the capture topic");
    assert_eq!(
        (topic["recording"].clone(), topic["n"].clone(), topic["last"]["kind"].clone(), topic["last"]["name"].clone()),
        (json!(false), json!(1), json!("shot"), json!(name))
    );
    let status = ask(&server, "screencap status");
    assert_eq!((status["captures"]["count"].clone(), status["last"]["ok"].clone()), (json!(1), json!(true)));
    assert!(server.eval(|s| s.journal.iter().any(|l| l.contains("screencap: screenshot shot-"))));
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn the_size_cap_removes_the_oldest_captures_to_make_room_and_nothing_that_is_not_a_capture() {
    let mut server = Server::start();
    let data = dir("cap");
    enable(&data, json!({"maxBytes": 16 << 20}));
    prepare(&server, &data, true);
    shell(&server, true, false);
    let _screen = card(&mut server, 0xff00_ff00);
    let caps = data.join("captures");
    std::fs::create_dir_all(&caps).unwrap();
    for (i, age) in [(1, 300u64), (2, 200), (3, 100)] {
        let p = caps.join(format!("shot-2026010{i}-000000.png"));
        let f = std::fs::File::create(&p).unwrap();
        f.set_len(6 << 20).unwrap();
        f.set_modified(std::time::SystemTime::now() - Duration::from_secs(age)).unwrap();
    }
    std::fs::write(caps.join("keep.txt"), b"mine").unwrap();
    let a = ask(&server, "screencap shot");
    assert_eq!(a["ok"], true, "{a}");
    assert!(!caps.join("shot-20260101-000000.png").exists(), "the oldest went");
    assert!(!caps.join("shot-20260102-000000.png").exists(), "and the next one: 18 MiB plus a new picture does not fit under 16");
    assert!(caps.join("shot-20260103-000000.png").exists());
    assert!(caps.join("keep.txt").exists());
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn a_recording_runs_through_the_encoder_into_a_playable_file_while_the_dot_is_up() {
    let mut server = Server::start();
    let data = dir("rec");
    enable(&data, json!({}));
    let seen = prepare(&server, &data, true);
    shell(&server, true, false);
    let (mut client, win) = card(&mut server, 0xffff_0000);

    // The start waits for the shell to say the dot is up; the dot is asked for at once.
    let waiting = std::thread::spawn({
        let reply = start(&server, "screencap record start");
        move || answer(reply)
    });
    assert!(wait_for(WAIT, || server.eval(|s| s.shell_rust.topic("capture").cloned()).is_some_and(|t| t["recording"] == true)), "the topic asks for the dot");
    std::thread::sleep(Duration::from_millis(100));
    assert!(!waiting.is_finished(), "not started before the dot is confirmed");
    shell(&server, false, true);
    let started = waiting.join().unwrap();
    assert_eq!(started["ok"], true, "{started}");
    let name = started["name"].as_str().unwrap().to_string();
    assert_eq!((started["width"].clone(), started["height"].clone()), (json!(640), json!(360)));
    assert_eq!(ask(&server, "screencap record start")["code"], "busy");
    let status = ask(&server, "screencap status");
    assert_eq!((status["recording"].clone(), status["recorder"]["stage"].clone()), (json!(true), json!("recording")));

    // Red for a while, then green: the encoder sees both, in order.
    std::thread::sleep(Duration::from_millis(400));
    let green = client.shm_buffer(1920, 1080, 0xff00_ff00);
    client.present(&win.surface, &green);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 2));
    std::thread::sleep(Duration::from_millis(400));

    let done = ask(&server, "screencap record stop");
    assert_eq!(done["ok"], true, "{done}");
    assert_eq!(done["name"], name.as_str());
    let path = data.join("captures").join(&name);
    assert!(path.exists() && !data.join("captures").join(format!("{name}.part")).exists());
    let file = std::fs::read(&path).unwrap();
    assert_eq!(file.len() as u64, done["bytes"].as_u64().unwrap());
    let boxes = top_level(&file);
    assert_eq!(&boxes[..2], ["ftyp", "moov"]);
    assert!(boxes.iter().filter(|b| *b == "moof").count() >= 1);
    assert!(done["samples"].as_u64().unwrap() >= 2, "{done}");

    let s = seen.lock();
    assert_eq!(s.spec, Some(EncoderSpec { width: 640, height: 360, fps: 30, kbps: 4000, gop: 30 }));
    assert!(s.closed);
    // Luma of red 255,0,0 is 63 and of green 0,255,0 is 173 in BT.709 limited range (16 + 219 * 0.2126 = 62.6, 16 + 219 * 0.7152 = 172.6).
    let red = s.lumas.iter().filter(|&&l| (61..=65).contains(&l)).count();
    let green_n = s.lumas.iter().filter(|&&l| (171..=175).contains(&l)).count();
    assert!(red >= 1 && green_n >= 1, "red then green: {:?}", s.lumas);
    let first_green = s.lumas.iter().position(|&l| (171..=175).contains(&l)).unwrap();
    assert!(s.lumas[..first_green].iter().all(|&l| (61..=65).contains(&l)), "all red before the first green: {:?}", s.lumas);
    drop(s);

    let topic = server.eval(|s| s.shell_rust.topic("capture").cloned()).unwrap();
    assert_eq!((topic["recording"].clone(), topic["last"]["kind"].clone(), topic["last"]["ok"].clone()), (json!(false), json!("recording"), json!(true)));
    assert_eq!(ask(&server, "screencap record stop")["code"], "not-recording");
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn a_recording_does_not_start_without_the_dot_and_says_so() {
    let mut server = Server::start();
    let data = dir("nodot");
    enable(&data, json!({}));
    let seen = prepare(&server, &data, true);
    shell(&server, true, false);
    let _screen = card(&mut server, 0xff00_00ff);
    let a = ask(&server, "screencap record start");
    assert_eq!((a["ok"].clone(), a["code"].clone()), (json!(false), json!("no-indicator")), "{a}");
    assert!(a["error"].as_str().unwrap().contains("dot"), "{a}");
    assert!(wait_for(WAIT, || !server.eval(|s| s.screencap_status()["recording"].as_bool().unwrap())), "the recording is gone");
    assert!(seen.lock().closed, "the encoder was given back");
    let topic = server.eval(|s| s.shell_rust.topic("capture").cloned()).unwrap();
    assert_eq!(topic["recording"], false, "the dot is taken down again");
    assert_eq!(std::fs::read_dir(data.join("captures")).map_or(0, |d| d.count()), 0, "and no file is left");
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn a_recording_stops_itself_when_the_dot_goes_away() {
    let mut server = Server::start();
    let data = dir("lost");
    enable(&data, json!({}));
    prepare(&server, &data, true);
    shell(&server, true, true);
    let (_client, _win) = card(&mut server, 0xffff_ffff);
    let started = ask(&server, "screencap record start");
    assert_eq!(started["ok"], true, "{started}");
    std::thread::sleep(Duration::from_millis(300));
    shell(&server, false, false);
    assert!(wait_for(Duration::from_secs(8), || !server.eval(|s| s.screencap_status()["recording"].as_bool().unwrap())), "ended within the grace");
    let status = ask(&server, "screencap status");
    assert_eq!((status["last"]["kind"].clone(), status["last"]["ok"].clone()), (json!("recording"), json!(false)));
    assert!(status["last"]["note"].as_str().unwrap().contains("dot"), "{status}");
    let name = status["last"]["name"].as_str().unwrap();
    assert!(data.join("captures").join(name).exists(), "what was recorded is kept, whole");
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn a_missing_encoder_is_refused_before_anything_starts_and_names_the_program() {
    let mut server = Server::start();
    let data = dir("noenc");
    enable(&data, json!({}));
    prepare(&server, &data, false);
    shell(&server, true, true);
    let _screen = card(&mut server, 0xff80_8080);
    let a = ask(&server, "screencap record start");
    assert_eq!(a["code"], "no-encoder", "{a}");
    assert!(a["error"].as_str().unwrap().contains("bin/lumatv-venc"), "{a}");
    let status = ask(&server, "screencap status");
    assert_eq!((status["encoder"]["available"].clone(), status["recording"].clone()), (json!(false), json!(false)));
    assert_eq!(ask(&server, "screencap bogus")["code"], "bad-command");
    let _ = std::fs::remove_dir_all(&data);
}
