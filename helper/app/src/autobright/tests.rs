use super::*;
use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Default)]
struct State {
    mode: String,
    range: String,
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
                let mut settings = json!({"backlight": backlight.to_string()});
                if let Some(e) = energy {
                    settings["energySaving"] = json!(e);
                }
                Ok(json!({"returnValue": true, "settings": settings, "dimension": {"pictureMode": mode, "input": "default", "dynamicRange": s.range}}))
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

const ON: Env = Env { on: true, screen: true, game: false, panel_dim: false, minute: None, context: String::new() };

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
fn the_room_light_moves_only_past_the_bands_and_after_the_debounce() {
    let t0 = Instant::now();
    let mut a = Ambient::default();
    let s = |i: u64| t0 + Duration::from_secs(2 * i);
    // a dark room's reading jumps between 1 and 3: it never moves the accepted light
    for i in 0..60 {
        a.push(s(i), [1.0, 2.0, 3.0, 1.0][i as usize % 4]);
        a.update(s(i));
    }
    let dark = a.lux().unwrap();
    assert!(dark < 3.0, "{dark}");
    // a lamp on (6 to 8 lux): not before the brightening debounce, then the short mean
    let on = 60;
    for i in on..on + 4 {
        a.push(s(i), 7.0);
        assert!(!a.update(s(i)), "moved {} s after the lamp", (i - on) * 2);
    }
    for i in on + 4..on + 8 {
        a.push(s(i), 7.0);
        a.update(s(i));
    }
    let lit = a.lux().unwrap();
    assert!((6.0..7.5).contains(&lit), "{lit}");
    // the lamp off: the darkening debounce is the longer one
    let off = on + 8;
    for i in off..off + 14 {
        a.push(s(i), 1.0);
        a.update(s(i));
        assert_eq!(a.lux(), Some(lit), "darkened {} s after the lamp went off", (i - off) * 2);
    }
    for i in off + 14..off + 18 {
        a.push(s(i), 1.0);
        a.update(s(i));
    }
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
fn taking_over_journals_first_goes_to_the_target_turns_lg_auto_off_then_ramps_a_unit_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(STATE);
    let tv = Tv::new("filmMaker", 75, Some("auto"));
    tv.0.lock().lux = 7.0;
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), Some(path.clone()), t0);
    let now = run(&mut e, &ON, t0, 1);
    let first = Curve::default().target(&inputs(7.0), 75.0).round() as u32;
    assert_eq!((tv.value("filmMaker"), tv.energy("filmMaker").as_deref()), (first, Some("off")));
    let j = load(&path);
    assert_eq!(j.modes["default/filmMaker"].own, 75);
    assert_eq!(j.modes["default/filmMaker"].energy.as_deref(), Some("auto"));

    // the room goes dark: after the debounce, down one unit a write, no faster than the rate, and never under the floor
    tv.0.lock().lux = 0.0;
    let end = run(&mut e, &ON, now, 240);
    let writes = tv.backlight_writes();
    let ramp = &writes[1..];
    assert!(ramp.windows(2).all(|w| w[0] == w[1] + 1), "{ramp:?}");
    assert_eq!(tv.value("filmMaker"), 38, "the floor is half the level");
    assert_eq!((gap(50, false), gap(50, true)), (Duration::from_secs(4), MIN_GAP));
    assert_eq!(e.status()["active"], true);

    // switched off: the owner's level and LG's Energy saving Auto come back, and the journal holds no mode
    run(&mut e, &Env { on: false, ..ON }, end, 1);
    assert_eq!((tv.value("filmMaker"), tv.energy("filmMaker").as_deref()), (75, Some("auto")));
    assert!(load(&path).modes.is_empty());
}

#[test]
fn a_hand_set_value_is_kept_and_learned_and_never_raises_the_level_above_it() {
    let tv = Tv::new("filmMaker", 80, None);
    tv.0.lock().lux = 7.0;
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), None, t0);
    let now = run(&mut e, &ON, t0, 30);
    let writes = tv.backlight_writes().len();
    // the owner moves the slider to 50 in the same light: nothing is written over it
    tv.0.lock().values.get_mut("filmMaker").unwrap().0 = 50;
    let now = run(&mut e, &ON, now, 60);
    assert_eq!((tv.value("filmMaker"), tv.backlight_writes().len()), (50, writes));
    assert_eq!(e.status()["ceiling"], 80);
    // and to 95: that is his level now, and a lit room stays at it
    tv.0.lock().values.get_mut("filmMaker").unwrap().0 = 95;
    let now = run(&mut e, &ON, now, 15);
    tv.0.lock().lux = 500.0;
    run(&mut e, &ON, now, 60);
    assert_eq!((tv.value("filmMaker"), e.status()["ceiling"].as_u64()), (95, Some(95)));
}

