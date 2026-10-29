//! A palette variant becomes a [`ResolvedTheme`]: the colours of the shell's recipes (design section 3.4.1) premultiplied once,
//! and the field tokens of the mode.
//!
//! Resolution here applies the recipes' own rules: the alpha floors of a sheet and an overlay, the selected wash composited in
//! straight colour before the one premultiplication, and the readable track (`TR`) held to 3:1 against its substrate. The
//! appearance owner's fuller contrast resolution (wallpaper extrema, state washes, `accentInk`) raises these floors further and
//! never lowers one; it is S10's and plugs in by building a [`ResolvedTheme`] of its own.

use super::data::{MATERIALS, Theme, Variant};
use super::material::{MaterialData, ResolvedMaterials, ResolvedTheme};
use super::palette::{Mode, PaletteRole, Roles, Straight};
use super::tokens::{card, contrast, floor, state};
use compositor_protocol::Rgba;

/// The default theme resolved in `mode`: what a screen that has no appearance service yet is drawn with.
pub fn default_theme(mode: Mode) -> ResolvedTheme {
    resolve(super::data::default_theme(), mode)
}

/// `theme` in `mode`, with the approved material numbers.
pub fn resolve(theme: &Theme, mode: Mode) -> ResolvedTheme {
    resolve_variant(theme.variant(mode), mode, &MATERIALS)
}

/// A variant of a palette and the material numbers of its mode as a theme the material resolver takes.
pub fn resolve_variant(variant: &Variant, mode: Mode, data: &MaterialData) -> ResolvedTheme {
    let roles = variant.roles;
    let glass_raised = roles[PaletteRole::GlassRaised];
    let wash = Straight {
        a: (state::SELECTED_WASH * 255.0).round() as u8,
        ..roles[PaletteRole::Accent]
    };
    let selected = over(wash, glass_raised);
    let glass_card = card_glass(glass_raised, mode);
    let materials = ResolvedMaterials {
        glass: roles[PaletteRole::Glass].premultiply(0.0),
        glass_raised: glass_raised.premultiply(0.0),
        glass_raised_selected: selected.premultiply(0.0),
        glass_raised_96: glass_raised.premultiply(floor::THUMB),
        glass_card: glass_card.premultiply(0.0),
        glass_card_selected: over(wash, glass_card).premultiply(0.0),
        sheet: roles[PaletteRole::Sheet].premultiply(floor::SHEET),
        overlay: roles[PaletteRole::Sheet].premultiply(floor::OVERLAY),
        accent: roles[PaletteRole::Accent].premultiply(1.0),
        edge: roles[PaletteRole::Edge].premultiply(0.0),
        edge_raised: roles[PaletteRole::EdgeRaised].premultiply(0.0),
        track: readable_track(&roles).premultiply(0.0),
        track_off: roles[PaletteRole::TrackOff].premultiply(0.0),
        knob: knob(&roles, mode).premultiply(1.0),
        focus_cue: focus_cue(&roles).premultiply(0.0),
    };
    ResolvedTheme {
        mode,
        roles,
        materials,
        fields: data.field_tokens(mode),
    }
}

/// A toggle's knob (`K`): the ink in a dark variant and white in a light one, which is what the approved mock-ups' `--thumb` is in each
/// (`#F2EFF6`, `#FFFFFF`). A light variant's ink is its dark colour, so the knob there is not the ink.
fn knob(roles: &Roles, mode: Mode) -> Straight {
    match mode {
        Mode::Dark => opaque(roles[PaletteRole::Ink]),
        Mode::Light => Straight::new(255, 255, 255, 255),
    }
}

/// The glass of a card and a timeline chip (`GC`) from the raised glass `GR` of the same variant, straight. A dark variant's card is
/// deeper than the raised glass of the controls that lie on it: `GR`'s own premultiplied colour read as a straight one, at
/// [`card::GLASS_ALPHA`]. That is what the approved mock-ups draw: `#2C263A8F` premultiplied is 24, 21, 32, and a card is
/// `rgba(24,21,32,.62)` over the wallpaper. A light variant's card is its raised glass, as before (no mock-up draws one, and a
/// colour made of the premultiplied white of its glass would be a grey).
fn card_glass(raised: Straight, mode: Mode) -> Straight {
    match mode {
        Mode::Dark => {
            let colour = raised.premultiply(0.0);
            Straight {
                r: colour.0,
                g: colour.1,
                b: colour.2,
                a: (card::GLASS_ALPHA * 255.0).round() as u8,
            }
        }
        Mode::Light => raised,
    }
}

