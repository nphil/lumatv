//! A labelled row with its control: links, values, switches and steppers.
use crate::game::screens::paint::{Fill, Paint};
use crate::game::{model::Focus, uplink::BITRATE_MAX_MBPS};
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{components::Align, geometry::Frame, lucent::palette::{Mode, PaletteRole as Role}};

#[derive(Clone, Copy)]
pub enum Control<'a> {
    Value(&'a str),
    /// A value that changes in place, in tabular figures.
    Figure(&'a str),
    Segments(&'a [&'a str], usize),
    Toggle(bool),
    Slider(u32),
    None,
}
fn row_metrics(p: &mut Paint<'_>, label: &str, description: Option<&str>, width: f32, control_w: f32, control_h: f32, rows: usize) -> (bool, f32, f32, f32, f32) {
    let available = width - 52.0;
    let stack = (p.width(label, Type::Body) + control_w + 80.0 > width || rows > 1) && control_w > 0.0;
    let text_w = if stack || control_w == 0.0 { available } else { available - control_w - 32.0 };
    let label_h = ui::components::wrap(p.fonts, label, p.ty(Type::Body), text_w).len().max(1) as f32 * p.line(Type::Body);
    let desc_h = description.map_or(0.0, |s| ui::components::wrap(p.fonts, s, p.ty(Type::Caption), text_w).len() as f32 * p.line(Type::Caption));
    let height = if stack { label_h + desc_h + control_h + 40.0 } else { 80.0f32.max(label_h + desc_h + 24.0).max(control_h + 16.0) };
    (stack, text_w, label_h, desc_h, height)
}

pub fn row(p: &mut Paint<'_>, id: u64, focus: Focus, label: &str, description: Option<&str>, control: Control<'_>, mut f: Frame) -> f32 {
    // An empty value would become a text layer with no width.
    let control = if let Control::Value("") = control { Control::None } else { control };
    let available = f.w - 52.0;
    let line = p.line(Type::Body);
    let sh = p.tokens.target.max(p.line(Type::Label) + 12.0);
    let widths = if let Control::Segments(options, _) = &control {
        options.iter().map(|s| (p.width(s, Type::Label) + 44.0).min(available - 8.0)).collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let slider_value = if let Control::Slider(value) = &control { format!("{value} Mb/s") } else { String::new() };
    let value_w = p.width(&slider_value, Type::Label) + 12.0;
    let figure = if let Control::Figure(value) = &control { p.figures(value, p.ty(Type::Body), p.ink(Role::Ink2, false), 0.0, 0.0) } else { Default::default() };
    let control_w = match &control {
        Control::Value(value) => p.width(value, Type::Body),
        Control::Figure(_) => figure.1,
        Control::Segments(..) => widths.iter().sum::<f32>() + 8.0,
        Control::Toggle(_) => 82.0,
        Control::Slider(_) => value_w + 284.0,
        Control::None => 0.0,
    }
    .min(available);
    let mut rows = 1;
    let mut used = 8.0;
    for width in &widths {
        if used + width > control_w && used > 8.0 {
            rows += 1;
            used = 8.0;
        }
        used += width;
    }
    let control_h = if matches!(control, Control::Segments(..)) { rows as f32 * sh + (rows - 1) as f32 * 8.0 } else { sh };
    let (stack, text_w, label_h, desc_h, height) = row_metrics(p, label, description, f.w, control_w, control_h, rows);
    f.h = height;
    let focused = p.focused(focus);
    let from = p.plan.nodes.len();
    if focused {
        p.surface(id, f, Fill::Tint(p.tokens.states.focus_wash[usize::from(p.mode == Mode::Light)]), Radius::Row, false, true, false, 10);
    }
    let text_y = if stack { 12.0 } else { (f.h - label_h - desc_h) / 2.0 };
    p.wrapped(id + 1, label, Frame::new(f.x + 26.0, f.y + text_y, text_w, 0.0), Type::Body, Role::Ink, false, 11);
    if let Some(description) = description {
        p.wrapped(id + 18, description, Frame::new(f.x + 26.0, f.y + text_y + label_h, text_w, 0.0), Type::Caption, Role::Ink2, false, 11);
    }
    let x = f.x + f.w - control_w - 26.0;
    let cy = if stack { f.y + label_h + desc_h + 24.0 } else { f.y + (f.h - control_h) / 2.0 };
    match control {
        Control::Value(value) => {
            let mut word = p.word(value, Type::Body, p.ink(Role::Ink2, false), 0.0, 0.0, control_w);
            word.align = Align::Right;
            p.words(id + 2, Frame::new(x, if stack { cy } else { f.y + (f.h - line) / 2.0 }, control_w, line), vec![word], vec![], 12);
        }
        Control::Figure(_) => {
            p.words(id + 2, Frame::new(x, if stack { cy } else { f.y + (f.h - line) / 2.0 }, control_w, line), figure.0, vec![], 12);
        }
        Control::Toggle(on) => p.toggle(id + 2, x, if stack { cy } else { f.y + (f.h - 48.0) / 2.0 }, on, false, 12),
        Control::Slider(value) => {
            let track = Frame::new(x, cy + (control_h - 6.0) / 2.0, (control_w - value_w - 24.0).max(1.0), 6.0);
            p.solid(id + 2, track, p.ink(Role::TrackOff, false), 3.0, 12);
            let fill = track.w * value as f32 / BITRATE_MAX_MBPS as f32;
            p.solid(id + 3, Frame::new(track.x, track.y, fill, 6.0), p.ink(Role::Accent, false), 3.0, 13);
            p.solid(id + 4, Frame::new(track.x + fill - 10.0, track.y - 7.0, 20.0, 20.0), p.ink(Role::Ink, false), 10.0, 14);
            p.text(
                id + 5,
                &slider_value,
                Frame::new(x + control_w - value_w, cy + (control_h - p.line(Type::Label)) / 2.0, value_w, p.line(Type::Label)),
                Type::Label,
                Role::Ink2,
                false,
                12,
            );
        }
        Control::Segments(options, selected) => {
            let mut sx = x + 4.0;
            let mut sy = cy;
            for (i, (label, width)) in options.iter().zip(widths).enumerate() {
                if sx + width > x + control_w && sx > x + 4.0 {
                    sx = x + 4.0;
                    sy += sh + 8.0;
                }
                let node = 100_000_000 + id * 256 + i as u64 * 2;
                p.surface(
                    node,
                    Frame::new(sx, sy + 4.0, width, sh - 8.0),
                    if i == selected { Fill::Accent } else { Fill::Tint(0.05) },
                    Radius::Pill,
                    false,
                    false,
                    false,
                    13,
                );
                let mut word = p.word(
                    *label,
                    Type::Label,
                    p.ink(if i == selected { Role::AccentInk } else { Role::Ink2 }, false),
                    0.0,
                    (sh - p.line(Type::Label)) / 2.0,
                    width,
                );
                word.align = Align::Centre;
                p.words(node + 1, Frame::new(sx, sy, width, sh), vec![word], vec![], 14);
                sx += width;
            }
        }
        Control::None => {}
    }
    if focused {
        p.zoom(from, f, p.tokens.focus.card_scale);
    }
    if focus != Focus::None {
        p.hit(focus, f);
    }
    f.h + 4.0
}
