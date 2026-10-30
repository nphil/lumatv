//! The panel dim against a stand-in settings service: dim and put back on every way out, a crash left to the next start, someone else's change
//! kept, no write in HDR or under a video, the ramp's pace and the fall back to the GPU dim.

use super::*;

const MS: u64 = 1_000_000;

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("paneldim-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A settings service as LG's answers: the picture setting `backlight` of one dimension, as strings, with the dimension named in each read.
struct Tv {
    value: u32,
    dimension: Value,
    refuse_writes: bool,
    silent: bool,
    /// Every write that reached it: (time, value as text, dimension it carried).
    writes: Vec<(u64, String, Value)>,
    reads: u64,
}

impl Tv {
    fn new(value: u32) -> Tv {
        Tv {
            value,
            dimension: json!({"pictureMode": "filmMaker", "dynamicRange": "sdr", "input": "hdmi2"}),
            refuse_writes: false,
            silent: false,
            writes: Vec::new(),
            reads: 0,
        }
    }

    fn call(&mut self, now: u64, c: &Call) -> Option<Value> {
        if self.silent {
            return None;
        }
        Some(match c.uri {
            READ_URI => {
                self.reads += 1;
                assert_eq!(c.payload["category"], "picture");
                assert_eq!(c.payload["keys"], json!(["backlight"]));
                // the service refuses a read whose dimension names the range (TV round 20)
                if c.payload["dimension"].get("dynamicRange").is_some() {
                    return Some(json!({"returnValue": false, "errorText": "no result from DB"}));
                }
                json!({"returnValue": true, "category": "picture", "settings": {"backlight": self.value.to_string()}, "dimension": self.dimension})
            }
            WRITE_URI => {
                assert_eq!(c.payload["category"], "picture");
                let v = c.payload["settings"]["backlight"].as_str().expect("the service wants strings").to_string();
                self.writes.push((now, v.clone(), c.payload["dimension"].clone()));
                if self.refuse_writes {
                    return Some(json!({"returnValue": false, "errorCode": -1, "errorText": "not now"}));
                }
                // a write to a dimension that is not the current one lands on that dimension's own value, not on this
                if c.payload["dimension"] == call_dimension(&self.dimension) {
                    self.value = v.parse().unwrap();
                }
                json!({"returnValue": true})
            }
            other => panic!("only the settings service is called: {other}"),
        })
    }
}

struct Rig {
    p: PanelDim,
    tv: Tv,
    now: u64,
    look: Look,
    dir: PathBuf,
}

impl Rig {
    fn new(name: &str, value: u32) -> Rig {
        let dir = dir(name);
        Rig { p: PanelDim::new(&dir), tv: Tv::new(value), now: 1000 * SEC, look: Look::default(), dir }
    }

    /// Runs the calls of one decision to the end, the service answering at once (a call's own latency is `latency`).
    fn pump(&mut self, first: Option<Call>) {
        let mut next = first;
        let mut guard = 0;
        while let Some(call) = next.take() {
            guard += 1;
            assert!(guard < 50, "a loop of calls");
            next = match self.tv.call(self.now, &call) {
                Some(answer) => self.p.answer(self.now, &answer),
                None => None,
            };
        }
    }

    /// Ticks of 100 ms for `ms`, with `want` and `block` as they are.
    fn run(&mut self, ms: u64) {
        let end = self.now + ms * MS;
        while self.now < end {
            self.now += 100 * MS;
            self.look = Look { now_ns: self.now, ..self.look };
            let call = self.p.step(self.look);
            self.pump(call);
        }
    }

    fn want(&mut self, want: f32) {
        self.look.want = want;
    }

    fn input(&mut self) {
        let call = self.p.input(self.now);
        self.pump(call);
    }

    fn written(&self) -> Vec<u32> {
        self.tv.writes.iter().map(|w| w.1.parse().unwrap()).collect()
    }

    fn journal(&self) -> bool {
        self.dir.join(JOURNAL).exists()
    }
}

#[test]
fn a_wanted_dim_is_read_once_and_walked_down_a_unit_at_a_time_two_seconds_apart() {
    let mut r = Rig::new("ramp", 80);
    r.run(1000);
    assert_eq!(r.tv.reads, 0, "nothing is asked of the panel while the screen wants nothing");
    r.want(0.85);
    r.run(40_000);
    let w = r.written();
    assert_eq!(w, (68..80).rev().collect::<Vec<_>>(), "one unit a write, down to round(80 x 0.85)");
    for pair in r.tv.writes.windows(2) {
        assert!(pair[1].0 - pair[0].0 >= 2 * SEC, "at most one write in two seconds: {pair:?}");
    }
    assert_eq!(r.tv.value, 68);
    assert!(
        r.tv.writes.iter().all(|w| w.2 == call_dimension(&r.tv.dimension)),
        "each write names the picture mode and input the baseline came with, and no range"
    );
    assert!(r.p.dimmed() && r.p.carries());
    assert!(r.journal(), "the journal stands while it is dimmed");
}

#[test]
fn the_ramp_follows_a_want_that_falls_and_stops_at_the_floor() {
    let mut r = Rig::new("floor", 80);
    r.want(0.9);
    r.run(30_000);
    assert_eq!(r.tv.value, 72);
    r.want(0.5);
    r.run(200_000);
    assert_eq!(r.tv.value, 40);
    r.want(0.01);
    r.run(200_000);
    assert_eq!(r.tv.value, FLOOR, "never below the floor");
    assert_eq!(target(8, 0.1), 8, "a user value under the floor is never lowered");
    assert_eq!(target(80, 1.0), 80);
}

#[test]
fn a_key_puts_the_exact_value_back_after_a_read_and_the_journal_goes() {
    let mut r = Rig::new("key", 73);
    r.want(0.6);
    r.run(30_000);
    assert_eq!(r.tv.value, 58, "a key in the middle of a ramp");
    let (reads, writes) = (r.tv.reads, r.tv.writes.len());
    r.input();
    assert_eq!(r.tv.value, 73, "the user's exact value");
    assert_eq!(r.tv.reads, reads + 1, "read first, always: the value is still ours");
    assert_eq!(r.tv.writes.len(), writes + 1, "one write puts it back, not a ramp");
    assert!(!r.journal() && !r.p.dimmed());
    // the screen still wants dark for a moment (the saver is on its way out): nothing dims again for the hold
    r.run(2_000);
    assert_eq!(r.tv.value, 73);
    r.want(1.0);
    r.run(10_000);
    assert_eq!(r.tv.value, 73);
    let lines = r.p.take_events();
    assert!(lines.iter().any(|l| l.contains("panel back at 73 (key or pointer)")), "{lines:?}");
}

#[test]
fn a_screen_that_wants_to_be_bright_again_puts_the_value_back_whole() {
    let mut r = Rig::new("bright", 80);
    r.want(0.4);
    r.run(120_000);
    assert_eq!(r.tv.value, 32);
    r.want(1.0);
    r.run(500);
    assert_eq!(r.tv.value, 80, "back in one write, however far down it was: an app that opens is not dim for a minute");
    assert!(!r.journal());
}

#[test]
fn a_video_a_standby_the_switch_and_the_exit_each_put_it_back() {
    for (name, how) in [("video", 0), ("standby", 1), ("off", 2), ("exit", 3)] {
        let mut r = Rig::new(name, 80);
        r.want(0.5);
        r.run(100_000);
        assert_eq!(r.tv.value, 40, "{name}");
        let reads = r.tv.reads;
        match how {
            0 => r.look.block = Some(Block::Video),
            1 => r.look.block = Some(Block::Standby),
            2 => r.p.settings.enabled = false,
            _ => {
                let c = r.p.exit(r.now);
                r.pump(c);
            }
        }
        r.run(500);
        assert_eq!(r.tv.value, 80, "{name}");
        assert!(!r.journal() && !r.p.dimmed(), "{name}");
        if how == 1 || how == 3 {
            assert_eq!(r.tv.reads, reads, "{name}: on its way out, no read first");
        }
        r.run(30_000);
        assert_eq!(r.tv.value, 80, "{name}: and it does not dim again while it is not allowed");
        let _ = std::fs::remove_dir_all(&r.dir);
    }
}

#[test]
fn nothing_is_written_under_a_video_in_an_hdr_mode_or_with_the_switch_off() {
    let mut r = Rig::new("excluded", 80);
    r.want(0.5);
    r.look.block = Some(Block::Video);
    r.run(30_000);
    assert_eq!((r.tv.reads, r.tv.writes.len()), (0, 0), "a video: not even a read");
    r.look.block = None;
    r.p.settings.enabled = false;
    r.run(30_000);
    assert_eq!((r.tv.reads, r.tv.writes.len()), (0, 0), "switched off");
    assert!(!r.p.carries(), "and the GPU dim does it");
    r.p.settings.enabled = true;
    for dimension in [
        json!({"pictureMode": "hdrCinema", "dynamicRange": "hdr10", "input": "hdmi2"}),
        json!({"pictureMode": "dolbyHdrCinema", "dynamicRange": "dolbyHdr", "input": "hdmi2"}),
        json!({"pictureMode": "cinema", "dynamicRange": "hlg", "input": "hdmi2"}),
        json!({"pictureMode": "hdrFilmMaker", "input": "hdmi2"}),
    ] {
        r.tv.dimension = dimension;
        r.run(100_000);
        assert!(r.tv.writes.is_empty(), "no write in {:?}", r.tv.dimension);
        assert!(r.p.carries() && !r.p.dimmed(), "no dim at all: the picture is not redrawn either");
    }
    r.tv.dimension = json!({"pictureMode": "filmMaker", "dynamicRange": "sdr", "input": "hdmi2"});
    r.run(40_000);
    assert!(r.p.dimmed(), "back in SDR it dims");
    assert!(hdr_dimension(&json!({"pictureMode": "dolbyHdrStandard"})) && !hdr_dimension(&json!({"pictureMode": "expert1", "dynamicRange": "sdr"})));
}

#[test]
fn an_hdr_mode_that_arrives_under_a_dim_puts_the_value_back_to_the_mode_it_was_taken_from() {
    let mut r = Rig::new("hdrunder", 80);
    let sdr = call_dimension(&r.tv.dimension);
    r.want(0.5);
    r.run(100_000);
    assert_eq!(r.tv.value, 40);
    // the panel goes to Dolby Vision (the poll reads it within 20 s); the value of that mode is its own
    r.tv.dimension = json!({"pictureMode": "dolbyHdrCinema", "dynamicRange": "dolbyHdr", "input": "hdmi2"});
    r.tv.value = 100;
    r.run(25_000);
    let last = r.tv.writes.last().unwrap();
    assert_eq!((last.1.as_str(), &last.2), ("80", &sdr), "the SDR value goes back to the SDR mode, and the HDR mode's value is not touched");
    assert_eq!(r.tv.value, 100);
    assert!(!r.journal());
}

#[test]
fn a_value_changed_by_someone_else_meanwhile_is_left_alone_and_a_poll_finds_it() {
    let mut r = Rig::new("theirs", 80);
    r.want(0.5);
    r.run(100_000);
    assert_eq!(r.tv.value, 40);
    // a phone, a voice command, LG's quick panel: the value is 55 now
    r.tv.value = 55;
    r.input();
    assert_eq!(r.tv.value, 55, "the read before the put back sees a value that is not ours: theirs stays");
    assert!(!r.journal() && !r.p.dimmed());
    let lines = r.p.take_events();
    assert!(lines.iter().any(|l| l.contains("changed by someone else")), "{lines:?}");
    // and by a poll, with no key
    let mut r = Rig::new("theirs-poll", 80);
    r.want(0.5);
    r.run(100_000);
    r.tv.value = 62;
    r.run(25_000);
    assert!(!r.p.dimmed(), "the poll found it");
    assert_eq!(r.tv.value, 62, "and put nothing over it");
    let n = r.tv.writes.len();
    r.run(120_000);
    assert_eq!(r.tv.writes.len(), n, "no dim begins again for minutes: their value stands");
}

#[test]
fn a_run_that_died_leaves_its_journal_and_the_next_start_puts_the_value_back() {
    let mut r = Rig::new("crash", 80);
    r.want(0.5);
    r.run(100_000);
    assert_eq!(r.tv.value, 40);
    assert!(r.journal());
    // the process is gone, the panel stays at 40; a new one starts (another pid)
    let mut next = PanelDim::new(&r.dir);
    next.pid += 1;
    let c = next.recover(r.now);
    assert_eq!(c.as_ref().map(|c| c.uri), Some(READ_URI), "read first");
    r.p = next;
    r.pump(c);
    assert_eq!(r.tv.value, 80);
    assert!(!r.journal());
    assert!(r.p.take_events().iter().any(|l| l.contains("a journal of a run that ended")));
    // recovery names the dimension it dimmed
    assert!(r.tv.writes.last().unwrap().2 == call_dimension(&r.tv.dimension));
}

#[test]
fn a_journal_whose_value_someone_has_changed_since_is_dropped_without_a_write() {
    let mut r = Rig::new("crash-theirs", 80);
    r.want(0.5);
    r.run(100_000);
    r.tv.value = 70;
    let n = r.tv.writes.len();
    let mut next = PanelDim::new(&r.dir);
    next.pid += 1;
    let c = next.recover(r.now);
    r.p = next;
    r.pump(c);
    assert_eq!((r.tv.value, r.tv.writes.len()), (70, n));
    assert!(!r.journal());
    // no journal, nothing to do
    let mut none = PanelDim::new(&r.dir);
    none.pid += 1;
    assert!(none.recover(r.now).is_none());
}

#[test]
fn a_refused_first_write_switches_the_module_off_for_ten_minutes_and_the_gpu_dim_takes_over() {
    let mut r = Rig::new("refused", 80);
    r.tv.refuse_writes = true;
    r.want(0.5);
    r.run(3_000);
    assert_eq!(r.tv.writes.len(), 1);
    assert!(!r.p.carries(), "the GPU dim does it");
    assert!(!r.p.dimmed() && !r.journal() || r.tv.value == 80);
    r.run(9 * 60_000);
    assert_eq!(r.tv.writes.len(), 1, "no new try for ten minutes");
    r.tv.refuse_writes = false;
    r.run(2 * 60_000);
    assert!(r.p.carries(), "after the wait it tries again");
    assert!(r.tv.value < 80);
}

#[test]
fn an_unreadable_baseline_does_the_same_and_so_does_an_answer_that_names_no_picture_mode() {
    let mut r = Rig::new("silent", 80);
    r.tv.silent = true;
    r.want(0.5);
    r.run(5_000);
    assert!(!r.p.carries(), "no answer in three seconds: off");
    let mut p = PanelDim::new(&dir("nodim"));
    let c = p.step(Look { now_ns: 5 * SEC, want: 0.5, block: None }).unwrap();
    assert_eq!(c.uri, READ_URI);
    assert!(p.answer(5 * SEC, &json!({"returnValue": true, "settings": {"backlight": "80"}})).is_none());
    assert!(!p.carries(), "a write that cannot name its picture mode is not made");
}

#[test]
fn a_put_back_that_is_refused_is_tried_again_with_a_longer_wait_and_the_journal_stays() {
    let mut r = Rig::new("retry", 80);
    r.want(0.5);
    r.run(100_000);
    r.tv.refuse_writes = true;
    r.want(1.0);
    r.run(3_000);
    let n = r.tv.writes.len();
    assert!(r.journal(), "the journal stays while the value is not back");
    r.run(100_000);
    let tries = r.tv.writes.len() - n;
    assert!((3..=8).contains(&tries), "a growing wait, not a hammer: {tries}");
    r.tv.refuse_writes = false;
    r.run(100_000);
    assert_eq!(r.tv.value, 80);
    assert!(!r.journal());
}

#[test]
fn a_value_at_or_under_the_floor_is_never_dimmed_and_a_dim_that_rounds_to_nothing_writes_nothing() {
    let mut r = Rig::new("low", FLOOR);
    r.want(0.2);
    r.run(30_000);
    assert!(r.tv.writes.is_empty());
    let mut r = Rig::new("near", 80);
    r.want(0.996);
    r.run(30_000);
    assert!(r.tv.writes.is_empty(), "round(80 x 0.996) is 80");
    assert!(!r.journal());
}

#[test]
fn the_settings_come_from_the_theme_block_and_the_panel_is_on_unless_it_says_off() {
    let d = Settings::default();
    assert!(d.enabled && d.step_ns == 2 * SEC);
    let o = |v: Value| Settings::from_object(v.as_object().unwrap());
    assert_eq!(o(json!({})), d);
    assert!(!o(json!({"panelDim": false})).enabled);
    assert_eq!(o(json!({"panelDimStepSec": 0})).step_ns, SEC);
    assert_eq!(o(json!({"panelDimStepSec": 500})).step_ns, 60 * SEC);
    assert_eq!(o(json!({"panelDim": "no", "panelDimStepSec": "fast"})), d);
}

#[test]
fn the_answers_numbers_are_read_as_the_settings_service_gives_them() {
    let r = |v: Value| reading(&json!({"settings": {"backlight": v}}));
    assert_eq!(r(json!("80")).map(|x| x.value), Some(80));
    assert_eq!(r(json!(79.6)).map(|x| x.value), Some(80));
    assert_eq!(r(json!("abc")), None);
    assert_eq!(r(json!(101)), None);
    assert_eq!(r(json!(-1)), None);
    assert_eq!(reading(&json!({"settings": {}})), None);
}
