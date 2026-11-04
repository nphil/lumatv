//! The scene: which windows show, in what order, where, and what the renderer must draw this frame.
//!
//! Stacking follows LG's views (`docs/research/compositor-c2-result.md` §6): one card at a time (the newest
//! mapped one is the foreground app), then overlays, system UI, popups, the keyboard and the cover. Surface
//! groups stack their members around the owner. Punch-through regions become holes cleared to transparent so the
//! display engine's video window shows through.
//!
//! [`Scene::plan`] writes into storage the scene keeps, so a frame allocates nothing once the scene has been
//! planned at its size (`alloc_tests`); the plan borrows the scene for the frame.

mod backdrop;

use std::collections::BTreeMap;

use wayland_server::protocol::wl_buffer::WlBuffer;

use backdrop::BackdropSig;
pub use backdrop::{Backdrop, BackdropSpec, PlannedPreview, PreviewSpec, coverage};

use crate::damage::Tiles;
use crate::mask::{MAX_RECTS, Mask, Op};
use crate::surface::{Role, SurfaceId, Surfaces};
use crate::transition::{FRect, Front, Kind, Party, Pose, Transitions, Visual};
use crate::window::{Layer, ShellState, Window, WindowId, WindowType};
use crate::{OUTPUT_HEIGHT, OUTPUT_WIDTH, Rect};

/// SAM's Home app: the Home stub's transparent card under LumaTV's compositor (`tools/lumatv homestub`), LG's own Home
/// otherwise. The card LumaTV's Home is drawn in place of.
pub const HOME_APP: &str = "com.webos.app.home";

/// LG's guide (the Guide and List keys, Live TV's menu entry, the banner's Guide button): an Overlay window of this app, which the compositor does
/// not draw while LumaTV's UI answers for it (`lg_guide.rs`).
pub const LG_GUIDE_APP: &str = "com.webos.app.livemenu";

/// Native Home is an engine layer, represented in transitions by a synthetic window ID.
pub const HOME_RUST: WindowId = WindowId(u32::MAX);

/// LG's keyboard at 1080p: maliit's `keyboardDefaultHeight` (`876 / resolution`, resolution 2) and what a window that was sized to the
/// keyboard is on the set (1920x438 in the gate's good runs). The keyboard sits in the top rows of its window (`normalKeyboard` at y 0).
pub const KEYBOARD_HEIGHT: i32 = 438;

/// The part of a keyboard window that is the keyboard, in the window's own coordinates. maliit's window is not always the keyboard's size:
/// on the TV it came 1920x1080 (the keyboard drawn in the top rows, the rest transparent) in 5 of 10 runs and 1920x438 in the other 5, and
/// round 7's journal shows the 1080 window with no input region, so neither the region nor the size says where the keyboard is. In order:
/// the bounding box of the window's input region, clipped to the window; else the whole window when it is not taller than three quarters
/// of the output (a window sized to its keyboard); else the top [`KEYBOARD_HEIGHT`] rows of it (a window the size of the screen, which
/// no keyboard is).
pub fn keyboard_geometry(size: (i32, i32), input: Option<&[Rect]>) -> Rect {
    let whole = Rect::new(0, 0, size.0, size.1);
    let bounds = input.map(|rects| rects.iter().map(|r| r.intersect(&whole)).fold(Rect::default(), |a, r| a.union(&r))).unwrap_or_default();
    if !bounds.is_empty() {
        bounds
    } else if size.1 > OUTPUT_HEIGHT as i32 * 3 / 4 {
        Rect::new(0, 0, size.0, KEYBOARD_HEIGHT.min(size.1))
    } else {
        whole
    }
}

/// Where a keyboard window's origin goes so that its keyboard (`g`, from [`keyboard_geometry`]) is LG's `KeyboardView` panel: the full
/// width's centre and the bottom edge of the output, or the corner of the rectangle the app asked for (`preferred`).
pub fn keyboard_origin(g: Rect, preferred: Option<Rect>) -> (i32, i32) {
    match preferred {
        Some(r) => (r.x - g.x, r.y - g.y),
        None => ((OUTPUT_WIDTH as i32 - g.w) / 2 - g.x, OUTPUT_HEIGHT as i32 - g.h - g.y),
    }
}

/// How the surfaces of one window are laid out and drawn in this frame.
#[derive(Clone, Copy)]
struct Look {
    window: WindowId,
    origin: (i32, i32),
    dim: f32,
    pose: Option<(Pose, FRect)>,
    ghost: bool,
    /// The surfaces are members drawn below a window whose hole covers them (`Scene::hole_rects`): what the hole covers is not drawn.
    under_hole: bool,
    /// An app's picture: drawn through the box dim's map while one is engaged (`boxdim`).
    gain: bool,
    /// What the OLED care moved the window by (the app features' shift): [`Item::drift`].
    drift: (i32, i32),
}

/// How a video window whose claim is a texture is drawn: the feeder's buffer (the picture, at whatever size it painted) stretched over the claim's rectangle, as
/// a window at rest on that rectangle. A window that has not painted yet (it is `vwin::TEXTURE_WINDOW_PX` square, and empty) is not drawn stretched.
fn texture_pose(w: &Window, surfaces: &Surfaces) -> Option<(Pose, FRect)> {
    if w.kind != WindowType::VideoWindow || !w.vwin_live || !w.vwin_texture || w.vwin_rect.is_empty() {
        return None;
    }
    let (sw, sh) = surfaces.get(w.surface)?.size();
    if sw <= crate::vwin::TEXTURE_WINDOW_PX || sh <= crate::vwin::TEXTURE_WINDOW_PX {
        return None;
    }
    Some((Pose::rest(FRect::from_rect(w.vwin_rect)), FRect::new(0.0, 0.0, sw as f32, sh as f32)))
}

/// A card as a transition sees it: its app, its rectangle and whether its picture is (or is about to be) on the video plane.
fn party(id: WindowId, info: &(String, bool), rect: FRect) -> Party<'_> {
    Party { id, app_id: &info.0, rect, video: info.1 }
}

/// LG's apps that map as full-screen overlay windows, not cards (SAM never names them as the foreground app): their opening and closing is a
/// change of the front for the transitions whether or not their surface says it is opaque.
const FULLSCREEN_OVERLAYS: &[&str] = &["com.palm.app.settings"];

/// What a window that was destroyed at the front of the screen left to draw while a transition takes it away: its last buffer (the texture of it
/// stays while the plan draws it) and where and how it was drawn. Made by [`Scene::zombify`] the moment its surface goes.
pub struct Zombie {
    pub window: WindowId,
    app_id: String,
    video: bool,
    layer: Layer,
    serial: u64,
    surface: SurfaceId,
    buffer: WlBuffer,
    rect: Rect,
    y_invert: bool,
    since_ns: u64,
}

/// How long a zombie is kept at most, whatever its transition does.
const ZOMBIE_NS: u64 = 1_000_000_000;

/// A named `wl_webos_surface_group`, owned by one surface.
pub struct Group {
    pub name: String,
    pub owner: SurfaceId,
    pub layers: BTreeMap<String, GroupLayer>,
    pub allow_anonymous: bool,
    /// Members attached without a layer: (surface, z_hint).
    pub anonymous: Vec<(SurfaceId, u32)>,
}

pub struct GroupLayer {
    pub z: i32,
    pub key_index: i32,
    pub member: Option<SurfaceId>,
}

impl Group {
    /// Whether `surface` is attached to this group, to a named layer or anonymously.
    pub fn has_member(&self, surface: SurfaceId) -> bool {
        self.layers.values().any(|l| l.member == Some(surface)) || self.anonymous.iter().any(|(s, _)| *s == surface)
    }

    /// The attached surfaces, each with where it stacks against the owner: a layer's z (negative below the owner), an anonymous
    /// surface's `z_hint` (below = 0, above = 1, top = 2) as a z beyond every layer's. Not sorted.
    pub fn member_stack(&self) -> impl Iterator<Item = (i32, SurfaceId)> + '_ {
        let layered = self.layers.values().filter_map(|l| l.member.map(|m| (l.z, m)));
        let anonymous = self.anonymous.iter().map(|(s, hint)| {
            (
                match hint {
                    0 => i32::MIN / 2,
                    2 => i32::MAX / 2,
                    _ => i32::MAX / 4,
                },
                *s,
            )
        });
        layered.chain(anonymous)
    }
}

/// A rectangle the renderer clears to transparent black: video shows through from the plane below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hole {
    pub surface: SurfaceId,
    pub rect: Rect,
}

/// A hole as the renderer draws it: where, and the index in the plan's items of the surface it belongs to. LG's compositor draws a
/// video's punch-through at z -1 inside its window (`views/punchThrough.js`): what is under the window is cleared and the window's own
/// pixels go on top of the clear, so the renderer clears a hole just before it draws that item. What is drawn above the item (Home's glass over the
/// moving wallpaper's video, an app's controls over its own video) is not touched by it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlannedHole {
    /// The surface-local request used to build this plan.
    pub source: Hole,
    /// The pixels the hole touches (a veiled hole: the veil's, as moved by its card's pose).
    pub rect: Rect,
    pub before: usize,
    /// A veil over the hole while its card arrives (`transition.rs`): drawn just after the hole's clear and under the card's own pixels.
    pub veil: Option<HoleVeil>,
}

/// The regions of a window's picture the plan does not draw (`mask.rs`): in output pixels, on the item of the window's surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlannedMask {
    pub window: WindowId,
    pub surface: SurfaceId,
    rects: [Rect; MAX_RECTS],
    len: usize,
}

impl PlannedMask {
    pub fn rects(&self) -> &[Rect] {
        &self.rects[..self.len]
    }
}

