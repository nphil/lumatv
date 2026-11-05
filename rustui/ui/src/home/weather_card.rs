//! The weather card (`widgets/WeatherWidget.qml`, redesigned for the room it has): the sky of the day drawn behind the reading, and under it what
//! to know from the couch at a glance: the place, the temperature large, the condition, how it feels with the day's high and low, one line that
//! says what is next (rain from four, a watch in force, the sunset), the next hours with their weather and their chance of rain, and, where the
//! card is tall or wide enough, the week as rows with their range bars.
//!
//! The sky is the card's picture ([`super::weather_art`]): a tint over the glass from the top (or, beside a week, from the left), the sprites the
//! compositor moves, and a hairline under the reading where the rain lands. Now and then (a storm's strike, a fall of snow) the weather takes over the
//! whole card for a few seconds. The card's words are one block (one layer, glyph quads) and its pictures one image layer painted on the CPU (icons,
//! bars), so a card costs the compositor a handful of layers besides the sky's.
//!
//! The plan ([`plan`]) is pure: from the box it has (in the text size's reduction, as the clock does it) and from what data there is (the hours or
//! not, a temperature of three characters or two), it decides which of the parts there is room for and where each goes: the reading, as large as
//! the room allows, with the next thing and the hours under it, or beside it when the card is wide; the picture of the sky goes where the reading
//! is not.

use super::weather_art::{Card, Cond, Live, Stage};
use super::weather_sprites::Pal;
use super::weather_view::{SmartTone, hour_label, kind_of, legible, smart_line};
use super::widgets::{Canvas, PAD_SIDE, PAD_TOP, Pictures, Words, art, warm};
use crate::kit::Kit;
use crate::kit::button::faded;
use compositor_protocol::{Fill, Props, Rgba, Shape};
use lumatv_tokens::{Radius, TypeRole};
use serde_json::Value;
use ui::components::text::ellipsise;
use ui::components::{Align, Style};
use ui::geometry::Frame;
use ui::scene::{ImageLayer, ShapeLayer};
use ui::text::Weight;

const HIDDEN: Props = Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) };

/// What data the card has to show, which decides its plan (the card is planned again when it changes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Data {
    /// The next hours are there.
    pub hours: bool,
    /// Some of the first of them have a chance of rain worth a bar.
    pub bars: bool,
    /// The temperature is of three characters or more with its degree ("100\u{b0}", "-12\u{b0}"), so that it is wider than "61\u{b0}".
    pub wide_temp: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ink {
    Primary,
    Secondary,
    Accent,
    Warning,
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    f: Frame,
    px: f32,
    weight: Weight,
    align: Align,
}

fn slot(x: f32, y: f32, w: f32, h: f32, px: f32, weight: Weight, align: Align) -> Slot {
    Slot { f: Frame::new(x, y, w, h), px, weight, align }
}

#[derive(Clone, Copy, Debug)]
struct Hours {
    x: f32,
    y: f32,
    col_w: f32,
    n: usize,
    icon: f32,
    /// The rows of a column, from `y`: the icon, the temperature and (when there are bars) the bar's top and its height; the size of the temperature.
    icon_y: f32,
    temp_y: f32,
    temp_px: f32,
    bar: Option<(f32, f32)>,
    /// How tall the whole strip is.
    height: f32,
}

#[derive(Clone, Copy, Debug)]
struct Days {
    x: f32,
    y: f32,
    w: f32,
    row_h: f32,
    rows: usize,
    day_w: f32,
    icon: f32,
    num_w: f32,
    /// A range bar between the low and the high: where the week is narrow there is none, and the two numbers stand at the end.
    bar: bool,
    /// The low is shown (the very narrow week has the high alone, and no picture of the day either).
    lo: bool,
}

/// The box the sky's picture has to itself (in the content box's coordinates, plan units): right of the reading's words and above the cond line.
#[derive(Clone, Copy, Debug)]
struct Art {
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
}

/// Where everything goes, in the content box at the text size's reduction (the caller scales it up by the text size).
#[derive(Clone, Debug)]
struct Plan {
    w: f32,
    /// The reading has the left of the card and the forecast the right, not the forecast under the reading.
    split: bool,
    hero_w: f32,
    hero_bottom: f32,
    place: Option<Slot>,
    temp: Slot,
    cond: Option<Slot>,
    feels: Option<Slot>,
    smart: Option<Slot>,
    hours: Option<Hours>,
    days: Option<Days>,
    art: Art,
}

/// The side of the largest picture of the sky, in pixels.
const ART_MAX: f32 = 260.0;
const PLACE_H: f32 = 32.0;
const COND_H: f32 = 36.0;
const FEELS_H: f32 = 32.0;
const SMART_H: f32 = 34.0;
const SMART_GAP: f32 = 10.0;
/// Between the reading and what is under it, and between the strips of the forecast.
const GAP: f32 = 14.0;
/// Between the reading's column and the forecast's.
const COL_GAP: f32 = 44.0;
const HOURS_COMPACT_H: f32 = 92.0;
const HOURS_H: f32 = 106.0;
const HOURS_BARS_H: f32 = 146.0;
const DAY_H: f32 = 46.0;

/// A size of the reading: the temperature's, and which of the place, the condition and how it feels go with it.
#[derive(Clone, Copy, Debug)]
struct Tier {
    temp: f32,
    place: bool,
    cond: bool,
    feels: bool,
}

impl Tier {
    const fn new(temp: f32, place: bool, feels: bool) -> Tier {
        Tier { temp, place, cond: true, feels }
    }

    fn height(&self) -> f32 {
        self.temp + 2.0 + if self.place { PLACE_H } else { 0.0 } + if self.cond { COND_H } else { 0.0 } + if self.feels { FEELS_H } else { 0.0 }
    }
}

/// The sizes of the reading, the largest first.
const TIERS: [Tier; 13] = [
    Tier::new(112.0, true, true),
    Tier::new(96.0, true, true),
    Tier::new(96.0, true, false),
    Tier::new(84.0, true, false),
    Tier::new(72.0, true, false),
    Tier::new(72.0, false, false),
    Tier::new(64.0, false, false),
    Tier::new(56.0, false, false),
    Tier::new(48.0, false, false),
    Tier::new(40.0, false, false),
    Tier::new(36.0, false, false),
    Tier { temp: 44.0, place: false, cond: false, feels: false },
    Tier { temp: 36.0, place: false, cond: false, feels: false },
];

