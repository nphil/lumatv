//! The on-screen keyboard and the text-entry layer around it.
//!
//! Node ids of the text-entry layer: 8000 its scrim, 8001 its sheet, 8010 the title, 8011 the field's label, 8020 and 8021 the input,
//! 8100 the keys' words and 8101 the focused key's wash, 8500 and 8501 the action button, 8510 the notice.
use crate::game::screens::{
    paint::{Fill, H, Paint, W},
    parts::motion::Enter,
};
use crate::game::{helper::LeadjoyState, model::{Focus, Layer, settings::Pane}};
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{components::Icon, geometry::Frame, lucent::palette::PaletteRole as Role};
use crate::game::screens::parts::dialog::*;

/// What the text-entry layer shows: a `title`, an optional `label` over the input, the `value` typed so far (`placeholder` while it is
/// empty, dots when `secret`), the key set, and below the keys an `action` button and a `notice`.
#[derive(Clone, Copy)]
pub struct TextEntry<'a> {
    pub title: &'a str,
    pub label: &'a str,
    pub value: &'a str,
    pub placeholder: &'a str,
    pub secret: bool,
    pub numeric: bool,
    /// Lower-case letters rather than capitals.
    pub lower: bool,
    pub action: Option<(&'a str, Icon)>,
    pub notice: Option<&'a str>,
}

/// Whether the keys of the Diagnostics page's fields are wanted: they add `_`.
fn diagnostic(p: &Paint<'_>) -> bool {
    p.model.in_diagnostics() && p.model.view.top() == Some(Layer::EditText)
}

