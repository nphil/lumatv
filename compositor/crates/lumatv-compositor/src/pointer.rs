//! The Magic Remote's pointer on the cursor plane (`--pointer plane`, `ctl pointer plane`).
//!
//! By default the pointer is a sprite the compositor draws into the frame (`render::cursor`): every move damages two small
//! rectangles, composes them, flips a whole page and, while the sprite shows, refuses direct scan-out of an app's buffer
//! (`NotSole::Cursor`). On the plane the sprite is an image the display keeps: a move is one call into the driver, no frame
//! is drawn, and the picture under it is untouched.
//!
//! What the driver does with it (`docs/research/hw-planes-result.md`, from `core/drm/drm.c` and `core/fbdev/fbdev_drm.c` of LG's
//! tree; the installed module is older, the TV round checks each point):
//!
//! - The plane is `crsr_fb` (OSD3, DRM plane 36, AR24 only). It takes `unsigned` positions in the UI's 1920x1080 space and scales both
//!   the position and the image by the panel's ratio (x2): the 28 pixel disc is 56 panel pixels, as the sprite in the frame is.
//! - The image is scaled by LG's 2D blitter (bilinear) when its buffer changes, never when only the position does. A buffer's width
//!   must keep its pitch a multiple of 64 bytes (`lg1k_drm_gem_fb_create` aligns pitches to 64 and the legacy cursor ioctl names the
//!   pitch as `width * 4`): 32 pixels.
//! - A hotspot is subtracted by the display driver, which also clips a sprite at the top and left edges (the hardware takes no negative
//!   position). That is the legacy cursor ioctls' job (`SetCursor2` with the hotspot, `MoveCursor`), which also take the kernel's
//!   unsynchronised path: no wait for a vblank and none for the flip in flight. The atomic route has no hotspot, so it clamps at the
//!   edge instead (a sprite within 16 pixels of the top or left shows shifted).
//!
//! The calls go to a thread of their own: the kernel's cursor path still waits for the programming of a commit in flight (its fence), a
//! few milliseconds at worst, which the frame loop must not spend. The thread keeps only the newest position.

use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use parking_lot::{Condvar, Mutex};
use serde_json::{Value, json};

/// The image's edge in pixels: a pitch of 128 bytes, which is a multiple of the 64 the driver aligns to.
pub const SPRITE: i32 = 32;
/// The pointer's hotspot in the image: the disc's centre, where the frame's sprite is centred too.
pub const HOTSPOT: (i32, i32) = (SPRITE / 2, SPRITE / 2);
/// Bytes in a row of the image.
pub const PITCH: usize = SPRITE as usize * 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// The sprite is drawn into the frame.
    #[default]
    Gpu,
    /// The sprite is the cursor plane's image.
    Plane,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "gpu" | "off" => Some(Mode::Gpu),
            "plane" | "on" => Some(Mode::Plane),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Gpu => "gpu",
            Mode::Plane => "plane",
        }
    }
}

/// How the plane reads the image's colour against its alpha. The frame is premultiplied (the compositor blends `ONE, ONE_MINUS_SRC_ALPHA`)
/// and LG's own compositor shows premultiplied output on the same OSD mixer, so the default is premultiplied; whether the cursor layer
/// agrees is a TV check (the rim of the disc is a bright halo if it does not, a dark fringe the other way).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Alpha {
    #[default]
    Premultiplied,
    Straight,
}

impl Alpha {
    pub fn parse(s: &str) -> Option<Alpha> {
        match s {
            "premultiplied" | "premul" => Some(Alpha::Premultiplied),
            "straight" => Some(Alpha::Straight),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Alpha::Premultiplied => "premultiplied",
            Alpha::Straight => "straight",
        }
    }
}

/// The plane's image: the frame's disc (`render::cursor::sprite`, 28 pixels) centred in [`SPRITE`] pixels, as DRM's ARGB8888 lays it out
/// in memory (blue, green, red, alpha), `SPRITE` rows of [`PITCH`] bytes.
pub fn plane_image(alpha: Alpha) -> Vec<u8> {
    let disc = crate::render::cursor::sprite();
    let size = crate::render::cursor::SIZE as usize;
    let margin = (SPRITE as usize - size) / 2;
    let mut out = vec![0u8; PITCH * SPRITE as usize];
    for y in 0..size {
        for x in 0..size {
            let s = (y * size + x) * 4;
            let (r, g, b, a) = (disc[s], disc[s + 1], disc[s + 2], disc[s + 3]);
            let lift = |c: u8| match alpha {
                Alpha::Premultiplied => c,
                // colour = premultiplied / alpha, rounded
                Alpha::Straight if a == 0 => 0,
                Alpha::Straight => ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8,
            };
            let d = (y + margin) * PITCH + (x + margin) * 4;
            out[d..d + 4].copy_from_slice(&[lift(b), lift(g), lift(r), a]);
        }
    }
    out
}

