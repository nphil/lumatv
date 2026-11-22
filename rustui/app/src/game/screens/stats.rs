use super::paint::{Fill, HUD, Mark, Paint, W, Word};
use crate::game::{diagnosis::{Evidence, Legs, Line, Report, Tone}, stats::{Mode, Telemetry}};
use lumagame_core::ipc::StreamInfo;
use lumatv_tokens::{Radius, TypeRole};
use serde_json::Value;
use ui::{components::{Align, Form}, geometry::Frame, lucent::palette::PaletteRole as Role};

struct Hud<'a, 'b> { p: &'a mut Paint<'b>, words: Vec<Word>, marks: Vec<Mark>, col: f32 }
impl Hud<'_, '_> {
    fn text(&mut self, text: &str, x: f32, y: f32, role: TypeRole, ink: Role) -> f32 {
        let w = self.p.width(text, role);
        self.words.push(self.p.word(text, role, self.p.ink(ink, true), x, y, w));
        w
    }
    fn number(&mut self, value: &str, unit: &str, right: f32, y: f32, role: TypeRole) -> f32 {
        let (mut words, width) = self.p.figures(value, self.p.bold(role), self.p.ink(Role::Ink, true), 0.0, y);
        let unit_w = if unit.is_empty() { 0.0 } else { self.p.width(unit, TypeRole::Caption) + 6.0 * self.p.scale };
        let x = right - width - unit_w;
        for word in &mut words { word.x += x; }
        self.words.extend(words);
        if !unit.is_empty() { self.text(unit, right - unit_w + 6.0 * self.p.scale, y, TypeRole::Caption, Role::Ink2); }
        width + unit_w
    }
    fn head(&mut self, name: &str, status: &str, colour: Role, x: f32, y: f32, live: bool) -> f32 {
        let mut word = self.p.word(name, TypeRole::Label, self.p.ink(Role::Ink, true), x, y, self.col);
        word.ty = self.p.bold(TypeRole::Label);
        self.words.push(word);
        let status = if live { status } else { "No data" };
        let width = self.p.width(status, TypeRole::Caption);
        self.text(status, x + self.col - width, y, TypeRole::Caption, Role::Ink);
        let dx = x + self.col - width - 22.0;
        let dy = y + (self.p.line(TypeRole::Label) - 12.0) / 2.0;
        self.marks.push(Mark { form: Form::Disc(12), colour: self.p.ink(if live { colour } else { Role::Ink3 }, true), x: dx, y: dy });
        if !live { self.marks.push(Mark { form: Form::Disc(8), colour: self.p.ink(Role::Sheet, true), x: dx + 2.0, y: dy + 2.0 }); }
        y + self.p.line(TypeRole::Label) + 8.0
    }
    fn row(&mut self, label: &str, value: &str, unit: &str, x: f32, y: &mut f32, live: bool) {
        let label_w = self.text(label, x, *y, TypeRole::Caption, Role::Ink2);
        let value = if live { value } else { "–" };
        let unit = if live && value != "–" { unit } else { "" };
        let available = self.col - label_w - 24.0;
        let (words, width) = self.p.figures(value, self.p.bold(TypeRole::Caption), self.p.ink(Role::Ink, true), 0.0, 0.0);
        let unit_w = if unit.is_empty() { 0.0 } else { self.p.width(unit, TypeRole::Caption) + 6.0 * self.p.scale };
        if width + unit_w <= available {
            for mut word in words { word.x += x + self.col - width - unit_w; word.y = *y; self.words.push(word); }
            if !unit.is_empty() { self.text(unit, x + self.col - unit_w + 6.0 * self.p.scale, *y, TypeRole::Caption, Role::Ink2); }
        } else {
            // Names and drop explanations wrap rather than losing the reason at the panel edge.
            let joined = if unit.is_empty() { value.to_owned() } else { format!("{value} {unit}") };
            let ty = self.p.ty(TypeRole::Caption);
            let lines = ui::components::wrap(self.p.fonts, &joined, ty, available.max(120.0));
            for (i, line) in lines.into_iter().enumerate() {
                if i > 0 { *y += self.p.line(TypeRole::Caption); }
                self.words.push(Word { text: line, ty, colour: self.p.ink(Role::Ink, true), x: x + label_w + 24.0, y: *y,
                    w: available, align: Align::Right });
            }
        }
        *y += self.p.line(TypeRole::Caption) + 3.0;
    }
    fn graph(&mut self, values: &[u64], target_us: f32, frame: Frame, slots: usize) {
        let line = (frame.h * 2.0 / 3.0).round();
        self.marks.push(Mark { form: Form::Rule((slots * 4 - 1) as u32), colour: self.p.ink(Role::EdgeRaised, true), x: frame.x, y: frame.y + line });
        let start = values.len().saturating_sub(slots);
        let offset = slots.saturating_sub(values.len());
        for (i, &value) in values[start..].iter().enumerate() {
            let value = value as f32;
            let rise = value >= target_us;
            let raw = if rise { ((value - target_us) / (2.0 * target_us)).min(1.0) * line }
                else { ((target_us - value) / target_us).min(1.0) * (frame.h - line) };
            if raw < 1.0 { continue; }
            let height = raw.round().max(2.0);
            self.marks.push(Mark { form: Form::Bar { w: 3, h: height as u32 },
                colour: self.p.ink(if value > target_us * 1.5 { Role::Ink } else { Role::Ink3 }, true),
                x: frame.x + ((offset + i) * 4) as f32, y: frame.y + if rise { line - height + 1.0 } else { line } });
        }
    }
    /// A line of words in the section's quiet ink under its figures, at most two lines, ending in an ellipsis rather than leaving the column.
    fn verdict(&mut self, text: &str, x: f32, y: f32) -> f32 {
        let lines = self.p.fit(text, TypeRole::Caption, self.col, 2);
        let line = self.p.line(TypeRole::Caption);
        let ty = self.p.ty(TypeRole::Caption);
        let colour = self.p.ink(Role::Ink2, true);
        let n = lines.len();
        for (i, text) in lines.into_iter().enumerate() {
            self.words.push(Word { text, ty, colour, x, y: y + i as f32 * line, w: self.col, align: Align::Left });
        }
        y + n as f32 * line + 6.0
    }
    /// Bars rising from a baseline, newest at the right: each is `(fill, colour)` with the fill 0 to 1 of the frame's height.
    fn bars(&mut self, frame: Frame, slots: usize, bars: &[(f32, Role)], baseline: bool) {
        if baseline {
            self.marks.push(Mark { form: Form::Rule((slots * 4 - 1) as u32), colour: self.p.ink(Role::EdgeRaised, true), x: frame.x, y: frame.y + frame.h });
        }
        let start = bars.len().saturating_sub(slots);
        let offset = slots.saturating_sub(bars.len());
        for (i, &(fill, role)) in bars[start..].iter().enumerate() {
            let height = (fill.clamp(0.0, 1.0) * frame.h).round().max(2.0);
            self.marks.push(Mark { form: Form::Bar { w: 3, h: height as u32 }, colour: self.p.ink(role, true),
                x: frame.x + ((offset + i) * 4) as f32, y: frame.y + frame.h - height });
        }
    }
    /// One word per leg in the legend, wrapping at the column; the colour of a leg is the colour of its part of the bar.
    fn legend(&mut self, legs: &Legs, x: f32, mut y: f32) -> f32 {
        let line = self.p.line(TypeRole::Caption);
        let mut at = 0.0;
        for (i, name) in Legs::NAMES.iter().enumerate() {
            let value = legs.parts[i].map_or_else(|| "–".to_owned(), |v| if v < 10.0 { format!("{v:.1}") } else { format!("{v:.0}") });
            let width = self.p.width(name, TypeRole::Caption) + 8.0 + self.p.width(&value, TypeRole::Caption);
            if at > 0.0 && at + width > self.col { at = 0.0; y += line + 2.0; }
            let measured = legs.parts[i].is_some();
            self.marks.push(Mark { form: Form::Disc(10), colour: self.p.ink(if measured { LEG_ROLES[i] } else { Role::Ink3 }, true), x: x + at, y: y + (line - 10.0) / 2.0 });
            let w = self.text(name, x + at + 16.0, y, TypeRole::Caption, Role::Ink2);
            let mut figure = self.p.figures(&value, self.p.bold(TypeRole::Caption), self.p.ink(if measured { Role::Ink } else { Role::Ink3 }, true), x + at + 16.0 + w + 6.0, y);
            self.words.append(&mut figure.0);
            at += 16.0 + width + 22.0;
        }
        y + line + 6.0
    }
}

