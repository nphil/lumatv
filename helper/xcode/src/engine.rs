//! The runtime around the scheduler. One thread, `lumaxcode-engine`, owns the child process and the clock: it feeds `sched::Core` what callers ask,
//! what the child reports and the time, and carries out what the core answers. Callers reach it through a channel, so a call from any thread returns
//! at once or after a bounded wait, never after a hung hardware call.

use crate::buffer::{Queue, Ring};
use crate::io::{self, ChildMsg, Heard, Threads, Writer};
use crate::proc::{ChildCfg, OsSpawn, Proc, Sig, Spawn};
use crate::proto::{self, Cfg};
use crate::sched::{Action, Core, Input, KEEP_ENDED, Spec, admit};
use crate::types::{
    Au, Clip, ClipError, Config, Event, Fault, H264Profile, JobId, JobSpec, JobState, Limits, MAX_RING_BYTES, Out, PushError, Reason, RecvError, Sink,
    Snapshot, Source, SourceCodec, StartError, Wait,
};
use parking_lot::{Condvar, Mutex};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Weak};
use std::thread;
use std::time::{Duration, Instant};

const STACK: usize = 256 << 10;
/// Events a subscriber may fall behind by; the engine never waits for one, so a slower reader loses the newest.
const SUBSCRIBER_QUEUE: usize = 64;
const SNAPSHOT_WAIT: Duration = Duration::from_millis(250);
/// A start that is waiting for the child is answered within the ready timeout plus a stop of the child it replaces; this is the margin on top.
const START_SLACK: Duration = Duration::from_secs(2);
const SHUTDOWN_WAIT: Duration = Duration::from_millis(1200);
/// How long the engine thread gives a finished child's pipe threads to end.
const IO_WAIT: Duration = Duration::from_millis(200);
const REAP_EVERY: Duration = Duration::from_millis(250);
const MIN_KBPS: u32 = 64;
const MAX_KBPS: u32 = 20_000;

enum Store {
    Queue(Queue),
    Ring(Ring),
}

impl Store {
    fn push(&mut self, out: Out) {
        match self {
            Self::Queue(queue) => queue.push(out),
            Self::Ring(ring) => ring.push(out),
        }
    }

    fn mark_gap(&mut self) {
        match self {
            Self::Queue(queue) => queue.mark_gap(),
            Self::Ring(ring) => ring.mark_gap(),
        }
    }
}

struct Inner {
    state: JobState,
    /// After a (re)start the child's first unit must be a key unit.
    want_key: bool,
    store: Store,
    /// The input of the child that has the job now.
    writer: Option<Arc<Writer>>,
    /// The child whose units are taken, 0 for none: a child being stopped must not deliver, and a new one must not hear of the old.
    epoch: u64,
}

struct JobShared {
    id: JobId,
    inner: Mutex<Inner>,
    /// A unit arrived or the state changed.
    moved: Condvar,
    units: AtomicU64,
    bytes: AtomicU64,
    /// What the owner last asked for (0: the job's own), and what the child made over the last statistics interval.
    want_kbps: AtomicU32,
    rate_kbps: AtomicU32,
}

fn take(inner: &mut Inner) -> Option<Out> {
    match &mut inner.store {
        Store::Queue(queue) => queue.pop(),
        Store::Ring(_) => None,
    }
}

/// One conversion. Dropping the handle stops the job, so the hardware goes back whatever the owner was doing.
pub struct Job {
    shared: Arc<JobShared>,
    tx: Sender<Msg>,
}

