//! Direct scan-out of a client's full-screen buffer (`docs/research/scanout-research.md`): the primary plane shows the window's own buffer
//! while it is the whole screen, and the compositor's copy of it into a page (4.1 ms of GPU a frame on the set) is not made. Off unless
//! `--scanout direct` (or `ctl scanout direct`).
//!
//! The plane is one real graphics layer (the C4's overlay plane is the same OSD hardware: the driver ignores plane 34's enable while the
//! primary is on, `docs/research/scanout-research.md` § Layers), so the rule is: direct while one window that the plane can show covers the
//! output alone; composed the frame anything else needs the screen (a system layer, the pointer, a transition, a dim, a video hole, a
//! capture) and direct again the frame it leaves. Every frame the plane cannot take is composed by the existing GL path in the same frame,
//! with the reason counted and the change of state in the journal.
//!
//! What a buffer must be ([`decide`]): a single plane, offset 0, one of the four 32-bit formats with a (format, modifier) the primary plane's
//! `IN_FORMATS` lists, the size of the mode, the pitch the OSD is configured for (`width * 4`), big enough, upright, and opaque. A
//! successful `ADDFB2` is not the test: the driver accepts any non-linear modifier as "AFBC" and decodes it as its own 32x8 layout, so the
//! client's 16x16 `...151` buffers are refused here, by the list. Contiguity is the kernel's: the PRIME import into lg1k fails on a buffer
//! made of scattered pages, and that failure is the refusal. The descriptor of a driver-served buffer comes from the protocol tap ([`tap`],
//! [`meta`]); the driver still creates and owns the resource, and the GL path imports it as before.
//!
//! A buffer the display reads is never released to its client, and its framebuffer is never destroyed, until a later flip has replaced it
//! ([`Scanout::holds`], `State::release_buffer`). The client's rendering is waited for the way composition waits for it: the GL side samples
//! the buffer (one texel) and the commit carries that job's fence as the plane's `IN_FENCE_FD` ([`Device::acquire`]). The explicit-sync
//! global LG's libmali knows is not offered, so what that job waits on is the dma-buf's implicit fences.

pub mod meta;
pub mod probe;
pub mod tap;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::sync::Arc;

use serde_json::{Value, json};
use wayland_backend::server::ObjectId;
use wayland_server::Resource;
use wayland_server::protocol::wl_buffer::WlBuffer;

use crate::buffer::{BufferKindRef, DmabufBuffer, fourcc, kind_of, modifier};
pub use crate::scene::NotSole;
use crate::scene::Plan;

/// The 32-bit formats the primary plane can show (`IN_FORMATS` of plane 32 on the set).
pub const FORMATS: [u32; 4] = [fourcc::ARGB8888, fourcc::XRGB8888, fourcc::ABGR8888, fourcc::XBGR8888];

/// How many distinct refusal texts the journal carries (each once); the counters have no limit.
const LOGGED_REFUSALS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Off,
    Direct,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "off" => Some(Mode::Off),
            "direct" => Some(Mode::Direct),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Direct => "direct",
        }
    }
}

/// How the client's rendering is waited for before the display reads its buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Acquire {
    /// The GL side samples the buffer and the commit waits for that job (the dma-buf's implicit fences): what composition does.
    Sample,
    /// No wait: the commit is trusted to come after the client's last pixel. For a measurement; a frame may be shown half drawn.
    Trust,
}

impl Acquire {
    pub fn parse(s: &str) -> Option<Acquire> {
        match s {
            "sample" => Some(Acquire::Sample),
            "none" => Some(Acquire::Trust),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Acquire::Sample => "sample",
            Acquire::Trust => "none",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    pub mode: Mode,
    pub acquire: Acquire,
    /// Modifiers tried although the plane does not list them for the format (`--scanout-trust`): the experiment that shows whether the
    /// OSD decodes a client's layout. Never set in production.
    pub trust: Vec<u64>,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { mode: Mode::Off, acquire: Acquire::Sample, trust: Vec::new() }
    }
}

/// Where a buffer's descriptor came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Our own `zwp_linux_dmabuf_v1` or `mali_buffer_sharing`: the descriptor is the buffer.
    Own,
    /// The driver's `mali_buffer_sharing`: the descriptor the tap copied out of the client's request.
    Driver,
}

/// What [`decide`] is told about a buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub origin: Origin,
    pub format: u32,
    pub modifier: u64,
    pub width: i32,
    pub height: i32,
    pub planes: usize,
    pub offset: u32,
    pub stride: u32,
    /// The dma-buf's size in bytes, when the fd said.
    pub bytes: Option<u64>,
    /// Row 0 is the top of the picture (what the plane reads, and what the GL path assumes when it does not flip).
    pub upright: bool,
    /// Never set by a descriptor today (neither protocol says); the field is the place a protected allocation would be refused.
    pub protected: bool,
}

