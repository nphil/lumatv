//! The model: anchors and offsets, the zones, the registry of items, what the owner placed, and where every item is as a rectangle.
//!
//! An item is placed by an anchor and an offset, never by pixels: `{"anchor": "top-right", "dx": 0, "dy": 0}`. The anchor is one of nine
//! places on the item's own zone (a corner, the middle of an edge, the middle) and the offsets are fractions of the room the item has left
//! in that zone (the zone less its own size). So an item flush in a corner is flush whatever its size, at any text size, in any toolkit, and
//! can never leave its zone. `layout.overlays.<id>` in `theme.json` holds what the owner placed; an item he never placed is at its default,
//! which is where it was drawn before the owner could place it.
//!
//! The zones are the rules the overlays already kept: nothing over Home's status bar's row, nothing in the bottom quarter (the film's
//! subtitles and the volume bar keep it), and the right and left edges are Home's own (`layout.json` `safe`), so a card at the top right
//! ends where the status bar ends.

use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::sync::LazyLock;

/// The screen, in the pixels every overlay is laid out in.
pub const W: i32 = 1920;
pub const H: i32 = 1080;

/// Home's own numbers (`layout.json` `safe` and `statusBar`) arrive in [`Frame::from_json`]; these are the fallbacks when Home has not
/// said, and held equal to the shipped `layout.json` by a test.
pub const SAFE_X: i32 = 64;
pub const SAFE_Y: i32 = 54;
pub const BAR: i32 = 60;
pub const BAR_GAP: i32 = 20;
/// No overlay card ends below this: three quarters of 1080 (theme `overlays.limit`).
pub const LIMIT: i32 = 810;
/// The volume bar's rest above the bottom edge over Home.
pub const REST: i32 = 64;
/// The toasts' own inset from the screen's edges; the QML shell's toasts have always been drawn this far in.
pub const TOAST: i32 = 24;
/// The mute mark: the TV's smallest target.
pub const MARK: i32 = 64;
/// The mark moves this far, four places of a square, so a panel never holds one still mark.
pub const DRIFT: i32 = 48;
/// Over an app the bar on the bottom edge rides this much higher, clear of the subtitles.
pub const LIFT: i32 = 120;
/// The least air between two items.
pub const GAP: i32 = 8;
/// Between the camera window and the cards under it (theme `overlays.gap`).
pub const STACK: i32 = 16;
/// Where the film's subtitles are (Netflix: they end about 70 px above the bottom edge and fill 105 px) and the air the lifted bar keeps above them.
pub const SUBS: Subs = Subs {
    top: 905,
    bottom: 1010,
    clear: 9,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Subs {
    pub top: i32,
    pub bottom: i32,
    pub clear: i32,
}

/// A rectangle in whole pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }
    /// From the edges, never negative in size.
    pub fn from_edges(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        Self::new(x0, y0, (x1 - x0).max(0), (y1 - y0).max(0))
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    /// `{"x", "y", "w", "h"}`, whole pixels.
    pub fn from_json(v: &Value) -> Option<Rect> {
        let n = |k: &str| {
            v.get(k)
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite() && n.abs() <= 8192.0)
                .map(round)
        };
        Some(Rect::new(n("x")?, n("y")?, n("w")?, n("h")?))
    }

    /// Whether the two are closer than `gap` (or overlap).
    pub fn meets(&self, other: &Rect, gap: i32) -> bool {
        self.x < other.x + other.w + gap
            && other.x < self.x + self.w + gap
            && self.y < other.y + other.h + gap
            && other.y < self.y + self.h + gap
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Size {
    pub w: i32,
    pub h: i32,
}

impl Size {
    pub const fn new(w: i32, h: i32) -> Self {
        Self { w, h }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// JavaScript's `Math.round`, which the placements in files written by the QML shell were made with: halves go up.
pub fn round(v: f64) -> i32 {
    (v + 0.5).floor() as i32
}

pub fn round4(v: f64) -> f64 {
    (v * 10000.0 + 0.5).floor() / 10000.0
}

/// `max(lo, min(hi, v))`: with an item larger than its room (`lo > hi`) the result is `lo`, where `f64::clamp` would panic.
pub fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    lo.max(hi.min(v))
}

pub fn clamp_i(v: i32, lo: i32, hi: i32) -> i32 {
    lo.max(hi.min(v))
}

// ---- anchors and placements ----------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    /// In the order a nearest-anchor search visits them, which settles ties.
    pub const ALL: [Anchor; 9] = [
        Anchor::TopLeft,
        Anchor::Top,
        Anchor::TopRight,
        Anchor::Left,
        Anchor::Center,
        Anchor::Right,
        Anchor::BottomLeft,
        Anchor::Bottom,
        Anchor::BottomRight,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Anchor::TopLeft => "top-left",
            Anchor::Top => "top",
            Anchor::TopRight => "top-right",
            Anchor::Left => "left",
            Anchor::Center => "center",
            Anchor::Right => "right",
            Anchor::BottomLeft => "bottom-left",
            Anchor::Bottom => "bottom",
            Anchor::BottomRight => "bottom-right",
        }
    }

    pub fn parse(name: &str) -> Option<Anchor> {
        Anchor::ALL.into_iter().find(|a| a.name() == name)
    }

    /// How far across the room the anchor is, from the left and from the top.
    pub fn fractions(self) -> (f64, f64) {
        match self {
            Anchor::TopLeft => (0.0, 0.0),
            Anchor::Top => (0.5, 0.0),
            Anchor::TopRight => (1.0, 0.0),
            Anchor::Left => (0.0, 0.5),
            Anchor::Center => (0.5, 0.5),
            Anchor::Right => (1.0, 0.5),
            Anchor::BottomLeft => (0.0, 1.0),
            Anchor::Bottom => (0.5, 1.0),
            Anchor::BottomRight => (1.0, 1.0),
        }
    }
}

impl Serialize for Anchor {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.name())
    }
}

