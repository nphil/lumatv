//! Settings › Controllers › Controller lab: the pad as the kernel reports it, its report timing, the stick settings the engines
//! apply, rumble, a flash test for a slow-motion video and, during a session, the stream's round trip as the engine measures it.

use super::{
    pages::nav,
    paint::{Fill, H, LIVE, Paint, W},
    settings::{Control, row},
};
use crate::game::model::{Capture, Focus, LabPad, Trip};
use compositor_protocol::Rect;
use lumagame_core::ipc::RoundTrip;
use lumatv_gfn_pad::{Curve, button, lab::EDGES_US, shape};
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{
    components::{Align, Icon, line_height, wrap},
    geometry::Frame,
    lucent::palette::PaletteRole as Role,
};

/// The reader's button bits in the order the chips show them.
const CHIPS: [(u16, &str); 14] = [
    (button::A, "A"),
    (button::B, "B"),
    (button::X, "X"),
    (button::Y, "Y"),
    (button::LEFT_SHOULDER, "LB"),
    (button::RIGHT_SHOULDER, "RB"),
    (button::LEFT_THUMB, "LS"),
    (button::RIGHT_THUMB, "RS"),
    (button::DPAD_UP, "Up"),
    (button::DPAD_DOWN, "Down"),
    (button::DPAD_LEFT, "Left"),
    (button::DPAD_RIGHT, "Right"),
    (button::BACK, "View"),
    (button::START, "Menu"),
];
const DISC: f32 = 200.0;
/// The sent dot's radius: the disc's edge is the end of its travel.
const DOT: f32 = 14.0;
const BAR_H: f32 = 12.0;
const HISTOGRAM_H: f32 = 112.0;
const CURVES: [Curve; 3] = [Curve::Linear, Curve::Precise, Curve::Quick];

pub fn lab(p: &mut Paint<'_>) {
    let m = p.model;
    if m.lab.capture == Some(Capture::Flash) {
        flash(p);
        return;
    }
    if m.lab.capture == Some(Capture::Buttons) {
        button_test(p);
        return;
    }
    nav(p, false);
    let (x, w) = (p.x, W - p.x * 2.0);
    let mut y = 170.0;
    p.text(100, "Controller lab", Frame::new(x + 26.0, y, w - 52.0, p.line(Type::Title)), Type::Title, Role::Ink, false, 10);
    y += p.line(Type::Title) + 24.0;
    let session = m.session_active();
    y = if session {
        stream(p, x, w, y)
    } else {
        let top = actions(p, x, w, y);
        pad(p, x, w, top)
    };
    y = sticks(p, x, w, y + 40.0);
    if !session {
        p.wrapped(
            995,
            "Stream timing: start a game, then open this lab from the playing controls.",
            Frame::new(x + 26.0, y + 24.0, w - 52.0, 0.0),
            Type::Caption,
            Role::Ink2,
            false,
            10,
        );
    }
}

/// The whole screen, dark until a button goes down and lit while any is held. Ids under 100 stay put when the page scrolls.
fn flash(p: &mut Paint<'_>) {
    let m = p.model;
    let press = m.lab.flash;
    let (fill, ink) = if press.is_some() { (Role::Ink, Role::Canvas) } else { (Role::Canvas, Role::Ink) };
    p.solid(40, Frame::new(0.0, 0.0, W, H), p.ink(fill, false), 0.0, 5);
    let (x, w) = (p.x + 26.0, W - p.x * 2.0 - 52.0);
    let mut y = 170.0;
    y += p.wrapped(41, "Flash test", Frame::new(x, y, w, 0.0), Type::Title, ink, false, 6) + 16.0;
    y += p.wrapped(
        42,
        "Film the controller and screen in slow motion. Press any button; the screen lights on the next frame.",
        Frame::new(x, y, w.min(1200.0), 0.0),
        Type::Body,
        ink,
        false,
        6,
    ) + 48.0;
    let colour = p.ink(ink, false);
    let display = p.ty(Type::Display);
    let (words, _) = p.figures(&format!("Frame {}", m.lab.frame), display, colour, 0.0, 0.0);
    let h = line_height(display).max(p.line(Type::Display));
    p.words(43, Frame::new(x, y, w, h), words, vec![], 6);
    y += h + 16.0;
    let stamp = press.map_or_else(|| "Waiting for a press".to_owned(), |ns| format!("Pressed at {:.3} ms", ns as f64 / 1e6));
    let (words, _) = p.figures(&stamp, p.ty(Type::Body), colour, 0.0, 0.0);
    p.words(44, Frame::new(x, y, w, p.line(Type::Body)), words, vec![], 6);
    let hw = p.width("Hold 1 s · Leave", Type::Label) + p.line(Type::Label) + 28.0;
    leave_hint(p, 45, W - p.x - hw, H - p.y - p.line(Type::Label) - 8.0, ink, fill);
}

