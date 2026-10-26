//! Sound-reactive ambient light, the analysis: the TV's own sound mix reduced to four numbers, and what they do to the lights'
//! colours.
//!
//! ```text
//!   arecord (hw:1,2, 48 kHz S16 stereo) ─► Analyser ─► Features ─► Meter ─► Ambient::step ─► with_picture / only ─► the lights
//!      ambient_audio_capture.rs            10 ms hops   4 floats    latest
//! ```
//!
//! - **Only four numbers leave the analyser**: `level` (how loud, against what has been loud lately), `bass` (the same for the
//!   low band), `pulse` (a beat: it jumps on an onset and dies away in about a quarter of a second) and `active` (whether there is
//!   any sound at all, smoothed so a pause does not flicker the lights). The samples are summed and dropped as they are read: no
//!   buffer of audio is kept, nothing is written, nothing here is published (the `ambient` topic carries the mode word, never a
//!   level).
//! - **The band split is two cheap filters**, not a transform: the mono mix goes through two cascaded one-pole low-passes
//!   (about 150 Hz, 12 dB per octave) for the bass, and loses a one-pole low-pass (about 2.5 kHz) for the top. A hop is 480 frames
//!   (10 ms) of three sums of squares and one `log10` or two; at 48 kHz that is about 10 multiply-adds a frame.
//! - **Loudness is relative.** The mix arrives at its source level (the TV's volume and the receiver's mute come after the tap), so
//!   the same film can be 20 dB apart from one stream to the next. `level` is the hop's loudness in dB against the loudest recent
//!   hop (a reference that falls 1 dB a second, never below -38 dBFS so a whisper is not blown up to full), over a 30 dB range.
//! - **A beat is a jump in a band's energy** over its own running average (about a third of a second): 2.4 times in the bass, 3.2
//!   times in the top, at least 120 ms apart, and only above an absolute floor so room noise never fires one. Its strength grows
//!   with the jump.
//!
//! The lights: [`with_picture`] scales the picture's colours (about 0.55 to 1 with the loudness, up to 1.4 on a beat, then
//! brought back under full scale without changing the hue) and eases to the plain picture when there is no sound; [`only`] draws the
//! still colour at the same gains with the chosen brightness as the top, and fades to dark when the sound stops.

use lumatv_ambient::{Light, Rgb};
use parking_lot::Mutex;
use std::time::{Duration, Instant};

/// What the capture is asked for, and what the analysis assumes.
pub const RATE: u32 = 48_000;
/// Frames in one hop: 10 ms.
pub const HOP: usize = 480;
/// Bytes in one frame: two channels of signed 16-bit little-endian.
const FRAME: usize = 4;

/// Below this a hop is silence (the mix is exactly zero when nothing plays; real content is far above it).
const GATE_DB: f32 = -64.0;
/// The loudness reference never falls below this, and the range below it that `level` maps to 0..1.
const LOUD_FLOOR_DB: f32 = -38.0;
const BASS_FLOOR_DB: f32 = -46.0;
const RANGE_DB: f32 = 30.0;
/// How far the reference falls each hop: 1 dB a second.
const REF_FALL_DB: f32 = 0.01;

/// Per hop: a rising level follows at once, a falling one over about 160 ms.
const ATTACK: f32 = 0.6;
const RELEASE: f32 = 0.06;

const BASS_HZ: f32 = 150.0;
const TOP_HZ: f32 = 2_500.0;

/// A beat: the band's energy over its running average, and the energy below which there is none (-60 dBFS).
const BASS_RATIO: f32 = 2.4;
const TOP_RATIO: f32 = 3.2;
const BEAT_FLOOR: f32 = 1.0e-6;
/// The running average moves this far to each hop's energy (about 330 ms).
const AVG_STEP: f32 = 0.03;
/// Hops after a beat in which there is no other (120 ms).
const REFRACTORY: u8 = 12;
/// What a beat's pulse keeps each hop (about 140 ms to half).
const PULSE_KEEP: f32 = 0.93;
/// A top-band beat counts for less than a bass one.
const TOP_WEIGHT: f32 = 0.7;

/// Hops of silence before `active` starts to fall (0.8 s), and what it keeps each hop after (about 0.5 s to a third).
const HOLD: u32 = 80;
const ACTIVE_KEEP: f32 = 0.98;
const ACTIVE_RISE: f32 = 0.25;