/// An anchor and the offsets from it, as fractions of the room the item has left in its zone.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Placement {
    pub anchor: Anchor,
    pub dx: f64,
    pub dy: f64,
}

impl Placement {
    pub const fn new(anchor: Anchor, dx: f64, dy: f64) -> Self {
        Self { anchor, dx, dy }
    }

    /// `{"anchor", "dx", "dy"}` as a file or a client writes it: all three, the anchor one of the nine. Whether it suits an item is `Registry::valid`.
    pub fn from_json(v: &Value) -> Option<Placement> {
        let o = v.as_object()?;
        Some(Placement::new(
            Anchor::parse(o.get("anchor")?.as_str()?)?,
            o.get("dx")?.as_f64()?,
            o.get("dy")?.as_f64()?,
        ))
    }

    /// The same place to the four decimals a file keeps.
    pub fn same(&self, other: &Placement) -> bool {
        self.anchor == other.anchor
            && (self.dx - other.dx).abs() < 0.00005
            && (self.dy - other.dy).abs() < 0.00005
    }

    /// Where it is across the room, 0 to 1 from the left and from the top.
    pub fn fractions(&self) -> (f64, f64) {
        let a = self.anchor.fractions();
        (
            clamp(a.0 + self.dx, 0.0, 1.0),
            clamp(a.1 + self.dy, 0.0, 1.0),
        )
    }

    /// How far along its edge an edge item is, 0 to 1: from the top of a vertical edge, from the left of the bottom one.
    pub fn along(&self) -> f64 {
        let (fx, fy) = self.fractions();
        if self.anchor == Anchor::Bottom {
            fx
        } else {
            fy
        }
    }

    /// The placement of an edge item at `along` of the edge `edge`.
    pub fn on_edge(edge: Anchor, along: f64) -> Placement {
        if edge == Anchor::Bottom {
            Placement::new(edge, round4(along - 0.5), 0.0)
        } else {
            Placement::new(edge, 0.0, round4(along - 0.5))
        }
    }

    /// An item that grows from the top of its zone gives its room downwards; one low in the zone, upwards.
    pub fn grows_down(&self) -> bool {
        self.fractions().1 <= 0.5
    }
}

/// A placement and whether the owner made it (else it is the item's default).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Placed {
    #[serde(flatten)]
    pub placement: Placement,
    pub custom: bool,
}

// ---- the frame and the zones ---------------------------------------------------------------------------------------------------------

/// Home's safe area and status bar and the overlays' limit: what every zone is cut from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    pub safe_x: i32,
    pub safe_y: i32,
    /// The status bar's height.
    pub bar: i32,
    /// Where Home's content starts: below the status bar's row and its gap.
    pub top: i32,
    pub limit: i32,
}

impl Default for Frame {
    fn default() -> Self {
        Frame::new(SAFE_X, SAFE_Y, BAR, BAR_GAP, LIMIT)
    }
}

