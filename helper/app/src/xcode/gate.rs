//! When the transcoder may hold the video hardware.
//!
//! The decoder and the encoder it drives are the ones every app plays video with, and the kernel node it opens is not known to LG's media
//! server, so an app that opens a player while the transcoder holds them can find them taken. The gate watches what could need them and
//! closes the engine (`hold`) as soon as it hears of it, and opens it again when nothing does:
//!
//! - an app other than Home and LumaTV's own shell is in front or starting: SAM's `getForegroundAppInfo` and `getAppLifeStatus`, pushed, so
//!   the engine hears of it at the app's launch, before its player exists. A native game ([`GAMES`](crate::app_priority::game::GAMES)) whose own
//!   stream is up is the exception: its player is open, so what it holds is known and the pipelines decide;
//! - the media server's pipelines hold what the converter needs: `getActivePipelines` against the totals of `umediaserver_resource_config.txt`,
//!   looked at twice a second while a conversion runs and once before one starts. The converter needs one video decoder, one MPEG-2 core and
//!   the encoder ([`NEED`]) and gives way when any of them is all held, by whoever (Netflix in 4K holds every decoder; a game's 1080p stream
//!   does not);
//! - SAM cannot be asked, or its subscription ended: closed, since nothing could be heard.

use crate::app_priority::lg;
use crate::luna::{self, Follow};
use parking_lot::Mutex;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How often a running conversion's pipelines are looked at, launches that never came up are forgotten and a lost subscription is renewed.
const LOOK: Duration = Duration::from_millis(500);
/// How long a launch that has not reached the front keeps the gate closed (the grace `app_priority` gives it).
const LAUNCH_GRACE: Duration = Duration::from_secs(5);
/// `getAppLifeStatus` words for an app on its way up. Not `preloading`: LG keeps apps preloaded behind Home, and they hold no decoder.
const STARTING: [&str; 3] = ["launching", "relaunching", "splash"];
/// Where the media server's totals of its resources are read.
const UMS_CONFIG: &str = "/etc/umediaserver/umediaserver_resource_config.txt";
/// The totals of this set (`VDEC` 4, `G1_MSVC` 4, `VENC` 1), for a config that cannot be read.
const TOTALS: [(&str, u32); 3] = [("VDEC", 4), ("G1_MSVC", 4), ("VENC", 1)];
/// What the converter takes of the media server's counted resources: a video decoder, the MPEG-2 core and the encoder. The nodes it opens are not
/// counted by the server, so nothing is held for it: the units have to be free when it starts and to stay free while it runs.
const NEED: [(&str, u32); 3] = [("VDEC", 1), ("G1_MSVC", 1), ("VENC", 1)];

/// Units of the media server's resources, by name.
type Units = BTreeMap<String, u32>;

/// What the gate tells the engine: why it is closed, or `None` when it is open.
pub type Apply = Box<dyn Fn(Option<String>) + Send + Sync>;

/// A media pipeline: the app it plays for (empty when it names none) and the counted units it holds.
#[derive(Debug, PartialEq, Eq)]
struct Pipeline {
    app: String,
    held: Units,
}

/// What the gate has heard of the TV.
#[derive(Default)]
struct Seen {
    /// The apps on screen, the primary first; `None` until SAM has said, and again when it cannot be heard.
    front: Option<Vec<String>>,
    /// Apps SAM is starting, and when it said so.
    starting: BTreeMap<String, Instant>,
    /// The media server's pipelines at the last look; `None` before the first.
    pipelines: Option<Vec<Pipeline>>,
    /// What was last told to the engine; `None` before the first time.
    said: Option<Option<String>>,
}

impl Seen {
    /// Why the video hardware is not the transcoder's now, in words for the owner's page.
    fn closed(&self, rules: &Rules, now: Instant) -> Option<String> {
        let Some(front) = &self.front else {
            return Some("the foreground app is not known".to_owned());
        };
        // an app in front may open a player at any moment, so it closes the gate, but for a game whose stream is up already
        let streaming = |app: &String| self.pipelines.iter().flatten().any(|p| p.app == *app && p.held.get("VDEC").is_some_and(|n| *n > 0));
        if let Some(app) = front.iter().find(|app| !rules.ours.contains(app) && !(rules.games.contains(app) && streaming(app))) {
            return Some(format!("{app} is in front"));
        }
        let starting = self
            .starting
            .iter()
            .find(|(app, since)| !rules.ours.contains(app) && now.saturating_duration_since(**since) < LAUNCH_GRACE);
        if let Some((app, _)) = starting {
            return Some(format!("{app} is starting"));
        }
        self.short(rules)
    }

