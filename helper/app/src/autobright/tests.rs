use super::*;
use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Default)]
struct State {
    mode: String,
    range: String,
    omit_mixed_range: bool,
    /// Per mode: backlight and Energy saving.
    values: BTreeMap<String, (u32, Option<String>)>,
    lux: f64,
    sensor_down: bool,
    write_refused: bool,
    writes: Vec<(String, Value)>,
}

#[derive(Default, Clone)]
struct Tv(Arc<Mutex<State>>);

impl Tv {
    fn new(mode: &str, backlight: u32, energy: Option<&str>) -> Tv {
        let tv = Tv::default();
        {
            let mut s = tv.0.lock();
            s.mode = mode.into();
            s.range = "sdr".into();
            s.values.insert(mode.into(), (backlight, energy.map(str::to_owned)));
        }
        tv
    }
    fn value(&self, mode: &str) -> u32 {
        self.0.lock().values[mode].0
    }
    fn energy(&self, mode: &str) -> Option<String> {
        self.0.lock().values[mode].1.clone()
    }
    fn backlight_writes(&self) -> Vec<u32> {
        self.0.lock().writes.iter().filter_map(|(_, s)| s["backlight"].as_str().map(|v| v.parse().unwrap())).collect()
    }
}

impl Settings for Tv {
    fn call(&self, uri: &str, payload: &Value) -> Result<Value, String> {
        let mut s = self.0.lock();
        match uri {
            SENSOR_URI if s.sensor_down => Err("luna-send did not answer".into()),
            SENSOR_URI => Ok(json!({"returnValue": true, "eyeQLux": s.lux})),
            READ_URI => {
                let mode = payload.pointer("/dimension/pictureMode").and_then(Value::as_str).map_or_else(|| s.mode.clone(), str::to_owned);
                let (backlight, energy) = s.values[&mode].clone();
                let mut settings = json!({"pictureMode": mode, "backlight": backlight.to_string()});
                if let Some(e) = energy {
                    settings["energySaving"] = json!(e);
                }
                let mut dimension = json!({"pictureMode": mode, "input": "default"});
                if !s.omit_mixed_range || payload["keys"] == json!(["pictureMode"]) {
                    dimension["dynamicRange"] = json!(s.range);
                }
                Ok(json!({"returnValue": true, "settings": settings, "dimension": dimension}))
            }
            WRITE_URI if s.write_refused => Ok(json!({"returnValue": false})),
            WRITE_URI => {
                let mode = payload["dimension"]["pictureMode"].as_str().unwrap().to_owned();
                let settings = payload["settings"].clone();
                let entry = s.values.get_mut(&mode).unwrap();
                if let Some(v) = settings["backlight"].as_str() {
                    entry.0 = v.parse().unwrap();
                }
                if let Some(e) = settings["energySaving"].as_str() {
                    entry.1 = Some(e.into());
                }
                s.writes.push((mode, settings));
                Ok(json!({"returnValue": true}))
            }
            _ => Err(format!("unexpected {uri}")),
        }
    }
}

const NITIN: &str = "nitin";
const ISABEL: &str = "isabel";

fn on() -> Env {
    Env { on: true, screen: true, epoch: T0, profile: Some((NITIN.into(), 0)), ..Env::default() }
}

fn learned_file(data: &Path, id: &str) -> PathBuf {
    data.join("profiles").join(id).join(learn::FILE)
}

fn profile_env(id: &str, switched: u64) -> Env {
    Env { profile: Some((id.into(), switched)), ..on() }
}

fn engine(tv: &Tv, journal: Journal, path: Option<PathBuf>, t0: Instant) -> Engine {
    if let Some(dir) = path.as_deref().and_then(Path::parent) {
        std::fs::create_dir_all(dir.join("profiles").join(NITIN)).unwrap();
        std::fs::create_dir_all(dir.join("profiles").join(ISABEL)).unwrap();
    }
    Engine::new(Box::new(tv.clone()), journal, path, t0)
}

/// Ticks every 500 ms from `from` for `secs`.
fn run(e: &mut Engine, env: &Env, from: Instant, secs: u64) -> Instant {
    let mut now = from;
    for _ in 0..secs * 2 {
        e.tick(now, env);
        now += Duration::from_millis(500);
    }
    now
}

const DAY: u64 = 86_400;
const T0: u64 = 1_800_000_000;
const ECO: &str = "default/eco";
const MARQUEE: &str = "com.beb.plxnative";

/// The default curve's level, before anything is learned.
fn prior_target(lux: f64, ceiling: f64) -> f64 {
    learn::prior(reach(lux)) * ceiling
}

/// The lux whose [`reach`] is `r`.
fn lux_at(r: f64) -> f64 {
    BRIGHT_LUX.powf(r)
}

fn level(l: &Learner, key: &str, lux: f64, epoch: u64) -> f64 {
    l.level(key, &Scene::default(), lux, epoch)
}

fn level_in(l: &Learner, scene: &Scene, lux: f64, epoch: u64) -> f64 {
    l.level(ECO, scene, lux, epoch)
}

fn scene(app: &str, playing: bool) -> Scene {
    Scene { app: app.into(), playing, game: false }
}

