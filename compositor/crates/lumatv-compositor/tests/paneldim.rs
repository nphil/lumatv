//! The panel dim on the scene (`paneldim`, `server/state.rs`): idle Home's ease and the Rust shell's saver go to the panel's own light through the
//! settings service (a stand-in on the luna bridge here), Home stays at full brightness in the plan so the plane takes it whole, and every way
//! out puts the user's value back.

mod harness;

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use harness::{Client, Server, Window};
use lumatv_compositor::Rect;
use lumatv_compositor::State;
use lumatv_compositor::luna::{Bridge, Event, FromBridge, ToBridge};
use lumatv_compositor::output::monotonic_ns;
use lumatv_compositor::paneldim::{JOURNAL, READ_URI, WRITE_URI};
use lumatv_compositor::scene::Hole;
use lumatv_compositor::window::WindowType;
use parking_lot::Mutex;
use serde_json::{Value, json};
use wayland_client::protocol::wl_buffer::WlBuffer;

const SEC: u64 = 1_000_000_000;
const MS: u64 = 1_000_000;

/// LG's settings service on the other end of the bridge: one picture setting of one picture mode, answered at once (the answer is read at the
/// next poll of the bridge, as it is on the set).
struct Tv {
    value: u32,
    dimension: Value,
    refuse: bool,
    writes: Vec<(String, Value)>,
    reads: u64,
    events: VecDeque<FromBridge>,
    /// Every call that reached the bridge, in order: (uri, token).
    sent: Vec<(String, u64)>,
}

#[derive(Clone)]
struct Stand(Arc<Mutex<Tv>>);

impl Stand {
    fn new(value: u32) -> Stand {
        Stand(Arc::new(Mutex::new(Tv {
            value,
            dimension: json!({"pictureMode": "filmMaker", "dynamicRange": "sdr", "input": "hdmi2"}),
            refuse: false,
            writes: Vec::new(),
            reads: 0,
            events: VecDeque::new(),
            sent: Vec::new(),
        })))
    }

    fn value(&self) -> u32 {
        self.0.lock().value
    }

    fn writes(&self) -> Vec<String> {
        self.0.lock().writes.iter().map(|w| w.0.clone()).collect()
    }
}

impl Bridge for Stand {
    fn send(&mut self, msg: ToBridge) {
        let ToBridge::Outbound(o) = msg else { return };
        let mut tv = self.0.lock();
        tv.sent.push((o.uri.clone(), o.token));
        let payload = match o.uri.as_str() {
            READ_URI => {
                tv.reads += 1;
                json!({"returnValue": true, "category": "picture", "settings": {"backlight": tv.value.to_string()}, "dimension": tv.dimension})
            }
            WRITE_URI => {
                let v = o.payload["settings"]["backlight"].as_str().unwrap().to_string();
                tv.writes.push((v.clone(), o.payload["dimension"].clone()));
                if tv.refuse {
                    json!({"returnValue": false, "errorText": "refused"})
                } else {
                    tv.value = v.parse().unwrap();
                    json!({"returnValue": true})
                }
            }
            _ => return,
        };
        tv.events.push_back(FromBridge::Event(Event { token: o.token, payload }));
    }

    fn poll(&mut self) -> Vec<FromBridge> {
        self.0.lock().events.drain(..).collect()
    }

    fn is_connected(&self) -> bool {
        true
    }
}

struct Shown {
    _client: Client,
    _win: Window,
    _buf: WlBuffer,
}

fn home(server: &mut Server) -> Shown {
    let mut client = Client::connect(server);
    let win = client.card("_WEBOS_WINDOW_TYPE_LUMATV_HOME", "LumaHome");
    win.webos.set_property("appId".into(), "org.nphil.lumatv.shell".into());
    let buf = client.shm_buffer(1920, 1080, 0xff00_00ff);
    client.present(&win.surface, &buf);
    client.roundtrip();
    Shown { _client: client, _win: win, _buf: buf }
}

