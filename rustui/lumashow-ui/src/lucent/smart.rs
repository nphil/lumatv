//! The one public material resolver of the shell: [`smart_material`] turns a component role and its state into the two prepared
//! [`Glass`] values of the design's component-to-glass table (section 3.4.2), the one the control rests in and the one it is
//! pressed in, plus how it moves between them. Components never build a [`Glass`] of their own.
//!
//! Every row is complete: the corner radius, tint, edge, highlight, sheen, shadows, depth and backdrop, in all eleven fields
//! (saturation 1.25 and an edge one unit wide throughout; refraction 0, because the baseline and everything in motion use none).
//! The result is resolved when a model, a theme or a tier changes, never per frame; a press is a crossfade between the two.
//!
//! The tiers keep the meaning and drop only the lighting: T0 is a verified opaque tint with no backdrop, sheen, shadow, depth or
//! lip, T1 is the table, and T2 is the table with a live backdrop where the table has a cached one (the caller decides which
//! surface may have it, and builds at T1 while the surface or what is behind it moves).

use super::material::{
    ControlState, FieldTokens, MaterialContext, MaterialPair, MaterialTier, PressTreatment,
    ResolvedTheme, SmartMaterial,
};
use super::palette::{Mode, Straight};
use super::tokens::{radius, state, switch};
use compositor_protocol::{Backdrop, Depth, Glass, Rgba, Shadow};

/// How a recipe's tint is chosen.
#[derive(Clone, Copy)]
enum Tint {
    Glass,
    GlassRaised,
    /// The deeper glass of a card and a timeline chip (`GC`).
    Card,
    Thumb,
    Sheet,
    Overlay,
    Accent,
    Track,
    /// A toggle's off track: the palette's `trackOff` as it is (`TO`); its boundary is the ring, not a raised alpha.
    SwitchOff,
    /// A toggle's knob (`K`): light in both modes.
    Knob,
}

#[derive(Clone, Copy, PartialEq)]
enum Edge {
    Plain,
    Raised,
    /// The ring of an off toggle's track (`RG`, two wide); an on track has none. Nothing is drawn but the width stays the table's.
    Ring,
    /// No edge: transparent, the table's width kept.
    None,
}

#[derive(Clone, Copy, PartialEq)]
enum Lip {
    Rest,
    Raised,
    /// No lip: transparent (a toggle's track: the mock draws none, and on the accent a lip would be a light line inside its edge).
    None,
}

#[derive(Clone, Copy, PartialEq)]
enum Cast {
    None,
    Rest,
    Raised,
    /// A toggle's knob's own small shadow (the mock's `1px 2px 4px rgba(0,0,0,.35)`), the same pressed.
    Knob,
}

#[derive(Clone, Copy, PartialEq)]
enum Pair {
    None,
    Raised,
    Inset,
}

#[derive(Clone, Copy)]
enum Press {
    None,
    Inset,
    Scale,
}

/// One row of the table, at rest and not selected.
#[derive(Clone, Copy)]
struct Recipe {
    radius: f32,
    tint: Tint,
    edge: Edge,
    lip: Lip,
    sheen: bool,
    cast: Cast,
    pair: Pair,
    cached: bool,
    press: Press,
}

#[allow(clippy::too_many_arguments)]
const fn recipe(
    radius: f32,
    tint: Tint,
    edge: Edge,
    lip: Lip,
    sheen: bool,
    cast: Cast,
    pair: Pair,
    cached: bool,
    press: Press,
) -> Recipe {
    Recipe {
        radius,
        tint,
        edge,
        lip,
        sheen,
        cast,
        pair,
        cached,
        press,
    }
}

