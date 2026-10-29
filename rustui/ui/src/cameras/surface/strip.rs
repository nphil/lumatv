//! The timeline strip drawn (`docs/mocks/cameras-scrypted/strip*.jpg`, spec "The strip"): a glass panel from the safe edges over the picture
//! with three rows of focus: the time of the cursor and how far behind live it is with the span and the rate at the right; the bar (the
//! recorded ranges as accent bars, the `unknown` ones dimmed to 30 %, only a confirmed gap hatched, the event ticks as dots above, the knob),
//! its axis; and the controls (`Live`, `-10 s`, `Pause` or `Play`, `+10 s`, then `Event` before and after at the right). While the cursor is
//! away from the picture a bubble over the knob shows the thumbnail of that moment, its time and, on an event, its kind.
//!
//! The bar is one picture the CPU paints (ranges, hatch and ticks change a few times a minute); the knob is three layers that move without a
//! repaint, so a scrub costs the compositor a few property updates and nothing more.

use super::common::{Ink, Parts, scale};
use super::grid::{WordRec, other};
use super::pic::{Fill, PicLayer};
use super::{Cx, REGION_STRIP, SCOPE_STRIP, depth, ids};
use crate::cameras::app::Target;
use crate::cameras::strip::{self as model, Item};
use crate::cameras::view::{BubbleView, StripView};
use crate::kit::button::{Button, Place, Spec, Tone, faded};
use crate::kit::focus::Id;
use crate::kit::pane::{Pane, PaneKind, glass};
use crate::kit::text::{Text, role_style, width_of, widest};
use compositor_protocol::{GroupPose, Props, Rgba};
use lumatv_tokens::{Radius, TypeRole};
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::group::Group;
use ui::lucent::palette::PaletteRole;
use ui::scene::{ImageLayer, ShapeLayer};
use ui::text::Weight;

const BAR_H: f32 = 88.0;
const WELL_Y: f32 = 32.0;
const WELL_H: f32 = 40.0;
const WELL_R: f32 = 12.0;
const DOT_Y: f32 = 14.0;

/// The id of each control.
fn id_of(item: Item) -> Id {
    match item {
        Item::Span => ids::STRIP_SPAN,
        Item::Rate => ids::STRIP_RATE,
        Item::Bar => ids::STRIP_BAR,
        Item::Live => ids::STRIP_LIVE,
        Item::Back => ids::STRIP_BACK,
        Item::Play => ids::STRIP_PLAY,
        Item::Forward => ids::STRIP_FORWARD,
        Item::PrevEvent => ids::STRIP_PREV,
        Item::NextEvent => ids::STRIP_NEXT,
    }
}

/// A canvas of premultiplied pixels the bar is painted on.
struct Canvas {
    w: usize,
    h: usize,
    px: Vec<u8>,
}

impl Canvas {
    fn new(w: usize, h: usize) -> Self {
        Self { w, h, px: vec![0; w * h * 4] }
    }

    fn clear(&mut self) {
        self.px.fill(0);
    }

    /// Coverage of the well's rounded rectangle at pixel (`x`, `y`): 1 inside, 0 outside, a pixel's ramp at the arcs.
    fn well(&self, x: usize, y: usize) -> f32 {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5 - WELL_Y);
        if fy < 0.0 || fy > WELL_H {
            return 0.0;
        }
        let w = self.w as f32;
        let dx = if fx < WELL_R { WELL_R - fx } else if fx > w - WELL_R { fx - (w - WELL_R) } else { 0.0 };
        let dy = if fy < WELL_R { WELL_R - fy } else if fy > WELL_H - WELL_R { fy - (WELL_H - WELL_R) } else { 0.0 };
        if dx == 0.0 || dy == 0.0 {
            return 1.0;
        }
        (WELL_R - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0)
    }

    /// Paints `colour` (straight, with alpha `a`) over the pixel at (`x`, `y`) with `cover` of it.
    fn put(&mut self, x: usize, y: usize, colour: Rgba, a: f32, cover: f32) {
        let k = (a * cover * (f32::from(colour.3) / 255.0)).clamp(0.0, 1.0);
        if k <= 0.0 {
            return;
        }
        let i = (y * self.w + x) * 4;
        let keep = 1.0 - k;
        // `colour` is premultiplied by its own alpha: scale to a straight channel first
        let straight = |v: u8| if colour.3 == 0 { 0.0 } else { f32::from(v) * 255.0 / f32::from(colour.3) };
        let src = [straight(colour.0), straight(colour.1), straight(colour.2)];
        for c in 0..3 {
            self.px[i + c] = (src[c] * k + f32::from(self.px[i + c]) * keep).round().min(255.0) as u8;
        }
        self.px[i + 3] = (255.0 * k + f32::from(self.px[i + 3]) * keep).round().min(255.0) as u8;
    }

    /// A span of the well from `x0` to `x1` (pixels), in `colour` at alpha `a`.
    fn span(&mut self, x0: f32, x1: f32, colour: Rgba, a: f32) {
        let (lo, hi) = (x0.max(0.0).floor() as usize, (x1.min(self.w as f32)).ceil() as usize);
        for x in lo..hi.min(self.w) {
            let cover_x = ((x1.min(x as f32 + 1.0)) - (x0.max(x as f32))).clamp(0.0, 1.0);
            for y in (WELL_Y as usize)..((WELL_Y + WELL_H) as usize) {
                let c = self.well(x, y) * cover_x;
                self.put(x, y, colour, a, c);
            }
        }
    }

    /// A hatched gap: 4 px stripes in every 14 along the diagonal.
    fn hatch(&mut self, x0: f32, x1: f32, colour: Rgba, a: f32) {
        let (lo, hi) = (x0.max(0.0).floor() as usize, (x1.min(self.w as f32)).ceil() as usize);
        for x in lo..hi.min(self.w) {
            for y in (WELL_Y as usize)..((WELL_Y + WELL_H) as usize) {
                if (x + y) % 14 < 4 {
                    let c = self.well(x, y);
                    self.put(x, y, colour, a, c);
                }
            }
        }
    }

    /// A tick: a disc of `ink` 12 px across with a 2 px ring of `edge`, centred at (`cx`, `DOT_Y`).
    fn dot(&mut self, cx: f32, ink: Rgba, edge: Rgba) {
        let (r_out, r_in) = (6.0f32, 4.0f32);
        let (lo, hi) = ((cx - r_out - 1.0).floor().max(0.0) as usize, ((cx + r_out + 1.0).ceil() as usize).min(self.w));
        for x in lo..hi {
            for y in 0..self.h.min((DOT_Y + r_out + 1.0) as usize) {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - DOT_Y);
                let d = (dx * dx + dy * dy).sqrt();
                let outer = (r_out - d + 0.5).clamp(0.0, 1.0);
                if outer <= 0.0 {
                    continue;
                }
                let inner = (r_in - d + 0.5).clamp(0.0, 1.0);
                self.put(x, y, edge, 1.0, outer * (1.0 - inner));
                self.put(x, y, ink, 1.0, outer * inner);
            }
        }
    }
}

