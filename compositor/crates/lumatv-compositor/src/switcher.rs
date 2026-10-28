//! The app switcher: a second press of Home shows the running apps as live pictures of their own windows, one card each, over a dim
//! Home. The shell (`panels/Switcher.qml`, in its system layer) draws everything about a card but its picture: where the cards go,
//! their names, the focus, Quit. This module is the compositor's half:
//!
//! - **the trigger** ([`HomeTaps`]): a Home tap acts when the key is let go, as it always did, and the first one is never held back
//!   to see whether a second comes. A second tap within [`DOUBLE_HOME`] of a tap that went Home from an app opens the switcher
//!   with that app shrinking into its card; a tap while Home is in front and at rest (the shell decides, it knows what Home has
//!   open) opens it too; a tap while it is open closes it. The 650 ms hold of Home (`input.rs`) is not touched by any of this.
//! - **the apps**: the cards the compositor holds, newest first, each with whether a picture of it can be drawn. An app whose picture
//!   is on the video plane (a hole in its window, or an input) is never sampled: the plane cannot be read here and the switcher shows
//!   the app's icon on a tile instead.
//! - **the pictures**: the shell tells where each card is (`layout`), the compositor moves the picture there on its own clock
//!   ([`Track`]: a 200 ms move lasts 200 ms at 60 or 120 Hz) and the scene draws it with the scrim, below the shell's layer
//!   (`scene/backdrop.rs`). Only a card that is open costs anything: closed, there is no state to plan, nothing to draw and no
//!   timer.
//! - **leaving**: OK on a card grows its picture to the whole screen while SAM launches the app; Quit asks SAM (`closeByAppId`, as
//!   LG's own recents does) and shows the card as closing until its window is gone.
//!
//! Everything that is a decision is a function of the clock an argument carries, so the host tests move time by hand.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::scene::{BackdropSpec, HOME_APP, PreviewSpec};
use crate::surface::SurfaceId;
use crate::window::{WindowId, WindowType};
use crate::{OUTPUT_HEIGHT, OUTPUT_WIDTH, Rect, State};

/// A second Home tap within this of the first (release to release) is a double press.
pub const DOUBLE_HOME: Duration = Duration::from_millis(400);
/// The pictures arrive (or an app shrinks into its card, or the row moves): Lucent `motion.layer` held to the 200 ms the switcher is allowed.
pub const OPEN_MS: u32 = 200;
/// The pictures leave, toward where they came from (`motion.exit`).
pub const EXIT_MS: u32 = 160;
/// The dim comes in and goes out.
pub const SCRIM_MS: u32 = 160;
/// A card that fades (a card out of the window, one being quit, one that is not the one chosen).
pub const FADE_MS: u32 = 120;
/// How far the shell has to answer an opening with the cards' places: past this the switcher is given up (the shell is gone or stuck).
const LAYOUT_WAIT: Duration = Duration::from_millis(600);
/// The chosen picture is held at full size this long at most for the app to be in front.
const ACTIVATE_HOLD: Duration = Duration::from_millis(1000);
/// An app asked to quit has this long to go before the card says it did not.
const QUIT_WAIT: Duration = Duration::from_secs(4);
/// The size a picture arrives from, and leaves toward, about its own centre.
const ARRIVE_SCALE: f32 = 0.94;
const LEAVE_SCALE: f32 = 0.96;
/// A picture fainter than one level of an 8-bit channel is not there.
const SEEN: f32 = 1.0 / 255.0;
/// How opaque the picture of an app being quit is.
const QUIT_ALPHA: f32 = 0.35;
/// The scrim until the shell says otherwise: the dark palette's `scrimHide` (`#F8120E1A`), premultiplied.
const SCRIM_DEFAULT: [f32; 4] = [0.0706, 0.0549, 0.1020, 1.0];

fn output_rect() -> Rect {
    Rect::new(0, 0, OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32)
}

// ---- the trigger ----

/// What a Home tap means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HomeTap {
    /// What Home always did: ask the app manager for Home, and the shell closes what it has open.
    Plain,
    /// [`HomeTap::Plain`], and the shell is told Home was pressed at rest with apps open: it may open the switcher.
    Offer,
    /// A second press: the switcher opens, with `from` shrinking into its card.
    Open { from: String },
    /// The switcher is open: it closes, and nothing else happens.
    Close,
}

/// What the routing knows when a Home tap is let go.
pub struct TapContext<'a> {
    /// The shell can show the switcher (it said so, which it does only while the owner's switch is on).
    pub enabled: bool,
    /// The switcher is open.
    pub open: bool,
    /// The foreground card's app id, empty when none.
    pub foreground: &'a str,
    /// LumaTV's Home is what is drawn.
    pub home_drawn: bool,
    /// At least one app is running that the switcher would list.
    pub apps: bool,
}

/// The last Home tap that went Home: when, and from which app (none when Home was already in front).
#[derive(Default)]
pub struct HomeTaps {
    last: Option<(Instant, Option<String>)>,
}

