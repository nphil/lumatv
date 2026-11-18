//! Shared Lucent palettes, stored as straight RGBA rather than Qt ARGB.
//!
//! [`catalog`] retains LumaTV's existing IDs and shipped Rosé Pine surfaces.
//! [`game_catalog`] provides twenty upstream families in dark and light, with
//! 7:1 reading inks and 3:1 control cues over canvas and composited sheet.
//! Sources and adapted counterparts are recorded in `palettes.json`; native
//! accents are never contrast-shifted. Low-contrast accents use the derived
//! `edgeRaised` boundary and `focus` cue rather than colour alone.
//! [`game_options`] adds [`MATCH_LUMATV`], which the app follows from live host
//! tokens instead of treating it as a saved family.
use std::sync::LazyLock;
use serde_json::{Map, Value, json};
use ui::lucent::palette::{Mode, PaletteRole, Roles, Straight};

static SOURCE: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("palettes.json")).expect("embedded palettes")
});

/// The selection follows the host's current tokens rather than a saved palette.
pub const MATCH_LUMATV: &str = "match-lumatv";

pub struct ThemeOption {
    pub id: &'static str,
    pub name: &'static str,
}

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

pub fn composite(f: Straight, b: Straight) -> Straight {
    let a = f.a as f64 / 255.0;
    let channel = |x: u8, y: u8| (x as f64 * a + y as f64 * (1.0-a)).round() as u8;
    Straight::new(channel(f.r,b.r),channel(f.g,b.g),channel(f.b,b.b),255)
}

