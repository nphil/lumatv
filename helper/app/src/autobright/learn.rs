//! The owner's own curve, learned from his corrections the way a phone's adaptive brightness is: each settled correction is a point
//! (the room's light, the level he chose as a share of his ceiling), and the level wanted in a light is a curve fitted through the
//! points, from the default curve where there are none.
//!
//! - **The fit** is a penalised spline: nine knots evenly spaced over [`super::reach`] (log lux, 1 to 30 lux), a point pulling the two
//!   knots either side of it, a weak pull of every knot to the default curve ([`PRIOR_WEIGHT`]) and a smoothness penalty on how far the
//!   knots stray from that curve in step ([`SMOOTHNESS`]). Together they make a point move the curve near its light and fade out over
//!   a few knots (to 0.6 at each knot, a lux factor of 1.5: a correction at 1 lux moves 6 lux by a seventh of it and 12 lux by a
//!   twentieth), and make many points override the default. The solution is then made non-decreasing (pooled adjacent violators) and
//!   the top knot is pinned to the ceiling: a brighter room never gets a dimmer picture and nothing goes above the ceiling.
//! - **Weights.** A point is worth [`NEW_WEIGHT`], four hundred times the default curve's pull, so the curve meets the newest
//!   correction within a unit and the picture is not pulled back after he lets go. It halves every [`HALF_LIFE_DAYS`], so a habit he
//!   stops repeating fades to the default over about four months. A new point that disagrees with an older one near the same light by
//!   more than [`AGREE`] cuts the older one's weight to a fiftieth: the newer wish replaces it instead of being averaged with it for
//!   ever. Agreeing points keep their weight and add up.
//! - **Modes.** Each picture mode has its own points; the other modes' points count a tenth ([`OTHER_MODE`]), so a mode with none
//!   follows the room's habits learned elsewhere and one with a few of its own follows those.
//! - **Contexts.** A point also carries the app in front and whether it was playing full-screen video, so Home, an app's browsing
//!   and its playback can each end up with their own level. Within a mode the curve is laid out in three steps: all its points,
//!   then the playing (or browsing) ones, then the app's own, each held to the step before by [`CONTEXT_PRIOR`]: a new app with
//!   one point is pulled partway to it, a few consistent ones make it its own. A newer point replaces an older one only in its
//!   own context.
//! - **Time of day** is kept with each point but not fitted: a few dozen points over a handful of evenings cannot tell an evening
//!   preference from a lamp-lit room's, and the room's light already carries most of the difference.

use super::{Inputs, Scene, reach, write_atomic};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The points, in the data directory beside the journal.
pub const FILE: &str = "autobright-learned.json";
/// Points kept per mode: the one with the least weight now goes first, the oldest among equals.
pub const MAX_POINTS: usize = 200;
/// A point's weight halves this often.
pub const HALF_LIFE_DAYS: f64 = 14.0;
/// A new point's weight, against [`PRIOR_WEIGHT`] for each knot's pull to the default curve.
pub const NEW_WEIGHT: f64 = 200.0;
pub const PRIOR_WEIGHT: f64 = 0.5;
pub const SMOOTHNESS: f64 = 2.0;
/// The weight of another mode's point, as a share.
pub const OTHER_MODE: f64 = 0.1;
/// Points count as near in [`super::reach`] (a lux factor of 1.3) and as agreeing within this share of the ceiling.
pub const NEAR: f64 = 0.08;
pub const AGREE: f64 = 0.04;
/// What an older, nearby, disagreeing point keeps of its weight.
pub const SUPERSEDED: f64 = 0.02;
/// A point whose weight, with age, falls below this is dropped (twice the default curve's pull: it no longer moves the curve).
pub const NEGLIGIBLE: f64 = 1.0;
const KNOTS: usize = 9;
const PIN: f64 = 1e6;
/// How hard a context's curve is held to its parent's, and how smooth its difference from it is: one point of [`NEW_WEIGHT`] takes
/// the context about 44% of the way to the level chosen, two 61%, five 80%.
pub const CONTEXT_PRIOR: f64 = 100.0;
pub const CONTEXT_SMOOTH: f64 = 400.0;

