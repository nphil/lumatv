//! The overlay layout editor drawn (`docs/mocks/overlay-layout`, `docs/research/overlay-layout-result.md`): the owner's pop-ups placed on a picture of the
//! screen with the D-pad and no numbers. The controller is `lumatv_placement::Editor` (browse, pick up, move, put down, put back, Reset, refusals); this
//! draws its [`View`] and hands the pointer to it.
//!
//! - **The picture:** an opaque screen with what the pop-ups keep clear of drawn as guides (the safe area, Home's status bar's row, the bottom quarter
//!   where the subtitles are) and the five items as stand-ins of the real surfaces at the sizes those measure, each at the rectangle the placement crate
//!   gives it, so what the owner sees is where it will be. An item the cards would wait for is faint; the one in the hand has its old place outlined; the
//!   one the mute mark drifts about has its box outlined.
//! - **The panel:** heading, the item and where it is, a notice when a move was refused, three buttons (Reset this, Reset all, Done) and the keys as
//!   glyphs of the remote, in the first place nearest the middle that no item is in or beside (`Editor::panel_spot`). It is built again when its words or
//!   its place change; the buttons are the kit's, in a modal scope of the shared engine, and the editor's focus is followed with the flowing lift.
//! - **The pointer:** hover is a cue on an item or a button, a press on a button runs it, an item is taken where it is pressed after 12 px, follows the
//!   pointer and is put down where the pointer lets go.

use super::parts::{HIDDEN, Sheet, hint_into, hint_width};
use crate::kit::button::{Button, Place, Spec, Tone, faded};
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::hint::{Action, Hint, Key};
use crate::kit::icon;
use crate::kit::lint::Lint;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::pointer::{Gesture, Pointer, Sample};
use crate::kit::text::{Text, role_style, width_of};
use crate::kit::{Engine, Kit, wrap};
use compositor_protocol::{Fill, Props, Rgba, Shape};
use lumatv_placement::editor::{Editor, View};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use ui::components::{Align, Style, plate};
use ui::geometry::Frame;
use ui::lucent::palette::PaletteRole;
use ui::scene::ShapeLayer;
use ui::text::Weight;

const ID_BASE: u32 = 0x5359_0200;
const SCOPE: Scope = Scope(110);
const REGION: Region = Region(110);
const W: f32 = 1920.0;
const H: f32 = 1080.0;
/// The buttons of the status bar's stand-in, drawn over its glass (which may be as wide as the row, with the buttons where the placement puts them).
const BUTTONS_PART: &str = "statusbar/buttons";

/// Depths above the surface's base: the picture, the items, what marks them, the panel.
const Z_GUIDE: i32 = 1;
const Z_ITEM: i32 = 10;
const Z_DECO: i32 = 60;
const Z_PANEL: i32 = 80;

fn id(n: usize) -> Id {
    Id(ID_BASE + n as u32)
}

struct Word {
    text: String,
    frame: Frame,
    style: Style,
}

/// The picture behind the items: the opaque screen and the guides.
struct Stat {
    sheet: Sheet,
    generation: u64,
    /// Whether Home's status bar's row is named: not once the status bar's glass covers the row, through which the words would show.
    words: bool,
}

/// An item's stand-in: built for one size and moved to wherever the editor puts it.
struct Standin {
    id: &'static str,
    size: (i32, i32),
    generation: u64,
    sheet: Sheet,
    at: (f32, f32),
    faint: bool,
}

/// What marks an item: a thin outline made for a size and a look.
struct Mark {
    key: (i32, i32, u8),
    layer: ShapeLayer,
    at: (f32, f32),
}

struct Tag {
    id: &'static str,
    generation: u64,
    sheet: Sheet,
    at: (f32, f32),
    size: (f32, f32),
    words: Vec<Word>,
    frame: Frame,
}

struct Panel {
    sig: String,
    sheet: Sheet,
    buttons: Vec<Button>,
    frame: Frame,
    paint: Rgba,
    words: Vec<Word>,
    /// Where the hints go: the foot of the panel.
    foot: (f32, f32, f32),
}

struct Hints {
    sig: String,
    sheet: Sheet,
    words: Vec<Word>,
}

#[derive(Default)]
struct Deco {
    ring: Option<Mark>,
    ghost: Option<Mark>,
    reach: Option<Mark>,
    hover: Option<Mark>,
}

pub struct LayoutSurface {
    z: i32,
    stat: Option<Stat>,
    items: Vec<Standin>,
    deco: Deco,
    tag: Option<Tag>,
    panel: Option<Panel>,
    hints: Option<Hints>,
    pointer: Pointer,
    /// The item the pointer has hold of, and whether the button was down at the last sample.
    grab: Option<&'static str>,
    was_down: bool,
    hover: Option<&'static str>,
    modal: bool,
    /// The pointer moved something that is drawn (a cue, an item in the hand): the layer is to draw again.
    dirty: bool,
    /// What the last draw said, for the harness and the pointer's hit test.
    last: Value,
    rects: Vec<(&'static str, Frame)>,
}

fn rect_frame(r: &lumatv_placement::Rect) -> Frame {
    Frame::new(r.x as f32, r.y as f32, r.w as f32, r.h as f32)
}

fn outline(kit: &Kit, size: (f32, f32), z: i32, border: Rgba, width: f32, radius: f32) -> ShapeLayer {
    let shape = Shape { corner_radius: radius, fill: Fill::Solid(Rgba(0, 0, 0, 0)), border_width: width, border };
    kit.ui.scene.shape(size.0.round().max(1.0) as u32, size.1.round().max(1.0) as u32, z, shape, HIDDEN)
}

impl LayoutSurface {
    pub fn new(z: i32) -> Self {
        Self {
            z,
            stat: None,
            items: Vec::new(),
            deco: Deco::default(),
            tag: None,
            panel: None,
            hints: None,
            pointer: Pointer::new(),
            grab: None,
            was_down: false,
            hover: None,
            modal: false,
            dirty: false,
            last: Value::Null,
            rects: Vec::new(),
        }
    }

    pub fn is_open(&self) -> bool {
        self.modal
    }

    // ---- the picture and the stand-ins

    /// Whether the picture names Home's status bar's row: it does not while the status bar's glass covers the whole row.
    fn row_words(view: &View) -> bool {
        !view.items.iter().any(|i| i.id == "statusbar" && i.glass.w >= view.guides.status_bar.w)
    }

