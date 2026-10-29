//! What opens over the page: the help behind a row's info mark (`kit/LuHelp.qml`: a card by the row, over a scrim that hides nothing
//! but dims the panes) and the question (`panels/Confirm.qml`: a card in the middle over a scrim that hides the page, the safe answer
//! first). Both are drawn when they open and given back when they close; the question's answers are the kit's buttons, focusables of a
//! modal scope, so the flowing lift moves between them as everywhere.

use super::layout::Sizes;
use super::list::ListArt;
use super::paint::{Paint, tile_pixels};
use super::rows::{other, pill};
use super::{PICK_ID, REGION_SHEET, SHEET_ID, SHEET_SCOPE};
use crate::kit::button::{Button, Place, Spec, Tone};
use crate::kit::focus::Id;
use crate::kit::motion::Beat;
use crate::kit::text::{Text, role_style};
use crate::kit::{Engine, Kit, icon};
use crate::settings::row::Row;
use crate::settings::view::SheetView;
use compositor_protocol::Props;
use lumatv_tokens::{Radius, TypeRole};
use ui::components::Align;
use ui::components::plate;
use ui::components::text::{Run, line_height, wrap};
use ui::components::text_layer::{Item, TextLayer};
use ui::geometry::Frame;
use ui::scene::{ImageLayer, ShapeLayer};

struct Help {
    sig: String,
    _scrim: ShapeLayer,
    _back: ShapeLayer,
    _card: ShapeLayer,
    _icon: Option<ImageLayer>,
    _title: Text,
    _body: TextLayer,
}

struct Confirm {
    sig: String,
    _scrim: ShapeLayer,
    _card: ShapeLayer,
    _tile: Option<ImageLayer>,
    _title: TextLayer,
    _body: TextLayer,
    buttons: Vec<Button>,
    frame: Frame,
}

/// The small picker by the row that asked: a card with its title and the options as rows of a list of its own.
struct PickCard {
    sig: String,
    _back: ShapeLayer,
    _card: ShapeLayer,
    _title: Text,
    list: ListArt,
    frame: Frame,
}

pub struct Sheets {
    z: i32,
    help: Option<Help>,
    confirm: Option<Confirm>,
    pick: Option<PickCard>,
    /// A press or its release on a button, applied at the next draw (the pointer's samples have no kit).
    pressed: Vec<(Id, bool)>,
}

impl Sheets {
    pub fn new(z: i32) -> Self {
        Sheets { z, help: None, confirm: None, pick: None, pressed: Vec::new() }
    }

    pub fn card_contains(&self, x: f32, y: f32) -> bool {
        self.confirm.as_ref().is_some_and(|c| c.frame.contains(x, y)) || self.pick.as_ref().is_some_and(|p| p.frame.contains(x, y))
    }

    pub fn set_pressed(&mut self, id: Id, on: bool) {
        self.pressed.push((id, on));
    }

    pub fn release(&mut self, engine: &mut Engine) {
        self.help = None;
        if let Some(c) = self.confirm.take() {
            for b in c.buttons {
                b.destroy(engine);
            }
        }
        self.drop_pick(engine);
    }

    fn drop_pick(&mut self, engine: &mut Engine) {
        if let Some(mut p) = self.pick.take() {
            p.list.release(engine);
        }
    }