fn leave_hint(p: &mut Paint<'_>, id: u64, x: f32, y: f32, ink: Role, fill: Role) {
    let side = p.line(Type::Label) + 8.0;
    p.solid(id, Frame::new(x, y, side, side), p.ink(ink, false), side / 2.0, 15);
    let mut button = p.word("B", Type::Label, p.ink(fill, false), 0.0, 4.0, side);
    button.align = Align::Centre;
    p.words(id + 1, Frame::new(x, y, side, side), vec![button], vec![], 16);
    let w = p.width("Hold 1 s · Leave", Type::Label);
    p.text(id + 2, "Hold 1 s · Leave", Frame::new(x + side + 20.0, y + 4.0, w, p.line(Type::Label)), Type::Label, ink, false, 16);
}

fn button_test(p: &mut Paint<'_>) {
    let (x, w) = (p.x + 26.0, W - p.x * 2.0 - 52.0);
    p.text(100, "Button test", Frame::new(x, 170.0, w, p.line(Type::Title)), Type::Title, Role::Ink, false, 10);
    let hw = p.width("Hold 1 s · Leave", Type::Label) + p.line(Type::Label) + 28.0;
    leave_hint(p, 320, W - p.x - hw, 170.0, Role::Ink, Role::Canvas);
    let y = 170.0 + p.line(Type::Title) + 16.0;
    let Some(pad) = p.model.lab.pad() else {
        p.wrapped(1003, "No controller connected", Frame::new(x, y, w, 0.0), Type::Body, Role::Ink2, false, 10);
        return;
    };
    let y = y + heading(p, 1001, &pad.name, x, y, w) + 24.0;
    let cell = 300.0;
    stick_view(p, pad, x, y, cell);
    buttons(p, pad, x + cell * 2.0 + 48.0, y, (w - cell * 2.0 - 48.0) / 2.0);
}

/// Actions stay separate from read-only sections so every reading can be reached at enlarged text sizes.
fn actions(p: &mut Paint<'_>, x: f32, w: f32, mut y: f32) -> f32 {
    let m = p.model;
    let h = p.tokens.target.max(p.line(Type::Label) + 24.0);
    let per_row = if m.prefs.text_percent > 100 { 2 } else { 4 };
    let mut bx = x + 26.0;
    for (i, label) in ["Test buttons", "Rumble left", "Rumble right", "Flash test"].into_iter().enumerate() {
        if i > 0 && i % per_row == 0 {
            bx = x + 26.0;
            y += h + 16.0;
        }
        bx += p.button(300 + i as u64 * 4, Focus::Lab(i), label, None, bx, y, false, false, 10) + 24.0;
    }
    y += h + 24.0;
    if let Some(notice) = &m.view.notice {
        y += marked(p, 101, Icon::Warning, Role::Warning, notice, Frame::new(x + 26.0, y, w - 52.0, 0.0)) + 24.0;
    }
    y
}