#[test]
fn the_room_light_drifts_past_the_bands_and_after_the_debounce_and_a_big_lasting_change_is_sudden() {
    let t0 = Instant::now();
    let mut a = Ambient::default();
    let s = |i: u64| t0 + Duration::from_secs(2 * i);
    // a dark room's reading jumps between 1 and 3 (and the screen's own light gives 4 to 5): it never moves the accepted light
    for i in 0..60 {
        a.push(s(i), [1.0, 2.0, 3.0, 1.0, 4.0, 1.0][i as usize % 6]);
        a.update(s(i));
    }
    let dark = a.lux().unwrap();
    assert!(dark < 3.0, "{dark}");
    // one reading of a lamp (a hand or a person at the sensor) is nothing; a second one in a row is a sudden shift
    a.push(s(60), 7.0);
    assert_eq!(a.update(s(60)), Move::Still);
    a.push(s(61), 7.0);
    assert_eq!(a.update(s(61)), Move::Sudden);
    let lit = a.lux().unwrap();
    assert!((6.9..7.1).contains(&lit), "{lit}");
    // a change inside the bands (7 to 8 and back) never moves it
    for i in 62..122 {
        a.push(s(i), [7.0, 8.0][i as usize % 2]);
        assert_eq!(a.update(s(i)), Move::Still);
    }
    // a lasting rise past the band but not a jump (7 to 10 lux) is a drift, after the brightening debounce and not before
    let rise = 122;
    for i in rise..rise + 4 {
        a.push(s(i), 10.0);
        assert_eq!(a.update(s(i)), Move::Still, "moved {} s after the rise", (i - rise) * 2);
    }
    for i in rise + 4..rise + 10 {
        a.push(s(i), 10.0);
        a.update(s(i));
    }
    let drifted = a.lux().unwrap();
    assert!((9.0..10.5).contains(&drifted), "{drifted}");
    // the lamp off: two readings are not enough, the third is a sudden shift down
    let off = rise + 10;
    for i in off..off + 2 {
        a.push(s(i), 1.0);
        assert_eq!(a.update(s(i)), Move::Still);
    }
    a.push(s(off + 2), 1.0);
    assert_eq!(a.update(s(off + 2)), Move::Sudden);
    assert!(a.lux().unwrap() < 2.0);
}

#[test]
fn the_default_curve_runs_from_half_the_ceiling_to_the_ceiling() {
    let l = Learner::default();
    let s = Scene::default();
    let at = |lux: f64| l.target(&Inputs { lux, mode: ECO, scene: &s, minute: None, epoch: T0 }, 80.0);
    assert_eq!((at(0.0), at(DARK_LUX), at(BRIGHT_LUX), at(BRIGHT_LUX * 10.0)), (40.0, 40.0, 80.0, 80.0));
    assert!((at(7.0) - prior_target(7.0, 80.0)).abs() < 1e-9);
}

#[test]
fn a_correction_moves_the_curve_near_its_light_and_barely_the_far_part() {
    // the owner's 20 of a level of 75 at 1 lux, where the default gives 37.5
    let mut l = Learner::default();
    let s = Scene::default();
    l.add(ECO, &s, 1.0, 20.0 / 75.0, None, T0);
    let at = |lux: f64| l.target(&Inputs { lux, mode: ECO, scene: &s, minute: None, epoch: T0 }, 75.0);
    assert!((at(1.0) - 20.0).abs() < 1.0, "met where it was made: {}", at(1.0));
    let step = |lux: f64| prior_target(lux, 75.0) - at(lux);
    assert!(step(1.0) > 17.0 && step(2.0) > 5.0, "still lower a lux or two up: {} {}", step(1.0), step(2.0));
    assert!(step(6.0) < 3.0 && step(12.0) < 1.5 && step(BRIGHT_LUX) < 1e-3, "the lit part is left alone: {} {}", step(6.0), step(12.0));
    let mut last = 0.0;
    for i in 0..400 {
        let v = at(f64::from(i) * 0.1);
        assert!(v >= last - 1e-9 && v <= 75.0, "non-decreasing and under the ceiling at {i}: {last} then {v}");
        last = v;
    }
}

#[test]
fn consistent_corrections_across_days_converge_on_the_owners_curve_and_beat_the_default() {
    // what the owner wants: 20% of the ceiling in the dark, rising to all of it
    let wanted = |r: f64| 0.2 + 0.8 * r;
    let mut l = Learner::default();
    for day in 0..8_u64 {
        for (i, r) in [0.0, 0.25, 0.5, 0.75].into_iter().enumerate() {
            let noise = if (day + i as u64) % 2 == 0 { 0.01 } else { -0.01 };
            l.add(ECO, &Scene::default(), lux_at(r), wanted(r) + noise, Some(21 * 60), T0 + day * DAY);
        }
    }
    let now = T0 + 8 * DAY;
    for r in [0.0, 0.125, 0.375, 0.5, 0.625, 0.875] {
        let got = level(&l, ECO, lux_at(r), now);
        assert!((got - wanted(r)).abs() < 0.04, "r {r}: wanted {} got {got}", wanted(r));
        assert!((learn::prior(r) - wanted(r)).abs() > 0.1 || r > 0.8, "the default was far from it at r {r}");
    }
    assert_eq!(l.adjustments(), 32);
}

#[test]
fn a_newer_correction_replaces_an_older_nearby_one_it_contradicts_and_agreeing_ones_add_up() {
    let s = Scene::default();
    let mut l = Learner::default();
    l.add(ECO, &s, 5.0, 0.30, None, T0);
    l.add(ECO, &s, 5.0, 0.65, None, T0 + DAY);
    assert!((level(&l, ECO, 5.0, T0 + DAY) - 0.65).abs() < 0.02, "not averaged: {}", level(&l, ECO, 5.0, T0 + DAY));
    // a third wish, again different: the second is replaced as well
    l.add(ECO, &s, 5.3, 0.45, None, T0 + 2 * DAY);
    assert!((level(&l, ECO, 5.3, T0 + 2 * DAY) - 0.45).abs() < 0.02);
    // far from it, an older correction is not contradicted
    let mut l = Learner::default();
    l.add(ECO, &s, lux_at(0.9), 0.95, None, T0);
    l.add(ECO, &s, 1.0, 0.2, None, T0 + DAY);
    assert!((level(&l, ECO, lux_at(0.9), T0 + DAY) - 0.95).abs() < 0.02);
    // agreeing corrections do not cancel each other
    let mut l = Learner::default();
    l.add(ECO, &s, 5.0, 0.30, None, T0);
    l.add(ECO, &s, 5.0, 0.31, None, T0 + DAY);
    assert!((level(&l, ECO, 5.0, T0 + DAY) - 0.305).abs() < 0.01);
    // another app's wish at the same light is not a contradiction: both are kept
    let mut l = Learner::default();
    l.add(ECO, &scene("home", false), 5.0, 0.8, None, T0);
    l.add(ECO, &scene(MARQUEE, false), 5.0, 0.4, None, T0 + DAY);
    assert_eq!((l.held_in(ECO, &scene("home", false)), l.held_in(ECO, &scene(MARQUEE, false))), (1, 1));
    assert!(level_in(&l, &scene("home", false), 5.0, T0 + DAY) > level_in(&l, &scene(MARQUEE, false), 5.0, T0 + DAY) + 0.1);
}

