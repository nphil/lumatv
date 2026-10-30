//! The weather view's radar page, drawn: the base map as one picture over the whole screen, every frame of the loop as a layer of its own (made
//! once when it arrives, posed over the square of the map it covers, hidden, and fading out where the radar's coverage ends), the home pin, and
//! the few controls that float over the map: what is shown (radar or the model's forecast, said in a chip, in the accent for the forecast), the
//! three views, the time bar with the radar's ticks, the forecast's and the clock's mark, and the legend of the colours. A step of the loop or a
//! scrub changes two layers' opacity on the compositor's clock; nothing is painted or uploaded for it.
//!
//! The controls are plates (a colour and an edge), not glass: glass samples the wallpaper's blur, not the map under it.

use super::clock;
use super::model::Home;
use super::radar_view::{AHEAD_S, BACK_S, Kind, Level, MAP_H, MAP_W};
use super::surface::{HIDDEN, faded};
use super::weather_gfx as gfx;
use super::widgets::{Canvas, Words};
use crate::kit::Kit;
use crate::kit::picture::Picture;
use crate::kit::text::role_style;
use compositor_protocol::{Edges, Ease, Fill, Props, Rect, Rgba, Shape};
use lumatv_tokens::TypeRole;
use std::collections::HashMap;
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::lucent::palette::{Mode, PaletteRole};
use ui::scene::{ImageLayer, ShapeLayer};
use ui::text::Weight;

const MARGIN: f32 = 64.0;
/// The controls: the chips under the view's bar, the legend and the time bar above the foot of the screen (inside the TV's safe area).
const CHIP_Y: f32 = 128.0;
const CHIP_H: f32 = 52.0;
const BAR: Frame = Frame { x: MARGIN, y: 924.0, w: 1792.0, h: 92.0 };
const LEGEND: Frame = Frame { x: MARGIN, y: 860.0, w: 330.0, h: 48.0 };
/// The forecast is drawn lighter than what a radar saw (the same ramp, 78 %).
const FORECAST_ALPHA: f32 = 0.78;
const FADE_MS: u32 = 240;
/// Where a frame's coverage ends, its edge fades out over this many pixels.
const EDGE_FEATHER: f32 = 72.0;
/// The time bar's track: where it starts and how wide it is, in the bar's own pixels.
const TRACK_X: f32 = 360.0;
const TRACK_W: f32 = 1384.0;

/// Where a place is in the world's units (Web Mercator, 0 to 1 each way).
fn world_of(lon: f64, lat: f64) -> (f64, f64) {
    let lat = lat.clamp(-85.05, 85.05).to_radians();
    ((lon + 180.0) / 360.0, 0.5 - ((std::f64::consts::FRAC_PI_4 + lat / 2.0).tan().ln()) / (2.0 * std::f64::consts::PI))
}

/// The pose of a frame's layer (`px` square) over the map: the square `extent` covers, on a map of `zoom` centred on the place.
pub fn frame_pose(extent: [f64; 4], centre: (f64, f64), zoom: u8, px: u32, origin: (f32, f32)) -> Props {
    let k = 256.0 * 2f64.powi(i32::from(zoom));
    let (w, h) = (f64::from(MAP_W), f64::from(MAP_H));
    let (x0, y0) = (w / 2.0 + (extent[0] - centre.0) * k, h / 2.0 + (extent[1] - centre.1) * k);
    let side = (extent[2] - extent[0]) * k;
    let scale = (side / f64::from(px.max(1))) as f32;
    // the layer scales about its middle: its top left is where the middle is, less half a layer
    let (cx, cy) = (x0 + side / 2.0, y0 + side / 2.0);
    Props { scale, ..Props::at(origin.0 + cx as f32 - px as f32 / 2.0, origin.1 + cy as f32 - px as f32 / 2.0) }
}

