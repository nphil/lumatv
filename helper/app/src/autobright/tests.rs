use super::*;
use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Default)]
struct State {
    mode: String,
    /// Per mode: backlight and Energy saving.
    values: BTreeMap<String, (u32, Option<String>)>,
    lux: f64,
    sensor_down: bool,
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
                let mut settings = json!({"backlight": backlight.to_string()});
                if let Some(e) = energy {
                    settings["energySaving"] = json!(e);
                }
                Ok(json!({"returnValue": true, "settings": settings, "dimension": {"pictureMode": mode, "input": "default"}}))
            }
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

/// The switch on, the picture SDR: the picture module's answer is the range, the read of the setting names none.
fn on() -> Env {
    Env { on: true, screen: true, game: false, panel_dim: false, minute: None, context: String::new(), range: "sdr".into() }
}

fn engine(tv: &Tv, journal: Journal, path: Option<PathBuf>, t0: Instant) -> Engine {
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

fn inputs(lux: f64) -> Inputs<'static> {
    Inputs { lux, history: &[], minute: None, content: None }
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
fn the_curve_runs_from_the_floor_to_the_level_and_a_hand_set_value_is_met_exactly() {
    let c = Curve::default();
    assert_eq!(c.target(&inputs(0.0), 80.0), 40.0);
    assert_eq!(c.target(&inputs(BRIGHT_LUX * 10.0), 80.0), 80.0);
    let lamp = c.target(&inputs(7.0), 80.0);
    assert!((40.0..80.0).contains(&lamp), "{lamp}");

    // set by hand in the lamp's light, below the level: met there, the level unchanged
    for chosen in [45.0, 70.0, 20.0] {
        let mut c = Curve::default();
        assert_eq!(c.learn(&inputs(7.0), 80.0, chosen), 80.0);
        assert!((c.target(&inputs(7.0), 80.0) - chosen).abs() < 1e-6, "{chosen}: {c:?}");
        assert_eq!(c.target(&inputs(BRIGHT_LUX), 80.0), 80.0);
        let back = Curve::load(&c.save());
        assert!((back.floor - c.floor).abs() < 1e-9 && (back.gamma - c.gamma).abs() < 1e-9);
    }
    // above the level: that is the new level, and no light goes above it
    let mut c = Curve::default();
    assert_eq!(c.learn(&inputs(7.0), 80.0, 90.0), 90.0);
    assert_eq!(c.target(&inputs(7.0), 90.0), 90.0);
    assert_eq!(c.target(&inputs(1000.0), 90.0), 90.0);
    // in the dark it is the floor
    let mut c = Curve::default();
    assert_eq!(c.learn(&inputs(0.5), 80.0, 30.0), 80.0);
    assert_eq!(c.target(&inputs(0.0), 80.0), 30.0);
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
    let target = Curve::default().target(&inputs(7.0), 75.0).round() as u32;
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
fn nothing_is_written_in_hdr_a_game_or_under_the_panel_dim_and_a_mode_coming_back_goes_to_its_target_at_once() {
    let tv = Tv::new("filmMaker", 80, None);
    tv.0.lock().values.insert("hdrFilmMaker".into(), (100, None));
    tv.0.lock().lux = 0.0;
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), None, t0);
    let mut now = run(&mut e, &on(), t0, 5);
    let n = tv.backlight_writes().len();
    for env in [Env { game: true, ..on() }, Env { panel_dim: true, ..on() }] {
        tv.0.lock().lux = 50.0;
        now = run(&mut e, &env, now, 60);
        assert_eq!(tv.backlight_writes().len(), n);
    }
    {
        let mut s = tv.0.lock();
        s.mode = "hdrFilmMaker".into();
    }
    let now = run(&mut e, &Env { context: "hdr".into(), range: "hdr10".into(), ..on() }, now, 60);
    assert_eq!((tv.backlight_writes().len(), tv.value("hdrFilmMaker")), (n, 100));
    assert_eq!(e.status()["hold"], "hdr");
    {
        let mut s = tv.0.lock();
        s.mode = "filmMaker".into();
    }
    // the picture module hears of the mode change: it is looked at at once
    run(&mut e, &Env { context: "sdr".into(), ..on() }, now, 1);
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
fn the_pictures_range_comes_from_the_picture_module_and_an_unknown_one_waits_without_writing() {
    let tv = Tv::new("eco", 75, Some("auto"));
    tv.0.lock().lux = 7.0;
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), None, t0);
    let unknown = Env { range: String::new(), ..on() };
    let now = run(&mut e, &unknown, t0, 1);
    assert_eq!((tv.0.lock().writes.len(), e.status()["hold"].as_str()), (0, Some("range")));
    // the picture module answers SDR: that is looked at at once, not at the next look
    run(&mut e, &Env { context: "sdr".into(), ..on() }, now, 1);
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
    let target = |lux: f64| Curve::default().target(&inputs(lux), 80.0).round() as u32;
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
