//! The owner's curve is fitted through settled brightness choices, expressed as shares of the ceiling.
//!
//! The fit uses nine knots over log lux. A correction pulls the knots around its light, and regularisation
//! keeps the curve smooth and near the default where little is known. The last knot stays at its prior
//! unless a bright-room correction pulls on it. The curve never falls as the room gets brighter or
//! exceeds the ceiling.
//!
//! Corrections decay with age. Games and other use have separate curves; other modes lend weaker evidence.
//! Within a mode, the playing or browsing curve and then the app's own curve refine the shared one.
//! A further time-of-day refinement needs corrections across different days in the same scene and mode.
//! Nearby minutes count more, including across midnight. A lone timed correction can inform the shared
//! curve, but does not establish a time-of-day habit.

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
/// What an older, nearby, disagreeing point keeps of its weight when replaced: in the same session, or both untimed.
pub const SUPERSEDED: f64 = 0.02;
/// What an older habit keeps when a later day disagrees at the same time of day: one outlier leaves it standing, three or four
/// days of the new choice take over.
pub const OUTLIVED: f64 = 0.6;
/// A point whose weight, with age, falls below this is dropped (twice the default curve's pull: it no longer moves the curve).
pub const NEGLIGIBLE: f64 = 1.0;
const KNOTS: usize = 9;
const PIN: f64 = 1e6;
const DAY_MINUTES: u32 = 24 * 60;
const TIME_WINDOW: u32 = 3 * 60;
// Two corrections across the same midnight are often one session, not a recurring habit.
const REPEAT_GAP: u64 = 12 * 60 * 60;
// Corrections this close together are one sitting: the later one is the choice, the earlier a pass on the way to it. Shorter
// than the gap between an evening and the next morning, which are separate habits.
const SESSION: u64 = 2 * 60 * 60;
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
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub game: bool,
    /// What is left of [`NEW_WEIGHT`] after later corrections near it that disagreed.
    pub weight: f64,
}

impl Point {
    fn is_in(&self, scene: &Scene) -> bool {
        self.app == scene.app && self.is_kind(scene)
    }

    fn is_kind(&self, scene: &Scene) -> bool {
        self.playing == scene.playing && self.game == scene.game
    }

    fn sound(&self) -> bool {
        self.lux.is_finite() && self.lux >= 0.0 && (0.0..=1.0).contains(&self.level) && self.weight > 0.0 && self.weight <= NEW_WEIGHT
    }

