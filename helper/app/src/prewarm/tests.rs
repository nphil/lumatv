use super::policy::*;
use super::*;
use parking_lot::Mutex;
use serde_json::json;
use std::collections::BTreeMap;

struct World {
    running: Vec<String>,
    playing: BTreeSet<String>,
    front: String,
    headroom: u64,
    cost: BTreeMap<&'static str, u64>,
    /// Apps that come to the front when preloaded, and apps that make sound.
    shows: Vec<&'static str>,
    sounds: Vec<&'static str>,
    refuse: Vec<&'static str>,
    launches: Vec<(u64, String)>,
    closes: Vec<(u64, String)>,
    homes: usize,
    now: u64,
}

struct Fake(Mutex<World>);

impl Platform for Fake {
    fn running(&self) -> Option<Vec<String>> {
        Some(self.0.lock().running.clone())
    }
    fn playing(&self) -> Option<BTreeSet<String>> {
        Some(self.0.lock().playing.clone())
    }
    fn launch_hidden(&self, app: &str) -> bool {
        let mut w = self.0.lock();
        if w.refuse.contains(&app) {
            return false;
        }
        let now = w.now;
        w.launches.push((now, app.to_owned()));
        w.running.push(app.to_owned());
        w.headroom -= w.cost[app];
        if w.shows.contains(&app) {
            w.front = app.to_owned();
        }
        if w.sounds.contains(&app) {
            w.playing.insert(app.to_owned());
        }
        true
    }
    fn close(&self, app: &str) -> bool {
        let mut w = self.0.lock();
        let now = w.now;
        w.closes.push((now, app.to_owned()));
        w.running.retain(|r| r != app);
        w.playing.remove(app);
        w.headroom += w.cost[app];
        true
    }
    fn home(&self) -> bool {
        let mut w = self.0.lock();
        w.front = HOME.to_owned();
        w.homes += 1;
        true
    }
    fn headroom_mb(&self) -> Option<u64> {
        Some(self.0.lock().headroom)
    }
}

struct Rig {
    driver: Driver,
    plat: Fake,
    settings: Settings,
    env: Env,
}

fn rig() -> Rig {
    let cost = BTreeMap::from([(MARQUEE, 50), (NETFLIX, 90), (PRIME, 120), (APPLE_TV, 100), (LIVE_TV, 40)]);
    let world = World {
        running: vec![HOME.to_owned()],
        playing: BTreeSet::new(),
        front: HOME.to_owned(),
        headroom: 700,
        cost,
        shows: vec![],
        sounds: vec![],
        refuse: vec![],
        launches: vec![],
        closes: vec![],
        homes: 0,
        now: 0,
    };
    Rig { driver: Driver::default(), plat: Fake(Mutex::new(world)), settings: Settings::default(), env: Env { screen_on: true, ..Env::default() } }
}

