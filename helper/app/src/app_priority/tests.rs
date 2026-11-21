use super::memchute::keeping;
use super::os::{Os, Pid, UNLIMITED};
use super::pressure::Sample;
use super::*;
use parking_lot::Mutex;

const PLEX: &str = "com.beb.plxnative";
const TWITCH: &str = "tv.twitch";
const NETFLIX: &str = "netflix";
const LUMAGAME: &str = "org.nphil.lumagame";
const HOME: &str = "com.webos.app.home";
const GAME: &str = "com.example.game";
const APPS: [(&str, i32); 6] = [(PLEX, 100), (TWITCH, 200), (NETFLIX, 300), (LUMAGAME, 400), (HOME, 500), (GAME, 600)];

#[derive(Default)]
struct World {
    procs: BTreeMap<String, Vec<Pid>>,
    oom: BTreeMap<i32, i32>,
    background: BTreeSet<i32>,
    freezer: BTreeMap<String, String>,
    soft: BTreeMap<String, u64>,
    usage: BTreeMap<String, u64>,
    reclaimed: Vec<String>,
    sample: Sample,
    available_kb: u64,
    vm: Vec<String>,
    vm_fails: bool,
    usable: u64,
    groups: BTreeSet<String>,
    attached: Vec<(String, i32)>,
    swappiness: BTreeMap<String, u32>,
    trims: usize,
}

#[derive(Default)]
struct FakeOs(Mutex<World>);

impl Os for FakeOs {
    fn app_procs(&self, app: &str) -> Option<Vec<Pid>> {
        self.0.lock().procs.get(app).cloned()
    }
    fn alive(&self, p: Pid) -> bool {
        self.0.lock().procs.values().flatten().any(|q| *q == p)
    }
    fn oom(&self, p: Pid) -> Option<i32> {
        self.0.lock().oom.get(&p.pid).copied()
    }
    fn set_oom(&self, p: Pid, value: i32) -> std::io::Result<()> {
        self.0.lock().oom.insert(p.pid, value);
        Ok(())
    }
    fn cpu_root(&self, p: Pid) -> bool {
        !self.0.lock().background.contains(&p.pid)
    }
    fn to_background(&self, p: Pid) -> std::io::Result<()> {
        self.0.lock().background.insert(p.pid);
        Ok(())
    }
    fn to_root(&self, pid: i32) -> std::io::Result<()> {
        self.0.lock().background.remove(&pid);
        Ok(())
    }
    fn drop_background(&self) -> std::io::Result<()> {
        self.0.lock().background.clear();
        Ok(())
    }
    fn freezer(&self, app: &str) -> Option<String> {
        self.0.lock().freezer.get(app).cloned()
    }
    fn set_freezer(&self, app: &str, state: &str) -> std::io::Result<()> {
        self.0.lock().freezer.insert(app.to_owned(), state.to_owned());
        Ok(())
    }
    fn soft_limit(&self, app: &str) -> Option<u64> {
        self.0.lock().soft.get(app).copied()
    }
    fn set_soft_limit(&self, app: &str, bytes: u64) -> std::io::Result<()> {
        self.0.lock().soft.insert(app.to_owned(), bytes);
        Ok(())
    }
    fn usage(&self, app: &str) -> u64 {
        self.0.lock().usage.get(app).copied().unwrap_or(0)
    }
    fn reclaim(&self, app: &str) -> std::io::Result<()> {
        let mut w = self.0.lock();
        w.reclaimed.push(app.to_owned());
        w.available_kb += 50 * 1024;
        Ok(())
    }
    fn sample(&self) -> Option<Sample> {
        Some(self.0.lock().sample)
    }
    fn available_kb(&self) -> Option<u64> {
        Some(self.0.lock().available_kb)
    }
    fn vm(&self, name: &str, value: &str) -> std::io::Result<()> {
        let mut w = self.0.lock();
        if w.vm_fails {
            return Err(std::io::Error::other("refused"));
        }
        w.vm.push(format!("{name}={value}"));
        w.available_kb += if name == "drop_caches" { 100 * 1024 } else { 10 * 1024 };
        Ok(())
    }
    fn usable_pages(&self) -> Option<u64> {
        Some(self.0.lock().usable)
    }
    fn group_pids(&self, app: &str) -> Option<Vec<i32>> {
        self.0.lock().procs.get(app).map(|p| p.iter().map(|p| p.pid).collect())
    }
    fn make_groups(&self, app: &str) -> std::io::Result<()> {
        self.0.lock().groups.insert(app.to_owned());
        Ok(())
    }
    fn attach(&self, app: &str, pid: i32) -> std::io::Result<bool> {
        let mut w = self.0.lock();
        let entry = (app.to_owned(), pid);
        let new = !w.attached.contains(&entry);
        if new {
            w.attached.push(entry);
        }
        Ok(new)
    }
    fn remove_groups(&self, app: &str) {
        self.0.lock().groups.remove(app);
    }
    fn set_swappiness(&self, app: &str, value: u32) -> std::io::Result<()> {
        self.0.lock().swappiness.insert(app.to_owned(), value);
        Ok(())
    }
    fn systrim(&self) -> usize {
        self.0.lock().trims += 1;
        1
    }
}

