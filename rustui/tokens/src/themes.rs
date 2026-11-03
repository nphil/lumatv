//! The TV's local Lucent palette catalog. Source colours are straight RGBA, not Qt ARGB.
use std::sync::LazyLock;
use serde_json::{Map, Value, json};
use ui::lucent::palette::{Mode, PaletteRole, Roles, Straight};

pub struct ThemeFamily {
    pub id: &'static str,
    pub name: &'static str,
    dark: Roles,
    light: Roles,
    wallpaper_dark: Option<[String; 3]>,
    wallpaper_light: Option<[String; 3]>,
}

impl ThemeFamily {
    pub fn roles(&self, mode: Mode) -> &Roles {
        match mode { Mode::Dark => &self.dark, Mode::Light => &self.light }
    }

    /// Only the selected variant is replaced; the host merges objects one level deep.
    pub fn patch(&self, mode: Mode) -> Value {
        let mode_name = match mode { Mode::Dark => "dark", Mode::Light => "light" };
        let roles = self.roles(mode);
        let mut block = Map::new();
        for role in PaletteRole::ALL {
            block.insert(role.name().to_owned(), Value::String(qt_colour(roles[role])));
        }
        for (alias, role) in [("glassEdge", PaletteRole::Edge), ("glassFocus", PaletteRole::GlassRaised), ("glassFocusEdge", PaletteRole::EdgeRaised)] {
            block.insert(alias.to_owned(), Value::String(qt_colour(roles[role])));
        }
        let hide = if self.id == "rose-pine" {
            shipped_block(mode_name)["scrimHide"].clone()
        } else {
            let mut hide = roles[PaletteRole::Canvas];
            hide.a = 248;
            Value::String(qt_colour(hide))
        };
        block.insert("scrimHide".into(), hide);
        let mut patch = Map::new();
        patch.insert("material".into(), json!(mode_name));
        patch.insert("palette".into(), json!({ (mode_name): self.id }));
        let wallpaper = match mode { Mode::Dark => &self.wallpaper_dark, Mode::Light => &self.wallpaper_light };
        let wallpaper = match wallpaper { Some(stops) => json!(stops), None => json!("original") };
        patch.insert("wallpaperPalette".into(), json!({ (mode_name): wallpaper }));
        patch.insert(mode_name.into(), Value::Object(block));
        Value::Object(patch)
    }
}

fn shipped_block(mode: &str) -> &'static Map<String, Value> {
    static SHIPPED: LazyLock<Value> = LazyLock::new(|| serde_json::from_str(crate::SHIPPED).expect("shipped theme"));
    SHIPPED[mode].as_object().expect("shipped theme mode")
}

fn wallpaper_palette(family: &Value, mode: &str) -> [String; 3] {
    let stops = family["wallpaper"][mode].as_array().expect("wallpaper palette");
    assert_eq!(stops.len(), 3, "wallpaper palette needs three stops");
    std::array::from_fn(|i| stops[i].as_str().expect("wallpaper stop").to_owned())
}

fn qt_colour(c: Straight) -> String {
    if c.a == 255 { format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b) }
    else { format!("#{:02X}{:02X}{:02X}{:02X}", c.a, c.r, c.g, c.b) }
}

fn composite(f: Straight, b: Straight) -> Straight {
    let a = f.a as f64 / 255.0;
    let channel = |x: u8, y: u8| (x as f64 * a + y as f64 * (1.0-a)).round() as u8;
    Straight::new(channel(f.r,b.r),channel(f.g,b.g),channel(f.b,b.b),255)
}

fn luminance(c: Straight) -> f64 {
    let linear = |x: u8| { let s = x as f64 / 255.0; if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) } };
    0.2126*linear(c.r) + 0.7152*linear(c.g) + 0.0722*linear(c.b)
}
fn contrast(a: Straight, b: Straight) -> f64 {
    let x = luminance(a); let y = luminance(b);
    (x.max(y)+0.05)/(x.min(y)+0.05)
}

// The Show's sampled-wallpaper gate does not certify the TV's resolved surfaces.
// Fit text and semantic colours against the TV canvas and its composited sheet.
fn fitted(mut c: Straight, backgrounds: [Straight; 2], min: f64, mode: Mode) -> Straight {
    let passes = |v: Straight| backgrounds.iter().all(|b| contrast(composite(v, *b), *b) >= min);
    while c.a < 255 && !passes(c) { c.a += 1; }
    let target = match mode { Mode::Dark => 255, Mode::Light => 0 };
    while !passes(c) && (c.r != target || c.g != target || c.b != target) {
        let step = |v: u8| if v < target { v.saturating_add(1) } else if v > target { v.saturating_sub(1) } else { v };
        c.r = step(c.r); c.g = step(c.g); c.b = step(c.b);
    }
    c
}