/// The parts of the latency bar, in the order a press travels. Hues the theme already names, so they follow it; none of them is a
/// tone colour, which belong to the verdicts.
const LEG_ROLES: [Role; 5] = [Role::Info, Role::Accent, Role::Live, Role::Ink, Role::Ink2];

fn tone(t: Tone) -> Role {
    match t { Tone::Good => Role::Positive, Tone::Warn => Role::Warning, Tone::Bad => Role::Danger, Tone::Quiet => Role::Ink3 }
}

fn decimal(value: Option<f64>, places: usize) -> String {
    value.filter(|v| v.is_finite() && *v >= 0.0).map_or_else(|| "–".into(), |v| format!("{v:.places$}"))
}
fn number(value: &Value) -> Option<f64> { value.as_f64().filter(|n| n.is_finite() && *n >= 0.0) }
fn pair(values: Option<[u64; 4]>, picks: &[usize]) -> String {
    values.map_or_else(|| "–".into(), |s| picks.iter().map(|i| format!("{:.1}", s[*i] as f64 / 1000.0)).collect::<Vec<_>>().join(" / "))
}

/// What a section reads: the telemetry, its diagnosis and what the stream was asked for.
struct Ctx<'a> {
    t: &'a Telemetry,
    ev: &'a Evidence,
    report: &'a Report,
    stream: Option<&'a StreamInfo>,
    target_us: f32,
    fps: String,
    cap_mbps: Option<f32>,
}
/// The head of a section: its name, its one-word verdict and the colour of that verdict.
fn head(h: &mut Hud, line: &Line, name: &str, x: f32, y: f32, live: bool) -> f32 { h.head(name, &line.tag, tone(line.tone), x, y, live) }

