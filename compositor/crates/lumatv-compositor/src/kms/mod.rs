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
pub mod timing;

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
use crate::output::{Image, Output, Presented, RenderStats, monotonic_ns as mono_ns};
use crate::render::gl::{Gl, TargetId};
use crate::scene::Plan;
use crate::surface::Surfaces;
use assign::{Assignment, Source};
use timing::{FlipTiming, Timing};

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
    timing: Timing,
    #[cfg(feature = "ui")]
    ui: Option<lumatv_render::host::FrameCost>,
}

/// The commit whose flip is pending: the frame's timing, a duplicate of its in-fence (the kernel holds its own) to read
/// the GPU's finish from when the flip lands, and the UI's receipt for that frame.
struct InFlight {
    timing: Timing,
    fence: Option<OwnedFd>,
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
    /// The UI ring for the overlay plane (ARGB8888), made when a UI is attached and the overlay takes the format.
    ui_pages: Vec<Page>,
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
    /// The pacer, the refresh period followed from the flips, and the counters they keep.
    pub timing: FlipTiming,
    /// Page-flip timestamps are CLOCK_MONOTONIC when the driver says so (DRM_CAP_TIMESTAMP_MONOTONIC), else
    /// CLOCK_REALTIME and translated on arrival; everything else here is CLOCK_MONOTONIC in ns.
    monotonic_flips: bool,
    last_assignment: Option<Frame>,
    /// What the flip in flight is of: its frame's timing and in-fence, for the pacer when it lands.
    in_flight: Option<InFlight>,
    /// Whether the last tick drew a frame: the run of frames the pacer keeps in step is on while ticks keep
    /// drawing; the first tick that finds nothing to draw ends it and the fixed tick takes over.
    in_run: bool,
    /// The last `warming` answer: a frame is held while a driver buffer warms with no page to stand in from, and the
    /// tick polls for the warm's end instead of waiting for the next fixed tick (up to a refresh on the eighth and
    /// ninth gates: a client's first frame reached the panel a refresh late for it).
    held: bool,
    /// The slot the last paced tick was latched for and how long before its refresh it fired: the latch moves later
    /// by at most `LATCH_STEP_NS` per slot (`next_frame_at`), since a client whose swaps follow the compositor's
    /// buffer releases, one per tick, sees a tick that comes 4 ms later than the last as a 20 ms gap.
    latch: std::cell::Cell<(u64, u64)>,
    /// When the last frame was drawn (CLOCK_MONOTONIC ns), for the `trace: tick` line on a long interval.
    last_draw_ns: u64,
    /// The last drawn frame was an OLED-care beat alone: the screen is at rest sooner after it ([`IDLE_QUIET_CARE_NS`]).
    care_draw: bool,
    tick_lines: u32,
    pub direct_frames: u64,
    pub split_frames: u64,
    /// Flips the loop handled more than 3 ms after they landed (their callbacks went out that late), those over 1 ms,
    /// and the worst.
    flips_reached_late: u64,
    flips_reached_over_1ms: u64,
    reach_worst_ns: u64,
    /// Flips left of the start-up trace: a new buffer prepared starts one (a client's first frames, the time the gates
    /// lose refreshes), and every tick, commit and flip until it runs out gets a `trace:` line.
    trace_left: u32,
    seq: u64,
    /// The OSD's vblank divider (`refresh`): written for 120 Hz, put back at 60 and at exit.
    divider: crate::refresh::Divider,
    /// The refresh asked for (`Config::refresh_hz`), as last set; the measured period is the pacer's.
    refresh_hz: u32,
}

