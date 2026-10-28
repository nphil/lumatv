//! Smooth transitions between the front of the screen's states: an app launching out of its tile, going back to Home, one app
//! replacing another. LG's compositor cuts between them (and shows the video plane's black while an app has no picture yet); this
//! one animates the pictures it already has.
//!
//! The scene's own state never waits for this: the front card, the keyboard focus and SAM's foreground change in the frame the app
//! maps, as without it. What moves is only how the windows are drawn for the next 120 to 220 ms. A window in a transition is drawn
//! through a [`Pose`] (where its whole picture is, what part of it shows and with which corner radius, how opaque and how dark), which a
//! [`Track`] moves from one pose to another on the monotonic clock: a 220 ms move lasts 220 ms at 60 or 120 Hz, a frame that comes late
//! catches up, nothing counts frames. A window that left the front stays drawn (a ghost) until its track ends; a window that arrives
//! is drawn at once, on the frame after its first commit, at the pose the track starts from.
//!
//! The video plane cannot be sampled: it lies under the whole UI plane, and an app's picture that is on it is only a hole in the
//! app's window. So a window with a hole never scales, and never gets a black plane either:
//!
//! - arriving, its card is the app's own picture on a *veil* (a flat dark fill where the hole is), moved and faded as any card is; the
//!   veil stays until the video has been placed for [`VEIL_SETTLE_MS`] (or [`VEIL_CAP_MS`] after the card has arrived) and is then
//!   faded away over [`VEIL_FADE_MS`], which the display engine blends over the video: a dissolve the GPU never reads the video for;
//! - leaving, the card stays where it is (hole and all) and Home dissolves in over it, the same blend.
//!
//! Everything here is pure: no clock of its own, no GL. [`crate::scene::Scene`] feeds it the changes of the front and the frame's time,
//! asks for the pose of each window it draws, and the renderers draw through it (`render/gl.rs`, `render/software.rs`).
//!
//! `docs/research/transitions-result.md` has the design, the numbers' reasons and the TV check.

use crate::Rect;
use crate::window::WindowId;

/// A launch or an app replacing another settles like a Lucent layer (`motion.layer`).
pub const ARRIVE_MS: u32 = 220;
/// Going back to Home returns toward the origin (`motion.exit`).
pub const LEAVE_MS: u32 = 180;
/// Reduced motion: one short fade, no travel (`motion.reduce`: at most 120 ms, linear opacity only).
pub const REDUCED_MS: u32 = 120;
/// The most a transition's start waits for its pictures to be drawable.
pub const WAIT_MS: u32 = 100;
/// A veil that is let go fades out over this long (a dissolve: linear; reduced motion: [`REDUCED_MS`]).
pub const VEIL_FADE_MS: u32 = 180;
/// The veil is held until the video has had this long since its hole (and with it the placement the compositor sends to
/// videooutputd in the same step) came, so the first decoded picture is there when the fade starts.
pub const VEIL_SETTLE_MS: u32 = 150;
/// The longest a veil is held after its card has arrived: an input with no signal, a player that never comes, ends in a dissolve to
/// whatever the plane shows, not a veil that stays.
pub const VEIL_CAP_MS: u32 = 1500;
/// How long after a leave has ended the picture's last pose is still handed out by [`Transitions::take_visual`] (a second Home press).
pub const RECENT_MS: u32 = 600;
/// The veil's colour, premultiplied RGB (the dark canvas of the active palette, `#191724`); it is opaque.
pub const VEIL_RGB: [u8; 3] = [0x19, 0x17, 0x24];

/// How opaque a card is when it first shows (it is seen in its first frame), and the share of the eased progress it takes to be whole.
const ARRIVE_ALPHA0: f32 = 0.25;
const ARRIVE_ALPHA_BY: f32 = 0.6;
/// A leaving card holds its opacity for this share of the eased progress, then fades.
const LEAVE_ALPHA_FROM: f32 = 0.35;
/// How dark Home is behind an arriving card at the end of the move, and a card under an arriving one.
const HOME_SHADE: f32 = 0.55;
const CARD_SHADE: f32 = 0.6;
/// A card that arrives or leaves without a tile to go from or to: its scale at its far end, and the radius it has then (`radii.card`).
const AWAY_SCALE: f32 = 0.94;
const AWAY_RADIUS: f32 = 26.0;

