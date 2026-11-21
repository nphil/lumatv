use super::*;

/// The helper's loop on a fake clock: a pass a second, the sensor answering `lux` (by second since the start), every write taken
/// and seen in force at the next pass.
struct Rig {
    engine: Engine,
    t0: Instant,
    t: u64,
    seen: Seen,
    night: bool,
    writes: Vec<(u64, &'static str)>,
}

impl Rig {
    fn new(mode: &str, night: bool) -> Rig {
        let seen = Seen { on: true, dv: true, mode: mode.into(), input: "default".into(), app: "netflix".into(), game: false };
        Rig { engine: Engine::default(), t0: Instant::now(), t: 0, seen, night, writes: Vec::new() }
    }

    fn run(&mut self, secs: u64, lux: impl Fn(u64) -> f64) {
        for _ in 0..secs {
            self.t += 1;
            let now = self.t0 + Duration::from_secs(self.t);
            if self.engine.sample_due(now) {
                self.engine.sample(now, lux(self.t));
            }
            let night = self.night;
            if let Some(mode) = self.engine.step(now, &self.seen, &|| night) {
                self.engine.wrote(now, mode);
                self.writes.push((self.t, mode));
                self.seen.mode = mode.into();
            }
        }
    }

    fn modes(&self) -> Vec<&'static str> {
        self.writes.iter().map(|(_, m)| *m).collect()
    }
}

#[test]
fn a_session_starts_in_the_rooms_cinema_and_between_the_bands_the_time_of_day_decides() {
    for (lux, night, start, want) in [
        (20.0, true, DARK_MODE, Some(BRIGHT_MODE)),
        (1.0, false, BRIGHT_MODE, Some(DARK_MODE)),
        (6.0, false, DARK_MODE, Some(BRIGHT_MODE)),
        (6.0, true, BRIGHT_MODE, Some(DARK_MODE)),
        (6.0, true, DARK_MODE, None),
        (2.0, false, "dolbyHdrVivid", Some(DARK_MODE)),
        (2.0, false, "dolbyHdrStandard", Some(DARK_MODE)),
    ] {
        let mut rig = Rig::new(start, night);
        rig.run(10, |_| lux);
        assert_eq!(rig.modes(), want.into_iter().collect::<Vec<_>>(), "{lux} lux, night {night}, from {start}");
        assert!(rig.writes.iter().all(|(t, _)| *t <= 4), "decided within the opening seconds: {:?}", rig.writes);
    }
}

#[test]
fn game_and_personalized_at_the_start_are_left_for_the_session() {
    for mode in OWNERS {
        let mut rig = Rig::new(mode, false);
        rig.run(20 * 60, |_| 20.0);
        assert!(rig.writes.is_empty(), "{mode}");
    }
}

#[test]
fn a_room_that_hovers_at_a_band_edge_never_flaps() {
    let mut rig = Rig::new(DARK_MODE, true);
    rig.run(10, |_| 2.0);
    // half a minute over the bright edge, half a minute under it, for an hour
    rig.run(60 * 60, |t| if t % 60 < 30 { 12.0 } else { 8.0 });
    assert!(rig.writes.is_empty(), "{:?}", rig.writes);
    // once bright, light anywhere inside the band keeps it bright
    let mut rig = Rig::new(BRIGHT_MODE, false);
    rig.run(60 * 60, |t| [4.0, 9.0, 5.0, 8.0][(t % 4) as usize]);
    assert!(rig.writes.is_empty(), "{:?}", rig.writes);
}

#[test]
fn a_sustained_change_switches_once_after_two_minutes() {
    let mut rig = Rig::new(DARK_MODE, true);
    rig.run(60, |_| 2.0);
    // one blip (a single reading) does nothing
    rig.run(60, |t| if (88..=92).contains(&t) { 40.0 } else { 2.0 });
    assert!(rig.writes.is_empty());
    rig.run(30 * 60, |_| 25.0);
    assert_eq!(rig.modes(), vec![BRIGHT_MODE]);
    let at = rig.writes[0].0;
    assert!((120 + 120..=120 + 120 + 30).contains(&at), "two minutes past the band, plus the window: {at}");
}

#[test]
fn switches_are_at_least_fifteen_minutes_apart() {
    let mut rig = Rig::new("dolbyHdrVivid", false);
    rig.run(10, |_| 2.0);
    rig.run(40 * 60, |t| if t < 20 * 60 { 30.0 } else { 1.0 });
    let times: Vec<u64> = rig.writes.iter().map(|(t, _)| *t).collect();
    assert_eq!(rig.modes(), vec![DARK_MODE, BRIGHT_MODE, DARK_MODE], "{times:?}");
    assert!(times.windows(2).all(|w| w[1] - w[0] >= SPACING.as_secs()), "{times:?}");
    assert!(times[1] - times[0] <= SPACING.as_secs() + 1, "the switch waited only for the spacing: {times:?}");
}

#[test]
fn a_mode_picked_by_hand_holds_until_the_next_session() {
    let mut rig = Rig::new(DARK_MODE, true);
    rig.run(60, |_| 2.0);
    rig.seen.mode = "dolbyHdrVivid".into();
    rig.run(30 * 60, |_| 25.0);
    assert!(rig.writes.is_empty());
    // a seek drops Dolby Vision for 9 s: the same session, still the owner's
    rig.seen.dv = false;
    rig.run(9, |_| 25.0);
    rig.seen.dv = true;
    rig.run(60, |_| 25.0);
    assert!(rig.writes.is_empty());
    // the video ends: the next one is managed again
    rig.seen.dv = false;
    rig.run(11, |_| 25.0);
    rig.seen.dv = true;
    rig.run(10, |_| 25.0);
    assert_eq!(rig.modes(), vec![BRIGHT_MODE]);
}

#[test]
fn another_app_starts_a_new_session() {
    let mut rig = Rig::new(DARK_MODE, true);
    rig.run(10, |_| 2.0);
    rig.seen.mode = "dolbyHdrVivid".into();
    rig.run(10, |_| 2.0);
    rig.seen.app = "plex".into();
    rig.run(10, |_| 2.0);
    assert_eq!(rig.modes(), vec![DARK_MODE]);
}

#[test]
fn a_write_the_tv_does_not_take_is_not_tried_again() {
    let mut rig = Rig::new("dolbyHdrVivid", false);
    rig.run(3, |_| 2.0);
    let now = rig.t0 + Duration::from_secs(4);
    rig.engine.sample(now, 2.0);
    assert_eq!(rig.engine.step(now, &rig.seen, &|| false), Some(DARK_MODE));
    rig.engine.wrote(now, DARK_MODE);
    // the mode never changes: after the echo's time the session lets go
    for s in 5..60 {
        let now = rig.t0 + Duration::from_secs(s);
        assert_eq!(rig.engine.step(now, &rig.seen, &|| false), None);
    }
    assert_eq!(rig.engine.status(true)["session"]["state"], "refused");
}
