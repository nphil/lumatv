//! The banners drawn (`panels/Banner.qml`, `panels/BannerButton.qml`, the mocks `20-channel-banner`, `25-input-banner`, `25-input-pill`, `18-no-signal`):
//!
//! - the **input pill**: a glass pill at the foot of the screen, 64 from the left, with the input's tile, its name and what it carries, and, when
//!   the spec asks for them, a divider and the Details, Inputs and Picture buttons;
//! - the **channel banner**: a glass bar, 1240 wide, with the channel's number and call sign on a tile, the programme, what is on after it and how much
//!   is left, and the signal facts as chips;
//! - the **no-signal screen**: black in both materials, the dim block (icon, port, what is wrong, what to do, Inputs and Home) at a spot that
//!   changes every minute, drawn in the dark material's inks whatever the TV's material is.
//!
//! The buttons are the kit's: focusables of the engine with the flowing lift, the hover cue and the press look. A banner with buttons opens a modal
//! scope for as long as it is up (so the keys are its), and the pill without buttons takes none. Everything hangs under one root that fades in and
//! out; the banner is made again when its words, its spot, the look or the text size change.

use super::banner::{Kind, SPOTS, Spec, View};
use super::parts::Part;
use crate::kit::avatar;
use crate::kit::button::{Button, Place, Spec as ButtonSpec, Tone, faded};
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::icon;
use crate::kit::lint::Lint;
use crate::kit::pane::{Pane, PaneKind};
use crate::kit::pointer::{Gesture, Pointer, Sample};
use crate::kit::text::{Text, role_style, width_of};
use crate::kit::widgets::{Px, edge, tile};
use crate::kit::{Engine, Kit, wrap};
use crate::panels::ui::{Arrive, Root};
use compositor_protocol::{Fill, GroupPose, Props, Rgba, Shape};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::{Value, json};
use ui::components::{Align, Style, plate};
use ui::geometry::Frame;
use ui::lucent::palette::{Mode, PaletteRole};
use ui::text::Weight;

pub const SCOPE: Scope = Scope(209);
const REGION: Region = Region(2900);
const BASE: u32 = 0x2900_0000;
const FADE_MS: u32 = 140;
/// The block of the no-signal screen is dim on purpose: a still frame on an OLED is what the screen must never hold.
const BLOCK_ALPHA: f32 = 0.62;
/// The channel banner's measures at 100 % text, in TV pixels: the number tile's side (the logo's plate is as high and 16:9, LG's own logos are
/// 356 by 200) and how many times as wide as that the tile may grow for a long number or name, the bar's width and least height, how wide it may
/// grow with the text, what share of it the chips may take and the progress bar's thickness. Everything else of its look (colours, radii, type,
/// spacing, depth, motion) is the theme's.
const CHANNEL_TILE: f32 = 104.0;
const CHANNEL_TILE_GROWS: f32 = 1.8;
const CHANNEL_WIDTH: f32 = 1240.0;
const CHANNEL_MAX_WIDTH: f32 = 1792.0;
const CHANNEL_MIN_HEIGHT: f32 = 148.0;
const CHANNEL_CHIPS_SHARE: f32 = 0.34;
const CHANNEL_BAR: f32 = 6.0;

/// Where the pointer can land: a button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Button(usize),
}

impl Target {
    fn id(i: usize) -> Id {
        Id(BASE + i as u32)
    }

    fn from_id(id: Id) -> Option<Target> {
        match id.0.checked_sub(BASE)? {
            i if i < 0x10 => Some(Target::Button(i as usize)),
            _ => None,
        }
    }
}

/// A word the lint is told of: where it is, in what style, on which panel (an index into `Built::panels`).
struct Word {
    name: String,
    text: String,
    frame: Frame,
    style: Style,
    panel: Option<usize>,
}

struct Built {
    sig: String,
    kind: Kind,
    parts: Vec<Part>,
    buttons: Vec<Button>,
    labels: Vec<String>,
    /// The panels the lint reads (name, rectangle, paint), and the words on them.
    panels: Vec<(String, Frame, Rgba, Option<usize>)>,
    words: Vec<Word>,
    /// Pictures that stand on a panel (a tile): name, rectangle, side.
    glyphs: Vec<(String, Frame, u32)>,
    geometry: Value,
    layers: usize,
    /// The group the no-signal block hangs under (dimmed as a whole).
    _groups: Vec<ui::group::Group>,
}

pub struct BannerSurface {
    z: i32,
    root: Option<Root>,
    built: Option<Built>,
    pointer: Pointer,
    modal: bool,
    /// The first focus after a banner is made is lifted at once.
    first: bool,
    leaving_until: Option<u64>,
    /// The button that has the focus, as the view said at the last draw.
    focus: Option<usize>,
}

/// Runs `f` with the dark material's look in force (the no-signal screen is black in both materials): what is made inside keeps the dark colours.
fn with_dark<R>(kit: &mut Kit, f: impl FnOnce(&mut Kit) -> R) -> R {
    let dark = kit.tokens.look(Mode::Dark, kit.generation(), kit.tier, kit.text_scale);
    let saved = std::mem::replace(&mut kit.ui.look, dark);
    let out = f(kit);
    kit.ui.look = saved;
    out
}

fn scaled(kit: &Kit, base: f32) -> f32 {
    (base * (1.0 + 0.6 * (kit.text_scale - 1.0))).round()
}

fn frame_json(f: Frame) -> Value {
    json!({"x": f.x, "y": f.y, "w": f.w, "h": f.h})
}

impl BannerSurface {
    pub fn new(z: i32) -> Self {
        Self { z, root: None, built: None, pointer: Pointer::new(), modal: false, first: true, leaving_until: None, focus: None }
    }

    pub fn is_up(&self) -> bool {
        self.root.is_some()
    }

    fn signature(kit: &Kit, v: &View, over_video: bool) -> String {
        format!("{}|{}|{:?}|{}|{}", kit.generation(), kit.text_scale, v.spec, if v.spec.kind == Kind::NoSignal { v.spot } else { 0 }, over_video)
    }

