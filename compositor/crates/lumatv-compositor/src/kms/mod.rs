//! The KMS output (C3): LumaTV as DRM master of `card0`, composing into its own scan-out ring on the primary
//! plane with the GL fence as the commit's in-fence, scanning a full-screen client buffer out directly when the
//! driver takes its layout (X5), and reading the page-flip timestamps for the pacer.
//!
//! What the set offers (`docs/research/arch/drmplanes-01.txt`): one CRTC (38) with `ACTIVE`, `MODE_ID` (blob
//! 41, 1920x1080@60) and `OUT_FENCE_PTR`; connector 31 with `CRTC_ID`; planes 32 (primary), 34 (overlay) and 36
//! (cursor), each with `FB_ID`, `CRTC_ID`, `CRTC_X/Y/W/H`, `SRC_X/Y/W/H`, `IN_FENCE_FD` and `IN_FORMATS`
//! (AR24/XR24/AB24/XB24 linear, plus AFBC `0x0800000000000062` for AB24/XB24 on the OSD planes). No zpos, no
//! alpha: the planes stack in hardware order (primary under overlay).
//!
//! Only the DRM master can commit, and LG's compositor is the master while it runs, so this output is used in a
//! maintenance window (`tools/tv-kms-window.sh`) and, after C4, as the live one. Nothing here is testable on the
//! workstation beyond compiling; the pure parts (`assign`) are.

pub mod assign;
pub mod frames;

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::time::{Duration, Instant};

use drm::buffer::{Buffer, DrmFourcc, DrmModifier, PlanarBuffer};
use drm::control::atomic::AtomicModeReq;
use drm::control::dumbbuffer::DumbBuffer;
use drm::control::{AtomicCommitFlags, Device as ControlDevice, Event, FbCmd2Flags, PlaneType, connector, crtc, framebuffer, plane, property};
use drm::{ClientCapability, Device, DriverCapability};
use wayland_backend::server::ObjectId;
use wayland_server::Resource;
use wayland_server::protocol::wl_buffer::WlBuffer;

use crate::Rect;
use crate::buffer::{BufferKindRef, kind_of};
use crate::output::{Image, Output, Presented, RenderStats};
use crate::pacer::{Pacer, Shown};
use crate::render::gl::{Gl, TargetId};
use crate::scene::Plan;
use crate::surface::Surfaces;
use assign::{Assignment, Source};

/// The DRM device file.
pub struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}
impl Device for Card {}
impl ControlDevice for Card {}

/// The properties of one plane, by handle, looked up once.
#[derive(Clone, Copy)]
pub struct PlaneProps {
    pub fb_id: property::Handle,
    pub crtc_id: property::Handle,
    pub crtc_x: property::Handle,
    pub crtc_y: property::Handle,
    pub crtc_w: property::Handle,
    pub crtc_h: property::Handle,
    pub src_x: property::Handle,
    pub src_y: property::Handle,
    pub src_w: property::Handle,
    pub src_h: property::Handle,
    pub in_fence_fd: Option<property::Handle>,
}

pub struct PlaneInfo {
    pub handle: plane::Handle,
    pub props: PlaneProps,
    /// `(fourcc, modifier)` pairs from `IN_FORMATS`.
    pub formats: Vec<(u32, u64)>,
}

/// One page of the compositor's own scan-out ring.
struct Page {
    dumb: DumbBuffer,
    fd: OwnedFd,
    fb: framebuffer::Handle,
    target: TargetId,
}

/// A client dma-buf imported for direct scan-out.
struct DirectFb {
    fb: framebuffer::Handle,
    gem: drm::buffer::Handle,
    buffer: WlBuffer,
}

/// What a plane showed in the last commit, to leave unchanged planes out of the request.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct PlaneState {
    fb: Option<u32>,
    dst: Rect,
}

#[derive(Clone, Copy)]
struct Frame {
    assignment: Assignment,
    #[cfg(feature = "ui")]
    ui: Option<lumatv_render::host::FrameCost>,
}