/// What a frame's layer is drawn inside: the screen, and the square the frame covers, which it fades out towards on every side of it that is on
/// the screen (the radar's coverage ends, and the edge of its data is not drawn as an edge).
fn frame_clip(at: &Props, layer_px: u32) -> (Rect, Edges) {
    let side = layer_px as f32 * at.scale;
    let (cx, cy) = (at.x + layer_px as f32 / 2.0, at.y + layer_px as f32 / 2.0);
    let (l, t, r, b) = (cx - side / 2.0, cy - side / 2.0, cx + side / 2.0, cy + side / 2.0);
    let (x0, y0, x1, y1) = (l.max(0.0), t.max(0.0), r.min(MAP_W as f32), b.min(MAP_H as f32));
    let f = |inside: bool| if inside { EDGE_FEATHER } else { 0.0 };
    (Rect { x: x0, y: y0, w: (x1 - x0).max(0.0), h: (y1 - y0).max(0.0) }, Edges::new(f(l > 0.5), f(r < MAP_W as f32 - 0.5), f(t > 0.5), f(b < MAP_H as f32 - 0.5)))
}

/// `10:40 PM`, or `22:40`.
fn clock_of(t: i64, twelve: bool) -> String {
    let c = clock::civil((t * 1000) as u64);
    let (time, ampm) = c.time_words(twelve);
    format!("{time} {ampm}").trim().to_owned()
}

fn span_words(secs: i64) -> String {
    let m = (secs.abs() + 30) / 60;
    if m < 60 {
        format!("{m} min")
    } else if m % 60 == 0 {
        format!("{} h", m / 60)
    } else {
        format!("{} h {} min", m / 60, m % 60)
    }
}

/// The radar's colours from light rain to heavy, as the helper paints them (`RampTheme`): where (0 to 1 over its dBZ range), the colour and its opacity.
fn ramp(dark: bool) -> [(f32, [u8; 3], f32); 6] {
    if dark {
        [(0.0, [156, 207, 216], 0.20), (0.23, [156, 207, 216], 0.59), (0.38, [62, 143, 176], 0.78), (0.52, [94, 168, 201], 0.86), (0.63, [246, 193, 119], 0.92), (0.77, [235, 111, 146], 0.96)]
    } else {
        [(0.0, [86, 148, 159], 0.24), (0.23, [86, 148, 159], 0.67), (0.38, [40, 105, 131], 0.78), (0.52, [60, 127, 153], 0.86), (0.63, [234, 157, 52], 0.92), (0.77, [180, 99, 122], 0.96)]
    }
}

/// A control's plate as a shape (the colour of the sheet mostly opaque over what is under it, with an edge): the view's bar over the radar's map.
pub(super) fn control(kit: &Kit, radius: f32, tint: f32) -> Shape {
    let sheet = kit.ui.look.colour(PaletteRole::Sheet);
    Shape { corner_radius: radius, fill: Fill::Solid(faded(sheet, tint)), border_width: 1.0, border: faded(kit.ui.look.ink(), 0.14) }
}

pub struct RadarPage {
    z: i32,
    map: Option<ImageLayer>,
    globe: Option<ImageLayer>,
    frames: HashMap<(i64, u32), (ImageLayer, Kind, Props)>,
    shown: Option<(i64, u32)>,
    _pin: ShapeLayer,
    _pin_ring: ShapeLayer,
    /// The place by the pin, and what is wrong in the middle of the map.
    place: Words,
    // the controls: each a picture (its plate and what is drawn on it) and its words, so that the page costs the scene few layers
    mode_img: ImageLayer,
    mode_words: Words,
    chips_img: ImageLayer,
    chip_words: Words,
    bar_img: ImageLayer,
    bar_words: Words,
    thumb: ShapeLayer,
    legend_img: ImageLayer,
    legend_words: Words,
    attribution: Words,
    mode_key: String,
    chips_key: String,
    bar_key: String,
    words_key: String,
    level: Option<Level>,
    reduced: bool,
}

/// The chips' widths, and the gap between them (the row ends at the margin).
const CHIPS: [(&str, f32); 3] = [("1  Local", 156.0), ("2  Region", 176.0), ("3  Globe", 156.0)];
const CHIP_GAP: f32 = 10.0;
const MODE_W: f32 = 440.0;
/// A picture of a control is this much larger than its plate on every side, for the plate's edge and its ring.
const PAD: f32 = 3.0;

fn chips_frame() -> Frame {
    let total: f32 = CHIPS.iter().map(|c| c.1).sum::<f32>() + CHIP_GAP * (CHIPS.len() as f32 - 1.0);
    Frame::new(MARGIN + 1792.0 - total, CHIP_Y, total, CHIP_H)
}

