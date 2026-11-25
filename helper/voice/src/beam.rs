//! The two microphones as one beam.
//!
//! The TV's far-field microphone is two PDM capsules read as a stereo stream. Measured on the set the helper runs on: both hear
//! the room at the same level to within 0.5 dB, the TV's speakers reach them in step (0.06 to 0.13 sample apart, so the pair
//! looks straight into the room), and the room's sound stays coherent between them up to 2 kHz and loses it between 2.5 and
//! 3 kHz, which puts them roughly 55 mm apart. Two capsules can only tell directions apart by the phase between them, which is a
//! small part of a cycle for anything near the front, so the beam is built to be safe before it is clever:
//!
//! * the look direction is distortionless: a source there passes at unity whatever the weights do;
//! * the weights are a minimum-variance solution (MVDR) per frequency bin on a slowly updated 2x2 noise covariance, pulled toward
//!   the diffuse field's (the superdirective prior) and loaded on the diagonal. The covariance learns only from blocks whose
//!   sound does not come from the look direction, so the person on the couch is never learned away;
//! * how far the weights may leave the plain sum is capped by the noise gain they would add to the capsules' own noise, which
//!   bounds what a wrong look direction can cost speech;
//! * when the capsules disagree (one dead or covered, or a block clips) the output is the plain sum, as before the beam.
//!
//! It is a 512-point STFT at 256-sample hops with sqrt-Hann windows (the echo canceller's block), both capsules packed into one
//! complex transform, so a hop costs two FFTs and allocates nothing. The output lags the input by one hop (16 ms) in every
//! mode, so a switch between the beam and its fallbacks is seamless.

use std::f32::consts::{PI, TAU};

use voice_dsp::fft::Fft;

/// The transform's length and the step between transforms.
const N: usize = 512;
const HOP: usize = N / 2;
const BINS: usize = HOP + 1;
const RATE: f32 = 16_000.0;
const SPEED_OF_SOUND: f32 = 343.0;

/// A sample this close to full scale is clipped.
const CLIP: f32 = 0.999;
/// Mean-square energy (about -75 dBFS) under which both capsules hear little but their own noise: no judgement of balance there.
const QUIET: f32 = 3.0e-8;
/// The capsules differ by more than this for a stretch (4 as a ratio of energies, 6 dB): one is dead, covered or broken, and the
/// weights would lean on the quieter one. The beam returns when the ratio is back under 2.5 (4 dB).
const UNBALANCED: f32 = 4.0;
const BALANCED: f32 = 2.5;
/// The balance is read over this many hops (about half a second).
const BALANCE_HOPS: f32 = 32.0;
/// Weight of the diffuse field's covariance in the one the weights are solved from, and the diagonal loading (a share of the
/// microphones' mean power).
const PRIOR: f32 = 0.3;
const LOADING: f32 = 0.02;
/// Mean-square floor per bin (far under the capsules' own noise): keeps a silent bin solvable.
const FLOOR: f32 = 1.0e-7;
/// The weights follow their target this fast per hop (a time constant of about 80 ms).
const FOLLOW: f32 = 0.2;
/// A block is "in front" when what the look direction cancels (the second capsule minus the first, aligned) is under this share
/// of the energy between 300 Hz and 3.4 kHz. Such a block is speech, or the TV, from the direction the beam protects.
const IN_FRONT: f32 = 0.12;
/// A bin's covariance is directional (its sound comes from one direction) when the cross term holds this share of the powers' product.
const COHERENT: f32 = 0.1;
const BAND: std::ops::Range<usize> = 10..110;

/// How the beam is built.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeamConfig {
    /// Off: [`Frames`](crate::pcm::Frames) mixes the channels as it always did.
    pub enabled: bool,
    /// The distance between the capsules in millimetres (the diffuse field's coherence is a sinc of it).
    pub spacing_mm: f32,
    /// A source in the look direction reaches the second capsule this many samples after the first (0 is straight ahead).
    pub look_delay: f32,
    /// The most the weights may add to the capsules' own noise over the plain sum, in dB.
    pub max_noise_gain_db: f32,
    /// How long the noise covariance remembers, in seconds.
    pub memory_secs: f32,
    /// Sound from within this many samples of delay of the look direction (1.3 is 30 degrees at 55 mm) is never nulled: the person
    /// talking is somewhere in that cone, and a null aimed at the TV's reflections there takes the talker with it.
    pub cone_samples: f32,
}

