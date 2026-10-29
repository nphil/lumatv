//! Hints are glyphs, not sentences (Lucent section 4, 2026-10-29): a hint is the key glyphs of the remote, then what they do (an
//! action glyph), then a label only where no glyph can say it. Holding is the key glyph followed by `hold`, except OK, which has
//! `key-ok-hold`. Each hint has one accessible name: the key names joined with "and", then the action, then the label
//! (`assets/glyphs/index.json`, `compose`).

use super::Kit;
use super::glyph;
use compositor_protocol::{Props, Rgba};
use lumatv_tokens::TypeRole;
use ui::components::{Align, Label, style};
use ui::geometry::Frame;
use ui::lucent::palette::PaletteRole;
use ui::scene::ImageLayer;

/// The glyphs of the remote's keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Ok,
    Up,
    Down,
    Left,
    Right,
    Back,
    Home,
    Menu,
    Mic,
    Red,
    Green,
    Yellow,
    Blue,
}

impl Key {
    pub fn glyph(self) -> &'static str {
        match self {
            Key::Ok => "key-ok",
            Key::Up => "key-up",
            Key::Down => "key-down",
            Key::Left => "key-left",
            Key::Right => "key-right",
            Key::Back => "key-back",
            Key::Home => "key-home",
            Key::Menu => "key-menu",
            Key::Mic => "key-mic",
            Key::Red => "key-red",
            Key::Green => "key-green",
            Key::Yellow => "key-yellow",
            Key::Blue => "key-blue",
        }
    }

    /// The key's accessible name (`index.json`).
    pub fn name(self) -> &'static str {
        match self {
            Key::Ok => "OK",
            Key::Up => "Up",
            Key::Down => "Down",
            Key::Left => "Left",
            Key::Right => "Right",
            Key::Back => "Back",
            Key::Home => "Home",
            Key::Menu => "Menu",
            Key::Mic => "Voice key",
            Key::Red => "Red",
            Key::Green => "Green",
            Key::Yellow => "Yellow",
            Key::Blue => "Blue",
        }
    }
}

/// What the keys do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Expand,
    Collapse,
    Close,
    Open,
    Confirm,
    Options,
    Move,
    Edit,
}

impl Action {
    pub fn glyph(self) -> &'static str {
        match self {
            Action::Expand => "expand",
            Action::Collapse => "collapse",
            Action::Close => "close",
            Action::Open => "open",
            Action::Confirm => "confirm",
            Action::Options => "options",
            Action::Move => "move",
            Action::Edit => "edit",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Action::Expand => "expand",
            Action::Collapse => "collapse",
            Action::Close => "close",
            Action::Open => "open",
            Action::Confirm => "confirm",
            Action::Options => "options",
            Action::Move => "move",
            Action::Edit => "edit",
        }
    }
}

/// One hint: the keys (held or pressed), the action, the label that only data can give.
#[derive(Clone, Debug, PartialEq)]
pub struct Hint {
    pub keys: Vec<(Key, bool)>,
    pub action: Option<Action>,
    pub label: Option<String>,
}

impl Hint {
    pub fn key(key: Key, action: Action) -> Self {
        Self { keys: vec![(key, false)], action: Some(action), label: None }
    }

    pub fn hold(key: Key, action: Action) -> Self {
        Self { keys: vec![(key, true)], action: Some(action), label: None }
    }

    pub fn labelled(key: Key, label: &str) -> Self {
        Self { keys: vec![(key, false)], action: None, label: Some(label.to_string()) }
    }

    /// The glyphs of the hint, in order: a held key is its glyph then `hold`, except OK.
    pub fn glyphs(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        for (key, held) in &self.keys {
            match (key, held) {
                (Key::Ok, true) => out.push("key-ok-hold"),
                (key, true) => {
                    out.push(key.glyph());
                    out.push("hold");
                }
                (key, false) => out.push(key.glyph()),
            }
        }
        if let Some(a) = self.action {
            out.push(a.glyph());
        }
        out
    }

    /// The accessible name: "OK, expand", "Hold OK, options", "Up and Down, move", "Blue, Record".
    pub fn name(&self) -> String {
        let keys: Vec<String> = self.keys.iter().map(|(k, held)| if *held { format!("Hold {}", k.name()) } else { k.name().to_string() }).collect();
        let mut parts = vec![keys.join(" and ")];
        if let Some(a) = self.action {
            parts.push(a.name().to_string());
        }
        if let Some(l) = &self.label {
            parts.push(l.clone());
        }
        parts.join(", ")
    }
}

