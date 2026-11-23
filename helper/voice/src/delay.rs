//! Where the TV's sound sits in the microphone's time.
//!
//! The sound the microphone hears left the TV some time before, by a path nothing here can see: the TV's mixer, a receiver behind an
//! HDMI link that adds its own processing, the air of the room. The reference reaches the engine by a path of its own, with its own
//! buffers. The two streams therefore do not line up, and the canceller's filter reaches back only 128 ms while the gap can be several
//! times that. The gap is read from the sound itself: the cross-correlation of the two signals, whitened so that its peak is sharp
//! (GCC-PHAT), peaks at the lag where the microphone holds a copy of the reference.
//!
//! The lag is `echo - reference`, in samples at 16 kHz: positive when the echo trails the reference (the usual case), negative when the
//! reference arrives after the echo it belongs to. Both streams are placed on one timeline by their first samples (a reference stream
//! begins at the microphone's position when it was opened), so the lag includes whatever the two capture paths hold back.

use crate::pcm::Decimator;
use std::f32::consts::PI;
use voice_dsp::fft::Fft;

/// The correlation runs at a quarter of 16 kHz: what identifies a copy of speech, music or noise is under 2 kHz, and the transform
/// is sixteen times cheaper than at 16 kHz.
pub(crate) const STEP: usize = 4;
const RATE: f32 = 16_000.0 / STEP as f32;
const FFT_LEN: usize = 16_384;
/// The band the peak is read from, in Hz, as `(rises from, full from, full to, falls to)`. Below it the TV's speakers and the
/// microphone's housing add hum; above it the 4 kHz rate has no content. The edges are smooth: a hard edge rings, and the ringing
/// of a band 1.6 kHz wide is the size of the noise the peak is measured against.
const BAND: (f32, f32, f32, f32) = (100.0, 200.0, 1_700.0, 1_900.0);
/// A bin whose power is under this fraction of the band's mean is not whitened up to full weight: it holds no signal, and lifting it
/// would lift its noise.
const FLOOR: f32 = 0.02;
/// The reference's mean square (full scale is 1) under which a window holds nothing worth correlating: about -75 dBFS.
const SILENT: f32 = 3e-8;
/// Samples around the peak left out of the measure of the sidelobes (2.5 ms): the peak's own skirt.
const GUARD: usize = 10;
/// Samples of history kept beyond what a window reaches, so that a stream delivered in lumps always has what the window asks for.
const SLACK: usize = 2_000;

/// How far the search reaches. The three spans together must fit the transform: 4 s at most at the quarter rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Search {
    /// The sound compared in one estimate.
    pub window_ms: u32,
    /// How much sooner than the reference's arrival the echo may be: the reference path's buffers can hold the sound back longer
    /// than the TV's sound takes to reach the microphone.
    pub before_ms: u32,
    /// How much later than the reference the echo may be: the receiver's processing and the air.
    pub after_ms: u32,
}

impl Default for Search {
    fn default() -> Self {
        Self {
            window_ms: 1_500,
            before_ms: 1_000,
            after_ms: 1_500,
        }
    }
}

impl Search {
    fn samples(ms: u32) -> usize {
        (ms as usize * RATE as usize).div_ceil(1_000)
    }

    /// Whether the transform is long enough for the spans.
    pub fn fits(&self) -> bool {
        Self::samples(self.window_ms) + Self::samples(self.before_ms) + Self::samples(self.after_ms)
            <= FFT_LEN
    }
}

/// What one estimate found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Estimate {
    /// `echo - reference`, in samples at 16 kHz.
    pub lag: i32,
    /// The peak over the deviation of everything else in the search: about 4 for noise, 8 or more for a copy.
    pub confidence: f32,
}

/// The newest samples of a stream with the index of the first.
#[derive(Default)]
struct Tail {
    data: Vec<f32>,
    start: i64,
}

impl Tail {
    fn end(&self) -> i64 {
        self.start + self.data.len() as i64
    }

    fn push(&mut self, samples: &[f32], keep: usize) {
        self.data.extend_from_slice(samples);
        // Trimmed in steps: a cut at every push would move the whole tail every 16 ms.
        if self.data.len() > keep + keep / 2 {
            let cut = self.data.len() - keep;
            self.data.drain(..cut);
            self.start += cut as i64;
        }
    }

    fn at(&self, i: i64) -> f32 {
        if i >= self.start && i < self.end() {
            self.data[(i - self.start) as usize]
        } else {
            0.0
        }
    }
}

pub struct Estimator {
    window: usize,
    before: usize,
    after: usize,
    mic_step: Decimator,
    ref_step: Decimator,
    mic: Tail,
    reference: Tail,
    open: bool,
    fft: Fft,
    hann: Vec<f32>,
    band: Vec<f32>,
    xr: Vec<f32>,
    xi: Vec<f32>,
    yr: Vec<f32>,
    yi: Vec<f32>,
    scratch: Vec<f32>,
    stepped: Vec<f32>,
}

