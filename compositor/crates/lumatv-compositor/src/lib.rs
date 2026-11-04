//! LumaTV's compositor: a 64-bit Wayland server for LG's webOS clients.
//!
//! Track C2 of `docs/PLAN.md`: it runs on a private socket beside LG's `surface-manager`, accepts LG's 32-bit
//! Mali clients (`mali_buffer_sharing` v5 buffers, AFBC dma-bufs) and the webOS shell protocols, and renders to an
//! offscreen target, since LG's compositor holds the DRM master until C4. The design and its reasons are in
//! `docs/research/compositor-c2-result.md`.
//!
//! Layout: `server/` is the protocol layer (one module per global), `surface`/`scene`/`window` the model it
//! builds, `render/` draws the scene into an [`output::Output`], `luna` is the contract with the 32-bit luna
//! bridge, and `main.rs` the process (sockets, tick, control).

#[cfg(test)]
extern crate self as lumatv_compositor;
#[cfg(test)]
#[path = "../tests/harness/mod.rs"]
mod harness;

#[cfg(test)]
mod alloc_tests;
pub mod afbc;
pub mod boot_direct;
pub mod boxdim;
pub mod buffer;
pub mod buttons;
pub mod capture;
pub mod childlog;
pub mod config;
pub mod control;
pub mod damage;
pub mod im_queue;
pub mod input;
pub mod keys;
pub mod kms;
pub mod lg_guide;
pub mod logsink;
pub mod luna;
pub mod luna_methods;
pub mod mask;
pub(crate) mod nooperation;
pub mod mem;
pub mod oledcare;
pub mod output;
pub mod paneldim;
/// LumaShow's frame pacer, copied verbatim by `tools/sync-lumashow.sh` (`vendor/LUMASHOW_REV` names the commit).
#[path = "vendor/pacer.rs"]
pub mod pacer;
pub mod pointer;
pub mod refresh;
pub mod render;
pub mod ring;
pub mod scanout;
pub mod scene;
pub mod sched;
pub mod screencap;
pub mod server;
pub mod shell;
pub mod shell_rust;
pub mod surface;
pub mod switcher;
pub mod system_ui;
pub mod ticker;
pub mod transition;
/// LumaTV's UI engine drawing into the compositor's pages (feature `ui`, `rustui/`).
#[cfg(feature = "ui")]
pub mod ui;
pub mod vwin;
pub mod wake;
pub mod window;

pub use server::state::State;

/// Pixel size of the UI output. PLAN: 1080p at 60 Hz, scaled to the panel by the display engine.
pub const OUTPUT_WIDTH: u32 = 1920;
pub const OUTPUT_HEIGHT: u32 = 1080;
/// The 65-inch panel's picture area, for `wl_output.geometry`.
pub const OUTPUT_MM: (i32, i32) = (1439, 809);
pub const REFRESH_MHZ: i32 = 60_000;

/// A rectangle in output pixels, `x`/`y` top-left.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect { x, y, w, h }
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    pub fn right(&self) -> i32 {
        self.x.saturating_add(self.w)
    }

    pub fn bottom(&self) -> i32 {
        self.y.saturating_add(self.h)
    }

    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        if r <= x || b <= y { Rect::default() } else { Rect::new(x, y, r - x, b - y) }
    }

    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        let r = self.right().max(o.right());
        let b = self.bottom().max(o.bottom());
        Rect::new(x, y, r - x, b - y)
    }

    pub fn translated(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.right() && py < self.bottom()
    }
}

#[cfg(test)]
mod rect_tests {
    use super::Rect;

    #[test]
    fn intersect_and_union() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 10, 10);
        assert_eq!(a.intersect(&b), Rect::new(5, 5, 5, 5));
        assert_eq!(a.union(&b), Rect::new(0, 0, 15, 15));
        assert!(a.intersect(&Rect::new(20, 20, 1, 1)).is_empty());
        assert_eq!(Rect::default().union(&b), b);
    }
}
