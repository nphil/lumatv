//! The full weather view: OK on the weather card opens it over Home (`Layer::Weather`). Right and Left walk its pages (now, the radar, the details)
//! with a slide the way they go, Back closes it. It is drawn from the helper's `weather` document (the card's) and `weather.more` (the hours, the ten days, the details of
//! now, the alerts), both kept fresh in the background by the helper, so the page is made from what is already there, with nothing fetched when
//! it opens.
//!
//! Each page is one picture across the whole screen, not a window with cards in it: the sky of the day (a tint and the moving picture of
//! [`super::weather_art`], behind everything) with the reading large on the left, the next hours under it and the ten days beside it, as glass
//! on the sky; the radar is the map itself with its few controls floating over it; the details are a grid of tiles that each draw their number.
//!
//! The model is pure ([`WeatherView`]: the page, the keys); [`WeatherSurface`] draws it: one scrim and bar, and the page's sky, glass,
//! words and pictures, made when the page (or the reading, or the text size) changes and dropped with it.

use super::clock;
use super::model::Home;
use super::radar_surface::{RadarPage, control};
use super::surface::{HIDDEN, faded};
use crate::kit::pane::glass_of;
use super::weather_art::{self, Cond, Live, Stage};
use super::weather_sprites::Pal;
use super::weather_gfx as gfx;
use super::widgets::{Canvas, Pictures, Words};
use crate::kit::Kit;
use crate::kit::pane::PaneKind;
use crate::kit::text::Text;
use compositor_protocol::{Fill, Props, Rgba, Shape};
use lumatv_tokens::TypeRole;
use serde_json::Value;
use ui::components::{Align, Style, plate, style};
use ui::geometry::Frame;
use ui::group::{Group, GroupPose, PoseBinding, PoseProperties};
use ui::lucent::palette::Mode;
use ui::motion::{Motion, MotionBounds, Units};
use ui::scene::{GlassLayer, ImageLayer, ShapeLayer};
use ui::text::Weight;

/// The pages, left to right: Right goes to the next, Left to the one before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Now,
    Radar,
    Details,
}

pub const PAGES: [Page; 3] = [Page::Now, Page::Radar, Page::Details];

impl Page {
    pub fn name(self) -> &'static str {
        match self {
            Page::Now => "now",
            Page::Radar => "radar",
            Page::Details => "details",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Page::Now => "Now",
            Page::Radar => "Radar",
            Page::Details => "Details",
        }
    }
}

/// An accent colour (opaque) for words: as it is on the dark look, and darkened towards the ink on the light one, where the accents are too pale to read as words.
pub fn legible(c: Rgba, ink: Rgba, light: bool) -> Rgba {
    if !light {
        return c;
    }
    let m = |a: u8, b: u8| (f32::from(a) * 0.52 + f32::from(b) * 0.48).round() as u8;
    Rgba(m(c.0, ink.0), m(c.1, ink.1), m(c.2, ink.2), 255)
}

/// What a key did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Done {
    Nothing,
    Redraw,
    Close,
}

#[derive(Clone, Debug)]
pub struct WeatherView {
    pub page: Page,
}

impl Default for WeatherView {
    fn default() -> Self {
        Self { page: Page::Now }
    }
}

impl WeatherView {
    pub fn open(&mut self) {
        self.page = Page::Now;
    }

    /// A key of the view: `step` is -1 for Left, 1 for Right (the pages sit side by side), 0 for any other; `Done::Close` for Back. Up and Down are not
    /// the view's: the pages fit the screen, so there is nothing for them to scroll, and the radar page takes them for its zoom.
    pub fn key(&mut self, name: &str, step: i32) -> Done {
        if name == "back" {
            return Done::Close;
        }
        let i = PAGES.iter().position(|p| *p == self.page).unwrap_or(0) as i32;
        match usize::try_from(i + step).ok().and_then(|j| PAGES.get(j)).filter(|_| step != 0) {
            Some(p) => {
                self.page = *p;
                Done::Redraw
            }
            None => Done::Nothing,
        }
    }
}

// ---- the words of the weather

/// The icon name of a WMO code (the helper's `kind_for`).
pub fn kind_of(code: i64, is_day: bool) -> &'static str {
    match code {
        0 => {
            if is_day {
                "sun"
            } else {
                "moon"
            }
        }
        1 | 2 => {
            if is_day {
                "partly"
            } else {
                "partly-night"
            }
        }
        3 => "cloud",
        45 | 48 => "fog",
        c if c >= 95 => "storm",
        71..=77 | 85 | 86 => "snow",
        c if c >= 61 => "rain",
        c if c >= 51 => "drizzle",
        _ => "cloud",
    }
}

/// What falls in a WMO code, in a word, if anything.
pub fn wet_kind(code: i64) -> Option<&'static str> {
    match code {
        51..=57 => Some("Drizzle"),
        61..=67 | 80..=82 => Some("Rain"),
        71..=77 | 85 | 86 => Some("Snow"),
        c if c >= 95 => Some("Thunderstorms"),
        _ => None,
    }
}

/// `10 PM` (or `22` on the 24-hour clock) of a local `YYYY-MM-DDTHH:MM`.
pub fn hour_label(time: &str, twelve: bool) -> String {
    let h: u32 = time.get(11..13).and_then(|t| t.parse().ok()).unwrap_or(0);
    if twelve { format!("{} {}", if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" }) } else { format!("{h:02}") }
}

/// `7:41 AM` of a local `YYYY-MM-DDTHH:MM`.
pub fn clock_label(time: &str, twelve: bool) -> String {
    let h: u32 = time.get(11..13).and_then(|t| t.parse().ok()).unwrap_or(0);
    let m: u32 = time.get(14..16).and_then(|t| t.parse().ok()).unwrap_or(0);
    if twelve { format!("{}:{m:02} {}", if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" }) } else { format!("{h:02}:{m:02}") }
}

pub fn compass(deg: f64) -> &'static str {
    const NAMES: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    NAMES[(((deg % 360.0 + 360.0) % 360.0 + 22.5) / 45.0) as usize % 8]
}

pub fn uv_word(uv: f64) -> &'static str {
    match uv {
        x if x < 3.0 => "Low",
        x if x < 6.0 => "Moderate",
        x if x < 8.0 => "High",
        x if x < 11.0 => "Very high",
        _ => "Extreme",
    }
}

/// Where the knob sits on the UV scale (a bar `w` wide from `x`, 0 to 11+): nowhere when the reading is unknown, the tile then a dash.
fn uv_knob(uv: Option<f32>, x: f32, w: f32) -> Option<f32> {
    uv.map(|v| x + w * (v / 11.0).clamp(0.0, 1.0))
}

/// `8 AM`, or `8:30 AM` when it is not on the hour (`08:00` and `08:30` on the 24-hour clock), of a local `YYYY-MM-DDTHH:MM`.
pub fn when_label(time: &str, twelve: bool) -> String {
    if time.get(14..16).and_then(|m| m.parse::<u32>().ok()).unwrap_or(0) == 0 { hour_label(time, twelve) } else { clock_label(time, twelve) }
}

/// What falls over the hours ahead and when it starts or stops: `None` when nothing does (and no chance worth a word).
pub fn precip_line(hours: &[Value], now_code: i64, twelve: bool) -> Option<String> {
    let code = |h: &Value| h.get("code").and_then(Value::as_i64).unwrap_or(0);
    let chance = |h: &Value| h.get("rainPct").and_then(Value::as_f64).unwrap_or(0.0);
    let wet = |h: &Value| wet_kind(code(h)).is_some() || chance(h) >= 60.0;
    let kind = |h: &Value| wet_kind(code(h)).unwrap_or("Rain");
    let time = |h: &Value| hour_label(h.get("time").and_then(Value::as_str).unwrap_or(""), twelve);
    let upcoming = || hours.iter().skip(1).take(24);
    if let Some(now) = wet_kind(now_code) {
        return Some(match upcoming().find(|h| !wet(h)) {
            Some(h) => format!("{now} until {}", time(h)),
            None => format!("{now} for the next day"),
        });
    }
    match upcoming().find(|h| wet(h)) {
        Some(h) => Some(format!("{} from {}", kind(h), time(h))),
        None => {
            let peak = hours.iter().take(24).map(chance).fold(0.0, f64::max);
            (peak >= 30.0).then(|| "A chance of rain today".to_owned())
        }
    }
}

/// One sentence for the hours ahead: when the rain starts or stops, or that there is none.
pub fn sentence(hours: &[Value], now_code: i64, twelve: bool) -> String {
    precip_line(hours, now_code, twelve).unwrap_or_else(|| "No rain for the next day".to_owned())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmartTone {
    Info,
    Warning,
}

/// The one line that says what is next, in full and in a shorter way for a card with less room: a weather alert in force, else what falls and
/// when, else the sun's next turn, else that it is dry; with only the reading and no hours, how humid and windy it is.
pub fn smart_line(doc: &Value, more: &Value, twelve: bool) -> (String, String, SmartTone) {
    if let Some(a) = more.get("alerts").and_then(Value::as_array).and_then(|a| a.first()) {
        let event = a.get("event").and_then(Value::as_str).unwrap_or("Alert");
        let until = a.get("ends").and_then(Value::as_str).filter(|e| e.len() >= 16).map(|e| format!(" until {}", when_label(e, twelve))).unwrap_or_default();
        return (format!("{event}{until}"), event.to_owned(), SmartTone::Warning);
    }
    let hours: Vec<Value> = more.get("hourly").and_then(Value::as_array).cloned().unwrap_or_default();
    if hours.is_empty() {
        let p = |k: &str| doc.pointer(&format!("/current/{k}")).and_then(Value::as_f64);
        return match (p("humidity"), p("windMph")) {
            (Some(h), Some(w)) => (format!("{}% humidity  \u{b7}  {} mph wind", h.round() as i64, w.round() as i64), format!("{}%  \u{b7}  {} mph", h.round() as i64, w.round() as i64), SmartTone::Info),
            _ => (String::new(), String::new(), SmartTone::Info),
        };
    }
    let now_code = doc.pointer("/current/code").and_then(Value::as_i64).unwrap_or(0);
    if let Some(line) = precip_line(&hours, now_code, twelve) {
        let short = if line.starts_with("A chance") { "Rain possible".to_owned() } else { line.replace("Thunderstorms", "Storms").replace(" until ", " till ") };
        return (line, short, SmartTone::Info);
    }
    // dry: the sun's next turn, when it is soon
    let now = doc.pointer("/current/time").and_then(Value::as_str).unwrap_or("");
    let day = |i: usize, k: &str| more.pointer(&format!("/daily/{i}/{k}")).and_then(Value::as_str).filter(|t| t.len() >= 16);
    if !now.is_empty() {
        let is_day = doc.pointer("/current/isDay").and_then(Value::as_bool).unwrap_or(true);
        if is_day && let Some(set) = day(0, "sunset").filter(|s| *s > now) {
            let t = clock_label(set, twelve);
            return (format!("Sunset at {t}"), format!("Sunset {t}"), SmartTone::Info);
        }
        if !is_day && let Some(rise) = [day(0, "sunrise"), day(1, "sunrise")].into_iter().flatten().find(|r| *r > now) {
            let t = clock_label(rise, twelve);
            return (format!("Sunrise at {t}"), format!("Sunrise {t}"), SmartTone::Info);
        }
    }
    ("No rain for the next day".to_owned(), "No rain today".to_owned(), SmartTone::Info)
}

// ---- the days

/// A day of the list of the week.
#[derive(Clone, Debug, PartialEq)]
pub struct DayRow {
    pub name: String,
    pub kind: String,
    pub hi: f32,
    pub lo: f32,
    /// The chance of rain, percent.
    pub rain: Option<f32>,
}

/// `Fri` of a local `YYYY-MM-DD`.
pub fn weekday(date: &str) -> &'static str {
    let num = |r: std::ops::Range<usize>| date.get(r).and_then(|t| t.parse::<i32>().ok());
    let (Some(y), Some(m), Some(d)) = (num(0..4), num(5..7), num(8..10)) else { return "" };
    if !(1..=12).contains(&m) {
        return "";
    }
    // Sakamoto's: 0 is a Sunday
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y - 1 } else { y };
    let w = (y + y / 4 - y / 100 + y / 400 + T[(m - 1) as usize] + d).rem_euclid(7);
    ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][w as usize]
}