impl HomeTaps {
    /// One Home tap, at the moment the key is let go. Answers at once: nothing waits for a second press.
    pub fn tap(&mut self, now: Instant, c: &TapContext) -> HomeTap {
        let last = self.last.take();
        if c.open {
            return HomeTap::Close;
        }
        if !c.enabled {
            return HomeTap::Plain;
        }
        if let Some((at, Some(from))) = last
            && c.apps
            && now.saturating_duration_since(at) <= DOUBLE_HOME
        {
            return HomeTap::Open { from };
        }
        let from = (!c.foreground.is_empty() && c.foreground != HOME_APP).then(|| c.foreground.to_string());
        self.last = Some((now, from));
        if c.home_drawn && c.apps { HomeTap::Offer } else { HomeTap::Plain }
    }
}

// ---- motion ----

/// Where a picture is and how it looks: its rectangle (fractions of a pixel), opacity and corner radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub alpha: f32,
    pub radius: f32,
}

impl Pose {
    pub fn new(r: Rect, alpha: f32, radius: f32) -> Pose {
        Pose { x: r.x as f32, y: r.y as f32, w: r.w as f32, h: r.h as f32, alpha, radius }
    }

    pub fn rect(&self) -> Rect {
        Rect::new(self.x.round() as i32, self.y.round() as i32, self.w.round() as i32, self.h.round() as i32)
    }

    fn lerp(a: &Pose, b: &Pose, e: f32) -> Pose {
        let l = |p: f32, q: f32| p + (q - p) * e;
        Pose { x: l(a.x, b.x), y: l(a.y, b.y), w: l(a.w, b.w), h: l(a.h, b.h), alpha: l(a.alpha, b.alpha), radius: l(a.radius, b.radius) }
    }

    /// The same picture at `s` of its size about its centre.
    fn scaled(&self, s: f32) -> Pose {
        Pose { x: self.x + self.w * (1.0 - s) / 2.0, y: self.y + self.h * (1.0 - s) / 2.0, w: self.w * s, h: self.h * s, ..*self }
    }
}

/// The easing of a move: `Settle` is Lucent's `cubic-bezier(.33, 1, .68, 1)` (everything that arrives), `Exit` its `cubic-bezier(.4, 0, 1, 1)`
/// (everything that leaves).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    Settle,
    Exit,
}

impl Curve {
    pub fn at(self, p: f32) -> f32 {
        match self {
            Curve::Settle => bezier(0.33, 1.0, 0.68, 1.0, p),
            Curve::Exit => bezier(0.4, 0.0, 1.0, 1.0, p),
        }
    }
}

/// `cubic-bezier(x1, y1, x2, y2)` at `x` in 0 to 1 (CSS's timing function: solve the curve's x for its parameter, answer its y).
fn bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    let at = |a: f32, b: f32, s: f32| 3.0 * (1.0 - s) * (1.0 - s) * s * a + 3.0 * (1.0 - s) * s * s * b + s * s * s;
    let x = x.clamp(0.0, 1.0);
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    let mut s = x;
    for _ in 0..24 {
        let v = at(x1, x2, s);
        if (v - x).abs() < 1e-5 {
            break;
        }
        if v < x {
            lo = s;
        } else {
            hi = s;
        }
        s = 0.5 * (lo + hi);
    }
    at(y1, y2, s)
}

/// A move of a pose on the monotonic clock: `p = (now - t0) / ms`, `output = from + ease(p) * (to - from)`. Retargeted from where it is.
#[derive(Clone, Copy, Debug)]
pub struct Track {
    from: Pose,
    to: Pose,
    t0: Instant,
    ms: u32,
    curve: Curve,
}

impl Track {
    pub fn still(p: Pose, now: Instant) -> Track {
        Track { from: p, to: p, t0: now, ms: 0, curve: Curve::Settle }
    }

    pub fn new(from: Pose, to: Pose, now: Instant, ms: u32, curve: Curve) -> Track {
        Track { from, to, t0: now, ms, curve }
    }

    pub fn at(&self, now: Instant) -> Pose {
        if self.ms == 0 {
            return self.to;
        }
        let p = (now.saturating_duration_since(self.t0).as_secs_f32() * 1000.0 / self.ms as f32).clamp(0.0, 1.0);
        Pose::lerp(&self.from, &self.to, self.curve.at(p))
    }

    pub fn done(&self, now: Instant) -> bool {
        self.ms == 0 || now.saturating_duration_since(self.t0) >= Duration::from_millis(self.ms as u64)
    }

    /// A new target from where the picture is now; `still_rect` holds the rectangle (reduced motion: opacity moves, nothing travels).
    pub fn retarget(&mut self, now: Instant, to: Pose, ms: u32, curve: Curve, still_rect: bool) {
        let mut from = self.at(now);
        if still_rect {
            (from.x, from.y, from.w, from.h, from.radius) = (to.x, to.y, to.w, to.h, to.radius);
        }
        *self = Track { from, to, t0: now, ms, curve };
    }
}

// ---- the apps ----

/// Whether a picture of an app can be drawn from its window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picture {
    /// Its window's buffer, scaled.
    Live,
    /// Its picture is on the video plane (a hole in its window, or an input): never sampled, the shell shows its icon.
    Video,
    /// Its window has nothing to draw.
    None,
}