impl Candidate {
    /// The format has no alpha, so the picture is opaque whatever the surface declared.
    pub fn opaque_format(&self) -> bool {
        matches!(self.format, fourcc::XRGB8888 | fourcc::XBGR8888)
    }

    /// A short text for the journal: `1920x1080 XB24 linear`.
    pub fn text(&self) -> String {
        format!("{}x{} {} {}", self.width, self.height, fourcc::name(self.format), modifier_name(self.modifier))
    }
}

/// A modifier in words where the set has one, else its hex.
pub fn modifier_name(m: u64) -> String {
    match m {
        modifier::LINEAR => "linear".into(),
        modifier::AFBC_16X16_YTR_SPARSE_TILED => format!("AFBC 16x16 YTR sparse tiled (0x{m:016x})"),
        modifier::AFBC_32X8_SPLIT_SPARSE => format!("AFBC 32x8 split sparse (0x{m:016x})"),
        _ => format!("0x{m:016x}"),
    }
}

/// Why a frame was composed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The switch is off.
    Disabled,
    /// The window side: the plan is not one picture the plane can show ([`NotSole`]).
    Plan(NotSole),
    /// A screenshot or recording is armed: every frame is composed so that the capture finds the screen in a page.
    Capture,
    /// The mode is not set yet: the session's first commit carries it, with a composed page, so that no plane is asked before the display is up.
    NoMode,
    /// An shm buffer: there is no dma-buf to show.
    Shm,
    /// A driver buffer the tap never saw (made before it was installed, or its request unreadable).
    NoDescriptor,
    /// More than one dma-buf plane.
    Planes(usize),
    /// A nonzero offset into the dma-buf.
    Offset(u32),
    /// Rows are bottom-up: the GL path flips it, the plane cannot.
    Upside,
    /// A protected allocation.
    Protected,
    /// A format the plane does not list.
    Format(u32),
    /// A modifier the plane does not list for the format.
    Modifier(u32, u64),
    /// Not the size of the mode.
    Size(i32, i32),
    /// A pitch other than the OSD's (`width * 4`).
    Stride(u32, u32),
    /// The dma-buf is smaller than its rows need.
    TooSmall(u64, u64),
    /// The PRIME import into the display driver failed (errno): scattered pages are `EINVAL`.
    Import(i32),
    /// `ADDFB2` refused the layout (errno).
    AddFb(i32),
    /// The driver refused the plane state: its test-only commit when the buffer was imported, or the commit of a frame that showed it (errno).
    Test(i32),
    /// The GL side could not make the acquire barrier for the buffer.
    Acquire,
    /// The frame was drawn with a layer over the window (the UI's) that the plane cannot take.
    UiLayer,
}

impl Refusal {
    /// The counter's name.
    pub fn key(&self) -> &'static str {
        match self {
            Refusal::Disabled => "off",
            Refusal::Plan(n) => n.key(),
            Refusal::Capture => "capture",
            Refusal::NoMode => "no-mode",
            Refusal::Shm => "shm",
            Refusal::NoDescriptor => "no-descriptor",
            Refusal::Planes(_) => "planes",
            Refusal::Offset(_) => "offset",
            Refusal::Upside => "upside-down",
            Refusal::Protected => "protected",
            Refusal::Format(_) => "format",
            Refusal::Modifier(..) => "modifier",
            Refusal::Size(..) => "size",
            Refusal::Stride(..) => "stride",
            Refusal::TooSmall(..) => "too-small",
            Refusal::Import(_) => "import",
            Refusal::AddFb(_) => "addfb",
            Refusal::Test(_) => "test-commit",
            Refusal::Acquire => "acquire",
            Refusal::UiLayer => "ui-layer",
        }
    }

    /// What the journal says.
    pub fn text(&self) -> String {
        let errno = |e: i32| std::io::Error::from_raw_os_error(e).to_string();
        match self {
            Refusal::Disabled => "direct scan-out is off".into(),
            Refusal::Plan(n) => n.text(),
            Refusal::Capture => "a capture is armed".into(),
            Refusal::NoMode => "the mode is not set yet".into(),
            Refusal::Shm => "an shm buffer".into(),
            Refusal::NoDescriptor => "a driver buffer the protocol tap did not see".into(),
            Refusal::Planes(n) => format!("{n} dma-buf planes"),
            Refusal::Offset(o) => format!("offset {o} into the dma-buf"),
            Refusal::Upside => "rows bottom-up".into(),
            Refusal::Protected => "a protected allocation".into(),
            Refusal::Format(f) => format!("format {} is not on the plane's list", fourcc::name(*f)),
            Refusal::Modifier(f, m) => format!("{} with {} is not on the plane's list", fourcc::name(*f), modifier_name(*m)),
            Refusal::Size(w, h) => format!("{w}x{h} is not the mode's size"),
            Refusal::Stride(s, want) => format!("pitch {s}, the OSD takes {want}"),
            Refusal::TooSmall(have, need) => format!("the dma-buf is {have} bytes, its rows need {need}"),
            Refusal::Import(e) => format!("the display driver would not import the dma-buf: {} (a buffer of scattered pages)", errno(*e)),
            Refusal::AddFb(e) => format!("ADDFB2 refused the layout: {}", errno(*e)),
            Refusal::Test(e) => format!("the driver refused the plane state: {}", errno(*e)),
            Refusal::Acquire => "the GL side could not wait for the client's rendering".into(),
            Refusal::UiLayer => "the system UI has a layer over the window".into(),
        }
    }

    /// Whether this is about the buffer (and stays true for it) rather than about what else is on the screen this frame.
    pub fn of_buffer(&self) -> bool {
        !matches!(self, Refusal::Disabled | Refusal::Plan(_) | Refusal::Capture | Refusal::NoMode | Refusal::UiLayer | Refusal::Acquire)
    }
}