impl Estimator {
    /// `None` when the spans do not fit the transform.
    pub fn new(search: Search) -> Option<Self> {
        if !search.fits() {
            return None;
        }
        let window = Search::samples(search.window_ms);
        let band = (0..=FFT_LEN / 2)
            .map(|k| {
                let f = k as f32 * RATE / FFT_LEN as f32;
                let ramp = |x: f32| 0.5 - 0.5 * (PI * x.clamp(0.0, 1.0)).cos();
                ramp((f - BAND.0) / (BAND.1 - BAND.0)) * ramp((BAND.3 - f) / (BAND.3 - BAND.2))
            })
            .collect();
        Some(Self {
            window,
            before: Search::samples(search.before_ms),
            after: Search::samples(search.after_ms),
            mic_step: Decimator::new(STEP),
            ref_step: Decimator::new(STEP),
            mic: Tail::default(),
            reference: Tail::default(),
            open: false,
            fft: Fft::new(FFT_LEN),
            hann: (0..window)
                .map(|k| 0.5 - 0.5 * (2.0 * PI * (k as f32 + 0.5) / window as f32).cos())
                .collect(),
            band,
            xr: vec![0.0; FFT_LEN],
            xi: vec![0.0; FFT_LEN],
            yr: vec![0.0; FFT_LEN],
            yi: vec![0.0; FFT_LEN],
            scratch: Vec::new(),
            stepped: Vec::new(),
        })
    }

    /// A reference stream begins: its first sample is taken to be the microphone's next. The streams are kept only from here.
    pub fn open(&mut self) {
        self.open = true;
        self.mic_step = Decimator::new(STEP);
        self.ref_step = Decimator::new(STEP);
        self.mic = Tail::default();
        self.reference = Tail::default();
    }

    pub fn close(&mut self) {
        self.open = false;
        self.mic = Tail::default();
        self.reference = Tail::default();
    }

    /// Reference samples held, at 16 kHz.
    pub fn reference_len(&self) -> usize {
        self.reference.end() as usize * STEP
    }

    fn take(
        step: &mut Decimator,
        tail: &mut Tail,
        scratch: &mut Vec<f32>,
        stepped: &mut Vec<f32>,
        samples: &[i16],
        keep: usize,
    ) {
        scratch.clear();
        scratch.extend(samples.iter().map(|&s| f32::from(s) / 32_768.0));
        stepped.clear();
        step.process(scratch, stepped);
        tail.push(stepped, keep);
    }

    pub fn push_mic(&mut self, samples: &[i16]) {
        if self.open {
            let keep = self.window + self.before + SLACK;
            Self::take(
                &mut self.mic_step,
                &mut self.mic,
                &mut self.scratch,
                &mut self.stepped,
                samples,
                keep,
            );
        }
    }

    pub fn push_reference(&mut self, samples: &[i16]) {
        if self.open {
            let keep = self.window + self.before + self.after + SLACK;
            Self::take(
                &mut self.ref_step,
                &mut self.reference,
                &mut self.scratch,
                &mut self.stepped,
                samples,
                keep,
            );
        }
    }