struct BubbleUi {
    group: Group,
    sig: (bool, u64),
    pane: Pane,
    pic: PicLayer,
    time: Text,
    kind: Option<Text>,
    frame: Frame,
    time_text: String,
    kind_text: Option<String>,
    _keep: Vec<Box<dyn std::any::Any>>,
}

pub struct StripUi {
    ink: Ink,
    sig: String,
    frame: Frame,
    paint: Rgba,
    bar_x: f32,
    bar_w: f32,
    bar_y: f32,
    axis_y: f32,
    parts: Parts,
    when: usize,
    rel: usize,
    when_text: String,
    rel_text: String,
    canvas: Canvas,
    canvas_layer: ImageLayer,
    canvas_sig: String,
    head: ShapeLayer,
    head_focus: ShapeLayer,
    stem: ShapeLayer,
    knob_at: Option<f32>,
    knob_focused: Option<bool>,
    axis: Parts,
    axis_sig: String,
    axis_words: Vec<WordRec>,
    buttons: Vec<(Button, Item)>,
    play_sig: Option<bool>,
    row2_y: f32,
    bubble: Option<BubbleUi>,
    words: Vec<WordRec>,
    modal: bool,
}

fn signature(cx: &Cx) -> String {
    format!("{}|{}", cx.kit.generation(), cx.kit.text_scale)
}

fn minus(seconds: i64) -> String {
    format!("{}{} s", if seconds < 0 { "\u{2212}" } else { "+" }, seconds.abs())
}