pub struct Kms {
    card: Card,
    crtc: crtc::Handle,
    connector: connector::Handle,
    mode_blob: u64,
    crtc_active: property::Handle,
    crtc_mode_id: property::Handle,
    crtc_out_fence: Option<property::Handle>,
    conn_crtc_id: property::Handle,
    pub primary: PlaneInfo,
    pub overlay: Option<PlaneInfo>,
    gl: Gl,
    width: u32,
    height: u32,
    pages: Vec<Page>,
    frames: frames::Frames,
    direct: HashMap<ObjectId, DirectFb>,
    /// Modifiers the driver refused in ADDFB2, so they are not tried again.
    refused_modifiers: Vec<u64>,
    state: [PlaneState; 2],
    modeset_done: bool,
    pending_flip: bool,
    /// When the commit whose flip is pending went out (CLOCK_MONOTONIC ns).
    committed_ns: u64,
    /// A drawn frame waiting for the flip in flight to land; `dispatch_events` commits it. The loop never blocks on
    /// a flip (it used to, up to 100 ms, stalling every client's frame callbacks with it).
    queued: Option<Frame>,
    /// Set when `render` took over the queued frame's page, so the next `present` reports a replacement.
    replacing: bool,
    pub pacer: Pacer,
    /// Page-flip timestamps are CLOCK_MONOTONIC when the driver says so (DRM_CAP_TIMESTAMP_MONOTONIC), else
    /// CLOCK_REALTIME and translated on arrival; everything else here is CLOCK_MONOTONIC in ns.
    monotonic_flips: bool,
    frame_started_ns: u64,
    frame_ready_ns: u64,
    frame_slot_ns: u64,
    last_assignment: Option<Frame>,
    #[cfg(feature = "ui")]
    pending_ui: Option<lumatv_render::host::FrameCost>,
    /// When the last flip landed, 0 before the first.
    last_flip_ns: u64,
    /// Whether the last tick drew a frame: the run of frames the pacer keeps in step is on while ticks keep
    /// drawing; the first tick that finds nothing to draw ends it and the fixed tick takes over.
    in_run: bool,
    pub flips: u64,
    pub direct_frames: u64,
    pub late_frames: u64,
    pub lost_refreshes: u64,
    seq: u64,
}

const PAGES: usize = 3;

fn other(msg: impl Into<String>) -> io::Error {
    io::Error::other(msg.into())
}

/// CLOCK_MONOTONIC in ns.
fn mono_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: a stack timespec for clock_gettime.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

impl Kms {
    /// Opens `path` (`/dev/dri/card0`), becomes master and finds the CRTC, connector and planes. `gl` renders into
    /// the pages this makes.
    pub fn open(path: &str, mut gl: Gl, width: u32, height: u32) -> io::Result<Kms> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let card = Card(file);
        card.acquire_master_lock().map_err(|e| other(format!("DRM master on {path}: {e} (is LG's compositor still running?)")))?;
        card.set_client_capability(ClientCapability::UniversalPlanes, true)?;
        card.set_client_capability(ClientCapability::Atomic, true)?;
        let res = card.resource_handles()?;
        let connector = *res.connectors().first().ok_or_else(|| other("no connector"))?;
        let crtc = *res.crtcs().first().ok_or_else(|| other("no CRTC"))?;
        let conn_info = card.get_connector(connector, false)?;
        let mode = conn_info
            .modes()
            .iter()
            .find(|m| m.size() == (width as u16, height as u16))
            .copied()
            .or_else(|| card.get_crtc(crtc).ok().and_then(|c| c.mode()))
            .ok_or_else(|| other(format!("no {width}x{height} mode")))?;
        let mode_blob = match card.create_property_blob(&mode)? {
            property::Value::Blob(b) => b,
            _ => return Err(other("mode blob")),
        };
        let crtc_props = card.get_properties(crtc)?.as_hashmap(&card)?;
        let conn_props = card.get_properties(connector)?.as_hashmap(&card)?;
        let need = |m: &HashMap<String, property::Info>, n: &str| m.get(n).map(|p| p.handle()).ok_or_else(|| other(format!("no property {n}")));
        let crtc_active = need(&crtc_props, "ACTIVE")?;
        let crtc_mode_id = need(&crtc_props, "MODE_ID")?;
        let crtc_out_fence = crtc_props.get("OUT_FENCE_PTR").map(|p| p.handle());
        let conn_crtc_id = need(&conn_props, "CRTC_ID")?;

