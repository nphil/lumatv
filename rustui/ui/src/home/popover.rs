//! The status bar's settings popover (`StatusBar.qml`, `popoverType: "settings"`): a glass panel under the gear with the wallpaper's motion in
//! live rows. Up and Down move, Left and Right change the value of the row and Home behind shows it at once (the cards and the dock step back so
//! the wallpaper is seen), OK on a toggle turns it and OK on "TV settings" opens LG's. What was changed is written to the
//! owner's theme when the popover closes (`wallpaperMotion`, one level deep).
//!
//! The rows share their ranges with the GPU wallpaper consumer; changes are previewed live and persisted on close.

use crate::kit::focus::{Region, Scope};
use serde_json::{Map, Value, json};
use ui::geometry::Frame;

pub const SCOPE: Scope = Scope(5);
pub const REGION: Region = Region(30);
pub const ROW_ID: u32 = 800;
pub const WIDTH: f32 = 440.0;
/// The panel's padding, the gap under the bar, and the rows' own height and gap at 100 % text.
pub const PAD: f32 = 20.0;
pub const GAP: f32 = 8.0;
/// The space between the head and the first row.
pub const HEAD_GAP: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// A row that opens LG's settings.
    Action,
    Toggle,
    Slider { min: f64, max: f64, step: f64, unit: &'static str },
}

#[derive(Clone, Copy, Debug)]
pub struct Def {
    pub label: &'static str,
    pub key: &'static str,
    pub kind: Kind,
    pub default: f64,
}

const fn slider(label: &'static str, key: &'static str, default: f64, min: f64, max: f64, step: f64, unit: &'static str) -> Def {
    Def { label, key, kind: Kind::Slider { min, max, step, unit }, default }
}

pub const ROWS: [Def; 13] = [
    Def { label: "Wallpaper motion", key: "enabled", kind: Kind::Toggle, default: 1.0 },
    slider("Amount", "amplitude", 4.0, 0.0, 16.0, 1.0, " px"),
    slider("Speed", "speed", 1.0, 0.2, 3.0, 0.1, "x"),
    slider("Depth", "depth", 0.25, 0.0, 1.0, 0.05, ""),
    slider("Wave size", "scale", 1.0, 0.5, 2.5, 0.1, "x"),
    slider("Detail", "detail", 0.35, 0.0, 1.0, 0.05, ""),
    slider("Frame rate", "fps", 10.0, 5.0, 60.0, 5.0, " fps"),
    slider("Vignette", "vignette", 0.0, 0.0, 1.0, 0.05, ""),
    slider("Specular", "specular", 0.0, 0.0, 1.0, 0.05, ""),
    slider("Bloom", "bloom", 0.0, 0.0, 1.0, 0.05, ""),
    slider("Saturation", "saturation", 1.0, 0.5, 1.5, 0.05, ""),
    slider("Contrast", "contrast", 1.0, 0.8, 1.3, 0.05, ""),
    Def { label: "TV settings", key: "tvsettings", kind: Kind::Action, default: 0.0 },
];

/// What a row did when it was pressed.
#[derive(Clone, Debug, PartialEq)]
pub enum Done {
    Nothing,
    /// A value changed: the key of `wallpaperMotion` and what it is now.
    Changed(&'static str, Value),
    TvSettings,
}

#[derive(Clone, Debug)]
pub struct Popover {
    pub index: usize,
    /// The status bar's item the panel hangs under.
    pub item: usize,
    /// The first row in view when the panel is taller than the screen has room for.
    pub first: usize,
    dirty: Map<String, Value>,
}

/// The value of a row now, from the theme's `wallpaperMotion`.
pub fn number(def: &Def, wm: &Value) -> f64 {
    let value = wm.get(def.key).and_then(Value::as_f64).filter(|n| n.is_finite()).unwrap_or(def.default);
    match def.kind {
        Kind::Slider { min, max, .. } => value.clamp(min, max),
        _ => value,
    }
}

pub fn is_on(def: &Def, wm: &Value) -> bool {
    wm.get(def.key).and_then(Value::as_bool).unwrap_or(def.default != 0.0)
}

/// A wallpaper control, using exactly the bounds and defaults displayed by the popover.
pub fn control(key: &str, wm: &Value) -> f32 {
    number(ROWS.iter().find(|d| d.key == key).expect("wallpaper control"), wm) as f32
}

/// The value as the row says it: "On", "4 px", "1.0x", "0.25".
pub fn text(def: &Def, wm: &Value) -> String {
    match def.kind {
        Kind::Action => String::new(),
        Kind::Toggle => if is_on(def, wm) { "On" } else { "Off" }.to_owned(),
        Kind::Slider { step, unit, .. } => {
            let n = number(def, wm);
            let v = if step >= 1.0 { format!("{}", n.round() as i64) } else if step >= 0.1 { format!("{n:.1}") } else { format!("{n:.2}") };
            format!("{v}{unit}")
        }
    }
}

impl Popover {
    pub fn new(item: usize) -> Self {
        Self { index: 0, item, first: 0, dirty: Map::new() }
    }