impl Picture {
    fn word(self) -> &'static str {
        match self {
            Picture::Live => "live",
            Picture::Video => "video",
            Picture::None => "none",
        }
    }
}

/// A running app as the switcher lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct App {
    pub id: String,
    pub title: String,
    pub window: WindowId,
    pub surface: SurfaceId,
    pub size: (i32, i32),
    pub picture: Picture,
}

/// An app whose window is only a hole over the video plane, by its id (an input, Live TV): no window of it has a picture worth drawing.
fn input_app(id: &str) -> bool {
    ["com.webos.app.hdmi", "com.webos.app.livetv", "com.webos.app.dp", "com.webos.app.usbc", "com.webos.app.externalinput"].iter().any(|p| id.starts_with(p))
}

/// LumaTV's own apps are not the owner's to switch between (Home, the helper, the card app), as LG's recents leave Home out.
fn own_app(id: &str) -> bool {
    id == HOME_APP || id.starts_with("org.nphil.luma")
}

// ---- the state ----

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Closed,
    /// Opened, and the shell has not said where the cards go.
    Waiting,
    Open,
    Closing,
    /// One card was chosen: its picture grows to the whole screen while the app comes to the front.
    Activating,
}

struct Card {
    app: String,
    surface: SurfaceId,
    picture: Picture,
    track: Track,
    /// Where the picture starts when the shell places it: the app's own screen (a second press from it), or none (it arrives).
    start: Option<Pose>,
    placed: bool,
    /// The app is gone: the picture fades and the card is dropped.
    gone: bool,
}

/// The switcher's state, one per compositor.
#[derive(Default)]
pub struct Switcher {
    pub taps: HomeTaps,
    phase: Phase,
    /// How many times it has opened: a new value tells the shell a new opening.
    seq: u64,
    from: Option<String>,
    cards: Vec<Card>,
    scrim_rgb: [f32; 3],
    scrim: Option<Track>,
    /// The apps asked to quit, and when.
    quitting: Vec<(String, Instant)>,
    /// The apps that did not quit in time.
    stuck: Vec<String>,
    activating: Option<String>,
    until: Option<Instant>,
    reduced: bool,
}

impl Switcher {
    /// Open, or opening: Home closes it.
    pub fn accepting(&self) -> bool {
        matches!(self.phase, Phase::Waiting | Phase::Open)
    }

    /// Anything of it is on the screen or on its way there or away.
    pub fn active(&self) -> bool {
        self.phase != Phase::Closed
    }
}

impl State {
    /// The running apps, newest first: the cards the compositor holds, minus Home, LumaTV's own apps and the members of surface groups.
    pub fn switcher_apps(&self) -> Vec<App> {
        let mut found: Vec<(u64, App)> = Vec::new();
        for w in self.scene.windows() {
            let id = w.app_id();
            if w.kind != WindowType::Card || w.in_group || w.map_serial == 0 || w.front == 0 || id.is_empty() || own_app(id) {
                continue;
            }
            let Some(s) = self.surfaces.get(w.surface).filter(|s| s.is_mapped()) else { continue };
            let size = s.size();
            let video = input_app(id)
                || self.scene.holes.iter().any(|h| h.surface == w.surface)
                || self.foreign.exported.values().any(|e| e.surface == w.surface && !e.contexts.is_empty());
            let picture = if video {
                Picture::Video
            } else if size.0 <= 0 || size.1 <= 0 {
                Picture::None
            } else {
                Picture::Live
            };
            let title = if w.title.is_empty() { id.to_string() } else { w.title.clone() };
            let app = App { id: id.to_string(), title, window: w.id, surface: w.surface, size, picture };
            match found.iter_mut().find(|(_, a)| a.id == app.id) {
                Some(old) if old.0 >= w.front => {}
                Some(old) => *old = (w.front, app),
                None => found.push((w.front, app)),
            }
        }
        found.sort_by_key(|a| std::cmp::Reverse(a.0));
        found.into_iter().map(|(_, a)| a).collect()
    }

    /// Whether any of the switcher is on the screen or on its way (the transitions leave the windows alone meanwhile).
    pub fn switcher_active(&self) -> bool {
        self.switcher.active()
    }

    /// A Home tap, let go before the escape's 650 ms: Home as ever, or the switcher (see the module).
    pub(crate) fn home_tap(&mut self, now: Instant, from_network: bool, shell_focus: bool) {
        let foreground = self.foreground_app();
        let ctx = TapContext {
            enabled: self.shell_can("switcher"),
            open: self.switcher.accepting(),
            foreground: &foreground,
            home_drawn: self.shell_home_drawn(),
            apps: !self.switcher_apps().is_empty(),
        };
        match self.switcher.taps.tap(now, &ctx) {
            HomeTap::Close => self.switcher_close("Home", now),
            HomeTap::Open { from } => {
                self.journal(format!("input: Home twice within {} ms: the app switcher, with {from} shrinking into its card", DOUBLE_HOME.as_millis()));
                self.switcher_open(Some(from), now);
            }
            tap @ (HomeTap::Plain | HomeTap::Offer) => {
                self.home_pressed(from_network);
                if shell_focus {
                    self.shell_home_key_offer(tap == HomeTap::Offer);
                }
            }
        }
    }