#[test]
fn the_slider_through_the_helper_is_written_and_learned_at_once() {
    let tv = Tv::new("filmMaker", 80, None);
    tv.0.lock().lux = 7.0;
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), None, t0);
    let now = run(&mut e, &ON, t0, 30);
    let status = e.by_hand(now, 30, &ON).unwrap();
    assert_eq!((tv.value("filmMaker"), status["level"].as_u64(), status["ceiling"].as_u64()), (30, Some(30), Some(80)));
    let writes = tv.backlight_writes().len();
    run(&mut e, &ON, now, 60);
    assert_eq!((tv.value("filmMaker"), tv.backlight_writes().len()), (30, writes), "the learned curve gives it in this light");
}

#[test]
fn nothing_is_written_in_hdr_a_game_or_under_the_panel_dim_and_a_mode_coming_back_goes_to_its_target_at_once() {
    let tv = Tv::new("filmMaker", 80, None);
    tv.0.lock().values.insert("hdrFilmMaker".into(), (100, None));
    tv.0.lock().lux = 0.0;
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), None, t0);
    let mut now = run(&mut e, &ON, t0, 5);
    let n = tv.backlight_writes().len();
    for env in [Env { game: true, ..ON }, Env { panel_dim: true, ..ON }] {
        tv.0.lock().lux = 50.0;
        now = run(&mut e, &env, now, 60);
        assert_eq!(tv.backlight_writes().len(), n);
    }
    {
        let mut s = tv.0.lock();
        s.mode = "hdrFilmMaker".into();
        s.range = "hdr".into();
    }
    let now = run(&mut e, &Env { context: "hdr".into(), ..ON }, now, 60);
    assert_eq!((tv.backlight_writes().len(), tv.value("hdrFilmMaker")), (n, 100));
    assert_eq!(e.status()["hold"], "hdr");
    {
        let mut s = tv.0.lock();
        s.mode = "filmMaker".into();
        s.range = "sdr".into();
    }
    // the picture module hears of the mode change: it is looked at at once
    run(&mut e, &Env { context: "sdr".into(), ..ON }, now, 1);
    assert_eq!((tv.backlight_writes().len(), tv.value("filmMaker")), (n + 1, 80), "one write to the lit room's level");
}

#[test]
fn a_failing_sensor_puts_everything_back_and_a_restart_follows_the_switch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(STATE);
    let tv = Tv::new("eco", 77, Some("auto"));
    let t0 = Instant::now();
    let mut e = engine(&tv, Journal::default(), Some(path.clone()), t0);
    let now = run(&mut e, &ON, t0, 2);
    assert_eq!(tv.energy("eco").as_deref(), Some("off"));
    tv.0.lock().sensor_down = true;
    let now = run(&mut e, &ON, now, 10);
    assert_eq!((tv.value("eco"), tv.energy("eco").as_deref()), (77, Some("auto")));
    assert_eq!(e.status()["hold"], "error");

    // a crash with the switch on: the next start carries on from the journal, without putting back first
    tv.0.lock().sensor_down = false;
    let mut e = engine(&tv, Journal::default(), Some(path.clone()), now);
    run(&mut e, &ON, now, 2);
    let journal = load(&path);
    let writes = tv.0.lock().writes.len();
    let mut e = engine(&tv, journal.clone(), Some(path.clone()), now);
    run(&mut e, &ON, now, 2);
    assert_eq!(tv.0.lock().writes.len(), writes);
    // with the switch off it puts back; a value someone changed since stays theirs
    tv.0.lock().values.get_mut("eco").unwrap().0 = 60;
    let mut e = engine(&tv, journal, Some(path.clone()), now);
    run(&mut e, &Env { on: false, ..ON }, now, 1);
    assert_eq!((tv.value("eco"), tv.energy("eco").as_deref()), (60, Some("auto")));
    assert!(load(&path).modes.is_empty());
}
