//! LumaTV's one token layer (design-language rule 3): `theme.json`, the file the QML shell, the web pieces (`tools/tokens.py`) and the
//! Rust shell all read, resolved into what the Lucent toolkit draws with. The shipped file (`payload/theme/theme.json`) is the base;
//! the owner's copy (`<data dir>/theme/theme.json`) is merged over it object by object, so a theme that names three colours keeps
//! everything else. Components read tokens only: a colour, a radius, a size or a duration never appears in a screen.
//!
//! The palette's twenty roles map onto Lucent's ([`ui::lucent::palette::PaletteRole`]) and resolve through the toolkit's own
//! resolver ([`ui::lucent::resolve::resolve_variant`]), so the glass, the washes and the readable track are the ones the Show's
//! gate holds; the TV profile's structural numbers (type at 1080p, radii, spacing, targets, focus scales, motion) come from the
//! same file's `type`, `radii`, `space`, `target`, `row`, `focus`, `states` and `motion` blocks. Colours are QML's `#AARRGGBB`.

use compositor_protocol::Rgba;
use serde_json::{Map, Value};
use std::path::Path;
use ui::context::Look;
use ui::lucent::data::{MATERIALS, Variant};
use ui::lucent::material::{MaterialTier, ResolvedTheme, ThemeGenerationId};
use ui::lucent::palette::{Mode, PaletteRole, Roles, Straight};
use ui::lucent::resolve::resolve_variant;
use ui::text::{Type, Weight};

/// The shipped theme, the base every owner's theme is merged over.
pub const SHIPPED: &str = include_str!("../../../payload/theme/theme.json");

/// The screen every TV token is laid out in.
pub const W: f32 = 1920.0;
pub const H: f32 = 1080.0;

/// How the dark or light material is chosen: by the theme, or by the wallpaper's brightness against a threshold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Material {
    Dark,
    Light,
    Auto { threshold: f32 },
}

/// The type roles of the TV profile (`type` in the theme: 92/40/28/26/24 at 1080p).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TypeRole {
    Display,
    Title,
    Body,
    Label,
    Caption,
}

impl TypeRole {
    pub const ALL: [TypeRole; 5] = [Self::Display, Self::Title, Self::Body, Self::Label, Self::Caption];

    pub fn name(self) -> &'static str {
        match self {
            Self::Display => "display",
            Self::Title => "title",
            Self::Body => "body",
            Self::Label => "label",
            Self::Caption => "caption",
        }
    }
}

/// A type role as the theme states it: its size, weight and line height (a multiple of the size).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeSpec {
    pub px: u32,
    pub weight: Weight,
    pub line: f32,
}

impl TypeSpec {
    pub fn ty(self) -> Type {
        Type { px: self.px, weight: self.weight }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Radius {
    Sheet,
    Card,
    Tile,
    Row,
    Control,
    Pill,
}

/// The focus scales and lifts (`focus` in the theme).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Focus {
    pub tile_scale: f32,
    pub card_scale: f32,
    pub row_scale: f32,
    pub button_scale: f32,
    pub press_scale: f32,
    pub card_lift: f32,
    pub landing: f32,
    pub ambient: f32,
}

/// The state washes (`states`), as fractions of the ink.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct States {
    pub focus_wash: [f32; 2],
    pub hover_wash: f32,
    pub selected_wash: f32,
    pub disabled_opacity: f32,
}

/// The one motion set (`motion`), in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub press_ms: u32,
    pub focus_ms: u32,
    pub card_ms: u32,
    pub scroll_ms: u32,
    pub label_ms: u32,
    pub layer_ms: u32,
    pub rest_ms: u32,
    pub hold_ms: u32,
    pub reduced: bool,
    pub transitions: bool,
}

/// Colours of a mode the Lucent roles do not carry: the scrim that hides, the edge of a focused pane, and how much of a pane's
/// glass is painted over an app's picture (`readingAlpha`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extra {
    pub scrim_hide: Rgba,
    pub glass_focus_edge: Rgba,
    pub reading_alpha: f32,
}