fn quiet_server(tv: &Stand) -> Server {
    let server = Server::start();
    server.ticking.store(false, Ordering::SeqCst);
    let bridge = tv.clone();
    server.eval(move |s| {
        s.luna = Box::new(bridge);
        // a data directory of this test's own: the journal is its file
        let dir = journal_of().parent().unwrap().to_path_buf();
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        s.panel = lumatv_compositor::paneldim::PanelDim::new(&dir);
    });
    server
}

fn journal_of() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("paneldim-it-{}-{:?}", std::process::id(), std::thread::current().id())).join(JOURNAL)
}

/// The bridge's answers delivered, as the loop does at each wake of the luna socket.
fn deliver(s: &mut State) {
    for m in s.luna.poll() {
        if let FromBridge::Event(e) = m {
            s.luna_event(e);
        }
    }
}

/// Ticks of 250 ms from `t` for `ms`; the time reached.
fn run(s: &mut State, mut t: u64, ms: u64) -> u64 {
    let end = t + ms * MS;
    while t < end {
        t += 250 * MS;
        s.tick_at(t);
        deliver(s);
    }
    t
}

/// What the last plan drew Home at, and whether it goes to the plane whole.
fn plan(s: &State) -> (f32, bool) {
    let p = s.scene.last_plan();
    let home = p.items.iter().find(|i| s.scene.window(i.window).is_some_and(|w| w.kind == WindowType::ShellHome)).expect("Home is drawn");
    (home.dim, p.sole_fullscreen().is_some())
}

fn care_on(s: &mut State) {
    let c = &mut s.scene.care.settings;
    c.enabled = true;
    c.radius = 0;
    c.dim_after_ns = SEC;
    c.dim_over_ns = SEC;
    c.dim_floor = 0.8;
}

#[test]
fn an_idle_home_dims_the_panel_not_the_picture_and_the_plane_still_takes_it_whole() {
    let tv = Stand::new(80);
    let mut server = quiet_server(&tv);
    let _home = home(&mut server);
    server.eval(|s| {
        care_on(s);
        let t = monotonic_ns();
        let mut t = run(s, t, 500);
        assert_eq!(plan(s), (1.0, true), "bright: direct");
        // still for longer than the wait, a beat every 30 s and its two seconds' grace: the care's level is 0.8 after the first beat
        t = run(s, t, 33_000);
        assert_eq!(s.scene.care.home_want(), 0.8, "Home's ease has run");
        t = run(s, t, 40_000);
        assert_eq!(s.scene.care.dim(0), 1.0, "and is not what is drawn");
        assert_eq!(plan(s), (1.0, true), "Home is drawn at full brightness and may go to the plane whole while the panel is dim");
        let _ = t;
    });
    assert_eq!(tv.value(), 64, "round(80 x 0.8)");
    assert!(tv.writes().iter().all(|w| w.parse::<u32>().is_ok()));
    assert_eq!(tv.0.lock().writes.len(), 16, "one unit a write");
}

#[test]
fn a_key_puts_the_value_back_and_the_journal_goes() {
    let tv = Stand::new(80);
    let mut server = quiet_server(&tv);
    let _home = home(&mut server);
    let t = server.eval(|s| {
        care_on(s);
        let t = run(s, monotonic_ns(), 75_000);
        assert!(journal_of().exists());
        t
    });
    assert_eq!(tv.value(), 64);
    server.eval(move |s| {
        s.care_input();
        deliver(s);
        deliver(s);
        assert!(!journal_of().exists());
        let t = run(s, t, 1_000);
        assert_eq!(plan(s), (1.0, true));
        // still, again: the hold after a key and the wait of the care come first
        run(s, t, 5_000);
    });
    assert_eq!(tv.value(), 80, "the user's exact value");
}

#[test]
fn with_the_switch_off_the_gpu_dims_as_before_and_the_panel_is_not_asked() {
    let tv = Stand::new(80);
    let mut server = quiet_server(&tv);
    let _home = home(&mut server);
    server.eval(|s| {
        care_on(s);
        s.scene.care.settings.paneldim.enabled = false;
        let t = run(s, monotonic_ns(), 500);
        let t = run(s, t, 75_000);
        assert_eq!(plan(s), (0.8, false), "drawn dark, composed");
        let _ = t;
    });
    let (value, reads, writes) = {
        let t = tv.0.lock();
        (t.value, t.reads, t.writes.len())
    };
    assert_eq!((value, reads, writes), (80, 0, 0));
}