#[test]
fn corrections_fade_with_a_half_life_of_two_weeks_back_to_the_default() {
    let mut l = Learner::default();
    l.add(ECO, &Scene::default(), 1.0, 0.2, None, T0);
    let gap = |days: u64| learn::prior(0.0) - level(&l, ECO, 1.0, T0 + days * DAY);
    let full = gap(0);
    assert!((full - 0.3).abs() < 0.01, "{full}");
    assert!(gap(14) < full && gap(56) < gap(14) && gap(112) < 0.5 * full && gap(365) < 0.05 * full, "{} {} {} {}", gap(14), gap(56), gap(112), gap(365));
    // an old correction is dropped when the next one is added, and never counted again
    l.add(ECO, &Scene::default(), lux_at(0.9), 0.95, None, T0 + 200 * DAY);
    assert_eq!(l.held(ECO), 1);
}

#[test]
fn at_most_two_hundred_points_a_mode_are_kept_and_the_least_weighty_go_first() {
    let mut l = Learner::default();
    for i in 0..260_u64 {
        let r = (i % 9) as f64 / 8.0;
        l.add(ECO, &Scene::default(), lux_at(r), (0.3 + 0.7 * r).min(1.0), None, T0 + i * 3_600);
    }
    assert_eq!(l.held(ECO), learn::MAX_POINTS);
    assert_eq!(l.adjustments(), 260);
}

#[test]
fn a_mode_with_no_points_follows_the_others_and_one_with_its_own_follows_those() {
    let s = Scene::default();
    let mut l = Learner::default();
    l.add("default/cinema", &s, 1.0, 0.2, None, T0);
    let other = level(&l, ECO, 1.0, T0);
    assert!(other < learn::prior(0.0) - 0.15 && other > 0.2, "shared, not as strong as its own: {other}");
    for _ in 0..3 {
        l.add(ECO, &s, 1.0, 0.45, None, T0);
    }
    assert!((level(&l, ECO, 1.0, T0) - 0.45).abs() < 0.02);
    assert!((level(&l, "default/cinema", 1.0, T0) - 0.2).abs() < 0.08, "its own point still leads: {}", level(&l, "default/cinema", 1.0, T0));
}

#[test]
fn home_and_an_app_learn_different_levels_at_the_same_light_and_playing_differs_from_browsing() {
    let (home, marquee, film) = (scene("home", false), scene(MARQUEE, false), scene(MARQUEE, true));
    let mut l = Learner::default();
    for day in 0..4 {
        l.add(ECO, &home, 5.0, 0.8, None, T0 + day * DAY);
        l.add(ECO, &marquee, 5.0, 0.55, None, T0 + day * DAY);
        l.add(ECO, &film, 5.0, 0.3, None, T0 + day * DAY);
    }
    let now = T0 + 4 * DAY;
    let at = |s: &Scene| level_in(&l, s, 5.0, now);
    assert!(at(&home) > 0.7 && at(&home) > at(&marquee) + 0.1, "home {} marquee {}", at(&home), at(&marquee));
    assert!(at(&marquee) > at(&film) + 0.15 && at(&film) < 0.4, "browsing {} playing {}", at(&marquee), at(&film));
    // an app never seen follows the others of its kind: browsing between home's and the app's, playing like the app's film
    let new = |playing| level_in(&l, &scene("com.netflix", playing), 5.0, now);
    assert!(new(false) > at(&marquee) && new(false) < at(&home), "{}", new(false));
    assert!((new(true) - at(&film)).abs() < 0.1, "{}", new(true));
}

#[test]
fn a_new_app_with_one_point_is_pulled_only_partway_and_a_few_consistent_ones_make_it_its_own() {
    let (home, marquee) = (scene("home", false), scene(MARQUEE, false));
    let mut l = Learner::default();
    for day in 0..5 {
        l.add(ECO, &home, 5.0, 0.8, None, T0 + day * DAY);
    }
    let shared = level_in(&l, &marquee, 5.0, T0 + 5 * DAY);
    l.add(ECO, &marquee, 5.0, 0.4, None, T0 + 5 * DAY);
    let one = level_in(&l, &marquee, 5.0, T0 + 5 * DAY);
    assert!(one < shared - 0.1 && one > 0.5, "partway from {shared}: {one}");
    assert!(level_in(&l, &home, 5.0, T0 + 5 * DAY) > 0.72, "home hardly moves for one point elsewhere");
    for day in 6..10 {
        l.add(ECO, &marquee, 5.0, 0.4, None, T0 + day * DAY);
    }
    let five = level_in(&l, &marquee, 5.0, T0 + 9 * DAY);
    assert!(five < one - 0.05 && five < 0.5, "its own now: {five}");
    assert!(level_in(&l, &home, 5.0, T0 + 9 * DAY) > 0.7);
}