    /// The weight at `now` (unix seconds).
    fn worth(&self, now: u64) -> f64 {
        self.weight * 0.5_f64.powf(now.saturating_sub(self.at) as f64 / 86_400.0 / HALF_LIFE_DAYS)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Ceiling {
    baseline: u32,
    generation: u64,
    value: u32,
    /// The limit before this raise, and when (unix seconds) and in what light it was raised: a lower choice in the same sitting
    /// and light takes it back.
    #[serde(default)]
    from: u32,
    #[serde(default)]
    at: u64,
    #[serde(default)]
    lux: f64,
}

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct File {
    /// Corrections learned since the last reset, kept points or not.
    #[serde(default)]
    adjustments: u64,
    #[serde(default)]
    modes: BTreeMap<String, Vec<Point>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    ceilings: BTreeMap<String, Ceiling>,
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
        file.ceilings.retain(|_, c| (c.baseline..=100).contains(&c.value));
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

    /// Forgets learned corrections, without changing the manually chosen limits.
    pub fn reset(&mut self) {
        self.file.adjustments = 0;
        self.file.modes.clear();
        self.save();
    }

    /// A settled correction: in `scene` and this `lux` the owner wants `level` of his ceiling in mode `key`.
    pub fn add(&mut self, key: &str, scene: &Scene, lux: f64, level: f64, minute: Option<u32>, now: u64) {
        let minute = minute.filter(|&m| m < DAY_MINUTES);
        let points = self.file.modes.entry(key.to_owned()).or_default();
        let r = reach(lux);
        for p in points.iter_mut().filter(|p| p.is_in(scene) && (reach(p.lux) - r).abs() <= NEAR && (p.level - level).abs() > AGREE) {
            if now.saturating_sub(p.at) < SESSION || (minute.is_none() && p.minute.is_none()) {
                p.weight *= SUPERSEDED;
            } else if minute.zip(p.minute).is_none_or(|(a, b)| apart(a, b) < TIME_WINDOW) {
                p.weight *= OUTLIVED;
            }
        }
        points.retain(|p| p.worth(now) >= NEGLIGIBLE);
        points.push(Point {
            lux,
            level: level.clamp(0.0, 1.0),
            at: now,
            minute,
            app: scene.app.clone(),
            playing: scene.playing,
            game: scene.game,
            weight: NEW_WEIGHT,
        });
        if points.len() > MAX_POINTS {
            let least = points
                .iter()
                .enumerate()
                .map(|(i, p)| (i, p.worth(now), p.at))
                .min_by(|a, b| a.1.total_cmp(&b.1).then(a.2.cmp(&b.2)));
            if let Some((i, _, _)) = least {
                points.swap_remove(i);
            }
        }
        self.file.adjustments += 1;
        self.save();
    }

    /// The explicit limit for this mode, valid only for the current physical baseline revision.
    pub fn ceiling(&self, key: &str, baseline: u32, generation: u64) -> u32 {
        self.file
            .ceilings
            .get(key)
            .filter(|c| c.baseline == baseline && c.generation == generation)
            .map_or(baseline, |c| c.value)
    }

    /// A settled manual value above the limit raises it for this profile, not the hardware baseline. A lower value settled in the
    /// same sitting and light takes the raise back down to it: passing over the top on the way to a lower level must not stick.
    pub fn fit_ceiling(&mut self, key: &str, baseline: u32, generation: u64, value: u32, lux: f64, now: u64) {
        let value = value.min(100);
        let current = self.ceiling(key, baseline, generation);
        let recent = self
            .file
            .ceilings
            .get(key)
            .copied()
            .filter(|c| c.baseline == baseline && c.generation == generation && now.saturating_sub(c.at) < SESSION);
        let next = if value > current {
            Ceiling { baseline, generation, value, from: recent.map_or(current, |c| c.from), at: now, lux }
        } else if let Some(c) = recent.filter(|c| value < c.value && (reach(c.lux) - reach(lux)).abs() <= NEAR) {
            Ceiling { value: value.max(c.from), ..c }
        } else {
            return;
        };
        if next.value <= baseline {
            self.file.ceilings.remove(key);
        } else {
            self.file.ceilings.insert(key.to_owned(), next);
        }
        self.rescale(key, current, next.value.max(baseline));
        self.save();
    }

    /// Discard a raise from an older physical baseline, even if its numeric value came back.
    pub fn rebase_ceiling(&mut self, key: &str, baseline: u32, generation: u64) {
        if let Some(c) = self.file.ceilings.get(key).copied()
            && (c.baseline != baseline || c.generation != generation)
        {
            self.file.ceilings.remove(key);
            self.rescale(key, c.value, baseline);
            self.save();
        }
    }

    fn rescale(&mut self, key: &str, old: u32, new: u32) {
        if old != 0 && new != 0 && old != new
            && let Some(points) = self.file.modes.get_mut(key)
        {
            let ratio = f64::from(old) / f64::from(new);
            for p in points {
                p.level = (p.level * ratio).clamp(0.0, 1.0);
            }
        }
    }

    /// The level wanted for `inputs`, between 0 and `ceiling`.
    pub fn target(&self, inputs: &Inputs, ceiling: f64) -> f64 {
        (self.level_at(inputs.mode, inputs.scene, inputs.lux, inputs.minute, inputs.epoch) * ceiling).clamp(0.0, ceiling)
    }

    #[cfg(test)]
    pub fn level(&self, key: &str, scene: &Scene, lux: f64, now: u64) -> f64 {
        self.level_at(key, scene, lux, None, now)
    }

    fn level_at(&self, key: &str, scene: &Scene, lux: f64, minute: Option<u32>, now: u64) -> f64 {
        let r = reach(lux);
        if self.file.modes.is_empty() {
            return prior(r);
        }
        let mut v = self.fit(key, scene.game, now);
        if let Some(points) = self.file.modes.get(key) {
            let kind = points.iter().filter(|p| p.is_kind(scene));
            v = layer(v, kind.clone().map(|p| (p, p.worth(now))));
            let own = kind.filter(|p| p.app == scene.app);
            v = layer(v, own.clone().map(|p| (p, p.worth(now))));
            if let Some(minute) = minute.filter(|&m| m < DAY_MINUTES) {
                v = time_layer(v, own, minute, now);
            }
        }
        let (j, t) = place(r);
        (v[j] * (1.0 - t) + v[j + 1] * t).clamp(0.0, 1.0)
    }

    fn fit(&self, key: &str, game: bool, now: u64) -> [f64; KNOTS] {
        let last = KNOTS - 1;
        let at = |k: usize| k as f64 / last as f64;
        let p: [f64; KNOTS] = std::array::from_fn(|k| prior(at(k)));
        let mut a = [[0.0; KNOTS]; KNOTS];
        let mut b = [0.0; KNOTS];
        let mut known = false;
        for (mode, points) in &self.file.modes {
            let share = if mode == key { 1.0 } else { OTHER_MODE };
            for point in points.iter().filter(|p| p.game == game) {
                let (j, t) = place(reach(point.lux));
                data(&mut a, &mut b, j, t, point.worth(now) * share, point.level);
                known = true;
            }
        }
        if !known {
            return p;
        }
        if a[last][last] == 0.0 {
            a[last][last] += PIN;
            b[last] += PIN;
        }
        for k in 0..KNOTS {
            a[k][k] += PRIOR_WEIGHT;
            b[k] += PRIOR_WEIGHT * p[k];
        }
        for k in 0..last {
            let d = p[k + 1] - p[k];
            a[k][k] += SMOOTHNESS;
            a[k + 1][k + 1] += SMOOTHNESS;
            a[k][k + 1] -= SMOOTHNESS;
            a[k + 1][k] -= SMOOTHNESS;
            b[k + 1] += SMOOTHNESS * d;
            b[k] -= SMOOTHNESS * d;
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
    let mut blocks = [(0.0, 0.0, 0); KNOTS];
    let mut n = 0;
    for k in 0..KNOTS {
        let (mut s, mut wt, mut knots) = (v[k] * w[k], w[k], 1);
        while n > 0 {
            let (prev_s, prev_w, prev_knots) = blocks[n - 1];
            if prev_s / prev_w <= s / wt {
                break;
            }
            s += prev_s;
            wt += prev_w;
            knots += prev_knots;
            n -= 1;
        }
        blocks[n] = (s, wt, knots);
        n += 1;
    }
    let mut k = 0;
    for (s, wt, knots) in blocks.into_iter().take(n) {
        v[k..k + knots].fill(s / wt);
        k += knots;
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

fn time_layer<'a>(
    base: [f64; KNOTS],
    points: impl Iterator<Item = &'a Point> + Clone,
    minute: u32,
    now: u64,
) -> [f64; KNOTS] {
    let nearby = points.filter_map(|p| {
        let m = p.minute.filter(|&m| m < DAY_MINUTES)?;
        let share = f64::from(TIME_WINDOW.saturating_sub(apart(m, minute))) / f64::from(TIME_WINDOW);
        (share > 0.0).then(|| (p, p.worth(now) * share))
    });
    let Some((strongest, _)) = nearby.clone().max_by(|a, b| a.1.total_cmp(&b.1)) else { return base };
    // Taper by the supporting sample as well, so the first sample cannot turn on a time fit by itself.
    let repeat = nearby
        .clone()
        .filter(|(p, _)| p.at.abs_diff(strongest.at) >= REPEAT_GAP)
        .map(|(_, weight)| weight / NEW_WEIGHT)
        .fold(0.0, f64::max)
        .min(1.0);
    if repeat == 0.0 {
        return base;
    }
    // Only corroborated evidence gets here; it needs less blending with the shared curve than a single app point.
    let strength = 2.0 * repeat;
    layer(base, nearby.map(|(p, weight)| (p, weight * strength)))
}

/// Minutes between two times of day, the short way round midnight.
fn apart(a: u32, b: u32) -> u32 {
    let d = a.abs_diff(b);
    d.min(DAY_MINUTES - d)
}

/// `base` moved toward the weighted points, without changing its unobserved bright endpoint.
fn layer<'a>(base: [f64; KNOTS], mut points: impl Iterator<Item = (&'a Point, f64)>) -> [f64; KNOTS] {
    let Some(first) = points.next() else { return base };
    let last = KNOTS - 1;
    let mut a = [[0.0; KNOTS]; KNOTS];
    let mut b = [0.0; KNOTS];
    for (p, weight) in std::iter::once(first).chain(points) {
        let (j, t) = place(reach(p.lux));
        data(&mut a, &mut b, j, t, weight, p.level - (base[j] * (1.0 - t) + base[j + 1] * t));
    }
    if a[last][last] == 0.0 {
        a[last][last] += PIN;
    }
    for (k, row) in a.iter_mut().enumerate() {
        row[k] += CONTEXT_PRIOR;
    }
    // Monotonic projection gives the observed knots more say than unmeasured neighbours.
    let stiffness: [f64; KNOTS] = std::array::from_fn(|k| a[k][k]);
    for k in 0..last {
        a[k][k] += CONTEXT_SMOOTH;
        a[k + 1][k + 1] += CONTEXT_SMOOTH;
        a[k][k + 1] -= CONTEXT_SMOOTH;
        a[k + 1][k] -= CONTEXT_SMOOTH;
    }
    let g = solve(a, b);
    let mut v = std::array::from_fn(|k| (base[k] + g[k]).clamp(0.0, 1.0));
    non_decreasing(&mut v, &stiffness);
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autobright::BRIGHT_LUX;

    const MODE: &str = "default/eco";
    const CEILING: f64 = 80.0;
    const EPOCH: u64 = 1_800_000_000;
    const DAY: u64 = 86_400;

    fn predict(learner: &Learner, scene: &Scene, lux: f64, minute: Option<u32>, epoch: u64) -> f64 {
        learner.target(&Inputs { lux, mode: MODE, scene, minute, epoch }, CEILING)
    }

    #[test]
    fn recurring_morning_and_night_choices_predict_an_unseen_day() {
        let mut learner = Learner::default();
        let scene = Scene { app: "movie".into(), playing: true, game: false };
        let lux = BRIGHT_LUX.sqrt();
        let morning = 8 * 60;
        let night = DAY_MINUTES - 5;
        for day in 0..6 {
            let noise = if day % 2 == 0 { 0.01 } else { -0.01 };
            for (minute, level) in [(morning, 0.8), (night, 0.3)] {
                learner.add(MODE, &scene, lux, level + noise, Some(minute), EPOCH + day * DAY + u64::from(minute) * 60);
            }
        }

        let morning_epoch = EPOCH + 6 * DAY + u64::from(morning + 5) * 60;
        let night_epoch = EPOCH + 7 * DAY + 5 * 60;
        for (epoch, minute, wanted) in [(morning_epoch, morning + 5, 64.0), (night_epoch, 5, 24.0)] {
            let blind = predict(&learner, &scene, lux, None, epoch);
            let timed = predict(&learner, &scene, lux, Some(minute), epoch);
            assert!(
                (timed - wanted).abs() < 7.0 && (blind - wanted).abs() - (timed - wanted).abs() >= 12.0,
                "minute {minute}: wanted {wanted}, timed {timed:.2}, time-blind {blind:.2}"
            );
        }

        let night_before = predict(&learner, &scene, lux, Some(5), night_epoch);
        learner.add(MODE, &scene, lux, 1.0, Some(morning + 5), morning_epoch);
        let night_after = predict(&learner, &scene, lux, Some(5), night_epoch);
        assert!((night_before - night_after).abs() < 2.0, "night before {night_before:.2}, after {night_after:.2}");
        assert_eq!(learner.held_in(MODE, &scene), 13);
    }

    #[test]
    fn two_corrections_across_one_midnight_do_not_establish_a_habit() {
        let mut learner = Learner::default();
        let scene = Scene::default();
        learner.add(MODE, &scene, 5.0, 0.25, Some(DAY_MINUTES - 5), EPOCH);
        learner.add(MODE, &scene, 5.0, 0.75, Some(5), EPOCH + 10 * 60);
        let epoch = EPOCH + DAY;
        let blind = predict(&learner, &scene, 5.0, None, epoch);
        assert_eq!(predict(&learner, &scene, 5.0, Some(0), epoch), blind);
    }

    #[test]
    fn gaming_is_distinct_even_in_the_same_app_and_light() {
        let mut learner = Learner::default();
        let ordinary = Scene { app: "same-app".into(), playing: true, game: false };
        let game = Scene { game: true, ..ordinary.clone() };
        let minute = Some(19 * 60);
        let lux = 5.0;
        learner.add(MODE, &ordinary, lux, 0.35, minute, EPOCH);
        assert!((predict(&learner, &game, lux, minute, EPOCH) - prior(reach(lux)) * CEILING).abs() < 1e-9);

        for day in 1..4 {
            learner.add(MODE, &game, lux, 0.75, minute, EPOCH + day * DAY);
            learner.add(MODE, &ordinary, lux, 0.35, minute, EPOCH + day * DAY);
        }
        for (scene, wanted) in [(&ordinary, 28.0), (&game, 60.0)] {
            let got = predict(&learner, scene, lux, minute, EPOCH + 4 * DAY);
            assert!((got - wanted).abs() < 2.0, "game {}: wanted {wanted}, got {got}", scene.game);
        }
    }

    #[test]
    fn a_bright_room_can_learn_below_the_ceiling_without_reversing_the_curve() {
        let mut learner = Learner::default();
        let scene = Scene::default();
        let minute = Some(20 * 60);
        learner.add(MODE, &scene, 1.0, 0.2, minute, EPOCH);
        for day in 0..4 {
            learner.add(MODE, &scene, BRIGHT_LUX * 2.0, 0.65, minute, EPOCH + day * DAY);
        }
        let at = |lux| predict(&learner, &scene, lux, minute, EPOCH + 4 * DAY);
        assert!((at(BRIGHT_LUX) - CEILING * 0.65).abs() < 2.0, "bright: {}", at(BRIGHT_LUX));
        let mut previous = 0.0;
        for lux in [0.0, 1.0, 2.0, 4.0, 8.0, 12.0, 18.0, 24.0, 29.0, BRIGHT_LUX, 100.0] {
            let got = at(lux);
            assert!((0.0..=CEILING).contains(&got) && got >= previous - 1e-9, "lux {lux}: {got} after {previous}");
            previous = got;
        }
    }

    #[test]
    fn a_manual_raise_survives_reset_but_not_a_changed_baseline_revision() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        let mut learner = Learner::load(&path);
        learner.add(MODE, &Scene::default(), 1.0, 0.5, None, EPOCH);
        learner.fit_ceiling(MODE, 60, 1, 80, 1.0, EPOCH);
        assert_eq!(learner.ceiling(MODE, 60, 1), 80);
        assert_eq!(learner.file.modes[MODE][0].level * 80.0, 30.0);
        learner.reset();

        let mut learner = Learner::load(&path);
        assert_eq!((learner.adjustments(), learner.held(MODE), learner.ceiling(MODE, 60, 1)), (0, 0, 80));
        assert_eq!(learner.ceiling(MODE, 40, 2), 40);
        assert_eq!(learner.ceiling(MODE, 60, 3), 60);
        learner.rebase_ceiling(MODE, 60, 3);
        assert_eq!(Learner::load(&path).ceiling(MODE, 60, 1), 60);
    }

    #[test]
    fn overshooting_on_the_way_down_leaves_no_raise_or_bright_point() {
        let mut learner = Learner::default();
        let scene = Scene::default();
        let minute = Some(21 * 60);
        for (value, at) in [(100, EPOCH), (40, EPOCH + 60)] {
            learner.fit_ceiling(MODE, 50, 1, value, 2.0, at);
            let own = learner.ceiling(MODE, 50, 1);
            learner.add(MODE, &scene, 2.0, f64::from(value) / f64::from(own), minute, at);
        }
        assert_eq!(learner.ceiling(MODE, 50, 1), 50);
        let later = Inputs { lux: 2.0, mode: MODE, scene: &scene, minute, epoch: EPOCH + 2 * DAY };
        let got = learner.target(&later, 50.0);
        assert!((got - 40.0).abs() < 3.0, "got {got}");

        // A raise kept past the sitting stays.
        learner.fit_ceiling(MODE, 50, 1, 70, 2.0, EPOCH + 3 * DAY);
        learner.fit_ceiling(MODE, 50, 1, 45, 2.0, EPOCH + 3 * DAY + SESSION);
        assert_eq!(learner.ceiling(MODE, 50, 1), 70);
    }

    #[test]
    fn a_changed_evening_habit_takes_over_within_days() {
        let mut learner = Learner::default();
        let scene = Scene { app: "movie".into(), playing: true, game: false };
        let minute = 21 * 60;
        let at = |day: u64| EPOCH + day * DAY + u64::from(minute) * 60;
        for day in 0..7 {
            learner.add(MODE, &scene, 3.0, 0.3, Some(minute), at(day));
        }
        let predict_on = |l: &Learner, day| predict(l, &scene, 3.0, Some(minute), at(day)) / CEILING;
        learner.add(MODE, &scene, 3.0, 0.5, Some(minute), at(7));
        assert!(predict_on(&learner, 8) < 0.4, "one evening moved it to {}", predict_on(&learner, 8));
        for day in 8..11 {
            learner.add(MODE, &scene, 3.0, 0.5, Some(minute), at(day));
        }
        let got = predict_on(&learner, 11);
        assert!((got - 0.5).abs() < 0.04, "after four evenings: {got}");
    }
}
