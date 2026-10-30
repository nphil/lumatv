//! The weather view's radar page, drawn: the base map as one picture, every frame of the loop as a layer of its own (made once when it
//! arrives, posed over the square of the map it covers, hidden), the home pin, the time bar with the radar's ticks, the forecast's and
//! the clock's mark, and the words that say what is shown and how old it is. A step of the loop or a scrub changes two layers' opacity
//! on the compositor's clock; nothing is painted or uploaded for it.

use super::clock;
use super::model::Home;
use super::radar_view::{AHEAD_S, BACK_S, Kind, Level, MAP_H, MAP_W};
use super::surface::{HIDDEN, faded, glass_of};
use super::widgets::{Canvas, Words};
use crate::kit::Kit;
use crate::kit::pane::PaneKind;
use crate::kit::picture::Picture;
use crate::kit::text::role_style;
use compositor_protocol::{Ease, Props, Rect};
use lumatv_tokens::TypeRole;
use std::collections::HashMap;
use ui::components::{Align, plate};
use ui::geometry::Frame;
use ui::scene::{GlassLayer, ImageLayer, ShapeLayer};
use ui::text::Weight;

const MARGIN: f32 = 64.0;
const TOP: f32 = 140.0;
/// The timeline card under the map.
const BAR_H: f32 = 116.0;
const BAR_GAP: f32 = 16.0;
const CORNER: f32 = 28.0;
/// The forecast is drawn lighter than what a radar saw (the same ramp, 78 %).
const FORECAST_ALPHA: f32 = 0.78;
const FADE_MS: u32 = 240;

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

pub struct RadarPage {
    z: i32,
    _map_glass: GlassLayer,
    map: Option<ImageLayer>,
    frames: HashMap<(i64, u32), (ImageLayer, Kind, Props)>,
    shown: Option<(i64, u32)>,
    _pin: ShapeLayer,
    _pin_ring: ShapeLayer,
    words: Words,
    _bar_glass: GlassLayer,
    track: Option<ImageLayer>,
    thumb: ShapeLayer,
    bar_words: Words,
    track_key: String,
    words_key: String,
    level: Option<Level>,
    reduced: bool,
}

impl RadarPage {
    pub fn new(kit: &mut Kit, z: i32) -> std::io::Result<Self> {
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let accent = kit.ui.look.accent();
        let map_f = Frame::new(MARGIN, TOP, MAP_W as f32, MAP_H as f32);
        let map_glass = kit.ui.scene.glass(MAP_W, MAP_H, z, glass_of(kit, PaneKind::Card, MAP_H as f32, false, false), Props::at(map_f.x, map_f.y));
        let (cap_h, body_h, label_h) = (kit.line(TypeRole::Caption), kit.line(TypeRole::Body), kit.line(TypeRole::Label));
        let mut words = Words::new(
            kit,
            Frame::new(0.0, 0.0, map_f.w, map_f.h),
            z + 6,
            vec![
                // the mode, top left; the two views, top right; the place under the pin; what is wrong, in the middle
                (Frame::new(48.0, 26.0, 600.0, label_h), role_style(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left), String::new()),
                (Frame::new(map_f.w - 640.0 - 32.0, 26.0, 640.0, cap_h), role_style(kit, TypeRole::Caption, None, ink2, Align::Right), String::new()),
                (Frame::new(map_f.w / 2.0 + 18.0, map_f.h / 2.0 - body_h / 2.0, 420.0, body_h), role_style(kit, TypeRole::Body, Some(Weight::SemiBold), ink, Align::Left), String::new()),
                (Frame::new(map_f.w / 2.0 - 360.0, map_f.h / 2.0 + 60.0, 720.0, body_h), role_style(kit, TypeRole::Body, None, ink2, Align::Centre), String::new()),
            ],
        )?;
        words.set_props(Props::at(map_f.x, map_f.y));
        let pin = kit.ui.scene.shape(14, 14, z + 4, plate::solid(ink, 7.0), Props::at(map_f.x + map_f.w / 2.0 - 7.0, map_f.y + map_f.h / 2.0 - 7.0));
        let pin_ring = kit.ui.scene.shape(30, 30, z + 3, plate::ring(faded(ink, 0.55), 15.0, 2.0), Props::at(map_f.x + map_f.w / 2.0 - 15.0, map_f.y + map_f.h / 2.0 - 15.0));
        let bar_f = Frame::new(MARGIN, TOP + MAP_H as f32 + BAR_GAP, MAP_W as f32, BAR_H);
        let bar_glass = kit.ui.scene.glass(bar_f.w as u32, bar_f.h as u32, z, glass_of(kit, PaneKind::Card, bar_f.h, false, false), Props::at(bar_f.x, bar_f.y));
        let thumb = kit.ui.scene.shape(8, 34, z + 5, plate::solid(accent, 4.0), HIDDEN);
        let mut bar_words = Words::new(
            kit,
            Frame::new(0.0, 0.0, bar_f.w, bar_f.h),
            z + 6,
            vec![
                (Frame::new(48.0, 12.0, 900.0, body_h), role_style(kit, TypeRole::Body, Some(Weight::SemiBold), ink, Align::Left), String::new()),
                (Frame::new(bar_f.w - 48.0 - 640.0, 12.0, 640.0, body_h), role_style(kit, TypeRole::Caption, None, ink2, Align::Right), String::new()),
                (Frame::new(48.0, bar_f.h - 14.0 - cap_h, 220.0, cap_h), role_style(kit, TypeRole::Caption, None, ink2, Align::Left), String::new()),
                (Frame::new(bar_f.w / 2.0 - 400.0, bar_f.h - 14.0 - cap_h, 800.0, cap_h), role_style(kit, TypeRole::Caption, None, ink2, Align::Centre), String::new()),
                (Frame::new(bar_f.w - 48.0 - 220.0, bar_f.h - 14.0 - cap_h, 220.0, cap_h), role_style(kit, TypeRole::Caption, None, ink2, Align::Right), String::new()),
            ],
        )?;
        bar_words.set_props(Props::at(bar_f.x, bar_f.y));
        Ok(Self {
            z,
            _map_glass: map_glass,
            map: None,
            frames: HashMap::new(),
            shown: None,
            _pin: pin,
            _pin_ring: pin_ring,
            words,
            _bar_glass: bar_glass,
            track: None,
            thumb,
            bar_words,
            track_key: String::new(),
            words_key: String::new(),
            level: None,
            reduced: false,
        })
    }