/// The largest of the first `last + 1` sizes that is no taller than `avail` and has a temperature no wider than `max_temp` allows; the last of them,
/// made to fit, when none does.
fn pick(avail: f32, max_temp: f32, last: usize) -> Tier {
    TIERS[..=last].iter().copied().find(|t| t.height() <= avail && t.temp <= max_temp).unwrap_or(Tier { temp: (avail - 2.0).clamp(24.0, 36.0).min(max_temp).floor(), place: false, cond: false, feels: false })
}

/// How many columns of hours a strip `bw` wide has.
fn columns(bw: f32) -> usize {
    ((bw / 70.0).floor() as usize).min(8)
}

/// The strip of the next hours in `height` (compact, plain, or tall enough for the bars of the rain, or for larger pictures when it is dry).
fn hours_strip(x: f32, y: f32, bw: f32, height: f32, data: Data) -> Hours {
    let n = columns(bw);
    let col_w = (bw / n as f32).floor();
    if height >= HOURS_BARS_H {
        if data.bars {
            Hours { x, y, col_w, n, icon: 40.0, icon_y: y + 30.0, temp_y: y + 72.0, temp_px: 26.0, bar: Some((y + 112.0, 34.0)), height: HOURS_BARS_H }
        } else {
            Hours { x, y, col_w, n, icon: 48.0, icon_y: y + 34.0, temp_y: y + 90.0, temp_px: 30.0, bar: None, height: HOURS_BARS_H }
        }
    } else if height >= HOURS_H {
        Hours { x, y, col_w, n, icon: 40.0, icon_y: y + 30.0, temp_y: y + 72.0, temp_px: 26.0, bar: None, height: HOURS_H }
    } else {
        Hours { x, y, col_w, n, icon: 30.0, icon_y: y + 26.0, temp_y: y + 58.0, temp_px: 26.0, bar: None, height: HOURS_COMPACT_H }
    }
}

/// The rows of the week in `rows` rows from `y`, `bw` wide: with the range bars where the week is wide enough to give them room, else the low and the high at
/// the end, else (a very narrow card) the high alone.
fn week(x: f32, y: f32, bw: f32, rows: usize) -> Days {
    if bw >= 330.0 {
        Days { x, y, w: bw, row_h: DAY_H, rows, day_w: 76.0, icon: 32.0, num_w: 52.0, bar: true, lo: true }
    } else if bw >= 206.0 {
        Days { x, y, w: bw, row_h: DAY_H, rows, day_w: 80.0, icon: 30.0, num_w: 46.0, bar: false, lo: true }
    } else {
        Days { x, y, w: bw, row_h: DAY_H, rows, day_w: bw - 46.0, icon: 0.0, num_w: 46.0, bar: false, lo: false }
    }
}

