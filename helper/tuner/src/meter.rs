//! The meter that helps aim the antenna ([`crate::Tuner::aim`]): the demodulator's reading about once a second while someone looks at
//! it, of an RF channel the meter tunes or of the frequency the tuner is on already, with the best reading of that channel since it was
//! picked, so the antenna can be moved and the spots compared.
//!
//! The meter is nobody's stream. It tunes only a free tuner: a stream or a check asked for takes the tuner from it at once, and the TV's
//! own use (Live TV in front, or running with the panel off) keeps it from tuning and takes the tuner back at once, as it does from a
//! stream. While it does not have the tuner it reads the frequency the tuner is on and says why. It lets go once nobody has asked for a
//! reading for `meter_lease`, so a page that went away without a word does not leave the tuner held.

use crate::Status;
use crate::docs::{self, Entry};
use crate::stream::{Facade, Hold, Lease, Signal};
use parking_lot::{Condvar, Mutex};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// One reading of the meter.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    /// The RF channel asked for; `None` for the frequency the tuner is on.
    pub aim: Option<u8>,
    /// The RF channel the reading is of, when known.
    pub rf: Option<u8>,
    /// The meter has the tuner on `aim`.
    pub tuned: bool,
    /// Why the reading is not of `aim` (the TV's own use, a stream, a check), or why the meter could not tune it.
    pub note: Option<String>,
    /// What the demodulator said; `None` when it did not answer.
    pub signal: Option<Signal>,
    /// The best reading of the channel since it was picked or the best was started over.
    pub best: Option<Signal>,
}

/// What the meter's thread is asked, and its last reading.
#[derive(Default)]
struct Asked {
    aim: Option<u8>,
    /// When a reading was last asked for, on the facade's clock.
    at: Duration,
    /// A new aim, a fresh best, or the end: the thread reads again at once.
    changed: bool,
    fresh: bool,
    over: bool,
    running: bool,
    reading: Option<Reading>,
}

/// The meter: what it is asked, and its thread while it runs.
#[derive(Default)]
pub(crate) struct Meter {
    asked: Mutex<Asked>,
    wake: Condvar,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Meter {
    /// Reads `rf` (or the frequency the tuner is on) from now, starting the thread when none runs. A new aim starts the best over.
    pub(crate) fn aim(self: &Arc<Self>, facade: Arc<Facade>, rf: Option<u8>) {
        let mut asked = self.asked.lock();
        if asked.aim != rf {
            asked.reading = None;
        }
        asked.aim = rf;
        asked.at = (facade.clock)();
        asked.over = false;
        asked.changed = true;
        self.wake.notify_all();
        if asked.running {
            return;
        }
        asked.running = true;
        drop(asked);
        let meter = self.clone();
        let spawned = std::thread::Builder::new()
            .name("tuner-meter".to_owned())
            .spawn(move || meter.run(Aimer::new(facade)));
        match spawned {
            Ok(thread) => {
                if let Some(old) = self.thread.lock().replace(thread) {
                    let _ = old.join();
                }
            }
            Err(e) => {
                self.asked.lock().running = false;
                log::warn!("tuner: the meter could not start: {e}");
            }
        }
    }

    /// The latest reading while the meter runs; asking keeps it running.
    pub(crate) fn reading(&self, now: Duration) -> Option<Reading> {
        let mut asked = self.asked.lock();
        if !asked.running {
            return None;
        }
        asked.at = now;
        asked.reading.clone()
    }

    /// The best reading starts over from the next one.
    pub(crate) fn fresh(&self) {
        let mut asked = self.asked.lock();
        asked.fresh = true;
        asked.changed = true;
        self.wake.notify_all();
    }

    /// Stops the meter and waits for it to let go of the tuner.
    pub(crate) fn stop(&self) {
        {
            let mut asked = self.asked.lock();
            asked.over = true;
            asked.changed = true;
            self.wake.notify_all();
        }
        if let Some(thread) = self.thread.lock().take() {
            let _ = thread.join();
        }
    }

    fn run(&self, mut aimer: Aimer) {
        let limits = aimer.facade.limits.clone();
        loop {
            let (aim, fresh) = {
                let mut asked = self.asked.lock();
                let now = (aimer.facade.clock)();
                let done = asked.over || now.saturating_sub(asked.at) > limits.meter_lease || aimer.facade.tuning.status() == Status::Off;
                if done {
                    if aimer.held.is_none() {
                        asked.running = false;
                        asked.reading = None;
                        return;
                    }
                    // the tuner is let go of first; asked again meanwhile, the meter goes on
                    drop(asked);
                    aimer.release();
                    continue;
                }
                asked.changed = false;
                (asked.aim, std::mem::take(&mut asked.fresh))
            };
            let reading = aimer.step(aim, fresh);
            let until = Instant::now() + limits.meter_every;
            let mut asked = self.asked.lock();
            asked.reading = Some(reading);
            // the next reading is due, or something changed, or a stream or a check wants the tuner
            while !asked.changed && !aimer.wanted() {
                let left = until.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                self.wake.wait_for(&mut asked, left.min(Duration::from_millis(100)));
            }
        }
    }
}

/// The tuner held on a channel for the meter, and the lease anything asked for takes it by.
struct Held {
    rf: u8,
    hold: Box<dyn Hold>,
    lease: Lease,
}

/// The meter's work between two readings: what it holds and the best it read.
pub(crate) struct Aimer {
    facade: Arc<Facade>,
    held: Option<Held>,
    aim: Option<u8>,
    best: Option<Signal>,
    /// The RF channel the last reading was of: reading the frequency the tuner is on, another one (the TV changed channel) starts the
    /// best over.
    read_rf: Option<u8>,
    note: Option<String>,
    /// When the meter may next try to tune, and when it last asked whether the TV's own use has the tuner.
    retry: Duration,
    owner: Duration,
}

/// The RF channel of a lineup entry.
fn rf_of(entry: &Entry) -> Option<u8> {
    docs::parse_channel_id(&entry.channel_id).map(|(rf, _, _)| rf)
}

impl Aimer {
    pub(crate) fn new(facade: Arc<Facade>) -> Self {
        Self {
            facade,
            held: None,
            aim: None,
            best: None,
            read_rf: None,
            note: None,
            retry: Duration::ZERO,
            owner: Duration::ZERO,
        }
    }

