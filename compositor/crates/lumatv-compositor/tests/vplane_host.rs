//! The video-plane player (`vplane/`, the real C program built for the host) as a client of LumaTV's compositor: the compositor is the one
//! its own tests use (in process, the real wayland-server library, the software renderer, a socket pair), the player's NDL library is a
//! stand-in that logs its calls, and the test plays the media pipeline (it imports the window id the library was given and attaches a
//! punch-through, as starfish-media-pipeline does) and the shell (it asks the compositor to close the window).
//!
//! What it shows is that the client the player writes on the wire, with no libwayland, is one this server library and this compositor's
//! LG protocol handlers accept: the window and its properties, the exported element and the rectangle it is given, the hole and the
//! placement call that follow the pipeline's attach, the drift's new placements, the veil the window carries, and the window leaving when
//! the player is stopped or the compositor closes it. Nothing here says anything about LG's compositor.
//!
//! Build the player first: `sh vplane/build.sh`. Without it the tests say so and pass. Run: `cargo test --test vplane_host`.

#[path = "harness/mod.rs"]
mod harness;

use harness::{Client, Server};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const APP: &str = "org.nphil.lumatv.vplane";

/// One at a time: each test starts a compositor (the software renderer composes every commit of the player's 8 MB window) and a player, and
/// timing is the point.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

struct Paths {
    player: PathBuf,
    ndl_dir: PathBuf,
    clips: PathBuf,
}

fn paths() -> Option<Paths> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let player = root.join("vplane/build/host/lumatv-vplane");
    let ndl_dir = root.join("vplane/build/host");
    let clips = root.join("payload/vplane/clips");
    if player.exists() && ndl_dir.join("libNDL_directmedia.so.1").exists() && clips.join("index.json").exists() {
        Some(Paths { player, ndl_dir, clips })
    } else {
        eprintln!("vplane is not built (sh vplane/build.sh): the test does nothing");
        None
    }
}

fn wait_for(timeout: Duration, mut what: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if what() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    what()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

fn high_dup(fd: i32) -> i32 {
    // SAFETY: a plain fcntl on an open descriptor; the copy is close-on-exec and lands above the numbers the child takes.
    let copy = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 100) };
    assert!(copy >= 100, "F_DUPFD_CLOEXEC");
    copy
}

struct Player {
    child: Child,
    mailbox: PathBuf,
    ndl_log: PathBuf,
    seq: i64,
}