/// A reading older than this is no reading: the capture has stopped.
pub const STALE: Duration = Duration::from_millis(300);

/// What the sound does to the lights, the setting `sound` of `base.ambient`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundMode {
    /// The picture or the still colour, as the mode says: the sound is not read.
    Off,
    /// The picture (or the still colour) with the sound's brightness and beats on it.
    WithPicture,
    /// The sound alone: the still colour, its brightness the sound's.
    Only,
}

impl SoundMode {
    pub fn parse(word: &str) -> Option<SoundMode> {
        match word {
            "picture" => Some(SoundMode::Off),
            "picture-sound" => Some(SoundMode::WithPicture),
            "sound" => Some(SoundMode::Only),
            _ => None,
        }
    }

    /// The choice id Settings stores.
    pub fn word(self) -> &'static str {
        match self {
            SoundMode::Off => "picture",
            SoundMode::WithPicture => "picture-sound",
            SoundMode::Only => "sound",
        }
    }

    /// Whether the sound is read at all.
    pub fn listens(self) -> bool {
        self != SoundMode::Off
    }
}

/// The sound as the lights use it; every field 0..=1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Features {
    /// How loud, against what has been loud lately.
    pub level: f32,
    /// The same for the low band.
    pub bass: f32,
    /// A beat just now, dying away.
    pub pulse: f32,
    /// Whether there is sound: 1 while it plays, 0 some seconds after it stops.
    pub active: f32,
}

impl Features {
    pub const SILENT: Features = Features {
        level: 0.0,
        bass: 0.0,
        pulse: 0.0,
        active: 0.0,
    };
}

/// The loudest recent loudness, and the 0..1 a hop's loudness makes against it.
struct Norm {
    reference: f32,
    floor: f32,
}

impl Norm {
    fn new(floor: f32) -> Norm {
        Norm {
            reference: floor,
            floor,
        }
    }

    fn level(&mut self, db: f32) -> f32 {
        self.reference = (self.reference - REF_FALL_DB).max(self.floor);
        if db > self.reference {
            self.reference = db;
        }
        ((db - (self.reference - RANGE_DB)) / RANGE_DB).clamp(0.0, 1.0)
    }
}

/// `x` moves toward `target`: at `ATTACK` going up, at `RELEASE` going down.
fn follow(x: &mut f32, target: f32) {
    let k = if target > *x { ATTACK } else { RELEASE };
    *x += (target - *x) * k;
}

/// One-pole low-pass coefficient for a corner at `hz`.
fn coefficient(hz: f32) -> f32 {
    1.0 - (-std::f32::consts::TAU * hz / RATE as f32).exp()
}

/// Raw PCM in, [`Features`] out, one set per hop. Feed it bytes as they come; it keeps a partial frame between calls.
pub struct Analyser {
    bass_k: f32,
    top_k: f32,
    // The filters' states.
    low1: f32,
    low2: f32,
    top_low: f32,
    carry: [u8; FRAME],
    carry_len: usize,
    // The hop in progress: frames so far and the sums of squares of the whole mix, the bass and the top.
    frames: usize,
    sum_all: f32,
    sum_bass: f32,
    sum_top: f32,
    loud: Norm,
    bass_norm: Norm,
    level: f32,
    bass: f32,
    avg_bass: f32,
    avg_top: f32,
    refractory: u8,
    pulse: f32,
    active: f32,
    quiet_hops: u32,
}

impl Default for Analyser {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyser {
    pub fn new() -> Analyser {
        Analyser {
            bass_k: coefficient(BASS_HZ),
            top_k: coefficient(TOP_HZ),
            low1: 0.0,
            low2: 0.0,
            top_low: 0.0,
            carry: [0; FRAME],
            carry_len: 0,
            frames: 0,
            sum_all: 0.0,
            sum_bass: 0.0,
            sum_top: 0.0,
            loud: Norm::new(LOUD_FLOOR_DB),
            bass_norm: Norm::new(BASS_FLOOR_DB),
            level: 0.0,
            bass: 0.0,
            avg_bass: 0.0,
            avg_top: 0.0,
            refractory: 0,
            pulse: 0.0,
            active: 0.0,
            quiet_hops: 0,
        }
    }

