//! The app's connection to the compositor and the layers it owns.
//!
//! Requests are staged and sent together by [`Scene::try_commit`], so everything changed between two commits appears on the
//! same frame. Layers, groups, motion tracks, prepared backdrops and atlases are owned handles: dropping one deletes it on the
//! compositor, in an order the compositor accepts (what hangs under a group goes before it, a track's bindings before the
//! track, glyph runs before their atlas, a prepared set before the groups it captures), and removes its shared-memory file.
//!
//! **Nothing here waits for the compositor.** The socket is written without blocking by a bounded writer (`writer.rs`): a
//! transaction the writer cannot take now is answered `SubmitError::WouldBlock` at once, with the staged transaction and the
//! commit's number untouched; the host waits for the socket to become writable ([`Scene::writer_fd`],
//! [`Scene::wants_writable`], [`Scene::writer_ready`]) and commits again. Requests that act at once (pointer samples, settles,
//! cancels, backdrop leases, the display and clock switches) never ride in a transaction: they are urgent frames that overtake
//! whatever has not started, and are written the moment the socket takes them.

use crate::atlas::AtlasEntry;
use crate::backdrop::BackdropEntry;
use crate::error::{CommitTag, SubmitError};
use crate::group::{Group, GroupEntry, GroupId};
use crate::handle::Slots;
use crate::limits::MAX_DRAWABLE_NODES;
use crate::motion::{Lead, MotionEntry};
use crate::writer::{Sink, Urgent, Writer};
use compositor_protocol::group::MAX_GROUPS;
use compositor_protocol::motion::MAX_MOTIONS;
use compositor_protocol::resources::{MAX_ATLASES, MAX_BACKDROPS};
use compositor_protocol::{
    Content, Ease, Event, Glass, Glyph, LayerId, Live, Mesh, MeshData, PROTOCOL_VERSION, Props,
    Rect, Request, Shape, SourceId, Wallpaper, image_path, mesh_path, read_frame, write_frame,
};
use memmap2::MmapMut;
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::io::{self, BufReader};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

/// What wakes the host when the compositor has said something (the host's eventfd): called from the thread that reads the
/// connection after every event is on the channel, and once more when the connection ends.
pub type Wake = Box<dyn Fn() + Send + 'static>;

/// The socket as the writer sees it: every write is `MSG_DONTWAIT`, which leaves the descriptor's own flags alone (the thread
/// that reads events shares the open file and reads it blocking), and `MSG_NOSIGNAL`, so a dead peer is an error, not a signal.
pub(crate) struct UnixSink(UnixStream);

impl Sink for UnixSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        loop {
            // SAFETY: `buf` is a live slice for the length given; the descriptor is open as long as `self` is.
            let n = unsafe {
                libc::send(
                    self.0.as_raw_fd(),
                    buf.as_ptr().cast(),
                    buf.len(),
                    libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
                )
            };
            if n >= 0 {
                return Ok(n as usize);
            }
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::Interrupted {
                return Err(e);
            }
        }
    }
}

/// `CLOCK_MONOTONIC` in nanoseconds: the clock the compositor stamps its events with and the digitiser stamps touches with.
pub fn monotonic_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: a plain clock read into a stack struct.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

/// How a scene is opened beyond its stream.
#[derive(Default)]
pub struct SceneOptions {
    /// Where the files of layers and staging slots are: the directory the protocol names (`/run/lumashow/shm`), unless a test
    /// or a preview on a machine that cannot write there moves it.
    pub shm_dir: Option<PathBuf>,
    /// Called after every event the compositor sends is on the channel, and once when the connection ends.
    pub wake: Option<Wake>,
    /// `SO_SNDBUF` for the socket, so that a test can fill it with little.
    pub send_buffer: Option<usize>,
}

pub(crate) struct Inner {
    pub(crate) out: Writer<UnixSink>,
    /// What has been asked since the last commit, in order.
    pub(crate) batch: Vec<Request>,
    /// Vectors for `SetGlyphs` quads, kept between commits so that a glyph swap allocates nothing once warm.
    pub(crate) glyph_pool: Vec<Vec<Glyph>>,
    /// Wire ids come from counters that only rise: an id is never reused.
    pub(crate) next_layer: u32,
    pub(crate) next_group: u32,
    pub(crate) next_motion: u32,
    pub(crate) next_backdrop: u32,
    pub(crate) next_atlas: u32,
    /// The number the next commit will carry. A commit that was not taken does not use it up.
    pub(crate) next_tag: u64,
    shm_dir_ok: bool,
    shm_dir: Option<PathBuf>,
    /// Files of layers that have been dropped, each with the tag of the commit that carries the layer's end: to be removed
    /// once the compositor has shown that commit or a later one (see [`Scene::retire`]).
    retired: Vec<(u64, PathBuf)>,
    pub(crate) clock: Rc<dyn Fn() -> u64>,
    pub(crate) lead: Lead,
    pub(crate) groups: Slots<GroupEntry>,
    pub(crate) motions: Slots<MotionEntry>,
    pub(crate) backdrops: Slots<BackdropEntry>,
    pub(crate) atlases: Slots<AtlasEntry>,
    /// Layers a prepared set captures, with how many sets do: their deletion waits for the sets'.
    pub(crate) pins: Vec<(LayerId, u32)>,
    /// Layers dropped while pinned: deleted once nothing captures them.
    pub(crate) deferred: Vec<LayerId>,
    /// While set, every layer made hangs under this group from the moment it is made (`Scene::adopting`).
    pub(crate) adopt: Option<GroupId>,
    /// While set, every group made under no parent of its own hangs under this group (`Scene::adopting_view`).
    pub(crate) nest: Option<GroupId>,
    /// Layers the compositor holds or will hold once the batch is sent, counted against the ceiling it refuses at.
    pub(crate) live_layers: usize,
    /// Layers whose creation was held back at the ceiling: nothing is ever sent for them, and the compositor never hears of them.
    refused: BTreeSet<LayerId>,
    /// A creation was held back (or the compositor refused a request) since the app last asked: what is on the screen may be
    /// missing a layer (see [`Scene::take_overflow`]).
    overflowed: bool,
    /// The compositor said it can draw `SmoothFirst::Full` (`Welcome::smooth_first_full`).
    smooth_first_full: bool,
}

impl Inner {
    pub(crate) fn push(&mut self, request: Request) {
        // Whatever follows a creation that was held back is about a layer that does not exist.
        if !self.refused.is_empty()
            && layer_of(&request).is_some_and(|id| self.refused.contains(&id))
        {
            return;
        }
        self.batch.push(request);
    }

    /// The time the screen will show a request made now: the clock plus the measured lead of a commit.
    pub(crate) fn estimate_ns(&self) -> u64 {
        (self.clock)().saturating_add(self.lead.ns())
    }

    /// A layer is dropped. The compositor is told to delete it, unless it has never heard of it, because the layer was
    /// made since the last commit: then everything staged for it is taken back, and the compositor never sees a layer
    /// whose picture is gone by the time it looks (it opens the file when it handles the `Create`, and would refuse
    /// it, and every request that follows for a layer that does not exist). A layer a prepared set captures stays until
    /// the set is deleted.
    pub(crate) fn delete_layer(&mut self, id: LayerId) {
        if self.pins.iter().any(|(pinned, n)| *pinned == id && *n > 0) {
            self.deferred.push(id);
            return;
        }
        if self.refused.remove(&id) {
            return;
        }
        self.live_layers = self.live_layers.saturating_sub(1);
        let unheard_of = self
            .batch
            .iter()
            .any(|r| matches!(r, Request::Create { id: made, .. } if *made == id));
        if unheard_of {
            self.batch.retain(|r| layer_of(r) != Some(id));
        } else {
            self.batch.push(Request::Delete { id });
        }
    }

    /// Takes back what is staged for layer `id` that only means something to the glyph run it is about to stop being: its list
    /// of quads and its palette.
    pub(crate) fn unstage_glyphs(&mut self, id: LayerId) {
        let mut at = 0;
        while at < self.batch.len() {
            if !matches!(
                &self.batch[at],
                Request::SetGlyphs { id: of, .. } | Request::Restyle { id: of, .. } if *of == id
            ) {
                at += 1;
                continue;
            }
            if let Request::SetGlyphs { mut glyphs, .. } = self.batch.remove(at) {
                glyphs.clear();
                self.glyph_pool.push(glyphs);
            }
        }
    }

    /// A prepared set captures `id`.
    pub(crate) fn pin_layer(&mut self, id: LayerId) {
        match self.pins.iter_mut().find(|(pinned, _)| *pinned == id) {
            Some((_, n)) => *n += 1,
            None => self.pins.push((id, 1)),
        }
    }

    /// A prepared set that captured `id` is deleted.
    pub(crate) fn unpin_layer(&mut self, id: LayerId) {
        let Some(at) = self.pins.iter().position(|(pinned, _)| *pinned == id) else {
            return;
        };
        self.pins[at].1 -= 1;
        if self.pins[at].1 == 0 {
            self.pins.swap_remove(at);
            if let Some(d) = self.deferred.iter().position(|l| *l == id) {
                self.deferred.swap_remove(d);
                self.delete_layer(id);
            }
        }
    }