#[test]
fn a_refused_write_hands_the_dim_back_to_the_gpu() {
    let tv = Stand::new(80);
    tv.0.lock().refuse = true;
    let mut server = quiet_server(&tv);
    let _home = home(&mut server);
    server.eval(|s| {
        care_on(s);
        let t = run(s, monotonic_ns(), 500);
        let _ = run(s, t, 75_000);
        assert!(!s.panel.carries());
        assert_eq!(plan(s), (0.8, false), "the picture is dimmed as it was before the panel did it");
        assert!(s.journal.iter().any(|l| l.contains("the GPU dim does the work")), "{:?}", s.journal);
    });
    assert_eq!(tv.0.lock().writes.len(), 1, "one try, then nothing for ten minutes");
}

#[test]
fn a_video_hole_a_standby_and_the_saver_leaving_each_put_the_value_back() {
    for how in 0..3 {
        let tv = Stand::new(80);
        let mut server = quiet_server(&tv);
        let _home = home(&mut server);
        server.eval(move |s| {
            let t = run(s, monotonic_ns(), 500);
            s.panel_want_ctl = Some(0.5);
            let t = run(s, t, 100_000);
            assert_eq!(s.panel.status()["written"], 40);
            match how {
                0 => {
                    let surface = s.scene.windows().find(|w| w.kind == WindowType::ShellHome).unwrap().surface;
                    s.scene.holes.push(Hole { surface, rect: Rect::new(160, 90, 1280, 720) });
                }
                1 => s.luna_state.power.rendering = false,
                _ => s.panel_want_ctl = Some(1.0),
            }
            let t = run(s, t, 2_000);
            assert_eq!(s.panel.status()["dimmed"], false, "{how}");
            run(s, t, 20_000);
        });
        assert_eq!(tv.value(), 80, "{how}");
        let n = tv.0.lock().writes.len();
        assert!(n >= 41, "40 down and one back: {n}");
    }
}

#[test]
fn the_rust_shells_saver_is_dimmed_at_its_own_level_when_it_says_it_does_not_draw_that_dim() {
    let tv = Stand::new(80);
    let mut server = quiet_server(&tv);
    let _home = home(&mut server);
    server.eval(|s| {
        let t = run(s, monotonic_ns(), 500);
        // a saver that still draws its own dim is not asked of the panel
        s.shell_rust.ui_state = json!({"saver": true, "saverState": {"level": 0.5, "carry": false}});
        let t = run(s, t, 30_000);
        assert_eq!(s.panel.status()["written"], Value::Null);
        // one that hands it over: the panel follows its level
        s.shell_rust.ui_state = json!({"saver": true, "saverState": {"level": 0.6, "carry": true}});
        let t = run(s, t, 100_000);
        assert_eq!(s.panel.status()["written"], 48, "round(80 x 0.6)");
        assert!(s.scene.care.saver);
        // it wakes: the next report says it is down
        s.shell_rust.ui_state = json!({"saver": false});
        run(s, t, 3_000);
        assert!(!s.panel.dimmed());
    });
    assert_eq!(tv.value(), 80);
}

