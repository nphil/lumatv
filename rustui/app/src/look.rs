//! LumaHome's look: the values of `payload/theme/theme.json` (Rosé Pine silk, dark material) and of the QML shaders in
//! `Glass.qml` and `Tile.qml`, written as the toolkit's glass, so that what this client draws is what the QML Home draws.
//!
//! Colours are premultiplied, as the compositor takes them.

use compositor_protocol::{Backdrop, Glass, Rgba, Shadow, Sheen};
use ui::lucent::palette::Straight;
use ui::text::{Type, Weight};

/// A Qt colour string: `#RRGGBB`, or `#AARRGGBB` (QML's order, as `theme.json` writes them), as the compositor's premultiplied colour.
pub fn hex(text: &str) -> Rgba {
    let digits = text.strip_prefix('#').unwrap_or_else(|| panic!("{text} is not a colour"));
    let byte = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).unwrap_or_else(|e| panic!("{text}: {e}"));
    let (a, r, g, b) = match digits.len() {
        6 => (255, byte(0), byte(2), byte(4)),
        8 => (byte(0), byte(2), byte(4), byte(6)),
        _ => panic!("{text} is not a colour"),
    };
    Straight::new(r, g, b, a).premultiply(0.0)
}

/// White at `alpha`, premultiplied.
pub fn white(alpha: f32) -> Rgba {
    let b = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgba(b, b, b, b)
}

/// Black at `alpha`, premultiplied.
pub fn black(alpha: f32) -> Rgba {
    Rgba(0, 0, 0, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

pub struct Palette {
    pub ink: Rgba,
    pub ink2: Rgba,
    pub ink3: Rgba,
    pub glass: Rgba,
    pub glass_edge: Rgba,
    pub glass_focus: Rgba,
    pub glass_focus_edge: Rgba,
    pub accent: Rgba,
    pub second: Rgba,
    pub ink_on_accent: Rgba,
}

pub fn palette() -> Palette {
    Palette {
        ink: hex("#F2EFF6"),
        ink2: hex("#A8F2EFF6"),
        ink3: hex("#6BF2EFF6"),
        glass: hex("#66120E1A"),
        glass_edge: hex("#21FFFFFF"),
        glass_focus: hex("#8F2C263A"),
        glass_focus_edge: hex("#33FFFFFF"),
        accent: hex("#A79BF5"),
        second: hex("#ff3b5c"),
        ink_on_accent: hex("#17131F"),
    }
}

/// Manrope at a pixel size, as the QML draws it (`font.weight` Font.Medium / Font.DemiBold; the toolkit has no Bold).
pub const fn ty(px: u32, weight: Weight) -> Type {
    Type { px, weight }
}

pub const MED: Weight = Weight::Medium;
pub const SEMI: Weight = Weight::SemiBold;

// Radii (theme.json "radii"), focus tokens and motion.
pub const R_CARD: f32 = 26.0;
pub const R_TILE: f32 = 22.0;
pub const TILE_SCALE: f32 = 1.12;
pub const CARD_SCALE: f32 = 1.03;
pub const CARD_LIFT: f32 = 4.0;
pub const BUTTON_SCALE: f32 = 1.04;
pub const FOCUS_MS: u32 = 150;
pub const CARD_MS: u32 = 180;
pub const REST_MS: u64 = 150;
pub const BLUR: f32 = 30.0;
/// `theme.saturation` 0.25 is HueSaturation's addition: the backdrop's colour is multiplied by 1.25.
pub const SATURATION: f32 = 1.25;
pub const AMBIENT: f32 = 0.3;

fn shadow(offset_y: f32, blur: f32, alpha: f32) -> Option<Shadow> {
    Some(Shadow { offset_y, blur, colour: black(alpha) })
}

/// The material of a card, the dock and a status button (`Glass.qml`): the glass over the cached blur of the wallpaper.
/// `focus` is the highlight (the focus tint, the lighter edge, the rim and sheen), which follows every key at once;
/// `lifted` is the deeper shadow, which comes once focus has rested.
pub fn card(p: &Palette, radius: f32, focus: bool, lifted: bool) -> Glass {
    Glass {
        corner_radius: radius,
        tint: if focus { p.glass_focus } else { p.glass },
        edge: if focus { p.glass_focus_edge } else { p.glass_edge },
        edge_width: 1.0,
        // The rim of Glass.qml: a white line along the top, 0.42 of the ink at focus.
        highlight: if focus { white(0.42) } else { Rgba(0, 0, 0, 0) },
        // Glass.qml: 0.16 falling to 0.05 over the first 38 % of the diagonal, gone by 60 %.
        sheen: focus.then_some(Sheen { peak: 0.16, floor: 0.0, start: 0.0, end: 0.6 }),
        // CSS: normal 0 24px 60px .28; focus 0 34px 80px .55 + 0 10px 24px .30.
        shadows: if lifted { [shadow(34.0, 80.0, 0.55), shadow(10.0, 24.0, 0.30)] } else { [shadow(24.0, 60.0, 0.28), None] },
        depth: None,
        backdrop: Backdrop::Cached { radius: BLUR },
        saturation: SATURATION,
        refraction: 0.0,
    }
}

/// A dock tile's plate (`Tile.qml`): opaque, the shadow of its depth, a faint edge band; at focus the white sheen, the rim and the
/// brighter band.
pub fn tile(plate: Rgba, focus: bool, lifted: bool) -> Glass {
    Glass {
        corner_radius: R_TILE,
        tint: plate,
        edge: white(if focus { 0.22 } else { 0.06 }),
        edge_width: 1.0,
        highlight: if focus { white(0.42) } else { Rgba(0, 0, 0, 0) },
        sheen: focus.then_some(Sheen { peak: 0.18, floor: 0.0, start: 0.0, end: 0.55 }),
        // sA, sY, sB of the shader run from (0.28, 6, 9) to (0.6, 26, 34); a blur here is the width of the smoothstep.
        shadows: if lifted { [shadow(26.0, 68.0, 0.6), None] } else { [shadow(6.0, 18.0, 0.28), None] },
        depth: None,
        backdrop: Backdrop::None,
        saturation: 1.0,
        refraction: 0.0,
    }
}

/// The soft accent halo under the focused tile (`LuHalo.qml`): glass with no body to speak of and an accent shadow all round.
pub fn halo(accent: Rgba) -> Glass {
    // LuHalo.qml: the accent at half alpha in the middle, fading to nothing at 70 % of the way out, then at `ambient` of that. A shadow's
    // strength at the body's edge is half its colour's, so the colour is the accent whole.
    let glow = Rgba(accent.0, accent.1, accent.2, 255);
    Glass {
        corner_radius: R_TILE,
        tint: Rgba(0, 0, 0, 0),
        edge: Rgba(0, 0, 0, 0),
        edge_width: 0.0,
        highlight: Rgba(0, 0, 0, 0),
        sheen: None,
        shadows: [Some(Shadow { offset_y: 0.0, blur: 72.0, colour: glow }), None],
        depth: None,
        backdrop: Backdrop::None,
        saturation: 1.0,
        refraction: 0.0,
    }
}

/// The name pill under the focused tile.
pub fn pill() -> Glass {
    Glass {
        corner_radius: 10.0,
        tint: hex("#db120e1a"),
        edge: hex("#21ffffff"),
        edge_width: 1.0,
        highlight: Rgba(0, 0, 0, 0),
        sheen: None,
        shadows: [shadow(6.0, 12.0, 0.35), None],
        depth: None,
        backdrop: Backdrop::None,
        saturation: 1.0,
        refraction: 0.0,
    }
}