    /// Has `path` (the file of something that is gone) removed once the compositor has shown the next commit or a later one.
    pub(crate) fn retire_file(&mut self, path: &Path) {
        let after = self.next_tag;
        self.retired.push((after, path.to_owned()));
    }

    /// Removes the files retired for commits up to `tag` (all of them for `u64::MAX`).
    fn remove_retired(&mut self, tag: u64) {
        self.retired.retain(|(after, path)| {
            let done = *after <= tag;
            if done {
                let _ = fs::remove_file(path);
            }
            !done
        });
    }

    /// Hands the staged requests' vectors back to the pool and empties the batch.
    fn recycle_batch(&mut self) {
        for request in self.batch.drain(..) {
            if let Request::SetGlyphs { mut glyphs, .. } = request {
                glyphs.clear();
                self.glyph_pool.push(glyphs);
            }
        }
    }

    /// Sends an urgent request, and whatever else the socket will take. The writer's error is the connection's.
    pub(crate) fn urgent(&mut self, request: &Request, how: Urgent) -> io::Result<()> {
        self.out.push_urgent(request, how)
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.remove_retired(u64::MAX);
    }
}

/// The layer a request is about, if it is about one.
fn layer_of(request: &Request) -> Option<LayerId> {
    match request {
        Request::Create { id, .. }
        | Request::Damage { id, .. }
        | Request::Restyle { id, .. }
        | Request::Ripple { id, .. }
        | Request::Set { id, .. }
        | Request::Animate { id, .. }
        | Request::Delete { id } => Some(*id),
        Request::Attach { id, .. } | Request::SetGlyphs { id, .. } => Some(*id),
        Request::Hello { .. }
        | Request::Commit { .. }
        | Request::Stats
        | Request::HoldGpuClock { .. }
        | Request::Ideas { .. }
        | Request::SmoothFirst { .. }
        | Request::Display { .. }
        | Request::GroupCreate { .. }
        | Request::GroupSet { .. }
        | Request::GroupParent { .. }
        | Request::GroupBind { .. }
        | Request::GroupUnbind { .. }
        | Request::GroupDelete { .. }
        | Request::MotionCreate { .. }
        | Request::MotionDelete { .. }
        | Request::MotionDrive { .. }
        | Request::MotionSettle { .. }
        | Request::MotionCancel { .. }
        | Request::BackdropCreate { .. }
        | Request::BackdropRefresh { .. }
        | Request::BackdropFreeze { .. }
        | Request::BackdropRelease { .. }
        | Request::BackdropDelete { .. }
        | Request::AtlasCreate { .. }
        | Request::AtlasUpload { .. }
        | Request::AtlasDelete { .. } => None,
    }
}

/// What the compositor said when the app connected.
#[derive(Clone, Copy, Debug)]
pub struct Welcome {
    pub panel_width: u32,
    pub panel_height: u32,
    pub refresh_hz: f32,
    /// The compositor can draw `SmoothFirst::Full`: Settings offers the level only while this is true.
    pub smooth_first_full: bool,
}

#[derive(Clone)]
pub struct Scene {
    pub(crate) inner: Rc<RefCell<Inner>>,
}

impl Scene {
    /// Connects, says hello and returns the scene plus the stream of compositor events (frames, stats,
    /// errors), read on a background thread until the connection closes.
    pub fn connect(path: &str) -> io::Result<(Self, Welcome, Receiver<Event>)> {
        Self::over(UnixStream::connect(path)?)
    }

    /// [`Scene::connect`] with `wake` called after every event is on the channel (and once when the connection ends), so a
    /// host that sleeps in `poll` on an eventfd needs no thread of its own to forward them.
    pub fn connect_waking(path: &str, wake: Wake) -> io::Result<(Self, Welcome, Receiver<Event>)> {
        Self::over_waking(UnixStream::connect(path)?, wake)
    }

    /// The same over an already connected stream (tests use a socket pair).
    pub fn over(stream: UnixStream) -> io::Result<(Self, Welcome, Receiver<Event>)> {
        Self::over_with(stream, SceneOptions::default())
    }

    /// [`Scene::connect_waking`] over an already connected stream.
    pub fn over_waking(
        stream: UnixStream,
        wake: Wake,
    ) -> io::Result<(Self, Welcome, Receiver<Event>)> {
        Self::over_with(
            stream,
            SceneOptions {
                wake: Some(wake),
                ..SceneOptions::default()
            },
        )
    }

    /// [`Scene::over`] with the files of layers and staging slots in `shm_dir` instead of where the protocol puts them
    /// (`/run/lumashow/shm`): for tests and previews on a machine that cannot write there.
    pub fn over_in(
        stream: UnixStream,
        shm_dir: PathBuf,
    ) -> io::Result<(Self, Welcome, Receiver<Event>)> {
        Self::over_with(
            stream,
            SceneOptions {
                shm_dir: Some(shm_dir),
                ..SceneOptions::default()
            },
        )
    }

    pub fn over_with(
        stream: UnixStream,
        options: SceneOptions,
    ) -> io::Result<(Self, Welcome, Receiver<Event>)> {
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut out = stream;
        write_frame(
            &mut out,
            &Request::Hello {
                version: PROTOCOL_VERSION,
            },
        )?;
        let mut buf = Vec::new();
        let welcome = match read_frame::<Event>(&mut reader, &mut buf)? {
            Some(Event::Welcome {
                version,
                panel_width,
                panel_height,
                refresh_hz,
                smooth_first_full,
            }) if version == PROTOCOL_VERSION => Welcome {
                panel_width,
                panel_height,
                refresh_hz,
                smooth_first_full,
            },
            Some(Event::Error { message }) => {
                return Err(io::Error::other(format!("compositor refused: {message}")));
            }
            other => {
                return Err(io::Error::other(format!(
                    "compositor answered hello with {other:?}"
                )));
            }
        };
        if let Some(bytes) = options.send_buffer {
            let bytes = bytes as libc::c_int;
            // SAFETY: a plain option write of an int on an open socket.
            unsafe {
                libc::setsockopt(
                    out.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_SNDBUF,
                    (&raw const bytes).cast(),
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                );
            }
        }
        let (tx, rx) = mpsc::channel();
        let wake = options.wake;
        thread::Builder::new()
            .name("compositor-events".into())
            .spawn(move || {
                let mut buf = Vec::new();
                while let Ok(Some(event)) = read_frame::<Event>(&mut reader, &mut buf) {
                    if tx.send(event).is_err() {
                        return;
                    }
                    if let Some(wake) = &wake {
                        wake();
                    }
                }
                // The channel closes first, so the host that wakes finds the connection gone.
                drop(tx);
                if let Some(wake) = &wake {
                    wake();
                }
            })?;
        let inner = Inner {
            out: Writer::new(UnixSink(out)),
            batch: Vec::new(),
            glyph_pool: Vec::new(),
            next_layer: 1,
            next_group: 1,
            next_motion: 1,
            next_backdrop: 1,
            next_atlas: 1,
            next_tag: 1,
            shm_dir_ok: false,
            shm_dir: options.shm_dir,
            retired: Vec::new(),
            clock: Rc::new(monotonic_ns),
            lead: Lead::default(),
            groups: Slots::with_limit(MAX_GROUPS),
            motions: Slots::with_limit(MAX_MOTIONS),
            backdrops: Slots::with_limit(MAX_BACKDROPS),
            atlases: Slots::with_limit(MAX_ATLASES),
            pins: Vec::new(),
            deferred: Vec::new(),
            adopt: None,
            nest: None,
            live_layers: 0,
            refused: BTreeSet::new(),
            overflowed: false,
            smooth_first_full: welcome.smooth_first_full,
        };
        Ok((
            Self {
                inner: Rc::new(RefCell::new(inner)),
            },
            welcome,
            rx,
        ))
    }

    fn allocate(&self) -> LayerId {
        let mut inner = self.inner.borrow_mut();
        let id = LayerId(inner.next_layer);
        inner.next_layer += 1;
        id
    }

    // -- Submission ------------------------------------------------------------------------------------------------

    /// Sends everything staged since the last commit as one transaction and returns its tag: the number
    /// `Event::Frame::tag` brings back once the frame that first shows it is on the screen. Never waits.
    ///
    /// `Err(SubmitError::WouldBlock)` is a full queue, not a failure: the compositor has not read what was sent before. The
    /// staged transaction is intact and the tag is not used up. Commit again after [`Scene::writer_ready`] says there is room
    /// (the host polls [`Scene::writer_fd`] for writability while [`Scene::wants_writable`]), not in a loop. A transaction
    /// that was taken is never sent twice, whatever happens to the connection.
    pub fn try_commit(&self) -> Result<CommitTag, SubmitError> {
        let mut guard = self.inner.borrow_mut();
        let inner = &mut *guard;
        let tag = inner.next_tag;
        inner.out.submit(&inner.batch, tag)?;
        inner.next_tag += 1;
        let now = (inner.clock)();
        inner.lead.committed(tag, now);
        inner.recycle_batch();
        inner.out.pump().map_err(SubmitError::Io)?;
        Ok(CommitTag(tag))
    }