    fn build_stat(&self, kit: &mut Kit, view: &View) -> std::io::Result<Stat> {
        let z = self.z;
        let mut sheet = Sheet::new();
        let canvas = kit.ui.look.canvas();
        sheet.shape(plate::shape(&kit.ui, Frame::new(0.0, 0.0, W, H), z, plate::solid(canvas, 0.0)), (0.0, 0.0));
        let (ink2, edge) = (kit.ui.look.ink2(), kit.ui.look.colour(PaletteRole::Edge));
        let g = &view.guides;
        // the safe area
        let safe = rect_frame(&g.safe);
        let ring = outline(kit, (safe.w, safe.h), z + Z_GUIDE, faded(edge, 0.8), 1.0, 0.0);
        sheet.shape(ring, (safe.x, safe.y));
        // Home's status bar's row and its words
        let bar = rect_frame(&g.status_bar);
        let wash = faded(kit.ui.look.ink(), 0.06);
        sheet.shape(plate::shape(&kit.ui, bar, z + Z_GUIDE, plate::solid(wash, kit.tokens.radius(Radius::Control))), (bar.x, bar.y));
        if Self::row_words(view) {
            let caption = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
            let line = kit.line(TypeRole::Caption);
            Self::word(kit, &mut sheet, Frame::new(bar.x + kit.space(6), bar.y + ((bar.h - line) / 2.0).round(), 500.0, line), z + Z_GUIDE + 1, caption, "Home\u{2019}s status bar")?;
        }
        // the bottom quarter, where the subtitles and the volume bar's room are, and the subtitles' band
        let quarter = rect_frame(&g.bottom_quarter);
        let danger = kit.ui.look.colour(PaletteRole::Danger);
        sheet.shape(plate::shape(&kit.ui, quarter, z + Z_GUIDE, plate::solid(faded(danger, 0.10), 0.0)), (quarter.x, quarter.y));
        sheet.shape(plate::shape(&kit.ui, Frame::new(0.0, quarter.y, W, 2.0), z + Z_GUIDE + 1, plate::solid(faded(danger, 0.55), 0.0)), (0.0, quarter.y));
        let subs = rect_frame(&g.subtitles);
        let centre = role_style(kit, TypeRole::Body, None, ink2, Align::Centre);
        let bh = kit.line(TypeRole::Body);
        sheet.text(Text::new(kit, Frame::new(0.0, subs.y + ((subs.h - bh) / 2.0).round(), W, bh), z + Z_GUIDE + 1, centre, "Subtitles")?);
        sheet.show(kit, true);
        Ok(Stat { sheet, generation: kit.generation(), words: Self::row_words(view) })
    }

    /// A word of a stand-in, cut with an ellipsis where it does not fit its box.
    fn word(kit: &mut Kit, sheet: &mut Sheet, frame: Frame, z: i32, style: Style, text: &str) -> std::io::Result<()> {
        let fitted = wrap::elide(kit, style, text, frame.w);
        sheet.text(Text::new(kit, frame, z, style, &fitted)?);
        Ok(())
    }

    /// A stand-in of the item `item` (`w` by `h`) at `at`, as its real surface looks.
    fn build_standin(kit: &mut Kit, z: i32, item: &str, size: (i32, i32), at: (f32, f32)) -> std::io::Result<Sheet> {
        let (w, h) = (size.0 as f32, size.1 as f32);
        let mut sheet = Sheet::new();
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let frame = Frame::new(at.0, at.1, w, h);
        let kind = if matches!(item, "volume" | "mute" | "statusbar" | "format") { PaneKind::Pill } else { PaneKind::Card };
        if item != BUTTONS_PART {
            sheet.pane(Pane::new(kit, frame, z, kind, false));
        }
        let pad = kit.space(5);
        match item {
            "volume" => {
                let label = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Centre);
                let line = kit.line(TypeRole::Label);
                let track_off = kit.ui.look.theme.materials.track_off;
                let t = kit.space(2);
                if w >= h {
                    let num_w = width_of(kit, label, "100").max(32.0);
                    let px = kit.space(6);
                    sheet.text(Text::new(kit, Frame::new(at.0 + px, at.1 + ((h - line) / 2.0).round(), num_w, line), z + 3, label, "42")?);
                    let track = Frame::new(at.0 + px + num_w + kit.space(5), at.1 + ((h - t) / 2.0).round(), w - 2.0 * px - num_w - kit.space(5), t);
                    sheet.shape(plate::shape(&kit.ui, track, z + 1, plate::solid(track_off, t / 2.0)), (track.x, track.y));
                    let fill = Frame::new(track.x, track.y, (track.w * 0.42).round(), t);
                    sheet.shape(plate::shape(&kit.ui, fill, z + 2, plate::solid(ink, t / 2.0)), (fill.x, fill.y));
                } else {
                    let px = kit.space(6);
                    sheet.text(Text::new(kit, Frame::new(at.0, at.1 + px, w, line), z + 3, label, "42")?);
                    let top = at.1 + px + line + kit.space(5);
                    let track = Frame::new(at.0 + ((w - t) / 2.0).round(), top, t, h - (top - at.1) - px);
                    sheet.shape(plate::shape(&kit.ui, track, z + 1, plate::solid(track_off, t / 2.0)), (track.x, track.y));
                    let fh = (track.h * 0.42).round();
                    let fill = Frame::new(track.x, track.y + track.h - fh, t, fh);
                    sheet.shape(plate::shape(&kit.ui, fill, z + 2, plate::solid(ink, t / 2.0)), (fill.x, fill.y));
                }
            }
            "mute" => {
                let side = 32u32;
                if let Some(g) = icon::layer(kit, "volume-x", side, ink, z + 3, HIDDEN)? {
                    sheet.image(g, (at.0 + ((w - side as f32) / 2.0).round(), at.1 + ((h - side as f32) / 2.0).round()));
                }
            }
            "format" => {
                let (marks, _) = crate::system::badge_surface::draw_marks(kit, &crate::system::badge_surface::SAMPLE, z + 3, HIDDEN)?;
                let spot = (at.0 + ((w - marks.width() as f32) / 2.0).round(), at.1 + ((h - marks.height() as f32) / 2.0).round());
                sheet.image(marks, spot);
            }
            // the glass only: the buttons are a stand-in of their own over it
            "statusbar" => {}
            BUTTONS_PART => {
                // the pill's buttons as the bar draws them: four glyphs and the profile chip (the real buttons are Home's)
                let inset = 6.0;
                let side = (h - 2.0 * inset).max(8.0);
                let glyph = ((side / 2.0).round() as u32).max(8);
                let gap = kit.space(2);
                let mut x = at.0 + inset;
                for name in ["search", "camera", "bell", "settings"] {
                    if x + side > at.0 + w - inset {
                        break;
                    }
                    if let Some(g) = icon::layer(kit, name, glyph, ink, z + 3, HIDDEN)? {
                        sheet.image(g, (x + ((side - glyph as f32) / 2.0).round(), at.1 + inset + ((side - glyph as f32) / 2.0).round()));
                    }
                    x += side + gap;
                }
                let chip = Frame::new(x, at.1 + inset, (at.0 + w - inset - x).max(0.0), side);
                if chip.w > side / 2.0 {
                    sheet.shape(plate::shape(&kit.ui, chip, z + 1, plate::solid(faded(ink, 0.12), (side / 2.0).floor())), (chip.x, chip.y));
                }
            }
            "popup" => {
                let name = role_style(kit, TypeRole::Body, Some(Weight::SemiBold), ink, Align::Left);
                let what = role_style(kit, TypeRole::Label, None, ink2, Align::Left);
                let caption = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
                let glyph = (36.0 * (1.0 + 0.6 * (kit.text_scale - 1.0))).round();
                if let Some(g) = icon::layer(kit, "bell-ring", glyph as u32, ink, z + 3, HIDDEN)? {
                    sheet.image(g, (at.0 + pad, at.1 + pad));
                }
                let tx = at.0 + pad + glyph + kit.space(4);
                let (nh, wh) = (kit.line(TypeRole::Body), kit.line(TypeRole::Label));
                let tw = w - (tx - at.0) - pad;
                Self::word(kit, &mut sheet, Frame::new(tx, at.1 + pad, tw, nh), z + 3, name, "Front door")?;
                Self::word(kit, &mut sheet, Frame::new(tx, at.1 + pad + nh + kit.space(1), tw, wh), z + 3, what, "Doorbell rang")?;
                let head = glyph.max(nh + kit.space(1) + wh);
                let foot_h = kit.line(TypeRole::Caption);
                let pic = Frame::new(at.0 + pad, at.1 + pad + head + kit.space(3), w - 2.0 * pad, (h - 2.0 * pad - head - foot_h - 2.0 * kit.space(3)).max(40.0));
                sheet.shape(plate::shape(&kit.ui, pic, z + 1, plate::solid(faded(ink, 0.10), kit.tokens.radius(Radius::Tile))), (pic.x, pic.y));
                let side = 64u32.min((pic.h as u32).saturating_sub(8)).max(16);
                if let Some(g) = icon::layer(kit, "video", side, kit.ui.look.ink3(), z + 2, HIDDEN)? {
                    sheet.image(g, (pic.x + ((pic.w - side as f32) / 2.0).round(), pic.y + ((pic.h - side as f32) / 2.0).round()));
                }
                sheet.text(Text::new(kit, Frame::new(at.0 + pad, pic.y + pic.h + kit.space(3), 200.0, foot_h), z + 3, caption, "Live")?);
            }
            "cards" => {
                let name = role_style(kit, TypeRole::Body, Some(Weight::SemiBold), ink, Align::Left);
                let what = role_style(kit, TypeRole::Label, None, ink2, Align::Left);
                let caption = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
                let lead = (56.0 * (1.0 + 0.6 * (kit.text_scale - 1.0))).round();
                let accent = kit.ui.look.accent();
                let (tile, glyph) = super::parts::tile(kit, lead, z + 1, "timer", false, accent)?;
                sheet.shape(tile, (at.0 + pad, at.1 + pad));
                if let Some(g) = glyph {
                    sheet.image(g, super::parts::tile_glyph_at((at.0 + pad, at.1 + pad), lead));
                }
                let tx = at.0 + pad + lead + kit.space(4);
                let (nh, wh) = (kit.line(TypeRole::Body), kit.line(TypeRole::Label));
                let tw = w - (tx - at.0) - pad;
                Self::word(kit, &mut sheet, Frame::new(tx, at.1 + pad, tw, nh), z + 3, name, "Pasta")?;
                Self::word(kit, &mut sheet, Frame::new(tx, at.1 + pad + nh + kit.space(1), tw, wh), z + 3, what, "Time\u{2019}s up")?;
                let foot_y = at.1 + pad + lead.max(nh + kit.space(1) + wh) + kit.space(3);
                Self::word(kit, &mut sheet, Frame::new(at.0 + pad, foot_y, w - 2.0 * pad, kit.line(TypeRole::Caption)), z + 3, caption, "Kitchen Show")?;
            }
            _ => {
                // the toast: the info tile, the title and the line under it
                let title = role_style(kit, TypeRole::Label, None, ink, Align::Left);
                let body = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
                let p = kit.space(3);
                let tile_px = (48.0 * (1.0 + 0.6 * (kit.text_scale - 1.0))).round();
                let (tile, glyph) = super::parts::icon_tile(kit, tile_px, z + 1, "info", true)?;
                sheet.shape(tile, (at.0 + p, at.1 + p));
                if let Some(g) = glyph {
                    sheet.image(g, super::parts::tile_glyph_at((at.0 + p, at.1 + p), tile_px));
                }
                let tx = at.0 + p + tile_px + kit.space(3);
                let (th, bh) = (kit.line(TypeRole::Label), kit.line(TypeRole::Caption));
                let tw = w - (tx - at.0) - p;
                Self::word(kit, &mut sheet, Frame::new(tx, at.1 + p, tw, th), z + 3, title, "Remote battery low")?;
                Self::word(kit, &mut sheet, Frame::new(tx, at.1 + p + th, tw, bh), z + 3, body, "Replace the batteries soon")?;
            }
        }
        sheet.show(kit, true);
        Ok(sheet)
    }