/// The card with the pad used last: sticks, triggers, buttons and report timing.
fn pad(p: &mut Paint<'_>, x: f32, w: f32, y: f32) -> f32 {
    let m = p.model;
    let (px, inner) = (x + 32.0, w - 64.0);
    let Some(pad) = m.lab.pad() else {
        let mut h = 32.0;
        h += heading(p, 1003, "No controller connected", px, y + h, inner) + 8.0;
        h += p.wrapped(
            1004,
            "Turn a controller on, or pair one in the TV settings: General › External Devices › Connect Bluetooth Controller.",
            Frame::new(px, y + h, inner, 0.0),
            Type::Caption,
            Role::Ink2,
            false,
            11,
        ) + 32.0;
        p.surface(1000, Frame::new(x, y, w, h), Fill::Tint(0.04), Radius::Card, false, false, false, 10);
        return y + h;
    };
    let mut top = y + 32.0;
    top += heading(p, 1001, &pad.name, px, top, inner);
    let connected = m.lab.connected();
    if connected > 1 {
        let text = format!("{connected} controllers connected; this is the one used last.");
        top += p.wrapped(1002, &text, Frame::new(px, top, inner, 0.0), Type::Caption, Role::Ink2, false, 11);
    }
    top += 24.0;
    // Side by side at the standard text size; larger text stacks the timing under the controls.
    let wide = m.prefs.text_percent <= 100;
    let cell = if wide { 280.0 } else { inner / 2.0 };
    let stick_end = stick_view(p, pad, px, top, cell);
    reading(p, 1300, Focus::Lab(5), Frame::new(px, top, cell * 2.0, stick_end - top));
    let button_top = stick_end + 24.0;
    let left = buttons(p, pad, px, button_top, cell);
    reading(p, 1303, Focus::Lab(6), Frame::new(px, button_top, cell * 2.0, left - button_top));
    let (rx, ry, rw) = if wide { (px + cell * 2.0 + 48.0, top, inner - cell * 2.0 - 48.0) } else { (px, left + 40.0, inner) };
    let right = timing(p, pad, rx, ry, rw);
    reading(p, 1306, Focus::Lab(7), Frame::new(rx, ry, rw, right - ry));
    let bottom = left.max(right) + 32.0;
    p.surface(1000, Frame::new(x, y, w, bottom - y), Fill::Tint(0.04), Radius::Card, false, false, false, 10);
    bottom
}

fn reading(p: &mut Paint<'_>, id: u64, focus: Focus, f: Frame) {
    if p.focused(focus) {
        p.land(id, f, p.ink(Role::Ink, false), 24.0, 0.0, 14);
    }
    p.hit(focus, f);
}

fn stick_view(p: &mut Paint<'_>, pad: &LabPad, x: f32, y: f32, cell: f32) -> f32 {
    let shaping = p.model.prefs.shaping();
    let travel = DISC / 2.0 - DOT;
    let (body, caption) = (p.line(Type::Body), p.line(Type::Caption));
    let axis = p.colour(Fill::Tint(0.12), false);
    for (i, (label, raw)) in [("Left stick", pad.report.left), ("Right stick", pad.report.right)].into_iter().enumerate() {
        let id = 1100 + i as u64 * 20;
        let live = LIVE.start + i as u64 * 10;
        let cx = x + cell * i as f32 + cell / 2.0;
        let cy = y + DISC / 2.0;
        p.surface(id, Frame::new(cx - DISC / 2.0, y, DISC, DISC), Fill::Tint(0.06), Radius::Pill, false, false, false, 11);
        let dz = f32::from(shaping.deadzone_percent) / 100.0 * travel;
        if dz >= 1.0 {
            p.surface(id + 1, Frame::new(cx - dz, cy - dz, dz * 2.0, dz * 2.0), Fill::Tint(0.12), Radius::Pill, false, false, false, 12);
        }
        p.solid(id + 2, Frame::new(cx - DISC / 2.0 + 12.0, cy - 1.0, DISC - 24.0, 2.0), axis, 1.0, 12);
        p.solid(id + 3, Frame::new(cx - 1.0, y + 12.0, 2.0, DISC - 24.0), axis, 1.0, 12);
        let (sx, sy) = shape(f64::from(raw.0), f64::from(raw.1), shaping);
        let (sx, sy) = unit(f32::from(sx) / 32767.0, f32::from(sy) / 32767.0);
        p.solid(live, Frame::new(cx + sx * travel - DOT, cy - sy * travel - DOT, DOT * 2.0, DOT * 2.0), p.ink(Role::Accent, false), DOT, 13);
        let (rx, ry) = unit(raw.0, raw.1);
        p.solid(live + 1, Frame::new(cx + rx * travel - 6.0, cy - ry * travel - 6.0, 12.0, 12.0), p.ink(Role::Ink, false), 6.0, 14);
        let drift = &pad.drift[i];
        let reading = drift.rest.map_or_else(|| "Drift —".to_owned(), |rest| format!("Drift {:.1}%", rest * 100.0));
        let worst = if drift.rest.is_some() || drift.worst > 0.0 { format!("Worst {:.1}%", drift.worst * 100.0) } else { String::new() };
        let mut word = p.word(label, Type::Body, p.ink(Role::Ink, false), 0.0, 0.0, cell);
        word.align = Align::Centre;
        let mut words = vec![word];
        for (line, text) in [reading, worst].into_iter().enumerate() {
            let (mut figures, width) = p.figures(&text, p.ty(Type::Caption), p.ink(Role::Ink2, false), 0.0, body + line as f32 * caption);
            for figure in &mut figures {
                figure.x += (cell - width) / 2.0;
            }
            words.extend(figures);
        }
        // Three lines are reserved so the layer keeps its size as the readings come and go.
        p.words(id + 4, Frame::new(x + cell * i as f32, y + DISC + 16.0, cell, body + caption * 2.0), words, vec![], 11);
    }
    let y = y + DISC + 16.0 + body + caption * 2.0 + 16.0;
    y + p.wrapped(
        1140,
        "Small dot: raw. Large dot: sent.",
        Frame::new(x, y, cell * 2.0, 0.0),
        Type::Caption,
        Role::Ink2,
        false,
        11,
    )
}