impl Rig {
    /// Ticks once a second up to `secs` and returns the last topic value.
    fn run_to(&mut self, secs: u64) -> Value {
        let mut last = Value::Null;
        let from = self.plat.0.lock().now / 1000;
        for t in from..=secs {
            self.plat.0.lock().now = t * 1000;
            self.env.front = Some(self.plat.0.lock().front.clone());
            last = self.driver.tick(&self.settings, &self.env, &self.plat, t * 1000);
        }
        last
    }
    fn launched(&self) -> Vec<String> {
        self.plat.0.lock().launches.iter().map(|(_, a)| a.clone()).collect()
    }
    fn closed(&self) -> Vec<String> {
        self.plat.0.lock().closes.iter().map(|(_, a)| a.clone()).collect()
    }
    fn set_headroom(&self, mb: u64) {
        self.plat.0.lock().headroom = mb;
    }
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn settings_default_to_on_with_the_five_apps_and_drop_bad_or_repeated_ids() {
    assert_eq!(Settings::from_theme(&json!({})), Settings::default());
    assert_eq!(Settings::default().order, ids(&DEFAULT_ORDER));
    let s = Settings::from_theme(&json!({"system": {"prewarm": {"on": false, "apps": ["netflix", "bad id", "netflix", "amazon", 5]}}}));
    assert_eq!((s.on, s.order), (false, ids(&["netflix", "amazon"])));
    let s = Settings::from_theme(&json!({"system": {"prewarm": {"on": true}}}));
    assert_eq!(s.order, ids(&DEFAULT_ORDER));
    let many: Vec<String> = (0..20).map(|i| format!("app.n{i}")).collect();
    assert_eq!(Settings::from_theme(&json!({"system": {"prewarm": {"apps": many}}})).order.len(), MAX_APPS);
}

#[test]
fn apps_are_launched_one_at_a_time_in_priority_order_until_memory_is_short() {
    let mut r = rig();
    r.run_to(19);
    assert!(r.launched().is_empty(), "the screen has been on only 19 s");
    r.run_to(20);
    assert_eq!(r.launched(), ids(&[NETFLIX]));
    r.run_to(29);
    assert_eq!(r.launched(), ids(&[NETFLIX]), "the next waits for the first to settle");
    r.run_to(200);
    // Marquee and Apple TV show themselves when preloaded: never launched. 700 - 90 - 120 = 490 still leaves Live TV its 440.
    assert_eq!(r.launched(), ids(&[NETFLIX, PRIME, LIVE_TV]));
    let topic = r.run_to(201);
    assert_eq!(topic["apps"][1], json!({"id": NETFLIX, "state": "ready", "costMb": 90, "note": null}));
    assert_eq!(topic["apps"][0], json!({"id": MARQUEE, "state": "skipped", "costMb": null, "note": "opens normally the first time"}));
    // 490 - 40 = 450: a sixth app that costs more than 50 MB does not fit.
    r.settings.order = ids(&[NETFLIX, PRIME, LIVE_TV, "tv.twitch"]);
    r.plat.0.lock().cost.insert("tv.twitch", 100);
    assert_eq!(r.run_to(260)["waiting"], "not enough memory");
    assert_eq!(r.launched().len(), 3);
}

#[test]
fn an_app_that_is_running_is_never_launched_and_a_closed_one_comes_back() {
    let mut r = rig();
    r.settings.order = ids(&[MARQUEE, NETFLIX, PRIME]);
    r.plat.0.lock().running.push(MARQUEE.to_owned());
    r.run_to(60);
    assert_eq!(r.launched(), ids(&[NETFLIX, PRIME]));
    // The owner closes Netflix: it is launched again once Home has been quiet for a moment.
    r.plat.0.lock().running.retain(|a| a != NETFLIX);
    r.plat.0.lock().headroom += 90;
    r.run_to(80);
    assert_eq!(r.launched(), ids(&[NETFLIX, PRIME, NETFLIX]));
}

#[test]
fn nothing_is_launched_while_a_gate_is_shut() {
    type Gate = (&'static str, fn(&mut Rig));
    let gates: [Gate; 7] = [
        ("off", |r| r.settings.on = false),
        ("standby", |r| r.env.screen_on = false),
        ("refresher", |r| r.env.refresher = true),
        ("game", |r| r.env.game = true),
        ("app on screen", |r| r.plat.0.lock().front = "tv.twitch".to_owned()),
        ("sound", |r| {
            r.plat.0.lock().playing.insert("tv.twitch".to_owned());
        }),
        ("memory", |r| r.set_headroom(440)),
    ];
    for (name, shut) in gates {
        let mut r = rig();
        shut(&mut r);
        r.run_to(90);
        assert!(r.launched().is_empty(), "{name}");
    }
    // The screen coming on starts the 20 s again.
    let mut r = rig();
    r.env.screen_on = false;
    r.run_to(100);
    r.env.screen_on = true;
    r.run_to(110);
    assert!(r.launched().is_empty());
    r.run_to(121);
    assert_eq!(r.launched(), ids(&[NETFLIX]));
}

#[test]
fn a_leaving_gate_waits_for_home_to_be_quiet() {
    let mut r = rig();
    r.plat.0.lock().front = "tv.twitch".to_owned();
    r.run_to(60);
    r.plat.0.lock().front = HOME.to_owned();
    r.run_to(64);
    assert!(r.launched().is_empty());
    r.run_to(66);
    assert_eq!(r.launched(), ids(&[NETFLIX]));
}

#[test]
fn pressure_closes_the_lowest_priority_app_first_one_every_five_seconds_and_spares_the_one_on_screen() {
    let mut r = rig();
    r.set_headroom(2000);
    r.plat.0.lock().running.extend(ids(&DEFAULT_ORDER));
    r.plat.0.lock().front = NETFLIX.to_owned();
    r.run_to(10);
    assert!(r.closed().is_empty());
    r.set_headroom(0);
    r.run_to(12);
    assert_eq!(r.closed(), ids(&[LIVE_TV]));
    r.run_to(14);
    assert_eq!(r.closed(), ids(&[LIVE_TV]), "five seconds apart");
    r.run_to(40);
    // Netflix is on screen and stays; Marquee goes last.
    assert_eq!(r.closed(), ids(&[LIVE_TV, APPLE_TV, PRIME, MARQUEE]));
    // Memory recovers: no pressure close, and the evicted are not launched again for ten minutes.
    r.set_headroom(2000);
    r.plat.0.lock().front = HOME.to_owned();
    r.run_to(300);
    assert_eq!(r.closed().len(), 4);
    assert!(r.launched().is_empty());
    r.run_to(700);
    assert_eq!(r.launched(), ids(&[PRIME, LIVE_TV]), "Marquee and Apple TV are only kept");
}

#[test]
fn an_app_that_is_playing_is_not_evicted() {
    let mut r = rig();
    r.plat.0.lock().running.extend(ids(&[MARQUEE, LIVE_TV]));
    r.plat.0.lock().playing.insert(LIVE_TV.to_owned());
    r.plat.0.lock().front = "tv.twitch".to_owned();
    r.set_headroom(250);
    r.run_to(30);
    assert_eq!(r.closed(), ids(&[MARQUEE]), "Live TV sounds, so the next one goes");
}

#[test]
fn an_app_that_shows_itself_is_sent_home_closed_and_left_out() {
    let mut r = rig();
    r.settings.order = ids(&["tv.twitch", NETFLIX, PRIME]);
    r.plat.0.lock().cost.insert("tv.twitch", 100);
    r.plat.0.lock().shows = vec!["tv.twitch"];
    r.run_to(60);
    assert_eq!(r.launched(), ids(&["tv.twitch", NETFLIX, PRIME]));
    assert_eq!(r.closed(), ids(&["tv.twitch"]));
    assert_eq!(r.plat.0.lock().homes, 1);
    let topic = r.run_to(61);
    assert_eq!(topic["apps"][0]["state"], "skipped");
    assert_eq!(topic["apps"][0]["note"], "shows itself when preloaded");
}

#[test]
fn an_app_that_makes_sound_is_closed_and_left_out() {
    let mut r = rig();
    r.plat.0.lock().sounds = vec![NETFLIX];
    r.run_to(120);
    assert_eq!(r.closed(), ids(&[NETFLIX]));
    assert_eq!(r.launched(), ids(&[NETFLIX, PRIME, LIVE_TV]));
}

#[test]
fn a_refused_launch_cools_the_app_off_and_launches_per_app_are_limited() {
    let mut r = rig();
    r.plat.0.lock().refuse = vec![NETFLIX];
    r.run_to(60);
    assert!(r.launched().is_empty(), "a refusal holds back the lower apps too");
    assert_eq!(r.run_to(61)["waiting"], "cooling off");
    // Killed by LG every time it is launched: three launches, then ten minutes.
    let mut r = rig();
    r.settings.order = ids(&[NETFLIX]);
    for round in 0..6 {
        r.run_to(40 + round * 40);
        r.plat.0.lock().running.retain(|a| a != NETFLIX);
        r.plat.0.lock().headroom = 700;
    }
    assert_eq!(r.launched(), ids(&[NETFLIX, NETFLIX, NETFLIX]));
}

#[test]
fn the_owners_order_decides_who_is_launched_first() {
    let mut r = rig();
    r.settings.order = ids(&[LIVE_TV, NETFLIX]);
    r.run_to(80);
    assert_eq!(r.launched(), ids(&[LIVE_TV, NETFLIX]));
}

#[test]
fn keep_protects_by_position_and_nothing_for_an_unlisted_app() {
    let keep = Keep::new();
    keep.set(ids(&DEFAULT_ORDER));
    let p = |a| keep.protection(a);
    assert!(p(LIVE_TV) < p(APPLE_TV) && p(APPLE_TV) < p(PRIME) && p(PRIME) < p(NETFLIX) && p(NETFLIX) < p(MARQUEE));
    assert_eq!((p("tv.twitch"), p(LIVE_TV)), (0, 1));
}

#[test]
fn the_topic_names_what_holds_launches_back() {
    let mut r = rig();
    assert_eq!(r.run_to(5)["waiting"], "screen just on");
    r.env.game = true;
    assert_eq!(r.run_to(30)["waiting"], "game");
    r.env.game = false;
    r.env.refresher = true;
    assert_eq!(r.run_to(31)["waiting"], "pixel refresher");
}
