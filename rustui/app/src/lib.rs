//! LumaHome as a client of LumaShow's Rust UI toolkit: the scene (`home`), its look (`look`), the drawing it does for itself
//! (`raster`) and the remote's scripts (`script`). The binary (`main.rs`) runs it as a window of LG's compositor or into a
//! pbuffer; the tests run it in-process through the renderer's host.

pub mod home;
pub mod look;
pub mod raster;
pub mod rig;
pub mod script;
