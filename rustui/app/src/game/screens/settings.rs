use super::{
    paint::{Fill, Paint, W},
    parts::{dialog::{panel, scrolling}, rail::rail, row::{Control, row}, topbar::{self, Here}},
};
use crate::game::helper::{GameSirState, LeadjoyState};
use crate::game::model::{Focus, Key, settings::Place};
use compositor_protocol::Rect;
use lumagame_core::{
    engine::{Codec, EngineKind},
    ipc::Link,
    settings::{FieldKind, Value},
};
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{
    components::{Form, Icon},
    geometry::Frame,
    lucent::palette::PaletteRole as Role,
};

pub fn settings(p: &mut Paint<'_>) {
    let attention = topbar::needs_attention(p);
    topbar::bar(p, Here::Elsewhere, attention);
    let x = rail(p);
    let w = W - p.x - x;
    match p.model.view.place {
        Place::Appearance => appearance(p, x, w),
        Place::Streaming => streaming(p, x, w),
        Place::Integrations => integrations(p, x, w),
        Place::Controllers => controllers(p, x, w),
        Place::Diagnostics => diagnostics(p, x, w),
        Place::About => about(p, x, w),
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
    let preview_h = if p.scale > 1.0 { 270.0 } else { 212.0 };
    let cols = if p.scale > 1.0 { 2 } else { 4 };
    let cw = (w - 20.0 * (cols - 1) as f32) / cols as f32;
    for i in 0..cols {
        theme_card(p, 200 + i as u64 * 10, Focus::Theme(i + 1), i + 1, Frame::new(x + i as f32 * (cw + 20.0), 170.0, cw, preview_h));
    }
    let mut y = 170.0 + preview_h + 16.0;
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
fn streaming(p: &mut Paint<'_>, x: f32, w: f32) {
    let m = p.model;
    let mut y = 170.0;
    p.text(200, "Picture", Frame::new(x + 26.0, y, w - 52.0, p.line(Type::Body)), Type::Body, Role::Ink2, false, 10);
    y += p.line(Type::Body) + 24.0;
    let resolution = if m.prefs.height == 2160 { "4K".into() } else { format!("{}p", m.prefs.height) };
    y += row(p, 300, Focus::Setting(0), "Resolution", None, Control::Value(&resolution), Frame::new(x, y, w, 0.0));
    y += row(p, 330, Focus::Setting(1), "Frame rate", None, Control::Value(&format!("{} fps", m.prefs.fps)), Frame::new(x, y, w, 0.0));
    y += row(p, 360, Focus::Setting(2), "HDR", None, Control::Toggle(m.prefs.hdr), Frame::new(x, y, w, 0.0));
    // The slider keeps the viewer's choice; what the TV's port lets through is said under it.
    let note = m.uplink.as_ref().and_then(|uplink| uplink.note(m.prefs.bitrate_mbps));
    y += row(p, 390, Focus::Setting(3), "Bitrate", note.as_deref(), Control::Slider(m.prefs.bitrate_mbps), Frame::new(x, y, w, 0.0));
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
    y += row(p, 540, Focus::Setting(5), "Frame pacing", None, Control::Segments(&["Smooth", "Lowest latency"], usize::from(!m.prefs.smooth_pacing)), Frame::new(x, y, w, 0.0));
    y += 24.0;
    p.text(460, "Playing", Frame::new(x + 26.0, y, w - 52.0, p.line(Type::Body)), Type::Body, Role::Ink2, false, 10);
    y += p.line(Type::Body) + 20.0;
    let preferred = m.state().profile().user.preferred_engine.as_ref().map_or("Ask each time", |id| m.engine_name(id));
    y += row(p, 480, Focus::Setting(6), "Preferred way to play", None, Control::Value(preferred), Frame::new(x, y, w, 0.0));
    row(p, 510, Focus::Setting(7), "Stats while playing", None, Control::Segments(&["Off", "Compact", "Full"], m.prefs.stats.mode.index()), Frame::new(x, y, w, 0.0));
}
fn integrations(p: &mut Paint<'_>, x: f32, w: f32) {
    let h = 100.0f32.max(p.line(Type::Body) + p.line(Type::Caption) + 24.0);
    let mut y = 170.0;
    let mut group = None;
    for (i, e) in p.model.integrations.iter().enumerate() {
        if group != Some(e.manifest.kind) {
            let name = match e.manifest.kind {
                EngineKind::Cloud => "Cloud",
                EngineKind::Host => "The PCs",
                EngineKind::Account => "Accounts",
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
        } else if let Some((_, error)) = &e.error {
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
            EngineKind::Account => Icon::Person,
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
    let labels = crate::game::input::Labels::ALL.map(|l| l.name());
    y += row(
        p,
        620,
        Focus::Setting(4),
        "Button labels",
        Some("The markings hints show on face buttons"),
        Control::Segments(&labels, p.model.prefs.button_labels.index()),
        Frame::new(x, y, w, 0.0),
    );
    y += 32.0;
    let pairs = [("D-pad", "Move focus"), ("A / Cross", "Select on release"), ("B / Circle", "Back"), ("Guide", "Open the playing controls")];
    for (i, (label, value)) in pairs.into_iter().enumerate() {
        y += row(p, 230 + i as u64 * 30, Focus::None, label, None, Control::Value(value), Frame::new(x, y, w, 0.0));
    }
    p.wrapped(410, "TV settings: General › External Devices › Connect Bluetooth Controller",
        Frame::new(x + 26.0, y + 24.0, w - 52.0, 0.0), Type::Caption, Role::Ink2, false, 10);
}

pub fn gamesir(p: &mut Paint<'_>) {
    let attention = topbar::needs_attention(p);
    topbar::bar(p, Here::Elsewhere, attention);
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
    let attention = topbar::needs_attention(p);
    topbar::bar(p, Here::Elsewhere, attention);
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
            FieldKind::Note => Control::None,
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
        let focus = if matches!(field.kind, FieldKind::Note) { Focus::None } else { Focus::Setting(i + 1) };
        y += row(p, 300 + i as u64 * 30, focus, &field.label, status, control, Frame::new(x, y, w, 0.0));
    }
    p.wrapped(1900, "Changes are written to the controller and read back.", Frame::new(tx, y + 24.0, tw, 0.0), Type::Caption, Role::Ink2, false, 10);
}
fn diagnostics(p: &mut Paint<'_>, x: f32, w: f32) {
    use crate::game::diagnostics::Status;
    let m = p.model;
    let status = match m.diagnostics_status {
        Status::Off => "Off", Status::NotConfigured => "Add an HTTPS server and topic",
        Status::Waiting => "Ready", Status::Sending => "Sending", Status::Sent => "Last send succeeded",
        Status::Error => "Send failed",
    };
    let detail = format!("{status} · Off stops sending; delivered messages remain");
    let level = m.diagnostics.level;
    // Short words: beside five segments the note gets a narrow column at large text sizes, where a long word would be cut off.
    let note = ["Failures only", "Plus degraded streams", "Plus session and TV state", "Plus pacing and decode stats", "Plus fine timing detail"][level.index()];
    let rows = [
        ("Remote diagnostics", Some(detail.as_str()), Control::Toggle(m.diagnostics.enabled)),
        ("Log level", Some(note), Control::Segments(&["Error", "Warn", "Info", "Debug", "Trace"], level.index())),
        ("Server URL", None, Control::Value(if m.diagnostics.server.is_empty() { "Add" } else { &m.diagnostics.server })),
        ("Topic", None, Control::Value(if m.diagnostics.topic.is_empty() { "Add" } else { &m.diagnostics.topic })),
        ("Write token", None, Control::Value(if m.diagnostics.token.is_empty() { "Add" } else { "Set" })),
        ("Clear write token", None, Control::Value("Clear")),
        ("Test diagnostics", None, Control::Value("Send")),
    ];
    let mut y = 170.0;
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
pub fn themes(p: &mut Paint<'_>) {
    let f = panel(p, 1728.0, 972.0);
    p.text(8010, "Choose a theme", Frame::new(f.x + 36.0, f.y + 36.0, f.w - 72.0, p.line(Type::Title)), Type::Title, Role::Ink, false, 32);
    let cols = if p.scale > 1.0 { 3 } else { 4 };
    let card_h = if p.scale > 1.0 { 270.0 } else { 212.0 };
    let w = (f.w - 72.0 - 24.0 * (cols - 1) as f32) / cols as f32;
    let rows = ((f.h - p.line(Type::Title) - 108.0) / (card_h + 24.0)).floor() as usize;
    let focus = if let Focus::Theme(i) = p.model.view.focus { i } else { 0 };
    let first_row = (focus / cols).saturating_sub(rows.saturating_sub(1));
    let top = f.y + 72.0 + p.line(Type::Title);
    let count = lumatv_tokens::themes::game_options().len();
    let view = Rect { x: f.x + 36.0, y: top, w: f.w - 72.0, h: rows as f32 * (card_h + 24.0) - 24.0 };
    scrolling(p, view, card_h + 24.0, count.div_ceil(cols), first_row, 8.0, |p, row, y| {
        for i in row * cols..((row + 1) * cols).min(count) {
            super::settings::theme_card(p, 8100 + i as u64 * 10, Focus::Theme(i), i, Frame::new(f.x + 36.0 + (i % cols) as f32 * (w + 24.0), y, w, card_h));
        }
    });
}
pub fn delete(p: &mut Paint<'_>) {
    let field = match &p.model.leadjoy.state {
        LeadjoyState::Ready(profile) => p.model.view.edit_field.and_then(|i| profile.schema.fields().nth(i)),
        _ => None,
    };
    let (title, body) = field.map_or_else(|| ("Delete".into(), String::new()), |field| (field.label.clone(), field.help.clone().unwrap_or_default()));
    let f = panel(p, 1080.0, if p.scale > 1.0 { 700.0 } else { 400.0 });
    let x = f.x + 40.0;
    p.text(8010, &title, Frame::new(x, f.y + 40.0, f.w - 80.0, p.line(Type::Title) * 2.0), Type::Title, Role::Ink, false, 32);
    p.wrapped(8020, &body, Frame::new(x, f.y + 78.0 + p.line(Type::Title), f.w - 80.0, 0.0), Type::Body, Role::Ink2, false, 32);
    let y = f.y + f.h - 104.0;
    let keep = p.lead(Key::Back, Icon::Back);
    let w = p.button_lead(8030, Focus::SheetAction(0), "Keep", Some(keep), x, y, true, false, 34);
    p.button(8040, Focus::SheetAction(1), "Delete", Some(Icon::Close), x + w + 16.0, y, false, false, 34);
}