    /// True when requests are waiting for the next commit.
    pub fn has_pending(&self) -> bool {
        !self.inner.borrow().batch.is_empty()
    }

    /// The socket, to wait on for writability (`POLLOUT`) while [`Scene::wants_writable`]. Waiting for it at any other time
    /// wakes at once and for nothing.
    pub fn writer_fd(&self) -> RawFd {
        self.inner.borrow().out.sink().0.as_raw_fd()
    }

    /// Whether accepted bytes are waiting for the socket to take them (the writer's queue or an urgent request): the host
    /// asks `poll` for `POLLOUT` on [`Scene::writer_fd`] exactly then.
    pub fn wants_writable(&self) -> bool {
        self.inner.borrow().out.wants_writable()
    }

    /// The socket became writable: writes what it takes. `Ok(true)` when the queue now has room for a transaction, so that a
    /// commit that answered `WouldBlock` can be made again. The error is the connection's.
    pub fn writer_ready(&self) -> io::Result<bool> {
        let mut inner = self.inner.borrow_mut();
        inner.out.pump()?;
        Ok(inner.out.has_room())
    }

    /// Whether a transaction could be taken now.
    pub fn has_room(&self) -> bool {
        self.inner.borrow().out.has_room()
    }

    /// Waits until the socket is writable, for at most `timeout`, and writes what it takes; whether the queue then has room.
    /// **Blocking, for the tools and tests that have nothing else to do** (the lab's offline raster, the examples); the UI thread
    /// polls [`Scene::writer_fd`] in its own loop instead.
    pub fn wait_for_room(&self, timeout: Duration) -> io::Result<bool> {
        let mut pollfd = libc::pollfd {
            fd: self.writer_fd(),
            events: libc::POLLOUT,
            revents: 0,
        };
        // SAFETY: one valid pollfd for the length given.
        let n = unsafe {
            libc::poll(
                &raw mut pollfd,
                1,
                timeout.as_millis().min(i32::MAX as u128) as i32,
            )
        };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::Interrupted {
                return Err(e);
            }
        }
        self.writer_ready()
    }

    /// Asks immediately for one status reply without submitting a frame (including while the screen is off).
    pub fn request_stats(&self) -> io::Result<()> {
        self.at_once(&Request::Stats)
    }

    /// Asks the compositor to hold the GPU at `mhz` (one of the device's clocks; anything else is refused with an
    /// `Event::Error`), or, with `None`, to go back to its own policy (the fastest clock while frames flow, the driver's
    /// scaling otherwise). The compositor is the only writer of the GPU's clock. It takes effect at once, not with the next
    /// `try_commit`, and ends when this connection does, so a client that dies holding a clock cannot leave the GPU there.
    pub fn hold_gpu_clock(&self, mhz: Option<u32>) -> io::Result<()> {
        self.at_once(&Request::HoldGpuClock { mhz })
    }

    /// Switches the glass ideas of `compositor_protocol::ideas` on and off: `mask` is all that is on. It is no part of a
    /// frame (a `try_commit` draws a still screen again), and ends when this connection does.
    pub fn set_ideas(&self, mask: u32) -> io::Result<()> {
        self.at_once(&Request::Ideas { mask })
    }

    /// Sets "Smooth first" (`compositor_protocol::SmoothFirst`): what the compositor may skip, or draw plainer, to keep every frame
    /// on its refresh. It stands for the glass ideas and the Full ladder both (so a host that uses it never sets the ideas itself),
    /// is no part of a frame, and ends when this connection does: the compositor then goes back to `Invisible`. `Full` is refused
    /// here, without a word to the compositor, while [`Scene::smooth_first_full`] is false: a level that cannot be drawn is never
    /// asked for, and never accepted to do nothing.
    pub fn set_smooth_first(&self, level: compositor_protocol::SmoothFirst) -> io::Result<()> {
        if level.ladder() && !self.smooth_first_full() {
            return Err(io::Error::other(
                "the compositor cannot draw Smooth first: Full",
            ));
        }
        self.at_once(&Request::SmoothFirst { level })
    }

    /// Whether the compositor can draw `SmoothFirst::Full` (it said so in its welcome): Settings offers the level only while this
    /// is true.
    pub fn smooth_first_full(&self) -> bool {
        self.inner.borrow().smooth_first_full
    }

    /// Tells the compositor whether anyone can see the screen (`Request::Display`): dark, it draws no frames at all; lit,
    /// it draws the scene as it is now straight away. Like `hold_gpu_clock`, at once rather than with the next commit.
    pub fn set_display(&self, on: bool) -> io::Result<()> {
        self.at_once(&Request::Display { on })
    }

    /// A request that acts at once and about nothing a transaction creates: queued as an urgent frame, in order with the
    /// other such requests.
    fn at_once(&self, request: &Request) -> io::Result<()> {
        self.inner.borrow_mut().urgent(
            request,
            Urgent {
                after_tag: 0,
                stream: 0,
                replaceable: None,
            },
        )
    }

    // -- What the compositor said ----------------------------------------------------------------------------------

    /// Takes in one event of the compositor, so that what depends on it moves on: a frame retires the files of layers that
    /// ended by its commit and teaches the estimate of how long a commit takes to reach the screen; a motion event
    /// reconciles the estimate of its track (without moving anything on the screen); a backdrop that became ready or was
    /// frozen, and an atlas upload that landed, update their handles. The host calls it for every event it receives, before
    /// it looks at the event itself.
    pub fn observe(&self, event: &Event) {
        let mut inner = self.inner.borrow_mut();
        match event {
            Event::Frame {
                tag, presented_ns, ..
            } => {
                inner.lead.presented(*tag, *presented_ns);
                inner.remove_retired(*tag);
            }
            Event::Motion {
                id,
                generation,
                kind,
                value,
                velocity,
                time_ns,
            } => inner.observe_motion(*id, *generation, *kind, *value, *velocity, *time_ns),
            Event::BackdropReady {
                id,
                generation,
                content,
            } => inner.observe_backdrop_ready(*id, *generation, *content),
            Event::BackdropFrozen { id, generation } => {
                inner.observe_backdrop_frozen(*id, *generation)
            }
            // A request the compositor refused leaves something unmade: the same news as a layer held back here.
            Event::Error { .. } => inner.overflowed = true,
            Event::AtlasUploaded {
                id,
                slot,
                generation,
            } => inner.observe_atlas_uploaded(*id, *slot, *generation),
            Event::Welcome { .. } | Event::Stats(_) => {}
        }
    }

    /// The compositor has shown the commit `tag` (an `Event::Frame` says so): it has been through every request up to that
    /// commit, so the files of layers that ended by then are of no more use to it. [`Scene::observe`] does this for a frame.
    pub fn shown(&self, tag: u64) {
        self.inner.borrow_mut().remove_retired(tag);
    }

    /// How long after a request is made the screen shows it, as measured on the commits so far (and a documented constant
    /// before the first frame): what the estimates of motion tracks add to the clock.
    pub fn present_lead_ns(&self) -> u64 {
        self.inner.borrow().lead.ns()
    }

    /// `CLOCK_MONOTONIC` now, nanoseconds.
    pub fn now_ns(&self) -> u64 {
        (self.inner.borrow().clock)()
    }

    /// Replaces the clock the estimates read, and fixes the lead at `lead_ns`: tests of a host or a screen make time explicit
    /// so that an estimate can be held to the shared equations exactly. Never needed outside tests.
    pub fn set_clock(&self, clock: impl Fn() -> u64 + 'static, lead_ns: u64) {
        let mut inner = self.inner.borrow_mut();
        inner.clock = Rc::new(clock);
        inner.lead = Lead::fixed(lead_ns);
    }

    /// Where the file of a layer is, given the path the protocol names for it.
    pub(crate) fn file_of(&self, protocol_path: String) -> PathBuf {
        match &self.inner.borrow().shm_dir {
            Some(dir) => dir.join(
                Path::new(&protocol_path)
                    .file_name()
                    .expect("the protocol names a file"),
            ),
            None => PathBuf::from(protocol_path),
        }
    }

    /// Has `path` (the file of a layer that is gone) removed once the compositor has shown the next commit or a later one
    /// ([`Scene::shown`]), not before: the compositor opens the file when it handles the layer's `Create`, which it may
    /// only do some commits after the app has written it (it is busy, or the app is fast), and a file that is gone by then
    /// makes it refuse the layer and everything asked of it.
    pub(crate) fn retire(&self, path: &Path) {
        self.inner.borrow_mut().retire_file(path);
    }

    /// Opens `path` as a fresh shared-memory file of `len` bytes, making the shm directory on first use.
    pub(crate) fn shm_file(&self, path: &PathBuf, len: u64) -> io::Result<fs::File> {
        {
            let mut inner = self.inner.borrow_mut();
            if !inner.shm_dir_ok {
                if let Some(dir) = path.parent() {
                    fs::create_dir_all(dir)?;
                }
                inner.shm_dir_ok = true;
            }
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        file.set_len(len)?;
        Ok(file)
    }

    /// The pixels of layer `id`, mapped: premultiplied RGBA, `width * 4` bytes per row.
    fn pixels(&self, id: LayerId, width: u32, height: u32) -> io::Result<Pixels> {
        let path = self.file_of(image_path(id));
        let file = self.shm_file(&path, u64::from(width) * u64::from(height) * 4)?;
        // SAFETY: the file is private to this layer; the compositor only reads it, and the mapping is dropped
        // with the layer before the file is removed.
        let map = unsafe { MmapMut::map_mut(&file)? };
        Ok(Pixels {
            map,
            width,
            height,
            path,
            scene: self.clone(),
        })
    }

    pub(crate) fn create(
        &self,
        id: LayerId,
        width: u32,
        height: u32,
        z: i32,
        content: Content,
        props: Props,
    ) -> Layer {
        {
            let mut inner = self.inner.borrow_mut();
            if inner.live_layers >= MAX_DRAWABLE_NODES {
                // The compositor would refuse it, and every request that follows. The layer exists here as a handle that does
                // nothing, and whoever builds is told (`take_overflow`) that what it built is not whole.
                inner.refused.insert(id);
                inner.overflowed = true;
            } else {
                inner.live_layers += 1;
                inner.push(Request::Create {
                    id,
                    width,
                    height,
                    z,
                    content,
                    props,
                });
            }
        }
        let layer = Layer {
            id,
            scene: self.clone(),
            group: Cell::new(None),
            props: Cell::new(props),
            handed_over: false,
        };
        let adopt = self.inner.borrow().adopt;
        if let Some(group) = adopt {
            // Made under the group with the props it was made with: what the component meant to show is what shows, in the
            // group's space. A group that has gone leaves the layer on the screen.
            let _ = self.inner.borrow_mut().group_attach(group, &layer, props);
        }
        layer
    }

    /// How many layers the compositor holds, or will once what is staged is sent. At most [`MAX_DRAWABLE_NODES`].
    pub fn live_layers(&self) -> usize {
        self.inner.borrow().live_layers
    }

    /// How many more layers can be made before the compositor's ceiling.
    pub fn layer_room(&self) -> usize {
        MAX_DRAWABLE_NODES.saturating_sub(self.live_layers())
    }

    /// Whether, since the last call, a layer could not be made because the ceiling was reached, or the compositor refused a request
    /// ([`Scene::note_refusal`]): what was built then is not whole and must be taken back or made again, never shown.
    pub fn take_overflow(&self) -> bool {
        std::mem::take(&mut self.inner.borrow_mut().overflowed)
    }

    /// The compositor refused a request (`Event::Error`): the same as a layer that could not be made.
    pub fn note_refusal(&self) {
        self.inner.borrow_mut().overflowed = true;
    }

    /// Runs `build` with every layer it makes hanging under `group` from the moment it is made, with the props it was made with
    /// (in the group's space). The way to build a component inside a view that fades or hides as one: nothing is attached hidden
    /// and posed afterwards, so no component has to know it is inside a group.
    pub fn adopting<R>(&self, group: &Group, build: impl FnOnce() -> R) -> R {
        let previous = self.inner.borrow_mut().adopt.replace(group.id());
        let out = build();
        self.inner.borrow_mut().adopt = previous;
        out
    }

    /// [`Scene::adopting`] for a whole view: the layers `build` makes hang under `group` as they are made, and so do the groups it
    /// makes under no parent of its own (a group made under one keeps it), whatever the view is made of (components with groups of
    /// parts, a shell with groups of its own). A view built so inside a group that is inert ([`Group::set_inert`]) is made and not
    /// shown, its layers applied and their pictures taken by the compositor between frames; the frame that lets it show draws it,
    /// with one request in all (`Group::set_inert(false)`) and nothing made. The group is the screen's own world, so its pose
    /// stays the identity unless the view is meant to move as one.
    pub fn adopting_view<R>(&self, group: &Group, build: impl FnOnce() -> R) -> R {
        let (adopt, nest) = {
            let mut inner = self.inner.borrow_mut();
            (
                inner.adopt.replace(group.id()),
                inner.nest.replace(group.id()),
            )
        };
        let out = build();
        {
            let mut inner = self.inner.borrow_mut();
            inner.adopt = adopt;
            inner.nest = nest;
        }
        out
    }

    pub(crate) fn allocate_layer(&self) -> LayerId {
        self.allocate()
    }

    /// A layer of pixels the app draws, `width` × `height`, premultiplied RGBA.
    pub fn image(&self, width: u32, height: u32, z: i32, props: Props) -> io::Result<ImageLayer> {
        let id = self.allocate();
        let pixels = self.pixels(id, width, height)?;
        Ok(ImageLayer {
            layer: self.create(id, width, height, z, Content::Image, props),
            pixels,
        })
    }

    /// A shape the compositor draws, `width` × `height` logical pixels.
    pub fn shape(&self, width: u32, height: u32, z: i32, shape: Shape, props: Props) -> ShapeLayer {
        let id = self.allocate();
        ShapeLayer {
            layer: self.create(id, width, height, z, Content::Shape(shape), props),
        }
    }

    /// A glass surface whose body is `width` × `height` logical pixels; its shadow falls outside that.
    pub fn glass(&self, width: u32, height: u32, z: i32, glass: Glass, props: Props) -> GlassLayer {
        let id = self.allocate();
        GlassLayer {
            layer: self.create(id, width, height, z, Content::Glass(glass), props),
        }
    }

    /// A wallpaper the compositor animates. For the kinds that use an image, write the picture into
    /// [`WallpaperLayer::pixels_mut`] and `damage` it.
    pub fn wallpaper(
        &self,
        width: u32,
        height: u32,
        z: i32,
        wallpaper: Wallpaper,
        props: Props,
    ) -> io::Result<WallpaperLayer> {
        let id = self.allocate();
        let pixels = if wallpaper.kind.uses_image() {
            Some(self.pixels(id, width, height)?)
        } else {
            None
        };
        Ok(WallpaperLayer {
            layer: self.create(id, width, height, z, Content::Wallpaper(wallpaper), props),
            pixels,
        })
    }

    /// A 3D scene of `geometry` drawn into a `width` × `height` rectangle. The geometry is fixed for the layer's
    /// life; a new one is a new layer.
    pub fn mesh(
        &self,
        width: u32,
        height: u32,
        z: i32,
        mesh: Mesh,
        geometry: &MeshData,
        props: Props,
    ) -> io::Result<MeshLayer> {
        let bytes = geometry.encode().map_err(io::Error::other)?;
        let id = self.allocate();
        let path = self.file_of(mesh_path(id));
        let mut file = self.shm_file(&path, 0)?;
        file.write_all(&bytes)?;
        Ok(MeshLayer {
            layer: self.create(id, width, height, z, Content::Mesh(mesh), props),
            path,
        })
    }

    /// The frames of the producer's source `source` (the frame endpoint), drawn into a `width` × `height` rectangle straight
    /// from the buffers the producer fills. Nothing is written or copied here, whatever the picture's rate; the layer shows
    /// nothing until the source has a picture.
    pub fn live(
        &self,
        width: u32,
        height: u32,
        z: i32,
        source: SourceId,
        props: Props,
    ) -> LiveLayer {
        let id = self.allocate();
        LiveLayer {
            layer: self.create(id, width, height, z, Content::Live(Live { source }), props),
            source,
        }
    }
}