#[derive(Default)]
struct Sam {
    running: Vec<String>,
    pids: BTreeMap<String, Vec<i32>>,
    playing: BTreeSet<String>,
    closed: Vec<String>,
    close_fails: bool,
    playing_unknown: bool,
    toasts: Vec<String>,
}

#[derive(Default)]
struct FakeLg(Mutex<Sam>);

impl Lg for FakeLg {
    fn running(&self) -> Option<Vec<Running>> {
        let sam = self.0.lock();
        let pids = |id: &str| sam.pids.get(id).cloned().unwrap_or_default();
        Some(sam.running.iter().map(|id| Running { id: id.clone(), pids: pids(id), app_type: "native".into() }).collect())
    }
    fn playing(&self) -> Option<BTreeSet<String>> {
        let sam = self.0.lock();
        (!sam.playing_unknown).then(|| sam.playing.clone())
    }
    fn close(&self, app: &str) -> bool {
        let mut sam = self.0.lock();
        if sam.close_fails {
            return false;
        }
        sam.closed.push(app.to_owned());
        sam.running.retain(|a| a != app);
        true
    }
    fn title(&self, app: &str) -> Option<String> {
        (app == PLEX).then(|| "Plex".to_owned())
    }
    fn toast(&self, message: &str) {
        self.0.lock().toasts.push(message.to_owned());
    }
}

struct Rig {
    os: Arc<FakeOs>,
    lg: Arc<FakeLg>,
    engine: Engine,
    t0: Instant,
    _dir: tempfile::TempDir,
    journal: PathBuf,
}

fn s(secs: f64) -> Duration {
    Duration::from_secs_f64(secs)
}

/// Every app running at -1000 with no soft limit and a thawed freezer group; Plex left the screen at 1 s, Twitch at 2 s, and the
/// game has been in front since.
fn rig(timing: Timing) -> Rig {
    let os = Arc::new(FakeOs::default());
    {
        let mut w = os.0.lock();
        for (app, pid) in APPS {
            w.procs.insert(app.to_owned(), vec![Pid { pid, start: 1 }]);
            w.oom.insert(pid, -1000);
            w.freezer.insert(app.to_owned(), "THAWED".to_owned());
            w.soft.insert(app.to_owned(), UNLIMITED);
            w.usage.insert(app.to_owned(), pid as u64);
        }
        w.sample = Sample { swap_total_kb: 600_000, swap_free_kb: 400_000, free_pages: 20_000, high_wmark_pages: 8_000, file_lru_pages: 100_000, ..Sample::default() };
    }
    let lg = Arc::new(FakeLg::default());
    {
        let mut sam = lg.0.lock();
        sam.running = APPS.iter().map(|(a, _)| (*a).to_owned()).collect();
        sam.pids = APPS.iter().map(|(a, pid)| ((*a).to_owned(), vec![*pid])).collect();
    }
    let dir = tempfile::tempdir().unwrap();
    let journal = dir.path().join("state.json");
    let t0 = Instant::now();
    let mut engine = Engine::new(os.clone(), lg.clone(), timing, Some(keeping(&[NETFLIX])), vec![], Some(journal.clone()), Box::new(|work| work()), t0);
    engine.refresh(t0);
    engine.front(vec![PLEX.into()], t0);
    engine.front(vec![TWITCH.into()], t0 + s(1.0));
    engine.front(vec![GAME.into()], t0 + s(2.0));
    Rig { os, lg, engine, t0, _dir: dir, journal }
}

