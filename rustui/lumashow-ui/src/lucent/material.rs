//! The material contract of the smart-display shell (`docs/g2-shell-design.md`, section 3.4).
//!
//! Every button and card of the shell is a Lucent glass surface with the same shallow raised-or-inset depth, from its
//! first working version. This module freezes what the pieces of that agree on, so that the one who imports the approved
//! palette data, the one who writes the resolver and the one who builds the renderer fixtures work against the same
//! numbers and names:
//!
//! - [`MaterialData`] is the shape of the approved material numbers (the data's `materials` block, and the few Lucent
//!   tokens beside them), and [`MaterialData::field_tokens`] turns them into the exact wire values of the design's field
//!   table ([`FieldTokens`]: `Hrest`, `Hraised`, `S`, `Sr`, `Su`, `Sp`, `R`, `I`). Lighting is achromatic and one cast
//!   shadow only: the far 34/80 lobe of Lucent's raised shadow is not in the shell's recipe in either mode.
//! - [`ResolvedMaterials`] and [`ResolvedTheme`] are what theme resolution hands the resolver: the colours of the
//!   recipes (`G`, `GR`, `GRs`, `GR96`, `O96`, `A`, `E`, `ER`, `TR`, the sheet tint) already premultiplied, once, with
//!   contrast resolution applied.
//! - [`SmartMaterial`], [`MaterialContext`], [`MaterialTier`], [`ControlState`], [`PressTreatment`] and
//!   [`MaterialPair`] are the vocabulary of the one public pure resolver the design calls `smart_material`. Its per-
//!   component recipe table is section 3.4.2 of the design, and implementing it is the toolkit's.
//!
//! This module holds no palette: the approved data is imported once, by the appearance owner, and the numbers here
//! are read from it, never copied beside it.

use super::palette::{Mode, Roles, black, white};
use compositor_protocol::{Backdrop, Depth, Glass, Rgba, Shadow, Sheen};

/// A cast shadow as the approved data writes it: black, `offset_y` down, `blur` wide, at `alpha`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CastShadow {
    pub offset_y: f64,
    pub blur: f64,
    pub alpha: f64,
}

impl CastShadow {
    /// The one shadow of a surface, in the first slot of the wire's two.
    fn wire(self) -> [Option<Shadow>; 2] {
        [
            Some(Shadow {
                offset_y: self.offset_y as f32,
                blur: self.blur as f32,
                colour: black(self.alpha),
            }),
            None,
        ]
    }
}

/// The material numbers that differ between dark and light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lighting {
    /// Alpha of the white inner top lip of a surface at rest (`highlightRest`).
    pub highlight_rest: f64,
    /// … of a raised surface (`highlightRaised`).
    pub highlight_raised: f64,
    /// Under a sheet or the dock (`shadowRest`).
    pub shadow_rest: CastShadow,
    /// Under a raised card or chip: the approved single cast shadow (`shadowRaised`).
    pub shadow_raised: CastShadow,
    /// While a large control is pressed (Lucent's `shadow.pressed`).
    pub shadow_pressed: CastShadow,
    /// Alpha of the white, light-side copy of the depth pair (`depthLight`).
    pub depth_light: f64,
    /// Alpha of the black, shade-side copy (`depthShade`).
    pub depth_shade: f64,
}

/// The numbers of the material, as the approved data states them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialData {
    pub sheen: Sheen,
    /// The blur of every cached backdrop (`backdrop.blur`).
    pub blur: f64,
    /// Multiplies the backdrop's saturation; ignored by the renderer where there is no backdrop (`backdrop.saturation`).
    pub saturation: f64,
    /// The refraction a separately measured, permitted, stationary variant may use (`backdrop.refraction`). Kept in the
    /// data as it is; the baseline recipe uses none.
    pub refraction: f64,
    pub edge_width: f64,
    /// The offset and blur of the depth pair (`depth.offset`, `depth.blur`).
    pub depth_offset: f64,
    pub depth_blur: f64,
    pub dark: Lighting,
    pub light: Lighting,
}

/// Where along a surface's diagonal, from the upper-left corner as 0 to the lower right as 1, text and icons begin: the
/// sheen's contrast is judged there, not at its corner peak.
pub const CONTENT_STARTS_AT: f64 = 0.20;

