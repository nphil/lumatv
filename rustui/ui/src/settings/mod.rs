//! Settings, the Rust shell's own (`docs/research/rust-shell-plan.md`, phase 4): a rail of sections and the page of the one in front,
//! each page a function from what the shell knows to rows (toggles, segmented rows, sliders, choices, cards, actions, help sheets),
//! over LG's settings service (`host::sio`, the `lg` owner), the owners of the helper's settings layer, the helper's topics
//! (`profiles`, `privacy`, `openapps`, `updateblock`) and the luna services the QML pages called. The model is pure; the surface
//! draws its view with the kit.

pub mod calendar;
pub mod ctx;
pub mod host;
pub mod names;
pub mod page;
pub mod row;
pub mod src;
pub mod model;
pub mod view;
pub mod pages;
pub mod surface;