    /// The first resource of [`NEED`] that the pipelines hold too much of for the converter to have its units, in words for the owner's page.
    fn short(&self, rules: &Rules) -> Option<String> {
        let pipelines = self.pipelines.as_ref()?;
        NEED.iter().find_map(|(name, need)| {
            // a resource the config does not count cannot run short
            let total = *rules.total.get(*name)?;
            let held: u32 = pipelines.iter().filter_map(|p| p.held.get(*name)).sum();
            if held + need <= total {
                return None;
            }
            let holder = pipelines.iter().filter(|p| p.held.contains_key(*name)).max_by_key(|p| p.held[*name]).map_or("an app", |p| if p.app.is_empty() { "a player" } else { &p.app });
            let what = match *name {
                "VDEC" => "video decoders",
                "G1_MSVC" => "MPEG-2 decoder cores",
                _ => "video encoder",
            };
            Some(format!("{holder} holds the {what} ({held} of {total})"))
        })
    }
}

/// The pipelines of a `getActivePipelines` answer: a list of `{currentAppId, resource: [{resource}]}`, each unit of a resource one entry (`VDEC` twice
/// is two decoders; a pipeline of the video tap holds none). `None`: the answer was not a list of pipelines.
fn pipelines(reply: &Value) -> Option<Vec<Pipeline>> {
    let list = match reply {
        Value::Array(list) => list,
        other => other.get("pipelines")?.as_array()?,
    };
    Some(
        list.iter()
            .map(|pipeline| {
                let mut held = Units::new();
                for name in pipeline.get("resource").and_then(Value::as_array).into_iter().flatten().filter_map(|r| r.get("resource")?.as_str()) {
                    *held.entry(name.to_owned()).or_default() += 1;
                }
                Pipeline { app: pipeline.get("currentAppId").and_then(Value::as_str).unwrap_or("").to_owned(), held }
            })
            .collect(),
    )
}

/// The media server's totals: [`TOTALS`], and what its config says (`id = "VDEC"; ... qty = 4;` in each resource's block) over them.
fn totals(config: &str) -> Units {
    fn value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
        Some(line.strip_prefix(key)?.trim_start().strip_prefix('=')?.trim().trim_end_matches(';').trim())
    }
    let mut total: Units = TOTALS.iter().map(|(name, qty)| ((*name).to_owned(), *qty)).collect();
    let mut id = None;
    for line in config.lines().map(str::trim) {
        if let Some(name) = value(line, "id") {
            id = Some(name.trim_matches('"').to_owned());
        } else if let Some(qty) = value(line, "qty").and_then(|q| q.parse().ok()) {
            if let Some(id) = id.take() {
                total.insert(id, qty);
            }
        }
    }
    total
}

/// What the gate judges by.
struct Rules {
    /// The apps that may be in front while the transcoder runs: Home and LumaTV's shell.
    ours: Vec<String>,
    /// The native games: in front, they close the gate only until their stream is up.
    games: Vec<String>,
    /// What the media server counts of each resource.
    total: Units,
}

struct Inner {
    luna: String,
    rules: Rules,
    seen: Mutex<Seen>,
    apply: Apply,
    /// Whether a conversion is running or about to start, which is when the media server is worth asking.
    busy: Box<dyn Fn() -> bool + Send + Sync>,
}

impl Inner {
    /// Tells the engine what the gate now says, when that changed. Under the lock, so that the engine hears the changes in their order.
    fn settle(&self, seen: &mut Seen, now: Instant) {
        let closed = seen.closed(&self.rules, now);
        if seen.said.as_ref() != Some(&closed) {
            seen.said = Some(closed.clone());
            (self.apply)(closed);
        }
    }

    fn front(&self, ids: Vec<String>, now: Instant) {
        let mut seen = self.seen.lock();
        // an app that came to the front has started
        seen.starting.retain(|app, _| !ids.contains(app));
        seen.front = Some(ids);
        self.settle(&mut seen, now);
    }

    fn life(&self, app: &str, status: &str, now: Instant) {
        let mut seen = self.seen.lock();
        if STARTING.contains(&status) {
            seen.starting.insert(app.to_owned(), now);
        } else {
            seen.starting.remove(app);
        }
        self.settle(&mut seen, now);
    }

    /// The subscriptions are gone: nothing can be heard of the apps.
    fn deaf(&self, now: Instant) {
        let mut seen = self.seen.lock();
        seen.front = None;
        self.settle(&mut seen, now);
    }

