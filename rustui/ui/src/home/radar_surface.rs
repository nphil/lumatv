//! The weather view's radar page, drawn: the base map as a picture of the cells that cover the screen (and a coarser one under it), the radar's
//! frame on show and the one after it as layers of their own (each made once when its picture arrives, posed over the part of the world it
//! covers, the frame hidden until it is the one shown), the home pin, and the few controls that float over the map: what is shown (radar or the
//! model's forecast, said in a chip, in the accent for the forecast), the time bar with the radar's ticks, the forecast's and the clock's mark,
//! and the legend of the colours. A step of the loop or a scrub changes two layers' opacity on the compositor's clock; nothing is painted or
//! uploaded for it. The pictures are the page's model's to ask for ([`super::radar_pics`]) and the layers of those it lets go are dropped here.
//!
//! The zoom is the compositor's too: the pictures hang under one group whose scale (and, for a zoom about the pointer, whose place) is bound to a
//! motion track, so that a turn of the wheel is one `settle` and the glide to it costs this process no frame ([`super::radar_zoom`]). The pictures
//! are posed in that group's own space, which is the world at the opening zoom with the place at the middle of the screen, and the view that the
//! track and the pan come to is told to the model ([`Radar::set_view`]), which asks for the pictures of that view.
//!
//! The controls are plates (a colour and an edge), not glass: glass samples the wallpaper's blur, not the map under it.

use super::clock;
use super::model::Home;
use super::radar_pics::{self, MapPic, Role, View};
use super::radar_view::{AHEAD_S, BACK_S, Kind, MAP_H, MAP_W};
use super::radar_zoom::{Pan, S_MAX, S_MIN, Z_HOME};
use super::surface::{HIDDEN, faded};
use super::weather_gfx as gfx;
use super::widgets::{Canvas, Words};
use crate::kit::Kit;
use crate::kit::text::role_style;
use compositor_protocol::{Ease, Fill, Props, Rgba, Shape};
use lumatv_tokens::TypeRole;
use std::collections::HashMap;
use ui::components::{Align, Style, plate};
use ui::geometry::Frame;
use ui::group::{Group, GroupPose, PoseBinding, PoseProperties};
use ui::lucent::palette::{Mode, PaletteRole};
use ui::motion::{Motion, MotionBounds, Units};
use ui::scene::{ImageLayer, ShapeLayer};
use ui::text::Weight;

const MARGIN: f32 = 64.0;
/// The controls: the mode chip under the view's bar, the legend and the time bar above the foot of the screen (inside the TV's safe area); their sizes are [`Geo`].
const CHIP_Y: f32 = 128.0;
/// The forecast is drawn lighter than what a radar saw (the same ramp, 78 %).
const FORECAST_ALPHA: f32 = 0.78;
const FADE_MS: u32 = 240;