    /// The base map's picture, with its corners cut to the card's.
    fn make_map(&mut self, kit: &mut Kit, mut p: Picture) -> std::io::Result<()> {
        if p.width != MAP_W || p.height != MAP_H {
            p = p.fitted(MAP_W, MAP_H, crate::kit::picture::Fit::Cover);
        }
        let mut layer = kit.ui.scene.image(MAP_W, MAP_H, self.z + 1, HIDDEN)?;
        p.paint(&mut layer);
        let (w, h) = (MAP_W as i32, MAP_H as i32);
        let r = CORNER as i32;
        {
            let px = layer.pixels_mut();
            for (cx, cy) in [(r, r), (w - r - 1, r), (r, h - r - 1), (w - r - 1, h - r - 1)] {
                for y in 0..r {
                    for x in 0..r {
                        let (px_, py_) = (if cx < w / 2 { cx - x } else { cx + x }, if cy < h / 2 { cy - y } else { cy + y });
                        let d = (((px_ - cx).pow(2) + (py_ - cy).pow(2)) as f32).sqrt();
                        if d > CORNER {
                            let i = ((py_ * w + px_) * 4) as usize;
                            px[i..i + 4].fill(0);
                        }
                    }
                }
            }
        }
        layer.damage(None);
        layer.set(Props::at(MARGIN, TOP));
        self.map = Some(layer);
        Ok(())
    }

    /// One frame's layer: made when its picture arrives, hidden until it is the one shown.
    fn make_frame(&mut self, kit: &mut Kit, home: &Home, t: i64, rev: u32, p: Picture) -> std::io::Result<()> {
        let r = &home.radar;
        let px = r.frame_px.max(p.width);
        let mut layer = kit.ui.scene.image(p.width, p.height, self.z + 2, HIDDEN)?;
        p.paint(&mut layer);
        layer.damage(None);
        let centre = world_of(r.lon, r.lat);
        let mut at = frame_pose(r.extent, centre, r.level().map_zoom(), px, (MARGIN, TOP));
        at.clip = Some(Rect { x: MARGIN, y: TOP, w: MAP_W as f32, h: MAP_H as f32 });
        at.opacity = 0.0;
        layer.set(at);
        let kind = r.frames.iter().find(|f| f.t == t).map_or(Kind::Observed, |f| f.kind);
        self.frames.insert((t, rev), (layer, kind, at));
        Ok(())
    }

