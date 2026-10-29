//! The saver drawn: black over everything, and in it one block of a few dim marks that moves, drifts and dims (the numbers are
//! [`look`](super::look)'s, the timeline [`model`](super::model)'s): an analog clock (a hairline ring and twelve ticks, an hour and a minute
//! hand and a second hand that sweeps, each with a hint of glow), the date, the weather, and the rows of the saver's small areas
//! ([`glance`]: slots and notices). Nothing is brighter than the cap: every colour is made from the dark material's inks at a fraction of it
//! and a layer's opacity only lowers it.
//!
//! The clock is three pictures of the CPU's raster (the compositor draws pictures, not rotated shapes): the face once, the hands every
//! [`HANDS_MS`], the second hand every [`SWEEP_MS`] over the small region it touched. A reduced-motion setting steps the second hand once a
//! second and keeps the moves of the block (they are burn-in care). The layers are made when the saver comes up (or what it says changes)
//! and dropped when it has gone: nothing hidden is kept alive.

use super::clock::{self, Canvas, Palette, Px};
use super::glance::{Glance, Kind as RowKind};
use super::look::{self, INK1, INK2};
use super::model::{Saver, View};
use crate::host::clock::{civil_at, system_offset_s};
use crate::kit::Kit;
use crate::kit::icon;
use crate::kit::lint::Lint;
use crate::kit::motion::Beat;
use crate::kit::text::{Text, role_style, width_of};
use crate::system::parts::{HIDDEN, Part};
use compositor_protocol::{Ease, Fill, Props, Rect, Rgba, Shape};
use lumatv_saver::{FALLBACK_GLYPH, Tone};
use lumatv_tokens::TypeRole;
use serde_json::{Value, json};
use ui::components::Align;
use ui::geometry::Frame;
use ui::lucent::palette::{Mode, PaletteRole, Straight};
use ui::scene::{ImageLayer, ShapeLayer};

const SCREEN: (f32, f32) = (1920.0, 1080.0);
/// The second hand's step: ten a second is a sweep to the eye at this brightness (its tip moves under two pixels), and a frame of
/// the compositor each; reduced motion steps once a second.
pub const SWEEP_MS: u64 = 100;
pub const SWEEP_REDUCED_MS: u64 = 1000;
/// The hour and minute hands are drawn again this often (the minute hand's tip moves 1.5 pixels).
pub const HANDS_MS: u64 = 6000;
/// Black comes in over this long when the saver comes up.
pub const FADE_IN_MS: u32 = 700;
/// The gap between the clock and the words.
const GAP: f32 = 64.0;

/// A word of the block, kept for the lint and the harness: its frame in the block's own space.
struct Spoken {
    name: String,
    frame: Frame,
    ink: Rgba,
    /// Drawn as glyph quads of the atlas, not as a picture of its own.
    run: bool,
    px: u32,
}

struct Clock {
    d: u32,
    pal: Palette,
    /// The clock's corner in the block.
    at: (f32, f32),
    face: ImageLayer,
    hands: ImageLayer,
    sweep: ImageLayer,
    hands_canvas: Canvas,
    sweep_canvas: Canvas,
    prev: Option<Px>,
}

fn day_seconds(now_ms: u64) -> f64 {
    civil_at(now_ms, system_offset_s(now_ms)).day_seconds()
}

/// Copies `r` of `canvas` into `layer` and tells the compositor it changed.
fn blit(layer: &mut ImageLayer, canvas: &Canvas, r: Px) {
    let r = Px { x0: r.x0.max(0), y0: r.y0.max(0), x1: r.x1.min(canvas.w as i32), y1: r.y1.min(canvas.h as i32) };
    if r.x1 <= r.x0 || r.y1 <= r.y0 {
        return;
    }
    let stride = canvas.w as usize * 4;
    let (a, b) = (r.x0 as usize * 4, r.x1 as usize * 4);
    let px = layer.pixels_mut();
    for y in r.y0 as usize..r.y1 as usize {
        px[y * stride + a..y * stride + b].copy_from_slice(&canvas.px[y * stride + a..y * stride + b]);
    }
    layer.damage(Some(Rect { x: r.x0 as f32, y: r.y0 as f32, w: (r.x1 - r.x0) as f32, h: (r.y1 - r.y0) as f32 }));
}

