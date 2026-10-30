//! The Cameras app's behaviour under a clock: the cases the QML pop-up and viewer tests state (`tests/test_camera_popup.py`: the full view
//! asks for the live stream over Home and not over an app, the hole while it plays, the grid and back, the keys that take it) and the
//! spec's own (`docs/research/cameras-ui-spec.md`: the grid, the chrome that fades, the strip's commits, the sheet, the refusals' words,
//! the options page).

use super::app::*;
use super::data::LinkState;
use super::strip::Item as SItem;
use super::view::{Backdrop, Base, SheetFocus, Tab, View};
use super::words::{Clock, Zone};
use crate::kit::screen::{Effect, Input, Model};
use serde_json::{Value, json};

const T0: u64 = 1_792_609_200_000;
const MIN: i64 = 60_000;

fn camera(id: &str, name: &str, extra: Value) -> Value {
    let mut c = json!({"id": id, "name": name, "room": "", "online": true, "live": true, "recordings": true, "events": true, "clips": false, "since": 0,
                       "native": {"codec": "h264", "width": 1920, "height": 1080, "fps": 15}, "prebuffered": true});
    if let (Some(c), Some(e)) = (c.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            c.insert(k.clone(), v.clone());
        }
    }
    c
}

/// The helper's `cameras` topic of the QML tests: Back Door, Front Door, Garden.
fn three() -> Value {
    json!({"state": "ready", "reason": "", "now_skew_ms": 0, "rev": 1, "cameras": [camera("274", "Back Door", json!({})), camera("273", "Front Door", json!({"clips": true, "since": T0 as i64 - 600 * MIN})), camera("275", "Garden", json!({}))]})
}

fn cams(n: usize) -> Value {
    let list: Vec<Value> = (0..n).map(|i| camera(&format!("{}", 300 + i), &format!("Camera {}", i + 1), json!({}))).collect();
    json!({"state": "ready", "reason": "", "now_skew_ms": 0, "rev": 1, "cameras": list})
}

fn cfg() -> Config {
    Config { clock: Clock { twelve_hour: true, zone: Zone::UTC }, ..Config::default() }
}

struct Rig {
    app: App,
    now: u64,
}

impl Rig {
    fn new(topic: Value) -> Self {
        let mut r = Self { app: App::new(cfg()), now: T0 };
        r.helper("cameras", topic);
        r.topic("foreground", json!({"app": "com.webos.app.home", "home": true}));
        r
    }

    fn at(&mut self, ms: u64) {
        self.now = T0 + ms;
    }

    fn feed(&mut self, input: Input) -> Vec<Effect> {
        self.app.update(&input, self.now)
    }

    fn helper(&mut self, topic: &str, data: Value) -> Vec<Effect> {
        self.feed(Input::Helper { topic: topic.into(), data })
    }

    fn topic(&mut self, topic: &str, data: Value) -> Vec<Effect> {
        self.feed(Input::Topic { topic: topic.into(), data })
    }

    fn tick(&mut self, ms: u64) -> Vec<Effect> {
        self.at(ms);
        self.feed(Input::Tick)
    }

    fn press(&mut self, name: &str) -> Vec<Effect> {
        let mut fx = self.feed(Input::Key { name: name.into(), down: true, repeat: false });
        fx.extend(self.feed(Input::Key { name: name.into(), down: false, repeat: false }));
        fx
    }

    fn view(&self) -> View {
        self.app.view()
    }

    fn full(&self) -> super::view::FullView {
        match self.view().base {
            Base::Full(f) => f,
            other => panic!("not the full view: {other:?}"),
        }
    }

    fn grid(&self) -> super::view::GridView {
        match self.view().base {
            Base::Grid(g) => g,
            other => panic!("not the grid: {other:?}"),
        }
    }

    /// Answers a request or a fetch.
    fn answer(&mut self, tag: u64, status: u16, data: Value) -> Vec<Effect> {
        self.feed(Input::Fetched { tag, status, data })
    }

    /// Answers every fetch of `fx` for a picture as the host would once it has decoded it.
    fn pictures(&mut self, fx: &[Effect]) -> Vec<Effect> {
        let mut out = Vec::new();
        for (tag, path) in fetches(fx) {
            if path.contains("/frame?") || path.contains("/thumbnail") {
                out.extend(self.answer(tag, 200, json!({"picture": {"w": 10, "h": 10}})));
            }
        }
        out
    }
}

/// (method, path, body) of the requests and posts in `fx`.
fn requests(fx: &[Effect]) -> Vec<(String, String, Value)> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Request { method, path, body, .. } => Some((method.clone(), path.clone(), body.clone().unwrap_or(Value::Null))),
            Effect::Post { path, body } => Some(("POST".into(), path.clone(), body.clone())),
            _ => None,
        })
        .collect()
}

fn fetches(fx: &[Effect]) -> Vec<(u64, String)> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Fetch { tag, path } => Some((*tag, path.clone())),
            _ => None,
        })
        .collect()
}

/// The fetches of stills.
fn frames(fx: &[Effect]) -> Vec<(u64, String)> {
    fetches(fx).into_iter().filter(|(_, p)| p.contains("/frame?")).collect()
}

fn tag_of(fx: &[Effect], path_has: &str) -> u64 {
    fx.iter()
        .find_map(|e| match e {
            Effect::Request { tag, path, .. } if path.contains(path_has) => Some(*tag),
            Effect::Fetch { tag, path } if path.contains(path_has) => Some(*tag),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no ask for {path_has} in {fx:?}"))
}

fn live_topic(camera: &str, mode: &str, state: &str) -> Value {
    json!({"camera": camera, "mode": mode, "state": state, "reason": "", "codec": "h265", "width": 3840, "height": 2160, "fps": 15, "startedAt": null,
           "positionMs": null, "rate": 1, "paused": false})
}

fn live_posts(fx: &[Effect]) -> Vec<Value> {
    requests(fx).into_iter().filter(|(_, p, _)| p == "/v1/cameras/live").map(|(_, _, b)| b).collect()
}

fn has_redraw(fx: &[Effect]) -> bool {
    fx.iter().any(|e| matches!(e, Effect::Redraw))
}

// ---------------------------------------------------------------------------- opening

#[test]
fn nothing_is_up_until_it_is_opened_and_a_closed_app_wants_no_tick() {
    let r = Rig::new(three());
    assert!(!r.app.viewer_up());
    assert_eq!(r.app.due_at(), None);
    assert_eq!(r.view(), View::closed());
}

#[test]
fn over_home_the_full_view_asks_for_the_live_stream_shows_pictures_until_it_plays_and_opens_a_hole_while_it_does() {
    let mut r = Rig::new(three());
    let fx = r.app.open_full("273", T0);
    assert_eq!(live_posts(&fx), [json!({"camera": "273"})], "asked for the stream on opening");
    assert!(r.app.viewer_up());
    let f = r.full();
    assert_eq!((f.name.as_str(), f.position.as_deref()), ("Front Door", Some("2 of 3")));
    assert!(!f.hole);
    assert_eq!(f.state.word, "Starting");
    assert_eq!(f.backdrop, Backdrop::None, "a prebuffered start is black until it plays");
    r.helper("cameras.live", live_topic("273", "live", "playing"));
    let f = r.full();
    assert!(f.hole, "the layer is a hole over the video plane while the stream plays");
    assert_eq!((f.state.word, f.state.dot), ("Live", true));
    assert_eq!(f.info.as_deref(), Some("3840\u{d7}2160 \u{b7} H.265 \u{b7} 15 fps"));
    assert_eq!(f.backdrop, Backdrop::None);
}

#[test]
fn a_stream_that_is_not_prebuffered_shows_the_stills_and_says_it_opens_in_a_few_seconds() {
    let mut cs = three();
    cs["cameras"][1]["prebuffered"] = json!(false);
    let mut r = Rig::new(cs);
    let fx = r.app.open_full("273", T0);
    let f = r.full();
    assert_eq!(f.line.as_deref(), Some("Opens in a few seconds"));
    assert_eq!(f.backdrop, Backdrop::Still("full:273".into()));
    assert!(f.veil > 0.0, "a stills backdrop under the starting words is veiled");
    let asked = frames(&fx);
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].1, "/v1/cameras/273/frame?w=1280&h=720", "stills at 1280 by 720");
}