        let mut primary = None;
        let mut overlay = None;
        for h in card.plane_handles()?.iter().copied() {
            let info = card.get_plane(h)?;
            if !res.filter_crtcs(info.possible_crtcs()).contains(&crtc) {
                continue;
            }
            let props = card.get_properties(h)?.as_hashmap(&card)?;
            let kind = match props
                .get("type")
                .and_then(|p| card.get_properties(h).ok().and_then(|set| set.iter().find(|(id, _)| **id == p.handle()).map(|(_, v)| *v)))
            {
                Some(0) => PlaneType::Overlay,
                Some(1) => PlaneType::Primary,
                Some(2) => PlaneType::Cursor,
                _ => continue,
            };
            let p = PlaneProps {
                fb_id: need(&props, "FB_ID")?,
                crtc_id: need(&props, "CRTC_ID")?,
                crtc_x: need(&props, "CRTC_X")?,
                crtc_y: need(&props, "CRTC_Y")?,
                crtc_w: need(&props, "CRTC_W")?,
                crtc_h: need(&props, "CRTC_H")?,
                src_x: need(&props, "SRC_X")?,
                src_y: need(&props, "SRC_Y")?,
                src_w: need(&props, "SRC_W")?,
                src_h: need(&props, "SRC_H")?,
                in_fence_fd: props.get("IN_FENCE_FD").map(|p| p.handle()),
            };
            let formats = props
                .get("IN_FORMATS")
                .and_then(|p| card.get_properties(h).ok().and_then(|set| set.iter().find(|(id, _)| **id == p.handle()).map(|(_, v)| *v)))
                .and_then(|blob| card.get_property_blob(blob).ok())
                .map(|b| parse_in_formats(&b))
                .unwrap_or_default();
            let pi = PlaneInfo { handle: h, props: p, formats };
            match kind {
                PlaneType::Primary if primary.is_none() => primary = Some(pi),
                PlaneType::Overlay if overlay.is_none() => overlay = Some(pi),
                _ => {}
            }
        }
        let primary = primary.ok_or_else(|| other("no primary plane"))?;
        log::info!(
            "kms: crtc {} connector {} mode {}x{}@{} primary plane {} ({} formats) overlay {}",
            u32::from(crtc),
            u32::from(connector),
            mode.size().0,
            mode.size().1,
            mode.vrefresh(),
            u32::from(primary.handle),
            primary.formats.len(),
            overlay.as_ref().map(|o| u32::from(o.handle).to_string()).unwrap_or_else(|| "none".into())
        );