fn read_roles(value: &Value, mode: Mode) -> Roles {
    let mut colours = [Straight::new(0,0,0,255); 20];
    for (i, role) in PaletteRole::ALL.iter().enumerate() {
        colours[i] = Straight::parse(value[role.name()].as_str().expect("palette role")).expect("straight RGBA palette colour");
    }
    fit_roles(colours, mode)
}

fn fit_roles(mut colours: [Straight; 20], mode: Mode) -> Roles {
    let get = |role: PaletteRole| PaletteRole::ALL.iter().position(|r| *r == role).expect("role");
    let canvas = colours[get(PaletteRole::Canvas)];
    let sheet = composite(colours[get(PaletteRole::Sheet)], canvas);
    for (role, min) in [(PaletteRole::Ink,7.0), (PaletteRole::Ink2,4.5), (PaletteRole::Ink3,7.0), (PaletteRole::Accent,4.5), (PaletteRole::Positive,4.5), (PaletteRole::Warning,4.5), (PaletteRole::Danger,4.5), (PaletteRole::Info,4.5), (PaletteRole::Live,4.5)] {
        let i = get(role); colours[i] = fitted(colours[i], [canvas,sheet], min, mode);
    }
    for (fg, bg) in [(PaletteRole::AccentInk, PaletteRole::Accent), (PaletteRole::FocusInk, PaletteRole::Focus)] {
        let background = composite(colours[get(bg)], canvas);
        let i = get(fg);
        if contrast(composite(colours[i], background), background) < 4.5 {
            let black = Straight::new(0,0,0,255); let white = Straight::new(255,255,255,255);
            colours[i] = if contrast(black,background) > contrast(white,background) { black } else { white };
        }
    }
    Roles::new(colours)
}

pub fn catalog() -> &'static [ThemeFamily] {
    static SOURCE: LazyLock<Value> = LazyLock::new(|| serde_json::from_str(include_str!("palettes.json")).expect("embedded palettes"));
    static CATALOG: LazyLock<Vec<ThemeFamily>> = LazyLock::new(|| {
        let source = &*SOURCE;
        source.as_array().expect("palette families").iter().map(|family| {
            let id = family["id"].as_str().expect("palette id");
            let (dark, light) = if id == "rose-pine" {
                let dark = crate::roles(shipped_block("dark"));
                let light = crate::roles(shipped_block("light"));
                (fit_roles(std::array::from_fn(|i| dark[PaletteRole::ALL[i]]), Mode::Dark),
                 fit_roles(std::array::from_fn(|i| light[PaletteRole::ALL[i]]), Mode::Light))
            } else {
                (read_roles(&family["dark"], Mode::Dark), read_roles(&family["light"], Mode::Light))
            };
            let wallpaper_dark = (id != "rose-pine").then(|| wallpaper_palette(family, "dark"));
            let wallpaper_light = (id != "rose-pine").then(|| wallpaper_palette(family, "light"));
            ThemeFamily { id, name: family["name"].as_str().expect("palette name"), dark, light, wallpaper_dark, wallpaper_light }
        }).collect()
    });
    &CATALOG
}