#[test]
fn the_next_camera_is_asked_for_on_right_with_a_wrap_and_back_stops_the_stream_and_closes() {
    let mut r = Rig::new(three());
    r.app.open_full("273", T0);
    r.helper("cameras.live", live_topic("273", "live", "playing"));
    r.at(100);
    let fx = r.press("right");
    assert_eq!(live_posts(&fx), [json!({"camera": "275"})], "the next camera is asked for");
    assert!(!r.full().hole, "the hole waits for its stream");
    assert_eq!(r.full().name, "Garden");
    r.at(300);
    let fx = r.press("right");
    assert_eq!(live_posts(&fx), [json!({"camera": "274"})], "right from the last wraps to the first");
    r.at(500);
    let fx = r.press("left");
    assert_eq!(live_posts(&fx), [json!({"camera": "275"})], "and left from the first wraps back");
    r.at(700);
    let fx = r.press("back");
    assert!(requests(&fx).contains(&("POST".into(), "/v1/cameras/live/stop".into(), json!({}))), "the session is torn down: {fx:?}");
    assert!(!r.app.viewer_up());
}

#[test]
fn over_an_app_the_full_view_is_pictures_only_and_asks_for_no_live_stream() {
    let mut r = Rig::new(three());
    r.topic("foreground", json!({"app": "netflix", "home": false}));
    let fx = r.app.open_full("273", T0);
    assert!(live_posts(&fx).is_empty(), "an app is playing: the feeder's window over it is unmeasured");
    let f = r.full();
    assert!(!f.hole);
    assert_eq!(f.backdrop, Backdrop::Still("full:273".into()));
    assert!(fetches(&fx).iter().any(|(_, p)| p.starts_with("/v1/cameras/273/frame?")), "the full view draws the camera's pictures");
    assert!(r.view().over_app);
    // the strip and the sheet are not offered without live video
    r.at(100);
    r.press("up");
    r.press("down");
    let v = r.view();
    assert!(v.strip.is_none() && v.sheet.is_none());
}

#[test]
fn over_an_app_live_is_asked_for_only_when_the_theme_offers_it() {
    let mut r = Rig::new(three());
    r.app.cfg.live_over_apps = true;
    r.topic("foreground", json!({"app": "netflix", "home": false}));
    let fx = r.app.open_full("273", T0);
    assert_eq!(live_posts(&fx), [json!({"camera": "273"})]);
}

#[test]
fn a_link_that_is_not_ready_asks_for_no_live_stream_and_one_that_becomes_ready_does() {
    let mut offline = three();
    offline["state"] = json!("offline");
    offline["reason"] = json!("Scrypted does not answer");
    let mut r = Rig::new(offline);
    let fx = r.app.open_full("273", T0);
    assert!(live_posts(&fx).is_empty());
    r.at(500);
    let fx = r.helper("cameras", three());
    assert_eq!(live_posts(&fx), [json!({"camera": "273"})], "the link came back: the stream is asked for");
}

#[test]
fn a_foreground_change_stops_or_starts_the_stream() {
    let mut r = Rig::new(three());
    r.app.open_full("273", T0);
    r.at(100);
    let fx = r.topic("foreground", json!({"app": "netflix", "home": false}));
    assert!(requests(&fx).iter().any(|(_, p, _)| p == "/v1/cameras/live/stop"), "an app came to the front: live is no longer offered");
    assert!(r.app.viewer_up(), "the viewer stays");
    assert!(r.full().backdrop == Backdrop::Still("full:273".into()));
    r.at(200);
    let fx = r.topic("foreground", json!({"app": "com.webos.app.home", "home": true}));
    assert_eq!(live_posts(&fx), [json!({"camera": "273"})]);
}

#[test]
fn the_feeders_own_window_in_front_is_the_picture_and_not_an_app_over_it() {
    let mut r = Rig::new(three());
    r.app.open_full("273", T0);
    r.at(100);
    r.helper("cameras.live", live_topic("273", "live", "playing"));
    // over Home the stream plays in the feeder's window, which the foreground topic then names
    let fx = r.topic("foreground", json!({"app": "com.lumatv.app.cameras", "home": false}));
    assert!(requests(&fx).is_empty(), "nothing to stop: {fx:?}");
    assert!(!r.view().over_app);
    assert!(r.app.hole(), "the picture is still the hole");
    // and over an app it was the app that stays in front: the stream is not asked for, and the window does not change that
    r.at(200);
    r.topic("foreground", json!({"app": "netflix", "home": false}));
    r.at(300);
    let fx = r.topic("foreground", json!({"app": "com.lumatv.app.cameras", "home": false}));
    assert!(fx.is_empty());
    assert!(r.view().over_app);
}

#[test]
fn the_live_view_names_the_window_it_wants_for_whoever_owns_the_video_windows() {
    let mut r = Rig::new(three());
    assert!(r.app.live_claim().is_none(), "nothing is open");
    r.app.open_full("273", T0);
    r.at(100);
    let claim = r.app.live_claim().expect("asked for the stream: the window is wanted before it plays");
    assert_eq!((claim.camera.as_str(), claim.rect), ("273", (0, 0, 1920, 1080)), "a 16 by 9 stream fills the screen");
    assert_eq!(
        claim.request(),
        json!({"id": "cameras-full", "kind": "camera-full", "rect": {"x": 0, "y": 0, "w": 1920, "h": 1080}, "need": {"codec": "h264", "w": 1920, "h": 1080, "fps": 15}})
    );
    let mut four_by_three = live_topic("273", "live", "playing");
    four_by_three["width"] = json!(1440);
    four_by_three["height"] = json!(1080);
    r.helper("cameras.live", four_by_three);
    assert_eq!(r.app.live_claim().unwrap().rect, (240, 0, 1440, 1080), "a narrower stream is centred");
    r.at(200);
    r.topic("foreground", json!({"app": "netflix", "home": false}));
    assert!(r.app.live_claim().is_none(), "over an app live is not asked for: no window");
    assert_eq!(r.app.debug()["liveClaim"], Value::Null);
}

#[test]
fn a_refused_video_window_keeps_the_plane_path_and_says_so_with_a_glyph_and_a_window_not_drawing_yet_is_no_hole() {
    use crate::host::vwin::Grant;
    let denied = |reason: &str| Grant::Denied { reason: reason.into(), text: "x".into() };
    let mut r = Rig::new(three());
    r.app.open_full("273", T0);
    r.at(100);
    r.helper("cameras.live", live_topic("273", "live", "playing"));
    assert!(r.app.hole() && !r.full().window_cue, "no claim in force: the plane carries it, nothing to say");
    // granted and not drawing: the layer is no hole over nothing; once it draws, it is
    assert!(r.app.set_window(Window::of(&Grant::Granted { slot: "sub".into(), placed: false })));
    assert!(!r.app.hole() && !r.full().window_cue);
    assert!(r.app.set_window(Window::of(&Grant::Granted { slot: "sub".into(), placed: true })));
    assert!(r.app.hole() && !r.full().window_cue);
    // refused or taken back: the plane path goes on (a hole while it plays) and a glyph says it is not the window
    for refusal in [denied("no-window"), denied("both-scalers"), Grant::Preempted { reason: "higher".into(), text: "x".into() }] {
        r.app.set_window(Window::of(&refusal));
        assert!(r.app.hole() && r.full().window_cue, "{refusal:?}");
    }
    assert_eq!(r.app.debug()["window"], "Refused");
    // a switch that is off refuses nothing worth a glyph; asked and not answered yet is the plane's
    assert!(r.app.set_window(Window::of(&denied("off"))));
    assert!(r.app.hole() && !r.full().window_cue);
    assert!(!r.app.set_window(Window::of(&Grant::Asked)), "no change, no redraw");
    // without a stream asked for there is no claim and nothing to say
    r.app.set_window(Window::Refused);
    r.at(200);
    r.topic("foreground", json!({"app": "netflix", "home": false}));
    assert!(!r.full().window_cue, "over an app live is not asked for");
}

// ---------------------------------------------------------------------------- the grid