impl Player {
    fn spawn(server: &mut Server, window: &str) -> Player {
        let p = paths().expect("paths");
        let tmp = std::env::temp_dir().join(format!("vplane-host-{}-{}", std::process::id(), window));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let mailbox = tmp.join("mb");
        let ndl_log = tmp.join("ndl.log");
        std::fs::write(&ndl_log, b"").unwrap();
        let wayland = server.client_stream.take().expect("the harness's client stream");
        let wl_fd = high_dup(wayland.as_raw_fd());
        let mut cmd = Command::new(&p.player);
        cmd.arg("-d")
            .arg(&mailbox)
            .arg("-c")
            .arg(&p.clips)
            .arg("-L")
            .arg(p.ndl_dir.join("libNDL_directmedia.so.1"))
            .args(["-w", window, "-m", "1"])
            .env("WAYLAND_SOCKET", "3")
            .env("LD_LIBRARY_PATH", &p.ndl_dir)
            .env("FAKE_NDL_LOG", &ndl_log)
            .env_remove("WAYLAND_DISPLAY")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(tmp.join("player.log")).unwrap());
        // SAFETY: dup2 only, between fork and exec; it clears close-on-exec on the copy the player inherits as descriptor 3.
        unsafe {
            cmd.pre_exec(move || {
                if libc::dup2(wl_fd, 3) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = cmd.spawn().expect("the player starts");
        // SAFETY: the parent's copy of the player's descriptor is closed.
        unsafe { libc::close(wl_fd) };
        drop(wayland);
        let me = Player { child, mailbox, ndl_log, seq: now_ms() };
        assert!(wait_for(Duration::from_secs(5), || me.status().get("state").is_some()), "the player wrote its status");
        me
    }

    fn send(&mut self, op: &str, extra: Value) {
        self.seq += 1;
        let mut req = json!({"seq": self.seq, "sent": now_ms(), "op": op});
        if let (Some(r), Some(e)) = (req.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                r.insert(k.clone(), v.clone());
            }
        }
        std::fs::write(self.mailbox.join("req.json"), req.to_string()).unwrap();
    }

    fn status(&self) -> Value {
        std::fs::read_to_string(self.mailbox.join("status.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null)
    }

    fn state(&self) -> String {
        self.status()["state"].as_str().unwrap_or("").to_string()
    }

    /// The window id the library was given, from the stand-in's log.
    fn window_id(&self) -> Option<String> {
        let log = std::fs::read_to_string(&self.ndl_log).ok()?;
        log.lines().find_map(|l| {
            l.split_whitespace().skip(2).collect::<Vec<_>>().as_slice().strip_prefix(&["SetWindowId"]).and_then(|r| r.first().map(|s| s.to_string()))
        })
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn placements(s: &mut lumatv_compositor::State) -> Vec<Value> {
    s.luna_sent().iter().filter(|(u, _)| u.ends_with("/setDisplayWindow")).map(|(_, p)| p.clone()).collect()
}

#[test]
fn the_player_makes_a_window_the_pipeline_attaches_to_and_the_compositor_places_it() {
    if paths().is_none() {
        return;
    }
    let _one = ONE_AT_A_TIME.lock();
    let mut server = Server::start();
    let mut player = Player::spawn(&mut server, "screensaver");
    let t0 = now_ms();
    // drift fast so the test sees it; dimmed half an hour into the hour so the window carries a veil; the clock shows a fixed text
    player.send("start", json!({"ttlMs": 60000, "moveMs": 120, "periodX": 2, "periodY": 3, "clockAt": "10:42", "since": t0 - 30 * 60000}));
    assert!(wait_for(Duration::from_secs(8), || player.state() == "playing"), "playing: {}", player.status());

    // the window: a mapped, fullscreen screen-saver window of the player's app, the output's size, with LG's playVideo flag
    let windows = server.eval(|s| s.windows_report());
    assert_eq!(windows.len(), 1, "{windows:?}");
    let w = &windows[0];
    assert_eq!(w["type"], "Screensaver", "{w}");
    assert_eq!(w["appId"], APP);
    assert_eq!(w["state"], "Fullscreen");
    assert_eq!(w["mapped"], true);
    assert_eq!(w["size"], json!([1920, 1080]));
    assert_eq!(w["props"]["playVideo"], "true");
    assert_eq!(w["props"]["_WEBOS_WINDOW_TYPE"], "_WEBOS_WINDOW_TYPE_SCREENSAVER");

    // the export: what the library was told is the id the compositor assigned, and the element has the picture's rectangle
    let window_id = player.window_id().expect("the library was given a window id");
    let (known, dst, src) = server.eval({
        let id = window_id.clone();
        move |s| {
            let e = s.foreign.exported.get(&id);
            (
                e.is_some(),
                e.map(|e| (e.destination.x, e.destination.y, e.destination.w, e.destination.h)),
                e.and_then(|e| e.source).map(|r| (r.x, r.y, r.w, r.h)),
            )
        }
    });
    assert!(known, "{window_id} is an export of this compositor");
    assert_eq!(src, Some((0, 0, 1856, 1024)), "the source is the clip");
    assert!(dst.is_some());

    // the pipeline's side: import the id, attach a punch-through with its context
    let mut pipeline = Client::connect_extra(&mut server);
    let imported = pipeline.foreign.import_element(window_id.clone(), 0, &pipeline.qh, ());
    imported.attach_punchthrough("_ctx_vplane".into());
    pipeline.roundtrip();
    assert!(pipeline.wait_until(Duration::from_secs(3), |s| s.punchthrough_attached == vec!["_ctx_vplane".to_string()]));
    std::thread::sleep(Duration::from_millis(100));

    // the placement call carries the picture's rectangle (in the panel's pixels), and the hole is where the picture is
    let calls = server.eval(placements);
    let mine: Vec<&Value> = calls.iter().filter(|c| c["context"] == "_ctx_vplane").collect();
    assert!(!mine.is_empty(), "{calls:?}");
    let out = &mine.last().unwrap()["displayOutput"];
    let (ox, oy, ow, oh) = (out["x"].as_i64().unwrap(), out["y"].as_i64().unwrap(), out["width"].as_i64().unwrap(), out["height"].as_i64().unwrap());
    assert_eq!(ow * 1024, oh * 1856, "the picture's aspect ratio is kept: {out}");
    assert_eq!(mine.last().unwrap()["fullScreen"], false, "it does not cover the output");
    let scale = ow / 1856;
    assert!(scale >= 1 && ow == 1856 * scale && oh == 1024 * scale, "{out}");
    assert!(ox >= 0 && ox <= 64 * scale && oy >= 0 && oy <= 56 * scale, "inside the room the drift has: {out}");

    // pictures: the hole is clear, and so is everything the window drew inside it
    let (image, holes) = server.eval(|s| (s.output.capture().unwrap(), s.scene.holes.len()));
    assert_eq!(holes, 1);
    assert_eq!(image.pixel(960, 540), [0, 0, 0, 0], "the hole");

    // the drift: the rectangle moves and the compositor places the video again, always inside the room
    assert!(wait_for(Duration::from_secs(4), || {
        let c = server.eval(placements);
        let mut seen: Vec<(i64, i64)> = c
            .iter()
            .filter(|c| c["context"] == "_ctx_vplane")
            .map(|c| (c["displayOutput"]["x"].as_i64().unwrap(), c["displayOutput"]["y"].as_i64().unwrap()))
            .collect();
        seen.sort();
        seen.dedup();
        seen.len() >= 4
    }));
    for c in server.eval(placements).iter().filter(|c| c["context"] == "_ctx_vplane") {
        let o = &c["displayOutput"];
        if o["width"].as_i64().unwrap() == 0 {
            continue;
        }
        let (x, y) = (o["x"].as_i64().unwrap() / scale, o["y"].as_i64().unwrap() / scale);
        assert!((2..=62).contains(&x) && (8..=48).contains(&y), "the drift stays in its room: {o}");
    }

    // stop: the window and the hole go, the video is hidden (a 0 x 0 placement), the player is idle
    player.send("stop", json!({}));
    assert!(wait_for(Duration::from_secs(5), || player.state() == "idle"), "{}", player.status());
    assert!(wait_for(Duration::from_secs(3), || server.eval(|s| s.windows_report().is_empty() && s.scene.holes.is_empty())));
    let calls = server.eval(placements);
    let last = calls.iter().rev().find(|c| c["context"] == "_ctx_vplane").unwrap();
    assert_eq!(last["displayOutput"]["width"], 0, "hidden: {last}");
}

/// What a window drawn above a video element looks like in this compositor: the player paints the margins around the picture opaque black
/// (what is behind the saver does not show), and what it draws inside the rectangle (the veil, the clock) is cleared with the hole. LG's
/// compositor draws the hole under the window (its `PunchThrough` item is at z -1, `views/punchThrough.js`), so there the clock shows;
/// docs/research/videoplane-saver-result.md, "Two compositors". This is the test to change when this compositor is changed to LG's order:
/// the clock's pixels are then in the picture.
#[test]
fn the_margins_are_opaque_and_the_hole_clears_what_the_window_drew_in_it() {
    if paths().is_none() {
        return;
    }
    let _one = ONE_AT_A_TIME.lock();
    let mut server = Server::start();
    let mut player = Player::spawn(&mut server, "overlay");
    let mut under = Client::connect_extra(&mut server);
    let card = under.card("_WEBOS_WINDOW_TYPE_CARD", "Under");
    let red = under.shm_buffer(1920, 1080, 0xffff_0000);
    under.present(&card.surface, &red);
    assert!(under.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1));
    player.send("start", json!({"ttlMs": 60000, "moveMs": 600000, "clockAt": "10:42", "since": now_ms() - 30 * 60000}));
    assert!(wait_for(Duration::from_secs(8), || player.state() == "playing"), "{}", player.status());
    let window_id = player.window_id().expect("a window id");
    let mut pipeline = Client::connect_extra(&mut server);
    let imported = pipeline.foreign.import_element(window_id, 0, &pipeline.qh, ());
    imported.attach_punchthrough("_ctx_vplane".into());
    pipeline.roundtrip();
    assert!(pipeline.wait_until(Duration::from_secs(3), |s| s.punchthrough_attached.len() == 1));
    std::thread::sleep(Duration::from_millis(150));
    let image = server.eval(|s| s.output.capture().unwrap());
    // outside the picture's rectangle (32 pixels of room at the left, 28 above): black, not the red card below
    assert_eq!(image.pixel(10, 500), [0, 0, 0, 255], "the left margin");
    assert_eq!(image.pixel(960, 4), [0, 0, 0, 255], "the top margin");
    // inside it: clear, and the clock the player drew at the corner of the window (1856, 1015 less its height) is cleared with it
    assert_eq!(image.pixel(960, 540), [0, 0, 0, 0]);
    let mut lit = 0;
    for y in 985..1020 {
        for x in 1740..1860 {
            if image.pixel(x, y) != [0, 0, 0, 0] {
                lit += 1;
            }
        }
    }
    assert_eq!(lit, 0, "nothing of the window shows inside the hole in this compositor");
}

#[test]
fn the_compositor_closing_the_window_ends_the_play() {
    if paths().is_none() {
        return;
    }
    let _one = ONE_AT_A_TIME.lock();
    let mut server = Server::start();
    let mut player = Player::spawn(&mut server, "card");
    player.send("start", json!({"ttlMs": 60000}));
    assert!(wait_for(Duration::from_secs(8), || player.state() == "playing"), "{}", player.status());
    let w = server.eval(|s| s.windows_report());
    assert_eq!(w[0]["type"], "Card", "{w:?}");
    assert!(w[0]["props"].get("playVideo").is_none(), "the playVideo flag is the screen-saver type's");
    assert_eq!(server.eval(|s| s.close_app(APP)), 1, "the compositor's closeByAppId reached the window");
    assert!(wait_for(Duration::from_secs(5), || player.state() == "idle"), "{}", player.status());
    assert!(player.status()["reason"].as_str().unwrap().contains("closed the window"), "{}", player.status());
    assert!(wait_for(Duration::from_secs(3), || server.eval(|s| s.windows_report().is_empty())));
}

#[test]
fn a_player_that_is_not_renewed_takes_its_window_away_by_itself() {
    if paths().is_none() {
        return;
    }
    let _one = ONE_AT_A_TIME.lock();
    let mut server = Server::start();
    let mut player = Player::spawn(&mut server, "screensaver");
    player.send("start", json!({"ttlMs": 1500}));
    assert!(wait_for(Duration::from_secs(8), || player.state() == "playing"));
    assert_eq!(server.eval(|s| s.windows_report().len()), 1);
    assert!(wait_for(Duration::from_secs(6), || player.state() == "idle"), "{}", player.status());
    assert!(player.status()["reason"].as_str().unwrap().starts_with("ttl"));
    assert!(wait_for(Duration::from_secs(3), || server.eval(|s| s.windows_report().is_empty())), "no window is left on the screen when nobody asked for it");
}