/// Where the atomic route puts the plane for a pointer at `at`: the hotspot taken away in user space, and held at the screen's top left
/// because the driver takes no negative position.
pub fn atomic_origin(at: (i32, i32)) -> (i32, i32) {
    ((at.0 - HOTSPOT.0).max(0), (at.1 - HOTSPOT.1).max(0))
}

/// What puts the image on the screen: one implementation over the DRM device, a recording one in the tests.
pub trait Backend: Send + 'static {
    /// The route's name, for the report.
    fn route(&self) -> &'static str;
    /// Gives the plane the image and shows it with its hotspot at `at` (output pixels).
    fn show(&mut self, at: (i32, i32)) -> io::Result<()>;
    /// Moves the shown image.
    fn move_to(&mut self, at: (i32, i32)) -> io::Result<()>;
    /// Takes the image off the screen.
    fn hide(&mut self) -> io::Result<()>;
}

#[derive(Default)]
struct Want {
    at: Option<(i32, i32)>,
    version: u64,
    quit: bool,
}

#[derive(Default)]
struct Counters {
    asked: AtomicU64,
    shows: AtomicU64,
    moves: AtomicU64,
    hides: AtomicU64,
    errors: AtomicU64,
    broken: AtomicBool,
    last_error: Mutex<String>,
    route: Mutex<&'static str>,
}

struct Shared {
    want: Mutex<Want>,
    cv: Condvar,
    n: Counters,
}

/// The pointer's thread and what it was last asked. Dropping it hides the pointer and ends the thread.
pub struct Plane {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl Plane {
    pub fn start(mut backend: impl Backend) -> Plane {
        let shared = Arc::new(Shared { want: Mutex::new(Want::default()), cv: Condvar::new(), n: Counters::default() });
        *shared.n.route.lock() = backend.route();
        let s = shared.clone();
        let worker = std::thread::Builder::new()
            .name("lumatv-pointer".into())
            .spawn(move || {
                let (mut seen, mut shown): (u64, Option<(i32, i32)>) = (0, None);
                loop {
                    let want = {
                        let mut w = s.want.lock();
                        while w.version == seen && !w.quit {
                            s.cv.wait(&mut w);
                        }
                        if w.quit {
                            break;
                        }
                        seen = w.version;
                        w.at
                    };
                    let done = match (shown, want) {
                        (None, Some(at)) => backend.show(at).inspect(|_| {
                            s.n.shows.fetch_add(1, Ordering::Relaxed);
                        }),
                        (Some(old), Some(at)) if old != at => backend.move_to(at).inspect(|_| {
                            s.n.moves.fetch_add(1, Ordering::Relaxed);
                        }),
                        (Some(_), None) => backend.hide().inspect(|_| {
                            s.n.hides.fetch_add(1, Ordering::Relaxed);
                        }),
                        _ => Ok(()),
                    };
                    match done {
                        Ok(()) => shown = want,
                        Err(e) => {
                            s.n.errors.fetch_add(1, Ordering::Relaxed);
                            *s.n.last_error.lock() = format!("{} {}: {e}", backend.route(), if want.is_some() { "show" } else { "hide" });
                            s.n.broken.store(true, Ordering::SeqCst);
                            break;
                        }
                    }
                }
                // the image does not stay on the screen without its owner
                if shown.is_some() {
                    let _ = backend.hide();
                }
            })
            .expect("the pointer's thread");
        Plane { shared, worker: Some(worker) }
    }

    /// Shows the pointer at `at` (output pixels, the hotspot), or hides it. The newest request wins. Returns whether the plane carries it: false
    /// once a call into the driver has failed, and the caller draws the sprite into the frame again.
    pub fn set(&self, at: Option<(i32, i32)>) -> bool {
        if self.shared.n.broken.load(Ordering::SeqCst) {
            return false;
        }
        let mut w = self.shared.want.lock();
        if w.at != at {
            w.at = at;
            w.version += 1;
            self.shared.n.asked.fetch_add(1, Ordering::Relaxed);
            self.shared.cv.notify_one();
        }
        true
    }

    pub fn broken(&self) -> bool {
        self.shared.n.broken.load(Ordering::SeqCst)
    }

    pub fn last_error(&self) -> String {
        self.shared.n.last_error.lock().clone()
    }