    pub fn def(&self) -> &'static Def {
        &ROWS[self.index.min(ROWS.len() - 1)]
    }

    pub fn moved(&mut self, d: i32) {
        self.index = (self.index as i32 + d).clamp(0, ROWS.len() as i32 - 1) as usize;
    }

    /// Left or Right on the row: a toggle turns, a slider takes a step (to the step's grid, inside its range), an action does nothing.
    pub fn adjust(&mut self, dir: i32, wm: &Value) -> Done {
        let def = self.def();
        let v = match def.kind {
            Kind::Action => return Done::Nothing,
            Kind::Toggle => json!(!is_on(def, wm)),
            Kind::Slider { min, max, step, .. } => {
                let v = ((number(def, wm) + f64::from(dir) * step) / step).round() * step;
                json!((v.clamp(min, max) * 100.0).round() / 100.0)
            }
        };
        self.dirty.insert(def.key.to_owned(), v.clone());
        Done::Changed(def.key, v)
    }

    /// OK on the row: a toggle turns, a slider has nothing to press, the actions go.
    pub fn press(&mut self, wm: &Value) -> Done {
        match self.def().kind {
            Kind::Toggle => self.adjust(1, wm),
            Kind::Slider { .. } => Done::Nothing,
            Kind::Action => Done::TvSettings,
        }
    }

    /// What the panel changed, to be written to the theme when it closes.
    pub fn take_dirty(&mut self) -> Option<Value> {
        (!self.dirty.is_empty()).then(|| Value::Object(std::mem::take(&mut self.dirty)))
    }

    /// The row heights at a text size: the head ("Wallpaper"), a row and its pitch.
    pub fn metrics(text_scale: f32) -> (f32, f32, f32) {
        let line = (26.0 * 1.3 * text_scale.max(1.0)).ceil();
        let row = 40.0f32.max(line + 8.0);
        (34.0f32.max(line + 6.0), row, row + 6.0)
    }

