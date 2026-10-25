//! The two renderers. `software` composes `wl_shm` buffers on the CPU, exactly, and is what the host tests
//! and a GPU-less run use; `gl` runs on the 64-bit Mali blob and is the path for LG's AFBC buffers.

pub mod egl;
pub mod gl;
pub mod software;
