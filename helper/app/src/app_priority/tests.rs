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
}

#[derive(Default)]
struct Sam {
    running: Vec<String>,
    playing: BTreeSet<String>,
    closed: Vec<String>,
    close_fails: bool,
    playing_unknown: bool,
    toasts: Vec<String>,
}

#[derive(Default)]
struct FakeLg(Mutex<Sam>);

impl Lg for FakeLg {
    fn running(&self) -> Option<Vec<String>> {
        Some(self.0.lock().running.clone())
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
    lg.0.lock().running = APPS.iter().map(|(a, _)| (*a).to_owned()).collect();
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