/// The resolved token set.
#[derive(Clone, Debug)]
pub struct Tokens {
    json: Value,
    pub name: String,
    pub material: Material,
    dark: Roles,
    light: Roles,
    dark_extra: Extra,
    light_extra: Extra,
    types: [TypeSpec; 5],
    radii: [f32; 6],
    space: [f32; 8],
    pub target: f32,
    pub row: f32,
    pub focus: Focus,
    pub states: States,
    pub motion: Motion,
    pub blur: f32,
    pub saturation: f32,
}

/// A QML colour (`#RRGGBB` or `#AARRGGBB`) as a straight colour.
pub fn colour(text: &str) -> Option<Straight> {
    let digits = text.strip_prefix('#')?;
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
    match digits.len() {
        6 => Some(Straight::new(byte(0)?, byte(2)?, byte(4)?, 255)),
        8 => Some(Straight::new(byte(2)?, byte(4)?, byte(6)?, byte(0)?)),
        _ => None,
    }
}

/// `over` merged into `base`: objects member by member, anything else replaced; nothing (`null`) changes nothing.
pub fn merge(base: &mut Value, over: &Value) {
    match (base, over) {
        (_, Value::Null) => {}
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(k) {
                    Some(existing) if existing.is_object() && v.is_object() => merge(existing, v),
                    _ => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, o) => *b = o.clone(),
    }
}

fn weight(name: &str) -> Weight {
    match name {
        "regular" | "normal" | "light" => Weight::Regular,
        "semibold" | "demibold" | "bold" => Weight::SemiBold,
        _ => Weight::Medium,
    }
}

fn f(v: Option<&Value>, default: f32) -> f32 {
    v.and_then(Value::as_f64).map_or(default, |n| n as f32)
}

fn ms(v: Option<&Value>, default: u32) -> u32 {
    v.and_then(Value::as_f64).filter(|n| n.is_finite() && *n >= 0.0).map_or(default, |n| n.round() as u32)
}

/// The roles of one mode from its colour block. A role the file does not name falls back as the QML `Lu.qml` falls back:
/// `glassRaised` to the focused glass, `edge` to the glass edge, `edgeRaised` to the focused glass edge.
fn roles(block: &Map<String, Value>) -> Roles {
    let get = |names: &[&str]| -> Straight {
        names.iter().find_map(|n| block.get(*n).and_then(Value::as_str).and_then(colour)).unwrap_or(Straight::new(255, 0, 255, 255))
    };
    let mut out = [Straight::new(0, 0, 0, 255); 20];
    for (i, role) in PaletteRole::ALL.iter().enumerate() {
        out[i] = match role {
            PaletteRole::Glass => get(&["glass"]),
            PaletteRole::GlassRaised => get(&["glassRaised", "glassFocus", "glass"]),
            PaletteRole::Edge => get(&["edge", "glassEdge"]),
            PaletteRole::EdgeRaised => get(&["edgeRaised", "glassFocusEdge", "glassEdge"]),
            PaletteRole::Info => get(&["info", "positive"]),
            PaletteRole::Live => get(&["live", "danger"]),
            other => get(&[other.name()]),
        };
    }
    Roles::new(out)
}

fn extra(block: &Map<String, Value>, roles: &Roles) -> Extra {
    let col = |n: &str| block.get(n).and_then(Value::as_str).and_then(colour);
    Extra {
        scrim_hide: col("scrimHide").unwrap_or(roles[PaletteRole::Scrim]).premultiply(0.0),
        glass_focus_edge: col("glassFocusEdge").unwrap_or(roles[PaletteRole::EdgeRaised]).premultiply(0.0),
        reading_alpha: f(block.get("readingAlpha"), 0.8).clamp(0.0, 1.0),
    }
}

impl Tokens {
    /// The shipped theme alone.
    pub fn shipped() -> Self {
        Self::from_json(&Value::Null)
    }