/// A flat fill where an arriving card's hole is, so the video plane is not what shows (black until it plays) while the card moves in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HoleVeil {
    /// Where the fill is drawn, and the mask it shows through (the card's own, with its corner radius).
    pub quad: FRect,
    pub clip: FRect,
    pub radius: f32,
    /// Premultiplied opacity: the card's, times the veil's own while it fades away.
    pub alpha: f32,
    /// Whether the hole is cleared to transparent before the fill goes on (a veil that fades away over the plane shows the plane
    /// through what is left of it; a veil that arrives lies over what is under the card).
    pub clear: bool,
}

/// How an item is drawn while its window is in a transition (`transition::Pose`): the whole window's mask, corner radius, opacity and
/// brightness, and where this item's own picture goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Posed {
    pub quad: FRect,
    pub clip: FRect,
    pub radius: f32,
    pub alpha: f32,
    pub shade: f32,
}

impl Posed {
    /// The pixels it draws on.
    pub fn bounds(&self) -> Rect {
        self.quad.bounds().intersect(&self.clip.bounds())
    }

    fn key(&self) -> [u32; 12] {
        let b = f32::to_bits;
        [
            b(self.quad.x),
            b(self.quad.y),
            b(self.quad.w),
            b(self.quad.h),
            b(self.clip.x),
            b(self.clip.y),
            b(self.clip.w),
            b(self.clip.h),
            b(self.radius),
            b(self.alpha),
            b(self.shade),
            0,
        ]
    }
}

/// One surface to draw.
pub struct Item {
    pub window: WindowId,
    pub surface: SurfaceId,
    pub buffer: WlBuffer,
    pub rect: Rect,
    /// This item's damage rectangles are `plan.damage_rects[damage.0..damage.1]`, in output pixels; `None` for
    /// the whole rect.
    pub damage: Option<(usize, usize)>,
    pub opaque: bool,
    pub y_invert: bool,
    /// The brightness this item is drawn at, 1 for all but the shell's windows while the OLED care dims them (`oledcare`).
    pub dim: f32,
    /// `Some` while the window is in a transition and is not at rest: the item is drawn through it, not on `rect`.
    pub pose: Option<Posed>,
    /// Drawn for a transition alone: the window left the stack already (the pointer and the keys are not its).
    pub ghost: bool,
    /// An app window's picture (not the shell's, not the wallpaper's): the box dim's map applies to it while [`Plan::gain`] is on.
    pub gain: bool,
    /// How far the OLED care moved `rect` from where the window places it: the shell's orbit for its own screens, the app features' shift for
    /// the card in front, (0, 0) otherwise. The box dim's sample takes it away, so it reads the app's own picture and a shift is never a
    /// change of it; a plane that can be placed carries it instead of composing the frame ([`Plan::sole_placed`]).
    pub drift: (i32, i32),
    /// The surface asked for a buffer transform or a scale: the output draws the picture as it is (it applies neither), but a plane could not
    /// show it any other way either, so a window that did is never scanned out whole.
    pub xform: bool,
}

// Map client canvas coordinates into the drawn quad, rounding outward.
fn subtitle_rect(bounds: crate::surface::SubtitleBounds, quad: FRect, clip: FRect) -> Option<Option<Rect>> {
    let r = bounds.rect;
    if bounds.canvas_width <= 0 || bounds.canvas_height <= 0 || quad.w <= 0.0 || quad.h <= 0.0
        || ![quad.x, quad.y, quad.w, quad.h, clip.x, clip.y, clip.w, clip.h].iter().all(|v| v.is_finite())
    {
        return None;
    }
    if r.w == 0 && r.h == 0 {
        return Some(None);
    }
    let sx = quad.w as f64 / bounds.canvas_width as f64;
    let sy = quad.h as f64 / bounds.canvas_height as f64;
    let left = (quad.x as f64 + r.x as f64 * sx).max(clip.x as f64).max(0.0);
    let top = (quad.y as f64 + r.y as f64 * sy).max(clip.y as f64).max(0.0);
    let right = (quad.x as f64 + (r.x as f64 + r.w as f64) * sx).min(clip.right() as f64).min(OUTPUT_WIDTH as f64);
    let bottom = (quad.y as f64 + (r.y as f64 + r.h as f64) * sy).min(clip.bottom() as f64).min(OUTPUT_HEIGHT as f64);
    if right <= left || bottom <= top {
        return Some(None);
    }
    let (x, y, right, bottom) = (left.floor() as i32, top.floor() as i32, right.ceil() as i32, bottom.ceil() as i32);
    Some(Some(Rect::new(x, y, right - x, bottom - y)))
}


impl Item {
    /// The pixels this item is drawn on: `rect`, or what its pose moves it to.
    pub fn drawn_rect(&self) -> Rect {
        self.pose.map_or(self.rect, |p| p.bounds())
    }
}

/// Why a plan is not one window the plane could take whole: the first thing it has that a scan-out cannot show ([`Plan::sole_candidate`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotSole {
    /// No window to show.
    Empty,
    /// More than one picture: a system layer, a keyboard, a member over or under the card, the wallpaper's window.
    Layers(usize),
    /// The app switcher's scrim and pictures are drawn over the windows.
    Switcher,
    /// A window is in a transition (drawn through a pose).
    Transition,
    /// The pointer's sprite is drawn over the windows.
    Cursor,
    /// A hole for the video plane, or a window that has exported video and expects one.
    Video,
    /// The OLED care dims the picture.
    Dimmed,
    /// The box dim's map multiplies the picture.
    Gain,
    /// The picture does not cover the output exactly.
    Geometry,
    /// The picture covers the output but the OLED care's orbit has it off the origin, where the plane cannot put it: a shift up or left (its
    /// position is unsigned), or any shift while the plane is not allowed to carry one (`--scanout-offset`).
    Shifted,
    /// Neither the surface nor the buffer's format says the picture is opaque.
    Translucent,
    /// The surface asked for a buffer transform or a scale.
    Transformed,
}

impl NotSole {
    /// A short name, the key of the counters on the control socket.
    pub fn key(self) -> &'static str {
        match self {
            NotSole::Empty => "empty",
            NotSole::Layers(_) => "layers",
            NotSole::Switcher => "switcher",
            NotSole::Transition => "transition",
            NotSole::Cursor => "cursor",
            NotSole::Video => "video",
            NotSole::Dimmed => "dimmed",
            NotSole::Gain => "gain",
            NotSole::Geometry => "geometry",
            NotSole::Shifted => "shifted",
            NotSole::Translucent => "translucent",
            NotSole::Transformed => "transformed",
        }
    }

    /// What it says in the journal.
    pub fn text(self) -> String {
        match self {
            NotSole::Empty => "no window".into(),
            NotSole::Layers(n) => format!("{n} pictures on the screen (a system layer, keyboard or member over the window)"),
            NotSole::Switcher => "the app switcher is open".into(),
            NotSole::Transition => "a window is moving (transition)".into(),
            NotSole::Cursor => "the pointer shows".into(),
            NotSole::Video => "a video hole or a window expecting one".into(),
            NotSole::Dimmed => "the OLED care dims the picture".into(),
            NotSole::Gain => "the box dim's map is on".into(),
            NotSole::Geometry => "the window does not cover the output".into(),
            NotSole::Shifted => "the OLED care's orbit has the window off the origin".into(),
            NotSole::Translucent => "the picture is not opaque".into(),
            NotSole::Transformed => "the surface has a buffer transform or scale".into(),
        }
    }
}

/// Everything the renderer needs for one frame, bottom to top. Lives in the [`Scene`]; `plan` refreshes it.
#[derive(Default)]
pub struct Plan {
    pub items: Vec<Item>,
    /// The app switcher's scrim and pictures, drawn before the shell's system layer (`scene/backdrop.rs`); `None` while it is closed.
    pub backdrop: Option<Backdrop>,
    pub holes: Vec<PlannedHole>,
    /// Where an item's picture is not drawn while the shell's overlay stands in for it ([`Plan::blank_of`]).
    pub masks: Vec<PlannedMask>,
    pub damage_rects: Vec<Rect>,
    /// Bounds of everything that changed, in output pixels; empty when nothing did.
    pub damage: Rect,
    /// The changed tiles (`damage::TILE` grid), for the scan-out rings.
    pub tiles: Option<Tiles>,
    /// Where the pointer's sprite goes, above everything, while the cursor shows.
    pub cursor: Option<Rect>,
    /// When the oldest commit this frame shows for the first time arrived (CLOCK_MONOTONIC ns); `None` when it
    /// shows no new commit (a layout request, the system UI), which is judged as due since the last flip.
    pub content_ns: Option<u64>,
    /// The box dim's gain map while any cell of it is below 1 (`gain_on`), copied when it changes (`gain_serial` counts the copies, so the
    /// renderer re-uploads only then). Items with [`Item::gain`] that are not posed are drawn through it.
    pub gain: crate::boxdim::GainMap,
    pub gain_on: bool,
    pub gain_serial: u64,
    /// A window of the plan has exported a video element (it has a hole or expects one): its picture shares the screen with the video plane.
    pub video: bool,
    /// The Rust Home (the engine's stack 0) is under a window that covers the output and nothing moves: nothing of it can be seen, so the frame
    /// does not draw it (and its changes owe nothing): what an overlay over a playing video asked the GPU for was Home's 70-odd layers again.
    pub home_hidden: bool,
    /// Black veil opacity over native Home, below windows and system UI.
    pub home_dim: f32,
    /// Number of wallpaper items below native Home.
    pub home_before: usize,
    /// A transition is moving or fading at the plan's time (the engine's own animations are asked of it: `Gl::ui_animating`): the frame after this
    /// one is due at the next refresh.
    pub animating: bool,
}

impl Plan {
    /// The window the plane could take whole, if the plan is one: a single picture that covers the output exactly, nothing drawn above or
    /// below it (no layer, hole, pointer, switcher, pose, dim or gain map), whatever its opacity. The first thing the plan has that a scan-out
    /// cannot show is the error. Opacity is for the caller: the plan says whether the surface declared one, the buffer's own format may say it
    /// too ([`crate::scanout`]).
    pub fn sole_candidate(&self) -> Result<&Item, NotSole> {
        match self.sole_placed()? {
            (item, (0, 0)) => Ok(item),
            _ => Err(NotSole::Shifted),
        }
    }

