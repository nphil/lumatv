//! Turns the approved Lucent palette data (`assets/lucent/palettes.json`) into Rust constants, so that no JSON is parsed while
//! the program runs. The file is read as it is; this script only checks that its shape is the one `ui::lucent::data` expects
//! and writes `lucent_data.rs` into `OUT_DIR`.

use serde_json::Value;
use std::fmt::Write as _;
use std::{env, fs, path::PathBuf};

const ROLES: [&str; 20] = [
    "canvas",
    "sheet",
    "scrim",
    "glass",
    "glassRaised",
    "edge",
    "edgeRaised",
    "ink",
    "ink2",
    "ink3",
    "focus",
    "focusInk",
    "trackOff",
    "accent",
    "accentInk",
    "positive",
    "warning",
    "danger",
    "info",
    "live",
];

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key]
        .as_str()
        .unwrap_or_else(|| panic!("palettes.json: {key} is not text"))
}

fn number(v: &Value, what: &str) -> f64 {
    v.as_f64()
        .unwrap_or_else(|| panic!("palettes.json: {what} is not a number"))
}

/// `#RRGGBB` or `#RRGGBBAA` as the source of a `Straight::new(..)`.
fn colour(hex: &str) -> String {
    let digits = hex.strip_prefix('#').expect("a colour starts with #");
    assert!(
        (digits.len() == 6 || digits.len() == 8) && digits.bytes().all(|b| b.is_ascii_hexdigit()),
        "palettes.json: {hex} is not a colour"
    );
    let byte = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).unwrap();
    let a = if digits.len() == 8 { byte(6) } else { 255 };
    format!("Straight::new({}, {}, {}, {a})", byte(0), byte(2), byte(4))
}

fn shadow(v: &Value, what: &str) -> String {
    let [y, blur, alpha] = v.as_array().map(Vec::as_slice).unwrap_or_default() else {
        panic!("palettes.json: {what} is not [offset, blur, alpha]");
    };
    format!(
        "CastShadow {{ offset_y: {:?}, blur: {:?}, alpha: {:?} }}",
        number(y, what),
        number(blur, what),
        number(alpha, what)
    )
}

fn lighting(v: &Value, what: &str) -> String {
    let n = |key: &str| format!("{:?}", number(&v[key], &format!("{what}.{key}")));
    format!(
        "Lighting {{ highlight_rest: {}, highlight_raised: {}, shadow_rest: {}, shadow_raised: {}, \
         shadow_pressed: CastShadow {{ offset_y: 6.0, blur: 14.0, alpha: {} }}, depth_light: {}, depth_shade: {} }}",
        n("highlightRest"),
        n("highlightRaised"),
        shadow(&v["shadowRest"], what),
        shadow(&v["shadowRaised"], what),
        // Lucent's interaction-only pressed shadow: 0 6px 14px, .30 on dark and .16 on light (tokens.json `shadow.pressed`).
        if what == "dark" { "0.30" } else { "0.16" },
        n("depthLight"),
        n("depthShade"),
    )
}

/// `glassRaised` as the name of the `PaletteRole` variant, `GlassRaised`.
fn camel(name: &str) -> String {
    let mut chars = name.chars();
    let first = chars.next().expect("a role name").to_ascii_uppercase();
    first.to_string() + chars.as_str()
}

fn variant(v: &Value, what: &str) -> String {
    let roles = &v["roles"];
    let list: Vec<String> = ROLES
        .iter()
        .map(|name| {
            colour(
                roles[name]
                    .as_str()
                    .unwrap_or_else(|| panic!("palettes.json: {what} has no role {name}")),
            )
        })
        .collect();
    let clouds = v["wallpaper"]["clouds"]
        .as_array()
        .unwrap_or_else(|| panic!("palettes.json: {what} has no wallpaper clouds"));
    let clouds: Vec<String> = clouds
        .iter()
        .map(|c| {
            format!(
                "Cloud {{ x: {:?}, y: {:?}, sigma: {:?}, k: {:?}, colour: {} }}",
                number(&c["x"], "cloud x") as f32,
                number(&c["y"], "cloud y") as f32,
                number(&c["sigma"], "cloud sigma") as f32,
                number(&c["k"], "cloud k") as f32,
                colour(text(c, "colour"))
            )
        })
        .collect();
    format!(
        "Variant {{ roles: Roles::new([{}]), clouds: &[{}] }}",
        list.join(", "),
        clouds.join(", ")
    )
}