/// The row of the table for `role`, in the context the control was built for.
fn row(role: SmartMaterial, context: MaterialContext) -> Recipe {
    use Cast as C;
    use Edge as E;
    use Lip as L;
    use Pair as P;
    use Press as Pr;
    use Tint as T;
    let nested = context == MaterialContext::Nested;
    match role {
        SmartMaterial::DockSheet => recipe(
            radius::PILL,
            T::Sheet,
            E::Plain,
            L::Rest,
            false,
            C::Rest,
            P::None,
            true,
            Pr::None,
        ),
        SmartMaterial::DockItem => recipe(
            radius::PILL,
            T::GlassRaised,
            E::Plain,
            L::Rest,
            true,
            C::None,
            P::Raised,
            false,
            Pr::Inset,
        ),
        SmartMaterial::StatusButton => recipe(
            radius::PILL,
            T::GlassRaised,
            E::Raised,
            L::Raised,
            true,
            C::None,
            P::Raised,
            false,
            Pr::Inset,
        ),
        SmartMaterial::Card => recipe(
            radius::CARD,
            T::Card,
            E::Raised,
            L::Raised,
            true,
            C::Raised,
            P::Raised,
            true,
            Pr::Scale,
        ),
        SmartMaterial::TimelineChip => recipe(
            radius::ROW,
            T::Card,
            E::Raised,
            L::Raised,
            true,
            C::Raised,
            P::Raised,
            true,
            Pr::Scale,
        ),
        SmartMaterial::NowMarker => recipe(
            radius::PILL,
            T::Accent,
            E::Raised,
            L::Raised,
            true,
            C::None,
            P::Raised,
            false,
            Pr::Scale,
        ),
        SmartMaterial::ShadeTile => recipe(
            radius::TILE,
            T::GlassRaised,
            E::Raised,
            L::Raised,
            true,
            C::None,
            P::Raised,
            false,
            Pr::Inset,
        ),
        SmartMaterial::ActionRow => recipe(
            radius::ROW,
            T::GlassRaised,
            E::Plain,
            L::Rest,
            true,
            C::None,
            P::Raised,
            false,
            Pr::Scale,
        ),
        SmartMaterial::Button => recipe(
            radius::CONTROL,
            T::GlassRaised,
            E::Raised,
            L::Raised,
            true,
            if nested { C::None } else { C::Raised },
            P::Raised,
            !nested,
            Pr::Inset,
        ),
        SmartMaterial::PrimaryButton => recipe(
            radius::CONTROL,
            T::Accent,
            E::Raised,
            L::Raised,
            true,
            if nested { C::None } else { C::Raised },
            P::Raised,
            !nested,
            Pr::Inset,
        ),
        // The toggle is the approved mock-ups' `.sw` (decision 36): an off track is the palette's own `trackOff` inside a ring, an on track
        // the accent; the knob is light, flat but for its lip and its small shadow, and shrinks a little under a finger.
        SmartMaterial::ToggleTrack => recipe(
            radius::PILL,
            T::SwitchOff,
            E::Ring,
            L::None,
            false,
            C::None,
            P::Inset,
            false,
            Pr::None,
        ),
        SmartMaterial::ToggleThumb => recipe(
            radius::PILL,
            T::Knob,
            E::None,
            L::Raised,
            false,
            C::Knob,
            P::None,
            false,
            Pr::Scale,
        ),
        SmartMaterial::SliderThumb => recipe(
            radius::PILL,
            T::Thumb,
            E::Raised,
            L::Raised,
            true,
            C::None,
            P::Raised,
            false,
            Pr::Scale,
        ),
        SmartMaterial::SliderTrack => recipe(
            radius::PILL,
            T::Track,
            E::Plain,
            L::Rest,
            false,
            C::None,
            P::Inset,
            false,
            Pr::None,
        ),
        SmartMaterial::SliderFill => recipe(
            radius::PILL,
            T::Accent,
            E::Plain,
            L::Rest,
            false,
            C::None,
            P::Inset,
            false,
            Pr::None,
        ),
        SmartMaterial::TimerDialWell => recipe(
            radius::PILL,
            T::Glass,
            E::Plain,
            L::Rest,
            false,
            C::None,
            P::Inset,
            false,
            Pr::None,
        ),
        SmartMaterial::FieldWell => recipe(
            radius::CONTROL,
            T::Sheet,
            E::Plain,
            L::Rest,
            false,
            C::None,
            P::Inset,
            false,
            Pr::None,
        ),
        SmartMaterial::Key => recipe(
            radius::CONTROL,
            T::GlassRaised,
            E::Raised,
            L::Raised,
            true,
            C::None,
            P::Raised,
            false,
            Pr::Inset,
        ),
        SmartMaterial::ContainingSheet => recipe(
            radius::SHEET,
            T::Sheet,
            E::Plain,
            L::Rest,
            false,
            C::Rest,
            P::None,
            true,
            Pr::None,
        ),
        SmartMaterial::Overlay => recipe(
            radius::SHEET,
            T::Overlay,
            E::Plain,
            L::Rest,
            true,
            C::Rest,
            P::Raised,
            false,
            Pr::None,
        ),
    }
}

