//! Pieces shared by LumaGame's screens.
// The drawer, dialog, strip, hint bar, glance card, rail shell, text entry and the new rows are reached only by their tests and the
// parts gallery until the page slices (F2b, F2c, F3) draw with them; this goes when they do.
#![allow(dead_code)]
pub mod card;
pub mod dialog;
pub mod drawer;
pub mod focus;
pub mod hints;
pub mod keyboard;
pub mod layer;
pub mod motion;
pub mod rail;
pub mod row;
pub mod stage;
pub mod strip;
pub mod topbar;

/// The edge of the screen or of a layer that a part sits on or reads from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// A layer's scrim and sheet sit at these depths, and what the layer holds above them. `layer::sheet` lifts every layer 100 higher, over
/// the page; a layer a page paints itself needs a page that stays under these.
pub const SCRIM_Z: i32 = 25;
pub const SHEET_Z: i32 = 30;

/// A `Paint` over the shipped theme and a default model, for the tests of the parts: `setup` changes the model first.
#[cfg(test)]
pub(super) fn painted<R>(setup: impl FnOnce(&mut crate::game::model::Model), f: impl FnOnce(&mut super::paint::Paint<'_>) -> R) -> R {
    painted_in(ui::lucent::palette::Mode::Dark, setup, f)
}
/// `painted` in the dark or the light theme.
#[cfg(test)]
pub(super) fn painted_in<R>(
    mode: ui::lucent::palette::Mode,
    setup: impl FnOnce(&mut crate::game::model::Model),
    f: impl FnOnce(&mut super::paint::Paint<'_>) -> R,
) -> R {
    use lumagame_core::{catalog::Listing, state::State};
    let host = lumatv_tokens::Tokens::shipped();
    let tokens = lumatv_tokens::themes::game_tokens("rose-pine", mode, &host).unwrap();
    let mut model = crate::game::model::Model::new(State::default(), Listing::default(), vec![]);
    setup(&mut model);
    let mut fonts = ui::Fonts::new();
    f(&mut super::paint::Paint::new(&tokens, &model, &mut fonts))
}