/// The exact wire values of the shared field tokens, for one mode. Names follow the design's table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldTokens {
    /// `Hrest`: the inner top lip of a surface at rest.
    pub highlight_rest: Rgba,
    /// `Hraised`.
    pub highlight_raised: Rgba,
    /// `S`.
    pub sheen: Sheen,
    /// `Sr`.
    pub shadow_rest: [Option<Shadow>; 2],
    /// `Su`: one 10/24 cast shadow, never the far lobe.
    pub shadow_raised: [Option<Shadow>; 2],
    /// `Sp`: the interaction-only pressed shadow.
    pub shadow_pressed: [Option<Shadow>; 2],
    /// `R`: the raised light/shade pair.
    pub raised: Depth,
    /// `I`: exactly `R` with `inset: true`, the pressed or recessed pair.
    pub inset: Depth,
    /// `C30`: the cached backdrop of the baseline recipe.
    pub cached_backdrop: Backdrop,
    pub edge_width: f32,
    pub saturation: f32,
}

impl FieldTokens {
    /// `N` for shadows: none.
    pub const NO_SHADOWS: [Option<Shadow>; 2] = [None, None];
    /// `B0`: no backdrop of its own. A nested control still draws its sheen, lip and depth.
    pub const NO_BACKDROP: Backdrop = Backdrop::None;
}

impl MaterialData {
    pub fn lighting(&self, mode: Mode) -> &Lighting {
        match mode {
            Mode::Dark => &self.dark,
            Mode::Light => &self.light,
        }
    }

    /// The design's field table for `mode`, computed with the one rounding rule (`W`, `K`: `round(255 * alpha)`).
    pub fn field_tokens(&self, mode: Mode) -> FieldTokens {
        let lighting = self.lighting(mode);
        let raised = Depth {
            inset: false,
            offset: self.depth_offset as f32,
            blur: self.depth_blur as f32,
            light: white(lighting.depth_light),
            shade: black(lighting.depth_shade),
        };
        FieldTokens {
            highlight_rest: white(lighting.highlight_rest),
            highlight_raised: white(lighting.highlight_raised),
            sheen: self.sheen,
            shadow_rest: lighting.shadow_rest.wire(),
            shadow_raised: lighting.shadow_raised.wire(),
            shadow_pressed: lighting.shadow_pressed.wire(),
            raised,
            inset: Depth {
                inset: true,
                ..raised
            },
            cached_backdrop: Backdrop::Cached {
                radius: self.blur as f32,
            },
            edge_width: self.edge_width as f32,
            saturation: self.saturation as f32,
        }
    }

    /// The sheen's alpha where content begins ([`CONTENT_STARTS_AT`]): linear from the peak at the corner to the
    /// floor at `start`, to three decimals. About .102 for the shell's sheen; never the .16 corner peak.
    pub fn sheen_over_content(&self) -> f64 {
        let (peak, floor, start) = (
            f64::from(self.sheen.peak),
            f64::from(self.sheen.floor),
            f64::from(self.sheen.start),
        );
        let alpha = peak + (floor - peak) * CONTENT_STARTS_AT / start;
        (alpha * 1000.0).round() / 1000.0
    }
}

/// The colours of the shell's recipes after theme resolution (contrast included), premultiplied exactly once for the
/// compositor. Names follow the design's notation (section 3.4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedMaterials {
    /// `G`.
    pub glass: Rgba,
    /// `GR`.
    pub glass_raised: Rgba,
    /// `GRs`: `GR` with the selected wash composited before the one premultiplication.
    pub glass_raised_selected: Rgba,
    /// `GR96`: `GR` with its alpha raised to .96, for a thumb.
    pub glass_raised_96: Rgba,
    /// `GC`: the glass of a card and a timeline chip, deeper than `GR`, the raised glass of the controls that lie on a card. In the dark
    /// variants it is `GR`'s own premultiplied colour, read as a straight one, at `tokens::card::GLASS_ALPHA` (the approved mock-ups'
    /// `rgba(24,21,32,.62)` in Rosé Pine); in the light variants, which no mock-up draws, it is `GR`.
    pub glass_card: Rgba,
    /// `GCs`: `GC` with the selected wash composited before the one premultiplication.
    pub glass_card_selected: Rgba,
    /// The sheet tint, `P(sheet, .88)` or the contrast-resolved raise of it.
    pub sheet: Rgba,
    /// `O96`: the overlay tint, `P(sheet, .96)` or more.
    pub overlay: Rgba,
    /// `A`.
    pub accent: Rgba,
    /// `E`.
    pub edge: Rgba,
    /// `ER`.
    pub edge_raised: Rgba,
    /// `TR`: the resolved readable track, never raw `trackOff` alone.
    pub track: Rgba,
    /// `TO`: the palette's `trackOff` as it is, premultiplied: what a toggle's off track is filled with. Its boundary is a ring in `TR`
    /// (the one colour the appearance fit proves 3:1 on every picture), not a raised alpha of the fill.
    pub track_off: Rgba,
    /// `K`: a toggle's knob: the ink in a dark variant and white in a light one (the approved mock-ups' `--thumb`).
    pub knob: Rgba,
    /// The focus ring: the theme's `focus` where that is a 3:1 line against the sheet it is drawn on, else `focusInk`. The light
    /// variants' `focus` is near white, which is no cue on a light sheet.
    pub focus_cue: Rgba,
}

