//! Settings: the rail of five places beside the pane the model's sheet fills, and the panes that keep a painter of their own (the theme
//! cards, the controller lab, GameSir and Leadjoy).
//!
//! Node ids of the pane: 900 the lede, 950 and up a group or note caption, from 1000 sixteen to a row (see [`row`](super::parts::row)),
//! 4000 and up the theme cards, 899 the words of all but the lit row. The shell holds 100 to 199, 199 the words of all but the lit place.
use super::{
    lab,
    paint::{Fill, Node, PAGE_EXTENT, PAGE_GROUP, Paint, ScrollGroup, merge_words},
    parts::{
        Side,
        dialog::{Action, modal},
        hints,
        rail::{self, RailSpec},
        row::{self, Control, Leading, Line},
    },
};
use crate::game::helper::{GameSirState, LeadjoyState};
use crate::game::model::{
    Focus,
    settings::{Entry, Glyph, Pane, Place, Shows, Tone},
};
use compositor_protocol::{Edges, Rect};
use lumatv_tokens::{Radius, TypeRole as Type};
use lumagame_core::settings::{FieldKind, Value};
use ui::{
    components::{Form, Icon, ellipsise},
    geometry::Frame,
    lucent::palette::PaletteRole as Role,
};

fn place_icon(place: Place) -> Icon {
    match place {
        Place::Accounts => Icon::Person,
        Place::Picture => Icon::Cards,
        Place::Controllers => Icon::Sliders,
        Place::Appearance => Icon::Palette,
        Place::System => Icon::Info,
    }
}
fn icon(glyph: Glyph) -> Icon {
    match glyph {
        Glyph::Cloud => Icon::Cloud,
        Glyph::Pc => Icon::Server,
        Glyph::Tv | Glyph::Qr => Icon::Tiles,
        Glyph::Account => Icon::Person,
        Glyph::Add => Icon::Plus,
        Glyph::Clock => Icon::Clock,
        Glyph::Games => Icon::Apps,
        Glyph::News => Icon::Message,
        Glyph::Mature => Icon::Shield,
        Glyph::SignOut => Icon::Power,
        Glyph::Picture => Icon::Video,
        Glyph::Motion => Icon::Refresh,
        Glyph::Pad => Icon::Sliders,
        Glyph::Palette => Icon::Palette,
        Glyph::Light => Icon::Sun,
        Glyph::Text => Icon::Eye,
        Glyph::Gauge => Icon::Gauge,
        Glyph::Info => Icon::Info,
        Glyph::Bug => Icon::Bug,
        Glyph::Setting => Icon::Cog,
    }
}
/// A state word's mark and colour.
fn state_mark(word: &str, tone: Tone) -> (Icon, Role) {
    match tone {
        Tone::Positive => (Icon::Tick, Role::Positive),
        Tone::Info if word == "Asleep" => (Icon::Moon, Role::Ink2),
        Tone::Info | Tone::Quiet => (Icon::Info, Role::Ink2),
        Tone::Warning => (Icon::Warning, Role::Warning),
        Tone::Danger => (Icon::WifiOff, Role::Danger),
    }
}

pub fn settings(p: &mut Paint<'_>) {
    let m = p.model;
    if m.view.pane == Some(Pane::Lab) && lab::capture(p) {
        return;
    }
    let sheet = m.settings_sheet();
    let states = m.rail_states();
    let places: Vec<rail::Place<'_>> =
        Place::ALL.iter().zip(&states).map(|(place, state)| rail::Place { icon: place_icon(*place), name: place.name(), state: Some(state) }).collect();
    let page_hints = m.hints();
    let spec = RailSpec {
        title: "Settings",
        places: &places,
        current: m.view.place.index(),
        crumb: sheet.crumb,
        heading: &sheet.title,
        chip: sheet.chip.map(|(word, _)| word),
        hints: &page_hints,
    };
    let shell = p.plan.nodes.len();
    let content = rail::shell(p, &spec);
    let lit = p.plan.hits.iter().find(|(id, _)| *id == m.view.focus && matches!(id, Focus::Rail(_))).map(|(_, f)| *f);
    merge_words(&mut p.plan.nodes, shell, 199, |n| lit.is_some_and(|f| inside(n, f)));
    let (nodes, hits) = (p.plan.nodes.len(), p.plan.hits.len());
    match m.view.pane {
        Some(Pane::Theme) => themes(p, content),
        Some(Pane::Lab) => lab::lab(p, content),
        Some(Pane::GameSir) => gamesir(p, content),
        Some(Pane::Leadjoy) => leadjoy(p, content),
        _ => list(p, &sheet, content),
    }
    scroll(p, content, nodes, hits);
    if m.view.top().is_none() {
        hints::bar(p, &page_hints, Side::Right, m.hints_dim);
    }
}

