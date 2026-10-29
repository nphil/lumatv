//! The approved Lucent palettes and material numbers, as constants generated from `assets/lucent/palettes.json` by `build.rs`.
//!
//! This is the one place the palette data enters the program; nothing else holds a colour of a theme (design section 10.1, one
//! token authority). A role here is the straight colour of the data, never premultiplied: [`super::resolve`] turns a variant
//! into the colours of the recipes, once.

use super::material::{CastShadow, Lighting, MaterialData};
use super::palette::{Mode, PaletteRole, Roles, Straight};
use compositor_protocol::Sheen;

/// One soft bloom of a theme's still wallpaper: a Gaussian of `colour` centred on (`x`, `y`) of the 960x480 screen, `sigma`
/// wide, mixed over the canvas with weight `k` at its centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cloud {
    pub x: f32,
    pub y: f32,
    pub sigma: f32,
    pub k: f32,
    pub colour: Straight,
}

/// One mode of a theme: its twenty roles and its wallpaper blooms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Variant {
    pub roles: Roles,
    pub clouds: &'static [Cloud],
}

/// A theme: a name and its two variants.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub id: &'static str,
    pub name: &'static str,
    pub mood: &'static str,
    pub is_default: bool,
    pub dark: Variant,
    pub light: Variant,
}

impl Theme {
    pub fn variant(&self, mode: Mode) -> &Variant {
        match mode {
            Mode::Dark => &self.dark,
            Mode::Light => &self.light,
        }
    }
}

/// What a contrast rule's foreground is: a palette role, or the toggle and slider track as it resolves (`.60` of the ink over its
/// ground, raised until it is 3:1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleFg {
    Role(PaletteRole),
    TrackReadable,
}

/// What a rule is for, which is the floor it sets: glance text 7:1, body text 4.5:1, a hint 3:1 and a control or a focus cue 3:1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleClass {
    Glance,
    Body,
    Hint,
    Control,
}

/// One approved contrast rule: `fg` over the ground named by the role `bg` must reach `min` after composition, at the worst of the
/// wallpaper. The same list holds for every variant of every theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rule {
    pub fg: RuleFg,
    pub bg: PaletteRole,
    pub min: f64,
    pub class: RuleClass,
}

include!(concat!(env!("OUT_DIR"), "/lucent_data.rs"));

/// Every approved theme, in the order of the data.
pub fn themes() -> &'static [Theme] {
    &THEMES
}

/// The theme called `id` (`rose-pine`, `slate`, ...).
pub fn theme(id: &str) -> Option<&'static Theme> {
    THEMES.iter().find(|t| t.id == id)
}

/// The default theme, Rosé Pine.
pub fn default_theme() -> &'static Theme {
    THEMES
        .iter()
        .find(|t| t.is_default)
        .expect("the approved data marks one theme as the default")
}

/// The approved contrast rules, the same for every variant.
pub fn rules() -> &'static [Rule] {
    &RULES
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lucent::palette::PaletteRole;

    #[test]
    fn there_are_ten_themes_with_rose_pine_the_only_default_and_every_id_found() {
        assert_eq!(themes().len(), 10);
        assert_eq!(THEMES.iter().filter(|t| t.is_default).count(), 1);
        assert_eq!(default_theme().id, "rose-pine");
        for t in themes() {
            assert_eq!(theme(t.id).map(|f| f.id), Some(t.id));
            assert_eq!(t.dark.clouds.len(), 5, "{}", t.id);
            assert_eq!(t.light.clouds.len(), 5, "{}", t.id);
        }
        assert!(theme("rose-pine-dawn").is_none());
    }

    #[test]
    fn the_materials_are_the_approved_numbers_not_the_far_lobe_of_the_token_set() {
        let m = MATERIALS;
        assert_eq!(m.depth_offset, 2.0);
        assert_eq!(m.depth_blur, 4.0);
        // One raised shadow of 10/24 in both modes, and the rest shadow of 24/60.
        for l in [&m.dark, &m.light] {
            assert_eq!(
                (l.shadow_raised.offset_y, l.shadow_raised.blur),
                (10.0, 24.0)
            );
            assert_eq!((l.shadow_rest.offset_y, l.shadow_rest.blur), (24.0, 60.0));
            assert_eq!(
                (l.shadow_pressed.offset_y, l.shadow_pressed.blur),
                (6.0, 14.0)
            );
        }
        assert_eq!(m.sheen_over_content(), 0.102);
        // The approved accent of the default dark theme is the contrast-adjusted one, not the seed colour.
        let accent = default_theme().dark.roles[PaletteRole::Accent];
        assert_eq!(accent, Straight::new(0xA9, 0x9D, 0xF5, 255));
    }
}