/// The buffer-level rules, in the order they are told: the first that fails is the reason. Opacity and the plan are not here (they change
/// frame to frame; [`Scanout::choose`] asks them).
pub fn decide(c: &Candidate, device: &dyn Device, policy: &Policy) -> Result<(), Refusal> {
    if c.protected {
        return Err(Refusal::Protected);
    }
    if !c.upright {
        return Err(Refusal::Upside);
    }
    if c.planes != 1 {
        return Err(Refusal::Planes(c.planes));
    }
    if c.offset != 0 {
        return Err(Refusal::Offset(c.offset));
    }
    if !FORMATS.contains(&c.format) || !device.lists_format(c.format) {
        return Err(Refusal::Format(c.format));
    }
    if !device.lists(c.format, c.modifier) && !policy.trust.contains(&c.modifier) {
        return Err(Refusal::Modifier(c.format, c.modifier));
    }
    let (w, h) = device.mode();
    if (c.width, c.height) != (w, h) {
        return Err(Refusal::Size(c.width, c.height));
    }
    let want = c.width as u32 * 4;
    if c.stride != want {
        return Err(Refusal::Stride(c.stride, want));
    }
    if let Some(bytes) = c.bytes {
        let need = u64::from(c.stride) * c.height as u64;
        if bytes < need {
            return Err(Refusal::TooSmall(bytes, need));
        }
    }
    Ok(())
}

/// The memory behind a candidate, kept alive while it is imported.
pub enum Backing {
    Dmabuf(Arc<DmabufBuffer>),
    Retained(Arc<meta::Descriptor>),
}

impl Backing {
    pub fn fd(&self) -> BorrowedFd<'_> {
        match self {
            Backing::Dmabuf(d) => d.planes[0].fd.as_fd(),
            Backing::Retained(d) => d.fd.as_fd(),
        }
    }
}

/// A buffer as the decision sees it, or why there is none. `retained` is asked only for a driver buffer, and `driver_upright` is the rows'
/// direction the driver gave for it (`EGL_WAYLAND_Y_INVERTED_WL`, the item's `y_invert`).
pub fn describe(
    kind: BufferKindRef<'_>,
    retained: impl FnOnce() -> Option<Arc<meta::Descriptor>>,
    driver_upright: bool,
) -> Result<(Candidate, Backing), Refusal> {
    match kind {
        BufferKindRef::Shm(_) => Err(Refusal::Shm),
        BufferKindRef::Dmabuf(d) => {
            let Some(plane) = d.planes.first() else { return Err(Refusal::Planes(0)) };
            let c = Candidate {
                origin: Origin::Own,
                format: d.format,
                modifier: d.modifier,
                width: d.width,
                height: d.height,
                planes: d.planes.len(),
                offset: plane.offset,
                stride: plane.stride,
                bytes: meta::fd_bytes(plane.fd.as_raw_fd()),
                upright: d.y_invert,
                protected: false,
            };
            Ok((c, Backing::Dmabuf(d.clone())))
        }
        BufferKindRef::External => {
            let Some(d) = retained() else { return Err(Refusal::NoDescriptor) };
            let c = Candidate {
                origin: Origin::Driver,
                format: d.format,
                modifier: d.modifier,
                width: d.width,
                height: d.height,
                planes: 1,
                offset: 0,
                stride: d.stride,
                bytes: d.bytes(),
                upright: driver_upright,
                protected: false,
            };
            Ok((c, Backing::Retained(d)))
        }
    }
}

/// A framebuffer made for a buffer: the DRM object and the GEM handle behind it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Imported {
    pub fb: u32,
    pub gem: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Prime,
    AddFb,
    Test,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportError {
    pub stage: Stage,
    pub errno: i32,
}

impl From<ImportError> for Refusal {
    fn from(e: ImportError) -> Refusal {
        match e.stage {
            Stage::Prime => Refusal::Import(e.errno),
            Stage::AddFb => Refusal::AddFb(e.errno),
            Stage::Test => Refusal::Test(e.errno),
        }
    }
}

