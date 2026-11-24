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
    /// `CmaFree`, when the world has one, and what each app takes out of it while it runs.
    cma: Option<u64>,
    pins: BTreeMap<&'static str, u64>,
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
        let pin = w.pins.get(app).copied().unwrap_or(0);
        w.cma = w.cma.map(|c| c.saturating_sub(pin));
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
        let pin = w.pins.get(app).copied().unwrap_or(0);
        w.cma = w.cma.map(|c| c + pin);
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
    fn cma_free_mb(&self) -> Option<u64> {
        self.0.lock().cma
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
        cma: None,
        pins: BTreeMap::from([(MARQUEE, 20), (NETFLIX, 40), (PRIME, 40), (APPLE_TV, 30), (LIVE_TV, 20)]),
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
    assert_eq!(r.launched(), ids(&[MARQUEE]));
    r.run_to(29);
    assert_eq!(r.launched(), ids(&[MARQUEE]), "the next waits for the first to settle");
    r.run_to(200);
    // 700 - 50 - 90 - 120 = 440: Apple TV needs 100 + 400, so it waits and so does everything behind it.
    assert_eq!(r.launched(), ids(&[MARQUEE, NETFLIX, PRIME]));
    let topic = r.run_to(201);
    assert_eq!(topic["apps"][0], json!({"id": MARQUEE, "state": "ready", "costMb": 50, "note": null}));
    assert_eq!(topic["apps"][1], json!({"id": NETFLIX, "state": "ready", "costMb": 90, "note": null}));
    assert_eq!(topic["apps"][3]["state"], "cold");
    assert_eq!(topic["waiting"], "not enough memory");
    // Live TV fits in the 440 left (its 40 + the 400 floor); then 400 are left and a 100 MB app needs 500.
    r.settings.order = ids(&[NETFLIX, PRIME, LIVE_TV, "tv.twitch"]);
    r.plat.0.lock().cost.insert("tv.twitch", 100);
    assert_eq!(r.run_to(260)["waiting"], "not enough memory");
    assert_eq!(r.launched().len(), 4);
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
    // gone without the policy's doing: it comes back after its backoff, and with the extra room of an app memory took
    r.plat.0.lock().headroom += 200;
    r.run_to(140);
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
    assert_eq!(r.launched(), ids(&[MARQUEE]));
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
    assert_eq!(r.launched(), ids(&[MARQUEE]));
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
    assert_eq!(r.launched(), ids(&[MARQUEE, PRIME, APPLE_TV, LIVE_TV]), "all four come back, in priority order");
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
    r.settings.order = ids(&[NETFLIX, PRIME, LIVE_TV]);
    r.plat.0.lock().sounds = vec![NETFLIX];
    r.run_to(120);
    assert_eq!(r.closed(), ids(&[NETFLIX]));
    assert_eq!(r.launched(), ids(&[NETFLIX, PRIME, LIVE_TV]));
}

