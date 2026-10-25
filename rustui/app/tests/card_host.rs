//! The card app, the real 64-bit binary, as a client of LumaTV's compositor on the host: the compositor is the one its own tests use
//! (`compositor/crates/lumatv-compositor/tests/harness`: in process, software renderer, a socket pair), the app draws with Mesa's
//! software GL, and the test plays the 32-bit launcher SAM starts (the other end of the app's channel) and SAM itself (luna calls
//! into the compositor, its `relaunch` and `close`). Nothing here says anything about the set: the buffers are shm, the timing is
//! llvmpipe's. What it shows is the contract: a card, its state changes, Back, Home, the switcher, SAM's foreground, `close`.
//!
//! Run: `LIBGL_ALWAYS_SOFTWARE=1 cargo test --features window --test card_host` in `rustui/`. Pictures of what the compositor showed go
//! to `rustui/build/cardhost/`.
#![cfg(feature = "window")]

#[path = "../../../compositor/crates/lumatv-compositor/tests/harness/mod.rs"]
mod harness;

use harness::{Client, Server};
use lumatv_compositor::server::state::{ClientEvent, ClientHandler};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

const APP: &str = "org.nphil.lumatv.card";
/// llvmpipe builds the shaders of the page on the first frame.
const SLOW: Duration = Duration::from_secs(40);
const BACK: u32 = 412;
const OK: u32 = 28;
const HOME: u32 = 102;

/// One at a time: each test starts a compositor and a software-GL app, and timing is the point.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn build_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../build/cardhost");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn wait_for(timeout: Duration, mut what: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if what() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    what()
}

/// The app, and the launcher's end of its channel.
struct App {
    child: Child,
    channel: UnixStream,
    pending: Vec<u8>,
    seen: Vec<Value>,
    log: PathBuf,
}

fn high_dup(fd: i32) -> i32 {
    // SAFETY: a plain fcntl on an open descriptor; the copy is close-on-exec and lands above the numbers the child takes.
    let copy = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 100) };
    assert!(copy >= 100, "F_DUPFD_CLOEXEC");
    copy
}

