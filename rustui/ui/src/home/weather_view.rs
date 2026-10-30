//! The full weather view: OK on the weather card opens it over Home (`Layer::Weather`). Up and Down walk its pages (now with the hours,
//! the week, the details with the alerts), Back closes it. It is drawn from the helper's `weather` document (the card's) and `weather.more`
//! (the hours, the details of now, each day's sun and UV, the alerts), both kept fresh in the background by the helper, so the page is
//! made from what is already there, with nothing fetched when it opens.
//!
//! The model is pure ([`WeatherView`]: the page, the keys); [`WeatherSurface`] draws it: one scrim and bar, and the page's glass cards,
//! words and pictures, made when the page (or the reading, or the text size) changes and dropped with it.

use super::clock;
use super::model::Home;
use super::surface::{HIDDEN, faded, glass_of};
use super::weather_art;
use super::widgets::{Canvas, Pictures, Words};
use crate::kit::pane::PaneKind;
use crate::kit::text::{Text, role_style};
use crate::kit::Kit;
use compositor_protocol::{Props, Rgba};
use lumatv_tokens::TypeRole;
use serde_json::Value;
use ui::components::{Align, Style, plate, style};
use ui::geometry::Frame;
use ui::lucent::palette::Mode;
use ui::scene::{GlassLayer, ImageLayer, ShapeLayer};
use ui::text::Weight;

/// The pages, in the order Down walks them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Now,
    Week,
    Details,
}

pub const PAGES: [Page; 3] = [Page::Now, Page::Week, Page::Details];

