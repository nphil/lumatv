//! The parts of the task manager that need nothing of the compositor's state, so each can be tested alone: what the app manager says is
//! running and the processes behind it, the numbers the stats strip shows, the System menu's actions, the theme's colours, and the basic
//! screen the compositor draws itself when the UI cannot (the font it draws with, its layout and its keys). The state machine that joins them
//! to the windows, the topics and the renderer is `switcher.rs` and its children.
//!
//! The contract with the UI and the rescue process is `docs/research/task-manager.md`.

pub mod action;
pub mod fallback;
pub mod font;
pub mod hook;
pub mod palette;
pub mod procs;
pub mod running;
pub mod stats;

pub use action::Action;
pub use palette::Palette;
pub use stats::Stats;