/// Which parts there is room for in a box of `w` by `h` (logical pixels, in the reduction `k` the text is scaled up by) and where each goes.
fn plan(w: f32, h: f32, k: f32, data: Data) -> Plan {
    let em = if data.wide_temp { 2.3 } else { 1.7 };
    // beside each other when the card is wide and not tall (a tall one has the forecast under the reading, where the width is for the hours and the week)
    let split = (w >= 620.0 && h < 360.0) || (w >= 500.0 && (90.0..300.0).contains(&h));
    let hero_w = if split { (w * 0.42).round().clamp(250.0, 430.0) } else { w };
    // the temperature may be as large as leaves the picture of the sky room beside it: 96 pixels on the screen, and less at a larger text size, which takes
    // the room (a card of two columns at the largest size has 218 pixels to share)
    let art_w = 96.0 / k.sqrt() / k;
    let max_temp = ((hero_w - 12.0 - art_w) / em).floor().max(24.0);
    let (x0, bw) = if split { (hero_w + COL_GAP, w - hero_w - COL_GAP) } else { (0.0, w) };
    let can_hours = data.hours && columns(bw) >= 3;
    // the next thing says itself in no fewer pixels than this, in its shortest way
    let smart_ok = hero_w >= 150.0;
    let smart_h = if smart_ok { SMART_H } else { 0.0 };
    // what is between the next thing and the hours
    let smart_gap = if smart_ok { SMART_GAP } else { 0.0 };

    // ---- the reading's size, and what is to go under it (or beside it)
    let (tier, under, hours_h) = if split {
        // the next thing under the reading when there is room for the smallest reading with it; the hours and the week fill the other column
        let under = smart_ok && h >= TIERS[10].height() + GAP + SMART_H;
        let tier = pick(h - if under { SMART_H + GAP } else { 0.0 }, max_temp, 12);
        let hours_h = if !can_hours || h < HOURS_COMPACT_H { 0.0 } else if h >= HOURS_BARS_H { HOURS_BARS_H } else if h >= HOURS_H { HOURS_H } else { HOURS_COMPACT_H };
        (tier, under, hours_h)
    } else {
        // under the reading, the richest of: the next thing and the hours with their bars, with plain hours, the next thing alone, nothing; the reading
        // is the largest that leaves them room (and no smaller than each is worth). The bars of the rain (or larger pictures, when it is dry) are worth a
        // reading of the sizes that go with them; plain hours a reading of 72 and the place, or the largest a narrow card has.
        let plain_last = if max_temp >= 72.0 { 4 } else { 6 };
        let options: &[(f32, usize)] = if !can_hours {
            &[(0.0, 10)]
        } else if data.bars {
            &[(HOURS_BARS_H, 3), (HOURS_H, plain_last), (0.0, 10)]
        } else {
            &[(HOURS_BARS_H, 1), (HOURS_H, plain_last), (0.0, 10)]
        };
        let chosen = options.iter().find_map(|&(hours_h, last)| {
            let lower = smart_h + if hours_h > 0.0 { smart_gap + hours_h } else { 0.0 } + GAP;
            let avail = h - lower;
            TIERS[..=last].iter().copied().find(|t| t.height() <= avail && t.temp <= max_temp).map(|t| (t, true, hours_h))
        });
        chosen.unwrap_or_else(|| (pick(h, max_temp, 12), false, 0.0))
    };

    // ---- the reading
    let mut y = 0.0;
    let place = tier.place.then(|| {
        let s = slot(0.0, y, hero_w, 30.0, 24.0, Weight::SemiBold, Align::Left);
        y += PLACE_H;
        s
    });
    let temp = slot(0.0, y, hero_w, tier.temp, tier.temp, Weight::SemiBold, Align::Left);
    y += tier.temp + 2.0;
    let cond = tier.cond.then(|| {
        let s = slot(0.0, y, hero_w, COND_H, if tier.temp >= 96.0 { 30.0 } else { 26.0 }, Weight::Medium, Align::Left);
        y += COND_H;
        s
    });
    let feels = tier.feels.then(|| {
        let s = slot(0.0, y, hero_w, FEELS_H, 24.0, Weight::Medium, Align::Left);
        y += FEELS_H;
        s
    });
    let hero_bottom = y;
    // the sky's picture is the largest of at most `ART_MAX` pixels the room right of the temperature has, and stays beside it on a card wider than that
    let x0_art = tier.temp * em + 12.0;
    let art = Art {
        x0: x0_art,
        x1: (hero_w + (if split { -4.0 } else { 20.0 }) / k).min(x0_art + (0.8 * ART_MAX + 8.0) / k),
        y0: -(PAD_TOP - 14.0) / k,
        y1: if tier.place { PLACE_H } else { 0.0 } + tier.temp + 2.0 + if tier.cond { 6.0 } else { 0.0 },
    };

    // ---- the forecast: the next thing, the hours, the week
    let smart_px = |bw: f32| if bw < 230.0 { 21.0 } else { 26.0 };
    let mut smart = None;
    let mut hours = None;
    let mut days = None;
    if split {
        // the left column's room under the reading is shared out round the next thing
        let extra = if under { ((h - hero_bottom - GAP - SMART_H) * 0.5).clamp(0.0, 24.0).floor() } else { 0.0 };
        if under {
            smart = Some(slot(0.0, hero_bottom + GAP + extra, hero_w, SMART_H, smart_px(hero_w), Weight::SemiBold, Align::Left));
        }
        let mut y = 0.0;
        if hours_h > 0.0 {
            let strip = hours_strip(x0, y, bw, hours_h, data);
            y += strip.height + GAP;
            hours = Some(strip);
        } else if !under && smart_ok && h >= SMART_H && bw >= 150.0 {
            // no room for the next thing under the reading and no hours: it stands in the other column
            smart = Some(slot(x0, 0.0, bw, SMART_H, smart_px(bw), Weight::SemiBold, Align::Left));
            y += SMART_H + SMART_GAP;
        }
        let rows = (((h - y) / DAY_H).floor() as usize).min(7);
        if rows >= 2 {
            days = Some(week(x0, y, bw, rows));
        }
    } else if under {
        let lead = if smart_ok || hours_h > 0.0 { GAP } else { 0.0 };
        let base = hero_bottom + GAP + smart_h + if hours_h > 0.0 { smart_gap + hours_h } else { 0.0 };
        let after = h - base;
        let rows = if after >= lead + 2.0 * DAY_H { (((after - lead) / DAY_H).floor() as usize).min(7) } else { 0 };
        let used = if rows >= 2 { lead + rows as f32 * DAY_H } else { 0.0 };
        // what is left over is shared out between the reading and the rest, so that a card a little taller than the plan needs is not left with a blank foot
        let extra = ((after - used).max(0.0) * 0.5).clamp(0.0, 24.0).floor();
        let mut y = hero_bottom + GAP + extra;
        if smart_ok {
            // a card as narrow as the largest text size makes the default one says it a little smaller
            smart = Some(slot(0.0, y, w, SMART_H, smart_px(w), Weight::SemiBold, Align::Left));
            y += SMART_H + if hours_h > 0.0 { SMART_GAP } else { GAP };
        }
        if hours_h > 0.0 {
            let strip = hours_strip(0.0, y, w, hours_h, data);
            y += strip.height + GAP;
            hours = Some(strip);
        }
        if rows >= 2 {
            days = Some(week(0.0, y, w, rows));
        }
    }
    Plan { w, split, hero_w, hero_bottom, place, temp, cond, feels, smart, hours, days, art }
}

/// The text of a line, and whose ink it is in.
type Change = (usize, String, Ink);

/// Which lines of the block are which.
#[derive(Default)]
struct Lines {
    place: Option<usize>,
    temp: usize,
    cond: Option<usize>,
    feels: Option<usize>,
    smart: Option<usize>,
    /// Per hour: its time and its temperature.
    hours: Vec<(usize, usize)>,
    /// Per day: its name, its low and its high.
    days: Vec<(usize, Option<usize>, usize)>,
    styles: Vec<Style>,
}

pub struct Weather {
    pub block: Words,
    rows: ImageLayer,
    /// The content box, in pixels, and the text size.
    w: f32,
    h: f32,
    k: f32,
    plan: Plan,
    data: Data,
    lines: Lines,
    at: Props,
    shown: Option<String>,
    /// The sky: the picture of the condition (the sprites in the card's group), the tint behind it and the stage they are made for (box and hero, in pixels).
    art: Option<Live>,
    wash: Option<(ShapeLayer, Cond, bool)>,
    stage: Stage,
    art_waits: bool,
    z: i32,
}

/// The sky's box and its hero in the card's coordinates: the padding is the card's own, so the sky reaches the card's edges. The sky is the top of the
/// card (above the horizon under the reading), or, beside the forecast, the reading's side of it; `radius` is the card's corner radius, which a moment's veils take.
fn sky_box(plan: &Plan, k: f32, w_px: f32, h_px: f32, radius: f32) -> Stage {
    let (card_w, card_h) = (w_px + 2.0 * PAD_SIDE, h_px + 2.0 * PAD_TOP);
    // the forecast beside the reading is on the plain glass: the sky ends in a soft edge before it
    let sky_w = if plan.split { (PAD_SIDE + (plan.hero_w + COL_GAP - 8.0) * k).min(card_w) } else { card_w };
    let sky_h = (PAD_TOP + plan.hero_bottom * k + 12.0).round().min(card_h);
    // the hero is the largest picture the room beside the reading has, on the right of it and as high as the temperature
    let a = plan.art;
    let (x0, x1, y0, y1) = (PAD_SIDE + a.x0 * k, PAD_SIDE + a.x1 * k, PAD_TOP + a.y0 * k, PAD_TOP + a.y1 * k);
    let size = ((x1 - x0) / 0.8).min((y1 - y0) / 0.84).clamp(56.0, 260.0).round();
    let hero = ((x1 - size * 0.42).round(), ((y0 + y1) / 2.0).round());
    let fade = [14.0, if plan.split { 48.0 } else { 14.0 }, 10.0, 26.0];
    Stage { w: sky_w, h: sky_h, hero, size, ground: sky_h - 5.0, fade, shy: 0.5, card: Some(Card { w: card_w, h: card_h, radius }) }
}