    fn standin_radius(kit: &Kit, item: &str, size: (i32, i32)) -> f32 {
        if matches!(item, "volume" | "mute" | "statusbar" | "format") { (size.0.min(size.1) as f32 / 2.0).floor() } else { kit.tokens.radius(Radius::Card) }
    }

    fn sync_items(&mut self, kit: &mut Kit, view: &View) -> std::io::Result<()> {
        self.items.retain(|s| s.generation == kit.generation());
        for it in &view.items {
            // the status bar is its glass, which may span the whole row, and over it its buttons, which are where the placement puts them
            if it.id == "statusbar" {
                self.sync_part(kit, it.id, &it.glass, it.waits, view.dragging)?;
                self.sync_part(kit, BUTTONS_PART, &it.rect, it.waits, view.dragging)?;
            } else {
                self.sync_part(kit, it.id, &it.rect, it.waits, view.dragging)?;
            }
        }
        Ok(())
    }

    /// One stand-in, built for its size and moved to `r`.
    fn sync_part(&mut self, kit: &mut Kit, id: &'static str, r: &lumatv_placement::Rect, waits: bool, dragging: bool) -> std::io::Result<()> {
        let size = (r.w, r.h);
        let at = (r.x as f32, r.y as f32);
        let z = self.z + Z_ITEM;
        let k = self.items.iter().position(|s| s.id == id);
        let fresh = match k {
            Some(k) => self.items[k].size != size,
            None => true,
        };
        if fresh {
            if let Some(k) = k {
                self.items.remove(k);
            }
            let sheet = Self::build_standin(kit, z, id, size, at)?;
            self.items.push(Standin { id, size, generation: kit.generation(), sheet, at, faint: false });
        }
        let s = self.items.iter_mut().find(|s| s.id == id).expect("made above");
        if s.at != at {
            // while the owner holds it down it follows at once, a key's step glides
            s.sheet.shift(kit, at.0 - s.at.0, at.1 - s.at.1, dragging);
            s.at = at;
        }
        if s.faint != waits {
            s.sheet.set_alpha(if waits { 0.4 } else { 1.0 });
            s.faint = waits;
        }
        Ok(())
    }

    fn mark(&self, kit: &Kit, slot: &mut Option<Mark>, want: Option<(Frame, u8, Rgba, f32, f32)>) {
        match want {
            None => *slot = None,
            Some((frame, kind, colour, width, radius)) => {
                let key = (frame.w as i32, frame.h as i32, kind);
                if slot.as_ref().is_none_or(|m| m.key != key) {
                    let layer = outline(kit, (frame.w, frame.h), self.z + Z_DECO, colour, width, radius);
                    *slot = Some(Mark { key, layer, at: (f32::NAN, f32::NAN) });
                }
                if let Some(m) = slot {
                    if m.at != (frame.x, frame.y) {
                        m.layer.set(Props::at(frame.x, frame.y));
                        m.at = (frame.x, frame.y);
                    }
                }
            }
        }
    }

    fn sync_deco(&mut self, kit: &mut Kit, view: &View) {
        let look = &kit.ui.look;
        let (accent, ink, danger) = (look.accent(), look.ink(), look.colour(PaletteRole::Danger));
        let sel = view.items.iter().find(|i| i.selected);
        let ring = sel.map(|i| {
            let r = rect_frame(&i.glass);
            let (colour, kind, width) = if i.flagged { (danger, 2u8, 4.0) } else if i.in_hand { (accent, 1, 4.0) } else { (faded(accent, 0.85), 0, 3.0) };
            let radius = Self::standin_radius(kit, i.id, (i.glass.w, i.glass.h)) + 6.0;
            (Frame::new(r.x - 6.0, r.y - 6.0, r.w + 12.0, r.h + 12.0), kind, colour, width, radius)
        });
        let ghost = view.held_rect.as_ref().zip(sel).map(|(r, i)| {
            let f = rect_frame(r);
            (f, 3u8, faded(ink, 0.5), 2.0, Self::standin_radius(kit, i.id, (r.w, r.h)))
        });
        let reach = view.items.iter().find_map(|i| i.reach.as_ref().map(|r| (r, i))).map(|(r, i)| {
            let f = rect_frame(r);
            (Frame::new(f.x - 4.0, f.y - 4.0, f.w + 8.0, f.h + 8.0), 4u8, faded(if i.selected { accent } else { ink }, 0.45), 2.0, 20.0)
        });
        let hover = self.hover.and_then(|h| view.items.iter().find(|i| i.id == h && !i.selected)).map(|i| {
            let r = rect_frame(&i.glass);
            (Frame::new(r.x - 4.0, r.y - 4.0, r.w + 8.0, r.h + 8.0), 5u8, faded(ink, 0.4), 2.0, Self::standin_radius(kit, i.id, (i.glass.w, i.glass.h)) + 4.0)
        });
        let mut deco = std::mem::take(&mut self.deco);
        self.mark(kit, &mut deco.ring, ring);
        self.mark(kit, &mut deco.ghost, ghost);
        self.mark(kit, &mut deco.reach, reach);
        self.mark(kit, &mut deco.hover, hover);
        self.deco = deco;
    }