/// The size of a hint's glyphs beside text of `role`: 1.45 times the role's size, at the person's text size.
pub fn glyph_side(kit: &Kit, role: TypeRole) -> u32 {
    (kit.ty(role).px as f32 * 1.45).round() as u32
}

/// The colour a glyph is drawn in: the colour keys in their own colour, every other glyph in `ink`.
pub fn glyph_colour_of(kit: &Kit, name: &str, ink: Rgba) -> Rgba {
    match name {
        "key-red" => kit.ui.look.colour(PaletteRole::Danger),
        "key-green" => kit.ui.look.colour(PaletteRole::Positive),
        "key-yellow" => kit.ui.look.colour(PaletteRole::Warning),
        "key-blue" => kit.ui.look.colour(PaletteRole::Info),
        _ => ink,
    }
}

/// Where a hint row goes: left-aligned at (`x`, `y`), `h` high, at depth `z`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowAt {
    pub x: f32,
    pub y: f32,
    pub h: f32,
    pub z: i32,
}

/// A hint drawn in a row: the glyphs, then the label in `role`.
pub struct HintRow {
    glyphs: Vec<(ImageLayer, Props)>,
    label: Option<(Label, Props)>,
    width: f32,
    name: String,
}

impl HintRow {
    pub fn new(kit: &mut Kit, hint: &Hint, role: TypeRole, at: RowAt, ink: Rgba) -> std::io::Result<Self> {
        let RowAt { x, y, h, z } = at;
        let side = glyph_side(kit, role);
        let gap = kit.space(2);
        let mut pen = x;
        let mut glyphs = Vec::new();
        for name in hint.glyphs() {
            let at = Props::at(pen, (y + (h - side as f32) / 2.0).round());
            let mut layer = kit.ui.scene.image(side, side, z, at)?;
            let colour = glyph_colour_of(kit, name, ink);
            glyph::draw(name, side, colour, layer.pixels_mut());
            layer.damage(None);
            glyphs.push((layer, at));
            pen += side as f32 + gap;
        }
        let label = match &hint.label {
            Some(text) => {
                let ty = kit.ty(role);
                let width = kit.ui.fonts.measure(text, ty).width.ceil() + 2.0;
                let line = kit.line(role);
                let at = Props::at(pen, (y + (h - line) / 2.0).round());
                let l = Label::new(&mut kit.ui, Frame::new(at.x, at.y, width, line), z, style(ty, ink, Align::Left), text)?;
                pen += width;
                Some((l, at))
            }
            None => {
                pen -= gap;
                None
            }
        };
        Ok(Self { glyphs, label, width: pen - x, name: hint.name() })
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Shows or hides the whole row.
    pub fn set_visible(&mut self, visible: bool) {
        for (g, at) in &self.glyphs {
            g.set(Props { visible, opacity: if visible { 1.0 } else { 0.0 }, ..*at });
        }
        if let Some((l, at)) = &mut self.label {
            l.set_props(Props { visible, opacity: if visible { 1.0 } else { 0.0 }, ..*at });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hint_is_glyphs_then_the_action_then_the_label_and_names_itself() {
        let h = Hint::key(Key::Ok, Action::Expand);
        assert_eq!(h.glyphs(), vec!["key-ok", "expand"]);
        assert_eq!(h.name(), "OK, expand");
        let h = Hint::hold(Key::Ok, Action::Options);
        assert_eq!(h.glyphs(), vec!["key-ok-hold", "options"]);
        assert_eq!(h.name(), "Hold OK, options");
        let h = Hint { keys: vec![(Key::Up, false), (Key::Down, false)], action: Some(Action::Move), label: None };
        assert_eq!(h.glyphs(), vec!["key-up", "key-down", "move"]);
        assert_eq!(h.name(), "Up and Down, move");
        let h = Hint { keys: vec![(Key::Back, true)], action: None, label: Some("Home".into()) };
        assert_eq!(h.glyphs(), vec!["key-back", "hold"]);
        assert_eq!(h.name(), "Hold Back, Home");
        for g in Hint::labelled(Key::Blue, "Record").glyphs() {
            assert!(glyph::known(g), "{g}");
        }
    }
}