        let mut pages = Vec::with_capacity(PAGES);
        for i in 0..PAGES {
            let dumb = card.create_dumb_buffer((width, height), DrmFourcc::Xrgb8888, 32)?;
            let fb = card.add_framebuffer(&dumb, 24, 32)?;
            let fd = card.buffer_to_prime_fd(dumb.handle(), libc::O_CLOEXEC as u32)?;
            let target = gl
                .add_dmabuf_target(fd.as_raw_fd(), width as i32, height as i32, crate::buffer::fourcc::XRGB8888, dumb.pitch(), 0)
                .map_err(|e| other(format!("page {i}: {e}")))?;
            pages.push(Page { dumb, fd, fb, target });
        }
        let period_ns = 1_000_000_000 / mode.vrefresh().max(1) as u64;
        let monotonic_flips = card.get_driver_capability(DriverCapability::MonotonicTimestamp).map(|v| v != 0).unwrap_or(false);
        log::info!("kms: page-flip timestamps are {}", if monotonic_flips { "CLOCK_MONOTONIC" } else { "CLOCK_REALTIME (translated)" });
        Ok(Kms {
            card,
            crtc,
            connector,
            mode_blob,
            crtc_active,
            crtc_mode_id,
            crtc_out_fence,
            conn_crtc_id,
            primary,
            overlay,
            gl,
            width,
            height,
            frames: frames::Frames::new(pages.iter().map(|p| p.target).collect(), width, height),
            pages,
            direct: HashMap::new(),
            refused_modifiers: Vec::new(),
            state: [PlaneState::default(); 2],
            modeset_done: false,
            pending_flip: false,
            committed_ns: 0,
            queued: None,
            replacing: false,
            pacer: Pacer::new(period_ns),
            monotonic_flips,
            frame_started_ns: 0,
            frame_ready_ns: 0,
            frame_slot_ns: 0,
            last_assignment: None,
            #[cfg(feature = "ui")]
            pending_ui: None,
            last_flip_ns: 0,
            in_run: false,
            flips: 0,
            direct_frames: 0,
            late_frames: 0,
            lost_refreshes: 0,
            seq: 0,
        })
    }

    pub fn gl(&self) -> &Gl {
        &self.gl
    }

    /// CLOCK_MONOTONIC in ns: the clock the kernel stamps page flips with, so the pacer's phase (flip time minus
    /// slot) is a small number and not the uptime. The first TV run used a process-relative clock here and armed
    /// the next tick hours ahead after the first flip (3 frames drawn in 30 s).
    fn now_ns(&self) -> u64 {
        mono_ns()
    }

    /// Whether frames are following one another closely enough for the pacer's timeline to be worth keeping.
    fn flowing(&self, now_ns: u64) -> bool {
        self.in_run && self.pacer.phase_ns().is_some() && self.pacer.flowing(now_ns)
    }

    /// Whether `fourcc`/`modifier` is in the primary plane's `IN_FORMATS`.
    pub fn primary_takes(&self, fourcc: u32, modifier: u64) -> bool {
        self.primary.formats.iter().any(|(f, m)| *f == fourcc && *m == modifier)
    }

    /// A framebuffer for a client dma-buf (direct scan-out), made once per buffer; `None` when the driver
    /// refuses its layout.
    fn direct_fb(&mut self, buffer: &WlBuffer) -> Option<framebuffer::Handle> {
        let id = buffer.id();
        if let Some(d) = self.direct.get(&id) {
            return Some(d.fb);
        }
        let BufferKindRef::Dmabuf(d) = kind_of(buffer) else { return None };
        if self.refused_modifiers.contains(&d.modifier) || d.planes.is_empty() {
            return None;
        }
        let gem = match self.card.prime_fd_to_buffer(d.planes[0].fd.as_fd()) {
            Ok(g) => g,
            Err(e) => {
                log::warn!("direct: prime import: {e}");
                return None;
            }
        };
        let pb = Planar {
            size: (d.width as u32, d.height as u32),
            fourcc: d.format,
            modifier: d.modifier,
            pitch: d.planes[0].stride,
            offset: d.planes[0].offset,
            handle: gem,
        };
        let flags = if d.modifier != 0 { FbCmd2Flags::MODIFIERS } else { FbCmd2Flags::empty() };
        match self.card.add_planar_framebuffer(&pb, flags) {
            Ok(fb) => {
                log::info!("direct: fb {} for {}x{} {} modifier 0x{:x}", u32::from(fb), d.width, d.height, crate::buffer::fourcc::name(d.format), d.modifier);
                self.direct.insert(id, DirectFb { fb, gem, buffer: buffer.clone() });
                Some(fb)
            }
            Err(e) => {
                log::warn!("direct: ADDFB2 refused {} modifier 0x{:x}: {e}; composing instead", crate::buffer::fourcc::name(d.format), d.modifier);
                self.refused_modifiers.push(d.modifier);
                let _ = self.card.close_buffer(gem);
                None
            }
        }
    }

    fn prune_direct(&mut self) {
        let dead: Vec<ObjectId> = self.direct.iter().filter(|(_, d)| !d.buffer.is_alive()).map(|(id, _)| id.clone()).collect();
        for id in dead {
            if let Some(d) = self.direct.remove(&id) {
                let _ = self.card.destroy_framebuffer(d.fb);
                let _ = self.card.close_buffer(d.gem);
            }
        }
    }

    /// Reads the DRM events that have arrived (without waiting); returns how many page flips landed.
    fn drain_events(&mut self) -> io::Result<u32> {
        let mut landed = 0;
        for ev in self.card.receive_events()? {
            if let Event::PageFlip(f) = ev {
                self.flip_landed(f.duration.as_nanos() as u64);
                landed += 1;
            }
        }
        Ok(landed)
    }

    fn flip_landed(&mut self, stamp_ns: u64) {
        self.pending_flip = false;
        self.flips += 1;
        let presented_ns = if self.monotonic_flips {
            stamp_ns
        } else {
            // CLOCK_REALTIME stamp: carry it over by the two clocks' current difference.
            let mut rt = libc::timespec { tv_sec: 0, tv_nsec: 0 };
            // SAFETY: a stack timespec for clock_gettime.
            unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut rt) };
            let rt_ns = rt.tv_sec as u64 * 1_000_000_000 + rt.tv_nsec as u64;
            mono_ns().saturating_sub(rt_ns.saturating_sub(stamp_ns))
        };
        self.last_flip_ns = presented_ns;
        #[cfg(feature = "ui")]
        if let Some(cost) = self.pending_ui.take() {
            self.gl.ui_presented(presented_ns, &cost);
        }
        let shown = Shown { slot_ns: self.frame_slot_ns, started_ns: self.frame_started_ns, ready_ns: self.frame_ready_ns, presented_ns, content_ns: None };
        let landing = self.pacer.presented(&shown);
        if landing.late > 0 {
            self.late_frames += 1;
        }
        self.lost_refreshes += landing.lost as u64;
    }

    /// Commits a drawn frame now: the GL fence of a composed page goes along as the in-fence.
    fn commit_frame(&mut self, frame: Frame) -> io::Result<()> {
        let a = frame.assignment;
        let fence = if matches!(a.primary, Source::Page(_)) { self.gl.export_fence() } else { None };
        self.commit(&a, fence)?;
        #[cfg(feature = "ui")]
        {
            self.pending_ui = frame.ui;
        }
        self.committed_ns = self.now_ns();
        Ok(())
    }

    /// A flip that has not reported 250 ms after its commit is taken as lost (the panel off, an event the driver
    /// dropped), so commits do not stop for good; the kernel refuses the next one if it really is still pending.
    fn flip_overdue(&mut self) -> bool {
        if self.pending_flip && self.now_ns().saturating_sub(self.committed_ns) > 250_000_000 {
            log::warn!("kms: no page-flip event 250 ms after the commit; going on without it");
            self.pending_flip = false;
            return true;
        }
        false
    }

    fn commit(&mut self, a: &Assignment, in_fence: Option<OwnedFd>) -> io::Result<()> {
        let mut req = AtomicModeReq::new();
        let mut flags = AtomicCommitFlags::PAGE_FLIP_EVENT | AtomicCommitFlags::NONBLOCK;
        if !self.modeset_done {
            req.add_property(self.connector, self.conn_crtc_id, property::Value::CRTC(Some(self.crtc)));
            req.add_property(self.crtc, self.crtc_mode_id, property::Value::Blob(self.mode_blob));
            req.add_property(self.crtc, self.crtc_active, property::Value::Boolean(true));
            flags |= AtomicCommitFlags::ALLOW_MODESET;
            flags.remove(AtomicCommitFlags::NONBLOCK);
        }
        let full = Rect::new(0, 0, self.width as i32, self.height as i32);
        let primary_fb = match a.primary {
            Source::Page(i) => Some(u32::from(self.pages[i].fb)),
            Source::Direct(fb) => Some(fb),
            Source::Off => None,
        };
        let primary_state = PlaneState { fb: primary_fb, dst: full };
        let mut new_state = self.state;
        if primary_state != self.state[0] || !self.modeset_done {
            set_plane(&mut req, &self.primary, self.crtc, primary_fb, full, (self.width, self.height), in_fence.as_ref().map(|f| f.as_raw_fd()));
            new_state[0] = primary_state;
        }
        if let Some(ov) = &self.overlay {
            let (fb, dst) = match a.overlay {
                Some((Source::Page(i), r)) => (Some(u32::from(self.pages[i].fb)), r),
                Some((Source::Direct(fb), r)) => (Some(fb), r),
                _ => (None, Rect::default()),
            };
            let st = PlaneState { fb, dst };
            if st != self.state[1] || !self.modeset_done {
                set_plane(&mut req, ov, self.crtc, fb, dst, (dst.w.max(0) as u32, dst.h.max(0) as u32), None);
                new_state[1] = st;
            }
        }
        let _ = self.crtc_out_fence;
        match self.card.atomic_commit(flags, req) {
            Ok(()) => {
                // The planes' recorded state follows only a commit the kernel took; after a refused one the next
                // request must carry everything again.
                self.state = new_state;
                self.modeset_done = true;
                self.pending_flip = true;
                Ok(())
            }
            Err(e) => Err(other(format!("atomic commit: {e}"))),
        }
    }
}

