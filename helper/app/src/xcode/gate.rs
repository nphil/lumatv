//! When the transcoder may hold the video hardware.
//!
//! The decoder and the encoder it drives are the ones every app plays video with, and the kernel node it opens is not known to LG's media
//! server, so an app that opens a player while the transcoder holds them can find them taken. The gate watches what could need them and
//! closes the engine (`hold`) as soon as it hears of it, and opens it again when nothing does:
//!
//! - an app other than Home and LumaTV's own shell is in front or starting: SAM's `getForegroundAppInfo` and `getAppLifeStatus`, pushed, so
//!   the engine hears of it at the app's launch, before its player exists;
//! - a media pipeline of such an app holds a video decoder: `getActivePipelines`, looked at twice a second while a conversion runs and once
//!   before one starts, which catches a decoder taken in the background;
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

/// What the gate tells the engine: why it is closed, or `None` when it is open.
pub type Apply = Box<dyn Fn(Option<String>) + Send + Sync>;

/// What the gate has heard of the TV.
#[derive(Default)]
struct Seen {
    /// The apps on screen, the primary first; `None` until SAM has said, and again when it cannot be heard.
    front: Option<Vec<String>>,
    /// Apps SAM is starting, and when it said so.
    starting: BTreeMap<String, Instant>,
    /// The holder of a video decoder at the last look at the media server.
    decoding: Option<String>,
    /// What was last told to the engine; `None` before the first time.
    said: Option<Option<String>>,
}

impl Seen {
    /// Why the video hardware is not the transcoder's now, in words for the owner's page.
    fn closed(&self, ours: &[String], now: Instant) -> Option<String> {
        let Some(front) = &self.front else {
            return Some("the foreground app is not known".to_owned());
        };
        if let Some(app) = front.iter().find(|app| !ours.contains(app)) {
            return Some(format!("{app} is in front"));
        }
        let starting = self
            .starting
            .iter()
            .find(|(app, since)| !ours.contains(app) && now.saturating_duration_since(**since) < LAUNCH_GRACE);
        if let Some((app, _)) = starting {
            return Some(format!("{app} is starting"));
        }
        self.decoding.as_ref().map(|who| format!("{who} is playing video"))
    }
}

/// The app whose media pipeline holds a video decoder, apart from `ours`. `getActivePipelines` answers a list of
/// `{currentAppId, resource: [{resource}]}` (`VDEC` is a decoder; a pipeline of the video tap holds none). The outer `None`: the answer was not
/// a list of pipelines.
fn decoding(reply: &Value, ours: &[String]) -> Option<Option<String>> {
    let list = match reply {
        Value::Array(list) => list,
        other => other.get("pipelines")?.as_array()?,
    };
    Some(list.iter().find_map(|pipeline| {
        let holds = pipeline.get("resource")?.as_array()?.iter().filter_map(|r| r.get("resource")?.as_str()).any(|r| r.starts_with("VDEC"));
        let app = pipeline.get("currentAppId").and_then(Value::as_str).unwrap_or("");
        (holds && !ours.iter().any(|o| o == app)).then(|| if app.is_empty() { "a player".to_owned() } else { app.to_owned() })
    }))
}

struct Inner {
    luna: String,
    /// The apps that may be in front while the transcoder runs: Home and LumaTV's shell.
    ours: Vec<String>,
    seen: Mutex<Seen>,
    apply: Apply,
    /// Whether a conversion is running or about to start, which is when the media server is worth asking.
    busy: Box<dyn Fn() -> bool + Send + Sync>,
}

impl Inner {
    /// Tells the engine what the gate now says, when that changed. Under the lock, so that the engine hears the changes in their order.
    fn settle(&self, seen: &mut Seen, now: Instant) {
        let closed = seen.closed(&self.ours, now);
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
        if let Some(holder) = decoding(&reply, &self.ours) {
            let mut seen = self.seen.lock();
            seen.decoding = holder;
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
    /// Starts watching and tells `apply` what it finds, first of all how things stand now. `ours` are the apps allowed in front; `busy`
    /// says whether a conversion runs or is about to start. `luna` is the `luna-send` program.
    pub fn start(luna: &str, ours: Vec<String>, apply: Apply, busy: Box<dyn Fn() -> bool + Send + Sync>) -> io::Result<Self> {
        let inner = Arc::new(Inner { luna: luna.to_owned(), ours, seen: Mutex::default(), apply, busy });
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

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    /// The gate over no TV: what it tells the engine is collected.
    fn gate() -> (Inner, Arc<Mutex<Vec<Option<String>>>>) {
        let told: Arc<Mutex<Vec<Option<String>>>> = Arc::default();
        let into = told.clone();
        let inner = Inner {
            luna: String::new(),
            ours: ids(&[HOME, SHELL]),
            seen: Mutex::default(),
            apply: Box::new(move |why| into.lock().push(why)),
            busy: Box::new(|| false),
        };
        (inner, told)
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
    fn a_pipeline_of_another_app_holding_a_video_decoder_closes_it_and_the_apps_own_do_not() {
        let ours = ids(&[HOME, SHELL]);
        let pipelines = json!([
            {"currentAppId": "tv.twitch", "type": "avconnector", "resource": [{"index": 0, "resource": "MAIN_SOUND"}]},
            {"type": "vtclient", "resource": []},
            {"currentAppId": HOME, "type": "media", "resource": [{"index": 0, "resource": "VDEC"}]},
        ]);
        assert_eq!(decoding(&pipelines, &ours), Some(None));
        let playing = json!([
            {"currentAppId": "netflix", "type": "media", "resource": [{"index": 1, "resource": "ADEC"}, {"index": 3, "resource": "VDEC"}]},
        ]);
        assert_eq!(decoding(&playing, &ours), Some(Some("netflix".to_owned())));
        let nameless = json!({"pipelines": [{"resource": [{"resource": "VDEC_BUS"}]}]});
        assert_eq!(decoding(&nameless, &ours), Some(Some("a player".to_owned())));
        assert_eq!(decoding(&json!({"returnValue": false}), &ours), None, "an error is not an empty list");

        let (gate, told) = gate();
        let t0 = Instant::now();
        gate.front(ids(&[HOME]), t0);
        let mut seen = gate.seen.lock();
        seen.decoding = Some("netflix".to_owned());
        gate.settle(&mut seen, t0);
        seen.decoding = None;
        gate.settle(&mut seen, t0);
        drop(seen);
        assert_eq!(*told.lock(), [None, Some("netflix is playing video".to_owned()), None]);
    }
}