fn latency(h: &mut Hud, c: &Ctx, x: f32, y: f32) -> f32 {
    let legs = Legs::of(c.ev);
    let mut y = head(h, &c.report.latency, "Latency", x, y, legs.total().is_some());
    let bar = 14.0;
    let present: Vec<(usize, f32)> = legs.parts.iter().enumerate().filter_map(|(i, v)| v.map(|v| (i, v.max(0.0)))).collect();
    let total: f32 = present.iter().map(|(_, v)| v).sum();
    if total > 0.0 {
        let room = h.col - 2.0 * (present.len() - 1) as f32;
        let mut at = x;
        for (i, v) in present {
            let w = (room * v / total).floor().max(3.0);
            h.marks.push(Mark { form: Form::Round { w: w as u32, h: bar as u32, radius: 4 }, colour: h.p.ink(LEG_ROLES[i], true), x: at, y });
            at += w + 2.0;
        }
    } else {
        h.marks.push(Mark { form: Form::Rule(h.col as u32), colour: h.p.ink(Role::Ink3, true), x, y: y + bar / 2.0 });
    }
    y += bar + 10.0;
    y = h.legend(&legs, x, y);
    h.verdict(&c.report.latency.text, x, y)
}

fn smoothness(h: &mut Hud, c: &Ctx, x: f32, y: f32) -> f32 {
    let t = c.t;
    let frames = &t.frames;
    let mut y = head(h, &c.report.smooth, "Smoothness", x, y, t.live[1]);
    let slots = ((h.col - 80.0 * h.p.scale) / 4.0).floor() as usize;
    let graph = Frame::new(x, y + 2.0, h.col, 44.0 * h.p.scale.max(1.0));
    h.graph(if t.live[1] { &frames.graph } else { &[] }, c.target_us, graph, slots);
    h.number(&format!("{:.1}", c.target_us / 1000.0), "ms", x + h.col, graph.y + (graph.h * 2.0 / 3.0).round() + 2.0, TypeRole::Caption);
    y += graph.h + 14.0;
    let gaps = frames.gaps.filter(|_| t.live[1]);
    h.row("p95 / p99", &pair(gaps, &[1, 2]), "ms", x, &mut y, t.live[1]);
    let drops: u64 = frames.dropped.iter().sum();
    let why = ["full", "not ready", "refused", "after loss", "player error"].into_iter().zip(frames.dropped)
        .filter(|(_, n)| *n > 0).map(|(s, n)| format!("{n} {s}")).collect::<Vec<_>>().join(" · ");
    h.row("Dropped", &if drops > 0 { why } else { format!("0 of {}", frames.fed) }, "", x, &mut y, t.live[1]);
    h.verdict(&c.report.smooth.text, x, y)
}

