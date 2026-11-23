//! How loud the TV itself is, from the stream that carries what it plays, and what that asks of the wake word.
//!
//! The microphones sit in the TV's bezel, a few metres from a receiver whose speakers play the TV's sound. The wake word's false
//! accepts come from that sound (speech on a screen is the hardest thing it hears), and echo cancellation needs the reference to
//! line up with what the microphones hear to within its tail; here the reference is the mix *before* the receiver and its delay
//! (ARC and eARC add tens of milliseconds, more than a 64 ms filter covers), so no cancellation is attempted. What the level
//! does buy is cheap and measured on the neighbouring Show device: a cutoff a couple of points higher costs a few points of recall
//! and removes the false accepts. The meter only decides how much higher.

use crate::pcm::Format;

/// Below this (dBFS, smoothed RMS) the TV is quiet enough to ask nothing extra; at and above [`LOUD_DB`] the whole [`MAX_EXTRA`].
pub const QUIET_DB: f32 = -50.0;
pub const LOUD_DB: f32 = -30.0;
/// The most a cutoff is raised by (0.97 becomes 0.99).
pub const MAX_EXTRA: f32 = 0.02;

/// The smoothed level of a stream: it rises at once and falls 3 dB per 100 ms.
pub struct Meter {
    format: Format,
    carry: Vec<u8>,
    window: usize,
    sum: f64,
    seen: usize,
    db: f32,
}

impl Meter {
    pub fn new(format: Format) -> Self {
        Self {
            window: (format.rate / 10).max(1) as usize,
            format,
            carry: Vec::new(),
            sum: 0.0,
            seen: 0,
            db: -120.0,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        let frame = self.format.frame_bytes();
        let width = self.format.sample.bytes();
        let channels = self.format.channels;
        self.carry.extend_from_slice(bytes);
        let whole = self.carry.len() / frame * frame;
        for f in self.carry[..whole].chunks_exact(frame) {
            let mean = (0..channels)
                .map(|c| self.format.sample.value(&f[c * width..(c + 1) * width]))
                .sum::<f32>()
                / channels as f32;
            self.sum += f64::from(mean) * f64::from(mean);
            self.seen += 1;
            if self.seen == self.window {
                let now = if self.sum > 0.0 {
                    (10.0 * (self.sum / self.seen as f64).log10()) as f32
                } else {
                    -120.0
                };
                self.db = now.max(self.db - 3.0).max(-120.0);
                self.sum = 0.0;
                self.seen = 0;
            }
        }
        self.carry.drain(..whole);
    }

    /// The smoothed level in dBFS; -120 is silence.
    pub fn db(&self) -> f32 {
        self.db
    }
}

/// What a TV level asks of a detection beyond the cutoff: nothing when quiet, [`MAX_EXTRA`] when loud, in a straight line between.
pub fn strictness(db: f32) -> f32 {
    ((db - QUIET_DB) / (LOUD_DB - QUIET_DB)).clamp(0.0, 1.0) * MAX_EXTRA
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pcm::Sample;

    fn s16_stereo(level: f32, frames: usize) -> Vec<u8> {
        let v = (level * 32_768.0) as i16;
        (0..frames).flat_map(|_| [v.to_le_bytes(), v.to_le_bytes()].concat()).collect()
    }

    fn format() -> Format {
        Format {
            rate: 48_000,
            channels: 2,
            sample: Sample::S16,
        }
    }

    #[test]
    fn a_constant_level_reads_as_its_decibels_and_silence_as_the_floor() {
        let mut meter = Meter::new(format());
        meter.push(&s16_stereo(0.1, 4_800));
        assert!((meter.db() - (-20.0)).abs() < 0.1, "{}", meter.db());
        let mut quiet = Meter::new(format());
        quiet.push(&s16_stereo(0.0, 4_800));
        assert_eq!(quiet.db(), -120.0);
    }

    #[test]
    fn the_level_falls_slowly_and_rises_at_once() {
        let mut meter = Meter::new(format());
        meter.push(&s16_stereo(0.1, 4_800));
        meter.push(&s16_stereo(0.0, 4_800));
        assert!((meter.db() - (-23.0)).abs() < 0.1, "3 dB down after 100 ms: {}", meter.db());
        meter.push(&s16_stereo(0.3, 4_800));
        assert!(meter.db() > -10.6);
    }

    #[test]
    fn a_read_that_cuts_a_frame_is_finished_by_the_next() {
        let bytes = s16_stereo(0.1, 4_800);
        let mut a = Meter::new(format());
        a.push(&bytes);
        let mut b = Meter::new(format());
        for piece in bytes.chunks(7) {
            b.push(piece);
        }
        assert!((a.db() - b.db()).abs() < 1e-3);
    }

    #[test]
    fn the_cutoff_is_raised_by_the_level_between_quiet_and_loud() {
        assert_eq!(strictness(-70.0), 0.0);
        assert_eq!(strictness(QUIET_DB), 0.0);
        assert!((strictness(-40.0) - 0.01).abs() < 1e-6);
        assert!((strictness(LOUD_DB) - MAX_EXTRA).abs() < 1e-6);
        assert!((strictness(0.0) - MAX_EXTRA).abs() < 1e-6);
    }
}