fn number(v: Option<&Value>, default: i32, lo: i32, hi: i32) -> i32 {
    match v.and_then(Value::as_f64) {
        Some(n) if n.is_finite() && n >= f64::from(lo) && n <= f64::from(hi) => round(n),
        _ => default,
    }
}

impl Frame {
    pub fn new(safe_x: i32, safe_y: i32, bar: i32, gap: i32, limit: i32) -> Self {
        Self {
            safe_x,
            safe_y,
            bar,
            top: safe_y + bar + gap,
            limit,
        }
    }

    /// From Home's layout (`layout.json`: `safe.x`, `safe.y`, `statusBar.height`, `statusBar.gap`) and the theme's `overlays.limit`. A value
    /// of the wrong kind or out of range is not taken.
    pub fn from_json(layout: &Value, limit: Option<&Value>) -> Self {
        let safe = layout.get("safe");
        let bar = layout.get("statusBar");
        Frame::new(
            number(safe.and_then(|s| s.get("x")), SAFE_X, 0, 240),
            number(safe.and_then(|s| s.get("y")), SAFE_Y, 0, 240),
            number(bar.and_then(|b| b.get("height")), BAR, 0, 200),
            number(bar.and_then(|b| b.get("gap")), BAR_GAP, 0, 100),
            number(limit, LIMIT, 360, H),
        )
    }
}

/// The room an item may be in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoneKind {
    /// Home's content area above the bottom quarter, below the status bar's row.
    Content,
    /// The same, down to the volume bar's rest above the bottom edge.
    Edge,
    /// The toasts' own, nearer the corner.
    Toast,
}

impl ZoneKind {
    pub fn name(self) -> &'static str {
        match self {
            ZoneKind::Content => "content",
            ZoneKind::Edge => "edge",
            ZoneKind::Toast => "toast",
        }
    }

    pub fn rect(self, f: &Frame) -> Rect {
        match self {
            ZoneKind::Content => Rect::from_edges(f.safe_x, f.top, W - f.safe_x, f.limit),
            ZoneKind::Edge => Rect::from_edges(f.safe_x, f.top, W - f.safe_x, H - REST),
            ZoneKind::Toast => Rect::from_edges(TOAST, TOAST, W - TOAST, f.limit),
        }
    }
}

// ---- the registry ----------------------------------------------------------------------------------------------------------------------

/// One kind of overlay. A new overlay is one entry in [`ITEMS`] and a surface that puts itself where `rect_of` says.
#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub id: &'static str,
    pub title: &'static str,
    /// The glyph's name in the shared set, for a list and the editor's tag.
    pub icon: &'static str,
    pub zone: ZoneKind,
    /// An item that lives on one of these edges and slides along it (the volume bar: vertical on the left and the right, horizontal at the
    /// bottom). Empty: it may be anywhere in its zone.
    pub edges: &'static [Anchor],
    /// How far over an app a bar on the bottom edge rises out of the subtitles' way; 0 for an item that does not.
    pub lift: i32,
    /// How far an item the owner placed moves, four places of a square, so a still panel never holds one pixel; 0 for an item that does not.
    pub drift: i32,
    /// An item with a way of its own until the owner places it (the mute mark's corners over an app).
    pub auto: bool,
    /// The item it goes beside when the two share a corner (the cards go under the camera window).
    pub stacks: Option<&'static str>,
    /// Where it was drawn before the owner could place it.
    pub default: Placement,
    /// Its size at 100 % text: what the surfaces are held to, and the stand-in's size when the toolkit does not measure one.
    pub sample: Size,
}