/// The display side of the decision: what the primary plane lists, and the calls that make a buffer showable. The KMS output implements it
/// over the DRM device; the tests over nothing.
pub trait Device {
    /// The output's size in pixels (the mode).
    fn mode(&self) -> (i32, i32);
    /// Whether the mode is set (the session's first commit went out): before that, no plane state can be asked about.
    fn live(&self) -> bool;
    /// Whether the primary plane lists `format` with `modifier` (its `IN_FORMATS`).
    fn lists(&self, format: u32, modifier: u64) -> bool;
    /// Whether it lists `format` with any modifier.
    fn lists_format(&self, format: u32) -> bool;
    /// PRIME-imports the dma-buf, makes a framebuffer of it and asks the driver whether the plane would take it (a test-only commit; the mode
    /// is set by then, [`Device::live`]). Nothing is left behind on failure.
    fn import(&mut self, c: &Candidate, fd: BorrowedFd<'_>) -> Result<Imported, ImportError>;
    /// Destroys the framebuffer and the GEM handle.
    fn destroy(&mut self, imported: Imported);
    /// The GL side samples the buffer, so the fence of the commit is behind the client's rendering. `false` when it cannot.
    fn acquire(&mut self, buffer: &WlBuffer) -> bool;
}

/// The frame's plane source.
#[derive(Clone, Debug)]
pub struct Choice {
    pub fb: u32,
    pub id: ObjectId,
    pub buffer: WlBuffer,
    /// The commit carries the acquire job's fence.
    pub acquired: bool,
}

/// What a frame put on the primary plane, as the display holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Slot {
    #[default]
    Empty,
    Composed,
    Direct(ObjectId),
}

impl Slot {
    fn is(&self, id: &ObjectId) -> bool {
        matches!(self, Slot::Direct(d) if d == id)
    }

    fn direct(&self) -> bool {
        matches!(self, Slot::Direct(_))
    }
}

/// Which buffers the display reads or is about to: the frame on the screen, the one whose flip is pending, the one drawn and waiting to be
/// committed. A buffer in any of them is not the client's to draw into, and its framebuffer is not to be destroyed.
#[derive(Debug, Default)]
struct Holds {
    shown: Slot,
    pending: Slot,
    waiting: Slot,
}

impl Holds {
    fn has(&self, id: &ObjectId) -> bool {
        self.shown.is(id) || self.pending.is(id) || self.waiting.is(id)
    }
}

struct Fb {
    imported: Imported,
    buffer: WlBuffer,
    candidate: Candidate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Shown {
    Direct,
    Composed(Option<Refusal>),
}

/// Counters since the start.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub direct: u64,
    pub composed: u64,
    pub refused: BTreeMap<&'static str, u64>,
    pub transitions: u64,
    pub imports: u64,
    pub import_failures: u64,
}

pub struct Scanout {
    policy: Policy,
    meta: meta::Registry,
    stats: Stats,
    fbs: HashMap<ObjectId, Fb>,
    refused: HashMap<ObjectId, (WlBuffer, Refusal)>,
    zombies: Vec<(ObjectId, Imported)>,
    holds: Holds,
    state: Shown,
    last_direct: Option<(Candidate, u32)>,
    last_refusal: Option<Refusal>,
    logged: HashSet<String>,
    journal: Vec<String>,
}