fn buttons(p: &mut Paint<'_>, pad: &LabPad, x: f32, mut y: f32, cell: f32) -> f32 {
    let caption = p.line(Type::Caption);
    for (i, (label, value)) in [("Left trigger", pad.report.left_trigger), ("Right trigger", pad.report.right_trigger)].into_iter().enumerate() {
        let id = 1200 + i as u64 * 10;
        let (bx, bw) = (x + cell * i as f32 + 20.0, cell - 40.0);
        let percent = (u32::from(value) * 100 + 127) / 255;
        let (mut words, width) = p.figures(&format!("{percent}%"), p.ty(Type::Caption), p.ink(Role::Ink, false), 0.0, 0.0);
        for word in &mut words {
            word.x += bw - width;
        }
        words.insert(0, p.word(label, Type::Caption, p.ink(Role::Ink2, false), 0.0, 0.0, bw - width - 8.0));
        p.words(id, Frame::new(bx, y, bw, caption), words, vec![], 11);
        let track = Rect { x: bx, y: y + caption + 8.0, w: bw, h: BAR_H };
        p.solid(id + 1, Frame::new(track.x, track.y, bw, BAR_H), p.ink(Role::TrackOff, false), BAR_H / 2.0, 12);
        // The fill keeps its size and slides under a clip that starts after the round cap, so a moving trigger restyles no layer.
        p.solid(id + 2, Frame::new(track.x, track.y, BAR_H, BAR_H), p.ink(if value > 0 { Role::Accent } else { Role::TrackOff }, false), BAR_H / 2.0, 13);
        let fill = bw * f32::from(value) / 255.0;
        p.solid(LIVE.start + 100 + i as u64, Frame::new(track.x - bw + fill, track.y, bw, BAR_H), p.ink(Role::Accent, false), BAR_H / 2.0, 13);
        clip(p, Rect { x: track.x + BAR_H / 2.0, w: bw - BAR_H / 2.0, ..track });
    }
    y += caption + 8.0 + BAR_H + 32.0;
    let ch = p.line(Type::Label) + 16.0;
    let mut cx = x;
    for (i, (bit, label)) in CHIPS.into_iter().enumerate() {
        let cw = p.width(label, Type::Label) + 40.0;
        if cx + cw > x + cell * 2.0 && cx > x {
            cx = x;
            y += ch + 10.0;
        }
        let on = pad.report.buttons & bit != 0;
        let f = Frame::new(cx, y, cw, ch);
        p.surface(1500 + i as u64 * 2, f, if on { Fill::Accent } else { Fill::Tint(0.06) }, Radius::Pill, false, false, false, 12);
        let mut word = p.word(label, Type::Label, p.ink(if on { Role::AccentInk } else { Role::Ink2 }, false), 0.0, 8.0, cw);
        word.align = Align::Centre;
        p.words(1501 + i as u64 * 2, f, vec![word], vec![], 13);
        cx += cw + 10.0;
    }
    y + ch
}