/// The days to list: the helper's ten (`weather.more`) when it has them, else the card's own (up to seven).
pub fn week_days(doc: &Value, more: &Value) -> Vec<DayRow> {
    let daily: Vec<Value> = more.get("daily").and_then(Value::as_array).cloned().unwrap_or_default();
    let rain = |i: usize| daily.get(i).and_then(|m| m.get("rainPct")).and_then(Value::as_f64).map(|p| p as f32);
    let ten: Vec<DayRow> = daily
        .iter()
        .enumerate()
        .filter_map(|(i, d)| {
            let (hi, lo) = (d.get("hi")?.as_f64()?, d.get("lo")?.as_f64()?);
            let name = if i == 0 { "Today".to_owned() } else { weekday(d.get("date")?.as_str()?).to_owned() };
            Some(DayRow { name, kind: kind_of(d.get("code").and_then(Value::as_i64).unwrap_or(3), true).to_owned(), hi: hi as f32, lo: lo as f32, rain: rain(i) })
        })
        .collect();
    if !ten.is_empty() {
        return ten;
    }
    let deg = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).and_then(|t| t.trim_end_matches('\u{b0}').parse::<f32>().ok());
    doc.pointer("/display/days")
        .and_then(Value::as_array)
        .map(|days| {
            days.iter()
                .enumerate()
                .filter_map(|(i, d)| Some(DayRow { name: d.get("day")?.as_str()?.to_owned(), kind: d.get("kind")?.as_str()?.to_owned(), hi: deg(d, "hi")?, lo: deg(d, "lo")?, rain: rain(i) }))
                .collect()
        })
        .unwrap_or_default()
}

/// Minutes into the day of a local `YYYY-MM-DDTHH:MM`.
fn minutes(time: &str) -> Option<i32> {
    Some(time.get(11..13)?.parse::<i32>().ok()? * 60 + time.get(14..16)?.parse::<i32>().ok()?)
}

struct Data<'a> {
    doc: &'a Value,
    more: &'a Value,
    twelve: bool,
    fahrenheit: bool,
}

impl<'a> Data<'a> {
    fn new(home: &'a Home) -> Self {
        let doc = home.weather.get("doc").unwrap_or(&Value::Null);
        let more = home.weather_more.get("doc").unwrap_or(&Value::Null);
        Self { doc, more, twelve: home.theme.pointer("/clock/twelveHour").and_then(Value::as_bool) != Some(false), fahrenheit: doc.get("units").and_then(Value::as_str) != Some("celsius") }
    }

    fn display(&self, k: &str) -> String {
        self.doc.pointer(&format!("/display/{k}")).and_then(Value::as_str).unwrap_or("").to_owned()
    }

    fn num(&self, path: &str) -> Option<f64> {
        self.doc.pointer(path).and_then(Value::as_f64)
    }

    fn more_num(&self, path: &str) -> Option<f64> {
        self.more.pointer(path).and_then(Value::as_f64)
    }

    fn hours(&self) -> Vec<Value> {
        self.more.get("hourly").and_then(Value::as_array).cloned().unwrap_or_default()
    }

    fn deg(&self, v: f64) -> String {
        format!("{}\u{b0}", v.round() as i64)
    }