#[test]
fn corrections_and_their_contexts_survive_a_restart_and_a_damaged_file_is_the_default_curve() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(learn::FILE);
    let (home, film) = (scene("home", false), scene(MARQUEE, true));
    let mut l = Learner::load(&path);
    l.add(ECO, &home, 1.0, 0.2, Some(60), T0);
    l.add(ECO, &film, 1.0, 0.5, Some(61), T0);
    assert!(!path.with_extension("json.tmp").exists());
    let back = Learner::load(&path);
    assert_eq!((back.adjustments(), back.held(ECO), back.apps()), (2, 2, 2));
    for s in [&home, &film] {
        assert!((level_in(&back, s, 1.0, T0) - level_in(&l, s, 1.0, T0)).abs() < 1e-12);
    }
    assert!(level_in(&back, &film, 1.0, T0) > level_in(&back, &home, 1.0, T0) + 0.1);

    std::fs::write(&path, b"{\"modes\": {\"default/eco\": [").unwrap();
    let damaged = Learner::load(&path);
    assert_eq!((damaged.adjustments(), damaged.held(ECO)), (0, 0));
    assert_eq!(level(&damaged, ECO, 1.0, T0), learn::prior(0.0));
    // one unusable point is dropped and the rest kept
    std::fs::write(
        &path,
        br#"{"adjustments": 2, "modes": {"default/eco": [{"lux": 1.0, "level": 7.0, "at": 1, "weight": 50.0}, {"lux": 1.0, "level": 0.2, "at": 1800000000, "weight": 50.0}]}}"#,
    )
    .unwrap();
    assert_eq!(Learner::load(&path).held(ECO), 1);
    // an engine over a damaged file starts from the default curve
    let profile = learned_file(dir.path(), NITIN);
    std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
    std::fs::write(profile, b"\0\0 not json").unwrap();
    let tv = Tv::new("eco", 75, None);
    tv.0.lock().lux = 1.0;
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), Some(dir.path().join(STATE)), t0);
    run(&mut e, &on(), t0, 1);
    assert_eq!(e.status()["learned"], 0);
}

#[test]
fn a_run_of_hand_changes_is_one_correction_taken_once_it_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let tv = Tv::new("eco", 75, None);
    tv.0.lock().lux = 1.0;
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), Some(dir.path().join(STATE)), t0);
    let mut now = run(&mut e, &on(), t0, 400);
    assert_eq!(tv.value("eco"), 38, "the default curve's floor");
    // 20, 25, 30, 25, 20 within 19 s, as on the TV
    for v in [20, 25, 30, 25, 20] {
        e.by_hand(now, v, &on()).unwrap();
        now = run(&mut e, &on(), now, 4);
        assert_eq!((tv.value("eco"), e.status()["learned"].as_u64()), (v, Some(0)), "not a correction yet at {v}");
    }
    let writes = tv.backlight_writes().len();
    now = run(&mut e, &on(), now, 2);
    assert_eq!(e.status()["learned"], 1, "one point for the run");
    assert_eq!(e.status()["target"], 20);
    run(&mut e, &on(), now, 120);
    assert_eq!((tv.value("eco"), tv.backlight_writes().len()), (20, writes), "nothing pulls it back toward the old automatic level");
    let saved = Learner::load(&learned_file(dir.path(), NITIN));
    assert_eq!((saved.adjustments(), saved.held(ECO)), (1, 1));
    assert!((level(&saved, ECO, 1.0, T0) - 20.0 / 75.0).abs() < 0.015);
}

#[test]
fn a_deliberate_raise_lifts_only_the_profile_limit_and_a_lit_room_can_learn_a_lower_preference() {
    let tv = Tv::new("eco", 80, None);
    tv.0.lock().lux = 7.0;
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), None, t0);
    let mut now = run(&mut e, &on(), t0, 30);
    e.by_hand(now, 95, &on()).unwrap();
    assert_eq!(e.status()["ceiling"], 95, "raised at once");
    now = run(&mut e, &on(), now, 20);
    for lux in [0.0, 3.0, 7.0, 25.0, 500.0] {
        tv.0.lock().lux = lux;
        now = run(&mut e, &on(), now, 400);
        assert!(tv.backlight_writes().iter().all(|v| *v <= 95), "{lux}");
        assert!(tv.value("eco") <= 95);
    }
    // A lit-room correction belongs to the curve, not to the shared LG baseline.
    e.by_hand(now, 60, &on()).unwrap();
    now = run(&mut e, &on(), now, 20);
    assert_eq!((e.status()["ceiling"].as_u64(), tv.value("eco")), (Some(95), 60));
    assert_eq!(e.status()["learned"], 2);
    e.tick(now, &Env { on: false, ..on() });
    assert_eq!(tv.value("eco"), 80);
}

#[test]
fn resetting_forgets_the_points_in_the_file_too() {
    let dir = tempfile::tempdir().unwrap();
    let tv = Tv::new("eco", 75, None);
    tv.0.lock().lux = 1.0;
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), Some(dir.path().join(STATE)), t0);
    let mut now = run(&mut e, &on(), t0, 10);
    e.by_hand(now, 20, &on()).unwrap();
    now = run(&mut e, &on(), now, 8);
    assert_eq!(e.status()["learned"], 1);
    assert_eq!(e.reset_learning(now, on().profile.as_ref())["learned"], 0);
    let saved = Learner::load(&learned_file(dir.path(), NITIN));
    assert_eq!((saved.adjustments(), saved.held(ECO)), (0, 0));
    assert_eq!(level(&saved, ECO, 1.0, T0), learn::prior(0.0));
    // the default curve is followed again, a unit at a time
    run(&mut e, &on(), now, 300);
    assert_eq!(tv.value("eco"), 38);
}

#[test]
fn taking_over_journals_first_ramps_from_the_value_turns_lg_auto_off_then_steps_a_unit_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(STATE);
    let tv = Tv::new("filmMaker", 75, Some("auto"));
    tv.0.lock().lux = 7.0;
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), Some(path.clone()), t0);
    let now = run(&mut e, &on(), t0, 1);
    assert_eq!((tv.value("filmMaker"), tv.energy("filmMaker").as_deref()), (74, Some("off")), "one step in the first pass, no jump");
    let j = load(&path);
    assert_eq!(j.modes["default/filmMaker"].own, 75);
    assert_eq!(j.modes["default/filmMaker"].energy.as_deref(), Some("auto"));

    // it goes to the lamp's level a unit at a time, no faster than the rate
    let now = run(&mut e, &on(), now, 120);
    let target = prior_target(7.0, 75.0).round() as u32;
    assert_eq!(tv.value("filmMaker"), target);
    assert!(tv.backlight_writes().windows(2).all(|w| w[0] == w[1] + 1), "{:?}", tv.backlight_writes());

    // the room goes dark: after the debounce, down a unit a write, never under the floor
    tv.0.lock().lux = 0.0;
    let end = run(&mut e, &on(), now, 400);
    assert_eq!(tv.value("filmMaker"), 38, "the floor is half the level");
    assert_eq!((gap(50, false, None), gap(50, true, None)), (Duration::from_secs(5), Duration::from_millis(2500)));
    assert_eq!(e.status()["active"], true);

    // switched off: the owner's level and LG's Energy saving Auto come back, and the journal holds no mode
    run(&mut e, &Env { on: false, ..on() }, end, 1);
    assert_eq!((tv.value("filmMaker"), tv.energy("filmMaker").as_deref()), (75, Some("auto")));
    assert!(load(&path).modes.is_empty());
}