    /// Looks at the media server's pipelines once. An answer that cannot be read leaves what was seen.
    fn look(&self, now: Instant) {
        let Some(reply) = luna::once(&self.luna, "luna://com.webos.media/getActivePipelines", "{}") else {
            return;
        };
        if let Some(list) = pipelines(&reply) {
            let mut seen = self.seen.lock();
            seen.pipelines = Some(list);
            self.settle(&mut seen, now);
        }
    }

    /// The pass the watching thread makes each [`LOOK`]: launches that never came up are forgotten, and a conversion's pipelines looked at.
    fn tick(&self, now: Instant) {
        if (self.busy)() {
            self.look(now);
        }
        let mut seen = self.seen.lock();
        seen.starting.retain(|_, since| now.saturating_duration_since(*since) < LAUNCH_GRACE);
        self.settle(&mut seen, now);
    }
}

/// The two subscriptions to SAM, renewed when one ends.
struct Subscriptions {
    front: Option<Follow>,
    life: Option<Follow>,
}

impl Subscriptions {
    fn start(inner: &Arc<Inner>) -> io::Result<Self> {
        let (front_to, life_to) = (inner.clone(), inner.clone());
        let front = Follow::start(
            "xcode-front",
            &inner.luna,
            lg::FRONT_URI,
            lg::FRONT_PAYLOAD,
            Box::new(move |reply| {
                if let Some(ids) = lg::front(reply) {
                    front_to.front(ids, Instant::now());
                }
            }),
        )?;
        let life = Follow::start(
            "xcode-life",
            &inner.luna,
            lg::LIFE_URI,
            lg::LIFE_PAYLOAD,
            Box::new(move |reply| {
                if let Some((app, status)) = lg::life(reply) {
                    life_to.life(&app, &status, Instant::now());
                }
            }),
        )?;
        Ok(Self { front: Some(front), life: Some(life) })
    }

    /// Starts again whichever subscription ended; while one is gone the gate is closed.
    fn renew(&mut self, inner: &Arc<Inner>) {
        if self.front.as_ref().is_some_and(Follow::alive) && self.life.as_ref().is_some_and(Follow::alive) {
            return;
        }
        inner.deaf(Instant::now());
        // both again: a reply of the one that lived is a repeat of what it said, and a subscription's first reply is where things stand
        self.front = None;
        self.life = None;
        if let Ok(fresh) = Self::start(inner) {
            *self = fresh;
        }
    }
}