impl Clock {
    fn new(kit: &Kit, z: i32, d: u32, pal: Palette, at: (f32, f32)) -> std::io::Result<Self> {
        let layer = |z| kit.ui.scene.image(d, d, z, HIDDEN);
        let mut c = Self {
            d,
            pal,
            at,
            face: layer(z)?,
            hands: layer(z + 1)?,
            sweep: layer(z + 2)?,
            hands_canvas: Canvas::new(d, d),
            sweep_canvas: Canvas::new(d, d),
            prev: None,
        };
        let mut face = Canvas::new(d, d);
        clock::paint_face(&mut face, d, &c.pal);
        blit(&mut c.face, &face, Px { x0: 0, y0: 0, x1: d as i32, y1: d as i32 });
        Ok(c)
    }

    /// The hour and minute hands for `now_ms`.
    fn step_hands(&mut self, now_ms: u64) {
        let (hour, minute, _) = clock::angles(day_seconds(now_ms));
        self.hands_canvas.clear();
        clock::paint_hands(&mut self.hands_canvas, self.d, &self.pal, hour, minute);
        blit(&mut self.hands, &self.hands_canvas, Px { x0: 0, y0: 0, x1: self.d as i32, y1: self.d as i32 });
    }

    /// The second hand for `now_ms` (a whole second when `whole`), over what it touched before.
    fn step_second(&mut self, now_ms: u64, whole: bool) {
        let s = day_seconds(now_ms);
        let (_, _, deg) = clock::angles(if whole { s.floor() } else { s });
        if let Some(p) = self.prev {
            self.sweep_canvas.clear_rect(p);
        }
        let touched = clock::paint_second(&mut self.sweep_canvas, self.d, &self.pal, deg);
        let region = self.prev.map_or(touched, |p| p.union(touched));
        blit(&mut self.sweep, &self.sweep_canvas, region);
        self.prev = Some(touched);
    }

    fn layers(&self) -> [&ui::scene::Layer; 3] {
        [&self.face, &self.hands, &self.sweep]
    }
}

/// Everything the block is made of, for one look of what it says.
struct Built {
    key: String,
    block: (f32, f32),
    clock: Clock,
    parts: Vec<(Part, (f32, f32))>,
    words: Vec<Spoken>,
    rows: usize,
}

pub struct SaverSurface {
    z: i32,
    built: Option<Built>,
    plate: Option<ShapeLayer>,
    shown: bool,
    faded: bool,
    reduced: bool,
    last: Option<((i32, i32), f32)>,
    sweep_at: u64,
    hands_at: u64,
    minute: u64,
    leaving_until: Option<u64>,
}

fn lit(at: (f32, f32), opacity: f32) -> Props {
    Props { opacity, ..Props::at(at.0, at.1) }
}

fn gone(at: (f32, f32)) -> Props {
    Props { visible: false, opacity: 0.0, ..Props::at(at.0, at.1) }
}