    /// [`Plan::sole_candidate`] for a plane that can be placed: the window and where the OLED care's orbit has it, (0, 0) at the origin. The
    /// picture is the output's size and sits exactly where the care moved it from the origin ([`Item::drift`]); a window placed anywhere else
    /// by its own position is `Geometry`, as a plane cannot show it the way composition does. The sign is the caller's: the primary plane's
    /// position is unsigned in the driver (`osd_hdr_xpos/ypos`, 12 bits), so a shift up or left is not for it.
    pub fn sole_placed(&self) -> Result<(&Item, (i32, i32)), NotSole> {
        let full = Rect::new(0, 0, OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32);
        let one = match self.items.as_slice() {
            [] => return Err(NotSole::Empty),
            [one] => one,
            many => return Err(NotSole::Layers(many.len())),
        };
        let at = (one.rect.x, one.rect.y);
        if self.backdrop.is_some() {
            Err(NotSole::Switcher)
        } else if one.pose.is_some() {
            Err(NotSole::Transition)
        } else if self.cursor.is_some() {
            Err(NotSole::Cursor)
        } else if !self.holes.is_empty() || !self.masks.is_empty() || self.video {
            Err(NotSole::Video)
        } else if one.dim != 1.0 || (self.home_dim > 0.0 && !self.home_hidden) {
            Err(NotSole::Dimmed)
        } else if self.gain_on {
            Err(NotSole::Gain)
        } else if (one.rect.w, one.rect.h) != (full.w, full.h) || at != one.drift {
            Err(NotSole::Geometry)
        } else if one.xform {
            Err(NotSole::Transformed)
        } else {
            Ok((one, at))
        }
    }

    /// [`Plan::sole_candidate`] when the surface declared the picture opaque.
    pub fn sole_fullscreen(&self) -> Option<&Item> {
        self.sole_candidate().ok().filter(|i| i.opaque)
    }

    pub fn item_damage(&self, item: &Item) -> Option<&[Rect]> {
        item.damage.map(|(a, b)| &self.damage_rects[a..b])
    }

    /// The regions of `item`'s picture the renderers leave undrawn (what the shell's overlay stands in for, `mask.rs`); none for almost every item.
    pub fn blank_of(&self, item: &Item) -> &[Rect] {
        if item.pose.is_some() {
            return &[];
        }
        self.masks.iter().find(|m| m.window == item.window && m.surface == item.surface).map_or(&[], PlannedMask::rects)
    }

    /// The gain map the renderer multiplies app items with, while one is engaged.
    pub fn gain(&self) -> Option<&crate::boxdim::GainMap> {
        self.gain_on.then_some(&self.gain)
    }

    fn clear(&mut self) {
        self.items.clear();
        self.backdrop = None;
        self.holes.clear();
        self.masks.clear();
        self.damage_rects.clear();
        self.damage = Rect::default();
        self.cursor = None;
        self.content_ns = None;
        self.video = false;
        self.home_hidden = false;
        self.animating = false;
        if let Some(t) = &mut self.tiles {
            t.clear();
        }
    }
}

/// What one drawn item looked like to the eye, apart from its content: the plans of two frames with the same list of
/// these, in the same order, and the same holes show the same picture wherever no surface damage says otherwise.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Placed {
    surface: SurfaceId,
    rect: Rect,
    opaque: bool,
    y_invert: bool,
    /// [`Item::dim`] in hundredths.
    dim: u8,
    /// The pose it was drawn through, bit for bit: a frame of a transition is never the same as the last.
    pose: Option<[u32; 12]>,
}

/// Whether two plans list the same windows, every one as it was, in another order: a restack.
fn only_restacked(last: &[Placed], now: &[Placed]) -> bool {
    last.len() == now.len() && last.iter().all(|p| now.contains(p))
}

/// Whether two plans show the same surfaces in the same order at the same places, as far as the box dim's grid cares: a dim level or
/// a drift of up to [`crate::oledcare::Settings::radius`]'s most (4 px, the care's orbit) does not change what is still on the screen, a
/// different surface, a pose or a move of more does.
fn same_layout(a: &[Placed], b: &[Placed]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(p, q)| {
            p.surface == q.surface
                && p.pose.is_none()
                && q.pose.is_none()
                && (p.rect.x - q.rect.x).abs() <= 4
                && (p.rect.y - q.rect.y).abs() <= 4
                && p.rect.w == q.rect.w
                && p.rect.h == q.rect.h
        })
}

#[derive(Default)]
pub struct Scene {
    windows: BTreeMap<WindowId, Window>,
    by_surface: BTreeMap<SurfaceId, WindowId>,
    next_window: u32,
    map_counter: u64,
    pub groups: Vec<Group>,
    pub holes: Vec<Hole>,
    /// The pointer's sprite rectangle while the cursor shows (set by the state before each plan).
    pub cursor: Option<Rect>,
    last_cursor: Option<Rect>,
    /// Where the field that holds the text-input activation wants the keyboard (`text_model.set_input_panel_rect`), in output pixels.
    pub keyboard_pref: Option<Rect>,
    /// Where the shell's own keyboard is on the screen while it is: the system layer takes the pointer there and nowhere else (its window has no
    /// key focus to be the pointer's by: the field's app keeps that).
    pub shell_pointer: Option<Rect>,
    /// The next plan damages everything it covers whatever changed ([`Scene::redraw_all`]).
    redraw_all: bool,
    /// The app switcher's frame (`set_backdrop`, set by the state before each plan) and what the last plan drew of it.
    backdrop: Option<BackdropSpec>,
    last_backdrop: BackdropSig,
    /// What the last plan drew and where, to damage what moved, appeared, restacked or vanished.
    last_placed: Vec<Placed>,
    /// Planned holes invalidated by surface mutations since the last plan.
    invalid_video_holes: Vec<SurfaceId>,
    last_holes: Vec<PlannedHole>,
    /// The shell's claim on a window's region (`mask.rs`), and where the last plan blanked.
    mask: Option<Mask>,
    last_masks: Vec<PlannedMask>,
    /// LumaTV's UI answers for LG's guide: its windows are left out of the stack, so they are never drawn, never take the keyboard focus and are never
    /// under the pointer (`lg_guide.rs`).
    guide_held: bool,
    plan: Plan,
    /// Whether the last stack built drew LumaTV's Home in the card slot ([`Scene::shell_home_shown`]).
    home_shown: bool,
    home_rust: bool,
    /// The windows in transition (`transition.rs`): drawn through poses while their tracks run.
    pub transitions: Transitions,
    /// The clock of the last plan, CLOCK_MONOTONIC ns: what `take_visual` answers at.
    plan_ns: u64,
    /// What the last stack built had at the front of the screen, and what the plan before it had (a change starts a transition).
    front: Front,
    last_front: Front,
    /// Surfaces whose app exported a video element: they expect a hole (the picture on the video plane) even before it is there.
    video: Vec<SurfaceId>,
    /// This frame's draw order: the stack and the windows a transition still draws.
    draw: Vec<(Layer, u64, WindowId)>,
    /// The windows of the last plan's stack: one that was drawn already when the front changed is revealed by it, not arriving.
    prev_stack: Vec<WindowId>,
    /// Windows destroyed at the front of the screen that a transition still draws.
    zombies: Vec<Zombie>,
    // Scratch space for planning, reused across frames.
    stack: Vec<(Layer, u64, WindowId)>,
    members: Vec<(i32, SurfaceId)>,
    tree: Vec<(SurfaceId, i32, i32)>,
    /// The holes of the window being laid out, in output pixels, as this frame will plan them.
    hole_rects: Vec<Rect>,
    placed: Vec<Placed>,
    /// The shell's own screens' OLED care: the orbit offset and the dim its beats set (`oledcare`).
    pub care: crate::oledcare::Care,
    /// The last plan was drawn for a care beat alone: no window had a commit to draw ([`Scene::care_frame`]).
    care_only: bool,
    /// The box dim of app pictures (`boxdim`): its grid, and the map the plan carries while it is engaged.
    pub boxdim: crate::boxdim::BoxDim,
}

impl Scene {
    /// Whether the last frame planned a visible clear for this unchanged surface-local hole.
    pub fn video_hole_visible(&self, window: WindowId, surface: SurfaceId, rect: Rect) -> bool {
        self.plan.holes.iter().any(|h| {
            h.source == (Hole { surface, rect })
                && !self.invalid_video_holes.contains(&surface)
                && self.holes.contains(&h.source)
                && self.plan.items.get(h.before).is_some_and(|item| item.window == window && item.surface == surface && !item.ghost)
                && h.veil.is_none_or(|veil| veil.clear && veil.alpha < 1.0)
                && !self.hole_covered_or_uncertain(h)
        })
    }

    // Later clears may reveal a different plane, whose identity the plan cannot establish. Count those pixels as uncertain.
    fn hole_covered_or_uncertain(&self, hole: &PlannedHole) -> bool {
        // Sweep without allocating. The hole's own item is excluded: its controls are drawn over the clear.
        let rects = || self.plan.items[hole.before + 1..].iter().filter(|item| item.opaque && item.pose.is_none()).map(|item| item.rect)
            .chain(self.plan.holes.iter().filter(|other| other.before > hole.before && other.veil.is_none_or(|veil| veil.clear)).map(|other| other.rect));
        let mut x = hole.rect.x;
        while x < hole.rect.right() {
            let mut next_x = hole.rect.right();
            for rect in rects() {
                if rect.x > x { next_x = next_x.min(rect.x); }
                if rect.right() > x { next_x = next_x.min(rect.right()); }
            }
            let mut y = hole.rect.y;
            while y < hole.rect.bottom() {
                let end = rects().filter(|rect| rect.x <= x && rect.right() >= next_x
                    && rect.y <= y && rect.bottom() > y)
                    .map(|rect| rect.bottom()).max().unwrap_or(y);
                if end == y { return false; }
                y = end;
            }
            x = next_x;
        }
        true
    }