    /// The four readings under the hero, each as the page shows it: an en dash for one the feed does not carry, never a blank.
    fn facts(&self) -> [(&'static str, String); 4] {
        let wind = self.num("/current/windMph").map(|v| format!("{} mph{}", v.round() as i64, self.more_num("/now/windDeg").map(|dg| format!(" {}", compass(dg))).unwrap_or_default()));
        let hum = self.num("/current/humidity").map(|v| format!("{}%", v.round() as i64));
        let uv = self.more_num("/now/uvIndex").map(|u| format!("{} {}", u.round() as i64, uv_word(u)));
        let rain = self.more.pointer("/daily/0/rainPct").and_then(Value::as_f64).map(|p| format!("{}%", p.round() as i64));
        [("WIND", wind), ("HUMIDITY", hum), ("UV INDEX", uv), ("RAIN TODAY", rain)].map(|(label, v)| (label, v.unwrap_or_else(|| NONE.to_owned())))
    }

    /// A key that tells the page when to be made again.
    fn key(&self) -> String {
        format!("{}|{}|{}", self.doc.get("fetchedAt").map(Value::to_string).unwrap_or_default(), self.more.get("fetchedAt").map(Value::to_string).unwrap_or_default(), self.twelve)
    }
}

// ---- the surface
type Line = (Frame, Style, String);

struct Card {
    _glass: GlassLayer,
    _words: Words,
}

struct Made {
    cards: Vec<Card>,
    words: Vec<Words>,
    pictures: Vec<ImageLayer>,
    sky: Option<ShapeLayer>,
    art: Option<Live>,
    key: String,
    page: Page,
    scale: f32,
    mode: Mode,
}

/// How far a page slides: the screen's width, so the one that leaves is gone and the one that comes starts out of sight.
const SLIDE_PX: f32 = 1920.0;
/// How long after a change of page the page that left is let go (the spring lands within about 270 ms).
const SLIDE_MAX_MS: u64 = 600;

/// A page and what it is made of, in a group of its own that a track of the compositor slides: the track's value is the group's x, 0 when the page is
/// the one shown, and so the keys cost the app one request and every frame of the slide costs it nothing.
struct Slot {
    page: Page,
    made: Option<Made>,
    radar: Option<RadarPage>,
    track: Motion,
    /// Last, so that what hangs under it goes first.
    group: Group,
}

impl Slot {
    /// A page whose group is `x` pixels off its place.
    fn new(kit: &Kit, page: Page, x: f32) -> std::io::Result<Self> {
        let other = |e: ui::error::UiError| std::io::Error::other(format!("{e}"));
        let group = kit.ui.scene.group(None, GroupPose::IDENTITY).map_err(other)?;
        let track = kit.ui.scene.motion(x, MotionBounds::range(-SLIDE_PX, SLIDE_PX, Units::Pixels)).map_err(other)?;
        group.bind(&track, PoseBinding { values: [0.0, 1.0], from: GroupPose::IDENTITY, to: GroupPose { x: 1.0, ..GroupPose::IDENTITY }, properties: PoseProperties::TRANSLATION }).map_err(other)?;
        Ok(Self { page, made: None, radar: None, track, group })
    }
}

pub struct WeatherSurface {
    _scrim: ShapeLayer,
    _bar: GlassLayer,
    /// Over the bar's glass on the radar page, where the map is under it and the glass (which samples the wallpaper's blur) would not do.
    radar_bar: ShapeLayer,
    title: Text,
    clock: Text,
    dots: Vec<ShapeLayer>,
    /// The page shown, and the pages that are sliding out of the way of it.
    cur: Option<Slot>,
    leaving: Vec<Slot>,
    /// When `leaving` is let go.
    gone_at: u64,
    z: i32,
    /// The bar's words are for this page and clock.
    bar_key: String,
}

/// What stands for a reading that is not known.
const NONE: &str = "\u{2013}";

const MARGIN: f32 = 64.0;
const TOP: f32 = 140.0;
const BOTTOM: f32 = 1016.0;
const WIDTH: f32 = 1792.0;

/// A style of `px` pixels in the Label role's face.
/// The first of `words` (the whole of it first, then shorter ways) that fits `room`; the last one cut with an ellipsis when none does.
fn fit(kit: &mut Kit, style: &Style, words: &[String], room: f32) -> String {
    for w in words {
        if kit.ui.fonts.measure(w, style.ty).width <= room {
            return w.clone();
        }
    }
    words.last().map(|w| ui::components::text::ellipsise(&mut kit.ui.fonts, w, style.ty, room)).unwrap_or_default()
}

fn sty(kit: &Kit, px: f32, weight: Weight, colour: Rgba, align: Align) -> Style {
    let mut ty = kit.ty(TypeRole::Label);
    ty.px = px.round().max(8.0) as u32;
    ty.weight = weight;
    style(ty, colour, align)
}

/// A picture layer at `rect`, painted by `paint`.
fn picture(kit: &mut Kit, rect: Frame, z: i32, paint: impl FnOnce(&mut Canvas)) -> std::io::Result<ImageLayer> {
    let mut img = kit.ui.scene.image(rect.w as u32, rect.h as u32, z, HIDDEN)?;
    img.clear();
    {
        let (w, h) = (img.width(), img.height());
        let mut canvas = Canvas { px: img.pixels_mut(), w, h };
        paint(&mut canvas);
    }
    img.damage(None);
    img.set(Props::at(rect.x, rect.y));
    Ok(img)
}

/// The rows of the hero's words: the place, the reading, the condition, how it feels, what is next, and the facts of now, flowed from the top at the
/// text size's reduction `s`; the facts go when the rest leaves no room for them, and then the reading shrinks.
struct Hero {
    temp_px: f32,
    facts: bool,
}

fn hero_flow(s: f32, avail: f32) -> Hero {
    let height = |tp: f32, facts: bool| -> f32 {
        let base = 40.0 * s * 1.3 - 6.0 + tp * 0.92 + 6.0 + 48.0 * s * 1.3 + 28.0 * s * 1.35 + 10.0 + 36.0 * s * 1.35;
        base + if facts { 26.0 + 24.0 * s * 1.3 + 36.0 * s * 1.3 + 4.0 } else { 0.0 }
    };
    let mut tp = (236.0 * s).round();
    if height(tp, true) <= avail {
        return Hero { temp_px: tp, facts: true };
    }
    while height(tp, false) > avail && tp > 96.0 {
        tp -= 8.0;
    }
    Hero { temp_px: tp, facts: false }
}

impl WeatherSurface {
    /// The words of the page that are wider than their boxes.
    #[cfg(test)]
    pub fn cut_words(&self, kit: &mut Kit) -> Vec<String> {
        let Some(made) = self.cur.as_ref().and_then(|s| s.made.as_ref()) else { return Vec::new() };
        let mut cut: Vec<String> = made.words.iter().flat_map(|w| w.cut(kit)).collect();
        cut.extend(made.cards.iter().flat_map(|c| c._words.cut(kit)));
        cut
    }

    pub fn new(kit: &mut Kit, home: &Home, z: i32) -> std::io::Result<Self> {
        // The theme's scrim hides the page by 97 % only; with a page of open sky and a map, Home's words behind it would show as faint ghosts.
        let Rgba(r, g, b, a) = kit.tokens.extra(kit.mode).scrim_hide;
        let straight = |c: u8| (u16::from(c) * 255 / u16::from(a.max(1))).min(255) as u8;
        let scrim = kit.ui.scene.shape(1920, 1080, z - 1, plate::solid(Rgba(straight(r), straight(g), straight(b), 255), 0.0), Props::at(0.0, 0.0));
        let bar_f = Frame::new(MARGIN, 56.0, WIDTH, 56.0);
        // the bar is above the page, whatever the page draws (the radar's map is over the whole screen)
        let bar = kit.ui.scene.glass(WIDTH as u32, 56, z + 20, glass_of(kit, PaneKind::Pill, 56.0, false, false), Props::at(bar_f.x, bar_f.y));
        let radar_bar = kit.ui.scene.shape(WIDTH as u32, 56, z + 21, control(kit, 28.0, 0.9), HIDDEN);
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let title_f = Frame::new(bar_f.x + kit.space(5) + 12.0, bar_f.y, 900.0, 56.0);
        let clock_f = Frame::new(bar_f.x + WIDTH - 400.0 - kit.space(6), bar_f.y, 400.0, 56.0);
        // the bar is 56 px high whatever the text size: its words grow a little with it, not as the page's do
        let bs = kit.text_scale.clamp(1.0, 1.35);
        let title = Text::new(kit, title_f, z + 22, sty(kit, 28.0 * bs, Weight::SemiBold, ink, Align::Left), "Weather")?;
        let clock = Text::new(kit, clock_f, z + 22, sty(kit, 26.0 * bs, Weight::Medium, ink2, Align::Right), "")?;
        let mut me = Self { _scrim: scrim, _bar: bar, radar_bar, title, clock, dots: Vec::new(), cur: None, leaving: Vec::new(), gone_at: 0, z, bar_key: String::new() };
        let _ = home;
        for _ in PAGES {
            me.dots.push(kit.ui.scene.shape(14, 14, z + 23, plate::solid(faded(ink2, 0.5), 7.0), HIDDEN));
        }
        Ok(me)
    }

    /// The page shown, how far (px) its group is from its place at `at_ns` by the track's own estimate, and the pages still sliding out.
    #[cfg(test)]
    pub fn paging(&self, at_ns: u64) -> Option<(Page, f32, Vec<Page>)> {
        let cur = self.cur.as_ref()?;
        Some((cur.page, cur.track.value_at(at_ns), self.leaving.iter().map(|s| s.page).collect()))
    }

    /// The zoom the map page has taken up, for the tests.
    #[cfg(test)]
    pub fn zoom_taken(&self) -> Option<(f32, super::radar_zoom::Pan)> {
        self.cur.as_ref()?.radar.as_ref()?.zoom_taken()
    }

    /// Whether the picture of the page moves: the same rule as the card's.
    pub fn update(&mut self, kit: &mut Kit, home: &mut Home, pictures: &mut Pictures, now: u64, run_art: bool) -> std::io::Result<()> {
        let page = home.wview.page;
        let scene = kit.ui.scene.clone();
        let settle = kit.flow_spec().settle();
        // another page: the one shown goes off the way the keys went and the new one comes in from there, on the focus's spring
        if self.cur.as_ref().is_some_and(|s| s.page != page) {
            let old = self.cur.take().expect("checked above");
            let dir = if PAGES.iter().position(|p| *p == page) > PAGES.iter().position(|p| *p == old.page) { 1.0 } else { -1.0 };
            let _ = old.track.settle(Some(-dir * SLIDE_PX), None, settle);
            self.leaving.push(old);
            let slot = Slot::new(kit, page, dir * SLIDE_PX)?;
            let _ = slot.track.settle(Some(0.0), None, settle);
            self.cur = Some(slot);
            self.gone_at = now + SLIDE_MAX_MS;
        }
        if self.cur.is_none() {
            self.cur = Some(Slot::new(kit, page, 0.0)?);
        }
        if now >= self.gone_at {
            self.leaving.clear();
        }
        let mut slot = self.cur.take().expect("made above");
        // the radar page has layers of its own (a map, a layer for every frame of the loop), made when it is the page and dropped when it leaves
        if page == Page::Radar {
            let reduced = kit.ui.look.reduced_motion;
            let z = self.z + 1;
            scene.adopting(&slot.group, || -> std::io::Result<()> {
                if slot.radar.is_none() {
                    slot.radar = Some(RadarPage::new(kit, z, &slot.group)?);
                }
                if let Some(r) = &mut slot.radar {
                    r.update(kit, home, now, reduced)?;
                }
                Ok(())
            })?;
        }
        let home: &Home = home;
        let data = Data::new(home);
        // the bar: where this is, and the place and the time
        let t = clock::civil(home.now);
        let (time, ampm) = t.time_words(data.twelve);
        let bar_key = format!("{}|{time}|{}", page.name(), data.display("place"));
        if bar_key != self.bar_key {
            self.bar_key = bar_key;
            let place = data.display("place");
            let title = if place.is_empty() { format!("Weather \u{b7} {}", page.title()) } else { format!("{place} \u{b7} {}", page.title()) };
            self.title.set(kit, &title);
            self.clock.set(kit, format!("{time} {ampm}").trim());
        }
        let accent = kit.ui.look.accent();
        let ink2 = kit.ui.look.ink2();
        // the page dots, a row in the bar: the one shown in the accent, the rest in the secondary ink
        let bar_right = MARGIN + WIDTH - 28.0;
        for (i, d) in self.dots.iter().enumerate() {
            let x = bar_right - (PAGES.len() - i) as f32 * 24.0 - 330.0;
            let on = PAGES[i] == page;
            d.reshape(plate::solid(if on { accent } else { faded(ink2, 0.5) }, 7.0));
            d.set(Props::at(x, 56.0 + 21.0));
        }
        self.radar_bar.set(if page == Page::Radar { Props::at(MARGIN, 56.0) } else { HIDDEN });
        if page != Page::Radar {
            let key = data.key();
            let need = slot.made.as_ref().is_none_or(|m| m.key != key || m.page != page || m.scale != kit.text_scale || m.mode != kit.mode);
            if need {
                slot.made = None;
                let made = scene.adopting(&slot.group, || self.build(kit, &data, page, pictures, key))?;
                slot.made = Some(made);
            }
            if let Some(m) = &mut slot.made
                && let Some(a) = m.art.as_mut()
            {
                if run_art {
                    a.step(now);
                } else if a.running() {
                    a.pause(now);
                }
            }
        }
        self.cur = Some(slot);
        Ok(())
    }

