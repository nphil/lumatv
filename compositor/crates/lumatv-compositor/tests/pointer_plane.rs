//! The pointer on the cursor plane (`pointer`, docs/research/hw-planes-result.md) under the real `State`: an output that has a plane and records what it is
//! asked (over the software renderer for the frames that are composed). What it checks is what the TV check list asks of the switch: a move draws no frame and
//! leaves the picture alone, the sprite is back in the frame when the plane refuses or a capture is armed, a window alone on the screen is not refused
//! direct scan-out for the pointer's sake, and the control socket's words.

mod harness;

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use harness::{Client, Server};
use lumatv_compositor::control;
use lumatv_compositor::output::{Image, Output, Presented, RenderStats};
use lumatv_compositor::pointer::Mode;
use lumatv_compositor::render::software::Software;
use lumatv_compositor::scene::{NotSole, Plan};
use lumatv_compositor::server::seat::{hide_cursor, pointer_motion};
use lumatv_compositor::surface::Surfaces;
use parking_lot::Mutex;
use serde_json::{Value, json};
use wayland_server::protocol::wl_buffer::WlBuffer;

const WAIT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Log {
    /// What the pointer was last asked, one entry per change.
    asked: Vec<Option<(i32, i32)>>,
    /// Frames rendered, and whether each had the pointer's sprite in it.
    frames: usize,
    sprites: Vec<bool>,
    modes: Vec<Mode>,
}

struct Knobs {
    /// The plane carries the pointer (false: it is not up, or it failed).
    carries: AtomicBool,
    compose_only: AtomicBool,
    log: Mutex<Log>,
}

impl Knobs {
    fn frames(&self) -> usize {
        self.log.lock().frames
    }

    fn asked(&self) -> Vec<Option<(i32, i32)>> {
        self.log.lock().asked.clone()
    }
}

struct PlaneOut {
    inner: Software,
    knobs: Arc<Knobs>,
}

impl Output for PlaneOut {
    fn name(&self) -> &'static str {
        "plane-test"
    }

    fn size(&self) -> (u32, u32) {
        self.inner.size()
    }

    fn query_buffer(&mut self, buffer: &WlBuffer) -> Option<((i32, i32), bool)> {
        self.inner.query_buffer(buffer)
    }

    fn forget_buffer(&mut self, buffer: &WlBuffer) {
        self.inner.forget_buffer(buffer);
    }

    fn render(&mut self, plan: &Plan, surfaces: &Surfaces) -> io::Result<RenderStats> {
        {
            let mut l = self.knobs.log.lock();
            l.frames += 1;
            l.sprites.push(plan.cursor.is_some());
        }
        self.inner.render(plan, surfaces)
    }

    fn present(&mut self) -> io::Result<Presented> {
        self.inner.present()
    }

    fn capture(&mut self) -> io::Result<Image> {
        self.inner.capture()
    }

    fn set_compose_only(&mut self, on: bool) {
        self.knobs.compose_only.store(on, Ordering::SeqCst);
        self.inner.set_compose_only(on);
    }

    /// As the KMS output answers: a capture being armed or a failed plane leaves the sprite to the frame.
    fn set_pointer(&mut self, at: Option<(i32, i32)>) -> bool {
        let mut l = self.knobs.log.lock();
        if l.asked.last() != Some(&at) {
            l.asked.push(at);
        }
        self.knobs.carries.load(Ordering::SeqCst) && !self.knobs.compose_only.load(Ordering::SeqCst)
    }

    fn set_pointer_mode(&mut self, mode: Mode) -> io::Result<()> {
        self.knobs.log.lock().modes.push(mode);
        if mode == Mode::Plane && !self.knobs.carries.load(Ordering::SeqCst) { Err(io::Error::other("the test plane is not there")) } else { Ok(()) }
    }

    fn pointer_report(&self) -> Option<Value> {
        let up = self.knobs.carries.load(Ordering::SeqCst);
        Some(json!({"carried": up, "lastError": if up { "" } else { "the test plane failed" }}))
    }
}

fn rig(carries: bool) -> (Server, Arc<Knobs>) {
    let knobs = Arc::new(Knobs { carries: AtomicBool::new(carries), compose_only: AtomicBool::new(false), log: Mutex::new(Log::default()) });
    let k = knobs.clone();
    let server = Server::start_with(Box::new(move || Box::new(PlaneOut { inner: Software::new(1920, 1080), knobs: k })));
    (server, knobs)
}