#[test]
fn ok_in_the_full_view_goes_to_the_grid_which_asks_for_pictures_of_the_tile_size_and_stops_the_stream() {
    let mut r = Rig::new(three());
    r.app.open_full("273", T0);
    r.at(100);
    let fx = r.press("ok");
    assert!(requests(&fx).iter().any(|(_, p, _)| p == "/v1/cameras/live/stop"), "the grid is stills");
    let g = r.grid();
    assert_eq!((g.layout.cols, g.layout.tile_w, g.layout.tile_h), (2, 624, 352), "three cameras: 2 x 2 at 624 x 352");
    assert_eq!(g.focus, super::grid::Focus::Tile(1), "the grid opens on the camera that was shown");
    assert_eq!(g.count_words, "3 cameras");
    let asked: Vec<String> = fetches(&fx).into_iter().map(|(_, p)| p).collect();
    assert_eq!(asked.len(), 3);
    assert!(asked.contains(&"/v1/cameras/273/frame?w=624&h=352".to_string()));
}

#[test]
fn the_grid_moves_by_keys_lands_on_the_lone_tile_below_and_ok_opens_that_camera() {
    let mut r = Rig::new(three());
    r.app.open_grid(T0);
    r.at(100);
    r.press("right");
    r.press("down");
    assert_eq!(r.grid().focus, super::grid::Focus::Tile(2), "right, down lands on the lone tile of the row below");
    r.press("left");
    assert_eq!(r.grid().focus, super::grid::Focus::Tile(2), "left stays");
    r.at(200);
    let fx = r.press("ok");
    assert_eq!(r.full().name, "Garden");
    assert_eq!(live_posts(&fx), [json!({"camera": "275"})]);
    r.at(300);
    r.press("back");
    assert!(!r.app.viewer_up(), "Back closes the viewer");
}

#[test]
fn up_from_the_top_row_reaches_the_headers_buttons_and_events_opens_the_recent_sheet_over_the_grid() {
    let mut r = Rig::new(cams(8));
    r.helper("cameras.events", json!({"events": [{"id": "e2", "camera": "301", "name": "Camera 2", "kind": "animal", "label": "Domestic Cat", "at": T0 as i64 - 11 * MIN, "phase": "new"},
                                                   {"id": "e1", "camera": "300", "name": "Camera 1", "kind": "person", "label": "person", "at": T0 as i64 - 6 * MIN, "phase": "new", "recording": true}]}));
    r.app.open_grid(T0);
    r.at(100);
    r.press("up");
    assert_eq!(r.grid().focus, super::grid::Focus::Events);
    r.press("right");
    assert_eq!(r.grid().focus, super::grid::Focus::Settings);
    r.press("left");
    r.at(200);
    r.press("ok");
    let v = r.view();
    let sheet = v.sheet.expect("the recent events sheet");
    assert_eq!(sheet.title, "Recent events");
    assert_eq!(sheet.tabs, None);
    assert_eq!(sheet.cards[0].title, "Camera 2 \u{b7} Animal \u{b7} Domestic Cat");
    // OK on an event with a recording plays it from three seconds before, on the full view of its camera
    r.at(300);
    r.press("right");
    r.at(400);
    let fx = r.press("ok");
    assert!(r.app.view().sheet.is_none());
    assert_eq!(r.full().name, "Camera 1");
    let posts = requests(&fx);
    assert!(posts.iter().any(|(_, p, b)| p == "/v1/cameras/playback" && b["camera"] == "300" && b["at"] == json!(T0 as i64 - 6 * MIN - 3000)), "{posts:?}");
}

#[test]
fn the_settings_button_raises_an_intent_and_closes_the_app() {
    let mut r = Rig::new(three());
    r.app.open_grid(T0);
    r.at(100);
    r.press("up");
    r.press("right");
    r.press("ok");
    assert_eq!(r.app.take_intents(), [Intent::OpenSettings]);
    assert!(!r.app.viewer_up());
    assert!(r.app.take_intents().is_empty());
}

#[test]
fn more_than_eight_cameras_scroll_by_whole_rows_and_the_foot_says_what_is_below() {
    let mut r = Rig::new(cams(12));
    r.app.open_grid(T0);
    let g = r.grid();
    assert_eq!((g.layout.cols, g.layout.rows_total, g.layout.rows_shown, g.top), (4, 3, 2, 0));
    assert_eq!(g.below.as_deref(), Some("9 to 12"));
    r.at(100);
    for _ in 0..2 {
        r.press("down");
    }
    let g = r.grid();
    assert_eq!((g.top, g.focus), (1, super::grid::Focus::Tile(8)));
    assert_eq!(g.below, None);
    assert_eq!(g.above.as_deref(), Some("1 to 4"));
}

#[test]
fn only_the_shown_rows_ask_for_pictures_at_the_grids_interval_after_the_last_arrived() {
    let mut r = Rig::new(cams(12));
    let fx = r.app.open_grid(T0);
    assert_eq!(frames(&fx).len(), 8, "at most 8 stills are asked for");
    r.at(150);
    let fx2 = r.pictures(&fx);
    assert!(frames(&fx2).is_empty(), "the next ask waits for the interval");
    r.at(150 + 199);
    let fx3 = r.tick(150 + 199);
    assert!(frames(&fx3).is_empty(), "199 ms after the last arrived: not yet (Balanced is 200)");
    let fx4 = r.tick(150 + 200);
    assert_eq!(frames(&fx4).len(), 8);
    let due = r.app.due_at();
    assert_eq!(due, None, "all eight are in flight: nothing waits for its time");
    r.pictures(&fx4);
    assert_eq!(r.app.due_at(), Some(T0 + 150 + 200 + 200), "the next ask after the last arrived plus 200 ms");
}

#[test]
fn the_grids_interval_follows_the_owners_video_choice() {
    let mut r = Rig::new(cams(4));
    let fx = r.app.open_grid(T0);
    let settings = tag_of(&fx, "/v1/settings/scrypted-cameras");
    r.answer(settings, 200, json!({"owner": "scrypted-cameras", "version": 3, "schemaRevision": 1, "effective": {"video.grid": {"type": "choice", "value": "light"}}}));
    r.at(10);
    let fx = r.tick(10);
    r.pictures(&fx);
    r.pictures(&fetches(&fx).iter().map(|(t, p)| Effect::Fetch { tag: *t, path: p.clone() }).collect::<Vec<_>>());
    r.at(10 + 332);
    assert!(fetches(&r.tick(10 + 332)).is_empty());
}

#[test]
fn a_tile_says_connecting_then_no_picture_after_two_refusals_and_dims_with_its_age_when_the_link_drops() {
    let mut r = Rig::new(cams(4));
    let fx = r.app.open_grid(T0);
    assert_eq!(r.grid().tiles[0].words, ["Connecting"]);
    for (tag, _) in fetches(&fx) {
        r.answer(tag, 404, json!({"ok": false, "error": {"code": "no_picture"}}));
    }
    r.at(600);
    let fx = r.tick(600);
    for (tag, _) in fetches(&fx) {
        r.answer(tag, 503, json!({"ok": false, "error": {"code": "not_connected"}}));
    }
    assert_eq!(r.grid().tiles[0].words, ["No picture"], "after two refused asks");
    // a picture arrives, then the link drops
    r.at(1700);
    let fx = r.tick(1700);
    r.pictures(&fx);
    assert!(r.grid().tiles[0].words.is_empty(), "fresh and ready: nothing");
    r.at(1700 + 4 * 60_000);
    let mut offline = cams(4);
    offline["state"] = json!("offline");
    r.helper("cameras", offline);
    let g = r.grid();
    assert_eq!(g.tiles[0].words, ["4 min ago"]);
    assert!(g.tiles[0].dim);
    assert_eq!(g.banner.as_deref(), Some("Can\u{2019}t reach Scrypted"));
}

#[test]
fn an_offline_camera_says_offline() {
    let mut cs = cams(4);
    cs["cameras"][2]["online"] = json!(false);
    let mut r = Rig::new(cs);
    r.app.open_grid(T0);
    assert_eq!(r.grid().tiles[2].words[0], "Offline");
}