fn network(h: &mut Hud, c: &Ctx, x: f32, y: f32) -> f32 {
    let t = c.t;
    let stream = &t.stream;
    let mut y = head(h, &c.report.network, "Network", x, y, t.live[0]);
    let slots = (h.col / 4.0).floor() as usize;
    let graph = Frame::new(x, y + 2.0, h.col, 28.0 * h.p.scale.max(1.0));
    // Bars are jitter against twice the threshold the diagnosis uses; a second that saw loss is drawn in the danger colour.
    let bars: Vec<(f32, Role)> = t.trend.iter().map(|s| {
        let lost = s.loss_pct.is_some_and(|l| l > 0.0);
        let jitter = s.jitter_ms.unwrap_or(0.0);
        (if lost { (jitter / 16.0).max(0.5) } else { jitter / 16.0 }, if lost { Role::Danger } else if jitter >= 8.0 { Role::Warning } else { Role::Ink3 })
    }).collect();
    h.bars(graph, slots, if t.live[0] { &bars } else { &[] }, true);
    y += graph.h + 8.0;
    h.row("Round trip / jitter", &format!("{} / {}", decimal(stream.rtt_ms.map(f64::from), 1), decimal(stream.jitter_ms.map(f64::from), 1)), "ms", x, &mut y, t.live[0]);
    h.row("Packet loss", &decimal(stream.loss_pct.map(f64::from), 1), "%", x, &mut y, t.live[0]);
    h.verdict(&c.report.network.text, x, y)
}

fn picture(h: &mut Hud, c: &Ctx, x: f32, y: f32) -> f32 {
    let t = c.t;
    let mut y = head(h, &c.report.picture, "Picture", x, y, t.live[0]);
    let slots = (h.col / 4.0).floor() as usize;
    let graph = Frame::new(x, y + 2.0, h.col, 28.0 * h.p.scale.max(1.0));
    // Each second's bitrate against what was asked for; under 60% of it is drawn in the warning colour, the share at which the
    // diagnosis calls the picture softer.
    let cap = c.cap_mbps.unwrap_or(0.0);
    let bars: Vec<(f32, Role)> = t.trend.iter().map(|s| {
        let b = s.bitrate_mbps.unwrap_or(0.0);
        (if cap > 0.0 { b / cap } else { 0.0 }, if cap > 0.0 && b < cap * 0.6 { Role::Warning } else { Role::Ink3 })
    }).collect();
    h.bars(graph, slots, if t.live[0] && cap > 0.0 { &bars } else { &[] }, false);
    h.marks.push(Mark { form: Form::Rule((slots * 4 - 1) as u32), colour: h.p.ink(Role::EdgeRaised, true), x, y: graph.y });
    y += graph.h + 8.0;
    let bitrate = decimal(t.stream.bitrate_kbps.map(|k| f64::from(k) / 1000.0), 1);
    h.row("Bitrate / asked", &format!("{bitrate} / {}", c.cap_mbps.map_or_else(|| "–".into(), |v| format!("{v:.0}"))), "Mb/s", x, &mut y, t.live[0]);
    let codec = c.stream.map_or("–", |s| match s.codec { lumagame_core::engine::Codec::H264 => "H.264", lumagame_core::engine::Codec::H265 => "H.265", lumagame_core::engine::Codec::Av1 => "AV1", lumagame_core::engine::Codec::Other => "–" });
    let format = c.stream.map_or_else(|| "–".into(), |s| format!("{}p · {codec} · {}", s.height, if s.hdr { "HDR" } else { "SDR" }));
    h.row("Stream", &format, "", x, &mut y, t.live[0]);
    h.verdict(&c.report.picture.text, x, y)
}

