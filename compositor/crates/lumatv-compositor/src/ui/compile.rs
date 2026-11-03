//! The engine's shader programs, linked off the loop thread.
//!
//! The native engine attaches on the loop thread at startup and on client restart. Linking its 48 programs there would block the loop:
//! the set's Mali took 1.3–1.5 s. They are linked on a separate shared context first, then the attach takes them from
//! `lumatv_render::programs::Cache` and links nothing (`Gl::attach_ui_when_ready`).
//!
//! Sharing is what lets a program made in one context be drawn with in another: programs, shaders, textures and buffers are objects of
//! the share group, where framebuffers and vertex arrays are the context's own, which is why a whole renderer cannot be made here and
//! handed over. [INFERENCE: not seen on the set] the Mali blob shares programs between contexts as the EGL and GL ES specifications
//! say; the engine takes a program only if it reads as linked in the loop's context (`Gles::program`) and links its own otherwise.

use crate::render::egl::Sharing;
use lumatv_render::Gles;
use lumatv_render::host::{self, Precompiled};
use lumatv_render::programs::Cache;
use std::ffi::CString;
use std::io;
use std::sync::Arc;
use std::thread::JoinHandle;

/// The thread's niceness: linking is CPU the loop and the clients in front need more than this does.
const NICE: i32 = 10;
/// The thread's stack: a renderer is built and drawn on it, as on the loop's.
const STACK: usize = 8 << 20;

/// The thread that links the programs, while it runs.
pub struct Compile {
    cache: Arc<Cache>,
    thread: Option<JoinHandle<io::Result<Precompiled>>>,
}

impl Compile {
    /// Starts the thread: it makes its context (`sharing`), builds a renderer for a panel of `width` by `height` through `cache`, draws
    /// every program once and ends, the programs left in `cache`.
    pub fn start(sharing: Sharing, driver: &str, width: u32, height: u32, cache: Arc<Cache>) -> io::Result<Self> {
        let driver = CString::new(driver).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "driver path"))?;
        let thread_cache = Arc::clone(&cache);
        let thread = std::thread::Builder::new().name("ui-shaders".into()).stack_size(STACK).spawn(move || {
            // SAFETY: setpriority with who 0 is the calling thread on Linux.
            unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, NICE) };
            let context = sharing.make_current()?;
            // SAFETY: the context just made is current on this thread and stays so until `context` is dropped, after the `Gles` is gone.
            let gles = unsafe { Gles::current(&driver) }?;
            let made = host::precompile(gles, &thread_cache, width, height);
            drop(context);
            made
        })?;
        Ok(Self { cache, thread: Some(thread) })
    }

    /// What the thread made, once it has ended (the loop asks without waiting); `None` while it runs.
    pub fn poll(&mut self) -> Option<io::Result<Precompiled>> {
        if !self.thread.as_ref()?.is_finished() {
            return None;
        }
        Some(self.thread.take()?.join().unwrap_or_else(|_| Err(io::Error::other("the shader thread panicked"))))
    }
}

impl Drop for Compile {
    /// The thread uses the display the renderer owns, so it ends first: at the next program, not after all 48.
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            self.cache.cancel();
            let _ = thread.join();
        }
    }
}
