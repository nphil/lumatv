//! The on-screen keyboard and the text-entry layer around it.
use crate::game::screens::paint::{Fill, Paint};
use crate::game::{helper::LeadjoyState, model::{Focus, Layer, Page, settings::Place}};
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{components::Icon, geometry::Frame, lucent::palette::PaletteRole as Role};
use crate::game::screens::parts::dialog::*;

pub fn keyboard(p: &mut Paint<'_>, id: u64, x: f32, y: f32, w: f32, numeric: bool, stage: bool) {
    let diagnostic = p.model.view.page == Page::Settings && p.model.view.place == crate::game::model::settings::Place::Diagnostics && p.model.view.sheet == Some(crate::game::model::Layer::EditText);
    let letters = match (p.model.view.keyboard_lower, diagnostic) {
        (true, true) => "abcdefghijklmnopqrstuvwxyz0123456789.-:/_",
        (false, true) => "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-:/_",
        (true, false) => "abcdefghijklmnopqrstuvwxyz0123456789.-:/",
        (false, false) => "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-:/",
    };
    let keys: Vec<String> = if numeric {
        "1234567890".chars().map(|n| n.to_string()).chain(["⌫".into(), "Done".into()]).collect()
    } else {
        letters
            .chars()
            .map(|c| c.to_string())
            .chain(["⌫".into(), "Space".into(), "Done".into(), if p.model.view.keyboard_lower { "ABC" } else { "abc" }.into()])
            .collect()
    };
    let columns = if numeric { 6 } else { 10 };
    let gap = 8.0;
    let width = (w - gap * (columns - 1) as f32) / columns as f32;
    let h = (52.0 * p.scale).min(80.0);
    for (i, key) in keys.iter().enumerate() {
        let f = Frame::new(x + (i % columns) as f32 * (width + gap), y + (i / columns) as f32 * (h + gap), width, h);
        let from = p.plan.nodes.len();
        p.surface(id + i as u64 * 3, f, Fill::Tint(0.06), Radius::Control, stage, p.focused(Focus::Keyboard(i)), false, 30);
        let mut word = p.word(key, Type::Caption, p.ink(Role::Ink, stage), 2.0, (h - p.line(Type::Caption)) / 2.0, width - 4.0);
        word.align = ui::components::Align::Centre;
        p.words(id + i as u64 * 3 + 1, f, vec![word], vec![], 31);
        if p.focused(Focus::Keyboard(i)) {
            p.zoom(from, f, p.tokens.focus.button_scale);
        }
        p.hit(Focus::Keyboard(i), f);
    }
}
pub fn edit_text(p: &mut Paint<'_>) {
    let editing = p.model.view.sheet == Some(Layer::EditText);
    let diagnostics = editing && p.model.view.page == Page::Settings && p.model.view.place == Place::Diagnostics;
    let secret = diagnostics && p.model.view.edit_field == Some(4);
    let label = if diagnostics {
        match p.model.view.edit_field { Some(2) => "Server URL", Some(3) => "Topic", Some(4) => "Write token", _ => "Edit setting" }
    } else if editing && p.model.view.page == Page::Leadjoy {
        match &p.model.leadjoy.state {
            LeadjoyState::Ready(profile) => p.model.view.edit_field.and_then(|i| profile.schema.fields().nth(i)).map_or("Edit setting", |field| field.label.as_str()),
            _ => "Edit setting",
        }
    } else if editing {
        p.model
            .view
            .selected_engine
            .as_ref()
            .and_then(|id| p.model.integration(id))
            .and_then(|e| e.schema.fields().nth(p.model.view.edit_field?))
            .map_or("Edit setting", |f| f.label.as_str())
    } else {
        "Add a PC"
    };
    let f = panel(p, if p.scale > 1.0 { 1728.0 } else { 1500.0 }, if p.scale > 1.0 { 972.0 } else { 850.0 });
    let x = f.x + 40.0;
    let w = f.w - 80.0;
    p.text(8010, label, Frame::new(x, f.y + 32.0, w, p.line(Type::Title)), Type::Title, Role::Ink, false, 32);
    let y = f.y + 48.0 + p.line(Type::Title);
    let input_h = p.line(Type::Body) + 24.0;
    p.surface(8020, Frame::new(x, y, w, input_h), Fill::Tint(0.09), Radius::Control, false, false, false, 32);
    let placeholder = if editing { "Enter a value" } else { "IP address or hostname" };
    let masked = if secret { "•".repeat(p.model.view.address.chars().count()) } else { String::new() };
    p.text(
        8021,
        if p.model.view.address.is_empty() { placeholder } else if secret { &masked } else { &p.model.view.address },
        Frame::new(x + 24.0, y + 12.0, w - 48.0, p.line(Type::Body)),
        Type::Body,
        Role::Ink,
        false,
        33,
    );
    keyboard(p, 8100, x, y + input_h + 16.0, w, false, false);
    let button_h = p.tokens.target.max(p.line(Type::Label) + 24.0);
    p.button(
        8500,
        Focus::SheetAction(0),
        if editing { "Save" } else { "Add PC" },
        Some(if editing { Icon::Tick } else { Icon::Plus }),
        x,
        f.y + f.h - 34.0 - button_h,
        true,
        false,
        34,
    );
    if let Some(notice) = &p.model.view.notice {
        p.text(8510, notice, Frame::new(x + 360.0, f.y + f.h - 32.0 - button_h, w - 360.0, button_h), Type::Caption, Role::Warning, false, 34);
    }
}