fn tv(h: &mut Hud, c: &Ctx, x: f32, y: f32) -> f32 {
    let t = c.t;
    let mut y = head(h, &c.report.tv, "TV", x, y, t.live[1] || t.live[2]);
    h.row("Queue / feed p95", &format!("{} / {}", t.frames.queue.map_or_else(|| "–".into(), |q| q.to_string()), pair(t.frames.feed, &[1])), "ms", x, &mut y, t.live[1]);
    let refresh = &t.display["refresh"];
    let (hz, fewest) = (number(&refresh["hz"]), refresh["fewest"].as_u64());
    let expected = hz.map(|v| (v / 10.0).round() as u64);
    let worst = fewest.zip(expected).map_or_else(|| "–".into(), |(f, e)| format!("{f} of {e}"));
    h.row("Refresh (vsync)", &decimal(hz, 0), "Hz", x, &mut y, t.live[2]);
    h.row("Fewest in 100 ms", &worst, "", x, &mut y, t.live[2]);
    h.verdict(&c.report.tv.text, x, y)
}

fn controller(h: &mut Hud, c: &Ctx, x: f32, y: f32) -> f32 {
    let t = c.t;
    let pad = t.stream.pad.as_ref();
    let mut y = head(h, &c.report.controller, "Controller", x, y, t.live[3]);
    h.row("Pad", pad.map_or("–", |p| p.name.as_str()), "", x, &mut y, t.live[3]);
    h.row("Rate / gap p99", &format!("{} Hz / {} ms", decimal(pad.map(|p| f64::from(p.rate_hz)), 0), decimal(pad.map(|p| f64::from(p.p99_us) / 1000.0), 1)), "", x, &mut y, t.live[3]);
    h.verdict(&c.report.controller.text, x, y)
}

/// One strip, MangoHud style: the verdict word, the frame rate, a frame-time sparkline and the end-to-end latency the measured
/// legs add up to. Just the words and marks over a thin veil, so the game stays in view.
fn compact(h: &mut Hud, c: &Ctx) -> (f32, f32) {
    let row_h = h.p.line(TypeRole::Label);
    let k = h.p.scale;
    let (pad_x, pad_y) = (16.0 * k, 7.0 * k);
    let dot = (10.0 * k).round();
    let (mut x, y) = (pad_x, pad_y);
    let live = c.t.live;
    let headline = &c.report.headline;
    h.marks.push(Mark { form: Form::Disc(dot as u32), colour: h.p.ink(tone(headline.tone), true), x, y: y + (row_h - dot) / 2.0 });
    x += dot + 8.0 * k;
    let mut word = h.p.word(&headline.tag, TypeRole::Label, h.p.ink(Role::Ink, true), x, y, 0.0);
    word.ty = h.p.bold(TypeRole::Label);
    word.w = h.p.fonts.measure(&word.text, word.ty).width.ceil();
    x += word.w + 22.0 * k;
    h.words.push(word);
    let fps = if live[0] { c.fps.as_str() } else { "–" };
    let (mut words, w) = h.p.figures(fps, h.p.bold(TypeRole::Label), h.p.ink(Role::Ink, true), x, y);
    h.words.append(&mut words);
    x += w + 6.0 * h.p.scale;
    x += h.text("fps", x, y, TypeRole::Caption, Role::Ink2) + 16.0 * k;
    let graph_h = (row_h - 6.0).max(16.0);
    h.graph(if live[1] { &c.t.frames.graph } else { &[] }, c.target_us, Frame::new(x, y + (row_h - graph_h) / 2.0, 79.0, graph_h), 20);
    x += 79.0 + 16.0 * k;
    let total = Legs::of(c.ev).total().filter(|_| live[0]);
    let latency = total.map_or_else(|| "–".to_owned(), |v| format!("{v:.0}"));
    let (mut words, w) = h.p.figures(&latency, h.p.bold(TypeRole::Label), h.p.ink(Role::Ink, true), x, y);
    h.words.append(&mut words);
    x += w + 6.0 * h.p.scale;
    x += h.text("ms", x, y, TypeRole::Caption, Role::Ink2);
    (x + pad_x, row_h + 2.0 * pad_y)
}

type Section = fn(&mut Hud, &Ctx, f32, f32) -> f32;

/// The sections in the order a problem is chased: what the verdict is, then each leg a press and a frame travel. Each takes the
/// column's x and the y it starts at and returns where the next starts.
const SECTIONS: [Section; 6] = [latency, smoothness, network, picture, tv, controller];