/// What every layer can do: be placed, animated, hang under a group, and be deleted when dropped.
pub struct Layer {
    id: LayerId,
    pub(crate) scene: Scene,
    /// The group it hangs under: its membership keeps the group on the compositor until this layer is gone.
    pub(crate) group: Cell<Option<GroupId>>,
    /// Where the app has put it (in the group's space under a group): what it was made with, then the last `set`, the target of
    /// the last `animate`, the `local` props of the last attach or detach. A layer the compositor is told to make again takes
    /// these up again ([`Layer::become_picture`]).
    pub(crate) props: Cell<Props>,
    /// Another handle has taken the compositor's layer over ([`Layer::become_picture`]): this one is left only to be dropped, and
    /// deletes nothing.
    handed_over: bool,
}

impl Layer {
    pub fn id(&self) -> LayerId {
        self.id
    }

    /// Places the layer. Under a group (see `Group::attach`) `props` are in the group's space.
    pub fn set(&self, props: Props) {
        self.props.set(props);
        self.scene
            .inner
            .borrow_mut()
            .push(Request::Set { id: self.id, props });
    }

    pub fn animate(&self, to: Props, duration_ms: u32, ease: Ease) {
        self.animate_stepped(to, duration_ms, ease, 0);
    }

    /// [`Layer::animate`] as an ambient motion (the weather's sky, which nobody is waiting for): the compositor wants a frame for it at
    /// most every `step_ms` and draws the layer where the motion is at that time, instead of at every refresh. 0 is `animate`.
    pub fn animate_stepped(&self, to: Props, duration_ms: u32, ease: Ease, step_ms: u32) {
        self.props.set(to);
        self.scene.inner.borrow_mut().push(Request::Animate {
            id: self.id,
            to,
            duration_ms,
            ease,
            step_ms,
        });
    }