/// A resolved theme as the material resolver takes it: the mode, the straight roles after resolution, the premultiplied
/// colours of the recipes and the field tokens of that mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedTheme {
    pub mode: Mode,
    pub roles: Roles,
    pub materials: ResolvedMaterials,
    pub fields: FieldTokens,
}

/// Names one resolved theme. The appearance owner, who alone counts, gives each resolution of palette, mode, wallpaper palette or
/// tier a higher id; everything prepared for a theme (plates, text, icons, materials) carries the id it was prepared for, so a stale
/// plate is told from a current one by comparing two numbers, never two whole themes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ThemeGenerationId(pub u64);

impl ThemeGenerationId {
    /// The generation after this one.
    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// Names one prepared set of backdrop plates (design 3.2): it rises when a plate is refreshed, whatever the reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BackdropGeneration(pub u64);

/// Names the content a backdrop was prepared from: theme, wallpaper, face and the line's and cards' content, never touch transforms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentGeneration(pub u64);

/// A resolved theme with its generation and the backdrop generation prepared for it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeGeneration {
    pub id: ThemeGenerationId,
    pub theme: ResolvedTheme,
    pub backdrop: BackdropGeneration,
}

impl ThemeGeneration {
    /// Whether what was prepared for generation `prepared_for` is still for this one.
    pub fn is_current(&self, prepared_for: ThemeGenerationId) -> bool {
        self.id == prepared_for
    }
}

#[cfg(test)]
mod generation_tests {
    use super::*;

    #[test]
    fn a_plate_prepared_for_an_older_theme_is_told_stale_by_its_number_and_a_newer_id_orders_after_it()
     {
        let (three, four) = (ThemeGenerationId(3), ThemeGenerationId(3).next());
        assert_eq!(four, ThemeGenerationId(4));
        assert!(three < four);
        assert_eq!(
            ThemeGenerationId(u64::MAX).next(),
            ThemeGenerationId(u64::MAX),
            "never wraps to an old id"
        );
        let theme = ResolvedTheme {
            mode: Mode::Dark,
            roles: Roles::new([crate::lucent::palette::Straight::new(0, 0, 0, 255); 20]),
            materials: ResolvedMaterials {
                glass: Rgba(0, 0, 0, 0),
                glass_raised: Rgba(0, 0, 0, 0),
                glass_raised_selected: Rgba(0, 0, 0, 0),
                glass_raised_96: Rgba(0, 0, 0, 0),
                glass_card: Rgba(0, 0, 0, 0),
                glass_card_selected: Rgba(0, 0, 0, 0),
                sheet: Rgba(0, 0, 0, 0),
                overlay: Rgba(0, 0, 0, 0),
                accent: Rgba(0, 0, 0, 0),
                edge: Rgba(0, 0, 0, 0),
                edge_raised: Rgba(0, 0, 0, 0),
                track: Rgba(0, 0, 0, 0),
                track_off: Rgba(0, 0, 0, 0),
                knob: Rgba(0, 0, 0, 0),
                focus_cue: Rgba(0, 0, 0, 0),
            },
            fields: FieldTokens {
                highlight_rest: Rgba(0, 0, 0, 0),
                highlight_raised: Rgba(0, 0, 0, 0),
                sheen: Sheen {
                    peak: 0.0,
                    floor: 0.0,
                    start: 0.0,
                    end: 1.0,
                },
                shadow_rest: FieldTokens::NO_SHADOWS,
                shadow_raised: FieldTokens::NO_SHADOWS,
                shadow_pressed: FieldTokens::NO_SHADOWS,
                raised: Depth {
                    inset: false,
                    offset: 0.0,
                    blur: 0.0,
                    light: Rgba(0, 0, 0, 0),
                    shade: Rgba(0, 0, 0, 0),
                },
                inset: Depth {
                    inset: true,
                    offset: 0.0,
                    blur: 0.0,
                    light: Rgba(0, 0, 0, 0),
                    shade: Rgba(0, 0, 0, 0),
                },
                cached_backdrop: Backdrop::None,
                edge_width: 1.0,
                saturation: 1.0,
            },
        };
        let current = ThemeGeneration {
            id: four,
            theme,
            backdrop: BackdropGeneration(9),
        };
        assert!(current.is_current(four));
        assert!(
            !current.is_current(three),
            "a plate prepared for generation 3 is stale under 4"
        );
    }
}