/// The straight (not premultiplied) colour of `c`, which is how the card's canvas takes the colour of a shape it paints.
fn straight(c: Rgba) -> Rgba {
    if c.3 == 0 {
        return c;
    }
    let un = |v: u8| (f32::from(v) * 255.0 / f32::from(c.3)).round().min(255.0) as u8;
    Rgba(un(c.0), un(c.1), un(c.2), c.3)
}

/// The first of `texts` that fits `room` pixels, else the last, cut with an ellipsis.
fn fit(kit: &mut Kit, ty: ui::text::Type, texts: &[String], room: f32) -> String {
    for t in texts {
        if kit.ui.fonts.measure(t, ty).width <= room {
            return t.clone();
        }
    }
    ellipsise(&mut kit.ui.fonts, texts.last().map_or("", String::as_str), ty, room)
}

/// A condition's words and, where the room is too small for them, the shorter ways of saying it (a card two columns wide at the largest text size has
/// room for a word of seven letters).
fn cond_texts(cond: &str) -> Vec<String> {
    let short = match cond {
        "Thunderstorm" => Some("Storm"),
        "Partly cloudy" => Some("Partly"),
        "Mostly clear" => Some("Clear"),
        _ => None,
    };
    std::iter::once(cond).chain(short).map(str::to_owned).collect()
}

impl Weather {
    pub fn new(kit: &mut Kit, w: f32, h: f32, z: i32, _theme: &Value) -> std::io::Result<Self> {
        let k = kit.text_scale.max(1.0);
        let data = Data::default();
        let p = plan(w / k, h / k, k, data);
        let (block, rows, lines) = Self::make(kit, &p, k, w, h, z)?;
        let stage = sky_box(&p, k, w, h, kit.tokens.radius(Radius::Card));
        Ok(Self { block, rows, w, h, k, plan: p, data, lines, at: HIDDEN, shown: None, art: None, wash: None, stage, art_waits: false, z })
    }

    /// The words and the picture of a plan.
    fn make(kit: &mut Kit, p: &Plan, k: f32, w: f32, h: f32, z: i32) -> std::io::Result<(Words, ImageLayer, Lines)> {
        let ink = kit.ui.look.ink();
        let types: std::cell::RefCell<Vec<ui::text::Type>> = std::cell::RefCell::new(Vec::new());
        let sized = |kit: &Kit, s: &Slot| -> Style {
            let mut ty = kit.ty(TypeRole::Label);
            ty.px = (s.px * k).round().max(8.0) as u32;
            ty.weight = s.weight;
            types.borrow_mut().push(ty);
            ui::components::style(ty, ink, s.align)
        };
        let mut lines: Vec<(Frame, Style, String)> = Vec::new();
        let mut map = Lines::default();
        let push = |kit: &Kit, s: &Slot, lines: &mut Vec<(Frame, Style, String)>| -> usize {
            let st = sized(kit, s);
            lines.push((Frame::new((s.f.x * k).round(), (s.f.y * k).round(), (s.f.w * k).round(), (s.f.h * k).round()), st, String::new()));
            lines.len() - 1
        };
        if let Some(s) = &p.place {
            map.place = Some(push(kit, s, &mut lines));
        }
        map.temp = push(kit, &p.temp, &mut lines);
        if let Some(s) = &p.cond {
            map.cond = Some(push(kit, s, &mut lines));
        }
        if let Some(s) = &p.feels {
            map.feels = Some(push(kit, s, &mut lines));
        }
        if let Some(s) = &p.smart {
            map.smart = Some(push(kit, s, &mut lines));
        }
        if let Some(hr) = &p.hours {
            for i in 0..hr.n {
                let x = hr.x + i as f32 * hr.col_w;
                let t = push(kit, &slot(x, hr.y, hr.col_w, 28.0, 24.0, Weight::Medium, Align::Centre), &mut lines);
                let v = push(kit, &slot(x, hr.temp_y, hr.col_w, hr.temp_px * 1.25, hr.temp_px, Weight::SemiBold, Align::Centre), &mut lines);
                map.hours.push((t, v));
            }
        }
        if let Some(d) = &p.days {
            for i in 0..d.rows {
                let y = d.y + i as f32 * d.row_h;
                let name = push(kit, &slot(d.x, y, d.day_w, d.row_h, 24.0, Weight::Medium, Align::Left), &mut lines);
                // with the range bar the low is left of it, beside the picture; without it the two numbers stand together at the end
                let lo_x = if d.bar { d.x + d.day_w + d.icon + 10.0 } else { d.x + d.w - 2.0 * d.num_w };
                let lo = d.lo.then(|| push(kit, &slot(lo_x, y, d.num_w, d.row_h, 24.0, Weight::Medium, Align::Right), &mut lines));
                let hi = push(kit, &slot(d.x + d.w - d.num_w, y, d.num_w, d.row_h, 24.0, Weight::Medium, Align::Right), &mut lines);
                map.days.push((name, lo, hi));
            }
        }
        warm(kit, types.into_inner());
        map.styles = lines.iter().map(|l| l.1).collect();
        let block = Words::new(kit, Frame::new(0.0, 0.0, w, h), z + 3, lines)?;
        let rows = art(kit, Frame::new(0.0, 0.0, w, h), z + 2)?;
        Ok((block, rows, map))
    }

    /// The card is planned again for what data there is: its words and picture are made anew (the caller is inside the card's `adopting`).
    fn replan(&mut self, kit: &mut Kit, data: Data) -> std::io::Result<()> {
        let p = plan(self.w / self.k, self.h / self.k, self.k, data);
        let (block, rows, lines) = Self::make(kit, &p, self.k, self.w, self.h, self.z)?;
        self.block = block;
        self.rows = rows;
        self.lines = lines;
        self.stage = sky_box(&p, self.k, self.w, self.h, kit.tokens.radius(Radius::Card));
        self.plan = p;
        self.data = data;
        self.shown = None;
        // the sky is made for the box the plan gives it: again
        self.art = None;
        self.wash = None;
        self.block.set_props(self.at);
        self.rows.set(self.at);
        Ok(())
    }