/// The key grid at `id` (see [`grid`]): digits six across, or letters ten across with `_` among them for the Diagnostics page's fields.
pub fn keyboard(p: &mut Paint<'_>, id: u64, x: f32, y: f32, w: f32, numeric: bool) {
    let keys = labels(numeric, p.model.view.keyboard_lower, diagnostic(p));
    grid(p, id, x, y, w, if numeric { 6 } else { 10 }, &keys, 31);
}
/// The keys' labels in focus order: `Focus::Keyboard(i)` is the `i`th.
fn labels(numeric: bool, lower: bool, diagnostic: bool) -> Vec<String> {
    let letters = match (lower, diagnostic) {
        (true, true) => "abcdefghijklmnopqrstuvwxyz0123456789.-:/_",
        (false, true) => "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-:/_",
        (true, false) => "abcdefghijklmnopqrstuvwxyz0123456789.-:/",
        (false, false) => "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-:/",
    };
    if numeric {
        "1234567890".chars().map(|n| n.to_string()).chain(["⌫".into(), "Done".into()]).collect()
    } else {
        letters
            .chars()
            .map(|c| c.to_string())
            .chain(["⌫".into(), "Space".into(), "Done".into(), if lower { "ABC" } else { "abc" }.into()])
            .collect()
    }
}
/// The letter keys `columns` across, without the Diagnostics page's `_`.
pub fn flat(p: &mut Paint<'_>, id: u64, x: f32, y: f32, w: f32, columns: usize, z: i32) {
    let keys = labels(false, p.model.view.keyboard_lower, false);
    grid(p, id, x, y, w, columns, &keys, z);
}
/// `keys` `columns` across as words on the sheet beneath them: one text layer for every key, and only the focused key gets its own
/// layer, the focus wash under its word. `id` is the text layer, `id + 1` the wash. A key of its own per letter cost two layers each,
/// 90 for the letter keys.
#[allow(clippy::too_many_arguments)]
fn grid(p: &mut Paint<'_>, id: u64, x: f32, y: f32, w: f32, columns: usize, keys: &[String], z: i32) {
    let gap = 8.0;
    let width = (w - gap * (columns - 1) as f32) / columns as f32;
    let h = (52.0 * p.scale).min(80.0);
    let rows = keys.len().div_ceil(columns);
    let mut words = Vec::with_capacity(keys.len());
    for (i, key) in keys.iter().enumerate() {
        let (kx, ky) = ((i % columns) as f32 * (width + gap), (i / columns) as f32 * (h + gap));
        let f = Frame::new(x + kx, y + ky, width, h);
        if p.focused(Focus::Keyboard(i)) {
            let wash = p.tokens.states.focus_wash[usize::from(p.mode == ui::lucent::palette::Mode::Light)];
            p.surface(id + 1, f, Fill::Tint(wash), Radius::Control, false, true, false, z);
        }
        let mut word = p.word(key, Type::Caption, p.ink(Role::Ink, false), kx + 2.0, ky + (h - p.line(Type::Caption)) / 2.0, width - 4.0);
        word.align = ui::components::Align::Centre;
        words.push(word);
        p.hit(Focus::Keyboard(i), f);
    }
    p.words(id, Frame::new(x, y, w, rows as f32 * (h + gap) - gap), words, vec![], z + 1);
}
pub fn edit_text(p: &mut Paint<'_>) {
    let model = p.model;
    let editing = model.view.top() == Some(Layer::EditText);
    let diagnostics = editing && model.in_diagnostics();
    let secret = diagnostics && model.view.edit_field == Some(4);
    let title = if diagnostics {
        match model.view.edit_field { Some(2) => "Server URL", Some(3) => "Topic", Some(4) => "Write token", _ => "Edit setting" }
    } else if editing && model.view.pane == Some(Pane::Leadjoy) {
        match &model.leadjoy.state {
            LeadjoyState::Ready(profile) => model.view.edit_field.and_then(|i| profile.schema.fields().nth(i)).map_or("Edit setting", |field| field.label.as_str()),
            _ => "Edit setting",
        }
    } else if editing {
        model
            .view
            .selected_engine
            .as_ref()
            .and_then(|id| model.integration(id))
            .and_then(|e| e.schema.fields().nth(model.view.edit_field?))
            .map_or("Edit setting", |f| f.label.as_str())
    } else {
        "Add a PC"
    };
    plan(
        p,
        &TextEntry {
            title,
            label: "",
            value: &model.view.address,
            placeholder: if editing { "Enter a value" } else { "IP address or hostname" },
            secret,
            numeric: false,
            lower: model.view.keyboard_lower,
            action: Some(if editing { ("Save", Icon::Tick) } else { ("Add PC", Icon::Plus) }),
            notice: model.view.notice.as_deref(),
        },
    );
}