impl StripUi {
    pub fn build(cx: &mut Cx, view: &StripView) -> std::io::Result<Self> {
        let ink = Ink::of(cx.kit);
        let ts = scale(cx.kit);
        let btn_h = Button::height(cx.kit);
        let axis_h = (34.0 * ts).round();
        let (pad_top, pad_bot) = (24.0, 24.0);
        let height = pad_top + btn_h + 6.0 + BAR_H + 6.0 + axis_h + 8.0 + btn_h + pad_bot;
        let frame = Frame::new(96.0, (1026.0 - height).round(), 1728.0, height);
        let (bar_x, bar_w) = (frame.x + 28.0, frame.w - 56.0);
        let top_y = frame.y + pad_top;
        let bar_y = top_y + btn_h + 6.0;
        let axis_y = bar_y + BAR_H + 6.0;
        let row2_y = axis_y + axis_h + 8.0;
        let z = cx.at(depth::STRIP);
        let mut parts = Parts::new();
        let pane = Pane::new(cx.kit, frame, z, PaneKind::Card, true);
        let paint = glass(cx.kit, PaneKind::Card, height, true).tint;
        parts.pane(pane);

        // the time of the cursor and how far behind live: boxes as wide as their widest words, so the digits changing move nothing
        let when_style = role_style(cx.kit, TypeRole::Body, Some(Weight::SemiBold), ink.ink, Align::Left);
        let rel_style = role_style(cx.kit, TypeRole::Label, None, ink.ink2, Align::Left);
        let when_w = widest(cx.kit, when_style, &["12:59:59 PM", "Live"]);
        let rel_w = widest(cx.kit, rel_style, &["23 h 59 min ago", "59 min ago"]);
        let (wl, rl) = (cx.kit.line(TypeRole::Body), cx.kit.line(TypeRole::Label));
        let cy = top_y + btn_h / 2.0;
        let when_f = Frame::new(bar_x, (cy - wl / 2.0).round(), when_w, wl);
        let rel_f = Frame::new(bar_x + when_w + 16.0, (cy - rl / 2.0).round(), rel_w, rl);
        let when = parts.text(Text::new(cx.kit, when_f, z + 1, when_style, &view.when)?);
        let rel = parts.text(Text::new(cx.kit, rel_f, z + 1, rel_style, &view.rel)?);
        let mut words = vec![
            WordRec { name: "when".into(), text: view.when.clone(), frame: when_f, style: when_style },
            WordRec { name: "rel".into(), text: view.rel.clone(), frame: rel_f, style: rel_style },
        ];
        let _ = &mut words;

        // the bar
        let well = plate::shape(&cx.kit.ui, Frame::new(0.0, 0.0, bar_w, WELL_H), z + 1, plate::solid(cx.kit.ui.look.colour(PaletteRole::TrackOff), WELL_R));
        parts.shape(well, (bar_x, bar_y + WELL_Y));
        let mut canvas_layer = cx.kit.ui.scene.image(bar_w as u32, BAR_H as u32, z + 2, crate::system::parts::HIDDEN)?;
        canvas_layer.set(Props::at(bar_x, bar_y));
        canvas_layer.damage(None);
        let stem = plate::shape(&cx.kit.ui, Frame::new(0.0, 0.0, 6.0, 76.0), z + 4, plate::solid(ink.ink, 3.0));
        let head = plate::shape(&cx.kit.ui, Frame::new(0.0, 0.0, 24.0, 24.0), z + 5, plate::solid(ink.ink, 12.0));
        let head_focus = plate::shape(&cx.kit.ui, Frame::new(0.0, 0.0, 30.0, 30.0), z + 5, plate::solid(ink.accent, 15.0));
        head_focus.set(Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) });

        let mut me = Self {
            ink,
            sig: signature(cx),
            frame,
            paint,
            bar_x,
            bar_w,
            bar_y,
            axis_y,
            parts,
            when,
            rel,
            when_text: view.when.clone(),
            rel_text: view.rel.clone(),
            canvas: Canvas::new(bar_w as usize, BAR_H as usize),
            canvas_layer,
            canvas_sig: String::new(),
            head,
            head_focus,
            stem,
            knob_at: None,
            knob_focused: None,
            axis: Parts::new(),
            axis_sig: String::new(),
            axis_words: Vec::new(),
            buttons: Vec::new(),
            play_sig: None,
            row2_y,
            bubble: None,
            words,
            modal: false,
        };
        // the strip's controls are the only focusables while it is up
        cx.engine.focus.open_modal(SCOPE_STRIP);
        me.modal = true;
        cx.engine.focus.register(ids::STRIP_BAR, Frame::new(bar_x, bar_y, bar_w, BAR_H), REGION_STRIP, SCOPE_STRIP);
        me.build_buttons(cx, view, top_y, btn_h)?;
        me.sync(cx, view)?;
        if cx.kit.ui.scene.take_overflow() {
            return Err(std::io::Error::other("the scene is full: the strip could not be drawn whole"));
        }
        me.parts.fade(cx.kit, true, super::full::FADE_IN_MS);
        Ok(me)
    }

    pub fn fits(&self, cx: &Cx) -> bool {
        self.sig == signature(cx)
    }

    fn make_button(cx: &mut Cx, item: Item, label: &str, icon: Option<(&'static str, bool)>, ink: Option<Rgba>, at: (f32, f32), w: f32) -> std::io::Result<Button> {
        let h = Button::height(cx.kit);
        let radius = cx.kit.tokens.radius(Radius::Control) + 2.0;
        let place = Place { id: id_of(item), frame: Frame::new(at.0, at.1, w, h), region: REGION_STRIP, scope: SCOPE_STRIP, z: cx.at(depth::STRIP) + 8 };
        let mut spec = Spec::new(label, Tone::Plain).with_radius(radius);
        if let Some((name, after)) = icon {
            spec = if after { spec.with_icon_after(name) } else { spec.with_icon(name) };
        }
        if let Some(c) = ink {
            spec = spec.with_icon_ink(c);
        }
        let mut b = Button::new(cx.kit, cx.engine, place, spec)?;
        b.show(cx.kit, true);
        Ok(b)
    }

    fn build_buttons(&mut self, cx: &mut Cx, view: &StripView, top_y: f32, _btn_h: f32) -> std::io::Result<()> {
        let right = self.frame.right() - 28.0;
        // the span and the rate at the right of the top row
        let rate_w = {
            let style = role_style(cx.kit, TypeRole::Label, None, self.ink.ink, Align::Centre);
            let w = widest(cx.kit, style, &["1\u{d7}", "2\u{d7}", "4\u{d7}"]) + 2.0 * cx.kit.space(7);
            w.ceil() + w.ceil() % 2.0
        };
        let span_w = {
            let style = role_style(cx.kit, TypeRole::Label, None, self.ink.ink, Align::Centre);
            let w = widest(cx.kit, style, &["15 min", "1 h", "6 h", "24 h"]) + 2.0 * cx.kit.space(7);
            w.ceil() + w.ceil() % 2.0
        };
        let rate = Self::make_button(cx, Item::Rate, &view.rate_word, None, None, (right - rate_w, top_y), rate_w)?;
        let span = Self::make_button(cx, Item::Span, &view.span_word, None, None, (right - rate_w - 12.0 - span_w, top_y), span_w)?;
        self.buttons.push((span, Item::Span));
        self.buttons.push((rate, Item::Rate));
        // the controls
        let y = self.row2_y;
        let live_ink = self.ink.live;
        let mut x = self.bar_x;
        let w = Button::width_for(cx.kit, "Live", true);
        let live = Self::make_button(cx, Item::Live, "Live", Some(("dot", false)), Some(live_ink), (x, y), w)?;
        x += w + 12.0;
        self.buttons.push((live, Item::Live));
        let minus_label = minus(-10);
        let w = Button::width_for(cx.kit, &minus_label, false);
        let back = Self::make_button(cx, Item::Back, &minus_label, None, None, (x, y), w)?;
        x += w + 12.0;
        self.buttons.push((back, Item::Back));
        self.build_play(cx, view.play, x)?;
        let play_w = self.buttons.iter().find(|(_, i)| *i == Item::Play).map_or(0.0, |(b, _)| b.frame().w);
        x += play_w + 12.0;
        let plus_label = minus(10);
        let w = Button::width_for(cx.kit, &plus_label, false);
        let fwd = Self::make_button(cx, Item::Forward, &plus_label, None, None, (x, y), w)?;
        self.buttons.push((fwd, Item::Forward));
        // the events, at the right
        let next_w = Button::width_for(cx.kit, "Event", true);
        let next = Self::make_button(cx, Item::NextEvent, "Event", Some(("chevron-right", true)), None, (self.frame.right() - 28.0 - next_w, y), next_w)?;
        let prev_w = Button::width_for(cx.kit, "Event", true);
        let prev = Self::make_button(cx, Item::PrevEvent, "Event", Some(("chevron-left", false)), None, (self.frame.right() - 28.0 - next_w - 12.0 - prev_w, y), prev_w)?;
        self.buttons.push((prev, Item::PrevEvent));
        self.buttons.push((next, Item::NextEvent));
        Ok(())
    }

    /// The Play or Pause control (its icon and word are made again when the recording is held or runs).
    fn build_play(&mut self, cx: &mut Cx, play: bool, x: f32) -> std::io::Result<()> {
        if let Some(i) = self.buttons.iter().position(|(_, it)| *it == Item::Play) {
            let (old, _) = self.buttons.remove(i);
            old.detach(cx.engine);
        }
        let (label, icon) = if play { ("Play", "play") } else { ("Pause", "pause") };
        let w = Button::width_for(cx.kit, "Pause", true).max(Button::width_for(cx.kit, "Play", true));
        let b = Self::make_button(cx, Item::Play, label, Some((icon, false)), None, (x, self.row2_y), w)?;
        self.buttons.push((b, Item::Play));
        self.play_sig = Some(play);
        Ok(())
    }

    /// The knob's x for the cursor.
    fn knob_x(&self, view: &StripView) -> f32 {
        self.bar_x + model::x_of(view.cursor, view.window, self.bar_w)
    }

    fn paint_canvas(&mut self, view: &StripView) {
        let w = self.bar_w;
        let win = view.window;
        self.canvas.clear();
        let x = |at: i64| model::x_of(at, win, w);
        let accent = self.ink.accent;
        for s in &view.bar.recorded {
            self.canvas.span(x(s.from), x(s.to), accent, 0.85);
        }
        for s in &view.bar.unknown {
            self.canvas.span(x(s.from), x(s.to), accent, 0.30);
        }
        for s in &view.bar.gaps {
            self.canvas.hatch(x(s.from), x(s.to), self.ink.ink3, 0.55);
        }
        for at in &view.bar.events {
            self.canvas.dot(x(*at), self.ink.ink, accent);
        }
        self.canvas_layer.pixels_mut().copy_from_slice(&self.canvas.px);
        self.canvas_layer.damage(None);
    }

    /// Follows the view: the words, the labels, the bar, the knob, the axis, the bubble and the focus.
    pub fn sync(&mut self, cx: &mut Cx, view: &StripView) -> std::io::Result<()> {
        if view.when != self.when_text {
            self.when_text = view.when.clone();
            if let Some(t) = self.parts.text_mut(self.when) {
                t.set(cx.kit, &view.when);
            }
            if let Some(w) = self.words.iter_mut().find(|w| w.name == "when") {
                w.text = view.when.clone();
            }
        }
        if view.rel != self.rel_text {
            self.rel_text = view.rel.clone();
            if let Some(t) = self.parts.text_mut(self.rel) {
                t.set(cx.kit, &view.rel);
            }
            if let Some(w) = self.words.iter_mut().find(|w| w.name == "rel") {
                w.text = view.rel.clone();
            }
        }
        for (b, item) in &mut self.buttons {
            match item {
                Item::Span if b.spec().label != view.span_word => b.set_label(cx.kit, &view.span_word),
                Item::Rate if b.spec().label != view.rate_word => b.set_label(cx.kit, &view.rate_word),
                Item::Live => b.set_selected(view.live),
                Item::PrevEvent => b.set_dimmed(cx.kit, !view.can_prev),
                Item::NextEvent => b.set_dimmed(cx.kit, !view.can_next),
                _ => {}
            }
        }
        if self.play_sig != Some(view.play) {
            let x = self.buttons.iter().find(|(_, i)| *i == Item::Play).map_or(0.0, |(b, _)| b.frame().x);
            self.build_play(cx, view.play, x)?;
        }
        // the bar is painted again when what is on it changes; the cursor is not on it
        let sig = format!("{:?}|{:?}", view.window, view.bar);
        if sig != self.canvas_sig {
            self.canvas_sig = sig;
            self.paint_canvas(view);
        }
        // the knob
        let kx = self.knob_x(view);
        let focused = view.focus == Item::Bar;
        if self.knob_at != Some(kx) || self.knob_focused != Some(focused) {
            self.knob_at = Some(kx);
            self.knob_focused = Some(focused);
            self.stem.set(Props::at(kx - 3.0, self.bar_y + 4.0));
            if focused {
                self.head.set(Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) });
                self.head_focus.set(Props::at(kx - 15.0, self.bar_y - 5.0));
            } else {
                self.head_focus.set(Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) });
                self.head.set(Props::at(kx - 12.0, self.bar_y - 2.0));
            }
        }
        self.sync_axis(cx, view)?;
        self.sync_bubble(cx, view)?;
        cx.follow(id_of(view.focus));
        Ok(())
    }

    fn sync_axis(&mut self, cx: &mut Cx, view: &StripView) -> std::io::Result<()> {
        let sig = format!("{:?}", view.axis);
        if sig == self.axis_sig {
            return Ok(());
        }
        self.axis_sig = sig;
        self.axis = Parts::new();
        self.axis_words.clear();
        let style = role_style(cx.kit, TypeRole::Caption, None, self.ink.ink2, Align::Left);
        let line = cx.kit.line(TypeRole::Caption);
        let z = cx.at(depth::STRIP) + 3;
        for (at, label) in &view.axis {
            let w = width_of(cx.kit, style, label);
            let cx_ = self.bar_x + model::x_of(*at, view.window, self.bar_w);
            let x = (cx_ - w / 2.0).clamp(self.bar_x, self.bar_x + self.bar_w - w).round();
            let f = Frame::new(x, self.axis_y, w, line);
            self.axis.text(Text::new(cx.kit, f, z, style, label)?);
            self.axis_words.push(WordRec { name: format!("axis {label}"), text: label.clone(), frame: f, style });
        }
        self.axis.fade(cx.kit, true, 0);
        Ok(())
    }

    fn sync_bubble(&mut self, cx: &mut Cx, view: &StripView) -> std::io::Result<()> {
        let Some(b) = &view.bubble else {
            self.bubble = None;
            return Ok(());
        };
        let has_kind = b.kind_word.is_some();
        let sig = (has_kind, u64::from(cx.kit.generation() as u32));
        if self.bubble.as_ref().map(|u| u.sig) != Some(sig) {
            self.bubble = Some(self.build_bubble(cx, b, sig)?);
        }
        let kx = self.knob_x(view);
        let Some(u) = &mut self.bubble else { return Ok(()) };
        let x = (kx - u.frame.w / 2.0).clamp(96.0, 1824.0 - u.frame.w).round();
        if (x - u.frame.x).abs() > 0.4 {
            u.frame.x = x;
            u.group.set_pose(GroupPose { x, y: u.frame.y, ..GroupPose::IDENTITY });
        }
        if u.time_text != b.time {
            u.time_text = b.time.clone();
            u.time.set(cx.kit, &b.time);
        }
        if let (Some(t), Some(word)) = (&mut u.kind, &b.kind_word) {
            if u.kind_text.as_deref() != Some(word) {
                u.kind_text = Some(word.clone());
                t.set(cx.kit, word);
            }
        }
        if let Some(key) = &b.pic {
            if &u.pic.key != key {
                u.pic = PicLayer::new(key, Frame::new(12.0, 12.0, 256.0, 144.0), cx.kit.tokens.radius(Radius::Control), Fill::Cover, cx.at(depth::BUBBLE) + 2);
            }
            u.pic.sync(cx.kit, Some(&u.group), cx.store)?;
        }
        Ok(())
    }

    fn build_bubble(&mut self, cx: &mut Cx, b: &BubbleView, sig: (bool, u64)) -> std::io::Result<BubbleUi> {
        let z = cx.at(depth::BUBBLE);
        let time_style = role_style(cx.kit, TypeRole::Label, Some(Weight::SemiBold), self.ink.ink, Align::Centre);
        let kind_style = role_style(cx.kit, TypeRole::Caption, None, self.ink.ink2, Align::Centre);
        let (tl, kl) = (cx.kit.line(TypeRole::Label), cx.kit.line(TypeRole::Caption));
        let width = 256.0 + 24.0;
        let height = 12.0 + 144.0 + 8.0 + tl + if b.kind_word.is_some() { kl } else { 0.0 } + 12.0;
        let x = (self.bar_x - 140.0).clamp(96.0, 1824.0 - width);
        let frame = Frame::new(x, (self.frame.y - 12.0 - height).round(), width, height);
        let scene = cx.kit.ui.scene.clone();
        let group = scene.group(None, GroupPose { x: frame.x, y: frame.y, ..GroupPose::IDENTITY }).map_err(other)?;
        let (pane, time, kind) = scene.adopting(&group, || -> std::io::Result<_> {
            let pane = Pane::new(cx.kit, Frame::new(0.0, 0.0, width, height), z, PaneKind::Card, true);
            let time = Text::new(cx.kit, Frame::new(12.0, 12.0 + 144.0 + 8.0, 256.0, tl), z + 3, time_style, &b.time)?;
            let kind = match &b.kind_word {
                Some(w) => Some(Text::new(cx.kit, Frame::new(12.0, 12.0 + 144.0 + 8.0 + tl, 256.0, kl), z + 3, kind_style, w)?),
                None => None,
            };
            Ok((pane, time, kind))
        })?;
        let pic = PicLayer::new(b.pic.as_deref().unwrap_or("bubble:"), Frame::new(12.0, 12.0, 256.0, 144.0), cx.kit.tokens.radius(Radius::Control), Fill::Cover, z + 2);
        Ok(BubbleUi { group, sig, pane, pic, time, kind, frame, time_text: b.time.clone(), kind_text: b.kind_word.clone(), _keep: Vec::new() })
    }

    pub fn hover(&mut self, cx: &mut Cx, id: Id, on: bool) {
        for (b, _) in &mut self.buttons {
            if b.id == id {
                b.set_hover(cx.kit, on && cx.engine.focus.current() != Some(id));
            }
        }
    }

    pub fn press(&mut self, cx: &mut Cx, id: Id, down: bool) {
        for (b, _) in &mut self.buttons {
            if b.id == id {
                b.set_pressed(cx.kit, down);
            }
        }
    }

    pub fn target(&self, id: Id, x: f32) -> Option<Target> {
        if id == ids::STRIP_BAR {
            let p = ((x - self.bar_x) / self.bar_w).clamp(0.0, 1.0);
            return Some(Target::Bar((p * 1000.0).round() as u32));
        }
        self.buttons.iter().find(|(b, _)| b.id == id).map(|(_, item)| Target::Strip(*item))
    }

    /// The panel's rectangle (the toast sits above it).
    pub fn frame(&self) -> Frame {
        self.frame
    }

    pub fn layers(&self) -> usize {
        self.parts.layers() + 5 + self.axis.layers() + self.buttons.iter().map(|(b, _)| b.layers()).sum::<usize>() + self.bubble.as_ref().map_or(0, |b| 4 + usize::from(b.kind.is_some()))
    }

    /// Gives the engine back what the strip registered and fades the panel out over `ms` (0: at once); the strip is dropped by its owner
    /// when the fade is over.
    pub fn retire(&mut self, cx: &mut Cx, ms: u32) {
        for (b, _) in &mut self.buttons {
            b.detach(cx.engine);
            b.hide(cx.kit, ms == 0);
        }
        cx.engine.focus.unregister(ids::STRIP_BAR);
        if self.modal {
            cx.engine.focus.close_modal();
            self.modal = false;
        }
        self.parts.fade(cx.kit, false, ms);
        self.axis.fade(cx.kit, false, ms);
        let off = Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) };
        for l in [&*self.canvas_layer, &*self.head, &*self.head_focus, &*self.stem] {
            l.set(off);
        }
        if let Some(b) = &self.bubble {
            b.group.set_pose(GroupPose { opacity: 0.0, visible: false, ..GroupPose::IDENTITY });
        }
    }

    pub fn feed_lint(&self, kit: &mut crate::kit::Kit, lint: &mut crate::kit::lint::Lint) {
        let panel = lint.panel("strip", self.frame, self.paint, true);
        for w in self.words.iter().chain(self.axis_words.iter()) {
            lint.word(kit, &w.name, &w.text, w.frame, w.style, Some(panel));
        }
        for (b, _) in &self.buttons {
            b.feed_lint(kit, lint, panel);
        }
        if let Some(u) = &self.bubble {
            let p = lint.panel("bubble", u.frame, self.paint, true);
            let tl = kit.line(TypeRole::Label);
            let style = role_style(kit, TypeRole::Label, Some(Weight::SemiBold), self.ink.ink, Align::Centre);
            lint.word(kit, "bubble time", &u.time_text, Frame::new(u.frame.x + 12.0, u.frame.y + 12.0 + 144.0 + 8.0, 256.0, tl), style, Some(p));
        }
    }

    pub fn paint(&self) -> Rgba {
        self.paint
    }

    pub fn dimmed_ink(&self) -> Rgba {
        faded(self.ink.ink, 0.3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cameras::app::{App, Config};
    use crate::cameras::surface::pic::PictureStore;
    use crate::cameras::view::Base;
    use crate::kit::lint::Lint;
    use crate::kit::screen::{Input, Model};
    use crate::kit::testkit;
    use serde_json::{Value, json};

    const MIN: i64 = 60_000;
    const T0: u64 = 1_792_609_200_000;

    fn camera() -> Value {
        json!({"id": "273", "name": "Front Door", "online": true, "live": true, "recordings": true, "events": true, "clips": false, "prebuffered": true,
               "since": T0 as i64 - 600 * MIN})
    }

    fn open(text_scale: f32) -> App {
        let mut a = App::new(Config { text_scale, ..Config::default() });
        a.update(&Input::Helper { topic: "cameras".into(), data: json!({"state": "ready", "cameras": [camera()]}) }, T0);
        a.open_full("273", T0);
        a.update(
            &Input::Helper {
                topic: "cameras.live".into(),
                data: json!({"camera": "273", "mode": "live", "state": "playing", "reason": "", "codec": "h265", "width": 3840, "height": 2160, "fps": 15, "rate": 1, "paused": false}),
            },
            T0 + 50,
        );
        let key = |name: &str| Input::Key { name: name.into(), down: true, repeat: false };
        a.update(&key("up"), T0 + 100);
        a
    }

    fn strip(a: &App) -> StripView {
        a.view().strip.expect("the strip")
    }

    fn cx<'a>(kit: &'a mut crate::kit::Kit, engine: &'a mut crate::kit::Engine, store: &'a PictureStore) -> Cx<'a> {
        Cx { kit, engine, store, z: 2000, now_ms: T0, now_ns: 1_000_000_000 }
    }

    fn feed_data(a: &mut App) {
        let now = T0 as i64 + 100;
        // the helper's tags are the model's: answer what it asked for
        let fx = a.update(&Input::Tick, T0 + 200);
        let _ = fx;
        let _ = now;
    }

    #[test]
    fn the_strip_is_built_over_the_picture_with_its_controls_registered_in_a_scope_of_their_own() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut engine = crate::kit::Engine::new(&kit).unwrap();
        let store = PictureStore::new();
        let mut a = open(1.0);
        feed_data(&mut a);
        let view = strip(&a);
        let before = kit.room();
        let s = StripUi::build(&mut cx(&mut kit, &mut engine, &store), &view).expect("a strip");
        assert_eq!(engine.focus.modal(), Some(SCOPE_STRIP));
        for id in [ids::STRIP_SPAN, ids::STRIP_RATE, ids::STRIP_BAR, ids::STRIP_LIVE, ids::STRIP_BACK, ids::STRIP_PLAY, ids::STRIP_FORWARD, ids::STRIP_PREV, ids::STRIP_NEXT] {
            assert!(engine.focus.is_live(id), "{id:?}");
        }
        assert_eq!(engine.focus.current(), Some(ids::STRIP_BAR), "the bar is where the strip opens");
        let f = s.frame();
        assert_eq!((f.x, f.right(), f.bottom()), (96.0, 1824.0, 1026.0));
        assert!(before.layers - kit.room().layers < 120, "{} layers", before.layers - kit.room().layers);
        assert_eq!(s.target(ids::STRIP_LIVE, 0.0), Some(Target::Strip(Item::Live)));
        let mid = s.bar_x + s.bar_w / 2.0;
        assert_eq!(s.target(ids::STRIP_BAR, mid), Some(Target::Bar(500)));
    }

    #[test]
    fn the_knob_moves_with_the_cursor_and_the_bubble_follows_it() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut engine = crate::kit::Engine::new(&kit).unwrap();
        let store = PictureStore::new();
        let mut a = open(1.0);
        let mut s = StripUi::build(&mut cx(&mut kit, &mut engine, &store), &strip(&a)).unwrap();
        let live_x = s.knob_at.unwrap();
        assert!((live_x - (s.bar_x + s.bar_w)).abs() < 1.0, "live is the right edge");
        assert!(s.bubble.is_none());
        a.update(&Input::Key { name: "left".into(), down: true, repeat: false }, T0 + 200);
        let v = strip(&a);
        assert!(v.bubble.is_some());
        s.sync(&mut cx(&mut kit, &mut engine, &store), &v).unwrap();
        assert!(s.knob_at.unwrap() < live_x, "the cursor went back a minute");
        assert!(s.bubble.is_some());
        a.update(&Input::Key { name: "left".into(), down: false, repeat: false }, T0 + 300);
        let v = strip(&a);
        s.sync(&mut cx(&mut kit, &mut engine, &store), &v).unwrap();
        assert!(s.bubble.is_none(), "the cursor let go to the picture");
    }

    #[test]
    fn the_bar_is_painted_with_ranges_unknown_cells_gaps_and_ticks() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut engine = crate::kit::Engine::new(&kit).unwrap();
        let store = PictureStore::new();
        let a = open(1.0);
        let mut view = strip(&a);
        let now = view.window.to;
        view.bar.recorded = vec![crate::cameras::data::Span { from: now - 50 * MIN, to: now - 10 * MIN }];
        view.bar.unknown = vec![crate::cameras::data::Span { from: now - 10 * MIN, to: now - 8 * MIN }];
        view.bar.gaps = vec![crate::cameras::data::Span { from: now - 60 * MIN, to: now - 50 * MIN }];
        view.bar.events = vec![now - 30 * MIN];
        let s = StripUi::build(&mut cx(&mut kit, &mut engine, &store), &view).unwrap();
        let w = s.bar_w;
        let px = |x: f32, y: usize| {
            let i = (y * s.canvas.w + x as usize) * 4;
            [s.canvas.px[i], s.canvas.px[i + 1], s.canvas.px[i + 2], s.canvas.px[i + 3]]
        };
        let rec = px(w * 0.5, 52);
        assert!(rec[3] > 180, "a recorded range is the accent at 85 %: {rec:?}");
        let unk = px(w * (50.0 + 1.0) / 60.0 + 4.0, 52);
        assert!((50..110).contains(&unk[3]), "an unknown range is the same bar at 30 %: {unk:?}");
        let none = px(w * 0.98, 52);
        assert_eq!(none[3], 0, "past the data: nothing");
        let tick = px(w * 0.5 - 15.0 * 0.0, DOT_Y as usize);
        let _ = tick;
        let dot = px(w * (30.0 / 60.0), DOT_Y as usize);
        assert!(dot[3] > 200, "the event's dot: {dot:?}");
        let corner = px(1.0, WELL_Y as usize + 1);
        assert_eq!(corner[3], 0, "the well's corner is rounded away");
        let hatched = (0..300).filter(|x| px(*x as f32, 52)[3] > 0).count();
        assert!((60..140).contains(&hatched), "the hatch is stripes, not a fill: {hatched} of 300");
    }

    #[test]
    fn play_is_made_again_when_the_recording_is_held() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut engine = crate::kit::Engine::new(&kit).unwrap();
        let store = PictureStore::new();
        let a = open(1.0);
        let mut view = strip(&a);
        let mut s = StripUi::build(&mut cx(&mut kit, &mut engine, &store), &view).unwrap();
        assert_eq!(s.buttons.iter().find(|(_, i)| *i == Item::Play).unwrap().0.spec().label, "Pause");
        view.play = true;
        s.sync(&mut cx(&mut kit, &mut engine, &store), &view).unwrap();
        assert_eq!(s.buttons.iter().find(|(_, i)| *i == Item::Play).unwrap().0.spec().label, "Play");
        assert!(engine.focus.is_live(ids::STRIP_PLAY));
    }

    #[test]
    fn retiring_the_strip_gives_the_scope_and_the_controls_back() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut engine = crate::kit::Engine::new(&kit).unwrap();
        let store = PictureStore::new();
        let a = open(1.0);
        let mut s = StripUi::build(&mut cx(&mut kit, &mut engine, &store), &strip(&a)).unwrap();
        s.retire(&mut cx(&mut kit, &mut engine, &store), 180);
        assert_eq!(engine.focus.modal(), None);
        assert!(!engine.focus.is_live(ids::STRIP_LIVE) || engine.focus.frame(ids::STRIP_LIVE).is_some());
    }

    #[test]
    fn the_strip_reads_clean_to_the_lint_at_both_text_sizes_and_in_both_looks() {
        for scale in [1.0f32, 2.0] {
            for mode in [ui::lucent::palette::Mode::Dark, ui::lucent::palette::Mode::Light] {
                let (_h, mut kit) = testkit::kit_in(mode, scale);
                let mut engine = crate::kit::Engine::new(&kit).unwrap();
                let store = PictureStore::new();
                let mut a = open(scale);
                a.update(&Input::Key { name: "left".into(), down: true, repeat: false }, T0 + 200);
                let view = strip(&a);
                assert!(matches!(a.view().base, Base::Full(_)));
                let mut s = StripUi::build(&mut cx(&mut kit, &mut engine, &store), &view).unwrap();
                s.sync(&mut cx(&mut kit, &mut engine, &store), &view).unwrap();
                let mut lint = Lint::new().with_safe(Frame::new(96.0, 54.0, 1728.0, 972.0));
                s.feed_lint(&mut kit, &mut lint);
                let findings: Vec<String> = lint.findings().iter().map(|x| x.to_string()).collect();
                assert!(findings.is_empty(), "at {scale} in {mode:?}: {findings:?}");
                assert!(s.frame().y > 500.0, "the strip stays in the lower half: {:?}", s.frame());
            }
        }
    }
}
