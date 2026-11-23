//! The capture device's frames to the audio the voice path works on: 16 kHz, mono, 16-bit.
//!
//! The TV's far-field microphone is two PDM microphones read as one stereo stream of 24-bit samples. [`Frames`] decodes a
//! period's bytes, mixes the channels down (both added, one of them, or whichever the stream shows to be the better one), applies
//! the software gain **to the full-resolution sample before it is cut to 16 bits** (cutting first throws the quiet end away), and
//! takes the rate down to 16 kHz when the device was opened at a multiple of it. No automatic gain and no gate: both degrade the
//! wake word over days (EchoMuse's finding, which LumaShow's voice path follows).

use std::f32::consts::PI;

/// The rate the wake word and the turn work at.
pub const RATE: u32 = 16_000;

/// How a sample sits in the bytes the device hands over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
    /// `S16_LE`.
    S16,
    /// `S24_LE`: 24 bits in the low three bytes of four.
    S24,
    /// `S32_LE`: the sample fills four bytes.
    S32,
}

impl Sample {
    pub const fn bytes(self) -> usize {
        match self {
            Self::S16 => 2,
            Self::S24 | Self::S32 => 4,
        }
    }

    /// The format's name for `arecord -f`.
    pub const fn alsa(self) -> &'static str {
        match self {
            Self::S16 => "S16_LE",
            Self::S24 => "S24_LE",
            Self::S32 => "S32_LE",
        }
    }

    /// One sample as a fraction of full scale (-1 to just under 1). `S24` carries its sign in bit 23, whatever the top byte holds.
    pub fn value(self, b: &[u8]) -> f32 {
        match self {
            Self::S16 => f32::from(i16::from_le_bytes([b[0], b[1]])) / 32_768.0,
            Self::S24 => {
                let raw = i32::from_le_bytes([b[0], b[1], b[2], 0]);
                ((raw << 8) >> 8) as f32 / 8_388_608.0
            }
            Self::S32 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
        }
    }
}

/// What the device streams.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub rate: u32,
    pub channels: usize,
    pub sample: Sample,
}

impl Format {
    pub const fn frame_bytes(&self) -> usize {
        self.channels * self.sample.bytes()
    }
}

/// Which of the microphones the voice path listens to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// The stream shows which is better (see [`Frames::pick`]): both added until it is sure, one when the other is dead or
    /// clearly the worse.
    Auto,
    /// The channels added and halved: two microphones a few centimetres apart add in step on speech from the front and out of
    /// step on most noise.
    Sum,
    Channel(usize),
}

/// A 20 ms block of one channel's energy is the unit the choice between the microphones is judged on.
const BLOCKS_PER_SECOND: usize = 50;
/// The floor of a channel rises at most this much per block (about 1.3 times a second), so it follows the quietest moments of the
/// last few seconds; the peak falls 0.2 % per block (a tenth in a second).
const FLOOR_RISE: f32 = 1.005;
const PEAK_FALL: f32 = 0.998;
/// A channel whose loudest block over the memory of the peak is under this (energy, -90 dBFS) hears nothing.
const DEAD_ENERGY: f32 = 1.0e-9;
/// One microphone is preferred over the sum when its range (peak over floor) is this many decibels better than every other's.
const PREFER_DB: f32 = 3.0;
/// The seconds of stream before a choice is made, and the evaluations in a row that must agree before the choice changes.
const SETTLE_SECONDS: u32 = 5;
const AGREE: u8 = 3;

struct Auto {
    block: usize,
    seen: usize,
    sum_sq: Vec<f64>,
    floor: Vec<f32>,
    peak: Vec<f32>,
    blocks: u32,
    chosen: Pick,
    pending: Option<(Pick, u8)>,
}

impl Auto {
    fn new(channels: usize, rate: u32) -> Self {
        Self {
            block: (rate as usize / BLOCKS_PER_SECOND).max(1),
            seen: 0,
            sum_sq: vec![0.0; channels],
            floor: vec![f32::MAX; channels],
            peak: vec![0.0; channels],
            blocks: 0,
            chosen: Pick::Sum,
            pending: None,
        }
    }

    fn frame(&mut self, channels: &[f32]) {
        for (sum, &v) in self.sum_sq.iter_mut().zip(channels) {
            *sum += f64::from(v) * f64::from(v);
        }
        self.seen += 1;
        if self.seen == self.block {
            self.end_block();
        }
    }