    /// When the page's picture is next due while it moves, and when the page that slid away is let go.
    pub fn due_at(&self, now: u64, run_art: bool) -> Option<u64> {
        let art = self.cur.as_ref().and_then(|s| s.made.as_ref()).and_then(|m| m.art.as_ref()).filter(|a| a.running() && run_art).map(|_| now - now % weather_art::TICK_MS + weather_art::TICK_MS);
        let slide = (!self.leaving.is_empty()).then_some(self.gone_at);
        [art, slide].into_iter().flatten().min()
    }

    fn card(&self, kit: &mut Kit, rect: Frame, z: i32, lines: Vec<Line>) -> std::io::Result<Card> {
        let glass = kit.ui.scene.glass(rect.w as u32, rect.h as u32, z, glass_of(kit, PaneKind::Card, rect.h, false, false), Props::at(rect.x, rect.y));
        let mut words = Words::new(kit, Frame::new(0.0, 0.0, rect.w, rect.h), z + 1, lines)?;
        words.set_props(Props::at(rect.x, rect.y));
        Ok(Card { _glass: glass, _words: words })
    }

    fn build(&mut self, kit: &mut Kit, d: &Data, page: Page, pictures: &mut Pictures, key: String) -> std::io::Result<Made> {
        let mut made = Made { cards: Vec::new(), words: Vec::new(), pictures: Vec::new(), sky: None, art: None, key, page, scale: kit.text_scale, mode: kit.mode };
        match page {
            Page::Now => self.page_now(kit, d, pictures, &mut made)?,
            Page::Details => self.page_details(kit, d, &mut made)?,
            Page::Radar => {}
        }
        Ok(made)
    }

    /// The sky behind a page: its tint from the top of the screen, and (on the first page) the moving picture of the condition across the top of it,
    /// both under the bar and the glass.
    fn sky(&self, kit: &mut Kit, d: &Data, made: &mut Made, art: bool) -> std::io::Result<()> {
        let Some(cond) = Cond::from_doc(d.doc) else { return Ok(()) };
        let light = kit.mode == Mode::Light;
        let (rgb, a) = cond.wash(&Pal::of(&kit.ui.look.theme.roles, light), light);
        let a = (a * 1.2).min(0.6);
        let top = Rgba((rgb[0] * a * 255.0).round() as u8, (rgb[1] * a * 255.0).round() as u8, (rgb[2] * a * 255.0).round() as u8, (a * 255.0).round() as u8);
        let shape = Shape { corner_radius: 0.0, fill: Fill::Linear { from: top, to: Rgba(0, 0, 0, 0), angle_deg: 90.0 }, border_width: 0.0, border: Rgba(0, 0, 0, 0) };
        made.sky = Some(kit.ui.scene.shape(1920, 860, self.z, shape, Props::at(0.0, 0.0)));
        if art {
            // what moves is the open sky between the reading and the ten days (and under both): a box of a bit over a thousand pixels across, fading
            // out at its edges, so that the compositor's frames are no more than that (the tint behind it is the whole screen's and still)
            let (ox, oy) = (500.0, 96.0);
            let st = Stage { w: 1160.0, h: 620.0, hero: (1060.0 - ox, 372.0 - oy), size: 470.0, ground: 600.0, fade: [150.0, 150.0, 50.0, 150.0], shy: 0.2, card: None };
            made.art = Live::new(kit, cond, st, light, (ox, oy), self.z).ok();
        }
        Ok(())
    }

