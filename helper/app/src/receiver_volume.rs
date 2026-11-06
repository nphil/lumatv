//! The receiver's volume as the display's number drives it: the target the TV shows is sent to the Denon as an absolute level.
//!
//! The volume bar counts a held key by itself (`rustui/ui/src/system/volume.rs`) and posts the level it shows. This thread sends
//! that level to the receiver (`MVxx` or `MVxx5` over its web port), at most one command per `receiver.volumeGapMs` with the newest
//! level only: a hold's intermediate levels are skipped, never queued. A command that fails is sent again after the gap.
//!
//! LG's audio service passes every key on over HDMI-CEC as well, and the Denon takes each step 130 to 300 ms after its key, often
//! after the level sent for it (measured on the set, `docs/research/volume-osd-result.md`). The receiver's own reports (`heard`,
//! every level the topic takes) say where it is: once it has reported the level sent, a report of another level within
//! `receiver.volumeHoldMs` of the last ask is such a step, and the level goes again once the asks have been quiet for
//! `receiver.volumeSettleMs` (sooner, a hold's next level would only be overtaken). A level never reported is read back that long
//! after its command and sent again if the receiver is elsewhere. Either way at most `receiver.volumeRetries` times; after the
//! hold a change is the receiver's own (its remote, an app).
//!
//! The level is held to the receiver's own limit (`volumeMax` in the `receiver` topic, from its `MVMAX`) when it is known.

use crate::ctx::Ctx;
use crate::receiver::TOPIC;
use lumatv_denon::{Client, volume_command};
use parking_lot::{Condvar, Mutex};
use std::io;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The panel's scale ends here; the receiver steps in halves.
pub const MAX_LEVEL: f64 = 98.0;

/// Consecutive failed sends of one level before it is given up.
const SEND_TRIES: u32 = 3;

/// A level on the receiver's scale, held to `0..=98` and rounded to its half steps the way `volume_command` does.
pub fn quantize(level: f64) -> Option<f64> {
    level
        .is_finite()
        .then(|| ((level.clamp(0.0, MAX_LEVEL) * 2.0 + 0.5).floor() / 2.0).min(MAX_LEVEL))
}

fn same(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.25
}

/// What the thread does next.
#[derive(Debug, PartialEq)]
enum Next {
    Send(f64),
    /// Nothing was heard of the last command: read the receiver.
    Check(f64),
    Wait(Instant),
    Idle,
}

/// When to send what: pure, on the clock it is given.
#[derive(Debug)]
struct Pacer {
    gap: Duration,
    settle: Duration,
    hold: Duration,
    retries: u32,
    /// The level the receiver is to end on, the newest asked; `None` once nothing is left to do for it.
    target: Option<f64>,
    asked_at: Option<Instant>,
    /// When the target is to go out (the gap after the last command permitting): newly asked, its send failed, or a correction.
    due: Option<Instant>,
    last_send: Option<Instant>,
    /// The receiver has reported the target since it was last sent.
    landed: bool,
    fixes: u32,
    failures: u32,
}

impl Pacer {
    fn new(gap: Duration, settle: Duration, hold: Duration, retries: u32) -> Self {
        Self {
            gap,
            settle,
            hold,
            retries,
            target: None,
            asked_at: None,
            due: None,
            last_send: None,
            landed: false,
            fixes: 0,
            failures: 0,
        }
    }

    fn ask(&mut self, level: f64, now: Instant) {
        if self.target.is_none_or(|t| !same(t, level)) {
            self.fixes = 0;
            self.failures = 0;
        }
        self.target = Some(level);
        self.asked_at = Some(now);
        self.due = Some(now);
        self.landed = false;
    }

    /// A level the receiver reported.
    fn heard(&mut self, level: f64, now: Instant) {
        let Some(target) = self.target else { return };
        if self.due.is_some() {
            return;
        }
        if same(level, target) {
            self.landed = true;
        } else if self.landed && self.asked_at.is_some_and(|at| now < at + self.hold) {
            // it was there and moved: a CEC step that landed late
            self.landed = false;
            self.correct();
        }
    }