/// Adds every property of one plane to the request.
fn set_plane(req: &mut AtomicModeReq, p: &PlaneInfo, crtc: crtc::Handle, fb: Option<u32>, dst: Rect, src: (u32, u32), in_fence: Option<i32>) {
    let h = p.handle;
    match fb {
        Some(fb) => {
            req.add_property(h, p.props.fb_id, property::Value::Framebuffer(Some(framebuffer::Handle::from(std::num::NonZeroU32::new(fb).unwrap()))));
            req.add_property(h, p.props.crtc_id, property::Value::CRTC(Some(crtc)));
            req.add_property(h, p.props.crtc_x, property::Value::SignedRange(dst.x as i64));
            req.add_property(h, p.props.crtc_y, property::Value::SignedRange(dst.y as i64));
            req.add_property(h, p.props.crtc_w, property::Value::UnsignedRange(dst.w as u64));
            req.add_property(h, p.props.crtc_h, property::Value::UnsignedRange(dst.h as u64));
            req.add_property(h, p.props.src_x, property::Value::UnsignedRange(0));
            req.add_property(h, p.props.src_y, property::Value::UnsignedRange(0));
            req.add_property(h, p.props.src_w, property::Value::UnsignedRange((src.0 as u64) << 16));
            req.add_property(h, p.props.src_h, property::Value::UnsignedRange((src.1 as u64) << 16));
            if let (Some(prop), Some(fd)) = (p.props.in_fence_fd, in_fence) {
                req.add_property(h, prop, property::Value::SignedRange(fd as i64));
            }
        }
        None => {
            req.add_property(h, p.props.fb_id, property::Value::Framebuffer(None));
            req.add_property(h, p.props.crtc_id, property::Value::CRTC(None));
        }
    }
}