/// A rectangle in output pixels with fractions: where a picture is in a frame that is between two poses.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl FRect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> FRect {
        FRect { x, y, w, h }
    }

    pub fn from_rect(r: Rect) -> FRect {
        FRect::new(r.x as f32, r.y as f32, r.w as f32, r.h as f32)
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn centre(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }

    pub fn lerp(a: &FRect, b: &FRect, t: f32) -> FRect {
        FRect::new(lerp(a.x, b.x, t), lerp(a.y, b.y, t), lerp(a.w, b.w, t), lerp(a.h, b.h, t))
    }

    /// The rectangle scaled about its centre.
    pub fn scaled(&self, s: f32) -> FRect {
        let (cx, cy) = self.centre();
        FRect::new(cx - self.w * s * 0.5, cy - self.h * s * 0.5, self.w * s, self.h * s)
    }

    /// The whole pixels that hold it, a pixel more each side for the soft edge of a rounded corner or a scaled picture: what must be
    /// redrawn when this is where a picture was or is.
    pub fn bounds(&self) -> Rect {
        let x0 = self.x.floor() as i32 - 1;
        let y0 = self.y.floor() as i32 - 1;
        let x1 = self.right().ceil() as i32 + 1;
        let y1 = self.bottom().ceil() as i32 + 1;
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// `self` as the picture of a window whose own rectangle is `window`, moved the way `window` moves into `to`: where a point of the
    /// window's, `r` (in the window's own pixels from its top left), lands.
    pub fn map_rect(window: &FRect, picture: &FRect, r: &Rect) -> FRect {
        let sx = if window.w > 0.0 { picture.w / window.w } else { 1.0 };
        let sy = if window.h > 0.0 { picture.h / window.h } else { 1.0 };
        FRect::new(picture.x + r.x as f32 * sx, picture.y + r.y as f32 * sy, r.w as f32 * sx, r.h as f32 * sy)
    }
}

/// `a` at 0 and `b` at 1 exactly (a pose at the end of its track is the pose at rest, to the bit, so that the frame that ends a transition
/// is drawn as any other).
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    if t <= 0.0 {
        a
    } else if t >= 1.0 {
        b
    } else {
        a + (b - a) * t
    }
}

/// How a window is drawn in a frame of a transition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    /// Where the window's whole picture is drawn.
    pub rect: FRect,
    /// The part of it that shows, with `radius` for its corners (a card growing out of a square tile shows a square's worth of the
    /// picture, not its stretched self).
    pub clip: FRect,
    pub radius: f32,
    /// Premultiplied opacity of the whole picture.
    pub alpha: f32,
    /// Brightness of the colour (1 leaves it, less darkens it): the scrim of what lies under an arriving card.
    pub shade: f32,
}

impl Pose {
    /// A window at rest on `rect`: drawn as it is without a transition.
    pub fn rest(rect: FRect) -> Pose {
        Pose { rect, clip: rect, radius: 0.0, alpha: 1.0, shade: 1.0 }
    }

    pub fn lerp(a: &Pose, b: &Pose, t: f32) -> Pose {
        Pose {
            rect: FRect::lerp(&a.rect, &b.rect, t),
            clip: FRect::lerp(&a.clip, &b.clip, t),
            radius: lerp(a.radius, b.radius, t),
            alpha: lerp(a.alpha, b.alpha, t),
            shade: lerp(a.shade, b.shade, t),
        }
    }

    /// Whether it is drawn as a window at rest on `window` is (a plain textured rectangle): no transform, no mask, nothing to blend.
    pub fn is_rest_on(&self, window: &FRect) -> bool {
        let close = |a: &FRect, b: &FRect| (a.x - b.x).abs() < 0.01 && (a.y - b.y).abs() < 0.01 && (a.w - b.w).abs() < 0.01 && (a.h - b.h).abs() < 0.01;
        close(&self.rect, window) && close(&self.clip, window) && self.radius < 0.01 && self.alpha >= 0.999 && self.shade >= 0.999
    }

    /// The pixels this pose touches.
    pub fn bounds(&self) -> Rect {
        self.clip.bounds().intersect(&self.rect.bounds())
    }
}

/// Which way a window's picture is eased: out for what arrives, the exit curve for what leaves, linear for a dissolve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ease {
    /// `1 - (1 - t)^3`: starts fast, settles (Lucent `motion.layer`).
    OutCubic,
    /// `cubic-bezier(.4, 0, 1, 1)`: returns toward the origin (Lucent `motion.exit`).
    Exit,
    Linear,
}

impl Ease {
    pub fn at(self, t: f32) -> f32 {
        if t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }
        match self {
            Ease::Linear => t,
            Ease::OutCubic => {
                let u = 1.0 - t;
                1.0 - u * u * u
            }
            Ease::Exit => bezier_y(0.4, 0.0, 1.0, 1.0, t),
        }
    }
}

/// `y` of the CSS cubic bezier `(x1, y1, x2, y2)` at `x`, by bisection on the curve's own parameter (the x of the curve is monotonic
/// for the control x in 0..1, as all the curves here are).
fn bezier_y(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    let at = |s: f32, a: f32, b: f32| {
        let u = 1.0 - s;
        3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s
    };
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let mid = (lo + hi) * 0.5;
        if at(mid, x1, x2) < x { lo = mid } else { hi = mid }
    }
    at((lo + hi) * 0.5, y1, y2)
}

/// How much of the pixel centred on (`px`, `py`) the rounded rectangle `clip` (corners of `radius`) covers: the signed distance to its edge, one pixel
/// of soft edge. The fragment shader of `render/gl.rs` computes the same, so the software renderer's pictures are what the GPU's are.
pub fn mask_coverage(clip: &FRect, radius: f32, px: f32, py: f32) -> f32 {
    let (cx, cy) = clip.centre();
    let (hx, hy) = (clip.w * 0.5, clip.h * 0.5);
    let r = radius.clamp(0.0, hx.min(hy).max(0.0));
    let qx = (px - cx).abs() - hx + r;
    let qy = (py - cy).abs() - hy + r;
    let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r;
    (0.5 - d).clamp(0.0, 1.0)
}

