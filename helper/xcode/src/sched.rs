//! The scheduler as a pure state machine. Commands, child reports and the clock go in as [`Input`]s; spawns, signals, job state changes and events
//! come out as [`Action`]s. Nothing here touches a thread, a pipe or the system clock, so the tests drive it with a clock they set.
//!
//! One child runs at a time. The best runnable job (highest priority, then the one already running, then the oldest) should run; when it is not the job
//! that has the child, the child is stopped, and only after it is gone does the next one start, so two children never hold the hardware together.

use crate::proc::Exit;
use crate::proto;
use crate::types::{EngineState, Event, Fault, HoldInfo, JobId, JobInfo, JobKind, JobState, Limits, Priority, PushError, Reason, Snapshot, StartError, Wait};
use std::cmp::Reverse;
use std::collections::VecDeque;
use std::time::Duration;

/// Ended jobs kept for the status page.
pub(crate) const KEEP_ENDED: usize = 4;

/// What the scheduler needs to know of a job; the rest of the job's spec is the runtime's.
#[derive(Clone, Debug)]
pub(crate) struct Spec {
    pub(crate) owner: String,
    pub(crate) priority: Priority,
    pub(crate) wait: Wait,
    /// The owner feeds the stream, and so owns its state: the engine cannot restart such a job by itself.
    pub(crate) units: bool,
}

#[derive(Debug)]
pub(crate) enum Input {
    Start { ticket: u64, spec: Spec },
    Stop { job: JobId },
    /// `reason: None` releases the key.
    Hold { key: String, reason: Option<String> },
    Ready { epoch: u64 },
    /// The child reported its own failure; it exits next.
    ChildError { epoch: u64, status: u32, text: String },
    OutClosed { epoch: u64 },
    Protocol { epoch: u64, why: String },
    /// The child process could not be started.
    SpawnFailed { epoch: u64, why: String },
    /// The child last sent a record at this time.
    Heard { epoch: u64, at: Duration },
    Exited { epoch: u64, exit: Exit, last_line: Option<String> },
    Tick,
    Shutdown,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Action {
    Accepted { ticket: u64, job: JobId },
    Refused { ticket: u64, error: StartError },
    /// Start a child for the job; `resume` when the job has run before (its first unit is a discontinuity).
    Spawn { epoch: u64, job: JobId, resume: bool },
    /// Stop taking the child's units and close its input.
    Detach { epoch: u64, job: JobId },
    Term { epoch: u64 },
    Kill { epoch: u64 },
    /// The child outlived SIGKILL: keep it where it can still be reaped.
    Park { epoch: u64 },
    State { job: JobId, state: JobState },
    Emit(Event),
}

struct JobRec {
    id: JobId,
    owner: String,
    priority: Priority,
    wait: Wait,
    units: bool,
    state: JobState,
    since: Duration,
    /// A child has been ready for this job at least once.
    ran: bool,
    restarts: u32,
    /// How many failures in a row the back-off has seen.
    step: u32,
    retry_at: Option<Duration>,
    restart_text: String,
    last_fault: Option<Fault>,
}

impl JobRec {
    fn live(&self) -> bool {
        !matches!(self.state, JobState::Ended(_))
    }

    fn runnable(&self, now: Duration) -> bool {
        self.live() && self.retry_at.is_none_or(|at| at <= now)
    }

