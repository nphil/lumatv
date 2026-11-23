//! The engine against children that run here: an echo child on real pipes (every unit in comes out again) for the data path, and `/bin/sh` children
//! for what a process does to its supervisor: never ready, silent, deaf to SIGTERM, failing, hanging the hardware, flooding its log.

use lumaxcode::proto::{self, Command, Evt, Ready};
use lumaxcode::{
    Au, ChildCfg, ClipError, Config, Engine, Event, Exit, Fault, H264Profile, JobSpec, JobState, Limits, MAX_RING_BYTES, OsSpawn, Output, Priority, Proc,
    PushError, Reason, RecvError, ScreenPoint, Sig, Sink, Snapshot, Source, SourceCodec, Spawn, StartError, Wait,
};
use parking_lot::Mutex;
use std::fs::{self, File, Permissions};
use std::io::{self, Read, Write};
use std::os::fd::FromRawFd;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(5);

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn output() -> Output {
    Output { width: 0, height: 0, keep_every: 1, fps: 30, kbps: 3500, cbr: true, gop: 60, profile: H264Profile::High }
}

fn spec(priority: Priority, wait: Wait, sink: Sink) -> JobSpec {
    let source = Source::Units { codec: SourceCodec::Mpeg2, width: 1920, height: 1088 };
    JobSpec { owner: "test".to_owned(), priority, source, output: output(), wait, sink }
}

fn live() -> JobSpec {
    spec(Priority::Live, Wait::Fail, Sink::Deliver)
}

fn recording() -> JobSpec {
    spec(Priority::Recording, Wait::Queue, Sink::Deliver)
}

fn replay_ring() -> JobSpec {
    spec(Priority::Replay, Wait::Queue, Sink::Ring { max_bytes: 1 << 20, max_seconds: 0, pts_hz: 90_000 })
}

fn au(pts: u64, key: bool) -> Au {
    Au { pts, key, data: vec![0, 0, 1, u8::from(key)] }
}

/// The TV's timings scaled down so a stop that has to be forced takes milliseconds.
fn quick() -> Limits {
    Limits { term_grace: ms(60), kill_wait: ms(300), stats_every: ms(20), ..Limits::default() }
}

fn engine(spawner: Arc<dyn Spawn>, limits: Limits) -> Engine {
    Engine::with_spawner(Config { child: PathBuf::from("/lumaxcode/test/child"), limits }, spawner).unwrap()
}

/// The first event `want` accepts.
fn wait_for(events: &Receiver<Event>, want: impl Fn(&Event) -> bool) -> Event {
    let deadline = Instant::now() + WAIT;
    loop {
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(event) if want(&event) => return event,
            Ok(_) => {}
            Err(e) => panic!("the event never came: {e}"),
        }
    }
}

/// Asks for snapshots until one satisfies `done`.
fn until(engine: &Engine, done: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let deadline = Instant::now() + WAIT;
    loop {
        let snapshot = engine.snapshot();
        if done(&snapshot) {
            return snapshot;
        }
        assert!(Instant::now() < deadline, "timed out; the engine is at {snapshot:?}");
        thread::sleep(ms(1));
    }
}

/// What a job ends with, for a start that may fail or may end the job right after it.
fn failure_of(engine: &Engine) -> Fault {
    match engine.start(live()) {
        Err(StartError::Setup(fault)) => fault,
        Ok(job) => match job.recv_timeout(WAIT) {
            Err(RecvError::Ended(Reason::Fault(fault))) => fault,
            other => panic!("the job did not end with a fault: {other:?}"),
        },
        Err(other) => panic!("the start was refused: {other}"),
    }
}

fn ended(job: &lumaxcode::Job) -> Reason {
    loop {
        match job.recv_timeout(WAIT) {
            Err(RecvError::Ended(reason)) => return reason,
            Err(RecvError::Timeout) => panic!("the job did not end"),
            Ok(_) => {}
        }
    }
}

// ---- an echo child on real pipes ----

fn pipe() -> (File, File) {
    let mut fds = [0; 2];
    // close-on-exec: a shell child forked by a test running beside must not hold these ends open
    assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0, "{}", io::Error::last_os_error());
    // SAFETY: pipe2 just made both descriptors and nothing else owns them
    unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) }
}