#[test]
fn a_hand_set_value_is_kept_and_learned_and_never_raises_the_level_above_it() {
    let tv = Tv::new("filmMaker", 80, None);
    tv.0.lock().lux = 7.0;
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), None, t0);
    let now = run(&mut e, &on(), t0, 30);
    let writes = tv.backlight_writes().len();
    // the owner moves the slider to 50 in the same light: nothing is written over it
    tv.0.lock().values.get_mut("filmMaker").unwrap().0 = 50;
    let now = run(&mut e, &on(), now, 60);
    assert_eq!((tv.value("filmMaker"), tv.backlight_writes().len()), (50, writes));
    assert_eq!(e.status()["ceiling"], 80);
    // and to 95: that is his level now, and a lit room stays at it
    tv.0.lock().values.get_mut("filmMaker").unwrap().0 = 95;
    let now = run(&mut e, &on(), now, 15);
    tv.0.lock().lux = 500.0;
    run(&mut e, &on(), now, 60);
    assert_eq!((tv.value("filmMaker"), e.status()["ceiling"].as_u64()), (95, Some(95)));
}

#[test]
fn the_slider_through_the_helper_is_written_and_learned_at_once() {
    let tv = Tv::new("filmMaker", 80, None);
    tv.0.lock().lux = 7.0;
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), None, t0);
    let now = run(&mut e, &on(), t0, 30);
    let status = e.by_hand(now, 30, &on()).unwrap();
    assert_eq!((tv.value("filmMaker"), status["level"].as_u64(), status["ceiling"].as_u64()), (30, Some(30), Some(80)));
    let writes = tv.backlight_writes().len();
    run(&mut e, &on(), now, 60);
    assert_eq!((tv.value("filmMaker"), tv.backlight_writes().len()), (30, writes), "the learned curve gives it in this light");
}

#[test]
fn hdr_and_panel_dim_hold_the_value_and_sdr_coming_back_goes_to_its_target_at_once() {
    let tv = Tv::new("filmMaker", 80, None);
    tv.0.lock().values.insert("hdrFilmMaker".into(), (100, None));
    tv.0.lock().lux = 0.0;
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), None, t0);
    let mut now = run(&mut e, &on(), t0, 5);
    let n = tv.backlight_writes().len();
    tv.0.lock().lux = 50.0;
    now = run(&mut e, &Env { panel_dim: true, ..on() }, now, 60);
    assert_eq!(tv.backlight_writes().len(), n);
    {
        let mut s = tv.0.lock();
        s.mode = "hdrFilmMaker".into();
        s.range = "hdr10".into();
    }
    let now = run(&mut e, &Env { context: json!({"range": "sdr"}), ..on() }, now, 60);
    assert_eq!((tv.backlight_writes().len(), tv.value("hdrFilmMaker")), (n, 100));
    assert_eq!(e.status()["hold"], "hdr");
    {
        let mut s = tv.0.lock();
        s.mode = "filmMaker".into();
        s.range = "sdr".into();
    }
    // the picture module hears of the mode change: it is looked at at once
    run(&mut e, &Env { context: json!({"mode": "filmMaker"}), ..on() }, now, 1);
    assert_eq!((tv.backlight_writes().len(), tv.value("filmMaker")), (n + 1, 80), "one write to the lit room's level");
}

#[test]
fn a_failing_sensor_puts_everything_back_and_a_restart_follows_the_switch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(STATE);
    let tv = Tv::new("eco", 77, Some("auto"));
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), Some(path.clone()), t0);
    let now = run(&mut e, &on(), t0, 2);
    assert_eq!(tv.energy("eco").as_deref(), Some("off"));
    tv.0.lock().sensor_down = true;
    let now = run(&mut e, &on(), now, 10);
    assert_eq!((tv.value("eco"), tv.energy("eco").as_deref()), (77, Some("auto")));
    assert_eq!(e.status()["hold"], "error");

    // a crash with the switch on: the next start carries on from the journal, without putting back first
    tv.0.lock().sensor_down = false;
    let mut e = engine(&tv, Journal::default(), Some(path.clone()), now);
    run(&mut e, &on(), now, 2);
    let journal = load(&path);
    let mut e = engine(&tv, journal.clone(), Some(path.clone()), now);
    run(&mut e, &on(), now, 2);
    assert_eq!((tv.value("eco") < 77, tv.energy("eco").as_deref()), (true, Some("off")), "carried on, not put back");
    // with the switch off it puts back; a value someone changed since stays theirs
    tv.0.lock().values.get_mut("eco").unwrap().0 = 60;
    let mut e = engine(&tv, journal, Some(path.clone()), now);
    run(&mut e, &Env { on: false, ..on() }, now, 1);
    assert_eq!((tv.value("eco"), tv.energy("eco").as_deref()), (60, Some("auto")));
    assert!(load(&path).modes.is_empty());
}

#[test]
fn switching_on_reads_the_light_and_writes_in_the_first_pass_without_waiting_for_the_timers() {
    let tv = Tv::new("eco", 75, Some("auto"));
    tv.0.lock().lux = 7.0;
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), None, t0);
    // off for a while: the idle ticks must not push the first sample or look out
    let t = run(&mut e, &Env { on: false, ..on() }, t0, 10);
    e.tick(t, &on());
    assert_eq!((tv.value("eco"), tv.energy("eco").as_deref(), e.status()["active"].as_bool()), (74, Some("off"), Some(true)), "taken over in the first pass");
}