#[test]
fn a_refused_launch_cools_the_app_off_and_launches_per_app_are_limited() {
    let mut r = rig();
    r.settings.order = ids(&[NETFLIX, PRIME, LIVE_TV]);
    r.plat.0.lock().refuse = vec![NETFLIX];
    r.run_to(60);
    assert!(r.launched().is_empty(), "a refusal holds back the lower apps too");
    assert_eq!(r.run_to(61)["waiting"], "cooling off");
    // Killed by LG every time it is launched: three launches, then ten minutes (the backoff between them is shorter than the rounds).
    let mut r = rig();
    r.settings.order = ids(&[NETFLIX]);
    for round in 0..5 {
        r.run_to(40 + round * 130);
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

#[test]
fn an_app_is_launched_only_while_cma_stays_above_the_floor() {
    let mut r = rig();
    r.set_headroom(2000);
    // Marquee takes 20 of 190 (needs 20 + 70), Netflix 40 of 170, Prime Video 40 of 130; Apple TV then needs 30 + 70 of the 90 left
    r.plat.0.lock().cma = Some(190);
    let topic = r.run_to(300);
    assert_eq!(r.launched(), ids(&[MARQUEE, NETFLIX, PRIME]));
    assert_eq!(topic["waiting"], "not enough CMA");
    assert_eq!(r.plat.0.lock().cma, Some(90));
}

#[test]
fn nothing_is_launched_while_memory_is_held_for_a_conversion() {
    let mut r = rig();
    r.env.room = true;
    assert_eq!(r.run_to(90)["waiting"], "memory held for a conversion");
    assert!(r.launched().is_empty());
    r.env.room = false;
    r.run_to(100);
    assert_eq!(r.launched().first(), Some(&MARQUEE.to_owned()));
}

#[test]
fn an_app_closed_for_a_conversion_comes_back_when_cma_allows() {
    let mut r = rig();
    r.set_headroom(2000);
    r.plat.0.lock().running = ids(&[HOME, MARQUEE, NETFLIX, PRIME, APPLE_TV, LIVE_TV]);
    r.plat.0.lock().cma = Some(30);
    let closed = room::clear(&r.plat, &ids(&DEFAULT_ORDER), None, 40, &|| {});
    assert_eq!(closed, Ok(ids(&[LIVE_TV])), "Live TV is the last of the list");
    r.env.room = true;
    r.run_to(60);
    assert!(r.launched().is_empty(), "held");
    r.env.room = false;
    assert_eq!(r.run_to(90)["waiting"], "not enough CMA", "Live TV needs 20 + 70 and 50 are free");
    assert!(r.launched().is_empty());
    r.plat.0.lock().cma = Some(100);
    r.run_to(120);
    assert_eq!(r.launched(), ids(&[LIVE_TV]));
}

#[test]
fn room_is_made_by_closing_the_kept_apps_last_of_the_list_first() {
    let r = rig();
    r.plat.0.lock().running = ids(&[HOME, MARQUEE, NETFLIX, PRIME, APPLE_TV, LIVE_TV]);
    r.plat.0.lock().cma = Some(10);
    let order = ids(&DEFAULT_ORDER);
    // Live TV gives 20 (30 free), Apple TV 30 (60): enough
    assert_eq!(room::clear(&r.plat, &order, None, 60, &|| {}), Ok(ids(&[LIVE_TV, APPLE_TV])));
    assert_eq!(room::clear(&r.plat, &order, None, 60, &|| {}), Ok(vec![]), "the room is there");
    // more than all of them hold: Marquee goes last, and the shortfall is said
    let short = room::clear(&r.plat, &order, None, 500, &|| {}).unwrap_err();
    assert_eq!(short, room::Short { free_mb: 160, closed: ids(&[PRIME, NETFLIX, MARQUEE]) });
    assert_eq!(r.closed(), ids(&[LIVE_TV, APPLE_TV, PRIME, NETFLIX, MARQUEE]));
}

#[test]
fn an_app_on_screen_or_making_sound_is_not_closed_for_room_and_an_unreadable_cma_asks_for_nothing() {
    let r = rig();
    r.plat.0.lock().running = ids(&[HOME, MARQUEE, NETFLIX, PRIME, APPLE_TV, LIVE_TV]);
    r.plat.0.lock().playing.insert(NETFLIX.to_owned());
    r.plat.0.lock().cma = Some(10);
    let short = room::clear(&r.plat, &ids(&DEFAULT_ORDER), Some(MARQUEE), 500, &|| {}).unwrap_err();
    assert_eq!(short, room::Short { free_mb: 100, closed: ids(&[LIVE_TV, APPLE_TV, PRIME]) });
    r.plat.0.lock().cma = None;
    assert_eq!(room::clear(&r.plat, &ids(&DEFAULT_ORDER), None, 500, &|| {}), Ok(vec![]));
}

#[test]
fn an_app_that_keeps_vanishing_is_launched_less_and_less_often() {
    let mut r = rig();
    r.settings.order = ids(&[LIVE_TV]);
    let (mut seen, mut last, mut gaps) = (0, 0, Vec::new());
    for t in 1..3000 {
        r.run_to(t);
        let launches = r.plat.0.lock().launches.len();
        if launches > seen {
            seen = launches;
            if last > 0 {
                gaps.push(t - last);
            }
            last = t;
            // LG's memory manager takes it again at once
            let mut w = r.plat.0.lock();
            w.running.retain(|a| a != LIVE_TV);
            w.headroom += 40;
        }
        if gaps.len() == 3 {
            break;
        }
    }
    assert_eq!(gaps.len(), 3, "{gaps:?}");
    assert!(gaps[0] >= 15 && gaps[1] >= 30 && gaps[2] >= 60, "{gaps:?}");
}