#[derive(Default)]
struct Echo {
    spawned: AtomicUsize,
    alive: AtomicUsize,
    peak: AtomicUsize,
    idrs: AtomicUsize,
    cfgs: Mutex<Vec<proto::Cfg>>,
    rates: Mutex<Vec<u32>>,
}

struct EchoSpawn(Arc<Echo>);

struct EchoProc {
    pid: u32,
    stdin: Option<File>,
    stdout: Option<File>,
    stderr: Option<File>,
    exit: Arc<Mutex<Option<Exit>>>,
}

impl Spawn for EchoSpawn {
    fn spawn(&self, _child: &Path, cfg: &ChildCfg) -> io::Result<Box<dyn Proc>> {
        let echo = Arc::clone(&self.0);
        echo.cfgs.lock().push(cfg.cfg);
        let pid = u32::try_from(echo.spawned.fetch_add(1, SeqCst) + 1).unwrap();
        let alive = echo.alive.fetch_add(1, SeqCst) + 1;
        echo.peak.fetch_max(alive, SeqCst);
        let (input, stdin) = pipe();
        let (stdout, output) = pipe();
        // the child logs nothing: its end of the log is closed at once
        let (stderr, _) = pipe();
        let exit = Arc::new(Mutex::new(None));
        let gone = Arc::clone(&exit);
        thread::spawn(move || {
            run_echo(&echo, input, output);
            echo.alive.fetch_sub(1, SeqCst);
            *gone.lock() = Some(Exit::Code(0));
        });
        Ok(Box::new(EchoProc { pid, stdin: Some(stdin), stdout: Some(stdout), stderr: Some(stderr), exit }))
    }
}

/// Ready on the configuration, every unit back as it came, and a note of the controls. It ends when its input does, which is what the engine does
/// to every child it stops before it signals it.
fn run_echo(echo: &Echo, mut input: File, mut output: File) {
    while let Ok(Some(command)) = proto::read_command(&mut input) {
        let reply = match command {
            Command::Config(_) => Some(proto::encode_ready(&Ready { version: proto::VERSION, max_w: 1920, max_h: 1088, out_size: 1 << 20, ncap: 6, hma_bytes: 0 })),
            Command::Au { key, pts, data } => Some(proto::encode_out(key, pts, 10, &data)),
            Command::Idr => {
                echo.idrs.fetch_add(1, SeqCst);
                None
            }
            Command::Bitrate(kbps) => {
                echo.rates.lock().push(kbps);
                None
            }
        };
        if reply.is_some_and(|bytes| output.write_all(&bytes).is_err()) {
            break;
        }
    }
}

impl Proc for EchoProc {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        self.stdin.take().map(|f| Box::new(f) as Box<dyn Write + Send>)
    }

    fn stdout(&mut self) -> Option<Box<dyn Read + Send>> {
        self.stdout.take().map(|f| Box::new(f) as Box<dyn Read + Send>)
    }

    fn stderr(&mut self) -> Option<Box<dyn Read + Send>> {
        self.stderr.take().map(|f| Box::new(f) as Box<dyn Read + Send>)
    }

    fn signal(&mut self, _sig: Sig) {}

    fn try_wait(&mut self) -> Option<Exit> {
        self.exit.lock().take()
    }
}

fn echo_engine() -> (Engine, Arc<Echo>) {
    let echo = Arc::new(Echo::default());
    (engine(Arc::new(EchoSpawn(Arc::clone(&echo))), quick()), echo)
}

#[test]
fn a_viewer_is_started_fed_and_stopped_and_the_engine_ends_with_its_last_handle() {
    let (engine, echo) = echo_engine();
    let events = engine.subscribe();
    let job = engine.start(live()).unwrap();
    assert_eq!(job.state(), JobState::Running);
    let cfg = echo.cfgs.lock()[0];
    assert_eq!((cfg.codec, cfg.src_w, cfg.src_h, cfg.kbps, cfg.gop, cfg.flags), (proto::CODEC_MPEG2, 1920, 1088, 3500, 60, proto::CFG_CBR));

    assert_eq!(job.push(au(1, false)), Err(PushError::WantKey), "a decode starts on a key unit");
    job.push(au(2, true)).unwrap();
    job.push(au(3, false)).unwrap();
    for (pts, key) in [(2, true), (3, false)] {
        let unit = job.recv_timeout(WAIT).unwrap();
        assert_eq!((unit.pts, unit.key, unit.data.len()), (pts, key, 4));
    }
    // controls go ahead of the units behind them, so a unit that comes back proves the child has had them
    job.force_idr();
    job.set_kbps(2000);
    job.push(au(4, false)).unwrap();
    job.recv_timeout(WAIT).unwrap();
    assert_eq!((echo.idrs.load(SeqCst), echo.rates.lock().clone()), (1, vec![2000]));

    let snapshot = until(&engine, |s| s.jobs.first().is_some_and(|j| j.units == 3));
    assert!(snapshot.child_pid.is_some() && snapshot.jobs[0].bytes == 12);
    wait_for(&events, |e| matches!(e, Event::Started { job: 1, restarts: 0, .. }));

    job.stop();
    assert_eq!(ended(&job), Reason::Stopped);
    assert_eq!(job.push(au(5, true)), Err(PushError::Ended(Reason::Stopped)));
    let second = engine.start(live()).unwrap();
    assert_eq!(second.state(), JobState::Running);
    drop(engine);
    assert_eq!(ended(&second), Reason::Shutdown);
    assert_eq!((echo.spawned.load(SeqCst), echo.peak.load(SeqCst)), (2, 1));
}