#[test]
fn ctl_says_what_the_panel_dim_is_doing_and_can_ask_for_a_dim_by_hand() {
    let tv = Stand::new(80);
    let server = quiet_server(&tv);
    server.eval(|s| {
        let out: Value = serde_json::from_str(&lumatv_compositor::control::handle(s, "paneldim want 0.9").text()).unwrap();
        assert_eq!(out["want"], 0.9f32 as f64);
        let t = run(s, monotonic_ns(), 30_000);
        let st: Value = serde_json::from_str(&lumatv_compositor::control::handle(s, "status").text()).unwrap();
        let p = &st["oledcare"]["panelDim"];
        assert_eq!(
            (p["enabled"].clone(), p["carries"].clone(), p["dimmed"].clone(), p["orig"].clone(), p["written"].clone()),
            (json!(true), json!(true), json!(true), json!(80), json!(72))
        );
        let bad = lumatv_compositor::control::handle(s, "paneldim want 3").text();
        assert!(bad.contains("\"ok\":false"), "{bad}");
        let out = lumatv_compositor::control::handle(s, "paneldim release").text();
        assert!(out.contains("\"ok\":true"));
        run(s, t, 2_000);
        assert!(!s.panel.dimmed());
    });
    assert_eq!(tv.value(), 80);
}

#[test]
fn the_exit_puts_the_value_back_and_waits_for_the_answer() {
    let tv = Stand::new(80);
    let server = quiet_server(&tv);
    server.eval(|s| {
        s.panel_want_ctl = Some(0.5);
        let _ = run(s, monotonic_ns(), 100_000);
        assert_eq!(s.panel.status()["written"], 40);
        s.panel_exit(std::time::Duration::from_millis(500));
        assert!(!s.panel.dimmed());
        assert!(!journal_of().exists());
    });
    assert_eq!(tv.value(), 80);
}

#[test]
fn the_put_back_goes_out_before_the_answer_that_lets_the_tv_power_down() {
    let tv = Stand::new(80);
    let server = quiet_server(&tv);
    let handle = tv.clone();
    server.eval(move |s| {
        s.luna_register();
        s.panel_want_ctl = Some(0.5);
        run(s, monotonic_ns(), 100_000);
        assert_eq!(s.panel.status()["written"], 40);
        let off = handle.0.lock().sent.iter().find(|(u, _)| u.ends_with("/registerPreparePowerOff")).expect("registered with tvpower").1;
        // tvpower asks the compositor to get ready for standby: the push on the prepare-off registration
        s.luna_event(Event { token: off, payload: json!({"timestamp": 9, "next sub state": "active standby"}) });
        deliver(s);
        deliver(s);
    });
    let order: Vec<String> = tv.0.lock().sent.iter().map(|(u, _)| u.clone()).collect();
    let ack = order.iter().position(|u| u.ends_with("/responsePreparePowerOff")).expect("the round is answered");
    let write = order.iter().rposition(|u| u == WRITE_URI).unwrap();
    assert!(write < ack, "the value is on its way back before tvpower is told it may go: {order:?}");
    assert_eq!(tv.value(), 80, "and it is back");
}

#[test]
fn the_rust_shells_home_is_idle_to_the_care_too_and_the_panel_carries_its_ease_with_a_call_a_step_and_a_poll_a_while() {
    let tv = Stand::new(80);
    let server = quiet_server(&tv);
    let handle = tv.clone();
    server.eval(move |s| {
        // Home is the Rust shell's: no window of the stack, nothing a commit counts; a key or a clock a second changes nothing here
        s.scene.set_home_rust(true);
        care_on(s);
        let t = run(s, monotonic_ns(), 500);
        assert!(s.scene.care.beats == 0 && s.scene.care.home_want() == 1.0);
        let t = run(s, t, 120_000);
        assert!(s.scene.care.beats >= 3, "the beats run on a Home with no window: {}", s.scene.care.beats);
        assert_eq!(s.scene.care.home_want(), 0.8, "the ease has run its length");
        assert_eq!(s.panel.status()["written"], 64, "and the panel is where it asks");
        let (reads, writes) = {
            let t = handle.0.lock();
            (t.reads, t.writes.len())
        };
        // two minutes more of a dim Home: only the poll every twenty seconds is asked, nothing per tick
        run(s, t, 120_000);
        let t = handle.0.lock();
        assert_eq!(t.writes.len(), writes, "no more writes");
        assert!((5..=7).contains(&(t.reads - reads)), "a read every 20 s: {}", t.reads - reads);
        // a key puts it back
        drop(t);
        s.care_input();
        deliver(s);
        deliver(s);
    });
    assert_eq!(tv.value(), 80);
}
