use super::{
    pages::nav,
    paint::{Fill, H, Paint, W},
};
use crate::game::model::{Focus, Section};
use crate::game::helper::GameSirState;
use lumagame_core::{
    engine::{Codec, EngineKind, Feature},
    ipc::{Link, Reach},
    settings::{FieldKind, Value},
};
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{
    components::{Align, Form, Icon},
    geometry::Frame,
    lucent::palette::{Mode, PaletteRole as Role},
};

fn rail(p: &mut Paint<'_>) -> f32 {
    let icons = [Icon::Palette, Icon::Cards, Icon::Puzzle, Icon::Apps, Icon::Info, Icon::Info];
    let side = 36.0 * p.scale;
    let mut w = 420.0f32;
    for section in Section::ALL {
        w = w.max(p.width(section.name(), Type::Body) + side + 80.0);
    }
    w = w.min(640.0);
    p.text(100, "Settings", Frame::new(p.x + 28.0, 170.0, w - 56.0, p.line(Type::Title)), Type::Title, Role::Ink, false, 10);
    let top = 170.0 + p.line(Type::Title) + 24.0;
    let h = 80.0f32.max(p.line(Type::Body) + 24.0);
    for (i, section) in Section::ALL.into_iter().enumerate() {
        let f = Frame::new(p.x, top + i as f32 * (h + 4.0), w, h);
        let from = p.plan.nodes.len();
        if p.model.view.section == section || p.focused(Focus::Rail(i)) {
            p.surface(110 + i as u64 * 3, f, Fill::Tint(if p.focused(Focus::Rail(i)) { 0.16 } else { 0.09 }), Radius::Row, false, false, false, 10);
        }
        let marks = if i == 3 { Vec::new() } else { vec![p.mark(icons[i], side, 28.0, (h - side) / 2.0, Role::Ink2, false)] };
        p.words(
            111 + i as u64 * 3,
            f,
            vec![p.word(section.name(), Type::Body, p.ink(Role::Ink, false), side + 52.0, (h - p.line(Type::Body)) / 2.0, w - side - 72.0)],
            marks,
            11,
        );
        if i == 3 {
            p.symbol(
                112 + i as u64 * 3,
                super::symbols::Symbol::Controller,
                Frame::new(f.x + 28.0, f.y + (h - side) / 2.0, side, side),
                p.ink(Role::Ink2, false),
                11,
            );
        }
        if p.focused(Focus::Rail(i)) {
            p.zoom(from, f, p.tokens.focus.card_scale);
        }
        p.hit(Focus::Rail(i), f);
    }
    p.x + w + 48.0
}
pub fn settings(p: &mut Paint<'_>) {
    nav(p, false);
    let x = rail(p);
    let w = W - p.x - x;
    match p.model.view.section {
        Section::Appearance => appearance(p, x, w),
        Section::Streaming => streaming(p, x, w),
        Section::Integrations => integrations(p, x, w),
        Section::Controllers => controllers(p, x, w),
        Section::Diagnostics => diagnostics(p, x, w),
        Section::About => about(p, x, w),
    }
}
pub fn theme_card(p: &mut Paint<'_>, id: u64, focus: Focus, index: usize, f: Frame) {
    let option = &lumatv_tokens::themes::game_options()[index];
    let selected = p.model.prefs.family == option.id;
    let focused = p.focused(focus);
    let from = p.plan.nodes.len();
    p.surface(id, f, Fill::Glass, Radius::Tile, false, focused, false, 31);
    if selected {
        let edge = p.plan.nodes.last_mut().unwrap();
        if let super::paint::Body::Surface(glass) = &mut edge.body {
            glass.edge = p.tokens.roles(p.mode)[Role::Accent].premultiply(1.0);
            glass.edge_width = 3.0;
        }
    }
    let preview = Frame::new(f.x + 14.0, f.y + 14.0, f.w - 28.0, 128.0);
    let roles = lumatv_tokens::themes::game_find(option.id).map_or(p.tokens.roles(p.mode), |family| family.roles(p.mode));
    p.solid(id + 1, preview, roles[Role::Canvas].premultiply(1.0), p.tokens.radius(Radius::Control), 32);
    let side = (preview.w / 5.0).min(64.0);
    let total = side * 3.8;
    let start = (preview.w - total) / 2.0;
    let marks = [Role::Accent, Role::Positive, Role::Warning, Role::Ink]
        .into_iter()
        .enumerate()
        .map(|(i, role)| super::paint::Mark {
            form: Form::Disc(side as u32),
            colour: roles[role].premultiply(1.0),
            x: start + i as f32 * side * 0.95,
            y: (preview.h - side) / 2.0,
        })
        .collect();
    p.words(id + 2, preview, vec![], marks, 33);
    let title = Frame::new(f.x + 20.0, f.y + 158.0, f.w - 40.0, p.line(Type::Label) * 2.0);
    p.wrapped(id + 3, option.name, title, Type::Label, Role::Ink, false, 33);
    if focused {
        p.zoom(from, f, p.tokens.focus.card_scale);
    }
    p.hit(focus, f);
}
fn appearance(p: &mut Paint<'_>, x: f32, w: f32) {
    let m = p.model;
    let focused = if let Focus::Setting(i) = m.view.focus { i } else { 0 };
    let preview_h = if p.scale > 1.0 { 270.0 } else { 212.0 };
    let row_h = 80.0f32.max(p.line(Type::Body) + 24.0);
    let scroll = if focused >= 4 { ((170.0 + preview_h + 16.0 + (focused + 1) as f32 * (row_h + 4.0)) - (H - p.y)).max(0.0) } else { 0.0 };
    let top = 170.0 - scroll;
    if top + preview_h > 150.0 {
        let cols = if p.scale > 1.0 { 2 } else { 4 };
        let cw = (w - 20.0 * (cols - 1) as f32) / cols as f32;
        for i in 0..cols {
            theme_card(p, 200 + i as u64 * 10, Focus::Theme(i + 1), i + 1, Frame::new(x + i as f32 * (cw + 20.0), top, cw, preview_h));
        }
    }
    let mut y = top + preview_h + 16.0;
    let name = lumatv_tokens::themes::game_options().iter().find(|f| f.id == m.prefs.family).map_or("Rosé Pine", |f| f.name);
    y += row(p, 300, Focus::Setting(0), "Theme", None, Control::Value(&format!("{name}  ·  All themes")), Frame::new(x, y, w, 0.0));
    y += row(p, 330, Focus::Setting(1), "Mode", None, Control::Segments(&["Dark", "Light"], usize::from(m.prefs.light)), Frame::new(x, y, w, 0.0));
    y += row(
        p,
        360,
        Focus::Setting(2),
        "Featured game",
        None,
        Control::Segments(&["Compact", "Standard", "Cinematic"], m.prefs.hero.index()),
        Frame::new(x, y, w, 0.0),
    );
    y += row(p, 390, Focus::Setting(3), "Covers", None, Control::Segments(&["Compact", "Standard", "Large"], m.prefs.covers.index()), Frame::new(x, y, w, 0.0));
    y += row(
        p,
        420,
        Focus::Setting(4),
        "Text size",
        None,
        Control::Segments(&["100%", "150%", "200%"], usize::from((m.prefs.text_percent - 100) / 50)),
        Frame::new(x, y, w, 0.0),
    );
    row(p, 450, Focus::Setting(5), "Reduce motion", Some("Focus changes at once, with no lift or fades"), Control::Toggle(m.prefs.reduce_motion), Frame::new(x, y, w, 0.0));
}
#[derive(Clone, Copy)]
pub(super) enum Control<'a> {
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