impl Default for BeamConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            spacing_mm: 55.0,
            look_delay: 0.0,
            max_noise_gain_db: 4.0,
            memory_secs: 6.0,
            cone_samples: 1.3,
        }
    }
}

/// What the output is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The beam.
    Beam,
    /// The capsules added and halved.
    Sum,
    /// One capsule.
    Mic(usize),
}

/// Everything one frequency bin keeps.
#[derive(Clone, Copy)]
struct Bin {
    /// The covariance of the two capsules: their powers and the cross term `E[x0 conj(x1)]`.
    p00: f32,
    p11: f32,
    p01r: f32,
    p01i: f32,
    /// The weights in use: the output is `conj(w0) x0 + conj(w1) x1`.
    w0r: f32,
    w0i: f32,
    w1r: f32,
    w1i: f32,
    /// What the second capsule hears of a source in the look direction, relative to the first.
    d1r: f32,
    d1i: f32,
    /// The coherence of a diffuse field between the capsules.
    gamma: f32,
    /// The cosine of the phase the cone's edge puts between the capsules in this bin (the cone wider than a half turn is all of it).
    cone: f32,
}

pub struct Beam {
    fft: Fft,
    window: Vec<f32>,
    bins: Vec<Bin>,
    /// The most the squared norm of the weights may exceed the plain sum's (0.5): `0.5 * (gain - 1)`.
    reach: f32,
    /// The covariance's memory in hops.
    memory: f32,
    learned: u32,
    adapt: bool,
    /// The input waiting for a whole hop, and the last transform's worth of it.
    pend: [Vec<f32>; 2],
    filled: usize,
    frame: [Vec<f32>; 2],
    tail: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    spectrum: Vec<[f32; 4]>,
    energy: [f32; 2],
    unbalanced: bool,
}

impl Beam {
    pub fn new(config: &BeamConfig) -> Self {
        let window = (0..N).map(|n| (PI * n as f32 / N as f32).sin()).collect();
        let bins = (0..BINS)
            .map(|k| {
                let phase = TAU * k as f32 * config.look_delay / N as f32;
                let x = TAU * k as f32 * RATE / N as f32 * config.spacing_mm * 1.0e-3
                    / SPEED_OF_SOUND;
                Bin {
                    p00: 0.0,
                    p11: 0.0,
                    p01r: 0.0,
                    p01i: 0.0,
                    w0r: 0.5,
                    w0i: 0.0,
                    w1r: 0.5 * phase.cos(),
                    w1i: -0.5 * phase.sin(),
                    d1r: phase.cos(),
                    d1i: -phase.sin(),
                    gamma: if x < 1.0e-6 { 1.0 } else { x.sin() / x },
                    cone: (TAU * k as f32 * config.cone_samples.max(0.0) / N as f32).min(PI).cos(),
                }
            })
            .collect();
        let gain = 10f32.powf(config.max_noise_gain_db.max(0.0) / 10.0);
        Self {
            fft: Fft::new(N),
            window,
            bins,
            reach: 0.5 * (gain - 1.0),
            memory: (config.memory_secs * RATE / HOP as f32).max(1.0),
            learned: 0,
            adapt: true,
            pend: [vec![0.0; HOP], vec![0.0; HOP]],
            filled: 0,
            frame: [vec![0.0; N], vec![0.0; N]],
            tail: vec![0.0; HOP],
            re: vec![0.0; N],
            im: vec![0.0; N],
            spectrum: vec![[0.0; 4]; BINS],
            energy: [0.0; 2],
            unbalanced: false,
        }
    }

    /// Whether the noise covariance learns. A turn is the person talking: the beam keeps what it knew.
    pub fn set_adapt(&mut self, on: bool) {
        self.adapt = on;
    }