    /// A stream or a check asked for the tuner the meter holds.
    fn wanted(&self) -> bool {
        self.held.as_ref().is_some_and(|h| h.lease.replaced.load(Ordering::Acquire))
    }

    /// One reading for `aim`: the tuner let go of, taken or kept as `aim` and whoever else wants it say, then the demodulator read.
    pub(crate) fn step(&mut self, aim: Option<u8>, fresh: bool) -> Reading {
        let now = (self.facade.clock)();
        if aim != self.aim || fresh {
            self.aim = aim;
            self.best = None;
        }
        if let Some(h) = self.held.as_mut() {
            let owner_due = now.saturating_sub(self.owner) >= self.facade.limits.owner_every;
            // let go of a channel no longer asked for; of a tuner a stream or a check asked for (the next reading says who has it); of one
            // the TV's own use took. The last two are not tuned again before the TV is asked again.
            let retarget = Some(h.rf) != aim;
            let gone = if retarget || h.lease.replaced.load(Ordering::Acquire) {
                Some(None)
            } else if let Some(why) = h.hold.taken() {
                Some(Some(why))
            } else if owner_due {
                self.owner = now;
                self.facade.hw.in_use().map(Some)
            } else {
                None
            };
            if let Some(why) = gone {
                self.release();
                if !retarget {
                    self.retry = now + self.facade.limits.owner_every;
                }
                self.note = why;
            }
        }
        if self.held.is_none()
            && let Some(rf) = aim
        {
            self.note = self.take(rf, now);
        }
        if aim.is_none() {
            self.note = None;
        }
        let signal = self.facade.hw.signal();
        let rf = match &self.held {
            Some(h) => Some(h.rf),
            None => self.current_rf(),
        };
        if rf != self.read_rf {
            if aim.is_none() {
                self.best = None;
            }
            self.read_rf = rf;
        }
        if (aim.is_none() || (rf.is_some() && rf == aim))
            && let Some(s) = signal
            && self.best.is_none_or(|b| s.better_than(&b))
        {
            self.best = Some(s);
        }
        Reading {
            aim,
            rf,
            tuned: self.held.is_some(),
            note: self.note.clone(),
            signal,
            best: self.best,
        }
    }

    /// Tunes `rf` when the tuner is free and the TV's own use does not have it; why not, otherwise.
    fn take(&mut self, rf: u8, now: Duration) -> Option<String> {
        let f = self.facade.clone();
        match f.tuning.status() {
            Status::Streaming { guide, .. } => return Some(format!("{guide} is streaming to a player")),
            Status::Checking { .. } => return Some("Every channel is being checked".to_owned()),
            Status::Off => return Some("The tuner is not shared".to_owned()),
            Status::Idle | Status::Busy(_) | Status::Aiming { .. } => {}
        }
        if now < self.retry {
            return self.note.clone();
        }
        self.retry = now + f.limits.owner_every;
        let in_use = f.hw.in_use();
        f.tuning.checked(in_use.clone());
        if let Some(why) = in_use {
            return Some(why);
        }
        let Some(entry) = f.entries().ok().and_then(|all| all.into_iter().find(|e| rf_of(e) == Some(rf))) else {
            return Some(format!("The TV has no channel on RF {rf}"));
        };
        // nobody's stream: anything asked for takes the tuner from the meter
        let Ok(lease) = f.tuning.take(IpAddr::V4(Ipv4Addr::UNSPECIFIED), Duration::ZERO) else {
            return Some("The tuner is busy".to_owned());
        };
        lease.over();
        match f.hw.hold(&entry.channel_id) {
            Ok(hold) => {
                self.held = Some(Held { rf, hold, lease });
                self.owner = now;
                f.tuning.change(|now| matches!(now, Status::Idle | Status::Busy(_)).then_some(Status::Aiming { rf }));
                None
            }
            Err(failure) => Some(failure.reason()),
        }
    }