/// A window's place in the front of the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Front {
    /// Nothing the transitions know: no card, or a window that is no app (the screensaver's card).
    #[default]
    None,
    /// LumaTV's Home is what is drawn in the card slot.
    Home,
    /// An app's card.
    Card(WindowId),
}

/// What the scene knows of a window that takes part in a change of the front.
#[derive(Clone, Copy, Debug)]
pub struct Party<'a> {
    pub id: WindowId,
    pub app_id: &'a str,
    /// The window's own rectangle in the output (a card's is the output).
    pub rect: FRect,
    /// The window has a hole (its picture is on the video plane) or an exported video element that has none yet.
    pub video: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Launch,
    Leave,
    Switch,
}

/// Where an app's tile is on Home: what its card grows out of and goes back into. `from` is the same card's start while a launch
/// splash covers the tile (the splash's icon), as the shell last said.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Origin {
    pub tile: FRect,
    pub radius: f32,
    pub from: Option<(FRect, f32, u64)>,
}

/// What a veil is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Veil {
    /// Opaque. `hole_ns` is when the card's hole came (None while it has none): the video is given [`VEIL_SETTLE_MS`] from then.
    Hold { hole_ns: Option<u64> },
    /// Going, since `start_ns`.
    Fade { start_ns: u64 },
}

/// One window moving between two poses.
#[derive(Clone, Debug)]
pub struct Track {
    pub window: WindowId,
    pub from: Pose,
    pub to: Pose,
    pub start_ns: u64,
    pub dur_ns: u64,
    pub ease: Ease,
    /// The share of the eased progress over which `alpha` goes from `from` to `to`.
    alpha_span: (f32, f32),
    /// The window is no longer in the scene's stack: it is drawn for this track alone and dropped when it ends.
    pub ghost: bool,
    /// Drawn above every card (Home dissolving in over an app whose picture cannot move).
    pub on_top: bool,
    /// The window's own rectangle (what a pose at rest is).
    pub window_rect: FRect,
    pub kind: Kind,
    /// The card is coming in (it may have a veil), not going out.
    pub arriving: bool,
    veil: Option<Veil>,
    /// When the track was made, and whether a frame has drawn the pictures yet: until one has (a first buffer the driver is still busy
    /// with is not drawn), the clock starts again at each frame, so the move is seen from its start, for [`WAIT_MS`] at most.
    created_ns: u64,
    settled: bool,
}

impl Track {
    fn progress(&self, now_ns: u64) -> f32 {
        if self.dur_ns == 0 { 1.0 } else { (now_ns.saturating_sub(self.start_ns) as f32 / self.dur_ns as f32).clamp(0.0, 1.0) }
    }

    pub fn end_ns(&self) -> u64 {
        self.start_ns + self.dur_ns
    }

    pub fn pose_at(&self, now_ns: u64) -> Pose {
        let e = self.ease.at(self.progress(now_ns));
        let (a0, a1) = self.alpha_span;
        let a = if a1 > a0 {
            ((e - a0) / (a1 - a0)).clamp(0.0, 1.0)
        } else if e >= a1 {
            1.0
        } else {
            0.0
        };
        Pose {
            rect: FRect::lerp(&self.from.rect, &self.to.rect, e),
            clip: FRect::lerp(&self.from.clip, &self.to.clip, e),
            radius: lerp(self.from.radius, self.to.radius, e),
            alpha: lerp(self.from.alpha, self.to.alpha, a),
            shade: lerp(self.from.shade, self.to.shade, e),
        }
    }

    fn moving(&self, now_ns: u64) -> bool {
        now_ns < self.end_ns()
    }
}

/// Where a leave's picture last was: handed out for [`RECENT_MS`] after the leave ended.
#[derive(Clone, Copy, Debug)]
struct Recent {
    window: WindowId,
    visual: Visual,
    at_ns: u64,
}

/// Where a window's picture is: what a second Home press takes over from a leave on its way (`take_visual`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Visual {
    pub rect: Rect,
    pub alpha: f32,
    pub radius: f32,
}

/// What a veil looks like in this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VeilNow {
    /// Opacity of the veil itself, 1 while it is held.
    pub alpha: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Transitions begun, by kind.
    pub launches: u64,
    pub leaves: u64,
    pub switches: u64,
    /// A change of the front that moved a track still on its way: the picture went on from where it was.
    pub interrupted: u64,
    /// A change the engine left alone (off, no picture to move).
    pub skipped: u64,
    /// Frames that sampled a transition, and the longest and the late (over 20 ms) gaps between two of them.
    pub frames: u64,
    pub max_gap_ms: u64,
    pub late_gaps: u64,
    /// Veils that went by their cap rather than their settle.
    pub veil_caps: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub reduced: bool,
}

impl Default for Settings {
    /// Off until the TV check passes (`payload/theme/theme.json` `motion.transitions`, which `tests/test_transitions.py` holds equal).
    fn default() -> Settings {
        Settings { enabled: false, reduced: false }
    }
}