    /// What the weights add to the capsules' own noise over the plain sum now, in dB, averaged over 300 Hz to 3.4 kHz.
    pub fn noise_gain_db(&self) -> f32 {
        let sum: f32 = self.bins[BAND]
            .iter()
            .map(|b| b.w0r * b.w0r + b.w0i * b.w0i + b.w1r * b.w1r + b.w1i * b.w1i)
            .sum();
        10.0 * (sum / BAND.len() as f32 / 0.5).log10()
    }

    /// Whether the capsules are judged too unequal for the beam (the output is the plain sum while that holds).
    pub fn unbalanced(&self) -> bool {
        self.unbalanced
    }

    /// Appends the output for `x0` and `x1` (the two capsules, the same length) to `out`: one hop at a time, as many as the input
    /// completes, each in the `mode` of the call that completes it.
    pub fn process(&mut self, x0: &[f32], x1: &[f32], mode: Mode, out: &mut Vec<f32>) {
        let len = x0.len().min(x1.len());
        let mut at = 0;
        while at < len {
            let take = (HOP - self.filled).min(len - at);
            self.pend[0][self.filled..self.filled + take].copy_from_slice(&x0[at..at + take]);
            self.pend[1][self.filled..self.filled + take].copy_from_slice(&x1[at..at + take]);
            self.filled += take;
            at += take;
            if self.filled == HOP {
                self.filled = 0;
                self.hop(mode, out);
            }
        }
    }

    fn hop(&mut self, mode: Mode, out: &mut Vec<f32>) {
        let mut clipped = false;
        for c in 0..2 {
            let mut energy = 0.0f32;
            for v in &mut self.pend[c] {
                if !v.is_finite() {
                    *v = 0.0;
                }
                clipped |= v.abs() >= CLIP;
                energy += *v * *v;
            }
            self.energy[c] += (energy / HOP as f32 - self.energy[c]) / BALANCE_HOPS;
            self.frame[c].copy_within(HOP.., 0);
            self.frame[c][HOP..].copy_from_slice(&self.pend[c]);
        }
        self.judge_balance();

        for n in 0..N {
            self.re[n] = self.window[n] * self.frame[0][n];
            self.im[n] = self.window[n] * self.frame[1][n];
        }
        self.fft.forward(&mut self.re, &mut self.im);
        // Both capsules went through one transform as `x0 + j x1`: their spectra are its conjugate-even and -odd parts.
        for k in 0..BINS {
            let (zr, zi) = (self.re[k], self.im[k]);
            let (yr, yi) = (self.re[(N - k) % N], self.im[(N - k) % N]);
            self.spectrum[k] = [
                0.5 * (zr + yr),
                0.5 * (zi - yi),
                0.5 * (zi + yi),
                0.5 * (yr - zr),
            ];
        }

        let adaptive = mode == Mode::Beam && !clipped && !self.unbalanced;
        if adaptive && self.adapt && !self.in_front() {
            self.learn();
        }
        let fixed = match mode {
            Mode::Beam | Mode::Sum => [0.5, 0.0, 0.5, 0.0],
            Mode::Mic(0) => [1.0, 0.0, 0.0, 0.0],
            Mode::Mic(_) => [0.0, 0.0, 1.0, 0.0],
        };
        for (k, (b, x)) in self.bins.iter_mut().zip(&self.spectrum).enumerate() {
            let w = if adaptive {
                let t = if protected(b) { [0.5, 0.0, 0.5 * b.d1r, 0.5 * b.d1i] } else { solve(b, self.reach) };
                b.w0r += FOLLOW * (t[0] - b.w0r);
                b.w0i += FOLLOW * (t[1] - b.w0i);
                b.w1r += FOLLOW * (t[2] - b.w1r);
                b.w1i += FOLLOW * (t[3] - b.w1i);
                [b.w0r, b.w0i, b.w1r, b.w1i]
            } else {
                fixed
            };
            let [x0r, x0i, x1r, x1i] = *x;
            self.re[k] = w[0] * x0r + w[1] * x0i + w[2] * x1r + w[3] * x1i;
            self.im[k] = w[0] * x0i - w[1] * x0r + w[2] * x1i - w[3] * x1r;
        }
        self.im[0] = 0.0;
        self.im[HOP] = 0.0;
        for k in 1..HOP {
            self.re[N - k] = self.re[k];
            self.im[N - k] = -self.im[k];
        }
        self.fft.inverse(&mut self.re, &mut self.im);
        for n in 0..HOP {
            out.push(self.tail[n] + self.re[n] * self.window[n]);
            self.tail[n] = self.re[HOP + n] * self.window[HOP + n];
        }
    }