    /// Where the panel is and which rows are in view: under the item of the bar it hangs under, from the item's right edge (every item sits at the bar's
    /// right end), `bar` the status row's frame and `item` the item's. A panel taller than the room scrolls its rows with the focus.
    pub fn layout(&mut self, bar: Frame, item: Frame, text_scale: f32) -> Layout {
        let (head_h, row_h, pitch) = Self::metrics(text_scale);
        let width = (WIDTH * text_scale.max(1.0)).round().min(bar.w);
        let top = bar.bottom() + GAP;
        let room = 1080.0 - bar.y - top - 2.0 * PAD - HEAD_GAP;
        let fit = (((room - head_h) / pitch).floor() as usize).clamp(1, ROWS.len());
        if self.index < self.first {
            self.first = self.index;
        } else if self.index >= self.first + fit {
            self.first = self.index + 1 - fit;
        }
        self.first = self.first.min(ROWS.len() - fit);
        let x = (item.right() - width).min(bar.right() - width).max(bar.x);
        let pane = Frame::new(x.round(), top.round(), width, 2.0 * PAD + head_h + HEAD_GAP + fit as f32 * pitch - (pitch - row_h));
        let rows = (self.first..self.first + fit).map(|i| (i, Frame::new(pane.x + PAD, pane.y + PAD + head_h + HEAD_GAP + (i - self.first) as f32 * pitch, pane.w - 2.0 * PAD, row_h))).collect();
        Layout { pane, head: Frame::new(pane.x + PAD, pane.y + PAD, pane.w - 2.0 * PAD, head_h), rows }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub pane: Frame,
    pub head: Frame,
    /// The rows in view: the row's number and where it is.
    pub rows: Vec<(usize, Frame)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slider_steps_on_its_own_grid_inside_its_range_and_says_its_value_as_the_qml_did() {
        let mut p = Popover::new(2);
        p.index = 1;
        let wm = json!({});
        assert_eq!(text(&ROWS[1], &wm), "4 px");
        assert_eq!(p.adjust(1, &wm), Done::Changed("amplitude", json!(5.0)));
        assert_eq!(text(&ROWS[1], &json!({"amplitude": 5})), "5 px");
        assert_eq!(p.adjust(-1, &json!({"amplitude": 0})), Done::Changed("amplitude", json!(0.0)), "never below the range");
        assert_eq!(p.adjust(1, &json!({"amplitude": 16})), Done::Changed("amplitude", json!(16.0)), "nor above");
        p.index = 2;
        assert_eq!(p.adjust(1, &json!({"speed": 0.95})), Done::Changed("speed", json!(1.1)), "a step of 0.1 lands on the grid, as the QML's rounding did");
        assert_eq!(text(&ROWS[2], &json!({"speed": 1.0})), "1.0x");
        p.index = 3;
        assert_eq!(text(&ROWS[3], &json!({})), "0.25");
        assert_eq!(p.adjust(-1, &json!({"depth": 0.25})), Done::Changed("depth", json!(0.2)));
        p.index = 6;
        assert_eq!(p.adjust(1, &json!({})), Done::Changed("fps", json!(15.0)));
        assert_eq!(text(&ROWS[6], &json!({})), "10 fps");
    }

    #[test]
    fn the_toggle_turns_on_left_right_and_ok_and_the_actions_only_on_ok() {
        let mut p = Popover::new(0);
        assert_eq!(text(&ROWS[0], &json!({})), "On", "the motion is on unless the theme says otherwise");
        assert_eq!(p.adjust(1, &json!({})), Done::Changed("enabled", json!(false)));
        assert_eq!(p.press(&json!({"enabled": false})), Done::Changed("enabled", json!(true)));
        assert_eq!(text(&ROWS[0], &json!({"enabled": false})), "Off");
        p.index = 12;
        assert_eq!(p.adjust(1, &json!({})), Done::Nothing);
        assert_eq!(p.press(&json!({})), Done::TvSettings);
        p.index = 4;
        assert_eq!(p.press(&json!({})), Done::Nothing, "a slider has nothing to press");
    }

    #[test]
    fn what_was_changed_is_kept_for_the_theme_once_and_the_panel_is_under_its_item_inside_the_bar() {
        let mut p = Popover::new(0);
        p.index = 1;
        p.adjust(1, &json!({}));
        p.index = 2;
        p.adjust(1, &json!({}));
        assert_eq!(p.take_dirty(), Some(json!({"amplitude": 5.0, "speed": 1.1})));
        assert_eq!(p.take_dirty(), None);
        let bar = Frame::new(64.0, 54.0, 1792.0, 60.0);
        let right = p.layout(bar, Frame::new(1744.0, 60.0, 48.0, 48.0), 1.0);
        assert_eq!((right.pane.x + right.pane.w, right.pane.y), (1792.0, 122.0), "from the item's right edge, 8 under the bar");
        let wide = p.layout(bar, Frame::new(70.0, 60.0, 48.0, 48.0), 1.0);
        assert_eq!(wide.pane.x, 64.0, "kept inside the bar's row");
        assert_eq!(right.rows.len(), 13, "all the rows fit at 100 %");
        assert!(right.pane.bottom() <= 1080.0 - 54.0);
    }

    #[test]
    fn a_panel_taller_than_the_room_scrolls_its_rows_with_the_focus() {
        let mut p = Popover::new(0);
        let bar = Frame::new(64.0, 54.0, 1792.0, 120.0);
        let l = p.layout(bar, Frame::new(1700.0, 60.0, 96.0, 96.0), 2.0);
        assert!(l.rows.len() < 13 && l.rows[0].0 == 0, "{}", l.rows.len());
        assert!(l.pane.bottom() <= 1080.0 - 54.0 + 0.5, "{:?}", l.pane);
        p.index = 12;
        let l = p.layout(bar, Frame::new(1700.0, 60.0, 96.0, 96.0), 2.0);
        assert_eq!(l.rows.last().map(|r| r.0), Some(12), "the last row is in view");
        p.index = 0;
        assert_eq!(p.layout(bar, Frame::new(1700.0, 60.0, 96.0, 96.0), 2.0).rows[0].0, 0);
    }
}