/// `IN_FORMATS` blob: `struct drm_format_modifier_blob` followed by the fourcc list and the modifier entries.
pub fn parse_in_formats(b: &[u8]) -> Vec<(u32, u64)> {
    let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let u64_at = |o: usize| u64::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3], b[o + 4], b[o + 5], b[o + 6], b[o + 7]]);
    if b.len() < 24 {
        return Vec::new();
    }
    // struct drm_format_modifier_blob { u32 version, flags, count_formats, formats_offset, count_modifiers, modifiers_offset }.
    let count_formats = u32_at(8) as usize;
    let formats_offset = u32_at(12) as usize;
    let count_modifiers = u32_at(16) as usize;
    let modifiers_offset = u32_at(20) as usize;
    let mut out = Vec::new();
    // struct drm_format_modifier { u64 formats; u32 offset; u32 pad; u64 modifier; }: 24 bytes.
    for m in 0..count_modifiers {
        let o = modifiers_offset + m * 24;
        if o + 24 > b.len() {
            break;
        }
        let formats_bits = u64_at(o);
        let offset = u32_at(o + 8) as usize;
        let modifier = u64_at(o + 16);
        for bit in 0..64 {
            if formats_bits & (1u64 << bit) != 0 {
                let fi = offset + bit;
                if fi < count_formats {
                    let fo = formats_offset + fi * 4;
                    if fo + 4 <= b.len() {
                        out.push((u32_at(fo), modifier));
                    }
                }
            }
        }
    }
    out
}