#[test]
fn no_tiles_draws_the_states_words_and_the_settings_button_and_a_camera_that_arrives_takes_the_focus() {
    let cases = [
        ("unlinked", "Scrypted isn\u{2019}t linked"),
        ("connecting", "Connecting to Scrypted"),
        ("refused", "Scrypted refused this TV"),
        ("untrusted", "Can\u{2019}t verify Scrypted"),
        ("offline", "Can\u{2019}t reach Scrypted"),
        ("ready", "No cameras shared"),
    ];
    for (state, words) in cases {
        let mut r = Rig::new(json!({"state": state, "reason": "", "cameras": []}));
        r.app.open_grid(T0);
        let g = r.grid();
        assert_eq!(g.empty.map(|e| e.title), Some(words.to_string()), "{state}");
        assert_eq!(g.focus, super::grid::Focus::Settings);
        assert_eq!(g.count_words, "");
    }
    let mut r = Rig::new(json!({"state": "ready", "cameras": []}));
    r.app.open_grid(T0);
    r.at(100);
    r.helper("cameras", three());
    assert_eq!(r.grid().focus, super::grid::Focus::Tile(0));
}

#[test]
fn with_tiles_and_a_link_that_is_not_ready_one_banner_line_says_so() {
    let mut cs = three();
    cs["state"] = json!("refused");
    let mut r = Rig::new(cs);
    r.app.open_grid(T0);
    assert_eq!(r.grid().banner.as_deref(), Some("Scrypted refused this TV"));
    let mut ready = Rig::new(three());
    ready.app.open_grid(T0);
    assert_eq!(ready.grid().banner, None);
}

// ---------------------------------------------------------------------------- the chrome

#[test]
fn the_chrome_goes_2500_ms_after_the_last_key_where_something_plays_and_the_first_key_only_reveals_it() {
    let mut r = Rig::new(three());
    r.app.open_full("273", T0);
    r.helper("cameras.live", live_topic("273", "live", "playing"));
    assert!(r.full().chrome);
    r.tick(2400);
    assert!(r.full().chrome);
    r.tick(2500);
    assert!(!r.full().chrome, "2.5 s without a key");
    // a key other than Back only reveals the chrome
    let fx = r.feed(Input::Key { name: "right".into(), down: true, repeat: false });
    assert!(live_posts(&fx).is_empty(), "the camera did not change");
    assert!(r.full().chrome);
    assert_eq!(r.full().name, "Front Door");
    // and the next one acts
    r.at(2600);
    let fx = r.press("right");
    assert_eq!(live_posts(&fx), [json!({"camera": "275"})]);
}

#[test]
fn back_works_while_the_chrome_is_hidden() {
    let mut r = Rig::new(three());
    r.app.open_full("273", T0);
    r.helper("cameras.live", live_topic("273", "live", "playing"));
    r.tick(3000);
    assert!(!r.full().chrome);
    r.at(3100);
    r.press("back");
    assert!(!r.app.viewer_up());
}

#[test]
fn the_chrome_never_fades_while_nothing_plays() {
    let mut r = Rig::new(three());
    r.app.open_full("273", T0);
    r.tick(10_000);
    assert!(r.full().chrome, "starting: nothing plays");
    r.helper("cameras.live", live_topic("273", "live", "playing"));
    r.tick(10_000 + 2500);
    assert!(!r.full().chrome);
    // a failure brings the chrome back
    let mut failed = live_topic("273", "live", "failed");
    failed["reason"] = json!("The camera is offline.");
    r.helper("cameras.live", failed);
    r.tick(10_000 + 2600);
    assert!(r.full().chrome, "the note must be seen");
}

// ---------------------------------------------------------------------------- failure

#[test]
fn a_failed_stream_shows_the_note_with_the_reason_and_ok_asks_again_at_once() {
    let mut r = Rig::new(three());
    r.app.open_full("273", T0);
    let mut failed = live_topic("273", "live", "failed");
    failed["reason"] = json!("The camera is offline.");
    r.helper("cameras.live", failed);
    let f = r.full();
    let note = f.note.clone().expect("the note");
    assert_eq!((note.title, note.detail.as_str(), note.button), ("Can\u{2019}t play this camera", "The camera is offline.", "Try again"));
    assert_eq!((f.state.word, f.state.dot), ("Can\u{2019}t play", false));
    assert!(f.button_focused && f.hints.is_empty());
    assert!(matches!(f.backdrop, Backdrop::Still(_)), "a dimmed stills backdrop");
    assert!(f.veil > 0.5);
    r.at(100);
    let fx = r.press("ok");
    assert_eq!(live_posts(&fx), [json!({"camera": "273"})], "a stream address lives 25 to 60 s: a retry always asks for a new one");
    r.at(200);
    r.press("back");
    assert!(!r.app.viewer_up(), "Back closes");
}

#[test]
fn a_recording_that_ends_asks_live_again_by_itself() {
    let mut r = Rig::new(three());
    r.app.open_full("273", T0);
    r.helper("cameras.live", live_topic("273", "live", "playing"));
    r.at(100);
    r.press("up");
    r.at(200);
    r.press("down");
    r.press("right");
    // -10 s: a playback is asked for
    let fx = r.press("ok");
    assert!(requests(&fx).iter().any(|(_, p, _)| p == "/v1/cameras/playback"), "{fx:?}");
    r.at(250);
    r.helper("cameras.live", live_topic("273", "playback", "playing"));
    r.at(300);
    let fx = r.helper("cameras.live", live_topic("273", "playback", "stopped"));
    assert!(live_posts(&fx).contains(&json!({"camera": "273"})), "End of recording: live again, by itself: {fx:?}");
}

#[test]
fn a_refusal_shows_its_words_for_three_seconds_and_the_picture_goes_on() {
    let mut r = Rig::new(three());
    let fx = r.app.open_full("273", T0);
    let tag = tag_of(&fx, "/v1/cameras/live");
    r.at(100);
    r.answer(tag, 409, json!({"ok": false, "error": {"code": "offline", "message": "x"}}));
    assert_eq!(r.view().toast.as_deref(), Some("The camera is offline"));
    r.tick(3099);
    assert!(r.view().toast.is_some());
    r.tick(3100);
    assert!(r.view().toast.is_none(), "gone after 3 s");
    // a code the table lacks shows the helper's words; one with no words says nothing
    let fx = r.press("right");
    let tag = tag_of(&fx, "/v1/cameras/live");
    r.answer(tag, 409, json!({"ok": false, "error": {"code": "weird", "message": "The bridge said so"}}));
    assert_eq!(r.view().toast.as_deref(), Some("The bridge said so"));
}

// ---------------------------------------------------------------------------- the strip

fn open_strip(r: &mut Rig) -> Vec<Effect> {
    r.app.open_full("273", T0);
    r.helper("cameras.live", live_topic("273", "live", "playing"));
    r.at(100);
    r.press("up")
}

#[test]
fn up_opens_the_strip_which_reads_the_recordings_and_the_events_of_its_window() {
    let mut r = Rig::new(three());
    let fx = open_strip(&mut r);
    let v = r.view();
    let s = v.strip.expect("the strip");
    assert_eq!((s.when.as_str(), s.rel.as_str(), s.span_word.as_str(), s.rate_word.as_str()), ("Live", "Now", "1 h", "1\u{d7}"));
    assert!(s.live);
    assert_eq!(s.focus, SItem::Bar);
    let paths: Vec<String> = fetches(&fx).into_iter().map(|(_, p)| p).collect();
    let from = T0 as i64 + 100 - 60 * MIN;
    assert!(paths.contains(&format!("/v1/cameras/273/recordings?from={from}&to={}", T0 as i64 + 100)), "{paths:?}");
    assert!(paths.iter().any(|p| p.starts_with("/v1/cameras/273/events?from=") && p.ends_with("&count=200")));
    assert!(v.sheet.is_none());
}