/// The track of a toggle or a slider: .60 of the ink over its substrate, the sheet over the canvas, raised in alpha until
/// its boundary against that substrate reaches 3:1. Raw `trackOff` alone does not promise a visible control.
fn readable_track(roles: &Roles) -> Straight {
    let substrate = over(
        roles[PaletteRole::Sheet],
        opaque(roles[PaletteRole::Canvas]),
    );
    let ink = roles[PaletteRole::Ink];
    let mut alpha = (0.60_f64 * 255.0).round() as u8;
    loop {
        let track = Straight { a: alpha, ..ink };
        if contrast_ratio(over(track, substrate), substrate) >= contrast::CONTROL || alpha == 255 {
            return track;
        }
        alpha = alpha.saturating_add(5);
    }
}

/// The focus ring's colour: the theme's `focus` when, over the sheet that is over the canvas, it reaches 3:1 against that sheet, and
/// `focusInk` (the dark or light ink the theme names for it) when it does not.
fn focus_cue(roles: &Roles) -> Straight {
    let sheet = over(
        roles[PaletteRole::Sheet],
        opaque(roles[PaletteRole::Canvas]),
    );
    let focus = roles[PaletteRole::Focus];
    if contrast_ratio(over(focus, sheet), sheet) >= contrast::CONTROL {
        focus
    } else {
        roles[PaletteRole::FocusInk]
    }
}

/// `colour` with its alpha made 1, for a ground that has been composited already.
pub fn opaque(colour: Straight) -> Straight {
    Straight { a: 255, ..colour }
}