const PAGES: usize = 3;
/// Flips a start-up trace runs for.
const TRACE_FLIPS: u32 = 90;
/// How long after the last flip the idle tick still follows the panel's grid from it: the panel's clock and
/// CLOCK_MONOTONIC drift apart slowly (the pacer corrects a sixteenth of the offset per frame once a run is on, and
/// the stamps sit within 0.05 ms of a refresh), so a few minutes keep the error well inside the latch's slack.
const GRID_MEMORY_NS: u64 = 180_000_000_000;
/// What an idle tick leaves between its frame being ready and the refresh: the pacer's own lead and slack (`FLIP_LEAD_NS`
/// and `SLACK_NS` in `pacer.rs`), so a frame drawn by it can still make the refresh when it costs more than usual.
const IDLE_LEAD_NS: u64 = 6_000_000;
/// The most of the recent frames' typical pipeline the idle tick budgets for: frames drawn on an idle GPU pay its
/// wake-up (10-11 ms against 4.7 in a run on the tenth gate), and a budget of their size put the idle tick right after
/// a refresh, so a commit arriving a millisecond later waited a whole one.
const IDLE_TYPICAL_CAP_NS: u64 = 6_000_000;
/// How much later than the last one a paced tick may be latched per slot. The pacer's latch is the refresh less the
/// slowest recent pipeline and its slack: one slow frame leaving the window moves it 3-4 ms later at once, and the
/// client's swaps (its buffer releases, one per tick) showed that as gaps of 20.7 and 20.9 ms on the tenth gate.
const LATCH_STEP_NS: u64 = 1_000_000;
/// How long nothing may have been drawn or flipped before the output calls the screen at rest (`Output::idle_ok`): a frame owed by something
/// that is not an event (a stand-in, a warm) is drawn within a few refreshes, so a second without one is a still screen.
const IDLE_QUIET_NS: u64 = 1_000_000_000;
/// The same after a frame that was an OLED-care beat alone (`Output::quiet_frame`): it owes nothing, so once it has flipped (a refresh or two) the
/// screen is at rest, and the beat does not hold the tick at the panel's rate for a second.
const IDLE_QUIET_CARE_NS: u64 = 150_000_000;

fn other(msg: impl Into<String>) -> io::Error {
    io::Error::other(msg.into())
}

