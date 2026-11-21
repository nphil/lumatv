use super::*;
use std::sync::Arc;

#[derive(Default)]
struct State {
    mode: String,
    input: String,
    range: String,
    /// Mode, key, value: what the service holds.
    values: BTreeMap<String, BTreeMap<String, String>>,
    mode_writes: Vec<String>,
    key_writes: usize,
}

#[derive(Default)]
struct Tv(Mutex<State>);

impl Settings for Arc<Tv> {
    fn call(&self, uri: &str, payload: &Value) -> Result<Value, String> {
        let mut s = self.0.lock();
        let dim = payload.get("dimension");
        if uri == READ_URI {
            let Some(mode) = dim.and_then(|d| d["pictureMode"].as_str()) else {
                return Ok(json!({"returnValue": true, "settings": {"pictureMode": s.mode}, "dimension": {"input": s.input, "dynamicRange": s.range}}));
            };
            let have = s.values.get(mode).cloned().unwrap_or_default();
            let keys: Vec<&str> = payload["keys"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
            let settings: Map<String, Value> = keys.iter().filter_map(|k| have.get(*k).map(|v| ((*k).to_owned(), json!(v)))).collect();
            let missing: Vec<&&str> = keys.iter().filter(|k| !have.contains_key(**k)).collect();
            return Ok(json!({"returnValue": missing.is_empty(), "settings": settings, "errorKey": missing}));
        }
        let settings = payload["settings"].as_object().unwrap();
        match dim.and_then(|d| d["pictureMode"].as_str()) {
            Some(mode) => {
                s.key_writes += 1;
                let entry = s.values.entry(mode.to_owned()).or_default();
                for (k, v) in settings {
                    entry.insert(k.clone(), v.as_str().unwrap().to_owned());
                }
            }
            None => {
                let mode = settings["pictureMode"].as_str().unwrap().to_owned();
                s.mode.clone_from(&mode);
                s.mode_writes.push(mode);
            }
        }
        Ok(json!({"returnValue": true}))
    }
}

fn ctx(mode: &str) -> Context {
    Context { mode: mode.into(), input: APPS_INPUT.into(), range: "sdr".into() }
}

fn film(app: &str) -> Option<Video> {
    Some(Video { app: app.into(), hdr: "none".into(), full: true })
}

const MARQUEE: &str = "com.beb.plxnative";

struct Rig {
    tv: Arc<Tv>,
    e: Engine,
    t0: Instant,
}

impl Rig {
    /// The TV in `mode`, nothing playing, the slots `ui` and `video` saved.
    fn new(mode: &str, ui: Option<&str>, video: Option<&str>) -> Self {
        let tv = Arc::new(Tv::default());
        {
            let mut s = tv.0.lock();
            s.mode = mode.into();
            s.input = APPS_INPUT.into();
            s.range = "sdr".into();
        }
        let saved = Saved { ui: ui.map(Into::into), video: video.map(Into::into), ..Saved::default() };
        let mut e = Engine::new(Box::new(tv.clone()), saved, None, vec!["com.brand.app".into()], true);
        let t0 = Instant::now();
        e.picture(t0, ctx(mode));
        e.video(t0, None);
        Self { tv, e, t0 }
    }

    fn at(&self, ms: u64) -> Instant {
        self.t0 + Duration::from_millis(ms)
    }

    fn tick(&mut self, ms: u64) {
        self.e.tick(self.at(ms));
    }

    /// The subscription reporting the mode the TV is in now.
    fn echo(&mut self, ms: u64) {
        let mode = self.tv.0.lock().mode.clone();
        self.e.picture(self.at(ms), ctx(&mode));
    }

