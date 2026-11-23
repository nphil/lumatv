//! The echo canceller of the voice path: what the TV plays, taken out of what the microphone hears.
//!
//! The microphones sit in the TV's bezel and its sound comes out of a receiver across the room, so while anything plays the wake word
//! hears the TV louder than it hears a person. The TV's mixer gives the sound back as a reference (a second stream, read by the
//! server), and the Show's own canceller, `voice_dsp::aec`, is a partitioned frequency-domain adaptive filter that subtracts the
//! reference's copy from the microphone. It reaches back 128 ms; this module makes the two streams meet inside that reach.
//!
//! - [`crate::delay`] reads the lag between the streams from their sound (it is the receiver's processing, the air and the two capture
//!   paths' buffers, and runs to hundreds of milliseconds).
//! - The lag is applied to the reference. A reference that arrives after the echo it belongs to is waited for: the microphone is held
//!   back by the difference, which is the price of the cancellation and goes away with the reference.
//! - Until a lag is found, and whenever the reference stops, the microphone passes through unchanged (bit for bit: the filter does
//!   nothing while its reference is silent) and the wake word's strictness is raised by the TV's level as before.
//!
//! The streams are placed on one timeline by their first samples: a reference stream begins at the microphone's position when it was
//! opened. Sample counts, not arrival times, give the positions, so the jitter of when the reads return does not move the lag.

use crate::delay::{Estimate, Estimator, Search};
use voice_dsp::aec::{Aec, AecConfig};

const RATE: usize = 16_000;
/// Reference samples between estimates while searching, and once locked (the lag is checked, not found again).
const SEARCH_EVERY: usize = RATE / 2;
const VERIFY_EVERY: usize = RATE * 8;
/// The confidence of a lag worth remembering, and the one that is enough on its own (noise is about 4).
const LOCK_CONFIDENCE: f32 = 8.0;
const SURE_CONFIDENCE: f32 = 16.0;
/// Two estimates agree within 8 ms.
const AGREE: i32 = 128;
/// A lag that is locked is moved only by one further than 2 ms away: the estimate has a resolution of a quarter of a millisecond.
const KEEP: i32 = 32;
/// Estimates that heard sound in the reference and found no copy in the microphone before the reference is given up on (10 s).
const GIVE_UP: u32 = 20;
/// How long beyond the lag's own hold the microphone waits for a reference that does not arrive: 500 ms.
const HOLD_SLACK: usize = RATE / 2;
/// A reference mean square under which the filter sees silence (-76 dBFS, the filter's own threshold), full scale being 1.
const FAR_ACTIVE: f32 = 2.5e-8;
/// The microphone's noise floor until the room has been heard without the TV (-62 dBFS after the gain).
const FLOOR_START: f32 = 6.3e-7;
/// The echo has to be this much over the floor for a reduction to be told.
const ABOVE_FLOOR: f32 = 4.0;
/// The most the cancellation is believed to take off the TV's level at the wake word, in dB.
const MAX_CREDIT_DB: f32 = 24.0;
/// How much loud reference a lock has to show itself in before it is believed, and the least it has to take off by then (dB): a lag
/// found by chance in a stream that is not the room's takes nothing off, and is given up for a new search.
const PROOF: usize = RATE * 8;
const MIN_ERLE_DB: f32 = 3.0;

#[derive(Clone, Debug)]
pub struct EchoConfig {
    /// Off: the microphone reaches the wake word as it is, and the reference is read for its level alone.
    pub enabled: bool,
    /// The filter: its `partitions` of 16 ms are the room's tail it covers after the direct sound (8 is 128 ms), and its steps.
    pub aec: AecConfig,
    pub search: Search,
    /// How far into the filter's reach the echo's strongest arrival is put, in milliseconds, so that an earlier arrival is inside it.
    pub margin_ms: u32,
}