/// The items, in the order an editor offers them.
pub const ITEMS: [Item; 5] = [
    Item {
        id: "volume",
        title: "Volume bar",
        icon: "volume-2",
        zone: ZoneKind::Edge,
        edges: &[Anchor::Left, Anchor::Bottom, Anchor::Right],
        lift: LIFT,
        drift: 0,
        auto: false,
        stacks: None,
        default: Placement::new(Anchor::Bottom, 0.0, 0.0),
        sample: Size::new(440, 60),
    },
    Item {
        id: "mute",
        title: "Mute mark",
        icon: "volume-x",
        zone: ZoneKind::Content,
        edges: &[],
        lift: 0,
        drift: DRIFT,
        auto: true,
        stacks: None,
        default: Placement::new(Anchor::TopLeft, 0.0185, 0.0),
        sample: Size::new(MARK, MARK),
    },
    Item {
        id: "popup",
        title: "Camera window",
        icon: "video",
        zone: ZoneKind::Content,
        edges: &[],
        lift: 0,
        drift: 0,
        auto: false,
        stacks: None,
        default: Placement::new(Anchor::TopRight, 0.0, 0.0),
        sample: Size::new(560, 469),
    },
    Item {
        id: "cards",
        title: "Notifications",
        icon: "bell",
        zone: ZoneKind::Content,
        edges: &[],
        lift: 0,
        drift: 0,
        auto: false,
        stacks: Some("popup"),
        default: Placement::new(Anchor::TopRight, 0.0, 0.0),
        sample: Size::new(560, 165),
    },
    Item {
        id: "toast",
        title: "Toasts",
        icon: "info",
        zone: ZoneKind::Toast,
        edges: &[],
        lift: 0,
        drift: 0,
        auto: false,
        stacks: None,
        default: Placement::new(Anchor::TopRight, 0.0, 0.0),
        sample: Size::new(560, 98),
    },
];

/// The items a model works on: [`Registry::shipped`], or the shipped ones and more (a test, a plugin's overlay).
#[derive(Clone, Debug)]
pub struct Registry {
    items: Vec<Item>,
}

