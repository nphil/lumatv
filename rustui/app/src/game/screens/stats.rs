use super::paint::{Fill, LIVE, Mark, Paint, W, Word};
use crate::game::stats::Mode;
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
    fn rule(&mut self, x: f32, y: f32) -> f32 {
        self.marks.push(Mark { form: Form::Rule(self.col.round() as u32), colour: self.p.ink(Role::Edge, true), x, y: y + 22.0 });
        y + 45.0
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
}
fn decimal(value: Option<f64>, places: usize) -> String {
    value.filter(|v| v.is_finite() && *v >= 0.0).map_or_else(|| "–".into(), |v| format!("{v:.places$}"))
}
fn number(value: &Value) -> Option<f64> { value.as_f64().filter(|n| n.is_finite() && *n >= 0.0) }
fn ms_pair(values: Option<[u64; 4]>, a: usize) -> String {
    values.map_or_else(|| "–".into(), |s| format!("{:.1} / {:.1}", s[a] as f64 / 1000.0, s[a + 1] as f64 / 1000.0))
}

pub fn show(p: &mut Paint<'_>) {
    if !p.model.hud_visible() { return; }
    let m = p.model;
    let full = m.prefs.stats.mode == Mode::Full;
    if full { p.scale = p.scale.min(1.5); }
    let t = &m.telemetry;
    let stream = &t.stream;
    let frames = &t.frames;
    let display = &t.display;
    let target = 1_000_000.0 / m.view.stream.as_ref().map_or(m.prefs.fps, |s| s.fps).max(1) as f32;
    let fps = decimal(stream.fps.map(f64::from), if full { 1 } else { 0 });
    let hz = decimal(number(&display["refresh"]["hz"]), if full { 1 } else { 0 });
    let col = 440.0 * p.scale;
    let mut h = Hud { p, words: Vec::new(), marks: Vec::new(), col };
    let (width, height) = if !full {
        let row_h = h.p.line(TypeRole::Label);
        let y = 13.0;
        let mut x = 28.0;
        for (i, (value, unit, live)) in [(fps.as_str(), "fps", t.live[0]), (hz.as_str(), "Hz", t.live[2])].into_iter().enumerate() {
            if i > 0 {
                h.marks.push(Mark { form: Form::Disc(5), colour: h.p.ink(Role::Ink2, true), x: x + 16.0, y: y + row_h / 2.0 - 2.5 });
                x += 37.0;
            }
            let value = if live { value } else { "–" };
            let (mut words, w) = h.p.figures(value, h.p.bold(TypeRole::Label), h.p.ink(Role::Ink, true), x, y);
            h.words.append(&mut words); x += w + 6.0 * h.p.scale;
            x += h.text(unit, x, y, TypeRole::Caption, Role::Ink2);
        }
        x += 16.0;
        let graph_h = 24.0 * h.p.scale;
        h.graph(if t.live[1] { &frames.graph } else { &[] }, target, Frame::new(x, y + (row_h - graph_h) / 2.0, 111.0, graph_h), 28);
        (x + 111.0 + 28.0, row_h + 26.0)
    } else {
        let (left, right) = (32.0, 32.0 + col + 48.0);
        let gaps = frames.gaps;
        let drops: u64 = frames.dropped.iter().sum();
        let (state, tone) = if gaps.is_some_and(|g| g[3] >= 100_000) { ("Stalling", Role::Danger) }
            else if gaps.is_some_and(|g| g[2] as f32 > target * 1.5) || drops > 0 { ("Uneven", Role::Warning) } else { ("Even", Role::Positive) };
        let mut y = h.head("Frame timing", state, tone, left, 26.0, t.live[1]);
        let slots = ((col - 88.0 * h.p.scale + 1.0) / 4.0).floor() as usize;
        h.graph(if t.live[1] { &frames.graph } else { &[] }, target, Frame::new(left, y + 6.0, col, 72.0), slots);
        h.number(&format!("{:.1}", target / 1000.0), "ms", left + col, y + 48.0, TypeRole::Caption);
        y += 88.0;
        for (i, label) in ["p50", "p95", "p99", "max"].into_iter().enumerate() {
            let edge = left + col * (i + 1) as f32 / 4.0;
            let w = h.p.width(label, TypeRole::Caption);
            h.text(label, edge - w, y, TypeRole::Caption, Role::Ink2);
            let value = decimal(gaps.filter(|_| t.live[1]).map(|s| s[i] as f64 / 1000.0), 1);
            h.number(&value, if value == "–" { "" } else { "ms" }, edge, y + h.p.line(TypeRole::Caption), TypeRole::Caption);
        }
        y += h.p.line(TypeRole::Caption) * 2.0 + 8.0;
        h.row("Frame rate", &fps, "fps", left, &mut y, t.live[0]);
        h.row("Frames / dropped", &format!("{} / {drops}", frames.fed), "", left, &mut y, t.live[1]);
        if drops > 0 {
            let reason = ["full", "not ready", "refused", "after loss", "player error"].into_iter().zip(frames.dropped)
                .filter(|(_, n)| *n > 0).map(|(s, n)| format!("{n} {s}")).collect::<Vec<_>>().join(" · ");
            h.row("Why dropped", &reason, "", left, &mut y, t.live[1]);
        }
        h.row("Decoder queue", &frames.queue.map_or_else(|| "–".into(), |q| q.to_string()), "frames", left, &mut y, t.live[1]);
        h.row("Feed p50 / p95", &ms_pair(frames.feed, 0), "ms", left, &mut y, t.live[1]);
        h.row("Feed p99 / max", &ms_pair(frames.feed, 2), "ms", left, &mut y, t.live[1]);
        if frames.omitted > 0 { h.row("Samples omitted", &frames.omitted.to_string(), "", left, &mut y, t.live[1]); }
        y = h.rule(left, y);
        let pad = stream.pad.as_ref();
        let bursty = pad.is_some_and(|p| p.bursts > 0);
        y = h.head("Controller", if bursty { "Bursty" } else { "Even" }, if bursty { Role::Warning } else { Role::Positive }, left, y, t.live[3]);
        h.row("Pad", pad.map_or("–", |p| p.name.as_str()), "", left, &mut y, t.live[3]);
        h.row("Report rate", &decimal(pad.map(|p| f64::from(p.rate_hz)), 0), "Hz", left, &mut y, t.live[3]);
        h.row("Report gap p99", &decimal(pad.map(|p| f64::from(p.p99_us) / 1000.0), 1), "ms", left, &mut y, t.live[3]);
        let bursts = pad.map_or_else(|| "–".into(), |p| if p.bursts == 0 { "None".into() } else { format!("{} after {:.0} ms gaps", p.bursts, f64::from(p.burst_gap_us) / 1000.0) });
        h.row("Bursts", &bursts, "", left, &mut y, t.live[3]);
        let bad_link = stream.loss_pct.is_some_and(|v| v >= 0.5);
        let mut r = h.head("Stream", if bad_link { "Losing packets" } else { "Good" }, if bad_link { Role::Warning } else { Role::Positive }, right, 26.0, t.live[0]);
        let codec = m.view.stream.as_ref().map_or("–", |s| match s.codec { lumagame_core::engine::Codec::H264 => "H.264", lumagame_core::engine::Codec::H265 => "H.265", lumagame_core::engine::Codec::Av1 => "AV1", lumagame_core::engine::Codec::Other => "–" });
        let source = m.session().map_or("–", |s| m.engine_name(&s.source.engine));
        h.row("Source", &format!("{source} · {codec}"), "", right, &mut r, t.live[0]);
        for (label, value, unit) in [("Bitrate", stream.bitrate_kbps.map(|v| f64::from(v) / 1000.0), "Mb/s"),
            ("Round trip", stream.rtt_ms.map(f64::from), "ms"), ("Jitter", stream.jitter_ms.map(f64::from), "ms"),
            ("Packet loss", stream.loss_pct.map(f64::from), "%")] {
            h.row(label, &decimal(value, 1), unit, right, &mut r, t.live[0]);
        }
        if let Some(host) = stream.host_ms { h.row("Host encode, reported", &decimal(Some(f64::from(host)), 1), "ms", right, &mut r, t.live[0]); }
        // Counts since the session began, shown only where the engine keeps them. The second is the TV's own network stack discarding packets, not the network.
        if let Some(n) = stream.dropped { h.row("Frames dropped", &n.to_string(), "", right, &mut r, t.live[0]); }
        if let Some(n) = stream.socket_drops { h.row("Dropped by the TV", &n.to_string(), "packets", right, &mut r, t.live[0]); }
        r = h.rule(right, r);
        let refresh = &display["refresh"];
        let expected = number(&refresh["hz"]).map(|v| (v / 10.0).round() as u64);
        let fewest = refresh["fewest"].as_u64();
        let steady = fewest.zip(expected).is_some_and(|(v, e)| v >= e.saturating_sub(1));
        r = h.head("Display", if steady { "Steady" } else { "Uneven" }, if steady { Role::Positive } else { Role::Warning }, right, r, t.live[2] && fewest.is_some());
        let window = &display["window"];
        let signal = window["width"].as_u64().zip(window["height"].as_u64()).map_or_else(|| "–".into(), |(w, h)| format!("{w} × {h} · {}", match window["hdrType"].as_u64() { Some(0) => "SDR", Some(_) => "HDR", None => "–" }));
        h.row("Signal", &signal, "", right, &mut r, t.live[2]);
        h.row("Refresh", &hz, "Hz", right, &mut r, t.live[2]);
        let worst = fewest.zip(expected).map_or_else(|| "–".into(), |(f, e)| format!("{f} of {e} refreshes"));
        h.row("Worst 100 ms", &worst, "", right, &mut r, t.live[2]);
        let firmware = &display["firmware"]["sysVeri10"];
        let mut picture = vec![if window["lowDelay"] == true { "Low delay" } else { "Standard" }];
        if firmware["vrr"] == true { picture.push("VRR"); }
        if firmware["frc"] == true { picture.push("Smoothing"); }
        if window["muted"] == true { picture.push("Muted"); }
        h.row("Picture", &picture.join(" · "), "", right, &mut r, t.live[2]);
        h.row("Panel delay, reported", &decimal(number(&firmware["videoDelayUs"]).map(|v| v / 1000.0), 1), "ms", right, &mut r, t.live[2]);
        (col * 2.0 + 112.0, y.max(r) + 28.0)
    };
    let frame = Frame::new(W - h.p.x - width, h.p.y, width, height);
    #[cfg(test)]
    assert!(frame.y + frame.h <= super::paint::H - h.p.y, "HUD exceeds the safe height: {frame:?}");
    h.p.surface(LIVE.start + 10, frame, Fill::Sheet, if full { Radius::Card } else { Radius::Pill }, true, false, false, 20);
    h.p.words(LIVE.start + 11, frame, h.words, h.marks, 21);
    #[cfg(test)]
    super::paint::assert_readable(&h.p.plan, h.p.fonts, true);
}
