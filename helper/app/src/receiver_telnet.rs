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
//! Only queries are ever sent (`Session::query` refuses everything else); the existing commands stay on HTTP.

use crate::timeutil::now_ms;
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
        state.target = target;
        state.generation += 1;
        state.events.clear();
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
    {
        let mut state = shared.state.lock();
        if state.stop || state.generation != generation {
            return Outcome::Ended;
        }
        state.closer = Some(closer);
    }
    let result = converse(shared, tuning, &mut link, generation, wake, ready);
    shared.state.lock().closer = None;
    match result {
        // A read that fails because the socket was closed on purpose is not a failure.
        Err(_) if over(shared, generation) => Outcome::Ended,
        Err(e) => Outcome::Failed(e),
        Ok(()) => Outcome::Ended,
    }
}

fn converse(
    shared: &Shared,
    tuning: &Tuning,
    link: &mut Session,
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
        if let Some(query) = queue.front()
            && now >= next_query
        {
            link.query(query)?;
            queue.pop_front();
            next_query = now + QUERY_GAP;
        }
        // Wait for data, but no longer than the next thing that has to happen.
        let mut until = last_heard + tuning.silence;
        if !*ready {
            until = until.min(started + tuning.ready_within);
        }
        until = until.min(if queue.is_empty() {
            next_round
        } else {
            next_query
        });
        lines.clear();
        if link.read(until.saturating_duration_since(Instant::now()), &mut lines)? {
            last_heard = Instant::now();
        }
        let mut heard_state = false;
        let mut volume_moved = false;
        for line in &lines {
            if let Some(heard) = parse_line(line) {
                volume_moved |= matches!(heard, telnet::Push::Volume(_));
                snapshot.apply(heard);
                heard_state = true;
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