/// Report timing from the kernel's stamps over the last few seconds of movement. Returns the bottom.
fn timing(p: &mut Paint<'_>, pad: &LabPad, x: f32, mut y: f32, w: f32) -> f32 {
    let s = &pad.summary;
    y += heading(p, 2000, "Report timing · 3 seconds", x, y, w);
    if s.reports < 2 {
        let text = "Move a stick in circles to measure: a controller held still sends nothing new.";
        return y + p.wrapped(2122, text, Frame::new(x, y, w, 0.0), Type::Body, Role::Ink2, false, 11);
    }
    let title = p.ty(Type::Title);
    let th = line_height(title).max(p.line(Type::Title));
    let (mut words, width) = p.figures(&format!("{:.0}", s.rate_hz), title, p.ink(Role::Ink, false), 0.0, 0.0);
    words.push(p.word("reports/s · intervals in ms", Type::Body, p.ink(Role::Ink2, false), width + 12.0, p.line(Type::Title) - p.line(Type::Body), w - width - 12.0));
    p.words(2002, Frame::new(x, y, w, th), words, vec![], 11);
    y += th + 16.0;
    let cols = 3;
    let cw = w / cols as f32;
    let (body, caption) = (p.line(Type::Body), p.line(Type::Caption));
    let cells = [
        ("Median", ms(s.p50_us)),
        ("95th percentile", ms(s.p95_us)),
        ("99th percentile", ms(s.p99_us)),
        ("Longest", ms(s.max_us)),
        ("Jitter", ms(s.jitter_us)),
        ("Reports", s.reports.to_string()),
    ];
    for (j, (label, value)) in cells.iter().enumerate() {
        let (gx, gy) = (x + (j % cols) as f32 * cw, y + (j / cols) as f32 * (caption + body + 16.0));
        p.text(2010 + j as u64 * 2, label, Frame::new(gx, gy, cw - 16.0, caption), Type::Caption, Role::Ink2, false, 11);
        let (words, _) = p.figures(value, p.ty(Type::Body), p.ink(Role::Ink, false), 0.0, 0.0);
        p.words(2011 + j as u64 * 2, Frame::new(gx, gy + caption, cw - 16.0, body), words, vec![], 11);
    }
    y += cells.len().div_ceil(cols) as f32 * (caption + body + 16.0) + 8.0;
    let gap = 8.0;
    let bw = (w - gap * 8.0) / 9.0;
    let base = y + HISTOGRAM_H;
    let most = s.histogram.iter().copied().max().unwrap_or(0).max(1);
    for (i, &count) in s.histogram.iter().enumerate() {
        let bx = x + i as f32 * (bw + gap);
        // A full-height bar slid down and cut at the baseline: its height never changes, so a new reading restyles no layer.
        let rise = HISTOGRAM_H * count as f32 / most as f32;
        p.solid(LIVE.start + 200 + i as u64, Frame::new(bx, base - rise, bw, HISTOGRAM_H), p.ink(Role::Accent, false), 4.0, 12);
        clip(p, Rect { x: bx, y, w: bw, h: HISTOGRAM_H });
        let label = match EDGES_US.get(i) {
            Some(&edge) => format!("<{}", ms_edge(edge)),
            None => format!("{}+", ms_edge(EDGES_US[EDGES_US.len() - 1])),
        };
        let mut word = p.word(label, Type::Caption, p.ink(Role::Ink2, false), 0.0, 0.0, bw + gap);
        word.align = Align::Centre;
        p.words(2110 + i as u64, Frame::new(bx - gap / 2.0, base + 8.0, bw + gap, caption), vec![word], vec![], 11);
    }
    p.solid(2100, Frame::new(x, base, w, 2.0), p.colour(Fill::Tint(0.12), false), 1.0, 12);
    y = base + 8.0 + caption + 16.0;
    let bursts = s.bursts;
    let f = Frame::new(x, y, w, 0.0);
    y + if bursts.count > 0 {
        let what = if bursts.count == 1 { "1 burst".to_owned() } else { format!("{} bursts", bursts.count) };
        let text = format!(
            "{what} · {} reports after {} gaps. Compare USB.",
            bursts.size,
            ms(bursts.gap_us)
        );
        marked(p, 2121, Icon::Warning, Role::Warning, &text, f)
    } else {
        marked(p, 2121, Icon::Check, Role::Positive, "No burst pattern in this window.", f)
    }
}