/// One scan-out page: a dumb buffer with a framebuffer and a GL target over its dma-buf. `alpha` makes it ARGB8888 for
/// the overlay plane, else XRGB8888 for the primary.
fn make_page(card: &Card, gl: &mut Gl, width: u32, height: u32, alpha: bool) -> io::Result<Page> {
    let (fourcc, drm, depth) =
        if alpha { (crate::buffer::fourcc::ARGB8888, DrmFourcc::Argb8888, 32) } else { (crate::buffer::fourcc::XRGB8888, DrmFourcc::Xrgb8888, 24) };
    let dumb = card.create_dumb_buffer((width, height), drm, 32)?;
    let fb = card.add_framebuffer(&dumb, depth, 32)?;
    let fd = card.buffer_to_prime_fd(dumb.handle(), libc::O_CLOEXEC as u32)?;
    let target = gl.add_dmabuf_target(fd.as_raw_fd(), width as i32, height as i32, fourcc, dumb.pitch(), 0)?;
    Ok(Page { dumb, fd, fb, target })
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
            pages.push(make_page(&card, &mut gl, width, height, false).map_err(|e| other(format!("page {i}: {e}")))?);
        }
        // The UI ring for the split (the UI on the overlay over a direct window): only with a UI to draw, and only
        // when the overlay plane lists the format with alpha. Three pages as the primary ring: one scanning out, one
        // whose flip is pending, one drawn into.
        let mut ui_pages = Vec::new();
        let overlay_takes_alpha = overlay.as_ref().is_some_and(|o| o.formats.contains(&(crate::buffer::fourcc::ARGB8888, 0)));
        if gl.has_ui() && overlay_takes_alpha {
            for i in 0..PAGES {
                match make_page(&card, &mut gl, width, height, true) {
                    Ok(p) => ui_pages.push(p),
                    Err(e) => {
                        log::warn!("kms: UI page {i}: {e}; the UI over a direct window composes instead");
                        ui_pages.clear();
                        break;
                    }
                }
            }
        }
        log::info!(
            "kms: split scan-out (UI on the overlay over a direct window) {}",
            if !ui_pages.is_empty() {
                "on"
            } else if !gl.has_ui() {
                "off: no UI attached"
            } else if !overlay_takes_alpha {
                "off: the overlay plane lists no AR24"
            } else {
                "off: no UI pages"
            }
        );
        // The first draw from an imported image and the program's first use are paid here, into page 0 from page 1,
        // not by the first client's first frame.
        gl.set_target(pages[0].target);
        let warmed = gl.warm_up(Some(pages[1].target));
        log::info!("kms: gl warm-up {:.1} ms", warmed.as_secs_f64() * 1000.0);
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
            frames: {
                let f = frames::Frames::new(pages.iter().map(|p| p.target).collect(), width, height);
                if ui_pages.is_empty() { f } else { f.with_ui_pages(ui_pages.iter().map(|p| p.target).collect()) }
            },
            pages,
            ui_pages,
            direct: HashMap::new(),
            refused_modifiers: Vec::new(),
            state: [PlaneState::default(); 2],
            modeset_done: false,
            pending_flip: false,
            committed_ns: 0,
            queued: None,
            replacing: false,
            timing: FlipTiming::new(period_ns),
            monotonic_flips,
            last_assignment: None,
            in_flight: None,
            in_run: false,
            held: false,
            latch: std::cell::Cell::new((0, 0)),
            last_draw_ns: 0,
            care_draw: false,
            tick_lines: 0,
            direct_frames: 0,
            split_frames: 0,
            flips_reached_late: 0,
            flips_reached_over_1ms: 0,
            reach_worst_ns: 0,
            trace_left: 0,
            seq: 0,
            divider: crate::refresh::Divider::new(),
            refresh_hz: crate::refresh::DEFAULT_HZ,
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
        self.in_run && self.timing.pacer.phase_ns().is_some() && self.timing.pacer.flowing(now_ns)
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

    /// Reads the DRM events that have arrived; returns how many page flips landed. Never waits: the card's descriptor
    /// blocks, and the tick asks whether or not one landed, so the read is made only when poll says one is there.
    fn drain_events(&mut self) -> io::Result<u32> {
        let mut p = libc::pollfd { fd: self.card.as_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: one pollfd on the card's open descriptor, no timeout.
        if unsafe { libc::poll(&mut p, 1, 0) } != 1 || p.revents & libc::POLLIN == 0 {
            return Ok(0);
        }
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
        // A flip nothing was waiting for (one taken as overdue) teaches the interval and nothing else.
        let frame = self.in_flight.take().map(|flight| {
            #[cfg(feature = "ui")]
            if let Some(cost) = flight.ui {
                self.gl.ui_presented(presented_ns, &cost);
            }
            (flight.timing, flight.fence.as_ref().and_then(crate::render::sync::signal_ns))
        });
        let landed = self.timing.landed(presented_ns, frame);
        // The frame drawn behind this one was planned a refresh after it: it moves with the timeline (`Timing::shift`).
        if landed.shift_ns != 0
            && let Some(q) = &mut self.queued
        {
            q.timing.shift(landed.shift_ns);
        }
        if self.trace_left > 0 {
            self.trace_left -= 1;
            match frame {
                Some((t, gpu)) => log::info!(
                    "trace: flip at {:.3} (+{:.1} ms to reach): frame drawn at {:.3} for slot {:.3}, late {} lost {}, gpu done {} ms after the draw",
                    presented_ns as f64 / 1e9,
                    mono_ns().saturating_sub(presented_ns) as f64 / 1e6,
                    t.started_ns as f64 / 1e9,
                    t.slot_ns as f64 / 1e9,
                    landed.late,
                    landed.lost,
                    gpu.map_or("?".to_string(), |g| format!("{:.1}", g.saturating_sub(t.started_ns) as f64 / 1e6))
                ),
                None => log::info!("trace: flip at {:.3} of no frame of ours", presented_ns as f64 / 1e9),
            }
        }
        // Consecutive flips are one refresh apart; a stamp that is not (two flips stamped alike, a refresh skipped)
        // is logged while frames follow one another. A skipped refresh is a frame the panel missed only when one was
        // due at it (`Landed::missed`); the gates count `page-flip stamp` lines over 20 ms as such, so a refresh with
        // nothing new to show gets a line of its own.
        if let Some(since) = landed.odd
            && self.timing.odd_stamps <= 200
        {
            let (since, period) = (since as f64 / 1e6, self.timing.period_ns() as f64 / 1e6);
            if landed.missed || since < period {
                log::info!("kms: page-flip stamp {since:.2} ms after the last (one refresh is {period:.2})");
            } else {
                log::info!("kms: page-flip interval {since:.2} ms, nothing due at the refresh between (one refresh is {period:.2})");
            }
        }
        if let Some((old, new)) = landed.rate_change {
            log::info!("kms: the panel refreshes every {:.2} ms now ({:.2} before): the pacer is keyed to it", new as f64 / 1e6, old as f64 / 1e6);
        }
        // The frame callbacks of a landed flip go out right after this returns, so the loop's distance from the
        // flip is the compositor's share of a client's gap: X1's two 20.6-21.0 ms gaps on the sixth gate came with
        // nothing logged above 8 ms, and this says whether a 4 ms delay was here or in the client.
        let reached_ns = mono_ns().saturating_sub(presented_ns);
        self.reach_worst_ns = self.reach_worst_ns.max(reached_ns);
        if reached_ns > 1_000_000 {
            self.flips_reached_over_1ms += 1;
        }
        if reached_ns > 3_000_000 {
            self.flips_reached_late += 1;
            if self.flips_reached_late <= 200 {
                log::info!("kms: flip reached {:.1} ms after it landed", reached_ns as f64 / 1e6);
            }
        }
    }

    /// Commits a drawn frame now: the GL fence of a drawn page (the primary's, or the UI ring's in the split) goes
    /// along as that plane's in-fence, and a duplicate of it stays with the flip for its signal time. The two steps
    /// are timed apart: the fence's export waits for the driver to submit the draw (what a frame costs between
    /// `render:` and the loop's own line), the commit is the kernel's atomic check.
    fn commit_frame(&mut self, frame: Frame) -> io::Result<()> {
        let a = frame.assignment;
        let t0 = Instant::now();
        let drawn = matches!(a.primary, Source::Page(_)) || matches!(a.overlay, Some((Source::UiPage(_), _)));
        let fence = if drawn { self.gl.export_fence() } else { None };
        let kept = fence.as_ref().and_then(|f| f.try_clone().ok());
        let t1 = Instant::now();
        let committed = self.commit(&a, fence);
        let t2 = Instant::now();
        if t2 - t0 > Duration::from_millis(4) {
            log::info!("present: fence export {:.1} ms, atomic commit {:.1} ms", (t1 - t0).as_secs_f64() * 1000.0, (t2 - t1).as_secs_f64() * 1000.0);
        }
        if self.trace_left > 0 {
            log::info!(
                "trace: committed at {:.3}: fence export {:.1} ms, atomic commit {:.1} ms",
                self.now_ns() as f64 / 1e9,
                (t1 - t0).as_secs_f64() * 1000.0,
                (t2 - t1).as_secs_f64() * 1000.0
            );
        }
        committed?;
        self.in_flight = Some(InFlight {
            timing: frame.timing,
            fence: kept,
            #[cfg(feature = "ui")]
            ui: frame.ui,
        });
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
        }
        let full = Rect::new(0, 0, self.width as i32, self.height as i32);
        let primary_fb = match a.primary {
            Source::Page(i) => Some(u32::from(self.pages[i].fb)),
            Source::UiPage(i) => Some(u32::from(self.ui_pages[i].fb)),
            Source::Direct(fb) => Some(fb),
            Source::Off => None,
        };
        let primary_fence = if matches!(a.primary, Source::Page(_)) { in_fence.as_ref().map(|f| f.as_raw_fd()) } else { None };
        let primary_state = PlaneState { fb: primary_fb, dst: full };
        let mut new_state = self.state;
        if primary_state != self.state[0] || !self.modeset_done {
            set_plane(&mut req, &self.primary, self.crtc, primary_fb, full, full, primary_fence);
            new_state[0] = primary_state;
        }
        if let Some(ov) = &self.overlay {
            // The overlay shows the same rectangle of its page as it covers of the output: no scaling (X6 unanswered).
            let (fb, dst, fence) = match a.overlay {
                Some((Source::UiPage(i), r)) => (Some(u32::from(self.ui_pages[i].fb)), r, in_fence.as_ref().map(|f| f.as_raw_fd())),
                Some((Source::Page(i), r)) => (Some(u32::from(self.pages[i].fb)), r, None),
                Some((Source::Direct(fb), r)) => (Some(fb), r, None),
                _ => (None, Rect::default(), None),
            };
            let st = PlaneState { fb, dst };
            if st != self.state[1] || !self.modeset_done {
                set_plane(&mut req, ov, self.crtc, fb, dst, dst, fence);
                new_state[1] = st;
            }
        }
        let _ = self.crtc_out_fence;
        // The session's first commit carries the mode. Made blocking, it held the loop until the GPU had the first
        // frame and the next vblank came: 21.8 ms on the eighth gate's trial, 13.3-16.1 ms on the gates before
        // (`present: ... atomic commit`), the trial's one loop pass over 20 ms. Non-blocking it returns at once and
        // the page-flip event says when it is on; a driver that refuses a non-blocking modeset gets the blocking one.
        let first = !self.modeset_done;
        let retry = first.then(|| req.clone());
        let committed = match (retry, self.card.atomic_commit(flags, req)) {
            (Some(req), Err(e)) => {
                log::warn!("kms: the first commit (with the mode) refused non-blocking: {e}; committing it blocking");
                flags.remove(AtomicCommitFlags::NONBLOCK);
                self.card.atomic_commit(flags, req)
            }
            (_, r) => r,
        };
        match committed {
            Ok(()) => {
                if first {
                    log::info!(
                        "kms: the mode went out with the first frame ({})",
                        if flags.contains(AtomicCommitFlags::NONBLOCK) { "non-blocking" } else { "blocking" }
                    );
                }
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

/// Adds every property of one plane to the request: `dst` in output pixels, `src` in the framebuffer's (16.16 fixed
/// point on the wire).
fn set_plane(req: &mut AtomicModeReq, p: &PlaneInfo, crtc: crtc::Handle, fb: Option<u32>, dst: Rect, src: Rect, in_fence: Option<i32>) {
    let h = p.handle;
    match fb {
        Some(fb) => {
            req.add_property(h, p.props.fb_id, property::Value::Framebuffer(Some(framebuffer::Handle::from(std::num::NonZeroU32::new(fb).unwrap()))));
            req.add_property(h, p.props.crtc_id, property::Value::CRTC(Some(crtc)));
            req.add_property(h, p.props.crtc_x, property::Value::SignedRange(dst.x as i64));
            req.add_property(h, p.props.crtc_y, property::Value::SignedRange(dst.y as i64));
            req.add_property(h, p.props.crtc_w, property::Value::UnsignedRange(dst.w.max(0) as u64));
            req.add_property(h, p.props.crtc_h, property::Value::UnsignedRange(dst.h.max(0) as u64));
            req.add_property(h, p.props.src_x, property::Value::UnsignedRange((src.x.max(0) as u64) << 16));
            req.add_property(h, p.props.src_y, property::Value::UnsignedRange((src.y.max(0) as u64) << 16));
            req.add_property(h, p.props.src_w, property::Value::UnsignedRange((src.w.max(0) as u64) << 16));
            req.add_property(h, p.props.src_h, property::Value::UnsignedRange((src.h.max(0) as u64) << 16));
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
    /// drawn frame) and re-arming at it spun the loop at 2000 ticks/s in the first TV run. A held frame (a driver
    /// buffer warming) is tried again in half a millisecond, run or no run.
    fn next_frame_at(&self, now: Instant) -> Option<Instant> {
        if self.held {
            return Some(now + Duration::from_micros(500));
        }
        let now_ns = self.now_ns();
        if !self.flowing(now_ns) {
            return None;
        }
        let latch = self.timing.pacer.latch_at(now_ns);
        // The latch as a lead before the refresh it aims at, moved later by at most a step per slot; a call for the
        // same slot keeps the lead it settled on.
        let slot = self.timing.pacer.next_slot_ns();
        let latch = match self.timing.pacer.phase_ns() {
            Some(phase) => {
                let display = (slot as i64 + phase).max(0) as u64;
                let mut lead = display.saturating_sub(latch);
                let (last_slot, last_lead) = self.latch.get();
                if last_slot == slot {
                    lead = last_lead;
                } else if last_slot != 0 && lead + LATCH_STEP_NS < last_lead {
                    lead = last_lead - LATCH_STEP_NS;
                }
                self.latch.set((slot, lead));
                display.saturating_sub(lead).max(now_ns)
            }
            None => latch,
        };
        if self.trace_left > 0 {
            log::info!(
                "trace: next tick at {:.3} (latch, {:.1} ms before the refresh at {:.3})",
                latch as f64 / 1e9,
                self.latch.get().1 as f64 / 1e6,
                (slot as i64 + self.timing.pacer.phase_ns().unwrap_or(0)) as f64 / 1e9
            );
        }
        Some(now + Duration::from_nanos(latch.saturating_sub(now_ns)))
    }

    fn last_flip_ns(&self) -> Option<u64> {
        Some(self.timing.last_flip_ns()).filter(|&t| t != 0)
    }

    /// The fixed tick on the panel's refresh grid while no run is on: the latch of the next refresh, from the last
    /// flip's stamp and whole refreshes since. The fixed tick's own phase is whatever it was last armed with: on the
    /// tenth gate it drew Home's frame 5 ms before a refresh and X1's second frame queued behind it, so X1's second
    /// and third frames landed a refresh apart from where they should have and the client's cadence slipped with them.
    fn idle_tick_at(&self, now: Instant) -> Option<Instant> {
        let now_ns = self.now_ns();
        let budget = self.timing.pacer.forecast().typical_ns.min(IDLE_TYPICAL_CAP_NS) + IDLE_LEAD_NS;
        let latch = self.timing.grid_latch_ns(now_ns, budget, GRID_MEMORY_NS)?;
        if self.trace_left > 0 {
            log::info!("trace: idle tick at {:.3} on the grid ({:.1} ms before the refresh)", latch as f64 / 1e9, budget as f64 / 1e6);
        }
        Some(now + Duration::from_nanos(latch.saturating_sub(now_ns)))
    }

    /// At rest: no flip pending, no frame queued or drawn and waiting, no driver buffer warming, no stand-in owed, and nothing drawn or flipped
    /// for [`IDLE_QUIET_NS`]. What the loop's tick does then is housekeeping (`ticker`).
    fn idle_ok(&self) -> bool {
        !self.held
            && !self.pending_flip
            && self.queued.is_none()
            && self.last_assignment.is_none()
            && !self.gl.standin_due()
            && self.now_ns().saturating_sub(self.last_draw_ns.max(self.timing.last_flip_ns()))
                >= if self.care_draw { IDLE_QUIET_CARE_NS } else { IDLE_QUIET_NS }
    }

    fn quiet_frame(&mut self) {
        self.care_draw = true;
    }

    /// `idle_tick_at` for the slowed tick: the first refresh's latch on the panel's grid that is at least `wait` away.
    fn idle_tick_after(&self, now: Instant, wait: Duration) -> Option<Instant> {
        let now_ns = self.now_ns();
        let budget = self.timing.pacer.forecast().typical_ns.min(IDLE_TYPICAL_CAP_NS) + IDLE_LEAD_NS;
        let latch = self.timing.grid_latch_ns(now_ns + wait.as_nanos() as u64, budget, GRID_MEMORY_NS)?;
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
        if landed > 0 {
            // The GPU is done with the frame that landed, the last that could have read a retired texture. A queued
            // frame went out just above, so the driver's unmapping never sits between a draw and its commit; when a
            // tick took the flip, its frame starts after the frees (`BURY_BUDGET` at most).
            self.gl.bury();
            self.prune_direct();
        }
        Ok(landed)
    }

    /// A commit onto a still screen is drawn at once: nothing is in flight to wait for and no run to keep in
    /// step with, so the tick would only delay it by up to a refresh.
    fn draws_on_damage(&self, _now: Instant) -> bool {
        !self.pending_flip && self.queued.is_none() && self.last_assignment.is_none() && !self.flowing(self.now_ns())
    }

    /// The divider first, so a kernel that refuses the write leaves the pacer where it was; then the pacer keyed to the nominal
    /// period by order (the measured path would not take 60 after 120: a doubled interval is a skipped refresh to it) and the
    /// latch and run state forgotten, so the first flips at the new rate start a run of their own.
    fn set_refresh(&mut self, hz: u32) -> io::Result<u32> {
        if !crate::refresh::supported(hz) {
            return Err(other(format!("refresh {hz}: 60 or 120")));
        }
        let divider = self.divider.set(hz)?;
        let period_ns = crate::refresh::period_ns(hz);
        self.timing.rekey(period_ns);
        self.latch.set((0, 0));
        self.in_run = false;
        self.refresh_hz = hz;
        log::info!("kms: refresh {hz} Hz: {} = {divider}, the pacer keyed to {:.2} ms", self.divider.path().display(), period_ns as f64 / 1e6);
        Ok(hz)
    }

    fn refresh_report(&self) -> Option<serde_json::Value> {
        let mut r = self.timing.measure.report(self.timing.period_ns());
        r["hz"] = self.refresh_hz.into();
        r["measured_period_ms"] = self.timing.mean_period_ns().map(|p| p / 1e6).into();
        r["divider"] = self.divider.report();
        Some(r)
    }

    fn stats_line(&self) -> String {
        let ui = self.gl.ui_stats_line();
        let t = &self.timing;
        format!(
            "flips {} late {} lost {} reached late {} (over 1 ms {}, worst {:.1} ms) direct {} split {} odd stamps {} ({} missed) refresh {:.2} ms (measured {}, changed {}){}{}",
            t.flips,
            t.late_frames,
            t.lost_refreshes,
            self.flips_reached_late,
            self.flips_reached_over_1ms,
            self.reach_worst_ns as f64 / 1e6,
            self.direct_frames,
            self.split_frames,
            t.odd_stamps,
            t.missed_stamps,
            t.period_ns() as f64 / 1e6,
            t.mean_period_ns().map_or("-".to_string(), |p| format!("{:.4}", p / 1e6)),
            t.rate_changes,
            if ui.is_empty() { "" } else { " " },
            ui
        )
    }

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The system UI's requests, and a frame owed to a surface a frame stood in for once its buffer is warm.
    fn ui_needs_frame(&mut self) -> io::Result<bool> {
        Ok(self.gl.ui_needs_frame()? || self.gl.standin_due())
    }

    fn ui_fd(&self) -> Option<RawFd> {
        self.gl.ui_fd()
    }

    fn query_buffer(&mut self, buffer: &WlBuffer) -> Option<((i32, i32), bool)> {
        self.gl.query_buffer(buffer)
    }

    fn prepare_buffer(&mut self, buffer: &WlBuffer) -> bool {
        let new = self.gl.prepare(buffer);
        if new {
            self.trace_left = TRACE_FLIPS;
        }
        if self.trace_left > 0 {
            log::info!(
                "trace: commit of buffer {} at {:.3}{}",
                buffer.id().protocol_id(),
                self.now_ns() as f64 / 1e9,
                if new { " (new: warming)" } else { "" }
            );
        }
        new
    }

    fn warming(&mut self) -> bool {
        self.held = self.gl.warming();
        if self.held && self.trace_left > 0 {
            log::info!("trace: tick held at {:.3}", self.now_ns() as f64 / 1e9);
        }
        self.held
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
        // Frees left over from the last flip's budget: on a still screen no flip is coming to carry them, nothing
        // is in flight, and this frame is unpaced (it draws on damage), so they cost no paced frame here.
        if !self.pending_flip && !self.flowing(now) {
            self.gl.bury();
        }
        // A drawing tick well over a refresh after the last while frames follow one another: the client's releases
        // (its swaps) stretch with it. Logged while the run is paced, 200 lines at most.
        if self.last_draw_ns != 0 && self.flowing(now) && now - self.last_draw_ns > 18_500_000 && self.tick_lines < 200 {
            self.tick_lines += 1;
            log::info!(
                "trace: tick {:.1} ms after the last drawing tick (latch lead {:.1} ms)",
                (now - self.last_draw_ns) as f64 / 1e6,
                self.latch.get().1 as f64 / 1e6
            );
        }
        self.last_draw_ns = now;
        self.care_draw = false;
        let slot_ns = self.timing.pacer.plan(now).slot_ns;
        // A frame still waiting behind the flip in flight is superseded by this one: its page is drawn again (the
        // ring owes it the damage since its own drawing) and it never goes out. Pages: one scanning out, one whose
        // flip is pending, one drawn into (a waiting frame's, or the next).
        // A superseded frame's content was due as long as it waited: the oldest of the two is this frame's (a frame of
        // no new commit counts as due since the last flip).
        let mut content_ns = plan.content_ns;
        if let Some(q) = self.queued.take() {
            self.frames.redraw(q.assignment.primary);
            if let Some((s, _)) = q.assignment.overlay {
                self.frames.redraw(s);
            }
            self.replacing = true;
            content_ns = match (content_ns, q.timing.content_ns) {
                (Some(a), Some(b)) => Some(a.min(b)),
                _ => None,
            };
        }
        let direct = plan.sole_fullscreen().and_then(|item| {
            let fb = self.direct_fb(&item.buffer)?;
            Some(u32::from(fb))
        });
        let (assignment, stats) = self.frames.draw(&mut self.gl, plan, surfaces, direct)?;
        if matches!(assignment.primary, Source::Direct(_)) {
            self.direct_frames += 1;
        }
        if assignment.overlay.is_some() {
            if self.split_frames == 0 {
                log::info!("kms: first split frame: the window stays on the primary plane, the UI goes to the overlay");
            }
            self.split_frames += 1;
        }
        if self.trace_left > 0 {
            log::info!(
                "trace: drew at {:.3} for slot {:.3} (on the panel at {}): items {}, stood in {}, render {} us",
                now as f64 / 1e9,
                slot_ns as f64 / 1e9,
                self.timing.pacer.phase_ns().map_or("?".to_string(), |p| format!("{:.3}", (slot_ns as i64 + p) as f64 / 1e9)),
                stats.items,
                stats.stood_in,
                stats.render_us
            );
        }
        self.last_assignment = Some(Frame {
            assignment,
            timing: Timing { slot_ns, started_ns: now, ready_ns: self.now_ns(), content_ns },
            #[cfg(feature = "ui")]
            ui: self.gl.take_ui_frame(),
        });
        Ok(stats)
    }

    fn present(&mut self) -> io::Result<Presented> {
        let at_ms = crate::output::monotonic_ms();
        let replaced = std::mem::take(&mut self.replacing);
        self.seq += 1;
        let Some(a) = self.last_assignment.take() else {
            // Nothing drawn: the planes keep what they show; no commit, no flip; the run is over.
            if self.trace_left > 0 {
                log::info!("trace: tick at {:.3} drew nothing", self.now_ns() as f64 / 1e9);
            }
            self.in_run = false;
            return Ok(Presented { seq: self.seq, at_ms, ..Default::default() });
        };
        self.in_run = true;
        // One flip in flight at a time (the kernel refuses a second non-blocking commit, and the page it reads must
        // not be reused): a frame drawn while one is pending waits for it and goes out from `dispatch_events`.
        if self.pending_flip && !self.flip_overdue() {
            if self.trace_left > 0 {
                log::info!("trace: frame queued at {:.3} behind the flip in flight", self.now_ns() as f64 / 1e9);
            }
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
        for p in self.pages.drain(..).chain(self.ui_pages.drain(..)) {
            let _ = self.card.destroy_framebuffer(p.fb);
            drop(p.fd);
            let _ = self.card.destroy_dumb_buffer(p.dumb);
        }
        let _ = self.card.destroy_property_blob(self.mode_blob);
        let _ = self.card.release_master_lock();
    }
}