fn luminance(c: Straight) -> f64 {
    let linear = |x: u8| { let s = x as f64 / 255.0; if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) } };
    0.2126*linear(c.r) + 0.7152*linear(c.g) + 0.0722*linear(c.b)
}
/// WCAG contrast of two colours; the first is composited by the caller where it is translucent.
pub fn contrast(a: Straight, b: Straight) -> f64 {
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

fn raw_roles(value: &Value) -> [Straight; 20] {
    std::array::from_fn(|i| {
        let role = PaletteRole::ALL[i];
        Straight::parse(value[role.name()].as_str().expect("palette role"))
            .expect("straight RGBA palette colour")
    })
}

fn read_roles(value: &Value, mode: Mode) -> Roles {
    fit_roles(raw_roles(value), mode)
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
    static CATALOG: LazyLock<Vec<ThemeFamily>> = LazyLock::new(|| {
        let source = &*SOURCE;
        source.as_array().expect("palette families").iter().filter(|family| {
            family["dark"].is_object()
        }).map(|family| {
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

/// LumaGame's twenty upstream families; legacy LumaTV choices remain in [`catalog`].
/// Brand accents stay unchanged. Reading inks and control cues are distance-fitted.
pub fn game_catalog() -> &'static [ThemeFamily] {
    static CATALOG: LazyLock<Vec<ThemeFamily>> = LazyLock::new(|| {
        SOURCE.as_array().expect("palette families").iter()
            .filter(|family| family["game"].is_object())
            .map(|family| ThemeFamily {
                id: family["id"].as_str().expect("palette id"),
                name: if family["id"] == "rose-pine" { "Rosé Pine" }
                    else { family["name"].as_str().expect("palette name") },
                dark: game_roles(&family["game"]["dark"], Mode::Dark),
                light: game_roles(&family["game"]["light"], Mode::Light),
                wallpaper_dark: None,
                wallpaper_light: None,
            }).collect()
    });
    &CATALOG
}

pub fn game_find(id: &str) -> Option<&'static ThemeFamily> {
    game_catalog().iter().find(|family| family.id == id)
}

/// Match LumaTV first, then the twenty families, with Rosé Pine as the first family.
/// The host resolves [`MATCH_LUMATV`] from its live theme; it is not a family.
pub fn game_options() -> &'static [ThemeOption] {
    static OPTIONS: LazyLock<Vec<ThemeOption>> = LazyLock::new(|| {
        std::iter::once(ThemeOption { id: MATCH_LUMATV, name: "Match LumaTV" })
            .chain(game_catalog().iter().map(|family| ThemeOption {
                id: family.id,
                name: family.name,
            })).collect()
    });
    &OPTIONS
}

/// The game TV profile over current host preferences. Unknown saved IDs are rejected.
/// Named choices populate both variants, including the always-dark artwork stage.
pub fn game_tokens(family: &str, mode: Mode, host: &crate::Tokens) -> Option<crate::Tokens> {
    static PROFILE: LazyLock<Value> = LazyLock::new(|| json!({
        "radii": { "sheet": 36.4, "card": 31.2, "tile": 26, "row": 23.4, "control": 18.2, "pill": 999 },
        "type": {
            "display": { "size": 92, "weight": "medium", "line": 1.04 },
            "title": { "size": 40, "weight": "semibold", "line": 1.2 },
            "body": { "size": 28, "weight": "regular", "line": 1.4 },
            "label": { "size": 26, "weight": "medium", "line": 1.3 },
            "caption": { "size": 24, "weight": "medium", "line": 1.3 }
        },
        "layout": { "edgeX": 96, "edgeY": 54, "gutter": 24 },
        "target": 64,
        "row": 80,
        "focus": { "tileScale": 1.03, "cardScale": 1.03, "rowScale": 1.0, "buttonScale": 1.03,
                   "pressScale": 0.97, "cardLift": 4, "landing": 2 },
        "states": { "focusWash": [0.09, 0.09] },
        "motion": { "focusMs": 150, "cardMs": 150, "layerMs": 180, "exitMs": 120, "artDissolveMs": 200 }
    }));
    let selected = if family == MATCH_LUMATV { None } else { Some(game_find(family)?) };
    let mut value = host.json().clone();
    if let Some(selected) = selected {
        crate::merge(&mut value, &selected.patch(Mode::Dark));
        crate::merge(&mut value, &selected.patch(Mode::Light));
    }
    crate::merge(&mut value, &PROFILE);
    value["material"] = json!(match mode { Mode::Dark => "dark", Mode::Light => "light" });
    Some(crate::Tokens::from_json(&value))
}

/// A primary action's derived fill and its paired ink; palette accents stay intact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrimaryColours {
    pub fill: Straight,
    pub ink: Straight,
}

/// Keep a distance-readable accent pair, or derive a 7:1 primary in the same hue.
pub fn game_primary(roles: &Roles) -> PrimaryColours {
    let canvas = roles[PaletteRole::Canvas];
    let fill = composite(roles[PaletteRole::Accent], canvas);
    let ink = roles[PaletteRole::AccentInk];
    if contrast(composite(ink, fill), fill) >= 7.0 {
        return PrimaryColours { fill, ink };
    }
    let light = luminance(canvas) > 0.179;
    let ink = if light { Straight::new(255, 255, 255, 255) }
        else { Straight::new(0, 0, 0, 255) };
    let mode = if light { Mode::Light } else { Mode::Dark };
    PrimaryColours { fill: fitted(fill, [ink, ink], 7.0, mode), ink }
}

fn game_roles(value: &Value, mode: Mode) -> Roles {
    let mut colours = raw_roles(value);
    let index = |role| PaletteRole::ALL.iter().position(|r| *r == role).expect("role");
    let canvas = colours[index(PaletteRole::Canvas)];
    let sheet = composite(colours[index(PaletteRole::Sheet)], canvas);
    for role in [PaletteRole::Ink, PaletteRole::Ink2, PaletteRole::Ink3] {
        let i = index(role);
        colours[i] = fitted(colours[i], [canvas, sheet], 7.0, mode);
    }
    // Light adaptations retain their brand hues; these derived cues carry visibility.
    for role in [PaletteRole::Focus, PaletteRole::EdgeRaised, PaletteRole::TrackOff,
                 PaletteRole::Positive, PaletteRole::Warning, PaletteRole::Danger,
                 PaletteRole::Info, PaletteRole::Live] {
        let i = index(role);
        colours[i] = fitted(colours[i], [canvas, sheet], 3.0, mode);
    }
    for (ink, fill) in [(PaletteRole::AccentInk, PaletteRole::Accent),
                        (PaletteRole::FocusInk, PaletteRole::Focus)] {
        let background = composite(colours[index(fill)], canvas);
        let i = index(ink);
        if contrast(composite(colours[i], background), background) < 4.5 {
            let black = Straight::new(0, 0, 0, 255);
            let white = Straight::new(255, 255, 255, 255);
            colours[i] = if contrast(black, background) > contrast(white, background) {
                black
            } else {
                white
            };
        }
    }
    Roles::new(colours)
}

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

    #[test]
    fn game_choices_preserve_the_legacy_catalog_and_upstream_accents() {
        let legacy = ["rose-pine", "slate", "sand", "forest", "ocean", "plum",
            "ember", "sage", "cocoa", "monochrome", "glacier", "indigo", "orchid",
            "rosewater", "bordeaux", "lagoon", "olive", "honey", "terracotta", "porcelain"];
        assert_eq!(catalog().iter().map(|f| f.id).collect::<Vec<_>>(), legacy);
        let expected = [
            ("rose-pine", "#C4A7E7", "#907AA9"), ("nord", "#88C0D0", "#5E81AC"),
            ("solarized", "#268BD2", "#268BD2"), ("gruvbox", "#FABD2F", "#B57614"),
            ("catppuccin", "#CBA6F7", "#8839EF"), ("dracula", "#BD93F9", "#644AC9"),
            ("monokai", "#A6E22E", "#A6E22E"), ("ayu", "#E6B450", "#F2AE49"),
            ("everforest", "#A7C080", "#8DA101"), ("kanagawa", "#7E9CD8", "#4D699B"),
            ("tokyo-night", "#7AA2F7", "#2E7DE9"), ("one", "#61AFEF", "#4078F2"),
            ("night-owl", "#A599E9", "#2AA298"), ("github", "#58A6FF", "#0969DA"),
            ("oxocarbon", "#33B1FF", "#0F62FE"), ("horizon", "#DF5273", "#DA103F"),
            ("material-palenight", "#C792EA", "#C792EA"),
            ("synthwave-84", "#FF7EDB", "#FF7EDB"), ("cobalt2", "#FFC600", "#FFC600"),
            ("papercolor", "#5FAFD7", "#005F87"),
        ];
        assert_eq!(game_catalog().len(), expected.len());
        assert_eq!(game_options().len(), 21);
        assert_eq!(game_options()[0].id, MATCH_LUMATV);
        assert_eq!(game_options()[0].name, "Match LumaTV");
        assert!(game_find(MATCH_LUMATV).is_none());
        assert!(game_find("not-a-theme").is_none());
        for (i, (id, dark, light)) in expected.iter().enumerate() {
            let family = game_find(id).unwrap();
            assert_eq!(game_catalog()[i].id, *id);
            assert_eq!(game_options()[i + 1].id, *id);
            let source = SOURCE.as_array().unwrap().iter().find(|f| f["id"] == *id).unwrap();
            assert!(source["game"]["source"].as_str().unwrap().starts_with("https://"));
            assert!(!source["game"]["modes"].as_str().unwrap().is_empty());
            for (mode, name, accent) in [(Mode::Dark, "dark", dark), (Mode::Light, "light", light)] {
                let roles = family.roles(mode);
                assert_eq!(roles[PaletteRole::Accent], Straight::parse(accent).unwrap());
                assert_eq!(roles[PaletteRole::Canvas],
                    Straight::parse(source["game"][name]["canvas"].as_str().unwrap()).unwrap());
                assert_eq!(Tokens::from_json(&family.patch(mode)).roles(mode), roles);
                assert_eq!(source["game"][name].as_object().unwrap().len(), PaletteRole::ALL.len());
            }
        }
    }

    #[test]
    fn all_forty_game_variants_meet_composited_distance_and_control_contrast() {
        for family in game_catalog() {
            for mode in [Mode::Dark, Mode::Light] {
                let roles = family.roles(mode);
                let canvas = roles[PaletteRole::Canvas];
                let sheet = composite(roles[PaletteRole::Sheet], canvas);
                for background in [canvas, sheet] {
                    for role in [PaletteRole::Ink, PaletteRole::Ink2, PaletteRole::Ink3] {
                        let ratio = contrast(composite(roles[role], background), background);
                        assert!(ratio >= 7.0, "{} {mode:?} {role:?}: {ratio}", family.id);
                    }
                    for role in [PaletteRole::Focus, PaletteRole::EdgeRaised, PaletteRole::TrackOff,
                                 PaletteRole::Positive, PaletteRole::Warning, PaletteRole::Danger,
                                 PaletteRole::Info, PaletteRole::Live] {
                        let ratio = contrast(composite(roles[role], background), background);
                        assert!(ratio >= 3.0, "{} {mode:?} {role:?}: {ratio}", family.id);
                    }
                    for (ink, fill) in [(PaletteRole::AccentInk, PaletteRole::Accent),
                                        (PaletteRole::FocusInk, PaletteRole::Focus)] {
                        let fill = composite(roles[fill], background);
                        let ratio = contrast(composite(roles[ink], fill), fill);
                        assert!(ratio >= 4.5, "{} {mode:?} {ink:?}: {ratio}", family.id);
                    }
                }
            }
        }
    }

    fn lab(c: Straight) -> [f64; 3] {
        let linear = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        let [r, g, b] = [linear(c.r), linear(c.g), linear(c.b)];
        let f = |v: f64| {
            if v > 216.0 / 24389.0 { v.cbrt() } else { v * 841.0 / 108.0 + 4.0 / 29.0 }
        };
        let x = f((0.4124564*r + 0.3575761*g + 0.1804375*b) / 0.95047);
        let y = f(0.2126729*r + 0.7151522*g + 0.0721750*b);
        let z = f((0.0193339*r + 0.1191920*g + 0.9503041*b) / 1.08883);
        [116.0*y - 16.0, 500.0*(x-y), 200.0*(y-z)]
    }

    #[test]
    fn dark_families_have_distinct_canvas_and_accent_signatures() {
        for (i, a) in game_catalog().iter().enumerate() {
            for b in &game_catalog()[i + 1..] {
                let mut squared_delta = 0.0;
                for role in [PaletteRole::Canvas, PaletteRole::Accent] {
                    let a = lab(a.roles(Mode::Dark)[role]);
                    let b = lab(b.roles(Mode::Dark)[role]);
                    squared_delta += a.into_iter().zip(b).map(|(a, b)| (a-b).powi(2)).sum::<f64>();
                }
                // Joint CIE76 distance over the two equally weighted identity swatches.
                let delta = (squared_delta / 2.0).sqrt();
                assert!(delta >= 5.0, "{} and {} are too similar: ΔE {delta}", a.id, b.id);
            }
        }
    }

    #[test]
    fn game_profile_uses_game_geometry_and_preserves_live_host_colours_and_preferences() {
        let host = Tokens::from_json(&json!({
            "dark": { "accent": "#123456" }, "light": { "accent": "#654321" },
            "motion": { "reduced": true }, "text": { "gridFit": false }
        }));
        for mode in [Mode::Dark, Mode::Light] {
            let matched = game_tokens(MATCH_LUMATV, mode, &host).unwrap();
            for variant in [Mode::Dark, Mode::Light] {
                assert_eq!(matched.roles(variant), host.roles(variant));
            }
            assert_eq!(matched.mode(None), mode);
            for (role, expected) in [(crate::Radius::Sheet, 36.4), (crate::Radius::Card, 31.2),
                (crate::Radius::Tile, 26.0), (crate::Radius::Row, 23.4), (crate::Radius::Control, 18.2)] {
                assert_eq!(matched.radius(role), expected);
            }
            assert_eq!(matched.ty(crate::TypeRole::Body).weight, ui::text::Weight::Regular);
            assert_eq!(matched.focus.tile_scale, 1.03);
            assert_eq!(matched.focus.card_scale, 1.03);
            assert_eq!(matched.focus.button_scale, 1.03);
            assert_eq!(matched.focus.row_scale, 1.0);
            assert_eq!(matched.focus.card_lift, 4.0);
            assert_eq!(matched.states.focus_wash, [0.09, 0.09]);
            assert_eq!(matched.motion.focus_ms, 150);
            assert_eq!(matched.motion.layer_ms, 180);
            assert!(matched.motion.reduced);
            assert!(!matched.grid_fit());
            assert_eq!(matched.json()["layout"]["edgeX"], 96);
            assert_eq!(matched.json()["layout"]["edgeY"], 54);
            for family in game_catalog() {
                let named = game_tokens(family.id, mode, &host).unwrap();
                for variant in [Mode::Dark, Mode::Light] {
                    assert_eq!(named.roles(variant), family.roles(variant));
                }
                assert!(named.motion.reduced);
            }
        }
        let changed_host = Tokens::from_json(&json!({ "dark": { "accent": "#ABCDEF" } }));
        assert_eq!(game_tokens(MATCH_LUMATV, Mode::Dark, &changed_host).unwrap().roles(Mode::Dark),
            changed_host.roles(Mode::Dark));
        assert!(game_tokens("unknown", Mode::Dark, &host).is_none());
        assert_eq!(host.focus.tile_scale, Tokens::shipped().focus.tile_scale);
        assert_eq!(host.roles(Mode::Dark)[PaletteRole::Accent], Straight::parse("#123456").unwrap());
    }

    #[test]
    fn every_game_primary_is_distance_readable_without_changing_brand_roles() {
        for family in game_catalog() {
            for mode in [Mode::Dark, Mode::Light] {
                let roles = family.roles(mode);
                let before = *roles;
                let primary = game_primary(roles);
                assert!(contrast(composite(primary.ink, primary.fill), primary.fill) >= 7.0,
                    "{} {mode:?}", family.id);
                assert_eq!(before, *roles);
                if contrast(composite(roles[PaletteRole::AccentInk], roles[PaletteRole::Accent]),
                            roles[PaletteRole::Accent]) >= 7.0 {
                    assert_eq!(primary.fill, roles[PaletteRole::Accent]);
                    assert_eq!(primary.ink, roles[PaletteRole::AccentInk]);
                }
            }
        }
        let light = game_find("rose-pine").unwrap().roles(Mode::Light);
        assert!(luminance(game_primary(light).fill) < luminance(light[PaletteRole::Accent]));
    }
}