/// During a session: what the engine measured of the stream's round trip. Returns the bottom.
fn stream(p: &mut Paint<'_>, x: f32, w: f32, mut y: f32) -> f32 {
    let m = p.model;
    let (tx, tw) = (x + 26.0, w - 52.0);
    y += p.wrapped(
        800,
        "The game has the controllers while it runs; leave it to test them here.",
        Frame::new(tx, y, tw, 0.0),
        Type::Body,
        Role::Ink2,
        false,
        10,
    ) + 32.0;
    y += heading(p, 801, "Stream timing", tx, y, tw) + 16.0;
    let engine = m.session().and_then(|s| m.integration(&s.source.engine)).map_or("The game's integration", |e| e.manifest.name.as_str());
    let measurable = m.round_trip_engine().is_some() && !matches!(m.lab.trip, Trip::Unsupported(_));
    let line = p.line(Type::Body);
    if !measurable {
        y += p.wrapped(813, &format!("Not supported by {engine}."), Frame::new(tx, y, tw, 0.0), Type::Body, Role::Ink, false, 10) + 16.0;
    } else {
        let h = p.tokens.target.max(p.line(Type::Label) + 24.0);
        let bw = p.button(810, Focus::Lab(4), "Measure", Some(Icon::Timer), tx, y, false, false, 10);
        let status = match &m.lab.trip {
            Trip::Waiting(_) => format!("Waiting for {engine}"),
            Trip::Answer(..) => format!("As {engine} measured it"),
            _ => format!("Asks {engine} for its timings of the last presses and frames"),
        };
        p.text(813, &status, Frame::new(tx + bw + 24.0, y + (h - line) / 2.0, tw - bw - 24.0, line), Type::Body, Role::Ink2, false, 10);
        y += h + 24.0;
        if let Trip::Answer(_, trip) = &m.lab.trip {
            y = answer(p, trip, x, w, y);
        }
    }
    y + p.wrapped(
        990,
        "The TV decoder and panel are not measured here. A flash-test video measures physical press-to-light, without stream decoding.",
        Frame::new(tx, y + 8.0, tw, 0.0),
        Type::Caption,
        Role::Ink2,
        false,
        10,
    ) + 8.0
}

fn answer(p: &mut Paint<'_>, trip: &RoundTrip, x: f32, w: f32, mut y: f32) -> f32 {
    let pair = |a: Option<f32>, b: Option<f32>, name: &str| a.map(|a| b.map_or_else(|| ms_f(a), |b| format!("{}  ·  {name} {}", ms_f(a), ms_f(b))));
    let presses = trip.presses.map(|n| format!("Median of the last {n} presses, from the controller's report until it leaves the TV"));
    let rows = [
        ("Press to send", pair(trip.press_p50_ms, trip.press_p99_ms, "99th"), presses),
        ("Network round trip", trip.network_ms.map(ms_f), None),
        ("Capture and encode", trip.host_ms.map(ms_f), Some("On the host".to_owned())),
        ("Frame arrival", pair(trip.arrival_p50_ms, trip.arrival_p95_ms, "95th"), Some("From capture on the host until the frame is here".to_owned())),
        ("Decoder hand-off", trip.client_ms.map(ms_f), Some("From a frame's last packet until the decoder has it".to_owned())),
    ];
    for (i, (label, value, description)) in rows.iter().enumerate() {
        let control = value.as_deref().map_or(Control::Value("Not reported"), Control::Figure);
        y += row(p, 820 + i as u64 * 30, Focus::Lab(8 + i), label, description.as_deref(), control, Frame::new(x, y, w, 0.0));
    }
    y + 8.0
}