    /// The RF channel the tuner is on while the meter does not hold it: a stream's, else the TV's last channel's.
    fn current_rf(&self) -> Option<u8> {
        let f = &self.facade;
        if let Status::Streaming { guide, .. } = f.tuning.status() {
            return f.entries().ok()?.iter().find(|e| e.guide == guide).and_then(rf_of);
        }
        f.hw.tuned_to().and_then(|id| docs::parse_channel_id(&id)).map(|(rf, _, _)| rf)
    }

    /// Lets go of the tuner: the TV is given back first, then the tuner is free.
    pub(crate) fn release(&mut self) {
        if let Some(Held { hold, lease, .. }) = self.held.take() {
            drop(hold);
            self.facade.tuning.change(|now| matches!(now, Status::Aiming { .. }).then_some(Status::Idle));
            drop(lease);
        }
    }
}

impl Drop for Aimer {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Grade;
    use crate::docs::Identity;
    use crate::stream::Limits;
    use crate::stream::tests::{CLEAN, Fake, PLEX, Shared, Time, fake_with, rf32, served};

    fn aimer(fake: &Arc<Fake>, time: &Time) -> Aimer {
        let identity = Identity { name: "LumaTV".into(), device_id: "0A1B2C3D".into() };
        Aimer::new(Arc::new(Facade::new(Arc::new(Shared(fake.clone())), identity, Limits::default(), time.clock(), served(), None)))
    }

    /// RF 29 on 2026-11-05.
    const WEAK: Signal = Signal { locked: true, mse: Some(355), snr: Some(17.8), errors: Some(81), ber: Some(584_211) };

    #[test]
    fn the_meter_tunes_a_free_tuner_and_keeps_the_best_reading_of_the_channel_picked() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), vec![]);
        let mut a = aimer(&fake, &time);
        *fake.signal.lock() = Some(WEAK);
        let r = a.step(Some(32), false);
        assert_eq!((r.rf, r.tuned, r.note.as_deref()), (Some(32), true, None));
        assert_eq!(*fake.held.lock(), ["1_32_2_1_0_0_0"], "the first channel of RF 32");
        assert_eq!(a.facade.tuning.status(), Status::Aiming { rf: 32 });
        assert_eq!(r.signal.map(|s| s.grade()), Some(Grade::Poor));
        *fake.signal.lock() = Some(CLEAN);
        assert_eq!(a.step(Some(32), false).best, Some(CLEAN), "the antenna moved to a better spot");
        *fake.signal.lock() = Some(WEAK);
        let r = a.step(Some(32), false);
        assert_eq!((r.signal, r.best), (Some(WEAK), Some(CLEAN)), "a worse one leaves the best");
        assert_eq!(a.step(Some(32), true).best, Some(WEAK), "started over");
        // the frequency the tuner is on: the meter lets go at once and reads the TV's last channel's
        *fake.last.lock() = Some("1_29_16_1_0_0_0".to_owned());
        let r = a.step(None, false);
        assert_eq!((r.rf, r.tuned, r.best), (Some(29), false, Some(WEAK)));
        assert_eq!((fake.released.load(Ordering::Relaxed), a.facade.tuning.status()), (1, Status::Idle));
    }

    #[test]
    fn the_meter_gives_the_tuner_up_to_a_stream_and_to_the_tvs_own_use() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), vec![]);
        let mut a = aimer(&fake, &time);
        *fake.signal.lock() = Some(CLEAN);
        assert!(a.step(Some(32), false).tuned);
        // a stream asked for waits for the meter, which lets go at its next look and reads what the stream tuned
        assert!(a.facade.tuning.take(PLEX, Duration::ZERO).is_err());
        assert!(a.wanted());
        assert!(!a.step(Some(32), false).tuned);
        let stream = a.facade.tuning.take(PLEX, Duration::ZERO).expect("free for the stream");
        a.facade.tuning.streaming(&a.facade.entries().unwrap()[1]);
        let r = a.step(Some(32), false);
        assert_eq!((r.tuned, r.note.as_deref(), r.rf), (false, Some("2.4 is streaming to a player"), Some(32)));
        drop(stream);
        a.facade.tuning.change(|_| Some(Status::Idle));
        time.advance(Duration::from_secs(5));
        assert!(a.step(Some(32), false).tuned, "the stream over, the meter tunes again");
        // the TV's own use takes the tuner back, and keeps the meter off while it has it
        *fake.taken.lock() = Some("the TV took the tuner (BROADCAST_STATE_UNREALIZED)".to_owned());
        *fake.owner.lock() = Some("Live TV is in front".to_owned());
        let r = a.step(Some(32), false);
        assert_eq!((r.tuned, r.note.as_deref()), (false, Some("the TV took the tuner (BROADCAST_STATE_UNREALIZED)")));
        time.advance(Duration::from_secs(5));
        assert_eq!(a.step(Some(32), false).note.as_deref(), Some("Live TV is in front"));
        assert_eq!(a.facade.tuning.status(), Status::Busy("Live TV is in front".to_owned()));
        assert_eq!((fake.held.lock().len(), fake.released.load(Ordering::Relaxed)), (2, 2), "not tuned again while the TV has it");
    }
}