/// The level wanted when nothing is learned: half the ceiling in the dark, the ceiling at [`super::BRIGHT_LUX`], on a log scale.
pub fn prior(reach: f64) -> f64 {
    0.5 + 0.5 * reach
}

/// One settled correction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub lux: f64,
    /// The level chosen, as a share of the ceiling.
    pub level: f64,
    /// Unix seconds.
    pub at: u64,
    /// Local minutes since midnight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minute: Option<u32>,
    /// The app in front, and whether it was playing full-screen video.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub app: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub playing: bool,
    /// What is left of [`NEW_WEIGHT`] after later corrections near it that disagreed.
    pub weight: f64,
}

impl Point {
    fn is_in(&self, scene: &Scene) -> bool {
        self.app == scene.app && self.playing == scene.playing
    }

    fn sound(&self) -> bool {
        self.lux.is_finite() && self.lux >= 0.0 && (0.0..=1.0).contains(&self.level) && self.weight.is_finite() && self.weight > 0.0
    }

    /// The weight at `now` (unix seconds).
    fn worth(&self, now: u64) -> f64 {
        self.weight * 0.5_f64.powf(now.saturating_sub(self.at) as f64 / 86_400.0 / HALF_LIFE_DAYS)
    }
}

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct File {
    /// Corrections learned since the last reset, kept points or not.
    #[serde(default)]
    adjustments: u64,
    #[serde(default)]
    modes: BTreeMap<String, Vec<Point>>,
}

#[derive(Debug, Default)]
pub struct Learner {
    path: Option<PathBuf>,
    file: File,
}

