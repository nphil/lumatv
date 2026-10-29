//! Palette roles, and the one conversion from a straight colour to the compositor's premultiplied one.
//!
//! A palette is data: twenty named roles, each a straight (not premultiplied) RGBA colour, in a dark and a light
//! variant. Straight is the only form a palette is stored, shown or tested in. A colour becomes the compositor's
//! premultiplied [`Rgba`] **exactly once**, at the material boundary, with [`Straight::premultiply`]; a role that
//! has already been premultiplied is never multiplied again and a straight hex is never passed to the wire as it is.

use compositor_protocol::Rgba;
use std::fmt;
use std::ops::Index;

/// Which variant of a palette, and of the material numbers that go with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    Dark,
    Light,
}

/// Lucent's palette roles: the full set of the design language, not the shorter list of an earlier brief.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PaletteRole {
    Canvas,
    Sheet,
    Scrim,
    Glass,
    GlassRaised,
    Edge,
    EdgeRaised,
    Ink,
    Ink2,
    Ink3,
    Focus,
    FocusInk,
    TrackOff,
    Accent,
    AccentInk,
    Positive,
    Warning,
    Danger,
    Info,
    Live,
}

impl PaletteRole {
    /// Every role, in the order of the palette data (`roleOrder`).
    pub const ALL: [PaletteRole; 20] = [
        Self::Canvas,
        Self::Sheet,
        Self::Scrim,
        Self::Glass,
        Self::GlassRaised,
        Self::Edge,
        Self::EdgeRaised,
        Self::Ink,
        Self::Ink2,
        Self::Ink3,
        Self::Focus,
        Self::FocusInk,
        Self::TrackOff,
        Self::Accent,
        Self::AccentInk,
        Self::Positive,
        Self::Warning,
        Self::Danger,
        Self::Info,
        Self::Live,
    ];

    /// The role's name in the palette data and in Lucent's tokens: `glassRaised`, `focusInk`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Canvas => "canvas",
            Self::Sheet => "sheet",
            Self::Scrim => "scrim",
            Self::Glass => "glass",
            Self::GlassRaised => "glassRaised",
            Self::Edge => "edge",
            Self::EdgeRaised => "edgeRaised",
            Self::Ink => "ink",
            Self::Ink2 => "ink2",
            Self::Ink3 => "ink3",
            Self::Focus => "focus",
            Self::FocusInk => "focusInk",
            Self::TrackOff => "trackOff",
            Self::Accent => "accent",
            Self::AccentInk => "accentInk",
            Self::Positive => "positive",
            Self::Warning => "warning",
            Self::Danger => "danger",
            Self::Info => "info",
            Self::Live => "live",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.name() == name)
    }
}

/// Why a string is not a hex colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColourError {
    /// Not `#RRGGBB` or `#RRGGBBAA`.
    Shape,
}

impl fmt::Display for ColourError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a colour is #RRGGBB or #RRGGBBAA in hexadecimal")
    }
}

impl std::error::Error for ColourError {}

/// A colour as a palette writes it: red, green, blue and alpha, each a byte, **not** premultiplied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Straight {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Straight {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// `#RRGGBB` (opaque) or `#RRGGBBAA`, in either case.
    pub fn parse(text: &str) -> Result<Self, ColourError> {
        let digits = text.strip_prefix('#').ok_or(ColourError::Shape)?;
        if !(digits.len() == 6 || digits.len() == 8)
            || !digits.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(ColourError::Shape);
        }
        let byte =
            |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).map_err(|_| ColourError::Shape);
        Ok(Self {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
            a: if digits.len() == 8 { byte(6)? } else { 255 },
        })
    }

    /// The design's `P(role, floor)` (section 3.4.1): the alpha is raised to at least `floor` (0 to 1), rounded to a
    /// byte once (`a = round(255 * alpha)`), and each colour channel is scaled by that alpha and rounded down
    /// (`floor(c * a / 255)`). That is the rule the lab's tokens and the glyph blend already use. Alphas are `f64` so
    /// that a decimal coefficient rounds as it reads: .90 is 229.5 bytes and rounds up to 230, which `f32` would not.
    pub fn premultiply(self, floor: f64) -> Rgba {
        let alpha = (f64::from(self.a) / 255.0).max(floor.clamp(0.0, 1.0));
        let a = (alpha * 255.0).round() as u8;
        let scale = |c: u8| (u16::from(c) * u16::from(a) / 255) as u8;
        Rgba(scale(self.r), scale(self.g), scale(self.b), a)
    }
}

/// The design's `W(a)`: premultiplied white of alpha `a`, `Rgba(b, b, b, b)` with `b = round(255 * a)`. Lighting is
/// achromatic, so a highlight or a light-side edge carries no palette colour.
pub fn white(alpha: f64) -> Rgba {
    let b = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgba(b, b, b, b)
}

