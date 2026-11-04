//! The banners drawn (`panels/Banner.qml`, `panels/BannerButton.qml`, the mocks `20-channel-banner`, `25-input-banner`, `25-input-pill`, `18-no-signal`):
//!
//! - the **input pill**: a glass pill at the foot of the screen, 64 from the left, with the input's tile, its name and what it carries, and, when
//!   the spec asks for them, a divider and the Details, Inputs and Picture buttons;
//! - the **channel banner**: one slim strip of the theme's glass, centred over the picture above the subtitles' band: the channel's logo (or its call
//!   sign's letters), number and name, then what is on with its time and how much is left; INFO grows it with what the programme is about, what is
//!   next and its marks;
//! - the **no-signal screen**: black in both materials, the dim block (icon, port, what is wrong, what to do, Inputs and Home) at a spot that
//!   changes every minute, drawn in the dark material's inks whatever the TV's material is.
//!
//! The buttons are the kit's: focusables of the engine with the flowing lift, the hover cue and the press look. A banner with buttons opens a modal
//! scope for as long as it is up (so the keys are its), and the pill without buttons takes none. Everything hangs under one root that fades in and
//! out; the banner is made again when its words, its spot, the look or the text size change.

use super::banner::{Kind, SPOTS, Spec, View};
use super::channel_feed::NO_INFORMATION;
use super::parts::Part;
use crate::kit::avatar;
use crate::kit::button::{Button, Place, Spec as ButtonSpec, Tone, faded};
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::icon;
use crate::kit::lint::Lint;
use crate::kit::pane::{self, Pane, PaneKind};
use crate::kit::pointer::{Gesture, Pointer, Sample};
use crate::kit::text::{Text, role_style, width_of};
use crate::kit::widgets::{Px, edge, tile};
use crate::kit::{Engine, Kit, wrap};
use crate::panels::ui::{Arrive, Root};
use compositor_protocol::{Fill, GroupPose, Props, Rgba, Shape};
use lumatv_placement::{Rect as PlacedRect, above_subtitles, model::{SAFE_X, STACK, TOAST_FOOT}};
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
/// The channel banner's measures at 100 % text, in TV pixels: the logo plate's height (16:9, LG's own logos are 356 by 200), the widest the number and
/// name may be, how narrow and how wide the column of what is on may get, the least INFO's strip is wide, what share of INFO's last row the marks may
/// take, the progress line's thickness and its least length. Everything else of its look (colours, radii, type, spacing, depth, motion) is the theme's.
const CHANNEL_PLATE: f32 = 72.0;
const CHANNEL_ID_MAX: f32 = 190.0;
const CHANNEL_COLUMN_MIN: f32 = 240.0;
const CHANNEL_COLUMN_MAX: f32 = 640.0;
const CHANNEL_INFO_WIDTH: f32 = 880.0;
const CHANNEL_CHIPS_SHARE: f32 = 0.4;
const CHANNEL_BAR: f32 = 4.0;
const CHANNEL_TRACK_MIN: f32 = 120.0;

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