    /// Takes the next bytes of the stream (any length, frames may be split between calls) and calls `out` with the features at
    /// the end of each hop they complete.
    pub fn push(&mut self, mut bytes: &[u8], mut out: impl FnMut(&Features)) {
        // A frame the last call left half done.
        while self.carry_len > 0 && !bytes.is_empty() {
            self.carry[self.carry_len] = bytes[0];
            self.carry_len += 1;
            bytes = &bytes[1..];
            if self.carry_len == FRAME {
                self.carry_len = 0;
                let frame = self.carry;
                self.run(&frame);
                if self.frames == HOP {
                    out(&self.hop());
                }
            }
        }
        while bytes.len() >= FRAME {
            let take = ((HOP - self.frames) * FRAME).min(bytes.len() / FRAME * FRAME);
            let (now, rest) = bytes.split_at(take);
            self.run(now);
            bytes = rest;
            if self.frames == HOP {
                out(&self.hop());
            }
        }
        if !bytes.is_empty() {
            self.carry[..bytes.len()].copy_from_slice(bytes);
            self.carry_len = bytes.len();
        }
    }

    /// Adds whole frames to the hop in progress (never more than the hop has room for).
    fn run(&mut self, frames: &[u8]) {
        let (mut low1, mut low2, mut top_low) = (self.low1, self.low2, self.top_low);
        let (mut all, mut bass, mut top) = (self.sum_all, self.sum_bass, self.sum_top);
        let (bass_k, top_k) = (self.bass_k, self.top_k);
        for f in frames.chunks_exact(FRAME) {
            let left = f32::from(i16::from_le_bytes([f[0], f[1]]));
            let right = f32::from(i16::from_le_bytes([f[2], f[3]]));
            let mono = (left + right) * (0.5 / 32768.0);
            low1 += bass_k * (mono - low1);
            low2 += bass_k * (low1 - low2);
            top_low += top_k * (mono - top_low);
            let high = mono - top_low;
            all += mono * mono;
            bass += low2 * low2;
            top += high * high;
        }
        self.low1 = low1;
        self.low2 = low2;
        self.top_low = top_low;
        self.sum_all = all;
        self.sum_bass = bass;
        self.sum_top = top;
        self.frames += frames.len() / FRAME;
    }

    /// Closes the hop: the loudness, the beats, the presence.
    fn hop(&mut self) -> Features {
        let n = HOP as f32;
        let (all, bass, top) = (self.sum_all / n, self.sum_bass / n, self.sum_top / n);
        self.frames = 0;
        self.sum_all = 0.0;
        self.sum_bass = 0.0;
        self.sum_top = 0.0;
        if all == 0.0 {
            // Exact silence: the filters' tails would only decay into denormal numbers.
            self.low1 = 0.0;
            self.low2 = 0.0;
            self.top_low = 0.0;
        }

        let db = 10.0 * (all + 1.0e-12).log10();
        let present = db > GATE_DB;
        let loud = self.loud.level(db);
        let bass_level = self.bass_norm.level(10.0 * (bass + 1.0e-12).log10());
        follow(&mut self.level, if present { loud } else { 0.0 });
        follow(&mut self.bass, if present { bass_level } else { 0.0 });

        self.refractory = self.refractory.saturating_sub(1);
        let jump = |ms: f32, avg: f32| ms / (avg + 1.0e-7);
        let (bass_jump, top_jump) = (jump(bass, self.avg_bass), jump(top, self.avg_top));
        if self.refractory == 0 && present {
            let beat = if bass > BEAT_FLOOR && bass_jump >= BASS_RATIO {
                Some((bass_jump, 1.0))
            } else if top > BEAT_FLOOR && top_jump >= TOP_RATIO {
                Some((top_jump, TOP_WEIGHT))
            } else {
                None
            };
            if let Some((jump, weight)) = beat {
                let strength = (jump.ln() / 10.0_f32.ln()).clamp(0.3, 1.0) * weight;
                self.pulse = self.pulse.max(strength);
                self.refractory = REFRACTORY;
            }
        }
        self.avg_bass += (bass - self.avg_bass) * AVG_STEP;
        self.avg_top += (top - self.avg_top) * AVG_STEP;
        self.pulse *= PULSE_KEEP;
        if self.pulse < 0.01 {
            self.pulse = 0.0;
        }

        if present {
            self.quiet_hops = 0;
            self.active += (1.0 - self.active) * ACTIVE_RISE;
        } else {
            self.quiet_hops = self.quiet_hops.saturating_add(1);
            if self.quiet_hops > HOLD {
                self.active *= ACTIVE_KEEP;
                if self.active < 0.005 {
                    self.active = 0.0;
                }
            }
        }
        Features {
            level: self.level,
            bass: self.bass,
            pulse: self.pulse,
            active: self.active,
        }
    }
}

/// The latest features, shared between the thread that reads the sound and the one that drives the lights.
pub struct Meter {
    slot: Mutex<Option<(Features, Instant)>>,
}

impl Default for Meter {
    fn default() -> Self {
        Self::new()
    }
}

impl Meter {
    pub fn new() -> Meter {
        Meter {
            slot: Mutex::new(None),
        }
    }