    fn writes(&self) -> Vec<String> {
        self.tv.0.lock().mode_writes.clone()
    }
}

#[test]
fn playback_switches_to_the_video_slot_and_back() {
    let mut r = Rig::new("filmMaker", Some("eco"), Some("filmMaker"));
    r.tick(0);
    r.echo(100);
    assert_eq!(r.writes(), ["eco"]);
    r.e.video(r.at(10_000), film(MARQUEE));
    r.tick(11_900);
    assert_eq!(r.writes(), ["eco"], "under PLAY");
    r.tick(12_000);
    r.echo(12_100);
    assert_eq!(r.writes(), ["eco", "filmMaker"]);
    assert_eq!(r.e.active(), Some(Slot::Video));
    r.e.video(r.at(20_000), None);
    r.tick(22_900);
    assert_eq!(r.writes().len(), 2, "under STOP");
    r.tick(23_000);
    assert_eq!(r.writes(), ["eco", "filmMaker", "eco"]);
}

#[test]
fn short_windowed_or_own_video_changes_nothing() {
    let mut r = Rig::new("eco", Some("eco"), Some("filmMaker"));
    r.e.video(r.at(1_000), film(MARQUEE));
    r.e.video(r.at(2_500), None);
    r.tick(10_000);
    r.e.video(r.at(11_000), Some(Video { app: MARQUEE.into(), hdr: "none".into(), full: false }));
    r.tick(20_000);
    for own in ["org.nphil.lumatv", "com.brand.app", "com.webos.app.home"] {
        r.e.video(r.at(21_000), film(own));
        r.tick(30_000);
    }
    assert!(r.writes().is_empty());
}

#[test]
fn hdr_is_left_to_lg() {
    let mut r = Rig::new("eco", Some("eco"), Some("filmMaker"));
    r.e.video(r.at(1_000), Some(Video { app: MARQUEE.into(), hdr: "HDR10".into(), full: true }));
    r.e.picture(r.at(1_200), Context { mode: "hdrFilmMaker".into(), input: APPS_INPUT.into(), range: "hdr".into() });
    r.tick(10_000);
    assert!(r.writes().is_empty());
    assert_eq!(r.e.status()["hold"], "hdr");
    assert_eq!(r.e.saved().video.as_deref(), Some("filmMaker"), "an HDR mode is never a slot's");
}

#[test]
fn games_are_left_alone() {
    let mut r = Rig::new("filmMaker", Some("eco"), Some("filmMaker"));
    r.e.set_game(true);
    r.tick(5_000);
    r.e.picture(r.at(6_000), ctx("cinema"));
    r.tick(10_000);
    assert!(r.writes().is_empty());
    assert_eq!(r.e.saved().ui.as_deref(), Some("eco"), "a change in a game is not learned");
    r.e.set_game(false);
    r.e.picture(r.at(11_000), ctx("game"));
    r.tick(20_000);
    assert!(r.writes().is_empty(), "LG's Game mode (ALLM) stays");
}

#[test]
fn a_manual_change_becomes_the_slot_choice() {
    let mut r = Rig::new("eco", Some("eco"), Some("filmMaker"));
    r.e.picture(r.at(1_000), ctx("cinema"));
    assert_eq!(r.e.saved().ui.as_deref(), Some("cinema"));
    r.e.video(r.at(2_000), film(MARQUEE));
    // LG's own switch as the film starts (Filmmaker Auto Start), the echo of LumaTV's write, and a change while the switch settles
    // are not choices
    r.e.picture(r.at(2_500), ctx("expert1"));
    assert_eq!(r.e.saved().ui.as_deref(), Some("cinema"));
    r.tick(4_000);
    r.echo(4_200);
    r.e.picture(r.at(6_000), ctx("expert2"));
    assert_eq!(r.e.saved().video.as_deref(), Some("filmMaker"));
    r.e.picture(r.at(10_000), ctx("normal"));
    assert_eq!(r.e.saved().video.as_deref(), Some("normal"));
    assert_eq!(r.e.saved().ui.as_deref(), Some("cinema"));
}

#[test]
fn writes_are_rate_limited_and_back_off() {
    let mut r = Rig::new("filmMaker", Some("eco"), Some("filmMaker"));
    // the mode is taken away by a context change each time: never learned, enforced again each time it is allowed
    for i in 0..30u64 {
        r.tick(i * 1_000);
        r.echo(i * 1_000 + 100);
        r.e.picture(r.at(i * 1_000 + 200), Context { mode: "filmMaker".into(), input: "hdmi2".into(), range: "sdr".into() });
        r.tv.0.lock().mode = "filmMaker".into();
        r.e.picture(r.at(i * 1_000 + 300), ctx("filmMaker"));
    }
    assert_eq!(r.writes().len(), PER_MINUTE);

    // writes that never show stop after three for BACKOFF
    let mut r = Rig::new("filmMaker", Some("eco"), Some("filmMaker"));
    for s in 0..30u64 {
        r.tv.0.lock().mode = "filmMaker".into();
        r.tick(s * 1_000);
    }
    assert_eq!(r.writes().len(), 3);
    assert!(r.e.status()["error"].is_string());
}

#[test]
fn turning_separate_off_puts_the_video_mode_back() {
    let mut r = Rig::new("eco", Some("eco"), Some("filmMaker"));
    r.e.set_separate(r.at(1_000), false);
    r.echo(1_100);
    assert_eq!(r.writes(), ["filmMaker"]);
    r.e.video(r.at(2_000), film(MARQUEE));
    r.e.video(r.at(10_000), None);
    r.tick(20_000);
    assert_eq!(r.writes(), ["filmMaker"]);
    r.e.picture(r.at(30_000), ctx("cinema"));
    assert_eq!(r.e.saved().video.as_deref(), Some("cinema"), "off: the one mode is the video slot's");
}

#[test]
fn a_preset_is_journalled_and_restored_exactly() {
    let mut r = Rig::new("eco", Some("filmMaker"), Some("filmMaker"));
    let eco: BTreeMap<String, String> =
        [("backlight", "77"), ("contrast", "100"), ("energySaving", "off"), ("color", "60"), ("colorGamut", "dynamic"), ("gamma", "medium")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
    r.tv.0.lock().values.insert("eco".into(), eco.clone());
    r.e.command(r.at(1_000), Cmd::Preset("lumaVivid".into())).unwrap();
    {
        let s = r.tv.0.lock();
        let now = &s.values["eco"];
        assert_eq!((now["backlight"].as_str(), now["contrast"].as_str(), now["energySaving"].as_str()), ("75", "85", "auto"));
        assert_eq!(now.len(), eco.len(), "keys the mode lacks are not written");
    }
    // a second preset write keeps the first originals
    r.e.command(r.at(2_000), Cmd::Preset("lumaVivid".into())).unwrap();
    let journal = &r.e.saved().journal["eco"];
    assert_eq!(journal.len(), eco.len());
    assert_eq!(journal["backlight"], "77");
    assert_eq!(r.e.saved().applied["eco"], "lumaVivid");
    assert_eq!(r.e.saved().ui.as_deref(), Some("eco"), "a preset becomes the slot in force's choice");
    r.e.command(r.at(3_000), Cmd::Restore("eco".into())).unwrap();
    assert_eq!(r.tv.0.lock().values["eco"], eco);
    assert!(r.e.saved().journal.is_empty() && r.e.saved().applied.is_empty());
}

#[test]
fn the_first_run_gives_the_shell_lumatv_vivid() {
    let mut r = Rig::new("filmMaker", None, None);
    r.tv.0.lock().values.insert("eco".into(), [("color".to_owned(), "60".to_owned()), ("colorGamut".to_owned(), "dynamic".to_owned())].into());
    r.tick(0);
    assert_eq!(r.e.saved().video.as_deref(), Some("filmMaker"));
    assert_eq!(r.e.saved().ui.as_deref(), Some("eco"));
    assert_eq!(r.tv.0.lock().values["eco"]["colorGamut"], "native");
    assert_eq!(r.writes(), ["eco"]);
}

#[test]
fn reads_videooutput_and_the_picture_subscription() {
    // as the TV answered while Marquee played (2026-11-21)
    let reply = json!({"video": [
        {"sink": "MAIN", "connected": true, "muted": false, "appId": MARQUEE, "fullScreen": true,
         "displayOutput": {"width": 3840, "height": 2160}, "videoInfo": {"hdrType": "none", "contentType": "movie"}},
        {"sink": "SUB0", "connected": false}
    ]});
    assert_eq!(Video::parse(&reply), Some(film(MARQUEE)));
    assert_eq!(Video::parse(&json!({"video": [{"sink": "MAIN", "connected": false}]})), Some(None));
    // a 2:1 film letterboxed by Marquee: LG says it is not full screen, but it fills the width
    let letterboxed = json!({"video": [{"sink": "MAIN", "connected": true, "muted": false, "appId": MARQUEE, "fullScreen": false,
        "displayOutput": {"x": 0, "y": 120, "width": 3840, "height": 1920}, "videoInfo": {"hdrType": "none"}}]});
    assert_eq!(Video::parse(&letterboxed), Some(film(MARQUEE)));
    let corner = json!({"video": [{"sink": "MAIN", "connected": true, "muted": false, "appId": MARQUEE, "fullScreen": false,
        "displayOutput": {"x": 2560, "y": 1440, "width": 1280, "height": 720}, "videoInfo": {"hdrType": "none"}}]});
    assert_eq!(Video::parse(&corner).map(|v| v.map(|v| v.full)), Some(Some(false)));
    let picture = json!({"settings": {"pictureMode": "filmMaker"}, "dimension": {"input": "default", "dynamicRange": "sdr"}, "subscribed": true});
    assert_eq!(Context::parse(&picture), Some(ctx("filmMaker")));
}
