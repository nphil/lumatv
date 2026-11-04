//! Leaving an app by Back when the app's window goes with it (a web app under WAM such as Homebrew Channel, a native app such as Punktfunk that
//! exits): Home takes the remote's keys again with no pointer movement, and the app manager's foreground list says Home is in front. The Rust
//! shell's Home wants the keys while the `foreground` topic says it is in front and not covered (`rustui/ui/src/shell.rs` `report`), so the topic has
//! to follow a card that is destroyed as it follows one that is unmapped or minimized. TV, 2026-11-03 23:43 and 23:44: after Back on Homebrew Channel
//! every arrow (and once a second Back) was dropped until a turn of the wheel showed the pointer and the cursor topic carried Home's state to the shell.

mod harness;

use std::io::Read;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use harness::{Client, Server, Window};
use lumatv_compositor::State;
use lumatv_compositor::input::code::*;
use lumatv_compositor::luna::{Inbound, ToBridge};
use lumatv_compositor::server::seat::SeatId;
use lumatv_compositor::shell::Msg;
use serde_json::{Value, json};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_keyboard::WlKeyboard;

const WAIT: Duration = Duration::from_secs(3);
const HOME: &str = "com.webos.app.home";
const HBC: &str = "org.webosbrew.hbchannel";
const PUNKTFUNK: &str = "io.unom.punktfunk.client-webos";
const FULLSCREEN: u32 = 3;

/// An app's client: its own connection, a keyboard and one fullscreen card.
struct App {
    client: Client,
    win: Window,
    _kb: WlKeyboard,
    _buf: WlBuffer,
}

/// A card of `app_id` in its own client, mapped and holding the keyboard focus.
fn app(server: &mut Server, app_id: &str) -> App {
    let mut client = Client::connect_extra(server);
    let kb = client.seat.get_keyboard(&client.qh, ());
    let win = client.card_as("_WEBOS_WINDOW_TYPE_CARD", app_id, app_id);
    let buf = client.shm_buffer(1920, 1080, 0xff00_0000);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.keyboard_enters >= 1), "{app_id} has the keyboard focus");
    App { client, win, _kb: kb, _buf: buf }
}

/// LG's Home card in front, the Rust shell's Home on, and the shell's channel open with a client the test answers for.
struct Rig {
    server: Server,
    /// LG's Home app: it asks for fullscreen when SAM launches it.
    home: Client,
    home_win: Window,
    _home_buf: WlBuffer,
    /// The shell's end of the key socket.
    keys: UnixStream,
    bytes: Vec<u8>,
}

fn rig() -> Rig {
    let mut server = Server::start();
    let mut home = Client::connect(&mut server);
    let home_win = home.card_as("_WEBOS_WINDOW_TYPE_CARD", "Home", HOME);
    let home_buf = home.shm_buffer(1920, 1080, 0xff20_3060);
    home.present(&home_win.surface, &home_buf);
    home.roundtrip();
    let (ours, keys) = UnixStream::pair().unwrap();
    ours.set_nonblocking(true).unwrap();
    keys.set_nonblocking(true).unwrap();
    server.eval(move |s| {
        s.scene.set_home_rust(true);
        s.shell_rust_open(0, "test".into()).unwrap();
        s.shell_rust.kinds = vec!["home".into()];
        s.shell_rust.adopt_key_socket(ours);
        s.shell_rust_handle(Msg::Connected);
        // SAM's subscription, to read what the app manager is told
        s.luna_inbound(Inbound {
            token: 11,
            service: "com.webos.surfacemanager".into(),
            method: "getForegroundAppInfo".into(),
            payload: json!({"subscribe": true}),
            subscribe: true,
            sender: "com.webos.applicationManager".into(),
            cancelled: false,
        });
        shell_follows_topics(s);
    });
    Rig { server, home, home_win, _home_buf: home_buf, keys, bytes: Vec::new() }
}