impl Registry {
    pub fn shipped() -> &'static Registry {
        static SHIPPED: LazyLock<Registry> = LazyLock::new(|| Registry {
            items: ITEMS.to_vec(),
        });
        &SHIPPED
    }

    pub fn new(items: Vec<Item>) -> Self {
        Self { items }
    }

    /// This registry and one more item.
    pub fn with(&self, item: Item) -> Registry {
        let mut items = self.items.clone();
        items.push(item);
        Registry { items }
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn item(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|it| it.id == id)
    }

    /// The zone an item is in (the content zone for an id the registry does not know).
    pub fn zone_of(&self, id: &str, f: &Frame) -> Rect {
        self.item(id)
            .map_or(ZoneKind::Content, |it| it.zone)
            .rect(f)
    }

    // ---- a placement

    /// Whether `p` is a placement of `it`: an item that lives on edges only takes those, and an offset is a fraction.
    pub fn valid(&self, it: &Item, p: &Placement) -> bool {
        (it.edges.is_empty() || it.edges.contains(&p.anchor))
            && p.dx.is_finite()
            && p.dy.is_finite()
            && p.dx.abs() <= 1.0
            && p.dy.abs() <= 1.0
    }

    /// Four decimals, and an edge item only slides along its edge: the offset across it is nothing.
    pub fn tidy(&self, it: &Item, p: Placement) -> Placement {
        let (mut dx, mut dy) = (round4(p.dx), round4(p.dy));
        if !it.edges.is_empty() {
            if p.anchor == Anchor::Bottom {
                dy = 0.0;
            } else {
                dx = 0.0;
            }
        }
        Placement::new(p.anchor, dx, dy)
    }

    /// The placement in force for `id`: the owner's, else the item's default.
    pub fn placement_of(&self, id: &str, user: &User) -> Option<Placed> {
        let it = self.item(id)?;
        Some(match user.get(id) {
            Some(p) => Placed {
                placement: p,
                custom: true,
            },
            None => Placed {
                placement: self.tidy(it, it.default),
                custom: false,
            },
        })
    }

    /// An item with a way of its own that the owner has not placed.
    pub fn is_auto(&self, id: &str, user: &User) -> bool {
        self.item(id).is_some_and(|it| it.auto) && !user.has(id)
    }

    /// Where a rectangle of `size` is: its place in the zone's room, whole pixels.
    pub fn rect_of(&self, id: &str, user: &User, f: &Frame, size: Size) -> Option<Rect> {
        let p = self.placement_of(id, user)?.placement;
        Some(rect_in(&self.zone_of(id, f), p.fractions(), size))
    }

    /// The placement that puts an item at (`fx`, `fy`) of its room: the nearest anchor and the small offsets from it, so a file reads "top
    /// right, a little lower". An edge item takes the nearest of its edges.
    pub fn from_fractions(&self, it: &Item, fx: f64, fy: f64) -> Placement {
        let names: &[Anchor] = if it.edges.is_empty() {
            &Anchor::ALL
        } else {
            it.edges
        };
        let (mut best, mut bd) = (names[0], f64::MAX);
        for a in names {
            let (ax, ay) = a.fractions();
            let d = (ax - fx) * (ax - fx) + (ay - fy) * (ay - fy);
            if d < bd - 1e-9 {
                bd = d;
                best = *a;
            }
        }
        let (ax, ay) = best.fractions();
        self.tidy(it, Placement::new(best, fx - ax, fy - ay))
    }

    // ---- what the owner placed

    /// Reads the owner's map (`layout.overlays` in the theme). Only whole, valid placements of items that exist are kept, and only those that
    /// differ from the default, so what is in force is what he changed and anything else is the default.
    pub fn user_from_json(&self, v: &Value) -> User {
        let mut out = User::default();
        let Some(obj) = v.as_object() else { return out };
        for it in &self.items {
            if let Some(p) = obj.get(it.id).and_then(Placement::from_json) {
                out = self.with_placement(&out, it.id, p);
            }
        }
        out
    }

    /// The owner's map with `p` for `id`. An invalid placement is not written; one equal to the default leaves the map.
    pub fn with_placement(&self, user: &User, id: &str, p: Placement) -> User {
        let mut out = user.clone();
        if let Some(it) = self.item(id) {
            if self.valid(it, &p) {
                let t = self.tidy(it, p);
                if t.same(&self.tidy(it, it.default)) {
                    out.0.remove(id);
                } else {
                    out.0.insert(id.to_string(), t);
                }
            }
        }
        out
    }

    // ---- the volume bar's edge and lift

    /// How far over an app an item rises out of the subtitles' way: only a bar lying along the bottom edge reaches them, and only while it is
    /// lifted can it stay above them.
    pub fn lift_over(&self, id: &str, user: &User, over_app: bool) -> i32 {
        match (self.item(id), self.placement_of(id, user)) {
            (Some(it), Some(p))
                if it.lift > 0 && over_app && p.placement.anchor == Anchor::Bottom =>
            {
                it.lift
            }
            _ => 0,
        }
    }

    /// Which way an item arrives from, so that it comes in from the edge it sits against: `x` 1 from the right, -1 from the left, `y` likewise.
    /// An item off-centre across comes in from its side; one in the middle of the screen's width, from the top or the bottom.
    pub fn slide_of(&self, id: &str, user: &User) -> Slide {
        let Some(p) = self.placement_of(id, user) else {
            return Slide { x: 0, y: 0 };
        };
        let (fx, fy) = p.placement.fractions();
        if (fx - 0.5).abs() >= 0.17 {
            return Slide {
                x: if fx > 0.5 { 1 } else { -1 },
                y: 0,
            };
        }
        Slide {
            x: 0,
            y: if fy > 0.5 { 1 } else { -1 },
        }
    }

    // ---- all the items at once

    /// Where every item is. `sizes`: what the toolkit measured; `work`: an item being moved, not in the file yet; `over_app`: an app's picture is
    /// behind (a bar on the bottom edge rides its lift higher). Items that stack go beside the item they stack on where the two meet; where they
    /// would then leave their zone they `waits` (the cards wait for the window to go).
    pub fn layout_all(
        &self,
        user: &User,
        f: &Frame,
        sizes: &Sizes,
        work: Option<(&str, Placement)>,
        over_app: bool,
    ) -> Layout {
        let mut items: Vec<Laid> = Vec::with_capacity(self.items.len());
        for it in &self.items {
            let placement = match work {
                Some((id, p)) if id == it.id => p,
                _ => self
                    .placement_of(it.id, user)
                    .map_or(it.default, |p| p.placement),
            };
            let size = sizes.at(it, placement.anchor);
            let mut rect = rect_in(&it.zone.rect(f), placement.fractions(), size);
            if over_app && it.lift > 0 && placement.anchor == Anchor::Bottom {
                rect.y -= it.lift;
            }
            items.push(Laid {
                id: it.id,
                rect,
                placement,
                waits: false,
            });
        }
        for it in &self.items {
            let Some(under) = it.stacks else { continue };
            let Some(base) = items.iter().find(|l| l.id == under).map(|l| l.rect) else {
                continue;
            };
            let Some(me) = items.iter_mut().find(|l| l.id == it.id) else {
                continue;
            };
            let beside = stack_beside(Some(&base), me.rect, STACK, me.placement.grows_down());
            let z = it.zone.rect(f);
            // beside the window it may not fit the zone: the cards then wait for it to go and an editor draws them faint
            me.waits = beside != me.rect && (beside.y < z.y || beside.bottom() > z.bottom());
            me.rect = beside;
        }
        Layout { items }
    }

    /// The first item that `r` would be on top of: the pair that stack by design are not in each other's way, and neither is what the item
    /// already was on top of where it was picked up (`from`, its rectangle then). A move may never make a new overlap, but it can always leave an
    /// old one (text at 200 % makes the items taller than the room between them).
    pub fn blocker(
        &self,
        layout: &Layout,
        id: &str,
        r: &Rect,
        from: Option<&Rect>,
    ) -> Option<&'static str> {
        let me = self.item(id)?;
        for o in &self.items {
            let Some(other) = layout.rect(o.id) else {
                continue;
            };
            if o.id == id || me.stacks == Some(o.id) || o.stacks == Some(me.id) {
                continue;
            }
            if r.meets(&other, GAP) && !from.is_some_and(|f| f.meets(&other, GAP)) {
                return Some(o.id);
            }
        }
        None
    }

    /// The room a column of `size` (the cards) has: the zone's, less the window's where the two meet, which the column then goes beside.
    pub fn column_room_for(
        &self,
        id: &str,
        user: &User,
        f: &Frame,
        window: Option<&Rect>,
        gap: i32,
        size: Size,
    ) -> i32 {
        let Some(p) = self.placement_of(id, user) else {
            return 0;
        };
        let mut window = window;
        if let Some(w) = window {
            if let Some(own) = self.rect_of(id, user, f, size) {
                if !w.meets(&own, gap) {
                    window = None;
                }
            }
        }
        column_room(&self.zone_of(id, f), window, gap, p.placement.grows_down())
    }

    // ---- the mute mark's places

    /// A mark the owner placed stays near its place: four spots of a small square that leans into the screen (so it stays inside the zone),
    /// visited in turn every few seconds, so no pixel of the panel holds a still mark. A spot that would be under a card that is up (`avoid`) is
    /// left out, unless none is free.
    pub fn drift_spots(
        &self,
        id: &str,
        user: &User,
        f: &Frame,
        size: Size,
        avoid: &[Rect],
    ) -> Vec<Point> {
        let (Some(it), Some(r)) = (self.item(id), self.rect_of(id, user, f, size)) else {
            return Vec::new();
        };
        let z = it.zone.rect(f);
        let d = if it.drift > 0 { it.drift } else { DRIFT };
        let sx = if f64::from(r.x) + f64::from(size.w) / 2.0 < f64::from(z.x) + f64::from(z.w) / 2.0
        {
            1
        } else {
            -1
        };
        let sy = if f64::from(r.y) + f64::from(size.h) / 2.0 < f64::from(z.y) + f64::from(z.h) / 2.0
        {
            1
        } else {
            -1
        };
        let at = |x: i32, y: i32| Point {
            x: clamp_i(x, z.x, z.x + z.w - size.w),
            y: clamp_i(y, z.y, z.y + z.h - size.h),
        };
        let all = vec![
            at(r.x, r.y),
            at(r.x + sx * d, r.y),
            at(r.x + sx * d, r.y + sy * d),
            at(r.x, r.y + sy * d),
        ];
        let free: Vec<Point> = all
            .iter()
            .copied()
            .filter(|s| {
                !avoid
                    .iter()
                    .any(|a| Rect::new(s.x, s.y, size.w, size.h).meets(a, 0))
            })
            .collect();
        if free.is_empty() { all } else { free }
    }

    /// The box all four spots are in: what an editor draws around the mark and what overlaps are judged on.
    pub fn drift_box(&self, id: &str, user: &User, f: &Frame, size: Size) -> Option<Rect> {
        let spots = self.drift_spots(id, user, f, size, &[]);
        if spots.is_empty() {
            return None;
        }
        let x0 = spots.iter().map(|s| s.x).min()?;
        let y0 = spots.iter().map(|s| s.y).min()?;
        let x1 = spots.iter().map(|s| s.x + size.w).max()?;
        let y1 = spots.iter().map(|s| s.y + size.h).max()?;
        Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
    }

    // ---- words

    /// Where an item is, in the fewest words: "Corners" for an item with a way of its own, its edge for one that lives on an edge, else the
    /// nearest corner, edge or the middle, and "moved" when it is not quite on a stop.
    pub fn words(&self, id: &str, user: &User) -> String {
        let (Some(it), Some(p)) = (self.item(id), self.placement_of(id, user)) else {
            return String::new();
        };
        if self.is_auto(id, user) {
            return "Corners".into();
        }
        let p = p.placement;
        let (fx, fy) = p.fractions();
        let kept = if on_stop(fx) && on_stop(fy) {
            ""
        } else {
            ", moved"
        };
        if !it.edges.is_empty() {
            let a = p.along();
            let bottom = p.anchor == Anchor::Bottom;
            let name = match p.anchor {
                Anchor::Bottom => "Bottom edge",
                Anchor::Left => "Left edge",
                _ => "Right edge",
            };
            let side = if a < 0.17 {
                if bottom { "left" } else { "top" }
            } else if a < 0.42 {
                if bottom { "left third" } else { "upper third" }
            } else if a > 0.83 {
                if bottom { "right" } else { "bottom" }
            } else if a > 0.58 {
                if bottom { "right third" } else { "lower third" }
            } else {
                ""
            };
            return if side.is_empty() {
                name.into()
            } else {
                format!("{name}, {side}")
            };
        }
        const COL: [&str; 5] = ["left", "left third", "middle", "right third", "right"];
        const ROW: [&str; 5] = ["top", "upper third", "middle", "lower third", "bottom"];
        let (bx, by) = (band(fx), band(fy));
        let text = if (bx == 0 || bx == 4) && (by == 0 || by == 4) {
            format!("{} {}", cap(ROW[by]), COL[bx])
        } else if by == 0 || by == 4 {
            format!(
                "{}{}",
                cap(ROW[by]),
                if bx == 2 {
                    String::new()
                } else {
                    format!(", {}", COL[bx])
                }
            )
        } else if bx == 0 || bx == 4 {
            format!(
                "{}{}",
                cap(COL[bx]),
                if by == 2 {
                    String::new()
                } else {
                    format!(", {}", ROW[by])
                }
            )
        } else if bx == 2 && by == 2 {
            "Middle".into()
        } else {
            format!("{}, {}", cap(ROW[by]), COL[bx])
        };
        text + kept
    }

    /// The registry as a client reads it: the items and the constants the rules are made of.
    pub fn describe(&self) -> Value {
        let items: Vec<Value> = self
            .items
            .iter()
            .map(|it| {
                json!({
                    "id": it.id, "title": it.title, "icon": it.icon, "zone": it.zone.name(),
                    "edges": it.edges.iter().map(|e| e.name()).collect::<Vec<_>>(),
                    "lift": it.lift, "drift": it.drift, "auto": it.auto, "stacks": it.stacks,
                    "default": it.default, "sample": it.sample,
                })
            })
            .collect();
        json!({
            "items": items,
            "screen": {"w": W, "h": H},
            "rules": {"gap": GAP, "stack": STACK, "toastInset": TOAST, "rest": REST, "step": crate::motion::STEP, "stops": crate::motion::STOPS, "subtitles": SUBS},
        })
    }
}