impl Page {
    pub fn name(self) -> &'static str {
        match self {
            Page::Now => "now",
            Page::Week => "week",
            Page::Details => "details",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Page::Now => "Now and next hours",
            Page::Week => "This week",
            Page::Details => "Details",
        }
    }
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

    /// A key of the view; `Done::Close` for Back.
    pub fn key(&mut self, name: &str, up: bool, down: bool) -> Done {
        let i = PAGES.iter().position(|p| *p == self.page).unwrap_or(0);
        match name {
            "back" => Done::Close,
            _ if up && i > 0 => {
                self.page = PAGES[i - 1];
                Done::Redraw
            }
            _ if down && i + 1 < PAGES.len() => {
                self.page = PAGES[i + 1];
                Done::Redraw
            }
            _ => Done::Nothing,
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

fn is_rain(code: i64) -> bool {
    (51..=67).contains(&code) || (80..=82).contains(&code) || code >= 95
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

/// One sentence for the hours ahead: when the rain starts or stops, or that there is none.
pub fn sentence(hours: &[Value], now_code: i64, twelve: bool) -> String {
    let code = |h: &Value| h.get("code").and_then(Value::as_i64).unwrap_or(0);
    let rain = |h: &Value| is_rain(code(h)) || h.get("rainPct").and_then(Value::as_f64).is_some_and(|p| p >= 60.0);
    let time = |h: &Value| hour_label(h.get("time").and_then(Value::as_str).unwrap_or(""), twelve);
    let upcoming = hours.iter().skip(1).take(24);
    if is_rain(now_code) {
        match upcoming.clone().find(|h| !rain(h)) {
            Some(h) => format!("Rain until {}", time(h)),
            None => "Rain for the next day".to_owned(),
        }
    } else {
        match upcoming.clone().find(|h| rain(h)) {
            Some(h) => format!("Rain from {}", time(h)),
            None => {
                let peak = hours.iter().take(24).filter_map(|h| h.get("rainPct").and_then(Value::as_f64)).fold(0.0, f64::max);
                if peak >= 30.0 { "A chance of rain today".to_owned() } else { "No rain for the next day".to_owned() }
            }
        }
    }
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
    pictures: Vec<ImageLayer>,
    art: Option<weather_art::Live>,
    key: String,
    page: Page,
    scale: f32,
    mode: Mode,
}

pub struct WeatherSurface {
    _scrim: ShapeLayer,
    _bar: GlassLayer,
    title: Text,
    clock: Text,
    dots: Vec<ShapeLayer>,
    made: Option<Made>,
    z: i32,
    /// The bar's words are for this page and clock.
    bar_key: String,
    /// The art waits for the glyph atlas to settle, as the card's does.
    art_wait: Option<(weather_art::Cond, f32, (f32, f32))>,
}

const MARGIN: f32 = 64.0;
const TOP: f32 = 140.0;
const BOTTOM: f32 = 1016.0;

impl WeatherSurface {
    pub fn new(kit: &mut Kit, home: &Home, z: i32) -> std::io::Result<Self> {
        let scrim = kit.ui.scene.shape(1920, 1080, z, plate::solid(kit.tokens.extra(kit.mode).scrim_hide, 0.0), Props::at(0.0, 0.0));
        let bar_f = Frame::new(MARGIN, 56.0, 1792.0, 56.0);
        let bar = kit.ui.scene.glass(1792, 56, z + 1, glass_of(kit, PaneKind::Pill, 56.0, false, false), Props::at(bar_f.x, bar_f.y));
        let ink = kit.ui.look.ink();
        let ink2 = kit.ui.look.ink2();
        let title_f = Frame::new(bar_f.x + kit.space(5) + 12.0, bar_f.y, 900.0, 56.0);
        let clock_f = Frame::new(bar_f.x + 1792.0 - 400.0 - kit.space(6), bar_f.y, 400.0, 56.0);
        let title = Text::new(kit, title_f, z + 2, role_style(kit, TypeRole::Body, Some(Weight::SemiBold), ink, Align::Left), "Weather")?;
        let clock = Text::new(kit, clock_f, z + 2, role_style(kit, TypeRole::Label, None, ink2, Align::Right), "")?;
        let mut me = Self { _scrim: scrim, _bar: bar, title, clock, dots: Vec::new(), made: None, z, bar_key: String::new(), art_wait: None };
        let accent = kit.ui.look.accent();
        let _ = (home, accent);
        for _ in PAGES {
            me.dots.push(kit.ui.scene.shape(14, 14, z + 3, plate::solid(faded(ink2, 0.5), 7.0), HIDDEN));
        }
        Ok(me)
    }

    /// Whether the picture of the page moves: the same rule as the card's.
    pub fn update(&mut self, kit: &mut Kit, home: &Home, pictures: &mut Pictures, now: u64, run_art: bool) -> std::io::Result<()> {
        let data = Data::new(home);
        let page = home.wview.page;
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
        let bar_right = MARGIN + 1792.0 - 28.0;
        for (i, d) in self.dots.iter().enumerate() {
            let x = bar_right - (PAGES.len() - i) as f32 * 24.0 - 330.0;
            let on = PAGES[i] == page;
            d.reshape(plate::solid(if on { accent } else { faded(ink2, 0.5) }, 7.0));
            d.set(Props::at(x, 56.0 + 21.0));
        }
        let key = data.key();
        let need = self.made.as_ref().is_none_or(|m| m.key != key || m.page != page || m.scale != kit.text_scale || m.mode != kit.mode);
        if need {
            self.made = None;
            self.made = Some(self.build(kit, &data, page, pictures, key)?);
        }
        if let Some(m) = &mut self.made {
            if let Some(a) = m.art.as_mut() {
                if run_art {
                    a.step(now);
                } else if a.running() {
                    a.rest();
                }
            }
            self.art_wait = None;
        }
        Ok(())
    }

    /// When the page's picture is next due while it moves.
    pub fn due_at(&self, now: u64) -> Option<u64> {
        self.made.as_ref()?.art.as_ref().filter(|a| a.running()).map(|_| now - now % weather_art::TICK_MS + weather_art::TICK_MS)
    }

    fn card(&self, kit: &mut Kit, rect: Frame, z: i32, lines: Vec<Line>) -> std::io::Result<Card> {
        let glass = kit.ui.scene.glass(rect.w as u32, rect.h as u32, z, glass_of(kit, PaneKind::Card, rect.h, false, false), Props::at(rect.x, rect.y));
        let mut words = Words::new(kit, Frame::new(0.0, 0.0, rect.w, rect.h), z + 1, lines)?;
        words.set_props(Props::at(rect.x, rect.y));
        Ok(Card { _glass: glass, _words: words })
    }

    fn build(&mut self, kit: &mut Kit, d: &Data, page: Page, pictures: &mut Pictures, key: String) -> std::io::Result<Made> {
        let mut made = Made { cards: Vec::new(), pictures: Vec::new(), art: None, key, page, scale: kit.text_scale, mode: kit.mode };
        match page {
            Page::Now => self.page_now(kit, d, pictures, &mut made)?,
            Page::Week => self.page_week(kit, d, pictures, &mut made)?,
            Page::Details => self.page_details(kit, d, &mut made)?,
        }
        Ok(made)
    }

    // ---- now: the reading with its picture, and the next hours
    fn page_now(&self, kit: &mut Kit, d: &Data, pictures: &mut Pictures, made: &mut Made) -> std::io::Result<()> {
        let z = self.z + 4;
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let k = kit.text_scale.max(1.0);
        let left = Frame::new(MARGIN, TOP, 700.0, BOTTOM - TOP);
        let hero = (210.0 * k.min(1.4)).round();
        let temp_ty = |kit: &Kit, px: f32| {
            let mut ty = kit.ty(TypeRole::Display);
            ty.px = (px * k.min(1.4)).round() as u32;
            ty.weight = Weight::SemiBold;
            ty
        };
        let temp_h = temp_ty(kit, 200.0).px as f32 * 1.1;
        let title_h = kit.line(TypeRole::Title);
        let body_h = kit.line(TypeRole::Body);
        let mut lines: Vec<Line> = vec![
            (Frame::new(40.0, 32.0, 560.0, title_h), role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left), d.display("place")),
            (Frame::new(40.0, 32.0 + title_h + 10.0, 440.0, temp_h), style(temp_ty(kit, 200.0), ink, Align::Left), d.display("temp")),
            (Frame::new(40.0, 32.0 + title_h + 10.0 + temp_h + 4.0, 620.0, title_h), role_style(kit, TypeRole::Title, None, ink, Align::Left), d.display("cond")),
        ];
        let mut y = 32.0 + title_h + 10.0 + temp_h + 4.0 + title_h + 22.0;
        if let (Some(feels), Some(hi), Some(lo)) = (d.num("/current/feelsLike"), d.doc.pointer("/display/days/0/hi").and_then(Value::as_str), d.doc.pointer("/display/days/0/lo").and_then(Value::as_str)) {
            lines.push((Frame::new(40.0, y, 620.0, body_h), role_style(kit, TypeRole::Body, None, ink2, Align::Left), format!("Feels like {} \u{b7} High {hi} \u{b7} Low {lo}", d.deg(feels))));
            y += body_h + 6.0;
        }
        let hours = d.hours();
        let now_code = d.num("/current/code").map(|c| c as i64).unwrap_or(0);
        if !hours.is_empty() {
            lines.push((Frame::new(40.0, y, 620.0, body_h), role_style(kit, TypeRole::Body, Some(Weight::SemiBold), kit.ui.look.accent(), Align::Left), sentence(&hours, now_code, d.twelve)));
        }
        // the facts of now, two by two, at the card's foot
        let (label_h0, body_h0) = (kit.line(TypeRole::Caption), kit.line(TypeRole::Body));
        let fact_w = (left.w - 80.0) / 2.0;
        let wind = d.num("/current/windMph").map(|v| format!("{} mph{}", v.round() as i64, d.more_num("/now/windDeg").map(|dg| format!(" {}", compass(dg))).unwrap_or_default())).unwrap_or_default();
        let hum = d.num("/current/humidity").map(|v| format!("{}%", v.round() as i64)).unwrap_or_default();
        let uv = d.more_num("/now/uvIndex").map(|u| format!("{} {}", u.round() as i64, uv_word(u))).unwrap_or_default();
        let rain = d.more.pointer("/daily/0/rainPct").and_then(Value::as_f64).map(|p| format!("{}%", p.round() as i64)).unwrap_or_default();
        let facts = [("WIND", wind), ("HUMIDITY", hum), ("UV INDEX", uv), ("RAIN TODAY", rain)];
        let cell_h = label_h0 + body_h0 + 18.0;
        for (i, (label, value)) in facts.into_iter().enumerate() {
            let (col, row) = (i % 2, i / 2);
            let x = 40.0 + col as f32 * fact_w;
            let fy = left.h - 40.0 - 2.0 * cell_h + row as f32 * (cell_h + 6.0);
            lines.push((Frame::new(x, fy, fact_w - 12.0, label_h0), role_style(kit, TypeRole::Caption, Some(Weight::SemiBold), ink2, Align::Left), label.to_owned()));
            lines.push((Frame::new(x, fy + label_h0 + 2.0, fact_w - 12.0, body_h0), role_style(kit, TypeRole::Body, None, ink, Align::Left), value));
        }
        made.cards.push(self.card(kit, left, z, lines)?);
        // the picture of the condition, on the card's right
        let origin = (left.x + left.w - hero - 36.0, left.y + 36.0);
        if let Some(cond) = weather_art::Cond::from_doc(d.doc) {
            made.art = weather_art::Live::new(kit, cond, hero, kit.mode == Mode::Light, origin, z + 2).ok();
        }
        // the hours
        let right = Frame::new(MARGIN + 700.0 + 24.0, TOP, 1792.0 - 700.0 - 24.0, BOTTOM - TOP);
        let cols = 12usize;
        let col_w = ((right.w - 64.0) / cols as f32).floor();
        let (label_h, num_h, cap_h) = (kit.line(TypeRole::Label), kit.line(TypeRole::Body), kit.line(TypeRole::Caption));
        let ink3 = faded(ink2, 0.8);
        let mut lines: Vec<Line> = vec![(Frame::new(40.0, 28.0, 600.0, title_h), role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left), "Next hours".to_owned())];
        let strip_y = 28.0 + title_h + 30.0;
        let shown: Vec<&Value> = hours.iter().take(cols).collect();
        for (i, h) in shown.iter().enumerate() {
            let x = 32.0 + i as f32 * col_w;
            let time = if i == 0 { "Now".to_owned() } else { hour_label(h.get("time").and_then(Value::as_str).unwrap_or(""), d.twelve) };
            lines.push((Frame::new(x, strip_y, col_w, label_h), role_style(kit, TypeRole::Label, None, ink2, Align::Centre), time));
            lines.push((Frame::new(x, strip_y + label_h + 60.0, col_w, num_h), role_style(kit, TypeRole::Body, Some(Weight::SemiBold), ink, Align::Centre), d.deg(h.get("temp").and_then(Value::as_f64).unwrap_or(0.0))));
            let rain = h.get("rainPct").and_then(Value::as_f64).filter(|p| *p >= 10.0).map(|p| format!("{}%", p.round() as i64)).unwrap_or_default();
            lines.push((Frame::new(x, strip_y + label_h + 60.0 + num_h, col_w, cap_h), role_style(kit, TypeRole::Caption, None, ink3, Align::Centre), rain));
        }
        // the curve of the temperature and the bars of the rain, over the whole of the hours held
        let chart = Frame::new(32.0, strip_y + label_h + 60.0 + num_h + cap_h + 40.0, right.w - 64.0, right.h - (strip_y + label_h + 60.0 + num_h + cap_h + 40.0) - 100.0);
        lines.push((Frame::new(32.0, chart.y + chart.h + cap_h + 24.0, right.w - 64.0, cap_h), role_style(kit, TypeRole::Caption, None, ink2, Align::Left), "Temperature and the chance of rain, the next 36 hours".to_owned()));
        // the axis: the hours a third of the way and two thirds of the way along, and the ends
        let n = hours.len().max(2);
        for i in [0usize, 11, 23, n - 1] {
            if let Some(h) = hours.get(i) {
                let x = chart.x + chart.w * i as f32 / (n - 1) as f32;
                let (align, fx) = if i == 0 { (Align::Left, x) } else if i == n - 1 { (Align::Right, x - 160.0) } else { (Align::Centre, x - 80.0) };
                let text = if i == 0 { "Now".to_owned() } else { hour_label(h.get("time").and_then(Value::as_str).unwrap_or(""), d.twelve) };
                lines.push((Frame::new(fx, chart.y + chart.h + 10.0, 160.0, cap_h), role_style(kit, TypeRole::Caption, None, ink3, align), text));
            }
        }
        made.cards.push(self.card(kit, right, z, lines)?);
        // pictures: each hour's icon and the chart
        let light = kit.mode == Mode::Light;
        let mut img = kit.ui.scene.image(right.w as u32, right.h as u32, z + 2, HIDDEN)?;
        img.clear();
        {
            let (w, h) = (img.width(), img.height());
            let mut canvas = Canvas { px: img.pixels_mut(), w, h };
            let side = 40u32;
            for (i, hh) in shown.iter().enumerate() {
                let code = hh.get("code").and_then(Value::as_i64).unwrap_or(0);
                let day = hh.get("isDay").and_then(Value::as_bool).unwrap_or(true);
                if let Some(p) = pictures.get(&format!("wxbig-{}", kind_of(code, day))) {
                    let x = 32.0 + i as f32 * col_w + (col_w - side as f32) / 2.0;
                    canvas.picture(p, x.round() as i32, (strip_y + label_h + 10.0).round() as i32, side, side);
                }
            }
            chart_paint(&mut canvas, chart, &hours, kit.ui.look.accent(), ink2, if light { 0.35 } else { 0.5 });
        }
        img.damage(None);
        img.set(Props::at(right.x, right.y));
        made.pictures.push(img);
        Ok(())
    }

    // ---- the week
    fn page_week(&self, kit: &mut Kit, d: &Data, pictures: &mut Pictures, made: &mut Made) -> std::io::Result<()> {
        let z = self.z + 4;
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let card = Frame::new(MARGIN, TOP, 1792.0, BOTTOM - TOP);
        let days: Vec<Value> = d.doc.pointer("/display/days").and_then(Value::as_array).cloned().unwrap_or_default();
        let more: Vec<Value> = d.more.get("daily").and_then(Value::as_array).cloned().unwrap_or_default();
        let rows = days.len().clamp(1, 7);
        let row_h = ((card.h - 40.0) / rows as f32).floor().min(120.0);
        let (title_h, body_h) = (kit.line(TypeRole::Title), kit.line(TypeRole::Body));
        let num = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).and_then(|t| t.trim_end_matches('°').parse::<f32>().ok());
        let (mut lo_min, mut hi_max) = (f32::MAX, f32::MIN);
        for dv in &days {
            if let (Some(lo), Some(hi)) = (num(dv, "lo"), num(dv, "hi")) {
                lo_min = lo_min.min(lo);
                hi_max = hi_max.max(hi);
            }
        }
        let (bar_x, bar_w) = (820.0, 560.0);
        let mut lines: Vec<Line> = Vec::new();
        for (i, dv) in days.iter().enumerate().take(rows) {
            let y = 20.0 + i as f32 * row_h;
            let text = |k: &str| dv.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
            lines.push((Frame::new(48.0, y, 240.0, row_h), role_style(kit, TypeRole::Title, if i == 0 { Some(Weight::SemiBold) } else { None }, ink, Align::Left), text("day")));
            let rain = more.get(i).and_then(|m| m.get("rainPct")).and_then(Value::as_f64).filter(|p| *p >= 10.0).map(|p| format!("{}%", p.round() as i64)).unwrap_or_default();
            lines.push((Frame::new(430.0, y, 100.0, row_h), role_style(kit, TypeRole::Body, None, ink2, Align::Right), rain));
            lines.push((Frame::new(bar_x - 110.0, y, 90.0, row_h), role_style(kit, TypeRole::Title, None, ink2, Align::Right), text("lo")));
            lines.push((Frame::new(bar_x + bar_w + 20.0, y, 100.0, row_h), role_style(kit, TypeRole::Title, Some(Weight::SemiBold), ink, Align::Left), text("hi")));
            if let Some(m) = more.get(i) {
                let rise = m.get("sunrise").and_then(Value::as_str).map(|t| clock_label(t, d.twelve)).unwrap_or_default();
                let set = m.get("sunset").and_then(Value::as_str).map(|t| clock_label(t, d.twelve)).unwrap_or_default();
                if !rise.is_empty() {
                    lines.push((Frame::new(1500.0, y, 240.0, row_h), role_style(kit, TypeRole::Body, None, ink2, Align::Left), format!("{rise}  {set}")));
                }
            }
        }
        let _ = (title_h, body_h);
        made.cards.push(self.card(kit, card, z, lines)?);
        let mut img = kit.ui.scene.image(card.w as u32, card.h as u32, z + 2, HIDDEN)?;
        img.clear();
        {
            let (w, h) = (img.width(), img.height());
            let accent = kit.ui.look.accent();
            let track = faded(ink, 0.12);
            let mut canvas = Canvas { px: img.pixels_mut(), w, h };
            for (i, dv) in days.iter().enumerate().take(rows) {
                let y = 20.0 + i as f32 * row_h;
                let k = dv.get("kind").and_then(Value::as_str).unwrap_or("");
                if let Some(p) = pictures.get(&format!("wxbig-{k}")) {
                    canvas.picture(p, 300, (y + (row_h - 64.0) / 2.0).round() as i32, 64, 64);
                }
                let by = y + (row_h - 12.0) / 2.0;
                canvas.rounded(Frame::new(bar_x, by, bar_w, 12.0), 6.0, track);
                if let (Some(lo), Some(hi)) = (num(dv, "lo"), num(dv, "hi"))
                    && hi_max > lo_min
                {
                    let (f0, f1) = ((lo - lo_min) / (hi_max - lo_min), (hi - lo_min) / (hi_max - lo_min));
                    canvas.rounded(Frame::new(bar_x + bar_w * f0, by, (bar_w * (f1 - f0)).max(12.0), 12.0), 6.0, accent);
                }
                if i + 1 < rows {
                    canvas.rounded(Frame::new(48.0, y + row_h - 1.0, card.w - 96.0, 1.0), 0.0, faded(ink, 0.10));
                }
            }
        }
        img.damage(None);
        img.set(Props::at(card.x, card.y));
        made.pictures.push(img);
        Ok(())
    }

    // ---- the details and the alerts
    fn page_details(&self, kit: &mut Kit, d: &Data, made: &mut Made) -> std::io::Result<()> {
        let z = self.z + 4;
        let (ink, ink2) = (kit.ui.look.ink(), kit.ui.look.ink2());
        let warn = kit.ui.look.warning();
        let alerts: Vec<Value> = d.more.get("alerts").and_then(Value::as_array).cloned().unwrap_or_default();
        let (cap_h, body_h, title_h) = (kit.line(TypeRole::Caption), kit.line(TypeRole::Body), kit.line(TypeRole::Title));
        let k = kit.text_scale.max(1.0);
        let mut top = TOP;
        if !alerts.is_empty() {
            let shown = alerts.len().min(2);
            let h = 28.0 + shown as f32 * (body_h + cap_h + 14.0) + 14.0;
            let mut lines: Vec<Line> = Vec::new();
            for (i, a) in alerts.iter().take(shown).enumerate() {
                let y = 22.0 + i as f32 * (body_h + cap_h + 14.0);
                let text = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
                lines.push((Frame::new(40.0, y, 1700.0, body_h), role_style(kit, TypeRole::Body, Some(Weight::SemiBold), warn, Align::Left), text("event")));
                let ends = a.get("ends").and_then(Value::as_str).map(|e| format!("  \u{b7}  until {}", clock_label(e, d.twelve))).unwrap_or_default();
                lines.push((Frame::new(40.0, y + body_h, 1700.0, cap_h), role_style(kit, TypeRole::Caption, None, ink2, Align::Left), format!("{}{ends}", text("headline"))));
            }
            if alerts.len() > shown {
                lines.push((Frame::new(1200.0, 22.0, 540.0, cap_h), role_style(kit, TypeRole::Caption, None, ink2, Align::Right), format!("and {} more", alerts.len() - shown)));
            }
            made.cards.push(self.card(kit, Frame::new(MARGIN, top, 1792.0, h), z, lines)?);
            top += h + 20.0;
        }
        // the tiles: two rows of four
        let tile_h = ((BOTTOM - top - 20.0) / 2.0).floor();
        let tile_w = ((1792.0f32 - 3.0 * 20.0) / 4.0).floor();
        let metric = d.fahrenheit;
        let w_in = |v: f64| if metric { format!("{:.2} in", v) } else { format!("{:.0} mm", v * 25.4) };
        let mut tiles: Vec<(String, String, String)> = Vec::new();
        let feels = d.num("/current/feelsLike").map(|v| d.deg(v)).unwrap_or_default();
        tiles.push(("FEELS LIKE".into(), feels, format!("Humidity {}%", d.num("/current/humidity").map(|v| v.round() as i64).unwrap_or(0))));
        let wind = d.num("/current/windMph").map(|v| format!("{} mph", v.round() as i64)).unwrap_or_default();
        let gust = d.more_num("/now/gustMph").map(|g| format!("Gusts {} mph", g.round() as i64)).unwrap_or_default();
        let from = d.more_num("/now/windDeg").map(|dg| format!(" \u{b7} from the {}", compass(dg))).unwrap_or_default();
        tiles.push(("WIND".into(), wind, format!("{gust}{from}").trim_start_matches(" \u{b7} ").to_owned()));
        let uv = d.more_num("/now/uvIndex");
        tiles.push(("UV INDEX".into(), uv.map(|u| format!("{}", u.round() as i64)).unwrap_or_else(|| "\u{2013}".into()), uv.map(|u| uv_word(u).to_owned()).unwrap_or_default()));
        let rain = d.more.pointer("/daily/0/rainPct").and_then(Value::as_f64);
        let amount = d.more.pointer("/daily/0/rainIn").and_then(Value::as_f64).map(&w_in).unwrap_or_default();
        tiles.push(("RAIN TODAY".into(), rain.map(|p| format!("{}%", p.round() as i64)).unwrap_or_else(|| "\u{2013}".into()), if amount.is_empty() { String::new() } else { format!("{amount} expected") }));
        let rise = d.more.pointer("/daily/0/sunrise").and_then(Value::as_str).map(|t| clock_label(t, d.twelve)).unwrap_or_else(|| "\u{2013}".into());
        let set = d.more.pointer("/daily/0/sunset").and_then(Value::as_str).map(|t| clock_label(t, d.twelve)).unwrap_or_default();
        tiles.push(("SUNRISE".into(), rise, String::new()));
        tiles.push(("SUNSET".into(), set, String::new()));
        let pressure = d.more_num("/now/pressureHpa").map(|p| if metric { format!("{:.2} inHg", p * 0.029_53) } else { format!("{} hPa", p.round() as i64) }).unwrap_or_else(|| "\u{2013}".into());
        tiles.push(("PRESSURE".into(), pressure, String::new()));
        let vis = d.more_num("/now/visibilityM").map(|m| if metric { format!("{} mi", (m / 1609.34).round() as i64) } else { format!("{} km", (m / 1000.0).round() as i64) }).unwrap_or_else(|| "\u{2013}".into());
        let cloud = d.more_num("/now/cloudPct").map(|c| format!("Cloud cover {}%", c.round() as i64)).unwrap_or_default();
        tiles.push(("VISIBILITY".into(), vis, cloud));
        let mut big = kit.ty(TypeRole::Display);
        big.px = (64.0 * k.min(1.3)).round() as u32;
        big.weight = Weight::SemiBold;
        let big_h = ui::components::line_height(big);
        for (i, (label, value, sub)) in tiles.into_iter().enumerate() {
            let (col, row) = (i % 4, i / 4);
            let rect = Frame::new(MARGIN + col as f32 * (tile_w + 20.0), top + row as f32 * (tile_h + 20.0), tile_w, tile_h);
            let lines = vec![
                (Frame::new(32.0, 26.0, tile_w - 64.0, cap_h), role_style(kit, TypeRole::Caption, Some(Weight::SemiBold), ink2, Align::Left), label),
                (Frame::new(32.0, 26.0 + cap_h + 12.0, tile_w - 64.0, big_h), style(big, ink, Align::Left), value),
                (Frame::new(32.0, tile_h - 24.0 - body_h.max(title_h * 0.8), tile_w - 64.0, body_h), role_style(kit, TypeRole::Body, None, ink2, Align::Left), sub),
            ];
            made.cards.push(self.card(kit, rect, z, lines)?);
        }
        Ok(())
    }
}