    /// A job the engine does not restart after a failure: one the owner feeds, and one that has not run yet and was to answer at once.
    fn no_retry(&self) -> bool {
        self.units || (self.wait == Wait::Fail && !self.ran)
    }
}

struct Stop {
    /// When the stop began, and SIGTERM was sent.
    at: Duration,
    killed: Option<Duration>,
    /// Report the stop as the job's `Paused` event once the child is gone.
    report: bool,
}

struct Child {
    job: JobId,
    epoch: u64,
    spawned: Duration,
    ready: Option<Duration>,
    heard: Duration,
    error: Option<(u32, String)>,
    closed: bool,
    /// The child said it is failing, or closed its output: it is expected to exit by itself before this.
    failing: Option<Duration>,
    stop: Option<Stop>,
}

pub(crate) struct Core {
    lim: Limits,
    jobs: Vec<JobRec>,
    next_job: JobId,
    next_epoch: u64,
    holds: Vec<(String, String)>,
    child: Option<Child>,
    wedged: Option<String>,
    closing: bool,
    /// The times of the newest failures, at most `flap.0` of them.
    fails: VecDeque<Duration>,
    cool_until: Option<Duration>,
    starts: u64,
    failures: u64,
    last_stop_ms: Option<u32>,
    now: Duration,
    out: Vec<Action>,
}

fn millis(d: Duration) -> u32 {
    u32::try_from(d.as_millis()).unwrap_or(u32::MAX)
}

fn earliest(slot: &mut Option<Duration>, at: Duration) {
    *slot = Some(slot.map_or(at, |t| t.min(at)));
}

/// What a child's end means for its job. `detail` is the child's own error text, or the last line it logged.
pub(crate) fn fault_for(exit: Exit, detail: Option<&str>) -> Fault {
    let (make, what): (fn(String) -> Fault, String) = match exit {
        Exit::Code(proto::EXIT_CONFIG) => (Fault::Unsupported, "the converter rejected the job's settings".into()),
        Exit::Code(proto::EXIT_SHAPE) => (Fault::Unsupported, "the stream has a picture size the converter cannot take".into()),
        Exit::Code(proto::EXIT_SETUP) => (Fault::Failed, "the converter could not set up the video hardware".into()),
        Exit::Code(proto::EXIT_DECODER) => (Fault::Failed, "the video decoder failed".into()),
        Exit::Code(proto::EXIT_ENCODER) => (Fault::Failed, "the video encoder failed".into()),
        Exit::Code(proto::EXIT_STALLED) => (Fault::Stalled, "the converter stopped making pictures".into()),
        Exit::Code(proto::EXIT_WEDGED) => (Fault::Wedged, "a call into the video hardware did not return".into()),
        // it was never asked to stop, so a clean exit is its own output failing
        Exit::Code(proto::EXIT_OK) => (Fault::Failed, "the converter ended by itself".into()),
        Exit::Code(code) => (Fault::Failed, format!("the converter exited with status {code}")),
        Exit::Signal(sig) => (Fault::Failed, format!("the converter exited with signal {sig}")),
    };
    make(match detail {
        Some(text) if !text.is_empty() => format!("{what} ({text})"),
        _ => what,
    })
}

/// Whether a unit pushed by the owner is taken, given where the job is: the rule behind `Job::push`. After a (re)start only a key unit may come first.
pub(crate) fn admit(state: &JobState, want_key: bool, key: bool) -> Result<(), PushError> {
    match state {
        JobState::Running if want_key && !key => Err(PushError::WantKey),
        JobState::Running => Ok(()),
        JobState::Queued => Err(PushError::Paused(Reason::Queued)),
        JobState::Paused(reason) => Err(PushError::Paused(reason.clone())),
        JobState::Ended(reason) => Err(PushError::Ended(reason.clone())),
    }
}

impl Core {
    pub(crate) fn new(lim: Limits) -> Self {
        Self {
            lim,
            jobs: Vec::new(),
            next_job: 0,
            next_epoch: 0,
            holds: Vec::new(),
            child: None,
            wedged: None,
            closing: false,
            fails: VecDeque::new(),
            cool_until: None,
            starts: 0,
            failures: 0,
            last_stop_ms: None,
            now: Duration::ZERO,
            out: Vec::new(),
        }
    }

    /// Applies one input at time `now` and returns what to do about it. Time never runs backwards: an earlier `now` is taken as the latest one seen.
    pub(crate) fn step(&mut self, now: Duration, input: Input) -> Vec<Action> {
        self.now = self.now.max(now);
        match input {
            Input::Start { ticket, spec } => self.start(ticket, spec),
            Input::Stop { job } => self.end_job(job, Reason::Stopped),
            Input::Hold { key, reason } => self.hold(key, reason),
            Input::Ready { epoch } => self.ready(epoch),
            Input::ChildError { epoch, status, text } => self.child_error(epoch, status, text),
            Input::OutClosed { epoch } => self.out_closed(epoch),
            Input::Protocol { epoch, why } => self.protocol(epoch, &why),
            Input::SpawnFailed { epoch, why } => self.spawn_failed(epoch, &why),
            Input::Heard { epoch, at } => self.heard(epoch, at),
            Input::Exited { epoch, exit, last_line } => self.exited(epoch, exit, last_line),
            Input::Tick => {}
            Input::Shutdown => self.shutdown(),
        }
        self.timers();
        self.reconcile();
        self.refresh();
        std::mem::take(&mut self.out)
    }