#[test]
fn a_viewer_replaces_a_viewer_with_one_child_at_a_time() {
    let (engine, echo) = echo_engine();
    let first = engine.start(live()).unwrap();
    let second = engine.start(live()).unwrap();
    assert_eq!(ended(&first), Reason::Replaced);
    assert_eq!(second.state(), JobState::Running);
    assert_eq!(first.push(au(1, true)), Err(PushError::Ended(Reason::Replaced)));
    second.push(au(1, true)).unwrap();
    assert_eq!(second.recv_timeout(WAIT).unwrap().pts, 1);
    assert_eq!((echo.spawned.load(SeqCst), echo.peak.load(SeqCst)), (2, 1));
}

#[test]
fn the_gate_pauses_a_job_that_can_wait_and_its_first_unit_after_it_follows_a_gap() {
    let (engine, _echo) = echo_engine();
    let events = engine.subscribe();
    let job = engine.start(recording()).unwrap();
    wait_for(&events, |e| matches!(e, Event::Started { .. }));
    job.push(au(1, true)).unwrap();
    assert!(!job.recv_timeout(WAIT).unwrap().discontinuity);

    engine.hold("app", Some("netflix is in front"));
    let held = Reason::Held("app: netflix is in front".to_owned());
    let Event::Paused { reason, stopped_ms, .. } = wait_for(&events, |e| matches!(e, Event::Paused { .. })) else { unreachable!() };
    assert_eq!(reason, held);
    assert!(stopped_ms < 1000, "{stopped_ms}");
    assert_eq!(job.state(), JobState::Paused(held.clone()));
    assert_eq!(job.push(au(2, true)), Err(PushError::Paused(held)));

    engine.hold("app", None);
    wait_for(&events, |e| matches!(e, Event::Resumed { .. }));
    assert_eq!(job.push(au(3, false)), Err(PushError::WantKey), "the new child starts on a key unit");
    job.push(au(4, true)).unwrap();
    let unit = job.recv_timeout(WAIT).unwrap();
    assert_eq!((unit.pts, unit.discontinuity), (4, true));
}

#[test]
fn a_viewer_takes_the_hardware_from_the_replay_ring_which_keeps_what_it_has_and_runs_again_after() {
    let (engine, echo) = echo_engine();
    let events = engine.subscribe();
    let ring = engine.start(replay_ring()).unwrap();
    wait_for(&events, |e| matches!(e, Event::Started { .. }));
    // three groups of pictures of 30, a key unit each
    for group in 0..3u64 {
        for n in 0..30 {
            ring.push(au((group * 30 + n) * 3000, n == 0)).unwrap();
        }
        until(&engine, |s| s.jobs.first().is_some_and(|j| j.units == (group + 1) * 30));
    }
    let clip = engine.save_last(ring.id(), 1).unwrap();
    assert_eq!((clip.units[0].pts, clip.units[0].key, clip.units.len(), clip.pts_hz), (90_000, true, 60, 90_000));
    assert!(engine.snapshot().jobs[0].ring_bytes > 0);
    assert_eq!(engine.save_last(99, 1).unwrap_err(), ClipError::NoSuchJob);

    let viewer = engine.start(live()).unwrap();
    assert_eq!(ring.state(), JobState::Paused(Reason::Preempted { by: viewer.id() }));
    wait_for(&events, |e| matches!(e, Event::Paused { job: 1, reason: Reason::Preempted { .. }, .. }));
    assert_eq!(engine.save_last(ring.id(), 1).unwrap().units.len(), 60, "a paused ring can still be saved");
    let other = engine.start(recording());
    assert_eq!(engine.save_last(other.unwrap().id(), 1).unwrap_err(), ClipError::NotARing);

    drop(viewer);
    wait_for(&events, |e| matches!(e, Event::Resumed { job: 1 }));
    assert_eq!(ring.state(), JobState::Running);
    assert_eq!(ring.push(au(90 * 3000, false)), Err(PushError::WantKey));
    assert_eq!(echo.peak.load(SeqCst), 1);
}