    fn end_block(&mut self) {
        for c in 0..self.sum_sq.len() {
            let energy = (self.sum_sq[c] / self.seen as f64) as f32;
            self.floor[c] = energy.min(self.floor[c] * FLOOR_RISE);
            self.peak[c] = energy.max(self.peak[c] * PEAK_FALL);
            self.sum_sq[c] = 0.0;
        }
        self.seen = 0;
        self.blocks += 1;
        if self.blocks as usize % BLOCKS_PER_SECOND == 0
            && self.blocks >= SETTLE_SECONDS * BLOCKS_PER_SECOND as u32
        {
            self.evaluate();
        }
    }

    fn range_db(&self, c: usize) -> f32 {
        10.0 * (self.peak[c].max(1e-12) / self.floor[c].max(1e-12)).log10()
    }

    fn candidate(&self) -> Pick {
        let channels = self.peak.len();
        let alive: Vec<usize> = (0..channels).filter(|&c| self.peak[c] > DEAD_ENERGY).collect();
        match alive.len() {
            0 => return Pick::Sum,
            1 if channels > 1 => return Pick::Channel(alive[0]),
            _ => {}
        }
        let best = alive
            .iter()
            .copied()
            .max_by(|&a, &b| self.range_db(a).total_cmp(&self.range_db(b)))
            .unwrap_or(0);
        let others_best = alive
            .iter()
            .copied()
            .filter(|&c| c != best)
            .map(|c| self.range_db(c))
            .fold(f32::MIN, f32::max);
        if self.range_db(best) - others_best >= PREFER_DB {
            Pick::Channel(best)
        } else {
            Pick::Sum
        }
    }

    fn evaluate(&mut self) {
        let candidate = self.candidate();
        if candidate == self.chosen {
            self.pending = None;
            return;
        }
        let agreed = match self.pending {
            Some((p, n)) if p == candidate => n + 1,
            _ => 1,
        };
        if agreed >= AGREE {
            self.chosen = candidate;
            self.pending = None;
        } else {
            self.pending = Some((candidate, agreed));
        }
    }
}

/// A low-pass and a step down by an integer factor, for a device opened at a multiple of 16 kHz.
struct Decimator {
    factor: usize,
    taps: Vec<f32>,
    buf: Vec<f32>,
    next: usize,
}

impl Decimator {
    fn new(factor: usize) -> Self {
        // Windowed sinc at 90 % of the output's Nyquist, a Kaiser window of beta 7 (about 70 dB down past the edge); unity at DC.
        let n = 24 * factor + 1;
        let cutoff = 0.45 / factor as f32;
        let mid = (n - 1) as f32 / 2.0;
        let beta = 7.0f32;
        let i0_beta = bessel_i0(beta);
        let mut taps: Vec<f32> = (0..n)
            .map(|i| {
                let x = i as f32 - mid;
                let sinc = if x == 0.0 {
                    2.0 * cutoff
                } else {
                    (2.0 * PI * cutoff * x).sin() / (PI * x)
                };
                let r = x / mid;
                let window = bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / i0_beta;
                sinc * window
            })
            .collect();
        let sum: f32 = taps.iter().sum();
        for t in &mut taps {
            *t /= sum;
        }
        Self {
            factor,
            buf: vec![0.0; n - 1],
            next: n - 1,
            taps,
        }
    }

    fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        self.buf.extend_from_slice(input);
        let n = self.taps.len();
        while self.next < self.buf.len() {
            let window = &self.buf[self.next + 1 - n..=self.next];
            out.push(window.iter().rev().zip(&self.taps).map(|(x, t)| x * t).sum());
            self.next += self.factor;
        }
        let keep = n - 1;
        let drop = self.buf.len().saturating_sub(keep);
        self.buf.drain(..drop);
        self.next -= drop;
    }
}

fn bessel_i0(x: f32) -> f32 {
    let mut sum = 1.0f32;
    let mut term = 1.0f32;
    let q = x * x / 4.0;
    for k in 1..40 {
        term *= q / (k * k) as f32;
        sum += term;
        if term < sum * 1e-9 {
            break;
        }
    }
    sum
}

/// Why a stream cannot be read.
#[derive(Debug, PartialEq, Eq)]
pub struct Unsupported(pub String);

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unsupported {}

/// The decoder of one stream.
pub struct Frames {
    format: Format,
    gain: f32,
    pick: Pick,
    auto: Option<Auto>,
    decimator: Option<Decimator>,
    carry: Vec<u8>,
    mixed: Vec<f32>,
    stepped: Vec<f32>,
    channels: Vec<f32>,
    clipped: u64,
}