/// A control's plate on a picture: the sheet colour mostly opaque over the map, with an edge of the ink (or `ring`, a coloured edge, for what is chosen).
fn paint_plate(kit: &Kit, c: &mut Canvas, f: Frame, radius: f32, tint: f32, ring: Option<Rgba>) {
    let sheet = kit.ui.look.colour(PaletteRole::Sheet);
    let ink = kit.ui.look.ink();
    let edge = ring.unwrap_or(Rgba(ink.0, ink.1, ink.2, 40));
    let grow = if ring.is_some() { 2.5 } else { 1.0 };
    c.rounded(Frame::new(f.x - grow, f.y - grow, f.w + 2.0 * grow, f.h + 2.0 * grow), radius + grow, edge);
    c.rounded(f, radius, Rgba(sheet.0, sheet.1, sheet.2, (tint * 255.0) as u8));
}

/// A picture over `f` grown by `PAD`, painted by `paint` (which gets the plate's frame in the picture's own pixels).
fn control_picture(kit: &mut Kit, f: Frame, z: i32, paint: impl FnOnce(&Kit, &mut Canvas, Frame)) -> std::io::Result<ImageLayer> {
    let mut img = kit.ui.scene.image((f.w + 2.0 * PAD) as u32, (f.h + 2.0 * PAD) as u32, z, HIDDEN)?;
    repaint(&mut img, f, |c, f| paint(kit, c, f));
    img.set(Props::at(f.x - PAD, f.y - PAD));
    Ok(img)
}

fn repaint(img: &mut ImageLayer, f: Frame, paint: impl FnOnce(&mut Canvas, Frame)) {
    img.clear();
    let (w, h) = (img.width(), img.height());
    {
        let mut c = Canvas { px: img.pixels_mut(), w, h };
        paint(&mut c, Frame::new(PAD, PAD, f.w, f.h));
    }
    img.damage(None);
}