    /// The pill: the tile, the name and the facts, then the buttons; at the foot of the screen, 64 from the left.
    fn pill(kit: &mut Kit, engine: &mut Engine, z: i32, spec: &Spec, over_video: bool, b: &mut Built) -> std::io::Result<()> {
        let (s2, s3, s4, s7) = (kit.space(2), kit.space(3), kit.space(4), kit.space(7));
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let name_style = role_style(kit, TypeRole::Body, Some(Weight::SemiBold), ink, Align::Left);
        let facts_style = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
        let (name_h, facts_h) = (kit.line(TypeRole::Body), kit.line(TypeRole::Caption));
        let side = scaled(kit, 64.0);
        let text_w = width_of(kit, name_style, &spec.name).max(if spec.facts.is_empty() { 0.0 } else { width_of(kit, facts_style, &spec.facts) });
        let text_h = name_h + if spec.facts.is_empty() { 0.0 } else { facts_h };
        let btn_h = Button::height(kit);
        let widths: Vec<f32> = spec.buttons.iter().map(|p| Button::width_for(kit, p.label(), true)).collect();
        let buttons_w = widths.iter().sum::<f32>() + s3 * widths.len().saturating_sub(1) as f32;
        let content_h = side.max(text_h).max(if widths.is_empty() { 0.0 } else { btn_h });
        let h = (content_h + 2.0 * s4).max(96.0).ceil();
        let row_w = side + s4 + text_w + if widths.is_empty() { 0.0 } else { s4 + 1.0 + s4 + buttons_w };
        let w = (row_w + s4 + s7).max(420.0).ceil();
        let frame = Frame::new(64.0, 1080.0 - 56.0 - h, w, h);
        let paint = kit.ui.look.colour(PaletteRole::Sheet);
        b.parts.push(Part::Pane(Pane::new(kit, frame, z, PaneKind::Pill, over_video)));
        b.panels.push(("banner pill".into(), frame, paint, None));
        let cy = frame.y + h / 2.0;
        let mut x = frame.x + s4;
        let tile_icon = if kit.icons.has(&spec.icon) { spec.icon.clone() } else { "cable".to_string() };
        let t = tile(kit, side as u32, &tile_icon, false).layer(kit, z + 1, Props::at(x, (cy - side / 2.0).round()))?;
        b.parts.push(Part::Image(t));
        b.glyphs.push(("banner tile".into(), Frame::new(x, (cy - side / 2.0).round(), side, side), side as u32));
        x += side + s4;
        let block_y = cy - text_h / 2.0;
        let name = Text::new(kit, Frame::new(x, block_y.round(), text_w, name_h), z + 1, name_style, &spec.name)?;
        b.words.push(Word { name: "banner name".into(), text: spec.name.clone(), frame: name.frame(), style: name_style, panel: Some(0) });
        b.parts.push(Part::Text(name));
        if !spec.facts.is_empty() {
            let facts = Text::new(kit, Frame::new(x, (block_y + name_h).round(), text_w, facts_h), z + 1, facts_style, &spec.facts)?;
            b.words.push(Word { name: "banner facts".into(), text: spec.facts.clone(), frame: facts.frame(), style: facts_style, panel: Some(0) });
            b.parts.push(Part::Text(facts));
        }
        x += text_w;
        if !widths.is_empty() {
            x += s4;
            let divider = plate::shape(&kit.ui, Frame::new(x, (cy - 20.0).round(), 1.0, 40.0), z + 1, plate::solid(edge(kit), 0.0));
            b.parts.push(Part::Shape(divider));
            x += 1.0 + s4;
            for (i, (p, bw)) in spec.buttons.iter().zip(&widths).enumerate() {
                let f = Frame::new(x, (cy - btn_h / 2.0).round(), *bw, btn_h);
                let mut btn = Button::new(kit, engine, Place { id: Target::id(i), frame: f, region: REGION, scope: SCOPE, z: z + 3 }, ButtonSpec::new(p.label(), Tone::Plain).with_icon(p.icon()))?;
                btn.show(kit, true);
                b.buttons.push(btn);
                b.labels.push(p.label().to_string());
                x += bw + s3;
            }
        }
        b.geometry = json!({"kind": "input", "pill": frame_json(frame), "name": spec.name, "facts": spec.facts, "tile": side, "gap": s2});
        Ok(())
    }