    pub fn put(&self, features: Features, now: Instant) {
        *self.slot.lock() = Some((features, now));
    }

    /// The latest features, or silence when there are none or the last is older than [`STALE`] (nothing is reading the sound).
    pub fn get(&self, now: Instant) -> Features {
        match *self.slot.lock() {
            Some((f, at)) if now.saturating_duration_since(at) <= STALE => f,
            _ => Features::SILENT,
        }
    }

    pub fn clear(&self) {
        *self.slot.lock() = None;
    }
}

/// How much of a beat a light takes: the bottom ones the most, as a thump is felt low.
fn weight(light: &Light) -> f32 {
    0.75 + 0.25 * light.y.clamp(0.0, 1.0)
}

/// The body of the sound: mostly how loud it is, some of it how heavy.
fn body(f: &Features) -> f32 {
    0.65 * f.level + 0.35 * f.bass
}

/// The picture's colours with the sound on them. With no sound they are left as they are; as it fades in, they go to a gain of
/// 0.55 (quiet) to 0.95 (loud) with up to 0.45 more on a beat, and a channel pushed past full scale takes the others down with it
/// so the hue holds.
pub fn with_picture(colours: &mut [Rgb], lights: &[Light], f: &Features) {
    if f.active <= 0.0 {
        return;
    }
    let body = body(f);
    for (c, light) in colours.iter_mut().zip(lights) {
        let sound = 0.55 + 0.4 * body + 0.45 * f.pulse * weight(light);
        let gain = 1.0 + (sound - 1.0) * f.active;
        let mut v = *c * gain;
        let top = v.max_channel();
        if top > 1.0 {
            v = v * (1.0 / top);
        }
        *c = v;
    }
}

/// The sound alone: `still` at a gain of 0.15 (quiet) to 0.6 (loud) with up to 0.6 more on a beat, never above `still` itself (the
/// chosen brightness is the top), and dark when there is no sound.
pub fn only(colours: &mut Vec<Rgb>, lights: &[Light], still: Rgb, f: &Features) {
    colours.clear();
    colours.extend(lights.iter().map(|light| {
        let gain = (0.15 + 0.45 * body(f) + 0.6 * f.pulse * weight(light)).min(1.0) * f.active;
        still * gain
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumatv_ambient::Target;

    /// `seconds` of stereo PCM: `f(t)` gives the left and right sample in -1..1 at time `t`.
    fn pcm(seconds: f32, f: impl Fn(f32) -> (f32, f32)) -> Vec<u8> {
        let frames = (seconds * RATE as f32) as usize;
        let mut out = Vec::with_capacity(frames * FRAME);
        for i in 0..frames {
            let (l, r) = f(i as f32 / RATE as f32);
            for v in [l, r] {
                out.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
            }
        }
        out
    }

    fn sine(hz: f32, t: f32) -> f32 {
        (std::f32::consts::TAU * hz * t).sin()
    }

    /// Every hop's features for `bytes`, fed in `chunk`-byte pieces.
    fn run(bytes: &[u8], chunk: usize) -> Vec<Features> {
        let mut a = Analyser::new();
        let mut all = Vec::new();
        for piece in bytes.chunks(chunk) {
            a.push(piece, |f| all.push(*f));
        }
        all
    }

    /// A kick drum: 60 Hz for 90 ms with a falling envelope, every `period` seconds, over a quiet bed.
    fn kicks(seconds: f32, period: f32, amp: f32) -> Vec<u8> {
        pcm(seconds, |t| {
            let into = t % period;
            let kick = if into < 0.09 {
                amp * (-into * 30.0).exp() * sine(60.0, t)
            } else {
                0.0
            };
            (kick, kick)
        })
    }

    fn beats(features: &[Features]) -> Vec<usize> {
        // A beat is where the pulse rises through 0.4.
        (1..features.len())
            .filter(|&i| features[i].pulse >= 0.4 && features[i - 1].pulse < 0.4)
            .collect()
    }

    #[test]
    fn a_hop_is_ten_milliseconds_whatever_size_the_pieces_are() {
        let bytes = pcm(1.0, |t| (0.3 * sine(440.0, t), 0.3 * sine(440.0, t)));
        for chunk in [1, 3, 4, 7, 1920, 4096, 100_000] {
            assert_eq!(run(&bytes, chunk).len(), 100, "pieces of {chunk} bytes");
        }
    }

    #[test]
    fn the_pieces_a_stream_is_cut_into_change_nothing() {
        let bytes = kicks(2.0, 0.5, 0.6);
        let whole = run(&bytes, bytes.len());
        for chunk in [1, 5, 1921] {
            assert_eq!(run(&bytes, chunk), whole, "pieces of {chunk} bytes");
        }
    }

    #[test]
    fn silence_is_inactive_and_reads_as_no_level_no_bass_no_beat() {
        let f = run(&vec![0u8; 48_000 * 4 * 3], 1920);
        assert!(f.iter().all(|f| *f == Features::SILENT), "{:?}", f.last());
    }

    #[test]
    fn a_steady_tone_is_active_and_loud_and_has_no_beats_after_it_starts() {
        let bytes = pcm(4.0, |t| (0.25 * sine(440.0, t), 0.25 * sine(440.0, t)));
        let f = run(&bytes, 1920);
        let last = f.last().unwrap();
        assert!(last.active > 0.99 && last.level > 0.95, "{last:?}");
        // The tone starting is one beat at most, in its first moments.
        assert!(beats(&f).iter().all(|&i| i < 10), "{:?}", beats(&f));
        assert!(f[100..].iter().all(|f| f.pulse == 0.0));
    }

    #[test]
    fn a_low_tone_is_bass_and_a_high_one_is_not() {
        let low = pcm(3.0, |t| (0.3 * sine(60.0, t), 0.3 * sine(60.0, t)));
        let high = pcm(3.0, |t| (0.3 * sine(6_000.0, t), 0.3 * sine(6_000.0, t)));
        let (low, high) = (run(&low, 1920), run(&high, 1920));
        // Each is as loud as anything it has heard, so the mix's level is full for both; the band split tells them apart.
        assert!(low.last().unwrap().bass > 0.9, "{:?}", low.last());
        assert!(high.last().unwrap().bass < 0.1, "{:?}", high.last());
    }

    #[test]
    fn a_kick_four_times_a_second_is_four_beats_a_second_each_with_its_own_pulse() {
        let f = run(&kicks(4.0, 0.25, 0.7), 1920);
        let b = beats(&f);
        // 16 kicks; the first of them is the start of the sound.
        assert!((15..=17).contains(&b.len()), "{b:?}");
        for pair in b.windows(2) {
            // 25 hops apart, give or take one.
            assert!((24..=26).contains(&(pair[1] - pair[0])), "{b:?}");
        }
        // Each beat dies away (to about a sixth) before the next.
        let between = &f[b[8] + 24];
        assert!(between.pulse < 0.25, "{between:?}");
        assert!(f[b[8]].pulse >= 0.4);
    }

    #[test]
    fn a_hard_kick_pulses_harder_than_a_soft_one() {
        let hard = run(&kicks(2.0, 0.5, 0.9), 1920);
        let soft = run(&kicks(2.0, 0.5, 0.08), 1920);
        let peak = |f: &[Features]| f[100..].iter().map(|f| f.pulse).fold(0.0, f32::max);
        assert!(peak(&hard) > 0.7, "{}", peak(&hard));
        // Against its own bed it is as sudden, so it is a beat too; but the floor keeps what is barely there from firing.
        assert!(peak(&soft) <= peak(&hard));
        let barely = run(&kicks(2.0, 0.5, 0.0008), 1920);
        assert!(beats(&barely).is_empty(), "{:?}", beats(&barely));
    }

    #[test]
    fn hi_hats_beat_too() {
        // Short bursts of 6 kHz, twice a second.
        let bytes = pcm(3.0, |t| {
            let into = t % 0.5;
            let v = if into < 0.03 {
                0.4 * sine(6_000.0, t)
            } else {
                0.0
            };
            (v, v)
        });
        let f = run(&bytes, 1920);
        let b = beats(&f);
        assert!(b.len() >= 4, "{b:?}");
        let peak = f[100..].iter().map(|f| f.pulse).fold(0.0, f32::max);
        assert!(peak > 0.3, "{peak}");
    }

    #[test]
    fn a_quiet_film_after_a_loud_one_reads_lower_and_comes_back_up_as_the_reference_falls() {
        let loud = pcm(2.0, |t| (0.4 * sine(300.0, t), 0.4 * sine(300.0, t)));
        let quiet = pcm(30.0, |t| (0.04 * sine(300.0, t), 0.04 * sine(300.0, t)));
        let mut bytes = loud;
        bytes.extend(quiet);
        let f = run(&bytes, 1920);
        let just_after = f[200 + 50].level;
        let much_later = f.last().unwrap().level;
        // 20 dB down on a 30 dB range: a third of the way up, then back to full once the reference has fallen to it.
        assert!((0.25..0.45).contains(&just_after), "{just_after}");
        assert!(much_later > 0.95, "{much_later}");
    }

    #[test]
    fn sound_that_stops_is_held_a_moment_and_then_fades_out() {
        let mut bytes = pcm(2.0, |t| (0.3 * sine(300.0, t), 0.3 * sine(300.0, t)));
        bytes.extend(vec![0u8; 48_000 * 4 * 6]);
        let f = run(&bytes, 1920);
        assert!(f[200].active > 0.99);
        // Half a second into the silence: still active, the level has dropped.
        assert!(f[250].active > 0.99 && f[250].level < 0.05, "{:?}", f[250]);
        // Two seconds in: well on its way; five seconds in: gone.
        assert!(f[400].active < 0.2, "{:?}", f[400]);
        assert_eq!(f[700].active, 0.0);
        // And back, at once.
        let mut again = bytes;
        again.extend(pcm(0.3, |t| (0.3 * sine(300.0, t), 0.3 * sine(300.0, t))));
        assert!(run(&again, 1920).last().unwrap().active > 0.9);
    }

    #[test]
    fn the_noise_floor_below_the_gate_is_silence() {
        // +-3 counts of 32768: about -80 dBFS.
        let mut seed = 12345u32;
        let bytes: Vec<u8> = (0..48_000 * 2)
            .flat_map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let v = ((seed >> 16) % 7) as i16 - 3;
                v.to_le_bytes()
            })
            .collect();
        let f = run(&bytes, 1920);
        assert!(f.iter().all(|f| f.active == 0.0 && f.pulse == 0.0));
    }

    fn light(y: f32) -> Light {
        Light {
            target: Target::Wled { led: 0 },
            x: 0.5,
            y,
        }
    }

    const PICTURE: Rgb = Rgb::new(0.6, 0.3, 0.1);

    fn feat(level: f32, pulse: f32, active: f32) -> Features {
        Features {
            level,
            bass: level,
            pulse,
            active,
        }
    }

    #[test]
    fn with_no_sound_the_picture_is_untouched() {
        let mut c = [PICTURE];
        with_picture(&mut c, &[light(0.5)], &Features::SILENT);
        assert_eq!(c[0], PICTURE);
    }

    #[test]
    fn the_picture_dims_with_a_quiet_mix_and_a_beat_brings_it_up_past_where_loud_leaves_it() {
        let lights = [light(0.5)];
        let at = |f: Features| {
            let mut c = [PICTURE];
            with_picture(&mut c, &lights, &f);
            c[0].r
        };
        let (quiet, loud) = (at(feat(0.0, 0.0, 1.0)), at(feat(1.0, 0.0, 1.0)));
        let beat = at(feat(1.0, 1.0, 1.0));
        assert!((quiet - 0.6 * 0.55).abs() < 1e-5, "{quiet}");
        assert!((loud - 0.6 * 0.95).abs() < 1e-5, "{loud}");
        assert!(beat > 0.6 * 1.3, "{beat}");
        // Half-faded sound is half the way between the plain picture and the sound's gain.
        let half = at(feat(0.0, 0.0, 0.5));
        assert!((half - 0.6 * 0.775).abs() < 1e-5, "{half}");
    }

    #[test]
    fn a_beat_that_overshoots_full_scale_keeps_the_hue() {
        let mut c = [Rgb::new(0.95, 0.5, 0.25)];
        with_picture(&mut c, &[light(1.0)], &feat(1.0, 1.0, 1.0));
        assert!((c[0].max_channel() - 1.0).abs() < 1e-5);
        assert!((c[0].g / c[0].r - 0.5 / 0.95).abs() < 1e-4);
        assert!((c[0].b / c[0].r - 0.25 / 0.95).abs() < 1e-4);
    }

    #[test]
    fn the_bottom_lights_take_more_of_a_beat_than_the_top_ones() {
        let mut c = [PICTURE, PICTURE];
        with_picture(&mut c, &[light(0.0), light(1.0)], &feat(0.5, 1.0, 1.0));
        assert!(c[1].r > c[0].r);
    }

    #[test]
    fn sound_alone_is_the_still_colour_at_the_sounds_gain_never_above_it_and_dark_with_no_sound() {
        let still = Rgb::new(1.0, 0.7, 0.4);
        let mut c = Vec::new();
        only(&mut c, &[light(0.5), light(1.0)], still, &Features::SILENT);
        assert_eq!(c, vec![Rgb::BLACK; 2]);
        only(&mut c, &[light(0.5)], still, &feat(1.0, 0.0, 1.0));
        assert!((c[0].r - 0.6).abs() < 1e-5, "{:?}", c[0]);
        only(&mut c, &[light(1.0)], still, &feat(1.0, 1.0, 1.0));
        assert_eq!(c[0], still);
        only(&mut c, &[light(1.0)], still, &feat(0.0, 0.0, 1.0));
        assert!((c[0].r - 0.15).abs() < 1e-5);
    }

    #[test]
    fn the_meter_gives_the_latest_features_and_silence_once_they_are_old() {
        let m = Meter::new();
        let t = Instant::now();
        assert_eq!(m.get(t), Features::SILENT);
        m.put(feat(0.5, 0.2, 1.0), t);
        assert_eq!(m.get(t + Duration::from_millis(100)).level, 0.5);
        assert_eq!(m.get(t + Duration::from_millis(400)), Features::SILENT);
        m.put(feat(0.5, 0.2, 1.0), t);
        m.clear();
        assert_eq!(m.get(t), Features::SILENT);
    }

    #[test]
    fn the_words_round_trip() {
        for mode in [SoundMode::Off, SoundMode::WithPicture, SoundMode::Only] {
            assert_eq!(SoundMode::parse(mode.word()), Some(mode));
        }
        assert_eq!(SoundMode::parse("loud"), None);
        assert!(!SoundMode::Off.listens() && SoundMode::Only.listens());
    }

    /// `cargo test --release -p lumatv-helper -- --ignored --nocapture analysis_cost`: the analyser's cost on this host.
    #[test]
    #[ignore = "a measurement, not a check"]
    fn analysis_cost() {
        let seconds = 120.0;
        let bytes = pcm(seconds, |t| {
            let kick = if t % 0.5 < 0.09 {
                0.6 * (-(t % 0.5) * 30.0).exp() * sine(60.0, t)
            } else {
                0.0
            };
            let bed = 0.1 * sine(220.0, t) + 0.05 * sine(3_300.0, t);
            (kick + bed, kick - bed)
        });
        let mut a = Analyser::new();
        let mut hops = 0u32;
        let started = Instant::now();
        for piece in bytes.chunks(1920) {
            a.push(piece, |_| hops += 1);
        }
        let took = started.elapsed();
        eprintln!(
            "{seconds} s of audio, {hops} hops: {took:?}, {:.4} % of a core in real time",
            took.as_secs_f64() / f64::from(seconds) * 100.0
        );
    }
}