impl Frames {
    /// `gain_db` is the software gain of the microphone.
    pub fn new(format: Format, gain_db: f32, pick: Pick) -> Result<Self, Unsupported> {
        if format.channels == 0 || format.rate == 0 || format.rate % RATE != 0 {
            return Err(Unsupported(format!(
                "a stream of {} channels at {} Hz cannot be brought to 16 kHz",
                format.channels, format.rate
            )));
        }
        if let Pick::Channel(c) = pick
            && c >= format.channels
        {
            return Err(Unsupported(format!(
                "channel {c} of a stream of {}",
                format.channels
            )));
        }
        let factor = (format.rate / RATE) as usize;
        Ok(Self {
            auto: (pick == Pick::Auto).then(|| Auto::new(format.channels, format.rate)),
            decimator: (factor > 1).then(|| Decimator::new(factor)),
            format,
            gain: 10f32.powf(gain_db / 20.0),
            pick,
            carry: Vec::new(),
            mixed: Vec::new(),
            stepped: Vec::new(),
            channels: vec![0.0; format.channels],
            clipped: 0,
        })
    }

    pub fn format(&self) -> Format {
        self.format
    }

    /// The microphones the audio is made of now (for `Auto`: what the stream has shown so far).
    pub fn pick(&self) -> Pick {
        self.auto.as_ref().map_or(self.pick, |a| a.chosen)
    }

    /// Samples the gain pushed past full scale and were cut: a count for whoever tunes the gain.
    pub fn clipped(&self) -> u64 {
        self.clipped
    }

    /// Appends the 16 kHz mono samples of the whole frames in `bytes` to `out`. A frame cut by the end of `bytes` waits for the
    /// next call.
    pub fn push(&mut self, bytes: &[u8], out: &mut Vec<i16>) {
        let frame = self.format.frame_bytes();
        if self.carry.is_empty() {
            let whole = bytes.len() / frame * frame;
            self.frames(&bytes[..whole], out);
            self.carry.extend_from_slice(&bytes[whole..]);
        } else {
            self.carry.extend_from_slice(bytes);
            let whole = self.carry.len() / frame * frame;
            let ready: Vec<u8> = self.carry.drain(..whole).collect();
            self.frames(&ready, out);
        }
    }

    fn frames(&mut self, bytes: &[u8], out: &mut Vec<i16>) {
        let width = self.format.sample.bytes();
        let sample = self.format.sample;
        let pick = self.pick();
        self.mixed.clear();
        for frame in bytes.chunks_exact(self.format.frame_bytes()) {
            for (c, slot) in self.channels.iter_mut().enumerate() {
                *slot = sample.value(&frame[c * width..(c + 1) * width]);
            }
            if let Some(auto) = self.auto.as_mut() {
                auto.frame(&self.channels);
            }
            let value = match pick {
                Pick::Channel(c) => self.channels[c],
                Pick::Sum | Pick::Auto => {
                    self.channels.iter().sum::<f32>() / self.channels.len() as f32
                }
            } * self.gain;
            if !(-1.0..1.0).contains(&value) {
                self.clipped += 1;
            }
            self.mixed.push(value);
        }
        match self.decimator.as_mut() {
            Some(d) => {
                self.stepped.clear();
                d.process(&self.mixed, &mut self.stepped);
                out.extend(self.stepped.iter().map(|&v| to_i16(v)));
            }
            None => out.extend(self.mixed.iter().map(|&v| to_i16(v))),
        }
    }
}