/// The set of windows in transition.
#[derive(Default)]
pub struct Transitions {
    pub settings: Settings,
    /// The app switcher is up (a second press of Home): it draws the pictures it moves itself, so no window is moved for a change of the front
    /// meanwhile (the app its OK raises appears at once under the switcher's own picture). Set by the loop before each plan.
    pub suspended: bool,
    tracks: Vec<Track>,
    origins: Vec<(String, Origin)>,
    recent: Vec<Recent>,
    pub stats: Stats,
    last_sample_ns: u64,
}

/// Most apps whose tiles are remembered.
const ORIGINS_KEPT: usize = 48;
/// A splash icon is the start of a launch only this soon after the shell said it was up.
const FROM_FRESH_NS: u64 = 15_000_000_000;

fn ns(ms: u32) -> u64 {
    ms as u64 * 1_000_000
}

impl Transitions {
    pub fn new() -> Transitions {
        Transitions::default()
    }

    pub fn enabled(&self) -> bool {
        self.settings.enabled
    }

    /// Whether anything is in transition: the scene draws through poses, the tick keeps its pace.
    pub fn active(&self) -> bool {
        !self.tracks.is_empty()
    }

    /// Whether a track is moving or fading at `now_ns` (a veil that only waits does not need a frame per refresh).
    pub fn animating(&self, now_ns: u64) -> bool {
        self.tracks.iter().any(|t| t.moving(now_ns) || matches!(t.veil, Some(Veil::Fade { .. })))
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn track_of(&self, window: WindowId) -> Option<&Track> {
        self.tracks.iter().find(|t| t.window == window)
    }

    /// Whether Home's windows are held as the underlay of a launch: Home is still being drawn then, so it is told it is still on the
    /// screen (and the video wallpaper's player is not stopped) until the card has arrived.
    pub fn holds_home(&self) -> bool {
        self.tracks.iter().any(|t| t.ghost && t.kind == Kind::Launch)
    }

    /// The shell's word on where an app's tile is (and, while a launch splash covers it, where the card starts).
    pub fn note_origin(&mut self, app_id: &str, tile: Option<(FRect, f32)>, from: Option<Option<(FRect, f32)>>, now_ns: u64) {
        if app_id.is_empty() {
            return;
        }
        let at = match self.origins.iter().position(|(a, _)| a == app_id) {
            Some(i) => i,
            None => {
                if self.origins.len() >= ORIGINS_KEPT {
                    self.origins.remove(0);
                }
                self.origins.push((app_id.to_string(), Origin { tile: FRect::default(), radius: 0.0, from: None }));
                self.origins.len() - 1
            }
        };
        let o = &mut self.origins[at].1;
        if let Some((rect, radius)) = tile {
            o.tile = rect;
            o.radius = radius;
        }
        if let Some(f) = from {
            o.from = f.map(|(rect, radius)| (rect, radius, now_ns));
        }
        // A word with no tile ever is no origin at all.
        if o.tile.w <= 0.0 || o.tile.h <= 0.0 {
            self.origins.remove(at);
        }
    }

    /// The tiles remembered, by app.
    pub fn origins(&self) -> &[(String, Origin)] {
        &self.origins
    }

    pub fn origin_of(&self, app_id: &str) -> Option<&Origin> {
        self.origins.iter().find(|(a, _)| a == app_id).map(|(_, o)| o)
    }

    fn dur(&self, ms: u32) -> u64 {
        ns(if self.settings.reduced { REDUCED_MS } else { ms })
    }

    /// Picks up `window`'s track as it is at `now_ns` (its pose then, and whether it has a veil), to move it on from there; `None` when
    /// the window is not in transition.
    fn take_track(&mut self, window: WindowId, now_ns: u64) -> Option<(Pose, Option<Veil>, bool)> {
        let i = self.tracks.iter().position(|t| t.window == window)?;
        let t = self.tracks.remove(i);
        Some((t.pose_at(now_ns), t.veil, t.ghost))
    }

    /// The front of the screen changed. Moves the windows of the change: a track still on its way is taken over where its picture is
    /// (so the change interrupts, it does not restart), a window that has none starts from the pose its part gives it. Returns whether
    /// the engine took the change (it is off, or `Kind` has nothing to move, otherwise).
    pub fn begin(&mut self, kind: Kind, now_ns: u64, incoming: Option<Party>, outgoing: Option<Party>, home: &[(WindowId, FRect)]) -> bool {
        if !self.settings.enabled || self.suspended {
            return false;
        }
        let was_active = !self.tracks.is_empty();
        match kind {
            Kind::Launch => {
                let Some(app) = incoming else { return false };
                self.arrive(app, kind, now_ns, true);
                // Home stays under the card, darkening: a ghost until the card is whole.
                for (id, rect) in home {
                    let d = self.dur(ARRIVE_MS);
                    let from = self.take_track(*id, now_ns).map(|(p, _, _)| p).unwrap_or_else(|| Pose::rest(*rect));
                    let mut to = Pose::rest(*rect);
                    to.shade = if self.settings.reduced { 1.0 } else { HOME_SHADE };
                    self.tracks.push(Track {
                        window: *id,
                        from,
                        to,
                        start_ns: now_ns,
                        dur_ns: d,
                        ease: if self.settings.reduced { Ease::Linear } else { Ease::OutCubic },
                        alpha_span: (0.0, 1.0),
                        ghost: true,
                        on_top: false,
                        window_rect: *rect,
                        kind,
                        arriving: false,
                        veil: None,
                        created_ns: now_ns,
                        settled: false,
                    });
                }
                self.stats.launches += 1;
            }
            Kind::Leave => {
                let Some(app) = outgoing else { return false };
                self.leave(app, now_ns, home);
                self.stats.leaves += 1;
            }
            Kind::Switch => {
                let (Some(from), Some(to)) = (outgoing, incoming) else { return false };
                // The card that goes stays under the one that comes, darkening.
                let d = self.dur(ARRIVE_MS);
                let start = self.take_track(from.id, now_ns).map(|(p, _, _)| p).unwrap_or_else(|| Pose::rest(from.rect));
                let mut end = Pose::rest(from.rect);
                end.shade = if self.settings.reduced { 1.0 } else { CARD_SHADE };
                self.tracks.push(Track {
                    window: from.id,
                    from: start,
                    to: end,
                    start_ns: now_ns,
                    dur_ns: d,
                    ease: if self.settings.reduced { Ease::Linear } else { Ease::OutCubic },
                    alpha_span: (0.0, 1.0),
                    ghost: true,
                    on_top: false,
                    window_rect: from.rect,
                    kind,
                    arriving: false,
                    veil: None,

                    created_ns: now_ns,

                    settled: false,
                });
                self.arrive(to, kind, now_ns, false);
                self.stats.switches += 1;
            }
        }
        if was_active {
            self.stats.interrupted += 1;
        }
        true
    }

    /// A card comes to the front: from its tile (a launch with one known), else grown from the screen's middle. A card with a video
    /// element does not scale: the veil makes its picture a card all the same, and it is the card that moves.
    fn arrive(&mut self, app: Party, kind: Kind, now_ns: u64, from_home: bool) {
        let reduced = self.settings.reduced;
        let w = app.rect;
        let origin = if from_home { self.origin_of(app.app_id).copied() } else { None };
        let (start, ease) = if reduced {
            let mut p = Pose::rest(w);
            p.alpha = 0.0;
            (p, Ease::Linear)
        } else if let Some(o) = origin {
            // The card starts as small as the tile (or the splash's icon) covers: its picture cover-fitted on the tile's rectangle,
            // masked by the tile's own, and it grows to the whole output.
            let (tile, radius) = match o.from {
                Some((rect, radius, at)) if now_ns.saturating_sub(at) < FROM_FRESH_NS => (rect, radius),
                _ => (o.tile, o.radius),
            };
            let s = (tile.w / w.w).max(tile.h / w.h);
            let (cx, cy) = tile.centre();
            let rect = FRect::new(cx - w.w * s * 0.5, cy - w.h * s * 0.5, w.w * s, w.h * s);
            (Pose { rect, clip: tile, radius, alpha: ARRIVE_ALPHA0, shade: 1.0 }, Ease::OutCubic)
        } else {
            let rect = w.scaled(AWAY_SCALE);
            (Pose { rect, clip: rect, radius: AWAY_RADIUS, alpha: ARRIVE_ALPHA0, shade: 1.0 }, Ease::OutCubic)
        };
        // Taken over where its picture is when it was on its way already (a card that left a moment ago comes back from there).
        let (from, veil) = match self.take_track(app.id, now_ns) {
            Some((pose, veil, _)) => (pose, veil),
            None => (start, None),
        };
        // (A veil only fades, so reduced motion keeps it: without one the plane's black is what shows through the hole as the card fades in.)
        let veil = veil.or(if app.video { Some(Veil::Hold { hole_ns: None }) } else { None });
        self.tracks.push(Track {
            window: app.id,
            from,
            to: Pose::rest(w),
            start_ns: now_ns,
            dur_ns: self.dur(ARRIVE_MS),
            ease,
            alpha_span: if reduced { (0.0, 1.0) } else { (0.0, ARRIVE_ALPHA_BY) },
            ghost: false,
            on_top: false,
            window_rect: w,
            kind,
            arriving: true,
            veil,

            created_ns: now_ns,

            settled: false,
        });
    }

    /// A card goes back to Home: shrinking into its tile and fading, Home brightening under it; a card whose picture is on the video
    /// plane stays where it is and Home dissolves in over it (the plane cannot shrink).
    fn leave(&mut self, app: Party, now_ns: u64, home: &[(WindowId, FRect)]) {
        let reduced = self.settings.reduced;
        let w = app.rect;
        let held = self.take_track(app.id, now_ns);
        let video = app.video || held.as_ref().is_some_and(|(_, v, _)| v.is_some());
        let tile = self.origin_of(app.app_id).copied();
        if reduced || video {
            // One dissolve: Home in over the card, which stays as it is (its hole shows the video until Home covers it).
            let d = self.dur(if reduced { REDUCED_MS } else { LEAVE_MS });
            let from = held.map(|(p, _, _)| p).unwrap_or_else(|| Pose::rest(w));
            self.tracks.push(Track {
                window: app.id,
                from,
                to: from,
                start_ns: now_ns,
                dur_ns: d,
                ease: Ease::Linear,
                alpha_span: (0.0, 1.0),
                ghost: true,
                on_top: false,
                window_rect: w,
                kind: Kind::Leave,
                arriving: false,
                veil: None,

                created_ns: now_ns,

                settled: false,
            });
            for (id, rect) in home {
                let mut start = Pose::rest(*rect);
                start.alpha = 0.0;
                if let Some((p, _, _)) = self.take_track(*id, now_ns) {
                    start = p;
                    start.alpha = start.alpha.min(0.999);
                }
                self.tracks.push(Track {
                    window: *id,
                    from: start,
                    to: Pose::rest(*rect),
                    start_ns: now_ns,
                    dur_ns: d,
                    ease: Ease::Linear,
                    alpha_span: (0.0, 1.0),
                    ghost: false,
                    on_top: true,
                    window_rect: *rect,
                    kind: Kind::Leave,
                    arriving: false,
                    veil: None,

                    created_ns: now_ns,

                    settled: false,
                });
            }
            return;
        }
        let from = held.map(|(p, _, _)| p).unwrap_or_else(|| Pose::rest(w));
        let to = match tile {
            Some(o) => {
                let s = (o.tile.w / w.w).max(o.tile.h / w.h);
                let (cx, cy) = o.tile.centre();
                let rect = FRect::new(cx - w.w * s * 0.5, cy - w.h * s * 0.5, w.w * s, w.h * s);
                Pose { rect, clip: o.tile, radius: o.radius, alpha: 0.0, shade: 1.0 }
            }
            None => {
                let rect = w.scaled(AWAY_SCALE);
                Pose { rect, clip: rect, radius: AWAY_RADIUS, alpha: 0.0, shade: 1.0 }
            }
        };
        self.tracks.push(Track {
            window: app.id,
            from,
            to,
            start_ns: now_ns,
            dur_ns: self.dur(LEAVE_MS),
            ease: Ease::Exit,
            alpha_span: (LEAVE_ALPHA_FROM, 1.0),
            ghost: true,
            on_top: false,
            window_rect: w,
            kind: Kind::Leave,
            arriving: false,
            veil: None,

            created_ns: now_ns,

            settled: false,
        });
        // Home comes back to its brightness behind the card (Home was dimmed behind the launch, or it is dimmed now as if it had been).
        for (id, rect) in home {
            let mut start = Pose::rest(*rect);
            start.shade = HOME_SHADE;
            if let Some((p, _, _)) = self.take_track(*id, now_ns) {
                start = p;
            }
            self.tracks.push(Track {
                window: *id,
                from: start,
                to: Pose::rest(*rect),
                start_ns: now_ns,
                dur_ns: self.dur(LEAVE_MS),
                ease: Ease::Exit,
                alpha_span: (0.0, 1.0),
                ghost: false,
                on_top: false,
                window_rect: *rect,
                kind: Kind::Leave,
                arriving: false,
                veil: None,

                created_ns: now_ns,

                settled: false,
            });
        }
    }

    /// One frame at `now_ns`: tracks that have ended are dropped (a leave leaves its last pose behind for [`Transitions::take_visual`]),
    /// the time between this frame and the last that sampled is counted. Returns whether anything is left in transition.
    pub fn frame(&mut self, now_ns: u64) -> bool {
        if self.tracks.is_empty() {
            self.last_sample_ns = 0;
            self.recent.retain(|r| now_ns.saturating_sub(r.at_ns) < ns(RECENT_MS));
            return false;
        }
        self.stats.frames += 1;
        if self.last_sample_ns != 0 {
            let gap = now_ns.saturating_sub(self.last_sample_ns) / 1_000_000;
            self.stats.max_gap_ms = self.stats.max_gap_ms.max(gap);
            if gap > 20 {
                self.stats.late_gaps += 1;
            }
        }
        self.last_sample_ns = now_ns;
        self.sweep(now_ns);
        !self.tracks.is_empty()
    }

    /// Drops the tracks that are over at `now_ns` (a track with a veil is over when the veil is), leaving a leave's last pose behind for
    /// [`Transitions::take_visual`] and forgetting what is older than [`RECENT_MS`].
    pub fn sweep(&mut self, now_ns: u64) {
        let mut ended: Vec<Recent> = Vec::new();
        self.tracks.retain(|t| {
            let over = now_ns >= t.end_ns() && t.veil.is_none();
            if over && t.ghost && t.kind == Kind::Leave && t.window_rect.w > 0.0 {
                let p = t.to;
                ended.push(Recent { window: t.window, visual: Visual { rect: p.rect.bounds(), alpha: p.alpha, radius: p.radius }, at_ns: now_ns });
            }
            !over
        });
        self.recent.extend(ended);
        self.recent.retain(|r| now_ns.saturating_sub(r.at_ns) < ns(RECENT_MS));
    }

    /// Starts the clock again at `now_ns` for the tracks no frame has drawn yet (a first buffer the driver is still busy with is not drawn, so the
    /// move would be seen from its middle): until [`Transitions::drawn`], [`WAIT_MS`] at most.
    pub fn anchor(&mut self, now_ns: u64) {
        for t in self.tracks.iter_mut().filter(|t| !t.settled) {
            if now_ns.saturating_sub(t.created_ns) < ns(WAIT_MS) {
                t.start_ns = now_ns;
            } else {
                t.settled = true;
            }
        }
    }

    /// A frame has been drawn. When it held every picture of the transitions (`complete`), they have started and keep their clocks.
    pub fn drawn(&mut self, complete: bool) {
        if complete {
            for t in &mut self.tracks {
                t.settled = true;
            }
        }
    }

    /// `window`'s pose in this frame, if it is in transition.
    pub fn pose(&self, window: WindowId, now_ns: u64) -> Option<Pose> {
        self.track_of(window).map(|t| t.pose_at(now_ns))
    }

    /// Whether `window` is drawn for a transition alone.
    pub fn is_ghost(&self, window: WindowId) -> bool {
        self.track_of(window).is_some_and(|t| t.ghost)
    }

    /// What the video-plane veil of `window` is in this frame. The veil is made when the card arrives expecting video (an element
    /// exported) or when its hole shows up while it arrives; it is held until the card has arrived and the video has had
    /// [`VEIL_SETTLE_MS`] since the hole came (or [`VEIL_CAP_MS`] after the arrival, hole or none), then faded away. `None` when the
    /// window has none, or it is over. `hole` is whether the window has a hole in this frame.
    pub fn veil(&mut self, window: WindowId, now_ns: u64, hole: bool) -> Option<VeilNow> {
        let fade_ns = ns(if self.settings.reduced { REDUCED_MS } else { VEIL_FADE_MS });
        let i = self.tracks.iter().position(|t| t.window == window)?;
        let t = &mut self.tracks[i];
        if !t.arriving {
            return None;
        }
        if t.veil.is_none() {
            if !hole {
                return None;
            }
            t.veil = Some(Veil::Hold { hole_ns: None });
        }
        let arrived_ns = t.end_ns();
        match t.veil? {
            Veil::Hold { hole_ns } => {
                let hole_ns = match (hole, hole_ns) {
                    (true, None) => Some(now_ns),
                    (false, _) => None,
                    (true, seen) => seen,
                };
                t.veil = Some(Veil::Hold { hole_ns });
                let settled = hole_ns.is_some_and(|h| now_ns >= arrived_ns && now_ns >= h + ns(VEIL_SETTLE_MS));
                let capped = now_ns >= arrived_ns + ns(VEIL_CAP_MS);
                if settled || capped {
                    if !settled {
                        self.stats.veil_caps += 1;
                    }
                    t.veil = Some(Veil::Fade { start_ns: now_ns });
                }
                Some(VeilNow { alpha: 1.0 })
            }
            Veil::Fade { start_ns } => {
                let f = now_ns.saturating_sub(start_ns) as f32 / fade_ns as f32;
                if f >= 1.0 {
                    t.veil = None;
                    return None;
                }
                Some(VeilNow { alpha: 1.0 - f })
            }
        }
    }

    /// Takes `window` out of the transitions and says where its picture is: a second press of Home hands the card that was on its way
    /// home to the switcher, which carries it on to its own slot. A leave that ended not long ago gives its last pose; `None` when the
    /// window had no transition.
    pub fn take_visual(&mut self, window: WindowId, now_ns: u64) -> Option<Visual> {
        if let Some(i) = self.tracks.iter().position(|t| t.window == window) {
            let t = self.tracks.remove(i);
            let p = t.pose_at(now_ns);
            return Some(Visual { rect: p.rect.bounds(), alpha: p.alpha, radius: p.radius });
        }
        let i = self.recent.iter().rposition(|r| r.window == window && now_ns.saturating_sub(r.at_ns) < ns(RECENT_MS))?;
        Some(self.recent.remove(i).visual)
    }

    /// A window is gone for good: nothing of it is left to draw.
    pub fn forget(&mut self, window: WindowId) {
        self.tracks.retain(|t| t.window != window);
        self.recent.retain(|r| r.window != window);
    }

    /// Ends every transition at once (the screen went off, the switch was turned off): everything is drawn as it is.
    pub fn clear(&mut self) {
        self.tracks.clear();
        self.recent.clear();
        self.last_sample_ns = 0;
    }
}

/// The owner's switch: the theme file's `motion.transitions` (Settings > Home & look, `lumatv theme set`) and `motion.reduced`, read again when the
/// file changes, at most once a second; `forced` is `ctl transitions on|off` over it, for a TV check. Off unless the theme says on:
/// `payload/theme/theme.json` ships it off until the TV check passes.
pub struct ThemeSwitch {
    path: std::path::PathBuf,
    stamp: Option<(std::time::SystemTime, u64)>,
    checked: Option<std::time::Instant>,
    pub on: bool,
    pub reduced: bool,
    pub forced: Option<bool>,
}

impl ThemeSwitch {
    pub fn new(data_dir: &std::path::Path) -> ThemeSwitch {
        ThemeSwitch { path: data_dir.join("theme/theme.json"), stamp: None, checked: None, on: false, reduced: false, forced: None }
    }