/// `text` cut with an ellipsis until it is at most `max` wide in `style`.
fn fit(kit: &mut Kit, style: ui::components::Style, text: &str, max: f32) -> String {
    if width_of(kit, style, text) <= max {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let cut: String = chars[..mid].iter().collect::<String>().trim_end().to_string() + "\u{2026}";
        if width_of(kit, style, &cut) <= max {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if lo == 0 { "\u{2026}".to_string() } else { chars[..lo].iter().collect::<String>().trim_end().to_string() + "\u{2026}" }
}

impl SaverSurface {
    pub fn new(z: i32) -> Self {
        Self { z, built: None, plate: None, shown: false, faded: false, reduced: false, last: None, sweep_at: 0, hands_at: 0, minute: 0, leaving_until: None }
    }

    pub fn is_up(&self) -> bool {
        self.shown
    }

    /// How many layers the saver holds (the scene's budget).
    pub fn layers(&self) -> usize {
        self.built.as_ref().map_or(0, |b| b.parts.len() + 3) + usize::from(self.plate.is_some())
    }

    /// The dark material's inks at `cap`: the saver sits on black, so it takes the dark inks whichever material Home is in.
    fn inks(kit: &Kit, cap: f32) -> (Palette, [Rgba; 3], Rgba, Rgba) {
        let roles = kit.tokens.roles(Mode::Dark);
        let three = |c: Straight, s: f32| [look::channel(cap, s, c.r), look::channel(cap, s, c.g), look::channel(cap, s, c.b)];
        let ink = roles[PaletteRole::Ink];
        let pal = Palette { ink1: three(ink, INK1), ink2: three(ink, INK2), ink3: three(ink, look::INK3), second: three(roles[PaletteRole::Accent], INK1) };
        let rgba = |c: [u8; 3]| Rgba(c[0], c[1], c[2], 255);
        let warn = three(roles[PaletteRole::Warning], INK1);
        let bad = three(roles[PaletteRole::Danger], INK1);
        (pal, [rgba(pal.ink1), rgba(pal.ink2), rgba(pal.ink3)], rgba(warn), rgba(bad))
    }

    fn key_of(kit: &Kit, glance: &Glance, cap: f32) -> String {
        format!("{glance:?}|{}|{}|{}", (cap * 1000.0).round(), kit.text_scale, kit.generation())
    }

    /// The clock's diameter at the person's text size (a little larger when the words are).
    fn diameter(kit: &Kit) -> u32 {
        (340.0 + 60.0 * (kit.text_scale - 1.0)).round() as u32
    }

    fn build(kit: &mut Kit, z: i32, glance: &Glance, cap: f32) -> std::io::Result<Built> {
        let (pal, [ink1, ink2, _ink3], warn, bad) = Self::inks(kit, cap);
        let scale = kit.text_scale;
        let d = Self::diameter(kit);
        let lh = kit.line(TypeRole::Label).ceil();
        let gs = (lh * 0.95).round().max(28.0);
        let gap = kit.space(3);
        let text_x = gs + gap;
        let col_max = (640.0 * scale).min(1100.0);
        let budget = d as f32 + 120.0 * scale;
        let title_style = role_style(kit, TypeRole::Title, None, ink1, Align::Left);
        // the roles' own weights only: the atlas warms Label in its Medium and the saver adds no weight of its own (a SemiBold alphabet at 200 %
        // text does not fit the warm bound and its words would be pictures); the value is told from its label by its ink
        let temp_style = role_style(kit, TypeRole::Label, None, ink1, Align::Left);
        let plain_style = role_style(kit, TypeRole::Label, None, ink2, Align::Left);
        let date_h = ui::components::line_height(title_style.ty);
        let row_h = lh;
        let glyph_ink = |tone: Tone| match tone {
            Tone::Warning => warn,
            Tone::Danger => bad,
            _ => ink2,
        };

        struct Plan {
            glyph: Option<(String, Rgba)>,
            runs: Vec<(String, ui::components::Style, f32)>,
            h: f32,
        }
        // the column: the date, the weather, the rows that fit
        let mut y = 0.0f32;
        let date_w = width_of(kit, title_style, &glance.date).min(col_max);
        let mut col_w = date_w;
        let mut plans: Vec<(Plan, f32)> = Vec::new();
        y += date_h;
        if let Some(w) = &glance.weather {
            y += kit.space(3);
            let x0 = if w.glyph.is_empty() { 0.0 } else { text_x };
            let temp_w = width_of(kit, temp_style, &w.temp);
            let cond = fit(kit, plain_style, &w.cond, (col_max - x0 - temp_w - gap).max(0.0));
            let cond_w = if cond.is_empty() { 0.0 } else { width_of(kit, plain_style, &cond) };
            let mut runs = vec![(w.temp.clone(), temp_style, x0)];
            if !cond.is_empty() {
                runs.push((cond, plain_style, x0 + temp_w + gap));
            }
            let width = x0 + temp_w + if cond_w > 0.0 { gap + cond_w } else { 0.0 };
            let glyph = (!w.glyph.is_empty()).then(|| (w.glyph.to_string(), ink2));
            col_w = col_w.max(width);
            plans.push((Plan { glyph, runs, h: gs.max(lh) }, y));
            y += gs.max(lh);
        }
        let mut rows_shown = 0;
        let mut last_kind = None;
        for row in &glance.rows {
            let before = if plans.is_empty() && rows_shown == 0 {
                kit.space(3)
            } else if last_kind == Some(row.kind) {
                kit.space(2)
            } else {
                kit.space(5)
            };
            if y + before + row_h > budget {
                break;
            }
            let (title_font, text_font) = match row.kind {
                RowKind::Slot => (plain_style, temp_style),
                RowKind::Notice => (temp_style, plain_style),
            };
            let avail = col_max - text_x;
            let mut title = row.title.clone();
            let mut text = row.text.clone();
            let mut age = row.age.clone();
            let tw = |kit: &mut Kit, s: &str, f| if s.is_empty() { 0.0 } else { width_of(kit, f, s) };
            let total = |kit: &mut Kit, t: &str, x: &str, a: &str| {
                let parts = [(t, title_font), (x, text_font), (a, plain_style)];
                let widths: Vec<f32> = parts.iter().map(|(s, f)| tw(kit, s, *f)).collect();
                widths.iter().sum::<f32>() + gap * widths.iter().filter(|w| **w > 0.0).count().saturating_sub(1) as f32
            };
            if total(kit, &title, &text, &age) > avail {
                age.clear();
            }
            if total(kit, &title, &text, &age) > avail {
                let room = (avail - tw(kit, &title, title_font) - if text.is_empty() { 0.0 } else { gap }).max(avail * 0.3);
                text = fit(kit, text_font, &text, room);
            }
            if total(kit, &title, &text, &age) > avail {
                let room = (avail - tw(kit, &text, text_font) - if text.is_empty() { 0.0 } else { gap }).max(avail * 0.3);
                title = fit(kit, title_font, &title, room);
            }
            let mut x = text_x;
            let mut runs = Vec::new();
            for (s, f) in [(&title, title_font), (&text, text_font), (&age, plain_style)] {
                if s.is_empty() {
                    continue;
                }
                runs.push((s.clone(), f, x));
                x += width_of(kit, f, s) + gap;
            }
            let width = (x - gap).max(text_x);
            col_w = col_w.max(width);
            y += before;
            plans.push((Plan { glyph: Some((row.glyph.clone(), glyph_ink(row.tone))), runs, h: row_h }, y));
            y += row_h;
            rows_shown += 1;
            last_kind = Some(row.kind);
        }
        let col_h = y;
        let block_h = (d as f32).max(col_h);
        let block_w = d as f32 + GAP + col_w;
        let clock_at = (0.0, ((block_h - d as f32) / 2.0).round());
        let col_x = d as f32 + GAP;
        let col_y = ((block_h - col_h) / 2.0).round();

        let mut clock = Clock::new(kit, z + 1, d, pal, clock_at)?;
        clock.step_hands(crate::host::now_ms());
        clock.step_second(crate::host::now_ms(), false);
        let mut parts: Vec<(Part, (f32, f32))> = Vec::new();
        let mut words: Vec<Spoken> = Vec::new();
        let put_text = |kit: &mut Kit,
                        name: &str,
                        text: &str,
                        style: ui::components::Style,
                        x: f32,
                        y: f32,
                        h: f32,
                        parts: &mut Vec<(Part, (f32, f32))>,
                        words: &mut Vec<Spoken>|
         -> std::io::Result<()> {
            let w = width_of(kit, style, text);
            let line = ui::components::line_height(style.ty);
            let frame = Frame::new(x, y + ((h - line) / 2.0).round(), w, line);
            let t = Text::new(kit, frame, z + 5, style, text)?;
            t.layer().set(HIDDEN);
            words.push(Spoken { name: name.to_string(), frame, ink: style.colour, px: style.ty.px, run: t.is_glyph_run() });
            parts.push((Part::Text(t), (frame.x, frame.y)));
            Ok(())
        };
        put_text(kit, "date", &glance.date, title_style, col_x, col_y, date_h, &mut parts, &mut words)?;
        for (i, (plan, py)) in plans.iter().enumerate() {
            let top = col_y + py;
            if let Some((name, ink)) = &plan.glyph {
                let side = gs as u32;
                let mut layer = icon::layer(kit, name, side, *ink, z + 5, HIDDEN)?;
                if layer.is_none() {
                    layer = icon::layer(kit, FALLBACK_GLYPH, side, *ink, z + 5, HIDDEN)?;
                }
                if let Some(l) = layer {
                    let at = (col_x, top + ((plan.h - gs) / 2.0).round());
                    parts.push((Part::Image(l), at));
                }
            }
            for (j, (text, style, rx)) in plan.runs.iter().enumerate() {
                let label = if i == 0 && glance.weather.is_some() { format!("weather {j}") } else { format!("row {i} {j}") };
                put_text(kit, &label, text, *style, col_x + rx, top, plan.h, &mut parts, &mut words)?;
            }
        }
        if kit.ui.scene.take_overflow() {
            return Err(std::io::Error::other("the scene is full: the saver could not be drawn whole"));
        }
        Ok(Built { key: String::new(), block: (block_w.ceil(), block_h.ceil()), clock, parts, words, rows: rows_shown })
    }

    fn put_plate(&mut self, kit: &Kit) {
        if self.plate.is_none() {
            let shape = Shape { corner_radius: 0.0, fill: Fill::Solid(Rgba(0, 0, 0, 255)), border_width: 0.0, border: Rgba(0, 0, 0, 0) };
            let plate = kit.ui.scene.shape(SCREEN.0 as u32, SCREEN.1 as u32, self.z, shape, gone((0.0, 0.0)));
            self.plate = Some(plate);
        }
    }

    /// Every layer of the block with its place in the block.
    fn each<'a>(b: &'a Built) -> Vec<(&'a ui::scene::Layer, (f32, f32))> {
        let mut out: Vec<(&ui::scene::Layer, (f32, f32))> = b.clock.layers().into_iter().map(|l| (l, b.clock.at)).collect();
        out.extend(b.parts.iter().map(|(p, at)| (p.layer(), *at)));
        out
    }

    /// Draws the saver for `saver`'s view and what it is to say. Returns whether anything of it is on the screen.
    pub fn apply(&mut self, kit: &mut Kit, saver: &mut Saver, glance: &Glance, now_ms: u64) -> std::io::Result<bool> {
        let mut view: View = saver.view(now_ms);
        self.reduced = view.reduced;
        if !view.up {
            return Ok(self.leave(kit, now_ms));
        }
        let key = Self::key_of(kit, glance, view.cap);
        let rebuilt = self.built.as_ref().is_none_or(|b| b.key != key);
        if rebuilt {
            let mut built = Self::build(kit, self.z + 1, glance, view.cap)?;
            built.key = key;
            saver.set_geometry(SCREEN, built.block);
            self.built = Some(built);
            view = saver.view(now_ms);
        }
        self.leaving_until = None;
        self.put_plate(kit);
        let b = self.built.as_ref().expect("built above");
        let pos = (view.pos.0 as f32, view.pos.1 as f32);
        let fading_in = !self.shown;
        if fading_in {
            if let Some(p) = &self.plate {
                p.set(gone((0.0, 0.0)));
                p.animate(Props::at(0.0, 0.0), FADE_IN_MS, Ease::Out);
            }
            for (layer, at) in Self::each(b) {
                layer.set(Props { opacity: 0.0, ..Props::at(pos.0 + at.0, pos.1 + at.1) });
                layer.animate(lit((pos.0 + at.0, pos.1 + at.1), view.level), FADE_IN_MS, Ease::Out);
            }
            self.shown = true;
            self.faded = false;
            self.sweep_at = now_ms;
            self.hands_at = now_ms + HANDS_MS;
            self.minute = now_ms / 60_000;
        } else if rebuilt {
            // what it says changed: the new layers take the place of the old at once
            for (layer, at) in Self::each(b) {
                layer.set(if self.faded { gone((pos.0 + at.0, pos.1 + at.1)) } else { lit((pos.0 + at.0, pos.1 + at.1), view.level) });
            }
        } else {
            let out = view.fade <= 0.0;
            if out && !self.faded {
                for (layer, at) in Self::each(b) {
                    layer.animate(gone((pos.0 + at.0, pos.1 + at.1)), view.fade_ms.max(1), Ease::In);
                }
                self.faded = true;
            } else if !out && self.faded {
                for (layer, at) in Self::each(b) {
                    layer.set(Props { opacity: 0.0, ..Props::at(pos.0 + at.0, pos.1 + at.1) });
                    layer.animate(lit((pos.0 + at.0, pos.1 + at.1), view.level), view.fade_ms.max(1), Ease::Out);
                }
                self.faded = false;
            } else if !self.faded && self.last != Some((view.pos, view.level)) {
                for (layer, at) in Self::each(b) {
                    layer.set(lit((pos.0 + at.0, pos.1 + at.1), view.level));
                }
            }
        }
        self.last = Some((view.pos, view.level));
        Ok(true)
    }

    /// Takes the saver off the screen over a layer beat; the layers are dropped when it has gone (`tick`).
    fn leave(&mut self, kit: &Kit, now_ms: u64) -> bool {
        if self.shown {
            let ms = kit.beat_ms(Beat::Layer).max(1);
            if let (Some(b), Some((pos, _))) = (&self.built, self.last) {
                for (layer, at) in Self::each(b) {
                    layer.animate(gone((pos.0 as f32 + at.0, pos.1 as f32 + at.1)), ms, Ease::In);
                }
            }
            if let Some(p) = &self.plate {
                p.animate(gone((0.0, 0.0)), ms, Ease::In);
            }
            self.shown = false;
            self.faded = false;
            self.leaving_until = Some(now_ms + u64::from(ms) + 40);
        }
        self.leaving_until.is_some()
    }

    /// Time passed: the hands and the sweep, the layers of a saver that has gone, the turn of a minute (the date and the ages).
    /// Returns whether what the saver says is to be looked at again.
    pub fn tick(&mut self, now_ms: u64) -> bool {
        if self.leaving_until.is_some_and(|t| now_ms >= t) {
            self.leaving_until = None;
            if !self.shown {
                self.built = None;
                self.plate = None;
                self.last = None;
            }
        }
        if !self.shown || self.faded {
            return false;
        }
        let Some(b) = &mut self.built else { return false };
        if now_ms >= self.hands_at {
            b.clock.step_hands(now_ms);
            self.hands_at = now_ms + HANDS_MS;
        }
        if now_ms >= self.sweep_at {
            let step = if self.reduced { SWEEP_REDUCED_MS } else { SWEEP_MS };
            b.clock.step_second(now_ms, self.reduced);
            self.sweep_at = (now_ms / step + 1) * step;
        }
        let minute = now_ms / 60_000;
        if minute != self.minute {
            self.minute = minute;
            return true;
        }
        false
    }

    pub fn due_at(&self) -> Option<u64> {
        let mut due = self.leaving_until;
        if self.shown && !self.faded {
            due = [due, Some(self.sweep_at), Some(self.hands_at), Some((self.minute + 1) * 60_000)].into_iter().flatten().min();
        }
        due
    }

    /// The look changed: made again at the next draw.
    pub fn restyle(&mut self) {
        self.built = None;
        self.last = None;
    }

    /// Where things are, for the harness and the tests: the block, the clock, every word, in screen pixels.
    pub fn geometry(&self) -> Value {
        let Some(b) = &self.built else { return json!({"up": false}) };
        let pos = self.last.map_or((0.0, 0.0), |(p, _)| (p.0 as f32, p.1 as f32));
        let r = |f: Frame| json!({"x": f.x + pos.0, "y": f.y + pos.1, "w": f.w, "h": f.h});
        json!({
            "up": self.shown,
            "faded": self.faded,
            "block": {"x": pos.0, "y": pos.1, "w": b.block.0, "h": b.block.1},
            "clock": {"x": pos.0 + b.clock.at.0, "y": pos.1 + b.clock.at.1, "d": b.clock.d},
            "level": self.last.map(|(_, l)| l),
            "rows": b.rows,
            "words": b.words.iter().map(|w| {
                let mut v = r(w.frame);
                v["name"] = json!(w.name);
                v["run"] = json!(w.run);
                v
            }).collect::<Vec<_>>(),
            "layers": self.layers(),
        })
    }

    /// Describes the words to the layout lint (no panel: the saver's contrast is traded for burn-in on purpose, the pixel tests hold its numbers).
    pub fn feed_lint(&self, lint: &mut Lint) {
        if !self.shown {
            return;
        }
        let (Some(b), Some((pos, _))) = (&self.built, self.last) else { return };
        for w in &b.words {
            let f = Frame::new(w.frame.x + pos.0 as f32, w.frame.y + pos.1 as f32, w.frame.w, w.frame.h);
            lint.word_at(&w.name, f, w.ink, w.px, true, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::clock::civil_at as civil;
    use crate::kit::screen::Input;
    use crate::kit::testkit;
    use crate::system::plugins::Hub;
    use crate::system::saver::glance;
    use crate::system::saver::look::Config;
    use serde_json::json;

    const NOW: u64 = 1_793_241_312_250;

    fn rig(scale: f32) -> (ui::testing::Harness, Kit, Saver, SaverSurface) {
        let (h, kit) = testkit::kit(scale);
        let cfg = Config::from_theme(&json!({}), false, 90);
        let mut saver = Saver::new(cfg, "test", 7);
        saver.force(true, NOW);
        (h, kit, saver, SaverSurface::new(1020))
    }

    fn glance_of(rows: usize, weather: bool) -> Glance {
        let mut hub = Hub::new();
        let items: Vec<Value> = (0..rows)
            .map(|i| json!({"id": format!("i{i}"), "source": "api", "glyph": "lock", "text": format!("Front door {i}"), "value": "Locked", "priority": i, "expiresAt": NOW + 600_000, "tone": if i == 1 { "warning" } else { "plain" }, "updatedAt": NOW}))
            .collect();
        hub.update(&Input::Helper { topic: "saver".into(), data: json!({"items": items, "rules": {}, "lines": [{"id": "camera.e1", "class": "camera.still", "glyph": "user", "title": "Driveway", "text": "Person", "at": NOW - 240_000, "expiresAt": NOW + 60_000}]}) }, NOW);
        let topic = if weather { json!({"state": "fresh", "doc": {"display": {"temp": "61\u{b0}", "cond": "Clear", "nowKind": "moon"}}}) } else { Value::Null };
        glance::build(&hub, &topic, &civil(NOW, 0), NOW, true)
    }

    fn rect(v: &Value) -> (f64, f64, f64, f64) {
        let f = |k: &str| v[k].as_f64().unwrap();
        (f("x"), f("y"), f("w"), f("h"))
    }

    #[test]
    fn the_block_is_a_clock_and_a_column_that_fits_the_safe_area_at_every_spot_and_both_text_sizes() {
        for scale in [1.0, 2.0] {
            let (_h, mut kit, mut saver, mut s) = rig(scale);
            let g = glance_of(4, true);
            assert!(s.apply(&mut kit, &mut saver, &g, NOW).unwrap());
            let geo = s.geometry();
            let (bx, by, bw, bh) = rect(&geo["block"]);
            assert!(bw > 700.0 && bh >= 340.0, "a block of a clock and words at {scale}: {geo}");
            assert!(
                bx >= 96.0 - 31.0 && by >= 54.0 - 21.0 && bx + bw <= 1920.0 - 96.0 + 31.0 && by + bh <= 1080.0 - 54.0 + 21.0,
                "in the safe area (less its drift) at {scale}: {geo}"
            );
            let (cx, cy, d) = (geo["clock"]["x"].as_f64().unwrap(), geo["clock"]["y"].as_f64().unwrap(), geo["clock"]["d"].as_f64().unwrap());
            assert!(cx >= bx && cy >= by && cx + d <= bx + bw + 0.5 && cy + d <= by + bh + 0.5, "the clock is in the block: {geo}");
            for w in geo["words"].as_array().unwrap() {
                let (x, y, ww, hh) = rect(w);
                assert!(x >= cx + d && x + ww <= bx + bw + 0.5 && y >= by - 0.5 && y + hh <= by + bh + 0.5, "{} is in the column at {scale}: {geo}", w["name"]);
            }
            assert!(s.layers() <= 60, "the saver's budget: {} layers at {scale}", s.layers());
            let mut lint = Lint::new();
            s.feed_lint(&mut lint);
            assert!(lint.findings().is_empty(), "at {scale}: {:?}", lint.findings());
        }
    }

    #[test]
    fn rows_that_do_not_fit_the_height_are_left_off_the_least_urgent_last() {
        let (_h, mut kit, mut saver, mut s) = rig(2.0);
        let g = glance_of(4, true);
        assert_eq!(g.rows.len(), 5, "four slots and a notice");
        s.apply(&mut kit, &mut saver, &g, NOW).unwrap();
        let shown = s.geometry()["rows"].as_u64().unwrap();
        assert!(shown >= 3 && shown <= 5, "at 200 % the column holds what the room does: {shown}");
        let (_h2, mut kit1, mut saver1, mut s1) = rig(1.0);
        s1.apply(&mut kit1, &mut saver1, &g, NOW).unwrap();
        assert_eq!(s1.geometry()["rows"], 5, "at 100 % all five fit");
    }

    #[test]
    fn a_long_word_is_cut_with_an_ellipsis_to_the_columns_width() {
        let (_h, mut kit, _saver, _s) = rig(1.0);
        let style = role_style(&kit, TypeRole::Label, None, Rgba(255, 255, 255, 255), Align::Left);
        let cut = fit(&mut kit, style, "A label that is certainly longer than the room the saver gives it", 200.0);
        assert!(cut.ends_with('\u{2026}') && width_of(&mut kit, style, &cut) <= 200.0, "{cut}");
        assert_eq!(fit(&mut kit, style, "Short", 200.0), "Short");
    }

    #[test]
    fn it_comes_in_on_black_moves_through_a_fade_dims_and_goes_when_the_saver_does() {
        let (_h, mut kit, mut saver, mut s) = rig(1.0);
        let g = glance_of(1, false);
        assert!(s.apply(&mut kit, &mut saver, &g, NOW).unwrap());
        assert!(s.is_up() && s.due_at().is_some(), "the sweep wants a tick");
        let spot = s.geometry()["block"].clone();
        // a move: faded out for its time, then the new spot
        let move_at = NOW + 60_000 + 2000 + 10;
        saver.update(&Input::Tick, move_at);
        s.apply(&mut kit, &mut saver, &g, move_at).unwrap();
        assert!(s.geometry()["faded"].as_bool().unwrap(), "fading out for the move");
        let again = move_at + 2000 + 10;
        saver.update(&Input::Tick, again);
        s.apply(&mut kit, &mut saver, &g, again).unwrap();
        assert!(!s.geometry()["faded"].as_bool().unwrap());
        assert_ne!(s.geometry()["block"], spot, "at another spot");
        // dimmed over the minutes
        let late = NOW + 25 * 60_000;
        saver.update(&Input::Tick, late);
        s.apply(&mut kit, &mut saver, &g, late).unwrap();
        let level = s.geometry()["level"].as_f64().unwrap();
        assert!((level - 0.4).abs() < 0.01, "dimmed to 0.4 of its first brightness: {level}");
        // gone: it fades over a beat and its layers are dropped
        saver.force(false, late + 10);
        assert!(s.apply(&mut kit, &mut saver, &g, late + 10).unwrap(), "leaving still counts as on the screen");
        assert!(!s.is_up());
        s.tick(late + 1000);
        assert_eq!(s.layers(), 0, "nothing hidden is kept alive");
        assert!(s.due_at().is_none());
    }

    #[test]
    fn the_second_hand_steps_ten_times_a_second_and_once_a_second_under_reduced_motion() {
        let (_h, mut kit, mut saver, mut s) = rig(1.0);
        let g = glance_of(0, false);
        s.apply(&mut kit, &mut saver, &g, NOW).unwrap();
        let first = s.due_at().unwrap();
        s.tick(first);
        let next = s.due_at().unwrap();
        assert!(next > first && next - first <= SWEEP_MS, "{first} {next}");
        let (_h2, kit2) = testkit::kit(1.0);
        let mut kit2 = kit2;
        let mut saver2 = Saver::new(Config::from_theme(&json!({}), true, 90), "t", 1);
        saver2.force(true, NOW);
        let mut s2 = SaverSurface::new(1020);
        s2.apply(&mut kit2, &mut saver2, &g, NOW).unwrap();
        let f2 = s2.due_at().unwrap();
        s2.tick(f2);
        let b1 = s2.due_at().unwrap();
        s2.tick(b1);
        assert_eq!(s2.due_at().unwrap() - b1, SWEEP_REDUCED_MS, "once a second under reduced motion, on the second");
    }

    #[test]
    fn the_turn_of_a_minute_asks_for_the_words_again_and_a_change_of_them_remakes_the_block() {
        let (_h, mut kit, mut saver, mut s) = rig(1.0);
        let g = glance_of(1, false);
        s.apply(&mut kit, &mut saver, &g, NOW).unwrap();
        assert!(!s.tick(NOW + 1000));
        assert!(s.tick(NOW + 61_000), "the minute turned");
        let before = s.layers();
        let more = glance_of(3, true);
        s.apply(&mut kit, &mut saver, &more, NOW + 62_000).unwrap();
        assert!(s.layers() > before, "more words, more layers");
        s.apply(&mut kit, &mut saver, &g, NOW + 63_000).unwrap();
        assert_eq!(s.layers(), before, "and back");
    }
}