    /// The channel banner: a glass bar at the foot of the screen: the number and name on a tile, the channel's logo on a plate of its own when
    /// the channel has one, the programme with its time, what is next and a bar of how much is left (and, for INFO, what it is about), and
    /// the programme's marks as chips. The measures are the bar's (`CHANNEL_*`), the rest is the theme's.
    fn channel(kit: &mut Kit, z: i32, spec: &Spec, over_video: bool, b: &mut Built) -> std::io::Result<()> {
        let (s2, s3, s5, s6) = (kit.space(2), kit.space(3), kit.space(5), kit.space(6));
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let number_style = role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Centre);
        let call_style = role_style(kit, TypeRole::Caption, Some(Weight::SemiBold), ink2, Align::Centre);
        let title_style = role_style(kit, TypeRole::Body, Some(Weight::SemiBold), ink, Align::Left);
        let sub_style = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
        let next_style = role_style(kit, TypeRole::Caption, None, ink2, Align::Right);
        let chip_style = role_style(kit, TypeRole::Caption, Some(Weight::SemiBold), ink2, Align::Left);
        let (title_h, cap_h, num_h) = (kit.line(TypeRole::Body), kit.line(TypeRole::Caption), kit.line(TypeRole::Title));
        let tile_side = scaled(kit, CHANNEL_TILE).max(num_h + cap_h + 2.0 * s3);
        // the number is read whole and the name as far as it goes: the tile widens for them, but only so far
        let words_w = width_of(kit, number_style, &spec.number).max(width_of(kit, call_style, &spec.call)).ceil() + 2.0 * s2;
        let tile_w = words_w.clamp(tile_side, (tile_side * CHANNEL_TILE_GROWS).round());
        let wide = (tile_side * 16.0 / 9.0).round();
        let logo = (!spec.logo.is_empty()).then(|| avatar::contained(&spec.logo, (wide - 2.0 * s2) as u32, (tile_side - 2.0 * s2) as u32)).flatten();
        let bar_h = CHANNEL_BAR;
        let w = (CHANNEL_WIDTH * (1.0 + 0.45 * (kit.text_scale - 1.0)) + (tile_w - tile_side) + if logo.is_some() { wide + s3 } else { 0.0 }).round().min(CHANNEL_MAX_WIDTH);
        // what the programme is about (INFO): two lines under the bar
        let detail_lines = if spec.detail.is_empty() { 0 } else { 2 };
        let inner_h = title_h + s2 + cap_h + s2 + bar_h + if detail_lines > 0 { s2 + detail_lines as f32 * cap_h } else { 0.0 };
        let h = (tile_side.max(inner_h) + 2.0 * s5).max(CHANNEL_MIN_HEIGHT).ceil();
        let frame = Frame::new(64.0, 1080.0 - 56.0 - h, w, h);
        let paint = kit.ui.look.colour(PaletteRole::Sheet);
        b.parts.push(Part::Pane(Pane::new(kit, frame, z, PaneKind::Card, over_video)));
        b.panels.push(("channel banner".into(), frame, paint, None));
        let cy = frame.y + h / 2.0;
        // the number and the name on a tile
        let tile_f = Frame::new(frame.x + s5, (cy - tile_side / 2.0).round(), tile_w, tile_side);
        let wash = faded(ink, kit.tokens.states.selected_wash);
        let plate_shape = Shape { corner_radius: kit.tokens.radius(Radius::Tile), fill: Fill::Solid(wash), border_width: 1.0, border: edge(kit) };
        b.parts.push(Part::Shape(plate::shape(&kit.ui, tile_f, z + 1, plate_shape)));
        let tile_panel = b.panels.len();
        b.panels.push(("channel tile".into(), tile_f, wash, Some(0)));
        let block = num_h + cap_h;
        let ny = (tile_f.y + (tile_side - block) / 2.0).round();
        let inner_w = tile_w - 2.0 * s2;
        let number = wrap::elide(kit, number_style, &spec.number, inner_w);
        let t = Text::new(kit, Frame::new(tile_f.x + s2, ny, inner_w, num_h), z + 2, number_style, &number)?;
        b.words.push(Word { name: "channel number".into(), text: number.clone(), frame: t.frame(), style: number_style, panel: Some(tile_panel) });
        b.parts.push(Part::Text(t));
        let call = wrap::elide(kit, call_style, &spec.call, inner_w);
        let t = Text::new(kit, Frame::new(tile_f.x + s2, ny + num_h, inner_w, cap_h), z + 2, call_style, &call)?;
        b.words.push(Word { name: "channel call".into(), text: call.clone(), frame: t.frame(), style: call_style, panel: Some(tile_panel) });
        b.parts.push(Part::Text(t));
        // the logo, whole, on a plate of its own beside the tile (the channel's art is its own: never tinted)
        let mut left = tile_f.right();
        let mut logo_frame = Value::Null;
        if let Some(pic) = logo {
            let plate_f = Frame::new(tile_f.right() + s3, tile_f.y, wide, tile_side);
            let mut px = Px::new(wide as u32, tile_side as u32);
            px.round_rect(Frame::new(0.0, 0.0, wide, tile_side), kit.tokens.radius(Radius::Tile), wash, Some((edge(kit), 1.0)));
            px.lay(&pic.rgba, pic.width, pic.height, ((wide as u32).saturating_sub(pic.width) / 2) as i32, ((tile_side as u32).saturating_sub(pic.height) / 2) as i32);
            b.parts.push(Part::Image(px.layer(kit, z + 1, Props::at(plate_f.x, plate_f.y))?));
            b.glyphs.push(("channel logo".into(), plate_f, tile_side as u32));
            logo_frame = frame_json(plate_f);
            left = plate_f.right();
        }
        // the chips, from the right edge in: the first of them are kept when there is not room for all (they take a share of the bar, the
        // programme's words the rest)
        let mut chip_right = frame.right() - s6;
        let chip_h = (cap_h + s3).ceil();
        let mut chips_left = chip_right;
        let (mut budget, mut kept) = (w * CHANNEL_CHIPS_SHARE, 0);
        for chip in &spec.chips {
            budget -= (width_of(kit, chip_style, chip) + 2.0 * s3 + 6.0).ceil() + s2;
            if budget < 0.0 {
                break;
            }
            kept += 1;
        }
        for chip in spec.chips[..kept].iter().rev() {
            let cw = (width_of(kit, chip_style, chip) + 2.0 * s3 + 6.0).ceil();
            let f = Frame::new(chip_right - cw, (cy - chip_h / 2.0).round(), cw, chip_h);
            let shape = Shape { corner_radius: chip_h / 2.0, fill: Fill::Solid(wash), border_width: 0.0, border: Rgba(0, 0, 0, 0) };
            b.parts.push(Part::Shape(plate::shape(&kit.ui, f, z + 1, shape)));
            let p = b.panels.len();
            b.panels.push(("channel chip".into(), f, wash, Some(0)));
            let t = Text::new(kit, Frame::new(f.x + s3 + 3.0, (f.y + (chip_h - cap_h) / 2.0).round(), cw - 2.0 * s3 - 6.0, cap_h), z + 2, chip_style, chip)?;
            b.words.push(Word { name: "channel chip".into(), text: chip.clone(), frame: t.frame(), style: chip_style, panel: Some(p) });
            b.parts.push(Part::Text(t));
            chip_right = f.x - s2;
            chips_left = f.x;
        }
        // the programme, its time and what is next, the bar of how much is left
        let text_x = left + s5;
        let text_w = (chips_left - s5 - text_x).max(80.0);
        let col_h = inner_h;
        let top = (cy - col_h / 2.0).round();
        let title = wrap::elide(kit, title_style, &spec.title, text_w);
        let t = Text::new(kit, Frame::new(text_x, top, text_w, title_h), z + 2, title_style, &title)?;
        b.words.push(Word { name: "channel title".into(), text: title, frame: t.frame(), style: title_style, panel: Some(0) });
        b.parts.push(Part::Text(t));
        let sub_y = top + title_h + s2;
        let next_text = if spec.next.is_empty() { String::new() } else { format!("Next \u{b7} {}", spec.next) };
        // the time of day comes first (its "min left" goes before any of it does); what is next has what that leaves when that is enough to say
        // something, and is left out when it is not
        let (mut next_w, mut sub_room) = (0.0, text_w);
        if !next_text.is_empty() {
            let range = spec.sub.split(" \u{b7} ").next().unwrap_or_default();
            let time_w = width_of(kit, sub_style, &format!("{range}\u{2026}")).min(width_of(kit, sub_style, &spec.sub)).ceil();
            let left = (text_w - s5 - time_w).floor();
            if left >= width_of(kit, next_style, "Next \u{b7} Wh\u{2026}").ceil() {
                next_w = width_of(kit, next_style, &next_text).ceil().min(left);
                sub_room = text_w - s5 - next_w;
            }
        }
        let sub = wrap::elide(kit, sub_style, &spec.sub, sub_room.max(40.0));
        let sub_w = width_of(kit, sub_style, &sub).min(text_w);
        let t = Text::new(kit, Frame::new(text_x, sub_y, sub_w, cap_h), z + 2, sub_style, &sub)?;
        b.words.push(Word { name: "channel sub".into(), text: sub.clone(), frame: t.frame(), style: sub_style, panel: Some(0) });
        b.parts.push(Part::Text(t));
        if next_w > 0.0 {
            let next = wrap::elide(kit, next_style, &next_text, next_w);
            let t = Text::new(kit, Frame::new(text_x + text_w - next_w, sub_y, next_w, cap_h), z + 2, next_style, &next)?;
            b.words.push(Word { name: "channel next".into(), text: next, frame: t.frame(), style: next_style, panel: Some(0) });
            b.parts.push(Part::Text(t));
        }
        let bar_y = sub_y + cap_h + s2;
        let track = Frame::new(text_x, bar_y, text_w, bar_h);
        b.parts.push(Part::Shape(plate::shape(&kit.ui, track, z + 1, plate::solid(kit.ui.look.colour(PaletteRole::TrackOff), bar_h / 2.0))));
        let fill_w = (text_w * spec.progress).round();
        if fill_w >= 1.0 {
            b.parts.push(Part::Shape(plate::shape(&kit.ui, Frame::new(text_x, bar_y, fill_w, bar_h), z + 2, plate::solid(ink2, bar_h / 2.0))));
        }
        // INFO: what the programme is about, two lines at most under the bar
        let mut detail_shown = 0;
        if detail_lines > 0 {
            for (i, line) in wrap::lines(kit, sub_style, &spec.detail, text_w, detail_lines).iter().enumerate() {
                let t = Text::new(kit, Frame::new(text_x, bar_y + bar_h + s2 + i as f32 * cap_h, text_w, cap_h), z + 2, sub_style, line)?;
                b.words.push(Word { name: "channel detail".into(), text: line.clone(), frame: t.frame(), style: sub_style, panel: Some(0) });
                b.parts.push(Part::Text(t));
                detail_shown += 1;
            }
        }
        b.geometry = json!({"kind": "channel", "bar": frame_json(frame), "tile": frame_json(tile_f), "number": number, "call": call, "sub": sub, "logo": logo_frame,
                            "chips": kept, "progress": spec.progress, "detail": detail_shown});
        Ok(())
    }

    /// The no-signal screen: black, and the block in the dark material's inks at its spot.
    fn no_signal(kit: &mut Kit, engine: &mut Engine, z: i32, root: &Root, spec: &Spec, spot: usize, b: &mut Built) -> std::io::Result<()> {
        let screen = Frame::new(0.0, 0.0, 1920.0, 1080.0);
        b.panels.push(("no signal".into(), screen, Rgba(0, 0, 0, 255), None));
        let scene = kit.ui.scene.clone();
        let block = scene.group(Some(root.group.id()), GroupPose { opacity: BLOCK_ALPHA, ..GroupPose::IDENTITY }).map_err(|e| std::io::Error::other(format!("{e}")))?;
        let (dx, dy) = SPOTS[spot % SPOTS.len()];
        let width = scaled(kit, 760.0).min(1500.0);
        // the dark material whatever the TV's: a still frame on an OLED is what the screen must never hold
        let mut inner: std::io::Result<()> = Ok(());
        with_dark(kit, |kit| {
            inner = scene.adopting_view(&block, || -> std::io::Result<()> {
                let (s1, s3, s4) = (kit.space(1), kit.space(3), kit.space(4));
                let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
                let port_style = role_style(kit, TypeRole::Caption, Some(Weight::SemiBold), ink2, Align::Centre);
                let title_style = role_style(kit, TypeRole::Title, None, ink, Align::Centre);
                let body_style = role_style(kit, TypeRole::Label, None, ink2, Align::Centre);
                let (port_h, title_h, body_lh) = (kit.line(TypeRole::Caption), kit.line(TypeRole::Title), kit.line(TypeRole::Label));
                let icon_side = scaled(kit, 64.0) as u32;
                let title = wrap::lines(kit, title_style, &spec.title, width, 3);
                let body = wrap::lines(kit, body_style, &spec.body, width, 4);
                let btn_h = Button::height(kit);
                let widths: Vec<f32> = spec.buttons.iter().map(|p| Button::width_for(kit, p.label(), true)).collect();
                let buttons_w = widths.iter().sum::<f32>() + s3 * widths.len().saturating_sub(1) as f32;
                let total = icon_side as f32 + s4 + port_h + s4 + title.len() as f32 * title_h + s4 + body.len() as f32 * body_lh + s4 + s1 + s4 + btn_h;
                let x0 = (screen.w - width) / 2.0 + dx;
                let mut y = ((screen.h - total) / 2.0 + dy).round();
                let lead = if kit.icons.has(&spec.icon) { spec.icon.clone() } else { "unplug".to_string() };
                if let Some(l) = icon::layer(kit, &lead, icon_side, ink2, z + 1, Props::at((x0 + (width - icon_side as f32) / 2.0).round(), y))? {
                    b.parts.push(Part::Image(l));
                }
                y += icon_side as f32 + s4;
                let t = Text::new(kit, Frame::new(x0, y, width, port_h), z + 1, port_style, &spec.port)?;
                b.words.push(Word { name: "no signal port".into(), text: spec.port.clone(), frame: t.frame(), style: dimmed(port_style), panel: Some(0) });
                b.parts.push(Part::Text(t));
                y += port_h + s4;
                for line in &title {
                    let t = Text::new(kit, Frame::new(x0, y, width, title_h), z + 1, title_style, line)?;
                    b.words.push(Word { name: "no signal title".into(), text: line.clone(), frame: t.frame(), style: dimmed(title_style), panel: Some(0) });
                    b.parts.push(Part::Text(t));
                    y += title_h;
                }
                y += s4;
                for line in &body {
                    let t = Text::new(kit, Frame::new(x0, y, width, body_lh), z + 1, body_style, line)?;
                    b.words.push(Word { name: "no signal body".into(), text: line.clone(), frame: t.frame(), style: dimmed(body_style), panel: Some(0) });
                    b.parts.push(Part::Text(t));
                    y += body_lh;
                }
                y += s4 + s1 + s4;
                let mut x = x0 + (width - buttons_w) / 2.0;
                for (i, (p, bw)) in spec.buttons.iter().zip(&widths).enumerate() {
                    let f = Frame::new(x.round(), y.round(), *bw, btn_h);
                    let mut btn = Button::new(kit, engine, Place { id: Target::id(i), frame: f, region: REGION, scope: SCOPE, z: z + 2 }, ButtonSpec::new(p.label(), Tone::Plain).with_icon(p.icon()))?;
                    btn.show(kit, true);
                    b.buttons.push(btn);
                    b.labels.push(p.label().to_string());
                    x += bw + s3;
                }
                Ok(())
            });
        });
        b._groups.push(block);
        inner?;
        b.geometry = json!({"kind": "nosignal", "spot": spot, "dx": dx, "dy": dy, "width": width});
        Ok(())
    }

    fn build(kit: &mut Kit, engine: &mut Engine, root: &Root, z: i32, v: &View, over_video: bool, sig: String) -> std::io::Result<Built> {
        let before = kit.room().layers;
        let mut b = Built { sig, kind: v.spec.kind, parts: Vec::new(), buttons: Vec::new(), labels: Vec::new(), panels: Vec::new(), words: Vec::new(), glyphs: Vec::new(), geometry: Value::Null, layers: 0, _groups: Vec::new() };
        let scene = kit.ui.scene.clone();
        match v.spec.kind {
            Kind::Input => scene.adopting_view(&root.group, || Self::pill(kit, engine, z, &v.spec, over_video, &mut b))?,
            Kind::Channel => scene.adopting_view(&root.group, || Self::channel(kit, z, &v.spec, over_video, &mut b))?,
            Kind::NoSignal => {
                // the black screen is under the root too, so it fades with the block; the block is a group of its own, dimmed as a whole
                scene.adopting_view(&root.group, || b.parts.push(Part::Shape(plate::shape(&kit.ui, Frame::new(0.0, 0.0, 1920.0, 1080.0), z, plate::solid(Rgba(0, 0, 0, 255), 0.0)))));
                Self::no_signal(kit, engine, z, root, &v.spec, v.spot, &mut b)?;
            }
        }
        b.layers = before.saturating_sub(kit.room().layers);
        Ok(b)
    }

    fn drop_built(&mut self, engine: &mut Engine) {
        if let Some(b) = self.built.take() {
            for btn in b.buttons {
                btn.destroy(engine);
            }
        }
    }

    fn release_modal(&mut self, engine: &mut Engine) {
        if self.modal {
            engine.focus.close_modal();
            self.modal = false;
        }
    }

    fn drop_all(&mut self, engine: &mut Engine) {
        self.drop_built(engine);
        self.release_modal(engine);
        self.root = None;
        self.leaving_until = None;
        self.first = true;
    }

    /// Fits the screen to the view: made for a banner that came (or whose words, spot, look or size changed), its keys opened as a modal scope while
    /// it has buttons, and faded out and dropped a moment after it has gone. Returns whether anything of it is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, view: Option<&View>, over_video: bool, now_ms: u64, now_ns: u64) -> std::io::Result<bool> {
        let Some(v) = view else {
            if self.root.is_some() && self.leaving_until.is_none() {
                self.release_modal(engine);
                if let Some(root) = self.root.as_mut() {
                    root.hide(kit.beat_ms(crate::kit::motion::Beat::Layer).min(FADE_MS));
                }
                self.leaving_until = Some(now_ms + u64::from(FADE_MS) + 60);
            }
            if self.leaving_until.is_some_and(|t| now_ms >= t) {
                self.drop_all(engine);
            }
            return Ok(self.root.is_some());
        };
        let sig = Self::signature(kit, v, over_video);
        if self.built.as_ref().is_some_and(|b| b.sig != sig) || self.leaving_until.is_some() {
            self.drop_all(engine);
        }
        if self.root.is_none() {
            self.root = Some(Root::new(kit, Arrive::Fade, false)?);
        }
        if self.built.is_none() {
            let root = self.root.as_ref().expect("a root");
            let built = Self::build(kit, engine, root, self.z, v, over_video, sig)?;
            if kit.ui.scene.take_overflow() {
                return Err(std::io::Error::other("the scene is full: the banner could not be drawn whole"));
            }
            self.built = Some(built);
            self.first = true;
        }
        if let Some(root) = self.root.as_mut().filter(|r| !r.is_shown()) {
            root.show(kit.beat_ms(crate::kit::motion::Beat::Layer).min(FADE_MS));
        }
        let interactive = v.spec.interactive();
        if interactive && !self.modal {
            engine.focus.open_modal(SCOPE);
            self.modal = true;
        } else if !interactive {
            self.release_modal(engine);
        }
        self.focus = interactive.then_some(v.index);
        if interactive {
            let want = Target::id(v.index);
            if self.first {
                if engine.focus.focus(want) {
                    let _ = engine.flow.snap(want, true, now_ns);
                    self.first = false;
                }
            } else {
                crate::panels::ui::follow(engine, Some(want), now_ns);
            }
        }
        Ok(true)
    }

    /// Whether the time a banner has to fade out is over: the next frame drops it.
    pub fn tick(&mut self, now_ms: u64) -> bool {
        self.leaving_until.is_some_and(|t| now_ms >= t)
    }

    pub fn due_at(&self) -> Option<u64> {
        self.leaving_until
    }

    /// A key of the remote: the pointer's cue goes.
    pub fn on_key(&mut self, kit: &Kit, engine: &mut Engine) {
        self.pointer.key(engine);
        if let Some(b) = self.built.as_mut() {
            for btn in &mut b.buttons {
                btn.set_hover(kit, false);
            }
        }
    }

    /// A sample of the pointer: the cue and the press follow it over the buttons, and a click is the button it ended on.
    pub fn pointer(&mut self, kit: &Kit, engine: &mut Engine, now_ns: u64, s: Sample) -> Option<Target> {
        let built = self.built.as_mut()?;
        let mut click = None;
        for g in self.pointer.feed(engine, now_ns, s) {
            match g {
                Gesture::Hover { from, to } => {
                    if let Some(Target::Button(i)) = from.and_then(Target::from_id)
                        && let Some(btn) = built.buttons.get_mut(i)
                    {
                        btn.set_hover(kit, false);
                    }
                    let landed = to.filter(|t| engine.focus.current() != Some(*t)).and_then(Target::from_id);
                    if let Some(Target::Button(i)) = landed
                        && let Some(btn) = built.buttons.get_mut(i)
                    {
                        btn.set_hover(kit, true);
                    }
                }
                Gesture::Down(id) | Gesture::Up(Some(id)) => {
                    if let Some(Target::Button(i)) = Target::from_id(id)
                        && let Some(btn) = built.buttons.get_mut(i)
                    {
                        btn.set_pressed(kit, matches!(g, Gesture::Down(_)));
                    }
                }
                Gesture::Click(id) => click = Target::from_id(id),
                Gesture::Up(None) => {}
            }
        }
        click
    }

    pub fn restyle(&mut self, engine: &mut Engine) {
        self.drop_all(engine);
    }

    pub fn layers(&self) -> usize {
        self.built.as_ref().map_or(0, |b| b.layers)
    }

    pub fn geometry(&self) -> Value {
        let Some(b) = &self.built else { return Value::Null };
        let mut g = b.geometry.clone();
        g["buttons"] = json!(b.buttons.iter().zip(&b.labels).map(|(btn, l)| {
            let f = btn.frame();
            json!({"label": l, "x": f.x, "y": f.y, "w": f.w, "h": f.h})
        }).collect::<Vec<_>>());
        g["focus"] = json!(self.focus);
        g["layers"] = json!(self.layers());
        g
    }

    /// Describes what is on the screen to a lint: the panels, the words on them and the buttons.
    pub fn feed_lint(&self, kit: &mut Kit, lint: &mut Lint) {
        let Some(b) = &self.built else { return };
        let mut ids = Vec::new();
        for (i, (name, frame, paint, over)) in b.panels.iter().enumerate() {
            // the main panel is held to the safe area: the pill and the channel bar sit inside it, the black screen is the whole screen's; a tile or a chip
            // paints its wash over the bar's glass
            let safe = i == 0 && b.kind != Kind::NoSignal;
            ids.push(match over.and_then(|o| ids.get(o).copied()) {
                Some(parent) => lint.panel_on(name, *frame, *paint, safe, parent),
                None => lint.panel(name, *frame, *paint, safe),
            });
        }
        for w in &b.words {
            lint.word(kit, &w.name, &w.text, w.frame, w.style, w.panel.and_then(|p| ids.get(p).copied()));
        }
        // a picture on a panel is a run of ink in it too: a pill with a tile holds a layout, not one word to be centred
        let ink = kit.ui.look.ink();
        for (name, frame, px) in &b.glyphs {
            lint.word_at(name, *frame, ink, *px, false, ids.first().copied());
        }
        let under = ids.first().copied().unwrap_or(0);
        for btn in &b.buttons {
            btn.feed_lint(kit, lint, under);
        }
    }
}