impl Learner {
    /// What is at `path`; a missing or damaged file is the default curve with no points.
    pub fn load(path: &Path) -> Learner {
        let mut file = match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice::<File>(&bytes).unwrap_or_else(|e| {
                crate::log_warn!("auto brightness: {} is damaged ({e}); starting from the default curve", path.display());
                File::default()
            }),
            Err(_) => File::default(),
        };
        for points in file.modes.values_mut() {
            points.retain(Point::sound);
            if points.len() > MAX_POINTS {
                points.sort_by_key(|p| p.at);
                points.drain(..points.len() - MAX_POINTS);
            }
        }
        file.modes.retain(|_, p| !p.is_empty());
        Learner { path: Some(path.to_owned()), file }
    }

    /// Corrections learned since the last reset.
    pub fn adjustments(&self) -> u64 {
        self.file.adjustments
    }

    /// Apps with a point kept.
    pub fn apps(&self) -> usize {
        let apps: std::collections::BTreeSet<&str> = self.file.modes.values().flatten().map(|p| p.app.as_str()).filter(|a| !a.is_empty()).collect();
        apps.len()
    }

    #[cfg(test)]
    pub fn held(&self, key: &str) -> usize {
        self.file.modes.get(key).map_or(0, Vec::len)
    }

    #[cfg(test)]
    pub fn held_in(&self, key: &str, scene: &Scene) -> usize {
        self.file.modes.get(key).map_or(0, |p| p.iter().filter(|p| p.is_in(scene)).count())
    }

    /// Forgets everything, in the file too.
    pub fn reset(&mut self) {
        self.file = File::default();
        self.save();
    }

    /// A settled correction: in `scene` and this `lux` the owner wants `level` of his ceiling in mode `key`.
    pub fn add(&mut self, key: &str, scene: &Scene, lux: f64, level: f64, minute: Option<u32>, now: u64) {
        let r = reach(lux);
        let points = self.file.modes.entry(key.to_owned()).or_default();
        for p in points.iter_mut().filter(|p| p.is_in(scene) && (reach(p.lux) - r).abs() <= NEAR && (p.level - level).abs() > AGREE) {
            p.weight *= SUPERSEDED;
        }
        points.retain(|p| p.worth(now) >= NEGLIGIBLE);
        points.push(Point { lux, level: level.clamp(0.0, 1.0), at: now, minute, app: scene.app.clone(), playing: scene.playing, weight: NEW_WEIGHT });
        if points.len() > MAX_POINTS {
            points.sort_by(|a, b| b.worth(now).total_cmp(&a.worth(now)).then(b.at.cmp(&a.at)));
            points.truncate(MAX_POINTS);
        }
        self.file.adjustments += 1;
        self.save();
    }

    /// The ceiling of mode `key` went from `old` to `new`: its points keep the level they stood for.
    pub fn rescale(&mut self, key: &str, old: f64, new: f64) {
        if let Some(points) = self.file.modes.get_mut(key) {
            for p in points {
                p.level = (p.level * old / new).clamp(0.0, 1.0);
            }
            self.save();
        }
    }

    /// The level wanted for `inputs`, between 0 and `ceiling`.
    pub fn target(&self, inputs: &Inputs, ceiling: f64) -> f64 {
        (self.level(inputs.mode, inputs.scene, inputs.lux, inputs.epoch) * ceiling).clamp(0.0, ceiling)
    }

    /// The level wanted in `lux` in mode `key` and `scene`, as a share of the ceiling, at `now` (unix seconds). Three fits, each
    /// laid over the one before and held back by [`CONTEXT_PRIOR`] so a context with few points stays near its parent: the mode's
    /// curve from all its points, then the one for the same kind of use (playing or browsing) in every app, then the app's own.
    pub fn level(&self, key: &str, scene: &Scene, lux: f64, now: u64) -> f64 {
        let r = reach(lux);
        if self.file.modes.is_empty() {
            return prior(r);
        }
        let mut v = self.fit(key, now);
        if let Some(points) = self.file.modes.get(key) {
            let kind: Vec<&Point> = points.iter().filter(|p| p.playing == scene.playing).collect();
            v = layer(v, &kind, now);
            let own: Vec<&Point> = kind.into_iter().filter(|p| p.app == scene.app).collect();
            v = layer(v, &own, now);
            non_decreasing(&mut v, &[1.0; KNOTS]);
        }
        let (j, t) = place(r);
        (v[j] * (1.0 - t) + v[j + 1] * t).clamp(0.0, 1.0)
    }

    fn fit(&self, key: &str, now: u64) -> [f64; KNOTS] {
        let last = KNOTS - 1;
        let at = |k: usize| k as f64 / last as f64;
        let p: [f64; KNOTS] = std::array::from_fn(|k| prior(at(k)));
        let mut a = [[0.0; KNOTS]; KNOTS];
        let mut b = [0.0; KNOTS];
        for k in 0..KNOTS {
            a[k][k] += PRIOR_WEIGHT;
            b[k] += PRIOR_WEIGHT * p[k];
        }
        a[last][last] += PIN;
        b[last] += PIN;
        for k in 0..last {
            let d = p[k + 1] - p[k];
            a[k][k] += SMOOTHNESS;
            a[k + 1][k + 1] += SMOOTHNESS;
            a[k][k + 1] -= SMOOTHNESS;
            a[k + 1][k] -= SMOOTHNESS;
            b[k + 1] += SMOOTHNESS * d;
            b[k] -= SMOOTHNESS * d;
        }
        for (mode, points) in &self.file.modes {
            let share = if mode == key { 1.0 } else { OTHER_MODE };
            for point in points {
                let (j, t) = place(reach(point.lux));
                data(&mut a, &mut b, j, t, point.worth(now) * share, point.level);
            }
        }
        let weights: [f64; KNOTS] = std::array::from_fn(|k| a[k][k]);
        let mut v = solve(a, b);
        non_decreasing(&mut v, &weights);
        v.map(|x| x.clamp(0.0, 1.0))
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let body = serde_json::to_vec(&self.file).unwrap_or_default();
        if let Err(e) = write_atomic(path, &body) {
            crate::log_warn!("auto brightness: saving {}: {e}", path.display());
        }
    }
}