    /// The lag of the newest window of microphone whose whole reach of reference has arrived, or `None` while there is too little
    /// of either stream or the reference is silent.
    pub fn estimate(&mut self) -> Option<Estimate> {
        if !self.open {
            return None;
        }
        let (w, before, after) = (self.window as i64, self.before as i64, self.after as i64);
        // The window ends early enough that the reference for the earliest echo it can hold (`before` ahead of the reference) is here.
        let b = self.mic.end().min(self.reference.end() - before);
        let a = b - w;
        if a < self.mic.start {
            return None;
        }
        let span = (w + before + after) as usize;
        let mut energy = 0.0f32;
        for (j, y) in self.yr[..span].iter_mut().enumerate() {
            *y = self.reference.at(a - after + j as i64);
            energy += *y * *y;
        }
        if energy / (span as f32) < SILENT {
            return None;
        }
        self.yr[span..].fill(0.0);
        self.yi.fill(0.0);
        for (k, x) in self.xr[..self.window].iter_mut().enumerate() {
            *x = self.mic.at(a + k as i64) * self.hann[k];
        }
        self.xr[self.window..].fill(0.0);
        self.xi.fill(0.0);
        self.fft.forward(&mut self.xr, &mut self.xi);
        self.fft.forward(&mut self.yr, &mut self.yi);

        let half = FFT_LEN / 2;
        let in_band = |k: usize| self.band[k] > 0.0;
        let (mut sum, mut count) = (0.0f32, 0usize);
        for k in (0..=half).filter(|&k| in_band(k)) {
            sum += (self.xr[k].hypot(self.xi[k])) * (self.yr[k].hypot(self.yi[k]));
            count += 1;
        }
        let floor = FLOOR * sum / count.max(1) as f32;
        // The cross-spectrum conj(X) Y over its own magnitude, in the band; the upper half is the conjugate of the lower.
        for k in 0..=half {
            let (re, im) = if in_band(k) {
                let (xr, xi, yr, yi) = (self.xr[k], self.xi[k], self.yr[k], self.yi[k]);
                let scale = self.band[k] / (xr.hypot(xi) * yr.hypot(yi) + floor);
                ((xr * yr + xi * yi) * scale, (xr * yi - xi * yr) * scale)
            } else {
                (0.0, 0.0)
            };
            self.xr[k] = re;
            self.xi[k] = im;
            if k > 0 && k < half {
                self.xr[FFT_LEN - k] = re;
                self.xi[FFT_LEN - k] = -im;
            }
        }
        self.xi[0] = 0.0;
        self.xi[half] = 0.0;
        self.fft.inverse(&mut self.xr, &mut self.xi);

        // R[s] is the correlation of the window with the reference `s` samples after the reference's first: s = after - lag.
        let reach = (before + after) as usize;
        let (mut at, mut peak) = (0usize, 0.0f32);
        for (s, r) in self.xr[..=reach].iter().enumerate() {
            if r.abs() > peak {
                (at, peak) = (s, r.abs());
            }
        }
        let (mut square, mut n) = (0.0f32, 0usize);
        for (s, r) in self.xr[..=reach].iter().enumerate() {
            if s.abs_diff(at) > GUARD {
                square += r * r;
                n += 1;
            }
        }
        let deviation = (square / n.max(1) as f32).sqrt();
        Some(Estimate {
            lag: (after - at as i64) as i32 * STEP as i32,
            confidence: if deviation > 0.0 {
                peak / deviation
            } else {
                0.0
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Speech-like noise: bursts of low-passed noise with syllable-rate gaps, deterministic.
    pub(crate) fn tv_sound(samples: usize, seed: u64) -> Vec<i16> {
        // A multiplier, so that neighbouring seeds are not the same sequence (`4 | 1` and `5 | 1` are).
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f32 / (1u64 << 53) as f32 - 0.5
        };
        let (mut lp, mut level) = (0.0f32, 0.0f32);
        (0..samples)
            .map(|i| {
                if i % 2_400 == 0 {
                    level = if next() > -0.3 {
                        0.04 + 0.2 * (next() + 0.5)
                    } else {
                        0.0
                    };
                }
                lp += 0.35 * (next() * 2.0 - lp);
                (lp * level * 32_768.0) as i16
            })
            .collect()
    }

    /// The microphone's side of a room: the sound `delay` samples late and scaled by `gain`.
    fn echo(sound: &[i16], delay: i64, gain: f32) -> Vec<i16> {
        (0..sound.len() as i64)
            .map(|t| {
                let i = t - delay;
                if i >= 0 && (i as usize) < sound.len() {
                    (f32::from(sound[i as usize]) * gain) as i16
                } else {
                    0
                }
            })
            .collect()
    }

    fn run(mic: &[i16], reference: &[i16]) -> Option<Estimate> {
        let mut e = Estimator::new(Search::default()).unwrap();
        e.open();
        for (m, r) in mic.chunks(320).zip(reference.chunks(320)) {
            e.push_mic(m);
            e.push_reference(r);
        }
        e.estimate()
    }

    #[test]
    fn a_receivers_delay_is_found_to_the_millisecond() {
        let sound = tv_sound(16_000 * 7, 1);
        let found = run(&echo(&sound, 3_100, 0.5), &sound).unwrap();
        assert!((found.lag - 3_100).abs() <= 8, "found {}", found.lag);
        assert!(found.confidence > 10.0, "confidence {}", found.confidence);
    }

    #[test]
    fn a_reference_that_arrives_after_its_echo_is_a_negative_lag() {
        // The echo is 40 ms behind the sound; the reference stream is 400 ms behind it, so it trails the echo by 360 ms.
        let sound = tv_sound(16_000 * 8, 2);
        let mut reference = vec![0i16; 6_400];
        reference.extend_from_slice(&sound);
        let found = run(&echo(&sound, 640, 0.5), &reference).unwrap();
        assert!(
            (found.lag - (640 - 6_400)).abs() <= 8,
            "found {}",
            found.lag
        );
        assert!(found.confidence > 10.0, "confidence {}", found.confidence);
    }

    #[test]
    fn an_echo_with_its_sign_flipped_is_still_found() {
        let sound = tv_sound(16_000 * 7, 3);
        let found = run(&echo(&sound, 2_000, -0.4), &sound).unwrap();
        assert!((found.lag - 2_000).abs() <= 8, "found {}", found.lag);
        assert!(found.confidence > 10.0);
    }

    #[test]
    fn a_reference_that_is_not_what_the_microphone_hears_is_not_trusted() {
        let found = run(
            &echo(&tv_sound(16_000 * 7, 4), 2_000, 0.5),
            &tv_sound(16_000 * 7, 5),
        )
        .unwrap();
        assert!(found.confidence < 7.0, "confidence {}", found.confidence);
    }

    #[test]
    fn a_silent_reference_has_no_estimate() {
        let sound = tv_sound(16_000 * 7, 6);
        assert_eq!(run(&echo(&sound, 2_000, 0.5), &vec![0; sound.len()]), None);
    }
}