    /// `(motion.transitions, motion.reduced)` of a theme file's text: false for what is not there or is no boolean.
    pub fn parse(text: &str) -> (bool, bool) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return (false, false) };
        let flag = |k: &str| v.pointer(&format!("/motion/{k}")).and_then(serde_json::Value::as_bool).unwrap_or(false);
        (flag("transitions"), flag("reduced"))
    }

    /// What the engine runs with.
    pub fn settings(&self) -> Settings {
        Settings { enabled: self.forced.unwrap_or(self.on), reduced: self.reduced }
    }

    /// Reads the file again if it is not the one last read (a file that is gone is no choice: off; one that cannot be read or parsed now,
    /// as the theme is written in place, leaves what was read until the next look). Returns whether the settings changed.
    pub fn poll(&mut self, now: std::time::Instant) -> bool {
        if self.checked.is_some_and(|t| now.saturating_duration_since(t) < std::time::Duration::from_secs(1)) {
            return false;
        }
        self.checked = Some(now);
        let before = self.settings();
        let stamp = std::fs::metadata(&self.path).ok().map(|m| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len()));
        if stamp != self.stamp {
            match (stamp, std::fs::read_to_string(&self.path)) {
                (None, _) => {
                    (self.on, self.reduced) = (false, false);
                    self.stamp = None;
                }
                (Some(_), Ok(text)) if serde_json::from_str::<serde_json::Value>(&text).is_ok() => {
                    (self.on, self.reduced) = Self::parse(&text);
                    self.stamp = stamp;
                }
                _ => {}
            }
        }
        self.settings() != before
    }
}

