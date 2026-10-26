//! The GeForce NOW app: the core client (`core`), the rules (`app`), what the settings screen changes (`prefs`), the box art of the
//! tiles (`art`), where everything goes (`layout`) and the layers that draw it (`ui`); `bin/gfn.rs` runs them in a card window. Account: docs/research/gfn-app-result.md.
pub mod app;
pub mod art;
pub mod core;
pub mod layout;
pub mod prefs;
pub mod ui;

// What the layout checks and the renders go through (tests only).
#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod shots;
