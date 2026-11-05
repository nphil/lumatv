//! The receiver's push link: one telnet connection, its own thread, reconnecting with backoff.
//!
//! The receiver thread (`receiver.rs`) points the link at a target while the screen is on and at nothing while it is off. The
//! link thread then holds exactly one connection to that target, asks the four state questions (`PW?`, `MV?`, `MU?`, `SI?`)
//! on connecting and again every `heartbeat`, and turns what comes back, including every change the receiver pushes on its
//! own, into [`Event::Reading`]s: a whole [`Status`] each time one of the four is heard once all four are known. The
//! repeated questions are what keep a reading's stamp moving while nothing changes, and what notice a receiver that went
//! silent without closing the socket.
//!
//! A failure (refused, closed, silent for `silence`, no complete state within `ready_within`) is one [`Event::Down`], then
//! a wait that doubles from `backoff_base` to `backoff_max` before the next attempt. A session that got as far as a reading
//! starts the backoff over. There is never a second connection: the next one opens only after the previous is closed.
//!
//! Settings use typed commands and readback on this same connection while the settings page is interested.

use crate::timeutil::now_ms;
use lumatv_denon::settings::{self, Query, Reply, Setting};
use lumatv_denon::telnet::{self, FORMAT_QUERIES, QUERIES, Session, Snapshot, parse_line};
use lumatv_denon::{Client, ClientError, Status};
use parking_lot::{Condvar, Mutex};
use std::collections::VecDeque;
use std::io;
use std::net::TcpStream;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The pause after each query: the receiver's own interface guide asks for 50 ms between commands.
pub const QUERY_GAP: Duration = Duration::from_millis(50);
const SETTINGS_INTEREST: Duration = Duration::from_secs(5);
const SETTINGS_TIMEOUT: Duration = Duration::from_secs(3);
const SETTINGS_POLL: Duration = Duration::from_millis(100);
const SETTINGS_QUIET: Duration = Duration::from_millis(200);

/// Where to connect: the receiver's address and its telnet port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Debug)]
pub struct Tuning {
    pub connect_timeout: Duration,
    /// The period between rounds of questions.
    pub heartbeat: Duration,
    /// The longest the receiver may say nothing at all.
    pub silence: Duration,
    /// How long after connecting all four answers may take.
    pub ready_within: Duration,
    pub backoff_base: Duration,
    pub backoff_max: Duration,
}