impl App {
    /// Starts the app with the server's one client stream as its Wayland socket (`WAYLAND_SOCKET`).
    fn spawn(server: &mut Server, name: &str, args: &[&str]) -> App {
        let wayland = server.client_stream.take().expect("the harness's client stream");
        let (launcher, app_end) = UnixStream::pair().unwrap();
        let (wl_fd, ch_fd) = (high_dup(wayland.as_raw_fd()), high_dup(app_end.as_raw_fd()));
        let log = build_dir().join(format!("{name}.log"));
        let shm = std::env::temp_dir().join(format!("lumatv-card-test-{}-{name}", std::process::id()));
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_lumatv-card-app"));
        cmd.args(["--app-id", APP, "--libmali", "libEGL.so.1", "--channel-fd", "4"])
            .arg("--assets")
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/../../payload"))
            .arg("--shm")
            .arg(&shm)
            .args(args)
            .env("WAYLAND_SOCKET", "3")
            .env("LIBGL_ALWAYS_SOFTWARE", "1")
            .env_remove("WAYLAND_DISPLAY")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log).unwrap());
        // SAFETY: dup2 only, between fork and exec; it clears close-on-exec on the copies the app inherits as 3 and 4.
        unsafe {
            cmd.pre_exec(move || {
                if libc::dup2(wl_fd, 3) < 0 || libc::dup2(ch_fd, 4) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = cmd.spawn().expect("the card app starts");
        // SAFETY: the parent's copies of the app's descriptors are closed.
        unsafe {
            libc::close(wl_fd);
            libc::close(ch_fd);
        }
        drop(wayland);
        drop(app_end);
        launcher.set_nonblocking(true).unwrap();
        App { child, channel: launcher, pending: Vec::new(), seen: Vec::new(), log }
    }

    /// What the launcher says to the app, one line.
    fn say(&mut self, message: Value) {
        self.channel.write_all(format!("{message}\n").as_bytes()).unwrap();
    }

    /// Takes in what the app has sent on the channel.
    fn listen(&mut self) {
        let mut chunk = [0u8; 4096];
        while let Ok(n) = self.channel.read(&mut chunk) {
            if n == 0 {
                break;
            }
            self.pending.extend_from_slice(&chunk[..n]);
        }
        while let Some(at) = self.pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=at).collect();
            if let Ok(v) = serde_json::from_slice::<Value>(&line) {
                self.seen.push(v);
            }
        }
    }

    /// Whether the app has sent a message with these fields (all of them, as given).
    fn heard(&mut self, fields: Value) -> bool {
        self.listen();
        self.seen.iter().any(|m| fields.as_object().unwrap().iter().all(|(k, v)| m.get(k) == Some(v)))
    }

    fn wait_heard(&mut self, timeout: Duration, fields: Value) {
        let ok = wait_for(timeout, || self.heard(fields.clone()));
        assert!(ok, "the app never sent {fields}; sent {:?}\n{}", self.seen, self.log_text());
    }

    fn exited(&mut self) -> Option<ExitStatus> {
        self.child.try_wait().unwrap()
    }

    fn wait_exit(&mut self, timeout: Duration) -> ExitStatus {
        let end = Instant::now() + timeout;
        loop {
            if let Some(status) = self.exited() {
                return status;
            }
            assert!(Instant::now() < end, "the app did not end\n{}", self.log_text());
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn log_text(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// What the compositor knows of the app's window: the control socket's `windows` row.
fn window(server: &Server, app_id: &str) -> Option<Value> {
    let app_id = app_id.to_owned();
    server.eval(move |s| s.windows_report().into_iter().find(|w| w["appId"] == app_id.as_str()))
}

fn state_of(server: &Server, app_id: &str) -> String {
    window(server, app_id).map(|w| w["state"].as_str().unwrap_or("").to_owned()).unwrap_or_default()
}

fn foreground(server: &Server) -> Vec<String> {
    server.eval(|s| {
        let v = s.luna_method("com.webos.surfacemanager", "getForegroundAppInfo", &json!({}), "test").unwrap();
        v["foregroundAppInfo"].as_array().unwrap().iter().map(|a| a["appId"].as_str().unwrap_or("").to_owned()).collect()
    })
}

/// A key as the remote sends it, through the compositor's routing.
fn remote(server: &Server, code: u32) {
    server.eval(move |s| {
        s.key_event(code, true, false);
        s.key_event(code, false, false);
    });
}

fn commits(server: &Server) -> u64 {
    server.eval(|s| s.stats.commits)
}

fn capture(server: &Server, name: &str) -> lumatv_compositor::output::Image {
    let image = server.eval(|s| s.output.capture().unwrap());
    lumatv_compositor::capture::write_png(&build_dir().join(format!("{name}.png")), &image).unwrap();
    image
}

/// SAM's side of a launch: the card maps, the app tells its launcher the window is up.
fn start(server: &mut Server, name: &str, args: &[&str]) -> App {
    let mut app = App::spawn(server, name, args);
    let ok = wait_for(SLOW, || window(server, APP).is_some_and(|w| w["mapped"] == true));
    assert!(ok, "the card never mapped\n{}", app.log_text());
    app.wait_heard(SLOW, json!({"kind": "ready"}));
    app
}

/// A second client of the compositor, for Home: the harness takes one client at the start, so the test adds the server end of
/// another socket pair itself.
fn second_client(server: &mut Server) -> Client {
    let (server_end, client_end) = UnixStream::pair().unwrap();
    let events: Arc<Mutex<Vec<ClientEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let handler_events = events.clone();
    server.eval(move |s| {
        s.dh.insert_client(server_end, Arc::new(ClientHandler { events: handler_events.clone() })).expect("a second client");
        s.drain_client_events(&handler_events);
    });
    server.client_stream = Some(client_end);
    Client::connect(server)
}

#[test]
fn the_app_maps_as_a_card_that_takes_back_and_is_in_the_switcher_and_sams_foreground() {
    let _one = ONE_AT_A_TIME.lock();
    let mut server = Server::start();
    let mut app = start(&mut server, "card", &[]);

    let w = window(&server, APP).unwrap();
    assert_eq!(w["type"], "Card", "{w}");
    assert_eq!(w["state"], "Fullscreen", "the app asked for fullscreen and the compositor said so");
    assert_eq!(w["title"], "LumaTV Card");
    assert_eq!(w["props"]["_WEBOS_WINDOW_TYPE"], "_WEBOS_WINDOW_TYPE_CARD");
    assert_eq!(w["props"]["appId"], APP);
    assert_eq!(w["props"]["_WEBOS_ACCESS_POLICY_KEYS_BACK"], "true", "the card takes Back itself");
    assert_eq!(w["props"].get("_WEBOS_ACCESS_POLICY_KEYS_EXIT"), None, "Exit (Back held) stays the system's");
    assert_eq!(w["size"], json!([1920, 1080]));
    app.wait_heard(Duration::from_secs(5), json!({"kind": "state", "state": "fullscreen"}));

    // SAM's foreground names it, with the client's process id, and the switcher's list has it.
    let (fg, recents) = server.eval(|s| {
        let f = s.luna_method("com.webos.surfacemanager", "getForegroundAppInfo", &json!({}), "sam").unwrap();
        let r = s.luna_method("com.webos.service.recent", "listRecents", &json!({}), "home").unwrap();
        (f, r)
    });
    assert_eq!(fg["foregroundAppInfo"][0]["appId"], APP, "{fg}");
    assert_eq!(fg["foregroundAppInfo"][0]["windowType"], "_WEBOS_WINDOW_TYPE_CARD");
    // (The pid is the socket pair's creator's, the test's: a socket made by connect() would say the app's.)
    assert!(fg["foregroundAppInfo"][0]["processId"].as_str().is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())));
    assert_eq!(fg["foregroundAppInfo"][0]["params"], json!({}));
    assert!(recents["recents"].as_array().unwrap().iter().any(|r| r["appId"] == APP), "{recents}");

    // What it shows: the shipped wallpaper, upright, with the card's glass over it; and not a blank page.
    std::thread::sleep(Duration::from_millis(300));
    let image = capture(&server, "status");
    let wall =
        lumatv_rustui::home::wallpaper_pixels(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../payload/wallpaper/wall_1080.png"), 1920, 1080)
            .unwrap();
    let diff = |flip: bool| -> u64 {
        let mut sum = 0u64;
        for y in (10..160).step_by(10) {
            for x in (10..300).step_by(10) {
                let sy = if flip { 1079 - y } else { y };
                let at = ((sy * 1920 + x) * 4) as usize;
                let got = image.pixel(x as u32, y as u32);
                sum += (0..3).map(|c| (i64::from(got[c]) - i64::from(wall[at + c])).unsigned_abs()).sum::<u64>();
            }
        }
        sum
    };
    assert!(diff(false) * 2 < diff(true), "the wallpaper is upright: {} against {} upside down", diff(false), diff(true));
    let glass = image.pixel(960, 700);
    let outside = image.pixel(100, 700);
    assert_ne!(glass[..3], outside[..3], "the card's glass is over the wallpaper");

    // No GL error, no protocol error.
    let log = app.log_text();
    assert!(!log.contains("GL error"), "{log}");
    assert!(log.contains("first frame presented"), "{log}");
    let (errors, journal) = server.eval(|s| (s.stats.gl_errors, s.journal.clone()));
    assert_eq!(errors, 0);
    assert!(!journal.iter().any(|l| l.contains("error")), "{journal:?}");
    drop(app);
}

#[test]
fn back_goes_up_a_page_and_at_the_first_page_the_app_ends_and_the_card_below_comes_back() {
    let _one = ONE_AT_A_TIME.lock();
    let mut server = Server::start();
    let mut app = start(&mut server, "back", &[]);
    let delivered = server.eval(|s| s.keys.delivered);

    // OK on the focused Keys button opens the keys page; the key reached the app (Back and OK are its, by KEYS_BACK).
    remote(&server, OK);
    app.wait_heard(Duration::from_secs(10), json!({"kind": "page", "page": "keys"}));
    remote(&server, 106); // a key to show on the page
    std::thread::sleep(Duration::from_millis(400));
    capture(&server, "keys");
    // Back goes up: the app is still there.
    remote(&server, BACK);
    app.wait_heard(Duration::from_secs(10), json!({"kind": "page", "page": "status"}));
    assert!(app.exited().is_none(), "Back on the keys page does not end the app");
    let (now, sent) = server.eval(|s| (s.keys.delivered, s.luna_sent()));
    assert!(now >= delivered + 6, "the keys reached the app: {delivered} -> {now}");
    assert!(!sent.iter().any(|(uri, _)| uri.ends_with("closeByAppId")), "the compositor did not close the card on Back: {sent:?}");
    assert_eq!(state_of(&server, APP), "Fullscreen");

    // Back on the first page ends it, and says why.
    remote(&server, BACK);
    let status = app.wait_exit(Duration::from_secs(10));
    assert!(status.success(), "{status:?}\n{}", app.log_text());
    app.wait_heard(Duration::from_secs(1), json!({"kind": "exit", "why": "Back at the first page"}));
    assert!(wait_for(Duration::from_secs(5), || window(&server, APP).is_none_or(|w| w["mapped"] == false)), "the card is gone");
    assert!(foreground(&server).is_empty(), "nothing in front");
}

#[test]
fn home_covers_the_card_which_stops_drawing_and_a_relaunch_brings_it_back() {
    let _one = ONE_AT_A_TIME.lock();
    let mut server = Server::start();
    let mut app = start(&mut server, "home", &[]);
    app.say(json!({"kind": "sam", "registered": true}));

    // The Home key is the compositor's: LG's `launchDefaultApp`, not a key for the app.
    let delivered = server.eval(|s| s.keys.delivered);
    remote(&server, HOME);
    let (after, sent) = server.eval(|s| (s.keys.delivered, s.luna_sent()));
    assert_eq!(after, delivered, "the app was not sent Home");
    let launch = sent.iter().find(|(uri, _)| uri.ends_with("applicationManager/launchDefaultApp")).expect("launchDefaultApp");
    assert_eq!(launch.1["category"], "home");
    assert_eq!(launch.1["params"]["launchType"], "homeKey");

    // SAM starts Home: it maps over the card, which is told it is minimized and stops drawing.
    let mut client = second_client(&mut server);
    let home = client.card("_WEBOS_WINDOW_TYPE_CARD", "Home");
    home.webos.set_property("appId".into(), "com.webos.app.home".into());
    let blue = client.shm_buffer(1920, 1080, 0xff10_1060);
    client.present(&home.surface, &blue);
    assert!(wait_for(Duration::from_secs(10), || state_of(&server, APP) == "Minimized"), "the card was covered");
    app.wait_heard(Duration::from_secs(5), json!({"kind": "state", "state": "minimized"}));
    assert_eq!(foreground(&server), ["com.webos.app.home"]);
    // The switcher lists it now that it is behind (LG's list holds only cards that are not in front).
    let recents = server.eval(|s| s.luna_method("com.webos.service.recent", "listRecents", &json!({}), "home").unwrap());
    assert!(recents["recents"].as_array().unwrap().iter().any(|r| r["appId"] == APP), "{recents}");
    std::thread::sleep(Duration::from_millis(500));
    let before = commits(&server);
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(commits(&server), before, "a covered card commits nothing, not even its uptime");
    assert!(app.exited().is_none());

    // SAM's relaunch: the app asks for fullscreen, comes to the front, Home is minimized, and the card draws again.
    app.say(json!({"kind": "relaunch", "reason": "undefined", "parameters": {"launchType": "homeKey"}}));
    assert!(wait_for(Duration::from_secs(10), || state_of(&server, APP) == "Fullscreen"), "relaunched card is in front");
    assert_eq!(foreground(&server), [APP]);
    assert_eq!(state_of(&server, "com.webos.app.home"), "Minimized");
    app.wait_heard(Duration::from_secs(5), json!({"kind": "state", "state": "fullscreen"}));
    assert!(wait_for(Duration::from_secs(10), || commits(&server) > before), "the card draws again");
    capture(&server, "relaunched");

    // SAM's close: the app ends, and Home is in front again.
    app.say(json!({"kind": "close", "reason": "undefined"}));
    let status = app.wait_exit(Duration::from_secs(10));
    assert!(status.success(), "{status:?}");
    assert!(wait_for(Duration::from_secs(5), || state_of(&server, "com.webos.app.home") == "Fullscreen"));
    assert_eq!(foreground(&server), ["com.webos.app.home"]);
    drop(client);
}

#[test]
fn a_preloaded_app_makes_no_window_to_see_until_it_is_relaunched() {
    let _one = ONE_AT_A_TIME.lock();
    let mut server = Server::start();
    let mut app = App::spawn(&mut server, "preload", &["--hidden", "--reason", "preload"]);
    assert!(wait_for(SLOW, || window(&server, APP).is_some()), "the window exists\n{}", app.log_text());
    std::thread::sleep(Duration::from_millis(1500));
    let w = window(&server, APP).unwrap();
    assert_eq!(w["mapped"], false, "no buffer, no picture: {w}");
    assert_eq!(w["state"], "Default", "nothing asked for");
    assert!(foreground(&server).is_empty());
    assert!(!app.heard(json!({"kind": "ready"})));

    app.say(json!({"kind": "relaunch", "reason": "undefined", "parameters": {}}));
    assert!(wait_for(SLOW, || window(&server, APP).is_some_and(|w| w["mapped"] == true)), "the relaunch shows the card\n{}", app.log_text());
    assert_eq!(state_of(&server, APP), "Fullscreen");
    assert_eq!(foreground(&server), [APP]);
    app.wait_heard(SLOW, json!({"kind": "ready"}));
}

#[test]
fn the_compositors_close_and_a_lost_launcher_end_the_app() {
    let _one = ONE_AT_A_TIME.lock();
    let mut server = Server::start();
    let mut app = start(&mut server, "close", &[]);
    // `closeByAppId` (SAM, the key filters, LG's recents): `close` to the window.
    let closed = server.eval(|s| s.close_app(APP));
    assert_eq!(closed, 1);
    let status = app.wait_exit(Duration::from_secs(10));
    assert!(status.success(), "{status:?}");
    app.wait_heard(Duration::from_secs(1), json!({"kind": "exit", "why": "the compositor closed the window"}));
    drop(app);
    drop(server);

    let mut server = Server::start();
    let mut app = start(&mut server, "gone", &[]);
    // The launcher died: nobody answers for the app any more.
    let AppParts { channel, .. } = app.take_channel();
    drop(channel);
    let status = app.wait_exit(Duration::from_secs(10));
    assert!(status.success(), "{status:?}");
}

struct AppParts {
    channel: UnixStream,
}

impl App {
    /// The launcher's end of the channel, taken so that dropping it hangs up.
    fn take_channel(&mut self) -> AppParts {
        let (replacement, _other) = UnixStream::pair().unwrap();
        AppParts { channel: std::mem::replace(&mut self.channel, replacement) }
    }
}