fn full(h: &mut Hud, c: &Ctx) -> (f32, f32) {
    const PAD: f32 = 24.0;
    const GAP: f32 = 18.0;
    const COLUMN_GAP: f32 = 36.0;
    let col = h.col;
    let mut y = PAD;
    let headline = &c.report.headline;
    h.marks.push(Mark { form: Form::Disc(14), colour: h.p.ink(tone(headline.tone), true), x: PAD, y: y + (h.p.line(TypeRole::Label) - 14.0) / 2.0 });
    let mut word = h.p.word(&headline.tag, TypeRole::Label, h.p.ink(Role::Ink, true), PAD + 26.0, y, col);
    word.ty = h.p.bold(TypeRole::Label);
    h.words.push(word);
    let right = format!("{} fps · {}", if c.t.live[0] { c.fps.as_str() } else { "–" }, Legs::of(c.ev).total().filter(|_| c.t.live[0]).map_or_else(|| "– ms".to_owned(), |v| format!("~{v:.0} ms")));
    let w = h.p.width(&right, TypeRole::Label);
    h.text(&right, PAD + col - w, y, TypeRole::Label, Role::Ink);
    y += h.p.line(TypeRole::Label) + 14.0;
    // Each section is measured once, then the list is cut where the two columns come out closest in height, so the panel is no
    // taller than it must be. The header sits above the first column.
    let heights: Vec<f32> = SECTIONS.iter().map(|section| {
        let (words, marks) = (h.words.len(), h.marks.len());
        let height = section(h, c, PAD, 0.0) + GAP;
        h.words.truncate(words);
        h.marks.truncate(marks);
        height
    }).collect();
    let tall = |k: usize| (y + heights[..k].iter().sum::<f32>()).max(PAD + heights[k..].iter().sum::<f32>());
    let split = (1..SECTIONS.len()).min_by(|a, b| tall(*a).total_cmp(&tall(*b))).unwrap_or(SECTIONS.len());
    let (mut left, mut right) = (y, PAD);
    for (i, section) in SECTIONS.iter().enumerate() {
        if i < split {
            section(h, c, PAD, left);
            left += heights[i];
        } else {
            section(h, c, PAD + col + COLUMN_GAP, right);
            right += heights[i];
        }
    }
    (PAD * 2.0 + col * 2.0 + COLUMN_GAP, left.max(right) - GAP + PAD)
}

pub fn show(p: &mut Paint<'_>) {
    if !p.model.hud_visible() { return; }
    let m = p.model;
    let is_full = m.prefs.stats.mode == Mode::Full;
    if is_full { p.scale = p.scale.min(1.25); }
    let t = &m.telemetry;
    let target_fps = m.view.stream.as_ref().map_or(m.prefs.fps, |s| s.fps).max(1);
    let c = Ctx {
        t, ev: &t.evidence, report: &t.report, stream: m.view.stream.as_ref(),
        target_us: 1_000_000.0 / target_fps as f32,
        fps: decimal(t.stream.fps.map(f64::from), if is_full { 1 } else { 0 }),
        cap_mbps: Some(m.stream_bitrate_mbps() as f32).filter(|v| *v > 0.0),
    };
    let col = 370.0 * p.scale;
    let mut h = Hud { p, words: Vec::new(), marks: Vec::new(), col };
    let (width, height) = if is_full { full(&mut h, &c) } else { compact(&mut h, &c) };
    let (dx, dy) = m.telemetry.rest.shift();
    let frame = Frame::new(W - h.p.x - width - dx, h.p.y + dy, width, height);
    #[cfg(test)]
    assert!(frame.y - dy + frame.h <= super::paint::H - h.p.y, "HUD exceeds the safe height: {frame:?}");
    // A veil, not a panel: the game shows through. The strip is the thinner of the two.
    h.p.surface(HUD.start, frame, Fill::Veil(if is_full { 0.80 } else { 0.58 }), if is_full { Radius::Card } else { Radius::Pill }, true, false, false, 20);
    h.p.words(HUD.start + 1, frame, h.words, h.marks, 21);
    // Panel care (see `stats::Rest`): the veil and the words dim together, never past what they were.
    let strength = m.telemetry.rest.strength();
    for node in h.p.plan.nodes.iter_mut().rev().take(2) { node.props.opacity *= strength; }
    #[cfg(test)]
    super::paint::assert_readable(&h.p.plan, h.p.fonts, true);
}