    pub fn show(&mut self, at: Props) {
        self.at = at;
        self.block.set_props(at);
        self.rows.set(at);
        if let Some((w, _, _)) = &self.wash {
            w.set(Props { x: at.x - PAD_SIDE, y: at.y - PAD_TOP, ..at });
        }
        if let Some(a) = &mut self.art {
            a.place((at.x - PAD_SIDE, at.y - PAD_TOP));
        }
    }

    pub fn art_waits(&self) -> bool {
        self.art_waits
    }

    /// The sky is made and nothing holds it back, so a step of it restates its paths and needs nothing of the card.
    pub fn art_steady(&self) -> bool {
        self.art.is_some() && !self.art_waits
    }

    /// Keeps the picture's motion going (`run`) or held where it is, for the tick `now`; the tick the next paths are due at, while it moves.
    pub fn animate(&mut self, now: u64, run: bool) -> Option<u64> {
        let a = self.art.as_mut()?;
        if !run {
            a.pause(now);
            return None;
        }
        Some(a.step(now))
    }

    /// The sky: its sprites and the tint behind them are made when the condition (or the look, or the box) changes. The caller is inside the card's
    /// `adopting`, so that the layers hang under the card's group.
    pub fn set_art(&mut self, kit: &mut Kit, topic: &Value) {
        let light = kit.mode == ui::lucent::palette::Mode::Light;
        let cond = Cond::from_doc(topic.pointer("/doc").unwrap_or(&Value::Null));
        // a sky that is made for this condition waits for nothing: the clock's new digits unsettle the atlas every second, and a sky that said it
        // waited then had its next step drawn by a whole apply of Home (`HomeSurface::tick`)
        if cond.is_some_and(|c| self.art.as_ref().is_some_and(|a| a.is(c, self.stage, light))) {
            self.art_waits = false;
            return;
        }
        // the words first: the sprites' pixels are a few hundred KiB the engine takes in a frame of their own, and the card's glyphs are
        // not to wait behind them (the picture comes on the first apply after the atlas has settled)
        if kit.ui.text_atlas().is_some_and(|a| !a.is_settled()) {
            self.art_waits = true;
            return;
        }
        self.art_waits = false;
        match cond {
            Some(cond) => {
                let origin = (self.at.x - PAD_SIDE, self.at.y - PAD_TOP);
                // the tint behind the sprites, under them: from the colour of the sky, in the palette's own, to nothing. Over the reading's top it is the card's
                // top corners' shape; beside the forecast it fades from the left to nothing before the forecast's side, so that it has no edge on the glass.
                if !self.wash.as_ref().is_some_and(|(_, c, l)| *c == cond && *l == light) {
                    let (rgb, a) = cond.wash(&Pal::of(&kit.ui.look.theme.roles, light), light);
                    let top = Rgba((rgb[0] * a * 255.0).round() as u8, (rgb[1] * a * 255.0).round() as u8, (rgb[2] * a * 255.0).round() as u8, (a * 255.0).round() as u8);
                    let (angle, w, h) = if self.plan.split { (0.0, (self.stage.w + 160.0).min(self.stage.card.map_or(f32::MAX, |c| c.w)), self.stage.card.map_or(self.stage.h, |c| c.h)) } else { (90.0, self.stage.w, self.stage.h + 20.0) };
                    let shape = Shape { corner_radius: kit.tokens.radius(Radius::Card), fill: Fill::Linear { from: top, to: Rgba(0, 0, 0, 0), angle_deg: angle }, border_width: 0.0, border: Rgba(0, 0, 0, 0) };
                    let props = Props { x: self.at.x - PAD_SIDE, y: self.at.y - PAD_TOP, ..self.at };
                    self.wash = Some((kit.ui.scene.shape(w as u32, h as u32, self.z, shape, props), cond, light));
                }
                self.art = Live::new(kit, cond, self.stage, light, origin, self.z + 1).ok();
            }
            None => {
                self.art = None;
                self.wash = None;
            }
        }
    }