    /// When the next tick is due, if any: the earliest watchdog, stop escalation or restart.
    pub(crate) fn next_deadline(&self) -> Option<Duration> {
        let lim = &self.lim;
        let mut due = None;
        if let Some(c) = &self.child {
            match (&c.stop, c.failing, c.ready) {
                (Some(stop), ..) => earliest(&mut due, stop.killed.map_or(stop.at + lim.term_grace, |k| k + lim.kill_wait)),
                (None, Some(by), _) => earliest(&mut due, by),
                (None, None, None) => earliest(&mut due, c.spawned + lim.ready),
                (None, None, Some(_)) => earliest(&mut due, c.heard + lim.silence),
            }
        }
        for at in self.jobs.iter().filter(|j| j.live()).filter_map(|j| j.retry_at).filter(|&at| at > self.now) {
            earliest(&mut due, at);
        }
        due
    }

    /// Shutting down and nothing left running.
    pub(crate) fn finished(&self) -> bool {
        self.closing && self.child.is_none()
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        let now = self.now;
        Snapshot {
            state: self.engine_state(),
            holds: self.holds.iter().map(|(key, reason)| HoldInfo { key: key.clone(), reason: reason.clone() }).collect(),
            jobs: self
                .jobs
                .iter()
                .map(|j| JobInfo {
                    id: j.id,
                    owner: j.owner.clone(),
                    priority: j.priority,
                    state: match j.state {
                        JobState::Queued => JobKind::Queued,
                        JobState::Running => JobKind::Running,
                        JobState::Paused(_) => JobKind::Paused,
                        JobState::Ended(_) => JobKind::Ended,
                    },
                    reason: match &j.state {
                        JobState::Paused(reason) | JobState::Ended(reason) => Some(reason.clone()),
                        _ => None,
                    },
                    since_ms: u64::try_from(now.saturating_sub(j.since).as_millis()).unwrap_or(u64::MAX),
                    units: 0,
                    bytes: 0,
                    kbps: 0,
                    restarts: j.restarts,
                    last_fault: j.last_fault.clone(),
                    ring_bytes: 0,
                })
                .collect(),
            child_starts: self.starts,
            child_failures: self.failures,
            last_stop_ms: self.last_stop_ms,
            ..Snapshot::default()
        }
    }

    fn engine_state(&self) -> EngineState {
        if self.wedged.is_some() {
            EngineState::Wedged
        } else if !self.holds.is_empty() {
            EngineState::Held
        } else if self.cool_until.is_some_and(|until| until > self.now) {
            EngineState::Cooling
        } else if self.active().is_some() {
            EngineState::Running
        } else {
            EngineState::Idle
        }
    }

    /// `<key>: <reason>` of the first hold, when the gate is closed.
    fn gate(&self) -> Option<String> {
        self.holds.first().map(|(key, reason)| format!("{key}: {reason}"))
    }

    /// The job whose child is up and not being stopped.
    fn active(&self) -> Option<JobId> {
        self.child.as_ref().filter(|c| c.stop.is_none()).map(|c| c.job)
    }

    /// The job that should have the child: none while the gate is closed.
    fn best(&self) -> Option<(JobId, Priority)> {
        if !self.holds.is_empty() {
            return None;
        }
        let running = self.active();
        self.jobs
            .iter()
            .filter(|j| j.runnable(self.now))
            .max_by_key(|j| (j.priority, Some(j.id) == running, Reverse(j.id)))
            .map(|j| (j.id, j.priority))
    }