    /// The group the layer hangs under, if it does.
    pub fn group(&self) -> Option<GroupId> {
        self.group.get()
    }

    /// Puts the layer back on the screen with `props` (in screen space), out of its group. Nothing if it hangs under none.
    pub fn detach(&self, props: Props) {
        if let Some(group) = self.group.take() {
            self.props.set(props);
            let mut inner = self.scene.inner.borrow_mut();
            inner.push(Request::Attach {
                id: self.id,
                group: None,
                local: props,
            });
            inner.release_member(group);
        }
    }

    fn push(&self, request: Request) {
        self.scene.inner.borrow_mut().push(request);
    }

    pub(crate) fn restyle(&self, content: Content) {
        self.push(Request::Restyle {
            id: self.id,
            content,
        });
    }

    /// Makes the compositor's layer this handle names a picture of `width` × `height` at depth `z`, in place, and returns the
    /// picture that holds it from now on: the same id, the same group and place in it, the same props, so that nothing which names
    /// the layer has to change. What is staged is a `Create` of the same id (the compositor replaces a layer that is made again,
    /// puts it above the layers of its depth made before, forgets its group and an animation it was in, which is why the layer is
    /// made where the animation ends) with the pixels of a file made for it, and, for a layer under a group, the `Attach` that puts
    /// it back there with the props it had; what was staged for the old content (quads, palette) is taken back. The count of live
    /// layers does not change, and a layer that was held back at the ceiling stays one: nothing is sent for it. This handle is
    /// spent: drop it, which deletes nothing. With `Err` (no file for the pixels) nothing has changed.
    pub(crate) fn become_picture(
        &mut self,
        width: u32,
        height: u32,
        z: i32,
    ) -> io::Result<ImageLayer> {
        let pixels = self.scene.pixels(self.id, width, height)?;
        let props = self.props.get();
        let layer = Layer {
            id: self.id,
            scene: self.scene.clone(),
            group: Cell::new(self.group.take()),
            props: Cell::new(props),
            handed_over: false,
        };
        self.handed_over = true;
        {
            let mut inner = self.scene.inner.borrow_mut();
            inner.unstage_glyphs(self.id);
            inner.push(Request::Create {
                id: self.id,
                width,
                height,
                z,
                content: Content::Image,
                props,
            });
            if let Some(group) = layer.group.get() {
                // Made again, the layer hangs under no group on the compositor; here it still does, so the two are made one again.
                let _ = inner.group_attach(group, &layer, props);
            }
        }
        Ok(ImageLayer { layer, pixels })
    }
}

impl Drop for Layer {
    fn drop(&mut self) {
        if self.handed_over {
            return;
        }
        let mut inner = self.scene.inner.borrow_mut();
        inner.delete_layer(self.id);
        // After the layer's own delete: a group that waited for its last member goes next.
        if let Some(group) = self.group.take() {
            inner.release_member(group);
        }
    }
}

/// A layer's shared-memory pixels. The file goes with it (after the next commit); the compositor's `Delete` is queued
/// by the layer.
struct Pixels {
    map: MmapMut,
    width: u32,
    height: u32,
    path: PathBuf,
    scene: Scene,
}

impl Pixels {
    fn damage(&self, layer: &Layer, rect: Option<Rect>) {
        let rect = rect.unwrap_or(Rect {
            x: 0.0,
            y: 0.0,
            w: self.width as f32,
            h: self.height as f32,
        });
        layer.push(Request::Damage { id: layer.id, rect });
    }
}

impl Drop for Pixels {
    fn drop(&mut self) {
        self.scene.retire(&self.path);
    }
}

pub struct ImageLayer {
    layer: Layer,
    pixels: Pixels,
}

impl ImageLayer {
    pub fn width(&self) -> u32 {
        self.pixels.width
    }

    pub fn height(&self) -> u32 {
        self.pixels.height
    }

    /// The layer's premultiplied RGBA pixels, `width * 4` bytes per row. Call `damage` after changing them.
    pub fn pixels_mut(&mut self) -> &mut [u8] {
        &mut self.pixels.map
    }

    /// The layer's pixels as they are, `width * 4` bytes per row.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels.map
    }

    /// Clears the pixels to transparent.
    pub fn clear(&mut self) {
        self.pixels.map.fill(0);
    }

    /// Tells the compositor `rect` (layer pixels) changed; the whole layer if `None`.
    pub fn damage(&self, rect: Option<Rect>) {
        self.pixels.damage(&self.layer, rect);
    }

    /// Lets go of the pages of the pixels this process has mapped, for a picture that is written and not touched again for a long
    /// while: they stay in the file, which is where the compositor reads them, and are mapped again if they are touched. The
    /// resident size of the app counts the mapped pages (1.8 MB for a wallpaper's picture).
    pub fn release_resident(&self) {
        // SAFETY: a shared mapping of a file: the pages are not lost by being unmapped, they are the file's.
        let _ = unsafe {
            self.pixels
                .map
                .unchecked_advise(memmap2::UncheckedAdvice::DontNeed)
        };
    }
}

impl std::ops::Deref for ImageLayer {
    type Target = Layer;
    fn deref(&self) -> &Layer {
        &self.layer
    }
}

pub struct ShapeLayer {
    layer: Layer,
}

impl ShapeLayer {
    pub fn reshape(&self, shape: Shape) {
        self.layer.restyle(Content::Shape(shape));
    }
}

impl std::ops::Deref for ShapeLayer {
    type Target = Layer;
    fn deref(&self) -> &Layer {
        &self.layer
    }
}

pub struct GlassLayer {
    layer: Layer,
}

impl GlassLayer {
    pub fn restyle(&self, glass: Glass) {
        self.layer.restyle(Content::Glass(glass));
    }
}

impl std::ops::Deref for GlassLayer {
    type Target = Layer;
    fn deref(&self) -> &Layer {
        &self.layer
    }
}

pub struct WallpaperLayer {
    layer: Layer,
    /// Present exactly when the kind uses an image.
    pixels: Option<Pixels>,
}

impl WallpaperLayer {
    /// The picture an image wallpaper warps (premultiplied RGBA, `width * 4` bytes per row); `None` for the
    /// kinds the compositor draws alone. Call `damage` after changing it.
    pub fn pixels_mut(&mut self) -> Option<&mut [u8]> {
        self.pixels.as_mut().map(|p| &mut p.map[..])
    }

    /// Tells the compositor `rect` of the picture changed; the whole picture if `None`. Nothing for the kinds
    /// without one.
    pub fn damage(&self, rect: Option<Rect>) {
        if let Some(pixels) = &self.pixels {
            pixels.damage(&self.layer, rect);
        }
    }

    /// Changes the motion or the kind. The new kind must use an image exactly when the old one did: the picture
    /// belongs to the layer.
    pub fn restyle(&self, wallpaper: Wallpaper) -> io::Result<()> {
        if wallpaper.kind.uses_image() != self.pixels.is_some() {
            return Err(io::Error::other(
                "a wallpaper cannot gain or lose its picture; create a new layer",
            ));
        }
        self.layer.restyle(Content::Wallpaper(wallpaper));
        Ok(())
    }

    /// Starts a ripple at (`x`, `y`), layer-local logical pixels (`Liquid` wallpapers).
    pub fn ripple(&self, x: f32, y: f32) {
        self.layer.push(Request::Ripple {
            id: self.layer.id,
            x,
            y,
        });
    }
}

impl std::ops::Deref for WallpaperLayer {
    type Target = Layer;
    fn deref(&self) -> &Layer {
        &self.layer
    }
}

/// The frames of a producer's source (the frame endpoint, `compositor_protocol::frames`), drawn into a rectangle by the
/// compositor straight from the buffers the producer fills: nothing is written here and nothing is copied, whatever the
/// picture's rate. The layer shows nothing until the source has a picture, and nothing again when the producer says so.
pub struct LiveLayer {
    layer: Layer,
    source: SourceId,
}

impl LiveLayer {
    /// The source the layer shows.
    pub fn source(&self) -> SourceId {
        self.source
    }
}

impl std::ops::Deref for LiveLayer {
    type Target = Layer;
    fn deref(&self) -> &Layer {
        &self.layer
    }
}

pub struct MeshLayer {
    layer: Layer,
    path: PathBuf,
}

impl MeshLayer {
    /// Changes the camera, the light or the look; the geometry stays.
    pub fn restyle(&self, mesh: Mesh) {
        self.layer.restyle(Content::Mesh(mesh));
    }
}

impl std::ops::Deref for MeshLayer {
    type Target = Layer;
    fn deref(&self) -> &Layer {
        &self.layer
    }
}

impl Drop for MeshLayer {
    fn drop(&mut self) {
        self.layer.scene.retire(&self.path);
    }
}