    /// Makes what the view asks for and gives back what it no longer does. `anchor` is the row a help is about, on the screen, and
    /// `bounds` where its card may sit.
    #[allow(clippy::too_many_arguments)]
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, paint: Paint, sheet: Option<&SheetView>, anchor: Option<Frame>, bounds: Frame, hover: Option<usize>, now_ns: u64) -> std::io::Result<()> {
        for (id, on) in std::mem::take(&mut self.pressed) {
            if let Some(b) = self.confirm.as_mut().and_then(|c| c.buttons.iter_mut().find(|b| b.id == id)) {
                b.set_pressed(kit, on);
            }
        }
        match sheet {
            Some(SheetView::Pick { title, rows, index }) => {
                self.help = None;
                self.drop_confirm(engine);
                let sig = format!("{title}|{}", rows.iter().map(|r| format!("{}:{}:{}", r.label, r.selected, r.enabled)).collect::<Vec<_>>().join(","));
                if self.pick.as_ref().is_none_or(|p| p.sig != sig) {
                    self.drop_pick(engine);
                    let card = build_pick(kit, engine, paint, self.z + 40, sig, title, rows, anchor.unwrap_or(Frame::new(bounds.x, bounds.y, bounds.w, 0.0)), bounds)?;
                    self.pick = Some(card);
                }
                if let Some(p) = &mut self.pick {
                    p.list.apply(kit, engine, paint, rows, Some(*index), hover, 0, true, now_ns, false)?;
                }
            }
            Some(SheetView::Help { row, title, text }) => {
                self.drop_confirm(engine);
                self.drop_pick(engine);
                let sig = format!("{row}|{title}|{text}");
                if self.help.as_ref().is_none_or(|h| h.sig != sig) {
                    self.help = Some(build_help(kit, self.z, sig, title, text, anchor.unwrap_or(Frame::new(bounds.x, bounds.y, bounds.w, 0.0)), bounds)?);
                }
            }
            Some(SheetView::Confirm { icon, title, body, answers, index }) => {
                self.help = None;
                self.drop_pick(engine);
                let sig = format!("{icon}|{title}|{body}|{}", answers.iter().map(|a| format!("{}:{}", a.label, a.danger)).collect::<Vec<_>>().join(","));
                if self.confirm.as_ref().is_none_or(|c| c.sig != sig) {
                    self.drop_confirm(engine);
                    self.confirm = Some(build_confirm(kit, engine, self.z + 20, sig, icon, title, body, answers)?);
                }
                let _ = index;
            }
            Some(SheetView::Entry { .. }) | None => {
                self.help = None;
                self.drop_confirm(engine);
                self.drop_pick(engine);
            }
        }
        Ok(())
    }

    fn drop_confirm(&mut self, engine: &mut Engine) {
        if let Some(c) = self.confirm.take() {
            for b in c.buttons {
                b.destroy(engine);
            }
        }
    }
}

/// The help card by the row it is about: below it when it fits, above it when it does not, kept inside the panes.
fn build_help(kit: &mut Kit, z: i32, sig: String, title: &str, text: &str, anchor: Frame, bounds: Frame) -> std::io::Result<Help> {
    let sz = Sizes::of(kit);
    let paint = Paint::of(kit);
    let edge = sz.s4;
    let pad = sz.s6;
    let card_w = (sz.target * 12.0).min((bounds.w - 2.0 * edge).max(0.0)).round();
    let body_ty = kit.ty(TypeRole::Body);
    let lines = wrap(&mut kit.ui.fonts, text, body_ty, card_w - 2.0 * pad);
    let body_line = line_height(body_ty);
    let icon_side = (sz.target * 0.4).round();
    let head_h = kit.line(TypeRole::Label).max(icon_side);
    let card_h = (sz.s5 + head_h + sz.s2 + lines.len() as f32 * body_line + sz.s5).ceil();
    let below = anchor.bottom() + sz.s2;
    let fits = below + card_h <= bounds.bottom() - edge;
    let x = (anchor.x + sz.s6).min(bounds.right() - card_w - edge).max(bounds.x + edge);
    let y = (if fits { below } else { anchor.y - card_h - sz.s2 }).max(bounds.y + edge).round();
    let radius = kit.tokens.radius(Radius::Card);
    let extra = kit.tokens.extra(kit.mode);
    let scrim = plate::shape(&kit.ui, Frame::new(0.0, 0.0, 1920.0, 1080.0), z, plate::solid(paint.scrim, 0.0));
    let frame = Frame::new(x, y, card_w, card_h);
    let back = plate::shape(&kit.ui, frame, z + 1, plate::solid(extra.scrim_hide, radius));
    let card = plate::shape(&kit.ui, frame, z + 2, pill(paint.sheet, paint.edge_raised, radius, 1.0));
    let icon = icon::layer(kit, "info", icon_side as u32, paint.ink2, z + 4, Props::at(x + pad, y + sz.s5 + ((head_h - icon_side) / 2.0).round()))?;
    let style = role_style(kit, TypeRole::Label, None, paint.ink, Align::Left);
    let tx = x + pad + icon_side + sz.s3;
    let title_t = Text::new(kit, Frame::new(tx, y + sz.s5 + ((head_h - kit.line(TypeRole::Label)) / 2.0).round(), card_w - pad - (tx - x), kit.line(TypeRole::Label)), z + 4, style, title)?;
    let by = y + sz.s5 + head_h + sz.s2;
    let items: Vec<Item> = lines.iter().enumerate().map(|(i, l)| Item::Text(Run { text: l, ty: body_ty, colour: paint.ink, x: 0.0, y: i as f32 * body_line, w: card_w - 2.0 * pad, align: Align::Left })).collect();
    let body = TextLayer::new(&mut kit.ui, Frame::new(x + pad, by, card_w - 2.0 * pad, lines.len() as f32 * body_line), z + 4, &items)?;
    // in on the layer beat
    for (layer, at) in [(&*scrim, Props::at(0.0, 0.0)), (&*back, Props::at(x, y)), (&*card, Props::at(x, y))] {
        kit.arrive(layer, at, Beat::Layer, false);
    }
    if let Some(i) = &icon {
        kit.arrive(i, Props::at(x + pad, y + sz.s5 + ((head_h - icon_side) / 2.0).round()), Beat::Layer, false);
    }
    Ok(Help { sig, _scrim: scrim, _back: back, _card: card, _icon: icon, _title: title_t, _body: body })
}