fn band(v: f64) -> usize {
    if v < 0.17 {
        0
    } else if v < 0.42 {
        1
    } else if v <= 0.58 {
        2
    } else if v <= 0.83 {
        3
    } else {
        4
    }
}

fn on_stop(v: f64) -> bool {
    crate::motion::STOPS.iter().any(|s| (v - s).abs() < 0.005)
}

fn cap(t: &str) -> String {
    let mut c = t.chars();
    c.next().map_or(String::new(), |f| {
        f.to_uppercase().collect::<String>() + c.as_str()
    })
}

/// Where a rectangle of `size` is at (`fx`, `fy`) of the room `z` leaves it, whole pixels.
pub fn rect_in(z: &Rect, (fx, fy): (f64, f64), size: Size) -> Rect {
    Rect::new(
        z.x + round(fx * f64::from((z.w - size.w).max(0))),
        z.y + round(fy * f64::from((z.h - size.h).max(0))),
        size.w,
        size.h,
    )
}

// ---- stacking ---------------------------------------------------------------------------------------------------------------------------

/// `r` placed beside `ref_` when they meet (the cards go under the camera window, or above it when they grow upwards); its place on the other
/// axis is its own.
pub fn stack_beside(ref_: Option<&Rect>, r: Rect, gap: i32, down: bool) -> Rect {
    match ref_ {
        Some(b) if b.meets(&r, gap) => Rect::new(
            r.x,
            if down {
                b.bottom() + gap
            } else {
                b.y - gap - r.h
            },
            r.w,
            r.h,
        ),
        _ => r,
    }
}