    pub fn update(&mut self, kit: &mut Kit, home: &mut Home, now: u64, reduced: bool) -> std::io::Result<()> {
        self.reduced = reduced;
        let level = home.radar.level();
        if self.level != Some(level) {
            // another view: the map and the frames are of the other level
            self.level = Some(level);
            self.map = None;
            self.frames.clear();
            self.shown = None;
            self.track_key.clear();
        }
        if let Some(p) = home.radar.take_map() {
            self.make_map(kit, p)?;
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
        self.bar(kit, home, now);
        Ok(())
    }

    /// The frame shown: the layer before it fades out and this one in (or they swap at once under reduced motion).
    fn show(&mut self, home: &Home) {
        let want = home.radar.current().map(|f| (f.t, f.rev)).filter(|k| self.frames.contains_key(k));
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

    /// The time bar, its words and the chip: repainted when the set of frames changes, moved when the frame does.
    fn bar(&mut self, kit: &mut Kit, home: &Home, now: u64) {
        let r = &home.radar;
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let accent = kit.ui.look.accent();
        let twelve = home.theme.pointer("/clock/twelveHour").and_then(serde_json::Value::as_bool) != Some(false);
        let win = r.window();
        let (t0, t1) = (r.now_s - BACK_S, r.now_s + AHEAD_S);
        let bar_f = Frame::new(MARGIN, TOP + MAP_H as f32 + BAR_GAP, MAP_W as f32, BAR_H);
        let (track_x, track_w) = (48.0f32, bar_f.w - 96.0);
        let x_of = |t: i64| track_x + track_w * ((t - t0) as f32 / (t1 - t0).max(1) as f32).clamp(0.0, 1.0);
        let track_y = bar_f.h / 2.0 - 6.0;
        // the track: a tick for every frame of the window (held ones strong), the radar's in the ink, the forecast's in the accent, and the clock
        let key = format!("{}|{}|{}", r.now_s / 60, win.iter().map(|f| format!("{}{}", f.t, u8::from(r.holds(f)))).collect::<Vec<_>>().join(","), u8::from(kit.mode == ui::lucent::palette::Mode::Light));
        if key != self.track_key {
            self.track_key = key;
            if self.track.is_none() {
                self.track = kit.ui.scene.image(bar_f.w as u32, bar_f.h as u32, self.z + 4, HIDDEN).ok();
                if let Some(t) = &self.track {
                    t.set(Props::at(bar_f.x, bar_f.y));
                }
            }
            if let Some(layer) = &mut self.track {
                layer.clear();
                let (w, h) = (layer.width(), layer.height());
                let mut c = Canvas { px: layer.pixels_mut(), w, h };
                c.rounded(Frame::new(track_x, track_y + 5.0, track_w, 2.0), 1.0, faded(ink, 0.16));
                let nowx = x_of(r.now_s);
                for f in &win {
                    let x = x_of(f.t);
                    let on = r.holds(f);
                    let colour = if f.kind == Kind::Forecast { faded(accent, if on { 0.95 } else { 0.35 }) } else { faded(ink, if on { 0.8 } else { 0.28 }) };
                    c.rounded(Frame::new(x - 1.5, track_y, 3.0, 12.0), 1.5, colour);
                }
                c.rounded(Frame::new(nowx - 1.0, track_y - 10.0, 2.0, 32.0), 1.0, faded(ink, 0.9));
                layer.damage(None);
            }
        }
        // the thumb
        match r.current() {
            Some(f) => self.thumb.set(Props::at(bar_f.x + x_of(f.t) - 4.0, bar_f.y + track_y - 11.0)),
            None => self.thumb.set(HIDDEN),
        }
        // the words
        let (mode, mode_colour, line, sub) = match r.current() {
            Some(f) => {
                let delta = f.t - r.now_s;
                let when = clock_of(f.t, twelve);
                match f.kind {
                    Kind::Forecast => ("FORECAST", accent, format!("{when}  \u{b7}  in {}", span_words(delta)), "HRRR model forecast, not what a radar saw".to_owned()),
                    _ if delta.abs() < 300 => ("RADAR", ink, format!("{when}  \u{b7}  now"), String::from("What the radar saw")),
                    _ => ("RADAR", ink, format!("{when}  \u{b7}  {} ago", span_words(delta)), String::from("What the radar saw")),
                }
            }
            None => ("RADAR", ink, String::new(), String::new()),
        };
        let status = if r.held().is_empty() {
            match r.status.as_str() {
                _ if !r.reached && r.now_s == 0 => ("Loading the radar\u{2026}".to_owned(), String::new()),
                "unavailable" => (format!("The radar is not available{}", if r.why.is_empty() { String::new() } else { format!(": {}", r.why) }), String::new()),
                _ => (format!("Loading the radar\u{2026}  {} of {}", r.have, r.want.max(r.have)), String::new()),
            }
        } else if r.status == "stale" {
            (format!("Not current: the newest radar is {} old", r.newest_s.map_or("some".into(), |n| span_words(r.now_s - n))), String::new())
        } else {
            (String::new(), String::new())
        };
        let place = home.weather.pointer("/doc/display/place").and_then(serde_json::Value::as_str).unwrap_or("").to_owned();
        let hint = format!("1  Home    2  Region    Left, Right: scrub    OK: {}", if r.playing { "pause" } else { "play" });
        let attribution = if r.attribution.is_empty() { "Radar: Iowa Environmental Mesonet, NOAA  \u{b7}  Map: OpenStreetMap, OpenFreeMap".to_owned() } else { format!("{}  \u{b7}  Map: OpenStreetMap, OpenFreeMap", r.attribution) };
        let key = format!("{mode}|{line}|{sub}|{}|{hint}|{attribution}|{place}|{}", status.0, r.level() as u8);
        if key != self.words_key {
            self.words_key = key;
            let level_words = if r.level() == Level::Home { "Home" } else { "Region" };
            self.words.set_many(kit, &[(0, mode, mode_colour), (1, &format!("{level_words}  \u{b7}  {hint}"), ink2), (2, &place, ink), (3, &status.0, ink)]);
            self.bar_words.set_many(kit, &[(0, &line, ink), (1, &sub, ink2), (2, "\u{2212}2 h", ink2), (3, &attribution, faded(ink2, 0.8)), (4, "+6 h", ink2)]);
        }
        let _ = now;
    }
}