    fn judge_balance(&mut self) {
        let [e0, e1] = self.energy;
        let (high, low) = if e0 > e1 { (e0, e1) } else { (e1, e0) };
        if high < QUIET {
            self.unbalanced = false;
            return;
        }
        let ratio = high / low.max(1.0e-12);
        self.unbalanced = ratio > if self.unbalanced { BALANCED } else { UNBALANCED };
    }

    /// Whether this block's sound is what the look direction passes: the energy left after aligning the capsules to it is small.
    fn in_front(&self) -> bool {
        let (mut off, mut all) = (0.0f32, 0.0f32);
        for k in BAND {
            let [x0r, x0i, x1r, x1i] = self.spectrum[k];
            let b = &self.bins[k];
            let r = x1r - (b.d1r * x0r - b.d1i * x0i);
            let i = x1i - (b.d1r * x0i + b.d1i * x0r);
            off += r * r + i * i;
            all += x0r * x0r + x0i * x0i + x1r * x1r + x1i * x1i;
        }
        off <= IN_FRONT * all
    }

    fn learn(&mut self) {
        // A running mean until the memory is full, so the first seconds are not biased toward zero.
        self.learned = self.learned.saturating_add(1);
        let rate = (1.0 / self.learned as f32).max(1.0 / self.memory);
        for (b, &[x0r, x0i, x1r, x1i]) in self.bins.iter_mut().zip(&self.spectrum) {
            b.p00 += rate * (x0r * x0r + x0i * x0i - b.p00);
            b.p11 += rate * (x1r * x1r + x1i * x1i - b.p11);
            b.p01r += rate * (x0r * x1r + x0i * x1i - b.p01r);
            b.p01i += rate * (x0i * x1r - x0r * x1i - b.p01i);
        }
    }
}

/// The MVDR weights of one bin toward its look direction, from its covariance blended with the diffuse field's, with the part
/// that leaves the plain sum cut so that the weights add at most `reach` (the squared norm over the sum's 0.5) of noise gain.
fn solve(b: &Bin, reach: f32) -> [f32; 4] {
    let power = 0.5 * (b.p00 + b.p11) + FLOOR;
    let diagonal = (PRIOR + LOADING) * power + FLOOR;
    let a = (1.0 - PRIOR) * b.p00 + diagonal;
    let d = (1.0 - PRIOR) * b.p11 + diagonal;
    let cr = (1.0 - PRIOR) * b.p01r + PRIOR * power * b.gamma;
    let ci = (1.0 - PRIOR) * b.p01i;
    let (dr, di) = (b.d1r, b.d1i);
    // The adjugate of the covariance times the steering vector `[1, d1]`; its inverse is that over the determinant, which the
    // normalisation takes out.
    let u0r = d - (cr * dr - ci * di);
    let u0i = -(cr * di + ci * dr);
    let u1r = a * dr - cr;
    let u1i = a * di + ci;
    let norm = (u0r + dr * u1r + di * u1i).max(1.0e-20).recip();
    // Its difference from the plain sum `[1/2, d1/2]` is orthogonal to the sum, so the squared norm adds up.
    let (e0r, e0i) = (u0r * norm - 0.5, u0i * norm);
    let (e1r, e1i) = (u1r * norm - 0.5 * dr, u1i * norm - 0.5 * di);
    let off = e0r * e0r + e0i * e0i + e1r * e1r + e1i * e1i;
    let keep = if off > reach { (reach / off).sqrt() } else { 1.0 };
    [
        0.5 + keep * e0r,
        keep * e0i,
        0.5 * dr + keep * e1r,
        0.5 * di + keep * e1i,
    ]
}