/// The height a column of cards may be, beside a window that is up or with the zone to itself.
pub fn column_room(z: &Rect, ref_: Option<&Rect>, gap: i32, down: bool) -> i32 {
    match ref_ {
        None => z.h,
        Some(b) => (if down {
            z.bottom() - (b.bottom() + gap)
        } else {
            b.y - gap - z.y
        })
        .max(0),
    }
}

// ---- what the owner placed -----------------------------------------------------------------------------------------------------------------

/// What the owner placed: `layout.overlays` in the theme. Only whole, valid placements that differ from the default are in it; build one with
/// [`Registry::user_from_json`] or [`Registry::with_placement`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct User(BTreeMap<String, Placement>);

impl User {
    pub fn get(&self, id: &str) -> Option<Placement> {
        self.0.get(id).copied()
    }

    pub fn has(&self, id: &str) -> bool {
        self.0.contains_key(id)
    }

    /// How many items the owner has placed.
    pub fn customised(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn without(&self, id: &str) -> User {
        let mut out = self.clone();
        out.0.remove(id);
        out
    }

    /// What goes in the file: the whole map, since a theme patch merges one level deep.
    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        for (id, p) in &self.0 {
            map.insert(id.clone(), json!(p));
        }
        Value::Object(map)
    }
}

