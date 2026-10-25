//! LumaShow's compositor renderer, scene and session as a library.
//!
//! Everything except this file is copied verbatim from LumaShow's `crates/compositor/src` by `tools/sync-lumashow.sh` (the
//! compositor is a binary crate, so a path dependency is not possible) and carries LumaShow's own module layout: the modules
//! name each other as `crate::<module>`, which is why they sit at this crate's root. The module list is `main.rs`'s, cut
//! to what draws a frame: the renderer and its GL wrapper, the scene that animates layers, the session that checks an app's
//! requests, and what they use. The scan-out, present and engine threads are not here: the host that links this crate
//! (`app/`) puts frames on a window itself.
#![allow(dead_code, unused_imports, clippy::all)]

mod anim;
mod atlas;
mod backdrop;
mod census;
mod clock;
pub mod damage;
mod feather;
mod geometry;
pub mod host;
pub mod gl;
mod glass;
mod groups;
mod idle;
mod ladder;
mod live;
mod log;
mod mesh;
pub mod ops;
pub mod pacer;
mod plates;
mod prepared;
mod profile;
pub mod renderer;
pub mod scene;
pub mod session;
mod shaders;
mod shadow;
mod shm;
pub mod stats;
mod still;
pub mod tiles;
mod wallpaper;
mod wallpapers;

pub use geometry::Panel;
pub use gl::{Gles, RenderTarget};
pub use ops::{Op, Transaction};
pub use renderer::{Draw, DrawReport, Renderer, Surface};
pub use scene::{Demand, Drawn, Scene};
pub use session::{Config, Limits, Session};
pub use stats::SharedStats;
pub use clock::now_ns;
pub use wallpapers::Look;