    /// The name of the chosen item on a pill above it (below it where there is no room above): what the picture's items are called.
    fn sync_tag(&mut self, kit: &mut Kit, view: &View) -> std::io::Result<()> {
        let Some(it) = view.items.iter().find(|i| i.selected) else {
            self.tag = None;
            return Ok(());
        };
        if self.tag.as_ref().is_none_or(|t| t.id != it.id || t.generation != kit.generation()) {
            let (ink, label) = (kit.ui.look.ink(), role_style(kit, TypeRole::Label, None, kit.ui.look.ink(), Align::Left));
            let side = 22u32.max((22.0 * kit.text_scale).round() as u32);
            let tw = width_of(kit, label, it.title);
            let h = (kit.line(TypeRole::Label) + 2.0 * kit.space(2)).max(side as f32 + 2.0 * kit.space(1)).ceil();
            let w = (kit.space(4) + side as f32 + kit.space(2) + tw + kit.space(4)).ceil();
            let z = self.z + Z_DECO + 2;
            let mut sheet = Sheet::new();
            let frame = Frame::new(0.0, 0.0, w, h);
            sheet.pane(Pane::new(kit, frame, z, PaneKind::Pill, false));
            if let Some(g) = icon::layer(kit, it.icon, side, ink, z + 1, HIDDEN)? {
                sheet.image(g, (kit.space(4), ((h - side as f32) / 2.0).round()));
            }
            let line = kit.line(TypeRole::Label);
            let wf = Frame::new(kit.space(4) + side as f32 + kit.space(2), ((h - line) / 2.0).round(), tw, line);
            sheet.text(Text::new(kit, wf, z + 1, label, it.title)?);
            sheet.show(kit, true);
            self.tag = Some(Tag { id: it.id, generation: kit.generation(), sheet, at: (0.0, 0.0), size: (w, h), words: vec![Word { text: it.title.to_string(), frame: wf, style: label }], frame });
        }
        let t = self.tag.as_mut().expect("made above");
        let r = rect_frame(&it.glass);
        let x = r.x.clamp(8.0, W - t.size.0 - 8.0);
        let above = r.y - t.size.1 - 10.0;
        let y = if above >= 8.0 { above } else { r.y + r.h + 10.0 };
        if t.at != (x, y) {
            t.sheet.shift(kit, x - t.at.0, y - t.at.1, true);
            for w in &mut t.words {
                w.frame.x += x - t.at.0;
                w.frame.y += y - t.at.1;
            }
            t.frame.x += x - t.at.0;
            t.frame.y += y - t.at.1;
            t.at = (x, y);
        }
        Ok(())
    }

    // ---- the panel

    fn hints_of(view: &View) -> Vec<Hint> {
        let two = |a: Key, b: Key, label: &str| Hint { keys: vec![(a, false), (b, false)], action: None, label: Some(label.to_string()) };
        if view.mode == "move" {
            let dirs = Hint { keys: vec![(Key::Up, false), (Key::Down, false), (Key::Left, false), (Key::Right, false)], action: Some(Action::Move), label: None };
            vec![dirs, Hint::key(Key::Ok, Action::Confirm), Hint::labelled(Key::Back, "Put back")]
        } else if view.focus == "buttons" {
            vec![two(Key::Left, Key::Right, "Choose"), Hint::key(Key::Ok, Action::Confirm), Hint::labelled(Key::Up, "Items")]
        } else {
            vec![two(Key::Left, Key::Right, "Choose"), Hint::key(Key::Ok, Action::Move), Hint::labelled(Key::Down, "Buttons"), Hint::key(Key::Back, Action::Close)]
        }
    }

    fn panel_plan(kit: &mut Kit, view: &View) -> (f32, f32, f32) {
        // the width: the widest of the heading, the item, the buttons and one row of keys, never narrower than 560
        let pad = kit.space(6);
        let title = role_style(kit, TypeRole::Title, None, kit.ui.look.ink(), Align::Left);
        let body = role_style(kit, TypeRole::Body, None, kit.ui.look.ink(), Align::Left);
        let label = role_style(kit, TypeRole::Label, None, kit.ui.look.ink2(), Align::Left);
        let mut inner = 0f32;
        if let Some(h) = view.heading {
            inner = inner.max(width_of(kit, title, h) + (36.0 * (1.0 + 0.6 * (kit.text_scale - 1.0))).round() + kit.space(3));
        }
        inner = inner.max(width_of(kit, body, &view.subject));
        if !view.notice.is_empty() {
            inner = inner.max(width_of(kit, label, &view.notice));
        }
        let sp = kit.space(5);
        let hints = Self::hints_of(view);
        let row: f32 = hints.iter().map(|h| hint_width(kit, h, TypeRole::Caption)).sum::<f32>() + sp * hints.len().saturating_sub(1) as f32;
        inner = inner.max(row);
        let gap = kit.space(3);
        if !view.buttons.is_empty() {
            let w: f32 = view.buttons.iter().map(|b| Button::width_for(kit, b.label, b.icon.is_some())).sum::<f32>() + gap * (view.buttons.len() - 1) as f32;
            inner = inner.max(w);
        }
        let w = (inner + 2.0 * pad).max(560.0 * (1.0 + 0.5 * (kit.text_scale - 1.0))).min(1500.0).ceil();
        (w, pad, inner)
    }

    /// Where each button of `widths` goes in rows of `room` pixels: its row and its offset along the row.
    fn button_rows(widths: &[f32], gap: f32, room: f32) -> Vec<(usize, f32)> {
        let (mut row, mut x) = (0, 0.0);
        widths
            .iter()
            .map(|w| {
                if x > 0.0 && x + w > room {
                    row += 1;
                    x = 0.0;
                }
                let at = (row, x);
                x += w + gap;
                at
            })
            .collect()
    }