#[test]
fn closing_the_gate_ends_a_viewer_at_once_and_refuses_the_next_until_it_opens() {
    let (engine, _echo) = echo_engine();
    let events = engine.subscribe();
    let viewer = engine.start(live()).unwrap();
    engine.hold("app", Some("netflix is in front"));
    let held = "app: netflix is in front".to_owned();
    assert_eq!(ended(&viewer), Reason::Held(held.clone()));
    assert_eq!(engine.start(live()).unwrap_err(), StartError::Held(held));
    let snapshot = engine.snapshot();
    assert_eq!((snapshot.state, snapshot.holds.len()), (lumaxcode::EngineState::Held, 1));
    wait_for(&events, |e| matches!(e, Event::Hold { reason: Some(_), .. }));

    engine.hold("app", None);
    assert_eq!(engine.start(live()).unwrap().state(), JobState::Running);
}

#[test]
fn a_job_the_child_cannot_do_is_refused_before_any_child_starts() {
    let (engine, echo) = echo_engine();
    let mut screen = live();
    screen.source = Source::Screen { point: ScreenPoint::Display };
    assert!(matches!(engine.start(screen), Err(StartError::Unsupported(_))));
    let mut big = replay_ring();
    big.sink = Sink::Ring { max_bytes: MAX_RING_BYTES + 1, max_seconds: 0, pts_hz: 90_000 };
    assert!(matches!(engine.start(big), Err(StartError::Unsupported(_))));
    let mut unit = replay_ring();
    unit.sink = Sink::Ring { max_bytes: 0, max_seconds: 0, pts_hz: 0 };
    assert!(matches!(engine.start(unit), Err(StartError::Unsupported(_))));
    assert_eq!(echo.spawned.load(SeqCst), 0);
}

// ---- shell children ----

/// Real processes, as the engine starts them on the TV; a script written a moment ago can be refused with ETXTBSY while another test forks.
struct Shell {
    inner: OsSpawn,
    pids: Mutex<Vec<u32>>,
}

impl Spawn for Shell {
    fn spawn(&self, child: &Path, cfg: &ChildCfg) -> io::Result<Box<dyn Proc>> {
        let mut tries = 0;
        loop {
            match self.inner.spawn(child, cfg) {
                Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) && tries < 50 => {
                    tries += 1;
                    thread::sleep(ms(2));
                }
                Ok(proc) => {
                    self.pids.lock().push(proc.pid());
                    return Ok(proc);
                }
                Err(e) => return Err(e),
            }
        }
    }
}

struct Run {
    engine: Engine,
    shell: Arc<Shell>,
    _dir: tempfile::TempDir,
}

/// An engine whose child is `body`, run by sh in a directory that holds a ready record (`ready.bin`) and an error report (`error.bin`).
fn run(body: &str, limits: Limits) -> Run {
    let dir = tempfile::tempdir().unwrap();
    let ready = Ready { version: proto::VERSION, max_w: 1920, max_h: 1088, out_size: 1 << 20, ncap: 6, hma_bytes: 0 };
    fs::write(dir.path().join("ready.bin"), proto::encode_ready(&ready)).unwrap();
    let error = Evt { kind: proto::EVT_ERROR, a: 4, text: "no decoder".to_owned(), ..Evt::default() };
    fs::write(dir.path().join("error.bin"), proto::encode_event(&error)).unwrap();
    let script = dir.path().join("child");
    fs::write(&script, format!("#!/bin/sh\ncd '{}'\n{body}\n", dir.path().display())).unwrap();
    fs::set_permissions(&script, Permissions::from_mode(0o755)).unwrap();
    let shell = Arc::new(Shell { inner: OsSpawn, pids: Mutex::new(Vec::new()) });
    let spawner: Arc<dyn Spawn> = shell.clone();
    let engine = Engine::with_spawner(Config { child: script, limits }, spawner).unwrap();
    Run { engine, shell, _dir: dir }
}