/// The watch over the TV for the transcoder, running until it is dropped.
pub struct Gate {
    inner: Arc<Inner>,
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Gate {
    /// Starts watching and tells `apply` what it finds, first of all how things stand now. `ours` are the apps allowed in front and `games` the
    /// ones that may be while their stream is up; `busy` says whether a conversion runs or is about to start. `luna` is the `luna-send` program.
    pub fn start(luna: &str, ours: Vec<String>, games: Vec<String>, apply: Apply, busy: Box<dyn Fn() -> bool + Send + Sync>) -> io::Result<Self> {
        let total = totals(&std::fs::read_to_string(UMS_CONFIG).unwrap_or_default());
        let inner = Arc::new(Inner { luna: luna.to_owned(), rules: Rules { ours, games, total }, seen: Mutex::default(), apply, busy });
        // nothing is known yet, and nothing may start until something is: the engine hears so before the first answer comes
        inner.deaf(Instant::now());
        let mut subscriptions = Subscriptions::start(&inner)?;
        let (stop, rx) = mpsc::channel::<()>();
        let watching = inner.clone();
        let thread = thread::Builder::new().name("xcode-gate".to_owned()).stack_size(256 * 1024).spawn(move || {
            while matches!(rx.recv_timeout(LOOK), Err(RecvTimeoutError::Timeout)) {
                subscriptions.renew(&watching);
                watching.tick(Instant::now());
            }
        })?;
        Ok(Self { inner, stop: Some(stop), thread: Some(thread) })
    }

    /// A handle that has the media server looked at on demand, for a conversion about to start.
    pub fn looker(&self) -> Look {
        Look(self.inner.clone())
    }
}

/// Looks at the media server now: the engine's gate has the answer when [`Look::now`] returns.
#[derive(Clone)]
pub struct Look(Arc<Inner>);

impl Look {
    pub fn now(&self) {
        self.0.look(Instant::now());
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        self.stop = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    const HOME: &str = "com.webos.app.home";
    const SHELL: &str = "org.nphil.lumatv";
    const GAME: &str = "org.nphil.lumagame";

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn rules() -> Rules {
        Rules { ours: ids(&[HOME, SHELL]), games: ids(&[GAME]), total: totals("") }
    }

    /// The gate over no TV: what it tells the engine is collected.
    fn gate() -> (Inner, Arc<Mutex<Vec<Option<String>>>>) {
        let told: Arc<Mutex<Vec<Option<String>>>> = Arc::default();
        let into = told.clone();
        let inner = Inner {
            luna: String::new(),
            rules: rules(),
            seen: Mutex::default(),
            apply: Box::new(move |why| into.lock().push(why)),
            busy: Box::new(|| false),
        };
        (inner, told)
    }

    /// A pipeline of `app` holding `held` units of each resource named, as `getActivePipelines` lists it.
    fn pipeline(app: &str, held: &[(&str, u32)]) -> Value {
        let units: Vec<Value> = held.iter().flat_map(|(name, qty)| (0..*qty).map(move |index| json!({"index": index, "resource": *name}))).collect();
        json!({"currentAppId": app, "type": "media", "resource": units})
    }

    /// Why the gate is closed with `front` on screen and the media server answering `reply`, if it is.
    fn closed(front: &[&str], reply: &Value) -> Option<String> {
        Seen { front: Some(ids(front)), pipelines: pipelines(reply), ..Seen::default() }.closed(&rules(), Instant::now())
    }

    #[test]
    fn it_is_closed_until_the_front_is_known_then_open_for_home_and_closed_for_any_other_app() {
        let (gate, told) = gate();
        let t0 = Instant::now();
        gate.deaf(t0);
        gate.front(ids(&[HOME]), t0);
        gate.front(ids(&[SHELL]), t0);
        gate.front(ids(&["youtube.leanback.v4"]), t0);
        gate.front(ids(&["youtube.leanback.v4", HOME]), t0);
        gate.front(vec![], t0);
        assert_eq!(
            *told.lock(),
            [
                Some("the foreground app is not known".to_owned()),
                None,
                Some("youtube.leanback.v4 is in front".to_owned()),
                None,
            ],
            "told on change only"
        );
    }

    #[test]
    fn a_launch_closes_it_before_the_app_is_in_front_and_is_forgotten_if_the_app_never_comes() {
        let (gate, told) = gate();
        let t0 = Instant::now();
        gate.front(ids(&[HOME]), t0);
        gate.life("netflix", "launching", t0);
        gate.life("com.webos.app.livetv", "preloading", t0);
        gate.life(HOME, "relaunching", t0);
        gate.tick(t0 + Duration::from_secs(4));
        gate.tick(t0 + LAUNCH_GRACE);
        gate.life("netflix", "launching", t0 + LAUNCH_GRACE);
        gate.front(ids(&["netflix"]), t0 + LAUNCH_GRACE);
        gate.life("netflix", "foreground", t0 + LAUNCH_GRACE);
        gate.front(ids(&[HOME]), t0 + LAUNCH_GRACE);
        assert_eq!(
            *told.lock(),
            [
                None,
                Some("netflix is starting".to_owned()),
                None,
                Some("netflix is starting".to_owned()),
                Some("netflix is in front".to_owned()),
                None,
            ],
            "a preloaded app and Home's own relaunch change nothing; the grace ends the closed state"
        );
    }

    #[test]
    fn the_pipelines_and_the_totals_are_read_as_the_media_server_words_them() {
        let reply = json!([
            {"currentAppId": "netflix", "type": "media", "resource": [{"index": 1, "resource": "ADEC"}, {"index": 0, "resource": "VDEC"}, {"index": 1, "resource": "VDEC"}]},
            {"type": "vtclient", "resource": []},
        ]);
        let list = pipelines(&reply).unwrap();
        assert_eq!(list[0], Pipeline { app: "netflix".to_owned(), held: Units::from([("ADEC".to_owned(), 1), ("VDEC".to_owned(), 2)]) });
        assert_eq!(list[1], Pipeline { app: String::new(), held: Units::new() });
        assert_eq!(pipelines(&json!({"pipelines": []})), Some(vec![]));
        assert_eq!(pipelines(&json!({"returnValue": false})), None, "an error is not an empty list");

        let config = "version = \"1.0\";\nresources = (\n\t{\n\t\tid = \"VDEC\";\n\t\tname = \"Digital Video Decoder\";\n\t\tqty = 2;\n\t},\n\t{\n\t\tid = \"ATU\";\n\t\tname = \"Analog Tuner\";\n\t\tqty = 7;\n\t},\n);\n";
        assert_eq!(totals(config), Units::from([("ATU".to_owned(), 7), ("G1_MSVC".to_owned(), 4), ("VDEC".to_owned(), 2), ("VENC".to_owned(), 1)]));
        assert_eq!(totals(""), TOTALS.iter().map(|(name, qty)| ((*name).to_owned(), *qty)).collect::<Units>(), "a config that cannot be read leaves this set's totals");
    }

    #[test]
    fn a_pipeline_closes_it_only_when_it_leaves_no_decoder_core_or_encoder_for_the_converter() {
        let at_home = |reply: Value| closed(&[HOME], &reply);
        assert_eq!(at_home(json!([])), None);
        let youtube = pipeline("youtube.leanback.v4", &[("VDEC", 2), ("G1_MSVC", 2)]);
        assert_eq!(at_home(json!([youtube])), None, "two of the four decoders and cores leave it room");
        let three = json!([pipeline("a", &[("VDEC", 1)]), pipeline("b", &[("VDEC", 1)]), pipeline("c", &[("VDEC", 1)])]);
        assert_eq!(at_home(three), None, "the last decoder is its");
        let four = json!([pipeline("a", &[("VDEC", 1)]), pipeline("b", &[("VDEC", 3)])]);
        assert_eq!(at_home(four), Some("b holds the video decoders (4 of 4)".to_owned()));
        assert_eq!(at_home(json!([pipeline("com.webos.app.livetv", &[("G1_MSVC", 4)])])), Some("com.webos.app.livetv holds the MPEG-2 decoder cores (4 of 4)".to_owned()));
        assert_eq!(at_home(json!([pipeline("com.webos.app.camera", &[("VENC", 1)])])), Some("com.webos.app.camera holds the video encoder (1 of 1)".to_owned()));
        assert_eq!(at_home(json!([pipeline(SHELL, &[("VDEC", 4)])])), Some("org.nphil.lumatv holds the video decoders (4 of 4)".to_owned()), "the units are the set's, whoever holds them");
        assert_eq!(at_home(json!([{"resource": [{"resource": "VDEC"}, {"resource": "VDEC"}, {"resource": "VDEC"}, {"resource": "VDEC"}]}])), Some("a player holds the video decoders (4 of 4)".to_owned()));
        assert_eq!(at_home(json!([pipeline("x", &[("VDEC_BUS", 6), ("ADEC", 4)])])), None, "what the converter does not use is not counted");
    }

    #[test]
    fn a_game_in_front_closes_it_until_its_own_stream_is_up_and_then_the_units_decide() {
        let stream = |units: &[(&str, u32)]| json!([pipeline(GAME, units)]);
        let menu = Some("org.nphil.lumagame is in front".to_owned());
        assert_eq!(closed(&[GAME], &json!([])), menu, "in its menu the game may open its player at any moment");
        assert_eq!(closed(&[GAME], &json!([pipeline("netflix", &[("VDEC", 2)])])), menu, "another app's stream is not the game's");
        assert_eq!(closed(&[GAME], &stream(&[("VDEC", 2), ("G1_H264", 2)])), None);
        assert_eq!(closed(&[GAME], &stream(&[("VDEC", 4)])), Some("org.nphil.lumagame holds the video decoders (4 of 4)".to_owned()), "a 4K stream takes every decoder");
        assert_eq!(closed(&["netflix"], &json!([pipeline("netflix", &[("VDEC", 2)])])), Some("netflix is in front".to_owned()), "only the games are let through");
        assert_eq!(closed(&[GAME, "netflix"], &stream(&[("VDEC", 2)])), Some("netflix is in front".to_owned()));
        let unseen = Seen { front: Some(ids(&[GAME])), ..Seen::default() };
        assert_eq!(unseen.closed(&rules(), Instant::now()), menu, "before the first look nothing is known of its stream");

        let t0 = Instant::now();
        let launching = Seen { front: Some(ids(&[GAME])), pipelines: pipelines(&stream(&[("VDEC", 2)])), starting: BTreeMap::from([("netflix".to_owned(), t0)]), ..Seen::default() };
        assert_eq!(launching.closed(&rules(), t0), Some("netflix is starting".to_owned()), "a launch closes it beside a game too");
    }

    #[test]
    fn what_a_look_finds_reaches_the_engine_on_change_only() {
        let (gate, told) = gate();
        let t0 = Instant::now();
        gate.front(ids(&[HOME]), t0);
        let netflix = json!([pipeline("netflix", &[("VDEC", 4)])]);
        for reply in [&netflix, &netflix, &json!([])] {
            let mut seen = gate.seen.lock();
            seen.pipelines = pipelines(reply);
            gate.settle(&mut seen, t0);
        }
        assert_eq!(*told.lock(), [None, Some("netflix holds the video decoders (4 of 4)".to_owned()), None]);
    }
}