/// The sheet's lede, captions and rows, from the top of `c`.
fn list(p: &mut Paint<'_>, sheet: &crate::game::model::settings::Sheet, c: Frame) {
    let mut y = c.y;
    if let Some(lede) = &sheet.lede {
        y += p.wrapped(900, lede, Frame::new(c.x + 28.0, y, c.w - 56.0, 0.0), Type::Body, Role::Ink2, false, 12) + 20.0;
    }
    let row_h = p.tokens.row.max(32.0 * p.scale + 20.0 + 16.0).max(p.line(Type::Body) + 2.0 + p.line(Type::Caption) + 24.0);
    for (i, entry) in sheet.entries.iter().enumerate() {
        match entry {
            Entry::Group(text) => {
                y += 16.0;
                y += p.wrapped(950 + i as u64, text, Frame::new(c.x + 28.0, y, c.w - 56.0, 0.0), Type::Label, Role::Ink2, false, 12) + 8.0;
            }
            Entry::Note(text) => {
                y += p.wrapped(950 + i as u64, text, Frame::new(c.x + 28.0, y, c.w - 56.0, 0.0), Type::Caption, Role::Warning, false, 12) + 12.0;
            }
            Entry::Row(item) => {
                let (id, focus) = (1000 + i as u64 * 16, Focus::Setting(i));
                let frame = Frame::new(c.x, y, c.w, row_h);
                let focused = p.focused(focus);
                let line = Line { lead: Leading::Glyph(icon(item.glyph)), name: &item.name, sub: item.sub.as_deref() };
                match &item.shows {
                    Shows::Opens => row::link(p, id, frame, &line, focused),
                    Shows::Value(value) => row::value(p, id, frame, &line, value, None, focused),
                    Shows::State(word, tone) => row::value(p, id, frame, &line, word, Some(state_mark(word, *tone)), focused),
                    Shows::Switch(on) => row::switch(p, id, frame, &line, *on, focused),
                    Shows::Stepper(value) => row::stepper(p, id, frame, &line, value, focused),
                    Shows::Nothing => row::value(p, id, frame, &line, "", None, focused),
                }
                if item.danger {
                    let (danger, glyph) = (p.ink(Role::Danger, false), icon(item.glyph));
                    if let Some(super::paint::Body::Text { marks, .. }) = p.plan.nodes.iter_mut().find(|n| n.id == id + 2).map(|n| &mut n.body) {
                        if let Some(mark) = marks.iter_mut().find(|m| matches!(m.form, Form::Icon(i, _) if i == glyph)) {
                            mark.colour = danger;
                        }
                    }
                }
                p.hit(focus, frame);
                y += row_h + 4.0;
            }
        }
    }
}