    fn admit_start(&self, spec: &Spec) -> Result<(), StartError> {
        if let Some(why) = &self.wedged {
            return Err(StartError::Wedged(why.clone()));
        }
        if self.closing {
            return Err(StartError::Setup(Fault::Failed("the engine is shutting down".into())));
        }
        // Live and Replay jobs replace their own kind, so the job that goes does not count
        let replaces = |j: &JobRec| matches!(spec.priority, Priority::Live | Priority::Replay) && j.priority == spec.priority;
        if self.jobs.iter().filter(|j| j.live() && !replaces(j)).count() >= self.lim.max_jobs {
            return Err(StartError::TooMany);
        }
        if spec.priority == Priority::Live
            && let Some(until) = self.cool_until
            && until > self.now
        {
            return Err(StartError::Cooling(until - self.now));
        }
        if spec.wait == Wait::Fail {
            if let Some(gate) = self.gate() {
                return Err(StartError::Held(gate));
            }
            if let Some(j) = self.jobs.iter().find(|j| j.runnable(self.now) && j.priority > spec.priority) {
                return Err(StartError::Busy(format!("a {} job has the video hardware", j.priority.name())));
            }
            if spec.priority == Priority::Recording && self.jobs.iter().any(|j| j.live() && j.priority == Priority::Recording) {
                return Err(StartError::Busy("another recording is running".into()));
            }
        }
        Ok(())
    }

    fn start(&mut self, ticket: u64, spec: Spec) {
        if let Err(error) = self.admit_start(&spec) {
            self.out.push(Action::Refused { ticket, error });
            return;
        }
        if matches!(spec.priority, Priority::Live | Priority::Replay) {
            let old: Vec<JobId> = self.jobs.iter().filter(|j| j.live() && j.priority == spec.priority).map(|j| j.id).collect();
            for id in old {
                self.end_job(id, Reason::Replaced);
            }
        }
        self.next_job += 1;
        let id = self.next_job;
        self.jobs.push(JobRec {
            id,
            owner: spec.owner,
            priority: spec.priority,
            wait: spec.wait,
            units: spec.units,
            state: JobState::Queued,
            since: self.now,
            ran: false,
            restarts: 0,
            step: 0,
            retry_at: None,
            restart_text: String::new(),
            last_fault: None,
        });
        self.out.push(Action::Accepted { ticket, job: id });
    }

    fn hold(&mut self, key: String, reason: Option<String>) {
        let changed = match &reason {
            Some(why) => match self.holds.iter_mut().find(|(k, _)| *k == key) {
                Some((_, old)) if old == why => false,
                Some((_, old)) => {
                    old.clone_from(why);
                    true
                }
                None => {
                    self.holds.push((key.clone(), why.clone()));
                    true
                }
            },
            None => {
                let before = self.holds.len();
                self.holds.retain(|(k, _)| *k != key);
                self.holds.len() != before
            }
        };
        if !changed {
            return;
        }
        self.out.push(Action::Emit(Event::Hold { key, reason }));
        let Some(gate) = self.gate() else {
            return;
        };
        // a job that must answer at once ends rather than waits: a tuner viewer reconnects, and a start that has not been answered yet is answered
        let ends: Vec<JobId> =
            self.jobs.iter().filter(|j| j.live() && j.wait == Wait::Fail && ((j.units && j.priority == Priority::Live) || !j.ran)).map(|j| j.id).collect();
        for id in ends {
            self.end_job(id, Reason::Held(gate.clone()));
        }
    }

    fn child_of(&mut self, epoch: u64) -> Option<&mut Child> {
        self.child.as_mut().filter(|c| c.epoch == epoch && c.stop.is_none())
    }

    fn ready(&mut self, epoch: u64) {
        let now = self.now;
        let Some(c) = self.child_of(epoch).filter(|c| c.ready.is_none()) else {
            return;
        };
        c.ready = Some(now);
        c.heard = now;
        let (job, setup_ms) = (c.job, millis(now.saturating_sub(c.spawned)));
        let Some(j) = self.jobs.iter_mut().find(|j| j.id == job) else {
            return;
        };
        let resumed = std::mem::replace(&mut j.ran, true);
        j.state = JobState::Running;
        j.since = now;
        let restarts = j.restarts;
        self.out.push(Action::State { job, state: JobState::Running });
        self.out.push(Action::Emit(Event::Started { job, restarts, setup_ms }));
        if resumed {
            self.out.push(Action::Emit(Event::Resumed { job }));
        }
    }