impl Scanout {
    pub fn new(policy: Policy, meta: meta::Registry) -> Scanout {
        Scanout {
            policy,
            meta,
            stats: Stats::default(),
            fbs: HashMap::new(),
            refused: HashMap::new(),
            zombies: Vec::new(),
            holds: Holds::default(),
            state: Shown::Composed(None),
            last_direct: None,
            last_refusal: None,
            logged: HashSet::new(),
            journal: Vec::new(),
        }
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    pub fn mode(&self) -> Mode {
        self.policy.mode
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if mode != self.policy.mode {
            self.policy.mode = mode;
            self.journal.push(format!("scanout: {} (switch)", mode.name()));
            // What was shown under the old mode is not a transition of the new one.
            self.state = Shown::Composed(None);
        }
    }

    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    /// Decides one frame: the buffer to scan out, or why the frame is composed. Nothing is counted here ([`Scanout::settle`] does, once the
    /// frame's assignment is known). A buffer is imported the first time it is a candidate and its framebuffer kept; a refusal that is about
    /// the buffer is remembered for it, so the import is not tried again every frame.
    pub fn choose(&mut self, device: &mut dyn Device, plan: &Plan, capture: bool) -> Result<Choice, Refusal> {
        if self.policy.mode == Mode::Off {
            return Err(Refusal::Disabled);
        }
        if capture {
            return Err(Refusal::Capture);
        }
        let item = plan.sole_candidate().map_err(Refusal::Plan)?;
        if !device.live() {
            return Err(Refusal::NoMode);
        }
        let id = item.buffer.id();
        if let Some((_, r)) = self.refused.get(&id) {
            return Err(r.clone());
        }
        let known: Option<(Candidate, u32)> = self.fbs.get(&id).map(|f| (f.candidate.clone(), f.imported.fb));
        let (candidate, backing) = match &known {
            Some((c, _)) => (c.clone(), None),
            None => {
                let described = describe(kind_of(&item.buffer), || self.meta.lookup_buffer(&item.buffer), item.y_invert);
                match described.and_then(|(c, b)| decide(&c, device, &self.policy).map(|()| (c, Some(b)))) {
                    Ok(ok) => ok,
                    Err(r) => return Err(self.remember(&item.buffer, r)),
                }
            }
        };
        if !item.opaque && !candidate.opaque_format() {
            return Err(Refusal::Plan(NotSole::Translucent));
        }
        let fb = match (known, backing) {
            (Some((_, fb)), _) => fb,
            (None, Some(backing)) => match device.import(&candidate, backing.fd()) {
                Ok(imported) => {
                    self.stats.imports += 1;
                    let fb = imported.fb;
                    self.journal.push(format!("scanout: fb {fb} for buffer {} ({})", id.protocol_id(), candidate.text()));
                    self.fbs.insert(id.clone(), Fb { imported, buffer: item.buffer.clone(), candidate: candidate.clone() });
                    fb
                }
                Err(e) => {
                    self.stats.import_failures += 1;
                    return Err(self.remember(&item.buffer, e.into()));
                }
            },
            (None, None) => unreachable!("a buffer without a framebuffer was described above"),
        };
        let acquired = match self.policy.acquire {
            Acquire::Sample => {
                if !device.acquire(&item.buffer) {
                    return Err(Refusal::Acquire);
                }
                true
            }
            Acquire::Trust => false,
        };
        self.last_direct = Some((candidate, fb));
        Ok(Choice { fb, id, buffer: item.buffer.clone(), acquired })
    }

    /// Remembers a refusal that is about the buffer.
    fn remember(&mut self, buffer: &WlBuffer, r: Refusal) -> Refusal {
        if r.of_buffer() {
            self.refused.insert(buffer.id(), (buffer.clone(), r.clone()));
        }
        r
    }

    /// The driver refused the commit of a frame that showed framebuffer `fb` (its test-only commit had taken it): the buffer is remembered as
    /// one the plane does not take, so that the next frame is composed instead of the same commit failing again, a screen that stops while its
    /// client goes on drawing.
    pub fn refused_at_commit(&mut self, fb: u32, errno: i32) {
        let Some((id, f)) = self.fbs.iter().find(|(_, f)| f.imported.fb == fb) else { return };
        let (id, buffer) = (id.clone(), f.buffer.clone());
        let r = Refusal::Test(errno);
        self.journal.push(format!("scanout: refused: fb {fb} at commit: {}", r.text()));
        self.refused.insert(id, (buffer, r));
    }

    /// Counts the frame and says what changed: `took_direct` is whether the frame's assignment is the window's buffer on the plane (the
    /// UI's own rules can still compose a frame this chose to scan out). The journal lines are taken with [`Scanout::take_journal`].
    pub fn settle(&mut self, verdict: &Result<Choice, Refusal>, took_direct: bool) {
        if self.policy.mode == Mode::Off {
            return;
        }
        let refusal = match verdict {
            Ok(_) if took_direct => None,
            Ok(_) => Some(Refusal::UiLayer),
            Err(r) => Some(r.clone()),
        };
        match refusal {
            None => {
                self.stats.direct += 1;
                if self.state != Shown::Direct {
                    self.stats.transitions += 1;
                    let what = self.last_direct.as_ref().map(|(c, fb)| format!("{} as fb {fb}", c.text())).unwrap_or_default();
                    self.journal.push(format!("scanout: direct ({what})"));
                    self.state = Shown::Direct;
                }
            }
            Some(r) => {
                self.stats.composed += 1;
                *self.stats.refused.entry(r.key()).or_insert(0) += 1;
                if self.state == Shown::Direct {
                    self.stats.transitions += 1;
                    self.journal.push(format!("scanout: composed ({})", r.text()));
                }
                // A reason about the buffer is told once, even when direct never starts (there is nothing to transition from).
                if r.of_buffer() && self.logged.len() < LOGGED_REFUSALS && self.logged.insert(r.text()) {
                    self.journal.push(format!("scanout: refused: {}", r.text()));
                }
                if r.of_buffer() {
                    self.last_refusal = Some(r.clone());
                }
                self.state = Shown::Composed(Some(r));
            }
        }
    }

    // The frame's life on the display, so `holds` knows what is read: drawn (waiting to be committed), dropped (superseded before it was),
    // committed (its flip is pending), landed (it is on the screen).

    /// A frame was drawn: `direct` is the choice it shows, `None` for a composed page.
    pub fn drawn(&mut self, direct: Option<&Choice>) {
        self.holds.waiting = match direct {
            Some(c) => Slot::Direct(c.id.clone()),
            None => Slot::Composed,
        };
    }

    /// The frame waiting behind the flip in flight never went out: a newer one took its place, or its commit failed.
    pub fn dropped(&mut self) {
        self.holds.waiting = Slot::Empty;
    }

    /// The waiting frame was committed: its flip is pending. A flip the driver never reported is taken as landed first.
    pub fn committed(&mut self) {
        if self.holds.pending != Slot::Empty {
            self.holds.shown = std::mem::take(&mut self.holds.pending);
        }
        self.holds.pending = std::mem::take(&mut self.holds.waiting);
    }

    /// A flip landed: the frame it carried is on the screen, and what was shown before it no longer is.
    pub fn landed(&mut self) {
        if self.holds.pending != Slot::Empty {
            self.holds.shown = std::mem::take(&mut self.holds.pending);
        }
    }

    /// Whether the display reads `buffer`, or will once a frame already drawn is committed: it is not the client's to draw into.
    pub fn holds(&self, buffer: &WlBuffer) -> bool {
        self.holds.has(&buffer.id())
    }

    /// A client destroyed `buffer`: its framebuffer goes, at once, or when the display has stopped reading it.
    pub fn forget(&mut self, device: &mut dyn Device, buffer: &WlBuffer) {
        let id = buffer.id();
        self.refused.remove(&id);
        if let Some(fb) = self.fbs.remove(&id) {
            self.retire(device, id, fb.imported);
        }
    }

    fn retire(&mut self, device: &mut dyn Device, id: ObjectId, imported: Imported) {
        if self.holds.has(&id) {
            self.zombies.push((id, imported));
        } else {
            device.destroy(imported);
        }
    }

    /// After a flip: framebuffers of dead buffers go, and so do those that were waiting for the display to let go.
    pub fn reap(&mut self, device: &mut dyn Device) {
        let dead: Vec<ObjectId> = self.fbs.iter().filter(|(_, fb)| !fb.buffer.is_alive()).map(|(id, _)| id.clone()).collect();
        for id in dead {
            if let Some(fb) = self.fbs.remove(&id) {
                self.retire(device, id, fb.imported);
            }
        }
        self.refused.retain(|_, (b, _)| b.is_alive());
        for (id, imported) in std::mem::take(&mut self.zombies) {
            if self.holds.has(&id) {
                self.zombies.push((id, imported));
            } else {
                device.destroy(imported);
            }
        }
    }

    /// Everything imported goes (the output is closing).
    pub fn close(&mut self, device: &mut dyn Device) {
        for (_, fb) in self.fbs.drain() {
            device.destroy(fb.imported);
        }
        for (_, imported) in self.zombies.drain(..) {
            device.destroy(imported);
        }
    }

    /// Framebuffers made, and those waiting for the display to let go of them.
    pub fn framebuffers(&self) -> (usize, usize) {
        (self.fbs.len(), self.zombies.len())
    }

    pub fn take_journal(&mut self) -> Vec<String> {
        std::mem::take(&mut self.journal)
    }

    /// For `ctl status` and `ctl scanout`.
    pub fn report(&self) -> Value {
        let m = self.meta.stats();
        let last = match &self.state {
            Shown::Direct => json!({
                "state": "direct",
                "buffer": self.last_direct.as_ref().map(|(c, fb)| json!({
                    "fb": fb,
                    "format": fourcc::name(c.format),
                    "modifier": modifier_name(c.modifier),
                    "size": [c.width, c.height],
                    "origin": format!("{:?}", c.origin),
                })),
            }),
            Shown::Composed(r) => json!({"state": "composed", "reason": r.as_ref().map(Refusal::key), "text": r.as_ref().map(Refusal::text)}),
        };
        json!({
            "mode": self.policy.mode.name(),
            "acquire": self.policy.acquire.name(),
            "trust": self.policy.trust.iter().map(|m| format!("0x{m:016x}")).collect::<Vec<_>>(),
            "frames": {"direct": self.stats.direct, "composed": self.stats.composed},
            "refused": self.stats.refused,
            "transitions": self.stats.transitions,
            "last": last,
            "lastBufferRefusal": self.last_refusal.as_ref().map(Refusal::text),
            "framebuffers": {"made": self.fbs.len(), "waiting": self.zombies.len(), "imports": self.stats.imports, "importFailures": self.stats.import_failures},
            "held": {"shown": self.holds.shown.direct(), "pending": self.holds.pending.direct(), "waiting": self.holds.waiting.direct()},
            "descriptors": {"retained": m.retained, "created": m.created, "destroyed": m.destroyed, "withClient": m.with_client, "undecoded": m.undecoded, "allocDevices": self.meta.alloc_devices()},
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plane that lists what a test says, in a mode of its own.
    struct FakePlane {
        mode: (i32, i32),
        list: Vec<(u32, u64)>,
    }

    impl FakePlane {
        /// The set's primary plane: the four formats linear, and AFBC `...62` for the two B-order ones.
        fn set() -> FakePlane {
            let mut list: Vec<(u32, u64)> = FORMATS.iter().map(|f| (*f, modifier::LINEAR)).collect();
            list.push((fourcc::ABGR8888, modifier::AFBC_32X8_SPLIT_SPARSE));
            list.push((fourcc::XBGR8888, modifier::AFBC_32X8_SPLIT_SPARSE));
            FakePlane { mode: (1920, 1080), list }
        }
    }

    impl Device for FakePlane {
        fn mode(&self) -> (i32, i32) {
            self.mode
        }
        fn live(&self) -> bool {
            true
        }
        fn lists(&self, format: u32, modifier: u64) -> bool {
            self.list.contains(&(format, modifier))
        }
        fn lists_format(&self, format: u32) -> bool {
            self.list.iter().any(|(f, _)| *f == format)
        }
        fn import(&mut self, _: &Candidate, _: BorrowedFd<'_>) -> Result<Imported, ImportError> {
            Ok(Imported { fb: 7, gem: 8 })
        }
        fn destroy(&mut self, _: Imported) {}
        fn acquire(&mut self, _: &WlBuffer) -> bool {
            true
        }
    }

    /// What LG's own output ring is: a 1080p XB24 in the OSD's AFBC layout.
    fn good() -> Candidate {
        Candidate {
            origin: Origin::Driver,
            format: fourcc::XBGR8888,
            modifier: modifier::AFBC_32X8_SPLIT_SPARSE,
            width: 1920,
            height: 1080,
            planes: 1,
            offset: 0,
            stride: 7680,
            bytes: Some(8_486_912),
            upright: true,
            protected: false,
        }
    }

    fn on() -> Policy {
        Policy { mode: Mode::Direct, ..Policy::default() }
    }

    fn verdict(c: &Candidate) -> Result<(), Refusal> {
        decide(c, &FakePlane::set(), &on())
    }

    fn memfd(len: i64) -> std::os::fd::OwnedFd {
        // SAFETY: a memfd for the test.
        let fd = unsafe { libc::memfd_create(c"scanout-test".as_ptr(), libc::MFD_CLOEXEC) };
        assert!(fd >= 0);
        // SAFETY: the descriptor is ours.
        let fd = unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd) };
        // SAFETY: sizing the memfd.
        assert_eq!(unsafe { libc::ftruncate(fd.as_raw_fd(), len) }, 0);
        fd
    }

    #[test]
    fn a_buffer_in_the_osds_own_layout_is_taken() {
        assert_eq!(verdict(&good()), Ok(()));
        let linear = Candidate { modifier: modifier::LINEAR, bytes: Some(7680 * 1080), ..good() };
        assert_eq!(verdict(&linear), Ok(()), "a linear XB24 of exactly its rows");
        for f in FORMATS {
            let c = Candidate { format: f, modifier: modifier::LINEAR, ..good() };
            assert_eq!(verdict(&c), Ok(()), "{}", fourcc::name(f));
        }
    }

    #[test]
    fn the_clients_afbc_layout_is_refused_by_the_list_not_by_the_driver() {
        // 16x16 YTR tiled: what every recorded LG client buffer is. The driver would take it (any non-linear modifier is "AFBC" to it) and decode
        // it as 32x8; the plane's list is what says no.
        let c = Candidate { modifier: modifier::AFBC_16X16_YTR_SPARSE_TILED, bytes: Some(8_986_624), ..good() };
        assert_eq!(verdict(&c), Err(Refusal::Modifier(fourcc::XBGR8888, modifier::AFBC_16X16_YTR_SPARSE_TILED)));
        // AFBC with the ARGB colour order is not on the OSD plane at all (lg1k: "AFBC does not supported with ARGB color order").
        let argb = Candidate { format: fourcc::XRGB8888, ..good() };
        assert_eq!(verdict(&argb), Err(Refusal::Modifier(fourcc::XRGB8888, modifier::AFBC_32X8_SPLIT_SPARSE)));
    }

    #[test]
    fn an_experiment_may_try_a_modifier_the_plane_does_not_list_but_nothing_else() {
        let c = Candidate { modifier: modifier::AFBC_16X16_YTR_SPARSE_TILED, bytes: Some(8_986_624), ..good() };
        let p = Policy { mode: Mode::Direct, trust: vec![modifier::AFBC_16X16_YTR_SPARSE_TILED], ..Policy::default() };
        assert_eq!(decide(&c, &FakePlane::set(), &p), Ok(()));
        let odd_format = Candidate { format: 0x3231_564e, ..c.clone() };
        assert_eq!(decide(&odd_format, &FakePlane::set(), &p), Err(Refusal::Format(0x3231_564e)), "NV12 is not trusted in");
        let small = Candidate { width: 256, height: 256, stride: 1024, bytes: Some(266_240), ..c };
        assert_eq!(decide(&small, &FakePlane::set(), &p), Err(Refusal::Size(256, 256)), "the size rule holds whatever the layout");
    }

    #[test]
    fn each_rule_has_its_own_reason() {
        let cases: Vec<(Candidate, Refusal)> = vec![
            (Candidate { protected: true, ..good() }, Refusal::Protected),
            (Candidate { upright: false, ..good() }, Refusal::Upside),
            (Candidate { planes: 2, ..good() }, Refusal::Planes(2)),
            (Candidate { offset: 4096, ..good() }, Refusal::Offset(4096)),
            (Candidate { format: 0x3231_564e, ..good() }, Refusal::Format(0x3231_564e)),
            (Candidate { width: 1280, height: 720, ..good() }, Refusal::Size(1280, 720)),
            (Candidate { stride: 8192, ..good() }, Refusal::Stride(8192, 7680)),
            (Candidate { bytes: Some(4096), ..good() }, Refusal::TooSmall(4096, 7680 * 1080)),
        ];
        for (c, want) in cases {
            assert_eq!(verdict(&c), Err(want.clone()), "{want:?}");
            assert!(!want.text().is_empty() && !want.key().is_empty());
            assert!(want.of_buffer(), "{want:?} is about the buffer and is remembered for it");
        }
        assert_eq!(verdict(&Candidate { bytes: None, ..good() }), Ok(()), "a dma-buf that does not tell its size is not refused for it");
    }

    #[test]
    fn what_is_about_the_screen_is_not_remembered_against_the_buffer() {
        for r in [Refusal::Disabled, Refusal::Plan(NotSole::Layers(2)), Refusal::Capture, Refusal::NoMode, Refusal::UiLayer, Refusal::Acquire] {
            assert!(!r.of_buffer(), "{r:?}");
        }
        assert!(Refusal::Import(libc::EINVAL).text().contains("scattered"));
    }

    #[test]
    fn the_mode_is_the_planes_not_a_constant() {
        let mut p = FakePlane::set();
        p.mode = (3840, 2160);
        assert_eq!(decide(&good(), &p, &on()), Err(Refusal::Size(1920, 1080)));
    }

    #[test]
    fn the_driver_buffer_without_a_descriptor_and_the_shm_buffer_are_refused_before_anything() {
        assert!(matches!(describe(BufferKindRef::External, || None, true), Err(Refusal::NoDescriptor)));
        let pool = crate::buffer::ShmPool::new(memfd(4096), 4096).unwrap();
        let shm = crate::buffer::ShmBuffer { pool, offset: 0, width: 16, height: 16, stride: 64, format: 1 };
        assert!(matches!(describe(BufferKindRef::Shm(&shm), || None, true), Err(Refusal::Shm)));
    }

    #[test]
    fn a_retained_descriptor_describes_a_driver_buffer() {
        let d = Arc::new(meta::Descriptor {
            width: 1920,
            height: 1080,
            stride: 7680,
            format: fourcc::ABGR8888,
            modifier: 0x0800_0000_0000_0151,
            fd: memfd(8_986_624),
            serial: 0,
        });
        let (c, backing) = describe(BufferKindRef::External, || Some(d.clone()), true).unwrap();
        assert_eq!(c.origin, Origin::Driver);
        assert_eq!((c.width, c.height, c.stride, c.format, c.modifier), (1920, 1080, 7680, fourcc::ABGR8888, 0x0800_0000_0000_0151));
        assert_eq!(c.bytes, Some(8_986_624));
        assert!(c.upright && !c.opaque_format(), "AB24 declares no opacity");
        assert!(matches!(backing, Backing::Retained(_)));
        let (c, _) = describe(BufferKindRef::External, || Some(d.clone()), false).unwrap();
        assert!(!c.upright, "the rows' direction is the driver's word, which is the item's");
        let xb = Arc::new(meta::Descriptor { width: 1, height: 1, stride: 4, format: fourcc::XBGR8888, modifier: 0, fd: memfd(4096), serial: 0 });
        assert!(describe(BufferKindRef::External, || Some(xb), true).unwrap().0.opaque_format(), "XB24 has no alpha");
    }

    #[test]
    fn modifiers_are_named_where_the_set_has_a_name() {
        assert_eq!(modifier_name(0), "linear");
        assert!(modifier_name(modifier::AFBC_32X8_SPLIT_SPARSE).starts_with("AFBC 32x8"));
        assert!(modifier_name(modifier::AFBC_16X16_YTR_SPARSE_TILED).contains("0x0800000000000151"));
        assert_eq!(modifier_name(0x1234), "0x0000000000001234");
    }

    #[test]
    fn mode_and_acquire_parse_what_the_command_line_says() {
        assert_eq!(Mode::parse("direct"), Some(Mode::Direct));
        assert_eq!(Mode::parse("off"), Some(Mode::Off));
        assert_eq!(Mode::parse("on"), None);
        assert_eq!(Acquire::parse("sample"), Some(Acquire::Sample));
        assert_eq!(Acquire::parse("none"), Some(Acquire::Trust));
        assert_eq!(Acquire::parse("fence"), None);
        assert_eq!(Policy::default().mode, Mode::Off, "off unless asked");
        assert_eq!(Policy::default().acquire, Acquire::Sample, "the client's rendering is waited for unless a measurement says not to");
    }
}