/// The curve of the temperature (a thick line of discs along the hours) and the bars of the chance of rain, in `chart` of the card's picture.
fn chart_paint(canvas: &mut Canvas, chart: Frame, hours: &[Value], accent: Rgba, ink: Rgba, bar_alpha: f32) {
    if hours.len() < 2 {
        return;
    }
    let temps: Vec<f32> = hours.iter().map(|h| h.get("temp").and_then(Value::as_f64).unwrap_or(0.0) as f32).collect();
    let (lo, hi) = temps.iter().fold((f32::MAX, f32::MIN), |(l, h), t| (l.min(*t), h.max(*t)));
    let span = (hi - lo).max(4.0);
    let n = hours.len();
    let x_of = |i: f32| chart.x + chart.w * i / (n - 1) as f32;
    let y_of = |t: f32| chart.y + chart.h * 0.08 + (chart.h * 0.62) * (1.0 - (t - lo) / span);
    // the chance of rain: bars from the chart's floor
    for (i, h) in hours.iter().enumerate() {
        let p = h.get("rainPct").and_then(Value::as_f64).unwrap_or(0.0) as f32 / 100.0;
        if p >= 0.1 {
            let bh = (chart.h * 0.28 * p).max(3.0);
            let w = (chart.w / n as f32 * 0.55).max(3.0);
            canvas.rounded(Frame::new(x_of(i as f32) - w / 2.0, chart.y + chart.h - bh, w, bh), (w / 2.0).min(4.0), faded(accent, bar_alpha));
        }
    }
    canvas.rounded(Frame::new(chart.x, chart.y + chart.h + 1.0, chart.w, 1.0), 0.0, faded(ink, 0.18));
    // the curve, smooth between the hours (a cosine ease), one disc a pixel and a half
    let steps = (chart.w * 1.4) as usize;
    for s in 0..=steps {
        let f = s as f32 / steps as f32 * (n - 1) as f32;
        let i = (f.floor() as usize).min(n - 2);
        let u = f - i as f32;
        let e = (1.0 - (std::f32::consts::PI * u).cos()) / 2.0;
        let t = temps[i] + (temps[i + 1] - temps[i]) * e;
        canvas.disc(x_of(f), y_of(t), 3.0, accent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_keys_walk_the_pages_and_back_closes() {
        let mut v = WeatherView::default();
        assert_eq!(v.key("up", true, false), Done::Nothing, "the first page has nothing above");
        assert_eq!(v.key("down", false, true), Done::Redraw);
        assert_eq!(v.page, Page::Week);
        assert_eq!(v.key("down", false, true), Done::Redraw);
        assert_eq!(v.page, Page::Details);
        assert_eq!(v.key("down", false, true), Done::Nothing, "nor the last below");
        assert_eq!(v.key("up", true, false), Done::Redraw);
        assert_eq!(v.key("back", false, false), Done::Close);
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
    }

    #[test]
    fn the_icon_of_a_code_is_the_helpers() {
        for (code, day, kind) in [(0, true, "sun"), (0, false, "moon"), (2, false, "partly-night"), (3, true, "cloud"), (45, true, "fog"), (53, true, "drizzle"), (63, true, "rain"), (73, true, "snow"), (95, true, "storm")] {
            assert_eq!(kind_of(code, day), kind, "{code}");
        }
    }
}