    fn heard(&mut self, epoch: u64, at: Duration) {
        let now = self.now;
        if let Some(c) = self.child_of(epoch) {
            c.heard = c.heard.max(at.min(now));
        }
    }

    fn child_error(&mut self, epoch: u64, status: u32, text: String) {
        let by = self.now + self.lim.term_grace;
        if let Some(c) = self.child_of(epoch) {
            c.error.get_or_insert((status, text));
            c.failing.get_or_insert(by);
        }
    }

    fn out_closed(&mut self, epoch: u64) {
        let by = self.now + self.lim.term_grace;
        if let Some(c) = self.child_of(epoch) {
            c.closed = true;
            c.failing.get_or_insert(by);
        }
    }

    fn protocol(&mut self, epoch: u64, why: &str) {
        if self.child_of(epoch).is_some() {
            self.fail_child(Fault::Failed(format!("protocol error: {why}")));
        }
    }

    fn spawn_failed(&mut self, epoch: u64, why: &str) {
        if self.child.as_ref().is_none_or(|c| c.epoch != epoch) {
            return;
        }
        let Some(c) = self.child.take() else {
            return;
        };
        self.failed(c.job, Fault::Failed(format!("the converter did not start: {why}")), Duration::ZERO);
    }

    fn exited(&mut self, epoch: u64, exit: Exit, last_line: Option<String>) {
        if self.child.as_ref().is_none_or(|c| c.epoch != epoch) {
            return;
        }
        let Some(c) = self.child.take() else {
            return;
        };
        let now = self.now;
        if let Some(stop) = &c.stop {
            let stopped_ms = millis(now.saturating_sub(stop.at));
            self.last_stop_ms = Some(stopped_ms);
            if stop.report
                && let Some(j) = self.jobs.iter().find(|j| j.id == c.job)
                && let JobState::Paused(reason) = &j.state
            {
                self.out.push(Action::Emit(Event::Paused { job: c.job, reason: reason.clone(), stopped_ms }));
            }
            // a hung hardware call is the child's own finding, whoever asked it to stop
            if exit == Exit::Code(proto::EXIT_WEDGED) {
                self.wedge(fault_for(exit, None).message().to_owned());
            }
            return;
        }
        self.out.push(Action::Detach { epoch, job: c.job });
        let detail = c.error.as_ref().map(|e| e.1.as_str()).or(last_line.as_deref());
        self.failed(c.job, fault_for(exit, detail), c.ready.map_or(Duration::ZERO, |r| now.saturating_sub(r)));
    }

    /// The live child failed: record it, decide what happens to its job, and stop the child.
    fn fail_child(&mut self, fault: Fault) {
        let Some(c) = self.child.as_ref().filter(|c| c.stop.is_none()) else {
            return;
        };
        let (job, ran_for) = (c.job, c.ready.map_or(Duration::ZERO, |r| self.now.saturating_sub(r)));
        self.failed(job, fault, ran_for);
        self.begin_stop(false);
    }

    fn failed(&mut self, job: JobId, fault: Fault, ran_for: Duration) {
        self.failures += 1;
        if let Fault::Wedged(why) = &fault {
            let why = why.clone();
            self.wedge(why);
            return;
        }
        let unsupported = matches!(fault, Fault::Unsupported(_));
        let (limit, window) = self.lim.flap;
        let limit = usize::try_from(limit).unwrap_or(usize::MAX).max(1);
        let mut flapped = false;
        if !unsupported {
            while self.fails.front().is_some_and(|t| self.now.saturating_sub(*t) > window) {
                self.fails.pop_front();
            }
            self.fails.push_back(self.now);
            while self.fails.len() > limit {
                self.fails.pop_front();
            }
            flapped = self.fails.len() >= limit;
            if flapped {
                self.cool_until = Some(self.now + self.lim.cooldown);
            }
        }
        let Some(j) = self.jobs.iter_mut().find(|j| j.id == job && j.live()) else {
            return;
        };
        j.last_fault = Some(fault.clone());
        if unsupported || flapped || j.no_retry() {
            let fault = if flapped { flapped_fault(fault, limit, window) } else { fault };
            self.end_job(job, Reason::Fault(fault));
            return;
        }
        // the delay doubles with each failure in a row, and starts over after the child ran as long as the longest delay
        let (first, max) = self.lim.backoff;
        if ran_for >= max {
            j.step = 0;
        }
        let delay = first.saturating_mul(1u32.checked_shl(j.step).unwrap_or(u32::MAX)).min(max);
        j.step = j.step.saturating_add(1);
        j.restarts += 1;
        j.retry_at = Some(self.now + delay);
        j.restart_text = fault.to_string();
    }