/// A full-screen red app, shown and settled.
fn red_app(server: &mut Server) -> Client {
    let mut client = Client::connect(server);
    let app = client.card("_WEBOS_WINDOW_TYPE_CARD", "App");
    let red = client.shm_buffer(1920, 1080, 0xffff_0000);
    client.present(&app.surface, &red);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1), "the app's first frame");
    std::thread::sleep(Duration::from_millis(120));
    client
}

fn plane_on(server: &Server) {
    server.eval(|s| {
        s.config.pointer = Mode::Plane;
        s.output.set_pointer_mode(Mode::Plane).unwrap();
        s.pointer_sync();
    });
}

const RED: [u8; 4] = [255, 0, 0, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

#[test]
fn a_pointer_on_the_plane_moves_without_a_frame_and_leaves_the_picture_alone() {
    let (mut server, knobs) = rig(true);
    let _app = red_app(&mut server);
    plane_on(&server);
    std::thread::sleep(Duration::from_millis(60));
    let before = knobs.frames();
    for i in 0..40 {
        server.eval(move |s| {
            pointer_motion(s, 200 + i * 10, 300 + i * 5);
            s.tick();
        });
    }
    std::thread::sleep(Duration::from_millis(60));
    assert_eq!(knobs.frames(), before, "not one frame was drawn for forty moves of the pointer");
    assert_eq!(knobs.asked().last(), Some(&Some((590, 495))), "the plane was told the newest place");
    let (cursor, carried, image) = server.eval(|s| (s.scene.cursor, s.pointer_carried, s.output.capture().unwrap()));
    assert!(cursor.is_none() && carried, "no sprite for the frame while the plane carries it");
    assert_eq!(image.pixel(590, 495), RED, "the picture under the pointer is the app's");
    assert!(server.eval(|s| s.journal.iter().any(|l| l.contains("the cursor plane carries it"))));
}

#[test]
fn the_pointer_in_the_frame_is_what_it_was_and_draws_for_every_move() {
    let (mut server, knobs) = rig(true);
    let _app = red_app(&mut server);
    let before = knobs.frames();
    let image = server.eval(|s| {
        pointer_motion(s, 500, 400);
        s.tick();
        s.tick();
        s.output.capture().unwrap()
    });
    assert!(knobs.frames() > before, "the move damaged the pointer's rectangles and a frame drew them");
    assert_eq!(image.pixel(500, 400), WHITE, "the sprite's white middle, in the frame");
    assert!(server.eval(|s| s.scene.cursor.is_some() && !s.pointer_carried));
    assert_eq!(knobs.asked().last(), Some(&None), "the plane, if there were one, is told to show nothing");
}

#[test]
fn a_plane_that_fails_hands_the_sprite_back_to_the_frame() {
    let (mut server, knobs) = rig(true);
    let _app = red_app(&mut server);
    plane_on(&server);
    server.eval(|s| pointer_motion(s, 700, 300));
    assert!(server.eval(|s| s.pointer_carried));
    // the driver refuses a call: the output answers false from then on
    knobs.carries.store(false, Ordering::SeqCst);
    let image = server.eval(|s| {
        pointer_motion(s, 710, 305);
        s.tick();
        s.tick();
        s.output.capture().unwrap()
    });
    assert!(server.eval(|s| !s.pointer_carried && s.scene.cursor.is_some()));
    assert_eq!(image.pixel(710, 305), WHITE, "the pointer did not vanish");
    let journal = server.eval(|s| s.journal.clone());
    assert!(journal.iter().any(|l| l.contains("drawn into the frame") && l.contains("the test plane failed")), "{journal:?}");
}

#[test]
fn a_capture_gets_the_sprite_in_its_frames_and_the_plane_has_it_again_after() {
    let (mut server, knobs) = rig(true);
    let _app = red_app(&mut server);
    plane_on(&server);
    server.eval(|s| pointer_motion(s, 640, 360));
    assert!(server.eval(|s| s.pointer_carried));
    // a screenshot or recording arms the output's compose-only mode
    let image = server.eval(|s| {
        s.output.set_compose_only(true);
        s.tick();
        s.tick();
        s.output.capture().unwrap()
    });
    assert!(server.eval(|s| !s.pointer_carried && s.scene.cursor.is_some()));
    assert_eq!(image.pixel(640, 360), WHITE, "the capture shows the pointer");
    let image = server.eval(|s| {
        s.output.set_compose_only(false);
        s.tick();
        s.tick();
        s.output.capture().unwrap()
    });
    assert!(server.eval(|s| s.pointer_carried && s.scene.cursor.is_none()));
    assert_eq!(image.pixel(640, 360), RED, "and the frame is the app's again");
    assert!(knobs.frames() > 0);
}

#[test]
fn the_plane_shows_nothing_while_the_pointer_is_hidden_or_the_screen_is_off() {
    let (mut server, knobs) = rig(true);
    let _app = red_app(&mut server);
    plane_on(&server);
    server.eval(|s| pointer_motion(s, 300, 300));
    assert_eq!(knobs.asked().last(), Some(&Some((300, 300))));
    server.eval(hide_cursor);
    assert_eq!(knobs.asked().last(), Some(&None), "a navigation key hides it");
    server.eval(|s| pointer_motion(s, 310, 320));
    assert_eq!(knobs.asked().last(), Some(&Some((310, 320))));
    server.eval(|s| {
        s.luna_state.power.rendering = false;
        s.pointer_sync();
    });
    assert_eq!(knobs.asked().last(), Some(&None), "the screen is off");
    server.eval(|s| {
        s.luna_state.power.rendering = true;
        s.pointer_sync();
    });
    assert_eq!(knobs.asked().last(), Some(&Some((310, 320))), "and back where it was when the screen is");
}

#[test]
fn a_window_alone_on_the_screen_is_refused_the_plane_for_the_pointer_only_while_the_sprite_is_in_the_frame() {
    let (mut server, _knobs) = rig(true);
    let _app = red_app(&mut server);
    let sole = |s: &mut lumatv_compositor::State| s.scene.last_plan().sole_candidate().map(|_| ());
    server.eval(|s| {
        pointer_motion(s, 500, 500);
        s.tick();
    });
    assert_eq!(server.eval(move |s| sole(s)), Err(NotSole::Cursor), "the sprite is a second picture on the screen");
    plane_on(&server);
    server.eval(|s| {
        pointer_motion(s, 510, 505);
        s.tick();
        s.tick();
    });
    assert_eq!(server.eval(move |s| sole(s)), Ok(()), "with the plane the window is the whole picture, and scan-out may take it");
}

#[test]
fn ctl_pointer_reads_switches_and_still_moves() {
    let (mut server, knobs) = rig(true);
    let _app = red_app(&mut server);
    let ctl = |server: &Server, line: &'static str| -> Value { serde_json::from_str(&server.eval(move |s| control::handle(s, line).text())).unwrap() };
    let r = ctl(&server, "pointer");
    assert_eq!((r["ok"].as_bool(), r["pointer"]["mode"].as_str(), r["pointer"]["carried"].as_bool()), (Some(true), Some("gpu"), Some(false)));
    let r = ctl(&server, "pointer plane");
    assert_eq!((r["ok"].as_bool(), r["pointer"]["mode"].as_str()), (Some(true), Some("plane")), "{r}");
    assert_eq!(knobs.log.lock().modes.last(), Some(&Mode::Plane));
    let r = ctl(&server, "pointer 400 300");
    assert_eq!(r["ok"], true, "the words that moved the pointer still do: {r}");
    let r = ctl(&server, "pointer");
    assert_eq!((r["pointer"]["carried"].as_bool(), r["pointer"]["at"].clone()), (Some(true), json!([400, 300])));
    assert_eq!(r["pointer"]["plane"]["carried"], true);
    let r = ctl(&server, "status");
    assert_eq!(r["pointerPlane"]["mode"], "plane");
    let r = ctl(&server, "pointer gpu");
    assert_eq!((r["ok"].as_bool(), r["pointer"]["mode"].as_str(), r["pointer"]["carried"].as_bool()), (Some(true), Some("gpu"), Some(false)));
    // an output with no plane refuses `plane` and the pointer stays where it was
    knobs.carries.store(false, Ordering::SeqCst);
    let r = ctl(&server, "pointer plane");
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["pointer"]["mode"], "gpu");
    assert_eq!(ctl(&server, "pointer sideways")["ok"], false);
}

#[test]
fn an_output_without_a_plane_leaves_the_pointer_in_the_frame_whatever_the_switch_says() {
    let mut server = Server::start();
    let _app = red_app(&mut server);
    server.eval(|s| s.config.pointer = Mode::Plane);
    let image = server.eval(|s| {
        pointer_motion(s, 800, 500);
        s.tick();
        s.tick();
        s.output.capture().unwrap()
    });
    assert!(server.eval(|s| !s.pointer_carried && s.scene.cursor.is_some()));
    assert_eq!(image.pixel(800, 500), WHITE);
}