pub(super) fn row(p: &mut Paint<'_>, id: u64, focus: Focus, label: &str, description: Option<&str>, control: Control<'_>, mut f: Frame) -> f32 {
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
            let fill = track.w * value as f32 / 150.0;
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
fn streaming(p: &mut Paint<'_>, x: f32, w: f32) {
    let m = p.model;
    let focus = if let Focus::Setting(i) = m.view.focus { i } else { 0 };
    let row_h = 80.0f32.max(p.line(Type::Body) + 24.0);
    let shift = if focus >= 5 { (170.0 + row_h * (focus + 2) as f32 + 120.0 - (H - p.y)).max(0.0) } else { 0.0 };
    let mut y = 170.0 - shift;
    p.text(200, "Picture", Frame::new(x + 26.0, y, w - 52.0, p.line(Type::Body)), Type::Body, Role::Ink2, false, 10);
    y += p.line(Type::Body) + 24.0;
    let resolution = if m.prefs.height == 2160 { "4K".into() } else { format!("{}p", m.prefs.height) };
    y += row(p, 300, Focus::Setting(0), "Resolution", None, Control::Value(&resolution), Frame::new(x, y, w, 0.0));
    y += row(p, 330, Focus::Setting(1), "Frame rate", None, Control::Value(&format!("{} fps", m.prefs.fps)), Frame::new(x, y, w, 0.0));
    y += row(p, 360, Focus::Setting(2), "HDR", None, Control::Toggle(m.prefs.hdr), Frame::new(x, y, w, 0.0));
    y += row(p, 390, Focus::Setting(3), "Bitrate", None, Control::Slider(m.prefs.bitrate_mbps), Frame::new(x, y, w, 0.0));
    let codecs = if m.prefs.fps <= 60 { vec!["Automatic", "HEVC", "AV1", "H.264"] } else { vec!["Automatic", "HEVC", "H.264"] };
    let value = match m.prefs.codec {
        Some(Codec::H264) => "H.264",
        Some(Codec::H265) => "HEVC",
        Some(Codec::Av1) => "AV1",
        _ => "Automatic",
    };
    let index = codecs.iter().position(|s| *s == value).unwrap_or(0);
    y += row(p, 420, Focus::Setting(4), "Codec", None, Control::Segments(&codecs, index), Frame::new(x, y, w, 0.0));
    if m.prefs.fps > 60 {
        p.text(450, "AV1 is available at 60 fps", Frame::new(x + 26.0, y, w - 52.0, p.line(Type::Caption)), Type::Caption, Role::Ink2, false, 10);
        y += p.line(Type::Caption) + 12.0;
    }
    y += 24.0;
    p.text(460, "Playing", Frame::new(x + 26.0, y, w - 52.0, p.line(Type::Body)), Type::Body, Role::Ink2, false, 10);
    y += p.line(Type::Body) + 20.0;
    let preferred = m.state().profile().user.preferred_engine.as_ref().map_or("Ask each time", |id| m.engine_name(id));
    y += row(p, 480, Focus::Setting(5), "Preferred way to play", None, Control::Value(preferred), Frame::new(x, y, w, 0.0));
    row(p, 510, Focus::Setting(6), "Stats while playing", None, Control::Segments(&["Off", "Compact", "Full"], m.prefs.stats.mode.index()), Frame::new(x, y, w, 0.0));
}
fn integrations(p: &mut Paint<'_>, x: f32, w: f32) {
    let focused = if let Focus::Integration(i) = p.model.view.focus { i } else { 0 };
    let h = 100.0f32.max(p.line(Type::Body) + p.line(Type::Caption) + 24.0);
    let first = focused.saturating_sub(if p.scale > 1.0 { 3 } else { 6 });
    let mut y = 170.0;
    let mut group = None;
    for (i, e) in p.model.integrations.iter().enumerate().skip(first) {
        if group != Some(e.manifest.kind) {
            let name = match e.manifest.kind {
                EngineKind::Cloud => "Cloud",
                EngineKind::Host => "The PCs",
                _ => "On this TV",
            };
            p.text(200 + i as u64 * 30, name, Frame::new(x + 26.0, y, w - 52.0, p.line(Type::Body)), Type::Body, Role::Ink2, false, 10);
            y += p.line(Type::Body) + 20.0;
            group = Some(e.manifest.kind);
        }
        let enabled = p.model.enabled(&e.manifest.id);
        let f = Frame::new(x, y, w, h);
        let active = p.focused(Focus::Integration(i));
        let from = p.plan.nodes.len();
        if active {
            p.surface(201 + i as u64 * 30, f, Fill::Tint(0.09), Radius::Row, false, true, false, 10);
        }
        let linked = e.targets.iter().filter(|t| t.link == Link::Linked).count();
        let subtitle = if !enabled {
            "Off".into()
        } else if let Some(error) = &e.error {
            error.clone()
        } else if linked > 0 {
            e.targets.iter().filter(|t| t.link == Link::Linked).map(|t| t.name.as_str()).collect::<Vec<_>>().join("  ·  ")
        } else if e.descriptor.is_none() {
            "Starting integration".into()
        } else {
            "Not linked".into()
        };
        let icon = match e.manifest.kind {
            EngineKind::Cloud => Icon::CloudOutline,
            EngineKind::Host => Icon::Tiles,
            _ => Icon::Package,
        };
        p.words(
            202 + i as u64 * 30,
            f,
            vec![
                p.word(&e.manifest.name, Type::Body, p.ink(Role::Ink, false), 92.0, 12.0, w - 220.0),
                p.word(subtitle, Type::Caption, p.ink(Role::Ink2, false), 92.0, 12.0 + p.line(Type::Body), w - 220.0),
            ],
            vec![
                p.mark(icon, 40.0, 26.0, (h - 40.0) / 2.0, Role::Ink2, false),
                p.mark(Icon::ChevronRight, 30.0, w - 50.0, (h - 30.0) / 2.0, Role::Ink2, false),
            ],
            11,
        );
        p.toggle(203 + i as u64 * 30, x + w - 150.0, y + (h - 48.0) / 2.0, enabled, false, 12);
        p.hit(Focus::Integration(i), f);
        if active {
            p.zoom(from, f, p.tokens.focus.card_scale);
        }
        y += h + 6.0;
        if y > H - p.y {
            break;
        }
    }
    if p.model.integrations.is_empty() {
        p.text(200, "No integrations are installed", Frame::new(x + 26.0, y, w - 52.0, p.line(Type::Body)), Type::Body, Role::Ink2, false, 10);
    }
}
fn controllers(p: &mut Paint<'_>, x: f32, w: f32) {
    let mut y = 170.0;
    y += p.wrapped(200, "Controllers", Frame::new(x + 26.0, y, w - 52.0, 0.0), Type::Title, Role::Ink, false, 10) + 24.0;
    for (i, label, description) in [
        (0, "Controller lab", "Buttons, sticks, rumble and timing"),
        (1, "TV controller settings", "Pair a Bluetooth controller"),
        (2, "GameSir Cyclone 2", "Settings stored on the controller"),
        (3, "Leadjoy Xeno", "Controller settings and logo lighting"),
    ] {
        y += row(p, 500 + i as u64 * 30, Focus::Setting(i), label, Some(description), Control::Value("Open"), Frame::new(x, y, w, 0.0));
    }
    y += 32.0;
    let pairs = [("D-pad", "Move focus"), ("A / Cross", "Select on release"), ("B / Circle", "Back"), ("Guide", "Open the playing controls")];
    for (i, (label, value)) in pairs.into_iter().enumerate() {
        y += row(p, 230 + i as u64 * 30, Focus::None, label, None, Control::Value(value), Frame::new(x, y, w, 0.0));
    }
    p.wrapped(410, "TV settings: General › External Devices › Connect Bluetooth Controller",
        Frame::new(x + 26.0, y + 24.0, w - 52.0, 0.0), Type::Caption, Role::Ink2, false, 10);
}

pub fn gamesir(p: &mut Paint<'_>) {
    nav(p, false);
    let (x, w) = (p.x, W - p.x * 2.0);
    let (tx, tw) = (x + 26.0, w - 52.0);
    let mut y = 170.0;
    y += p.wrapped(100, "GameSir Cyclone 2", Frame::new(tx, y, tw, 0.0), Type::Title, Role::Ink, false, 10) + 24.0;
    let device = &p.model.gamesir;
    let status = if device.pending {
        "Reading the controller; changes wait for read-back.".to_owned()
    } else if let Some(error) = &device.error {
        if matches!(device.state, GameSirState::Ready(_)) { format!("{error} The values below are the last confirmed reading.") } else { error.clone() }
    } else {
        match &device.state {
            GameSirState::Idle => "Refresh to read the controller.".into(),
            GameSirState::Off => "Switch on the Gaming plugin in LumaTV to change controller settings.".into(),
            GameSirState::Missing => "No GameSir controller found. Connect its USB cable or 2.4 GHz receiver, then refresh.".into(),
            GameSirState::Mode { .. } => "To change its settings, switch to X-input: hold View+Menu for 2 seconds, then refresh.".into(),
            GameSirState::Ready(profile) => format!("Profile {} · Firmware {}{}", profile.profile, profile.firmware,
                if profile.verified { " · Saved and read back" } else { " · Read from the controller" }),
        }
    };
    y += p.wrapped(101, &status, Frame::new(tx, y, tw, 0.0), Type::Body, Role::Ink, false, 10) + 24.0;
    y += row(p, 200, Focus::Setting(0), "Refresh", None, Control::Value(if device.pending { "Waiting" } else { "Read" }), Frame::new(x, y, w, 0.0)) + 16.0;
    let GameSirState::Ready(profile) = &device.state else { return };
    let values = &profile.values;
    let rate = values.report_rate.map_or_else(|| "Unknown".into(), |rate| format!("{rate} Hz"));
    let left = format!("{} / 4", values.rumble_left);
    let right = format!("{} / 4", values.rumble_right);
    let dead_left = format!("{}%", values.deadzone_left);
    let dead_right = format!("{}%", values.deadzone_right);
    for (i, label, control) in [
        (1, "Report rate", Control::Value(&rate)),
        (2, "Left rumble strength", Control::Value(&left)),
        (3, "Right rumble strength", Control::Value(&right)),
        (4, "D-pad diagonal lock", Control::Toggle(values.dpad_diagonal_lock)),
        (5, "Swap A/B and X/Y", Control::Toggle(values.xinput_abxy_swap)),
        (6, "Left stick dead zone", Control::Value(&dead_left)),
        (7, "Right stick dead zone", Control::Value(&dead_right)),
    ] {
        let status = if p.focused(Focus::Setting(i)) {
            if device.pending { Some("Waiting for read-back") } else if profile.verified { Some("Saved and read back") } else { None }
        } else { None };
        y += row(p, 300 + i as u64 * 30, Focus::Setting(i), label, status, control, Frame::new(x, y, w, 0.0));
    }
    p.wrapped(900, "These settings stay on the controller. The lab's stick settings are applied separately by each game integration.",
        Frame::new(tx, y + 24.0, tw, 0.0), Type::Caption, Role::Ink2, false, 10);
}

pub fn leadjoy(p: &mut Paint<'_>) {
    use crate::game::helper::LeadjoyState;
    nav(p, false);
    let (x, w) = (p.x, W - p.x * 2.0);
    let (tx, tw) = (x + 26.0, w - 52.0);
    let mut y = 170.0;
    y += p.wrapped(100, "Leadjoy Xeno", Frame::new(tx, y, tw, 0.0), Type::Title, Role::Ink, false, 10) + 24.0;
    let device = &p.model.leadjoy;
    let status = if device.pending {
        "Reading the controller; changes wait for read-back.".to_owned()
    } else if let Some(error) = &device.error {
        if matches!(device.state, LeadjoyState::Ready(_)) { format!("{error} The values below are the last confirmed reading.") } else { error.clone() }
    } else {
        match &device.state {
            LeadjoyState::Idle => "Refresh to read the controller.".into(),
            LeadjoyState::Off => "Switch on the Gaming plugin in LumaTV to change controller settings.".into(),
            LeadjoyState::Missing => "No Xeno configuration interface found. Connect its USB cable or receiver, then refresh.".into(),
            LeadjoyState::Unsupported { device, firmware } => format!("{device} · Firmware {firmware} · No supported settings"),
            LeadjoyState::Ready(profile) => format!("{} · Firmware {} · {}", profile.device, profile.firmware,
                if profile.verified { "Saved and read back" } else { "Read from the controller" }),
        }
    };
    y += p.wrapped(101, &status, Frame::new(tx, y, tw, 0.0), Type::Body, Role::Ink, false, 10) + 24.0;
    y += row(p, 200, Focus::Setting(0), "Refresh", None, Control::Value(if device.pending { "Waiting" } else { "Read" }), Frame::new(x, y, w, 0.0)) + 16.0;
    let LeadjoyState::Ready(profile) = &device.state else { return };
    for (i, field) in profile.schema.fields().enumerate() {
        let current = profile.values.get(&field.key);
        let text = match current { Some(Value::Text(v)) => v.clone(), Some(Value::Int(v)) => v.to_string(), Some(Value::Bool(v)) => v.to_string(), None => "Not reported".into() };
        let value;
        let control = match &field.kind {
            FieldKind::Toggle { .. } => Control::Toggle(current == Some(&Value::Bool(true))),
            FieldKind::Choice { options, .. } => Control::Value(options.iter().find(|option| option.id == text).map_or(text.as_str(), |option| option.label.as_str())),
            FieldKind::Action => Control::Value("Apply"),
            FieldKind::Text { .. } if text.is_empty() => Control::Value("Add"),
            FieldKind::Number { unit: Some(unit), .. } => { value = format!("{text}{unit}"); Control::Value(&value) }
            _ => Control::Value(&text),
        };
        let status = if p.focused(Focus::Setting(i + 1)) {
            if device.pending { Some("Waiting for read-back") } else if profile.verified { Some("Saved and read back") } else { field.help.as_deref() }
        } else { field.help.as_deref() };
        y += row(p, 300 + i as u64 * 30, Focus::Setting(i + 1), &field.label, status, control, Frame::new(x, y, w, 0.0));
    }
    p.wrapped(1900, "Changes are written to the controller and read back.", Frame::new(tx, y + 24.0, tw, 0.0), Type::Caption, Role::Ink2, false, 10);
}
fn diagnostics(p: &mut Paint<'_>, x: f32, w: f32) {
    use crate::game::diagnostics::Status;
    let m = p.model;
    let focus = if let Focus::Setting(i) = m.view.focus { i } else { 0 };
    let status = match m.diagnostics_status {
        Status::Off => "Off", Status::NotConfigured => "Add an HTTPS server and topic",
        Status::Waiting => "Ready", Status::Sending => "Sending", Status::Sent => "Last send succeeded",
        Status::Error => "Send failed",
    };
    let detail = format!("{status} · Off stops sending; delivered messages remain");
    let rows = [
        ("Remote diagnostics", Some(detail.as_str()), Control::Toggle(m.diagnostics.enabled)),
        ("Server URL", None, Control::Value(if m.diagnostics.server.is_empty() { "Add" } else { &m.diagnostics.server })),
        ("Topic", None, Control::Value(if m.diagnostics.topic.is_empty() { "Add" } else { &m.diagnostics.topic })),
        ("Write token", None, Control::Value(if m.diagnostics.token.is_empty() { "Add" } else { "Set" })),
        ("Clear write token", None, Control::Value("Clear")),
        ("Test diagnostics", None, Control::Value("Send")),
    ];
    let control_h = p.tokens.target.max(p.line(Type::Label) + 12.0);
    let focused_bottom = 170.0 + rows.iter().take(focus + 1).map(|(label, description, control)| {
        let control_w = match control {
            Control::Value(value) => p.width(value, Type::Body),
            Control::Toggle(_) => 82.0,
            _ => unreachable!(),
        }.min(w - 52.0);
        row_metrics(p, label, *description, w, control_w, control_h, 1).4 + 4.0
    }).sum::<f32>();
    let mut y = 170.0 - (focused_bottom + 16.0 - (H - p.y)).max(0.0);
    for (i, (label, description, control)) in rows.into_iter().enumerate() {
        y += row(p, 600 + i as u64 * 30, Focus::Setting(i), label, description, control, Frame::new(x, y, w, 0.0));
    }
}

fn about(p: &mut Paint<'_>, x: f32, w: f32) {
    let mut y = 170.0;
    y += p.wrapped(200, "LumaGame", Frame::new(x + 26.0, y, w - 52.0, 0.0), Type::Title, Role::Ink, false, 10) + 16.0;
    p.text(201, env!("CARGO_PKG_VERSION"), Frame::new(x + 26.0, y, w - 52.0, p.line(Type::Caption)), Type::Caption, Role::Ink2, false, 10);
    y += p.line(Type::Caption) + 32.0;
    for (i, text) in [lumagame_meta::STEAM_CREDIT, lumagame_meta::SGDB_CREDIT, lumagame_meta::IGDB_CREDIT].into_iter().enumerate() {
        let h = ui::components::wrap(p.fonts, text, p.ty(Type::Body), w - 52.0).len() as f32 * p.line(Type::Body) + 32.0;
        let f = Frame::new(x, y, w, h);
        let from = p.plan.nodes.len();
        if p.focused(Focus::Setting(i)) {
            p.surface(230 + i as u64 * 3, f, Fill::Tint(0.09), Radius::Row, false, true, false, 10);
        }
        p.wrapped(231 + i as u64 * 3, text, Frame::new(x + 26.0, y + 16.0, w - 52.0, 0.0), Type::Body, Role::Ink2, false, 11);
        if p.focused(Focus::Setting(i)) {
            p.zoom(from, f, p.tokens.focus.card_scale);
        }
        p.hit(Focus::Setting(i), f);
        y += h + 8.0;
    }
}
pub fn integration(p: &mut Paint<'_>) {
    nav(p, false);
    let m = p.model;
    let Some(e) = m.view.selected_engine.as_ref().and_then(|id| m.integration(id)) else {
        return;
    };
    let x = p.x;
    let w = W - p.x * 2.0;
    let mut y = 170.0;
    let enabled = m.enabled(&e.manifest.id);
    p.text(100, &e.manifest.name, Frame::new(x + 26.0, y, w - 52.0, p.line(Type::Title)), Type::Title, Role::Ink, false, 10);
    y += p.line(Type::Title) + 24.0;
    y += row(p, 200, Focus::Setting(0), "Enable integration", Some("Runs only while enabled"), Control::Toggle(enabled), Frame::new(x, y, w, 0.0));
    if !enabled {
        return;
    }
    if let Some(error) = &e.error {
        y += p.wrapped(225, error, Frame::new(x + 26.0, y + 16.0, w - 52.0, 0.0), Type::Body, Role::Warning, false, 10) + 32.0;
    }
    if let Some(d) = &e.descriptor {
        if !d.capabilities.auth.is_empty() {
            y += row(
                p,
                230,
                Focus::Setting(1),
                if e.manifest.kind == EngineKind::Cloud { "Sign in" } else { "Pair a PC" },
                None,
                Control::Value("Connect"),
                Frame::new(x, y, w, 0.0),
            );
        }
        if d.capabilities.has(Feature::ManualHost) {
            y += row(p, 260, Focus::Setting(2), "Add PC by address", None, Control::Value("Add"), Frame::new(x, y, w, 0.0));
        }
    }
    for (i, target) in e.targets.iter().enumerate() {
        let state = match (target.link, target.reach) {
            (Link::Expired, _) => "Sign in again",
            (Link::Linked, Reach::Online) => "Connected",
            (_, Reach::Asleep) => "Asleep",
            (_, Reach::Offline) => "Offline",
            _ => "Not linked",
        };
        y += row(p, 10_000 + i as u64 * 100, Focus::Target(i), &target.name, None, Control::Value(state), Frame::new(x, y, w, 0.0));
    }
    for (i, field) in e.schema.fields().enumerate() {
        let saved = m.state().engines.get(&e.manifest.id).and_then(|e| e.settings.get(&field.key)).cloned().or_else(|| field.kind.default());
        let label = &field.label;
        let description = field.help.as_deref();
        let value = match &saved {
            Some(Value::Text(v)) => v.clone(),
            Some(Value::Int(v)) => v.to_string(),
            Some(Value::Bool(v)) => v.to_string(),
            None => String::new(),
        };
        let choices;
        let control = match &field.kind {
            FieldKind::Toggle { .. } => Control::Toggle(saved == Some(Value::Bool(true))),
            FieldKind::Choice { options, .. } => {
                choices = options.iter().map(|c| c.label.as_str()).collect::<Vec<_>>();
                Control::Segments(&choices, options.iter().position(|o| o.id == value).unwrap_or(0))
            }
            FieldKind::Action => Control::Value("Open"),
            FieldKind::Note => Control::None,
            _ => Control::Value(&value),
        };
        y += row(
            p,
            100_000 + i as u64 * 100,
            if field.kind == FieldKind::Note { Focus::None } else { Focus::EngineField(i) },
            label,
            description,
            control,
            Frame::new(x, y, w, 0.0),
        );
    }
}