impl Rig {
    fn oom(&self, pid: i32) -> i32 {
        self.os.0.lock().oom[&pid]
    }
    fn background(&self) -> Vec<i32> {
        self.os.0.lock().background.iter().copied().collect()
    }
    fn frozen(&self) -> Vec<String> {
        self.os.0.lock().freezer.iter().filter(|(_, s)| *s == "FROZEN").map(|(a, _)| a.clone()).collect()
    }
    fn soft(&self, app: &str) -> u64 {
        self.os.0.lock().soft[app]
    }
    fn at(&mut self, secs: f64) {
        self.engine.tick(self.t0 + s(secs));
    }
    /// One second of medium pressure (300 ms of partial stall), with `f` changing the rest.
    fn pressure(&mut self, secs: f64, f: impl Fn(&mut Sample)) {
        {
            let mut w = self.os.0.lock();
            w.sample.some_us += 300_000;
            f(&mut w.sample);
        }
        self.at(secs);
    }
    fn assert_all_back(&self) {
        for (_, pid) in APPS {
            assert_eq!(self.oom(pid), -1000);
        }
        assert!(self.background().is_empty());
        assert!(self.frozen().is_empty());
        for (app, _) in APPS {
            assert_eq!(self.soft(app), UNLIMITED);
        }
    }
}

#[test]
fn background_apps_are_ranked_by_when_they_left_and_get_everything_back_in_front() {
    let mut r = rig(Timing::default());
    // Twitch left last: the lowest cached adj. Netflix was never seen in front and ranks with the oldest; the game, LumaGame and Home keep theirs.
    assert_eq!((r.oom(200), r.oom(100), r.oom(300)), (900, 910, 920));
    assert_eq!((r.oom(400), r.oom(500), r.oom(600)), (-1000, -1000, -1000));
    assert_eq!(r.background(), vec![100, 200, 300]);
    assert_eq!((r.soft(PLEX), r.soft(NETFLIX), r.soft(LUMAGAME)), (0, 0, UNLIMITED));

    // A process Plex starts while demoted inherits 910 and gets Plex's original back with it.
    {
        let mut w = r.os.0.lock();
        w.procs.get_mut(PLEX).unwrap().push(Pid { pid: 101, start: 1 });
        w.oom.insert(101, 910);
    }
    r.at(3.0);
    r.engine.front(vec![PLEX.into()], r.t0 + s(4.0));
    assert_eq!((r.oom(100), r.oom(101)), (-1000, -1000));
    assert!(!r.background().contains(&100) && !r.background().contains(&101));
    assert_eq!(r.soft(PLEX), UNLIMITED);
    // The game left last now.
    assert_eq!((r.oom(600), r.oom(200)), (900, 910));
}

#[test]
fn visible_launching_and_playing_apps_lg_apps_and_lumatvs_are_never_frozen_or_ranked() {
    let mut r = rig(Timing::default());
    r.lg.0.lock().playing.insert(TWITCH.into());
    r.engine.refresh(r.t0 + s(2.5));
    r.engine.front(vec![GAME.into(), PLEX.into()], r.t0 + s(3.0));
    assert_eq!((r.oom(100), r.oom(200)), (-1000, -1000));
    assert_eq!(r.background(), vec![300]);
    r.at(60.0);
    // Netflix is memchute's: ranked and demoted, never frozen; LumaGame and Home are not touched at all.
    assert!(r.frozen().is_empty());
    assert_eq!((r.oom(300), r.oom(400), r.oom(500)), (900, -1000, -1000));
    r.engine.front(vec![GAME.into()], r.t0 + s(61.0));
    r.engine.life(PLEX, "launching", r.t0 + s(61.5));
    r.at(62.0);
    assert_eq!(r.oom(100), -1000);
    r.at(66.5);
    assert_eq!(r.oom(100), 900);
}