impl Default for EchoConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            // The step is divided by the reference's power over all the partitions, so twice the partitions halve the step per tap:
            // the Show's 0.15 at 4 becomes 0.3 at 8.
            aec: AecConfig {
                partitions: 8,
                mu: 0.3,
                ..AecConfig::default()
            },
            search: Search::default(),
            margin_ms: 10,
        }
    }
}

impl EchoConfig {
    pub fn check(&self) -> Result<(), String> {
        if self.aec.partitions == 0
            || !self.aec.block.is_power_of_two()
            || self.aec.rate != RATE as u32
        {
            return Err(
                "the echo filter needs a power-of-two block, at least one partition and 16 kHz"
                    .into(),
            );
        }
        if !self.search.fits() {
            return Err("the echo search is longer than its transform: window, before and after together are 4 s at most".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EchoState {
    /// No reference is being read (the TV is not playing, or the canceller is off): the microphone is as it is.
    #[default]
    Off,
    /// The reference is open and its lag is not known yet.
    Searching,
    /// The lag is known and the filter is subtracting the reference.
    Cancelling,
    /// The reference is open and nothing in the microphone matches it (the stream is stalled, or is not the TV's sound): the microphone
    /// is as it is, and the wake word is made stricter by the TV's level instead.
    Lost,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EchoStatus {
    pub state: EchoState,
    /// How late the echo is behind the reference as locked, in milliseconds (negative: the reference comes after the echo).
    pub lag_ms: Option<f32>,
    /// The correlation's confidence at the lock.
    pub confidence: f32,
    /// How much the filter takes out of the microphone while the TV plays, in dB, once the echo is over the room's noise.
    pub erle_db: Option<f32>,
    /// How much of the microphone is waiting for the reference, in milliseconds.
    pub held_ms: f32,
    /// Times a lag has been locked or moved since the microphone opened.
    pub locks: u32,
}

#[derive(Clone, Copy)]
struct Lock {
    lag: i32,
    /// The samples the reference is put later by: `lag` less the margin (negative: the microphone waits).
    applied: i64,
    confidence: f32,
}

/// The reduction the filter makes: smoothed powers of what goes in and what comes out while the reference is loud.
struct Erle {
    in_sum: f64,
    in_n: usize,
    out_sum: f64,
    out_n: usize,
    p_in: f32,
    p_out: f32,
    floor: f32,
    floor_known: bool,
    quiet_sum: f64,
    quiet_n: usize,
    /// Microphone samples since the reference was last loud.
    silent_for: usize,
}

impl Erle {
    const WINDOW: usize = RATE / 10;
    /// The room rings for more than 100 ms after the reference stops.
    const TAIL: usize = RATE / 2;

    fn new() -> Self {
        Self {
            in_sum: 0.0,
            in_n: 0,
            out_sum: 0.0,
            out_n: 0,
            p_in: 0.0,
            p_out: 0.0,
            floor: FLOOR_START,
            floor_known: false,
            quiet_sum: 0.0,
            quiet_n: 0,
            silent_for: 0,
        }
    }

    fn square(samples: &[i16]) -> f64 {
        samples
            .iter()
            .map(|&s| (f64::from(s) / 32_768.0).powi(2))
            .sum()
    }

    fn restart(&mut self) {
        self.in_n = 0;
        self.out_n = 0;
        self.in_sum = 0.0;
        self.out_sum = 0.0;
        self.p_in = 0.0;
        self.p_out = 0.0;
    }

    /// Microphone samples that went in and the cleaned ones that came out, while the reference was loud.
    fn heard(&mut self, mic: &[i16], cleaned: &[i16]) {
        self.silent_for = 0;
        self.in_sum += Self::square(mic);
        self.in_n += mic.len();
        self.out_sum += Self::square(cleaned);
        self.out_n += cleaned.len();
        if self.in_n >= Self::WINDOW {
            let p_in = (self.in_sum / self.in_n as f64) as f32;
            let p_out = if self.out_n > 0 {
                (self.out_sum / self.out_n as f64) as f32
            } else {
                p_in
            };
            if self.p_in == 0.0 {
                (self.p_in, self.p_out) = (p_in, p_out);
            } else {
                self.p_in += 0.2 * (p_in - self.p_in);
                self.p_out += 0.2 * (p_out - self.p_out);
            }
            self.in_n = 0;
            self.out_n = 0;
            self.in_sum = 0.0;
            self.out_sum = 0.0;
        }
    }

    /// Microphone samples while the reference was silent, once the tail of what was loud has died away: the room's noise floor is the
    /// lowest this goes, and rises slowly, so that a person talking while the TV is silent is not taken for noise.
    fn quiet(&mut self, mic: &[i16]) {
        self.silent_for += mic.len();
        if self.silent_for <= Self::TAIL {
            return;
        }
        self.quiet_sum += Self::square(mic);
        self.quiet_n += mic.len();
        if self.quiet_n >= Self::WINDOW {
            let p = (self.quiet_sum / self.quiet_n as f64) as f32;
            self.floor = if !self.floor_known || p < self.floor {
                p
            } else {
                self.floor + 0.01 * (p - self.floor)
            };
            self.floor_known = true;
            self.quiet_n = 0;
            self.quiet_sum = 0.0;
        }
    }

    fn db(&self) -> Option<f32> {
        (self.p_in > ABOVE_FLOOR * self.floor)
            .then(|| 10.0 * (self.p_in / self.p_out.max(1e-12)).log10())
    }
}

pub struct Canceller {
    est: Option<Estimator>,
    aec: Aec,
    margin: i64,
    /// How far the lag can reach back or forward, in samples, for the reference's ring.
    reach: usize,
    open: bool,

    /// The microphone waiting for the filter: samples from position `done` to `total`.
    pending: Vec<i16>,
    total: u64,
    done: u64,
    far: Vec<i16>,

    /// The reference from position `ring_at` on the microphone's timeline.
    ring: Vec<i16>,
    ring_at: i64,
    /// The reference's samples received since it opened.
    received: usize,

    lock: Option<Lock>,
    lost: bool,
    candidate: Option<i32>,
    differing: Option<i32>,
    weak: u32,
    /// Samples of loud reference since the lock was found or last proved.
    proving: usize,
    next_at: usize,
    /// The reference a first estimate needs: a window of microphone and the reach before it.
    first: usize,
    locks: u32,
    erle: Erle,
}

impl Canceller {
    pub fn new(cfg: &EchoConfig) -> Self {
        let est = if cfg.enabled && cfg.check().is_ok() {
            Estimator::new(cfg.search)
        } else {
            None
        };
        let after = cfg.search.after_ms as usize * RATE / 1_000;
        let margin = (cfg.margin_ms as usize * RATE / 1_000) as i64;
        let aec = Aec::new(if est.is_some() {
            cfg.aec
        } else {
            AecConfig::default()
        });
        Self {
            est,
            aec,
            margin,
            reach: after + margin as usize + RATE / 4,
            open: false,
            pending: Vec::new(),
            total: 0,
            done: 0,
            far: Vec::new(),
            ring: Vec::new(),
            ring_at: 0,
            received: 0,
            lock: None,
            lost: false,
            candidate: None,
            differing: None,
            weak: 0,
            proving: 0,
            next_at: 0,
            first: (cfg.search.window_ms + cfg.search.before_ms) as usize * RATE / 1_000,
            locks: 0,
            erle: Erle::new(),
        }
    }

    pub fn status(&self) -> EchoStatus {
        let state = match (&self.est, self.open) {
            (Some(_), true) if self.lock.is_some() => EchoState::Cancelling,
            (Some(_), true) if self.lost => EchoState::Lost,
            (Some(_), true) => EchoState::Searching,
            _ => EchoState::Off,
        };
        EchoStatus {
            state,
            lag_ms: self.lock.map(|l| l.lag as f32 * 1_000.0 / RATE as f32),
            confidence: self.lock.map_or(0.0, |l| l.confidence),
            erle_db: if state == EchoState::Cancelling {
                self.erle.db()
            } else {
                None
            },
            held_ms: if self.lock.is_some() {
                self.pending.len() as f32 * 1_000.0 / RATE as f32
            } else {
                0.0
            },
            locks: self.locks,
        }
    }

    /// What the cancellation is worth at the wake word, in dB of the TV's level: nothing until it is cancelling and measured.
    pub fn credit_db(&self) -> f32 {
        match (self.lock, self.erle.db()) {
            (Some(_), Some(db)) if self.open => db.clamp(0.0, MAX_CREDIT_DB),
            _ => 0.0,
        }
    }

    /// Freezes or releases the filter's learning (it is frozen while a turn is open, so that the user's voice is not learned as echo).
    pub fn set_adapt(&mut self, adapt: bool) {
        self.aec.set_adapt(adapt);
    }

    /// A reference stream begins now: its first sample is taken to be the microphone's next. One that is still open is ended first.
    pub fn open_reference(&mut self, out: &mut Vec<i16>) {
        self.close_reference(out);
        let Some(est) = self.est.as_mut() else { return };
        est.open();
        self.open = true;
        self.ring.clear();
        self.ring_at = self.total as i64;
        self.received = 0;
        self.lock = None;
        self.lost = false;
        self.candidate = None;
        self.differing = None;
        self.weak = 0;
        self.proving = 0;
        self.next_at = self.first;
        self.erle.restart();
    }

    /// The reference stream ended: what the microphone was held for is let go as it is.
    pub fn close_reference(&mut self, out: &mut Vec<i16>) {
        if !self.open {
            return;
        }
        self.open = false;
        self.lock = None;
        self.lost = false;
        self.ring.clear();
        if let Some(est) = self.est.as_mut() {
            est.close();
        }
        self.pump(out);
    }

    /// The reference stopped arriving while its stream is open: the microphone is let go as it is until the lag is found again.
    pub fn stalled(&mut self, out: &mut Vec<i16>) {
        if self.open && self.lock.is_some() {
            self.lose();
            self.pump(out);
        }
    }

    /// The microphone's next samples (16 kHz mono). What comes out is what the wake word should hear: in order, all of it in the end,
    /// less than one block behind unless the microphone is waiting for the reference.
    pub fn push_mic(&mut self, mic: &[i16], out: &mut Vec<i16>) {
        let Some(est) = self.est.as_mut() else {
            out.extend_from_slice(mic);
            return;
        };
        est.push_mic(mic);
        self.total += mic.len() as u64;
        self.pending.extend_from_slice(mic);
        self.pump(out);
    }

    /// The reference's next samples (16 kHz mono, from the same mixer the TV plays).
    pub fn push_reference(&mut self, reference: &[i16], out: &mut Vec<i16>) {
        if !self.open {
            return;
        }
        let Some(est) = self.est.as_mut() else { return };
        est.push_reference(reference);
        self.ring.extend_from_slice(reference);
        self.received += reference.len();
        self.track();
        self.trim();
        self.pump(out);
    }

    fn ring_end(&self) -> i64 {
        self.ring_at + self.ring.len() as i64
    }

    /// Drops the reference no lock can ask for again: the filter's input is never older than `reach` behind the microphone's position.
    fn trim(&mut self) {
        let oldest = self.done as i64 - self.reach as i64;
        let cut = (oldest - self.ring_at).clamp(0, self.ring.len() as i64) as usize;
        if cut > RATE {
            self.ring.drain(..cut);
            self.ring_at += cut as i64;
        }
    }

    fn lose(&mut self) {
        self.lock = None;
        self.lost = true;
        self.candidate = None;
        self.differing = None;
        self.next_at = self.received + SEARCH_EVERY;
    }

    /// Gives the filter the microphone that its reference has reached, and waits with the rest.
    fn pump(&mut self, out: &mut Vec<i16>) {
        while !self.pending.is_empty() {
            let n = match self.lock.filter(|_| self.open) {
                Some(lock) => {
                    let start = self.done as i64 - lock.applied;
                    let have = (self.ring_end() - start).max(0) as usize;
                    let n = self.pending.len().min(have);
                    if n == 0 {
                        let limit = (-lock.applied).max(0) as usize + HOLD_SLACK;
                        if self.pending.len() > limit {
                            self.lose();
                            continue;
                        }
                        break;
                    }
                    let (ring, ring_at) = (&self.ring, self.ring_at);
                    self.far.clear();
                    self.far.extend((start..start + n as i64).map(|p| {
                        // Before the reference's first sample, or past its newest: silence.
                        usize::try_from(p - ring_at)
                            .ok()
                            .and_then(|i| ring.get(i))
                            .copied()
                            .unwrap_or(0)
                    }));
                    n
                }
                None => {
                    self.far.clear();
                    self.far.resize(self.pending.len(), 0);
                    self.pending.len()
                }
            };
            self.filter(n, out);
        }
    }

    fn filter(&mut self, n: usize, out: &mut Vec<i16>) {
        let from = out.len();
        let mic = &self.pending[..n];
        self.aec.process(mic, &self.far, out);
        // Without a lock the reference is silence by construction, and what the microphone hears then is the TV, not the room's noise.
        if self.lock.is_some() && self.open {
            if Erle::square(&self.far) / n as f64 > f64::from(FAR_ACTIVE) {
                self.erle.heard(mic, &out[from..]);
                self.proving += n;
            } else {
                self.erle.quiet(mic);
            }
        }
        self.pending.drain(..n);
        self.done += n as u64;
        self.prove();
    }

    /// A lock that has had its seconds of loud reference and is still not taking enough off the microphone is given up: the lag is
    /// looked for again. Where the echo is too weak over the room's noise to be measured, the lock stands.
    fn prove(&mut self) {
        if self.proving < PROOF {
            return;
        }
        self.proving = 0;
        if self.erle.db().is_some_and(|db| db < MIN_ERLE_DB) {
            self.lose();
        }
    }

    /// Looks for the lag, or checks it, when the reference has grown by an interval.
    fn track(&mut self) {
        if self.received < self.next_at {
            return;
        }
        let Some(est) = self.est.as_mut() else { return };
        self.next_at = self.received
            + if self.lock.is_some() {
                VERIFY_EVERY
            } else {
                SEARCH_EVERY
            };
        let Some(found) = est.estimate() else { return };
        self.judge(found);
    }

    fn judge(&mut self, found: Estimate) {
        let strong = found.confidence >= LOCK_CONFIDENCE;
        match self.lock {
            None => {
                if !strong {
                    self.candidate = None;
                    self.weak += 1;
                    if self.weak >= GIVE_UP {
                        self.lost = true;
                    }
                    return;
                }
                match self.candidate {
                    Some(c) if (c - found.lag).abs() <= AGREE => self.lock_on(found),
                    _ if found.confidence >= SURE_CONFIDENCE => self.lock_on(found),
                    _ => self.candidate = Some(found.lag),
                }
            }
            Some(lock) => {
                if !strong || (found.lag - lock.lag).abs() <= KEEP {
                    self.differing = None;
                    return;
                }
                match self.differing {
                    Some(d) if (d - found.lag).abs() <= AGREE => self.lock_on(found),
                    _ => self.differing = Some(found.lag),
                }
            }
        }
    }

    fn lock_on(&mut self, found: Estimate) {
        self.lock = Some(Lock {
            lag: found.lag,
            applied: i64::from(found.lag) - self.margin,
            confidence: found.confidence,
        });
        self.lost = false;
        self.candidate = None;
        self.differing = None;
        self.weak = 0;
        self.proving = 0;
        self.locks += 1;
        self.erle.restart();
    }
}