/// The design's `K(a)`: premultiplied black of alpha `a`, `Rgba(0, 0, 0, b)` with `b = round(255 * a)`.
pub fn black(alpha: f64) -> Rgba {
    Rgba(0, 0, 0, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// A palette variant: all twenty roles, straight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Roles([Straight; 20]);

impl Roles {
    /// The roles in [`PaletteRole::ALL`] order.
    pub const fn new(colours: [Straight; 20]) -> Self {
        Self(colours)
    }
}

impl Index<PaletteRole> for Roles {
    type Output = Straight;

    fn index(&self, role: PaletteRole) -> &Straight {
        &self.0[PaletteRole::ALL
            .iter()
            .position(|r| *r == role)
            .expect("ALL has every role")]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_roles_are_the_twenty_of_the_language_in_the_data_order_and_found_by_their_names() {
        let names: Vec<&str> = PaletteRole::ALL.iter().map(|r| r.name()).collect();
        assert_eq!(
            names,
            [
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
                "live"
            ]
        );
        for role in PaletteRole::ALL {
            assert_eq!(PaletteRole::from_name(role.name()), Some(role));
        }
        assert_eq!(PaletteRole::from_name("Glass"), None);
        assert_eq!(PaletteRole::from_name("glass_raised"), None);
    }

    #[test]
    fn a_hex_colour_is_six_or_eight_digits_and_nothing_else() {
        assert_eq!(
            Straight::parse("#191724"),
            Ok(Straight::new(0x19, 0x17, 0x24, 255))
        );
        assert_eq!(
            Straight::parse("#1F1D2EE8"),
            Ok(Straight::new(0x1F, 0x1D, 0x2E, 0xE8))
        );
        assert_eq!(
            Straight::parse("#f2eff6b1"),
            Ok(Straight::new(0xF2, 0xEF, 0xF6, 0xB1))
        );
        for bad in [
            "",
            "191724",
            "#19172",
            "#1917244",
            "#19172400FF",
            "#GGGGGG",
            "#19 724",
            "#+1+1+1",
            "##191724",
        ] {
            assert_eq!(Straight::parse(bad), Err(ColourError::Shape), "{bad:?}");
        }
    }

    #[test]
    fn premultiplying_raises_the_alpha_to_the_floor_then_scales_once_rounding_down() {
        // A sheet colour that is already above the floor is only scaled by its own alpha.
        let sheet = Straight::parse("#1F1D2EE8").unwrap();
        assert_eq!(sheet.premultiply(0.0), Rgba(28, 26, 41, 232));
        assert_eq!(sheet.premultiply(0.88), Rgba(28, 26, 41, 232));
        // A floor above its alpha raises the alpha first (0.96 is 245 of 255), then scales.
        assert_eq!(sheet.premultiply(0.96), Rgba(29, 27, 44, 245));
        // Opaque stays as it is; transparent stays nothing.
        assert_eq!(
            Straight::parse("#A99DF5").unwrap().premultiply(1.0),
            Rgba(0xA9, 0x9D, 0xF5, 255)
        );
        assert_eq!(
            Straight::new(255, 255, 255, 0).premultiply(0.0),
            Rgba(0, 0, 0, 0)
        );
        // The floors of the design are the bytes the lab's tokens already use.
        assert_eq!(Straight::new(0, 0, 0, 0).premultiply(0.88).3, 224);
        assert_eq!(Straight::new(0, 0, 0, 0).premultiply(0.96).3, 245);
    }

    #[test]
    fn no_channel_of_a_premultiplied_colour_exceeds_its_alpha() {
        for a in [0u8, 1, 17, 128, 200, 254, 255] {
            for floor in [0.0, 0.5, 0.88, 1.0] {
                let Rgba(r, g, b, alpha) = Straight::new(255, 200, 7, a).premultiply(floor);
                assert!(
                    r <= alpha && g <= alpha && b <= alpha,
                    "a={a} floor={floor}"
                );
                // The floor is met to the byte: round(255 * floor), as the design's rule rounds it.
                assert!(
                    alpha >= (floor * 255.0_f64).round() as u8,
                    "a={a} floor={floor}"
                );
            }
        }
    }

    #[test]
    fn white_and_black_are_the_designs_exact_wire_values() {
        // The exact numbers of the design's field table, both variants.
        for (alpha, wire) in [
            (0.07, 18),
            (0.22, 56),
            (0.60, 153),
            (0.90, 230),
            (0.10, 26),
            (0.86, 219),
        ] {
            assert_eq!(white(alpha), Rgba(wire, wire, wire, wire), "W({alpha})");
        }
        for (alpha, wire) in [
            (0.28, 71),
            (0.18, 46),
            (0.30, 77),
            (0.14, 36),
            (0.16, 41),
            (0.32, 82),
            (0.10, 26),
        ] {
            assert_eq!(black(alpha), Rgba(0, 0, 0, wire), "K({alpha})");
        }
        assert_eq!(white(2.0), Rgba(255, 255, 255, 255));
        assert_eq!(black(-1.0), Rgba(0, 0, 0, 0));
    }

    #[test]
    fn roles_are_looked_up_by_role() {
        let mut colours = [Straight::new(0, 0, 0, 255); 20];
        colours[13] = Straight::new(0xA9, 0x9D, 0xF5, 255);
        let roles = Roles::new(colours);
        assert_eq!(
            roles[PaletteRole::Accent],
            Straight::new(0xA9, 0x9D, 0xF5, 255)
        );
        assert_eq!(roles[PaletteRole::Canvas], Straight::new(0, 0, 0, 255));
    }
}