    /// The read-back of the target: `None` when the receiver could not be read.
    fn checked(&mut self, read: Option<f64>) {
        let Some(target) = self.target else { return };
        match read {
            Some(level) if same(level, target) => self.landed = true,
            Some(_) => self.correct(),
            None => self.target = None,
        }
    }

    fn correct(&mut self) {
        if self.fixes < self.retries {
            self.fixes += 1;
            self.due = self.asked_at.map(|at| at + self.settle);
        } else {
            self.target = None;
        }
    }

    /// A send of `level` finished at `now`.
    fn sent(&mut self, level: f64, ok: bool, now: Instant) {
        self.last_send = Some(now);
        if self.target.is_none_or(|t| !same(t, level)) {
            return;
        }
        if ok {
            self.due = None;
            self.failures = 0;
        } else {
            self.failures += 1;
            if self.failures >= SEND_TRIES {
                self.target = None;
            }
        }
    }

    fn next(&mut self, now: Instant) -> Next {
        let Some(target) = self.target else {
            return Next::Idle;
        };
        if let Some(due) = self.due {
            let ready = self.last_send.map_or(due, |at| due.max(at + self.gap));
            return if now >= ready {
                Next::Send(target)
            } else {
                Next::Wait(ready)
            };
        }
        if self.landed {
            // listening for a late step until the hold is over
            let end = self.asked_at.map_or(now, |at| at + self.hold);
            if now >= end {
                self.target = None;
                return Next::Idle;
            }
            return Next::Wait(end);
        }
        let check_at = self.last_send.map_or(now, |at| at + self.settle);
        if now >= check_at {
            Next::Check(target)
        } else {
            Next::Wait(check_at)
        }
    }
}