/// The style as the no-signal block is seen: its colour at the block's opacity, which is what the lint reads the contrast of.
fn dimmed(s: Style) -> Style {
    Style { colour: faded(s.colour, BLOCK_ALPHA), ..s }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::screen::Input;
    use crate::kit::testkit;
    use crate::system::banner::{Banner, SHOW_TOPIC};
    use ui::lucent::palette::Mode;

    const NS: u64 = 1_000_000;

    fn show(b: &mut Banner, spec: Value, now: u64) {
        b.update(&Input::Topic { topic: SHOW_TOPIC.into(), data: spec }, now);
    }

    fn draw(kit: &mut Kit, engine: &mut Engine, s: &mut BannerSurface, b: &Banner, over: bool, now: u64) -> bool {
        s.apply(kit, engine, b.view().as_ref(), over, now, now * NS).expect("the banner draws")
    }

    fn lint_of(kit: &mut Kit, s: &BannerSurface) -> Vec<String> {
        let mut lint = Lint::new();
        s.feed_lint(kit, &mut lint);
        lint.findings().iter().map(ToString::to_string).collect()
    }

    fn pill() -> Value {
        json!({"kind": "input", "icon": "gamepad-2", "name": "HDMI 3 \u{b7} PlayStation 5", "facts": "4K 120 Hz HDR10"})
    }

    fn bar() -> Value {
        json!({"kind": "input", "icon": "gamepad-2", "name": "HDMI 3 \u{b7} PlayStation 5", "facts": "4K 120 Hz HDR10", "buttons": true})
    }

    fn nosignal() -> Value {
        json!({"kind": "nosignal", "icon": "unplug", "port": "HDMI 4", "title": "Nothing is connected here", "body": "Plug a device into HDMI 4, or pick another input. The screen stays dark to protect the panel."})
    }

    fn channel() -> Value {
        json!({"kind": "channel", "number": "7", "call": "KABC", "title": "Evening News with a title that runs on and on", "sub": "6:00 PM - 6:30 PM", "progress": 0.4,
               "next": "Jeopardy!", "chips": ["1080i", "Stereo", "CC"]})
    }

    #[test]
    fn the_pill_is_at_the_foot_of_the_screen_64_from_the_left_inside_the_safe_area_and_reads_clean_in_both_materials_and_sizes() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (_h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = Engine::new(&kit).expect("an engine");
                let mut b = Banner::new(&json!({}));
                show(&mut b, pill(), 1000);
                let mut s = BannerSurface::new(1000);
                assert!(draw(&mut kit, &mut engine, &mut s, &b, true, 1100));
                let g = s.geometry();
                let p = &g["pill"];
                assert_eq!(p["x"].as_f64().unwrap(), 64.0, "{g}");
                assert_eq!(p["y"].as_f64().unwrap() + p["h"].as_f64().unwrap(), 1080.0 - 56.0, "{mode:?} at {scale}: the foot of the screen: {g}");
                assert!(p["h"].as_f64().unwrap() >= 96.0);
                assert_eq!(engine.focus.modal(), None, "a pill with no buttons takes no key: no modal scope");
                let found = lint_of(&mut kit, &s);
                assert!(found.is_empty(), "{mode:?} at {scale}: {found:?}");
            }
        }
    }

    #[test]
    fn the_bar_with_buttons_opens_a_modal_scope_focuses_the_first_button_follows_the_model_and_gives_the_keys_back_when_it_goes() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (_h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = Engine::new(&kit).expect("an engine");
                let mut b = Banner::new(&json!({}));
                show(&mut b, bar(), 1000);
                let mut s = BannerSurface::new(1000);
                draw(&mut kit, &mut engine, &mut s, &b, false, 1100);
                assert_eq!(engine.focus.modal(), Some(SCOPE));
                assert_eq!(engine.focus.current(), Some(Target::id(0)));
                let g = s.geometry();
                let labels: Vec<&str> = g["buttons"].as_array().unwrap().iter().map(|x| x["label"].as_str().unwrap()).collect();
                assert_eq!(labels, vec!["Details", "Inputs", "Picture"]);
                let pill = &g["pill"];
                for btn in g["buttons"].as_array().unwrap() {
                    assert!(btn["x"].as_f64().unwrap() >= pill["x"].as_f64().unwrap() && btn["x"].as_f64().unwrap() + btn["w"].as_f64().unwrap() <= pill["x"].as_f64().unwrap() + pill["w"].as_f64().unwrap(), "{mode:?} at {scale}: a button inside the bar: {g}");
                }
                let found = lint_of(&mut kit, &s);
                assert!(found.is_empty(), "{mode:?} at {scale}: {found:?}");
                // the model moves the focus; the surface follows
                b.update(&Input::Key { name: "right".into(), down: true, repeat: false }, 1200);
                draw(&mut kit, &mut engine, &mut s, &b, false, 1300);
                assert_eq!(engine.focus.current(), Some(Target::id(1)));
                assert_eq!(s.geometry()["focus"], 1);
                // it goes: faded out, dropped, and the keys are given back at once
                b.update(&Input::Key { name: "back".into(), down: true, repeat: false }, 1400);
                assert!(draw(&mut kit, &mut engine, &mut s, &b, false, 1500), "fading");
                assert_eq!(engine.focus.modal(), None, "the keys are no longer the banner's while it fades");
                assert!(s.tick(1500 + 1000));
                assert!(!draw(&mut kit, &mut engine, &mut s, &b, false, 3000));
                assert!(!s.is_up() && s.layers() == 0, "nothing hidden is kept");
                assert!(engine.focus.frame(Target::id(0)).is_none(), "its buttons are gone from the engine");
            }
        }
    }

    #[test]
    fn no_signal_is_black_and_dim_in_the_dark_materials_inks_whatever_the_tvs_material_is_and_its_spot_moves_every_minute() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (_h, mut kit) = testkit::kit_in(mode, scale);
                let ink_before = kit.ui.look.ink();
                let mut engine = Engine::new(&kit).expect("an engine");
                let mut b = Banner::new(&json!({}));
                show(&mut b, nosignal(), 1000);
                let mut s = BannerSurface::new(1000);
                draw(&mut kit, &mut engine, &mut s, &b, false, 1100);
                assert_eq!(kit.ui.look.ink(), ink_before, "the TV's own look is back in force after the dark one made the block");
                assert_eq!(engine.focus.modal(), Some(SCOPE), "two buttons: the keys are the screen's");
                let g = s.geometry();
                assert_eq!(g["buttons"].as_array().unwrap().len(), 2);
                let found = lint_of(&mut kit, &s);
                assert!(found.is_empty(), "{mode:?} at {scale}: {found:?}");
                let first = g["buttons"][0]["x"].as_f64().unwrap();
                b.update(&Input::Tick, 1000 + super::super::banner::SPOT_MS);
                draw(&mut kit, &mut engine, &mut s, &b, false, 1000 + super::super::banner::SPOT_MS + 100);
                assert_ne!(s.geometry()["buttons"][0]["x"].as_f64().unwrap(), first, "the block hopped to the next spot");
                assert_eq!(s.geometry()["spot"], 1);
                assert_eq!(engine.focus.current(), Some(Target::id(0)), "the focus is where it was");
            }
        }
    }

    #[test]
    fn the_channel_banner_reads_clean_with_its_chips_and_a_title_that_does_not_fit() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (_h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = Engine::new(&kit).expect("an engine");
                let mut b = Banner::new(&json!({}));
                show(&mut b, channel(), 1000);
                let mut s = BannerSurface::new(1000);
                draw(&mut kit, &mut engine, &mut s, &b, true, 1100);
                let g = s.geometry();
                assert_eq!(g["chips"], 3);
                assert_eq!(engine.focus.modal(), None);
                let found = lint_of(&mut kit, &s);
                assert!(found.is_empty(), "{mode:?} at {scale}: {found:?}");
            }
        }
    }

    /// A logo as LG keeps them on the set: a 16:9 PNG of 356 by 200 with art on a ground of its own.
    fn logo_file() -> String {
        let path = std::env::temp_dir().join(format!("lumatv-banner-logo-{}.png", std::process::id()));
        let (w, h) = (356u32, 200u32);
        let mut data = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let on_letter = (60..300).contains(&x) && (70..130).contains(&y) && (x / 20 + y / 20) % 2 == 0;
                data.extend_from_slice(if on_letter { &[255, 255, 255, 255] } else { &[200, 40, 60, 255] });
            }
        }
        let mut enc = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(&path).expect("a file")), w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().expect("a header").write_image_data(&data).expect("the pixels");
        path.to_string_lossy().into_owned()
    }

    /// Long words everywhere the channel banner has a place for them.
    fn long_channel(logo: &str) -> Value {
        json!({"kind": "channel", "number": "11-12", "call": "Atlanta Channel 2 Action News Weekend Edition", "logo": logo,
               "title": "Antiques Roadshow Atlanta, Hour 2: a title that runs on and on and on past the end of the line",
               "sub": "8:00 PM \u{2013} 9:00 PM \u{b7} 23 min left", "progress": 0.62, "next": "Nova: Secrets of the Shining Knight and the Lost Library",
               "chips": ["TV-PG", "CC", "AD", "Internet"]})
    }

    #[test]
    fn the_channel_banner_gives_the_logo_a_plate_of_its_own_beside_the_tile_and_reads_clean_with_long_words_in_both_materials_and_sizes() {
        let logo = logo_file();
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (_h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = Engine::new(&kit).expect("an engine");
                let mut b = Banner::new(&json!({}));
                show(&mut b, long_channel(&logo), 1000);
                let mut s = BannerSurface::new(1000);
                draw(&mut kit, &mut engine, &mut s, &b, true, 1100);
                let g = s.geometry();
                let (bar, tile, plate) = (&g["bar"], &g["tile"], &g["logo"]);
                assert!(plate.is_object(), "{mode:?} at {scale}: the logo has its plate: {g}");
                let f = |v: &Value, k: &str| v[k].as_f64().unwrap();
                assert!(f(plate, "x") >= f(tile, "x") + f(tile, "w"), "{mode:?} at {scale}: beside the tile, not on it: {g}");
                assert_eq!((f(plate, "y"), f(plate, "h")), (f(tile, "y"), f(tile, "h")), "as high as the tile, on its line");
                assert!((f(plate, "w") / f(plate, "h") - 16.0 / 9.0).abs() < 0.02, "a 16:9 plate: {g}");
                assert!(f(plate, "x") + f(plate, "w") <= f(bar, "x") + f(bar, "w"), "inside the bar");
                assert!(f(bar, "x") >= 64.0 && f(bar, "x") + f(bar, "w") <= 1920.0 - 64.0 && f(bar, "y") + f(bar, "h") <= 1080.0 - 56.0, "64 from the sides and 56 from the foot: {g}");
                assert!(g["sub"].as_str().is_some_and(|t| t.starts_with("8:00 PM \u{2013} 9:00 PM")), "{mode:?} at {scale}: the time of day is read whole, its minutes left and what is next give way first: {g}");
                assert!(g["chips"].as_u64().is_some_and(|n| n >= 2 && (scale > 1.0 || n == 4)), "{mode:?} at {scale}: the first marks are kept, and all of them where there is room: {g}");
                let found = lint_of(&mut kit, &s);
                assert!(found.is_empty(), "{mode:?} at {scale}: {found:?}");
            }
        }
    }

    #[test]
    fn a_logo_that_is_not_there_is_no_plate_and_no_gap_and_what_the_programme_is_about_is_two_lines_at_most() {
        for scale in [1.0, 2.0] {
            let (_h, mut kit) = testkit::kit_in(Mode::Dark, scale);
            let mut engine = Engine::new(&kit).expect("an engine");
            let mut b = Banner::new(&json!({}));
            show(&mut b, long_channel("/nowhere/logo.png"), 1000);
            let mut s = BannerSurface::new(1000);
            draw(&mut kit, &mut engine, &mut s, &b, true, 1100);
            assert!(s.geometry()["logo"].is_null(), "a file that is not there leaves no empty plate: {}", s.geometry());
            let narrow = s.geometry()["bar"]["w"].as_f64().unwrap();
            let logo = logo_file();
            show(&mut b, long_channel(&logo), 1200);
            draw(&mut kit, &mut engine, &mut s, &b, true, 1300);
            assert!(s.geometry()["bar"]["w"].as_f64().unwrap() >= narrow, "a bar with a logo is as wide or wider, never narrower");
            // INFO: the description under the bar, cut at two lines
            let mut spec = long_channel(&logo);
            spec["info"] = json!(true);
            spec["detail"] = json!("A long description of the programme that goes on for a good while about the appraisers and the owners and the things they brought to be looked at, and then some more of it, and more again, so that it cannot fit on two lines of the bar.");
            show(&mut b, spec, 1400);
            draw(&mut kit, &mut engine, &mut s, &b, true, 1500);
            assert_eq!(s.geometry()["detail"], 2, "at {scale}: two lines, the rest cut: {}", s.geometry());
            let found = lint_of(&mut kit, &s);
            assert!(found.is_empty(), "at {scale}: {found:?}");
        }
    }

    #[test]
    fn the_number_and_name_tile_stays_square_for_short_words_and_widens_so_that_a_long_number_is_read_whole() {
        for scale in [1.0, 2.0] {
            let (_h, mut kit) = testkit::kit_in(Mode::Dark, scale);
            let mut engine = Engine::new(&kit).expect("an engine");
            let mut b = Banner::new(&json!({}));
            let mut s = BannerSurface::new(1000);
            show(&mut b, json!({"kind": "channel", "number": "7", "call": "KABC", "title": "News"}), 1000);
            draw(&mut kit, &mut engine, &mut s, &b, true, 1100);
            let g = s.geometry();
            assert_eq!(g["tile"]["w"], g["tile"]["h"], "at {scale}: a short number and name leave the tile square: {g}");
            let side = g["tile"]["h"].as_f64().unwrap();
            show(&mut b, json!({"kind": "channel", "number": "100-12", "call": "Georgia Public Broadcasting Atlanta Multicast", "title": "News"}), 1200);
            draw(&mut kit, &mut engine, &mut s, &b, true, 1300);
            let g = s.geometry();
            let wide = g["tile"]["w"].as_f64().unwrap();
            assert!(wide > side && wide <= (side * f64::from(CHANNEL_TILE_GROWS)).round(), "at {scale}: wider for what it holds, not without limit: {g}");
            assert_eq!(g["number"], "100-12", "at {scale}: the number is read whole: {g}");
            let found = lint_of(&mut kit, &s);
            assert!(found.is_empty(), "at {scale}: {found:?}");
        }
    }

    #[test]
    fn the_channel_banner_has_no_hard_coded_colour_radius_or_time_it_takes_them_from_the_theme() {
        let src = include_str!("banner_surface.rs");
        let from = src.find("    fn channel(").expect("the channel banner");
        let to = from + src[from..].find("    fn no_signal(").expect("the no-signal screen follows it");
        for (n, line) in src[from..to].lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            assert!(!code.contains("0x") && !code.contains('#'), "line {n} of the channel banner writes a colour: {line}");
            assert!(!code.contains("Rgba(") || code.contains("Rgba(0, 0, 0, 0)"), "line {n} of the channel banner writes a colour of its own: {line}");
            assert!(!code.contains("Duration") && !code.contains("_ms") && !code.contains("Fill::Linear"), "line {n} of the channel banner writes a time or a gradient: {line}");
            if let Some(i) = code.find("corner_radius:") {
                let value = &code[i + "corner_radius:".len()..];
                assert!(value.trim_start().starts_with("kit.tokens.radius(") || value.trim_start().starts_with("chip_h / 2.0"), "line {n}: a radius is the theme's or a pill's: {line}");
            }
        }
        // and what is drawn follows the theme: the sheet and the inks are the two materials' own
        let colours: Vec<_> = [Mode::Dark, Mode::Light]
            .into_iter()
            .map(|mode| {
                let (_h, mut kit) = testkit::kit_in(mode, 1.0);
                let mut engine = Engine::new(&kit).expect("an engine");
                let mut b = Banner::new(&json!({}));
                show(&mut b, channel(), 1000);
                let mut s = BannerSurface::new(1000);
                draw(&mut kit, &mut engine, &mut s, &b, true, 1100);
                (kit.ui.look.ink(), kit.ui.look.colour(PaletteRole::Sheet))
            })
            .collect();
        assert_ne!(colours[0], colours[1], "the banner's inks and sheet are the material's, which differ between dark and light");
    }
}