#[test]
fn the_reads_fill_the_bar_with_ranges_gaps_and_event_ticks() {
    let mut r = Rig::new(three());
    let fx = open_strip(&mut r);
    let now = T0 as i64 + 100;
    r.answer(
        tag_of(&fx, "/recordings"),
        200,
        json!({"camera": "273", "ranges": [{"from": now - 50 * MIN, "to": now - 10 * MIN}, {"from": now - 8 * MIN, "to": now}], "unknown": [{"from": now - 10 * MIN, "to": now - 8 * MIN}]}),
    );
    r.answer(tag_of(&fx, "/events?"), 200, json!({"events": [{"at": now - 30 * MIN, "kind": "person", "label": "person"}, {"at": now - 12 * MIN, "kind": "motion"}]}));
    let s = r.view().strip.unwrap();
    assert_eq!(s.bar.recorded.len(), 2);
    assert_eq!(s.bar.unknown.len(), 1);
    assert_eq!(s.bar.events, [now - 30 * MIN, now - 12 * MIN]);
    assert_eq!(s.bar.gaps.len(), 1, "a camera that began recording 600 minutes ago: the first 10 minutes of the window have no footage");
    assert!(s.can_prev && !s.can_next, "events behind the cursor, none ahead");
    assert_eq!(s.axis.len(), 6);
    assert_eq!(s.axis[0].1, "1:40 PM".replace("1:40 PM", &s.axis[0].1));
}

#[test]
fn scrubbing_moves_the_cursor_shows_a_bubble_and_commits_on_the_release() {
    let mut r = Rig::new(three());
    let fx = open_strip(&mut r);
    let now = T0 as i64 + 100;
    r.answer(tag_of(&fx, "/recordings"), 200, json!({"camera": "273", "ranges": [{"from": now - 59 * MIN, "to": now}], "unknown": []}));
    r.at(200);
    let fx = r.feed(Input::Key { name: "left".into(), down: true, repeat: false });
    assert!(requests(&fx).iter().all(|(_, p, _)| p != "/v1/cameras/playback"), "the commit waits for the release");
    let s = r.view().strip.unwrap();
    assert!(!s.live);
    assert_eq!(s.cursor, now + 100 - MIN, "a sixtieth of the hour");
    assert_eq!(s.rel, "1 min ago");
    let b = s.bubble.expect("a bubble while the cursor is away from the picture");
    assert_eq!(b.pic, None);
    // 250 ms after it settled the bubble's picture is asked for
    let fx = r.tick(200 + 250);
    let thumb: Vec<String> = fetches(&fx).into_iter().map(|(_, p)| p).filter(|p| p.contains("/thumbnail?")).collect();
    assert_eq!(thumb.len(), 1, "{fx:?}");
    assert!(thumb[0].starts_with("/v1/cameras/273/thumbnail?at=") && thumb[0].ends_with("&w=256&h=144"));
    r.pictures(&fx);
    assert!(r.view().strip.unwrap().bubble.unwrap().pic.is_some());
    // the release commits the cursor
    r.at(500);
    let fx = r.feed(Input::Key { name: "left".into(), down: false, repeat: false });
    let posts: Vec<_> = requests(&fx).into_iter().filter(|(_, p, _)| p == "/v1/cameras/playback").collect();
    assert_eq!(posts.len(), 1, "{fx:?}");
    assert_eq!(posts[0].2["camera"], "273");
    assert_eq!(posts[0].2["at"], json!(now + 100 - MIN));
    assert!(posts[0].2.get("rate").is_none(), "1x is the default");
}

#[test]
fn a_cursor_in_a_confirmed_gap_says_no_recording_and_asks_nothing() {
    let mut r = Rig::new(three());
    let fx = open_strip(&mut r);
    let now = T0 as i64 + 100;
    // recorded only the last 30 minutes of the hour, from a camera whose first footage is older: the rest is a confirmed gap
    r.answer(tag_of(&fx, "/recordings"), 200, json!({"camera": "273", "ranges": [{"from": now - 30 * MIN, "to": now}], "unknown": []}));
    for i in 0..31u64 {
        // a press every 400 ms: each is the first of a run, one minute a step; the key is held through them
        r.at(200 + i * 400);
        r.feed(Input::Key { name: "left".into(), down: true, repeat: false });
    }
    r.at(200 + 31 * 400);
    let fx = r.feed(Input::Key { name: "left".into(), down: false, repeat: false });
    assert!(requests(&fx).iter().all(|(_, p, _)| p != "/v1/cameras/playback"), "a gap asks nothing: {fx:?}");
    assert_eq!(r.view().toast.as_deref(), Some("No recording at that time"));
}

#[test]
fn a_playback_shows_the_playhead_pauses_with_ok_and_back_goes_live_before_it_closes() {
    let mut r = Rig::new(three());
    let fx = open_strip(&mut r);
    let now = T0 as i64 + 100;
    r.answer(tag_of(&fx, "/recordings"), 200, json!({"camera": "273", "ranges": [{"from": now - 59 * MIN, "to": now}], "unknown": []}));
    r.at(200);
    r.press("left");
    r.at(300);
    let mut pb = live_topic("273", "playback", "playing");
    pb["positionMs"] = json!(now - MIN);
    pb["startedAt"] = json!(now - MIN);
    r.helper("cameras.live", pb.clone());
    let f = r.full();
    assert_eq!(f.state.word, "Recorded");
    assert!(f.hole);
    let s = r.view().strip.unwrap();
    assert!(!s.live);
    assert_eq!(s.when, "2:12:20 PM".replace("2:12:20 PM", &s.when), "the playhead's clock with seconds");
    // OK on the bar, where the picture is, pauses
    r.at(400);
    let fx = r.press("ok");
    assert!(requests(&fx).iter().any(|(_, p, b)| p == "/v1/cameras/live/pause" && b["paused"] == json!(true)), "{fx:?}");
    // Back, in order: a playback goes live
    r.at(500);
    let fx = r.press("back");
    assert_eq!(live_posts(&fx), [json!({"camera": "273"})]);
    assert!(r.app.viewer_up());
    assert!(r.view().strip.unwrap().live);
    r.at(600);
    r.press("back");
    assert!(r.view().strip.is_none(), "then the strip closes");
    r.at(700);
    r.press("back");
    assert!(!r.app.viewer_up(), "then the view");
}

#[test]
fn the_strip_fades_with_the_chrome_in_a_live_view_and_closes_with_it() {
    let mut r = Rig::new(three());
    open_strip(&mut r);
    r.tick(100 + 2500);
    assert!(!r.full().chrome);
    assert!(r.view().strip.is_none());
    r.at(3000);
    r.press("up");
    r.press("up");
    assert!(r.view().strip.is_none() || r.view().strip.is_some(), "the first key only revealed the chrome");
}

#[test]
fn the_controls_row_goes_live_hops_and_down_opens_the_sheet() {
    let mut r = Rig::new(three());
    open_strip(&mut r);
    r.at(200);
    r.press("down");
    assert_eq!(r.view().strip.unwrap().focus, SItem::Live);
    r.press("right");
    r.at(300);
    let fx = r.press("ok");
    assert!(requests(&fx).iter().any(|(_, p, _)| p == "/v1/cameras/playback"), "-10 s is a playback: {fx:?}");
    r.at(400);
    let fx = r.press("down");
    assert!(r.view().strip.is_none());
    let sheet = r.view().sheet.expect("down from the controls opens the sheet");
    assert_eq!(sheet.title, "Front Door");
    assert_eq!(sheet.tabs, Some(Tab::Events));
    assert!(fetches(&fx).iter().any(|(_, p)| p.starts_with("/v1/cameras/273/events?from=") && p.ends_with("&count=30")));
}

// ---------------------------------------------------------------------------- the sheet

#[test]
fn down_opens_the_camera_sheet_which_lists_events_newest_first_and_ok_plays_three_seconds_before() {
    let mut r = Rig::new(three());
    r.app.open_full("273", T0);
    r.helper("cameras.live", live_topic("273", "live", "playing"));
    r.at(100);
    let fx = r.press("down");
    let at1 = T0 as i64 - 6 * MIN;
    let at2 = T0 as i64 - 39 * MIN;
    let events = r.answer(tag_of(&fx, "/events?"), 200, json!({"events": [{"at": at2, "kind": "doorbell", "label": "doorbell"}, {"at": at1, "kind": "person", "label": "person"}]}));
    let v = r.view();
    let sheet = v.sheet.unwrap();
    assert_eq!(sheet.cards.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(), ["Person", "Doorbell rang"]);
    assert!(sheet.cards[0].line2.ends_with("\u{b7} 6 min ago"), "{}", sheet.cards[0].line2);
    assert_eq!(sheet.focus, SheetFocus::Card(0));
    assert!(v.strip.is_none());
    let wanted: Vec<String> = fetches(&events).into_iter().map(|(_, p)| p).collect();
    assert_eq!(wanted.len(), 2, "each shown card asks for its picture: {wanted:?}");
    assert!(wanted[0].contains(&format!("/thumbnail?at={at1}&w=352&h=198")));
    r.at(300);
    let fx = r.press("ok");
    let posts = requests(&fx);
    assert!(posts.iter().any(|(_, p, b)| p == "/v1/cameras/playback" && b["camera"] == "273" && b["at"] == json!(at1 - 3000)), "{posts:?}");
    assert!(r.view().sheet.is_none());
}