    // ---- now: the reading over the sky, the next hours and the ten days
    fn page_now(&self, kit: &mut Kit, d: &Data, pictures: &mut Pictures, made: &mut Made) -> std::io::Result<()> {
        let z = self.z + 4;
        let (ink, ink2, accent, warn) = (kit.ui.look.ink(), kit.ui.look.ink2(), kit.ui.look.accent(), kit.ui.look.warning());
        let light = kit.mode == Mode::Light;
        let s = kit.text_scale.clamp(1.0, 1.35);
        self.sky(kit, d, made, true)?;
        let hours = d.hours();
        let now_code = d.num("/current/code").map(|c| c as i64).unwrap_or(0);
        let _ = now_code;

        // the reading
        let flow = hero_flow(s, 550.0);
        let (pp, tp, cp, fp, sp) = (40.0 * s, flow.temp_px, 48.0 * s, 28.0 * s, 36.0 * s);
        let mut lines: Vec<Line> = Vec::new();
        let mut y = 0.0;
        lines.push((Frame::new(8.0, y, 760.0, pp * 1.3), sty(kit, pp, Weight::SemiBold, ink, Align::Left), d.display("place")));
        y += pp * 1.3 - 6.0;
        lines.push((Frame::new(4.0, y, 760.0, tp), sty(kit, tp, Weight::SemiBold, ink, Align::Left), d.display("temp")));
        y += tp * 0.92 + 6.0;
        lines.push((Frame::new(8.0, y, 760.0, cp * 1.3), sty(kit, cp, Weight::Medium, ink, Align::Left), d.display("cond")));
        y += cp * 1.3;
        let hilo = d.doc.pointer("/display/days/0").map(|d0| format!("H {}  L {}", d0.get("hi").and_then(Value::as_str).unwrap_or(""), d0.get("lo").and_then(Value::as_str).unwrap_or(""))).unwrap_or_default();
        let feels = d.num("/current/feelsLike").map(|f| format!("Feels like {}", d.deg(f))).unwrap_or_default();
        let sub = [feels, hilo].into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join("  \u{b7}  ");
        lines.push((Frame::new(8.0, y, 760.0, fp * 1.35), sty(kit, fp, Weight::Medium, ink2, Align::Left), sub));
        y += fp * 1.35 + 10.0;
        let (smart, _, tone) = smart_line(d.doc, d.more, d.twelve);
        lines.push((Frame::new(8.0, y, 780.0, sp * 1.35), sty(kit, sp, Weight::SemiBold, legible(if tone == SmartTone::Warning { warn } else { accent }, ink, light), Align::Left), smart));
        y += sp * 1.35 + 26.0;
        if flow.facts {
            let (lp, vp) = (24.0 * s, 34.0 * s);
            for (i, (label, value)) in d.facts().into_iter().enumerate() {
                let x = 8.0 + i as f32 * 222.0;
                lines.push((Frame::new(x, y, 216.0, lp * 1.3), sty(kit, lp, Weight::SemiBold, ink2, Align::Left), label.to_owned()));
                lines.push((Frame::new(x, y + lp * 1.3 + 2.0, 216.0, vp * 1.3), sty(kit, vp, Weight::Medium, ink, Align::Left), value));
            }
        }
        let mut hero = Words::new(kit, Frame::new(0.0, 0.0, 920.0, 560.0), z + 2, lines)?;
        hero.set_props(Props::at(MARGIN, TOP));
        made.words.push(hero);

        // the next hours: a glass band under the reading
        if !hours.is_empty() {
            let band = Frame::new(MARGIN, 704.0, 1240.0, BOTTOM - 704.0);
            let n = ((band.w - 48.0) / (99.0 * s.max(1.0))).floor().clamp(4.0, 12.0) as usize;
            let col_w = ((band.w - 48.0) / n as f32).floor();
            let (tp, vp, pp) = (24.0 * s, 34.0 * s, 24.0 * s);
            let ic = (52.0 * s.min(1.15)).round();
            let mut y = 16.0;
            let time_y = y;
            y += tp * 1.3 + 2.0;
            let icon_y = y;
            y += ic + 2.0;
            let temp_y = y;
            y += vp * 1.25 + 6.0;
            let bar_top = y;
            let bar_h = (band.h - 16.0 - pp * 1.3 - 4.0 - bar_top).max(0.0);
            let pct_y = bar_top + bar_h + 4.0;
            let mut lines: Vec<Line> = Vec::new();
            let shown: Vec<&Value> = hours.iter().take(n).collect();
            for (i, h) in shown.iter().enumerate() {
                let x = 24.0 + i as f32 * col_w;
                let time = if i == 0 { "Now".to_owned() } else { hour_label(h.get("time").and_then(Value::as_str).unwrap_or(""), d.twelve) };
                let temp = if i == 0 { d.display("temp") } else { d.deg(h.get("temp").and_then(Value::as_f64).unwrap_or(0.0)) };
                lines.push((Frame::new(x, time_y, col_w, tp * 1.3), sty(kit, tp, Weight::Medium, ink2, Align::Centre), time));
                lines.push((Frame::new(x, temp_y, col_w, vp * 1.25), sty(kit, vp, Weight::SemiBold, ink, Align::Centre), temp));
                let pct = h.get("rainPct").and_then(Value::as_f64).filter(|p| *p >= 10.0).map(|p| format!("{}%", p.round() as i64)).unwrap_or_default();
                lines.push((Frame::new(x, pct_y, col_w, pp * 1.3), sty(kit, pp, Weight::Medium, ink2, Align::Centre), pct));
            }
            made.cards.push(self.card(kit, band, z, lines)?);
            let now_kind = d.num("/current/code").map(|c| c as i64).zip(d.doc.pointer("/current/isDay").and_then(Value::as_bool)).map(|(c, day)| kind_of(c, day));
            // rain to show: its bars; none: the curve of the temperature, so that the room is not empty and the day's turn is seen
            let any_rain = shown.iter().any(|h| h.get("rainPct").and_then(Value::as_f64).is_some_and(|p| p >= 10.0));
            let temps: Vec<f32> = shown.iter().enumerate().map(|(i, h)| if i == 0 { d.num("/current/temp").unwrap_or(0.0) as f32 } else { h.get("temp").and_then(Value::as_f64).unwrap_or(0.0) as f32 }).collect();
            let img = picture(kit, band, z + 3, |canvas| {
                for (i, h) in shown.iter().enumerate() {
                    let cx = 24.0 + (i as f32 + 0.5) * col_w;
                    let code = h.get("code").and_then(Value::as_i64).unwrap_or(0);
                    let day = h.get("isDay").and_then(Value::as_bool).unwrap_or(true);
                    let kind = if i == 0 { now_kind } else { None }.unwrap_or_else(|| kind_of(code, day));
                    if let Some(p) = pictures.get_look(&format!("wxbig-{kind}"), light) {
                        canvas.picture(p, (cx - ic / 2.0).round() as i32, icon_y.round() as i32, ic as u32, ic as u32);
                    }
                    if any_rain && bar_h >= 20.0 {
                        let bw = 24.0 * s.min(1.15);
                        let prob = (h.get("rainPct").and_then(Value::as_f64).unwrap_or(0.0) / 100.0).clamp(0.0, 1.0) as f32;
                        canvas.rounded(Frame::new(cx - bw / 2.0, bar_top, bw, bar_h), bw / 2.0, faded(ink, 0.08));
                        if prob >= 0.03 {
                            let fh = (bar_h * prob).max(bw * 0.6);
                            canvas.rounded(Frame::new(cx - bw / 2.0, bar_top + bar_h - fh, bw, fh), bw / 2.0, accent);
                        }
                    }
                }
                if !any_rain && bar_h >= 50.0 && temps.len() >= 2 {
                    let (lo, hi) = temps.iter().fold((f32::MAX, f32::MIN), |(l, h), t| (l.min(*t), h.max(*t)));
                    let span = (hi - lo).max(6.0);
                    let (top, bottom) = (bar_top + 14.0, bar_top + bar_h + pp * 1.3);
                    let pts: Vec<(f32, f32)> = temps.iter().enumerate().map(|(i, t)| (24.0 + (i as f32 + 0.5) * col_w, bottom - 12.0 - (bottom - top - 24.0) * (t - lo) / span)).collect();
                    gfx::area(canvas, &pts, bottom, faded(accent, 0.30));
                    gfx::curve(canvas, &pts, 4.0, accent);
                    for p in &pts {
                        canvas.disc(p.0, p.1, 7.5, accent);
                        canvas.disc(p.0, p.1, 3.5, if light { Rgba(255, 255, 255, 255) } else { Rgba(30, 27, 44, 255) });
                    }
                }
            })?;
            made.pictures.push(img);
        }

        // the days, with the span of each on the span of them all
        let days = week_days(d.doc, d.more);
        if !days.is_empty() {
            let panel = Frame::new(MARGIN + 1264.0, TOP, WIDTH - 1264.0, BOTTOM - TOP);
            // the panel's words grow with the text size to a point: past it the ranges would be left no room
            let sd = s.min(1.15);
            let (cap_px, dp, np) = (24.0 * sd, 30.0 * sd, 28.0 * sd);
            let head_h = 22.0 + cap_px * 1.3 + 14.0;
            let row_h = (79.0 * s.min(1.2)).round();
            let rows = (((panel.h - head_h - 14.0) / row_h).floor() as usize).min(days.len()).max(1);
            let lo_min = days.iter().take(rows).map(|r| r.lo).fold(f32::MAX, f32::min);
            let hi_max = days.iter().take(rows).map(|r| r.hi).fold(f32::MIN, f32::max);
            let (name_w, ic, num_w) = (112.0 * sd, 44.0 * sd.min(1.2), 54.0 * sd);
            let ix = 28.0 + name_w + 4.0;
            let lo_x = ix + ic + 6.0;
            let bar_x = lo_x + num_w + 12.0;
            let bar_w = (panel.w - 28.0 - num_w - 10.0 - bar_x).max(40.0);
            let hi_x = bar_x + bar_w + 10.0;
            let mut lines: Vec<Line> = vec![(Frame::new(28.0, 22.0, panel.w - 56.0, cap_px * 1.3), sty(kit, cap_px, Weight::SemiBold, ink2, Align::Left), format!("{}-DAY FORECAST", days.len()))];
            for (i, r) in days.iter().take(rows).enumerate() {
                let y = head_h + i as f32 * row_h;
                let has_rain = r.rain.is_some_and(|p| p >= 20.0);
                let name_y = if has_rain { y + row_h * 0.12 } else { y + (row_h - dp * 1.3) / 2.0 };
                lines.push((Frame::new(28.0, name_y, name_w, dp * 1.3), sty(kit, dp, if i == 0 { Weight::SemiBold } else { Weight::Medium }, ink, Align::Left), r.name.clone()));
                let rain = r.rain.filter(|p| *p >= 20.0).map(|p| format!("{}%", p.round() as i64)).unwrap_or_default();
                lines.push((Frame::new(28.0, y + row_h * 0.12 + dp * 1.3 - 4.0, name_w, 24.0 * sd * 1.3), sty(kit, 24.0 * sd, Weight::Medium, legible(accent, ink, light), Align::Left), rain));
                lines.push((Frame::new(lo_x, y, num_w, row_h), sty(kit, np, Weight::Medium, ink2, Align::Right), format!("{}\u{b0}", r.lo.round() as i64)));
                lines.push((Frame::new(hi_x, y, num_w, row_h), sty(kit, np, Weight::SemiBold, ink, Align::Right), format!("{}\u{b0}", r.hi.round() as i64)));
            }
            made.cards.push(self.card(kit, panel, z, lines)?);
            let now_temp = d.num("/current/temp").map(|t| t as f32);
            let img = picture(kit, panel, z + 3, |canvas| {
                for (i, r) in days.iter().take(rows).enumerate() {
                    let y = head_h + i as f32 * row_h;
                    if let Some(p) = pictures.get_look(&format!("wxbig-{}", r.kind), light) {
                        canvas.picture(p, ix.round() as i32, (y + (row_h - ic) / 2.0).round() as i32, ic as u32, ic as u32);
                    }
                    let by = y + (row_h - 10.0) / 2.0;
                    canvas.rounded(Frame::new(bar_x, by, bar_w, 10.0), 5.0, faded(ink, 0.12));
                    if hi_max > lo_min {
                        let (f0, f1) = ((r.lo - lo_min) / (hi_max - lo_min), (r.hi - lo_min) / (hi_max - lo_min));
                        canvas.rounded(Frame::new(bar_x + bar_w * f0, by, (bar_w * (f1 - f0)).max(10.0), 10.0), 5.0, accent);
                        // where it is now, on today's bar
                        if i == 0
                            && let Some(t) = now_temp
                        {
                            let f = ((t - lo_min) / (hi_max - lo_min)).clamp(0.0, 1.0);
                            canvas.disc(bar_x + bar_w * f, by + 5.0, 8.0, if light { faded(Rgba(0, 0, 0, 255), 0.7) } else { Rgba(255, 255, 255, 255) });
                            canvas.disc(bar_x + bar_w * f, by + 5.0, 4.5, accent);
                        }
                    }
                    if i + 1 < rows {
                        canvas.rounded(Frame::new(28.0, y + row_h - 0.5, panel.w - 56.0, 1.0), 0.0, faded(ink, 0.08));
                    }
                }
            })?;
            made.pictures.push(img);
        }
        Ok(())
    }