impl RadarPage {
    pub fn new(kit: &mut Kit, z: i32) -> std::io::Result<Self> {
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let accent = kit.ui.look.accent();
        let cap_h = kit.line(TypeRole::Caption);
        let pin = kit.ui.scene.shape(14, 14, z + 3, plate::solid(ink, 7.0), Props::at(MAP_W as f32 / 2.0 - 7.0, MAP_H as f32 / 2.0 - 7.0));
        let pin_ring = kit.ui.scene.shape(30, 30, z + 2, plate::ring(faded(ink, 0.55), 15.0, 2.0), Props::at(MAP_W as f32 / 2.0 - 15.0, MAP_H as f32 / 2.0 - 15.0));
        let (cx, cy) = (MAP_W as f32 / 2.0, MAP_H as f32 / 2.0);
        let body = kit.line(TypeRole::Body);
        // the place beside the pin, and what is wrong (loading, not available) under it
        let mut place = Words::new(
            kit,
            Frame::new(0.0, 0.0, 800.0, 170.0),
            z + 6,
            vec![
                (Frame::new(420.0, 40.0 - body / 2.0, 380.0, body), role_style(kit, TypeRole::Body, Some(Weight::SemiBold), ink, Align::Left), String::new()),
                (Frame::new(40.0, 100.0, 720.0, body), role_style(kit, TypeRole::Body, None, ink2, Align::Centre), String::new()),
            ],
        )?;
        place.set_props(Props::at(cx - 400.0, cy - 40.0));
        // what is shown, top left: the mode in a word and what it is
        let mode_f = Frame::new(MARGIN, CHIP_Y, MODE_W, CHIP_H);
        let mode_img = control_picture(kit, mode_f, z + 4, |k, c, f| paint_plate(k, c, f, CHIP_H / 2.0, 0.88, None))?;
        let mut mode_words = Words::new(
            kit,
            Frame::new(0.0, 0.0, MODE_W, CHIP_H),
            z + 7,
            vec![
                (Frame::new(26.0, 0.0, 170.0, CHIP_H), role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left), String::new()),
                (Frame::new(190.0, 0.0, MODE_W - 190.0 - 22.0, CHIP_H), role_style(kit, TypeRole::Caption, None, ink2, Align::Left), String::new()),
            ],
        )?;
        mode_words.set_props(Props::at(mode_f.x, mode_f.y));
        // the three views, top right
        let cf = chips_frame();
        let chips_img = control_picture(kit, cf, z + 4, |k, c, f| {
            let mut x = f.x;
            for (_, w) in CHIPS {
                paint_plate(k, c, Frame::new(x, f.y, w, CHIP_H), CHIP_H / 2.0, 0.88, None);
                x += w + CHIP_GAP;
            }
        })?;
        let mut lines = Vec::new();
        let mut x = 0.0;
        for (label, w) in CHIPS {
            lines.push((Frame::new(x, 0.0, w, CHIP_H), role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink2, Align::Centre), label.to_owned()));
            x += w + CHIP_GAP;
        }
        let mut chip_words = Words::new(kit, Frame::new(0.0, 0.0, cf.w, CHIP_H), z + 7, lines)?;
        chip_words.set_props(Props::at(cf.x, cf.y));
        // the time bar, along the foot
        let bar_img = control_picture(kit, BAR, z + 4, |k, c, f| paint_plate(k, c, f, 30.0, 0.9, None))?;
        let thumb = kit.ui.scene.shape(8, 36, z + 8, plate::solid(accent, 4.0), HIDDEN);
        let now_x = TRACK_X + TRACK_W * BACK_S as f32 / (BACK_S + AHEAD_S) as f32;
        let mut bar_words = Words::new(
            kit,
            Frame::new(0.0, 0.0, BAR.w, BAR.h),
            z + 7,
            vec![
                (Frame::new(96.0, 12.0, 250.0, 40.0), role_style(kit, TypeRole::Body, Some(Weight::SemiBold), ink, Align::Left), String::new()),
                (Frame::new(96.0, 50.0, 250.0, cap_h), role_style(kit, TypeRole::Caption, None, ink2, Align::Left), String::new()),
                (Frame::new(TRACK_X - 6.0, BAR.h - 14.0 - cap_h, 140.0, cap_h), role_style(kit, TypeRole::Caption, None, ink2, Align::Left), String::new()),
                (Frame::new(now_x - 70.0, BAR.h - 14.0 - cap_h, 140.0, cap_h), role_style(kit, TypeRole::Caption, None, ink, Align::Centre), String::new()),
                (Frame::new(TRACK_X + TRACK_W - 134.0, BAR.h - 14.0 - cap_h, 140.0, cap_h), role_style(kit, TypeRole::Caption, None, ink2, Align::Right), String::new()),
                // on the globe, in place of the track: what it shows
                (Frame::new(TRACK_X - 6.0, 12.0, 1100.0, 40.0), role_style(kit, TypeRole::Body, Some(Weight::SemiBold), ink, Align::Left), String::new()),
                (Frame::new(TRACK_X - 6.0, 50.0, 1100.0, cap_h), role_style(kit, TypeRole::Caption, None, ink2, Align::Left), String::new()),
            ],
        )?;
        bar_words.set_props(Props::at(BAR.x, BAR.y));
        // the legend of the colours, above the bar: its plate and the colours in one picture
        let light = kit.mode == Mode::Light;
        let legend_img = control_picture(kit, LEGEND, z + 4, |k, c, f| {
            paint_plate(k, c, f, LEGEND.h / 2.0, 0.88, None);
            let stops: Vec<(f32, Rgba)> = ramp(!light).iter().map(|(p, rgb, a)| (*p / 0.77, Rgba((f32::from(rgb[0]) * a) as u8, (f32::from(rgb[1]) * a) as u8, (f32::from(rgb[2]) * a) as u8, (a * 255.0) as u8))).collect();
            gfx::gradient_bar(c, Frame::new(f.x + 92.0, f.y + LEGEND.h / 2.0 - 6.0, LEGEND.w - 184.0, 12.0), 6.0, &stops);
        })?;
        let mut legend_words = Words::new(
            kit,
            Frame::new(0.0, 0.0, LEGEND.w, LEGEND.h),
            z + 7,
            vec![
                (Frame::new(20.0, 0.0, 80.0, LEGEND.h), role_style(kit, TypeRole::Caption, None, ink2, Align::Left), "Light".to_owned()),
                (Frame::new(LEGEND.w - 100.0, 0.0, 80.0, LEGEND.h), role_style(kit, TypeRole::Caption, None, ink2, Align::Right), "Heavy".to_owned()),
            ],
        )?;
        legend_words.set_props(Props::at(LEGEND.x, LEGEND.y));
        // the attributions the data and the map ask for, small, over the bar's right
        let mut attribution = Words::new(
            kit,
            Frame::new(0.0, 0.0, 1300.0, cap_h),
            z + 7,
            vec![(Frame::new(0.0, 0.0, 1300.0, cap_h), role_style(kit, TypeRole::Caption, None, faded(ink2, 0.85), Align::Right), String::new())],
        )?;
        attribution.set_props(Props::at(MARGIN + 1792.0 - 1300.0, BAR.y - cap_h - 14.0));
        Ok(Self {
            z,
            map: None,
            globe: None,
            frames: HashMap::new(),
            shown: None,
            _pin: pin,
            _pin_ring: pin_ring,
            place,
            mode_img,
            mode_words,
            chips_img,
            chip_words,
            bar_img,
            bar_words,
            thumb,
            legend_img,
            legend_words,
            attribution,
            mode_key: String::new(),
            chips_key: String::new(),
            bar_key: String::new(),
            words_key: String::new(),
            level: None,
            reduced: false,
        })
    }

    /// The base map's picture, over the whole screen.
    fn make_map(&mut self, kit: &mut Kit, mut p: Picture) -> std::io::Result<()> {
        if p.width != MAP_W || p.height != MAP_H {
            p = p.fitted(MAP_W, MAP_H, crate::kit::picture::Fit::Cover);
        }
        let mut layer = kit.ui.scene.image(MAP_W, MAP_H, self.z, HIDDEN)?;
        p.paint(&mut layer);
        layer.damage(None);
        layer.set(Props::at(0.0, 0.0));
        self.map = Some(layer);
        Ok(())
    }

    /// One frame's layer: made when its picture arrives, hidden until it is the one shown.
    fn make_frame(&mut self, kit: &mut Kit, home: &Home, t: i64, rev: u32, p: Picture) -> std::io::Result<()> {
        let r = &home.radar;
        let px = r.frame_px.max(p.width);
        let mut layer = kit.ui.scene.image(p.width, p.height, self.z + 1, HIDDEN)?;
        p.paint(&mut layer);
        layer.damage(None);
        let centre = world_of(r.lon, r.lat);
        let mut at = frame_pose(r.extent, centre, r.level().map_zoom(), px, (0.0, 0.0));
        let (clip, feather) = frame_clip(&at, p.width);
        at.clip = Some(clip);
        at.feather = feather;
        at.opacity = 0.0;
        layer.set(at);
        let kind = r.frames.iter().find(|f| f.t == t).map_or(Kind::Observed, |f| f.kind);
        self.frames.insert((t, rev), (layer, kind, at));
        Ok(())
    }

    pub fn update(&mut self, kit: &mut Kit, home: &mut Home, _now: u64, reduced: bool) -> std::io::Result<()> {
        self.reduced = reduced;
        let level = home.radar.level();
        if self.level != Some(level) {
            // another view: the map and the frames are of the other level
            self.level = Some(level);
            self.map = None;
            self.frames.clear();
            self.shown = None;
            self.bar_key.clear();
            self.chips_key.clear();
            self.mode_key.clear();
            self.words_key.clear();
        }
        if let Some(p) = home.radar.take_map() {
            self.make_map(kit, p)?;
        }
        // the globe: one layer of the helper's picture, centred on the screen, replaced when a new one comes (it turns, or its textures arrive)
        if let Some(p) = home.radar.take_globe() {
            let mut layer = kit.ui.scene.image(p.width, p.height, self.z + 1, HIDDEN)?;
            p.paint(&mut layer);
            layer.damage(None);
            layer.set(Props::at(((MAP_W as f32 - p.width as f32) / 2.0).round(), ((MAP_H as f32 - p.height as f32) / 2.0).round()));
            self.globe = Some(layer);
        }
        if level != Level::Globe {
            self.globe = None;
        }
        if let Some(m) = &self.map {
            m.set(if level == Level::Globe { HIDDEN } else { Props::at(0.0, 0.0) });
        }
        for (t, rev, p) in home.radar.take_arrived() {
            self.make_frame(kit, home, t, rev, p)?;
        }
        // frames the helper replaced or the window has left go
        let r = &home.radar;
        self.frames.retain(|k, _| r.frames.iter().any(|f| f.t == k.0 && f.rev == k.1) || r.frames.is_empty());
        if self.shown.is_some_and(|s| !self.frames.contains_key(&s)) {
            self.shown = None;
        }
        self.show(home);
        self.bar(kit, home);
        Ok(())
    }

    /// The frame shown: the layer before it fades out and this one in (or they swap at once under reduced motion).
    fn show(&mut self, home: &Home) {
        let want = home.radar.current().map(|f| (f.t, f.rev)).filter(|k| self.frames.contains_key(k)).filter(|_| home.radar.level() != Level::Globe);
        if want == self.shown {
            return;
        }
        let pose = |base: Props, kind: Kind, on: bool| Props { opacity: if on { if kind == Kind::Forecast { FORECAST_ALPHA } else { 1.0 } } else { 0.0 }, ..base };
        if let Some(old) = self.shown.and_then(|k| self.frames.get(&k)) {
            let to = pose(old.2, old.1, false);
            if self.reduced {
                old.0.set(to);
            } else {
                old.0.animate(to, FADE_MS, Ease::Linear);
            }
        }
        if let Some(new) = want.and_then(|k| self.frames.get(&k)) {
            let to = pose(new.2, new.1, true);
            if self.reduced {
                new.0.set(to);
            } else {
                new.0.animate(to, FADE_MS, Ease::Linear);
            }
        }
        self.shown = want;
    }

    /// The controls: repainted when what they show changes, moved when the frame does.
    fn bar(&mut self, kit: &mut Kit, home: &Home) {
        let r = &home.radar;
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let accent = kit.ui.look.accent();
        let light = kit.mode == Mode::Light;
        let twelve = home.theme.pointer("/clock/twelveHour").and_then(serde_json::Value::as_bool) != Some(false);
        let (t0, t1) = (r.now_s - BACK_S, r.now_s + AHEAD_S);
        let x_of = |t: i64| TRACK_X + TRACK_W * ((t - t0) as f32 / (t1 - t0).max(1) as f32).clamp(0.0, 1.0);
        let track_y = BAR.h / 2.0 - 6.0 - 4.0;
        let globe = r.level() == Level::Globe;
        // the bar's picture: its plate, the play and pause button (on the globe the pad that turns it), and the track: a tick for every frame of the
        // window (held ones strong), the radar's in the ink, the forecast's in the accent on a tint of it, and the clock
        let win: Vec<_> = if globe { Vec::new() } else { r.window() };
        let key = format!("{}|{}|{}|{}|{}", r.now_s / 60, win.iter().map(|f| format!("{}{}", f.t, u8::from(r.holds(f)))).collect::<Vec<_>>().join(","), u8::from(light), u8::from(r.playing), u8::from(globe));
        if key != self.bar_key {
            self.bar_key = key;
            let playing = r.playing;
            let held: Vec<bool> = win.iter().map(|f| r.holds(f)).collect();
            repaint(&mut self.bar_img, BAR, |c, f| {
                paint_plate(kit, c, f, 30.0, 0.9, None);
                let at = |x: f32, y: f32| (x + PAD, y + PAD);
                if !win.is_empty() {
                    let nowx = x_of(r.now_s);
                    // what is ahead of the clock is the model's: tinted, so that it reads apart from what the radar saw
                    let (tx, ty) = at(nowx, track_y - 12.0);
                    c.rounded(Frame::new(tx, ty, TRACK_X + TRACK_W - nowx + 10.0, 40.0), 12.0, faded(accent, 0.13));
                    let (lx, ly) = at(TRACK_X, track_y + 5.0);
                    c.rounded(Frame::new(lx, ly, TRACK_W, 2.0), 1.0, faded(ink, 0.18));
                    for (f, on) in win.iter().zip(&held) {
                        let (x, y) = at(x_of(f.t) - 1.5, track_y);
                        let colour = if f.kind == Kind::Forecast { faded(accent, if *on { 0.95 } else { 0.35 }) } else { faded(ink, if *on { 0.8 } else { 0.28 }) };
                        c.rounded(Frame::new(x, y, 3.0, 12.0), 1.5, colour);
                    }
                    let (x, y) = at(nowx - 1.0, track_y - 10.0);
                    c.rounded(Frame::new(x, y, 2.0, 32.0), 1.0, faded(ink, 0.9));
                }
                let (bx, by) = at(50.0, BAR.h / 2.0);
                if !globe {
                    gfx::arc(c, bx, by, 27.0, 3.0, 0.0, std::f32::consts::TAU, accent);
                    if playing {
                        // pause: two bars
                        c.rounded(Frame::new(bx - 9.0, by - 10.0, 6.0, 20.0), 2.0, ink);
                        c.rounded(Frame::new(bx + 3.0, by - 10.0, 6.0, 20.0), 2.0, ink);
                    } else {
                        gfx::head(c, (bx + 11.0, by), std::f32::consts::FRAC_PI_2, 20.0, 22.0, ink);
                    }
                } else {
                    // the pad: four arrows round the OK that zooms
                    for q in 0..4 {
                        let a = q as f32 * std::f32::consts::FRAC_PI_2;
                        let (sa, ca) = (a.sin(), -a.cos());
                        gfx::head(c, (bx + sa * 26.0, by + ca * 26.0), a, 11.0, 14.0, ink);
                    }
                    gfx::arc(c, bx, by, 10.0, 3.0, 0.0, std::f32::consts::TAU, accent);
                }
            });
        }
        // the thumb
        match r.current().filter(|_| !globe) {
            Some(f) => self.thumb.set(Props::at(BAR.x + x_of(f.t) - 4.0, BAR.y + track_y - 12.0)),
            None => self.thumb.set(HIDDEN),
        }
        // the legend is for the radar's colours: not on the globe
        self.legend_img.set(if globe { HIDDEN } else { Props::at(LEGEND.x - PAD, LEGEND.y - PAD) });
        // the words
        let (mode, mode_colour, line, sub) = if globe {
            ("GLOBE", ink, String::new(), String::new())
        } else {
            match r.current() {
                Some(f) => {
                    let delta = f.t - r.now_s;
                    let when = clock_of(f.t, twelve);
                    match f.kind {
                        Kind::Forecast => ("FORECAST", accent, when, format!("in {}", span_words(delta))),
                        _ if delta.abs() < 300 => ("RADAR", ink, when, String::from("now")),
                        _ => ("RADAR", ink, when, format!("{} ago", span_words(delta))),
                    }
                }
                None => ("RADAR", ink, String::new(), String::new()),
            }
        };
        let detail = if globe { "" } else if r.current().is_some_and(|f| f.kind == Kind::Forecast) { "model, not radar" } else { "what the radar saw" };
        // the forecast's chip is tinted with the accent, the radar's is plain; the chip of the view that is shown is ringed
        let mode_key = format!("{mode}|{}", u8::from(light));
        if mode_key != self.mode_key {
            self.mode_key = mode_key;
            let forecast = mode == "FORECAST";
            repaint(&mut self.mode_img, Frame::new(0.0, 0.0, MODE_W, CHIP_H), |c, f| {
                paint_plate(kit, c, f, CHIP_H / 2.0, 0.88, None);
                if forecast {
                    c.rounded(f, CHIP_H / 2.0, faded(accent, 0.20));
                }
            });
        }
        let chips_key = format!("{}|{}", r.level() as u8, u8::from(light));
        if chips_key != self.chips_key {
            self.chips_key = chips_key;
            let level = r.level() as usize;
            repaint(&mut self.chips_img, chips_frame(), |c, f| {
                let mut x = f.x;
                for (i, (_, w)) in CHIPS.iter().enumerate() {
                    paint_plate(kit, c, Frame::new(x, f.y, *w, CHIP_H), CHIP_H / 2.0, if i == level { 0.97 } else { 0.88 }, (i == level).then_some(accent));
                    x += w + CHIP_GAP;
                }
            });
        }
        let status = if globe {
            if r.globe_ready() { String::new() } else { "Loading the globe\u{2026}".to_owned() }
        } else if r.held().is_empty() {
            match r.status.as_str() {
                _ if !r.reached && r.now_s == 0 => "Loading the radar\u{2026}".to_owned(),
                "unavailable" => format!("The radar is not available{}", if r.why.is_empty() { String::new() } else { format!(": {}", r.why) }),
                _ => format!("Loading the radar\u{2026}  {} of {}", r.have, r.want.max(r.have)),
            }
        } else if r.status == "stale" {
            format!("Not current: the newest radar is {} old", r.newest_s.map_or("some".into(), |n| span_words(r.now_s - n)))
        } else {
            String::new()
        };
        let place = home.weather.pointer("/doc/display/place").and_then(serde_json::Value::as_str).unwrap_or("").to_owned();
        let attribution = if globe { "Rain: NASA IMERG, about 6 h behind  \u{b7}  Cloud: satellite  \u{b7}  Earth: NASA".to_owned() } else { "Radar: NEXRAD and HRRR via Iowa Environmental Mesonet  \u{b7}  Map: OpenStreetMap, OpenFreeMap".to_owned() };
        let key = format!("{mode}|{line}|{sub}|{detail}|{status}|{attribution}|{place}|{}", r.level() as u8);
        if key != self.words_key {
            self.words_key = key;
            let level = r.level() as usize;
            self.place.set_many(kit, &[(0, &place, ink), (1, &status, ink)]);
            self.mode_words.set_many(kit, &[(0, mode, mode_colour), (1, detail, ink2)]);
            let chip: Vec<(usize, &str, Rgba)> = CHIPS.iter().enumerate().map(|(i, (label, _))| (i, *label, if i == level { ink } else { ink2 })).collect();
            self.chip_words.set_many(kit, &chip);
            let (left, now_l, right) = if globe { ("", "", "") } else { ("\u{2212}2 h", "Now", "+6 h") };
            let (title, about) = if globe { ("Rain and cloud from space", "NASA rain, about 6 h behind; satellite cloud") } else { ("", "") };
            self.bar_words.set_many(kit, &[(0, &line, ink), (1, &sub, ink2), (2, left, ink2), (3, now_l, ink), (4, right, ink2), (5, title, ink), (6, about, ink2)]);
            self.legend_words.set_many(kit, &[(0, if globe { "" } else { "Light" }, ink2), (1, if globe { "" } else { "Heavy" }, ink2)]);
            self.attribution.set_many(kit, &[(0, &attribution, faded(ink2, 0.85))]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_fades_out_where_its_square_ends_on_the_screen_and_not_at_the_screens_own_edge() {
        // a square of 400 px in the middle of the screen: it fades on every side
        let (r, e) = frame_clip(&Props { scale: 1.0, ..Props::at(760.0, 340.0) }, 400);
        assert_eq!((r.x, r.y, r.w, r.h), (760.0, 340.0, 400.0, 400.0));
        assert!([e.left, e.right, e.top, e.bottom].iter().all(|f| *f == EDGE_FEATHER));
        // one that is five times as large covers the screen: the screen's edges are hard
        let (r, e) = frame_clip(&Props { scale: 5.0, ..Props::at(760.0, 340.0) }, 400);
        assert_eq!((r.x, r.y, r.w, r.h), (0.0, 0.0, MAP_W as f32, MAP_H as f32));
        assert_eq!(e, Edges::NONE);
        // one that ends on the screen's left only fades there
        let (r, e) = frame_clip(&Props { scale: 5.0, ..Props::at(260.0, 340.0) }, 400);
        assert_eq!((r.x, e.left, e.right), (460.0, EDGE_FEATHER, 0.0), "{r:?}");
    }

    #[test]
    fn the_radars_colours_run_from_light_rain_to_heavy_in_both_looks() {
        for dark in [true, false] {
            let r = ramp(dark);
            assert!(r.windows(2).all(|w| w[0].0 < w[1].0 && w[0].2 <= w[1].2), "ordered, and the heavier the more opaque");
            assert_eq!(r[0].0, 0.0);
        }
    }

    #[test]
    fn a_place_is_where_web_mercator_puts_it_and_a_frame_is_posed_on_the_square_it_covers() {
        let (x, y) = world_of(0.0, 0.0);
        assert!((x - 0.5).abs() < 1e-9 && (y - 0.5).abs() < 1e-9);
        // a frame whose square is centred on the place sits on the screen's middle
        let c = world_of(-84.39, 33.75);
        let half = 0.01;
        let pose = frame_pose([c.0 - half, c.1 - half, c.0 + half, c.1 + half], c, 8, 384, (0.0, 0.0));
        let side = 2.0 * half * 256.0 * 256.0;
        assert!((pose.x + 192.0 - MAP_W as f32 / 2.0).abs() < 1.0 && (pose.y + 192.0 - MAP_H as f32 / 2.0).abs() < 1.0, "{pose:?}");
        assert!((pose.scale * 384.0 - side as f32).abs() < 0.5);
    }
}