/// `top` over `bottom`, both straight, composited in straight colour (Porter-Duff over); the result is straight too.
pub fn over(top: Straight, bottom: Straight) -> Straight {
    let (ta, ba) = (f64::from(top.a) / 255.0, f64::from(bottom.a) / 255.0);
    let alpha = ta + ba * (1.0 - ta);
    if alpha <= 0.0 {
        return Straight::new(0, 0, 0, 0);
    }
    let mix = |t: u8, b: u8| {
        ((f64::from(t) * ta + f64::from(b) * ba * (1.0 - ta)) / alpha)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Straight::new(
        mix(top.r, bottom.r),
        mix(top.g, bottom.g),
        mix(top.b, bottom.b),
        (alpha * 255.0).round() as u8,
    )
}

/// WCAG relative luminance of an sRGB colour (alpha ignored).
pub fn luminance(colour: Straight) -> f64 {
    let lin = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(colour.r) + 0.7152 * lin(colour.g) + 0.0722 * lin(colour.b)
}

/// WCAG contrast ratio of two colours, 1 to 21. Alpha is ignored: composite first.
pub fn contrast_ratio(a: Straight, b: Straight) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

impl ResolvedTheme {
    /// A palette role as the compositor takes a colour: premultiplied, once.
    pub fn colour(&self, role: PaletteRole) -> Rgba {
        self.roles[role].premultiply(0.0)
    }

    /// A palette role with its alpha scaled by `factor` (0 to 1): a disabled icon is the ink at .55.
    pub fn colour_scaled(&self, role: PaletteRole, factor: f32) -> Rgba {
        let c = self.roles[role];
        Straight {
            a: (f32::from(c.a) * factor.clamp(0.0, 1.0)).round() as u8,
            ..c
        }
        .premultiply(0.0)
    }

    /// The straight colour of the focus wash: the ink at .07 on dark and .06 on light.
    pub fn focus_wash(&self) -> Straight {
        let alpha = match self.mode {
            Mode::Dark => state::FOCUS_WASH_DARK,
            Mode::Light => state::FOCUS_WASH_LIGHT,
        };
        Straight {
            a: (alpha * 255.0).round() as u8,
            ..self.roles[PaletteRole::Ink]
        }
    }

    /// The opaque colour of the canvas, which T0 surfaces are composited over.
    pub fn canvas(&self) -> Straight {
        opaque(self.roles[PaletteRole::Canvas])
    }
}

#[cfg(test)]
mod tests {
    use super::super::data::themes;
    use super::*;

    #[test]
    fn straight_over_is_porter_duff_and_leaves_an_opaque_top_alone() {
        let red = Straight::new(255, 0, 0, 255);
        let blue = Straight::new(0, 0, 255, 255);
        assert_eq!(over(red, blue), red);
        // Half red over opaque blue.
        let half = Straight::new(255, 0, 0, 128);
        let mixed = over(half, blue);
        assert_eq!((mixed.r, mixed.b, mixed.a), (128, 127, 255));
        // Over nothing it is itself; nothing over nothing is nothing.
        assert_eq!(over(half, Straight::new(9, 9, 9, 0)), half);
        assert_eq!(
            over(Straight::new(1, 2, 3, 0), Straight::new(9, 9, 9, 0)).a,
            0
        );
    }

    #[test]
    fn contrast_is_wcag_black_on_white_is_21_and_a_colour_with_itself_is_1() {
        let (black, white) = (
            Straight::new(0, 0, 0, 255),
            Straight::new(255, 255, 255, 255),
        );
        assert!((contrast_ratio(black, white) - 21.0).abs() < 1e-9);
        assert!((contrast_ratio(white, white) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_focus_cue_is_a_three_to_one_line_on_the_sheet_in_every_variant_of_every_theme() {
        let (mut ink, mut focus) = (0, 0);
        for theme in themes() {
            for mode in [Mode::Dark, Mode::Light] {
                let r = resolve(theme, mode);
                let sheet = over(r.roles[PaletteRole::Sheet], r.canvas());
                let cue = r.materials.focus_cue;
                let straight = Straight::new(
                    unmultiply(cue.0, cue.3),
                    unmultiply(cue.1, cue.3),
                    unmultiply(cue.2, cue.3),
                    cue.3,
                );
                assert!(
                    contrast_ratio(over(straight, sheet), sheet) >= 3.0 - 0.15,
                    "{} {mode:?}: the focus ring is not a cue on the sheet",
                    theme.id
                );
                if r.materials.focus_cue == r.colour(PaletteRole::FocusInk) {
                    ink += 1;
                } else {
                    focus += 1;
                }
            }
        }
        // The dark variants keep their own `focus`; the light ones, whose `focus` is near white, use the ink.
        assert_eq!((focus, ink), (themes().len(), themes().len()));
    }

    #[test]
    fn every_variant_of_every_theme_resolves_with_the_floors_and_a_track_that_is_readable() {
        for theme in themes() {
            for mode in [Mode::Dark, Mode::Light] {
                let r = resolve(theme, mode);
                let m = r.materials;
                let name = format!("{} {mode:?}", theme.id);
                assert!(m.sheet.3 >= 224, "{name}: sheet alpha {}", m.sheet.3);
                assert!(m.overlay.3 >= 245, "{name}: overlay alpha {}", m.overlay.3);
                assert!(m.glass_raised_96.3 >= 245, "{name}: thumb alpha");
                assert_eq!(m.accent.3, 255, "{name}: accent is solid");
                // Premultiplied colours never exceed their alpha.
                for c in [
                    m.glass,
                    m.glass_raised,
                    m.glass_raised_selected,
                    m.glass_raised_96,
                    m.sheet,
                    m.overlay,
                    m.accent,
                    m.edge,
                    m.edge_raised,
                    m.track,
                ] {
                    assert!(c.0 <= c.3 && c.1 <= c.3 && c.2 <= c.3, "{name}: {c:?}");
                }
                // The track, composited over its substrate, is 3:1 against it.
                let substrate = over(r.roles[PaletteRole::Sheet], r.canvas());
                let track = Straight::new(
                    unmultiply(m.track.0, m.track.3),
                    unmultiply(m.track.1, m.track.3),
                    unmultiply(m.track.2, m.track.3),
                    m.track.3,
                );
                assert!(
                    contrast_ratio(over(track, substrate), substrate) >= 3.0 - 0.15,
                    "{name}: the track is too faint against its sheet"
                );
                // A toggle's knob is solid, and its off track is the palette's own `trackOff` (its ring is the track above).
                assert_eq!(m.knob.3, 255, "{name}: the toggle's knob is solid");
                assert_eq!(
                    m.track_off,
                    r.roles[PaletteRole::TrackOff].premultiply(0.0),
                    "{name}: an off toggle's track is the palette's own trackOff"
                );
            }
        }
    }

    /// Premultiplication rounds down, so the straight channel is recovered to within a step; enough for a ratio.
    fn unmultiply(c: u8, a: u8) -> u8 {
        if a == 0 {
            0
        } else {
            (u32::from(c) * 255).div_ceil(u32::from(a)).min(255) as u8
        }
    }

    #[test]
    fn the_selected_wash_tints_the_raised_glass_towards_the_accent_without_a_second_premultiplication()
     {
        let r = default_theme(Mode::Dark);
        let (plain, selected) = (r.materials.glass_raised, r.materials.glass_raised_selected);
        assert_ne!(plain, selected);
        // The wash is .04 of the accent: a few units, never a new colour.
        assert!(plain.3.abs_diff(selected.3) <= 12, "{plain:?} {selected:?}");
        assert!(selected.3 >= plain.3);
        // The approved Rosé Pine dark raised glass is #2C263A at 8F.
        assert_eq!(
            plain,
            Straight::new(0x2C, 0x26, 0x3A, 0x8F).premultiply(0.0)
        );
    }

    /// The mock-ups' card is `rgba(24,21,32,.62)` over what is behind it (`live-cards/app.css` `.lc`). Rosé Pine's raised glass, `#2C263A8F`,
    /// premultiplied is 24, 21, 32: the card glass is that colour, straight, at .62.
    #[test]
    fn the_card_glass_of_a_dark_variant_is_the_mock_ups_card_and_deeper_than_its_raised_glass_in_every_theme()
     {
        let card = default_theme(Mode::Dark).materials.glass_card;
        assert_eq!(card, Straight::new(24, 21, 32, 158).premultiply(0.0));
        // What the browser makes of `rgba(24,21,32,.62)` over a ground and what the wire's premultiplied card makes of it agree to a
        // level, over the dark wallpaper of the mock-ups and over black, white and a colour.
        for behind in [
            [0.0, 0.0, 0.0],
            [28.0, 25.0, 70.0],
            [255.0, 255.0, 255.0],
            [200.0, 120.0, 40.0],
        ] {
            for (i, mock) in [24.0, 21.0, 32.0].into_iter().enumerate() {
                let mock = 0.62 * mock + 0.38 * behind[i];
                let wire = f64::from([card.0, card.1, card.2][i])
                    + (1.0 - f64::from(card.3) / 255.0) * behind[i];
                assert!(
                    (mock - wire).abs() <= 1.0,
                    "channel {i} over {behind:?}: the mock {mock}, the wire {wire}"
                );
            }
        }
        // Every dark variant: the card gives less light than the raised glass over black and over white, on every channel; the
        // selected wash lies over the card, and a light variant keeps its raised glass.
        for theme in themes() {
            let dark = resolve(theme, Mode::Dark).materials;
            for ground in [0.0, 255.0] {
                let seen = |c: Rgba| {
                    [c.0, c.1, c.2].map(|v| f64::from(v) + (1.0 - f64::from(c.3) / 255.0) * ground)
                };
                let (card, raised) = (seen(dark.glass_card), seen(dark.glass_raised));
                for i in 0..3 {
                    assert!(
                        card[i] < raised[i],
                        "{} dark over {ground}: card {card:?}, raised {raised:?}",
                        theme.id
                    );
                }
            }
            assert_ne!(dark.glass_card_selected, dark.glass_card, "{}", theme.id);
            let light = resolve(theme, Mode::Light).materials;
            assert_eq!(light.glass_card, light.glass_raised, "{} light", theme.id);
            assert_eq!(
                light.glass_card_selected, light.glass_raised_selected,
                "{} light",
                theme.id
            );
            for c in [dark.glass_card, dark.glass_card_selected] {
                assert!(c.0 <= c.3 && c.1 <= c.3 && c.2 <= c.3, "{c:?}");
            }
        }
    }

    #[test]
    fn the_focus_wash_is_the_ink_at_seven_percent_on_dark_and_six_on_light() {
        let dark = default_theme(Mode::Dark).focus_wash();
        let light = default_theme(Mode::Light).focus_wash();
        assert_eq!((dark.a, light.a), (18, 15));
    }

    #[test]
    fn a_disabled_icon_is_the_ink_at_fifty_five_percent_of_its_alpha() {
        let r = default_theme(Mode::Dark);
        let ink = r.colour(PaletteRole::Ink);
        let dim = r.colour_scaled(PaletteRole::Ink, 0.55);
        assert_eq!(ink.3, 255);
        assert_eq!(dim.3, 140);
    }
}