/// Moves what the pane painted from node `nodes` and hit `hits` on so the focus stays inside `view`, clipped to it as one scroll
/// group. Focus on the pane's last row brings what follows it into view as well. The words of all but the lit row become one layer.
fn scroll(p: &mut Paint<'_>, view: Frame, nodes: usize, hits: usize) {
    let focus = p.model.view.focus;
    let bottom = p.plan.nodes[nodes..].iter().map(|n| n.frame.y + n.frame.h).fold(view.y, f32::max);
    let room = (bottom - (view.y + view.h)).max(0.0);
    let ours = &p.plan.hits[hits..];
    let last = ours.iter().map(|(_, f)| f.y).fold(f32::MIN, f32::max);
    let shift = ours.iter().find(|(id, _)| *id == focus).map_or(0.0, |(_, f)| {
        if f.y >= last {
            room
        } else if f.h > view.h {
            // A reading taller than the pane shows from its top.
            (f.y - view.y).clamp(0.0, room)
        } else {
            (f.y + f.h + 24.0 - (view.y + view.h)).clamp(0.0, room)
        }
    });
    let from = p.scroll_positions.get(&PAGE_GROUP).copied().unwrap_or(shift);
    let (lo, hi) = (from.min(shift), from.max(shift));
    let mut kept = p.plan.nodes.split_off(nodes);
    kept.retain(|n| n.frame.y + n.frame.h > view.y + lo - 40.0 && n.frame.y < view.y + view.h + hi + 40.0);
    for n in &mut kept {
        n.frame.y -= shift;
        n.props.y -= shift;
    }
    let lit = p.plan.hits[hits..].iter().find(|(id, _)| *id == focus).map(|(_, f)| Frame { y: f.y - shift, ..*f });
    merge_words(&mut kept, 0, 899, |n| lab::apart(n.id) || lit.is_some_and(|f| inside(n, f)));
    let ids = kept.iter().map(|n| n.id).collect();
    p.plan.nodes.extend(kept);
    for k in hits..p.plan.hits.len() {
        p.plan.hits[k].1.y -= shift;
        let id = p.plan.hits[k].0;
        p.plan.hit_groups.insert(id, PAGE_GROUP);
    }
    p.plan.groups.push(ScrollGroup {
        id: PAGE_GROUP,
        parent: None,
        clip: Rect { x: view.x - 32.0, y: view.y - 8.0, w: view.w + 64.0, h: view.h + 16.0 },
        feather: Edges::new(0.0, 0.0, 16.0, 24.0),
        horizontal: false,
        offset: shift,
        extent: PAGE_EXTENT,
        nodes: ids,
    });
}

/// Whether `n`'s centre lies in `f`.
fn inside(n: &Node, f: Frame) -> bool {
    let (x, y) = (n.frame.x + n.frame.w / 2.0, n.frame.y + n.frame.h / 2.0);
    x >= f.x && x <= f.x + f.w && y >= f.y && y <= f.y + f.h
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
    // The swatches and the name share one layer, so a card costs three.
    let marks = [Role::Accent, Role::Positive, Role::Warning, Role::Ink]
        .into_iter()
        .enumerate()
        .map(|(i, role)| super::paint::Mark {
            form: Form::Disc(side as u32),
            colour: roles[role].premultiply(1.0),
            x: preview.x - f.x + start + i as f32 * side * 0.95,
            y: preview.y - f.y + (preview.h - side) / 2.0,
        })
        .collect();
    let ty = p.ty(Type::Label);
    let name = ellipsise(p.fonts, option.name, ty, f.w - 40.0);
    let word = p.word(name, Type::Label, p.ink(Role::Ink, false), 20.0, 158.0, f.w - 40.0);
    p.words(id + 2, f, vec![word], marks, 33);
    if focused {
        p.zoom(from, f, p.tokens.focus.card_scale);
    }
    p.hit(focus, f);
}

/// The theme cards, in the model's columns.
fn themes(p: &mut Paint<'_>, c: Frame) {
    let cols = p.model.theme_columns();
    let card_h = if p.scale > 1.0 { 270.0 } else { 212.0 };
    let w = (c.w - 16.0 - 24.0 * (cols - 1) as f32) / cols as f32;
    for i in 0..lumatv_tokens::themes::game_options().len() {
        let f = Frame::new(c.x + 8.0 + (i % cols) as f32 * (w + 24.0), c.y + 8.0 + (i / cols) as f32 * (card_h + 24.0), w, card_h);
        theme_card(p, 4000 + i as u64 * 10, Focus::Theme(i), i, f);
    }
}