fn to_i16(v: f32) -> i16 {
    (v * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16
}

/// The loudest sample in dBFS (full scale is 0; nothing at all is -120): how loud, never what.
pub fn peak_dbfs(samples: &[i16]) -> f32 {
    let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
    if peak == 0 {
        -120.0
    } else {
        (20.0 * (f32::from(peak) / 32_768.0).log10()).max(-120.0)
    }
}

/// The root mean square of samples in dBFS (a full-scale square wave is 0).
pub fn rms_dbfs(samples: impl Iterator<Item = f32>) -> f32 {
    let (sum, n) = samples.fold((0.0f64, 0u32), |(s, n), v| (s + f64::from(v) * f64::from(v), n + 1));
    if n == 0 || sum == 0.0 {
        return -120.0;
    }
    (10.0 * (sum / f64::from(n)).log10() as f32).max(-120.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s24(v: f32) -> [u8; 4] {
        let raw = (v * 8_388_608.0).round().clamp(-8_388_608.0, 8_388_607.0) as i32;
        // The top byte is not guaranteed to carry the sign: the decoder reads bit 23.
        let b = raw.to_le_bytes();
        [b[0], b[1], b[2], 0]
    }

    fn stereo(rate: u32, left: impl Fn(usize) -> f32, right: impl Fn(usize) -> f32, frames: usize) -> Vec<u8> {
        let _ = rate;
        let mut bytes = Vec::with_capacity(frames * 8);
        for i in 0..frames {
            bytes.extend_from_slice(&s24(left(i)));
            bytes.extend_from_slice(&s24(right(i)));
        }
        bytes
    }

    fn format(rate: u32) -> Format {
        Format {
            rate,
            channels: 2,
            sample: Sample::S24,
        }
    }

    #[test]
    fn a_24_bit_sample_is_read_with_its_sign_from_bit_23() {
        for v in [0.5f32, -0.5, 0.999, -1.0, 1.0e-4, -1.0e-4] {
            let got = Sample::S24.value(&s24(v));
            assert!((got - v).abs() < 2.0e-7, "{v} read as {got}");
        }
    }

    #[test]
    fn the_other_formats_read_as_a_fraction_of_full_scale() {
        assert_eq!(Sample::S16.value(&16_384i16.to_le_bytes()), 0.5);
        assert_eq!(Sample::S32.value(&(-1_073_741_824i32).to_le_bytes()), -0.5);
    }

    #[test]
    fn the_gain_goes_on_before_the_cut_so_a_quiet_floor_survives() {
        // A signal 2 LSB high at 16 bits would read 0 or 1 without the gain; with +12 dB on the 24-bit sample it keeps its shape.
        let mut frames = Frames::new(format(16_000), 12.0, Pick::Channel(0)).unwrap();
        let bytes = stereo(16_000, |i| if i % 2 == 0 { 2.0 / 32_768.0 } else { -2.0 / 32_768.0 }, |_| 0.0, 64);
        let mut out = Vec::new();
        frames.push(&bytes, &mut out);
        assert_eq!(out.len(), 64);
        assert_eq!(out[0], 8, "2 LSB at +12 dB (x3.98) is 8");
        assert_eq!(out[1], -8);
    }

    #[test]
    fn sum_halves_and_one_channel_passes_whole() {
        let bytes = stereo(16_000, |_| 0.2, |_| 0.4, 16);
        let mut out = Vec::new();
        Frames::new(format(16_000), 0.0, Pick::Sum).unwrap().push(&bytes, &mut out);
        assert!((f32::from(out[0]) / 32_768.0 - 0.3).abs() < 1e-3);
        out.clear();
        Frames::new(format(16_000), 0.0, Pick::Channel(1)).unwrap().push(&bytes, &mut out);
        assert!((f32::from(out[0]) / 32_768.0 - 0.4).abs() < 1e-3);
    }

    #[test]
    fn samples_cut_by_the_gain_are_counted() {
        let mut frames = Frames::new(format(16_000), 20.0, Pick::Channel(0)).unwrap();
        let bytes = stereo(16_000, |_| 0.5, |_| 0.0, 10);
        let mut out = Vec::new();
        frames.push(&bytes, &mut out);
        assert_eq!(frames.clipped(), 10);
        assert_eq!(out[0], 32_767);
    }

    #[test]
    fn a_frame_cut_in_two_by_the_read_is_finished_by_the_next() {
        let bytes = stereo(16_000, |i| i as f32 / 100.0, |_| 0.0, 8);
        let mut whole = Vec::new();
        Frames::new(format(16_000), 0.0, Pick::Channel(0)).unwrap().push(&bytes, &mut whole);
        let mut split = Vec::new();
        let mut frames = Frames::new(format(16_000), 0.0, Pick::Channel(0)).unwrap();
        for piece in bytes.chunks(5) {
            frames.push(piece, &mut split);
        }
        assert_eq!(split, whole);
    }

    #[test]
    fn a_48_khz_stream_comes_down_to_16_with_the_band_kept_and_the_alias_gone() {
        let rate = 48_000.0f32;
        let tone = |hz: f32| move |i: usize| 0.25 * (2.0 * PI * hz * i as f32 / rate).sin();
        let run = |hz: f32| {
            let mut frames = Frames::new(format(48_000), 0.0, Pick::Channel(0)).unwrap();
            let bytes = stereo(48_000, tone(hz), |_| 0.0, 4_800);
            let mut out = Vec::new();
            frames.push(&bytes, &mut out);
            // the filter's start-up is half a window; judge the rest
            let settled = &out[out.len() / 2..];
            rms_dbfs(settled.iter().map(|&s| f32::from(s) / 32_768.0))
        };
        let reference = 20.0 * (0.25f32 / 2f32.sqrt()).log10();
        assert!((run(1_000.0) - reference).abs() < 0.3, "1 kHz is kept: {}", run(1_000.0));
        assert!((run(6_000.0) - reference).abs() < 1.0, "6 kHz is kept: {}", run(6_000.0));
        assert!(run(11_000.0) < reference - 45.0, "11 kHz would alias to 5 kHz: {}", run(11_000.0));
    }

    #[test]
    fn a_48_khz_stream_gives_a_third_of_the_samples() {
        let mut frames = Frames::new(format(48_000), 0.0, Pick::Sum).unwrap();
        let bytes = stereo(48_000, |_| 0.1, |_| 0.1, 4_800);
        let mut out = Vec::new();
        frames.push(&bytes, &mut out);
        assert_eq!(out.len(), 1_600);
    }

    #[test]
    fn a_rate_that_is_not_a_multiple_of_16_khz_is_refused() {
        assert!(Frames::new(format(44_100), 0.0, Pick::Sum).is_err());
        assert!(Frames::new(format(16_000), 0.0, Pick::Channel(2)).is_err());
    }

    /// Two seconds of stream every call: a hum with bursts on the channel that is meant to be the good one.
    fn auto_run(frames: &mut Frames, seconds: usize, good: usize, dead: Option<usize>) {
        let rate = 16_000usize;
        let mut rng = 12_345u32;
        let mut noise = move || {
            rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (rng >> 8) as f32 / 8_388_608.0 - 1.0
        };
        for s in 0..seconds {
            let mut bytes = Vec::new();
            for i in 0..rate {
                // speech-like bursts: loud for 300 ms of every second
                let burst = if i < 4_800 { 0.05 } else { 0.0 };
                let mut ch = [0.0f32; 2];
                for (c, v) in ch.iter_mut().enumerate() {
                    let floor = if c == good { 0.0005 } else { 0.004 };
                    *v = noise() * (floor + if c == good { burst } else { burst * 0.3 });
                    if Some(c) == dead {
                        *v = 0.0;
                    }
                }
                bytes.extend_from_slice(&s24(ch[0]));
                bytes.extend_from_slice(&s24(ch[1]));
            }
            let mut out = Vec::new();
            frames.push(&bytes, &mut out);
            let _ = s;
        }
    }

    #[test]
    fn auto_adds_both_until_it_has_seen_enough_and_when_they_are_alike() {
        let mut frames = Frames::new(format(16_000), 0.0, Pick::Auto).unwrap();
        auto_run(&mut frames, 3, 0, None);
        assert_eq!(frames.pick(), Pick::Sum, "too early to choose");
        let mut alike = Frames::new(format(16_000), 0.0, Pick::Auto).unwrap();
        for _ in 0..12 {
            let rate = 16_000usize;
            let mut rng = 7u32;
            let mut bytes = Vec::new();
            for i in 0..rate {
                let burst = if i < 4_800 { 0.05 } else { 0.0 };
                rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let v = ((rng >> 8) as f32 / 8_388_608.0 - 1.0) * (0.002 + burst);
                bytes.extend_from_slice(&s24(v));
                bytes.extend_from_slice(&s24(v));
            }
            alike.push(&bytes, &mut Vec::new());
        }
        assert_eq!(alike.pick(), Pick::Sum);
    }

    #[test]
    fn auto_takes_the_microphone_with_the_better_range_and_the_live_one_when_the_other_is_dead() {
        let mut frames = Frames::new(format(16_000), 0.0, Pick::Auto).unwrap();
        auto_run(&mut frames, 14, 1, None);
        assert_eq!(frames.pick(), Pick::Channel(1));
        let mut frames = Frames::new(format(16_000), 0.0, Pick::Auto).unwrap();
        auto_run(&mut frames, 14, 0, Some(0));
        assert_eq!(frames.pick(), Pick::Channel(1), "channel 0 hears nothing");
    }
}