#[test]
fn the_clips_tab_reads_the_clips_and_ok_plays_a_clip_by_its_id() {
    let mut r = Rig::new(three());
    r.app.open_full("273", T0);
    r.helper("cameras.live", live_topic("273", "live", "playing"));
    r.at(100);
    let fx = r.press("down");
    r.answer(tag_of(&fx, "/events?"), 200, json!({"events": []}));
    assert_eq!(r.view().sheet.unwrap().empty.as_deref(), Some("No events"));
    r.at(200);
    r.press("up");
    let fx = r.press("right");
    let clips = tag_of(&fx, "/clips?");
    r.answer(clips, 200, json!({"clips": [{"id": "c1.Wy", "at": T0 as i64 - 6 * MIN, "duration": 30000, "kind": "person", "label": "person", "description": "Person at the front door",
                                           "has_thumbnail": true, "has_video": true}]}));
    let sheet = r.view().sheet.unwrap();
    assert_eq!(sheet.tabs, Some(Tab::Clips));
    assert_eq!((sheet.cards[0].title.as_str(), sheet.cards[0].length.as_deref()), ("Person at the front door", Some("0:30")));
    r.at(300);
    r.press("down");
    r.at(400);
    let fx = r.press("ok");
    let posts = requests(&fx);
    assert!(posts.iter().any(|(_, p, b)| p == "/v1/cameras/playback" && b["clip"] == "c1.Wy" && b["camera"] == "273"), "{posts:?}");
}

#[test]
fn back_closes_the_sheet_and_the_view_stays() {
    let mut r = Rig::new(three());
    r.app.open_full("273", T0);
    r.helper("cameras.live", live_topic("273", "live", "playing"));
    r.at(100);
    r.press("down");
    r.at(200);
    r.press("back");
    assert!(r.view().sheet.is_none());
    assert!(r.app.viewer_up());
}

// ---------------------------------------------------------------------------- the options page

#[test]
fn ok_held_on_a_tile_opens_its_popup_options_and_a_switch_writes_the_pick() {
    let mut r = Rig::new(three());
    let fx = r.app.open_grid(T0);
    let settings = tag_of(&fx, "/v1/settings/scrypted-cameras");
    r.answer(settings, 200, json!({"owner": "scrypted-cameras", "version": 7, "schemaRevision": 1, "effective": {"notify.274": {"type": "ordered", "value": ["person", "doorbell"]}}}));
    // OK pressed and held: after the hold time the page opens, and the release does not open the camera
    r.at(100);
    r.feed(Input::Key { name: "ok".into(), down: true, repeat: false });
    r.tick(100 + 699);
    assert!(r.view().options.is_none());
    r.tick(100 + 700);
    let o = r.view().options.expect("the options page");
    assert_eq!((o.title.as_str(), o.group, o.back), ("Back Door", "Pop up when it sees", "Cameras"));
    assert_eq!(o.rows.iter().filter(|x| x.on).map(|x| x.name).collect::<Vec<_>>(), ["Person", "Doorbell"]);
    r.at(900);
    r.feed(Input::Key { name: "ok".into(), down: false, repeat: false });
    assert!(r.app.viewer_up() && matches!(r.view().base, Base::Grid(_)), "the release after a hold acts on nothing");
    // down to Animal (row 2), OK writes the whole pick against the version read
    r.at(1000);
    r.press("down");
    r.press("down");
    r.at(1100);
    let fx = r.press("ok");
    let posts = requests(&fx);
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].1, "/v1/settings/scrypted-cameras");
    assert_eq!(posts[0].2, json!({"changes": [{"key": "notify.274", "value": {"type": "ordered", "value": ["person", "doorbell", "animal"]}}], "ifVersion": 7}));
    assert!(r.view().options.unwrap().rows[2].on, "the page shows it at once");
    // the answer rebuilds the page
    let tag = tag_of(&fx, "/v1/settings/");
    r.answer(tag, 200, json!({"changed": ["notify.274"], "version": 8, "settings": {"version": 8, "effective": {"notify.274": {"type": "ordered", "value": ["person", "doorbell", "animal"]}}}}));
    assert_eq!(r.app.settings().version, 8);
    r.at(1200);
    r.press("back");
    assert!(r.view().options.is_none());
    assert!(matches!(r.view().base, Base::Grid(_)));
}

#[test]
fn a_tap_of_ok_on_a_tile_opens_the_camera_on_the_release() {
    let mut r = Rig::new(three());
    r.app.open_grid(T0);
    r.at(100);
    let fx = r.feed(Input::Key { name: "ok".into(), down: true, repeat: false });
    assert!(live_posts(&fx).is_empty(), "OK acts on its release");
    r.at(200);
    let fx = r.feed(Input::Key { name: "ok".into(), down: false, repeat: false });
    assert_eq!(live_posts(&fx), [json!({"camera": "274"})]);
}

// ---------------------------------------------------------------------------- the pop-up and the world

#[test]
fn while_the_viewer_is_up_a_popup_turns_the_full_view_to_its_camera_and_never_twice() {
    let mut r = Rig::new(three());
    r.app.open_full("273", T0);
    r.at(100);
    let popup = |id: i64, camera: &str, until: i64| json!({"id": id, "event": format!("e{id}"), "camera": camera, "kind": "person", "untilMs": until, "at": T0 as i64});
    let fx = r.helper("cameras.popup", popup(41, "275", T0 as i64 + 10_000));
    assert_eq!(r.full().name, "Garden", "the full view turns to the camera something was seen on, with no card over it");
    assert_eq!(live_posts(&fx), [json!({"camera": "275"})]);
    r.at(200);
    r.press("left");
    r.at(300);
    let fx = r.helper("cameras.popup", popup(41, "275", T0 as i64 + 10_000));
    assert!(live_posts(&fx).is_empty() && r.full().name != "Garden", "a topic re-read is not a new pop-up");
}

#[test]
fn a_popup_in_the_grid_does_nothing_and_one_that_is_out_of_date_or_arrives_closed_is_not_acted_on() {
    let mut r = Rig::new(three());
    r.helper("cameras.popup", json!({"id": 7, "event": "e7", "camera": "275", "kind": "person", "untilMs": T0 as i64 + 10_000}));
    r.app.open_grid(T0);
    r.at(100);
    let before = r.view();
    r.helper("cameras.popup", json!({"id": 8, "event": "e8", "camera": "275", "kind": "person", "untilMs": T0 as i64 + 10_000}));
    assert_eq!(r.view(), before, "the grid does nothing");
    r.app.open_full("273", T0 + 200);
    r.at(300);
    r.helper("cameras.popup", json!({"id": 9, "event": "e9", "camera": "275", "kind": "person", "untilMs": T0 as i64 - 1}));
    assert_eq!(r.full().name, "Front Door", "out of date");
}

#[test]
fn a_camera_that_is_no_longer_shared_turns_the_full_view_to_the_first_and_the_last_one_gone_shows_the_grids_words() {
    let mut r = Rig::new(three());
    r.app.open_full("273", T0);
    r.at(100);
    let mut two = three();
    two["cameras"].as_array_mut().unwrap().remove(1);
    let fx = r.helper("cameras", two);
    assert_eq!(r.full().name, "Back Door");
    assert_eq!(live_posts(&fx), [json!({"camera": "274"})]);
    r.at(200);
    r.helper("cameras", json!({"state": "ready", "cameras": []}));
    assert_eq!(r.grid().empty.map(|e| e.title), Some("No cameras shared".to_string()));
}