fn gamesir(p: &mut Paint<'_>, c: Frame) {
    let (x, w) = (c.x, c.w);
    let (tx, tw) = (x + 26.0, w - 52.0);
    let mut y = c.y;
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
    y += p.wrapped(210, &status, Frame::new(tx, y, tw, 0.0), Type::Body, Role::Ink, false, 12) + 24.0;
    y += row::row(p, 220, Focus::Setting(0), "Refresh", None, Control::Value(if device.pending { "Waiting" } else { "Read" }), Frame::new(x, y, w, 0.0)) + 16.0;
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
        y += row::row(p, 300 + i as u64 * 30, Focus::Setting(i), label, status, control, Frame::new(x, y, w, 0.0));
    }
    p.wrapped(900, "These settings stay on the controller. The lab's stick settings are applied separately by each game integration.",
        Frame::new(tx, y + 24.0, tw, 0.0), Type::Caption, Role::Ink2, false, 12);
}

fn leadjoy(p: &mut Paint<'_>, c: Frame) {
    let (x, w) = (c.x, c.w);
    let (tx, tw) = (x + 26.0, w - 52.0);
    let mut y = c.y;
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
    y += p.wrapped(210, &status, Frame::new(tx, y, tw, 0.0), Type::Body, Role::Ink, false, 12) + 24.0;
    y += row::row(p, 220, Focus::Setting(0), "Refresh", None, Control::Value(if device.pending { "Waiting" } else { "Read" }), Frame::new(x, y, w, 0.0)) + 16.0;
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
        y += row::row(p, 300 + i as u64 * 30, focus, &field.label, status, control, Frame::new(x, y, w, 0.0));
    }
    p.wrapped(1900, "Changes are written to the controller and read back.", Frame::new(tx, y + 24.0, tw, 0.0), Type::Caption, Role::Ink2, false, 12);
}

/// Asks before a Leadjoy profile is deleted.
pub fn delete(p: &mut Paint<'_>) {
    let field = match &p.model.leadjoy.state {
        LeadjoyState::Ready(profile) => p.model.view.edit_field.and_then(|i| profile.schema.fields().nth(i)),
        _ => None,
    };
    let (title, body) = field.map_or_else(|| ("Delete".into(), String::new()), |field| (field.label.clone(), field.help.clone().unwrap_or_default()));
    let actions = [
        Action { label: "Keep", icon: Some(Icon::Back), focus: Focus::SheetAction(0) },
        Action { label: "Delete", icon: Some(Icon::Close), focus: Focus::SheetAction(1) },
    ];
    modal(p, &title, &body, &actions, 0);
    hints::bar(p, &p.model.hints(), Side::Right, p.model.hints_dim);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::model::tests::no_pc_found;

    /// Every place paints its rows inside the pane, focus reachable, at both text sizes.
    #[test]
    fn every_place_fits_its_rows_in_the_pane() {
        let host = lumatv_tokens::Tokens::shipped();
        let mut fonts = ui::Fonts::new();
        let mut m = no_pc_found();
        for percent in [100, 200] {
            m.prefs.text_percent = percent;
            let tokens = lumatv_tokens::themes::game_tokens("rose-pine", ui::lucent::palette::Mode::Dark, &host).unwrap();
            for place in Place::ALL {
                m.open_settings(place);
                m.rebuild_navigation();
                let rows = m.settings_sheet().rows();
                for &i in &rows {
                    m.view.focus = Focus::Setting(i);
                    let mut p = super::super::paint::Paint::new(&tokens, &m, &mut fonts);
                    settings(&mut p);
                    p.fit_focus();
                    let (_, frame) = *p.plan.hits.iter().find(|(id, _)| *id == Focus::Setting(i)).unwrap();
                    let pane = p.plan.nodes.iter().find(|n| n.id == 102).unwrap().frame;
                    assert!(frame.y >= pane.y && frame.y + frame.h <= pane.y + pane.h, "{place:?} row {i} at {percent}%: {frame:?}");
                    super::super::paint::assert_readable(&p.plan, p.fonts, false);
                }
            }
        }
    }
}
