//! The cursor plane's two routes over the DRM device (`pointer::Backend`).
//!
//! **Legacy** (`DRM_IOCTL_MODE_CURSOR2` and `_CURSOR`): the way the driver is written for. The image goes in once with its hotspot, a move is one call, and
//! the kernel takes both without waiting for a vblank or for the commit in flight (`drm_atomic_helper_update_plane`, `legacy_cursor_update`). The
//! driver subtracts the hotspot and clips the image at the top and left edge. **Atomic** (the plane's own properties): no hotspot, so the origin is
//! held at the screen's top left, and every commit waits for its vblank; it is the route when the legacy one is refused.
//!
//! Both run on the pointer's thread (`pointer::Plane`), on a duplicate of the device's descriptor: the same open file, so the same master.

use std::io;

use drm::buffer::{Buffer, DrmFourcc};
use drm::control::atomic::AtomicModeReq;
use drm::control::dumbbuffer::DumbBuffer;
use drm::control::{AtomicCommitFlags, Device as ControlDevice, crtc, framebuffer};

use super::{Card, PlaneInfo, set_plane};
use crate::Rect;
use crate::pointer::{self, Alpha, Backend, HOTSPOT, PITCH, SPRITE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    Legacy,
    Atomic,
}

pub struct DrmCursor {
    card: Card,
    crtc: crtc::Handle,
    /// The cursor plane, for the atomic route; `None` when the device has none (the legacy route is the only one then).
    plane: Option<PlaneInfo>,
    dumb: Option<DumbBuffer>,
    /// The framebuffer over the image, made when the atomic route needs it.
    fb: Option<framebuffer::Handle>,
    route: Route,
}

/// `errno`s that say the legacy cursor calls are not there for this driver, so the atomic route is tried.
fn not_there(e: &io::Error) -> bool {
    matches!(e.raw_os_error(), Some(libc::ENOSYS | libc::EINVAL | libc::ENXIO | libc::EOPNOTSUPP | libc::ENOTTY))
}

impl DrmCursor {
    /// Makes the image's buffer and writes the sprite into it. Nothing is shown yet.
    pub fn new(card: Card, crtc: crtc::Handle, plane: Option<PlaneInfo>, alpha: Alpha) -> io::Result<DrmCursor> {
        let mut dumb = card.create_dumb_buffer((SPRITE as u32, SPRITE as u32), DrmFourcc::Argb8888, 32)?;
        // the legacy ioctl names the pitch as `width * 4`: a buffer that has another is not the image
        if dumb.pitch() as usize != PITCH {
            let (pitch, handle) = (dumb.pitch(), dumb);
            let _ = card.destroy_dumb_buffer(handle);
            return Err(io::Error::other(format!("the cursor's dumb buffer has a pitch of {pitch}, not {PITCH}")));
        }
        {
            let image = pointer::plane_image(alpha);
            let mut map = card.map_dumb_buffer(&mut dumb)?;
            let bytes: &mut [u8] = &mut map;
            if bytes.len() < image.len() {
                return Err(io::Error::other("the cursor's buffer is smaller than its image"));
            }
            bytes[..image.len()].copy_from_slice(&image);
        }
        Ok(DrmCursor { card, crtc, plane, dumb: Some(dumb), fb: None, route: Route::Legacy })
    }

    fn dumb(&self) -> &DumbBuffer {
        self.dumb.as_ref().expect("the image is there until the cursor is dropped")
    }

    #[allow(deprecated)]
    fn legacy_show(&mut self, at: (i32, i32)) -> io::Result<()> {
        // The position first: the plane is off, the call only keeps it, and the image then appears where the pointer is.
        self.card.move_cursor(self.crtc, at)?;
        self.card.set_cursor2(self.crtc, Some(self.dumb()), HOTSPOT)
    }

    fn atomic_commit(&mut self, at: Option<(i32, i32)>) -> io::Result<()> {
        let Some(plane) = &self.plane else { return Err(io::Error::other("the device has no cursor plane")) };
        if self.fb.is_none() {
            // ADDFB with depth 32 and 32 bits per pixel is ARGB8888
            self.fb = Some(self.card.add_framebuffer(self.dumb.as_ref().expect("the image"), 32, 32)?);
        }
        let mut req = AtomicModeReq::new();
        match at {
            Some(at) => {
                let (x, y) = pointer::atomic_origin(at);
                let size = SPRITE;
                set_plane(&mut req, plane, self.crtc, self.fb.map(u32::from), Rect::new(x, y, size, size), Rect::new(0, 0, size, size), None);
            }
            None => set_plane(&mut req, plane, self.crtc, None, Rect::default(), Rect::default(), None),
        }
        // blocking, on the pointer's thread: the commit waits for its refresh, which is the rate a pointer can be shown at on this route
        self.card.atomic_commit(AtomicCommitFlags::empty(), req)
    }
}

impl Backend for DrmCursor {
    fn route(&self) -> &'static str {
        match self.route {
            Route::Legacy => "legacy cursor ioctls",
            Route::Atomic => "atomic plane commits",
        }
    }

    fn show(&mut self, at: (i32, i32)) -> io::Result<()> {
        if self.route == Route::Legacy {
            match self.legacy_show(at) {
                Ok(()) => return Ok(()),
                Err(e) if not_there(&e) && self.plane.is_some() => {
                    log::warn!("pointer: the legacy cursor ioctls are refused ({e}); using the cursor plane's own properties");
                    self.route = Route::Atomic;
                }
                Err(e) => return Err(e),
            }
        }
        self.atomic_commit(Some(at))
    }

    #[allow(deprecated)]
    fn move_to(&mut self, at: (i32, i32)) -> io::Result<()> {
        match self.route {
            Route::Legacy => self.card.move_cursor(self.crtc, at),
            Route::Atomic => self.atomic_commit(Some(at)),
        }
    }

    #[allow(deprecated)]
    fn hide(&mut self) -> io::Result<()> {
        match self.route {
            Route::Legacy => self.card.set_cursor2::<DumbBuffer>(self.crtc, None, (0, 0)),
            Route::Atomic => self.atomic_commit(None),
        }
    }
}

impl Drop for DrmCursor {
    fn drop(&mut self) {
        let _ = self.hide();
        if let Some(fb) = self.fb.take() {
            let _ = self.card.destroy_framebuffer(fb);
        }
        if let Some(dumb) = self.dumb.take() {
            let _ = self.card.destroy_dumb_buffer(dumb);
        }
    }
}