/// The straight-colour wash of `wash` over a premultiplied tint, as a premultiplied colour (over is linear, so doing it on
/// premultiplied channels is the same composite).
fn washed(tint: Rgba, wash: Straight) -> Rgba {
    let wa = f64::from(wash.a) / 255.0;
    let channel = |t: u8, w: u8| {
        (f64::from(t) * (1.0 - wa) + f64::from(w) * wa)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    let alpha = (f64::from(tint.3) * (1.0 - wa) + 255.0 * wa)
        .round()
        .clamp(0.0, 255.0) as u8;
    Rgba(
        channel(tint.0, wash.r).min(alpha),
        channel(tint.1, wash.g).min(alpha),
        channel(tint.2, wash.b).min(alpha),
        alpha,
    )
}

/// A premultiplied colour over an opaque ground, as an opaque premultiplied colour: what T0 draws instead of a translucent tint.
fn opaque_over(tint: Rgba, ground: Straight) -> Rgba {
    let keep = 255 - u32::from(tint.3);
    let mix = |t: u8, g: u8| (u32::from(t) + (u32::from(g) * keep + 127) / 255).min(255) as u8;
    Rgba(
        mix(tint.0, ground.r),
        mix(tint.1, ground.g),
        mix(tint.2, ground.b),
        255,
    )
}

fn tint_of(tint: Tint, state: &ControlState, theme: &ResolvedTheme) -> Rgba {
    let m = &theme.materials;
    match tint {
        Tint::Glass => m.glass,
        Tint::GlassRaised if state.selected => m.glass_raised_selected,
        Tint::GlassRaised => m.glass_raised,
        Tint::Card if state.selected => m.glass_card_selected,
        Tint::Card => m.glass_card,
        Tint::Thumb => m.glass_raised_96,
        Tint::Sheet => m.sheet,
        Tint::Overlay => m.overlay,
        Tint::Accent => m.accent,
        // A toggle's track is the readable track when off and the accent when on; a slider's track is always the track.
        Tint::Track => m.track,
        Tint::SwitchOff => m.track_off,
        Tint::Knob => m.knob,
    }
}

fn shadows_of(cast: Cast, f: &FieldTokens) -> [Option<Shadow>; 2] {
    match cast {
        Cast::None => FieldTokens::NO_SHADOWS,
        Cast::Rest => f.shadow_rest,
        Cast::Raised => f.shadow_raised,
        Cast::Knob => [
            Some(Shadow {
                offset_y: switch::SHADOW_OFFSET,
                blur: switch::SHADOW_BLUR,
                colour: Rgba(0, 0, 0, (switch::SHADOW_ALPHA * 255.0).round() as u8),
            }),
            None,
        ],
    }
}

fn depth_of(pair: Pair, f: &FieldTokens) -> Option<Depth> {
    match pair {
        Pair::None => None,
        Pair::Raised => Some(f.raised),
        Pair::Inset => Some(f.inset),
    }
}

/// The glass of `recipe` at rest in `state`, at tier T1 or T2.
fn rest_glass(
    role: SmartMaterial,
    recipe: Recipe,
    state: &ControlState,
    theme: &ResolvedTheme,
    tier: MaterialTier,
) -> Glass {
    let f = &theme.fields;
    let m = &theme.materials;
    let selected = state.selected;
    // Selection raises a dock item and a key from the rest edge and lip to the raised ones; a toggle's track changes its tint.
    let lifted = selected
        && matches!(
            role,
            SmartMaterial::DockItem
                | SmartMaterial::Key
                | SmartMaterial::StatusButton
                | SmartMaterial::ShadeTile
        );
    let mut tint = match (role, selected) {
        (SmartMaterial::ToggleTrack, true) => m.accent,
        _ => tint_of(recipe.tint, state, theme),
    };
    if state.focus_visible && !matches!(recipe.press, Press::None) {
        tint = washed(tint, theme.focus_wash());
    }
    let edge_raised = recipe.edge == Edge::Raised || lifted;
    let (edge, edge_width) = match recipe.edge {
        Edge::Ring if !selected => (m.track, switch::RING_WIDTH),
        Edge::Ring | Edge::None => (Rgba(0, 0, 0, 0), f.edge_width),
        Edge::Plain | Edge::Raised => (
            if edge_raised { m.edge_raised } else { m.edge },
            f.edge_width,
        ),
    };
    let lip_raised = recipe.lip == Lip::Raised || lifted;
    Glass {
        corner_radius: recipe.radius,
        tint,
        edge,
        edge_width,
        highlight: match recipe.lip {
            Lip::None => Rgba(0, 0, 0, 0),
            _ if lip_raised => f.highlight_raised,
            _ => f.highlight_rest,
        },
        sheen: recipe.sheen.then_some(f.sheen),
        shadows: shadows_of(recipe.cast, f),
        depth: depth_of(recipe.pair, f),
        backdrop: if recipe.cached {
            match tier {
                MaterialTier::T2 => Backdrop::Live {
                    radius: radius_of_blur(f),
                    downsample: 8,
                },
                _ => f.cached_backdrop,
            }
        } else {
            FieldTokens::NO_BACKDROP
        },
        saturation: f.saturation,
        refraction: 0.0,
    }
}

fn radius_of_blur(f: &FieldTokens) -> f32 {
    match f.cached_backdrop {
        Backdrop::Cached { radius } => radius,
        _ => 30.0,
    }
}

/// The glass of a control pressed, from its rest glass (which carries the selection and the focus) and its treatment.
fn pressed_glass(rest: &Glass, recipe: Recipe, theme: &ResolvedTheme) -> Glass {
    let f = &theme.fields;
    match recipe.press {
        Press::None => *rest,
        // Raised gives way to inset: the same shape, tint and backdrop; the lip, sheen and shadows go, and the edge is the
        // raised one (the dock's pressed row says so, and every inset press agrees).
        Press::Inset => Glass {
            highlight: f.highlight_rest,
            sheen: None,
            shadows: FieldTokens::NO_SHADOWS,
            depth: Some(f.inset),
            edge: theme.materials.edge_raised,
            ..*rest
        },
        // The full raised pair and the sheen stay; a surface that casts a shadow casts the prepared pressed one.
        Press::Scale => Glass {
            shadows: match recipe.cast {
                // A knob shrinks under a finger and keeps its own small shadow.
                Cast::Knob => rest.shadows,
                _ if rest.shadows[0].is_some() => f.shadow_pressed,
                _ => FieldTokens::NO_SHADOWS,
            },
            ..*rest
        },
    }
}

/// What T0 draws: an opaque tint, nothing of the lighting, the one-unit edge, and the same shape.
fn minimal(glass: Glass, theme: &ResolvedTheme, ground: Straight) -> Glass {
    Glass {
        tint: opaque_over(glass.tint, ground),
        highlight: Rgba(0, 0, 0, 0),
        sheen: None,
        shadows: FieldTokens::NO_SHADOWS,
        depth: None,
        backdrop: Backdrop::None,
        refraction: 0.0,
        saturation: theme.fields.saturation,
        ..glass
    }
}

/// The material of `role` in `state`: its rest glass, its pressed glass and how it moves between them.
///
/// `state.pressed` is not read: the component prepares both states and chooses between them (`pressed` of the result). A
/// disabled control ignores press (its pressed glass is its rest glass and its treatment none) and keeps its substrate.
pub fn smart_material(
    role: SmartMaterial,
    state: &ControlState,
    context: MaterialContext,
    theme: &ResolvedTheme,
    tier: MaterialTier,
) -> MaterialPair {
    let recipe = row(role, context);
    let rest_full = rest_glass(role, recipe, state, theme, tier);
    let pressed_full = pressed_glass(&rest_full, recipe, theme);
    let treatment = match recipe.press {
        Press::None => PressTreatment::None,
        Press::Inset => PressTreatment::Inset,
        Press::Scale => PressTreatment::Scale {
            factor: state::PRESSED_SCALE,
        },
    };
    let (mut rest, mut pressed, mut press) = (rest_full, pressed_full, treatment);
    if tier == MaterialTier::T0 {
        // Nested controls sit on a sheet, and so do tracks and wells; the rest sit on the canvas.
        let sheet_ground = super::resolve::over(
            theme.roles[super::palette::PaletteRole::Sheet],
            theme.canvas(),
        );
        let ground = if context == MaterialContext::Nested
            || matches!(
                role,
                SmartMaterial::ToggleTrack
                    | SmartMaterial::SliderTrack
                    | SmartMaterial::SliderFill
                    | SmartMaterial::FieldWell
                    | SmartMaterial::TimerDialWell
                    | SmartMaterial::Key
            ) {
            super::resolve::opaque(sheet_ground)
        } else {
            theme.canvas()
        };
        rest = minimal(rest, theme, ground);
        // Without depth a press has to show in the tint: the standard wash, as the focus wash is.
        let wash = match (recipe.press, role) {
            (Press::None, _) => rest.tint,
            // The standard wash is the ink, which a knob in the ink (the dark themes') cannot show: a press darkens it.
            (_, SmartMaterial::ToggleThumb) => washed(
                rest.tint,
                Straight {
                    r: 0,
                    g: 0,
                    b: 0,
                    ..theme.focus_wash()
                },
            ),
            _ => washed(rest.tint, theme.focus_wash()),
        };
        pressed = Glass {
            tint: wash,
            ..minimal(pressed, theme, ground)
        };
    }
    if !state.enabled {
        pressed = rest;
        press = PressTreatment::None;
    }
    MaterialPair {
        rest,
        pressed,
        press,
    }
}

impl MaterialPair {
    /// The pair with each state's corner radius limited to half of `width` and `height`, as a pill is as round as its height
    /// allows. A component calls this once, where its size is known.
    pub fn sized(self, width: f32, height: f32) -> Self {
        let limit = width.min(height) / 2.0;
        Self {
            rest: Glass {
                corner_radius: self.rest.corner_radius.min(limit),
                ..self.rest
            },
            pressed: Glass {
                corner_radius: self.pressed.corner_radius.min(limit),
                ..self.pressed
            },
            press: self.press,
        }
    }

    /// Whether the two states are the same glass (a sheet, a track, a disabled control): nothing to crossfade to.
    pub fn is_still(&self) -> bool {
        self.rest == self.pressed
    }
}

/// Whether `theme`'s mode is dark: the one question a few components ask of it.
pub fn is_dark(theme: &ResolvedTheme) -> bool {
    theme.mode == Mode::Dark
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lucent::resolve::default_theme;
    use SmartMaterial as M;

    const ALL: [SmartMaterial; 20] = [
        M::DockSheet,
        M::DockItem,
        M::StatusButton,
        M::Card,
        M::TimelineChip,
        M::NowMarker,
        M::ShadeTile,
        M::ActionRow,
        M::Button,
        M::PrimaryButton,
        M::ToggleTrack,
        M::ToggleThumb,
        M::SliderTrack,
        M::SliderFill,
        M::SliderThumb,
        M::TimerDialWell,
        M::FieldWell,
        M::Key,
        M::ContainingSheet,
        M::Overlay,
    ];

    fn pair(
        role: SmartMaterial,
        state: ControlState,
        context: MaterialContext,
        mode: Mode,
        tier: MaterialTier,
    ) -> MaterialPair {
        smart_material(role, &state, context, &default_theme(mode), tier)
    }

    fn t1(role: SmartMaterial, state: ControlState, mode: Mode) -> MaterialPair {
        pair(role, state, MaterialContext::Nested, mode, MaterialTier::T1)
    }

    fn selected() -> ControlState {
        ControlState {
            selected: true,
            ..ControlState::default()
        }
    }

    #[test]
    fn every_button_and_card_carries_its_raised_or_inset_pair_at_the_full_tier_in_both_modes() {
        let actionable = [
            M::DockItem,
            M::StatusButton,
            M::Card,
            M::TimelineChip,
            M::NowMarker,
            M::ShadeTile,
            M::ActionRow,
            M::Button,
            M::PrimaryButton,
            M::SliderThumb,
            M::Key,
        ];
        for mode in [Mode::Dark, Mode::Light] {
            for role in actionable {
                for context in [MaterialContext::Nested, MaterialContext::Independent] {
                    let p = pair(
                        role,
                        ControlState::default(),
                        context,
                        mode,
                        MaterialTier::T1,
                    );
                    let d = p
                        .rest
                        .depth
                        .unwrap_or_else(|| panic!("{role:?} has no depth"));
                    assert!(!d.inset, "{role:?} rests raised");
                    assert!(p.rest.sheen.is_some(), "{role:?} has its sheen");
                    assert_eq!(
                        (p.rest.saturation, p.rest.edge_width, p.rest.refraction),
                        (1.25, 1.0, 0.0)
                    );
                }
            }
            for role in [
                M::ToggleTrack,
                M::SliderTrack,
                M::SliderFill,
                M::TimerDialWell,
                M::FieldWell,
            ] {
                let d = t1(role, ControlState::default(), mode)
                    .rest
                    .depth
                    .expect("a well has depth");
                assert!(d.inset, "{role:?} is a recess");
            }
        }
    }

    /// Nitin's decision 17 (10-07 22:36): the cards of the home match the approved mock-ups, which draw a card and a timeline chip in a
    /// deeper glass (`mock-kit/kit.css` `.m-card`, `live-cards/app.css` `.lc`: `rgba(24,21,32,.62)`) than the raised glass of a control
    /// (`.m-raised`: the shade's tiles, buttons, keys, the dock's selected item), which the shade's pictures were matched to and which
    /// stays as it is. The layers and the cost of a card are those of any glass: only its tint is another.
    #[test]
    fn a_card_and_a_timeline_chip_are_the_mock_ups_card_glass_and_every_other_raised_row_is_the_raised_glass()
     {
        for mode in [Mode::Dark, Mode::Light] {
            let m = default_theme(mode).materials;
            for role in [M::Card, M::TimelineChip] {
                for context in [MaterialContext::Nested, MaterialContext::Independent] {
                    let rest = pair(
                        role,
                        ControlState::default(),
                        context,
                        mode,
                        MaterialTier::T1,
                    );
                    assert_eq!(rest.rest.tint, m.glass_card, "{role:?} {mode:?}");
                    assert_eq!(rest.pressed.tint, m.glass_card, "{role:?} {mode:?} pressed");
                    let lit = pair(role, selected(), context, mode, MaterialTier::T1);
                    assert_eq!(
                        lit.rest.tint, m.glass_card_selected,
                        "{role:?} {mode:?} selected"
                    );
                }
            }
            for role in [
                M::DockItem,
                M::StatusButton,
                M::ShadeTile,
                M::ActionRow,
                M::Button,
                M::Key,
            ] {
                assert_eq!(
                    t1(role, ControlState::default(), mode).rest.tint,
                    m.glass_raised,
                    "{role:?} {mode:?}: raised glass, as the mock's m-raised"
                );
            }
        }
        // The dark card is deeper than the raised glass (what the mock shows, and what Nitin asked for); the light one is the raised glass.
        let dark = default_theme(Mode::Dark).materials;
        assert!(
            u32::from(dark.glass_card.0)
                + u32::from(dark.glass_card.1)
                + u32::from(dark.glass_card.2)
                < u32::from(dark.glass_raised.0)
                    + u32::from(dark.glass_raised.1)
                    + u32::from(dark.glass_raised.2),
            "{:?} against {:?}",
            dark.glass_card,
            dark.glass_raised
        );
        let light = default_theme(Mode::Light).materials;
        assert_eq!(light.glass_card, light.glass_raised);
    }

    #[test]
    fn the_dock_item_rows_are_the_designs_dark_values_exactly() {
        let theme = default_theme(Mode::Dark);
        let f = theme.fields;
        let rest = t1(M::DockItem, ControlState::default(), Mode::Dark);
        let g = rest.rest;
        assert_eq!(g.corner_radius, 999.0);
        assert_eq!(g.tint, theme.materials.glass_raised);
        assert_eq!(g.edge, theme.materials.edge);
        assert_eq!(g.highlight, Rgba(18, 18, 18, 18));
        assert_eq!(g.sheen, Some(f.sheen));
        assert_eq!(g.shadows, [None, None]);
        assert_eq!(g.depth, Some(f.raised));
        assert_eq!(g.backdrop, Backdrop::None);
        // Active: the selected wash, the raised edge and lip; the selection is not the pressed state.
        let active = t1(M::DockItem, selected(), Mode::Dark).rest;
        assert_eq!(active.tint, theme.materials.glass_raised_selected);
        assert_eq!(
            (active.edge, active.highlight),
            (theme.materials.edge_raised, Rgba(56, 56, 56, 56))
        );
        // Pressed, active or not: the raised edge, the resting lip, no sheen or shadow, the inset pair, selection kept.
        for (state, tint) in [
            (ControlState::default(), theme.materials.glass_raised),
            (selected(), theme.materials.glass_raised_selected),
        ] {
            let p = t1(M::DockItem, state, Mode::Dark);
            assert_eq!(p.press, PressTreatment::Inset);
            assert_eq!(p.pressed.tint, tint);
            assert_eq!(p.pressed.edge, theme.materials.edge_raised);
            assert_eq!(p.pressed.highlight, Rgba(18, 18, 18, 18));
            assert_eq!((p.pressed.sheen, p.pressed.shadows), (None, [None, None]));
            assert_eq!(p.pressed.depth, Some(f.inset));
            assert_eq!(p.pressed.corner_radius, 999.0);
        }
    }

    #[test]
    fn the_inset_is_the_raised_pair_turned_over_in_light_too_and_the_light_lip_is_the_light_coefficient()
     {
        let theme = default_theme(Mode::Light);
        let p = t1(M::Key, ControlState::default(), Mode::Light);
        assert_eq!(p.rest.highlight, Rgba(230, 230, 230, 230));
        let (raised, inset) = (p.rest.depth.unwrap(), p.pressed.depth.unwrap());
        assert_eq!(
            (raised.light, raised.shade),
            (Rgba(219, 219, 219, 219), Rgba(0, 0, 0, 26))
        );
        assert_eq!(
            inset,
            Depth {
                inset: true,
                ..raised
            }
        );
        assert_eq!(p.pressed.highlight, theme.fields.highlight_rest);
        assert_eq!(p.pressed.highlight, Rgba(153, 153, 153, 153));
    }

    #[test]
    fn a_card_scales_when_pressed_keeps_its_pair_and_sheen_and_swaps_its_shadow_for_the_prepared_pressed_one()
     {
        for mode in [Mode::Dark, Mode::Light] {
            let f = default_theme(mode).fields;
            let p = pair(
                M::Card,
                ControlState::default(),
                MaterialContext::Independent,
                mode,
                MaterialTier::T1,
            );
            assert_eq!(p.press, PressTreatment::Scale { factor: 0.97 });
            assert_eq!(p.rest.shadows, f.shadow_raised);
            assert_eq!(p.rest.backdrop, Backdrop::Cached { radius: 30.0 });
            assert_eq!(p.pressed.shadows, f.shadow_pressed);
            assert_eq!(
                (p.pressed.depth, p.pressed.sheen),
                (p.rest.depth, p.rest.sheen)
            );
            // An actionable row owns no shadow, so it keeps none while pressed.
            let row = t1(M::ActionRow, ControlState::default(), mode);
            assert_eq!(row.press, PressTreatment::Scale { factor: 0.97 });
            assert_eq!(
                (row.rest.shadows, row.pressed.shadows),
                ([None, None], [None, None])
            );
            assert_eq!(row.rest.corner_radius, 18.0);
        }
    }

    #[test]
    fn there_is_one_cast_shadow_everywhere_and_never_the_far_lobe() {
        for mode in [Mode::Dark, Mode::Light] {
            for role in ALL {
                for context in [MaterialContext::Nested, MaterialContext::Independent] {
                    let p = pair(role, selected(), context, mode, MaterialTier::T1);
                    for g in [p.rest, p.pressed] {
                        assert!(g.shadows[1].is_none(), "{role:?}");
                        if let Some(s) = g.shadows[0] {
                            // The toggle's knob casts the mock's own small shadow (decision 36); everything else one of the three.
                            let allowed: &[(f32, f32)] = if role == M::ToggleThumb {
                                &[(2.0, 4.0)]
                            } else {
                                &[(24.0, 60.0), (10.0, 24.0), (6.0, 14.0)]
                            };
                            assert!(
                                allowed.contains(&(s.offset_y, s.blur)),
                                "{role:?} casts {s:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_nested_button_has_no_second_blur_and_a_standalone_one_has_the_cached_backdrop_and_the_raised_shadow()
     {
        let theme = default_theme(Mode::Dark);
        let nested = pair(
            M::Button,
            ControlState::default(),
            MaterialContext::Nested,
            Mode::Dark,
            MaterialTier::T1,
        );
        assert_eq!(
            (nested.rest.backdrop, nested.rest.shadows),
            (Backdrop::None, [None, None])
        );
        assert!(
            nested.rest.depth.is_some_and(|d| !d.inset),
            "no second blur is not flat"
        );
        let alone = pair(
            M::Button,
            ControlState::default(),
            MaterialContext::Independent,
            Mode::Dark,
            MaterialTier::T1,
        );
        assert_eq!(alone.rest.backdrop, Backdrop::Cached { radius: 30.0 });
        assert_eq!(alone.rest.shadows, theme.fields.shadow_raised);
        assert_eq!(alone.press, PressTreatment::Inset);
        assert_eq!(alone.pressed.shadows, [None, None]);
        let primary = pair(
            M::PrimaryButton,
            ControlState::default(),
            MaterialContext::Nested,
            Mode::Dark,
            MaterialTier::T1,
        );
        assert_eq!(primary.rest.tint, theme.materials.accent);
    }

    #[test]
    fn the_sheets_have_their_shadow_and_no_sheen_at_rest_and_no_response_to_a_press() {
        for (role, radius) in [(M::DockSheet, 999.0), (M::ContainingSheet, 28.0)] {
            let p = t1(role, ControlState::default(), Mode::Dark);
            assert_eq!(p.rest.corner_radius, radius);
            assert_eq!(p.rest.sheen, None);
            assert_eq!(p.rest.depth, None);
            assert_eq!(p.rest.backdrop, Backdrop::Cached { radius: 30.0 });
            assert_eq!(p.rest.shadows, default_theme(Mode::Dark).fields.shadow_rest);
            assert_eq!(p.press, PressTreatment::None);
            assert!(p.is_still());
        }
        // A dialog is an overlay: the denser tint, its own sheen and raised pair, no blur of its own.
        let overlay = t1(M::Overlay, ControlState::default(), Mode::Dark);
        assert_eq!(
            overlay.rest.tint,
            default_theme(Mode::Dark).materials.overlay
        );
        assert_eq!(
            (overlay.rest.backdrop, overlay.rest.sheen.is_some()),
            (Backdrop::None, true)
        );
    }

    /// Nitin's decision 36 (10-08): the toggle is the approved mock-ups' `.sw` everywhere (`ux-prototype/settings-*`, `mock-kit/kit.css`):
    /// an off track is the palette's own `trackOff` inside a ring two wide in the ink at .55, an on track the accent with no ring, and the
    /// knob is light (the ink in a dark variant, white in a light one) with the mock's small shadow, not the raised glass it was.
    #[test]
    fn a_toggle_is_the_mocks_sw_an_off_track_in_its_ring_an_on_track_in_the_accent_and_a_light_knob()
     {
        for mode in [Mode::Dark, Mode::Light] {
            let theme = default_theme(mode);
            let m = theme.materials;
            let off = t1(M::ToggleTrack, ControlState::default(), mode);
            let on = t1(M::ToggleTrack, selected(), mode);
            assert_eq!(off.rest.tint, m.track_off, "{mode:?}");
            assert_eq!(
                (off.rest.edge, off.rest.edge_width),
                (m.track, 2.0),
                "{mode:?}: the ring"
            );
            assert_eq!(on.rest.tint, m.accent, "{mode:?}");
            assert_eq!(on.rest.edge.3, 0, "{mode:?}: an on track has no ring");
            assert!(off.is_still() && on.is_still(), "{mode:?}");
            assert_eq!(
                (off.rest.highlight, on.rest.highlight),
                (Rgba(0, 0, 0, 0), Rgba(0, 0, 0, 0)),
                "{mode:?}: the mock draws no lip on a track"
            );
            assert_eq!(
                (off.rest.depth, on.rest.depth),
                (Some(theme.fields.inset), Some(theme.fields.inset)),
                "{mode:?}"
            );
            // A slider keeps the readable track and the raised thumb.
            let track = t1(M::SliderTrack, ControlState::default(), mode);
            assert_eq!(track.rest.tint, m.track, "{mode:?}");
            let slider = t1(M::SliderThumb, ControlState::default(), mode);
            assert_eq!(slider.rest.tint, m.glass_raised_96, "{mode:?}");
            let knob = t1(M::ToggleThumb, ControlState::default(), mode);
            assert_eq!(knob.rest.tint, m.knob, "{mode:?}");
            assert_eq!(knob.rest.tint.3, 255, "{mode:?}: solid");
            assert_eq!(knob.press, PressTreatment::Scale { factor: 0.97 });
            assert_eq!(knob.rest.edge.3, 0, "{mode:?}: no edge");
            assert_eq!(
                (knob.rest.sheen, knob.rest.depth, knob.rest.backdrop),
                (None, None, Backdrop::None),
                "{mode:?}"
            );
            let shadow = Shadow {
                offset_y: 2.0,
                blur: 4.0,
                colour: Rgba(0, 0, 0, 89),
            };
            assert_eq!(knob.rest.shadows, [Some(shadow), None], "{mode:?}");
            assert_eq!(
                knob.pressed.shadows, knob.rest.shadows,
                "{mode:?}: it shrinks and keeps its shadow"
            );
            assert_eq!(knob.rest.highlight, theme.fields.highlight_raised);
        }
        // The mocks' `--thumb`: #F2EFF6 in the dark theme (its ink) and #FFFFFF in the light one.
        assert_eq!(
            default_theme(Mode::Dark).materials.knob,
            Rgba(0xF2, 0xEF, 0xF6, 255)
        );
        assert_eq!(
            default_theme(Mode::Light).materials.knob,
            Rgba(255, 255, 255, 255)
        );
    }

    #[test]
    fn a_disabled_control_keeps_its_substrate_and_ignores_press() {
        let dim = ControlState {
            enabled: false,
            ..ControlState::default()
        };
        for role in [M::Button, M::Key, M::Card, M::DockItem] {
            let enabled = t1(role, ControlState::default(), Mode::Dark);
            let p = t1(role, dim, Mode::Dark);
            assert_eq!(p.rest, enabled.rest, "{role:?} keeps its substrate");
            assert!(p.is_still(), "{role:?}");
            assert_eq!(p.press, PressTreatment::None);
        }
    }

    #[test]
    fn focus_is_a_wash_apart_from_selection_and_press_and_does_not_touch_the_lighting() {
        let focus = ControlState {
            focus_visible: true,
            ..ControlState::default()
        };
        let plain = t1(M::Key, ControlState::default(), Mode::Dark);
        let focused = t1(M::Key, focus, Mode::Dark);
        assert_ne!(plain.rest.tint, focused.rest.tint);
        assert_eq!(
            (plain.rest.depth, plain.rest.sheen),
            (focused.rest.depth, focused.rest.sheen)
        );
        assert_eq!(
            focused.pressed.tint, focused.rest.tint,
            "the pressed glass keeps the focus"
        );
    }

    #[test]
    fn the_minimal_tier_is_opaque_and_flat_and_keeps_the_shape_the_press_and_the_selection() {
        for mode in [Mode::Dark, Mode::Light] {
            for role in ALL {
                let full = pair(
                    role,
                    selected(),
                    MaterialContext::Nested,
                    mode,
                    MaterialTier::T1,
                );
                let t0 = pair(
                    role,
                    selected(),
                    MaterialContext::Nested,
                    mode,
                    MaterialTier::T0,
                );
                for g in [t0.rest, t0.pressed] {
                    assert_eq!(g.tint.3, 255, "{role:?}: a verified opaque tint");
                    assert_eq!(
                        (g.backdrop, g.sheen, g.shadows, g.depth),
                        (Backdrop::None, None, [None, None], None),
                        "{role:?}"
                    );
                    assert_eq!(
                        (g.highlight, g.refraction, g.saturation, g.edge_width),
                        (Rgba(0, 0, 0, 0), 0.0, 1.25, 1.0)
                    );
                }
                assert_eq!(t0.rest.corner_radius, full.rest.corner_radius, "{role:?}");
                assert_eq!(t0.press, full.press, "{role:?}");
                if full.press != PressTreatment::None {
                    assert_ne!(
                        t0.pressed.tint, t0.rest.tint,
                        "{role:?}: a press still shows without depth"
                    );
                }
            }
        }
        // Selected and unselected still differ at T0: no meaning lives in the lighting alone.
        let (on, off) = (
            pair(
                M::DockItem,
                selected(),
                MaterialContext::Nested,
                Mode::Dark,
                MaterialTier::T0,
            ),
            pair(
                M::DockItem,
                ControlState::default(),
                MaterialContext::Nested,
                Mode::Dark,
                MaterialTier::T0,
            ),
        );
        assert_ne!(on.rest.tint, off.rest.tint);
    }

    #[test]
    fn the_second_tier_has_a_live_backdrop_only_where_the_table_has_a_cached_one() {
        for role in ALL {
            let t1 = pair(
                role,
                ControlState::default(),
                MaterialContext::Independent,
                Mode::Dark,
                MaterialTier::T1,
            );
            let t2 = pair(
                role,
                ControlState::default(),
                MaterialContext::Independent,
                Mode::Dark,
                MaterialTier::T2,
            );
            match t1.rest.backdrop {
                Backdrop::Cached { radius } => {
                    assert_eq!(
                        t2.rest.backdrop,
                        Backdrop::Live {
                            radius,
                            downsample: 8
                        },
                        "{role:?}"
                    )
                }
                other => assert_eq!(t2.rest.backdrop, other, "{role:?}"),
            }
            assert_eq!(
                t2.rest.shadows, t1.rest.shadows,
                "{role:?}: no further shadow at a higher tier"
            );
        }
    }

    #[test]
    fn a_pill_is_as_round_as_its_height_allows_and_never_more() {
        let p = t1(M::StatusButton, ControlState::default(), Mode::Dark).sized(64.0, 40.0);
        assert_eq!(
            (p.rest.corner_radius, p.pressed.corner_radius),
            (20.0, 20.0)
        );
        let b = t1(M::Button, ControlState::default(), Mode::Dark).sized(200.0, 64.0);
        assert_eq!(b.rest.corner_radius, 14.0);
    }
}