/// The question: a card in the middle over a scrim that hides the page; the icon on an accent tile, the title, what happens, and the answers.
#[allow(clippy::too_many_arguments)]
fn build_confirm(kit: &mut Kit, engine: &mut Engine, z: i32, sig: String, icon_name: &str, title: &str, body: &str, answers: &[crate::settings::view::AnswerView]) -> std::io::Result<Confirm> {
    let sz = Sizes::of(kit);
    let paint = Paint::of(kit);
    let radius = kit.tokens.radius(Radius::Sheet);
    let extra = kit.tokens.extra(kit.mode);
    let card_w = 760.0;
    let inner = card_w - 2.0 * sz.s6;
    let tile = 56.0;
    let title_ty = kit.ty(TypeRole::Title);
    let title_lines = wrap(&mut kit.ui.fonts, title, title_ty, inner - tile - sz.s4);
    let title_line = line_height(title_ty);
    let head_h = tile.max(title_lines.len() as f32 * title_line);
    let body_ty = kit.ty(TypeRole::Body);
    let body_lines = if body.is_empty() { Vec::new() } else { wrap(&mut kit.ui.fonts, body, body_ty, inner) };
    let body_line = line_height(body_ty);
    let btn_h = Button::height(kit);
    let body_h = body_lines.len() as f32 * body_line;
    let card_h = (sz.s6 + head_h + sz.s4 + if body_lines.is_empty() { 0.0 } else { body_h + sz.s4 } + btn_h + sz.s6).ceil();
    let x = ((1920.0 - card_w) / 2.0).round();
    let y = ((1080.0 - card_h) / 2.0).round();
    let frame = Frame::new(x, y, card_w, card_h);
    let scrim = plate::shape(&kit.ui, Frame::new(0.0, 0.0, 1920.0, 1080.0), z, plate::solid(extra.scrim_hide, 0.0));
    let card = plate::shape(&kit.ui, frame, z + 1, pill(paint.sheet, paint.edge, radius, 1.0));
    // the icon on an accent tile
    let name = if icon_name.is_empty() { "circle-help" } else { icon_name };
    let glyph_side = (tile * 0.55).round() as u32;
    let g = kit.icons.draw(name, glyph_side, paint.accent_ink);
    let px = tile_pixels(tile as u32, (tile * 0.32).round(), paint.accent, plate::CLEAR, g.as_deref().map(|g| (g, glyph_side)));
    let mut tile_layer = kit.ui.scene.image(tile as u32, tile as u32, z + 3, Props::at(x + sz.s6, y + sz.s6 + ((head_h - tile) / 2.0).round()))?;
    tile_layer.pixels_mut().copy_from_slice(&px);
    tile_layer.damage(None);
    let tx = x + sz.s6 + tile + sz.s4;
    let items: Vec<Item> = title_lines.iter().enumerate().map(|(i, l)| Item::Text(Run { text: l, ty: title_ty, colour: paint.ink, x: 0.0, y: i as f32 * title_line, w: inner - tile - sz.s4, align: Align::Left })).collect();
    let title_t = TextLayer::new(&mut kit.ui, Frame::new(tx, y + sz.s6 + ((head_h - title_lines.len() as f32 * title_line) / 2.0).round(), inner - tile - sz.s4, title_lines.len() as f32 * title_line), z + 3, &items)?;
    let by = y + sz.s6 + head_h + sz.s4;
    let items: Vec<Item> = body_lines.iter().enumerate().map(|(i, l)| Item::Text(Run { text: l, ty: body_ty, colour: paint.ink2, x: 0.0, y: i as f32 * body_line, w: inner, align: Align::Left })).collect();
    let body_t = TextLayer::new(&mut kit.ui, Frame::new(x + sz.s6, by, inner, body_h.max(1.0)), z + 3, &items)?;
    // the answers
    let btn_y = y + card_h - sz.s6 - btn_h;
    let mut bx = x + sz.s6;
    let last_plain = answers.iter().rposition(|a| !a.danger);
    let mut buttons = Vec::new();
    for (i, a) in answers.iter().enumerate() {
        let tone = if a.danger { Tone::Danger } else if Some(i) == last_plain && (i > 0 || answers.len() == 1) { Tone::Primary } else { Tone::Plain };
        let w = Button::width_for(kit, &a.label, false);
        let place = Place { id: Id(SHEET_ID + i as u32), frame: Frame::new(bx, btn_y, w, btn_h), region: REGION_SHEET, scope: SHEET_SCOPE, z: z + 5 };
        let mut b = Button::new(kit, engine, place, Spec::new(&a.label, tone))?;
        b.show(kit, true);
        buttons.push(b);
        bx += w + sz.s4;
    }
    let _ = (other, line_height);
    Ok(Confirm { sig, _scrim: scrim, _card: card, _tile: Some(tile_layer), _title: title_t, _body: body_t, buttons, frame })
}