/// The components that have a material (the rows of the design's component table). Selected, pressed and focused are
/// states ([`ControlState`]), not further roles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SmartMaterial {
    DockSheet,
    DockItem,
    /// Status icon buttons, and the Back and Close chips.
    StatusButton,
    /// A full or half live card.
    Card,
    /// A timeline event or summary chip.
    TimelineChip,
    /// The NOW marker and the return-to-Now control.
    NowMarker,
    ShadeTile,
    /// An actionable settings or notification row.
    ActionRow,
    /// A settings button, or a standalone action.
    Button,
    /// The one primary action of a task, on the accent (a small primary action only).
    PrimaryButton,
    ToggleTrack,
    ToggleThumb,
    SliderTrack,
    SliderFill,
    SliderThumb,
    TimerDialWell,
    /// The well of a text or password field.
    FieldWell,
    Key,
    /// The sheet that contains the shade, a settings page or the keyboard.
    ContainingSheet,
    /// A dialog, an owner view over the home or a toast.
    Overlay,
}

/// Whether a control sits on a surface that already has the glass backdrop (`Nested`: no second blur, but its lip,
/// sheen and depth still draw) or stands alone over the wallpaper. Chosen where the control is built, never guessed
/// from its layer order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MaterialContext {
    Independent,
    Nested,
}

/// Lucent's effect tiers: what a renderer is permitted to spend, not a rating of its GPU. T0 is a real, usable fallback
/// with no effects; T1 is the cached-glass baseline of the shell; T2 may add live blur over a settled region. During a
/// gesture every tier does without live blur and without captures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MaterialTier {
    T0,
    T1,
    T2,
}

/// What a control is in, for choosing its material. Whether it is being pressed is the component's to say, by choosing
/// the pressed state of the pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlState {
    pub pressed: bool,
    pub selected: bool,
    pub enabled: bool,
    /// A change has been asked for and is not confirmed: the last confirmed state stays shown.
    pub pending: bool,
    pub focus_visible: bool,
}

impl Default for ControlState {
    /// Idle, enabled and not selected.
    fn default() -> Self {
        Self {
            pressed: false,
            selected: false,
            enabled: true,
            pending: false,
            focus_visible: false,
        }
    }
}

/// How a control looks while it is pressed, besides the pressed material.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PressTreatment {
    /// Nothing: a sheet does not respond to a press.
    None,
    /// The raised pair gives way to the inset one, crossfaded over the press duration; no scale.
    Inset,
    /// The whole visual group scales about its centre, keeping its raised pair.
    Scale { factor: f32 },
}