    /// The helper's reading (`topic.doc`) and the details (`more.doc`, the hours and the alerts), drawn when either changed. The caller is inside the
    /// card's `adopting` (a change of what data there is plans the card again).
    pub fn set(&mut self, kit: &mut Kit, pictures: &mut Pictures, topic: &Value, more: &Value, twelve: bool) {
        let key = format!("{}|{}|{twelve}|{}", topic.pointer("/doc/fetchedAt").map(Value::to_string).unwrap_or_default(), more.pointer("/doc/fetchedAt").map(Value::to_string).unwrap_or_default(), self.k);
        if self.shown.as_deref() == Some(key.as_str()) {
            return;
        }
        let hours: Vec<Value> = more.pointer("/doc/hourly").and_then(Value::as_array).cloned().unwrap_or_default();
        let temp_text = topic.pointer("/doc/display/temp").and_then(Value::as_str).unwrap_or("");
        let data = Data {
            hours: hours.len() >= 4,
            bars: hours.iter().take(8).any(|h| h.get("rainPct").and_then(Value::as_f64).is_some_and(|p| p >= 15.0)),
            wide_temp: temp_text.chars().count() >= 4,
        };
        if data != self.data && self.replan(kit, data).is_err() {
            return;
        }
        self.shown = Some(key);
        let doc = topic.pointer("/doc").unwrap_or(&Value::Null);
        let d = doc.get("display").cloned().unwrap_or(Value::Null);
        let s = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        let k = self.k;
        let mut changes: Vec<Change> = Vec::new();
        // the reading: no reading yet means the helper is starting again or not up, which the card says instead of staying empty
        let place = if d.is_null() { "Reconnecting\u{2026}".to_owned() } else { s("place") };
        let hero_room = self.plan.hero_w * k;
        if let Some(i) = self.lines.place {
            let ty = self.lines.styles[i].ty;
            changes.push((i, ellipsise(&mut kit.ui.fonts, &place, ty, hero_room), Ink::Primary));
        }
        changes.push((self.lines.temp, s("temp"), Ink::Primary));
        if let Some(i) = self.lines.cond {
            let ty = self.lines.styles[i].ty;
            changes.push((i, fit(kit, ty, &cond_texts(&s("cond")), hero_room), Ink::Primary));
        }
        let days: Vec<Value> = d.get("days").and_then(Value::as_array).cloned().unwrap_or_default();
        if let Some(i) = self.lines.feels {
            let feels = doc.pointer("/current/feelsLike").and_then(Value::as_f64).map(|f| f.round() as i64);
            let hl = days.first().map(|d0| format!("H {}  L {}", d0.get("hi").and_then(Value::as_str).unwrap_or(""), d0.get("lo").and_then(Value::as_str).unwrap_or("")));
            // the whole of it when it fits, else the shorter ways of saying it
            let texts: Vec<String> = match (feels, hl) {
                (Some(f), Some(h)) => vec![format!("Feels like {f}\u{b0}  \u{b7}  {h}"), format!("Feels {f}\u{b0}  \u{b7}  {h}"), h.clone(), format!("Feels like {f}\u{b0}")],
                (Some(f), None) => vec![format!("Feels like {f}\u{b0}")],
                (None, Some(h)) => vec![h],
                (None, None) => vec![s("stats").split(" \u{b7} ").next().unwrap_or("").to_owned()],
            };
            let ty = self.lines.styles[i].ty;
            changes.push((i, fit(kit, ty, &texts, hero_room), Ink::Secondary));
        }
        if let Some(i) = self.lines.smart {
            let (full, short, tone) = smart_line(doc, more.pointer("/doc").unwrap_or(&Value::Null), twelve);
            let (room, ty) = (self.plan.smart.map_or(0.0, |s| s.f.w) * k, self.lines.styles[i].ty);
            // the whole of it when it fits, else the shorter way of saying it, else as much as there is room for
            let text = if kit.ui.fonts.measure(&full, ty).width <= room { full } else { ellipsise(&mut kit.ui.fonts, &short, ty, room) };
            changes.push((i, text, if tone == SmartTone::Warning { Ink::Warning } else { Ink::Accent }));
        }
        if self.plan.hours.is_some() {
            for (i, (t, v)) in self.lines.hours.iter().enumerate() {
                let (time, temp) = if i == 0 {
                    ("Now".to_owned(), s("temp"))
                } else {
                    let h = hours.get(i);
                    (hour_label(h.and_then(|h| h.get("time")).and_then(Value::as_str).unwrap_or(""), twelve), format!("{}\u{b0}", h.and_then(|h| h.get("temp")).and_then(Value::as_f64).unwrap_or(0.0).round() as i64))
                };
                changes.push((*t, time, Ink::Secondary));
                changes.push((*v, temp, Ink::Primary));
            }
        }
        for (i, (name, lo, hi)) in self.lines.days.iter().enumerate() {
            match days.get(i) {
                Some(dv) => {
                    let ds = |k: &str| dv.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
                    changes.push((*name, ds("day"), Ink::Primary));
                    if let Some(lo) = lo {
                        changes.push((*lo, ds("lo"), Ink::Secondary));
                    }
                    changes.push((*hi, ds("hi"), Ink::Primary));
                }
                None => {
                    changes.push((*name, String::new(), Ink::Primary));
                    if let Some(lo) = lo {
                        changes.push((*lo, String::new(), Ink::Primary));
                    }
                    changes.push((*hi, String::new(), Ink::Primary));
                }
            }
        }
        let (ink, ink2, accent, warn) = (kit.ui.look.ink(), kit.ui.look.ink2(), kit.ui.look.accent(), kit.ui.look.warning());
        let light = kit.mode == ui::lucent::palette::Mode::Light;
        let colour = |i: Ink| match i {
            Ink::Primary => ink,
            Ink::Secondary => ink2,
            Ink::Accent => legible(accent, ink, light),
            Ink::Warning => legible(warn, ink, light),
        };
        let refs: Vec<(usize, &str, Rgba)> = changes.iter().map(|(i, t, c)| (*i, t.as_str(), colour(*c))).collect();
        self.block.set_many(kit, &refs);
        self.paint(kit, pictures, doc, &hours, &days);
    }