    /// The counters: requests that changed what was wanted, and the calls made for them (the difference is what a newer position
    /// overtook before the thread got to it).
    pub fn report(&self) -> Value {
        let n = &self.shared.n;
        let (shows, moves, hides) = (n.shows.load(Ordering::Relaxed), n.moves.load(Ordering::Relaxed), n.hides.load(Ordering::Relaxed));
        let asked = n.asked.load(Ordering::Relaxed);
        json!({
            "route": *n.route.lock(),
            "asked": asked,
            "shows": shows,
            "moves": moves,
            "hides": hides,
            "coalesced": asked.saturating_sub(shows + moves + hides),
            "errors": n.errors.load(Ordering::Relaxed),
            "broken": n.broken.load(Ordering::Relaxed),
            "lastError": self.last_error(),
        })
    }
}

impl Drop for Plane {
    fn drop(&mut self) {
        self.shared.want.lock().quit = true;
        self.shared.cv.notify_one();
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

/// The report of a pointer that is on the GPU, or whose plane is not up (`reason`).
pub fn idle_report(mode: Mode, alpha: Alpha, reason: &str) -> Value {
    json!({
        "mode": mode.name(),
        "carried": false,
        "sprite": format!("{SPRITE}x{SPRITE}, hotspot {},{}, {}", HOTSPOT.0, HOTSPOT.1, alpha.name()),
        "reason": reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[derive(Clone, Debug, PartialEq)]
    enum Call {
        Show((i32, i32)),
        Move((i32, i32)),
        Hide,
    }

    #[derive(Clone, Default)]
    struct Recorder {
        calls: Arc<Mutex<Vec<Call>>>,
        fail_show: bool,
        hold: Option<Arc<Mutex<()>>>,
    }

    impl Backend for Recorder {
        fn route(&self) -> &'static str {
            "test"
        }

        fn show(&mut self, at: (i32, i32)) -> io::Result<()> {
            if self.fail_show {
                return Err(io::Error::from_raw_os_error(libc::EINVAL));
            }
            self.calls.lock().push(Call::Show(at));
            Ok(())
        }

        fn move_to(&mut self, at: (i32, i32)) -> io::Result<()> {
            if let Some(h) = &self.hold {
                drop(h.lock());
            }
            self.calls.lock().push(Call::Move(at));
            Ok(())
        }

        fn hide(&mut self) -> io::Result<()> {
            self.calls.lock().push(Call::Hide);
            Ok(())
        }
    }

    fn wait(f: impl Fn() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(3);
        while Instant::now() < end {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        false
    }

    #[test]
    fn the_image_is_a_disc_centred_in_a_buffer_whose_pitch_the_driver_keeps() {
        // the driver aligns a framebuffer's pitch to 64 bytes and the legacy ioctl says it is `width * 4`: the two agree
        assert_eq!(PITCH % 64, 0);
        const { assert!(SPRITE <= 128, "the plane's cursor size") };
        let img = plane_image(Alpha::Premultiplied);
        assert_eq!(img.len(), PITCH * SPRITE as usize);
        let at = |x: usize, y: usize| {
            let o = y * PITCH + x * 4;
            [img[o], img[o + 1], img[o + 2], img[o + 3]]
        };
        assert_eq!(at(16, 16), [255, 255, 255, 255], "white in the middle");
        assert_eq!(at(0, 0)[3], 0, "the margin is clear");
        assert_eq!(at(31, 31)[3], 0);
        let rim = at(16, 16 - 12 + 1);
        assert_eq!(rim[3], 255);
        assert!(rim[0] < 0x40, "dark rim: {rim:?}");
        // the disc is where the frame's sprite is, about the hotspot
        let lit = |x: usize, y: usize| at(x, y)[3] > 0;
        assert!(lit(16 - 11, 16) && lit(16 + 11, 16) && !lit(16 - 14, 16) && !lit(16, 16 + 14));
    }

    #[test]
    fn straight_alpha_undoes_the_premultiply_and_clear_pixels_stay_clear() {
        let pre = plane_image(Alpha::Premultiplied);
        let straight = plane_image(Alpha::Straight);
        assert_eq!(pre.len(), straight.len());
        let mut edge = 0;
        for (p, s) in pre.chunks_exact(4).zip(straight.chunks_exact(4)) {
            assert_eq!(p[3], s[3], "alpha is alpha");
            if p[3] == 0 {
                assert_eq!(s, [0, 0, 0, 0]);
            } else if p[3] < 255 {
                edge += 1;
                for k in 0..3 {
                    assert!(s[k] >= p[k], "unpremultiplying never darkens: {p:?} {s:?}");
                    // premultiplying the straight colour again gives the colour back to within a step
                    let again = (u32::from(s[k]) * u32::from(s[3]) + 127) / 255;
                    assert!(again.abs_diff(u32::from(p[k])) <= 1, "{p:?} {s:?}");
                }
            }
        }
        assert!(edge > 20, "the disc's antialiased edge has partial pixels: {edge}");
    }

    #[test]
    fn the_atomic_route_holds_the_origin_at_the_top_left() {
        assert_eq!(atomic_origin((960, 540)), (944, 524));
        assert_eq!(atomic_origin((3, 1079)), (0, 1063));
        assert_eq!(atomic_origin((0, 0)), (0, 0));
    }

    #[test]
    fn a_show_is_one_call_then_moves_then_a_hide_and_the_next_show_is_one_call_again() {
        let rec = Recorder::default();
        let calls = rec.calls.clone();
        let p = Plane::start(rec);
        assert!(p.set(Some((100, 200))));
        assert!(wait(|| calls.lock().len() == 1));
        assert!(p.set(Some((110, 205))));
        assert!(wait(|| calls.lock().len() == 2));
        assert!(p.set(None));
        assert!(wait(|| calls.lock().len() == 3));
        assert!(p.set(Some((5, 6))));
        assert!(wait(|| calls.lock().len() == 4));
        assert_eq!(*calls.lock(), vec![Call::Show((100, 200)), Call::Move((110, 205)), Call::Hide, Call::Show((5, 6))]);
        let r = p.report();
        assert_eq!((r["shows"].as_u64(), r["moves"].as_u64(), r["hides"].as_u64(), r["errors"].as_u64()), (Some(2), Some(1), Some(1), Some(0)));
    }

    #[test]
    fn the_same_position_asked_twice_is_one_request_and_a_hide_of_a_hidden_pointer_is_none() {
        let rec = Recorder::default();
        let calls = rec.calls.clone();
        let p = Plane::start(rec);
        assert!(p.set(None));
        assert!(p.set(Some((1, 2))));
        assert!(p.set(Some((1, 2))));
        assert!(wait(|| calls.lock().len() == 1));
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(*calls.lock(), vec![Call::Show((1, 2))]);
        assert_eq!(p.report()["asked"], 1);
    }

    #[test]
    fn a_slow_driver_gets_the_newest_position_and_none_of_those_it_missed() {
        let rec = Recorder::default();
        let gate = Arc::new(Mutex::new(()));
        let rec = Recorder { hold: Some(gate.clone()), ..rec };
        let calls = rec.calls.clone();
        let p = Plane::start(rec);
        assert!(p.set(Some((0, 0))));
        assert!(wait(|| calls.lock().len() == 1));
        let held = gate.lock();
        // the thread blocks in the first move; a hundred more positions arrive meanwhile
        assert!(p.set(Some((1, 1))));
        std::thread::sleep(Duration::from_millis(30));
        for i in 2..=100 {
            assert!(p.set(Some((i, i))));
        }
        drop(held);
        assert!(wait(|| calls.lock().last() == Some(&Call::Move((100, 100)))));
        let n = calls.lock().len();
        assert!(n <= 4, "the positions in between were never sent: {n} calls");
        let r = p.report();
        assert!(r["coalesced"].as_u64().unwrap() >= 90, "{r}");
    }

    #[test]
    fn a_driver_that_refuses_breaks_the_plane_and_the_caller_is_told_to_draw() {
        let rec = Recorder { fail_show: true, ..Recorder::default() };
        let p = Plane::start(rec);
        assert!(p.set(Some((10, 10))), "not known to fail until it has");
        assert!(wait(|| p.broken()));
        assert!(!p.set(Some((20, 20))), "the next request says the sprite is the frame's again");
        assert!(p.last_error().contains("show"), "{}", p.last_error());
        assert_eq!(p.report()["errors"], 1);
    }

    #[test]
    fn dropping_the_plane_takes_the_pointer_off_the_screen() {
        let rec = Recorder::default();
        let calls = rec.calls.clone();
        let p = Plane::start(rec);
        assert!(p.set(Some((7, 8))));
        assert!(wait(|| calls.lock().len() == 1));
        drop(p);
        assert_eq!(*calls.lock(), vec![Call::Show((7, 8)), Call::Hide]);
    }

    #[test]
    fn the_modes_and_alphas_parse() {
        assert_eq!(Mode::parse("plane"), Some(Mode::Plane));
        assert_eq!(Mode::parse("gpu"), Some(Mode::Gpu));
        assert_eq!(Mode::parse("hw"), None);
        assert_eq!(Mode::default(), Mode::Gpu, "off unless asked");
        assert_eq!(Alpha::parse("straight"), Some(Alpha::Straight));
        assert_eq!(Alpha::parse("premul"), Some(Alpha::Premultiplied));
        assert_eq!(Alpha::parse("x"), None);
    }
}