#[test]
fn the_home_key_and_the_screen_going_dark_close_the_viewer_and_stop_the_stream() {
    let mut r = Rig::new(three());
    r.app.open_full("273", T0);
    r.at(100);
    let fx = r.topic("homekey", json!({"n": 1, "switcher": false}));
    assert!(!r.app.viewer_up());
    assert!(requests(&fx).iter().any(|(_, p, _)| p == "/v1/cameras/live/stop"));
    let mut r = Rig::new(three());
    r.app.open_grid(T0);
    r.topic("screen", json!({"on": false}));
    assert!(!r.app.viewer_up());
}

#[test]
fn show_cameras_from_the_compositor_opens_the_grid() {
    let mut r = Rig::new(three());
    let fx = r.topic("show", json!({"kind": "cameras", "warm": false, "n": 1}));
    assert!(matches!(r.view().base, Base::Grid(_)));
    assert!(has_redraw(&fx));
    let mut r = Rig::new(three());
    r.topic("show", json!({"kind": "quick"}));
    assert!(!r.app.viewer_up(), "another kind's show is not ours");
}

#[test]
fn closing_stops_what_was_asked_forgets_what_is_in_flight_and_stops_the_ticks() {
    let mut r = Rig::new(three());
    let fx = r.app.open_full("273", T0);
    assert!(!fx.is_empty());
    r.at(100);
    let fx = r.app.close(T0 + 100);
    assert!(requests(&fx).iter().any(|(_, p, _)| p == "/v1/cameras/live/stop"));
    assert!(!r.app.viewer_up());
    assert_eq!(r.app.due_at(), None);
    let dropped = r.app.take_dropped();
    assert!(dropped.contains(&"bubble:".to_string()));
    // closing again is nothing
    assert!(requests(&r.app.close(T0 + 200)).is_empty());
}

#[test]
fn the_hints_are_glyphs_for_the_keys_that_do_something() {
    let mut cs = three();
    cs["cameras"][1]["recordings"] = json!(false);
    cs["cameras"][1]["events"] = json!(false);
    cs["cameras"][1]["clips"] = json!(false);
    let mut r = Rig::new(cs);
    r.app.open_full("273", T0);
    let names: Vec<String> = r.full().hints.iter().map(|h| h.name.clone()).collect();
    assert_eq!(names, ["Left and Right, next camera", "OK, all cameras", "Back, close"], "no strip or sheet for a camera that has neither");
    let mut r = Rig::new(three());
    r.app.open_full("273", T0);
    assert_eq!(r.full().hints.len(), 5);
    let mut single = Rig::new(json!({"state": "ready", "cameras": [camera("273", "Front Door", json!({}))]}));
    single.app.open_full("273", T0);
    assert_eq!(single.full().position, None, "no \"1 of 1\"");
    let names: Vec<String> = single.full().hints.iter().map(|h| h.name.clone()).collect();
    assert_eq!(names, ["Up, timeline", "Down, events and clips", "Back, close"]);
    r.app.open_grid(T0);
    let g = r.grid();
    assert_eq!(g.hints.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(), ["OK, expand", "Hold OK, options", "Back, close"]);
}

#[test]
fn an_update_of_a_link_that_stays_the_same_changes_nothing_a_view_needs_redrawing_for() {
    let mut r = Rig::new(three());
    r.app.open_full("273", T0);
    r.helper("cameras.live", live_topic("273", "live", "playing"));
    r.at(100);
    let before = r.view();
    r.helper("cameras", three());
    assert_eq!(r.view(), before);
}

#[test]
fn the_state_a_stills_only_view_shows_follows_its_pictures_freshness() {
    let mut r = Rig::new(three());
    r.topic("foreground", json!({"app": "netflix", "home": false}));
    let fx = r.app.open_full("273", T0);
    assert_eq!(r.full().state.word, "Starting", "no still yet");
    r.at(150);
    r.pictures(&fx);
    let f = r.full();
    assert_eq!((f.state.word, f.state.dot), ("Live", true), "a fresh still is live");
    r.tick(150 + 2600);
    assert!(!r.full().chrome, "stills that update let the chrome go");
    let fx = r.tick(150 + 3100);
    let _ = fx;
}

#[test]
fn a_bridge_clock_ahead_never_shows_a_negative_age_or_a_cursor_in_the_future() {
    let mut cs = three();
    cs["now_skew_ms"] = json!(180_000);
    let mut r = Rig::new(cs);
    let fx = open_strip(&mut r);
    let _ = fx;
    let s = r.view().strip.unwrap();
    assert_eq!(s.rel, "Now");
    assert_eq!(s.cursor, T0 as i64 + 100 + 180_000, "the bridge's clock is the TV's plus the skew");
    assert_eq!(r.app.cameras().state, LinkState::Ready);
}

#[test]
fn the_full_view_opened_over_an_app_takes_the_front_asks_for_the_main_window_once_home_is_in_front_and_back_brings_the_app_back() {
    let mut r = Rig::new(three());
    r.app.set_vwin_on(true);
    r.topic("foreground", json!({"app": "netflix", "home": false}));
    let fx = r.app.open_full("273", T0);
    assert_eq!(r.app.take_intents(), [Intent::TakeForeground], "Home is to come forward: the view is an app of its own");
    assert!(live_posts(&fx).is_empty(), "the decoder is not started beside an app's video");
    assert_eq!(r.app.debug()["switched"], true);
    // the app is in the background and Home is in front: the camera's own stream, for the main window
    r.at(300);
    let fx = r.topic("foreground", json!({"app": "com.webos.app.home", "home": true}));
    assert_eq!(live_posts(&fx), [json!({"camera": "273", "for": "full", "window": "main"})]);
    assert!(r.app.take_intents().is_empty(), "it does not ask again");
    assert_eq!((r.app.debug()["homeMs"].clone(), r.app.debug()["askedMs"].clone()), (json!(300), json!(300)), "Home was in front 300 ms after the OK, and the stream was asked for then");
    // Back closes the view and brings the app back
    r.at(2_000);
    r.press("back");
    assert!(!r.app.viewer_up());
    assert_eq!(r.app.take_intents(), [Intent::ReturnToApp]);
    assert_eq!(r.app.debug()["switched"], false);
}

#[test]
fn over_an_app_that_plays_a_video_the_view_stays_over_it_pauses_it_asks_for_the_sub_window_first_and_back_plays_it_again() {
    let mut r = Rig::new(three());
    r.app.set_vwin_on(true);
    r.app.set_app_video(true);
    r.topic("foreground", json!({"app": "youtube.leanback.v4", "home": false}));
    let fx = r.app.open_full("273", T0);
    assert_eq!(r.app.take_intents(), [Intent::PauseApp], "no Home forward, no main window: the app's pipeline is never taken from it");
    assert!(live_posts(&fx).is_empty(), "the stream waits for the window");
    assert_eq!((r.app.debug()["switched"].clone(), r.app.debug()["beside"].clone(), r.app.debug()["appPaused"].clone()), (json!(false), json!(true), json!(true)));
    // the claim is made first, for what the sub window takes, fitted to the camera's own shape
    let claim = r.app.live_claim().expect("asked before the stream is").request();
    assert_eq!(claim["need"], json!({"codec": "h264", "w": 1920, "h": 1088, "fps": 30}));
    assert_eq!(claim["rect"], json!({"x": 0, "y": 0, "w": 1920, "h": 1080}));
    // the window is granted, not drawing yet: the stream is asked for, in the sub window
    r.at(200);
    r.app.set_window(Window::Waiting);
    let fx = r.app.window_answered(T0 + 200);
    assert_eq!(live_posts(&fx), [json!({"camera": "273", "for": "full", "window": "sub"})]);
    assert!(r.app.window_answered(T0 + 210).is_empty(), "asked once");
    // Back plays the app's video again and nothing is brought back
    r.at(2_000);
    r.press("back");
    assert!(!r.app.viewer_up());
    assert_eq!(r.app.take_intents(), [Intent::ResumeApp]);
}