#[test]
fn unknown_range_blocks_writes_even_if_the_picture_topic_says_sdr() {
    let tv = Tv::new("eco", 75, Some("auto"));
    {
        let mut s = tv.0.lock();
        s.lux = 7.0;
        s.range.clear();
    }
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), None, t0);
    let unknown = Env { context: json!({"range": "sdr"}), ..on() };
    let now = run(&mut e, &unknown, t0, 1);
    assert_eq!((tv.0.lock().writes.len(), e.status()["hold"].as_str()), (0, Some("range")));
    // A fresh LG reply confirms SDR; the picture topic only wakes the read.
    tv.0.lock().range = "sdr".into();
    run(&mut e, &Env { context: json!({"range": "sdr", "mode": "eco"}), ..on() }, now, 1);
    assert_eq!((tv.value("eco"), e.status()["hold"].is_null()), (74, true));
}

/// Ticks every 100 ms (the helper's loop wakes as soon as a step is due) from `from` for `secs` and returns the end and every backlight write as seconds after `from` and the value.
fn run_log(e: &mut Engine, tv: &Tv, env: &Env, from: Instant, secs: u64) -> (Instant, Vec<(f64, u32)>) {
    let mut now = from;
    let mut log = Vec::new();
    let mut seen = tv.backlight_writes().len();
    for _ in 0..secs * 10 {
        e.tick(now, env);
        let writes = tv.backlight_writes();
        log.extend(writes[seen..].iter().map(|v| (now.duration_since(from).as_secs_f64(), *v)));
        seen = writes.len();
        now += Duration::from_millis(100);
    }
    (now, log)
}

/// A dark room, the mode at its floor (half of 80), the helper running: the state the curve is judged from.
fn settled_in_the_dark() -> (Tv, Engine, Instant) {
    let tv = Tv::new("filmMaker", 80, None);
    tv.0.lock().lux = 1.0;
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), None, t0);
    let now = run(&mut e, &on(), t0, 400);
    assert_eq!(tv.value("filmMaker"), 40);
    (tv, e, now)
}

#[test]
fn a_slow_drift_of_the_daylight_moves_one_unit_at_a_time_no_faster_than_the_rate() {
    let (tv, mut e, mut now) = settled_in_the_dark();
    // a window brightening the room from 1 to 25 lux over 40 minutes, the sensor reading whole lux
    let start = now;
    let mut log = Vec::new();
    for minute in 0..40 {
        tv.0.lock().lux = (1.0 + 24.0 * f64::from(minute) / 40.0).round();
        let (end, mut part) = run_log(&mut e, &tv, &on(), now, 60);
        part.iter_mut().for_each(|(t, _)| *t += now.duration_since(start).as_secs_f64());
        log.extend(part);
        now = end;
    }
    assert!(log.len() > 10, "it follows the daylight: {log:?}");
    let mut last = (-100.0, 40);
    for (t, v) in log {
        assert_eq!(v.abs_diff(last.1), 1, "a unit at a time: {v} after {}", last.1);
        assert!(v > last.1, "brighter only");
        let dt = t - last.0;
        assert!(dt + 1e-6 >= gap(v, true, None).as_secs_f64().max(MIN_GAP.as_secs_f64()), "{v} came {dt} s after {}", last.1);
        assert!(1.0 / f64::from(v) / dt <= BRIGHTEN_RATE + 1e-9, "{v}: {} of the level a second", 1.0 / f64::from(v) / dt);
        last = (t, v);
    }
    assert!(tv.value("filmMaker") <= 80, "never above the owner's level");
}

#[test]
fn a_lamp_on_and_off_is_a_confirmed_eased_ramp_in_seconds_and_dimming_is_the_slower() {
    let (tv, mut e, now) = settled_in_the_dark();
    let target = |lux: f64| prior_target(lux, 80.0).round() as u32;
    tv.0.lock().lux = 8.0;
    let (now, up) = run_log(&mut e, &tv, &on(), now, 20);
    let first = up.first().expect("it follows the lamp").0;
    assert!((2.0..=4.5).contains(&first), "confirmed by the second reading, not before: first write {first} s after the lamp");
    assert_eq!(up.last().unwrap().1, target(8.0));
    let done_up = up.last().unwrap().0;
    assert!(done_up <= 12.0, "the ramp is over in {done_up} s");
    let mut last = 40;
    for (_, v) in &up {
        assert_eq!(*v, last + 1, "a unit at a time, no jump");
        last = *v;
    }
    // eased: the gaps are longest at the ends and shortest in the middle
    let gaps: Vec<f64> = up.windows(2).map(|w| w[1].0 - w[0].0).collect();
    let (head, mid) = (gaps[0], gaps[gaps.len() / 2]);
    assert!(head > mid && gaps[gaps.len() - 1] > mid, "{gaps:?}");
    assert!(gaps.iter().all(|g| *g >= FAST_GAP.as_secs_f64() - 1e-9));

    // the lamp off: confirmed by three readings, then slower than the way up, and still smooth
    tv.0.lock().lux = 1.0;
    let (_, down) = run_log(&mut e, &tv, &on(), now, 30);
    let first = down.first().expect("it follows the lamp off").0;
    assert!((4.0..=6.5).contains(&first), "first write {first} s after the lamp went off");
    assert_eq!(down.last().unwrap().1, 40);
    let done_down = down.last().unwrap().0 - first;
    assert!(done_down <= 20.0 && done_down > done_up - first, "down took {done_down} s, up {}", done_up - first);
    assert!(down.windows(2).all(|w| w[0].1 == w[1].1 + 1));
}