// ---- sizes and the layout ---------------------------------------------------------------------------------------------------------------

/// What the toolkit measured: the size of each item as it draws it (at the text size in force), and of an edge item on each of its edges.
/// An item with no size here is its sample's.
#[derive(Clone, Debug, Default)]
pub struct Sizes(BTreeMap<String, Size>);

impl Sizes {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, id: &str, size: Size) -> &mut Self {
        self.0.insert(id.to_string(), size);
        self
    }

    /// An edge item's size on one edge.
    pub fn set_edge(&mut self, id: &str, edge: Anchor, size: Size) -> &mut Self {
        self.0.insert(format!("{id}/{}", edge.name()), size);
        self
    }

    pub fn at(&self, it: &Item, anchor: Anchor) -> Size {
        if !it.edges.is_empty() {
            if let Some(s) = self.0.get(&format!("{}/{}", it.id, anchor.name())) {
                return *s;
            }
        }
        self.0.get(it.id).copied().unwrap_or(it.sample)
    }

    /// `{"popup": {"w", "h"}, "volume": {"left": {"w", "h"}, "bottom": {...}, "right": {...}}}`: a size is whole pixels, anything else is left
    /// out.
    pub fn from_json(v: &Value) -> Sizes {
        let mut out = Sizes::new();
        let Some(obj) = v.as_object() else { return out };
        for (id, entry) in obj {
            if let Some(s) = parse_size(entry) {
                out.set(id, s);
            } else if let Some(edges) = entry.as_object() {
                for (edge, s) in edges {
                    if let (Some(a), Some(s)) = (Anchor::parse(edge), parse_size(s)) {
                        out.set_edge(id, a, s);
                    }
                }
            }
        }
        out
    }
}

fn parse_size(v: &Value) -> Option<Size> {
    let (w, h) = (v.get("w")?.as_f64()?, v.get("h")?.as_f64()?);
    (w.is_finite() && h.is_finite() && (0.0..=4096.0).contains(&w) && (0.0..=4096.0).contains(&h))
        .then(|| Size::new(round(w), round(h)))
}

/// Which way an item arrives from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Slide {
    pub x: i32,
    pub y: i32,
}

/// One item laid out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Laid {
    pub id: &'static str,
    pub rect: Rect,
    pub placement: Placement,
    /// Beside the window it would leave its zone: it waits for the window to go.
    pub waits: bool,
}

/// Every item, in the registry's order.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub items: Vec<Laid>,
}

impl Layout {
    pub fn get(&self, id: &str) -> Option<&Laid> {
        self.items.iter().find(|l| l.id == id)
    }

    pub fn rect(&self, id: &str) -> Option<Rect> {
        self.get(id).map(|l| l.rect)
    }

    pub fn waits(&self, id: &str) -> bool {
        self.get(id).is_some_and(|l| l.waits)
    }

    /// `{id: {"rect", "placement", "waits"}}`.
    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        for l in &self.items {
            map.insert(
                l.id.to_string(),
                json!({"rect": l.rect, "placement": l.placement, "waits": l.waits}),
            );
        }
        Value::Object(map)
    }
}