    /// Invalidate planned holes on a surface and its subsurface/group descendants until the next plan.
    pub fn invalidate_video_holes_for_surface(&mut self, surface: SurfaceId, surfaces: &Surfaces) {
        for hole in &self.plan.holes {
            let mut current = hole.source.surface;
            loop {
                if current == surface {
                    if !self.invalid_video_holes.contains(&hole.source.surface) {
                        self.invalid_video_holes.push(hole.source.surface);
                    }
                    break;
                }
                current = match surfaces.get(current).map(|s| s.role) {
                    Some(Role::Subsurface { parent }) => parent,
                    Some(Role::GroupMember { owner }) => owner,
                    _ => break,
                };
            }
        }
    }

    /// Invalidate the previous frame when the foreground card changes before a new plan.
    pub fn invalidate_planned_video_holes(&mut self) {
        for hole in &self.plan.holes {
            if !self.invalid_video_holes.contains(&hole.source.surface) {
                self.invalid_video_holes.push(hole.source.surface);
            }
        }
    }

    pub fn create_window(&mut self, surface: SurfaceId) -> WindowId {
        if let Some(id) = self.by_surface.get(&surface) {
            return *id;
        }
        self.next_window += 1;
        let id = WindowId(self.next_window);
        self.windows.insert(id, Window::new(id, surface));
        self.by_surface.insert(surface, id);
        id
    }

    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.get(&id)
    }

    pub fn window_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.windows.get_mut(&id)
    }

    pub fn window_of(&self, surface: SurfaceId) -> Option<WindowId> {
        self.by_surface.get(&surface).copied()
    }

    /// The topmost drawn non-video surface under an output point, from the last plan.
    pub fn surface_at(&self, x: i32, y: i32) -> Option<(SurfaceId, Rect)> {
        self.plan
            .items
            .iter()
            .rev()
            .filter(|i| !i.ghost && i.rect.contains(x, y))
            // (a video window is transparent and nobody's: the pointer goes through it to what is under)
            .find(|i| self.windows.get(&i.window).is_none_or(|w| w.kind != WindowType::VideoWindow))
            .map(|i| (i.surface, i.rect))
    }

    /// Where `window`'s picture is now, taken out of its transition: a second press of Home hands the card that was on its way home to
    /// the switcher, which carries it on to its own slot (`Transitions::take_visual`, at the last plan's clock).
    pub fn plan_ns(&self) -> u64 {
        self.plan_ns
    }

    pub fn take_visual(&mut self, window: WindowId) -> Option<Visual> {
        let now = self.plan_ns;
        self.transitions.take_visual(window, now)
    }

    /// Whether the last stack drew LumaTV's Home in the card slot: the shell's Home window is mapped and the card at the front is
    /// SAM's Home (the stub) or there is none.
    pub fn shell_home_shown(&self) -> bool {
        self.home_shown
    }

    /// Whether the moving wallpaper's window is mapped and has a video rectangle: the hole that goes under LumaTV's Home (the caller adds
    /// that Home is drawn: the wallpaper's window is left out of the stack otherwise, and so is its hole). The shell takes its own
    /// wallpaper away only while this holds (without the hole the plane's black would show), and puts it back the frame it stops.
    pub fn wall_hole(&self) -> bool {
        self.holes
            .iter()
            .any(|h| self.by_surface.get(&h.surface).and_then(|w| self.windows.get(w)).is_some_and(|w| w.kind == WindowType::Wallpaper && w.map_serial > 0))
    }

    /// Whether LumaTV's Home, drawn or not, is under a window that covers the whole output (an app's card, LG's Settings, the gate's
    /// X1 probe as an overlay): the client stops drawing it then, as nothing of it can be seen. Builds the stack.
    pub fn shell_home_covered(&mut self, surfaces: &Surfaces) -> bool {
        self.build_stack(surfaces);
        self.home_covered_in(surfaces)
    }

    /// [`Scene::shell_home_covered`] over the stack as it stands (the plan builds it once a frame).
    fn home_covered_in(&self, surfaces: &Surfaces) -> bool {
        if !self.home_rust { return true; }
        self.stack.iter().filter(|(layer, _, _)| *layer >= Layer::Cards).any(|(_, _, id)| {
            self.windows.get(id).is_some_and(|w| {
                w.kind != WindowType::VideoWindow
                    && surfaces.get(w.surface).map(|s| s.size()).is_some_and(|(sw, sh)| sw >= OUTPUT_WIDTH as i32 && sh >= OUTPUT_HEIGHT as i32)
            })
        })
    }

    /// The shell's claim, engage or release (`POST /v1/mask`). Returns whether a frame should show it.
    pub fn mask_op(&mut self, op: Op) -> bool {
        match op {
            Op::Claim(mut m) => {
                let same = self.mask.as_ref().is_some_and(|c| (&c.app, &c.rects, &c.keys, c.hold_ns) == (&m.app, &m.rects, &m.keys, m.hold_ns));
                if !same {
                    if let Some(old) = &self.mask {
                        m.carry(old);
                    }
                    self.mask = Some(m);
                }
                !same
            }
            Op::Engage(ms) => self.mask.as_mut().map(|m| m.engage(ms)).is_some(),
            Op::Release => self.mask.take().is_some() || !self.last_masks.is_empty(),
        }
    }

    /// Whether a press of `code` could start a blanking: a cheap test on the key's way, before its window is looked up.
    pub fn mask_listens(&self, code: u32) -> bool {
        self.mask.as_ref().is_some_and(|m| m.listens(code))
    }

    /// A press of `code` was delivered to a window of `app`: the claim's blanking starts at the next plan.
    pub fn mask_key(&mut self, app: &str, code: u32) -> bool {
        self.mask.as_mut().is_some_and(|m| m.key(app, code))
    }

    /// What `ctl status` says of the shell's claim.
    pub fn mask_report(&self) -> serde_json::Value {
        let now_ns = self.plan_ns;
        self.mask.as_ref().map_or(serde_json::Value::Null, |m| m.report(now_ns))
    }

    /// The Rust shell's Home is on the engine (or no longer is). Returns whether that changed.
    pub fn set_home_rust(&mut self, on: bool) -> bool {
        let changed = self.home_rust != on;
        self.home_rust = on;
        changed
    }

    pub fn home_rust(&self) -> bool {
        self.home_rust
    }

    /// LumaTV's UI answers for LG's guide (or no longer does): a window of that app is left out of the stack, or is drawn as any overlay is. Returns
    /// whether that changed.
    pub fn set_guide_held(&mut self, on: bool) -> bool {
        let changed = self.guide_held != on;
        self.guide_held = on;
        changed
    }

    pub fn guide_held(&self) -> bool {
        self.guide_held
    }

    /// Whether `w` is a window of LG's guide that the stack leaves out.
    fn is_held_guide(&self, w: &Window) -> bool {
        self.guide_held && w.app_id() == LG_GUIDE_APP
    }

    pub fn windows(&self) -> impl Iterator<Item = &Window> {
        self.windows.values()
    }

    pub fn remove_window(&mut self, id: WindowId) -> Option<Window> {
        let w = self.windows.remove(&id)?;
        self.by_surface.remove(&w.surface);
        if !self.zombies.iter().any(|z| z.window == id) {
            self.transitions.forget(id);
        }
        Some(w)
    }

    pub fn remove_surface(&mut self, surface: SurfaceId) {
        if let Some(id) = self.by_surface.get(&surface).copied() {
            self.remove_window(id);
        }
        self.groups.retain(|g| g.owner != surface);
        for g in &mut self.groups {
            for l in g.layers.values_mut() {
                if l.member == Some(surface) {
                    l.member = None;
                }
            }
            g.anonymous.retain(|(s, _)| *s != surface);
        }
        self.holes.retain(|h| h.surface != surface);
        self.video.retain(|s| *s != surface);
    }

    /// The app that owns `surface` exported an element for video (`wl_webos_foreign`), or let go of it: the window's picture is on the video
    /// plane, a hole it may not have yet, so a transition treats it as one.
    pub fn set_video_expected(&mut self, surface: SurfaceId, expected: bool) {
        let at = self.video.iter().position(|s| *s == surface);
        match (expected, at) {
            (true, None) => self.video.push(surface),
            (false, Some(i)) => {
                self.video.swap_remove(i);
            }
            _ => {}
        }
    }

    /// Records that a window's surface got its first buffer (or lost its last). A card that maps comes to the front
    /// (LG's fullscreen container takes every mapping as a fullscreen request); one that unmaps leaves it.
    pub fn set_mapped(&mut self, surface: SurfaceId, mapped: bool) {
        if let Some(id) = self.by_surface.get(&surface).copied()
            && let Some(w) = self.windows.get_mut(&id)
        {
            if mapped {
                self.map_counter += 1;
                w.map_serial = self.map_counter;
                // A group member never comes to the front: it is no card (`Window::in_group`).
                if !w.in_group {
                    w.front = self.map_counter;
                }
            } else {
                w.map_serial = 0;
                w.front = 0;
            }
        }
    }

    /// Whether `w` is at the front of the cards: mapped, or raised by a fullscreen request while it waits for its
    /// first buffer; a window whose surface lost its buffer behind the bookkeeping's back is not.
    fn in_front(w: &Window, surfaces: &Surfaces) -> bool {
        w.kind.is_card() && !w.in_group && w.front > 0 && (w.map_serial == 0 || surfaces.get(w.surface).map(|s| s.is_mapped()).unwrap_or(false))
    }

    /// The foreground card: the card that came to the front last and is not minimized, whether or not it has a
    /// buffer yet (SAM is told, and tells a preloaded app it is foreground, which is when it draws).
    pub fn foreground(&self, surfaces: &Surfaces) -> Option<&Window> {
        self.windows.values().filter(|w| Self::in_front(w, surfaces) && w.state != ShellState::Minimized).max_by_key(|w| w.front)
    }

    /// The foreground's committed subtitle report as drawn by the last scene plan.
    pub fn subtitle_bounds<'a>(&'a self, surfaces: &Surfaces) -> Option<(&'a str, SurfaceId, Option<Rect>)> {
        let front = self.foreground(surfaces)?;
        let mut found = None;
        for item in self.plan.items.iter().filter(|item| item.window == front.id && !item.ghost) {
            let Some(surface) = surfaces.get(item.surface).filter(|s| s.is_mapped()) else { continue };
            let Some(bounds) = surface.current.subtitle_bounds else { continue };
            // Only the foreground root has an independently published identity.
            if item.surface != front.surface {
                return None;
            }
            if item.xform || found.is_some() {
                return None;
            }
            let (quad, clip) = match item.pose {
                Some(pose) if pose.alpha > 0.0 => (pose.quad, pose.clip),
                Some(_) => return None,
                None => (FRect::from_rect(item.rect), FRect::from_rect(item.rect)),
            };
            found = Some((front.app_id(), item.surface, subtitle_rect(bounds, quad, clip)?));
        }
        found
    }


    /// Brings the card `id` to the front: what LG's fullscreen container does for a card that asks for fullscreen,
    /// mapped (a relaunch over another card) or not yet (a preloaded app relaunched). Nothing for a window that is
    /// not a card.
    pub fn raise(&mut self, id: WindowId) {
        if let Some(w) = self.windows.get_mut(&id)
            && w.kind.is_card()
            && !w.in_group
        {
            self.map_counter += 1;
            w.front = self.map_counter;
            if w.map_serial > 0 {
                w.map_serial = self.map_counter;
            }
        }
    }

    /// Makes the window of `surface`, if it has one, a member of a surface group or not (`wl_webos_surface_group.attach` and `detach`).
    /// LG's compositor takes no window attached to a group for a fullscreen card (`StarfishFullscreenController.__allowedFullscreen`):
    /// one that was at the front leaves it (the card it covered comes back), and none is at the front again until it is detached and asks
    /// for fullscreen. Returns whether a card was at the front and is not now: the cards are to be settled again then.
    pub fn set_group_member(&mut self, surface: SurfaceId, member: bool) -> bool {
        let Some(w) = self.by_surface.get(&surface).copied().and_then(|id| self.windows.get_mut(&id)) else { return false };
        let left_the_front = w.kind.is_card() && w.front > 0;
        w.in_group = member;
        w.front = 0;
        left_the_front
    }

    /// The group `surface` is attached to as a member, if it is attached to one.
    pub fn group_of_member(&self, surface: SurfaceId) -> Option<&Group> {
        self.groups.iter().find(|g| g.owner != surface && g.has_member(surface))
    }

    /// The members of the groups `owner` owns, from the lowest (negative z, below the owner) to the highest.
    pub fn group_members(&self, owner: SurfaceId) -> Vec<(i32, SurfaceId)> {
        let mut members: Vec<(i32, SurfaceId)> = self.groups.iter().filter(|g| g.owner == owner).flat_map(Group::member_stack).collect();
        members.sort_unstable();
        members
    }

    /// The card at the front whatever its state, other than `except`: the one that comes back when the foreground
    /// card goes or minimizes itself.
    pub fn front_card(&self, surfaces: &Surfaces, except: Option<WindowId>) -> Option<WindowId> {
        self.windows.values().filter(|w| Self::in_front(w, surfaces) && Some(w.id) != except).max_by_key(|w| w.front).map(|w| w.id)
    }

    /// The window that should hold keyboard focus: the topmost mapped window that takes it (the on-screen keyboard, over the app
    /// whose field it types into, does not).
    pub fn focus(&mut self, surfaces: &Surfaces) -> Option<WindowId> {
        self.build_stack(surfaces);
        let top = self.stack.iter().rev().map(|(_, _, id)| *id).find(|id| self.windows.get(id).is_some_and(|w| w.needs_focus()))?;
        Some(self.group_focus(top, surfaces).unwrap_or(top))
    }

    /// LG's group model (`StarfishGroupModel.updateFocus`): of the members of the group a window owns, the topmost mapped one takes the
    /// keys when it stacks above the owner (z of 0 or more); a member below the owner, or none, leaves them with the owner. A member
    /// that is a bare surface or says `needFocus` false is not one the keys can go to. LG's key ordering (`commit_key_index`) is not
    /// here: the layers' key indexes are kept, not read.
    fn group_focus(&self, owner: WindowId, surfaces: &Surfaces) -> Option<WindowId> {
        let owner_surface = self.windows.get(&owner)?.surface;
        let (z, member) = self
            .groups
            .iter()
            .filter(|g| g.owner == owner_surface)
            .flat_map(Group::member_stack)
            .filter(|(_, m)| surfaces.get(*m).is_some_and(|s| s.is_mapped()))
            .max()?;
        if z < 0 {
            return None;
        }
        let w = self.windows.get(self.by_surface.get(&member)?)?;
        w.needs_focus().then_some(w.id)
    }
    /// Mapped windows in draw order. Native Home occupies the card slot while no app card is in front;
    /// the moving wallpaper and its video hole are drawn only under Home.
    fn build_stack(&mut self, surfaces: &Surfaces) {
        self.stack.clear();
        for w in self.windows.values() {
            if w.map_serial == 0 || w.state == ShellState::Minimized || !surfaces.get(w.surface).map(|s| s.is_mapped()).unwrap_or(false) {
                continue;
            }
            // The native compositor never displays vendor keyboard windows.
            if w.kind == WindowType::Keyboard {
                continue;
            }
            // LG's guide is answered by LumaTV's own: its window is never in a frame, from its first commit (`lg_guide.rs`). It stays mapped for
            // SAM and the apps that watch the foreground list, which see what they see of any overlay.
            if self.is_held_guide(w) {
                continue;
            }
            // A group member is drawn with its owner (`plan`), and a card that is not in front is none of the stack's: a window that left
            // its group while mapped waits for its own `set_state(fullscreen)`.
            if w.in_group || (w.kind.is_card() && w.front == 0) {
                continue;
            }
            // A video window is in the stack only while a claim is granted for it (`vwin`): without one it is a transparent window of nobody's.
            if w.kind == WindowType::VideoWindow && !w.vwin_live {
                continue;
            }
            self.stack.push((w.kind.layer(), w.map_serial, w.id));
        }
        self.stack.sort_unstable();
        // Only the newest card shows: drop every card but the last of the Cards run.
        let mut last_card = None;
        for (i, (layer, _, _)) in self.stack.iter().enumerate() {
            if *layer == Layer::Cards {
                last_card = Some(i);
            }
        }
        let card_is_home = last_card.map(|i| self.windows.get(&self.stack[i].2).is_some_and(|w| w.app_id() == HOME_APP)).unwrap_or(true);
        let has_shell_home = self.home_rust;
        self.home_shown = has_shell_home && card_is_home;
        // What is at the front of the screen, for the transitions: Home, an app's card (the Home stub's own card when LumaTV's Home is not
        // there to take its place), or nothing they move (no card, the screensaver's).
        self.front = match last_card.and_then(|i| self.windows.get(&self.stack[i].2)) {
            _ if self.home_shown => Front::Home,
            Some(w) if w.kind == WindowType::Card => Front::Card(w.id),
            _ => Front::None,
        };
        // A full-screen overlay window that is an app's (LG's Settings): it is what the screen shows, over Home or over a card.
        let covering = self
            .stack
            .iter()
            .rev()
            .find(|(layer, _, id)| {
                *layer == Layer::Overlays && self.windows.get(id).is_some_and(|w| surfaces.get(w.surface).is_some_and(|s| Self::is_app_overlay(w, s)))
            })
            .map(|(_, _, id)| *id);
        if let Some(id) = covering {
            self.front = Front::Card(id);
        }
        let drop_card = self.home_shown;
        let mut i = 0;
        self.stack.retain(|(layer, _, _)| {
            let keep_it = match layer {
                Layer::Cards => last_card == Some(i) && !drop_card,
                Layer::Wallpaper => has_shell_home && card_is_home,
                _ => true,
            };
            i += 1;
            keep_it
        });
    }

    /// Members of the groups `owner` owns, sorted by z, into `self.members` (negative z below the owner).
    fn build_members(&mut self, owner: SurfaceId) {
        self.members.clear();
        self.members.extend(self.groups.iter().filter(|g| g.owner == owner).flat_map(Group::member_stack));
        self.members.sort_unstable();
    }

    /// Whether an overlay window is an app on the screen: it covers the output and is opaque (or one of LG's known apps that map so).
    fn is_app_overlay(w: &Window, s: &crate::surface::Surface) -> bool {
        let (sw, sh) = s.size();
        if sw < OUTPUT_WIDTH as i32 || sh < OUTPUT_HEIGHT as i32 {
            return false;
        }
        let opaque = s.current.opaque.as_ref().is_some_and(|o| o.iter().any(|r| r.x <= 0 && r.y <= 0 && r.right() >= sw && r.bottom() >= sh))
            || s.current.buffer.as_ref().is_some_and(|b| crate::buffer::kind_of(b).is_opaque_format());
        opaque || FULLSCREEN_OVERLAYS.contains(&w.app_id())
    }

    /// A window at the front of the screen is being destroyed: when a transition would take it away, its last picture is kept to draw (the
    /// buffer's texture stays while the plan has it). Call before the surface is removed.
    pub fn zombify(&mut self, surface: SurfaceId, surfaces: &Surfaces) {
        if !self.transitions.enabled() {
            return;
        }
        let Some(wid) = self.by_surface.get(&surface).copied() else { return };
        if self.front != Front::Card(wid) {
            return;
        }
        let (Some(w), Some(s)) = (self.windows.get(&wid), surfaces.get(surface)) else { return };
        // (a buffer the client destroyed before its window still has its import in the renderer, which keeps drawing it: `Gl::prune`)
        let Some(buffer) = s.current.buffer.clone() else { return };
        let origin = if w.kind.is_card() { (0, 0) } else { w.position };
        let rect = s.rect_at(origin.0, origin.1);
        if rect.is_empty() {
            return;
        }
        let video = self.video.contains(&surface) || self.holes.iter().any(|h| h.surface == surface);
        self.zombies.retain(|z| z.window != wid);
        self.zombies.push(Zombie {
            window: wid,
            app_id: w.app_id().to_string(),
            video,
            layer: w.kind.layer(),
            serial: w.map_serial,
            surface,
            buffer,
            rect,
            y_invert: s.current.y_invert,
            since_ns: self.plan_ns,
        });
    }

    /// Builds this frame's plan at the clock now.
    pub fn plan(&mut self, surfaces: &Surfaces) -> &Plan {
        self.plan_at(surfaces, crate::output::monotonic_ns())
    }

    /// The window of a transition whose track poses it now, with the rectangle its pose is relative to; `None` at rest (the window is
    /// drawn as it is).
    fn pose_of(&self, window: WindowId, now_ns: u64) -> Option<(Pose, FRect)> {
        let t = self.transitions.track_of(window)?;
        let pose = t.pose_at(now_ns);
        if pose.is_rest_on(&t.window_rect) { None } else { Some((pose, t.window_rect)) }
    }

    /// A destroyed window's last picture, through its pose.
    fn push_zombie(&mut self, window: WindowId, (pose, win): (Pose, FRect)) {
        let Some(z) = self.zombies.iter().find(|z| z.window == window) else { return };
        let (buffer, rect, y_invert, surface) = (z.buffer.clone(), z.rect, z.y_invert, z.surface);
        let posed = Posed { quad: FRect::map_rect(&win, &pose.rect, &rect), clip: pose.clip, radius: pose.radius, alpha: pose.alpha, shade: pose.shade };
        let drawn = posed.bounds().intersect(&Rect::new(0, 0, OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32));
        if drawn.is_empty() {
            return;
        }
        self.plan.items.push(Item {
            window,
            surface,
            buffer,
            rect,
            damage: None,
            opaque: false,
            y_invert,
            dim: 1.0,
            pose: Some(posed),
            ghost: true,
            gain: false,
            drift: (0, 0),
            xform: false,
        });
    }

    /// Looks at what is at the front of the screen against the last plan's: a change starts the transition for it. Then the tracks that
    /// have ended go.
    fn step_transitions(&mut self, surfaces: &Surfaces, now_ns: u64) {
        if self.front != self.last_front {
            let (prev, now) = (self.last_front, self.front);
            self.last_front = now;
            self.begin_transition(prev, now, surfaces, now_ns);
        }
        self.transitions.anchor(now_ns);
        self.transitions.frame(now_ns);
    }

    fn begin_transition(&mut self, prev: Front, now: Front, surfaces: &Surfaces, now_ns: u64) {
        if !self.transitions.enabled() {
            return;
        }
        let full = FRect::new(0.0, 0.0, OUTPUT_WIDTH as f32, OUTPUT_HEIGHT as f32);
        // Something to draw for the window: it is mapped, or it was destroyed at the front and left its picture.
        let drawable = |scene: &Scene, id: WindowId| {
            scene.windows.get(&id).is_some_and(|w| surfaces.get(w.surface).is_some_and(|s| s.is_mapped())) || scene.zombies.iter().any(|z| z.window == id)
        };
        // (app id, expects video) of a window
        let info = |scene: &Scene, id: WindowId| -> (String, bool) {
            match (scene.windows.get(&id), scene.zombies.iter().find(|z| z.window == id)) {
                (Some(w), _) => (w.app_id().to_string(), scene.video.contains(&w.surface) || scene.holes.iter().any(|h| h.surface == w.surface)),
                (None, Some(z)) => (z.app_id.clone(), z.video),
                _ => (String::new(), false),
            }
        };
        // Home's windows: under a launch Home and the moving wallpaper's window (its hole) both stay drawn; Home alone comes back on a leave.
        let mut home: Vec<(WindowId, FRect)> = self
            .windows
            .values()
            .filter(|w| w.kind == WindowType::Wallpaper && w.map_serial > 0)
            .map(|w| (w.id, full))
            .collect();
        let mut home_ui: Vec<(WindowId, FRect)> = Vec::new();
        if self.home_rust {
            home.push((HOME_RUST, full));
            home_ui.push((HOME_RUST, full));
        }
        let started = match (prev, now) {
            (Front::Home, Front::Card(b)) if drawable(self, b) => {
                let b_info = info(self, b);
                self.transitions.begin(Kind::Launch, now_ns, Some(party(b, &b_info, full)), None, &home)
            }
            (Front::Card(a), Front::Home) if drawable(self, a) => {
                let a_info = info(self, a);
                self.transitions.begin(Kind::Leave, now_ns, None, Some(party(a, &a_info, full)), &home_ui)
            }
            // One card over another that was drawn under it already (an overlay goes, the card it was over is the screen): the card that
            // goes leaves, what was under it is revealed, as Home is.
            (Front::Card(a), Front::Card(b)) if a != b && drawable(self, a) && self.prev_stack.contains(&b) => {
                let mut a_info = info(self, a);
                a_info.0.clear();
                self.transitions.begin(Kind::Leave, now_ns, None, Some(party(a, &a_info, full)), &[(b, full)])
            }
            (Front::Card(a), Front::Card(b)) if a != b && drawable(self, b) => {
                let (a_info, b_info) = (info(self, a), info(self, b));
                let from = drawable(self, a).then(|| party(a, &a_info, full));
                self.transitions.begin(Kind::Switch, now_ns, Some(party(b, &b_info, full)), from, &[])
            }
            _ => false,
        };
        if !started && prev != now {
            self.transitions.stats.skipped += 1;
        }
    }

    /// This frame's draw order: the stack, and the windows a transition draws that are not in it (a card that left, Home under a card
    /// that arrives), each at its own place; Home dissolving in over a card whose picture is on the video plane goes above the cards.
    fn merge_draw_list(&mut self, surfaces: &Surfaces) {
        self.draw.clear();
        self.draw.extend_from_slice(&self.stack);
        if !self.transitions.active() {
            return;
        }
        for t in self.transitions.tracks() {
            let Some(w) = self.windows.get(&t.window) else {
                if let Some(z) = self.zombies.iter().find(|z| z.window == t.window) {
                    self.draw.push((z.layer, z.serial, z.window));
                }
                continue;
            };
            match self.draw.iter().position(|(_, _, id)| *id == t.window) {
                Some(i) => {
                    if t.on_top {
                        self.draw[i] = (Layer::Cards, u64::MAX, t.window);
                    }
                }
                None => {
                    if t.ghost && surfaces.get(w.surface).is_some_and(|s| s.is_mapped()) {
                        self.draw.push((w.kind.layer(), w.map_serial, w.id));
                    }
                }
            }
        }
        self.draw.sort_unstable();
    }

    /// The card in front as the OLED care's app features see it (`oledcare::AppView`): the app's own card (not Home's stub, not a covering
    /// overlay), while either feature is on. `busy` is a video's hole on any surface of the card's group, or a transition on the screen;
    /// `changed` that a sample of the box dim's grid found the app's picture changed over a [`crate::oledcare::CHANGE_FRACTION`] of the
    /// output since the last look (`boxdim::BoxDim::take_app_change`): a commit of the same picture again is not a change.
    fn app_view(&mut self) -> Option<crate::oledcare::AppView> {
        // Taken whether or not there is a card to look at: a change seen under another window is not this card's.
        let changed = self.boxdim.take_app_change();
        if !self.care.wants_app() {
            return None;
        }
        let Front::Card(wid) = self.front else { return None };
        let w = self.windows.get(&wid)?;
        if w.kind != WindowType::Card || w.app_id() == HOME_APP || !self.stack.iter().any(|(_, _, id)| *id == wid) {
            return None;
        }
        let owner = w.surface;
        self.build_members(owner);
        let mut busy = self.transitions.active() || self.holes.iter().any(|h| h.surface == owner);
        for k in 0..self.members.len() {
            busy |= self.holes.iter().any(|h| h.surface == self.members[k].1);
        }
        Some(crate::oledcare::AppView { window: wid.0, changed, busy })
    }

    /// [`Scene::plan`] at a given time (CLOCK_MONOTONIC ns), for the OLED care's beats.
    pub fn plan_at(&mut self, surfaces: &Surfaces, now_ns: u64) -> &Plan {
        let output = Rect::new(0, 0, OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32);
        self.plan.clear();
        self.invalid_video_holes.clear();
        if self.plan.tiles.is_none() {
            self.plan.tiles = Some(Tiles::new(OUTPUT_WIDTH, OUTPUT_HEIGHT));
        }
        self.plan_ns = now_ns;
        self.build_stack(surfaces);
        // The OLED care looks at the frame before it is laid out: which shell windows are on it and whether any window has a commit
        // to draw (a frame that is drawn anyway is the one a beat rides on).
        let mut views = [None; crate::oledcare::LAYERS];
        let mut pending = false;
        for (_, _, wid) in &self.stack {
            let Some(w) = self.windows.get(wid) else { continue };
            pending |= w.commits > w.commits_drawn;
        }
        // Native Home is not a window; its clock commits do not reset the idle timer.
        if self.home_rust && self.home_shown {
            views[0] = Some(0);
        }
        self.care.step(now_ns, views, pending);
        let home_dim = if self.home_rust && self.home_shown && !self.care.saver { 1.0 - self.care.dim(0) } else { 0.0 };
        if self.plan.home_dim != home_dim {
            self.plan.home_dim = home_dim;
            self.plan.damage = self.plan.damage.union(&output);
            self.plan.tiles.as_mut().unwrap().add(&output);
        }
        let app = self.app_view();
        self.care.step_app(now_ns, app, pending);
        self.care_only = self.care.beat_frame() && !pending;
        // The card the app features look at, and where and how bright they draw it (the origin and 1 unless it is engaged).
        let app_card = app.map(|a| WindowId(a.window));
        let (app_shift, app_dim) = (self.care.app_offset(), self.care.app_dim());
        self.step_transitions(surfaces, now_ns);
        self.merge_draw_list(surfaces);
        let n = self.draw.len();
        for i in 0..n {
            let wid = self.draw[i].2;
            let Some(w) = self.windows.get(&wid) else {
                if let Some(pose) = self.pose_of(wid, now_ns) {
                    self.push_zombie(wid, pose);
                } else if let Some(z) = self.zombies.iter().find(|z| z.window == wid) {
                    let (buffer, rect, y_invert, surface) = (z.buffer.clone(), z.rect, z.y_invert, z.surface);
                    self.plan.items.push(Item {
                        window: wid,
                        surface,
                        buffer,
                        rect,
                        damage: None,
                        opaque: false,
                        y_invert,
                        dim: 1.0,
                        pose: None,
                        ghost: true,
                        gain: false,
                        drift: (0, 0),
                        xform: false,
                    });
                }
                continue;
            };
            let mut dim = 1.0;
            let mut drift = (0, 0);
            let origin = if w.kind.is_card() {
                if app_card == Some(wid) {
                    dim = app_dim;
                    drift = app_shift;
                    app_shift
                } else {
                    (0, 0)
                }
            } else if w.kind == WindowType::Keyboard {
                // LG's KeyboardView: anchored at the bottom of the output, as high as the window, the window centred in that width; the
                // rectangle the app asked for when it asked for one.
                let g = surfaces.get(w.surface).map(|s| keyboard_geometry(s.size(), s.current.input.as_deref())).unwrap_or_default();
                keyboard_origin(g, self.keyboard_pref)
            } else {
                w.position
            };
            let owner = w.surface;
            let pose = self.pose_of(wid, now_ns).or_else(|| texture_pose(w, surfaces));
            let ghost = self.transitions.active() && !self.stack.iter().any(|(_, _, id)| *id == wid);
            let gain = !matches!(w.kind, WindowType::Wallpaper | WindowType::VideoWindow);
            let look = Look { window: wid, origin, dim, pose, ghost, under_hole: false, gain, drift };
            self.build_members(owner);
            let m = self.members.len();
            // The members below a window whose hole covers them (Live TV's Dmost and its other layers under the video) are under the plane's
            // picture: the hole is cleared (or the veil lies opaque) over them, so drawing them is a full-screen layer for nothing, and their
            // commits would draw frames that show nothing new. A window in transition is posed, not at rest: it draws them as it did.
            self.hole_rects.clear();
            if m > 0 && pose.is_none() {
                self.window_holes(surfaces, owner, origin);
            }
            let below = Look { under_hole: !self.hole_rects.is_empty(), ..look };
            for k in 0..m {
                let (z, s) = self.members[k];
                if z < 0 {
                    self.push_tree(surfaces, s, below);
                }
            }
            self.push_tree(surfaces, owner, look);
            for k in 0..m {
                let (z, s) = self.members[k];
                if z >= 0 {
                    self.push_tree(surfaces, s, look);
                }
            }
        }
        self.plan.home_before = self.plan.items.iter().take_while(|i| {
            self.windows.get(&i.window).is_some_and(|w| w.kind == WindowType::Wallpaper)
        }).count();
        for (_, _, wid) in &self.draw {
            if let Some(w) = self.windows.get_mut(wid) {
                w.commits_drawn = w.commits;
            }
        }
        self.plan.video = self.plan.items.iter().any(|i| self.video.contains(&i.surface));
        // Home is hidden for the frames in which an app's card has covered it and nothing is in transition (a launch and a return draw it as the
        // underlay, and a frame that finds it back draws it whole: `Gl::set_home_hidden`).
        self.plan.home_hidden = self.home_rust && !self.transitions.active() && self.home_covered_in(surfaces);
        self.plan.animating = self.transitions.animating(now_ns);
        for h in &self.holes {
            let Some(before) = self.plan.items.iter().position(|i| i.surface == h.surface) else { continue };
            let item = &self.plan.items[before];
            let logical = h.rect.translated(item.rect.x, item.rect.y);
            // An arriving card whose picture is on the video plane has a veil under it (`transition.rs`): a flat fill where the hole is,
            // moved and faded with the card, that fades away over the plane once the video has had time.
            let veil = self.transitions.veil(item.window, now_ns, true);
            let planned = match (veil, item.pose) {
                (None, _) => {
                    let r = logical.intersect(&output);
                    (!r.is_empty()).then_some(PlannedHole { source: *h, rect: r, before, veil: None })
                }
                (Some(v), pose) => {
                    let (quad, clip, radius, alpha) = match pose {
                        Some(p) => {
                            let (sx, sy) = (p.quad.w / item.rect.w.max(1) as f32, p.quad.h / item.rect.h.max(1) as f32);
                            let quad = FRect::new(
                                p.quad.x + (logical.x - item.rect.x) as f32 * sx,
                                p.quad.y + (logical.y - item.rect.y) as f32 * sy,
                                logical.w as f32 * sx,
                                logical.h as f32 * sy,
                            );
                            (quad, p.clip, p.radius, p.alpha * v.alpha)
                        }
                        None => (FRect::from_rect(logical), FRect::from_rect(logical), 0.0, v.alpha),
                    };
                    let r = quad.bounds().intersect(&clip.bounds()).intersect(&output);
                    (!r.is_empty()).then_some(PlannedHole {
                        source: *h,
                        rect: r,
                        before,
                        veil: Some(HoleVeil { quad, clip, radius, alpha, clear: pose.is_none() && v.alpha < 1.0 }),
                    })
                }
            };
            if let Some(hole) = planned {
                self.plan.holes.push(hole);
            }
        }
        // A veil that ended in this frame's holes ends its track with it (and one whose card has no hole runs its cap here); a zombie goes with
        // its track.
        self.transitions.idle_veils(now_ns);
        self.transitions.sweep(now_ns);
        if !self.zombies.is_empty() {
            let t = &self.transitions;
            self.zombies.retain(|z| t.track_of(z.window).is_some() && now_ns.saturating_sub(z.since_ns) < ZOMBIE_NS);
        }
        self.prev_stack.clear();
        self.prev_stack.extend(self.stack.iter().map(|(_, _, id)| *id));
        // Whatever moved, appeared, restacked or vanished is drawn again in full, old place and new. A request that
        // changes nothing on the screen (a window made, named or given a state before its first buffer) draws no
        // frame: on every gate the full redraw of the same picture at X1's window creation flipped 14-15 ms before
        // X1's first commit, and that frame's flip two refreshes later was logged as a skipped refresh (33.37 ms).
        self.placed.clear();
        self.placed.extend(self.plan.items.iter().map(|i| Placed {
            surface: i.surface,
            rect: i.drawn_rect(),
            opaque: i.opaque,
            y_invert: i.y_invert,
            dim: (i.dim * 100.0).round() as u8,
            pose: i.pose.map(|p| p.key()),
        }));
        // The box dim (`boxdim`): which cells of the app's pictures may have changed this frame (their own damage, before a redraw of
        // everything below wipes it), whether the layout changed, and the map's step. A step damages the cells it changed, the items keep
        // their own. Nothing of it runs while neither it nor an app feature is on.
        let boxdim_settings = self.care.settings.boxdim;
        let track = boxdim_settings.on || self.care.wants_app();
        self.boxdim.begin();
        if track {
            for i in &self.plan.items {
                if i.gain && !i.ghost && i.pose.is_none() {
                    self.boxdim.seen(now_ns, i.rect, i.drift, i.damage.map(|(a, b)| &self.plan.damage_rects[a..b]));
                }
            }
        }
        self.boxdim.app_engaged = self.care.app_engaged();
        let blocked = if self.boxdim.capturing {
            Some(crate::boxdim::Restart::Capture)
        } else if self.care.saver {
            Some(crate::boxdim::Restart::Saver)
        } else if self.transitions.active() {
            Some(crate::boxdim::Restart::Transition)
        } else {
            None
        };
        let layout_changed = !same_layout(&self.placed, &self.last_placed) || self.plan.holes != self.last_holes;
        if let Some(r) = self.boxdim.step(now_ns, &boxdim_settings, track, blocked, layout_changed) {
            let r = r.intersect(&output);
            self.plan.damage = self.plan.damage.union(&r);
            self.plan.tiles.as_mut().unwrap().add(&r);
            self.care_only |= !pending;
        }
        match self.boxdim.map() {
            Some((map, serial)) => {
                if serial != self.plan.gain_serial || !self.plan.gain_on {
                    self.plan.gain.cells = map.cells;
                    self.plan.gain_serial = serial;
                }
                self.plan.gain_on = true;
            }
            None => self.plan.gain_on = false,
        }
        // What changed on the screen is what is drawn again, old place and new: a window that came, went, moved, changed its pose or its dim, and
        // a hole that came, went or moved. The windows that stayed as they were are not: a toast over a full-screen video owes the toast's
        // rectangle, not the screen (the whole page is the hole, and redrawing it per refresh is the work this exists to avoid). A restack with
        // the same windows changes what overlaps, wherever it does, and `redraw_all` asks for everything: those owe all of it.
        let everything = std::mem::take(&mut self.redraw_all) || (self.placed != self.last_placed && only_restacked(&self.last_placed, &self.placed));
        let holes_changed = self.plan.holes != self.last_holes;
        if everything || holes_changed || self.placed != self.last_placed {
            let tiles = self.plan.tiles.as_mut().unwrap();
            let (last, now) = (&self.last_placed, &self.placed);
            let stale = last.iter().filter(|p| everything || !now.contains(p));
            let fresh = now.iter().filter(|p| everything || !last.contains(p));
            let holes = (everything || holes_changed).then(|| self.last_holes.iter().chain(self.plan.holes.iter()).map(|h| h.rect)).into_iter().flatten();
            let mut touched = Vec::new();
            for r in stale.chain(fresh).map(|p| p.rect).chain(holes) {
                self.plan.damage = self.plan.damage.union(&r);
                tiles.add(&r);
                touched.push(r);
            }
            // their pictures are uploaded whole again; a window that stayed keeps what its own commits damaged
            for i in &mut self.plan.items {
                if everything || touched.iter().any(|r| !r.intersect(&i.rect).is_empty()) {
                    i.damage = None;
                }
            }
        }
        std::mem::swap(&mut self.last_placed, &mut self.placed);
        self.last_holes.clear();
        self.last_holes.extend_from_slice(&self.plan.holes);
        // The sprite sits above everything: where it was and where it is are redrawn when it moved or hid.
        self.plan.cursor = self.cursor;
        if self.cursor != self.last_cursor {
            let tiles = self.plan.tiles.as_mut().unwrap();
            for r in self.last_cursor.iter().chain(self.cursor.iter()) {
                let r = r.intersect(&output);
                if !r.is_empty() {
                    self.plan.damage = self.plan.damage.union(&r);
                    tiles.add(&r);
                }
            }
            self.last_cursor = self.cursor;
        }
        self.plan_masks(&output, now_ns);
        self.plan_backdrop(surfaces);
        self.plan.damage = self.plan.damage.intersect(&output);
        &self.plan
    }

    /// Where the claim blanks in this plan: the rectangles on the item of the claimed app's window (not one in a transition, which is posed and
    /// not drawn where its picture is), and the pixels the blanking moved from or to drawn again.
    fn plan_masks(&mut self, output: &Rect, now_ns: u64) {
        if let Some(m) = self.mask.as_mut() {
            m.step(now_ns);
            if m.active(now_ns) {
                for item in self.plan.items.iter().filter(|i| i.pose.is_none() && !i.ghost) {
                    let Some(w) = self.windows.get(&item.window).filter(|w| w.app_id() == m.app && w.surface == item.surface) else { continue };
                    let mut blank = PlannedMask { window: w.id, surface: item.surface, rects: [Rect::default(); MAX_RECTS], len: 0 };
                    for r in &m.rects {
                        let r = r.translated(item.rect.x, item.rect.y).intersect(&item.rect).intersect(output);
                        if !r.is_empty() {
                            blank.rects[blank.len] = r;
                            blank.len += 1;
                        }
                    }
                    if blank.len > 0 {
                        self.plan.masks.push(blank);
                    }
                }
            }
        }
        if self.plan.masks != self.last_masks {
            let tiles = self.plan.tiles.as_mut().expect("tiles are made at the start of a plan");
            for r in self.last_masks.iter().chain(&self.plan.masks).flat_map(PlannedMask::rects) {
                self.plan.damage = self.plan.damage.union(r);
                tiles.add(r);
            }
            self.last_masks.clone_from(&self.plan.masks);
        }
    }

    /// The last plan, as built by [`Scene::plan`].
    pub fn last_plan(&self) -> &Plan {
        &self.plan
    }

    /// Whether the last plan was drawn for an OLED-care beat alone, with no window's commit in it: the frame is owed nothing after it, so
    /// the output may take the screen for at rest sooner than after a frame that follows an event (`Output::quiet_frame`).
    pub fn care_frame(&self) -> bool {
        self.care_only
    }

    /// A key or the pointer: the OLED care's dims and shifts come back (`oledcare`) and the box dim's grid starts over (`boxdim`).
    pub fn input(&mut self, now_ns: u64) {
        self.care.input(now_ns);
        self.boxdim.input(now_ns);
    }

    /// Damages everything the next plan covers, whatever changed.
    pub fn redraw_all(&mut self) {
        self.redraw_all = true;
    }

    /// Into `hole_rects`: the holes of `owner`'s window as the plan below will make them (the logical rectangle on the window's picture, in
    /// output pixels). None while the window has no picture to put a hole in: no hole is planned then, and nothing is covered.
    fn window_holes(&mut self, surfaces: &Surfaces, owner: SurfaceId, origin: (i32, i32)) {
        let Some(s) = surfaces.get(owner) else { return };
        if s.current.buffer.is_none() {
            return;
        }
        let at = s.rect_at(origin.0, origin.1);
        if at.is_empty() {
            return;
        }
        let output = Rect::new(0, 0, OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32);
        for h in self.holes.iter().filter(|h| h.surface == owner) {
            let r = h.rect.translated(at.x, at.y).intersect(&output);
            if !r.is_empty() {
                self.hole_rects.push(r);
            }
        }
    }

    fn push_tree(&mut self, surfaces: &Surfaces, root: SurfaceId, look: Look) {
        let Look { window, origin, dim, pose, ghost, under_hole, gain, drift } = look;
        surfaces.tree_in_draw_order_into(root, &mut self.tree);
        let n = self.tree.len();
        for k in 0..n {
            let (sid, dx, dy) = self.tree[k];
            let Some(s) = surfaces.get(sid) else { continue };
            let Some(buffer) = s.current.buffer.clone() else { continue };
            if matches!(s.role, Role::Exported | Role::Cursor) {
                continue;
            }
            let rect = s.rect_at(origin.0 + dx, origin.1 + dy);
            if rect.is_empty() {
                continue;
            }
            if under_hole && self.hole_rects.iter().any(|h| rect.intersect(h) == rect) {
                continue;
            }
            // In a transition the item is drawn through its window's pose: its own picture goes where the pose moves it, behind the window's
            // mask. What it draws on is what is damaged when it has anything new to show (a pose that changed is `Placed`'s business).
            let posed =
                pose.map(|(p, win)| Posed { quad: FRect::map_rect(&win, &p.rect, &rect), clip: p.clip, radius: p.radius, alpha: p.alpha, shade: p.shade });
            let tiles = self.plan.tiles.as_mut().unwrap();
            let damage = if let Some(p) = &posed {
                let drawn = p.bounds().intersect(&Rect::new(0, 0, OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32));
                if drawn.is_empty() {
                    continue;
                }
                if s.current.damage.as_ref().is_none_or(|d| !d.is_empty()) {
                    self.plan.damage = self.plan.damage.union(&drawn);
                    tiles.add(&drawn);
                }
                None
            } else {
                match &s.current.damage {
                    None => {
                        self.plan.damage = self.plan.damage.union(&rect);
                        tiles.add(&rect);
                        None
                    }
                    Some(d) => {
                        let start = self.plan.damage_rects.len();
                        for r in d {
                            let r = r.translated(rect.x, rect.y).intersect(&rect);
                            if !r.is_empty() {
                                self.plan.damage = self.plan.damage.union(&r);
                                tiles.add(&r);
                                self.plan.damage_rects.push(r);
                            }
                        }
                        Some((start, self.plan.damage_rects.len()))
                    }
                }
            };
            let opaque = posed.is_none()
                && (s.current
                        .opaque
                        .as_ref()
                        .map(|o| o.iter().any(|r| r.x <= 0 && r.y <= 0 && r.right() >= rect.w && r.bottom() >= rect.h))
                        .unwrap_or(false)
                    || crate::buffer::kind_of(&buffer).is_opaque_format());
            let y_invert = s.current.y_invert;
            if let Some(c) = s.current.content_ns {
                self.plan.content_ns = Some(self.plan.content_ns.map_or(c, |p| p.min(c)));
            }
            let xform = s.current.transform != 0 || s.current.scale != 1;
            self.plan.items.push(Item { window, surface: sid, buffer, rect, damage, opaque, y_invert, dim, pose: posed, ghost, gain, drift, xform });
        }
    }
}