/// Keeps the event reader's end of a test's socket pair and the writer's: what a fake compositor needs to answer a hello
/// and then read what the scene sends.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use compositor_protocol::Event;

    /// A compositor stand-in that is as strict as the real one about the order of a connection: it answers
    /// only a correct Hello, and hands back the compositor's end for inspection.
    pub(crate) fn connect(options: SceneOptions) -> (Scene, UnixStream, Receiver<Event>) {
        let (app, mut comp) = UnixStream::pair().unwrap();
        let server = thread::spawn(move || {
            let mut buf = Vec::new();
            let hello = read_frame::<Request>(&mut comp, &mut buf).unwrap();
            assert_eq!(
                hello,
                Some(Request::Hello {
                    version: PROTOCOL_VERSION
                })
            );
            write_frame(
                &mut comp,
                &Event::Welcome {
                    version: PROTOCOL_VERSION,
                    panel_width: 480,
                    panel_height: 960,
                    refresh_hz: 59.64,
                    smooth_first_full: true,
                },
            )
            .unwrap();
            comp
        });
        let (scene, welcome, events) = Scene::over_with(app, options).unwrap();
        assert_eq!((welcome.panel_width, welcome.panel_height), (480, 960));
        // What the compositor says it can draw reaches the host (a compositor that says it cannot: the test below).
        assert!(welcome.smooth_first_full);
        let comp = server.join().unwrap();
        (scene, comp, events)
    }

    #[test]
    fn a_compositor_that_cannot_draw_full_says_so_and_the_host_is_told() {
        for can in [true, false] {
            let (app, mut comp) = UnixStream::pair().unwrap();
            let server = thread::spawn(move || {
                let mut buf = Vec::new();
                read_frame::<Request>(&mut comp, &mut buf).unwrap();
                write_frame(
                    &mut comp,
                    &Event::Welcome {
                        version: PROTOCOL_VERSION,
                        panel_width: 960,
                        panel_height: 480,
                        refresh_hz: 59.64,
                        smooth_first_full: can,
                    },
                )
                .unwrap();
                comp
            });
            let (scene, welcome, _events) = Scene::over(app).unwrap();
            assert_eq!(welcome.smooth_first_full, can);
            assert_eq!(scene.smooth_first_full(), can);
            let mut comp = server.join().unwrap();
            // A level the compositor cannot draw is refused here, and says so; the others go out whatever it said.
            use compositor_protocol::SmoothFirst;
            assert_eq!(scene.set_smooth_first(SmoothFirst::Full).is_ok(), can);
            scene.set_smooth_first(SmoothFirst::Invisible).unwrap();
            scene.try_commit().unwrap();
            let mut buf = Vec::new();
            let mut asked = Vec::new();
            loop {
                match read_frame::<Request>(&mut comp, &mut buf).unwrap() {
                    Some(Request::Commit { .. }) | None => break,
                    Some(request) => asked.push(request),
                }
            }
            let mut expected = Vec::new();
            if can {
                expected.push(Request::SmoothFirst {
                    level: SmoothFirst::Full,
                });
            }
            expected.push(Request::SmoothFirst {
                level: SmoothFirst::Invisible,
            });
            assert_eq!(asked, expected, "a compositor that can: {can}");
        }
    }

    /// The same, with a reader that returns the requests of one batch (up to its commit) from the compositor's end.
    pub(crate) fn fake_compositor_in(
        shm_dir: Option<PathBuf>,
    ) -> (Scene, impl FnMut() -> Vec<Request>) {
        let (scene, mut comp, _events) = connect(SceneOptions {
            shm_dir,
            ..SceneOptions::default()
        });
        comp.set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        let read_batch = move || {
            let mut out = Vec::new();
            let mut buf = Vec::new();
            loop {
                match read_frame::<Request>(&mut comp, &mut buf) {
                    Ok(Some(Request::Commit { .. })) => return out,
                    Ok(Some(r)) => out.push(r),
                    Ok(None) | Err(_) => return out,
                }
            }
        };
        (scene, read_batch)
    }

    /// A fresh directory for the files of layers and staging slots.
    pub(crate) fn tempdir() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("lumashow-ui-{}-{n}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Every request from the compositor's end until there is nothing more to read for a moment.
    pub(crate) fn drain(comp: &mut UnixStream) -> Vec<Request> {
        comp.set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let mut out = Vec::new();
        let mut buf = Vec::new();
        while let Ok(Some(r)) = read_frame::<Request>(comp, &mut buf) {
            out.push(r);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use compositor_protocol::{Fill, Rgba};

    fn fake_compositor() -> (Scene, impl FnMut() -> Vec<Request>) {
        fake_compositor_in(None)
    }

    fn shape() -> Shape {
        Shape {
            corner_radius: 8.0,
            fill: Fill::Solid(Rgba(1, 2, 3, 255)),
            border_width: 0.0,
            border: Rgba(0, 0, 0, 0),
        }
    }

    #[test]
    fn nothing_reaches_the_compositor_until_commit_and_then_all_of_it_at_once() {
        let (scene, mut batch) = fake_compositor();
        let card = scene.shape(100, 50, 1, shape(), Props::at(10.0, 20.0));
        card.animate(Props::at(0.0, 20.0), 200, Ease::Out);
        scene.try_commit().unwrap();
        let sent = batch();
        assert!(matches!(
            sent[0],
            Request::Create {
                width: 100,
                height: 50,
                z: 1,
                ..
            }
        ));
        assert!(matches!(
            sent[1],
            Request::Animate {
                duration_ms: 200,
                ..
            }
        ));
        assert_eq!(sent.len(), 2);
    }

    #[test]
    fn dropping_a_layer_deletes_it_and_its_pixels() {
        let (scene, mut batch) = fake_compositor();
        // Tests cannot write /run/lumashow; image layers are exercised on the device. Shapes cover the drop path.
        let card = scene.shape(10, 10, 0, shape(), Props::at(0.0, 0.0));
        let id = card.id();
        scene.try_commit().unwrap();
        batch();
        drop(card);
        scene.try_commit().unwrap();
        assert_eq!(batch(), vec![Request::Delete { id }]);
    }

    #[test]
    fn a_gpu_clock_hold_goes_out_at_once_and_not_with_the_next_commit() {
        let (scene, mut batch) = fake_compositor();
        let _card = scene.shape(100, 50, 1, shape(), Props::at(0.0, 0.0));
        scene.hold_gpu_clock(Some(416)).unwrap();
        scene.hold_gpu_clock(None).unwrap();
        scene.try_commit().unwrap();
        // Written before the commit that carries the layer staged ahead of them.
        let sent = batch();
        assert!(matches!(sent[0], Request::HoldGpuClock { mhz: Some(416) }));
        assert!(matches!(sent[1], Request::HoldGpuClock { mhz: None }));
        assert!(matches!(sent[2], Request::Create { .. }));
        assert_eq!(sent.len(), 3);
    }

    #[test]
    fn smooth_first_goes_out_at_once_as_the_one_request_for_all_three_levels() {
        use compositor_protocol::SmoothFirst;
        let (scene, mut batch) = fake_compositor();
        let _card = scene.shape(100, 50, 1, shape(), Props::at(0.0, 0.0));
        for level in SmoothFirst::ALL {
            scene.set_smooth_first(level).unwrap();
        }
        scene.try_commit().unwrap();
        let sent = batch();
        // In order, ahead of the commit that carries the layer staged before them; no request for the glass ideas goes with them.
        for (request, level) in sent.iter().zip(SmoothFirst::ALL) {
            assert_eq!(request, &Request::SmoothFirst { level });
        }
        assert!(matches!(sent[3], Request::Create { .. }));
        assert_eq!(sent.len(), 4, "{sent:?}");
    }

    #[test]
    fn a_layer_made_and_dropped_between_two_commits_is_never_sent() {
        let (scene, mut batch) = fake_compositor();
        let gone = scene.shape(100, 50, 1, shape(), Props::at(10.0, 20.0));
        gone.animate(Props::at(0.0, 20.0), 200, Ease::Out);
        gone.set(Props::at(5.0, 5.0));
        let kept = scene.shape(10, 10, 2, shape(), Props::at(0.0, 0.0));
        drop(gone);
        scene.try_commit().unwrap();
        // Of everything asked, only the layer that stayed reaches the compositor.
        let sent = batch();
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert!(matches!(&sent[0], Request::Create { id, .. } if *id == kept.id()));
        // A layer it has heard of is deleted the usual way, in the batch of the commit after.
        let id = kept.id();
        drop(kept);
        scene.try_commit().unwrap();
        assert_eq!(batch(), vec![Request::Delete { id }]);
    }

    #[test]
    fn a_layer_past_the_ceiling_is_held_back_with_everything_asked_of_it_and_room_comes_back_when_layers_go()
     {
        let (scene, mut batch) = fake_compositor();
        let mut layers: Vec<_> = (0..MAX_DRAWABLE_NODES)
            .map(|_| scene.shape(10, 10, 0, shape(), Props::at(0.0, 0.0)))
            .collect();
        assert_eq!(
            (scene.live_layers(), scene.layer_room()),
            (MAX_DRAWABLE_NODES, 0)
        );
        assert!(!scene.take_overflow());
        // One more: the compositor is never asked, nor for what follows (it would refuse each), and the builder is told.
        let extra = scene.shape(10, 10, 0, shape(), Props::at(0.0, 0.0));
        extra.set(Props::at(5.0, 5.0));
        extra.animate(Props::at(9.0, 9.0), 100, Ease::Out);
        assert!(scene.take_overflow());
        assert!(!scene.take_overflow(), "told once");
        scene.try_commit().unwrap();
        let sent = batch();
        assert_eq!(sent.len(), MAX_DRAWABLE_NODES, "{} sent", sent.len());
        assert!(
            sent.iter()
                .all(|r| matches!(r, Request::Create { id, .. } if *id != extra.id()))
        );
        // Letting go of it deletes nothing and frees nothing; letting go of a real one gives its room back.
        drop(extra);
        assert_eq!(scene.live_layers(), MAX_DRAWABLE_NODES);
        let gone = layers.pop().unwrap();
        let id = gone.id();
        drop(gone);
        assert_eq!(scene.layer_room(), 1);
        scene.try_commit().unwrap();
        assert_eq!(batch(), vec![Request::Delete { id }]);
        let again = scene.shape(10, 10, 0, shape(), Props::at(0.0, 0.0));
        assert!(!scene.take_overflow());
        drop(again);
        // The compositor's own refusal is the same news.
        scene.note_refusal();
        assert!(scene.take_overflow());
    }

    #[test]
    fn a_held_back_layer_made_inside_a_group_is_not_attached_to_it_either() {
        let (scene, mut batch) = fake_compositor();
        let _full: Vec<_> = (0..MAX_DRAWABLE_NODES)
            .map(|_| scene.shape(10, 10, 0, shape(), Props::at(0.0, 0.0)))
            .collect();
        let group = scene.group(None, crate::GroupPose::IDENTITY).unwrap();
        let held = scene.adopting(&group, || {
            scene.shape(10, 10, 0, shape(), Props::at(0.0, 0.0))
        });
        assert!(scene.take_overflow());
        scene.try_commit().unwrap();
        let sent = batch();
        assert!(
            !sent
                .iter()
                .any(|r| matches!(r, Request::Attach { id, .. } if *id == held.id())),
            "the compositor was told to attach a layer it never heard of"
        );
    }

    #[test]
    fn a_view_adopted_in_an_inert_group_hangs_its_layers_and_its_own_groups_under_it_and_shows_with_one_request()
     {
        let (scene, mut batch) = fake_compositor();
        let hold = scene.group(None, crate::GroupPose::IDENTITY).unwrap();
        hold.set_inert(true);
        let (layer, own, inside) = scene.adopting_view(&hold, || {
            let layer = scene.shape(10, 10, 0, shape(), Props::at(1.0, 2.0));
            // The view's own groups: one under no parent, one under a parent of its own.
            let own = scene.group(None, crate::GroupPose::at(5.0, 6.0)).unwrap();
            let inside = scene
                .group(Some(own.id()), crate::GroupPose::IDENTITY)
                .unwrap();
            own.attach(&layer, Props::at(3.0, 4.0)).unwrap();
            (layer, own, inside)
        });
        // Made after the view, a group hangs under nothing again, and so does a layer.
        let later = scene.group(None, crate::GroupPose::IDENTITY).unwrap();
        let free = scene.shape(10, 10, 0, shape(), Props::at(0.0, 0.0));
        scene.try_commit().unwrap();
        let sent = batch();
        let parents: Vec<_> = sent
            .iter()
            .filter_map(|r| match r {
                Request::GroupCreate { id, parent, .. } => Some((*id, *parent)),
                _ => None,
            })
            .collect();
        assert_eq!(
            parents,
            vec![
                (hold.wire_id(), None),
                (own.wire_id(), Some(hold.wire_id())),
                (inside.wire_id(), Some(own.wire_id())),
                (later.wire_id(), None),
            ],
            "{sent:?}"
        );
        let attached = |id| {
            sent.iter()
                .filter_map(|r| match r {
                    Request::Attach { id: i, group, .. } if *i == id => Some(*group),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        // Made under the hold from the first, then taken by the view's own group, which hangs under the hold.
        assert_eq!(
            attached(layer.id()),
            vec![Some(hold.wire_id()), Some(own.wire_id())]
        );
        assert!(attached(free.id()).is_empty(), "{sent:?}");
        // The view shows with one request, and nothing is made or sent for it.
        hold.set_inert(false);
        scene.try_commit().unwrap();
        let shown = batch();
        assert!(
            matches!(shown.as_slice(), [Request::GroupSet { id, .. }] if *id == hold.wire_id()),
            "{shown:?}"
        );
    }

    #[test]
    fn a_layer_made_a_picture_in_place_is_one_replacing_create_and_the_picture_deletes_it_once_where_the_old_handle_deletes_nothing()
     {
        let dir = tempdir();
        let (scene, mut batch) = fake_compositor_in(Some(dir.clone()));
        let ShapeLayer { mut layer } = scene.shape(100, 50, 2, shape(), Props::at(10.0, 20.0));
        let id = layer.id();
        scene.try_commit().unwrap();
        batch();
        layer.set(Props::at(11.0, 21.0));
        let live = scene.live_layers();
        let picture = layer.become_picture(80, 40, 2).unwrap();
        // The old handle is spent: letting go of it asks for nothing, not even a delete.
        drop(layer);
        assert_eq!((picture.id(), scene.live_layers()), (id, live));
        scene.try_commit().unwrap();
        assert_eq!(
            batch(),
            vec![
                Request::Set {
                    id,
                    props: Props::at(11.0, 21.0)
                },
                Request::Create {
                    id,
                    width: 80,
                    height: 40,
                    z: 2,
                    content: Content::Image,
                    props: Props::at(11.0, 21.0)
                },
            ]
        );
        // The picture has a file of its own for the id, and is what deletes the layer, once.
        let file = dir.join(format!("layer-{}.rgba", id.0));
        assert!(file.exists());
        drop(picture);
        let tag = scene.try_commit().unwrap().get();
        assert_eq!(batch(), vec![Request::Delete { id }]);
        scene.shown(tag);
        assert!(!file.exists());
    }

    #[test]
    fn the_file_of_a_dropped_picture_stays_until_the_compositor_has_shown_a_later_commit_and_a_picture_never_committed_is_never_sent()
     {
        let dir = tempdir();
        let (scene, mut batch) = fake_compositor_in(Some(dir.clone()));
        // Made, drawn on and dropped before any commit: the compositor is told nothing of it (it would find no file to
        // open, and refuse the layer and everything asked of it).
        let picture = scene.image(4, 4, 0, Props::at(0.0, 0.0)).unwrap();
        let file = dir.join(format!("layer-{}.rgba", picture.id().0));
        assert!(file.exists());
        picture.damage(None);
        drop(picture);
        let first = scene.try_commit().unwrap().get();
        assert!(batch().is_empty());
        // The file goes when the compositor says it has shown that commit, and not when the commit is written.
        assert!(file.exists());
        scene.shown(first - 1);
        assert!(file.exists());
        scene.shown(first);
        assert!(!file.exists());
        // One that was committed is deleted at the compositor; its file stays until a commit after the one that says so has
        // been shown, however many commits the app makes meanwhile.
        let picture = scene.image(4, 4, 0, Props::at(0.0, 0.0)).unwrap();
        let (id, file) = (
            picture.id(),
            dir.join(format!("layer-{}.rgba", picture.id().0)),
        );
        let made = scene.try_commit().unwrap().get();
        assert!(matches!(batch()[0], Request::Create { .. }));
        drop(picture);
        let ended = scene.try_commit().unwrap().get();
        assert_eq!(batch(), vec![Request::Delete { id }]);
        let later = scene.try_commit().unwrap().get();
        batch();
        // The compositor is still on the commit that made it: it has not necessarily opened the file yet.
        scene.shown(made);
        assert!(file.exists());
        scene.shown(ended);
        assert!(!file.exists());
        let _ = later;
        // Files left when the scene ends are removed with it.
        let last = scene.image(4, 4, 0, Props::at(0.0, 0.0)).unwrap();
        let file = dir.join(format!("layer-{}.rgba", last.id().0));
        drop(last);
        drop(scene);
        assert!(!file.exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn layers_get_distinct_ids() {
        let (scene, _) = fake_compositor();
        let a = scene.shape(1, 1, 0, shape(), Props::at(0.0, 0.0));
        let b = scene.shape(1, 1, 0, shape(), Props::at(0.0, 0.0));
        assert_ne!(a.id(), b.id());
    }

    /// The compositor reads nothing: the writer takes what the socket and its three segments hold, then answers `WouldBlock` at
    /// once, with the tag and the staged transaction as they were.
    #[test]
    fn a_compositor_that_reads_nothing_makes_a_commit_answer_would_block_at_once_and_keeps_the_transaction_and_the_tag()
     {
        let (scene, mut comp, _events) = connect(SceneOptions {
            send_buffer: Some(4096),
            ..SceneOptions::default()
        });
        let card = scene.shape(100, 50, 1, shape(), Props::at(0.0, 0.0));
        let mut accepted = Vec::new();
        let blocked_at = loop {
            // Each commit carries a few hundred bytes of restyles.
            for i in 0..8 {
                card.set(Props::at(i as f32, 0.0));
            }
            let began = std::time::Instant::now();
            match scene.try_commit() {
                Ok(tag) => accepted.push(tag.get()),
                Err(SubmitError::WouldBlock) => {
                    assert!(
                        began.elapsed() < Duration::from_millis(50),
                        "it answered at once"
                    );
                    break accepted.len();
                }
                Err(other) => panic!("{other}"),
            }
            assert!(accepted.len() < 5000, "the queue never filled");
        };
        assert!(scene.wants_writable(), "bytes wait for the socket");
        assert!(scene.has_pending(), "the staged transaction is intact");
        assert!(!scene.has_room());
        // The tag was not used up: the next commit that is taken carries the next number.
        assert_eq!(accepted, (1..=blocked_at as u64).collect::<Vec<_>>());
        // The reader catches up; the writer, told that the socket is writable, has room again; the transaction is taken whole.
        let reader = thread::spawn(move || {
            let mut buf = Vec::new();
            let mut commits = Vec::new();
            comp.set_read_timeout(Some(Duration::from_millis(300)))
                .unwrap();
            while let Ok(Some(r)) = read_frame::<Request>(&mut comp, &mut buf) {
                if let Request::Commit { tag } = r {
                    commits.push(tag);
                }
            }
            commits
        });
        let mut waited = 0;
        while !scene.writer_ready().unwrap() {
            assert!(scene.wait_for_room(Duration::from_millis(500)).is_ok());
            waited += 1;
            assert!(waited < 1000);
        }
        let tag = scene.try_commit().expect("room again").get();
        assert_eq!(tag, blocked_at as u64 + 1);
        let mut left = 0;
        while scene.wants_writable() {
            scene.wait_for_room(Duration::from_millis(500)).unwrap();
            left += 1;
            assert!(left < 1000);
        }
        let commits = reader.join().unwrap();
        assert_eq!(
            commits,
            (1..=tag).collect::<Vec<_>>(),
            "in order, none lost, none twice"
        );
    }

    /// Commits restyles until the writer says `WouldBlock`: the compositor (a test that has not started reading) is behind by the
    /// socket's buffer and the three segments. The tags that were taken, in order.
    fn fill_until_blocked(scene: &Scene, card: &ShapeLayer) -> Vec<u64> {
        let mut taken = Vec::new();
        loop {
            for i in 0..8 {
                card.set(Props::at(i as f32, 0.0));
            }
            match scene.try_commit() {
                Ok(tag) => taken.push(tag.get()),
                Err(SubmitError::WouldBlock) => return taken,
                Err(other) => panic!("{other}"),
            }
            assert!(taken.len() < 5000, "the queue never filled");
        }
    }

    /// A reader that collects every request in order until the connection is quiet.
    fn collect(mut comp: UnixStream) -> thread::JoinHandle<Vec<Request>> {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let mut all = Vec::new();
            comp.set_read_timeout(Some(Duration::from_millis(400)))
                .unwrap();
            while let Ok(Some(r)) = read_frame::<Request>(&mut comp, &mut buf) {
                all.push(r);
            }
            all
        })
    }

    #[test]
    fn input_overtakes_a_backlog_of_commits_on_a_real_socket_without_waiting_and_only_moves_are_replaced()
     {
        use compositor_protocol::{DrivePhase, MotionBounds, Units};
        let (scene, comp, _events) = connect(SceneOptions {
            send_buffer: Some(4096),
            ..SceneOptions::default()
        });
        let card = scene.shape(100, 50, 1, shape(), Props::at(0.0, 0.0));
        let motion = scene
            .motion(0.0, MotionBounds::range(0.0, 1000.0, Units::Pixels))
            .unwrap();
        // The track exists on the wire (the socket was empty): requests about it may go whenever.
        scene.try_commit().unwrap();
        let taken = fill_until_blocked(&scene, &card);
        assert!(taken.len() >= 2);
        // The compositor has read nothing. Input is written all the same, and never waits.
        let began = std::time::Instant::now();
        motion
            .drive(crate::motion::sample::down(1, 500.0, 1))
            .unwrap();
        for k in 0..300 {
            motion
                .drive(crate::motion::sample::moved(1, 500.0 - k as f32, 2 + k))
                .unwrap();
        }
        motion
            .drive(crate::motion::sample::up(1, 150.0, 400))
            .unwrap();
        motion
            .settle(
                None,
                None,
                compositor_protocol::Settle::CriticalSpring(compositor_protocol::SpringToken {
                    response_ms: 300.0,
                }),
            )
            .unwrap();
        assert!(
            began.elapsed() < Duration::from_millis(200),
            "300 samples into a socket nobody reads took {:?}",
            began.elapsed()
        );
        // Now the compositor reads.
        let reader = collect(comp);
        let mut left = 0;
        while scene.wants_writable() {
            scene.wait_for_room(Duration::from_millis(500)).unwrap();
            left += 1;
            assert!(left < 2000);
        }
        let wire = reader.join().unwrap();
        let at = |f: &dyn Fn(&Request) -> bool| wire.iter().position(f).expect("on the wire");
        let down = at(
            &|r| matches!(r, Request::MotionDrive { sample, .. } if sample.phase == DrivePhase::Down),
        );
        let last_commit =
            at(&|r| matches!(r, Request::Commit { tag } if *tag == *taken.last().unwrap()));
        assert!(
            down < last_commit,
            "the finger went ahead of the commits queued behind the socket: {down} vs {last_commit}"
        );
        let moves: Vec<f32> = wire
            .iter()
            .filter_map(|r| match r {
                Request::MotionDrive { sample, .. } if sample.phase == DrivePhase::Move => {
                    Some(sample.position)
                }
                _ => None,
            })
            .collect();
        assert_eq!(moves, vec![500.0 - 299.0], "only the newest move was kept");
        let up = at(
            &|r| matches!(r, Request::MotionDrive { sample, .. } if sample.phase == DrivePhase::Up),
        );
        let settle = at(&|r| matches!(r, Request::MotionSettle { .. }));
        assert!(down < up && up < settle, "down, up, settle in order");
        // And every commit that was taken arrived, once, in order.
        let commits: Vec<u64> = wire
            .iter()
            .filter_map(|r| match r {
                Request::Commit { tag } => Some(*tag),
                _ => None,
            })
            .collect();
        assert_eq!(commits, (1..=*taken.last().unwrap()).collect::<Vec<_>>());
    }

    #[test]
    fn what_was_asked_of_a_track_that_is_dropped_before_it_started_is_never_sent() {
        use compositor_protocol::{MotionBounds, Units};
        let (scene, comp, _events) = connect(SceneOptions {
            send_buffer: Some(4096),
            ..SceneOptions::default()
        });
        let card = scene.shape(100, 50, 1, shape(), Props::at(0.0, 0.0));
        let motion = scene
            .motion(0.0, MotionBounds::range(0.0, 1000.0, Units::Pixels))
            .unwrap();
        let wire_id = motion.wire_id();
        scene.try_commit().unwrap();
        fill_until_blocked(&scene, &card);
        motion
            .drive(crate::motion::sample::down(1, 5.0, 1))
            .unwrap();
        motion
            .drive(crate::motion::sample::moved(1, 6.0, 2))
            .unwrap();
        motion.drive(crate::motion::sample::up(1, 7.0, 3)).unwrap();
        drop(motion);
        let reader = collect(comp);
        // The deletion is staged for the next commit; the socket drains first.
        let mut left = 0;
        while scene.wants_writable() {
            scene.wait_for_room(Duration::from_millis(500)).unwrap();
            left += 1;
            assert!(left < 2000);
        }
        scene.try_commit().unwrap();
        while scene.wants_writable() {
            scene.wait_for_room(Duration::from_millis(500)).unwrap();
        }
        let wire = reader.join().unwrap();
        let drives: Vec<_> = wire
            .iter()
            .filter(|r| matches!(r, Request::MotionDrive { id, .. } if *id == wire_id))
            .collect();
        // The finger's landing had started when the track was dropped; the rest never reaches a track that is gone.
        assert!(drives.len() <= 1, "{drives:?}");
        let delete = wire
            .iter()
            .position(|r| matches!(r, Request::MotionDelete { id } if *id == wire_id))
            .expect("deleted");
        let last_drive = wire
            .iter()
            .rposition(|r| matches!(r, Request::MotionDrive { id, .. } if *id == wire_id));
        assert!(last_drive.is_none_or(|d| d < delete));
    }

    #[test]
    fn a_dead_compositor_is_an_io_error_not_a_panic_and_not_a_signal() {
        let (scene, comp, _events) = connect(SceneOptions::default());
        let card = scene.shape(10, 10, 0, shape(), Props::at(0.0, 0.0));
        drop(comp);
        card.set(Props::at(1.0, 1.0));
        // The first write may be taken by the kernel before it knows; keep going until it knows.
        let mut failed = None;
        for _ in 0..100 {
            card.set(Props::at(2.0, 2.0));
            match scene.try_commit() {
                Ok(_) => thread::sleep(Duration::from_millis(2)),
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }
        match failed {
            Some(SubmitError::Io(e)) => assert!(
                matches!(
                    e.kind(),
                    io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
                ),
                "{e}"
            ),
            other => panic!("{other:?}"),
        }
        assert!(
            scene.request_stats().is_err(),
            "and every later request says so"
        );
    }
}