/// A control's two prepared states and how it moves between them. Both are prepared when the theme or the model
/// changes, never per frame, so a press is a crossfade of what is already there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialPair {
    pub rest: Glass,
    pub pressed: Glass,
    pub press: PressTreatment,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The approved numbers as the design states them (section 3.4.1), written here only as a fixture: the product's
    /// copy is the imported data.
    fn approved() -> MaterialData {
        let shadow = |offset_y, blur, alpha| CastShadow {
            offset_y,
            blur,
            alpha,
        };
        MaterialData {
            sheen: Sheen {
                peak: 0.16,
                floor: 0.05,
                start: 0.38,
                end: 0.6,
            },
            blur: 30.0,
            saturation: 1.25,
            refraction: 6.0,
            edge_width: 1.0,
            depth_offset: 2.0,
            depth_blur: 4.0,
            dark: Lighting {
                highlight_rest: 0.07,
                highlight_raised: 0.22,
                shadow_rest: shadow(24.0, 60.0, 0.28),
                shadow_raised: shadow(10.0, 24.0, 0.30),
                shadow_pressed: shadow(6.0, 14.0, 0.30),
                depth_light: 0.10,
                depth_shade: 0.32,
            },
            light: Lighting {
                highlight_rest: 0.60,
                highlight_raised: 0.90,
                shadow_rest: shadow(24.0, 60.0, 0.18),
                shadow_raised: shadow(10.0, 24.0, 0.14),
                shadow_pressed: shadow(6.0, 14.0, 0.16),
                depth_light: 0.86,
                depth_shade: 0.10,
            },
        }
    }

    fn cast(offset_y: f32, blur: f32, alpha: u8) -> [Option<Shadow>; 2] {
        [
            Some(Shadow {
                offset_y,
                blur,
                colour: Rgba(0, 0, 0, alpha),
            }),
            None,
        ]
    }

    #[test]
    fn the_dark_field_tokens_are_the_designs_exact_wire_values() {
        let t = approved().field_tokens(Mode::Dark);
        assert_eq!(t.highlight_rest, Rgba(18, 18, 18, 18));
        assert_eq!(t.highlight_raised, Rgba(56, 56, 56, 56));
        assert_eq!(t.shadow_rest, cast(24.0, 60.0, 71));
        assert_eq!(t.shadow_raised, cast(10.0, 24.0, 77));
        assert_eq!(t.shadow_pressed, cast(6.0, 14.0, 77));
        assert_eq!(
            t.raised,
            Depth {
                inset: false,
                offset: 2.0,
                blur: 4.0,
                light: Rgba(26, 26, 26, 26),
                shade: Rgba(0, 0, 0, 82),
            }
        );
    }

    #[test]
    fn the_light_field_tokens_are_the_designs_exact_wire_values() {
        let t = approved().field_tokens(Mode::Light);
        assert_eq!(t.highlight_rest, Rgba(153, 153, 153, 153));
        assert_eq!(t.highlight_raised, Rgba(230, 230, 230, 230));
        assert_eq!(t.shadow_rest, cast(24.0, 60.0, 46));
        assert_eq!(t.shadow_raised, cast(10.0, 24.0, 36));
        assert_eq!(t.shadow_pressed, cast(6.0, 14.0, 41));
        assert_eq!(
            t.raised,
            Depth {
                inset: false,
                offset: 2.0,
                blur: 4.0,
                light: Rgba(219, 219, 219, 219),
                shade: Rgba(0, 0, 0, 26),
            }
        );
    }

    #[test]
    fn what_is_common_to_both_modes_is_common_and_the_inset_is_the_raised_pair_turned_over() {
        let data = approved();
        for mode in [Mode::Dark, Mode::Light] {
            let t = data.field_tokens(mode);
            assert_eq!(
                t.sheen,
                Sheen {
                    peak: 0.16,
                    floor: 0.05,
                    start: 0.38,
                    end: 0.6
                }
            );
            assert_eq!(t.cached_backdrop, Backdrop::Cached { radius: 30.0 });
            assert_eq!((t.edge_width, t.saturation), (1.0, 1.25));
            assert_eq!(
                t.inset,
                Depth {
                    inset: true,
                    ..t.raised
                }
            );
            assert!(!t.raised.inset && t.inset.inset);
            // One cast shadow at every level: the far lobe of the token set is not in the recipe.
            for shadows in [t.shadow_rest, t.shadow_raised, t.shadow_pressed] {
                assert!(shadows[0].is_some() && shadows[1].is_none(), "{mode:?}");
            }
        }
        assert_eq!(FieldTokens::NO_SHADOWS, [None, None]);
        assert_eq!(FieldTokens::NO_BACKDROP, Backdrop::None);
    }

    #[test]
    fn the_sheen_over_content_is_the_lower_figure_the_contrast_is_judged_at() {
        // .16 + (.05 - .16) * .20 / .38 = .1021: about 10%, not the .16 at the corner.
        assert_eq!(approved().sheen_over_content(), 0.102);
        let flat = MaterialData {
            sheen: Sheen {
                peak: 0.05,
                floor: 0.05,
                start: 0.38,
                end: 0.6,
            },
            ..approved()
        };
        assert_eq!(flat.sheen_over_content(), 0.05);
    }

    #[test]
    fn a_control_is_enabled_and_idle_until_something_says_otherwise() {
        let s = ControlState::default();
        assert!(s.enabled && !s.pressed && !s.selected && !s.pending && !s.focus_visible);
    }
}