    /// The shipped theme under the owner's `theme/theme.json` in `data_dir`, when there is one.
    pub fn load(data_dir: &Path) -> Self {
        let user = std::fs::read(data_dir.join("theme/theme.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
        Self::from_json(&user)
    }

    /// The shipped theme with `user` merged over it.
    pub fn from_json(user: &Value) -> Self {
        let mut json: Value = serde_json::from_str(SHIPPED).expect("the shipped theme is JSON");
        merge(&mut json, user);
        let empty = Map::new();
        let block = |name: &str| json.get(name).and_then(Value::as_object).unwrap_or(&empty);
        let dark = roles(block("dark"));
        let light = roles(block("light"));
        let dark_extra = extra(block("dark"), &dark);
        let light_extra = extra(block("light"), &light);
        let material = match json.get("material").and_then(Value::as_str) {
            Some("dark") => Material::Dark,
            Some("light") => Material::Light,
            _ => Material::Auto { threshold: f(json.get("materialThreshold"), 0.55) },
        };
        let shipped_px = [92, 40, 28, 26, 24];
        let ty = block("type");
        let mut types = [TypeSpec { px: 26, weight: Weight::Medium, line: 1.3 }; 5];
        for (i, role) in TypeRole::ALL.iter().enumerate() {
            let spec = ty.get(role.name());
            types[i] = TypeSpec {
                px: f(spec.and_then(|s| s.get("size")), shipped_px[i] as f32).round().max(8.0) as u32,
                weight: weight(spec.and_then(|s| s.get("weight")).and_then(Value::as_str).unwrap_or("medium")),
                line: f(spec.and_then(|s| s.get("line")), 1.3).max(1.0),
            };
        }
        let r = block("radii");
        let radii = [
            f(r.get("sheet"), 34.0),
            f(r.get("card"), 26.0),
            f(r.get("tile"), 22.0),
            f(r.get("row"), 22.0),
            f(r.get("control"), 18.0),
            f(r.get("pill"), 999.0),
        ];
        let mut space = [4.0, 8.0, 12.0, 16.0, 20.0, 24.0, 32.0, 40.0];
        if let Some(list) = json.get("space").and_then(Value::as_array) {
            for (i, v) in list.iter().take(8).enumerate() {
                space[i] = f(Some(v), space[i]);
            }
        }
        let fo = block("focus");
        let focus = Focus {
            tile_scale: f(fo.get("tileScale"), 1.12),
            card_scale: f(fo.get("cardScale"), 1.03),
            row_scale: f(fo.get("rowScale"), 1.02),
            button_scale: f(fo.get("buttonScale"), 1.04),
            press_scale: f(fo.get("pressScale"), 0.97),
            card_lift: f(fo.get("cardLift"), 4.0),
            landing: f(fo.get("landing"), 2.0),
            ambient: f(fo.get("ambient"), 0.3),
        };
        let st = block("states");
        let wash = st.get("focusWash").and_then(Value::as_array);
        let states = States {
            focus_wash: [f(wash.and_then(|w| w.first()), 0.05), f(wash.and_then(|w| w.get(1)), 0.06)],
            hover_wash: f(st.get("hoverWash"), 0.04),
            selected_wash: f(st.get("selectedWash"), 0.04),
            disabled_opacity: f(st.get("disabledOpacity"), 0.76),
        };
        let mo = block("motion");
        let motion = Motion {
            press_ms: ms(mo.get("pressMs"), 90),
            focus_ms: ms(mo.get("focusMs"), 150),
            card_ms: ms(mo.get("cardMs"), 180),
            scroll_ms: ms(mo.get("scrollMs"), 300),
            label_ms: ms(mo.get("labelMs"), 120),
            layer_ms: ms(mo.get("layerMs"), 220),
            rest_ms: ms(mo.get("restMs"), 150),
            hold_ms: ms(mo.get("holdMs"), 700),
            reduced: mo.get("reduced").and_then(Value::as_bool).unwrap_or(false),
            transitions: mo.get("transitions").and_then(Value::as_bool).unwrap_or(false),
        };
        Self {
            name: json.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
            material,
            dark,
            light,
            dark_extra,
            light_extra,
            types,
            radii,
            space,
            target: f(json.get("target"), 64.0),
            row: f(json.get("row"), 80.0),
            focus,
            states,
            motion,
            blur: f(json.get("blurRadius"), 30.0),
            saturation: 1.0 + f(json.get("saturation"), 0.25),
            json,
        }
    }

    /// The merged document, for what a screen reads of its own block (`overlays`, `screensaver`, `layout.overlays`).
    pub fn json(&self) -> &Value {
        &self.json
    }

    /// The mode the theme asks for: its own, or by the wallpaper's brightness (0 to 1) against the threshold; dark when the
    /// brightness is not known.
    pub fn mode(&self, wallpaper_luma: Option<f32>) -> Mode {
        match (self.material, wallpaper_luma) {
            (Material::Dark, _) => Mode::Dark,
            (Material::Light, _) => Mode::Light,
            (Material::Auto { threshold }, Some(luma)) if luma >= threshold => Mode::Light,
            (Material::Auto { .. }, _) => Mode::Dark,
        }
    }

    pub fn roles(&self, mode: Mode) -> &Roles {
        match mode {
            Mode::Dark => &self.dark,
            Mode::Light => &self.light,
        }
    }

    pub fn extra(&self, mode: Mode) -> Extra {
        match mode {
            Mode::Dark => self.dark_extra,
            Mode::Light => self.light_extra,
        }
    }

    /// The theme resolved through the toolkit's resolver: the recipes' colours premultiplied once, the readable track, the focus cue.
    pub fn theme(&self, mode: Mode) -> ResolvedTheme {
        resolve_variant(&Variant { roles: *self.roles(mode), clouds: &[] }, mode, &MATERIALS)
    }

    /// The look components are built with. `generation` names it (a new one for every theme change); `text_scale` is the person's
    /// text size, 1 to 2.
    pub fn look(&self, mode: Mode, generation: u64, tier: MaterialTier, text_scale: f32) -> Look {
        let mut look = Look::new(self.theme(mode), ThemeGenerationId(generation), tier);
        look.reduced_motion = self.motion.reduced;
        look.text_scale = text_scale.clamp(1.0, 2.0);
        look
    }

    pub fn spec(&self, role: TypeRole) -> TypeSpec {
        self.types[TypeRole::ALL.iter().position(|r| *r == role).expect("every role")]
    }

    /// A type role at the TV's own size (before the person's text size, which [`Look::ty`] applies).
    pub fn ty(&self, role: TypeRole) -> Type {
        self.spec(role).ty()
    }

    /// `Lu.space(n)`: the n-th step of the spacing scale, 1 to 8 (4, 8, 12, 16, 20, 24, 32, 40).
    pub fn space(&self, n: usize) -> f32 {
        self.space[n.clamp(1, 8) - 1]
    }

    pub fn radius(&self, r: Radius) -> f32 {
        self.radii[match r {
            Radius::Sheet => 0,
            Radius::Card => 1,
            Radius::Tile => 2,
            Radius::Row => 3,
            Radius::Control => 4,
            Radius::Pill => 5,
        }]
    }

    /// A duration with the theme's reduced-motion preference applied, as the toolkit applies it.
    pub fn duration(&self, ms: u32) -> u32 {
        if self.motion.reduced { ms.min(ui::lucent::tokens::motion::REDUCED_MAX_MS) } else { ms }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn qml_colours_are_read_in_qts_argb_order() {
        assert_eq!(colour("#A79BF5"), Some(Straight::new(0xA7, 0x9B, 0xF5, 255)));
        assert_eq!(colour("#66120E1A"), Some(Straight::new(0x12, 0x0E, 0x1A, 0x66)));
        assert_eq!(colour("A79BF5"), None);
        assert_eq!(colour("#12345"), None);
    }

    #[test]
    fn the_shipped_theme_resolves_in_both_modes_with_its_own_roles() {
        let t = Tokens::shipped();
        assert_eq!(t.name, "Rosé Pine silk");
        assert_eq!(t.roles(Mode::Dark)[PaletteRole::Accent], Straight::new(0xA7, 0x9B, 0xF5, 255));
        assert_eq!(t.roles(Mode::Light)[PaletteRole::Accent], Straight::new(0x90, 0x7A, 0xA9, 255));
        assert_eq!(t.roles(Mode::Dark)[PaletteRole::GlassRaised], Straight::new(0x2C, 0x26, 0x3A, 0x8F), "glassFocus stands in for glassRaised");
        let dark = t.theme(Mode::Dark);
        assert_eq!(dark.mode, Mode::Dark);
        assert_eq!(dark.colour(PaletteRole::Ink), Rgba(0xF2, 0xEF, 0xF6, 255));
        let light = t.theme(Mode::Light);
        assert_eq!(light.colour(PaletteRole::Ink), Rgba(0x17, 0x13, 0x1F, 255));
        assert_eq!(t.extra(Mode::Dark).reading_alpha, 0.8);
    }

    #[test]
    fn the_tv_profiles_structural_tokens_come_from_the_file() {
        let t = Tokens::shipped();
        assert_eq!(t.ty(TypeRole::Body), Type { px: 28, weight: Weight::Medium });
        assert_eq!(t.ty(TypeRole::Title), Type { px: 40, weight: Weight::SemiBold });
        assert_eq!(t.ty(TypeRole::Display).px, 92);
        assert_eq!(t.space(6), 24.0);
        assert_eq!(t.space(1), 4.0);
        assert_eq!(t.radius(Radius::Card), 26.0);
        assert_eq!(t.target, 64.0);
        assert_eq!(t.row, 80.0);
        assert_eq!(t.focus.tile_scale, 1.12);
        assert_eq!(t.motion.focus_ms, 150);
        assert_eq!(t.motion.rest_ms, 150);
        assert_eq!(t.blur, 30.0);
        assert_eq!(t.saturation, 1.25);
    }

    #[test]
    fn the_owners_theme_merges_over_the_shipped_one_a_value_at_a_time() {
        let t = Tokens::from_json(&json!({"material": "light", "dark": {"accent": "#FF0000"}, "motion": {"reduced": true}, "space": [5]}));
        assert_eq!(t.material, Material::Light);
        assert_eq!(t.mode(Some(0.1)), Mode::Light);
        assert_eq!(t.roles(Mode::Dark)[PaletteRole::Accent], Straight::new(255, 0, 0, 255));
        assert_eq!(t.roles(Mode::Dark)[PaletteRole::Ink], Straight::new(0xF2, 0xEF, 0xF6, 255), "the rest of the block stays");
        assert!(t.motion.reduced);
        assert_eq!(t.motion.focus_ms, 150);
        assert_eq!(t.space(1), 5.0);
        assert_eq!(t.space(2), 8.0);
        assert_eq!(t.duration(300), 120);
    }

    #[test]
    fn auto_material_follows_the_wallpapers_brightness() {
        let t = Tokens::shipped();
        assert_eq!(t.material, Material::Auto { threshold: 0.55 });
        assert_eq!(t.mode(None), Mode::Dark);
        assert_eq!(t.mode(Some(0.3)), Mode::Dark);
        assert_eq!(t.mode(Some(0.7)), Mode::Light);
    }

    #[test]
    fn the_look_carries_the_text_size_and_the_reduced_motion_preference() {
        let t = Tokens::from_json(&json!({"motion": {"reduced": true}}));
        let look = t.look(Mode::Dark, 3, MaterialTier::T1, 2.0);
        assert_eq!(look.generation, ThemeGenerationId(3));
        assert!(look.reduced_motion);
        assert_eq!(look.ty(t.ty(TypeRole::Label)).px, 52);
    }
}