impl crate::buffer::BufferKindRef<'_> {
    /// Whether the pixel format has no alpha (XRGB/XBGR).
    pub fn is_opaque_format(&self) -> bool {
        match self {
            crate::buffer::BufferKindRef::Shm(s) => s.format == 1,
            crate::buffer::BufferKindRef::Dmabuf(d) => matches!(d.format, crate::buffer::fourcc::XRGB8888 | crate::buffer::fourcc::XBGR8888),
            crate::buffer::BufferKindRef::External => false,
        }
    }
}

#[allow(dead_code)]
fn _assert_types(_: WindowType) {}

#[cfg(test)]
mod subtitle_tests {
    use super::*;
    use crate::surface::SubtitleBounds;

    #[test]
    fn canvas_is_scaled_clipped_and_rounded_outward() {
        let bounds = SubtitleBounds { canvas_width: 100, canvas_height: 100, rect: Rect::new(10, 20, 60, 40) };
        let quad = FRect::new(-10.0, 30.0, 101.0, 200.0);
        let clip = FRect::new(20.0, 0.0, 40.0, 100.0);
        assert_eq!(subtitle_rect(bounds, quad, clip), Some(Some(Rect::new(20, 70, 40, 30))));
        assert_eq!(subtitle_rect(bounds, quad, FRect::new(200.0, 0.0, 20.0, 20.0)), Some(None));
        let empty = SubtitleBounds { rect: Rect::new(0, 0, 0, 0), ..bounds };
        assert_eq!(subtitle_rect(empty, quad, clip), Some(None));
        assert_eq!(subtitle_rect(bounds, FRect::new(0.0, 0.0, 0.0, 1.0), clip), None);
    }
}