/// Paints the text-entry layer for `entry`: its scrim fades in and its sheet rises 24 px into place as it fades in. The layer opens
/// the scrim itself, so one the caller laid before it (`layer::sheet` does) is replaced by this one, which carries the arrival.
pub fn plan(p: &mut Paint<'_>, entry: &TextEntry<'_>) {
    let (width, height) = if p.scale > 1.0 { (1728.0, 972.0) } else { (1500.0, 850.0) };
    p.plan.nodes.retain(|n| n.id != 8000);
    p.arrives(Frame::new(0.0, 0.0, W, H), Enter::FADE);
    curtain(p);
    p.arrives(Frame::new((W - width) / 2.0, (H - height) / 2.0, width, height), Enter::slide(0.0, 24.0));
    let f = panel(p, width, height);
    let x = f.x + 40.0;
    let w = f.w - 80.0;
    p.text(8010, entry.title, Frame::new(x, f.y + 32.0, w, p.line(Type::Title)), Type::Title, Role::Ink, false, 32);
    let mut y = f.y + 48.0 + p.line(Type::Title);
    if !entry.label.is_empty() {
        p.text(8011, entry.label, Frame::new(x, y, w, p.line(Type::Caption)), Type::Caption, Role::Ink2, false, 32);
        y += p.line(Type::Caption) + 8.0;
    }
    let input_h = p.line(Type::Body) + 24.0;
    p.surface(8020, Frame::new(x, y, w, input_h), Fill::Tint(0.09), Radius::Control, false, false, false, 32);
    let masked = if entry.secret { "•".repeat(entry.value.chars().count()) } else { String::new() };
    p.text(
        8021,
        if entry.value.is_empty() { entry.placeholder } else if entry.secret { &masked } else { entry.value },
        Frame::new(x + 24.0, y + 12.0, w - 48.0, p.line(Type::Body)),
        Type::Body,
        Role::Ink,
        false,
        33,
    );
    let keys = labels(entry.numeric, entry.lower, diagnostic(p));
    grid(p, 8100, x, y + input_h + 16.0, w, if entry.numeric { 6 } else { 10 }, &keys, 31);
    let button_h = p.tokens.target.max(p.line(Type::Label) + 24.0);
    if let Some((label, icon)) = entry.action {
        p.button(8500, Focus::SheetAction(0), label, Some(icon), x, f.y + f.h - 34.0 - button_h, true, false, 34);
    }
    if let Some(notice) = entry.notice {
        p.text(8510, notice, Frame::new(x + 360.0, f.y + f.h - 32.0 - button_h, w - 360.0, button_h), Type::Caption, Role::Warning, false, 34);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::screens::{paint::Body, parts::painted};

    fn entry<'a>() -> TextEntry<'a> {
        TextEntry { title: "Add a PC", label: "", value: "", placeholder: "IP address or hostname", secret: false, numeric: false, lower: false, action: Some(("Add PC", Icon::Plus)), notice: None }
    }
    fn texts(p: &Paint<'_>, id: u64) -> Vec<String> {
        match &p.plan.nodes.iter().find(|n| n.id == id).unwrap_or_else(|| panic!("node {id}")).body {
            Body::Text { words, .. } => words.iter().map(|w| w.text.clone()).collect(),
            _ => panic!("node {id} is not text"),
        }
    }

    #[test]
    fn the_input_shows_its_placeholder_until_there_is_a_value_and_dots_for_a_secret() {
        painted(|_| {}, |p| {
            plan(p, &entry());
            assert_eq!(texts(p, 8021), ["IP address or hostname"]);
        });
        painted(|_| {}, |p| {
            plan(p, &TextEntry { value: "abc", secret: true, ..entry() });
            assert_eq!(texts(p, 8021), ["•••"]);
        });
    }

    #[test]
    fn the_keys_follow_the_case_and_the_numeric_flag_and_the_button_and_notice_only_appear_when_given() {
        painted(|_| {}, |p| {
            plan(p, &TextEntry { lower: true, action: None, notice: Some("Not reachable"), ..entry() });
            assert_eq!(texts(p, 8100)[0], "a");
            assert_eq!(texts(p, 8510), ["Not reachable"]);
            assert!(p.plan.nodes.iter().all(|n| n.id != 8500));
        });
        painted(|_| {}, |p| {
            plan(p, &TextEntry { numeric: true, ..entry() });
            assert_eq!(texts(p, 8100)[..2], ["1", "2"]);
            assert!(p.plan.nodes.iter().any(|n| n.id == 8500));
        });
    }

    #[test]
    fn the_layer_fades_its_scrim_in_and_lifts_its_sheet_and_replaces_a_scrim_laid_before_it() {
        painted(|_| {}, |p| {
            curtain(p);
            plan(p, &entry());
            p.finish();
            assert_eq!(p.plan.nodes.iter().filter(|n| n.id == 8000).count(), 1);
            let scrim = p.plan.enters[&8000];
            let sheet = p.plan.enters[&8001];
            assert_eq!((scrim.dx, scrim.dy), (0.0, 0.0));
            assert_eq!((sheet.dx, sheet.dy), (0.0, 24.0));
            assert_eq!(p.plan.enters[&8500].dy, 24.0);
        });
    }
}