    /// The card's picture: the hairline under the reading, the hours' icons and bars, the week's icons and range bars.
    fn paint(&mut self, kit: &mut Kit, pictures: &mut Pictures, doc: &Value, hours: &[Value], days: &[Value]) {
        let k = self.k;
        let (w, h) = (self.w as u32, self.h as u32);
        let (ink, accent) = (kit.ui.look.ink(), kit.ui.look.accent());
        let light = kit.mode == ui::lucent::palette::Mode::Light;
        self.rows.clear();
        let p = &self.plan;
        let mut canvas = Canvas { px: self.rows.pixels_mut(), w, h };
        // the horizon the rain lands on: a hairline under the reading, where something follows it, that fades out at both ends
        let below = if p.split { p.smart.is_some_and(|s| s.f.x == 0.0) } else { p.smart.is_some() || p.hours.is_some() || p.days.is_some() };
        let line_w = ((if p.split { p.hero_w } else { p.w }) * k).round() as u32;
        let y = ((p.hero_bottom + 6.0) * k).round() as u32;
        let fade = (24.0 * k).min(line_w as f32 / 2.0).max(1.0);
        let line = faded(ink, 0.14);
        if below && y < h {
            for x in 0..line_w.min(w) {
                let edge = ((x as f32 + 0.5) / fade).min((line_w as f32 - x as f32 - 0.5) / fade).clamp(0.0, 1.0);
                let i = ((y * w + x) * 4) as usize;
                canvas.px[i..i + 4].copy_from_slice(&[(f32::from(line.0) * edge).round() as u8, (f32::from(line.1) * edge).round() as u8, (f32::from(line.2) * edge).round() as u8, (f32::from(line.3) * edge).round() as u8]);
            }
        }
        if let Some(hr) = &p.hours {
            let now_kind = doc.pointer("/current/code").and_then(Value::as_i64).zip(doc.pointer("/current/isDay").and_then(Value::as_bool)).map(|(c, d)| kind_of(c, d));
            for i in 0..hr.n {
                let Some(hv) = hours.get(i) else { break };
                let cx = (hr.x + (i as f32 + 0.5) * hr.col_w) * k;
                let kind = if i == 0 { now_kind } else { None }.unwrap_or_else(|| kind_of(hv.get("code").and_then(Value::as_i64).unwrap_or(0), hv.get("isDay").and_then(Value::as_bool).unwrap_or(true)));
                let side = (hr.icon * k).round() as u32;
                if let Some(pic) = pictures.get_look(&format!("wxbig-{kind}"), light) {
                    canvas.picture(pic, (cx - side as f32 / 2.0).round() as i32, (hr.icon_y * k).round() as i32, side, side);
                }
                if let Some((top, bh)) = hr.bar {
                    let prob = (hv.get("rainPct").and_then(Value::as_f64).unwrap_or(0.0) / 100.0).clamp(0.0, 1.0) as f32;
                    let bw = 18.0 * k;
                    canvas.rounded(Frame::new(cx - bw / 2.0, top * k, bw, bh * k), bw / 2.0, straight(faded(ink, 0.10)));
                    if prob >= 0.04 {
                        let fh = (bh * k * prob).max(bw * 0.5);
                        canvas.rounded(Frame::new(cx - bw / 2.0, (top + bh) * k - fh, bw, fh), bw / 2.0, if light { faded(accent, 0.9) } else { accent });
                    }
                }
            }
        }
        if let Some(d) = &p.days {
            let deg = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).and_then(|t| t.trim_end_matches('\u{b0}').parse::<f32>().ok());
            let (mut lo_min, mut hi_max) = (f32::MAX, f32::MIN);
            for dv in days {
                if let (Some(lo), Some(hi)) = (deg(dv, "lo"), deg(dv, "hi")) {
                    lo_min = lo_min.min(lo);
                    hi_max = hi_max.max(hi);
                }
            }
            let track = straight(faded(ink, 0.12));
            for (i, dv) in days.iter().enumerate().take(d.rows) {
                let kind = dv.get("kind").and_then(Value::as_str).unwrap_or("");
                let y = (d.y + i as f32 * d.row_h) * k;
                let icon = (d.icon * k).round() as u32;
                if d.icon > 0.0
                    && let Some(pic) = pictures.get_look(&format!("wxbig-{kind}"), light)
                {
                    canvas.picture(pic, ((d.x + d.day_w) * k).round() as i32, (y + (d.row_h * k - icon as f32) / 2.0).round() as i32, icon, icon);
                }
                if !d.bar {
                    continue;
                }
                // the range bar: the day's span of the week's
                let bx = (d.x + d.day_w + d.icon + 10.0 + d.num_w + 14.0) * k;
                let bw = ((d.x + d.w - d.num_w - 14.0) * k - bx).max(0.0);
                let by = y + (d.row_h * k - 8.0 * k) / 2.0;
                canvas.rounded(Frame::new(bx, by, bw, 8.0 * k), 4.0 * k, track);
                if let (Some(lo), Some(hi)) = (deg(dv, "lo"), deg(dv, "hi"))
                    && hi_max > lo_min
                {
                    let (f0, f1) = ((lo - lo_min) / (hi_max - lo_min), (hi - lo_min) / (hi_max - lo_min));
                    canvas.rounded(Frame::new(bx + bw * f0, by, (bw * (f1 - f0)).max(8.0 * k), 8.0 * k), 4.0 * k, accent);
                }
            }
        }
        self.rows.damage(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inside(p: &Plan, h: f32, what: &str) {
        let all: Vec<(&str, Slot)> = [("place", p.place), ("temp", Some(p.temp)), ("cond", p.cond), ("feels", p.feels), ("smart", p.smart)].into_iter().filter_map(|(n, s)| s.map(|s| (n, s))).collect();
        for (n, s) in all {
            assert!(s.f.x >= 0.0 && s.f.y >= 0.0 && s.f.right() <= p.w + 0.5 && s.f.bottom() <= h + 0.5, "{what}: {n} {:?} outside {}x{}", s.f, p.w, h);
        }
        if let Some(hr) = &p.hours {
            assert!(hr.x >= 0.0 && hr.x + hr.n as f32 * hr.col_w <= p.w + 0.5 && hr.y + hr.height <= h + 0.5, "{what}: hours {hr:?} outside {}x{}", p.w, h);
            assert!(hr.temp_y + hr.temp_px * 1.25 <= hr.y + hr.height + 0.5, "{what}: the hours' temperatures are inside the strip {hr:?}");
        }
        if let Some(d) = &p.days {
            assert!(d.x >= 0.0 && d.x + d.w <= p.w + 0.5 && d.y + d.rows as f32 * d.row_h <= h + 0.5, "{what}: days {d:?} outside {}x{}", p.w, h);
        }
    }

    /// The boxes of the parts of a plan, to be checked against one another.
    fn boxes(p: &Plan) -> Vec<(String, Frame)> {
        let mut v: Vec<(String, Frame)> = Vec::new();
        for (n, s) in [("place", p.place), ("temp", Some(p.temp)), ("cond", p.cond), ("feels", p.feels), ("smart", p.smart)] {
            if let Some(s) = s {
                v.push((n.to_owned(), s.f));
            }
        }
        if let Some(h) = &p.hours {
            v.push(("hours".to_owned(), Frame::new(h.x, h.y, h.n as f32 * h.col_w, h.height)));
        }
        if let Some(d) = &p.days {
            v.push(("days".to_owned(), Frame::new(d.x, d.y, d.w, d.rows as f32 * d.row_h)));
        }
        v
    }

    fn data_sets() -> [Data; 5] {
        [Data::default(), Data { hours: true, bars: false, wide_temp: false }, Data { hours: true, bars: true, wide_temp: false }, Data { hours: true, bars: true, wide_temp: true }, Data { hours: false, bars: false, wide_temp: true }]
    }

    #[test]
    fn every_part_there_is_room_for_is_inside_the_box_and_apart_from_the_others_whatever_the_box_and_the_text_size() {
        for k in [1.0, 1.5, 2.0] {
            // the content boxes of every card the grid allows: two to twelve columns of 131 px with 20 between, three to sixteen rows of 37 with 20 between, less the card's padding
            for cols in 2..=12 {
                for rows in 3..=16 {
                    let (w, h) = (f64::from(cols) * 131.0 + f64::from(cols - 1) * 20.0 - 64.0, f64::from(rows) * 37.0 + f64::from(rows - 1) * 20.0 - 56.0);
                    for data in data_sets() {
                        let (w, h) = (w as f32 / k, h as f32 / k);
                        let p = plan(w, h, k, data);
                        let what = format!("{cols}x{rows} at {k}: {data:?}");
                        inside(&p, h, &what);
                        let b = boxes(&p);
                        for (i, (an, a)) in b.iter().enumerate() {
                            for (bn, bb) in &b[i + 1..] {
                                let (ox, oy) = (a.right().min(bb.right()) - a.x.max(bb.x), a.bottom().min(bb.bottom()) - a.y.max(bb.y));
                                assert!(ox <= 0.5 || oy <= 0.5, "{what}: {an} {a:?} and {bn} {bb:?} overlap ({w}x{h})");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_default_card_has_the_reading_the_next_thing_and_the_hours_with_their_rain() {
        let p = plan(369.0, 424.0, 1.0, Data { hours: true, bars: true, wide_temp: false });
        assert!(!p.split);
        assert!(p.place.is_some() && p.feels.is_some() && p.smart.is_some(), "the whole reading and the line that says what is next");
        assert_eq!(p.temp.px, 112.0);
        let hr = p.hours.expect("the hours");
        assert_eq!(hr.n, 5);
        assert!(hr.bar.is_some(), "with the bars of the rain");
        let dry = plan(369.0, 424.0, 1.0, Data { hours: true, bars: false, wide_temp: false }).hours.expect("the hours");
        assert!(dry.bar.is_none() && dry.icon > hr.icon, "with no rain to show, the room is the pictures'");
        assert!(hr.y >= p.hero_bottom, "under the reading");
        assert!(p.days.is_none(), "no room for the week as well: that is the view's");
        // without the hours the week fills the room
        let p = plan(369.0, 424.0, 1.0, Data::default());
        assert!(p.hours.is_none() && p.days.is_some_and(|d| d.rows >= 3));
    }

    #[test]
    fn a_card_a_little_short_of_the_default_trades_the_feels_line_for_the_hours_and_one_shorter_still_keeps_the_next_thing() {
        let data = Data { hours: true, bars: true, wide_temp: false };
        // 3 columns by 6 rows: the reading as large as it can be with the next thing under it, no hours
        let p = plan(369.0, 266.0, 1.0, data);
        assert!(p.hours.is_none() && p.smart.is_some() && p.feels.is_some() && p.place.is_some(), "{:?}", p.temp);
        assert!(p.temp.px >= 96.0);
        // one row more makes room for the hours, and the reading is a size smaller
        let p = plan(369.0, 323.0, 1.0, data);
        assert!(p.hours.is_some() && p.smart.is_some());
        // 3 by 4: the temperature, the condition and the next thing
        let p = plan(369.0, 152.0, 1.0, data);
        assert!(p.smart.is_some() && p.cond.is_some() && p.hours.is_none() && p.place.is_none());
        // 3 by 3: the temperature and the condition
        let p = plan(369.0, 95.0, 1.0, data);
        assert!(p.smart.is_none() && p.cond.is_some());
    }

    #[test]
    fn a_wide_card_has_the_week_beside_the_reading_and_a_small_one_only_the_reading() {
        let p = plan(822.0, 324.0, 1.0, Data { hours: true, bars: true, wide_temp: false });
        assert!(p.split && p.hours.is_some_and(|h| h.n >= 5) && p.days.is_some_and(|d| d.rows >= 3));
        assert!(p.days.is_some_and(|d| d.x >= p.hero_w), "beside the reading, not under it");
        assert!(p.smart.is_some_and(|s| s.f.x == 0.0), "the next thing is under the reading");
        let p = plan(184.0, 212.0, 1.0, Data { hours: true, bars: true, wide_temp: false });
        assert!(p.hours.is_none() && p.days.is_none(), "a card as small as the largest text size makes the default one has the reading");
        let p = plan(300.0, 60.0, 1.0, Data::default());
        assert!(p.place.is_none() && p.feels.is_none() && p.smart.is_none());
        // a wide short card (6 by 3) has its hours, compact, beside the reading
        let p = plan(822.0, 95.0, 1.0, Data { hours: true, bars: true, wide_temp: false });
        assert!(p.split && p.hours.is_some_and(|h| h.height < HOURS_H && h.n >= 5));
        // a wide tall one has the forecast under the reading, the hours across all of it, and the sky stays beside the temperature
        let p = plan(822.0, 424.0, 1.0, Data { hours: true, bars: true, wide_temp: false });
        assert!(!p.split && p.hours.is_some_and(|h| h.n == 8) && p.smart.is_some());
        assert!(p.art.x1 - p.art.x0 <= 0.8 * ART_MAX + 8.0 + 0.5, "{:?}", p.art);
    }

    #[test]
    fn the_picture_of_the_sky_is_beside_the_temperature_not_on_it() {
        for k in [1.0, 2.0] {
            for (w, h) in [(218.0, 266.0), (369.0, 424.0), (369.0, 152.0), (520.0, 266.0), (822.0, 437.0)] {
                for wide in [false, true] {
                    let (w, h) = (w / k, h / k);
                    let p = plan(w, h, k, Data { hours: true, bars: true, wide_temp: wide });
                    let em = if wide { 2.3 } else { 1.7 };
                    // the free box starts to the right of the widest temperature there can be
                    assert!(p.art.x0 >= p.temp.px * em, "{w}x{h} at {k}: {:?}", p.art);
                    assert!(p.art.x1 > p.art.x0 && p.art.y1 > p.art.y0, "{w}x{h} at {k}: a box for it: {:?}", p.art);
                }
            }
        }
    }

    #[test]
    fn the_sky_covers_the_reading_and_a_split_card_fades_it_out_before_the_forecast() {
        let data = Data { hours: true, bars: true, wide_temp: false };
        let p = plan(369.0, 424.0, 1.0, data);
        let st = sky_box(&p, 1.0, 369.0, 424.0, 24.0);
        assert_eq!(st.w, 433.0, "the whole width of the card");
        assert!(st.h > p.hero_bottom && st.h < 480.0 && st.card.is_some_and(|c| c.w == 433.0 && c.h == 480.0));
        let p = plan(822.0, 324.0, 1.0, data);
        let st = sky_box(&p, 1.0, 822.0, 324.0, 24.0);
        assert!(st.w < 886.0 && st.w >= PAD_SIDE + p.hero_w, "the reading's side");
        assert!(st.fade[1] >= 48.0, "and it fades out softly at the forecast's side");
        assert!(st.hero.0 + st.size * 0.42 <= st.w - st.fade[1] + 12.0 && st.hero.0 - st.size * 0.42 >= 0.0, "the hero (what is seen of it, 0.42 of its square each way) is inside it, and clear of the fade but for its last rays");
    }
}