/// Gaussian elimination with partial pivoting; the matrix is symmetric and positive definite.
fn solve(mut a: [[f64; KNOTS]; KNOTS], mut b: [f64; KNOTS]) -> [f64; KNOTS] {
    for col in 0..KNOTS {
        let pivot = (col..KNOTS).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs())).unwrap_or(col);
        a.swap(col, pivot);
        b.swap(col, pivot);
        for row in col + 1..KNOTS {
            let f = a[row][col] / a[col][col];
            for c in col..KNOTS {
                a[row][c] -= f * a[col][c];
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = [0.0; KNOTS];
    for row in (0..KNOTS).rev() {
        let tail: f64 = (row + 1..KNOTS).map(|c| a[row][c] * x[c]).sum();
        x[row] = (b[row] - tail) / a[row][row];
    }
    x
}

/// Pooled adjacent violators: the closest non-decreasing values, each knot weighted by its stiffness.
fn non_decreasing(v: &mut [f64; KNOTS], w: &[f64; KNOTS]) {
    // each block: weighted sum, weight, knots
    let mut blocks: Vec<(f64, f64, usize)> = Vec::with_capacity(KNOTS);
    for k in 0..KNOTS {
        blocks.push((v[k] * w[k], w[k], 1));
        while blocks.len() > 1 {
            let (s1, w1, _) = blocks[blocks.len() - 2];
            let (s2, w2, _) = blocks[blocks.len() - 1];
            if s1 / w1 <= s2 / w2 {
                break;
            }
            let (_, _, n2) = blocks.pop().unwrap_or_default();
            let last = blocks.len() - 1;
            blocks[last] = (s1 + s2, w1 + w2, blocks[last].2 + n2);
        }
    }
    let mut k = 0;
    for (s, wt, n) in blocks {
        v[k..k + n].fill(s / wt);
        k += n;
    }
}

/// The knot at or below `reach` and how far past it, from 0 to 1.
fn place(reach: f64) -> (usize, f64) {
    let pos = reach * (KNOTS - 1) as f64;
    let j = (pos as usize).min(KNOTS - 2);
    (j, pos - j as f64)
}

/// A point of `weight` at `t` past knot `j` pulling the curve toward `y`.
fn data(a: &mut [[f64; KNOTS]; KNOTS], b: &mut [f64; KNOTS], j: usize, t: f64, weight: f64, y: f64) {
    a[j][j] += weight * (1.0 - t) * (1.0 - t);
    a[j + 1][j + 1] += weight * t * t;
    a[j][j + 1] += weight * (1.0 - t) * t;
    a[j + 1][j] += weight * (1.0 - t) * t;
    b[j] += weight * (1.0 - t) * y;
    b[j + 1] += weight * t * y;
}

/// `base` moved toward `points`: a smooth, small difference that the top knot (the ceiling) never takes part in.
fn layer(base: [f64; KNOTS], points: &[&Point], now: u64) -> [f64; KNOTS] {
    if points.is_empty() {
        return base;
    }
    let last = KNOTS - 1;
    let mut a = [[0.0; KNOTS]; KNOTS];
    let mut b = [0.0; KNOTS];
    for (k, row) in a.iter_mut().enumerate() {
        row[k] += CONTEXT_PRIOR;
    }
    a[last][last] += PIN;
    for k in 0..last {
        a[k][k] += CONTEXT_SMOOTH;
        a[k + 1][k + 1] += CONTEXT_SMOOTH;
        a[k][k + 1] -= CONTEXT_SMOOTH;
        a[k + 1][k] -= CONTEXT_SMOOTH;
    }
    for p in points {
        let (j, t) = place(reach(p.lux));
        data(&mut a, &mut b, j, t, p.worth(now), p.level - (base[j] * (1.0 - t) + base[j + 1] * t));
    }
    let g = solve(a, b);
    std::array::from_fn(|k| (base[k] + g[k]).clamp(0.0, 1.0))
}
