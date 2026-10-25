//! Time-based smoothing in the manner of HyperHDR's linear smoothing, with its anti-flicker option.
//!
//! Every new target starts a straight run from wherever the colours are now to the target, taking `settle_ms`; a
//! target that arrives mid-run restarts the run from the current value. The result depends on timestamps, not on how
//! often frames or ticks come, so a 60 Hz capture and a 25 Hz one settle in the same time.
//!
//! Anti-flicker (HyperHDR's `antiFlickeringTreshold`, `Step` and `Timeout`): a target within `threshold` of the
//! current output on every channel is not followed, unless nothing has moved for `timeout_ms`, in which case the
//! output creeps toward it by at most `step` per tick. Camera or encoder noise in a still scene then leaves the
//! lights still, while a real change goes through at once.

use crate::rgb::Rgb;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SmoothConfig {
    /// Time to run from one target to the next; 0 follows the target exactly.
    pub settle_ms: u32,
    pub anti_flicker: AntiFlicker,
}

impl Default for SmoothConfig {
    fn default() -> Self {
        SmoothConfig { settle_ms: 180, anti_flicker: AntiFlicker::default() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AntiFlicker {
    /// Largest per-channel difference (0..1) that counts as noise; 0 turns the feature off.
    pub threshold: f32,
    /// Largest move per tick once the timeout has passed.
    pub step: f32,
    pub timeout_ms: u32,
}

impl Default for AntiFlicker {
    fn default() -> Self {
        AntiFlicker { threshold: 0.012, step: 0.004, timeout_ms: 1500 }
    }
}

#[derive(Clone, Debug)]
pub struct Smoother {
    cfg: SmoothConfig,
    current: Vec<Rgb>,
    start: Vec<Rgb>,
    target: Vec<Rgb>,
    /// The latest target that was within the anti-flicker threshold of the output and so was not followed.
    pending: Vec<Rgb>,
    has_pending: bool,
    run_from_ms: u64,
    run_to_ms: u64,
}

impl Smoother {
    pub fn new(cfg: SmoothConfig, lights: usize) -> Smoother {
        Smoother {
            cfg,
            current: vec![Rgb::BLACK; lights],
            start: vec![Rgb::BLACK; lights],
            target: vec![Rgb::BLACK; lights],
            pending: vec![Rgb::BLACK; lights],
            has_pending: false,
            run_from_ms: 0,
            run_to_ms: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.current.len()
    }

    pub fn is_empty(&self) -> bool {
        self.current.is_empty()
    }

    /// Jumps to these colours with no run (first frame, or a deliberate cut).
    pub fn snap(&mut self, now_ms: u64, colors: &[Rgb]) {
        assert_eq!(colors.len(), self.current.len());
        self.current.copy_from_slice(colors);
        self.start.copy_from_slice(colors);
        self.target.copy_from_slice(colors);
        self.has_pending = false;
        self.run_from_ms = now_ms;
        self.run_to_ms = now_ms;
    }

    /// A new target frame, timestamped with the time it is meant to start showing.
    pub fn set_target(&mut self, now_ms: u64, colors: &[Rgb]) {
        assert_eq!(colors.len(), self.current.len());
        let af = self.cfg.anti_flicker;
        // Bring the output up to `now_ms` under the old run first: a target that arrives every tick must not
        // throw away the travel since the last one.
        self.advance(now_ms);
        let settled = now_ms >= self.run_to_ms;
        if af.threshold > 0.0 && settled && colors.iter().zip(&self.current).all(|(n, c)| n.max_delta(*c) <= af.threshold) {
            // Noise around where the output already is: remember it for the slow creep, do not start a run.
            self.pending.copy_from_slice(colors);
            self.has_pending = colors != self.current.as_slice();
            return;
        }
        self.has_pending = false;
        self.start.copy_from_slice(&self.current);
        self.target.copy_from_slice(colors);
        self.run_from_ms = now_ms;
        self.run_to_ms = now_ms + u64::from(self.cfg.settle_ms);
    }

    fn advance(&mut self, now_ms: u64) {
        if now_ms < self.run_to_ms {
            let span = (self.run_to_ms - self.run_from_ms).max(1) as f32;
            let k = now_ms.saturating_sub(self.run_from_ms) as f32 / span;
            for ((c, s), t) in self.current.iter_mut().zip(&self.start).zip(&self.target) {
                *c = s.lerp(*t, k);
            }
        } else {
            self.current.copy_from_slice(&self.target);
        }
    }

    /// The colours to show at `now_ms`.
    pub fn step(&mut self, now_ms: u64) -> &[Rgb] {
        self.advance(now_ms);
        if now_ms >= self.run_to_ms {
            let af = self.cfg.anti_flicker;
            if self.has_pending && now_ms.saturating_sub(self.run_to_ms) >= u64::from(af.timeout_ms) {
                // Still for the timeout: close the small offset, at most `step` per tick on the furthest channel.
                let mut open = false;
                for (c, p) in self.current.iter_mut().zip(&self.pending) {
                    let d = c.max_delta(*p);
                    if d > af.step {
                        *c = c.lerp(*p, af.step / d);
                        open = true;
                    } else {
                        *c = *p;
                    }
                }
                self.target.copy_from_slice(&self.current);
                self.has_pending = open;
            }
        }
        &self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFF: AntiFlicker = AntiFlicker { threshold: 0.0, step: 0.0, timeout_ms: 0 };

    fn cfg(settle_ms: u32) -> SmoothConfig {
        SmoothConfig { settle_ms, anti_flicker: OFF }
    }

    #[test]
    fn runs_straight_to_the_target_in_settle_time() {
        let mut s = Smoother::new(cfg(200), 1);
        s.snap(0, &[Rgb::new(0.0, 0.0, 0.0)]);
        s.set_target(1000, &[Rgb::new(1.0, 0.5, 0.0)]);
        assert_eq!(s.step(1000)[0], Rgb::BLACK);
        let mid = s.step(1100)[0];
        assert!((mid.r - 0.5).abs() < 1e-6 && (mid.g - 0.25).abs() < 1e-6);
        assert_eq!(s.step(1200)[0], Rgb::new(1.0, 0.5, 0.0));
        assert_eq!(s.step(5000)[0], Rgb::new(1.0, 0.5, 0.0));
    }

    #[test]
    fn a_new_target_restarts_the_run_from_where_it_is() {
        let mut s = Smoother::new(cfg(100), 1);
        s.snap(0, &[Rgb::BLACK]);
        s.set_target(0, &[Rgb::new(1.0, 0.0, 0.0)]);
        let half = s.step(50)[0];
        assert!((half.r - 0.5).abs() < 1e-6);
        s.set_target(50, &[Rgb::new(0.0, 0.0, 1.0)]);
        let mid = s.step(100)[0];
        assert!((mid.r - 0.25).abs() < 1e-6 && (mid.b - 0.5).abs() < 1e-6, "{mid:?}");
        assert_eq!(s.step(150)[0], Rgb::new(0.0, 0.0, 1.0));
    }

    #[test]
    fn result_does_not_depend_on_tick_rate() {
        let run = |period: u64| {
            let mut s = Smoother::new(cfg(300), 1);
            s.snap(0, &[Rgb::BLACK]);
            s.set_target(0, &[Rgb::new(1.0, 1.0, 1.0)]);
            let mut t = 0;
            let mut last = Rgb::BLACK;
            while t <= 150 {
                last = s.step(t)[0];
                t += period;
            }
            last
        };
        assert!((run(10).r - run(25).r).abs() < 1e-6);
        assert!((run(10).r - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_target_every_tick_still_gets_there() {
        // The pipeline sets a target on every captured frame; the output must keep moving under that.
        let mut s = Smoother::new(cfg(100), 1);
        s.snap(0, &[Rgb::BLACK]);
        let mut t = 0;
        let mut first_half = None;
        while t <= 1000 {
            s.set_target(t, &[Rgb::new(1.0, 1.0, 1.0)]);
            let c = s.step(t)[0].r;
            if first_half.is_none() && c >= 0.5 {
                first_half = Some(t);
            }
            t += 33;
        }
        assert!(s.step(1000)[0].r > 0.99);
        // Chasing a fresh target each tick is an exponential approach with time constant settle_ms.
        let h = first_half.expect("passes the halfway point");
        assert!((50..=100).contains(&h), "half way at {h} ms");
    }

    #[test]
    fn zero_settle_follows_exactly() {
        let mut s = Smoother::new(cfg(0), 2);
        s.set_target(10, &[Rgb::new(0.3, 0.3, 0.3), Rgb::new(0.9, 0.0, 0.0)]);
        assert_eq!(s.step(10)[1], Rgb::new(0.9, 0.0, 0.0));
    }

    fn af_cfg() -> SmoothConfig {
        SmoothConfig { settle_ms: 100, anti_flicker: AntiFlicker { threshold: 0.02, step: 0.005, timeout_ms: 1000 } }
    }

    #[test]
    fn noise_below_the_threshold_leaves_the_output_still() {
        let mut s = Smoother::new(af_cfg(), 1);
        s.snap(0, &[Rgb::new(0.5, 0.5, 0.5)]);
        for i in 1..50u64 {
            let jitter = if i % 2 == 0 { 0.01 } else { -0.01 };
            s.set_target(i * 20, &[Rgb::new(0.5 + jitter, 0.5, 0.5)]);
            assert_eq!(s.step(i * 20)[0], Rgb::new(0.5, 0.5, 0.5), "moved at {i}");
        }
    }

    #[test]
    fn a_real_change_passes_at_once() {
        let mut s = Smoother::new(af_cfg(), 1);
        s.snap(0, &[Rgb::new(0.5, 0.5, 0.5)]);
        s.set_target(20, &[Rgb::new(0.8, 0.5, 0.5)]);
        assert!(s.step(70)[0].r > 0.6);
        assert!((s.step(120)[0].r - 0.8).abs() < 1e-6);
    }

    #[test]
    fn a_small_lasting_offset_is_closed_slowly_after_the_timeout() {
        let mut s = Smoother::new(af_cfg(), 1);
        s.snap(0, &[Rgb::new(0.5, 0.5, 0.5)]);
        // The scene settled 0.015 higher: inside the threshold, so not followed at first.
        s.set_target(0, &[Rgb::new(0.515, 0.5, 0.5)]);
        assert_eq!(s.step(500)[0].r, 0.5);
        // Same target keeps arriving; once the output has been still for the timeout it creeps.
        s.set_target(1500, &[Rgb::new(0.515, 0.5, 0.5)]);
        let a = s.step(1500)[0].r;
        let b = s.step(1520)[0].r;
        assert!(b > a || a > 0.5, "creeping: {a} {b}");
        let mut t = 1540;
        let mut last = b;
        while t < 3000 {
            last = s.step(t)[0].r;
            t += 20;
        }
        assert!((last - 0.515).abs() < 1e-4, "{last}");
    }
}
