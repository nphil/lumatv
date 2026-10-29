//! The system layer's screens (plan section 4.3, order 1): the volume and mute display and the toasts, each a pure model and a
//! surface that draws its view with the kit.

pub mod ai;
pub mod ai_surface;
pub mod alert;
pub mod alert_surface;
pub mod banner;
pub mod banner_surface;
pub mod capture;
pub mod cards;
pub mod cards_surface;
pub mod extras;
pub mod extras_surface;
pub mod format;
pub mod layer;
pub mod layout_surface;
pub mod overlay;
pub mod parts;
pub mod plugins;
pub mod popup;
pub mod popup_surface;
pub mod saver;
pub mod toast;
pub mod toast_surface;
pub mod volume;
pub mod volume_surface;
