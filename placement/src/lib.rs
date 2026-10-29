//! Where the overlay items sit, and how the owner moves them. No toolkit is in this crate: the QML shell, the Rust shell and anything that
//! speaks to the helper's `/v1/placement` API (`helper/app/src/placement.rs`) draw the same places from the same rules.
//!
//! - [`model`]: the registry of items (the volume bar, the mute mark, the camera window, the notification cards, the toasts), the zones they
//!   may be in, a placement (an anchor and offsets as fractions of the room left, never pixels), what the owner placed (`layout.overlays` in
//!   `theme.json`), and where every item is as a rectangle: flush in the chosen corner at any size, beside the window it stacks on, never in
//!   the bottom quarter.
//! - [`motion`]: one arrow press (a fine step that stops on the corners, thirds and middle; a held key that glides), the volume bar that
//!   slides along three edges and turns the corner, a pointer's drop.
//! - [`editor`]: the controller of the editor screen: browse, pick up, move, put down, put back, Reset, refusals ("Camera window is in the
//!   way"), and the [`editor::View`] a toolkit draws. It asks its caller to save and never saves itself.
//!
//! A surface asks `Registry::rect_of` (or `layout_all` for the whole screen) where its item is, given the size it measured; it never keeps a
//! margin of its own. A new overlay is one entry in [`model::ITEMS`]: the editor, the page that lists the places in words, the file, the
//! overlap rule and the stepping come with it.
//!
//! `theme.json`:
//!
//! ```json
//! { "layout": { "overlays": { "volume": { "anchor": "right", "dx": 0, "dy": 0.1 }, "popup": { "anchor": "top-left", "dx": 0, "dy": 0 } } } }
//! ```
//!
//! An item with no entry is at its default, which is where it was drawn before the owner could place it. `dx` and `dy` are fractions (-1 to 1)
//! of the room the item has left in its zone, from the anchor.

pub mod editor;
pub mod model;
pub mod motion;

pub use editor::{Editor, Effect, Key, View};
pub use model::{
    Anchor, Frame, ITEMS, Item, Laid, Layout, Placed, Placement, Point, Rect, Registry, Size,
    Sizes, Slide, User, ZoneKind, column_room, stack_beside,
};
pub use motion::{Dir, Press, Step};