/// Where a place is in the world's units (Web Mercator, 0 to 1 each way).
fn world_of(lon: f64, lat: f64) -> (f64, f64) {
    let lat = lat.clamp(-85.05, 85.05).to_radians();
    ((lon + 180.0) / 360.0, 0.5 - ((std::f64::consts::FRAC_PI_4 + lat / 2.0).tan().ln()) / (2.0 * std::f64::consts::PI))
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

/// A picture's layer, as the page's model made it known.
struct PicLayer {
    layer: ImageLayer,
    role: Role,
    /// Whether the frame is the model's forecast (drawn lighter) and not a radar's.
    kind: Kind,
    /// Where the layer is in the group's space, at full opacity.
    at: Props,
}

pub struct RadarPage {
    z: i32,
    /// The pictures hang under `zoom`, the group the track scales. The pin follows the track's move, not its scale.
    zoom: Group,
    pin_group: Group,
    track: Motion,
    /// How the view is moved off the middle as the track's scale changes, and the zoom it is for.
    pan: Pan,
    zoom_seen: Option<(u32, Option<(u32, u32)>)>,
    globe: Option<ImageLayer>,
    /// The layers of the pictures the model holds, by its number for them.
    pics: HashMap<u64, PicLayer>,
    /// The radar frame shown.
    shown: Option<u64>,
    /// Whether the coarse map under the map is seen: only while the map does not cover the view (it is drawn over the whole screen, and a layer that
    /// is not seen costs the graphics chip nothing).
    backdrop_seen: bool,
    _pin: ShapeLayer,
    _pin_ring: ShapeLayer,
    /// The place by the pin, and what is wrong in the middle of the map.
    place: Words,
    // the controls: each a picture (its plate and what is drawn on it) and its words, so that the page costs the scene few layers
    mode_img: ImageLayer,
    mode_words: Words,
    bar_img: ImageLayer,
    bar_words: Words,
    thumb: ShapeLayer,
    legend_img: ImageLayer,
    legend_words: Words,
    attribution: Words,
    geo: Geo,
    mode_key: String,
    bar_key: String,
    words_key: String,
    /// Whether the globe is what is shown, as the page has taken it up.
    shown_globe: Option<bool>,
    reduced: bool,
}

/// A picture of a control is this much larger than its plate on every side, for the plate's edge and its ring.
const PAD: f32 = 3.0;

/// The chrome's sizes. At the 100 % text size they are the layout drawn first; at a larger one the words and their plates grow together, to at most
/// 1.3 times (the map is the page, and the chrome is not to take it), and a plate is as wide as its words need when that is more.
#[derive(Clone, Copy)]
struct Geo {
    /// How much larger than at 100 % the chrome is.
    k: f32,
    chip_h: f32,
    mode_w: f32,
    /// Where the mode's second word starts (the first is the radar, the forecast or the globe).
    mode_caption_x: f32,
    bar: Frame,
    legend: Frame,
    /// The time bar's track in the bar's own pixels: where it starts, how wide it is, and where its ticks are.
    track_x: f32,
    track_w: f32,
    track_y: f32,
    /// The line height of a caption.
    cap_h: f32,
}

/// A style of the role's face at the chrome's size: the role at the 100 % text size, times `k`.
fn style_k(kit: &Kit, role: TypeRole, weight: Option<Weight>, colour: Rgba, align: Align, k: f32) -> Style {
    let mut s = role_style(kit, role, weight, colour, align);
    s.ty.px = (kit.ty(role).px as f32 / kit.text_scale * k).round().max(8.0) as u32;
    s
}

fn measured(kit: &mut Kit, text: &str, style: &Style) -> f32 {
    kit.ui.fonts.measure(text, style.ty).width.ceil()
}

impl Geo {
    fn new(kit: &mut Kit) -> Self {
        let k = kit.text_scale.clamp(1.0, 1.3);
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let label = style_k(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left, k);
        let caption = style_k(kit, TypeRole::Caption, None, ink2, Align::Left, k);
        let label_w = ["RADAR", "FORECAST", "GLOBE"].iter().map(|t| measured(kit, t, &label)).fold(0.0, f32::max);
        let caption_w = ["what the radar saw", "model, not radar"].iter().map(|t| measured(kit, t, &caption)).fold(0.0, f32::max);
        let mode_caption_x = (190.0 * k).max(26.0 * k + label_w + 24.0 * k);
        let bar_h = (92.0 * k).round();
        let bar = Frame::new(MARGIN, 1016.0 - bar_h, 1792.0, bar_h);
        let legend_h = (48.0 * k).round();
        let track_x = (360.0 * k).round();
        Self {
            k,
            chip_h: (52.0 * k).round(),
            mode_w: (440.0 * k).max(mode_caption_x + caption_w + 22.0 * k),
            mode_caption_x,
            bar,
            legend: Frame::new(MARGIN, bar.y - 16.0 - legend_h, (330.0 * k).round(), legend_h),
            track_x,
            track_w: bar.w - track_x - 48.0 * k,
            track_y: (28.0 * k).round(),
            cap_h: ui::components::line_height(caption.ty),
        }
    }

    fn mode(&self) -> Frame {
        Frame::new(MARGIN, CHIP_Y, self.mode_w, self.chip_h)
    }

    /// Where Now is on the track.
    fn now_x(&self) -> f32 {
        self.track_x + self.track_w * BACK_S as f32 / (BACK_S + AHEAD_S) as f32
    }
}

/// Binds the place of `groups` to `track`: moved off the middle by `pan` at the scale the track is at.
fn bind_pan(groups: &[&Group], track: &Motion, pan: Pan) -> std::io::Result<()> {
    let (from, to) = (pan.at(S_MIN), pan.at(S_MAX));
    for g in groups {
        let binding = PoseBinding { values: [S_MIN, S_MAX], from: GroupPose { x: from.0, y: from.1, ..GroupPose::IDENTITY }, to: GroupPose { x: to.0, y: to.1, ..GroupPose::IDENTITY }, properties: PoseProperties::TRANSLATION };
        g.bind(track, binding).map_err(|e| std::io::Error::other(format!("{e}")))?;
    }
    Ok(())
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
    /// `parent` is the group the page hangs under (the view's slide of it).
    pub fn new(kit: &mut Kit, z: i32, parent: &Group) -> std::io::Result<Self> {
        let other = |e: ui::error::UiError| std::io::Error::other(format!("{e}"));
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let accent = kit.ui.look.accent();
        let g = Geo::new(kit);
        let (k, cap_h) = (g.k, g.cap_h);
        let (cx, cy) = (MAP_W as f32 / 2.0, MAP_H as f32 / 2.0);
        // the zoom: the group the pictures hang under scales about the middle of the screen on the track, which is the scale itself (1 at the
        // opening view); the pin keeps its size and follows only the move the zoom makes when it is about somewhere else than the middle
        let scene = kit.ui.scene.clone();
        let about_middle = GroupPose { pivot_x: cx, pivot_y: cy, ..GroupPose::IDENTITY };
        let zoom = scene.group(Some(parent.id()), about_middle).map_err(other)?;
        let pin_group = scene.group(Some(parent.id()), GroupPose::IDENTITY).map_err(other)?;
        let track = scene.motion(1.0, MotionBounds::range(S_MIN, S_MAX, Units::Progress)).map_err(other)?;
        zoom.bind(&track, PoseBinding { values: [S_MIN, S_MAX], from: GroupPose { scale: S_MIN, ..about_middle }, to: GroupPose { scale: S_MAX, ..about_middle }, properties: PoseProperties::SCALE }).map_err(other)?;
        bind_pan(&[&zoom, &pin_group], &track, Pan::NONE)?;
        let (pin, pin_ring) = scene.adopting(&pin_group, || {
            let pin = kit.ui.scene.shape(14, 14, z + 3, plate::solid(ink, 7.0), Props::at(cx - 7.0, cy - 7.0));
            let ring = kit.ui.scene.shape(30, 30, z + 2, plate::ring(faded(ink, 0.55), 15.0, 2.0), Props::at(cx - 15.0, cy - 15.0));
            (pin, ring)
        });
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
        let mode_f = g.mode();
        let mode_img = control_picture(kit, mode_f, z + 4, |kit, c, f| paint_plate(kit, c, f, g.chip_h / 2.0, 0.88, None))?;
        let mut mode_words = Words::new(
            kit,
            Frame::new(0.0, 0.0, g.mode_w, g.chip_h),
            z + 7,
            vec![
                (Frame::new(26.0 * k, 0.0, g.mode_caption_x - 26.0 * k, g.chip_h), style_k(kit, TypeRole::Label, Some(Weight::SemiBold), ink, Align::Left, k), String::new()),
                (Frame::new(g.mode_caption_x, 0.0, g.mode_w - g.mode_caption_x - 22.0 * k, g.chip_h), style_k(kit, TypeRole::Caption, None, ink2, Align::Left, k), String::new()),
            ],
        )?;
        mode_words.set_props(Props::at(mode_f.x, mode_f.y));
        // the time bar, along the foot
        let bar_img = control_picture(kit, g.bar, z + 4, |kit, c, f| paint_plate(kit, c, f, 30.0 * k, 0.9, None))?;
        let thumb = kit.ui.scene.shape(8, (30.0 * k).round() as u32, z + 8, plate::solid(accent, 4.0), HIDDEN);
        let (now_x, foot) = (g.now_x(), g.bar.h - 14.0 * k - cap_h);
        let mut bar_words = Words::new(
            kit,
            Frame::new(0.0, 0.0, g.bar.w, g.bar.h),
            z + 7,
            vec![
                (Frame::new(96.0 * k, 12.0 * k, 250.0 * k, 40.0 * k), style_k(kit, TypeRole::Body, Some(Weight::SemiBold), ink, Align::Left, k), String::new()),
                (Frame::new(96.0 * k, 50.0 * k, 250.0 * k, cap_h), style_k(kit, TypeRole::Caption, None, ink2, Align::Left, k), String::new()),
                (Frame::new(g.track_x - 6.0 * k, foot, 140.0 * k, cap_h), style_k(kit, TypeRole::Caption, None, ink2, Align::Left, k), String::new()),
                (Frame::new(now_x - 70.0 * k, foot, 140.0 * k, cap_h), style_k(kit, TypeRole::Caption, None, ink, Align::Centre, k), String::new()),
                (Frame::new(g.track_x + g.track_w - 134.0 * k, foot, 140.0 * k, cap_h), style_k(kit, TypeRole::Caption, None, ink2, Align::Right, k), String::new()),
                // on the globe, in place of the track: what it shows
                (Frame::new(g.track_x - 6.0 * k, 12.0 * k, 1100.0, 40.0 * k), style_k(kit, TypeRole::Body, Some(Weight::SemiBold), ink, Align::Left, k), String::new()),
                (Frame::new(g.track_x - 6.0 * k, 50.0 * k, 1100.0, cap_h), style_k(kit, TypeRole::Caption, None, ink2, Align::Left, k), String::new()),
            ],
        )?;
        bar_words.set_props(Props::at(g.bar.x, g.bar.y));
        // the legend of the colours, above the bar: its plate and the colours in one picture
        let light = kit.mode == Mode::Light;
        let legend_img = control_picture(kit, g.legend, z + 4, |kit, c, f| {
            paint_plate(kit, c, f, g.legend.h / 2.0, 0.88, None);
            let stops: Vec<(f32, Rgba)> = ramp(!light).iter().map(|(p, rgb, a)| (*p / 0.77, Rgba((f32::from(rgb[0]) * a) as u8, (f32::from(rgb[1]) * a) as u8, (f32::from(rgb[2]) * a) as u8, (a * 255.0) as u8))).collect();
            gfx::gradient_bar(c, Frame::new(f.x + 92.0 * k, f.y + g.legend.h / 2.0 - 6.0, g.legend.w - 184.0 * k, 12.0), 6.0, &stops);
        })?;
        let mut legend_words = Words::new(
            kit,
            Frame::new(0.0, 0.0, g.legend.w, g.legend.h),
            z + 7,
            vec![
                (Frame::new(20.0 * k, 0.0, 80.0 * k, g.legend.h), style_k(kit, TypeRole::Caption, None, ink2, Align::Left, k), "Light".to_owned()),
                (Frame::new(g.legend.w - 100.0 * k, 0.0, 80.0 * k, g.legend.h), style_k(kit, TypeRole::Caption, None, ink2, Align::Right, k), "Heavy".to_owned()),
            ],
        )?;
        legend_words.set_props(Props::at(g.legend.x, g.legend.y));
        // the attributions the data and the map ask for, small, over the bar's right
        let att_w = 1792.0 - g.legend.w - 24.0 * k;
        let mut attribution = Words::new(
            kit,
            Frame::new(0.0, 0.0, att_w, cap_h),
            z + 7,
            vec![(Frame::new(0.0, 0.0, att_w, cap_h), style_k(kit, TypeRole::Caption, None, faded(ink2, 0.85), Align::Right, k), String::new())],
        )?;
        attribution.set_props(Props::at(MARGIN + 1792.0 - att_w, g.bar.y - cap_h - 14.0));
        Ok(Self {
            z,
            zoom,
            pin_group,
            track,
            pan: Pan::NONE,
            zoom_seen: None,
            globe: None,
            pics: HashMap::new(),
            shown: None,
            backdrop_seen: true,
            _pin: pin,
            _pin_ring: pin_ring,
            place,
            mode_img,
            mode_words,
            bar_img,
            bar_words,
            thumb,
            legend_img,
            legend_words,
            attribution,
            geo: g,
            mode_key: String::new(),
            bar_key: String::new(),
            words_key: String::new(),
            shown_globe: None,
            reduced: false,
        })
    }

    /// A picture's layer: made when the picture arrives, posed over the part of the world it covers. The map's pictures fade in over what they
    /// replace (which the model keeps for that long); a radar frame is hidden until it is the one shown.
    fn make_pic(&mut self, kit: &mut Kit, home: &Home, p: MapPic) -> std::io::Result<()> {
        let (w, h) = (p.picture.width, p.picture.height);
        // under the map, the map, over the map: a picture that is made later is over the ones made before it in its band
        let band = match p.role {
            Role::Backdrop => -1,
            Role::Map => 0,
            Role::Radar { .. } => 1,
        };
        let scene = kit.ui.scene.clone();
        let mut layer = scene.adopting(&self.zoom, || kit.ui.scene.image(w, h, self.z + band, HIDDEN))?;
        p.picture.paint(&mut layer);
        // written once and not touched again: this process need not keep its pages of it (the compositor reads the file)
        layer.release_resident();
        let (lon, lat) = (home.radar.lon, home.radar.lat);
        let (x, y, scale) = radar_pics::placement(p.world, (w, h), world_of(lon, lat), f64::from(Z_HOME));
        let at = Props { scale, ..Props::at(x, y) };
        let kind = match p.role {
            Role::Radar { t, .. } => home.radar.frames.iter().find(|f| f.t == t).map_or(Kind::Observed, |f| f.kind),
            _ => Kind::Observed,
        };
        match p.role {
            Role::Radar { .. } => layer.set(Props { opacity: 0.0, ..at }),
            Role::Backdrop => layer.set(if self.backdrop_seen { at } else { HIDDEN }),
            _ if self.reduced || !self.pics.values().any(|o| o.role == p.role) => layer.set(at),
            _ => {
                layer.set(Props { opacity: 0.0, ..at });
                layer.animate(at, FADE_MS, Ease::Linear);
            }
        }
        self.pics.insert(p.id, PicLayer { layer, role: p.role, kind, at });
        Ok(())
    }

    pub fn update(&mut self, kit: &mut Kit, home: &mut Home, _now: u64, reduced: bool) -> std::io::Result<()> {
        self.reduced = reduced;
        let globe = home.radar.globe();
        if self.shown_globe != Some(globe) {
            // the earth or the map: the pictures of the map are not seen with the globe (the model lets them go), and the pin is not on it
            self.shown_globe = Some(globe);
            self.bar_key.clear();
            self.mode_key.clear();
            self.words_key.clear();
            self.pin_group.set_pose(GroupPose { visible: !globe, ..GroupPose::IDENTITY });
            if globe {
                for p in self.pics.values() {
                    p.layer.set(HIDDEN);
                }
            }
        }
        self.zoom(kit, home);
        // the layers of the pictures the model let go are dropped before those of the new ones are made
        for id in home.radar.take_released() {
            self.pics.remove(&id);
            if self.shown == Some(id) {
                self.shown = None;
            }
        }
        for p in home.radar.take_pics() {
            self.make_pic(kit, home, p)?;
        }
        let seen = !globe && !home.radar.map_serves();
        if seen != self.backdrop_seen {
            self.backdrop_seen = seen;
            for p in self.pics.values().filter(|p| p.role == Role::Backdrop) {
                p.layer.set(if seen { p.at } else { HIDDEN });
            }
        }
        // the globe: one layer of the helper's picture, centred on the screen, replaced when a new one comes (it turns, or its textures arrive)
        if let Some(p) = home.radar.take_globe() {
            let mut layer = kit.ui.scene.image(p.width, p.height, self.z + 1, HIDDEN)?;
            p.paint(&mut layer);
            layer.release_resident();
            layer.set(Props::at(((MAP_W as f32 - p.width as f32) / 2.0).round(), ((MAP_H as f32 - p.height as f32) / 2.0).round()));
            self.globe = Some(layer);
        }
        if !globe {
            self.globe = None;
        }
        self.show(home);
        self.bar(kit, home);
        Ok(())
    }

    /// The zoom the page has taken up and how it moves the picture, for the tests.
    #[cfg(test)]
    pub fn zoom_taken(&self) -> Option<(f32, Pan)> {
        self.zoom_seen.map(|(t, _)| (f32::from_bits(t), self.pan))
    }

    /// How many pictures the page has layers for, for the tests.
    #[cfg(test)]
    pub fn picture_layers(&self) -> usize {
        self.pics.len()
    }

    /// The zoom the model is going to, taken up: the track is sent there on the focus's spring (at once under reduced motion, which has a crossfade for
    /// it), and when the zoom is about a place the picture is moved as it goes so that the place stays where it is. The view it comes to is told to
    /// the model, which asks for the pictures of it.
    fn zoom(&mut self, kit: &Kit, home: &mut Home) {
        let z = home.radar.zoom;
        let seen = (z.target.to_bits(), z.anchor.map(|(x, y)| (x.to_bits(), y.to_bits())));
        if self.zoom_seen != Some(seen) {
            self.zoom_seen = Some(seen);
            let s0 = self.track.value();
            self.pan = Pan::new(z.anchor, (MAP_W as f32 / 2.0, MAP_H as f32 / 2.0), s0, self.pan.at(s0));
            let _ = bind_pan(&[&self.zoom, &self.pin_group], &self.track, self.pan);
            let _ = self.track.settle(Some(z.scale()), None, kit.flow_spec().settle());
        }
        // the world at the middle of the screen when the track has come to the target: the group's space has the place there at scale 1, and a
        // point of it that is at the middle at scale `s` is `off / s` from the middle of the group's space
        let (s, off) = (f64::from(z.scale()), self.pan.at(z.scale()));
        let k = 256.0 * 2f64.powf(f64::from(Z_HOME));
        let c = world_of(home.radar.lon, home.radar.lat);
        home.radar.set_view(View { cx: c.0 - f64::from(off.0) / (s * k), cy: c.1 - f64::from(off.1) / (s * k), zoom: f64::from(z.target) });
    }

    /// The layer of the newest picture of the radar's frame (`t`, `rev`).
    fn frame_layer(&self, t: i64, rev: u32) -> Option<u64> {
        self.pics.iter().filter(|(_, p)| p.role == Role::Radar { t, rev }).map(|(id, _)| *id).max()
    }

    /// The frame shown: the layer before it fades out and this one in (or they swap at once under reduced motion). A frame whose picture has not
    /// come yet leaves the one before it on the screen.
    fn show(&mut self, home: &Home) {
        let globe = home.radar.globe();
        let want = home.radar.current().and_then(|f| self.frame_layer(f.t, f.rev)).or(self.shown.filter(|id| self.pics.contains_key(id))).filter(|_| !globe);
        if want == self.shown {
            return;
        }
        let pose = |base: Props, kind: Kind, on: bool| Props { opacity: if on { if kind == Kind::Forecast { FORECAST_ALPHA } else { 1.0 } } else { 0.0 }, ..base };
        if let Some(old) = self.shown.and_then(|id| self.pics.get(&id)) {
            let to = pose(old.at, old.kind, false);
            if self.reduced {
                old.layer.set(to);
            } else {
                old.layer.animate(to, FADE_MS, Ease::Linear);
            }
        }
        if let Some(new) = want.and_then(|id| self.pics.get(&id)) {
            let to = pose(new.at, new.kind, true);
            if self.reduced {
                new.layer.set(to);
            } else {
                new.layer.animate(to, FADE_MS, Ease::Linear);
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
        let g = self.geo;
        let k = g.k;
        let (t0, t1) = (r.now_s - BACK_S, r.now_s + AHEAD_S);
        let x_of = |t: i64| g.track_x + g.track_w * ((t - t0) as f32 / (t1 - t0).max(1) as f32).clamp(0.0, 1.0);
        // the track sits in the upper half of the bar, its words in the lower, and the thumb (30 px) is as tall as the ticks and the clock's mark are together
        let track_y = g.track_y;
        let globe = r.globe();
        // the bar's picture: its plate, the play and pause button (on the globe the pad that turns it), and the track: a tick for every frame of the
        // window (the ones the helper has strong), the radar's in the ink, the forecast's in the accent on a tint of it, and the clock
        let win: Vec<_> = if globe { Vec::new() } else { r.window() };
        let key = format!("{}|{}|{}|{}|{}", r.now_s / 60, win.iter().map(|f| format!("{}{}", f.t, u8::from(f.ready))).collect::<Vec<_>>().join(","), u8::from(light), u8::from(r.playing), u8::from(globe));
        if key != self.bar_key {
            self.bar_key = key;
            let playing = r.playing;
            let held: Vec<bool> = win.iter().map(|f| f.ready).collect();
            repaint(&mut self.bar_img, g.bar, |c, f| {
                paint_plate(kit, c, f, 30.0 * k, 0.9, None);
                let at = |x: f32, y: f32| (x + PAD, y + PAD);
                if !win.is_empty() {
                    let nowx = x_of(r.now_s);
                    // what is ahead of the clock is the model's: tinted, so that it reads apart from what the radar saw
                    let (tx, ty) = at(nowx, track_y - 12.0 * k);
                    c.rounded(Frame::new(tx, ty, g.track_x + g.track_w - nowx + 10.0 * k, 34.0 * k), 12.0 * k, faded(accent, 0.13));
                    let (lx, ly) = at(g.track_x, track_y + 5.0 * k);
                    c.rounded(Frame::new(lx, ly, g.track_w, 2.0), 1.0, faded(ink, 0.18));
                    for (f, on) in win.iter().zip(&held) {
                        let (x, y) = at(x_of(f.t) - 1.5, track_y);
                        let colour = if f.kind == Kind::Forecast { faded(accent, if *on { 0.95 } else { 0.35 }) } else { faded(ink, if *on { 0.8 } else { 0.28 }) };
                        c.rounded(Frame::new(x, y, 3.0, 12.0 * k), 1.5, colour);
                    }
                    let (x, y) = at(nowx - 1.0, track_y - 10.0 * k);
                    c.rounded(Frame::new(x, y, 2.0, 28.0 * k), 1.0, faded(ink, 0.9));
                }
                let (bx, by) = at(50.0 * k, g.bar.h / 2.0);
                if !globe {
                    gfx::arc(c, bx, by, 27.0 * k, 3.0, 0.0, std::f32::consts::TAU, accent);
                    if playing {
                        // pause: two bars
                        c.rounded(Frame::new(bx - 9.0 * k, by - 10.0 * k, 6.0 * k, 20.0 * k), 2.0, ink);
                        c.rounded(Frame::new(bx + 3.0 * k, by - 10.0 * k, 6.0 * k, 20.0 * k), 2.0, ink);
                    } else {
                        gfx::head(c, (bx + 11.0 * k, by), std::f32::consts::FRAC_PI_2, 20.0 * k, 22.0 * k, ink);
                    }
                }
            });
        }
        // the thumb
        match r.current().filter(|_| !globe) {
            Some(f) => self.thumb.set(Props::at(g.bar.x + x_of(f.t) - 4.0, g.bar.y + track_y - 9.0 * k)),
            None => self.thumb.set(HIDDEN),
        }
        // the legend is for the radar's colours: not on the globe
        self.legend_img.set(if globe { HIDDEN } else { Props::at(g.legend.x - PAD, g.legend.y - PAD) });
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
        // the forecast's chip is tinted with the accent, the radar's is plain
        let mode_key = format!("{mode}|{}", u8::from(light));
        if mode_key != self.mode_key {
            self.mode_key = mode_key;
            let forecast = mode == "FORECAST";
            repaint(&mut self.mode_img, Frame::new(0.0, 0.0, g.mode_w, g.chip_h), |c, f| {
                paint_plate(kit, c, f, g.chip_h / 2.0, 0.88, None);
                if forecast {
                    c.rounded(f, g.chip_h / 2.0, faded(accent, 0.20));
                }
            });
        }
        let status = if globe {
            if r.globe_ready() { String::new() } else { "Loading the globe\u{2026}".to_owned() }
        } else if !r.has_any_picture() {
            match r.status.as_str() {
                "unavailable" => format!("The radar is not available{}", if r.why.is_empty() { String::new() } else { format!(": {}", r.why) }),
                _ => "Loading the radar\u{2026}".to_owned(),
            }
        } else if r.status == "stale" {
            format!("Not current: the newest radar is {} old", r.newest_s.map_or("some".into(), |n| span_words(r.now_s - n)))
        } else {
            String::new()
        };
        let place = home.weather.pointer("/doc/display/place").and_then(serde_json::Value::as_str).unwrap_or("").to_owned();
        // what the data and the map ask to be said, whole when it fits what the legend leaves and a shorter way of saying it when it does not
        let words: [&str; 2] = if globe {
            ["Rain: NASA IMERG, about 6 h behind  \u{b7}  Cloud: satellite  \u{b7}  Earth: NASA", "Rain: NASA IMERG  \u{b7}  Cloud: satellite  \u{b7}  Earth: NASA"]
        } else {
            ["Radar: NEXRAD and HRRR via Iowa Environmental Mesonet  \u{b7}  Map: OpenStreetMap, OpenFreeMap", "Radar: NEXRAD, HRRR via IEM  \u{b7}  Map: OpenStreetMap, OpenFreeMap"]
        };
        let att_style = style_k(kit, TypeRole::Caption, None, ink2, Align::Right, k);
        let room = g.bar.w - g.legend.w - 24.0 * k;
        let attribution = match words.iter().find(|t| measured(kit, t, &att_style) <= room) {
            Some(t) => (*t).to_owned(),
            None => ui::components::text::ellipsise(&mut kit.ui.fonts, words[1], att_style.ty, room),
        };
        let key = format!("{mode}|{line}|{sub}|{detail}|{status}|{attribution}|{place}|{}", u8::from(globe));
        if key != self.words_key {
            self.words_key = key;
            self.place.set_many(kit, &[(0, &place, ink), (1, &status, ink)]);
            self.mode_words.set_many(kit, &[(0, mode, mode_colour), (1, detail, ink2)]);
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
    fn the_radars_colours_run_from_light_rain_to_heavy_in_both_looks() {
        for dark in [true, false] {
            let r = ramp(dark);
            assert!(r.windows(2).all(|w| w[0].0 < w[1].0 && w[0].2 <= w[1].2), "ordered, and the heavier the more opaque");
            assert_eq!(r[0].0, 0.0);
        }
    }

    #[test]
    fn a_place_is_where_web_mercator_puts_it() {
        let (x, y) = world_of(0.0, 0.0);
        assert!((x - 0.5).abs() < 1e-9 && (y - 0.5).abs() < 1e-9);
        // north is up: a place north of the equator has a smaller y
        assert!(world_of(0.0, 40.0).1 < 0.5 && world_of(90.0, 0.0).0 > 0.5);
    }

    #[test]
    fn the_chrome_keeps_clear_of_itself_and_of_the_screens_edge_at_either_text_size() {
        use crate::kit::testkit;
        for scale in [1.0, 2.0] {
            let (h, mut kit) = testkit::kit_in(Mode::Dark, scale);
            let g = Geo::new(&mut kit);
            assert!((1.0..=1.3).contains(&g.k), "{scale}: the chrome grows with the words, but to 1.3 times at most");
            // the chip under the view's bar, inside the margins
            assert!(g.mode().right() <= MARGIN + 1792.0 + 0.5 && g.mode().x >= MARGIN, "{scale}");
            // from the foot up: the bar inside the safe area, the legend above it, and the attribution over the bar clear of the legend's top
            assert!(g.bar.bottom() <= 1016.5, "{scale}: the bar is below the safe area");
            assert!(g.legend.bottom() + 8.0 <= g.bar.y, "{scale}: the legend meets the bar");
            // the words are in their plates: the chip's words are as tall as the chip allows, the time and what it is fit their boxes, the track starts after them
            let body = style_k(&kit, TypeRole::Body, Some(Weight::SemiBold), Rgba(255, 255, 255, 255), Align::Left, g.k);
            let cap = style_k(&kit, TypeRole::Caption, None, Rgba(255, 255, 255, 255), Align::Left, g.k);
            let label = style_k(&kit, TypeRole::Label, Some(Weight::SemiBold), Rgba(255, 255, 255, 255), Align::Left, g.k);
            assert!(ui::components::line_height(label.ty) <= g.chip_h, "{scale}: a chip is not as tall as its words");
            assert!(measured(&mut kit, "12:00 AM", &body) <= 250.0 * g.k, "{scale}: the time is wider than its box");
            for words in ["in 5 h 59 min", "1 h 59 min ago", "now"] {
                assert!(measured(&mut kit, words, &cap) <= 250.0 * g.k, "{scale}: {words}");
            }
            assert!(g.track_x >= (96.0 + 250.0) * g.k, "{scale}: the track starts under the time");
            std::mem::forget(h);
        }
    }

}