    /// Opens the switcher over what is on the screen. `from` is the app whose picture is on the screen now and shrinks into its card.
    /// Returns whether it opened (it does not while it is open, or when no app runs).
    pub fn switcher_open(&mut self, from: Option<String>, now: Instant) -> bool {
        if matches!(self.switcher.phase, Phase::Waiting | Phase::Open | Phase::Activating) {
            return false;
        }
        let apps = self.switcher_apps();
        if apps.is_empty() {
            self.journal("switcher: no app is open".into());
            return false;
        }
        let foreground = self.foreground_app();
        let from = from.filter(|f| apps.iter().any(|a| a.id == *f));
        let mut cards = Vec::with_capacity(apps.len());
        for a in &apps {
            let mut start = None;
            if a.picture == Picture::Live && from.as_deref() == Some(a.id.as_str()) {
                // Where its picture is: on its way home (the transition hands it over), else it is the whole screen if it still is.
                start = self.scene.take_visual(a.window).filter(|v| v.alpha > 0.05).map(|v| Pose::new(v.rect, v.alpha, v.radius));
                if start.is_none() && foreground == a.id {
                    start = Some(Pose::new(output_rect(), 1.0, 0.0));
                }
            }
            let hidden = Pose::new(Rect::default(), 0.0, 0.0);
            cards.push(Card {
                app: a.id.clone(),
                surface: a.surface,
                picture: a.picture,
                track: Track::still(start.unwrap_or(hidden), now),
                start,
                placed: false,
                gone: false,
            });
        }
        let sw = &mut self.switcher;
        sw.phase = Phase::Waiting;
        sw.seq += 1;
        sw.from = from.clone();
        sw.cards = cards;
        sw.scrim_rgb = [SCRIM_DEFAULT[0], SCRIM_DEFAULT[1], SCRIM_DEFAULT[2]];
        sw.scrim = Some(Track::still(Pose { x: 0.0, y: 0.0, w: 0.0, h: 0.0, alpha: 0.0, radius: 0.0 }, now));
        sw.quitting.clear();
        sw.stuck.clear();
        sw.activating = None;
        sw.until = Some(now + LAYOUT_WAIT);
        sw.reduced = false;
        let seq = sw.seq;
        self.journal(format!(
            "switcher: opened #{seq} over {} app(s){}",
            apps.len(),
            from.as_deref().map(|f| format!(", {f} shrinking into its card")).unwrap_or_default()
        ));
        self.shell_mark("Q:switcher-show");
        // The pictures' first-use costs are paid while the shell is placing the cards, not in the frame that first shows them.
        let buffers: Vec<_> =
            apps.iter().filter(|a| a.picture == Picture::Live).filter_map(|a| self.surfaces.get(a.surface).and_then(|s| s.current.buffer.clone())).collect();
        self.output.prepare_previews(&buffers);
        self.shell_sync_topics();
        true
    }

    /// Closes it: the pictures and the dim leave over [`EXIT_MS`].
    pub fn switcher_close(&mut self, why: &str, now: Instant) {
        if !self.switcher.accepting() {
            return;
        }
        let reduced = self.switcher.reduced;
        for c in &mut self.switcher.cards {
            let cur = c.track.at(now);
            let to = Pose { alpha: 0.0, ..if reduced { cur } else { cur.scaled(LEAVE_SCALE) } };
            c.track.retarget(now, to, if reduced { FADE_MS } else { EXIT_MS }, Curve::Exit, reduced);
        }
        if let Some(t) = &mut self.switcher.scrim {
            t.retarget(now, Pose { alpha: 0.0, ..t.at(now) }, SCRIM_MS, Curve::Exit, true);
        }
        self.switcher.phase = Phase::Closing;
        self.switcher.until = Some(now + Duration::from_millis(EXIT_MS as u64));
        self.journal(format!("switcher: closed ({why})"));
        self.shell_sync_topics();
    }

    /// What the shell asks over `POST /v1/switcher`: `open`, `layout`, `close`, `activate`, `quit`, `quitall`. Returns whether the picture
    /// changed in a way a frame should show now.
    pub fn switcher_op(&mut self, body: &Value, now: Instant) -> bool {
        let op = body.get("op").and_then(Value::as_str).unwrap_or("");
        let id = body.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        match op {
            "open" => return self.switcher_open(None, now),
            "layout" => return self.switcher_layout(body, now),
            "close" => self.switcher_close("the shell asked", now),
            "activate" => self.switcher_activate(&id, now),
            "quit" => self.switcher_quit(&id, now),
            "quitall" => {
                let ids: Vec<String> = self.switcher_apps().into_iter().map(|a| a.id).collect();
                self.journal(format!("switcher: quit all ({} app(s))", ids.len()));
                for id in ids {
                    self.switcher_quit(&id, now);
                }
            }
            other => self.journal(format!("switcher: the shell asked for {other:?}, which is not one of its requests")),
        }
        self.switcher.active()
    }