    // ---- the details and the alerts
    fn page_details(&self, kit: &mut Kit, d: &Data, made: &mut Made) -> std::io::Result<()> {
        let z = self.z + 4;
        let (ink, ink2, accent, warn) = (kit.ui.look.ink(), kit.ui.look.ink2(), kit.ui.look.accent(), kit.ui.look.warning());
        let (good, bad) = (kit.ui.look.positive(), kit.ui.look.danger());
        let s = kit.text_scale.clamp(1.0, 1.35);
        self.sky(kit, d, made, false)?;
        let alerts: Vec<Value> = d.more.get("alerts").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut top = TOP;
        if !alerts.is_empty() {
            let shown = alerts.len().min(2);
            let (ep, hp) = (30.0 * s, 24.0 * s);
            let row = ep * 1.3 + hp * 1.3 + 12.0;
            let h = 24.0 + shown as f32 * row + 8.0;
            let mut lines: Vec<Line> = Vec::new();
            for (i, a) in alerts.iter().take(shown).enumerate() {
                let y = 20.0 + i as f32 * row;
                let text = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
                lines.push((Frame::new(40.0, y, 1700.0, ep * 1.3), sty(kit, ep, Weight::SemiBold, legible(warn, ink, kit.mode == Mode::Light), Align::Left), text("event")));
                let ends = a.get("ends").and_then(Value::as_str).map(|e| format!("  \u{b7}  until {}", when_label(e, d.twelve))).unwrap_or_default();
                lines.push((Frame::new(40.0, y + ep * 1.3, 1700.0, hp * 1.3), sty(kit, hp, Weight::Medium, ink2, Align::Left), format!("{}{ends}", text("headline"))));
            }
            if alerts.len() > shown {
                lines.push((Frame::new(1200.0, 20.0, 540.0, hp * 1.3), sty(kit, hp, Weight::Medium, ink2, Align::Right), format!("and {} more", alerts.len() - shown)));
            }
            made.cards.push(self.card(kit, Frame::new(MARGIN, top, WIDTH, h), z, lines)?);
            top += h + 20.0;
        }
        let gap = 20.0;
        let tile_w = ((WIDTH - 3.0 * gap) / 4.0).floor();
        let tile_h = ((BOTTOM - top - gap) / 2.0).floor();
        let metric = d.fahrenheit;
        let w_in = |v: f64| if metric { format!("{v:.2} in") } else { format!("{:.0} mm", v * 25.4) };
        let (lp, vp, sp) = (24.0 * s, 64.0 * s.min(1.2), 26.0 * s);
        let (lh, vh, sh) = (lp * 1.3, vp * 1.1, sp * 1.35);

        // (column, row, columns spanned, label, value, sub-line, what it draws)
        #[derive(Clone)]
        enum Draw {
            Feels(f32, f32, f32, f32),
            Wind(f32),
            /// The UV index, when the feed gives one.
            Uv(Option<f32>),
            Rain(Vec<f32>),
            Sun(Option<f32>, i32),
            Humidity(f32),
            Pressure(f32),
            /// Nothing to draw: the tile has no reading.
            Blank,
        }
        let temp = d.num("/current/temp").unwrap_or(0.0);
        let feels = d.num("/current/feelsLike");
        let wind_mph = d.num("/current/windMph");
        // each in its whole and in a shorter way, for a tile that is narrow at a large text size
        let feels_note: Vec<String> = match (feels, wind_mph) {
            (Some(f), Some(_)) if (f - temp).abs() < 2.0 => vec!["Similar to the actual temperature".into(), "About the same".into()],
            (Some(f), Some(w)) if f < temp => {
                if w >= 8.0 {
                    vec!["The wind makes it feel colder".into(), "Colder with the wind".into()]
                } else {
                    vec!["Feels colder than it is".into(), "Colder than it is".into()]
                }
            }
            (Some(_), _) => vec!["Feels warmer than it is".into(), "Warmer than it is".into()],
            _ => Vec::new(),
        };
        // the day's low and high, which the actual and the felt temperature are set against
        let day_deg = |k: &str| d.doc.pointer(&format!("/display/days/0/{k}")).and_then(Value::as_str).and_then(|t| t.trim_end_matches('\u{b0}').parse::<f32>().ok());
        let (day_lo, day_hi) = (day_deg("lo").unwrap_or(temp as f32 - 8.0).min(temp as f32).min(feels.unwrap_or(temp) as f32), day_deg("hi").unwrap_or(temp as f32 + 8.0).max(temp as f32).max(feels.unwrap_or(temp) as f32));
        // a tile: its column, row and span, its label, its value, its sub line (the whole of it, then shorter ways) and what it draws
        type Tile<'a> = (usize, usize, usize, &'a str, String, Vec<String>, Draw);
        let mut tiles: Vec<Tile> = Vec::new();
        tiles.push((0, 0, 1, "FEELS LIKE", feels.map(|v| d.deg(v)).unwrap_or_else(|| NONE.into()), feels_note, Draw::Feels(temp as f32, feels.unwrap_or(temp) as f32, day_lo, day_hi)));
        let wind = d.facts()[0].1.clone();
        let gust = d.more_num("/now/gustMph").map(|g| format!("Gusts {} mph", g.round() as i64)).unwrap_or_default();
        tiles.push((1, 0, 1, "WIND", wind, vec![gust], Draw::Wind(d.more_num("/now/windDeg").unwrap_or(0.0) as f32)));
        let uv = d.more_num("/now/uvIndex");
        tiles.push((2, 0, 1, "UV INDEX", uv.map(|u| format!("{}", u.round() as i64)).unwrap_or_else(|| NONE.into()), vec![uv.map(|u| uv_word(u).to_owned()).unwrap_or_default()], Draw::Uv(uv.map(|u| u as f32))));
        let rain = d.more.pointer("/daily/0/rainPct").and_then(Value::as_f64);
        let amount = d.more.pointer("/daily/0/rainIn").and_then(Value::as_f64).map(&w_in).unwrap_or_default();
        let chances: Vec<f32> = d.hours().iter().take(24).map(|h| h.get("rainPct").and_then(Value::as_f64).unwrap_or(0.0) as f32).collect();
        tiles.push((3, 0, 1, "RAIN TODAY", rain.map(|p| format!("{}%", p.round() as i64)).unwrap_or_else(|| NONE.into()), vec![if amount.is_empty() { String::new() } else { format!("{amount} expected") }], Draw::Rain(chances)));
        let rise = d.more.pointer("/daily/0/sunrise").and_then(Value::as_str).filter(|t| t.len() >= 16);
        let set = d.more.pointer("/daily/0/sunset").and_then(Value::as_str).filter(|t| t.len() >= 16);
        let now_t = d.doc.pointer("/current/time").and_then(Value::as_str).and_then(minutes);
        let (dayfrac, daylen) = match (rise.and_then(minutes), set.and_then(minutes), now_t) {
            (Some(r), Some(sn), Some(n)) if sn > r => (Some(((n - r) as f32 / (sn - r) as f32).clamp(-0.1, 1.1)), sn - r),
            (Some(r), Some(sn), None) if sn > r => (None, sn - r),
            _ => (None, 0),
        };
        let daylight = if daylen > 0 { format!("{} h {} min of daylight", daylen / 60, daylen % 60) } else { String::new() };
        // a feed with no sunrise or sunset (Home Assistant's weather entities) leaves the tile a dash and a line saying so, not an empty arc
        tiles.push(if rise.is_none() && set.is_none() {
            (0, 1, 2, "SUNRISE AND SUNSET", NONE.into(), vec!["Not reported".into()], Draw::Blank)
        } else {
            (0, 1, 2, "SUNRISE AND SUNSET", String::new(), vec![daylight], Draw::Sun(dayfrac, daylen))
        });
        let hum = d.num("/current/humidity");
        let cloud = d.more_num("/now/cloudPct").map(|c| format!("Cloud cover {}%", c.round() as i64)).unwrap_or_default();
        tiles.push((2, 1, 1, "HUMIDITY", hum.map(|v| format!("{}%", v.round() as i64)).unwrap_or_else(|| NONE.into()), vec![cloud], Draw::Humidity(hum.unwrap_or(0.0) as f32)));
        let pressure = d.more_num("/now/pressureHpa");
        let vis = d.more_num("/now/visibilityM").map(|m| if metric { format!("Visibility {} mi", (m / 1609.34).round() as i64) } else { format!("Visibility {} km", (m / 1000.0).round() as i64) }).unwrap_or_default();
        let p_text = pressure.map(|p| if metric { format!("{:.2} inHg", p * 0.029_53) } else { format!("{} hPa", p.round() as i64) }).unwrap_or_else(|| NONE.into());
        tiles.push((3, 1, 1, "PRESSURE", p_text, vec![vis], Draw::Pressure(pressure.unwrap_or(1013.0) as f32)));