    /// The hardware is stuck: nothing can run until the helper restarts.
    fn wedge(&mut self, why: String) {
        if self.wedged.is_some() {
            return;
        }
        self.wedged = Some(why.clone());
        self.out.push(Action::Emit(Event::Wedged));
        let ids: Vec<JobId> = self.jobs.iter().filter(|j| j.live()).map(|j| j.id).collect();
        for id in ids {
            self.end_job(id, Reason::Fault(Fault::Wedged(why.clone())));
        }
    }

    fn shutdown(&mut self) {
        self.closing = true;
        let ids: Vec<JobId> = self.jobs.iter().filter(|j| j.live()).map(|j| j.id).collect();
        for id in ids {
            self.end_job(id, Reason::Shutdown);
        }
    }

    fn end_job(&mut self, id: JobId, reason: Reason) {
        let now = self.now;
        let Some(j) = self.jobs.iter_mut().find(|j| j.id == id && j.live()) else {
            return;
        };
        j.state = JobState::Ended(reason.clone());
        j.since = now;
        j.retry_at = None;
        self.out.push(Action::State { job: id, state: JobState::Ended(reason.clone()) });
        self.out.push(Action::Emit(Event::Ended { job: id, reason }));
        if self.child.as_ref().is_some_and(|c| c.job == id) {
            if let Some(stop) = self.child.as_mut().and_then(|c| c.stop.as_mut()) {
                // the stop that was to be reported as a pause is now the end of the job
                stop.report = false;
            } else {
                self.begin_stop(false);
            }
        }
        self.prune();
    }

    fn prune(&mut self) {
        let mut surplus = self.jobs.iter().filter(|j| !j.live()).count().saturating_sub(KEEP_ENDED);
        self.jobs.retain(|j| {
            let drop = surplus > 0 && !j.live();
            if drop {
                surplus -= 1;
            }
            !drop
        });
    }

    /// SIGTERM now; escalation follows in `timers`.
    fn begin_stop(&mut self, report: bool) {
        let now = self.now;
        let Some(c) = self.child.as_mut().filter(|c| c.stop.is_none()) else {
            return;
        };
        c.stop = Some(Stop { at: now, killed: None, report });
        let (epoch, job) = (c.epoch, c.job);
        self.out.push(Action::Detach { epoch, job });
        self.out.push(Action::Term { epoch });
    }

    fn spawn(&mut self, job: JobId) {
        self.next_epoch += 1;
        let epoch = self.next_epoch;
        let now = self.now;
        self.child = Some(Child { job, epoch, spawned: now, ready: None, heard: now, error: None, closed: false, failing: None, stop: None });
        self.starts += 1;
        let Some(j) = self.jobs.iter_mut().find(|j| j.id == job) else {
            return;
        };
        j.retry_at = None;
        let resume = j.ran;
        let starting = JobState::Queued;
        if j.state != starting {
            j.state = starting.clone();
            j.since = now;
            self.out.push(Action::State { job, state: starting });
        }
        self.out.push(Action::Spawn { epoch, job, resume });
    }

