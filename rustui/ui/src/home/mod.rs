//! Home, the `home` kind (`docs/research/rust-shell-plan.md` phase 3): the wallpaper, the status bar with the profile chip, the cards
//! (clock, weather, at home, news, cameras), the dock, the library, search with the keyboard hook, the layout editor with its live
//! preview, and "Who's watching?" with the switch veil, drawn on stack 0 under the compositor's windows with the flowing focus on every
//! focusable. The models are pure ([`model::Home`] and its parts); [`surface::HomeSurface`] draws them with the kit.

pub mod apps;
pub mod clock;
pub mod dock;
pub mod editor;
pub mod grid;
pub mod layers;
pub mod model;
pub mod picker;
pub mod radar_surface;
pub mod radar_view;
pub mod popover;
pub mod search;
pub mod stills;
pub mod surface;
pub mod wallpaper;
pub mod wallvideo;
pub mod weather_art;
pub mod weather_view;
pub mod widgets;

pub use model::Home;
pub use surface::HomeSurface;