struct Planar {
    size: (u32, u32),
    fourcc: u32,
    modifier: u64,
    pitch: u32,
    offset: u32,
    handle: drm::buffer::Handle,
}

impl PlanarBuffer for Planar {
    fn size(&self) -> (u32, u32) {
        self.size
    }
    fn format(&self) -> DrmFourcc {
        DrmFourcc::try_from(self.fourcc).unwrap_or(DrmFourcc::Xrgb8888)
    }
    fn modifier(&self) -> Option<DrmModifier> {
        if self.modifier == 0 { None } else { Some(DrmModifier::from(self.modifier)) }
    }
    fn pitches(&self) -> [u32; 4] {
        [self.pitch, 0, 0, 0]
    }
    fn handles(&self) -> [Option<drm::buffer::Handle>; 4] {
        [Some(self.handle), None, None, None]
    }
    fn offsets(&self) -> [u32; 4] {
        [self.offset, 0, 0, 0]
    }
}

impl Output for Kms {
    fn name(&self) -> &'static str {
        "kms"
    }

    /// The pacer's latch while a run of frames is on (start late, aim early, one frame per refresh). Between
    /// runs the fixed tick rules: the latch would be in the past then (the slot timeline stops with the last
    /// drawn frame) and re-arming at it spun the loop at 2000 ticks/s in the first TV run.
    fn next_frame_at(&self, now: Instant) -> Option<Instant> {
        let now_ns = self.now_ns();
        if !self.flowing(now_ns) {
            return None;
        }
        let latch = self.pacer.latch_at(now_ns);
        Some(now + Duration::from_nanos(latch.saturating_sub(now_ns)))
    }

    fn event_fd(&self) -> Option<RawFd> {
        Some(self.card.as_fd().as_raw_fd())
    }

    fn dispatch_events(&mut self) -> io::Result<u32> {
        let mut landed = self.drain_events()?;
        if !self.pending_flip
            && let Some(a) = self.queued.take()
            && let Err(e) = self.commit_frame(a)
        {
            // The state's callbacks for it go out as if it had landed; the next frame carries its damage anyway.
            log::warn!("kms: queued frame: {e}");
            landed += 1;
        }
        Ok(landed)
    }

    /// A commit onto a still screen is drawn at once: nothing is in flight to wait for and no run to keep in
    /// step with, so the tick would only delay it by up to a refresh.
    fn draws_on_damage(&self, _now: Instant) -> bool {
        !self.pending_flip && self.queued.is_none() && self.last_assignment.is_none() && !self.flowing(self.now_ns())
    }

    fn stats_line(&self) -> String {
        let ui = self.gl.ui_stats_line();
        format!(
            "flips {} late {} lost {} direct {}{}{}",
            self.flips,
            self.late_frames,
            self.lost_refreshes,
            self.direct_frames,
            if ui.is_empty() { "" } else { " " },
            ui
        )
    }

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn ui_needs_frame(&mut self) -> io::Result<bool> {
        self.gl.ui_needs_frame()
    }

    fn ui_fd(&self) -> Option<RawFd> {
        self.gl.ui_fd()
    }

    fn query_buffer(&mut self, buffer: &WlBuffer) -> Option<((i32, i32), bool)> {
        self.gl.query_buffer(buffer)
    }

    fn forget_buffer(&mut self, buffer: &WlBuffer) {
        self.gl.forget_buffer(buffer);
        if let Some(d) = self.direct.remove(&buffer.id()) {
            let _ = self.card.destroy_framebuffer(d.fb);
            let _ = self.card.close_buffer(d.gem);
        }
    }

    /// Draws the plan: direct scan-out or composition into the next page (only the tiles the page owes; `frames`
    /// decides), kicks the GPU. The commit is in `present`.
    fn render(&mut self, plan: &Plan, surfaces: &Surfaces) -> io::Result<RenderStats> {
        let now = self.now_ns();
        self.frame_started_ns = now;
        self.frame_slot_ns = self.pacer.plan(now).slot_ns;
        self.prune_direct();
        // A frame still waiting behind the flip in flight is superseded by this one: its page is drawn again (the
        // ring owes it the damage since its own drawing) and it never goes out. Pages: one scanning out, one whose
        // flip is pending, one drawn into (a waiting frame's, or the next).
        if let Some(q) = self.queued.take() {
            if let Source::Page(i) = q.assignment.primary {
                self.frames.redraw(i);
            }
            self.replacing = true;
        }
        let direct = plan.sole_fullscreen().and_then(|item| {
            let fb = self.direct_fb(&item.buffer)?;
            Some(u32::from(fb))
        });
        let (primary, stats) = self.frames.draw(&mut self.gl, plan, surfaces, direct)?;
        if matches!(primary, Source::Direct(_)) {
            self.direct_frames += 1;
        }
        self.last_assignment = Some(Frame {
            assignment: Assignment { primary, overlay: None },
            #[cfg(feature = "ui")]
            ui: self.gl.take_ui_frame(),
        });
        self.frame_ready_ns = self.now_ns();
        Ok(stats)
    }

    fn present(&mut self) -> io::Result<Presented> {
        let at_ms = crate::output::monotonic_ms();
        let replaced = std::mem::take(&mut self.replacing);
        self.seq += 1;
        let Some(a) = self.last_assignment.take() else {
            // Nothing drawn: the planes keep what they show; no commit, no flip; the run is over.
            self.in_run = false;
            return Ok(Presented { seq: self.seq, at_ms, ..Default::default() });
        };
        self.in_run = true;
        // One flip in flight at a time (the kernel refuses a second non-blocking commit, and the page it reads must
        // not be reused): a frame drawn while one is pending waits for it and goes out from `dispatch_events`.
        if self.pending_flip && !self.flip_overdue() {
            self.queued = Some(a);
        } else {
            self.commit_frame(a)?;
        }
        Ok(Presented { seq: self.seq, at_ms, pending: true, replaced })
    }

    fn capture(&mut self) -> io::Result<Image> {
        // The page last composed, through the CPU mapping, once the GPU is done with it.
        self.gl.finish();
        let idx = self.frames.last_page();
        let (w, h, pitch) = (self.width as usize, self.height as usize, self.pages[idx].dumb.pitch() as usize);
        let mapping = self.card.map_dumb_buffer(&mut self.pages[idx].dumb)?;
        let bytes: &[u8] = &mapping;
        let mut rgba = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            let row = &bytes[y * pitch..y * pitch + w * 4];
            for px in row.chunks_exact(4) {
                rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
            }
        }
        Ok(Image { width: self.width, height: self.height, rgba })
    }
}

impl Drop for Kms {
    fn drop(&mut self) {
        for d in self.direct.values() {
            let _ = self.card.destroy_framebuffer(d.fb);
            let _ = self.card.close_buffer(d.gem);
        }
        for p in self.pages.drain(..) {
            let _ = self.card.destroy_framebuffer(p.fb);
            drop(p.fd);
            let _ = self.card.destroy_dumb_buffer(p.dumb);
        }
        let _ = self.card.destroy_property_blob(self.mode_blob);
        let _ = self.card.release_master_lock();
    }
}