#[test]
fn a_cached_app_freezes_after_ten_seconds_and_thaws_at_any_status_but_background() {
    let mut r = rig(Timing::default());
    r.os.0.lock().freezer.insert(TWITCH.into(), "FROZEN".into());
    assert_eq!(r.engine.next(r.t0 + s(2.0)), Some(r.t0 + s(2.0)));
    r.at(10.9);
    assert_eq!(r.frozen(), vec![TWITCH]);
    r.at(11.0);
    assert_eq!(r.frozen(), vec![PLEX, TWITCH]);
    r.engine.life(PLEX, "relaunching", r.t0 + s(12.0));
    r.engine.life(TWITCH, "relaunching", r.t0 + s(12.0));
    // Plex is thawed at once; the group memchute froze is not this policy's to thaw.
    assert_eq!(r.frozen(), vec![TWITCH]);
    assert_eq!(r.oom(100), -1000);
}

#[test]
fn pressure_reclaims_first_then_closes_the_oldest_cached_app_one_at_a_time() {
    let mut r = rig(Timing::default());
    r.at(12.0);
    assert_eq!(r.frozen(), vec![PLEX, TWITCH]);
    // No stall: free memory under the watermark alone does nothing.
    r.os.0.lock().sample.free_pages = 1_000;
    r.at(13.0);
    r.at(14.0);
    assert!(r.os.0.lock().reclaimed.is_empty() && r.lg.0.lock().closed.is_empty());
    r.os.0.lock().sample.free_pages = 20_000;
    // Medium pressure with no second reason: reclaim, the oldest frozen app first, once each.
    r.pressure(15.0, |_| {});
    r.pressure(16.0, |_| {});
    r.pressure(17.0, |_| {});
    assert_eq!(r.os.0.lock().reclaimed, vec![PLEX, TWITCH]);
    assert!(r.lg.0.lock().closed.is_empty());
    // Low swap and low memory: Plex (oldest; Netflix is memchute's) is thawed and closed, with a toast.
    r.pressure(18.0, |s| {
        s.swap_free_kb = 30_000;
        s.free_pages = 1_000
    });
    assert_eq!(r.lg.0.lock().closed, vec![PLEX]);
    assert_eq!(r.os.0.lock().freezer[PLEX], "THAWED");
    assert_eq!(r.lg.0.lock().toasts, vec!["Closed Plex to free memory for the app on screen"]);
    // Plex has not exited yet: no second close.
    r.pressure(19.0, |_| {});
    assert_eq!(r.lg.0.lock().closed, vec![PLEX]);
    // It exits: lmkd's 100 ms pass first, then Twitch goes.
    r.os.0.lock().procs.remove(PLEX);
    r.pressure(20.0, |_| {});
    assert_eq!(r.lg.0.lock().closed, vec![PLEX]);
    r.pressure(21.0, |_| {});
    assert_eq!(r.lg.0.lock().closed, vec![PLEX, TWITCH]);
    assert!(r.lg.0.lock().closed.iter().all(|a| a != NETFLIX && a != GAME));
}

#[test]
fn a_complete_stall_closes_at_once_and_closes_are_limited_per_minute() {
    let mut r = rig(Timing { closes_per_minute: 1, ..Timing::default() });
    r.at(3.0);
    r.pressure(4.0, |s| s.full_us += 800_000);
    assert_eq!(r.lg.0.lock().closed, vec![PLEX]);
    r.os.0.lock().procs.remove(PLEX);
    r.pressure(5.0, |s| s.full_us += 800_000);
    r.pressure(6.0, |s| s.full_us += 800_000);
    assert_eq!(r.lg.0.lock().closed, vec![PLEX]);
    // The minute's quiet gap: the stall is spread over the 58.5 s since the last sample.
    r.pressure(64.5, |s| s.full_us += 60_000_000);
    assert_eq!(r.lg.0.lock().closed, vec![PLEX, TWITCH]);
}

#[test]
fn every_way_out_puts_everything_back() {
    let mut r = rig(Timing::default());
    r.at(12.0);
    assert!(r.journal.exists());
    r.engine.restore_all("test");
    r.assert_all_back();
    assert!(!r.journal.exists());

    // A lost subscription: nothing stays demoted while the front is unknown.
    let mut r = rig(Timing::default());
    r.at(12.0);
    r.engine.lost("test");
    r.assert_all_back();
    r.at(30.0);
    r.assert_all_back();

    // A crash: the next start puts back what the journal names.
    let mut r = rig(Timing::default());
    r.at(12.0);
    let (os, journal) = (r.os.clone(), r.journal.clone());
    recover(&*os, &journal);
    r.assert_all_back();
    assert!(!journal.exists());
}