/// Whether the covariance holds directional sound from inside the cone the beam protects, which the weights then leave alone: the phase
/// of its cross term against the look direction's is within the cone's.
fn protected(b: &Bin) -> bool {
    let cr = b.p01r * b.d1r - b.p01i * b.d1i;
    let ci = b.p01r * b.d1i + b.p01i * b.d1r;
    let cross = cr * cr + ci * ci;
    cross >= COHERENT * b.p00 * b.p11 && cr > cross.sqrt() * b.cone
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Lcg(u32);

    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            f64::from(self.0 >> 8) / f64::from(1u32 << 24) - 0.5
        }
    }

    /// A plane wave of sinusoids `(hz, amplitude, phase)` as one capsule hears it, `delay` samples late.
    fn wave(parts: &[(f64, f64, f64)], delay: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                parts
                    .iter()
                    .map(|&(hz, a, p)| a * (std::f64::consts::TAU * hz * (i as f64 - delay) / 16_000.0 + p).sin())
                    .sum::<f64>() as f32
            })
            .collect()
    }

    /// `count` components between `lo` and `hi` Hz with random phases, none within 60 Hz of a `keep_clear` frequency, at an RMS of `rms`.
    fn band(rng: &mut Lcg, lo: f64, hi: f64, count: usize, rms: f64, keep_clear: &[f64]) -> Vec<(f64, f64, f64)> {
        let mut parts = Vec::new();
        for i in 0..count {
            let hz = lo + (hi - lo) * (i as f64 + 0.5) / count as f64;
            if keep_clear.iter().all(|c| (c - hz).abs() > 60.0) {
                parts.push((hz, 0.0, rng.next() * std::f64::consts::TAU));
            }
        }
        let a = rms * (2.0 / parts.len() as f64).sqrt();
        parts.iter_mut().for_each(|p| p.1 = a);
        parts
    }

    fn noise(rng: &mut Lcg, rms: f64, n: usize) -> Vec<f32> {
        (0..n).map(|_| (rng.next() * rms * 12f64.sqrt()) as f32).collect()
    }

    fn run(beam: &mut Beam, x0: &[f32], x1: &[f32], mode: Mode) -> Vec<f32> {
        let mut out = Vec::new();
        beam.process(x0, x1, mode, &mut out);
        out
    }

    /// The amplitude of the sinusoid at `hz` in `x`, over the whole of it.
    fn amplitude(x: &[f32], hz: f64) -> f64 {
        let (mut s, mut c) = (0.0f64, 0.0f64);
        for (i, &v) in x.iter().enumerate() {
            let t = std::f64::consts::TAU * hz * i as f64 / 16_000.0;
            s += f64::from(v) * t.sin();
            c += f64::from(v) * t.cos();
        }
        2.0 * (s * s + c * c).sqrt() / x.len() as f64
    }

    fn power_at(x: &[f32], parts: &[(f64, f64, f64)]) -> f64 {
        parts.iter().map(|p| amplitude(x, p.0).powi(2)).sum()
    }

    fn on() -> BeamConfig {
        BeamConfig { enabled: true, ..BeamConfig::default() }
    }

    #[test]
    fn the_modes_without_the_beam_are_the_capsules_a_hop_late() {
        let mut rng = Lcg(1);
        let (x0, x1) = (noise(&mut rng, 0.05, 4096), noise(&mut rng, 0.05, 4096));
        let mut beam = Beam::new(&on());
        for (mode, want) in [
            (Mode::Sum, (0..4096).map(|i| 0.5 * (x0[i] + x1[i])).collect::<Vec<_>>()),
            (Mode::Mic(0), x0.clone()),
            (Mode::Mic(1), x1.clone()),
        ] {
            let out = run(&mut beam, &x0, &x1, mode);
            assert_eq!(out.len(), 4096);
            for i in HOP..4096 {
                assert!((out[i] - want[i - HOP]).abs() < 1e-5, "{mode:?} at {i}");
            }
            beam = Beam::new(&on());
        }
    }

    #[test]
    fn a_source_in_the_look_direction_passes_at_unity_and_one_beside_it_is_cut() {
        let mut rng = Lcg(7);
        let n = 256 * 320;
        let tones = [(700.0, 0.02, 0.3), (2300.0, 0.02, 1.1), (3200.0, 0.02, 2.0)];
        let interferer = band(&mut rng, 1500.0, 3000.0, 40, 0.1, &[700.0, 2300.0, 3200.0]);
        let target = wave(&tones, 0.0, n);
        let noise_0 = noise(&mut rng, 1.0e-4, n);
        let noise_1 = noise(&mut rng, 1.0e-4, n);
        let (i0, i1) = (wave(&interferer, 0.0, n), wave(&interferer, 2.0, n));
        let x0: Vec<f32> = (0..n).map(|i| target[i] + i0[i] + noise_0[i]).collect();
        let x1: Vec<f32> = (0..n).map(|i| target[i] + i1[i] + noise_1[i]).collect();
        let mut beam = Beam::new(&on());
        let out = run(&mut beam, &x0, &x1, Mode::Beam);
        let sum = run(&mut Beam::new(&on()), &x0, &x1, Mode::Sum);
        let late = 16_000 * 3..n;
        for t in &tones {
            let gain = amplitude(&out[late.clone()], t.0) / t.1;
            assert!((gain - 1.0).abs() < 0.06, "{} Hz passes at {gain}", t.0);
        }
        let cut = 10.0 * (power_at(&sum[late.clone()], &interferer) / power_at(&out[late], &interferer)).log10();
        assert!(cut > 3.0, "the interferer is cut by {cut} dB");
        assert!(beam.noise_gain_db() <= 4.01, "{} dB of noise gain", beam.noise_gain_db());
    }

    #[test]
    fn sound_from_inside_the_cone_is_not_nulled() {
        let mut rng = Lcg(9);
        let n = 256 * 320;
        let interferer = band(&mut rng, 1000.0, 2800.0, 40, 0.1, &[]);
        let (i0, i1) = (wave(&interferer, 0.0, n), wave(&interferer, 0.9, n));
        let (n0, n1) = (noise(&mut rng, 1.0e-4, n), noise(&mut rng, 1.0e-4, n));
        let x0: Vec<f32> = (0..n).map(|i| i0[i] + n0[i]).collect();
        let x1: Vec<f32> = (0..n).map(|i| i1[i] + n1[i]).collect();
        let late = 16_000 * 3..n;
        let passed = |config: BeamConfig| {
            let out = run(&mut Beam::new(&config), &x0, &x1, Mode::Beam);
            let sum = run(&mut Beam::new(&config), &x0, &x1, Mode::Sum);
            10.0 * (power_at(&sum[late.clone()], &interferer) / power_at(&out[late.clone()], &interferer)).log10()
        };
        let kept = passed(on());
        let cut = passed(BeamConfig { cone_samples: 0.0, ..on() });
        assert!(kept.abs() < 0.5, "inside the cone the sound is cut by {kept} dB");
        assert!(cut > 3.0, "without the cone the same sound is cut by only {cut} dB");
    }

    #[test]
    fn the_look_direction_is_honoured_and_the_plain_sum_is_not_it() {
        let n = 16_000 * 4;
        let tone = [(3500.0, 0.05, 0.4)];
        let (x0, x1) = (wave(&tone, 0.0, n), wave(&tone, 1.0, n));
        let beam = &mut Beam::new(&BeamConfig { look_delay: 1.0, ..on() });
        let aimed = amplitude(&run(beam, &x0, &x1, Mode::Beam)[n / 2..], 3500.0) / 0.05;
        let summed = amplitude(&run(&mut Beam::new(&on()), &x0, &x1, Mode::Sum)[n / 2..], 3500.0) / 0.05;
        assert!((aimed - 1.0).abs() < 0.03, "aimed {aimed}");
        assert!(summed < 0.85, "the sum of out-of-step capsules is {summed}");
    }

    #[test]
    fn a_dead_capsule_leaves_the_plain_sum_and_nothing_worse() {
        let mut rng = Lcg(3);
        let n = 16_000 * 3;
        let tone = wave(&[(900.0, 0.1, 0.0)], 0.0, n);
        let hiss = noise(&mut rng, 1.0e-4, n);
        let x0: Vec<f32> = (0..n).map(|i| tone[i] + hiss[i]).collect();
        let x1 = vec![0.0; n];
        let out = run(&mut Beam::new(&on()), &x0, &x1, Mode::Beam);
        assert!(out.iter().all(|v| v.is_finite()));
        let got = amplitude(&out[n / 2..], 900.0);
        assert!((got - 0.05).abs() < 0.005, "half of the live capsule, as the plain sum has it: {got}");
    }

    #[test]
    fn clipped_blocks_are_not_learned_from_and_pass_as_the_plain_sum() {
        let mut rng = Lcg(5);
        let n = 256 * 188;
        let loud = band(&mut rng, 1500.0, 3400.0, 30, 0.1, &[]);
        let (x0, x1) = (wave(&loud, 0.0, n), wave(&loud, 0.9, n));
        let mut beam = Beam::new(&on());
        run(&mut beam, &x0, &x1, Mode::Beam);
        let before = beam.noise_gain_db();
        let (c0, c1): (Vec<f32>, Vec<f32>) = (0..8_000).map(|i| (if i % 2 == 0 { 1.0 } else { -1.0 }, 0.3 * x1[i])).unzip();
        let out = run(&mut beam, &c0, &c1, Mode::Beam);
        assert_eq!(beam.noise_gain_db(), before);
        for i in 2 * HOP..out.len() - HOP {
            assert!((out[i] - 0.5 * (c0[i - HOP] + c1[i - HOP])).abs() < 1e-4, "at {i}");
        }
    }

    #[test]
    fn the_output_does_not_depend_on_how_the_input_is_cut() {
        let mut rng = Lcg(11);
        let n = 16_000;
        let (x0, x1) = (noise(&mut rng, 0.05, n), noise(&mut rng, 0.05, n));
        let whole = run(&mut Beam::new(&on()), &x0, &x1, Mode::Beam);
        let mut beam = Beam::new(&on());
        let (mut out, mut at) = (Vec::new(), 0);
        let mut size = 1;
        while at < n {
            let end = (at + size).min(n);
            beam.process(&x0[at..end], &x1[at..end], Mode::Beam, &mut out);
            at = end;
            size = size * 7 % 613 + 1;
        }
        assert_eq!(out, whole);
    }

    #[test]
    fn silence_and_garbage_stay_finite() {
        let mut beam = Beam::new(&on());
        let silence = vec![0.0f32; 8192];
        assert!(run(&mut beam, &silence, &silence, Mode::Beam).iter().all(|&v| v == 0.0));
        let mut junk = vec![0.01f32; 8192];
        junk[100] = f32::NAN;
        junk[2000] = f32::INFINITY;
        junk[5000] = f32::NEG_INFINITY;
        assert!(run(&mut beam, &junk, &junk, Mode::Beam).iter().all(|v| v.is_finite()));
        assert!(beam.noise_gain_db().is_finite());
    }

    #[test]
    fn the_weights_never_lose_the_look_direction() {
        let mut rng = Lcg(13);
        let mut b = Bin { p00: 0.0, p11: 0.0, p01r: 0.0, p01i: 0.0, w0r: 0.0, w0i: 0.0, w1r: 0.0, w1i: 0.0, d1r: 0.0, d1i: 0.0, gamma: 0.0, cone: 1.0 };
        for _ in 0..500 {
            let phase = rng.next() * TAU as f64;
            (b.d1r, b.d1i) = (phase.cos() as f32, -phase.sin() as f32);
            b.gamma = rng.next() as f32 * 2.0;
            let (p, q) = ((rng.next() + 0.6) as f32, (rng.next() + 0.6) as f32);
            let c = (rng.next() as f32 * 0.9 * (p * q).sqrt(), rng.next() as f32 * 0.9 * (p * q).sqrt());
            (b.p00, b.p11, b.p01r, b.p01i) = (p, q, c.0, c.1);
            let reach = 0.5 * (10f32.powf(0.4) - 1.0);
            let [w0r, w0i, w1r, w1i] = solve(&b, reach);
            // wᴴd = conj(w0) + conj(w1) d1 = 1
            let (re, im) = (w0r + w1r * b.d1r + w1i * b.d1i, -w0i + w1r * b.d1i - w1i * b.d1r);
            assert!((re - 1.0).abs() < 1e-4 && im.abs() < 1e-4, "look response {re}+{im}j");
            let norm = w0r * w0r + w0i * w0i + w1r * w1r + w1i * w1i;
            assert!(norm <= 0.5 + reach + 1e-4, "norm {norm}");
        }
    }
}