    fn build_panel(&self, kit: &mut Kit, engine: &mut Engine, view: &View, spot: (f32, f32), w: f32) -> std::io::Result<Panel> {
        let z = self.z + Z_PANEL;
        let (pad, gap) = (kit.space(6), kit.space(3));
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let title = role_style(kit, TypeRole::Title, None, ink, Align::Left);
        let body = role_style(kit, TypeRole::Body, None, ink, Align::Left);
        let flagged = view.items.iter().any(|i| i.flagged);
        let notice_ink = if flagged { kit.ui.look.colour(PaletteRole::Warning) } else { ink2 };
        let notice = role_style(kit, TypeRole::Label, None, notice_ink, Align::Left);
        let inner = w - 2.0 * pad;
        let mut y = pad;
        let mut layout: Vec<(Frame, Style, String)> = Vec::new();
        let mut heading_icon = None;
        if let Some(h) = view.heading {
            let th = kit.line(TypeRole::Title);
            let side = (36.0 * (1.0 + 0.6 * (kit.text_scale - 1.0))).round();
            heading_icon = Some(((spot.0 + pad, spot.1 + y + ((th - side) / 2.0).round().max(0.0)), side as u32));
            layout.push((Frame::new(pad + side + gap, y, inner - side - gap, th), title, h.to_string()));
            y += th.max(side) + gap;
        }
        let bh = kit.line(TypeRole::Body);
        layout.push((Frame::new(pad, y, inner, bh), body, view.subject.clone()));
        y += bh + gap;
        if !view.notice.is_empty() {
            let lh = kit.line(TypeRole::Label);
            layout.push((Frame::new(pad, y, inner, lh), notice, view.notice.clone()));
            y += lh + gap;
        }
        let buttons_y = y;
        // the buttons that do not fit in a row go on to another (five of them at 200 % text are wider than the panel can be)
        let widths: Vec<f32> = view.buttons.iter().map(|b| Button::width_for(kit, b.label, b.icon.is_some())).collect();
        let at = Self::button_rows(&widths, gap, inner);
        if !view.buttons.is_empty() {
            let rows = at.last().map_or(0, |r| r.0 + 1);
            y += rows as f32 * (Button::height(kit) + gap);
        }
        let foot_y = y;
        // the keys' rows: room for as many rows as the hints need at this width
        let caption = TypeRole::Caption;
        let hint_h = crate::kit::hint::glyph_side(kit, caption).max(kit.line(caption) as u32) as f32;
        let hints = Self::hints_of(view);
        let sp = kit.space(5);
        let (mut rows, mut cur) = (1usize, 0f32);
        for h in &hints {
            let hw = hint_width(kit, h, caption);
            if cur > 0.0 && cur + sp + hw > inner {
                rows += 1;
                cur = 0.0;
            }
            cur += if cur > 0.0 { sp + hw } else { hw };
        }
        let height = foot_y + rows as f32 * hint_h + (rows - 1) as f32 * kit.space(1) + pad;
        let frame = Frame::new(spot.0, spot.1, w, height);
        let mut sheet = Sheet::new();
        sheet.pane(Pane::new(kit, frame, z, PaneKind::Sheet, false));
        if let Some((at, side)) = heading_icon {
            if let Some(g) = icon::layer(kit, "layout-grid", side, ink, z + 1, HIDDEN)? {
                sheet.image(g, at);
            }
        }
        let mut words = Vec::new();
        for (f, style, text) in layout {
            let frame = Frame::new(spot.0 + f.x, spot.1 + f.y, f.w, f.h);
            sheet.text(Text::new(kit, frame, z + 1, style, &text)?);
            words.push(Word { text, frame, style });
        }
        let mut buttons = Vec::new();
        if !view.buttons.is_empty() {
            let bh = Button::height(kit);
            for (i, b) in view.buttons.iter().enumerate() {
                let bw = widths[i];
                let (row, x) = at[i];
                let tone = if b.primary { Tone::Primary } else { Tone::Plain };
                let mut spec = Spec::new(b.label, tone);
                spec.enabled = b.enabled;
                spec.icon = b.icon;
                spec.selected = b.selected;
                let place = Place { id: id(i), frame: Frame::new(spot.0 + pad + x, spot.1 + buttons_y + row as f32 * (bh + gap), bw, bh), region: REGION, scope: SCOPE, z: z + 2 };
                buttons.push(Button::new(kit, engine, place, spec)?);
            }
        }
        let paint = kit.ui.look.colour(PaletteRole::Sheet);
        Ok(Panel { sig: String::new(), sheet, buttons, frame, paint, words, foot: (spot.0 + pad, spot.1 + foot_y, inner) })
    }

    fn build_hints(kit: &mut Kit, z: i32, view: &View, foot: (f32, f32, f32), sig: String) -> std::io::Result<Hints> {
        let ink2 = kit.ui.look.ink2();
        let caption = TypeRole::Caption;
        let hint_h = crate::kit::hint::glyph_side(kit, caption).max(kit.line(caption) as u32) as f32;
        let sp = kit.space(5);
        let mut sheet = Sheet::new();
        let mut words = Vec::new();
        let (mut x, mut y) = (foot.0, foot.1);
        for h in Self::hints_of(view) {
            let hw = hint_width(kit, &h, caption);
            if x > foot.0 && x + hw > foot.0 + foot.2 {
                x = foot.0;
                y += hint_h + kit.space(1);
            }
            hint_into(kit, &mut sheet, &h, caption, x, y, hint_h, z, ink2)?;
            if let Some(label) = &h.label {
                let style = role_style(kit, caption, None, ink2, Align::Left);
                let lw = width_of(kit, style, label);
                words.push(Word { text: label.clone(), frame: Frame::new(x + hw - lw, y + ((hint_h - kit.line(caption)) / 2.0).round(), lw, kit.line(caption)), style });
            }
            x += hw + sp;
        }
        sheet.show(kit, true);
        Ok(Hints { sig, sheet, words })
    }

    fn sync_panel(&mut self, kit: &mut Kit, engine: &mut Engine, view: &View, editor: &Editor, now_ns: u64) -> std::io::Result<()> {
        let (w, _, _) = Self::panel_plan(kit, view);
        // a taller panel than the plan knows: measured by building it once at the spot the editor would give a panel of this width
        let probe_h = if view.heading.is_some() { 330.0 } else { 220.0 };
        let spot = editor.panel_spot(w as i32, (probe_h * (1.0 + 0.8 * (kit.text_scale - 1.0))) as i32);
        let spot = (spot.x as f32, spot.y as f32);
        let sig = format!(
            "{:?}",
            (view.mode, view.heading, &view.subject, &view.notice, view.items.iter().any(|i| i.flagged), view.buttons.iter().map(|b| (b.label, b.enabled, b.icon, b.selected)).collect::<Vec<_>>(), spot, kit.generation(), w as i32)
        );
        let changed = self.panel.as_ref().is_none_or(|p| p.sig != sig);
        if changed {
            let first = self.panel.is_none();
            if let Some(old) = self.panel.take() {
                for b in &old.buttons {
                    b.detach(engine);
                }
            }
            self.hints = None;
            let mut p = self.build_panel(kit, engine, view, spot, w)?;
            p.sig = sig;
            p.sheet.show(kit, !first);
            for b in &mut p.buttons {
                b.show(kit, !first);
            }
            self.panel = Some(p);
        }
        let p = self.panel.as_ref().expect("built above");
        let hsig = format!("{:?}", (view.mode, view.focus, kit.generation(), p.sig.len()));
        if self.hints.as_ref().is_none_or(|h| h.sig != hsig) {
            let h = Self::build_hints(kit, self.z + Z_PANEL + 3, view, p.foot, hsig)?;
            self.hints = Some(h);
        }
        // the editor's focus, followed with the flowing lift: a button while the buttons have it, none while the items do
        let want = if view.focus == "buttons" { view.buttons.iter().position(|b| b.focused) } else { None };
        match want {
            Some(i) if engine.focus.current() != Some(id(i)) => {
                engine.focus(id(i), now_ns);
            }
            None if engine.focus.current().is_some() => engine.clear(now_ns),
            _ => {}
        }
        Ok(())
    }