/// The compositor's side of it: the switch, the shell's word on where a tile is (`luna://com.webos.surfacemanager/setLaunchOrigin`), the report.
impl crate::State {
    /// The tick's housekeeping: the theme file's switch, looked at once a second.
    pub fn motion_tick(&mut self, now: std::time::Instant) {
        if self.motion.poll(now) {
            self.apply_motion();
        }
    }

    /// Puts the switch's settings in force: off ends every transition at once (everything is drawn as it is), and the shell is told.
    pub fn apply_motion(&mut self) {
        let settings = self.motion.settings();
        if settings == self.scene.transitions.settings {
            return;
        }
        self.scene.transitions.settings = settings;
        if !settings.enabled {
            self.scene.transitions.clear();
            self.scene.redraw_all();
        }
        self.journal(format!("transitions: {}{}", if settings.enabled { "on" } else { "off" }, if settings.reduced { ", reduced motion" } else { "" }));
        self.shell_sync_topics();
    }

    /// `setLaunchOrigin {appId, tile: {x, y, width, height, radius}, from: {…} | null}`: where the app's tile is on Home (a card grows out of it
    /// and goes back into it) and, while the launch splash covers it, where the card starts instead (null takes that back). Output pixels.
    pub fn set_launch_origin(&mut self, payload: &serde_json::Value) -> serde_json::Value {
        let rect = |v: &serde_json::Value| -> Option<(FRect, f32)> {
            let n = |k: &str| v.get(k).and_then(serde_json::Value::as_f64).map(|x| x as f32);
            Some((FRect::new(n("x")?, n("y")?, n("width")?, n("height")?), n("radius").unwrap_or(0.0)))
        };
        let Some(app) = payload.get("appId").and_then(serde_json::Value::as_str) else {
            return serde_json::json!({"returnValue": false, "errorText": "appId required"});
        };
        let tile = payload.get("tile").and_then(rect);
        let from = match payload.get("from") {
            Some(serde_json::Value::Null) => Some(None),
            Some(v) => rect(v).map(Some),
            None => None,
        };
        let now = crate::output::monotonic_ns();
        self.scene.transitions.note_origin(app, tile, from, now);
        serde_json::json!({"returnValue": true})
    }