/// What the Rust shell does with the `foreground` topic (`rustui/ui/src/home/model.rs`, `shell.rs` `report`): Home is active when the compositor says it
/// is in front and not covered, and an active Home wants the keys. The shell answers each topic over `POST /v1/ui`; here the test says when it has.
fn shell_follows_topics(s: &mut State) {
    let topic = s.shell_rust.topic("foreground").cloned().expect("the foreground topic");
    let home = topic["home"].as_bool().unwrap_or(false);
    let covered = topic["covered"].as_bool().unwrap_or(!home);
    s.shell_rust_handle(Msg::Ui { can: None, wants: Some(home && !covered), live: None, state: None });
}

impl Rig {
    /// The shell has read what the compositor published and answered.
    fn shell_answers(&self) {
        self.server.eval(shell_follows_topics);
    }

    /// A key of the remote, pressed and released, with no pointer.
    fn tap(&self, code: u32) {
        self.server.eval(move |s| {
            let now = Instant::now();
            s.key_event_at(SeatId(0), code, true, false, now);
            s.key_event_at(SeatId(0), code, false, false, now + Duration::from_millis(60));
        });
    }

    /// The keys the shell was sent since the last call (presses, by name).
    fn shell_keys(&mut self) -> Vec<String> {
        let mut buf = [0; 4096];
        loop {
            match self.keys.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => self.bytes.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => panic!("key socket: {e}"),
            }
        }
        let text = String::from_utf8(std::mem::take(&mut self.bytes)).unwrap();
        text.lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .filter(|e| e["key"]["d"] == true)
            .map(|e| e["key"]["k"].as_str().unwrap().to_string())
            .collect()
    }

    /// The apps SAM was last told are in front.
    fn sam_foreground(&self) -> Vec<String> {
        self.server.eval(|s| {
            let last = s.luna.history().iter().rev().find_map(|m| match m {
                ToBridge::Reply(r) if r.token == 11 => Some(r.payload.clone()),
                _ => None,
            });
            last.map(|p| p["foregroundAppInfo"].as_array().unwrap().iter().map(|a| a["appId"].as_str().unwrap().to_string()).collect()).unwrap_or_default()
        })
    }

    fn foreground_topic(&self) -> Value {
        self.server.eval(|s| s.shell_rust.topic("foreground").cloned().unwrap())
    }

    /// Nothing in the test moved or showed the pointer.
    fn no_pointer(&self) {
        let (motions, visible) = self.server.eval(|s| (s.seat.motions, s.seat.cursor_visible));
        assert_eq!((motions, visible), (0, false), "the test never touches the pointer");
    }

    /// Waits for the server thread to have `f` hold (a client's disconnect is handled there, apart from the test's own calls).
    fn wait_server(&self, what: &str, f: impl Fn(&mut State) -> bool + Send + Clone + 'static) {
        let deadline = Instant::now() + WAIT;
        while !self.server.eval(f.clone()) {
            assert!(Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[test]
fn back_out_of_a_web_app_that_closes_its_window_gives_home_the_keys_with_no_pointer() {
    let mut rig = rig();
    rig.tap(KEY_RIGHT);
    assert_eq!(rig.shell_keys(), ["right"], "Home in front has the keys");
    assert_eq!(rig.sam_foreground(), [HOME]);

    // Homebrew Channel under WAM: a card of its own client, which has the focus and no policy for Back.
    let mut hbc = app(&mut rig.server, HBC);
    rig.shell_answers();
    assert_eq!(rig.sam_foreground(), [HBC]);
    rig.tap(KEY_RIGHT);
    assert_eq!(rig.shell_keys(), Vec::<String>::new(), "under an app Home gets no key");

    // Back is the compositor's: the app is asked to close, and WAM closes the window by destroying its surface (nothing is unmapped first).
    rig.tap(KEY_PREVIOUS);
    assert!(hbc.client.wait_until(WAIT, |s| s.closes >= 1), "Back asks the app to close");
    hbc.win.surface.destroy();
    hbc.client.roundtrip();

    rig.shell_answers();
    rig.tap(KEY_RIGHT);
    rig.tap(KEY_ENTER);
    let topic = rig.foreground_topic();
    assert_eq!(rig.shell_keys(), ["right", "ok"], "Home has the keys again; the shell was last told {topic}");
    assert_eq!((topic["app"].as_str(), topic["home"].as_bool(), topic["covered"].as_bool()), (Some(HOME), Some(true), Some(false)), "{topic}");
    assert_eq!(rig.sam_foreground(), [HOME], "the app manager was told");
    rig.no_pointer();
}

#[test]
fn an_app_that_exits_leaves_home_the_keys() {
    // Its connection closes (a native app such as Punktfunk ends, WAM is restarted): the surface and both shell roles go with the client.
    let mut rig = rig();
    let app = app(&mut rig.server, PUNKTFUNK);
    rig.shell_answers();
    rig.tap(KEY_RIGHT);
    assert_eq!(rig.shell_keys(), Vec::<String>::new());
    drop(app);
    rig.wait_server("the app's card goes with its client", |s| s.scene.windows().all(|w| w.app_id() != PUNKTFUNK));
    rig.shell_answers();
    assert_eq!(rig.sam_foreground(), [HOME]);
    rig.tap(KEY_LEFT);
    assert_eq!(rig.shell_keys(), ["left"], "Home has the keys again");
    rig.no_pointer();
}

#[test]
fn an_app_that_stays_covered_by_homes_launch_leaves_home_the_keys() {
    // An app that leaves by the Home key: LG's Home app asks for fullscreen and the app is covered, its window stays (Marquee, Netflix).
    let mut rig = rig();
    let _app = app(&mut rig.server, "org.test.native");
    rig.shell_answers();
    rig.tap(KEY_RIGHT);
    assert_eq!(rig.shell_keys(), Vec::<String>::new());
    rig.home_win.webos.set_state(FULLSCREEN);
    rig.home.roundtrip();
    rig.shell_answers();
    assert_eq!(rig.sam_foreground(), [HOME]);
    rig.tap(KEY_DOWN);
    assert_eq!(rig.shell_keys(), ["down"]);
    rig.no_pointer();
}

#[test]
fn closing_the_app_over_an_app_gives_the_keys_to_the_one_below_not_to_home() {
    // App to app: B was launched over A and closes by Back; A is in front again, has the keyboard and the keys, and Home stays covered.
    let mut rig = rig();
    let mut a = app(&mut rig.server, "org.test.a");
    let mut b = app(&mut rig.server, HBC);
    rig.shell_answers();
    rig.tap(KEY_PREVIOUS);
    assert!(b.client.wait_until(WAIT, |s| s.closes >= 1), "Back asks the app in front to close");
    b.win.surface.destroy();
    b.client.roundtrip();
    rig.shell_answers();
    assert_eq!(rig.sam_foreground(), ["org.test.a"]);
    let topic = rig.foreground_topic();
    assert_eq!((topic["app"].as_str(), topic["home"].as_bool()), (Some("org.test.a"), Some(false)), "{topic}");
    rig.tap(KEY_RIGHT);
    assert!(a.client.wait_until(WAIT, |s| s.keys.iter().any(|(code, _)| *code == KEY_RIGHT)), "the app below has the key: {:?}", a.client.state.keys);
    assert_eq!(rig.shell_keys(), Vec::<String>::new(), "Home is covered");
    rig.no_pointer();
}

#[test]
fn an_overlay_that_goes_leaves_the_foreground_list_at_once() {
    // A window that is not a card is in the list too (LG's Settings over an app); its surface going is the same publish.
    let mut rig = rig();
    let _a = app(&mut rig.server, "org.test.a");
    let mut other = Client::connect_extra(&mut rig.server);
    let overlay = other.card_as("_WEBOS_WINDOW_TYPE_OVERLAY", "Settings", "org.test.settings");
    let buf = other.shm_buffer(640, 360, 0xff20_2020);
    other.present(&overlay.surface, &buf);
    other.roundtrip();
    assert_eq!(rig.sam_foreground(), ["org.test.a", "org.test.settings"]);
    overlay.surface.destroy();
    other.roundtrip();
    assert_eq!(rig.sam_foreground(), ["org.test.a"]);
}