    /// Draws `view`. Returns whether anything is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, view: &View, editor: &Editor, now_ms: u64, now_ns: u64) -> std::io::Result<bool> {
        let _ = now_ms;
        if self.stat.as_ref().is_some_and(|s| s.generation != kit.generation()) {
            self.reset(engine);
        }
        if !self.modal {
            engine.focus.open_modal(SCOPE);
            self.modal = true;
        }
        if self.stat.as_ref().is_some_and(|s| s.words != Self::row_words(view)) {
            self.stat = None;
        }
        if self.stat.is_none() {
            self.stat = Some(self.build_stat(kit, view)?);
        }
        self.sync_items(kit, view)?;
        self.sync_deco(kit, view);
        self.sync_tag(kit, view)?;
        self.sync_panel(kit, engine, view, editor, now_ns)?;
        self.rects = view.items.iter().map(|i| (i.id, rect_frame(&i.glass))).collect();
        self.last = json!({
            "mode": view.mode,
            "focus": view.focus,
            "selected": view.selected,
            "subject": view.subject,
            "notice": view.notice,
            "customised": view.customised,
            "items": view.items.iter().map(|i| json!({"id": i.id, "x": i.rect.x, "y": i.rect.y, "w": i.rect.w, "h": i.rect.h, "glass": {"x": i.glass.x, "y": i.glass.y, "w": i.glass.w, "h": i.glass.h}, "words": i.words, "selected": i.selected, "inHand": i.in_hand, "waits": i.waits, "custom": i.custom})).collect::<Vec<_>>(),
            "panel": self.panel.as_ref().map(|p| json!({"x": p.frame.x, "y": p.frame.y, "w": p.frame.w, "h": p.frame.h})),
            "buttons": self.panel.as_ref().map(|p| p.buttons.iter().map(|b| { let f = b.frame(); json!({"x": f.x, "y": f.y, "w": f.w, "h": f.h, "label": b.spec().label, "enabled": b.spec().enabled}) }).collect::<Vec<_>>()),
        });
        if kit.ui.scene.take_overflow() {
            return Err(std::io::Error::other("the scene is full: the layout editor could not be drawn whole"));
        }
        Ok(true)
    }

    fn reset(&mut self, engine: &mut Engine) {
        if let Some(p) = self.panel.take() {
            for b in &p.buttons {
                b.detach(engine);
            }
        }
        self.stat = None;
        self.items.clear();
        self.deco = Deco::default();
        self.tag = None;
        self.hints = None;
    }

    /// The editor is closed: everything is dropped at once and the modal scope given back.
    pub fn close(&mut self, engine: &mut Engine) {
        self.reset(engine);
        if self.modal {
            engine.focus.close_modal();
            self.modal = false;
        }
        self.grab = None;
        self.hover = None;
        self.was_down = false;
        self.last = Value::Null;
        self.rects.clear();
    }

    /// The look changed: the editor is built again at the next draw.
    pub fn restyle(&mut self, engine: &mut Engine) {
        self.reset(engine);
    }

    /// A key of the remote: the pointer's cue goes.
    pub fn on_key(&mut self, kit: &Kit, engine: &mut Engine) {
        self.pointer.key(engine);
        self.hover = None;
        if let Some(p) = &mut self.panel {
            for b in &mut p.buttons {
                b.set_hover(kit, false);
            }
        }
    }

    /// The item under (`x`, `y`): the one in the hand or chosen first, else the last drawn.
    fn item_at(&self, x: f32, y: f32, selected: &str) -> Option<&'static str> {
        let hit = |f: &Frame| x >= f.x && x < f.x + f.w && y >= f.y && y < f.y + f.h;
        self.rects.iter().filter(|(_, f)| hit(f)).max_by_key(|(i, _)| usize::from(*i == selected)).map(|(i, _)| *i)
    }

    /// A sample of the pointer. Buttons are the kit's (hover is a cue, a click presses); an item is taken, followed and put down by the editor.
    /// Returns the editor's effects.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, now_ms: u64, sample: Sample, editor: &mut Editor) -> Vec<lumatv_placement::Effect> {
        let mut out = Vec::new();
        let mut on_button = false;
        for g in self.pointer.feed(engine, now_ns, sample) {
            match g {
                Gesture::Hover { from, to } => {
                    if let Some(p) = &mut self.panel {
                        for b in &mut p.buttons {
                            let on = Some(b.id) == to && engine.focus.current() != Some(b.id);
                            if Some(b.id) == from || Some(b.id) == to {
                                b.set_hover(kit, on);
                            }
                        }
                    }
                }
                Gesture::Down(i) => {
                    on_button = true;
                    if let Some(b) = self.panel.as_mut().and_then(|p| p.buttons.iter_mut().find(|b| b.id == i)) {
                        b.set_pressed(kit, true);
                    }
                }
                Gesture::Up(Some(i)) => {
                    if let Some(b) = self.panel.as_mut().and_then(|p| p.buttons.iter_mut().find(|b| b.id == i)) {
                        b.set_pressed(kit, false);
                    }
                }
                Gesture::Up(None) => {}
                Gesture::Click(i) => {
                    if let Some(index) = (0..self.panel.as_ref().map_or(0, |p| p.buttons.len())).find(|n| id(*n) == i) {
                        out.extend(editor.press_button(index, now_ms));
                    }
                }
            }
        }
        // the items: not the engine's focusables, the editor's own
        let down = sample.buttons & 1 != 0;
        let over_button = engine.focus.hit(sample.x, sample.y).is_some();
        let selected = editor.selected();
        if sample.visible {
            if down && !self.was_down && !on_button && !over_button {
                if let Some(item) = self.item_at(sample.x, sample.y, selected) {
                    editor.pointer_down(item, f64::from(sample.x), f64::from(sample.y));
                    self.grab = Some(item);
                }
            } else if down && self.grab.is_some() {
                editor.pointer_move(f64::from(sample.x), f64::from(sample.y), true, now_ms);
            }
            let hover = if over_button || down { None } else { self.item_at(sample.x, sample.y, "") };
            self.dirty |= hover != self.hover || (down && self.grab.is_some());
            self.hover = hover;
        } else {
            self.hover = None;
        }
        if !down && self.was_down && self.grab.take().is_some() {
            out.extend(editor.pointer_up());
        }
        self.was_down = down && sample.visible;
        out
    }

    /// Whether the pointer changed what is drawn since the last call.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    pub fn geometry(&self) -> Value {
        self.last.clone()
    }

    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        let Some(p) = &self.panel else { return };
        let panel = lint.panel("layout panel", p.frame, p.paint, true);
        for (i, w) in p.words.iter().enumerate() {
            lint.word(kit, &format!("layout panel words {i}"), &w.text, w.frame, w.style, Some(panel));
        }
        for b in &p.buttons {
            b.feed_lint(kit, lint, panel);
        }
        if let Some(h) = &self.hints {
            for (i, w) in h.words.iter().enumerate() {
                lint.word(kit, &format!("layout hint {i}"), &w.text, w.frame, w.style, Some(panel));
            }
        }
        if let Some(t) = &self.tag {
            // the pill's words, with the pill's air on each side of them (its glyph is the lead of the pill, not a run of words)
            let w = &t.words[0].frame;
            let pad = kit.space(4);
            let tp = lint.panel("layout tag", Frame::new(w.x - pad, t.at.1, w.w + 2.0 * pad, t.size.1), kit.ui.look.colour(PaletteRole::Sheet), false);
            for (i, w) in t.words.iter().enumerate() {
                lint.word(kit, &format!("layout tag words {i}"), &w.text, w.frame, w.style, Some(tp));
            }
        }
    }

    /// The layers the editor holds (the budget).
    pub fn layers(&self) -> usize {
        self.stat.iter().map(|s| s.sheet.layers()).sum::<usize>()
            + self.items.iter().map(|s| s.sheet.layers()).sum::<usize>()
            + self.panel.iter().map(|p| p.sheet.layers() + p.buttons.iter().map(Button::layers).sum::<usize>()).sum::<usize>()
            + self.hints.iter().map(|h| h.sheet.layers()).sum::<usize>()
            + self.tag.iter().map(|t| t.sheet.layers()).sum::<usize>()
            + [&self.deco.ring, &self.deco.ghost, &self.deco.reach, &self.deco.hover].iter().filter(|m| m.is_some()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use lumatv_placement::editor::Key as EKey;
    use lumatv_placement::{Frame as PFrame, Registry, Sizes, User};
    use ui::lucent::palette::Mode;

    const NS: u64 = 1_000_000;

    fn sizes(scale: f32) -> Sizes {
        let mut s = Sizes::new();
        let k = |v: i32| (v as f32 * (1.0 + 0.6 * (scale - 1.0))).round() as i32;
        s.set("volume", lumatv_placement::Size::new(440, k(60)));
        for edge in [lumatv_placement::Anchor::Left, lumatv_placement::Anchor::Right] {
            s.set_edge("volume", edge, lumatv_placement::Size::new(k(60), 440));
        }
        s.set("mute", lumatv_placement::Size::new(64, 64));
        s.set("popup", lumatv_placement::Size::new(560, k(469)));
        s.set("cards", lumatv_placement::Size::new(560, k(165)));
        s.set("toast", lumatv_placement::Size::new(560, k(98)));
        s.set("format", lumatv_placement::Size::new(k(392), k(60)));
        s.set("statusbar", lumatv_placement::Size::new((450.0 * scale) as i32, (60.0 * scale) as i32));
        s
    }

    fn rig(mode: Mode, scale: f32) -> (ui::testing::Harness, Kit, Engine, LayoutSurface, Editor) {
        let (h, kit) = testkit::kit_in(mode, scale);
        let engine = Engine::new(&kit).expect("an engine");
        let layout: Value = serde_json::from_str(include_str!("../../../../payload/theme/layout.json")).unwrap();
        let editor = Editor::new(Registry::shipped().clone(), User::default(), PFrame::from_json(&layout, None), sizes(scale));
        (h, kit, engine, LayoutSurface::new(1500), editor)
    }

    fn draw(kit: &mut Kit, engine: &mut Engine, s: &mut LayoutSurface, e: &Editor, now: u64) {
        let view = e.view(now);
        s.apply(kit, engine, &view, e, now, now * NS).expect("the editor draws");
    }

    fn lint_of(kit: &mut Kit, s: &LayoutSurface) -> Vec<String> {
        let mut lint = Lint::new();
        s.feed_lint(kit, &mut lint);
        lint.findings().iter().map(ToString::to_string).collect()
    }

    fn rect(v: &Value) -> (f64, f64, f64, f64) {
        (v["x"].as_f64().unwrap(), v["y"].as_f64().unwrap(), v["w"].as_f64().unwrap(), v["h"].as_f64().unwrap())
    }

    fn meets(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> bool {
        a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
    }

    #[test]
    fn the_five_items_are_drawn_where_the_placement_says_with_a_panel_that_covers_none_and_reads_clean_in_both_materials_and_sizes() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (_h, mut kit, mut engine, mut s, e) = rig(mode, scale);
                draw(&mut kit, &mut engine, &mut s, &e, 1000);
                let g = s.geometry();
                let items = g["items"].as_array().unwrap();
                assert_eq!(items.len(), 7, "volume, mute, camera window, notifications, toasts, format badge, status bar");
                let popup = items.iter().find(|i| i["id"] == "popup").unwrap();
                assert_eq!((rect(popup).0 + rect(popup).2, rect(popup).1), (1888.0, 32.0), "the camera window flush in its corner at {scale}: {popup}");
                let panel = rect(&g["panel"]);
                for i in items {
                    if i["waits"] != true {
                        assert!(!meets(panel, rect(i)), "the panel covers no item ({}) at {scale}: {panel:?}", i["id"]);
                    }
                }
                assert_eq!(g["buttons"].as_array().unwrap().len(), 3, "Reset this, Reset all, Done");
                assert!(g["buttons"][0]["enabled"] == false, "nothing placed: nothing to reset");
                let found = lint_of(&mut kit, &s);
                assert!(found.is_empty(), "{mode:?} at {scale}: {found:?}");
                assert!(s.layers() < 150, "the editor keeps to its share of the scene: {}", s.layers());
            }
        }
    }

    #[test]
    fn the_keys_choose_pick_up_and_move_an_item_and_the_picture_follows_and_back_puts_it_back() {
        let (_h, mut kit, mut engine, mut s, mut e) = rig(Mode::Dark, 1.0);
        draw(&mut kit, &mut engine, &mut s, &e, 1000);
        assert_eq!(s.geometry()["selected"], "volume");
        e.key(EKey::Right, 1100);
        e.key(EKey::Right, 1150);
        draw(&mut kit, &mut engine, &mut s, &e, 1200);
        assert_eq!(s.geometry()["selected"], "popup");
        assert!(s.geometry()["subject"].as_str().unwrap().starts_with("Camera pop-up"));
        let before = rect(s.geometry()["items"].as_array().unwrap().iter().find(|i| i["id"] == "popup").unwrap());
        e.key(EKey::Ok, 1300);
        e.key(EKey::Left, 1400);
        draw(&mut kit, &mut engine, &mut s, &e, 1450);
        let g = s.geometry();
        assert_eq!(g["mode"], "move");
        assert!(g["buttons"].is_null() || g["buttons"].as_array().is_some_and(Vec::is_empty), "no buttons while it is in the hand: {}", g["buttons"]);
        let moved = rect(g["items"].as_array().unwrap().iter().find(|i| i["id"] == "popup").unwrap());
        assert!(moved.0 < before.0, "a press of Left moved it left: {before:?} -> {moved:?}");
        e.key(EKey::Back, 1500);
        draw(&mut kit, &mut engine, &mut s, &e, 1550);
        let back = rect(s.geometry()["items"].as_array().unwrap().iter().find(|i| i["id"] == "popup").unwrap());
        assert_eq!(back, before, "Back put it back");
        assert!(lint_of(&mut kit, &s).is_empty());
    }

    #[test]
    fn a_refused_move_names_the_item_in_the_way_in_the_panel_and_the_panel_reads_clean() {
        let (_h, mut kit, mut engine, mut s, mut e) = rig(Mode::Dark, 1.0);
        draw(&mut kit, &mut engine, &mut s, &e, 1000);
        // the mute mark is picked up and sent along the top toward the camera window until something is in the way
        e.key(EKey::Right, 1050);
        e.key(EKey::Ok, 1100);
        let mut now = 1200;
        while e.view(now).notice.is_empty() && now < 1200 + 400 * 300 {
            e.key(EKey::Right, now);
            now += 300;
        }
        assert!(e.view(now).notice.contains("in the way"), "the window stops it: {}", e.view(now).notice);
        draw(&mut kit, &mut engine, &mut s, &e, now);
        assert!(s.geometry()["notice"].as_str().unwrap().contains("in the way"));
        let found = lint_of(&mut kit, &s);
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn the_volume_bar_turned_onto_the_left_edge_is_drawn_standing_and_flush_with_it() {
        let (_h, mut kit, mut engine, mut s, mut e) = rig(Mode::Dark, 1.0);
        draw(&mut kit, &mut engine, &mut s, &e, 1000);
        let bar = |s: &LayoutSurface| rect(s.geometry()["items"].as_array().unwrap().iter().find(|i| i["id"] == "volume").unwrap());
        let (_, _, w0, h0) = bar(&s);
        assert!(w0 > h0, "lying along the bottom edge");
        e.key(EKey::Ok, 1100);
        // along the bottom to its end, one press more turns the corner onto the left edge
        let mut now = 1200;
        for _ in 0..400 {
            e.key(EKey::Left, now);
            now += 300;
            if e.view(now).items.iter().any(|i| i.id == "volume" && i.rect.w < i.rect.h) {
                break;
            }
        }
        draw(&mut kit, &mut engine, &mut s, &e, now);
        let (x, _, w, h) = bar(&s);
        assert!(h > w, "standing on a side edge: {w} by {h}");
        assert_eq!(x, 64.0, "flush with the left edge of the zone");
        assert!(lint_of(&mut kit, &s).is_empty());
    }

    #[test]
    fn done_closes_it_with_a_click_and_the_modal_scope_is_given_back() {
        let (_h, mut kit, mut engine, mut s, mut e) = rig(Mode::Dark, 1.0);
        draw(&mut kit, &mut engine, &mut s, &e, 1000);
        assert!(s.is_open() && engine.focus.modal().is_some());
        let g = s.geometry();
        let done = &g["buttons"][2];
        let (x, y) = (done["x"].as_f64().unwrap() as f32 + 10.0, done["y"].as_f64().unwrap() as f32 + 10.0);
        let sample = |buttons| Sample { x, y, buttons, visible: true };
        s.pointer(&kit, &mut engine, 2 * NS, 2000, sample(0), &mut e);
        s.pointer(&kit, &mut engine, 3 * NS, 2010, sample(1), &mut e);
        let out = s.pointer(&kit, &mut engine, 4 * NS, 2020, sample(0), &mut e);
        assert_eq!(out, vec![lumatv_placement::Effect::Close], "a click on Done leaves");
        s.close(&mut engine);
        assert!(!s.is_open() && engine.focus.modal().is_none(), "the scope is given back");
        assert_eq!(s.layers(), 0, "nothing is kept");
    }

    #[test]
    fn a_drag_with_the_pointer_takes_an_item_and_puts_it_down_where_the_pointer_lets_go() {
        let (_h, mut kit, mut engine, mut s, mut e) = rig(Mode::Dark, 1.0);
        draw(&mut kit, &mut engine, &mut s, &e, 1000);
        let g = s.geometry();
        let mute = rect(g["items"].as_array().unwrap().iter().find(|i| i["id"] == "mute").unwrap());
        let (x, y) = (mute.0 as f32 + 20.0, mute.1 as f32 + 20.0);
        let at = |x: f32, y: f32, buttons| Sample { x, y, buttons, visible: true };
        s.pointer(&kit, &mut engine, 2 * NS, 2000, at(x, y, 0), &mut e);
        s.pointer(&kit, &mut engine, 3 * NS, 2010, at(x, y, 1), &mut e);
        s.pointer(&kit, &mut engine, 4 * NS, 2020, at(x + 300.0, y + 100.0, 1), &mut e);
        s.pointer(&kit, &mut engine, 5 * NS, 2030, at(x + 600.0, y + 200.0, 1), &mut e);
        let out = s.pointer(&kit, &mut engine, 6 * NS, 2040, at(x + 600.0, y + 200.0, 0), &mut e);
        assert!(out.iter().any(|f| matches!(f, lumatv_placement::Effect::Save(_))), "put down where it let go: {out:?}");
        draw(&mut kit, &mut engine, &mut s, &e, 2100);
        let after = rect(s.geometry()["items"].as_array().unwrap().iter().find(|i| i["id"] == "mute").unwrap());
        assert!(after.0 > mute.0 + 200.0 && after.1 > mute.1 + 50.0, "it moved with the pointer: {mute:?} -> {after:?}");
    }

    #[test]
    fn full_width_on_the_status_bar_is_a_switch_in_the_panel_and_draws_its_glass_across_the_row_with_the_buttons_where_they_were() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (_h, mut kit, mut engine, mut s, mut e) = rig(mode, scale);
                e.start(Some("statusbar"));
                draw(&mut kit, &mut engine, &mut s, &e, 1000);
                let bar = |g: &Value| g["items"].as_array().unwrap().iter().find(|i| i["id"] == "statusbar").unwrap().clone();
                let g = s.geometry();
                let (own, glass) = (rect(&bar(&g)), rect(&bar(&g)["glass"]));
                assert_eq!(own, glass, "{mode:?} at {scale}: off, the glass is the buttons' own box");
                assert!(s.stat.as_ref().expect("the picture").words, "{mode:?} at {scale}: the row is named while the glass hugs the buttons");
                let labels: Vec<&str> = g["buttons"].as_array().unwrap().iter().map(|b| b["label"].as_str().unwrap()).collect();
                assert_eq!(labels, ["Full width", "Notification ticker", "Reset this", "Reset all", "Done"], "{mode:?} at {scale}");
                assert!(g["buttons"][1]["enabled"] == false, "{mode:?} at {scale}: the ticker is a part of the full bar");
                // a click on the switch spreads the glass over the whole row
                let b = &g["buttons"][0];
                let (x, y) = (b["x"].as_f64().unwrap() as f32 + 10.0, b["y"].as_f64().unwrap() as f32 + 10.0);
                let sample = |buttons| Sample { x, y, buttons, visible: true };
                s.pointer(&kit, &mut engine, 2 * NS, 2000, sample(0), &mut e);
                s.pointer(&kit, &mut engine, 3 * NS, 2010, sample(1), &mut e);
                let out = s.pointer(&kit, &mut engine, 4 * NS, 2020, sample(0), &mut e);
                assert!(matches!(&out[..], [lumatv_placement::Effect::Save(v)] if v["statusbar"]["full"] == true), "{mode:?} at {scale}: a click on Full width saves it: {out:?}");
                draw(&mut kit, &mut engine, &mut s, &e, 2100);
                let g = s.geometry();
                assert!(!s.stat.as_ref().expect("the picture").words, "{mode:?} at {scale}: and not once the glass covers it, as the words would show through");
                let (held, spread) = (rect(&bar(&g)), rect(&bar(&g)["glass"]));
                assert_eq!((spread.0, spread.0 + spread.2, spread.3), (64.0, 1856.0, 60.0 * f64::from(scale)), "{mode:?} at {scale}: the glass spans the safe area's width: {spread:?}");
                assert_eq!(held, own, "{mode:?} at {scale}: the buttons did not move");
                assert!(g["subject"].as_str().unwrap().ends_with("full width"), "{}", g["subject"]);
                // and its ticker can be turned off there: it is on until then
                assert!(g["buttons"][1]["enabled"] == true, "{mode:?} at {scale}");
                let t = &g["buttons"][1];
                let (tx, ty) = (t["x"].as_f64().unwrap() as f32 + 10.0, t["y"].as_f64().unwrap() as f32 + 10.0);
                let tap = |buttons| Sample { x: tx, y: ty, buttons, visible: true };
                s.pointer(&kit, &mut engine, 5 * NS, 2200, tap(0), &mut e);
                s.pointer(&kit, &mut engine, 6 * NS, 2210, tap(1), &mut e);
                let out = s.pointer(&kit, &mut engine, 7 * NS, 2220, tap(0), &mut e);
                assert!(matches!(&out[..], [lumatv_placement::Effect::Save(v)] if v["statusbar"]["ticker"] == false), "{mode:?} at {scale}: a click on Notification ticker saves it off: {out:?}");
                draw(&mut kit, &mut engine, &mut s, &e, 2300);
                // the pointer takes it by the glass where there are no buttons
                assert!(s.rects.iter().any(|(id, f)| *id == "statusbar" && f.x == 64.0 && f.w == 1792.0), "the glass is what the pointer finds: {:?}", s.rects);
                let found = lint_of(&mut kit, &s);
                assert!(found.is_empty(), "{mode:?} at {scale}: {found:?}");
                assert!(s.layers() < 150, "{}", s.layers());
            }
        }
    }
}