impl fmt::Debug for Job {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Job").field("id", &self.shared.id).finish_non_exhaustive()
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Job {
    pub fn id(&self) -> JobId {
        self.shared.id
    }

    pub fn state(&self) -> JobState {
        self.shared.inner.lock().state.clone()
    }

    /// Gives the child an access unit. Never waits: a unit that cannot be taken is the owner's to drop.
    pub fn push(&self, au: Au) -> Result<(), PushError> {
        let mut inner = self.shared.inner.lock();
        admit(&inner.state, inner.want_key, au.key)?;
        let Some(writer) = inner.writer.clone() else {
            return Err(PushError::Paused(Reason::Queued));
        };
        let key = au.key;
        writer.push_au(au)?;
        if key {
            inner.want_key = false;
        }
        Ok(())
    }

    /// Moves every unit that has been made into `out`. `Err` once the job has ended; the units made before that are still in `out`.
    pub fn pull(&self, out: &mut Vec<Out>) -> Result<(), Reason> {
        let mut inner = self.shared.inner.lock();
        while let Some(unit) = take(&mut inner) {
            out.push(unit);
        }
        match &inner.state {
            JobState::Ended(reason) => Err(reason.clone()),
            _ => Ok(()),
        }
    }

    pub fn recv_timeout(&self, wait: Duration) -> Result<Out, RecvError> {
        let deadline = Instant::now() + wait;
        let mut inner = self.shared.inner.lock();
        loop {
            if let Some(unit) = take(&mut inner) {
                return Ok(unit);
            }
            if let JobState::Ended(reason) = &inner.state {
                return Err(RecvError::Ended(reason.clone()));
            }
            if self.shared.moved.wait_until(&mut inner, deadline).timed_out() {
                return take(&mut inner).ok_or(RecvError::Timeout);
            }
        }
    }

    /// The next unit is an IDR. A job that is not running has nothing to ask: its next child starts with one.
    pub fn force_idr(&self) {
        if let Some(writer) = &self.shared.inner.lock().writer {
            writer.idr();
        }
    }

    /// The bit rate to aim for from now on, kept across restarts.
    pub fn set_kbps(&self, kbps: u32) {
        let kbps = kbps.clamp(MIN_KBPS, MAX_KBPS);
        self.shared.want_kbps.store(kbps, Ordering::Relaxed);
        if let Some(writer) = &self.shared.inner.lock().writer {
            writer.bitrate(kbps);
        }
    }

    pub fn stop(&self) {
        // the engine may be gone already, and then there is nothing to stop
        let _ = self.tx.send(Msg::Stop(self.shared.id));
    }
}

enum Msg {
    Start { spec: JobSpec, reply: Sender<Result<Job, StartError>> },
    Stop(JobId),
    Hold { key: String, reason: Option<String> },
    Subscribe(SyncSender<Event>),
    Snapshot(Sender<Snapshot>),
    Child { epoch: u64, msg: ChildMsg },
    Shutdown,
}

struct Handle {
    tx: Sender<Msg>,
    origin: Instant,
    limits: Limits,
    last_tick: Arc<AtomicU64>,
    jobs: Arc<Mutex<HashMap<JobId, Weak<JobShared>>>>,
    done: Mutex<Option<Receiver<()>>>,
}

impl Handle {
    fn shutdown(&self) {
        let Some(done) = self.done.lock().take() else { return };
        let _ = self.tx.send(Msg::Shutdown);
        // a child that does not die is parked by the engine within `term_grace + kill_wait`; a stuck engine thread is left behind rather than waited for
        let _ = done.recv_timeout(SHUTDOWN_WAIT);
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The scheduler of the TV's one video encoder. Cheap to clone; the last clone dropped shuts it down.
#[derive(Clone)]
pub struct Engine {
    handle: Arc<Handle>,
}

impl Engine {
    /// Starts the engine thread. `config.child` is run as `<child> xcode` when a job needs it.
    pub fn new(config: Config) -> std::io::Result<Self> {
        Self::with_spawner(config, Arc::new(OsSpawn))
    }

    /// The same with the children made by `spawner`: tests run the engine against scripted ones.
    pub fn with_spawner(config: Config, spawner: Arc<dyn Spawn>) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let origin = Instant::now();
        let last_tick = Arc::new(AtomicU64::new(0));
        let jobs = Arc::new(Mutex::new(HashMap::new()));
        let limits = config.limits.clone();
        let runtime = Runtime::new(config, spawner, tx.clone(), origin, Arc::clone(&last_tick), Arc::clone(&jobs));
        thread::Builder::new().name("lumaxcode-engine".to_owned()).stack_size(STACK).spawn(move || {
            runtime.run(&rx);
            let _ = done_tx.send(());
        })?;
        Ok(Self { handle: Arc::new(Handle { tx, origin, limits, last_tick, jobs, done: Mutex::new(Some(done_rx)) }) })
    }

    /// Starts a job. A job with `Wait::Fail` returns once its child is ready, or with the reason it cannot run; one with `Wait::Queue` returns at once.
    pub fn start(&self, spec: JobSpec) -> Result<Job, StartError> {
        let (reply, answer) = mpsc::channel();
        let gone = || StartError::Setup(Fault::Failed("the converter engine is not running".to_owned()));
        self.handle.tx.send(Msg::Start { spec, reply }).map_err(|_| gone())?;
        let lim = &self.handle.limits;
        match answer.recv_timeout(lim.ready + lim.term_grace + lim.kill_wait + START_SLACK) {
            Ok(result) => result,
            Err(RecvTimeoutError::Disconnected) => Err(gone()),
            Err(RecvTimeoutError::Timeout) => Err(StartError::Setup(Fault::Stalled("the converter engine did not answer".to_owned()))),
        }
    }

    /// Closes or opens the gate for `key`: a held engine runs no job. `None` releases.
    pub fn hold(&self, key: &str, reason: Option<&str>) {
        let _ = self.handle.tx.send(Msg::Hold { key: key.to_owned(), reason: reason.map(str::to_owned) });
    }

    pub fn stop(&self, job: JobId) {
        let _ = self.handle.tx.send(Msg::Stop(job));
    }

    /// Events from now on. A reader that falls 64 behind loses the newest.
    pub fn subscribe(&self) -> Receiver<Event> {
        let (tx, rx) = mpsc::sync_channel(SUBSCRIBER_QUEUE);
        let _ = self.handle.tx.send(Msg::Subscribe(tx));
        rx
    }

    /// Where everything is. An engine that does not answer within 250 ms is stuck: the snapshot is then an empty one saying for how long.
    pub fn snapshot(&self) -> Snapshot {
        let (reply, answer) = mpsc::channel();
        if self.handle.tx.send(Msg::Snapshot(reply)).is_ok() {
            if let Ok(snapshot) = answer.recv_timeout(SNAPSHOT_WAIT) {
                return snapshot;
            }
        }
        let last = Duration::from_millis(self.handle.last_tick.load(Ordering::Relaxed));
        Snapshot { last_tick_ms: millis(self.handle.origin.elapsed().saturating_sub(last)), ..Snapshot::default() }
    }

    /// A copy of the newest `seconds` of a ring job's units, from a key unit. Takes no turn of the engine thread: a ring can be read while it is busy.
    pub fn save_last(&self, job: JobId, seconds: u32) -> Result<Clip, ClipError> {
        let shared = self.handle.jobs.lock().get(&job).and_then(Weak::upgrade).ok_or(ClipError::NoSuchJob)?;
        let inner = shared.inner.lock();
        match &inner.store {
            Store::Ring(ring) => ring.clip(seconds),
            Store::Queue(_) => Err(ClipError::NotARing),
        }
    }

    /// Ends every job, stops the child and waits for that for at most 1.2 s. Later calls are refused.
    pub fn shutdown(&self) {
        self.handle.shutdown();
    }
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// What the child is told for `spec`, or why this job is not one it can do.
fn cfg_of(spec: &JobSpec) -> Result<Cfg, StartError> {
    let Source::Units { codec, width, height } = &spec.source else {
        return Err(StartError::Unsupported("the converter takes access units, not the screen".to_owned()));
    };
    if let Sink::Ring { max_bytes, pts_hz, .. } = &spec.sink {
        if *max_bytes > MAX_RING_BYTES {
            return Err(StartError::Unsupported(format!("a ring of {max_bytes} bytes is more than the {MAX_RING_BYTES} the engine keeps")));
        }
        if *pts_hz == 0 {
            return Err(StartError::Unsupported("a ring needs the unit of the pts it is given".to_owned()));
        }
    }
    let out = &spec.output;
    Ok(Cfg {
        version: proto::VERSION,
        codec: match codec {
            SourceCodec::Mpeg2 => proto::CODEC_MPEG2,
            SourceCodec::H264 => proto::CODEC_H264,
        },
        src_w: *width,
        src_h: *height,
        out_w: out.width,
        out_h: out.height,
        keep_every: out.keep_every,
        fps: out.fps,
        kbps: out.kbps,
        gop: out.gop,
        flags: if out.cbr { proto::CFG_CBR } else { 0 },
        profile: match out.profile {
            H264Profile::High => proto::PROFILE_HIGH,
            H264Profile::Main => proto::PROFILE_MAIN,
            H264Profile::Baseline => proto::PROFILE_BASELINE,
        },
        ..Cfg::default()
    })
}

/// A start that failed after it was accepted, as the error `Engine::start` answers.
fn start_error(reason: &Reason) -> StartError {
    match reason {
        Reason::Fault(Fault::Wedged(why)) => StartError::Wedged(why.clone()),
        Reason::Fault(fault) => StartError::Setup(fault.clone()),
        Reason::Held(why) => StartError::Held(why.clone()),
        Reason::Preempted { .. } | Reason::Replaced => StartError::Busy(reason.to_string()),
        Reason::Queued | Reason::Restarting { .. } | Reason::Stopped | Reason::Shutdown => StartError::Setup(Fault::Failed(reason.to_string())),
    }
}

struct Pending {
    cfg: Cfg,
    spec: JobSpec,
    reply: Sender<Result<Job, StartError>>,
}

struct JobRt {
    cfg: Cfg,
    shared: Arc<JobShared>,
    /// The job's bytes at the last statistics report.
    seen_bytes: u64,
}

struct ChildRt {
    epoch: u64,
    job: JobId,
    proc: Box<dyn Proc>,
    writer: Arc<Writer>,
    heard: Arc<Heard>,
    last_heard: Option<Duration>,
    /// What the child last wrote to its log.
    tail: Arc<Mutex<VecDeque<String>>>,
    /// Stopping, or its output ended or broke the protocol: it is expected to exit now, so it is asked often.
    leaving: bool,
    polled: Duration,
}

struct Runtime {
    core: Core,
    child_path: PathBuf,
    lim: Limits,
    spawner: Arc<dyn Spawn>,
    tx: Sender<Msg>,
    origin: Instant,
    last_tick: Arc<AtomicU64>,
    registry: Arc<Mutex<HashMap<JobId, Weak<JobShared>>>>,
    threads: Arc<Threads>,
    jobs: HashMap<JobId, JobRt>,
    ended: VecDeque<JobId>,
    pending: HashMap<u64, Pending>,
    /// Jobs that answer only once their child is ready.
    answers: HashMap<JobId, (Sender<Result<Job, StartError>>, Job)>,
    next_ticket: u64,
    subscribers: Vec<SyncSender<Event>>,
    child: Option<ChildRt>,
    last_tail: Arc<Mutex<VecDeque<String>>>,
    /// Children that outlived SIGKILL, kept where they can still be reaped.
    graveyard: Vec<Box<dyn Proc>>,
    slow_poll: Duration,
    fast_poll: Duration,
    stats_at: Duration,
    stats_prev: Duration,
    reap_at: Duration,
}

impl Runtime {
    fn new(
        config: Config,
        spawner: Arc<dyn Spawn>,
        tx: Sender<Msg>,
        origin: Instant,
        last_tick: Arc<AtomicU64>,
        registry: Arc<Mutex<HashMap<JobId, Weak<JobShared>>>>,
    ) -> Self {
        let lim = config.limits;
        // the silence watchdog sees a child that speaks through a timestamp, so the loop looks at it often enough for `silence` to mean what it says
        let slow_poll = (lim.silence / 4).clamp(Duration::from_millis(5), Duration::from_millis(250));
        let fast_poll = (lim.term_grace / 8).clamp(Duration::from_millis(1), Duration::from_millis(20));
        Self {
            core: Core::new(lim.clone()),
            child_path: config.child,
            stats_at: lim.stats_every,
            lim,
            spawner,
            tx,
            origin,
            last_tick,
            registry,
            threads: Arc::new(Threads::default()),
            jobs: HashMap::new(),
            ended: VecDeque::new(),
            pending: HashMap::new(),
            answers: HashMap::new(),
            next_ticket: 0,
            subscribers: Vec::new(),
            child: None,
            last_tail: Arc::new(Mutex::new(VecDeque::new())),
            graveyard: Vec::new(),
            slow_poll,
            fast_poll,
            stats_prev: Duration::ZERO,
            reap_at: Duration::ZERO,
        }
    }

    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    fn run(mut self, rx: &Receiver<Msg>) {
        loop {
            let now = self.now();
            self.last_tick.store(millis(now), Ordering::Relaxed);
            let msg = match self.wait(now) {
                Some(wait) => rx.recv_timeout(wait),
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match msg {
                Ok(msg) => self.handle(msg),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.turn();
            if self.core.finished() {
                break;
            }
        }
        // a dead child's pipes are closed, so its threads end on their own; a parked child keeps its pipes and its threads are left to it
        if !self.threads.wait_idle(Instant::now() + IO_WAIT) {
            log::warn!("lumaxcode: {} pipe threads were still running at shutdown", self.threads.live());
        }
    }

    /// How long the loop may sleep: until the earliest thing it has to look at. `None`: until a message comes.
    fn wait(&self, now: Duration) -> Option<Duration> {
        let mut due = self.core.next_deadline().map(|at| at.saturating_sub(now));
        let mut within = |d: Duration| due = Some(due.map_or(d, |x| x.min(d)));
        if let Some(child) = &self.child {
            within(if child.leaving { self.fast_poll } else { self.slow_poll });
            within(self.stats_at.saturating_sub(now));
        }
        if !self.graveyard.is_empty() {
            within(REAP_EVERY);
        }
        // a deadline already due that the core did not clear must not spin the loop
        due.map(|d| d.max(Duration::from_millis(1)))
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Start { spec, reply } => self.start(spec, reply),
            Msg::Stop(job) => self.input(Input::Stop { job }),
            Msg::Hold { key, reason } => self.input(Input::Hold { key, reason }),
            Msg::Subscribe(events) => self.subscribers.push(events),
            Msg::Snapshot(reply) => {
                let _ = reply.send(self.snapshot());
            }
            Msg::Child { epoch, msg } => self.child_msg(epoch, msg),
            Msg::Shutdown => self.input(Input::Shutdown),
        }
    }

    fn start(&mut self, spec: JobSpec, reply: Sender<Result<Job, StartError>>) {
        let cfg = match cfg_of(&spec) {
            Ok(cfg) => cfg,
            Err(error) => {
                let _ = reply.send(Err(error));
                return;
            }
        };
        self.next_ticket += 1;
        let ticket = self.next_ticket;
        let core = Spec { owner: spec.owner.clone(), priority: spec.priority, wait: spec.wait, units: true };
        self.pending.insert(ticket, Pending { cfg, spec, reply });
        self.input(Input::Start { ticket, spec: core });
    }

    fn child_msg(&mut self, epoch: u64, msg: ChildMsg) {
        let input = match msg {
            ChildMsg::Ready => Input::Ready { epoch },
            ChildMsg::Error { status, text } => Input::ChildError { epoch, status, text },
            ChildMsg::Protocol(why) => Input::Protocol { epoch, why },
            ChildMsg::OutClosed => Input::OutClosed { epoch },
            ChildMsg::ErrClosed => return,
        };
        if !matches!(input, Input::Ready { .. }) {
            if let Some(child) = self.child.as_mut().filter(|c| c.epoch == epoch) {
                child.leaving = true;
            }
        }
        self.input(input);
    }

    fn input(&mut self, input: Input) {
        self.drain(VecDeque::from([input]));
    }

    /// Feeds the core every input, and the ones its actions lead to, until nothing is left.
    fn drain(&mut self, mut queue: VecDeque<Input>) {
        while let Some(input) = queue.pop_front() {
            let actions = self.core.step(self.now(), input);
            for action in actions {
                self.act(action, &mut queue);
            }
        }
    }

    /// One turn of the loop: what the child did that no message says (it spoke, it exited), the clock, the reports.
    fn turn(&mut self) {
        let now = self.now();
        let mut queue = VecDeque::new();
        let mut gone = None;
        if let Some(child) = self.child.as_mut() {
            if let Some(at) = child.heard.get() {
                if child.last_heard != Some(at) {
                    child.last_heard = Some(at);
                    queue.push_back(Input::Heard { epoch: child.epoch, at });
                }
            }
            if child.leaving || now.saturating_sub(child.polled) >= self.slow_poll {
                child.polled = now;
                gone = child.proc.try_wait();
            }
        }
        if let Some(exit) = gone {
            if let Some(child) = self.child.take() {
                // the child's end of the pipe is gone: the input thread would wait for a unit that nobody takes
                child.writer.close();
                let last_line = child.tail.lock().back().cloned();
                queue.push_back(Input::Exited { epoch: child.epoch, exit, last_line });
            }
        }
        queue.push_back(Input::Tick);
        self.drain(queue);
        self.reap(now);
        self.stats(now);
    }

    fn reap(&mut self, now: Duration) {
        if !self.graveyard.is_empty() && now >= self.reap_at {
            self.reap_at = now + REAP_EVERY;
            self.graveyard.retain_mut(|proc| proc.try_wait().is_none());
        }
    }

    /// `Event::Bytes` and `Event::Dropped` for the job that is running, every `stats_every`.
    fn stats(&mut self, now: Duration) {
        if now < self.stats_at {
            return;
        }
        let since = now.saturating_sub(self.stats_prev);
        self.stats_prev = now;
        self.stats_at = now + self.lim.stats_every;
        let Some(rt) = self.child.as_ref().and_then(|c| self.jobs.get_mut(&c.job)) else { return };
        let (dropped, running) = {
            let mut inner = rt.shared.inner.lock();
            let dropped = match &mut inner.store {
                Store::Queue(queue) => queue.take_dropped(),
                Store::Ring(_) => 0,
            };
            (dropped, inner.state == JobState::Running)
        };
        if !running {
            return;
        }
        let (job, units, bytes) = (rt.shared.id, rt.shared.units.load(Ordering::Relaxed), rt.shared.bytes.load(Ordering::Relaxed));
        // bits per millisecond are kilobits per second
        let kbps = u32::try_from(u128::from(bytes.saturating_sub(rt.seen_bytes)) * 8 / since.as_millis().max(1)).unwrap_or(u32::MAX);
        rt.seen_bytes = bytes;
        rt.shared.rate_kbps.store(kbps, Ordering::Relaxed);
        if dropped > 0 {
            self.emit(Event::Dropped { job, units: dropped });
        }
        self.emit(Event::Bytes { job, units, bytes, kbps });
    }

    fn snapshot(&self) -> Snapshot {
        let mut snapshot = self.core.snapshot();
        for info in &mut snapshot.jobs {
            if let Some(rt) = self.jobs.get(&info.id) {
                info.units = rt.shared.units.load(Ordering::Relaxed);
                info.bytes = rt.shared.bytes.load(Ordering::Relaxed);
                info.kbps = rt.shared.rate_kbps.load(Ordering::Relaxed);
                if let Store::Ring(ring) = &rt.shared.inner.lock().store {
                    info.ring_bytes = ring.bytes();
                }
            }
        }
        snapshot.child_pid = self.child.as_ref().map(|c| c.proc.pid());
        snapshot.child_log = self.last_tail.lock().iter().cloned().collect();
        snapshot
    }

    fn act(&mut self, action: Action, queue: &mut VecDeque<Input>) {
        match action {
            Action::Accepted { ticket, job } => self.accepted(ticket, job),
            Action::Refused { ticket, error } => {
                if let Some(pending) = self.pending.remove(&ticket) {
                    let _ = pending.reply.send(Err(error));
                }
            }
            Action::Spawn { epoch, job, resume } => self.spawn(epoch, job, resume, queue),
            Action::Detach { epoch, job } => self.detach(epoch, job),
            Action::Term { epoch } => self.signal(epoch, Sig::Term),
            Action::Kill { epoch } => self.signal(epoch, Sig::Kill),
            Action::Park { epoch } => {
                if let Some(child) = self.child.take_if(|c| c.epoch == epoch) {
                    child.writer.close();
                    self.graveyard.push(child.proc);
                }
            }
            Action::State { job, state } => self.state(job, state),
            Action::Emit(event) => self.emit(event),
        }
    }

    fn accepted(&mut self, ticket: u64, id: JobId) {
        let Some(Pending { cfg, spec, reply }) = self.pending.remove(&ticket) else { return };
        let store = match spec.sink {
            Sink::Deliver => Store::Queue(Queue::new(self.lim.queue_units, self.lim.queue_bytes)),
            Sink::Ring { max_bytes, max_seconds, pts_hz } => {
                Store::Ring(Ring::new(if max_bytes == 0 { self.lim.ring_bytes } else { max_bytes }, max_seconds, pts_hz))
            }
        };
        let shared = Arc::new(JobShared {
            id,
            inner: Mutex::new(Inner { state: JobState::Queued, want_key: true, store, writer: None, epoch: 0 }),
            moved: Condvar::new(),
            units: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            want_kbps: AtomicU32::new(0),
            rate_kbps: AtomicU32::new(0),
        });
        {
            let mut registry = self.registry.lock();
            registry.retain(|_, job| job.strong_count() > 0);
            registry.insert(id, Arc::downgrade(&shared));
        }
        let job = Job { shared: Arc::clone(&shared), tx: self.tx.clone() };
        self.jobs.insert(id, JobRt { cfg, shared, seen_bytes: 0 });
        match spec.wait {
            Wait::Queue => {
                let _ = reply.send(Ok(job));
            }
            Wait::Fail => {
                self.answers.insert(id, (reply, job));
            }
        }
    }

    fn spawn(&mut self, epoch: u64, job: JobId, resume: bool, queue: &mut VecDeque<Input>) {
        let Some(rt) = self.jobs.get(&job) else {
            queue.push_back(Input::SpawnFailed { epoch, why: "the job is gone".to_owned() });
            return;
        };
        let mut cfg = rt.cfg;
        match rt.shared.want_kbps.load(Ordering::Relaxed) {
            0 => {}
            kbps => cfg.kbps = kbps,
        }
        let shared = Arc::clone(&rt.shared);
        let mut proc = match self.spawner.spawn(&self.child_path, &ChildCfg { job, cfg }) {
            Ok(proc) => proc,
            Err(e) => {
                queue.push_back(Input::SpawnFailed { epoch, why: format!("the converter did not start: {e}") });
                return;
            }
        };
        let writer = Arc::new(Writer::new(&cfg));
        let heard = Arc::new(Heard::new(self.origin));
        let tail = Arc::new(Mutex::new(VecDeque::new()));
        let pid = proc.pid();
        {
            let mut inner = shared.inner.lock();
            inner.epoch = epoch;
            inner.writer = Some(Arc::clone(&writer));
            if resume {
                inner.store.mark_gap();
            }
        }
        let tell = |tx: &Sender<Msg>| {
            let tx = tx.clone();
            move |msg| {
                let _ = tx.send(Msg::Child { epoch, msg });
            }
        };
        let streams = (proc.stdin(), proc.stdout(), proc.stderr());
        let started = match streams {
            (Some(mut stdin), Some(mut stdout), Some(mut stderr)) => {
                let (input, output, errors) = (Arc::clone(&writer), (Arc::clone(&heard), Arc::clone(&shared), tell(&self.tx)), (Arc::clone(&tail), tell(&self.tx)));
                io::spawn(&self.threads, "xcode-in", move || io::run_writer(&input, &mut *stdin))
                    .and_then(|()| {
                        let (heard, shared, emit) = output;
                        io::spawn(&self.threads, "xcode-out", move || {
                            let deliver = |out: Out| {
                                let len = out.data.len() as u64;
                                let mut inner = shared.inner.lock();
                                if inner.epoch != epoch {
                                    return;
                                }
                                inner.store.push(out);
                                shared.units.fetch_add(1, Ordering::Relaxed);
                                shared.bytes.fetch_add(len, Ordering::Relaxed);
                                drop(inner);
                                shared.moved.notify_all();
                            };
                            io::run_out(&mut *stdout, &heard, deliver, emit);
                        })
                    })
                    .and_then(|()| {
                        let (tail, emit) = errors;
                        io::spawn(&self.threads, "xcode-err", move || io::run_err(&mut *stderr, pid, &tail, emit))
                    })
                    .map_err(|e| format!("a thread of the converter's pipes did not start: {e}"))
            }
            _ => Err("the converter's pipes are missing".to_owned()),
        };
        self.last_tail = Arc::clone(&tail);
        self.child = Some(ChildRt { epoch, job, proc, writer, heard, last_heard: None, tail, leaving: false, polled: self.now() });
        if let Err(why) = started {
            queue.push_back(Input::Protocol { epoch, why });
        }
    }

    fn detach(&mut self, epoch: u64, job: JobId) {
        if let Some(rt) = self.jobs.get(&job) {
            let mut inner = rt.shared.inner.lock();
            if inner.epoch == epoch {
                inner.epoch = 0;
                if let Some(writer) = inner.writer.take() {
                    writer.close();
                }
            }
        }
    }

    fn signal(&mut self, epoch: u64, sig: Sig) {
        if let Some(child) = self.child.as_mut().filter(|c| c.epoch == epoch) {
            child.proc.signal(sig);
            child.leaving = true;
        }
    }

    fn state(&mut self, job: JobId, state: JobState) {
        let Some(rt) = self.jobs.get(&job) else { return };
        let mut inner = rt.shared.inner.lock();
        if state == JobState::Running && inner.state != JobState::Running {
            inner.want_key = true;
        }
        inner.state = state;
        drop(inner);
        rt.shared.moved.notify_all();
    }

    fn emit(&mut self, event: Event) {
        match &event {
            Event::Started { job, .. } => {
                if let Some((reply, handle)) = self.answers.remove(job) {
                    let _ = reply.send(Ok(handle));
                }
            }
            Event::Ended { job, reason } => {
                if let Some((reply, _handle)) = self.answers.remove(job) {
                    let _ = reply.send(Err(start_error(reason)));
                }
                self.retire(*job);
            }
            _ => {}
        }
        self.subscribers.retain(|events| !matches!(events.try_send(event.clone()), Err(TrySendError::Disconnected(_))));
    }

    /// Forgets the oldest ended jobs, as the core does.
    fn retire(&mut self, job: JobId) {
        self.ended.push_back(job);
        while self.ended.len() > KEEP_ENDED {
            if let Some(old) = self.ended.pop_front() {
                self.jobs.remove(&old);
            }
        }
    }
}