impl Run {
    /// Shuts down and checks that every child the engine started has been reaped.
    fn finish(self) {
        self.engine.shutdown();
        for pid in self.shell.pids.lock().iter() {
            assert!(!Path::new(&format!("/proc/{pid}")).exists(), "child {pid} is still there");
        }
    }
}

#[test]
fn a_child_that_cannot_be_started_is_a_setup_failure() {
    let engine = engine(Arc::new(OsSpawn), quick());
    let Err(StartError::Setup(Fault::Failed(why))) = engine.start(live()) else { panic!("the start did not fail") };
    assert!(why.starts_with("the converter did not start"), "{why}");
    assert_eq!(engine.snapshot().child_failures, 1);
}

#[test]
fn a_child_that_is_never_ready_is_stopped_and_reaped() {
    let run = run("exec sleep 30", Limits { ready: ms(60), ..quick() });
    let Err(StartError::Setup(Fault::Failed(why))) = run.engine.start(live()) else { panic!("the start did not fail") };
    assert_eq!(why, "the converter was not ready after 60 ms");
    run.finish();
}

#[test]
fn a_child_that_goes_silent_is_stalled_and_stopped() {
    let run = run("cat ready.bin\nexec sleep 30", Limits { silence: ms(80), ..quick() });
    let job = run.engine.start(live()).unwrap();
    assert_eq!(ended(&job), Reason::Fault(Fault::Stalled("the converter sent nothing for 80 ms".to_owned())));
    run.finish();
}

#[test]
fn a_child_that_ignores_sigterm_is_killed_after_the_grace_and_the_hardware_is_free_after_that() {
    // deaf from its first instruction: a TERM that lands before the trap would end the shell at once
    let run = run("trap '' TERM\ncat ready.bin\nexec sleep 30", quick());
    let events = run.engine.subscribe();
    let job = run.engine.start(recording()).unwrap();
    wait_for(&events, |e| matches!(e, Event::Started { .. }));
    run.engine.hold("app", Some("netflix is in front"));
    let Event::Paused { stopped_ms, .. } = wait_for(&events, |e| matches!(e, Event::Paused { .. })) else { unreachable!() };
    assert!((60..600).contains(&stopped_ms), "the stop took {stopped_ms} ms: the grace is 60");
    assert_eq!(run.engine.snapshot().last_stop_ms, Some(stopped_ms));
    assert!(matches!(job.state(), JobState::Paused(Reason::Held(_))));
    run.finish();
}

#[test]
fn a_failing_child_ends_the_job_with_its_own_words_and_counts_as_a_failure() {
    let run = run("cat ready.bin error.bin\nexit 4", quick());
    let Fault::Failed(why) = failure_of(&run.engine) else { panic!("not a failure") };
    assert!(why.starts_with("the video decoder failed"), "{why}");
    let snapshot = until(&run.engine, |s| s.child_pid.is_none());
    assert_eq!(snapshot.child_failures, 1);
    run.finish();
}

#[test]
fn a_child_that_reports_a_hung_call_stops_the_engine_until_it_is_restarted() {
    let run = run("cat ready.bin\nexit 75", quick());
    let events = run.engine.subscribe();
    assert!(matches!(failure_of(&run.engine), Fault::Wedged(_)));
    wait_for(&events, |e| matches!(e, Event::Wedged));
    assert_eq!(run.engine.snapshot().state, lumaxcode::EngineState::Wedged);
    assert!(matches!(run.engine.start(live()), Err(StartError::Wedged(_))));
    run.finish();
}

#[test]
fn a_child_that_floods_its_log_neither_stalls_the_engine_nor_grows_what_it_keeps() {
    let run = run("cat ready.bin\ni=0\nwhile [ $i -lt 3000 ]; do echo \"noise $i\" >&2; i=$((i+1)); done\nexec sleep 30", quick());
    let job = run.engine.start(live()).unwrap();
    let snapshot = until(&run.engine, |s| s.child_log.last().is_some_and(|line| line == "noise 2999"));
    assert_eq!((snapshot.child_log.len(), snapshot.last_tick_ms), (8, 0));
    assert_eq!(job.state(), JobState::Running);
    run.finish();
}