/// The picker: a card beside the row that asked (below it when it fits, above when it does not, its right edge on the row's), the title in
/// the secondary ink and the options as rows. A list longer than six options scrolls.
#[allow(clippy::too_many_arguments)]
fn build_pick(kit: &mut Kit, engine: &mut Engine, paint: Paint, z: i32, sig: String, title: &str, rows: &[Row], anchor: Frame, bounds: Frame) -> std::io::Result<PickCard> {
    let sz = Sizes::of(kit);
    let radius = kit.tokens.radius(Radius::Card);
    let extra = kit.tokens.extra(kit.mode);
    let pad = sz.s3;
    let w = 640.0f32.min(bounds.w - 2.0 * sz.s4).round();
    let title_line = kit.line(TypeRole::Label);
    let head = title_line + sz.s2;
    // whole rows only: a half row under the last would look cut off
    let fit = ((bounds.h - 2.0 * sz.s4 - head - 2.0 * pad) / sz.row).floor().clamp(1.0, 6.0);
    let list_h = (rows.len() as f32).min(fit) * sz.row;
    let h = (2.0 * pad + head + list_h).ceil();
    let below = anchor.bottom() + sz.s2;
    let fits = below + h <= bounds.bottom() - sz.s4;
    let x = (anchor.right() - w - sz.s6).max(bounds.x + sz.s4).min(bounds.right() - w - sz.s4).round();
    let y = (if fits { below } else { anchor.y - h - sz.s2 }).max(bounds.y + sz.s4).min(bounds.bottom() - h - sz.s4).round();
    let frame = Frame::new(x, y, w, h);
    let back = plate::shape(&kit.ui, frame, z, plate::solid(extra.scrim_hide, radius));
    let card = plate::shape(&kit.ui, frame, z + 1, pill(paint.sheet, paint.edge_raised, radius, 1.0));
    let style = role_style(kit, TypeRole::Label, None, paint.ink2, Align::Left);
    let title_t = Text::new(kit, Frame::new(x + pad + sz.s3, y + pad, w - 2.0 * pad - 2.0 * sz.s3, title_line), z + 3, style, title)?;
    let list = ListArt::new(kit, engine, None, "pick".into(), (x + pad, y + pad + head), (w - 2.0 * pad, list_h), REGION_SHEET, SHEET_SCOPE, z + 4, PICK_ID, false)?;
    Ok(PickCard { sig, _back: back, _card: card, _title: title_t, list, frame })
}