#[derive(Debug)]
pub enum Event {
    /// The receiver's state, heard at `at` (`at_ms` on the wall clock). Its display mode is unknown: the link cannot learn it. `mode` and `signal` are
    /// the sound mode and the input signal's words as last heard, raw, `None` until the receiver has said them; `max_volume` is its volume limit
    /// on the panel's 0 to 98 scale (`MVMAX`, sent with every volume answer).
    Reading {
        status: Status,
        at: Instant,
        at_ms: u64,
        mode: Option<String>,
        signal: Option<String>,
        max_volume: Option<f64>,
    },
    /// The connection failed or was lost; the link is already waiting to try again.
    Down(ClientError),
    SettingsConnected {
        session: u64,
        context_revision: u64,
        target: Target,
    },
    SettingsReply {
        session: u64,
        context_revision: u64,
        target: Target,
        reply: Reply,
        line: String,
        at: Instant,
        at_ms: u64,
    },
    SettingsMode {
        session: u64,
        context_revision: u64,
        target: Target,
        mode: String,
        at: Instant,
        at_ms: u64,
    },
    SettingsSignal {
        session: u64,
        context_revision: u64,
        target: Target,
        signal: String,
        at: Instant,
        at_ms: u64,
    },
    SettingsContextChanged {
        session: u64,
        target: Target,
        context_revision: u64,
        field: ContextField,
    },
    SettingsResult {
        session: u64,
        target: Target,
        request_id: String,
        status: SettingsStatus,
        error: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextField {
    Power,
    Input,
    Mode,
    Signal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsStatus {
    Confirmed,
    Failed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsSubmitError {
    Busy,
    Unavailable,
    InvalidRequestId,
}

struct SettingsRequest {
    session: u64,
    expected_context: u64,
    target: Target,
    request_id: String,
    command: Setting,
    written: bool,
    queried: bool,
    quiet_since: Option<Instant>,
    valid_until: Instant,
    authorized: Box<dyn Fn() -> bool + Send>,
    deadline: Instant,
}

fn settings_verdict(
    request: &SettingsRequest,
    reply: Reply,
) -> Option<(bool, Option<&'static str>)> {
    if !reply.same_field(request.command) {
        return None;
    }
    if !request.written {
        Some((false, Some("ambiguous reply before write")))
    } else if !request.queried {
        None
    } else if !request.command.matches_reply(&reply) {
        Some((false, Some("settings reply mismatch")))
    } else if !(request.authorized)() {
        Some((false, Some("receiver target or screen changed")))
    } else {
        Some((true, None))
    }
}

#[derive(Default)]
struct State {
    target: Option<Target>,
    /// Counts target changes, so a session and the events it raised can tell they belong to an old target.
    generation: u64,
    stop: bool,
    /// The live session's socket, so a target change or the stop can end its blocking read.
    closer: Option<TcpStream>,
    events: VecDeque<Event>,
    session: u64,
    context_revision: u64,
    connected: bool,
    settings_interest: Option<Instant>,
    settings_request: Option<SettingsRequest>,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

/// The handle: set the target, collect events. Dropping it ends the thread and closes the connection.
pub struct Link {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Link {
    /// Starts the thread, idle until a target is set. `wake` is called (from the link's thread) after an event is queued.
    pub fn spawn(tuning: Tuning, wake: impl Fn() + Send + 'static) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        });
        let thread = {
            let shared = shared.clone();
            thread::Builder::new()
                .name("receiver-telnet".to_owned())
                .stack_size(256 * 1024)
                .spawn(move || run(&shared, &tuning, &wake))?
        };
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    /// Points the link at `target`, or at nothing. The same target again changes nothing; another one closes the connection
    /// at once, drops what it had queued and starts over.
    pub fn set_target(&self, target: Option<Target>) {
        let mut state = self.shared.state.lock();
        if state.target == target {
            return;
        }
        finish_request(&mut state, SettingsStatus::Failed, Some("target changed"));
        state.target = target;
        state.generation += 1;
        state.connected = false;
        state.settings_interest = None;
        state
            .events
            .retain(|event| matches!(event, Event::SettingsResult { .. }));
        if let Some(closer) = &state.closer {
            telnet::close(closer);
        }
        drop(state);
        self.shared.changed.notify_all();
    }

    /// Everything queued since the last call, oldest first.
    pub fn drain(&self) -> Vec<Event> {
        self.shared.state.lock().events.drain(..).collect()
    }

    pub fn refresh_settings_interest(&self) {
        self.shared.state.lock().settings_interest = Some(Instant::now() + SETTINGS_INTEREST);
        self.shared.changed.notify_all();
    }

    pub fn submit_settings_command(
        &self,
        request_id: String,
        expected_session: u64,
        expected_context: u64,
        command: Setting,
        valid_until: Instant,
        authorized: Box<dyn Fn() -> bool + Send>,
    ) -> Result<(), SettingsSubmitError> {
        if request_id.is_empty() || request_id.len() > 128 {
            return Err(SettingsSubmitError::InvalidRequestId);
        }
        let mut state = self.shared.state.lock();
        if state.settings_request.is_some()
            || state
                .events
                .iter()
                .any(|event| matches!(event, Event::SettingsResult { .. }))
        {
            return Err(SettingsSubmitError::Busy);
        }
        if !state.connected
            || state.session != expected_session
            || state.context_revision != expected_context
            || valid_until <= Instant::now()
            || !state
                .settings_interest
                .is_some_and(|until| until > Instant::now())
        {
            return Err(SettingsSubmitError::Unavailable);
        }
        let target = state
            .target
            .clone()
            .ok_or(SettingsSubmitError::Unavailable)?;
        state.settings_request = Some(SettingsRequest {
            session: state.session,
            expected_context,
            target,
            request_id,
            command,
            written: false,
            queried: false,
            quiet_since: None,
            valid_until,
            authorized,
            deadline: Instant::now() + SETTINGS_TIMEOUT,
        });
        drop(state);
        self.shared.changed.notify_all();
        Ok(())
    }
}

fn finish_request(state: &mut State, status: SettingsStatus, error: Option<&str>) {
    if let Some(request) = state.settings_request.take() {
        state.events.push_back(Event::SettingsResult {
            session: request.session,
            target: request.target,
            request_id: request.request_id,
            status,
            error: error.map(str::to_owned),
        });
    }
}

fn prerequisite_affects(index: usize, command: Setting) -> bool {
    matches!(
        (index, command),
        (
            0,
            Setting::Bass(_)
                | Setting::Treble(_)
                | Setting::ToneEnabled(_)
                | Setting::ReferenceLevel(_)
        ) | (
            1,
            Setting::DynamicEq(_) | Setting::DynamicVolume(_) | Setting::ReferenceLevel(_)
        ) | (2, Setting::Bass(_) | Setting::Treble(_))
    )
}

fn coalesce_reply(
    events: &mut VecDeque<Event>,
    session: u64,
    reply: Reply,
    preserve_changes: bool,
) {
    let mut count = 0;
    let mut last = None;
    let mut same = false;
    for (index, event) in events.iter().enumerate() {
        if let Event::SettingsReply {
            session: old_session,
            reply: old,
            ..
        } = event
            && *old_session == session
            && std::mem::discriminant(old) == std::mem::discriminant(&reply)
        {
            count += 1;
            last = Some(index);
            same = *old == reply;
        }
    }
    if let Some(index) = last
        && (!preserve_changes || same || count >= 3)
    {
        events.remove(index);
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        {
            let mut state = self.shared.state.lock();
            state.stop = true;
            if let Some(closer) = &state.closer {
                telnet::close(closer);
            }
        }
        self.shared.changed.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// How a session ended.
enum Outcome {
    /// The target changed or the link is stopping: nothing to report.
    Ended,
    Failed(ClientError),
}

fn run(shared: &Shared, tuning: &Tuning, wake: &dyn Fn()) {
    let mut failures: u32 = 0;
    loop {
        let (target, generation) = {
            let mut state = shared.state.lock();
            loop {
                if state.stop {
                    return;
                }
                if let Some(target) = &state.target {
                    break (target.clone(), state.generation);
                }
                shared.changed.wait(&mut state);
            }
        };
        let mut ready = false;
        match session(shared, tuning, &target, generation, wake, &mut ready) {
            Outcome::Ended => failures = 0,
            Outcome::Failed(error) => {
                if ready {
                    failures = 0;
                }
                push(shared, generation, Event::Down(error), wake);
                let factor = 1u32 << failures.min(10);
                failures = failures.saturating_add(1);
                let until = Instant::now() + (tuning.backoff_base * factor).min(tuning.backoff_max);
                let mut state = shared.state.lock();
                while !state.stop && state.generation == generation && Instant::now() < until {
                    shared.changed.wait_until(&mut state, until);
                }
            }
        }
    }
}

/// True once the session's target is gone or the link is stopping.
fn over(shared: &Shared, generation: u64) -> bool {
    let state = shared.state.lock();
    state.stop || state.generation != generation
}

/// Queues an event unless it belongs to an old target. A reading replaces a reading still waiting to be collected: it carries
/// the whole state, so only the newest matters.
fn push(shared: &Shared, generation: u64, event: Event, wake: &dyn Fn()) {
    {
        let mut state = shared.state.lock();
        if state.stop || state.generation != generation {
            return;
        }
        if matches!(event, Event::Reading { .. })
            && matches!(state.events.back(), Some(Event::Reading { .. }))
        {
            state.events.pop_back();
        }
        state.events.push_back(event);
    }
    wake();
}

fn session(
    shared: &Shared,
    tuning: &Tuning,
    target: &Target,
    generation: u64,
    wake: &dyn Fn(),
    ready: &mut bool,
) -> Outcome {
    let client = Client::new(target.host.clone(), target.port);
    let mut link = match Session::open(&client, target.port, tuning.connect_timeout) {
        Ok(link) => link,
        Err(e) => {
            return if over(shared, generation) {
                Outcome::Ended
            } else {
                Outcome::Failed(e)
            };
        }
    };
    let closer = match link.closer() {
        Ok(closer) => closer,
        Err(e) => {
            return Outcome::Failed(ClientError::Io(format!("{:?}", e.kind()).to_lowercase()));
        }
    };
    let session = {
        let mut state = shared.state.lock();
        if state.stop || state.generation != generation {
            return Outcome::Ended;
        }
        state.closer = Some(closer);
        state.session = state.session.wrapping_add(1);
        state.connected = true;
        state.context_revision = 0;
        let session = state.session;
        state.events.push_back(Event::SettingsConnected {
            session,
            context_revision: 0,
            target: target.clone(),
        });
        session
    };
    wake();
    let result = converse(
        shared, tuning, &mut link, target, session, generation, wake, ready,
    );
    {
        let mut state = shared.state.lock();
        state.closer = None;
        state.connected = false;
        finish_request(&mut state, SettingsStatus::Failed, Some("connection lost"));
    }
    wake();
    match result {
        // A read that fails because the socket was closed on purpose is not a failure.
        Err(_) if over(shared, generation) => Outcome::Ended,
        Err(e) => Outcome::Failed(e),
        Ok(()) => Outcome::Ended,
    }
}

#[allow(clippy::too_many_arguments)] // one conversation is made of exactly these: the shared state, the link, whom it is for and its generation
fn converse(
    shared: &Shared,
    tuning: &Tuning,
    link: &mut Session,
    target: &Target,
    session: u64,
    generation: u64,
    wake: &dyn Fn(),
    ready: &mut bool,
) -> Result<(), ClientError> {
    let started = Instant::now();
    let mut snapshot = Snapshot::default();
    let mut last_heard = started;
    let mut queue: VecDeque<&str> = QUERIES.into_iter().chain(FORMAT_QUERIES).collect();
    let mut asked_format = started;
    let mut next_query = started;
    let mut next_round = started + tuning.heartbeat;
    let mut lines = Vec::new();
    let mut settings_index = 0;
    let mut next_settings_round = started;
    let mut context: [Option<String>; 4] = Default::default();
    let mut prerequisites: [Option<Reply>; 3] = [None; 3];
    loop {
        if over(shared, generation) {
            return Ok(());
        }
        let now = Instant::now();
        if now.duration_since(last_heard) > tuning.silence {
            return Err(ClientError::Timeout);
        }
        if !*ready && now.duration_since(started) > tuning.ready_within {
            return Err(ClientError::BadResponse("no complete state"));
        }
        if queue.is_empty() && now >= next_round {
            queue.extend(QUERIES);
            queue.extend(FORMAT_QUERIES);
            next_round = now + tuning.heartbeat;
        }
        let mut sent = false;
        let mut barrier = false;
        {
            let mut state = shared.state.lock();
            if state
                .settings_request
                .as_ref()
                .is_some_and(|request| now >= request.deadline)
            {
                finish_request(
                    &mut state,
                    SettingsStatus::Failed,
                    Some("settings request timed out"),
                );
                drop(state);
                wake();
            } else if now >= next_query {
                let interested = state.settings_interest.is_some_and(|until| until > now);
                let valid_session = state.generation == generation && state.session == session;
                let context_revision = state.context_revision;
                if let Some(request) = state.settings_request.as_mut() {
                    if !valid_session {
                        finish_request(&mut state, SettingsStatus::Failed, Some("session changed"));
                        drop(state);
                        wake();
                    } else if !request.written && request.expected_context != context_revision {
                        finish_request(
                            &mut state,
                            SettingsStatus::Failed,
                            Some("receiver context changed"),
                        );
                        drop(state);
                        wake();
                    } else if !request.written && !interested {
                        finish_request(
                            &mut state,
                            SettingsStatus::Failed,
                            Some("settings interest expired"),
                        );
                        drop(state);
                        wake();
                    } else if !request.written {
                        let quiet_since = *request.quiet_since.get_or_insert(now);
                        if now.duration_since(quiet_since) < SETTINGS_QUIET {
                            barrier = true;
                        } else if Instant::now() >= request.valid_until {
                            finish_request(
                                &mut state,
                                SettingsStatus::Failed,
                                Some("settings data expired"),
                            );
                            drop(state);
                            wake();
                        } else if !(request.authorized)() {
                            finish_request(
                                &mut state,
                                SettingsStatus::Failed,
                                Some("receiver target or screen changed"),
                            );
                            drop(state);
                            wake();
                        } else if Instant::now() >= request.valid_until {
                            finish_request(
                                &mut state,
                                SettingsStatus::Failed,
                                Some("settings data expired"),
                            );
                            drop(state);
                            wake();
                        } else {
                            link.write_setting(request.command)?;
                            request.written = true;
                            request.deadline = Instant::now() + SETTINGS_TIMEOUT;
                            sent = true;
                        }
                    } else if !request.queried {
                        link.query_setting(request.command.query())?;
                        request.queried = true;
                        sent = true;
                    }
                }
            }
        }
        if now >= next_query && !sent && !barrier {
            if let Some(query) = queue.pop_front() {
                link.query(query)?;
                sent = true;
            } else {
                let interested = shared
                    .state
                    .lock()
                    .settings_interest
                    .is_some_and(|until| until > now);
                if interested && now >= next_settings_round {
                    link.query_setting(Query::ALL[settings_index])?;
                    settings_index += 1;
                    if settings_index == Query::ALL.len() {
                        settings_index = 0;
                        next_settings_round = now + SETTINGS_INTEREST;
                    }
                    sent = true;
                } else if !interested {
                    settings_index = 0;
                    next_settings_round = now;
                }
            }
        }
        if sent {
            next_query = Instant::now() + QUERY_GAP;
        }
        // Wait for data, but no longer than the next thing that has to happen.
        let mut until = last_heard + tuning.silence;
        if !*ready {
            until = until.min(started + tuning.ready_within);
        }
        if !barrier {
            until = until.min(if queue.is_empty() {
                next_round
            } else {
                next_query
            });
        }
        let settings_active = {
            let state = shared.state.lock();
            state.settings_request.is_some()
                || state
                    .settings_interest
                    .is_some_and(|expiry| expiry > Instant::now())
        };
        if settings_active {
            until = until.min(Instant::now() + SETTINGS_POLL);
            if !barrier && Instant::now() < next_query {
                until = until.min(next_query);
            }
        }
        lines.clear();
        if link.read(until.saturating_duration_since(Instant::now()), &mut lines)? {
            last_heard = Instant::now();
            if let Some(request) = shared.state.lock().settings_request.as_mut() {
                if !request.written {
                    request.quiet_since = Some(last_heard);
                }
            }
        }
        let mut heard_state = false;
        let mut volume_moved = false;
        for line in &lines {
            let settings_active = {
                let state = shared.state.lock();
                state.settings_request.is_some()
                    || state
                        .settings_interest
                        .is_some_and(|until| until > Instant::now())
            };
            if let Some(heard) = parse_line(line) {
                let context_field = match &heard {
                    telnet::Push::Power(_) => Some(0),
                    telnet::Push::Input(_) => Some(1),
                    telnet::Push::Mode(_) => Some(2),
                    telnet::Push::Signal(_) => Some(3),
                    _ => None,
                };
                if let Some(field) = context_field {
                    let value = line.trim();
                    if context[field].as_deref() != Some(value) {
                        let mut state = shared.state.lock();
                        state.context_revision = state.context_revision.wrapping_add(1);
                        let context_changed = settings_active && context[field].is_some();
                        if context_changed {
                            let field = match field {
                                0 => ContextField::Power,
                                1 => ContextField::Input,
                                2 => ContextField::Mode,
                                _ => ContextField::Signal,
                            };
                            let context_revision = state.context_revision;
                            state.events.push_back(Event::SettingsContextChanged {
                                session,
                                target: target.clone(),
                                context_revision,
                                field,
                            });
                        }
                        let own = state.settings_request.as_ref().is_some_and(|r| {
                            r.written
                                && matches!(
                                    (field, r.command),
                                    (0, Setting::Power(_))
                                        | (1, Setting::Source(_))
                                        | (2, Setting::SoundMode(_))
                                )
                        });
                        let failed = !own && state.settings_request.is_some();
                        if failed {
                            finish_request(
                                &mut state,
                                SettingsStatus::Failed,
                                Some("receiver context changed"),
                            );
                        }
                        drop(state);
                        if failed || context_changed {
                            wake();
                        }
                        context[field] = Some(value.to_owned());
                    }
                }
                let context_revision = if settings_active {
                    shared.state.lock().context_revision
                } else {
                    0
                };
                if settings_active {
                    match &heard {
                        telnet::Push::Mode(mode) => push(
                            shared,
                            generation,
                            Event::SettingsMode {
                                session,
                                context_revision,
                                target: target.clone(),
                                mode: mode.clone(),
                                at: Instant::now(),
                                at_ms: now_ms(),
                            },
                            wake,
                        ),
                        telnet::Push::Signal(signal) => push(
                            shared,
                            generation,
                            Event::SettingsSignal {
                                session,
                                context_revision,
                                target: target.clone(),
                                signal: signal.clone(),
                                at: Instant::now(),
                                at_ms: now_ms(),
                            },
                            wake,
                        ),
                        _ => {}
                    }
                }
                volume_moved |= matches!(heard, telnet::Push::Volume(_));
                snapshot.apply(heard);
                heard_state = true;
            }
            if let Some(reply) = settings::parse_reply(line) {
                let at = Instant::now();
                let mut state = shared.state.lock();
                if state.generation != generation || state.session != session {
                    continue;
                }
                let prerequisite = match reply {
                    Reply::DynamicEq(_) => Some(0),
                    Reply::MultEq(_) => Some(1),
                    Reply::ToneEnabled(_) => Some(2),
                    _ => None,
                };
                if let Some(index) = prerequisite {
                    if prerequisites[index] != Some(reply) {
                        prerequisites[index] = Some(reply);
                        state.context_revision = state.context_revision.wrapping_add(1);
                        let affected = state
                            .settings_request
                            .as_ref()
                            .is_some_and(|request| prerequisite_affects(index, request.command));
                        if affected {
                            finish_request(
                                &mut state,
                                SettingsStatus::Failed,
                                Some("settings prerequisite changed"),
                            );
                        }
                    }
                }
                if !settings_active {
                    continue;
                }
                coalesce_reply(&mut state.events, session, reply, prerequisite.is_some());
                let context_revision = state.context_revision;
                state.events.push_back(Event::SettingsReply {
                    session,
                    context_revision,
                    target: target.clone(),
                    reply,
                    line: line.clone(),
                    at,
                    at_ms: now_ms(),
                });
                let verdict = state
                    .settings_request
                    .as_ref()
                    .and_then(|request| settings_verdict(request, reply));
                if let Some((ok, error)) = verdict {
                    finish_request(
                        &mut state,
                        if ok {
                            SettingsStatus::Confirmed
                        } else {
                            SettingsStatus::Failed
                        },
                        if ok { None } else { error },
                    );
                }
                drop(state);
                wake();
            }
        }
        // The signal and the mode can change with what plays, and the receiver does not say so on its own: one more pair of questions per burst of
        // volume changes (a held key is one burst), never faster than every 800 ms.
        if volume_moved
            && queue.is_empty()
            && now.duration_since(asked_format) > Duration::from_millis(800)
        {
            queue.extend(FORMAT_QUERIES);
            asked_format = now;
        }
        if heard_state && let Some(status) = snapshot.status() {
            *ready = true;
            let event = Event::Reading {
                status,
                at: Instant::now(),
                at_ms: now_ms(),
                mode: snapshot.mode().map(str::to_owned),
                signal: snapshot.signal().map(str::to_owned),
                max_volume: snapshot.max_volume(),
            };
            push(shared, generation, event, wake);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_readback_requires_authorization_after_write() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let allowed = Arc::new(AtomicBool::new(true));
        let live = Arc::clone(&allowed);
        let request = SettingsRequest {
            session: 1,
            expected_context: 0,
            target: Target {
                host: "receiver".into(),
                port: 23,
            },
            request_id: "request".into(),
            command: Setting::Power(true),
            written: true,
            queried: true,
            quiet_since: None,
            valid_until: Instant::now() + SETTINGS_TIMEOUT,
            authorized: Box::new(move || live.load(Ordering::SeqCst)),
            deadline: Instant::now() + SETTINGS_TIMEOUT,
        };
        assert_eq!(
            settings_verdict(&request, Reply::Power(true)),
            Some((true, None))
        );
        allowed.store(false, Ordering::SeqCst);
        assert_eq!(
            settings_verdict(&request, Reply::Power(true)),
            Some((false, Some("receiver target or screen changed")))
        );
    }

    #[test]
    fn queued_prerequisite_transitions_keep_change_evidence_bounded() {
        let mut events = VecDeque::new();
        for value in [false, true, false, true, false] {
            let reply = Reply::DynamicEq(value);
            coalesce_reply(&mut events, 1, reply, true);
            events.push_back(Event::SettingsReply {
                session: 1,
                context_revision: 1,
                target: Target {
                    host: "receiver".into(),
                    port: 23,
                },
                reply,
                line: String::new(),
                at: Instant::now(),
                at_ms: 0,
            });
        }
        let values: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                Event::SettingsReply {
                    reply: Reply::DynamicEq(value),
                    ..
                } => Some(*value),
                _ => None,
            })
            .collect();
        assert_eq!(values, [false, true, false]);
    }

    #[test]
    fn changed_prerequisites_fence_dependents_not_their_own_readback() {
        let tone = lumatv_denon::settings::Tone::new(0).unwrap();
        assert!(prerequisite_affects(0, Setting::Bass(tone)));
        assert!(prerequisite_affects(1, Setting::DynamicEq(true)));
        assert!(prerequisite_affects(2, Setting::Treble(tone)));
        assert!(!prerequisite_affects(0, Setting::DynamicEq(true)));
        assert!(!prerequisite_affects(
            1,
            Setting::MultEq(lumatv_denon::settings::MultEq::Off)
        ));
        assert!(!prerequisite_affects(2, Setting::ToneEnabled(true)));
    }

    fn connected_link() -> Link {
        let state = State {
            target: Some(Target {
                host: "receiver".into(),
                port: 23,
            }),
            session: 7,
            connected: true,
            settings_interest: Some(Instant::now() + SETTINGS_INTEREST),
            ..State::default()
        };
        Link {
            shared: Arc::new(Shared {
                state: Mutex::new(state),
                changed: Condvar::new(),
            }),
            thread: None,
        }
    }

    #[test]
    fn settings_submission_is_bounded_and_fenced_by_session() {
        let link = connected_link();
        assert_eq!(
            link.submit_settings_command(
                "a".into(),
                6,
                0,
                Setting::Power(true),
                Instant::now() + SETTINGS_TIMEOUT,
                Box::new(|| true),
            ),
            Err(SettingsSubmitError::Unavailable)
        );
        assert_eq!(
            link.submit_settings_command(
                "a".into(),
                7,
                1,
                Setting::Power(true),
                Instant::now() + SETTINGS_TIMEOUT,
                Box::new(|| true),
            ),
            Err(SettingsSubmitError::Unavailable)
        );
        assert_eq!(
            link.submit_settings_command(
                "a".into(),
                7,
                0,
                Setting::Power(true),
                Instant::now() + SETTINGS_TIMEOUT,
                Box::new(|| true),
            ),
            Ok(())
        );
        assert_eq!(
            link.submit_settings_command(
                "b".into(),
                7,
                0,
                Setting::Power(false),
                Instant::now() + SETTINGS_TIMEOUT,
                Box::new(|| true),
            ),
            Err(SettingsSubmitError::Busy)
        );
    }

    #[test]
    fn target_change_reports_failure_and_keeps_result_until_drained() {
        let link = connected_link();
        assert_eq!(
            link.submit_settings_command(
                "request".into(),
                7,
                0,
                Setting::Power(true),
                Instant::now() + SETTINGS_TIMEOUT,
                Box::new(|| true),
            ),
            Ok(())
        );
        link.set_target(None);
        let events = link.drain();
        assert!(
            matches!(&events[..], [Event::SettingsResult { session: 7, request_id, status: SettingsStatus::Failed, error: Some(error), .. }] if request_id == "request" && error == "target changed")
        );
    }
}