        for (col, row, span, label, value, sub, draw) in tiles {
            let w = tile_w * span as f32 + gap * (span as f32 - 1.0);
            let rect = Frame::new(MARGIN + col as f32 * (tile_w + gap), top + row as f32 * (tile_h + gap), w, tile_h);
            let vpx = if value.chars().count() > 6 { vp * 0.78 } else { vp };
            let label_style = sty(kit, lp, Weight::SemiBold, ink2, Align::Left);
            let label = fit(kit, &label_style, &[label.to_owned()], w - 56.0);
            let mut lines: Vec<Line> = vec![(Frame::new(28.0, 24.0, w - 56.0, lh), label_style, label)];
            let mut y = 24.0 + lh + 4.0;
            if !value.is_empty() {
                let value_style = sty(kit, vpx, Weight::SemiBold, ink, Align::Left);
                let value = fit(kit, &value_style, &[value], w - 56.0);
                lines.push((Frame::new(28.0, y, w - 56.0, vh), value_style, value));
                y += vh + 2.0;
            }
            let sub_style = sty(kit, sp, Weight::Medium, ink2, Align::Left);
            let sub = fit(kit, &sub_style, &sub, w - 56.0);
            lines.push((Frame::new(28.0, y, w - 56.0, sh), sub_style, sub));
            y += sh + 14.0;
            // the graphic's room: under the words to the tile's foot
            let g = Frame::new(28.0, y, w - 56.0, (tile_h - y - 26.0).max(0.0));
            // the words that go with a graphic (the ends of a scale, the times under an arc)
            let mut extra: Vec<Line> = Vec::new();
            if g.h >= 60.0 {
                match &draw {
                    Draw::Pressure(_) => {
                        extra.push((Frame::new(g.x, g.bottom() - sh, 120.0, sh), sty(kit, 24.0 * s, Weight::Medium, ink2, Align::Left), "Low".to_owned()));
                        extra.push((Frame::new(g.right() - 120.0, g.bottom() - sh, 120.0, sh), sty(kit, 24.0 * s, Weight::Medium, ink2, Align::Right), "High".to_owned()));
                    }
                    Draw::Uv(_) => {
                        extra.push((Frame::new(g.x, g.bottom() - sh, 120.0, sh), sty(kit, 24.0 * s, Weight::Medium, ink2, Align::Left), "0".to_owned()));
                        extra.push((Frame::new(g.right() - 120.0, g.bottom() - sh, 120.0, sh), sty(kit, 24.0 * s, Weight::Medium, ink2, Align::Right), "11+".to_owned()));
                    }
                    Draw::Sun(_, _) => {
                        let (r, sn) = (rise.map(|t| clock_label(t, d.twelve)).unwrap_or_default(), set.map(|t| clock_label(t, d.twelve)).unwrap_or_default());
                        extra.push((Frame::new(g.x, g.bottom() - sh, 300.0, sh), sty(kit, sp, Weight::SemiBold, ink, Align::Left), r));
                        extra.push((Frame::new(g.right() - 300.0, g.bottom() - sh, 300.0, sh), sty(kit, sp, Weight::SemiBold, ink, Align::Right), sn));
                    }
                    Draw::Feels(a, f, lo, hi) => {
                        let x_of = |v: f32| g.x + 24.0 + (g.w - 48.0) * (v - lo) / (hi - lo).max(1.0);
                        let (xa, xf) = (x_of(*a), x_of(*f));
                        let mid = g.y + g.h / 2.0;
                        // each label is centred on its dot, kept inside the tile and cut or shortened to the room it has
                        let mut mark = |kit: &mut Kit, x: f32, y: f32, weight: Weight, colour: Rgba, word: &str, v: f32| {
                            let style = sty(kit, 24.0 * s, weight, colour, Align::Centre);
                            let room = (180.0 * s.max(1.0)).min(g.w * 0.7);
                            let text = fit(kit, &style, &[format!("{word} {}", d.deg(f64::from(v))), d.deg(f64::from(v))], room);
                            let left = (x - room / 2.0).clamp(g.x, g.right() - room);
                            extra.push((Frame::new(left, y, room, sh), style, text));
                        };
                        mark(kit, xa, mid - 20.0 - sh, Weight::Medium, ink2, "Actual", *a);
                        mark(kit, xf, mid + 22.0, Weight::SemiBold, ink, "Feels", *f);
                        if s <= 1.2 {
                            extra.push((Frame::new(g.x, g.bottom() - sh, 120.0, sh), sty(kit, 24.0 * s, Weight::Medium, ink2, Align::Left), d.deg(f64::from(*lo))));
                            extra.push((Frame::new(g.right() - 120.0, g.bottom() - sh, 120.0, sh), sty(kit, 24.0 * s, Weight::Medium, ink2, Align::Right), d.deg(f64::from(*hi))));
                        }
                    }
                    Draw::Rain(_) => {
                        extra.push((Frame::new(g.x, g.bottom() - sh, 120.0, sh), sty(kit, 24.0 * s, Weight::Medium, ink2, Align::Left), "Now".to_owned()));
                        extra.push((Frame::new(g.right() - 120.0, g.bottom() - sh, 120.0, sh), sty(kit, 24.0 * s, Weight::Medium, ink2, Align::Right), "24 h".to_owned()));
                    }
                    Draw::Wind(_) | Draw::Humidity(_) | Draw::Blank => {}
                }
            }
            lines.extend(extra);
            made.cards.push(self.card(kit, rect, z, lines)?);
            if g.h < 60.0 || matches!(draw, Draw::Blank) {
                continue;
            }
            let img = picture(kit, rect, z + 3, |c| match draw {
                Draw::Wind(from) => {
                    let (cx, cy, r) = (g.x + g.w / 2.0, g.y + g.h / 2.0, (g.w.min(g.h) / 2.0 - 4.0).max(20.0));
                    gfx::arc(c, cx, cy, r, 3.0, 0.0, std::f32::consts::TAU, faded(ink, 0.16));
                    for q in 0..4 {
                        let a = q as f32 * std::f32::consts::FRAC_PI_2;
                        let (sa, ca) = (a.sin(), -a.cos());
                        let len = if q == 0 { 14.0 } else { 8.0 };
                        gfx::line(c, (cx + sa * (r - len), cy + ca * (r - len)), (cx + sa * r, cy + ca * r), 3.0, if q == 0 { ink } else { faded(ink, 0.5) });
                    }
                    // the arrow points where the wind goes
                    let a = (from + 180.0).to_radians();
                    let (sa, ca) = (a.sin(), -a.cos());
                    gfx::line(c, (cx - sa * r * 0.5, cy - ca * r * 0.5), (cx + sa * r * 0.58, cy + ca * r * 0.58), 5.0, accent);
                    gfx::head(c, (cx + sa * r * 0.8, cy + ca * r * 0.8), a, 26.0, 26.0, accent);
                    c.disc(cx, cy, 5.0, accent);
                }
                Draw::Uv(v) => {
                    let by = g.y + g.h / 2.0 - 8.0;
                    let stops = [(0.0, faded(good, 1.0)), (0.27, faded(good, 1.0)), (0.5, faded(warn, 1.0)), (0.72, faded(bad, 0.95)), (1.0, faded(accent, 1.0))];
                    gfx::gradient_bar(c, Frame::new(g.x, by, g.w, 16.0), 8.0, &stops);
                    if let Some(x) = uv_knob(v, g.x, g.w) {
                        c.disc(x, by + 8.0, 17.0, faded(Rgba(0, 0, 0, 255), 0.55));
                        c.disc(x, by + 8.0, 13.0, ink);
                    }
                }
                Draw::Rain(chances) => {
                    let n = chances.len().max(1);
                    let gap_b = 4.0;
                    let bw = ((g.w - gap_b * (n as f32 - 1.0)) / n as f32).max(3.0);
                    let base = g.bottom() - sh - 6.0;
                    let max_h = (g.h - sh - 14.0).max(10.0);
                    for (i, p) in chances.iter().enumerate() {
                        let x = g.x + i as f32 * (bw + gap_b);
                        c.rounded(Frame::new(x, base - max_h, bw, max_h), bw.min(6.0) / 1.5, faded(ink, 0.07));
                        let hh = (max_h * p / 100.0).max(if *p >= 5.0 { 4.0 } else { 0.0 });
                        if hh > 0.0 {
                            c.rounded(Frame::new(x, base - hh, bw, hh), bw.min(6.0) / 1.5, accent);
                        }
                    }
                }
                Draw::Sun(frac, len) => {
                    let r = ((g.w - 80.0) / 2.0).min(g.h - sh - 24.0).max(40.0);
                    let (cx, cy) = (g.x + g.w / 2.0, g.bottom() - sh - 12.0);
                    gfx::arc(c, cx, cy, r, 4.0, -std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2, faded(ink, 0.16));
                    gfx::line(c, (g.x + 12.0, cy), (g.right() - 12.0, cy), 2.0, faded(ink, 0.22));
                    if let Some(f) = frac.filter(|_| len > 0) {
                        let f = f.clamp(0.0, 1.0);
                        gfx::arc(c, cx, cy, r, 4.0, -std::f32::consts::FRAC_PI_2, -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * f, accent);
                        let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * f;
                        let (sx, sy) = (cx + r * a.sin(), cy - r * a.cos());
                        c.disc(sx, sy, 20.0, faded(accent, 0.28));
                        c.disc(sx, sy, 11.0, Rgba(246, 193, 119, 255));
                    }
                }
                Draw::Blank => {}
                Draw::Humidity(v) => {
                    let (cx, cy, r) = (g.x + g.w / 2.0, g.y + g.h / 2.0, (g.w.min(g.h) / 2.0 - 8.0).max(24.0));
                    let (a0, sweep) = (-0.75 * std::f32::consts::PI, 1.5 * std::f32::consts::PI);
                    gfx::arc(c, cx, cy, r, 12.0, a0, a0 + sweep, faded(ink, 0.12));
                    let f = (v / 100.0).clamp(0.0, 1.0);
                    if f > 0.0 {
                        gfx::arc(c, cx, cy, r, 12.0, a0, a0 + sweep * f, accent);
                    }
                }
                Draw::Pressure(p) => {
                    let by = g.y + g.h / 2.0 - 5.0;
                    c.rounded(Frame::new(g.x, by, g.w, 10.0), 5.0, faded(ink, 0.14));
                    let f = ((p - 980.0) / 60.0).clamp(0.0, 1.0);
                    c.rounded(Frame::new(g.x, by, (g.w * f).max(10.0), 10.0), 5.0, faded(accent, 0.55));
                    c.disc(g.x + g.w * f, by + 5.0, 14.0, faded(Rgba(0, 0, 0, 255), 0.5));
                    c.disc(g.x + g.w * f, by + 5.0, 10.0, ink);
                }
                Draw::Feels(a, f, lo, hi) => {
                    let x_of = |v: f32| g.x + 24.0 + (g.w - 48.0) * (v - lo) / (hi - lo).max(1.0);
                    let by = g.y + g.h / 2.0 - 6.0;
                    // the day's span, and where the actual and the felt temperature are in it
                    c.rounded(Frame::new(g.x + 24.0, by, g.w - 48.0, 12.0), 6.0, faded(ink, 0.14));
                    let (x0, x1) = (x_of(a).min(x_of(f)), x_of(a).max(x_of(f)));
                    c.rounded(Frame::new(x0, by, (x1 - x0).max(12.0), 12.0), 6.0, faded(accent, 0.6));
                    c.disc(x_of(a), by + 6.0, 14.0, faded(ink, 0.7));
                    c.disc(x_of(a), by + 6.0, 9.0, faded(Rgba(0, 0, 0, 255), 0.55));
                    c.disc(x_of(f), by + 6.0, 15.0, accent);
                }
            })?;
            made.pictures.push(img);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn right_and_left_walk_the_pages_and_nothing_else_does() {
        let mut v = WeatherView::default();
        assert_eq!(v.key("left", -1), Done::Nothing, "the first page has none before it");
        assert_eq!(v.key("down", 0), Done::Nothing, "Down does not page");
        assert_eq!(v.key("right", 1), Done::Redraw);
        assert_eq!(v.page, Page::Radar);
        assert_eq!(v.key("right", 1), Done::Redraw);
        assert_eq!(v.page, Page::Details);
        assert_eq!(v.key("right", 1), Done::Nothing, "nor the last one after it");
        assert_eq!(v.key("left", -1), Done::Redraw);
        assert_eq!(v.page, Page::Radar);
        assert_eq!(v.key("back", 0), Done::Close);
        v.open();
        assert_eq!(v.page, Page::Now);
    }

    #[test]
    fn hours_and_times_read_as_the_clock_does() {
        assert_eq!(hour_label("2026-10-29T22:00", true), "10 PM");
        assert_eq!(hour_label("2026-10-29T00:00", true), "12 AM");
        assert_eq!(hour_label("2026-10-29T12:00", true), "12 PM");
        assert_eq!(hour_label("2026-10-29T07:00", false), "07");
        assert_eq!(clock_label("2026-10-30T07:42", true), "7:42 AM");
        assert_eq!(clock_label("2026-10-30T18:24", true), "6:24 PM");
        assert_eq!(clock_label("2026-10-30T18:24", false), "18:24");
        assert_eq!(when_label("2026-10-31T08:00:00-04:00", true), "8 AM", "on the hour: the hour");
        assert_eq!(when_label("2026-10-31T08:30:00-04:00", true), "8:30 AM");
        assert_eq!((compass(0.0), compass(225.0), compass(359.0), compass(90.0)), ("N", "SW", "N", "E"));
        assert_eq!((uv_word(1.0), uv_word(4.0), uv_word(7.0), uv_word(9.0), uv_word(12.0)), ("Low", "Moderate", "High", "Very high", "Extreme"));
    }

    #[test]
    fn the_sentence_says_when_the_rain_comes_or_goes() {
        let hour = |t: &str, code: i64, p: f64| json!({"time": format!("2026-10-29T{t}:00"), "code": code, "rainPct": p});
        let dry_then_rain = [hour("21", 0, 0.0), hour("22", 0, 10.0), hour("23", 61, 80.0), hour("00", 61, 90.0)];
        assert_eq!(sentence(&dry_then_rain, 0, true), "Rain from 11 PM");
        let rain_then_dry = [hour("21", 61, 90.0), hour("22", 61, 80.0), hour("23", 3, 10.0)];
        assert_eq!(sentence(&rain_then_dry, 63, true), "Rain until 11 PM");
        let dry = [hour("21", 0, 0.0), hour("22", 0, 5.0)];
        assert_eq!(sentence(&dry, 0, true), "No rain for the next day");
        let maybe = [hour("21", 0, 0.0), hour("22", 3, 40.0)];
        assert_eq!(sentence(&maybe, 3, true), "A chance of rain today");
        // what falls is named: snow is snow, a storm is a storm
        let snow = [hour("21", 73, 90.0), hour("22", 73, 80.0), hour("23", 3, 10.0)];
        assert_eq!(sentence(&snow, 73, true), "Snow until 11 PM");
        let storms_later = [hour("21", 0, 0.0), hour("22", 0, 0.0), hour("23", 95, 80.0)];
        assert_eq!(sentence(&storms_later, 0, true), "Thunderstorms from 11 PM");
    }

    #[test]
    fn the_smart_line_is_an_alert_first_then_what_falls_then_the_suns_next_turn() {
        let doc = |code: i64, day: bool| json!({"current": {"code": code, "isDay": day, "time": "2026-10-30T15:00", "humidity": 53.0, "windMph": 6.0}});
        let hours = |rain: bool| (0..6).map(|i| json!({"time": format!("2026-10-30T{:02}:00", 15 + i), "code": if rain && i >= 2 { 61 } else { 0 }, "rainPct": if rain && i >= 2 { 80.0 } else { 0.0 }})).collect::<Vec<_>>();
        let more = |rain: bool, alerts: Value| json!({"hourly": hours(rain), "daily": [{"sunrise": "2026-10-30T07:41", "sunset": "2026-10-30T18:25"}, {"sunrise": "2026-10-31T07:42"}], "alerts": alerts});
        let (full, short, tone) = smart_line(&doc(0, true), &more(false, json!([{"event": "Flood Watch", "ends": "2026-10-31T08:00:00-04:00"}])), true);
        assert_eq!((full.as_str(), short.as_str(), tone), ("Flood Watch until 8 AM", "Flood Watch", SmartTone::Warning));
        let (full, _, tone) = smart_line(&doc(0, true), &more(true, json!([])), true);
        assert_eq!((full.as_str(), tone), ("Rain from 5 PM", SmartTone::Info));
        let (full, short, _) = smart_line(&doc(0, true), &more(false, json!([])), true);
        assert_eq!((full.as_str(), short.as_str()), ("Sunset at 6:25 PM", "Sunset 6:25 PM"));
        let (full, _, _) = smart_line(&doc(0, false), &more(false, json!([])), true);
        assert_eq!(full, "Sunrise at 7:42 AM", "after dark: tomorrow's sunrise");
        // no hours: how it is out
        let (full, short, _) = smart_line(&doc(0, true), &json!({}), true);
        assert_eq!((full.as_str(), short.as_str()), ("53% humidity  \u{b7}  6 mph wind", "53%  \u{b7}  6 mph"));
    }

    #[test]
    fn a_reading_the_feed_lacks_is_a_dash_not_a_blank() {
        let (doc, more) = (json!({"current": {"windMph": 11.4, "humidity": 33.0}}), json!({"now": {"windDeg": 315.0}, "daily": [{"rainPct": 0.0}]}));
        let d = Data { doc: &doc, more: &more, twelve: true, fahrenheit: true };
        let facts = d.facts();
        assert_eq!(facts.clone().map(|(_, v)| v), ["11 mph NW", "33%", NONE, "0%"]);
        assert_eq!(facts[2].0, "UV INDEX");
        let (doc, more) = (Value::Null, Value::Null);
        let d = Data { doc: &doc, more: &more, twelve: true, fahrenheit: true };
        assert!(d.facts().iter().all(|(_, v)| v == NONE));
        assert_eq!(uv_knob(None, 10.0, 110.0), None, "an unknown UV has no knob, not one at 0 under the dash");
        assert_eq!((uv_knob(Some(4.0), 10.0, 110.0), uv_knob(Some(14.0), 10.0, 110.0)), (Some(50.0), Some(120.0)));
    }

    #[test]
    fn the_week_is_the_helpers_ten_days_or_the_cards_seven() {
        assert_eq!((weekday("2026-10-30"), weekday("2026-11-01"), weekday("2026-03-01"), weekday("2024-02-29"), weekday("nope")), ("Fri", "Sun", "Sun", "Thu", ""));
        let doc = json!({"display": {"days": [{"day": "Today", "kind": "sun", "hi": "71\u{b0}", "lo": "51\u{b0}"}, {"day": "Mon", "kind": "cloud", "hi": "73\u{b0}", "lo": "51\u{b0}"}]}});
        let more = json!({"daily": [{"date": "2026-10-30", "code": 0, "hi": 71.4, "lo": 51.0, "rainPct": 20.0}, {"date": "2026-10-31", "code": 61, "hi": 62.0, "lo": 48.0, "rainPct": 70.0}, {"date": "2026-11-01", "code": 3, "hi": 60.0, "lo": 45.0}]});
        let ten = week_days(&doc, &more);
        assert_eq!(ten.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["Today", "Sat", "Sun"]);
        assert_eq!((ten[1].kind.as_str(), ten[1].hi, ten[1].lo, ten[1].rain), ("rain", 62.0, 48.0, Some(70.0)));
        // documents kept before the ten days have no highs and lows: the card's days are the list
        let old = json!({"daily": [{"date": "2026-10-30", "rainPct": 20.0}]});
        let seven = week_days(&doc, &old);
        assert_eq!((seven.len(), seven[0].name.as_str(), seven[0].hi, seven[0].rain, seven[1].rain), (2, "Today", 71.0, Some(20.0), None));
        assert!(week_days(&json!({}), &json!({})).is_empty());
    }

    #[test]
    fn the_hero_gives_up_the_facts_and_then_the_size_before_it_runs_into_what_is_under_it() {
        assert!(hero_flow(1.0, 550.0).facts, "at the size it is made for, everything fits");
        let big = hero_flow(1.35, 550.0);
        assert!(!big.facts && big.temp_px >= 96.0, "at the largest text size the facts go");
    }

    #[test]
    fn the_icon_of_a_code_is_the_helpers() {
        for (code, day, kind) in [(0, true, "sun"), (0, false, "moon"), (2, false, "partly-night"), (3, true, "cloud"), (45, true, "fog"), (53, true, "drizzle"), (63, true, "rain"), (73, true, "snow"), (95, true, "storm")] {
            assert_eq!(kind_of(code, day), kind, "{code}");
        }
    }
}