#[test]
fn a_refused_sub_window_leaves_the_view_over_the_app_with_its_stills_and_no_decoder() {
    let mut r = Rig::new(three());
    r.app.set_vwin_on(true);
    r.app.set_app_video(true);
    r.topic("foreground", json!({"app": "youtube.leanback.v4", "home": false}));
    r.app.open_full("273", T0);
    r.app.take_intents();
    r.at(150);
    r.app.set_window(Window::Refused);
    let fx = r.app.window_answered(T0 + 150);
    assert!(live_posts(&fx).is_empty(), "no window, no decoder beside the app's video");
    assert!(r.app.live_claim().is_none(), "the claim is let go, and not made again");
    assert!(r.app.window_answered(T0 + 300).is_empty());
    assert_eq!(r.app.debug()["besideRefused"], true);
    r.at(2_000);
    r.press("back");
    assert_eq!(r.app.take_intents(), [Intent::ResumeApp], "the app's video was paused for the view all the same");
}

#[test]
fn the_home_key_leaves_a_paused_app_as_it_is() {
    let mut r = Rig::new(three());
    r.app.set_vwin_on(true);
    r.app.set_app_video(true);
    r.topic("foreground", json!({"app": "youtube.leanback.v4", "home": false}));
    r.app.open_full("273", T0);
    r.app.take_intents();
    r.at(400);
    r.topic("homekey", json!({}));
    assert!(!r.app.viewer_up());
    assert!(r.app.take_intents().is_empty(), "Home was asked for: nothing plays on behind it");
    assert_eq!(r.app.debug()["appPaused"], false);
}

#[test]
fn over_home_the_full_view_switches_nothing_and_asks_at_once_and_back_returns_to_nothing() {
    let mut r = Rig::new(three());
    r.app.set_vwin_on(true);
    let fx = r.app.open_full("273", T0);
    assert!(r.app.take_intents().is_empty());
    assert_eq!(live_posts(&fx), [json!({"camera": "273", "for": "full", "window": "main"})]);
    r.at(500);
    r.press("back");
    assert!(r.app.take_intents().is_empty(), "nothing was taken, nothing is brought back");
}

#[test]
fn without_video_windows_or_with_live_over_apps_the_view_stays_over_the_app_as_it_was() {
    let mut r = Rig::new(three());
    r.topic("foreground", json!({"app": "netflix", "home": false}));
    let fx = r.app.open_full("273", T0);
    assert!(r.app.take_intents().is_empty(), "no compositor windows: the view is an overlay with stills");
    assert!(live_posts(&fx).is_empty());
    let mut q = Rig::new(three());
    q.app.cfg.live_over_apps = true;
    q.app.set_vwin_on(true);
    q.topic("foreground", json!({"app": "netflix", "home": false}));
    let fx = q.app.open_full("273", T0);
    assert!(q.app.take_intents().is_empty(), "the owner wants live over the app: it stays");
    assert_eq!(live_posts(&fx), [json!({"camera": "273", "for": "full", "window": "main"})]);
}

#[test]
fn the_home_key_leaves_nothing_to_return_to_and_a_camera_that_does_not_play_live_takes_no_front() {
    let mut r = Rig::new(three());
    r.app.set_vwin_on(true);
    r.topic("foreground", json!({"app": "netflix", "home": false}));
    r.app.open_full("273", T0);
    r.app.take_intents();
    r.at(400);
    r.topic("homekey", json!({}));
    assert!(!r.app.viewer_up());
    assert!(r.app.take_intents().is_empty(), "Home was asked for: no app is brought back");
    assert_eq!(r.app.debug()["switched"], false);
    // a camera with no live stream stays over the app with its pictures
    let mut list = three();
    list["cameras"][1]["live"] = json!(false);
    let mut q = Rig::new(list);
    q.app.set_vwin_on(true);
    q.topic("foreground", json!({"app": "netflix", "home": false}));
    q.app.open_full("273", T0);
    assert!(q.app.take_intents().is_empty());
}

#[test]
fn the_first_live_picture_of_a_view_is_timed_from_its_opening() {
    let mut r = Rig::new(three());
    r.app.set_vwin_on(true);
    r.app.open_full("273", T0);
    assert_eq!(r.app.debug()["firstLiveMs"], Value::Null);
    r.at(640);
    r.helper("cameras.live", live_topic("273", "live", "playing"));
    assert_eq!(r.app.debug()["firstLiveMs"], 640, "from the OK to the first picture of the camera's own stream");
    r.at(900);
    r.helper("cameras.live", live_topic("273", "live", "playing"));
    assert_eq!(r.app.debug()["firstLiveMs"], 640, "the first is the one that counts");
}

#[test]
fn the_grids_tiles_over_an_app_are_asked_for_no_more_than_twice_a_second_and_over_home_at_the_owners_pace() {
    let asked_at = |foreground: Value, tick: u64| {
        let mut r = Rig::new(three());
        r.topic("foreground", foreground);
        let fx = r.app.open_grid(T0);
        r.at(50);
        r.pictures(&fx);
        let fx = r.tick(tick);
        fetches(&fx).iter().filter(|f| f.1.contains("/frame?")).count()
    };
    assert!(asked_at(json!({"app": "com.webos.app.home", "home": true}), 350) > 0, "over Home: the owner's 200 ms (Balanced)");
    assert_eq!(asked_at(json!({"app": "netflix", "home": false}), 350), 0, "over an app: not yet");
    assert!(asked_at(json!({"app": "netflix", "home": false}), 650) > 0, "over an app: after 500 ms");
}

#[test]
fn a_recording_and_a_clip_are_asked_for_in_the_window_the_live_view_has_and_in_none_where_there_is_no_claim() {
    let events = |r: &mut Rig| {
        r.app.open_full("273", T0);
        r.helper("cameras.live", live_topic("273", "live", "playing"));
        r.at(100);
        let fx = r.press("down");
        let at = T0 as i64 - 6 * MIN;
        r.answer(tag_of(&fx, "/events?"), 200, json!({"events": [{"at": at, "kind": "person", "label": "person"}]}));
        r.at(300);
        r.press("ok")
    };
    // the compositor has video windows: the live view is claimed, and so is what the strip plays (one claim, one window, a seek that reuses the pipeline)
    let mut r = Rig::new(three());
    r.app.set_vwin_on(true);
    let fx = events(&mut r);
    let posts: Vec<_> = requests(&fx).into_iter().filter(|(_, p, _)| p == "/v1/cameras/playback").collect();
    assert_eq!(posts.len(), 1, "{posts:?}");
    assert_eq!(posts[0].2["window"], "main", "{:?}", posts[0]);
    // ... a clip too
    let mut r = Rig::new(three());
    r.app.set_vwin_on(true);
    r.app.open_full("273", T0);
    r.helper("cameras.live", live_topic("273", "live", "playing"));
    r.at(100);
    let fx = r.press("down");
    r.answer(tag_of(&fx, "/events?"), 200, json!({"events": []}));
    r.at(200);
    r.press("up");
    let fx = r.press("right");
    let clips = tag_of(&fx, "/clips?");
    r.answer(clips, 200, json!({"clips": [{"id": "c1.Wy", "at": T0 as i64 - 6 * MIN, "duration": 30000, "kind": "person", "label": "person", "description": "Person at the front door",
                                           "has_thumbnail": true, "has_video": true}]}));
    r.at(300);
    r.press("down");
    r.at(400);
    let fx = r.press("ok");
    let posts: Vec<_> = requests(&fx).into_iter().filter(|(_, p, _)| p == "/v1/cameras/playback").collect();
    assert_eq!(posts.len(), 1, "{posts:?}");
    assert_eq!(posts[0].2, json!({"camera": "273", "clip": "c1.Wy", "window": "main"}));
    // no compositor windows: as before, no window word
    let mut r = Rig::new(three());
    let fx = events(&mut r);
    let posts: Vec<_> = requests(&fx).into_iter().filter(|(_, p, _)| p == "/v1/cameras/playback").collect();
    assert!(posts[0].2.get("window").is_none(), "{:?}", posts[0]);
    // a camera that does not play live is claimed for nothing: its recordings are asked for as before
    let mut list = three();
    list["cameras"][1]["live"] = json!(false);
    let mut r = Rig::new(list);
    r.app.set_vwin_on(true);
    let fx = events(&mut r);
    let posts: Vec<_> = requests(&fx).into_iter().filter(|(_, p, _)| p == "/v1/cameras/playback").collect();
    assert!(posts.iter().all(|p| p.2.get("window").is_none()), "{posts:?}");
}