/// The stick settings the engines apply. Returns the bottom.
fn sticks(p: &mut Paint<'_>, x: f32, w: f32, mut y: f32) -> f32 {
    let m = p.model;
    let (tx, tw) = (x + 26.0, w - 52.0);
    y += heading(p, 590, "Sticks", tx, y, tw);
    let engines = m
        .integrations
        .iter()
        .filter(|e| m.enabled(&e.manifest.id) && e.schema.fields().any(|f| f.key == "stick-curve"))
        .map(|e| e.manifest.name.as_str())
        .collect::<Vec<_>>();
    let applies = match engines.as_slice() {
        [] => "No integration that is switched on applies these yet.".to_owned(),
        [one] => format!("{one} applies these from the next game."),
        [rest @ .., last] => format!("{} and {last} apply these from the next game.", rest.join(", ")),
    };
    y += p.wrapped(591, &applies, Frame::new(tx, y, tw, 0.0), Type::Caption, Role::Ink2, false, 10) + 16.0;
    let prefs = &m.prefs;
    y += row(
        p,
        600,
        Focus::Setting(0),
        "Dead zone",
        Some("How far a stick moves before games see it"),
        Control::Figure(&format!("{}%", prefs.deadzone)),
        Frame::new(x, y, w, 0.0),
    );
    y += row(
        p,
        630,
        Focus::Setting(1),
        "Anti-deadzone",
        Some("The least movement games see once a stick leaves the dead zone, for games that add a dead zone of their own"),
        Control::Figure(&format!("{}%", prefs.anti_deadzone)),
        Frame::new(x, y, w, 0.0),
    );
    let curve = CURVES.iter().position(|c| *c == prefs.stick_curve).unwrap_or(0);
    y += row(
        p,
        660,
        Focus::Setting(2),
        "Response curve",
        Some("Precise slows the first part of the travel for fine aim; Quick speeds it up"),
        Control::Segments(&["Linear", "Precise", "Quick"], curve),
        Frame::new(x, y, w, 0.0),
    );
    let worst = m.lab.pad().filter(|_| !m.session_active()).map_or(0.0, |pad| pad.drift.iter().map(|d| d.worst).fold(0.0, f32::max));
    if worst * 100.0 > f32::from(prefs.deadzone) {
        let text = format!("A stick came to rest {:.1}% off centre, past the {}% dead zone: games may see it drift.", worst * 100.0, prefs.deadzone);
        y += marked(p, 690, Icon::Warning, Role::Warning, &text, Frame::new(tx, y + 16.0, tw, 0.0)) + 16.0;
    }
    y
}

/// A bold single line. Returns its line height.
fn heading(p: &mut Paint<'_>, id: u64, text: &str, x: f32, y: f32, w: f32) -> f32 {
    let ty = p.bold(Type::Body);
    let line = p.line(Type::Body);
    let mut word = p.word(text, Type::Body, p.ink(Role::Ink, false), 0.0, 0.0, w);
    word.ty = ty;
    p.words(id, Frame::new(x, y, w, line_height(ty).max(line)), vec![word], vec![], 11);
    line
}

/// A caption wrapped beside a leading icon, as one layer. Returns its height.
fn marked(p: &mut Paint<'_>, id: u64, icon: Icon, role: Role, text: &str, f: Frame) -> f32 {
    let side = 26.0 * p.scale;
    let line = p.line(Type::Caption);
    let ty = p.ty(Type::Caption);
    let lines = wrap(p.fonts, text, ty, f.w - side - 12.0);
    let colour = p.ink(Role::Ink2, false);
    let words = lines.into_iter().enumerate().map(|(i, l)| p.word(l, Type::Caption, colour, side + 12.0, i as f32 * line, f.w - side - 12.0)).collect::<Vec<_>>();
    let h = words.len().max(1) as f32 * line;
    let mark = p.mark(icon, side, 0.0, (line - side) / 2.0, role, false);
    p.words(id, Frame::new(f.x, f.y, f.w, h - line + line_height(ty).max(line)), words, vec![mark], 11);
    h
}

fn clip(p: &mut Paint<'_>, rect: Rect) {
    if let Some(node) = p.plan.nodes.last_mut() {
        node.props.clip = Some(rect);
    }
}

/// A point past the unit circle, pulled back onto it: the disc's edge is the end of the travel.
fn unit(x: f32, y: f32) -> (f32, f32) {
    let mag = x.hypot(y);
    if mag > 1.0 { (x / mag, y / mag) } else { (x, y) }
}

fn ms(us: u32) -> String {
    ms_f(us as f32 / 1000.0)
}

fn ms_f(ms: f32) -> String {
    if ms < 10.0 {
        format!("{ms:.2} ms")
    } else if ms < 100.0 {
        format!("{ms:.1} ms")
    } else {
        format!("{ms:.0} ms")
    }
}