mod game_mode {
    use super::*;
    use crate::app_priority::game::{EXIT_AFTER, Pause};
    use std::sync::atomic::AtomicUsize;

    struct GameRig {
        r: Rig,
        switch: Arc<AtomicBool>,
        pause: &'static Pause,
        resumed: Arc<AtomicUsize>,
    }

    /// The core rig with game mode switched on and Twitch playing; LumaGame comes to the front at 3 s.
    fn game_rig() -> GameRig {
        let mut r = rig(Timing::default());
        let switch = Arc::new(AtomicBool::new(true));
        let pause: &'static Pause = Box::leak(Box::new(Pause::new()));
        let resumed = Arc::new(AtomicUsize::new(0));
        {
            let resumed = resumed.clone();
            pause.on_resume(move || {
                resumed.fetch_add(1, Ordering::Relaxed);
            });
        }
        let s = switch.clone();
        r.engine.set_game(Box::new(move || s.load(Ordering::Relaxed)), pause);
        r.lg.0.lock().playing.insert(TWITCH.into());
        r.os.0.lock().available_kb = 300 * 1024;
        GameRig { r, switch, pause, resumed }
    }

    impl GameRig {
        fn status(&self) -> Value {
            self.r.engine.game_status()
        }
        fn on(&self) -> bool {
            self.status()["on"] == true
        }
        fn exit_reason(&self) -> Value {
            self.status()["lastExit"]["reason"].clone()
        }
        /// The closed apps exit, freeing 200 MB.
        fn exited(&self, apps: &[&str]) {
            let mut w = self.r.os.0.lock();
            for a in apps {
                w.procs.remove(*a);
            }
            w.available_kb += 200 * 1024;
        }
        fn front(&mut self, app: &str, secs: f64) {
            self.r.engine.front(vec![app.into()], self.r.t0 + s(secs));
        }
    }