pub fn find(id: &str) -> Option<&'static ThemeFamily> { catalog().iter().find(|family| family.id == id) }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Tokens, merge};
    #[test]
    fn straight_rgba_is_converted_to_tv_argb() {
        let c = Straight::parse("#12345678").unwrap();
        assert_eq!(qt_colour(c), "#78123456");
        assert_eq!(crate::colour(&qt_colour(c)), Some(c));
    }
    #[test]
    fn selecting_a_variant_keeps_other_variant_and_custom_settings() {
        let family = find("lagoon").unwrap();
        let mut saved = json!({"dark": {"accent":"#102030"}, "light":{"wallScrim":[0.21,0.31,0.41],"readingAlpha":0.61}, "glassStrength":{"clear":0.3}, "cardGlass":"liquid", "palette":{"dark":"custom"}, "wallpaperPalette":{"dark":["#010101", "#020202", "#030303"], "owner":"keep"}, "wallpaperMotion":{"speed":2}, "depth":{"level":"deep"}});
        merge(&mut saved, &family.patch(Mode::Light));
        let tokens = Tokens::from_json(&saved);
        assert_eq!(tokens.roles(Mode::Light), family.roles(Mode::Light));
        assert_eq!(saved["dark"]["accent"], "#102030");
        assert_eq!(saved["palette"]["dark"], "custom");
        assert_eq!(saved["palette"]["light"], "lagoon");
        assert_eq!(saved["glassStrength"]["clear"], 0.3);
        assert_eq!(saved["cardGlass"], "liquid");
        assert_eq!(saved["wallpaperPalette"]["dark"], json!(["#010101", "#020202", "#030303"]));
        assert_eq!(saved["wallpaperPalette"]["light"], family.patch(Mode::Light)["wallpaperPalette"]["light"]);
        assert_eq!(saved["wallpaperPalette"]["owner"], "keep");
        assert_eq!(saved["wallpaperMotion"]["speed"], 2);
        assert_eq!(saved["depth"]["level"], "deep");
        merge(&mut saved, &find("rose-pine").unwrap().patch(Mode::Light));
        assert_eq!(saved["wallpaperPalette"]["light"], "original");
        assert_eq!(saved["wallpaperPalette"]["dark"], json!(["#010101", "#020202", "#030303"]));
        assert_eq!(saved["wallpaperPalette"]["owner"], "keep");
        assert_eq!(saved["light"]["wallScrim"], json!([0.21,0.31,0.41]));
        assert_eq!(saved["light"]["readingAlpha"], 0.61);
        assert_eq!(saved["dark"]["accent"], "#102030");
        assert_eq!(saved["glassStrength"]["clear"], 0.3);
        assert_eq!(saved["wallpaperMotion"]["speed"], 2);
    }
    #[test]
    fn rose_pine_preserves_shipped_surfaces_after_another_family() {
        let shipped: Value = serde_json::from_str(crate::SHIPPED).unwrap();
        for mode in [Mode::Dark, Mode::Light] {
            let name = match mode { Mode::Dark => "dark", Mode::Light => "light" };
            let mut saved = shipped.clone();
            merge(&mut saved, &find("lagoon").unwrap().patch(mode));
            merge(&mut saved, &find("rose-pine").unwrap().patch(mode));
            assert_eq!(Tokens::from_json(&saved).roles(mode), find("rose-pine").unwrap().roles(mode));
            for key in ["canvas", "sheet", "scrim", "scrimHide", "glass", "glassEdge", "glassFocus", "glassFocusEdge", "wallScrim", "readingAlpha"] {
                assert_eq!(saved[name][key], shipped[name][key], "{name}.{key}");
            }
            assert_eq!(saved["wallpaperPalette"][name], "original");
        }
    }
    #[test]
    fn wallpaper_palettes_preserve_original_and_neutral_families() {
        for mode in [Mode::Dark, Mode::Light] {
            let name = match mode { Mode::Dark => "dark", Mode::Light => "light" };
            let rose = find("rose-pine").unwrap().patch(mode);
            assert_eq!(rose["wallpaperPalette"][name], "original");
            for family in catalog().iter().filter(|f| f.id != "rose-pine") {
                let patch = family.patch(mode);
                let stops = patch["wallpaperPalette"][name].as_array().unwrap();
                assert_eq!(stops.len(), 3);
                let mut previous = None;
                for stop in stops {
                    let s = stop.as_str().unwrap();
                    assert_eq!(s.len(), 7);
                    let c = Straight::parse(s).unwrap();
                    assert_eq!(c.a, 255);
                    if family.id == "monochrome" { assert!(c.r == c.g && c.g == c.b); }
                    let value = luminance(c);
                    if let Some(last) = previous { assert!(value > last, "{} {name} stops must increase in luminance", family.id); }
                    previous = Some(value);
                }
            }
        }
    }
    #[test]
    fn reading_and_state_colours_contrast_with_resolved_surfaces() {
        for family in catalog() { for mode in [Mode::Dark,Mode::Light] {
            let r = family.roles(mode);
            let canvas = r[PaletteRole::Canvas];
            let sheet = composite(r[PaletteRole::Sheet], canvas);
            for (role, min) in [(PaletteRole::Ink,7.0), (PaletteRole::Ink2,4.5), (PaletteRole::Ink3,7.0), (PaletteRole::Accent,4.5), (PaletteRole::Positive,4.5), (PaletteRole::Warning,4.5), (PaletteRole::Danger,4.5), (PaletteRole::Info,4.5), (PaletteRole::Live,4.5)] {
                for bg in [canvas,sheet] { assert!(contrast(composite(r[role],bg),bg) >= min, "{} {:?} {:?}",family.id,mode,role); }
            }
        }}
    }
}