struct State {
    pacer: Pacer,
    stop: bool,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

/// The handle. Dropping it ends the thread.
pub struct VolumeDriver {
    ctx: Ctx,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl VolumeDriver {
    pub fn spawn(ctx: &Ctx) -> io::Result<Self> {
        let tuning = &ctx.config.receiver;
        let pacer = Pacer::new(
            Duration::from_millis(tuning.volume_gap_ms),
            Duration::from_millis(tuning.volume_settle_ms),
            Duration::from_millis(tuning.volume_hold_ms),
            tuning.volume_retries,
        );
        let shared = Arc::new(Shared {
            state: Mutex::new(State { pacer, stop: false }),
            changed: Condvar::new(),
        });
        let thread = {
            let (shared, ctx) = (shared.clone(), ctx.clone());
            thread::Builder::new()
                .name("receiver-volume".to_owned())
                .stack_size(256 * 1024)
                .spawn(move || run(&ctx, &shared))?
        };
        Ok(Self {
            ctx: ctx.clone(),
            shared,
            thread: Some(thread),
        })
    }

    /// Asks for `level` (0 to 98): the newest ask replaces one that has not been sent yet.
    pub fn set(&self, level: f64) {
        let level = limited(&self.ctx, level);
        self.shared.state.lock().pacer.ask(level, Instant::now());
        self.shared.changed.notify_all();
    }

    /// The receiver reported `level`.
    pub fn heard(&self, level: f64) {
        self.shared.state.lock().pacer.heard(level, Instant::now());
        self.shared.changed.notify_all();
    }
}

impl Drop for VolumeDriver {
    fn drop(&mut self) {
        self.shared.state.lock().stop = true;
        self.shared.changed.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(ctx: &Ctx, shared: &Shared) {
    let tuning = &ctx.config.receiver;
    let mut client: Option<((String, u16), Client)> = None;
    let mut failing = false;
    loop {
        let next = {
            let mut state = shared.state.lock();
            loop {
                if state.stop {
                    return;
                }
                match state.pacer.next(Instant::now()) {
                    Next::Wait(at) => {
                        shared.changed.wait_until(&mut state, at);
                    }
                    Next::Idle => shared.changed.wait(&mut state),
                    next => break next,
                }
            }
        };
        let settings = ctx.settings.receiver();
        let target = (settings.host.clone(), settings.port);
        if client.as_ref().map(|(t, _)| t) != Some(&target) {
            let c = Client::new(settings.host.clone(), settings.port).with_timeouts(
                Duration::from_millis(tuning.connect_timeout_ms),
                Duration::from_millis(tuning.request_timeout_ms),
            );
            client = Some((target, c));
        }
        let Some((_, client)) = &client else { continue };
        if !settings.enabled {
            shared.state.lock().pacer.target = None;
            continue;
        }
        match next {
            Next::Send(level) => {
                let result = match volume_command(level) {
                    Some(command) => client.send_command(&command),
                    None => Ok(()),
                };
                match &result {
                    Ok(()) if failing => {
                        crate::log_info!("receiver: volume commands reach {}", client.host());
                        failing = false;
                    }
                    Err(e) if !failing => {
                        crate::log_warn!("receiver: volume command to {}: {e}", client.host());
                        failing = true;
                    }
                    _ => {}
                }
                shared
                    .state
                    .lock()
                    .pacer
                    .sent(level, result.is_ok(), Instant::now());
            }
            Next::Check(_) => {
                let read = client.status().ok().and_then(|s| s.volume_absolute());
                shared.state.lock().pacer.checked(read);
            }
            Next::Wait(_) | Next::Idle => {}
        }
    }
}

/// `level` held to the receiver's own limit, if the topic says one, and to its half steps.
fn limited(ctx: &Ctx, level: f64) -> f64 {
    let limit = ctx
        .hub
        .get(TOPIC)
        .and_then(|doc| doc.get("volumeMax").and_then(serde_json::Value::as_f64))
        .filter(|m| *m > 0.0);
    let level = limit.map_or(level, |m| level.min(m));
    quantize(level).unwrap_or(level)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_are_held_to_the_scale_and_its_half_steps() {
        assert_eq!(quantize(46.5), Some(46.5));
        assert_eq!(quantize(46.74), Some(46.5));
        assert_eq!(quantize(46.75), Some(47.0));
        assert_eq!(quantize(-3.0), Some(0.0));
        assert_eq!(quantize(120.0), Some(98.0));
        assert_eq!(quantize(f64::NAN), None);
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// The pacer on a fake clock: `base` plus milliseconds. Every send it asks for goes out at once and `send_ok` says whether it
    /// worked; a read-back reads `read`.
    struct Rig {
        pacer: Pacer,
        base: Instant,
        sends: Vec<(u64, f64)>,
        checks: usize,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                pacer: Pacer::new(ms(100), ms(400), ms(1500), 3),
                base: Instant::now(),
                sends: Vec::new(),
                checks: 0,
            }
        }

        fn at(&self, t: u64) -> Instant {
            self.base + ms(t)
        }

        fn run(&mut self, from: u64, to: u64, send_ok: bool, read: Option<f64>) {
            for t in from..to {
                loop {
                    match self.pacer.next(self.at(t)) {
                        Next::Send(level) => {
                            self.sends.push((t, level));
                            self.pacer.sent(level, send_ok, self.at(t));
                        }
                        Next::Check(_) => {
                            self.checks += 1;
                            self.pacer.checked(read);
                        }
                        Next::Wait(_) | Next::Idle => break,
                    }
                }
            }
        }

        fn ask(&mut self, t: u64, level: f64) {
            let at = self.at(t);
            self.pacer.ask(level, at);
        }

        fn heard(&mut self, t: u64, level: f64) {
            let at = self.at(t);
            self.pacer.heard(level, at);
        }
    }

    #[test]
    fn a_hold_sends_the_newest_level_at_most_once_a_gap_and_ends_on_the_last() {
        let mut rig = Rig::new();
        let mut level = 40.0;
        for t in (0..1000).step_by(40) {
            level += 0.5;
            rig.ask(t, level);
            rig.heard(t, level - 2.0);
            rig.run(t, t + 40, true, None);
        }
        rig.run(1000, 1200, true, None);
        assert!(
            rig.sends.windows(2).all(|w| w[1].0 - w[0].0 >= 100),
            "{:?}",
            rig.sends
        );
        assert!(
            rig.sends.len() <= 11,
            "{} sends in a second",
            rig.sends.len()
        );
        assert_eq!(
            rig.sends.last().map(|s| s.1),
            Some(level),
            "the last level asked is the last sent"
        );
    }

    #[test]
    fn a_send_that_fails_goes_again_after_the_gap_and_three_failures_give_the_level_up() {
        let mut rig = Rig::new();
        rig.ask(0, 40.0);
        rig.run(0, 150, false, None);
        assert_eq!(rig.sends, vec![(0, 40.0), (100, 40.0)]);
        rig.run(150, 1000, false, None);
        assert_eq!(rig.sends.len(), 3);
        assert_eq!(rig.pacer.next(rig.at(1000)), Next::Idle);
    }

    /// A held Volume+ recorded on the set (2026-11-05, `docs/research/volume-osd-result.md`): the levels the bar posted (ms, level)
    /// and what the receiver then reported (ms, level), its own CEC steps among them; the last, 40.0 at 1194 ms, landed 285 ms after
    /// the last level and after the receiver had reported it.
    const POSTED: [(u64, f64); 6] = [
        (10, 37.0),
        (310, 37.5),
        (507, 38.0),
        (617, 38.5),
        (809, 39.0),
        (909, 39.5),
    ];
    const REPORTED: [(u64, f64); 7] = [
        (48, 37.0),
        (179, 37.5),
        (599, 38.0),
        (838, 38.5),
        (1038, 39.0),
        (1078, 39.5),
        (1194, 40.0),
    ];

    #[test]
    fn a_recorded_hold_is_put_right_once_after_the_last_level_and_a_change_after_the_hold_stands() {
        let mut rig = Rig::new();
        let mut events: Vec<(u64, bool, f64)> = POSTED.iter().map(|&(t, l)| (t, true, l)).collect();
        events.extend(REPORTED.iter().map(|&(t, l)| (t, false, l)));
        // the receiver's answer to the correction, and later a change made at the receiver itself
        events.extend([(1350, false, 39.5), (2500, false, 42.0)]);
        events.sort_by_key(|e| e.0);
        let mut i = 0;
        for t in 0..3000 {
            while let Some(&(at, ask, level)) = events.get(i).filter(|e| e.0 == t) {
                if ask {
                    rig.ask(at, level)
                } else {
                    rig.heard(at, level)
                }
                i += 1;
            }
            rig.run(t, t + 1, true, None);
        }
        let posted: Vec<(u64, f64)> = POSTED.to_vec();
        assert_eq!(
            rig.sends[..6],
            posted[..],
            "every level as the bar posted it, no correction while the keys came"
        );
        assert_eq!(
            rig.sends[6..],
            [(1309, 39.5)],
            "the late step put right once the asks were quiet for the settle"
        );
        assert_eq!(rig.checks, 0, "every level was heard: nothing read back");
    }

    #[test]
    fn a_level_never_reported_is_read_back_and_sent_again_when_the_receiver_is_elsewhere() {
        let mut rig = Rig::new();
        rig.ask(0, 40.0);
        rig.run(0, 410, true, Some(40.5));
        assert_eq!(
            (rig.sends.clone(), rig.checks),
            (vec![(0, 40.0), (400, 40.0)], 1)
        );
        rig.run(410, 2000, true, Some(40.0));
        assert_eq!(
            (rig.sends.len(), rig.checks),
            (2, 2),
            "read back once more, and there"
        );
        assert_eq!(rig.pacer.next(rig.at(2000)), Next::Idle);
    }
}