    #[test]
    fn entry_closes_waits_pauses_and_reclaims_in_order_sparing_lg_ours_kept_and_playing_apps() {
        let mut g = game_rig();
        g.front(LUMAGAME, 3.0);
        assert!(g.on());
        // Twitch plays, Netflix is memchute's, Home is LG's, LumaGame is the game: Plex and the other game are closed, with a toast.
        assert_eq!(g.r.lg.0.lock().closed, vec![PLEX, GAME]);
        assert_eq!(g.r.lg.0.lock().toasts, vec!["Game mode closed Plex, com.example.game to free memory for the game"]);
        // Nothing more until they have exited.
        g.r.at(3.5);
        assert!(!g.pause.paused() && g.r.os.0.lock().reclaimed.is_empty());
        g.exited(&[PLEX, GAME]);
        g.r.at(3.6);
        assert!(g.pause.paused());
        // Home's group is reclaimed; Twitch's (playing) and LumaGame's are not.
        assert_eq!(g.r.os.0.lock().reclaimed, vec![NETFLIX, HOME]);
        assert_eq!(g.r.os.0.lock().vm, vec!["compact_memory=1", "drop_caches=1"]);
        let steps: Vec<(String, i64)> = g.status()["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["step"].as_str().unwrap().to_owned(), s["freedMb"].as_i64().unwrap()))
            .collect();
        let want = [("close", 200), ("pause", 0), ("reclaim", 100), ("compact", 10), ("dropCaches", 100)];
        assert_eq!(steps, want.map(|(s, mb)| (s.to_owned(), mb)));
        assert_eq!(g.status()["app"], LUMAGAME);
    }

    #[test]
    fn with_no_word_on_who_plays_nothing_is_closed_or_reclaimed() {
        let mut g = game_rig();
        g.r.lg.0.lock().playing_unknown = true;
        g.front(LUMAGAME, 3.0);
        assert!(g.on() && g.pause.paused());
        assert!(g.r.lg.0.lock().closed.is_empty());
        assert!(g.r.os.0.lock().reclaimed.is_empty());
        assert_eq!(g.status()["steps"][0]["note"], "it is unknown which app plays: none closed");
    }

    #[test]
    fn leaving_the_front_ends_it_after_five_seconds_and_the_feeds_resume() {
        let mut g = game_rig();
        g.front(LUMAGAME, 3.0);
        g.exited(&[PLEX, GAME]);
        g.r.at(3.1);
        g.front(HOME, 10.0);
        // A trip out and back inside the five seconds keeps it.
        g.front(LUMAGAME, 12.0);
        g.front(HOME, 13.0);
        g.r.at(13.0 + EXIT_AFTER.as_secs_f64() - 0.1);
        assert!(g.on() && g.pause.paused());
        g.r.at(13.0 + EXIT_AFTER.as_secs_f64());
        assert!(!g.on() && !g.pause.paused());
        assert_eq!(g.exit_reason(), "left");
        assert_eq!(g.resumed.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn an_api_lease_counts_thirty_seconds_while_its_app_is_in_front_and_is_renewed_by_asking_again() {
        let mut g = game_rig();
        let t0 = g.r.t0;
        // The rig's game is in front and is not a named game.
        assert!(!g.on());
        let (t3, t30, t64) = (t0 + s(3.0), t0 + s(30.0), t0 + s(64.0));
        g.r.engine.game_request(true, GAME, t3);
        assert!(g.on());
        g.r.engine.game_request(true, GAME, t30);
        g.r.at(40.0);
        assert!(g.on(), "renewed at 30 s");
        g.r.at(59.9);
        assert!(g.on());
        g.r.engine.tick(t64);
        assert!(!g.on());
        assert_eq!(g.exit_reason(), "lease");
    }

    #[test]
    fn on_false_ends_it_and_keeps_it_off_until_the_game_leaves_the_front() {
        let mut g = game_rig();
        g.front(LUMAGAME, 3.0);
        let status = g.r.engine.game_request(false, LUMAGAME, g.r.t0 + s(4.0));
        assert_eq!((status["on"].clone(), status["lastExit"]["reason"].clone()), (json!(false), json!("off")));
        g.r.at(30.0);
        assert!(!g.on());
        g.front(HOME, 31.0);
        g.front(LUMAGAME, 32.0);
        assert!(g.on());
    }

    #[test]
    fn the_switch_off_keeps_it_from_starting_and_ends_it_at_once() {
        let mut g = game_rig();
        g.switch.store(false, Ordering::Relaxed);
        g.front(LUMAGAME, 3.0);
        assert!(!g.on() && g.r.lg.0.lock().closed.is_empty());
        assert_eq!(g.status()["switch"], false);
        assert_eq!(g.r.engine.game_request(true, LUMAGAME, g.r.t0 + s(3.5))["switch"], false);
        g.switch.store(true, Ordering::Relaxed);
        g.r.at(4.0);
        assert!(g.on());
        g.switch.store(false, Ordering::Relaxed);
        g.r.at(5.0);
        assert!(!g.on());
        assert_eq!(g.exit_reason(), "switch");
    }

    #[test]
    fn a_failed_step_ends_it_resumes_the_feeds_and_waits_for_the_game_to_leave() {
        let mut g = game_rig();
        g.r.os.0.lock().vm_fails = true;
        g.front(LUMAGAME, 3.0);
        g.exited(&[PLEX, GAME]);
        g.r.at(3.1);
        // The reclaim went through; the compaction was refused.
        assert!(g.pause.paused());
        g.r.at(3.2);
        assert!(!g.on() && !g.pause.paused());
        assert_eq!(g.exit_reason(), "failed");
        g.r.at(10.0);
        assert!(!g.on());

        // A close SAM refuses ends it before anything is paused.
        let mut g = game_rig();
        g.r.lg.0.lock().close_fails = true;
        g.front(LUMAGAME, 3.0);
        assert!(!g.on() && !g.pause.paused());
        assert_eq!(g.exit_reason(), "failed");
    }

    #[test]
    fn a_lost_subscription_ends_it_and_the_worker_stops_at_its_next_step() {
        let mut g = game_rig();
        type Queue = Mutex<Vec<Box<dyn FnOnce() + Send>>>;
        let queued: Arc<Queue> = Arc::default();
        {
            let queued = queued.clone();
            g.r.engine.run = Box::new(move |work| queued.lock().push(work));
        }
        g.front(LUMAGAME, 3.0);
        g.exited(&[PLEX, GAME]);
        g.r.at(3.1);
        assert!(g.pause.paused());
        g.r.engine.lost("test");
        assert!(!g.on() && !g.pause.paused());
        assert_eq!(g.exit_reason(), "subscription");
        for work in queued.lock().drain(..) {
            work();
        }
        assert!(g.r.os.0.lock().reclaimed.is_empty() && g.r.os.0.lock().vm.is_empty());
    }
}

mod ours {
    use super::*;
    use crate::app_priority::levels::PAGES_PER_MB as MB;
    use crate::app_priority::manager::Shim;
    use crate::app_priority::memchute::with;

    const LIVETV: &str = "com.webos.app.livetv";
    const YOUTUBE: &str = "youtube.leanback.v4";

    #[derive(Default)]
    struct FakeShim(Mutex<Vec<Value>>);

    impl Shim for FakeShim {
        fn send(&self, message: Value) {
            self.0.lock().push(message);
        }
    }

    impl FakeShim {
        fn kinds(&self, kind: &str) -> Vec<Value> {
            self.0.lock().iter().filter(|m| m["kind"] == kind).cloned().collect()
        }
        fn signals(&self, name: &str) -> Vec<Value> {
            self.kinds("signal").into_iter().filter(|m| m["name"] == name).map(|m| m["payload"].clone()).collect()
        }
    }

    struct Ours {
        r: Rig,
        shim: Arc<FakeShim>,
    }

    impl Ours {
        fn usable_at(&mut self, secs: f64, mb: u64) {
            self.r.os.0.lock().usable = mb * MB;
            self.r.at(secs);
        }
    }

    /// The rig with Live TV running behind, 400 MB usable and LumaTV's memory manager serving from 2.5 s.
    fn ours() -> Ours {
        let mut r = rig(Timing::default());
        {
            let mut w = r.os.0.lock();
            w.procs.insert(LIVETV.into(), vec![Pid { pid: 700, start: 1 }]);
            w.oom.insert(700, -1000);
            w.freezer.insert(LIVETV.into(), "THAWED".into());
            w.usable = 400 * MB;
        }
        {
            let mut sam = r.lg.0.lock();
            sam.running.push(LIVETV.into());
            sam.pids.insert(LIVETV.into(), vec![700]);
        }
        r.engine.memchute = Some(with(&[("KeepAlive", &[NETFLIX]), ("EfsTarget", &[YOUTUBE])]));
        let shim = Arc::new(FakeShim::default());
        r.engine.take_over(shim.clone(), r.t0 + s(2.5));
        Ours { r, shim }
    }

    #[test]
    fn a_launch_closes_third_party_apps_oldest_first_then_lgs_until_its_memory_is_free() {
        let mut o = ours();
        o.r.os.0.lock().usable = 100 * MB;
        let t = o.r.t0 + s(3.0);
        o.r.engine.launch(&json!({"appId": "com.example.browser", "requiredMemory": 160}), t);
        // One at a time: the next waits for the last one's processes.
        assert_eq!(o.r.lg.0.lock().closed, vec![PLEX]);
        o.r.engine.tick(t + s(0.05));
        assert_eq!(o.r.lg.0.lock().closed, vec![PLEX]);
        for i in 1..=10 {
            {
                let mut w = o.r.os.0.lock();
                for app in o.r.lg.0.lock().closed.iter() {
                    if w.procs.remove(app).is_some() {
                        w.usable += 70 * MB;
                    }
                }
            }
            o.r.engine.tick(t + s(0.1 * f64::from(i)));
        }
        // 160 + 120 MB was met after Live TV; Netflix (kept), Home, LumaGame and the game in front stay.
        assert_eq!(o.r.lg.0.lock().closed, vec![PLEX, TWITCH, LIVETV]);
        let closing: Vec<Value> = o.shim.kinds("closing").iter().map(|m| m["id"].clone()).collect();
        assert_eq!(closing, vec![json!(PLEX), json!(TWITCH), json!(LIVETV)]);
        assert_eq!(o.shim.signals("clearContainers").len(), 1);
        assert_eq!(o.r.lg.0.lock().toasts, vec![format!("Closed Plex, {TWITCH}, {LIVETV} to free memory for the app starting")]);
    }

    #[test]
    fn a_preload_or_a_launch_with_room_closes_nothing() {
        let mut o = ours();
        o.r.os.0.lock().usable = 100 * MB;
        o.r.engine.launch(&json!({"appId": YOUTUBE, "requiredMemory": 0, "preload": "semi-full"}), o.r.t0 + s(3.0));
        o.r.os.0.lock().usable = 241 * MB;
        o.r.engine.launch(&json!({"appId": "com.example.browser", "requiredMemory": 0}), o.r.t0 + s(3.5));
        o.r.at(4.0);
        assert!(o.r.lg.0.lock().closed.is_empty());
        assert!(o.r.os.0.lock().groups.contains(YOUTUBE));
    }

    #[test]
    fn levels_are_signalled_on_change_and_each_worsening_asks_for_one_trim() {
        let mut o = ours();
        o.usable_at(4.0, 300);
        o.usable_at(5.0, 240);
        o.usable_at(6.0, 280);
        o.usable_at(7.0, 190);
        o.usable_at(8.0, 300);
        let changes: Vec<Value> = o.shim.signals("thresholdChanged").iter().map(|p| p["current"].clone()).collect();
        assert_eq!(changes, vec![json!("medium"), json!("low"), json!("normal")]);
        assert_eq!(o.shim.signals("thresholdChanged")[0]["previous"], "normal");
        assert_eq!(o.r.os.0.lock().trims, 2);
        assert_eq!(o.shim.kinds("state").last().unwrap()["level"], "normal");
    }

    #[test]
    fn groups_hold_sams_processes_and_swappiness_follows_the_screen() {
        let mut o = ours();
        {
            let w = o.r.os.0.lock();
            assert!(APPS.iter().all(|(a, _)| w.groups.contains(*a)) && w.groups.contains(LIVETV));
            assert!(w.attached.contains(&(PLEX.into(), 100)));
            assert_eq!((w.swappiness[GAME], w.swappiness[PLEX], w.swappiness[HOME]), (10, 100, 100));
        }
        o.r.engine.front(vec![PLEX.into()], o.r.t0 + s(3.0));
        {
            let w = o.r.os.0.lock();
            assert_eq!((w.swappiness[GAME], w.swappiness[PLEX]), (100, 10));
        }
        // A process SAM names for two apps goes in neither; an app gone takes its groups with it.
        {
            let mut sam = o.r.lg.0.lock();
            sam.pids.insert(TWITCH.into(), vec![200, 900]);
            sam.pids.insert(GAME.into(), vec![600, 900]);
            sam.running.retain(|a| a != LIVETV);
        }
        o.r.engine.refresh(o.r.t0 + s(4.0));
        let w = o.r.os.0.lock();
        assert!(!w.attached.iter().any(|(_, p)| *p == 900));
        assert!(!w.groups.contains(LIVETV));
    }

    #[test]
    fn takeover_adopts_frozen_cached_apps_thaws_the_rest_and_stock_puts_everything_back() {
        let mut r = rig(Timing::default());
        {
            let mut w = r.os.0.lock();
            w.freezer.insert(TWITCH.into(), "FROZEN".into());
            w.freezer.insert(HOME.into(), "FROZEN".into());
            w.usable = 400 * MB;
        }
        r.engine.take_over(Arc::new(FakeShim::default()), r.t0 + s(13.0));
        assert_eq!(r.frozen(), vec![PLEX, TWITCH]);
        assert!(r.engine.held[TWITCH].frozen);
        r.engine.give_back("test");
        assert!(!r.engine.ours());
        r.assert_all_back();
    }

    #[test]
    fn preloads_wait_while_an_efs_app_is_in_front_told_on_change_only() {
        let mut o = ours();
        let t = o.r.t0;
        o.r.engine.front(vec![YOUTUBE.into()], t + s(3.0));
        o.r.at(3.5);
        assert_eq!(o.shim.kinds("state").last().unwrap()["limit"], true);
        o.r.engine.front(vec![YOUTUBE.into(), PLEX.into()], t + s(4.0));
        o.r.engine.front(vec![GAME.into()], t + s(5.0));
        let statuses: Vec<Value> = o.shim.kinds("outbound").iter().map(|m| m["payload"]["status"].clone()).collect();
        assert_eq!(statuses, vec![json!("start"), json!("limit"), json!("start")]);
    }
}