/// A histogram edge in milliseconds, as short as it reads: 0.5, 1, 64.
fn ms_edge(us: u32) -> String {
    if us % 1000 == 0 { (us / 1000).to_string() } else { format!("{}", us as f32 / 1000.0) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{helper::gamesir_fixture, model::{Model, Page}};
    use lumagame_core::{catalog::Listing, state::State};
    use lumatv_tokens::Tokens;
    use ui::lucent::palette::Mode;

    #[test]
    fn readings_capture_and_hardware_rows_fit_at_both_text_sizes() {
        let host = Tokens::shipped();
        let mut fonts = ui::Fonts::new();
        let mut m = Model::new(State::default(), Listing::default(), vec![]);
        let leadjoy = ["general", "profiles", "buttons", "macros", "sticks", "curves", "directions", "triggers", "motion", "gestures", "rumble"].map(crate::game::helper::leadjoy_fixture);
        let mut pad = LabPad::default();
        pad.name = "GameSir Cyclone 2".into();
        pad.summary.reports = 32;
        pad.summary.rate_hz = 500.0;
        pad.report.left_trigger = 158;
        pad.drift[0].rest = Some(0.2);
        pad.drift[0].worst = 0.2;
        m.lab.pads[0] = Some(pad);
        m.gamesir.state = gamesir_fixture();
        for percent in [100, 200] {
            m.prefs.text_percent = percent;
            for mode in [Mode::Dark, Mode::Light] {
                let tokens = lumatv_tokens::themes::game_tokens("rose-pine", mode, &host).unwrap();
                m.view.page = Page::Lab;
                m.lab.capture = None;
                for focus in [Focus::Lab(5), Focus::Lab(6), Focus::Lab(7), Focus::Setting(0), Focus::Setting(1), Focus::Setting(2)] {
                    m.view.focus = focus;
                    let mut p = Paint::new(&tokens, &m, &mut fonts);
                    lab(&mut p);
                    p.fit_focus();
                    let (_, frame) = p.plan.hits.iter().find(|(id, _)| *id == focus).unwrap();
                    assert!(frame.y >= 150.0 && frame.y + frame.h <= H - p.y + 1.0, "unreachable {percent}% {focus:?}: {frame:?}");
                    super::super::paint::assert_readable(&p.plan, p.fonts, false);
                }
                for capture in [Capture::Buttons, Capture::Flash] {
                    m.lab.capture = Some(capture);
                    m.view.focus = Focus::Lab(0);
                    let mut p = Paint::new(&tokens, &m, &mut fonts);
                    lab(&mut p);
                    super::super::paint::assert_readable(&p.plan, p.fonts, true);
                }
                m.lab.capture = None;
                for state in std::iter::once(None).chain(leadjoy.iter().map(Some)) {
                    m.view.page = if state.is_some() { Page::Leadjoy } else { Page::GameSir };
                    if let Some(state) = state { m.leadjoy.state = state.clone(); }
                    m.rebuild_navigation();
                    let rows: Vec<Focus> = m.navigation.iter().map(|n| n.id).filter(|id| matches!(id, Focus::Setting(_))).collect();
                    assert!(rows.len() > 3, "{:?} lists its settings", m.view.page);
                    let last = rows[rows.len() - 1];
                    for focus in rows {
                        m.view.focus = focus;
                        let mut p = Paint::new(&tokens, &m, &mut fonts);
                        if state.is_none() { super::super::settings::gamesir(&mut p); } else { super::super::settings::leadjoy(&mut p); }
                        p.fit_focus();
                        let (_, frame) = p.plan.hits.iter().find(|(id, _)| *id == focus).unwrap();
                        assert!(frame.y >= 150.0 && frame.y + frame.h <= H - p.y + 1.0, "unreachable {percent}% {focus:?}: {frame:?}");
                        let end = p.plan.nodes.iter().map(|n| n.frame.y + n.frame.h).fold(0.0, f32::max);
                        assert!(focus != last || end <= H - p.y + 1.0, "closing note cut off at {percent}%: {end}");
                        super::super::paint::assert_readable(&p.plan, p.fonts, false);
                    }
                }
            }
        }
    }
}