#[test]
fn flicker_a_passing_shadow_and_the_screens_own_light_change_nothing() {
    let (tv, mut e, now) = settled_in_the_dark();
    let writes = tv.backlight_writes().len();
    // the reading flutters 1 to 5 (the screen's own light): bands and debounce hold it
    let mut now = now;
    for i in 0..300 {
        tv.0.lock().lux = [1.0, 4.0, 2.0, 5.0, 1.0, 3.0][i % 6];
        now = run(&mut e, &on(), now, 2);
    }
    // one bright reading in a row of dark ones (a person's hand, a flash) and a lamp-like flutter that never holds for two readings
    for i in 0..150 {
        tv.0.lock().lux = if i % 5 == 0 { 20.0 } else { 1.0 };
        now = run(&mut e, &on(), now, 2);
    }
    for i in 0..150 {
        tv.0.lock().lux = if i % 2 == 0 { 9.0 } else { 1.0 };
        now = run(&mut e, &on(), now, 2);
    }
    assert_eq!((tv.backlight_writes().len(), tv.value("filmMaker")), (writes, 40));
    // in a lit room the same holds with no oscillation between the thresholds: 7 and 9 lux around a lamp
    tv.0.lock().lux = 8.0;
    let now = run(&mut e, &on(), now, 120);
    let writes = tv.backlight_writes().len();
    let mut now = now;
    for i in 0..200 {
        tv.0.lock().lux = if i % 2 == 0 { 7.0 } else { 9.0 };
        now = run(&mut e, &on(), now, 2);
    }
    assert_eq!(tv.backlight_writes().len(), writes);
}

fn in_app(app: &str, playing: bool) -> Env {
    Env { scene: scene(app, playing), ..on() }
}

#[test]
fn a_correction_right_after_a_scene_change_is_not_counted_unless_repeated_and_belongs_to_the_scene_it_settled_in() {
    let dir = tempfile::tempdir().unwrap();
    let tv = Tv::new("eco", 75, None);
    tv.0.lock().lux = 1.0;
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), Some(dir.path().join(STATE)), t0);
    let (home, app) = (in_app("home", false), in_app(MARQUEE, false));
    let mut now = run(&mut e, &home, t0, 400);
    let file = learned_file(dir.path(), NITIN);

    // the app comes to the front and 2 s later the slider moves: that was meant for the home screen it left
    now = run(&mut e, &app, now, 2);
    e.by_hand(now, 20, &app).unwrap();
    now = run(&mut e, &app, now, 12);
    assert_eq!((e.status()["learned"].as_u64(), Learner::load(&file).held(ECO)), (Some(0), 0));

    // once the switch is more than 3 s old a change is meant for the app, and is added to its context
    now = run(&mut e, &app, now, 20);
    e.by_hand(now, 22, &app).unwrap();
    now = run(&mut e, &app, now, 8);
    let l = Learner::load(&file);
    assert_eq!((e.status()["learned"].as_u64(), l.held_in(ECO, &scene(MARQUEE, false)), l.held_in(ECO, &scene("home", false))), (Some(1), 1, 0));

    // back at home: a first change, then another 4 s later, is a repeated one and counts
    now = run(&mut e, &home, now, 1);
    e.by_hand(now, 30, &home).unwrap();
    now = run(&mut e, &home, now, 4);
    e.by_hand(now, 26, &home).unwrap();
    run(&mut e, &home, now, 8);
    let l = Learner::load(&file);
    assert_eq!((e.status()["learned"].as_u64(), l.held_in(ECO, &scene("home", false)), e.status()["apps"].as_u64()), (Some(2), 1, Some(2)));
}

#[test]
fn changing_scene_changes_the_target_and_the_value_ramps_there_a_unit_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let tv = Tv::new("eco", 75, None);
    tv.0.lock().lux = 1.0;
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), Some(dir.path().join(STATE)), t0);
    let (home, app) = (in_app("home", false), in_app(MARQUEE, false));
    let mut now = run(&mut e, &home, t0, 400);
    e.by_hand(now, 20, &home).unwrap();
    now = run(&mut e, &home, now, 20);
    // the app has no habits yet: it follows what was learned at home
    now = run(&mut e, &app, now, 30);
    assert_eq!(tv.value("eco"), 20);
    e.by_hand(now, 40, &app).unwrap();
    now = run(&mut e, &app, now, 20);
    assert_eq!((tv.value("eco"), e.status()["learned"].as_u64()), (40, Some(2)), "held as chosen in the app");
    // home again: its own level, between what it was and what the app was given, reached by a unit a write
    let before = tv.backlight_writes().len();
    run(&mut e, &home, now, 200);
    let writes = tv.backlight_writes();
    let got = tv.value("eco");
    assert!((20..35).contains(&got), "{got}");
    assert!(writes[before..].windows(2).all(|w| w[0] == w[1] + 1), "{writes:?}");
}

#[test]
fn switching_profiles_discards_temporary_choices_and_reset_only_forgets_the_selected_profile() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(STATE);
    let global = dir.path().join(learn::FILE);
    Learner::load(&global).add(ECO, &scene("old", false), 1.0, 0.2, None, T0);
    let legacy = std::fs::read(&global).unwrap();
    let tv = Tv::new("eco", 80, None);
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), Some(path.clone()), t0);
    let mut nitin = Env { scene: scene("home", false), ..profile_env(NITIN, 1) };
    let mut isabel = Env { scene: scene(MARQUEE, true), ..profile_env(ISABEL, 3) };
    e.tick(t0, &nitin);
    assert_eq!(e.status()["learned"], 0);
    let mut now = t0 + WATCH;
    e.by_hand(now, 24, &nitin).unwrap();
    now += SETTLE;
    e.tick(now, &nitin);
    assert!(e.held.is_some());

    now += WATCH;
    e.by_hand(now, 30, &nitin).unwrap();
    nitin.profile.as_mut().unwrap().1 = 2;
    now += WATCH;
    e.tick(now, &nitin);
    assert!(e.held.is_none() && e.pending.is_none(), "a switch away and back cannot carry a pending correction");
    assert_eq!(e.status()["learned"], 1);

    now += WATCH;
    e.tick(now, &isabel);
    assert_eq!(e.status()["profile"], ISABEL);
    assert_eq!((e.status()["learned"].as_u64(), e.status()["apps"].as_u64(), e.status()["target"].as_u64()), (Some(0), Some(0), Some(40)));
    now += WATCH;
    e.by_hand(now, 55, &isabel).unwrap();
    now += SETTLE;
    e.tick(now, &isabel);
    assert_eq!(e.status()["learned"], 1);
    nitin.profile.as_mut().unwrap().1 = 4;
    e.tick(now + WATCH, &nitin);
    assert_eq!(e.status()["learned"], 1);
    assert!(e.held.is_none());

    // The reset arrives before the next settings turn has noticed the profile switch.
    isabel.profile.as_mut().unwrap().1 = 5;
    let status = e.reset_learning(now + 2 * WATCH, isabel.profile.as_ref());
    assert_eq!((status["profile"].as_str(), status["learned"].as_u64()), (Some(ISABEL), Some(0)));
    assert_eq!(Learner::load(&learned_file(dir.path(), ISABEL)).adjustments(), 0);
    let mut restarted = engine(&tv, load(&path), Some(path), now + 3 * WATCH);
    restarted.tick(now + 3 * WATCH, &nitin);
    assert_eq!((restarted.status()["learned"].as_u64(), restarted.status()["apps"].as_u64()), (Some(1), Some(1)));
    assert_eq!(std::fs::read(global).unwrap(), legacy);
}