/// The few letters that stand for a channel where its logo is missing: its call sign when that is a short word in capitals (WSB, KABC, PBS of PBS
/// Kids), a single short word as it is, a longer one by its first three letters, else the first letters of its first three words (Georgia Public
/// Broadcasting Atlanta Multicast is GPB). A trailing "HD", "DT" or "SD" is not part of the name.
fn monogram(name: &str) -> String {
    let words: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let named = words.iter().rposition(|w| !["HD", "DT", "SD"].iter().any(|s| w.eq_ignore_ascii_case(s))).map_or(words.len(), |last| last + 1);
    let Some(first) = words[..named].first().copied() else { return String::new() };
    let len = first.chars().count();
    let call_sign = (2..=5).contains(&len) && !first.chars().any(char::is_lowercase);
    let letters: String = if call_sign || (named == 1 && len <= 5) {
        first.to_string()
    } else if named == 1 {
        first.chars().take(3).collect()
    } else {
        words[..named].iter().take(3).filter_map(|w| w.chars().next()).collect()
    };
    letters.to_uppercase()
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

    /// The channel banner: one slim strip of the theme's glass, centred over the picture and resting above the subtitles' band as the toasts do. From
    /// the left: the channel's logo on a plate (its call sign's letters where it has none), its number and name, a rule, and what is on: the
    /// programme's title over its time and a line of how much is left. INFO grows it upward with what the programme is about and, under that, what is
    /// next and the programme's marks. What the TV knows nothing of ("No information") is said quietly, with no time and no line. The glass hides the
    /// picture (`pane::reading_glass`), so nothing behind it reads through its words. The measures are the strip's (`CHANNEL_*`), the rest is the theme's.
    fn channel(kit: &mut Kit, z: i32, spec: &Spec, b: &mut Built) -> std::io::Result<()> {
        let (s1, s2, s3) = (kit.space(1), kit.space(2), kit.space(3));
        let (pad_x, pad_y, gap, rule) = (scaled(kit, kit.space(5)), scaled(kit, kit.space(3)), scaled(kit, kit.space(4)), scaled(kit, kit.space(5)));
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let strong = role_style(kit, TypeRole::Body, Some(Weight::SemiBold), ink, Align::Left);
        let number_style = role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left);
        let mono_style = Style { align: Align::Centre, ..strong };
        let quiet = spec.title == NO_INFORMATION;
        let title_style = if quiet { role_style(kit, TypeRole::Body, None, ink2, Align::Left) } else { strong };
        let small = role_style(kit, TypeRole::Caption, None, ink2, Align::Left);
        let chip_style = role_style(kit, TypeRole::Caption, Some(Weight::SemiBold), ink2, Align::Left);
        let (num_h, body_h, cap_h) = (kit.line(TypeRole::Title), kit.line(TypeRole::Body), kit.line(TypeRole::Caption));
        let (id_h, prog_h) = (num_h + cap_h, body_h + cap_h);
        let plate_h = scaled(kit, CHANNEL_PLATE);
        let plate_w = (plate_h * 16.0 / 9.0 / 2.0).round() * 2.0;
        let content_h = id_h.max(prog_h).max(plate_h);
        let logo = (!spec.logo.is_empty()).then(|| avatar::contained(&spec.logo, (plate_w - 2.0 * s2) as u32, (plate_h - 2.0 * s2) as u32)).flatten();
        let mono = if logo.is_some() { String::new() } else { monogram(&spec.call) };
        let has_plate = logo.is_some() || !mono.is_empty();
        let has_id = !spec.number.is_empty() || !spec.call.is_empty();
        let has_title = !spec.title.is_empty();
        // the widths: the number and the name as far as they go, the column of what is on as its title and its time want it, the strip as those are
        let id_w = if has_id { width_of(kit, number_style, &spec.number).max(width_of(kit, small, &spec.call)).ceil().min(scaled(kit, CHANNEL_ID_MAX)) } else { 0.0 };
        let meta = has_title && !quiet && (!spec.sub.is_empty() || spec.progress > 0.0);
        let track_min = scaled(kit, CHANNEL_TRACK_MIN);
        let time_gap = if spec.sub.is_empty() { 0.0 } else { gap };
        let time_w = width_of(kit, small, &spec.sub).ceil() + time_gap + track_min;
        let want = width_of(kit, title_style, &spec.title).ceil().max(if meta { time_w } else { 0.0 });
        let plate_run = if has_plate { plate_w } else { 0.0 };
        let plate_gap = if has_plate && has_id { gap } else { 0.0 };
        let lead_w = plate_run + plate_gap + id_w;
        let rule_w = if lead_w > 0.0 && has_title { 2.0 * rule + 1.0 } else { 0.0 };
        let fixed = 2.0 * pad_x + lead_w + rule_w;
        let about = spec.info && !spec.detail.is_empty();
        let foot = spec.info && (!spec.next.is_empty() || !spec.chips.is_empty());
        let mut w = fixed + if has_title { want.clamp(scaled(kit, CHANNEL_COLUMN_MIN), scaled(kit, CHANNEL_COLUMN_MAX)) } else { 0.0 };
        if about || foot {
            w = w.max(scaled(kit, CHANNEL_INFO_WIDTH));
        }
        let w = w.min(1920.0 - 2.0 * SAFE_X as f32).ceil();
        let w = w + w % 2.0;
        let col_w = if has_title { (w - fixed).max(0.0) } else { 0.0 };
        let inner_w = w - 2.0 * pad_x;
        // INFO's rows: what it is about (two lines at most), then what is next and the marks
        let detail = if about { wrap::lines(kit, small, &spec.detail, inner_w, 2) } else { Vec::new() };
        let chip_h = (cap_h + s2).ceil();
        let foot_h = if spec.chips.is_empty() { cap_h } else { chip_h };
        let mut info_h = 0.0;
        if !detail.is_empty() || foot {
            info_h = gap + 1.0 + gap + detail.len() as f32 * cap_h;
            if foot {
                info_h += foot_h + if detail.is_empty() { 0.0 } else { s1 };
            }
        }
        let h = (2.0 * pad_y + content_h + info_h).ceil();
        let h = h + h % 2.0;
        // centred on the screen, on the toasts' grid: they rest on the TV-safe foot and are raised above the subtitles' band over an app
        let rest = PlacedRect::new(((1920.0 - w) / 2.0).round() as i32, 1080 - TOAST_FOOT - h as i32, w as i32, h as i32);
        let placed = above_subtitles(rest, STACK);
        let frame = Frame::new(placed.x as f32, placed.y as f32, w, h);
        b.parts.push(Part::Pane(Pane::reading(kit, frame, z, PaneKind::Card)));
        b.panels.push(("channel banner".into(), frame, pane::reading_glass(kit, PaneKind::Card, h).tint, None));
        let wash = faded(ink, kit.tokens.states.selected_wash);
        let top = frame.y + pad_y;
        let mut x = frame.x + pad_x;
        // the plate: the logo whole, or the letters of the call sign, on a quiet wash (the channel's art is its own: never tinted)
        let mut plate_frame = Value::Null;
        if has_plate {
            let f = Frame::new(x, top + ((content_h - plate_h) / 2.0).round(), plate_w, plate_h);
            let mut px = Px::new(plate_w as u32, plate_h as u32);
            px.round_rect(Frame::new(0.0, 0.0, plate_w, plate_h), kit.tokens.radius(Radius::Tile), wash, Some((edge(kit), 1.0)));
            if let Some(pic) = &logo {
                px.lay(&pic.rgba, pic.width, pic.height, ((plate_w as u32).saturating_sub(pic.width) / 2) as i32, ((plate_h as u32).saturating_sub(pic.height) / 2) as i32);
                b.glyphs.push(("channel logo".into(), f, plate_h as u32));
            }
            b.parts.push(Part::Image(px.layer(kit, z + 1, Props::at(f.x, f.y))?));
            if logo.is_none() {
                let on_plate = b.panels.len();
                b.panels.push(("channel plate".into(), f, wash, Some(0)));
                let letters = wrap::elide(kit, mono_style, &mono, plate_w - 2.0 * s2);
                let t = Text::new(kit, Frame::new(f.x + s2, (f.y + (plate_h - body_h) / 2.0).round(), plate_w - 2.0 * s2, body_h), z + 2, mono_style, &letters)?;
                b.words.push(Word { name: "channel monogram".into(), text: letters, frame: t.frame(), style: mono_style, panel: Some(on_plate) });
                b.parts.push(Part::Text(t));
            }
            plate_frame = frame_json(f);
            x += plate_w + plate_gap;
        }
        // the number over the name
        let number = wrap::elide(kit, number_style, &spec.number, id_w);
        let call = wrap::elide(kit, small, &spec.call, id_w);
        if has_id {
            let rows = [(number.as_str(), "channel number", number_style, num_h), (call.as_str(), "channel call", small, cap_h)];
            let used: f32 = rows.iter().filter(|r| !r.0.is_empty()).map(|r| r.3).sum();
            let mut y = top + ((content_h - used) / 2.0).round();
            for (text, name, style, line) in rows.into_iter().filter(|r| !r.0.is_empty()) {
                let t = Text::new(kit, Frame::new(x, y, id_w, line), z + 2, style, text)?;
                b.words.push(Word { name: name.into(), text: text.to_string(), frame: t.frame(), style, panel: Some(0) });
                b.parts.push(Part::Text(t));
                y += line;
            }
            x += id_w;
        }
        // what is on: its title, and under it the time and a line of how much is left
        let (mut title, mut sub) = (String::new(), String::new());
        let mut track_frame = Value::Null;
        if has_title {
            if lead_w > 0.0 {
                x += rule;
                let divider = Frame::new(x, top + ((content_h - prog_h) / 2.0).round(), 1.0, prog_h);
                b.parts.push(Part::Shape(plate::shape(&kit.ui, divider, z + 1, plate::solid(edge(kit), 0.0))));
                x += 1.0 + rule;
            }
            let col_top = top + ((content_h - if meta { prog_h } else { body_h }) / 2.0).round();
            title = wrap::elide(kit, title_style, &spec.title, col_w);
            let t = Text::new(kit, Frame::new(x, col_top, col_w, body_h), z + 2, title_style, &title)?;
            b.words.push(Word { name: "channel title".into(), text: title.clone(), frame: t.frame(), style: title_style, panel: Some(0) });
            b.parts.push(Part::Text(t));
            if meta {
                let row = col_top + body_h;
                sub = wrap::elide(kit, small, &spec.sub, (col_w - time_gap - track_min).max(0.0));
                let sub_w = width_of(kit, small, &sub).ceil();
                if !sub.is_empty() {
                    let t = Text::new(kit, Frame::new(x, row, sub_w, cap_h), z + 2, small, &sub)?;
                    b.words.push(Word { name: "channel sub".into(), text: sub.clone(), frame: t.frame(), style: small, panel: Some(0) });
                    b.parts.push(Part::Text(t));
                }
                let bar_h = scaled(kit, CHANNEL_BAR);
                let track_x = if sub.is_empty() { x } else { x + sub_w + gap };
                let track = Frame::new(track_x, row + ((cap_h - bar_h) / 2.0).round(), (x + col_w - track_x).max(0.0), bar_h);
                if track.w >= 1.0 {
                    b.parts.push(Part::Shape(plate::shape(&kit.ui, track, z + 1, plate::solid(kit.ui.look.colour(PaletteRole::TrackOff), bar_h / 2.0))));
                    let fill_w = (track.w * spec.progress).round();
                    if fill_w >= 1.0 {
                        b.parts.push(Part::Shape(plate::shape(&kit.ui, Frame::new(track.x, track.y, fill_w, bar_h), z + 2, plate::solid(ink2, bar_h / 2.0))));
                    }
                }
                track_frame = frame_json(track);
            }
        }
        // INFO: a rule, what the programme is about, then what is next at the left and its marks, from the right edge in, at the right
        let (mut kept, mut next) = (0, String::new());
        if info_h > 0.0 {
            let left = frame.x + pad_x;
            let mut y = top + content_h + gap;
            b.parts.push(Part::Shape(plate::shape(&kit.ui, Frame::new(left, y, inner_w, 1.0), z + 1, plate::solid(edge(kit), 0.0))));
            y += 1.0 + gap;
            for line in &detail {
                let t = Text::new(kit, Frame::new(left, y, inner_w, cap_h), z + 2, small, line)?;
                b.words.push(Word { name: "channel detail".into(), text: line.clone(), frame: t.frame(), style: small, panel: Some(0) });
                b.parts.push(Part::Text(t));
                y += cap_h;
            }
            if !detail.is_empty() {
                y += s1;
            }
            let mut chips_left = frame.right() - pad_x;
            let mut budget = inner_w * CHANNEL_CHIPS_SHARE;
            for chip in &spec.chips {
                budget -= (width_of(kit, chip_style, chip) + 2.0 * s3 + 6.0).ceil() + s2;
                if budget < 0.0 {
                    break;
                }
                kept += 1;
            }
            for chip in spec.chips[..kept].iter().rev() {
                let cw = (width_of(kit, chip_style, chip) + 2.0 * s3 + 6.0).ceil();
                let f = Frame::new(chips_left - cw, y, cw, chip_h);
                let shape = Shape { corner_radius: chip_h / 2.0, fill: Fill::Solid(wash), border_width: 0.0, border: Rgba(0, 0, 0, 0) };
                b.parts.push(Part::Shape(plate::shape(&kit.ui, f, z + 1, shape)));
                let p = b.panels.len();
                b.panels.push(("channel chip".into(), f, wash, Some(0)));
                let t = Text::new(kit, Frame::new(f.x + s3 + 3.0, (f.y + (chip_h - cap_h) / 2.0).round(), cw - 2.0 * s3 - 6.0, cap_h), z + 2, chip_style, chip)?;
                b.words.push(Word { name: "channel chip".into(), text: chip.clone(), frame: t.frame(), style: chip_style, panel: Some(p) });
                b.parts.push(Part::Text(t));
                chips_left = f.x - s2;
            }
            if !spec.next.is_empty() {
                let room = (chips_left - gap - left).max(0.0);
                next = wrap::elide(kit, small, &format!("Next \u{b7} {}", spec.next), room);
                let t = Text::new(kit, Frame::new(left, y + ((foot_h - cap_h) / 2.0).round(), room, cap_h), z + 2, small, &next)?;
                b.words.push(Word { name: "channel next".into(), text: next.clone(), frame: t.frame(), style: small, panel: Some(0) });
                b.parts.push(Part::Text(t));
            }
        }
        let words: Vec<Value> = b.words.iter().map(|w| json!({"name": w.name, "x": w.frame.x, "y": w.frame.y, "w": w.frame.w, "h": w.frame.h})).collect();
        b.geometry = json!({"kind": "channel", "bar": frame_json(frame), "plate": plate_frame, "logo": logo.is_some(), "monogram": mono, "number": number, "call": call,
                            "title": title, "sub": sub, "quiet": quiet, "track": track_frame, "progress": spec.progress, "detail": detail.len(), "chips": kept,
                            "next": next, "words": words});
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
            Kind::Channel => scene.adopting_view(&root.group, || Self::channel(kit, z, &v.spec, &mut b))?,
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
    use crate::kit::lint::contrast_on;
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

    /// Shows `spec`, draws it and returns what was drawn of it.
    fn drawn(kit: &mut Kit, engine: &mut Engine, s: &mut BannerSurface, b: &mut Banner, spec: Value, at: u64) -> Value {
        show(b, spec, at);
        draw(kit, engine, s, b, true, at + 100);
        s.geometry()
    }

    fn num(v: &Value, k: &str) -> f64 {
        v[k].as_f64().unwrap()
    }

    /// What INFO adds to a channel's spec: what the programme is about, too long for two lines at any size of text.
    fn about(mut spec: Value) -> Value {
        spec["info"] = json!(true);
        spec["detail"] = json!("A long description of the programme that goes on for a good while about the appraisers and the owners and the things they brought to be looked at, \
                                and then some more of it, and more again, so that it cannot fit on two lines of the strip, however wide it is, at any size of text, \
                                and the rest of it is cut where the second line ends.");
        spec
    }

    fn no_information() -> Value {
        json!({"kind": "channel", "number": "5-3", "call": "BUZZR", "title": "No information"})
    }

    #[test]
    fn the_channel_banner_is_one_slim_strip_centred_over_the_picture_above_the_subtitles_and_reads_clean_in_both_materials_and_sizes() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (_h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = Engine::new(&kit).expect("an engine");
                let (mut b, mut s) = (Banner::new(&json!({})), BannerSurface::new(1000));
                let g = drawn(&mut kit, &mut engine, &mut s, &mut b, channel(), 1000);
                let bar = &g["bar"];
                assert_eq!(num(bar, "x") * 2.0 + num(bar, "w"), 1920.0, "{mode:?} at {scale}: centred on the screen: {g}");
                assert_eq!(num(bar, "y") + num(bar, "h"), 889.0, "{mode:?} at {scale}: where the toasts rest over an app, 16 above the subtitles' band: {g}");
                // slim: the bar this replaces was 154 by 1455 at 100 % text on the set
                let (h, w) = if scale == 1.0 { (120.0, 1040.0) } else { (230.0, 1700.0) };
                assert!(num(bar, "h") <= h && num(bar, "w") <= w, "{mode:?} at {scale}: at most {w} by {h}: {g}");
                assert_eq!((g["chips"].as_u64(), g["detail"].as_u64(), g["next"].as_str()), (Some(0), Some(0), Some("")), "{mode:?} at {scale}: the marks and what is next are INFO's: {g}");
                assert_eq!(engine.focus.modal(), None);
                let found = lint_of(&mut kit, &s);
                assert!(found.is_empty(), "{mode:?} at {scale}: {found:?}");
            }
        }
    }

    /// A white ad with black print behind the strip is the worst picture it can land on: every word has to read at 4.5 to 1 over either, and the glass
    /// lets no more of the picture through than `pane::HIDES_MIN` allows (the strip's own colour at the reading alpha, 0.8, let the print read through).
    #[test]
    fn the_strip_hides_a_white_ad_and_its_black_print_so_every_word_reads_4_5_to_1_in_both_materials_and_sizes() {
        let logo = logo_file();
        let specs = [channel(), long_channel(&logo), no_information(), about(long_channel(&logo))];
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (_h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = Engine::new(&kit).expect("an engine");
                let (mut b, mut s) = (Banner::new(&json!({})), BannerSurface::new(1000));
                for (i, spec) in specs.iter().enumerate() {
                    drawn(&mut kit, &mut engine, &mut s, &mut b, spec.clone(), 1000 + 400 * i as u64);
                    let mut lint = Lint::new();
                    s.feed_lint(&mut kit, &mut lint);
                    let glass = lint.panels()[0].paint;
                    assert!(glass.3 >= (pane::HIDES_MIN * 255.0).round() as u8, "{mode:?} at {scale}, spec {i}: the glass is {} of 255: a picture shows through it", glass.3);
                    for w in lint.words() {
                        let Some(paint) = w.panel.and_then(|p| lint.panels().get(p)).map(|p| p.paint) else { continue };
                        let (ad, print) = (contrast_on(w.ink, paint, [255.0; 3]), contrast_on(w.ink, paint, [0.0; 3]));
                        assert!(ad >= 4.5 && print >= 4.5, "{mode:?} at {scale}, spec {i}: {} reads {ad:.2} to 1 over a white ad and {print:.2} over its black print", w.name);
                    }
                }
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
    fn the_plate_holds_the_logo_or_the_call_signs_letters_and_is_the_same_for_both_so_the_strip_does_not_move_between_channels() {
        let logo = logo_file();
        for scale in [1.0, 2.0] {
            let (_h, mut kit) = testkit::kit_in(Mode::Dark, scale);
            let mut engine = Engine::new(&kit).expect("an engine");
            let (mut b, mut s) = (Banner::new(&json!({})), BannerSurface::new(1000));
            let with = drawn(&mut kit, &mut engine, &mut s, &mut b, long_channel(&logo), 1000);
            assert_eq!((&with["logo"], &with["monogram"]), (&json!(true), &json!("")), "at {scale}: {with}");
            let plate = &with["plate"];
            assert!((num(plate, "w") / num(plate, "h") - 16.0 / 9.0).abs() < 0.02, "at {scale}: a 16:9 plate, as LG's logos are: {with}");
            // a file that is not there is no logo
            let without = drawn(&mut kit, &mut engine, &mut s, &mut b, long_channel("/nowhere/logo.png"), 1200);
            assert_eq!((&without["logo"], &without["monogram"]), (&json!(false), &json!("AC2")), "at {scale}: {without}");
            assert!(without["words"].as_array().is_some_and(|w| w.iter().any(|w| w["name"] == "channel monogram")), "at {scale}: {without}");
            assert_eq!((&with["plate"], &with["bar"]), (&without["plate"], &without["bar"]), "at {scale}: one plate and one strip, with a logo and without");
            let found = lint_of(&mut kit, &s);
            assert!(found.is_empty(), "at {scale}: {found:?}");
        }
    }

    #[test]
    fn a_monogram_is_the_call_sign_where_it_is_short_and_the_first_letters_of_the_words_where_it_is_not() {
        for (name, want) in [
            ("WSB-HD", "WSB"),
            ("KABC-DT", "KABC"),
            ("BUZZR", "BUZZR"),
            ("Dabl", "DABL"),
            ("PBS Kids", "PBS"),
            ("CW 69", "CW"),
            ("Telemundo", "TEL"),
            ("Georgia Public Broadcasting Atlanta Multicast", "GPB"),
            ("Atlanta Channel 2 Action News Weekend Edition", "AC2"),
            ("HD", "HD"),
            ("  - ", ""),
            ("", ""),
        ] {
            assert_eq!(monogram(name), want, "{name:?}");
        }
    }

    #[test]
    fn no_information_is_said_quietly_with_no_time_and_no_line_in_a_narrower_strip() {
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (_h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = Engine::new(&kit).expect("an engine");
                let (mut b, mut s) = (Banner::new(&json!({})), BannerSurface::new(1000));
                let known = drawn(&mut kit, &mut engine, &mut s, &mut b, channel(), 1000);
                let quiet = drawn(&mut kit, &mut engine, &mut s, &mut b, no_information(), 1200);
                assert_eq!((&quiet["quiet"], &quiet["sub"]), (&json!(true), &json!("")), "{mode:?} at {scale}: {quiet}");
                assert!(quiet["track"].is_null(), "{mode:?} at {scale}: no line of how much is left of nothing: {quiet}");
                assert!(num(&quiet["bar"], "w") < num(&known["bar"], "w"), "{mode:?} at {scale}: a narrower strip than one with a programme in it");
                let mut lint = Lint::new();
                s.feed_lint(&mut kit, &mut lint);
                assert!(lint.words().iter().any(|w| w.name == "channel title" && w.muted), "{mode:?} at {scale}: said in the secondary ink");
                assert!(!lint.words().iter().any(|w| w.name == "channel sub"), "{mode:?} at {scale}: no time");
                let found: Vec<String> = lint.findings().iter().map(ToString::to_string).collect();
                assert!(found.is_empty(), "{mode:?} at {scale}: {found:?}");
            }
        }
    }

    #[test]
    fn long_names_are_cut_where_they_stop_fitting_inside_the_safe_area_and_the_time_of_day_is_read_whole() {
        let logo = logo_file();
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (_h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = Engine::new(&kit).expect("an engine");
                let (mut b, mut s) = (Banner::new(&json!({})), BannerSurface::new(1000));
                let g = drawn(&mut kit, &mut engine, &mut s, &mut b, long_channel(&logo), 1000);
                let bar = &g["bar"];
                assert!(num(bar, "x") >= 64.0 && num(bar, "x") + num(bar, "w") <= 1856.0 && num(bar, "y") >= 54.0 && num(bar, "y") + num(bar, "h") <= 1026.0, "{mode:?} at {scale}: inside the safe area: {g}");
                let cut = |k: &str| g[k].as_str().is_some_and(|t| t.ends_with('\u{2026}'));
                assert!(cut("title") && cut("call"), "{mode:?} at {scale}: the title and the name are cut with an ellipsis: {g}");
                assert!(g["sub"].as_str().is_some_and(|t| t.starts_with("8:00 PM \u{2013} 9:00 PM")), "{mode:?} at {scale}: the time of day is read whole: {g}");
                assert_eq!(g["number"], "11-12", "{mode:?} at {scale}: the number is read whole: {g}");
                let found = lint_of(&mut kit, &s);
                assert!(found.is_empty(), "{mode:?} at {scale}: {found:?}");
            }
        }
    }

    #[test]
    fn info_grows_the_strip_upward_from_the_same_foot_with_what_the_programme_is_about_what_is_next_and_its_marks() {
        let logo = logo_file();
        for mode in [Mode::Dark, Mode::Light] {
            for scale in [1.0, 2.0] {
                let (_h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = Engine::new(&kit).expect("an engine");
                let (mut b, mut s) = (Banner::new(&json!({})), BannerSurface::new(1000));
                let compact = drawn(&mut kit, &mut engine, &mut s, &mut b, channel(), 1000);
                let info = drawn(&mut kit, &mut engine, &mut s, &mut b, about(channel()), 1200);
                let (c, i) = (&compact["bar"], &info["bar"]);
                assert!(num(i, "h") > num(c, "h"), "{mode:?} at {scale}: taller: {info}");
                assert_eq!(num(i, "y") + num(i, "h"), num(c, "y") + num(c, "h"), "{mode:?} at {scale}: from the same foot: {info}");
                assert!(num(i, "w") >= (880.0 * (1.0 + 0.6 * (f64::from(scale) - 1.0))).round(), "{mode:?} at {scale}: wide enough for lines of what it is about: {info}");
                assert_eq!(info["detail"], 2, "{mode:?} at {scale}: two lines of what it is about at most: {info}");
                assert!(info["chips"].as_u64() >= Some(2) && info["next"].as_str() == Some("Next \u{b7} Jeopardy!"), "{mode:?} at {scale}: what is next and its marks: {info}");
                let found = lint_of(&mut kit, &s);
                assert!(found.is_empty(), "{mode:?} at {scale}: {found:?}");
                // with long names and a logo, and with nothing to add (the strip stays as it was)
                let long = drawn(&mut kit, &mut engine, &mut s, &mut b, about(long_channel(&logo)), 1400);
                let found = lint_of(&mut kit, &s);
                assert!(found.is_empty(), "{mode:?} at {scale}: {found:?}: {long}");
                let bare = drawn(&mut kit, &mut engine, &mut s, &mut b, json!({"kind": "channel", "number": "5-3", "call": "BUZZR", "title": "No information", "info": true}), 1600);
                assert_eq!((bare["detail"].as_u64(), bare["chips"].as_u64(), bare["next"].as_str()), (Some(0), Some(0), Some("")), "{mode:?} at {scale}: {bare}");
            }
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
                let (mut b, mut s) = (Banner::new(&json!({})), BannerSurface::new(1000));
                drawn(&mut kit, &mut engine, &mut s, &mut b, channel(), 1000);
                (kit.ui.look.ink(), kit.ui.look.colour(PaletteRole::Sheet))
            })
            .collect();
        assert_ne!(colours[0], colours[1], "the banner's inks and sheet are the material's, which differ between dark and light");
    }
}