fn main() {
    let path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../../../LumaShow/lumashow/assets/lucent/palettes.json");
    println!("cargo:rerun-if-changed={}", path.display());
    println!("cargo:rerun-if-changed=build.rs");
    let data: Value = serde_json::from_str(&fs::read_to_string(&path).expect("palettes.json"))
        .expect("palettes.json is JSON");
    assert_eq!(
        text(&data, "schema"),
        "lucent-palette-set/1",
        "an unknown palette schema"
    );
    let order: Vec<&str> = data["roleOrder"]
        .as_array()
        .expect("roleOrder")
        .iter()
        .map(|r| r.as_str().expect("a role name"))
        .collect();
    assert_eq!(order, ROLES, "the role order of the data changed");

    let m = &data["materials"];
    let sheen = &m["sheen"];
    let n = |v: &Value, what: &str| format!("{:?}", number(v, what));
    let mut out = String::new();
    writeln!(
        out,
        "pub const MATERIALS: MaterialData = MaterialData {{\n    sheen: Sheen {{ peak: {}, floor: {}, start: {}, end: {} }},\n    blur: {},\n    saturation: {},\n    refraction: {},\n    edge_width: {},\n    depth_offset: {},\n    depth_blur: {},\n    dark: {},\n    light: {},\n}};",
        number(&sheen["peak"], "peak") as f32,
        number(&sheen["floor"], "floor") as f32,
        number(&sheen["start"], "start") as f32,
        number(&sheen["end"], "end") as f32,
        n(&m["backdrop"]["blur"], "blur"),
        n(&m["backdrop"]["saturation"], "saturation"),
        n(&m["backdrop"]["refraction"], "refraction"),
        n(&m["edgeWidth"], "edgeWidth"),
        n(&m["depth"]["offset"], "depth offset"),
        n(&m["depth"]["blur"], "depth blur"),
        lighting(&m["dark"], "dark"),
        lighting(&m["light"], "light"),
    )
    .unwrap();

    let themes = data["themes"].as_array().expect("themes");
    writeln!(out, "pub static THEMES: [Theme; {}] = [", themes.len()).unwrap();
    for t in themes {
        writeln!(
            out,
            "    Theme {{ id: {:?}, name: {:?}, mood: {:?}, is_default: {}, dark: {}, light: {} }},",
            text(t, "id"),
            text(t, "name"),
            text(t, "mood"),
            t["default"].as_bool().unwrap_or(false),
            variant(&t["variants"]["dark"], "dark"),
            variant(&t["variants"]["light"], "light"),
        )
        .unwrap();
    }
    writeln!(out, "];").unwrap();

    // The approved contrast rules: the pairs the preview proved, and the class and floor of each.
    let rules = data["rules"].as_array().expect("rules");
    writeln!(out, "pub static RULES: [Rule; {}] = [", rules.len()).unwrap();
    for r in rules {
        let fg = text(r, "fg");
        let fg = if fg == "trackReadable" {
            "RuleFg::TrackReadable".to_owned()
        } else {
            assert!(ROLES.contains(&fg), "palettes.json: rule foreground {fg}");
            format!("RuleFg::Role(PaletteRole::{})", camel(fg))
        };
        let bg = text(r, "bg");
        assert!(
            [
                "canvas",
                "sheet",
                "glass",
                "glassRaised",
                "scrim",
                "accent",
                "focus"
            ]
            .contains(&bg),
            "palettes.json: rule ground {bg}"
        );
        let class = match text(r, "class") {
            "glance" => "Glance",
            "body" => "Body",
            "hint" => "Hint",
            "control" => "Control",
            other => panic!("palettes.json: rule class {other}"),
        };
        writeln!(
            out,
            "    Rule {{ fg: {fg}, bg: PaletteRole::{}, min: {:?}, class: RuleClass::{class} }},",
            camel(bg),
            number(&r["min"], "rule min")
        )
        .unwrap();
    }
    writeln!(out, "];").unwrap();

    // The grid of wallpaper samples the preview judged contrast at: the same for every variant.
    let grid = themes[0]["variants"]["dark"]["wallpaper"]["grid"]
        .as_array()
        .expect("grid");
    let (cols, rows) = (
        number(&grid[0], "grid columns") as u32,
        number(&grid[1], "grid rows") as u32,
    );
    for t in themes {
        for mode in ["dark", "light"] {
            assert_eq!(
                t["variants"][mode]["wallpaper"]["grid"],
                themes[0]["variants"]["dark"]["wallpaper"]["grid"],
                "the wallpaper sample grid differs between variants"
            );
        }
    }
    writeln!(
        out,
        "pub const PREVIEW_GRID: (u32, u32) = ({cols}, {rows});"
    )
    .unwrap();
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("lucent_data.rs"),
        out,
    )
    .unwrap();
}