    /// The shell's places for the cards: `{"cards": [{"id", "x", "y", "w", "h", "r"}], "scrim": {"r", "g", "b", "a"}, "ms", "reduced"}`. A
    /// card it names moves there (the first time, it arrives or shrinks in); one it leaves out fades. Returns whether anything moves.
    fn switcher_layout(&mut self, body: &Value, now: Instant) -> bool {
        if !self.switcher.accepting() {
            return false;
        }
        let first = self.switcher.phase == Phase::Waiting;
        let ms = body.get("ms").and_then(Value::as_u64).unwrap_or(OPEN_MS as u64).min(400) as u32;
        let reduced = body.get("reduced").and_then(Value::as_bool).unwrap_or(false);
        self.switcher.reduced = reduced;
        if let Some(s) = body.get("scrim") {
            let f = |k: &str, d: f32| s.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32);
            let a = f("a", 1.0).clamp(0.0, 1.0);
            self.switcher.scrim_rgb = [f("r", SCRIM_DEFAULT[0]), f("g", SCRIM_DEFAULT[1]), f("b", SCRIM_DEFAULT[2])];
            let target = Pose { x: 0.0, y: 0.0, w: 0.0, h: 0.0, alpha: a, radius: 0.0 };
            let ms = if first { SCRIM_MS.min(ms) } else { ms };
            if let Some(t) = &mut self.switcher.scrim {
                t.retarget(now, target, ms, Curve::Settle, true);
            }
        }
        // an app that came after the switcher opened has a card as soon as the shell places it
        let wanted: Vec<String> =
            body.get("cards").and_then(Value::as_array).into_iter().flatten().filter_map(|c| c.get("id").and_then(Value::as_str)).map(str::to_string).collect();
        if wanted.iter().any(|w| !self.switcher.cards.iter().any(|c| c.app == *w)) {
            for a in self.switcher_apps() {
                if wanted.contains(&a.id) && !self.switcher.cards.iter().any(|c| c.app == a.id) {
                    let hidden = Pose::new(Rect::default(), 0.0, 0.0);
                    self.switcher.cards.push(Card {
                        app: a.id,
                        surface: a.surface,
                        picture: a.picture,
                        track: Track::still(hidden, now),
                        start: None,
                        placed: false,
                        gone: false,
                    });
                }
            }
        }
        let quiet = self.switcher.quitting.clone();
        for c in &mut self.switcher.cards {
            let spec = body.get("cards").and_then(Value::as_array).into_iter().flatten().find(|s| s.get("id").and_then(Value::as_str) == Some(c.app.as_str()));
            let alpha = if quiet.iter().any(|(a, _)| *a == c.app) { QUIT_ALPHA } else { 1.0 };
            let Some(spec) = spec else {
                // out of the window: it fades where it is
                if c.placed {
                    let cur = c.track.at(now);
                    c.track.retarget(now, Pose { alpha: 0.0, ..cur }, FADE_MS, Curve::Exit, true);
                }
                continue;
            };
            let g = |k: &str| spec.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let to = Pose { x: g("x"), y: g("y"), w: g("w"), h: g("h"), alpha, radius: g("r") };
            if !c.placed {
                // Reduced motion arrives by opacity alone: an app's picture does not shrink, its card is there and fades in.
                let from = if reduced { Pose { alpha: 0.0, ..to } } else { c.start.unwrap_or(Pose { alpha: 0.0, ..to.scaled(ARRIVE_SCALE) }) };
                c.track = Track::new(from, to, now, ms, Curve::Settle);
                c.placed = true;
            } else {
                c.track.retarget(now, to, ms, Curve::Settle, reduced);
            }
        }
        if first {
            self.switcher.phase = Phase::Open;
            self.switcher.until = None;
            self.journal("switcher: the shell placed the cards".into());
        }
        true
    }

    /// OK on a card: its picture grows to the whole screen while the app manager brings the app to the front.
    fn switcher_activate(&mut self, id: &str, now: Instant) {
        if !self.switcher.accepting() {
            return;
        }
        let Some(i) = self.switcher.cards.iter().position(|c| c.app == id && !c.gone) else {
            self.journal(format!("switcher: {id} was chosen, and has no card"));
            return;
        };
        let reduced = self.switcher.reduced;
        for (j, c) in self.switcher.cards.iter_mut().enumerate() {
            let cur = c.track.at(now);
            if j == i {
                c.track.retarget(now, Pose::new(output_rect(), 1.0, 0.0), OPEN_MS, Curve::Settle, reduced);
            } else {
                c.track.retarget(now, Pose { alpha: 0.0, ..cur }, FADE_MS, Curve::Exit, true);
            }
        }
        if let Some(t) = &mut self.switcher.scrim {
            t.retarget(now, Pose { alpha: 0.0, ..t.at(now) }, OPEN_MS, Curve::Exit, true);
        }
        self.switcher.phase = Phase::Activating;
        self.switcher.activating = Some(id.to_string());
        self.switcher.until = Some(now + ACTIVATE_HOLD);
        self.journal(format!("switcher: {id} chosen, launching it"));
        self.luna_call("luna://com.webos.applicationManager/launch".into(), json!({"id": id}));
        self.shell_sync_topics();
    }

    /// Quit on a card: the window is asked to close and the app manager is told to close the app (`closeByAppId`, the reason LG's own
    /// recents gives, `windowPolicy`). The card says it is closing until its window is gone.
    fn switcher_quit(&mut self, id: &str, now: Instant) {
        if id.is_empty() || own_app(id) || !self.switcher_apps().iter().any(|a| a.id == id) {
            self.journal(format!("switcher: quit {id:?}: no such app is open"));
            return;
        }
        if self.switcher.quitting.iter().any(|(a, _)| a == id) {
            return;
        }
        self.switcher.quitting.push((id.to_string(), now));
        self.switcher.stuck.retain(|a| a != id);
        self.journal(format!("switcher: quit {id}"));
        self.close_app(id);
        self.luna_call("luna://com.webos.applicationManager/closeByAppId".into(), json!({"id": id, "reason": "windowPolicy"}));
        for c in self.switcher.cards.iter_mut().filter(|c| c.app == id && c.placed) {
            let cur = c.track.at(now);
            c.track.retarget(now, Pose { alpha: QUIT_ALPHA, ..cur }, FADE_MS, Curve::Settle, true);
        }
        self.shell_sync_topics();
    }

    /// Every frame while any of it is open: moves what is moving, retires what is done, and hands the scene this frame's pictures.
    pub fn switcher_frame(&mut self, now: Instant) {
        if self.switcher.phase == Phase::Closed {
            return;
        }
        let foreground = self.foreground_app();
        let apps = self.switcher_apps();
        // an app that is gone fades out with its card; one that did not quit in time says so
        let mut changed_topic = false;
        for c in &mut self.switcher.cards {
            if !c.gone && !apps.iter().any(|a| a.id == c.app) {
                c.gone = true;
                let cur = c.track.at(now);
                c.track.retarget(now, Pose { alpha: 0.0, ..cur }, FADE_MS, Curve::Exit, true);
                changed_topic = true;
            }
        }
        let before = self.switcher.quitting.len();
        self.switcher.quitting.retain(|(a, _)| apps.iter().any(|x| x.id == *a));
        changed_topic |= self.switcher.quitting.len() != before;
        let late: Vec<String> =
            self.switcher.quitting.iter().filter(|(_, at)| now.saturating_duration_since(*at) > QUIT_WAIT).map(|(a, _)| a.clone()).collect();
        if !late.is_empty() {
            for a in &late {
                self.journal(format!("switcher: {a} did not quit in {} s", QUIT_WAIT.as_secs()));
                self.switcher.stuck.push(a.clone());
                if let Some(c) = self.switcher.cards.iter_mut().find(|c| c.app == *a && c.placed) {
                    let cur = c.track.at(now);
                    c.track.retarget(now, Pose { alpha: 1.0, ..cur }, FADE_MS, Curve::Settle, true);
                }
            }
            self.switcher.quitting.retain(|(a, _)| !late.contains(a));
            changed_topic = true;
        }
        self.switcher.cards.retain(|c| !(c.gone && c.track.done(now)));
        match self.switcher.phase {
            Phase::Waiting if self.switcher.until.is_some_and(|t| now >= t) => {
                self.journal("switcher: the shell did not place the cards in time; closed".into());
                self.switcher.phase = Phase::Open;
                self.switcher_close("no layout", now);
            }
            Phase::Waiting | Phase::Open => {
                // The app manager brought something else to the front (a launch by voice, by an app of its own): the switcher is not what is wanted.
                let from = self.switcher.from.as_deref().unwrap_or("");
                if !foreground.is_empty() && foreground != HOME_APP && foreground != from {
                    self.switcher_close("another app came to the front", now);
                }
            }
            Phase::Closing if self.switcher.until.is_some_and(|t| now >= t) => self.switcher_finish(),
            Phase::Activating => {
                let id = self.switcher.activating.clone().unwrap_or_default();
                let grown = self.switcher.cards.iter().find(|c| c.app == id).is_none_or(|c| c.track.done(now));
                if (foreground == id && grown) || self.switcher.until.is_some_and(|t| now >= t) {
                    self.switcher_finish();
                }
            }
            _ => {}
        }
        if changed_topic {
            self.shell_sync_topics();
        }
        if self.switcher.phase == Phase::Closed {
            return;
        }
        let previews: Vec<PreviewSpec> = self
            .switcher
            .cards
            .iter()
            .filter(|c| c.placed && c.picture == Picture::Live)
            .filter_map(|c| {
                let p = c.track.at(now);
                (p.alpha >= SEEN && p.w >= 1.0 && p.h >= 1.0).then_some(PreviewSpec { surface: c.surface, rect: p.rect(), alpha: p.alpha, radius: p.radius })
            })
            .collect();
        let a = self.switcher.scrim.as_ref().map_or(0.0, |t| t.at(now).alpha);
        let [r, g, b] = self.switcher.scrim_rgb;
        self.scene.set_backdrop(Some(BackdropSpec { scrim: [r * a, g * a, b * a, a], previews }));
    }

    /// Everything is over: the pictures and the dim are dropped from the scene, and with them the renderer's copies.
    fn switcher_finish(&mut self) {
        self.switcher.phase = Phase::Closed;
        self.switcher.cards.clear();
        self.switcher.scrim = None;
        self.switcher.quitting.clear();
        self.switcher.activating = None;
        self.switcher.until = None;
        self.scene.set_backdrop(None);
        self.shell_sync_topics();
    }

    /// The shell client is gone: nothing is left to place the cards or to take the keys, so it is closed at once, without a motion.
    pub fn switcher_reset(&mut self) {
        if self.switcher.phase != Phase::Closed {
            self.journal("switcher: the shell went away; closed".into());
            self.switcher_finish();
        }
    }

    /// The topics the shell follows: `apps` (what is running, newest first) and `switcher` (whether it is open, and from where).
    pub fn switcher_sync_topics(&mut self) {
        let apps = self.switcher_apps();
        let state = |id: &str| {
            if self.switcher.quitting.iter().any(|(a, _)| a == id) {
                "quitting"
            } else if self.switcher.stuck.iter().any(|a| a == id) {
                "stuck"
            } else {
                ""
            }
        };
        let list: Vec<Value> = apps
            .iter()
            .map(|a| json!({"id": a.id, "title": a.title, "picture": a.picture.word(), "w": a.size.0, "h": a.size.1, "state": state(&a.id)}))
            .collect();
        self.shell.push_if_changed("apps", Value::Array(list));
        let open = self.switcher.accepting();
        let from = if open { self.switcher.from.clone().unwrap_or_default() } else { String::new() };
        self.shell.push_if_changed("switcher", json!({"open": open, "from": from, "n": self.switcher.seq}));
    }

    /// `ctl switcher`.
    pub fn switcher_report(&self) -> Value {
        let now = Instant::now();
        json!({
            "ok": true,
            "phase": format!("{:?}", self.switcher.phase),
            "open": self.switcher.accepting(),
            "from": self.switcher.from,
            "apps": self.switcher_apps().iter().map(|a| json!({"id": a.id, "picture": a.picture.word(), "size": [a.size.0, a.size.1]})).collect::<Vec<_>>(),
            "cards": self.switcher.cards.iter().map(|c| { let p = c.track.at(now); json!({"id": c.app, "placed": c.placed, "rect": [p.x, p.y, p.w, p.h], "alpha": p.alpha}) }).collect::<Vec<_>>(),
            "quitting": self.switcher.quitting.iter().map(|(a, _)| a).collect::<Vec<_>>(),
            "stuck": self.switcher.stuck,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(fg: &'a str) -> TapContext<'a> {
        TapContext { enabled: true, open: false, foreground: fg, home_drawn: false, apps: true }
    }

    #[test]
    fn a_single_tap_is_home_at_once_and_nothing_is_waiting_for_a_second() {
        let mut t = HomeTaps::default();
        let t0 = Instant::now();
        // the answer to the first press comes with the press: there is no state in which it is held back
        assert_eq!(t.tap(t0, &ctx("netflix")), HomeTap::Plain);
        // nothing came: a later tap is a first one again
        assert_eq!(t.tap(t0 + Duration::from_millis(900), &ctx("com.webos.app.home")), HomeTap::Plain);
    }

    #[test]
    fn a_second_tap_within_the_window_from_an_app_opens_it_with_that_app() {
        let mut t = HomeTaps::default();
        let t0 = Instant::now();
        assert_eq!(t.tap(t0, &ctx("netflix")), HomeTap::Plain);
        assert_eq!(t.tap(t0 + Duration::from_millis(300), &ctx("com.webos.app.home")), HomeTap::Open { from: "netflix".into() });
    }

    #[test]
    fn the_window_is_four_hundred_milliseconds_from_one_release_to_the_next() {
        let mut t = HomeTaps::default();
        let t0 = Instant::now();
        t.tap(t0, &ctx("plex"));
        assert_eq!(t.tap(t0 + DOUBLE_HOME, &ctx("com.webos.app.home")), HomeTap::Open { from: "plex".into() }, "400 ms exactly is still a double press");
        let mut t = HomeTaps::default();
        t.tap(t0, &ctx("plex"));
        assert_eq!(t.tap(t0 + DOUBLE_HOME + Duration::from_millis(1), &ctx("com.webos.app.home")), HomeTap::Plain, "401 ms is a second first press");
    }

    #[test]
    fn a_tap_on_home_with_apps_open_is_offered_to_the_shell_and_two_on_home_are_not_a_double_press() {
        let mut t = HomeTaps::default();
        let t0 = Instant::now();
        let on_home = TapContext { home_drawn: true, ..ctx("com.webos.app.home") };
        assert_eq!(t.tap(t0, &on_home), HomeTap::Offer);
        // the first tap was not from an app: the second is not "from" anything, it is a first again
        assert_eq!(t.tap(t0 + Duration::from_millis(100), &on_home), HomeTap::Offer);
        let none = TapContext { apps: false, ..on_home };
        assert_eq!(t.tap(t0 + Duration::from_millis(200), &none), HomeTap::Plain, "with nothing open there is nothing to offer");
    }

    #[test]
    fn with_the_switch_off_every_tap_is_plain_and_none_is_remembered() {
        let mut t = HomeTaps::default();
        let t0 = Instant::now();
        let off = TapContext { enabled: false, ..ctx("netflix") };
        assert_eq!(t.tap(t0, &off), HomeTap::Plain);
        assert_eq!(t.tap(t0 + Duration::from_millis(100), &off), HomeTap::Plain);
        assert_eq!(t.tap(t0 + Duration::from_millis(150), &ctx("com.webos.app.home")), HomeTap::Plain, "switched on since: the taps before it do not count");
    }

    #[test]
    fn a_tap_while_it_is_open_closes_it_and_does_not_start_a_double_press() {
        let mut t = HomeTaps::default();
        let t0 = Instant::now();
        t.tap(t0, &ctx("netflix"));
        let open = TapContext { open: true, ..ctx("com.webos.app.home") };
        assert_eq!(t.tap(t0 + Duration::from_millis(100), &open), HomeTap::Close);
        assert_eq!(t.tap(t0 + Duration::from_millis(150), &ctx("com.webos.app.home")), HomeTap::Plain, "the close was the end of it");
    }

    #[test]
    fn a_double_press_without_an_app_to_show_is_a_plain_one() {
        let mut t = HomeTaps::default();
        let t0 = Instant::now();
        t.tap(t0, &ctx("netflix"));
        let none = TapContext { apps: false, ..ctx("com.webos.app.home") };
        assert_eq!(t.tap(t0 + Duration::from_millis(200), &none), HomeTap::Plain);
    }

    #[test]
    fn the_curves_start_and_end_where_they_should_and_never_go_backwards() {
        for c in [Curve::Settle, Curve::Exit] {
            assert!(c.at(0.0).abs() < 1e-4 && (c.at(1.0) - 1.0).abs() < 1e-4);
            let mut last = 0.0;
            for i in 0..=100 {
                let v = c.at(i as f32 / 100.0);
                assert!(v + 1e-4 >= last, "{c:?} at {i}");
                last = v;
            }
        }
        assert!(Curve::Settle.at(0.5) > 0.8, "a settle is most of the way there at half time");
        assert!(Curve::Exit.at(0.5) < 0.4, "an exit starts slow");
    }

    #[test]
    fn a_track_lasts_its_time_whatever_the_refresh_and_a_retarget_starts_from_where_it_is() {
        let t0 = Instant::now();
        let a = Pose::new(Rect::new(0, 0, 1920, 1080), 1.0, 0.0);
        let b = Pose::new(Rect::new(112, 340, 400, 225), 1.0, 22.0);
        let mut tr = Track::new(a, b, t0, 200, Curve::Settle);
        assert_eq!(tr.at(t0).rect(), a.rect());
        assert_eq!(tr.at(t0 + Duration::from_millis(200)).rect(), b.rect());
        assert_eq!(tr.at(t0 + Duration::from_millis(900)).rect(), b.rect(), "a late frame catches up");
        assert!(!tr.done(t0 + Duration::from_millis(199)) && tr.done(t0 + Duration::from_millis(200)));
        let mid = tr.at(t0 + Duration::from_millis(80));
        tr.retarget(t0 + Duration::from_millis(80), a, 200, Curve::Settle, false);
        assert_eq!(tr.at(t0 + Duration::from_millis(80)), mid, "no jump at the turn");
        assert_eq!(tr.at(t0 + Duration::from_millis(280)).rect(), a.rect());
    }

    #[test]
    fn reduced_motion_moves_opacity_only() {
        let t0 = Instant::now();
        let a = Pose::new(Rect::new(0, 0, 100, 100), 0.0, 10.0);
        let b = Pose::new(Rect::new(400, 300, 200, 200), 1.0, 22.0);
        let mut tr = Track::still(a, t0);
        tr.retarget(t0, b, 120, Curve::Settle, true);
        let mid = tr.at(t0 + Duration::from_millis(40));
        assert_eq!(mid.rect(), b.rect(), "nothing travels");
        assert!(mid.alpha > 0.0 && mid.alpha < 1.0);
    }

    #[test]
    fn a_picture_scales_about_its_centre() {
        let p = Pose::new(Rect::new(100, 100, 400, 200), 1.0, 22.0).scaled(0.5);
        assert_eq!(p.rect(), Rect::new(200, 150, 200, 100));
    }

    #[test]
    fn inputs_and_lumatvs_own_apps_are_told_by_their_ids() {
        assert!(input_app("com.webos.app.hdmi2") && input_app("com.webos.app.livetv") && !input_app("netflix"));
        assert!(own_app("com.webos.app.home") && own_app("org.nphil.lumatv.helper") && !own_app("com.webos.app.discovery"));
    }
}