    /// Watchdogs and the stop escalation: the part of a step that depends on the clock alone.
    fn timers(&mut self) {
        let now = self.now;
        let lim = &self.lim;
        let due = self.child.as_ref().filter(|c| c.stop.is_none()).and_then(|c| match (c.failing, c.ready) {
            (Some(by), _) if now >= by => Some(match &c.error {
                Some((status, text)) => fault_for(Exit::Code(i32::try_from(*status).unwrap_or(-1)), Some(text)),
                None => Fault::Failed("the converter closed its output".into()),
            }),
            (Some(_), _) => None,
            (None, None) if now >= c.spawned + lim.ready => {
                Some(Fault::Failed(format!("the converter was not ready after {} ms", millis(lim.ready))))
            }
            (None, Some(_)) if now >= c.heard + lim.silence => Some(Fault::Stalled(format!("the converter sent nothing for {} ms", millis(lim.silence)))),
            _ => None,
        });
        if let Some(fault) = due {
            self.fail_child(fault);
        }
        let (term_grace, kill_wait) = (self.lim.term_grace, self.lim.kill_wait);
        let mut parked = None;
        if let Some(c) = self.child.as_mut()
            && let Some(stop) = c.stop.as_mut()
        {
            if stop.killed.is_none() && now >= stop.at + term_grace {
                stop.killed = Some(now);
                self.out.push(Action::Kill { epoch: c.epoch });
            }
            if stop.killed.is_some_and(|k| now >= k + kill_wait) {
                parked = Some(c.epoch);
            }
        }
        if let Some(epoch) = parked {
            self.child = None;
            self.out.push(Action::Park { epoch });
            self.wedge("the converter did not exit after SIGKILL".into());
        }
    }

    /// Makes the child match the best job: stop it if it is for another, start one if there is none.
    fn reconcile(&mut self) {
        if self.closing || self.wedged.is_some() {
            return;
        }
        let best = self.best().map(|(id, _)| id);
        match &self.child {
            Some(c) if c.stop.is_none() && Some(c.job) != best => self.begin_stop(true),
            None => {
                if let Some(id) = best {
                    self.spawn(id);
                }
            }
            Some(_) => {}
        }
    }

    /// Brings the state of every job that has no child in line with why it has none.
    fn refresh(&mut self) {
        let (now, gate, best, active) = (self.now, self.gate(), self.best(), self.active());
        let reported = self.child.as_ref().filter(|c| c.stop.as_ref().is_some_and(|s| s.report)).map(|c| c.job);
        let changes: Vec<(JobId, JobState)> = self
            .jobs
            .iter()
            .filter(|j| j.live() && Some(j.id) != active)
            .filter_map(|j| {
                let want = if j.retry_at.is_some_and(|at| at > now) {
                    JobState::Paused(Reason::Restarting { fault: j.restart_text.clone() })
                } else if let Some(gate) = &gate {
                    JobState::Paused(Reason::Held(gate.clone()))
                } else {
                    match best {
                        Some((by, priority)) if priority > j.priority => JobState::Paused(Reason::Preempted { by }),
                        _ => JobState::Queued,
                    }
                };
                (j.state != want).then_some((j.id, want))
            })
            .collect();
        for (id, want) in changes {
            // a job that has not run and must answer at once is not parked behind a higher priority job: it ends
            if let JobState::Paused(Reason::Preempted { by }) = &want
                && self.jobs.iter().any(|j| j.id == id && j.wait == Wait::Fail && !j.ran)
            {
                self.end_job(id, Reason::Preempted { by: *by });
                continue;
            }
            if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
                j.state = want.clone();
                j.since = now;
            }
            self.out.push(Action::State { job: id, state: want.clone() });
            // the child that is being stopped for this job reports the pause itself, with how long the stop took
            if let JobState::Paused(reason) = want
                && Some(id) != reported
            {
                self.out.push(Action::Emit(Event::Paused { job: id, reason, stopped_ms: 0 }));
            }
        }
    }
}

fn flapped_fault(fault: Fault, count: usize, window: Duration) -> Fault {
    let text = format!("{} (failed {count} times in {} s)", fault.message(), window.as_secs());
    match fault {
        Fault::Unsupported(_) => Fault::Unsupported(text),
        Fault::Failed(_) => Fault::Failed(text),
        Fault::Stalled(_) => Fault::Stalled(text),
        Fault::Wedged(_) => Fault::Wedged(text),
    }
}

#[cfg(test)]
mod tests;