    /// What `ctl transitions` answers: the switch, what is in transition, and the counters a TV check reads.
    pub fn transitions_report(&self) -> serde_json::Value {
        let t = &self.scene.transitions;
        let now = self.scene.plan_ns();
        let tracks: Vec<serde_json::Value> = t
            .tracks()
            .iter()
            .map(|k| {
                let p = k.pose_at(now);
                serde_json::json!({
                    "window": k.window.0, "kind": format!("{:?}", k.kind), "ghost": k.ghost, "arriving": k.arriving,
                    "pose": {"x": p.rect.x, "y": p.rect.y, "w": p.rect.w, "h": p.rect.h, "alpha": p.alpha, "shade": p.shade, "radius": p.radius},
                })
            })
            .collect();
        serde_json::json!({
            "ok": true,
            "enabled": t.settings.enabled,
            "reduced": t.settings.reduced,
            "theme": {"on": self.motion.on, "reduced": self.motion.reduced},
            "forced": self.motion.forced,
            "active": t.active(),
            "origins": t.origins().iter().map(|(app, o)| serde_json::json!({"app": app, "tile": {"x": o.tile.x, "y": o.tile.y, "w": o.tile.w, "h": o.tile.h, "radius": o.radius}, "from": o.from.is_some()})).collect::<Vec<_>>(),
            "tracks": tracks,
            "stats": {
                "launches": t.stats.launches, "leaves": t.stats.leaves, "switches": t.stats.switches, "interrupted": t.stats.interrupted,
                "skipped": t.stats.skipped, "frames": t.stats.frames, "maxGapMs": t.stats.max_gap_ms, "lateGaps": t.stats.late_gaps, "veilCaps": t.stats.veil_caps,
            },
        })
    }
}

#[cfg(test)]
mod tests;