#[test]
fn the_learning_path_rejects_traversal_and_links_to_another_profile() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(STATE);
    let file = learned_file(dir.path(), NITIN);
    let owner = file.parent().unwrap();
    std::fs::create_dir_all(owner).unwrap();
    assert_eq!(profile_file(&path, NITIN), Some(file.clone()));
    assert!(profile_file(&path, "../profiles/nitin").is_none());
    assert!(profile_file(&path, owner.to_str().unwrap()).is_none());
    symlink(owner, learned_file(dir.path(), ISABEL).parent().unwrap()).unwrap();
    assert!(profile_file(&path, ISABEL).is_none());
    let legacy = dir.path().join(learn::FILE);
    std::fs::write(&legacy, b"{}").unwrap();
    symlink(legacy, file).unwrap();
    assert!(profile_file(&path, NITIN).is_none());
}

#[test]
fn a_raise_cannot_change_another_profile_or_revive_after_the_physical_limit_changed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(STATE);
    let tv = Tv::new("eco", 80, None);
    tv.0.lock().lux = 50.0;
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), Some(path.clone()), t0);
    let (nitin, isabel) = (on(), profile_env(ISABEL, 1));
    let off = Env { on: false, ..isabel.clone() };
    e.tick(t0, &nitin);
    e.by_hand(t0 + WATCH, 90, &nitin).unwrap();
    let now = t0 + WATCH + SETTLE;
    e.tick(now, &nitin);
    assert_eq!(e.reset_learning(now, nitin.profile.as_ref())["ceiling"], 90);

    e.tick(now + WATCH, &isabel);
    assert_eq!((e.status()["ceiling"].as_u64(), tv.value("eco")), (Some(80), 80));
    e.tick(now + 2 * WATCH, &off);
    tv.0.lock().values.get_mut("eco").unwrap().0 = 60;
    e.tick(now + 3 * WATCH, &isabel);
    assert_eq!(e.status()["ceiling"], 60);
    e.tick(now + 4 * WATCH, &off);
    tv.0.lock().values.get_mut("eco").unwrap().0 = 80;
    e.tick(now + 5 * WATCH, &isabel);

    let mut restarted = engine(&tv, load(&path), Some(path), now + 6 * WATCH);
    restarted.tick(now + 6 * WATCH, &nitin);
    assert_eq!((restarted.status()["ceiling"].as_u64(), restarted.status()["target"].as_u64()), (Some(80), Some(80)));
}

#[test]
fn the_normal_slider_learns_sdr_game_optimizer_only_after_a_successful_write() {
    let dir = tempfile::tempdir().unwrap();
    let tv = Tv::new("game", 80, None);
    {
        let mut s = tv.0.lock();
        s.lux = 50.0;
        s.range = "sdrALLM".into();
        s.omit_mixed_range = true;
    }
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), Some(dir.path().join(STATE)), t0);
    let mut game = Env { minute: Some(60), scene: scene("game.app", false), ..on() };
    e.tick(t0, &game);
    assert_eq!(e.status()["active"], true, "the live mode identifies SDR game even without a game hint");
    tv.0.lock().write_refused = true;
    assert!(e.by_hand(t0 + WATCH, 54, &game).is_err());
    assert!(e.pending.is_none());
    tv.0.lock().write_refused = false;
    let mut now = t0 + LOOK;
    e.tick(now, &game);
    assert_eq!(e.status()["learned"], 0);
    assert_eq!(e.by_hand(now, 55, &game).unwrap()["level"], 55);
    game.minute = Some(61);
    now += SETTLE;
    e.tick(now, &game);
    let path = learned_file(dir.path(), NITIN);
    let model = Learner::load(&path);
    let scene = Scene { game: true, ..game.scene.clone() };
    assert_eq!((model.adjustments(), model.held_in("default/game", &scene)), (1, 1));
    let file: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(file["modes"]["default/game"][0]["minute"], 60);

    let expires = e.held.as_ref().unwrap().until;
    e.tick(expires, &game);
    assert!(e.held.is_none(), "the clock-aware model must eventually take over in the same scene and light");
    e.by_hand(expires + WATCH, 33, &game).unwrap();
    let writes = tv.backlight_writes().len();
    now = expires + WATCH;
    for (mode, range) in [("game", "hdrALLM"), ("hdrGame", "sdr"), ("dolbyHdrGame", "dolbyHdr")] {
        now += 2 * WATCH;
        {
            let mut s = tv.0.lock();
            s.mode = mode.into();
            s.range = range.into();
            s.values.entry(mode.into()).or_insert((80, None));
        }
        game.context = json!({"mode": mode, "range": "sdrALLM"});
        assert!(e.by_hand(now, 65, &game).is_err(), "a stale active UI must not write after the picture changed");
        e.tick(now, &game);
        assert_eq!(e.status()["hold"], "hdr");
        assert!(e.pending.is_none());
        assert_eq!((tv.backlight_writes().len(), e.status()["learned"].as_u64()), (writes, Some(1)));
    }
}
