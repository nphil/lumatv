//! Cameras through Scrypted, end to end on the host: the whole helper (its loopback API, its hub, the plugin broker with the Cameras page)
//! against a fake LumaShow bridge over real TLS with a pinned certificate (`support::bridge`), a fake RTSP camera (`rtsp_client::fake`) and,
//! where it is built (`sh gfn/feeder/build.sh`), the real feeder with the fake starfish and NDL libraries. Nothing leaves the loopback.
//!
//! What is shown: nothing runs until the link is made; a linked TV connects and lists the cameras; a wrong pin sends nothing; a removed device is
//! refused; an event pops up through the policy (and only through it); stills come from one stream a camera and stop when nobody asks; the
//! timeline routes relay the bridge; live starts fast, switches, refuses without disturbing, pauses and ends in words; forgetting the link ends
//! everything; the TV's standby closes every connection.

mod support;

use ha_link::AddressPolicy;
use lumatv_helper::scrypted::link::{self, Args};
use rtsp_client::Codec;
use rtsp_client::fake::{FakeCamera, Script as CameraScript, annexb};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};
use support::bridge::{FakeBridge, STILL, pin};
use support::*;

const WAIT: Duration = Duration::from_secs(10);

fn config(cameras: Value) -> lumatv_helper::config::Config {
    test_config(json!({"cameras": cameras, "images": {"enabled": false}}))
}

struct World {
    rig: Rig,
    bridge: FakeBridge,
    verifier: String,
}

impl World {
    /// A helper with no link and a bridge that knows no device.
    fn unlinked(cameras: Value, deps: lumatv_helper::scrypted::Deps) -> Self {
        let mut rig = Rig::new();
        receiver_off(&rig.paths);
        rig.start_cameras(config(cameras), Script::new(), deps);
        Self {
            rig,
            bridge: FakeBridge::start(""),
            verifier: String::new(),
        }
    }

    /// Makes the link as the owner does (`scrypted-link --new`) and adds the printed verifier to the bridge's devices.
    fn link(&mut self) {
        self.link_with(&pin());
    }

    fn link_with(&mut self, pin: &str) {
        let args = Args {
            address: Some(self.bridge.address()),
            pin: Some(pin.to_owned()),
            new: true,
            ..Args::default()
        };
        let (code, out) = link::link(&self.rig.paths, &args, AddressPolicy::TESTING);
        assert_eq!(code, 0, "{out}");
        self.verifier = out["verifier"].as_str().unwrap().to_owned();
        self.bridge.add_device(&self.verifier);
    }

    fn wait_state(&self, state: &str) -> Value {
        self.rig
            .wait_topic("cameras", WAIT, |t| t["state"] == state)
            .unwrap_or_else(|| {
                let now = self.rig.topic("cameras").unwrap_or_default();
                let seen: Vec<String> = self
                    .rig
                    .service()
                    .hub()
                    .events_after(Some(0), None, Some(&["cameras".to_owned()]))
                    .events
                    .iter()
                    .map(|e| e.data["state"].as_str().unwrap_or("?").to_owned())
                    .collect();
                panic!(
                    "the state never became {state}: {} ({}); it went {seen:?}; the bridge saw {} connections, {} push channels open of {}; {:?}",
                    now["state"],
                    now["reason"],
                    self.bridge.connections(),
                    self.bridge.events_open(),
                    self.bridge.events_total(),
                    self.bridge.requests()
                )
            })
    }

    fn ready(&self) -> Value {
        self.rig
            .wait_topic("cameras", WAIT, |t| {
                t["state"] == "ready" && t["cameras"].as_array().is_some_and(|c| c.len() >= 2)
            })
            .unwrap_or_else(|| panic!("never ready: {:?}", self.rig.topic("cameras")))
    }

    fn get(&self, path: &str) -> Reply {
        get(self.rig.addr, path)
    }

    fn post(&self, path: &str, body: Value) -> Reply {
        post(self.rig.addr, path, &body.to_string())
    }

    fn settings(&self, changes: Value) -> Reply {
        self.post("/v1/settings/scrypted-cameras", json!({"changes": changes}))
    }

    fn popup(&self) -> Option<Value> {
        self.rig.topic("cameras.popup").filter(|v| !v.is_null())
    }

    fn wait_popup(&self, check: impl Fn(&Value) -> bool) -> Option<Value> {
        self.rig
            .wait_topic("cameras.popup", WAIT, |v| !v.is_null() && check(v))
    }
}

fn linked_world(cameras: Value) -> World {
    let mut world = World::unlinked(cameras, lumatv_helper::scrypted::testing::deps());
    world.link();
    world.ready();
    world
}

// ---- off until linked, and the states ------------------------------------------------------------------------------------------------

#[test]
fn nothing_runs_until_the_tv_is_linked_and_the_topics_say_so() {
    let world = World::unlinked(json!({}), lumatv_helper::scrypted::testing::deps());
    let topic = world.rig.topic("cameras").unwrap();
    assert_eq!(topic["state"], "unlinked");
    assert_eq!(topic["cameras"], json!([]));
    assert!(
        topic["reason"].as_str().unwrap().contains("scrypted-link"),
        "{topic}"
    );
    assert!(world.popup().is_none());
    assert_eq!(world.rig.topic("cameras.live").unwrap()["state"], "idle");
    assert_eq!(
        world.rig.topic("cameras.events").unwrap()["events"],
        json!([])
    );
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        world.bridge.connections(),
        0,
        "no connection is made without a link"
    );
    // the routes answer in words and fetch nothing
    let reply = world.get("/v1/cameras/273/frame?w=480&h=270");
    assert_eq!(reply.status, 404, "an unlinked TV shows no camera at all");
    let reply = world.post("/v1/cameras/live", json!({"camera": "273"}));
    assert_eq!(reply.status, 404);
    assert_eq!(
        world.get("/v1/cameras").json()["cameras"]["state"],
        "unlinked"
    );
    // the page exists and says it is not linked; its camera picker says so too
    let facts = world.rig.topic("plugin.scrypted-cameras.facts").unwrap();
    assert!(
        facts["facts"]["link"]
            .as_str()
            .unwrap()
            .contains("not linked"),
        "{facts}"
    );
    assert_eq!(
        (facts["facts"].get("device"), facts["facts"].get("verifier")),
        (None, None),
        "an unlinked TV has no name or verifier to show"
    );
    let choices = world
        .get("/v1/plugins/scrypted-cameras/choices/cameras")
        .json();
    assert_eq!(choices["page"]["state"], "unconfigured", "{choices}");
    assert_eq!(world.bridge.connections(), 0);
}

#[test]
fn a_linked_tv_connects_lists_the_cameras_and_keeps_the_bridges_clock() {
    let mut world = World::unlinked(json!({}), lumatv_helper::scrypted::testing::deps());
    world.bridge.set_skew_ms(1500);
    world.link();
    let topic = world.ready();
    assert_eq!(topic["reason"], "");
    let cameras = topic["cameras"].as_array().unwrap();
    assert_eq!(cameras.len(), 2);
    let door = &cameras[0];
    assert_eq!(
        (
            door["id"].as_str(),
            door["name"].as_str(),
            door["room"].as_str(),
            door["type"].as_str()
        ),
        (
            Some("273"),
            Some("Front Door"),
            Some("Entry"),
            Some("doorbell")
        )
    );
    assert_eq!(
        (
            door["online"].clone(),
            door["live"].clone(),
            door["recordings"].clone(),
            door["events"].clone(),
            door["clips"].clone()
        ),
        (
            json!(true),
            json!(true),
            json!(true),
            json!(true),
            json!(true)
        )
    );
    assert_eq!(
        door["native"],
        json!({"codec": "h264", "width": 640, "height": 360, "fps": 15.0})
    );
    assert_eq!(
        (door["prebuffered"].clone(), door["since"].clone()),
        (json!(true), json!(1_792_500_000_000i64))
    );
    // bridge minus TV: the bridge's clock is 1.5 s ahead
    let skew = topic["now_skew_ms"].as_i64().unwrap();
    assert!((1400..=1600).contains(&skew), "{skew}");
    assert!(topic["rev"].as_u64().unwrap() >= 1);
    // one kept-alive connection for the calls and one for the push channel: not a connection a call
    assert!(wait_until(WAIT, || world.bridge.events_open() == 1));
    assert!(
        world.bridge.connections() <= 3,
        "{}",
        world.bridge.connections()
    );
    let asked = world.bridge.requests();
    assert!(
        asked.iter().any(|r| r.starts_with("GET /v1/health"))
            && asked
                .iter()
                .any(|r| r.starts_with("GET /v1/cameras?scope=all")),
        "{asked:?}"
    );
    // the page: a pick of events for each camera, with the design's defaults
    let page = world.get("/v1/settings/scrypted-cameras").json();
    let keys: Vec<String> = page["schema"]["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| {
            s["fields"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f["key"].as_str().unwrap().to_owned())
        })
        .collect();
    assert!(
        keys.contains(&"notify.273".to_owned()) && keys.contains(&"notify.274".to_owned()),
        "{keys:?}"
    );
    assert_eq!(
        page["effective"]["notify.273"],
        json!({"type": "ordered", "value": ["person", "doorbell"]})
    );
    assert_eq!(
        page["effective"]["popups.seconds"],
        json!({"type": "integer", "value": 12})
    );
    let facts = world.rig.topic("plugin.scrypted-cameras.facts").unwrap();
    assert_eq!(facts["facts"]["link"], "Connected \u{b7} 2 cameras");
    assert_eq!(
        (
            facts["facts"]["device"].as_str(),
            facts["facts"]["verifier"].as_str()
        ),
        (Some("LumaTV"), Some(world.verifier.as_str())),
        "the name and verifier the owner adds to the bridge are on the page"
    );
    let choices = world
        .get("/v1/plugins/scrypted-cameras/choices/cameras")
        .json();
    assert_eq!(
        choices["page"]["rows"].as_array().unwrap().len(),
        2,
        "{choices}"
    );
    assert_eq!(choices["page"]["rows"][0]["detail"], "Entry");
    let kinds = world
        .get("/v1/plugins/scrypted-cameras/choices/kinds")
        .json();
    assert_eq!(kinds["page"]["rows"][4]["id"], "doorbell");
}

#[test]
fn nothing_secret_is_in_a_topic_a_route_or_the_log_and_the_pin_is_the_only_trust() {
    let world = linked_world(json!({}));
    let token = std::fs::read_to_string(world.rig.paths.scrypted_link_file()).unwrap();
    let token = serde_json::from_str::<Value>(&token).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let everything = serde_json::to_string(&get(world.rig.addr, "/v1/state").json()).unwrap();
    for needle in [token.as_str(), &token[4..], "127.0.0.1:", "rtsp://", &pin()] {
        assert!(
            !everything.contains(needle),
            "{needle} is in the hub's topics"
        );
    }
    // the file is the owner's: private, in a private directory
    use std::os::unix::fs::PermissionsExt;
    let file = world.rig.paths.scrypted_link_file();
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(file.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    // nothing that can reach the loopback API writes it: not the settings, not a plugin action, not a route
    let settings = world.post(
        "/v1/settings/scrypted-cameras",
        json!({"changes": [{"key": "link.token", "value": {"type": "text", "value": "x"}}]}),
    );
    assert_eq!(settings.status, 422);
    assert_eq!(
        world
            .post(
                "/v1/cameras/link",
                json!({"address": "10.0.0.1", "token": "x"})
            )
            .status,
        404
    );
    assert_eq!(
        world
            .post("/v1/plugins/scrypted-cameras/actions/link", json!({}))
            .status,
        404
    );
}

#[test]
fn a_wrong_pin_is_untrusted_and_nothing_is_sent() {
    let mut world = World::unlinked(json!({}), lumatv_helper::scrypted::testing::deps());
    let wrong = pin().replace(&pin()[..4], "0000");
    world.link_with(&wrong);
    world.wait_state("untrusted");
    let topic = world.rig.topic("cameras").unwrap();
    assert_eq!(topic["cameras"], json!([]));
    assert!(
        topic["reason"]
            .as_str()
            .unwrap()
            .contains("not the one pinned"),
        "{topic}"
    );
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        world.bridge.requests().is_empty(),
        "the token never reached the bridge: {:?}",
        world.bridge.requests()
    );
    assert_eq!(world.bridge.unauthorized(), 0);
    // the owner mends the pin (the token stays): the link comes up by itself
    let (code, _) = link::link(
        &world.rig.paths,
        &Args {
            pin: Some(pin()),
            ..Args::default()
        },
        AddressPolicy::TESTING,
    );
    assert_eq!(code, 0);
    world.ready();
}

#[test]
fn a_device_the_bridge_removed_is_refused_and_comes_back_when_added_again() {
    let world = linked_world(json!({}));
    world.bridge.remove_device(&world.verifier);
    let topic = world.wait_state("refused");
    assert_eq!(topic["reason"], "Scrypted refused this TV");
    assert_eq!(
        topic["cameras"],
        json!([]),
        "a TV that is refused shows no cameras"
    );
    // the stills' streams and the push channel end with it
    assert!(wait_until(WAIT, || world.bridge.events_open() == 0));
    world.bridge.add_device(&world.verifier);
    world.ready();
}

#[test]
fn a_bridge_that_does_not_answer_is_offline_with_its_last_list_and_the_tv_reconnects_by_itself() {
    let world = linked_world(json!({}));
    world.bridge.refuse(true);
    let topic = world.wait_state("offline");
    assert_eq!(
        topic["cameras"].as_array().unwrap().len(),
        2,
        "the last list is kept"
    );
    assert!(
        topic["reason"].as_str().unwrap().contains("not answering"),
        "{topic}"
    );
    world.bridge.refuse(false);
    let back = world.ready();
    assert_eq!(back["cameras"].as_array().unwrap().len(), 2);
}

#[test]
fn the_tvs_standby_closes_every_connection_and_the_wake_opens_them_again() {
    let world = linked_world(json!({}));
    assert!(wait_until(WAIT, || world.bridge.events_open() == 1));
    let hub = world.rig.service().hub();
    hub.set("screen", json!({"on": false, "state": "Active Standby"}));
    assert!(
        wait_until(WAIT, || world.bridge.events_open() == 0),
        "the push channel closes in standby"
    );
    let topic = world.wait_state("offline");
    assert!(
        topic["reason"].as_str().unwrap().contains("asleep"),
        "{topic}"
    );
    assert_eq!(
        topic["cameras"].as_array().unwrap().len(),
        2,
        "the list is there when the TV wakes"
    );
    let before = world.bridge.connections();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        world.bridge.connections(),
        before,
        "nothing connects while the TV sleeps"
    );
    hub.set("screen", json!({"on": true, "state": "Active"}));
    world.ready();
    assert!(wait_until(WAIT, || world.bridge.events_open() == 1));
}

#[test]
fn forgetting_the_link_in_settings_ends_everything_at_once() {
    let world = linked_world(json!({}));
    assert!(wait_until(WAIT, || world.bridge.events_open() == 1));
    // a picture is being served
    assert_eq!(world.get("/v1/cameras/273/frame?w=480&h=270").status, 200);
    assert!(wait_until(WAIT, || world.bridge.stills_open() == 1));
    let reply = world.post("/v1/plugins/scrypted-cameras/actions/forget", json!({}));
    assert_eq!(
        reply.status,
        202,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    let topic = world
        .rig
        .wait_topic("cameras", WAIT, |t| t["state"] == "unlinked")
        .expect("unlinked");
    assert_eq!(topic["cameras"], json!([]));
    assert!(
        !world.rig.paths.scrypted_link_file().exists(),
        "the file is gone"
    );
    assert!(wait_until(WAIT, || world.bridge.events_open() == 0
        && world.bridge.stills_open() == 0));
    assert!(world.popup().is_none());
    assert_eq!(
        world.rig.topic("cameras.events").unwrap()["events"],
        json!([])
    );
    let before = world.bridge.connections();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(world.bridge.connections(), before, "nothing reconnects");
    let facts = world.rig.topic("plugin.scrypted-cameras.facts").unwrap();
    assert!(
        facts["facts"]["link"]
            .as_str()
            .unwrap()
            .contains("not linked")
    );
    assert_eq!(
        (facts["facts"].get("device"), facts["facts"].get("verifier")),
        (None, None)
    );
}

#[test]
fn unlinking_with_the_command_does_the_same_within_a_look() {
    let world = linked_world(json!({}));
    let (code, out) = link::unlink(&world.rig.paths);
    assert_eq!((code, out["was"].clone()), (0, json!(true)));
    world.wait_state("unlinked");
    assert!(wait_until(WAIT, || world.bridge.events_open() == 0));
}

// ---- a camera on a battery ---------------------------------------------------------------------------------------------------------

#[test]
fn a_camera_on_a_battery_is_dialled_only_for_the_full_view_the_owner_opened_and_its_pop_up_shows_stills() {
    // a feeder that is configured and never started: the live path is on (so a card may offer live video), and nothing plays
    let dir = tempfile::tempdir().unwrap();
    let mut deps = lumatv_helper::scrypted::testing::deps();
    deps.feeder = Some(lumatv_helper::live::default_spec(dir.path(), "card", "", Duration::from_secs(1)));
    let mut world = World::unlinked(json!({"live": true}), deps);
    let mut door = FakeBridge::camera("273", "Front Door", "doorbell");
    door["battery"] = json!(true);
    world.bridge.set_cameras(vec![door, FakeBridge::camera("274", "Garage", "camera")]);
    world.link();
    let topic = world.ready();
    assert_eq!((topic["cameras"][0]["battery"].clone(), topic["cameras"][1]["battery"].clone()), (json!(true), json!(false)));
    let code = |reply: Reply| reply.json()["error"]["code"].as_str().unwrap_or_default().to_owned();

    // nothing but the owner opening that camera's full view may start its call: not a pop-up, not a view the screen or a swipe turned to it
    for body in [
        json!({"camera": "273"}),
        json!({"camera": "273", "for": "full", "window": "main"}),
        json!({"camera": "273", "for": "popup", "window": "main"}),
        json!({"camera": "273", "for": "popup", "window": "texture", "open": true}),
        json!({"camera": "273", "quality": "light"}),
    ] {
        let reply = world.post("/v1/cameras/live", body.clone());
        assert_eq!((reply.status, code(reply)), (409, "battery_still_only".to_owned()), "{body}");
    }
    assert_eq!(world.bridge.requests_matching("POST /v1/live"), 0, "the camera was never asked for");

    // the opened full view is past the gate and asks the bridge; a camera on the mains never was behind it
    for body in [json!({"camera": "273", "open": true}), json!({"camera": "273", "for": "full", "window": "main", "open": true}), json!({"camera": "274"})] {
        let before = world.bridge.requests_matching("POST /v1/live");
        let reply = world.post("/v1/cameras/live", body.clone());
        assert_ne!(code(reply), "battery_still_only", "{body}");
        assert_eq!(world.bridge.requests_matching("POST /v1/live"), before + 1, "{body}");
    }

    // its pop-up card is stills only
    assert!(wait_until(WAIT, || world.bridge.events_open() == 1));
    world.bridge.push_event(world.bridge.event("273", "person", "new"));
    let popup = world.wait_popup(|_| true).expect("a card");
    assert_eq!(popup["live"], json!(false), "{popup}");
    world.bridge.push_event(world.bridge.event("274", "person", "new"));
    let popup = world.wait_popup(|p| p["camera"] == "274").expect("a card for the garage");
    assert_eq!(popup["live"], json!(true), "a camera on the mains still offers live video: {popup}");
}

// ---- events and pop-ups ----------------------------------------------------------------------------------------------------------

#[test]
fn a_person_at_the_door_pops_up_within_a_moment_with_everything_the_card_needs() {
    let world = linked_world(json!({}));
    assert!(wait_until(WAIT, || world.bridge.events_open() == 1));
    let started = Instant::now();
    let seq = world
        .bridge
        .push_event(world.bridge.event("273", "person", "new"));
    let popup = world.wait_popup(|_| true).expect("a card");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(popup["camera"], "273");
    assert_eq!(popup["name"], "Front Door");
    assert_eq!(
        (popup["kind"].as_str(), popup["label"].as_str()),
        (Some("person"), Some("person"))
    );
    assert_eq!(popup["seq"], seq);
    assert_eq!(popup["event"], format!("e{seq}"));
    assert_eq!(
        (popup["thumbnail"].clone(), popup["overApps"].clone()),
        (json!(true), json!(true))
    );
    let shown = popup["shownAt"].as_i64().unwrap();
    let until = popup["untilMs"].as_i64().unwrap();
    assert_eq!(until - shown, 12_000, "the card stays 12 s");
    // the event is in the list too, as the bridge sent it
    let events = world.rig.topic("cameras.events").unwrap();
    assert_eq!(events["events"][0]["id"], format!("e{seq}"));
    assert_eq!(events["events"][0]["phase"], "new");
    // the event's own picture, asked for at once and kept
    let before = world.bridge.requests_matching("GET /v1/event-thumbnail");
    let picture = world.get(&format!("/v1/cameras/events/e{seq}/thumbnail?w=520&h=292"));
    assert_eq!(
        picture.status,
        200,
        "{}",
        String::from_utf8_lossy(&picture.body)
    );
    assert_eq!(picture.body, STILL);
    assert_eq!(
        world
            .get(&format!("/v1/cameras/events/e{seq}/thumbnail?w=520&h=292"))
            .status,
        200
    );
    assert_eq!(
        world.bridge.requests_matching("GET /v1/event-thumbnail") - before,
        1,
        "the second ask is the TV's own"
    );
    assert_eq!(world.get("/v1/cameras/events/nope/thumbnail").status, 404);
    // Back dismisses it: every reader sees it go
    assert_eq!(world.post("/v1/cameras/popup/clear", json!({})).status, 200);
    assert!(world.popup().is_none());
}

#[test]
fn a_camera_with_no_calm_stream_pops_up_as_a_texture_and_one_with_a_low_stream_in_a_window() {
    let world = linked_world(json!({}));
    assert!(wait_until(WAIT, || world.bridge.events_open() == 1));
    let only = |id: &str, streams: Value| {
        let mut doc = FakeBridge::camera(id, id, "camera");
        doc["native"] = json!({"codec": "h264", "width": 1920, "height": 1080, "fps": 25});
        doc["streams"] = streams;
        doc
    };
    let native = json!({"quality": "native", "codec": "h264", "width": 1920, "height": 1080, "fps": 25, "prebuffered": true});
    let light = json!({"quality": "light", "codec": "h264", "width": 640, "height": 360, "fps": 15, "prebuffered": false});
    world.bridge.set_cameras(vec![only("240", json!([native.clone()])), only("241", json!([native, light]))]);
    let list = world
        .rig
        .wait_topic("cameras", WAIT, |t| t["cameras"].as_array().is_some_and(|c| c.iter().any(|c| c["id"] == "240")))
        .expect("two cameras");
    let by = |id: &str| list["cameras"].as_array().unwrap().iter().find(|c| c["id"] == id).unwrap().clone();
    assert_eq!((by("240")["popupVideo"].as_str(), by("241")["popupVideo"].as_str()), (Some("texture"), Some("window")));
    assert_eq!(by("241")["streams"][1]["quality"], "light", "the list says which streams a camera has");
    world.bridge.push_event(world.bridge.event("240", "person", "new"));
    let popup = world.wait_popup(|p| p["camera"] == "240").expect("a card for 240");
    assert_eq!(popup["textureOnly"], true, "1080p at 25 would take the panel to 100 Hz");
    world.post("/v1/cameras/popup/clear", json!({}));
    world.bridge.push_event(world.bridge.event("241", "person", "new"));
    let popup = world.wait_popup(|p| p["camera"] == "241").expect("a card for 241");
    assert_eq!(popup["textureOnly"], false, "its low stream plays in a window");
}

#[test]
fn only_the_kinds_the_owner_picked_pop_up_and_the_defaults_are_a_person_and_a_doorbell() {
    let world = linked_world(json!({}));
    assert!(wait_until(WAIT, || world.bridge.events_open() == 1));
    // an animal: heard (it is in the list) and no card, by default
    world
        .bridge
        .push_event(world.bridge.event("274", "animal", "new"));
    assert!(
        world
            .rig
            .wait_topic("cameras.events", WAIT, |e| e["events"]
                .as_array()
                .is_some_and(|l| l.len() == 1))
            .is_some()
    );
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        world.popup().is_none(),
        "a bird or a cat is not a pop-up a minute"
    );
    // the owner asks for animals at the garage (and for the person to stay)
    let reply = world.settings(
        json!([{"key": "notify.274", "value": {"type": "ordered", "value": ["person", "animal"]}}]),
    );
    assert_eq!(
        reply.status,
        200,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    world
        .bridge
        .push_event(world.bridge.event("274", "animal", "new"));
    let popup = world.wait_popup(|_| true).expect("the animal pops up now");
    assert_eq!(
        (popup["camera"].as_str(), popup["kind"].as_str()),
        (Some("274"), Some("animal"))
    );
    // a vehicle at the door was never asked for
    world.post("/v1/cameras/popup/clear", json!({}));
    world
        .bridge
        .push_event(world.bridge.event("273", "vehicle", "new"));
    std::thread::sleep(Duration::from_millis(300));
    assert!(world.popup().is_none());
    // the pop-ups can be turned off, and the stay is the page's
    world.settings(json!([{"key": "popups.on", "value": {"type": "bool", "value": false}}]));
    world
        .bridge
        .push_event(world.bridge.event("273", "person", "new"));
    std::thread::sleep(Duration::from_millis(300));
    assert!(world.popup().is_none(), "off is off");
    world.settings(json!([{"key": "popups.on", "value": {"type": "bool", "value": true}}, {"key": "popups.seconds", "value": {"type": "integer", "value": 30}},
                          {"key": "popups.over_apps", "value": {"type": "bool", "value": false}}]));
    world
        .bridge
        .push_event(world.bridge.event("273", "person", "new"));
    let popup = world.wait_popup(|_| true).unwrap();
    assert_eq!(
        popup["untilMs"].as_i64().unwrap() - popup["shownAt"].as_i64().unwrap(),
        30_000
    );
    assert_eq!(
        popup["overApps"], false,
        "the shell is told whether it may show it over an app"
    );
}

#[test]
fn the_same_event_again_waits_for_the_cooldown_a_doorbell_never_does_and_an_update_changes_the_words()
 {
    let world = linked_world(json!({}));
    assert!(wait_until(WAIT, || world.bridge.events_open() == 1));
    world
        .bridge
        .push_event(world.bridge.event("273", "person", "new"));
    let first = world.wait_popup(|_| true).unwrap();
    // the same object gains a name: the words of the same card change, and it is not a new card
    let mut update = world.bridge.event("273", "person", "update");
    update["label"] = json!("Nitin");
    let seq = world.bridge.push_event(update);
    let changed = world
        .wait_popup(|p| p["label"] == "Nitin")
        .expect("the words change");
    assert_eq!(
        (changed["id"].clone(), changed["untilMs"].clone()),
        (first["id"].clone(), first["untilMs"].clone())
    );
    assert_eq!(changed["seq"], seq);
    // a second person within the cooldown, and an end: no new card
    world
        .bridge
        .push_event(world.bridge.event("273", "person", "new"));
    world
        .bridge
        .push_event(world.bridge.event("273", "person", "end"));
    assert!(
        world
            .rig
            .wait_topic("cameras.events", WAIT, |e| e["events"]
                .as_array()
                .is_some_and(|l| l.len() == 4))
            .is_some()
    );
    assert_eq!(
        world.popup().unwrap()["id"],
        first["id"],
        "the cooldown held the second person back"
    );
    // two rings are two cards, the second half as long again as a person's
    world
        .bridge
        .push_event(world.bridge.event("273", "doorbell", "new"));
    let ring = world.wait_popup(|p| p["kind"] == "doorbell").unwrap();
    assert_eq!(
        ring["untilMs"].as_i64().unwrap() - ring["shownAt"].as_i64().unwrap(),
        18_000
    );
    world
        .bridge
        .push_event(world.bridge.event("273", "doorbell", "new"));
    let again = world
        .wait_popup(|p| p["kind"] == "doorbell" && p["id"] != ring["id"])
        .expect("a second ring is a second card");
    assert!(again["id"].as_u64().unwrap() > ring["id"].as_u64().unwrap());
}

#[test]
fn history_the_bridge_replays_is_listed_and_never_a_card_and_a_card_goes_by_itself() {
    let world = linked_world(json!({}));
    assert!(wait_until(WAIT, || world.bridge.events_open() == 1));
    let mut old = world.bridge.event("273", "person", "new");
    old["at"] = json!(world.bridge.now() - 120_000);
    world.bridge.push_event(old);
    assert!(
        world
            .rig
            .wait_topic("cameras.events", WAIT, |e| e["events"]
                .as_array()
                .is_some_and(|l| l.len() == 1))
            .is_some()
    );
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        world.popup().is_none(),
        "a person two minutes ago is not news"
    );
    // a card of the shortest stay goes by itself, and the topic says so
    world.settings(json!([{"key": "popups.seconds", "value": {"type": "integer", "value": 5}}]));
    world
        .bridge
        .push_event(world.bridge.event("273", "person", "new"));
    world.wait_popup(|_| true).unwrap();
    assert!(
        wait_until(Duration::from_secs(8), || world.popup().is_none()),
        "the card ends at untilMs"
    );
}

#[test]
fn a_reconnect_resumes_after_the_last_event_and_a_reset_starts_from_now() {
    let world = linked_world(json!({}));
    assert!(wait_until(WAIT, || world.bridge.events_open() == 1));
    let seq = world
        .bridge
        .push_event(world.bridge.event("274", "motion", "new"));
    assert!(
        world
            .rig
            .wait_topic("cameras.events", WAIT, |e| e["events"]
                .as_array()
                .is_some_and(|l| l.len() == 1))
            .is_some()
    );
    world.bridge.kill_streams();
    assert!(wait_until(WAIT, || world.bridge.events_total() >= 2));
    assert!(
        wait_until(WAIT, || world.bridge.events_open() == 1),
        "the channel opens again by itself"
    );
    assert_eq!(
        world.bridge.last_event_ids()[1].as_deref(),
        Some(seq.to_string().as_str()),
        "it resumes after what it heard"
    );
    // a bridge that restarted knows nothing of that id: it says so, and the next reconnect asks for nothing
    world.bridge.clear_events();
    world.bridge.kill_streams();
    assert!(wait_until(WAIT, || world.bridge.events_total() >= 3));
    assert!(wait_until(WAIT, || world.bridge.events_open() == 1));
    world.bridge.kill_streams();
    assert!(wait_until(WAIT, || world.bridge.events_total() >= 4));
    assert_eq!(
        world.bridge.last_event_ids()[3],
        None,
        "after a reset nothing is resumed"
    );
}

#[test]
fn motion_is_asked_of_the_bridge_only_when_a_camera_wants_it() {
    let world = linked_world(json!({}));
    assert!(wait_until(WAIT, || world.bridge.events_open() == 1));
    assert!(
        world
            .bridge
            .requests()
            .iter()
            .any(|r| r == "GET /v1/events/stream"),
        "{:?}",
        world.bridge.requests()
    );
    world.settings(
        json!([{"key": "notify.274", "value": {"type": "ordered", "value": ["person", "motion"]}}]),
    );
    assert!(
        wait_until(WAIT, || world.bridge.requests().iter().any(|r| r
            .contains("/v1/events/stream?kinds=")
            && r.contains("motion"))),
        "{:?}",
        world.bridge.requests()
    );
    assert!(
        wait_until(WAIT, || world.bridge.events_open() == 1),
        "one channel, the old one closed"
    );
}

#[test]
fn the_test_event_route_is_off_unless_the_switch_is_on_and_goes_through_the_policy() {
    let world = linked_world(json!({}));
    let reply = world.post(
        "/v1/cameras/event",
        json!({"camera": "273", "kind": "person"}),
    );
    assert_eq!(reply.status, 403);
    drop(world);
    let world = linked_world(json!({"testEvents": true}));
    assert_eq!(
        world
            .post(
                "/v1/cameras/event",
                json!({"camera": "273", "kind": "ghost"})
            )
            .status,
        400
    );
    assert_eq!(
        world
            .post(
                "/v1/cameras/event",
                json!({"camera": "999", "kind": "person"})
            )
            .status,
        404
    );
    let reply = world
        .post(
            "/v1/cameras/event",
            json!({"camera": "273", "kind": "person"}),
        )
        .json();
    assert_eq!(
        (
            reply["ok"].clone(),
            reply["popup"].clone(),
            reply["lastsMs"].clone()
        ),
        (json!(true), json!(true), json!(12_000))
    );
    let popup = world.wait_popup(|_| true).unwrap();
    assert_eq!(popup["test"], true);
    let held = world
        .post(
            "/v1/cameras/event",
            json!({"camera": "273", "kind": "person"}),
        )
        .json();
    assert_eq!(held["held"], "cooldown");
    let kind = world
        .post(
            "/v1/cameras/event",
            json!({"camera": "273", "kind": "vehicle"}),
        )
        .json();
    assert_eq!(kind["held"], "kind");
}

// ---- the camera list and the page ------------------------------------------------------------------------------------------------

#[test]
fn the_show_setting_filters_and_orders_the_cameras_and_a_new_camera_rebuilds_the_page() {
    let world = linked_world(json!({}));
    let rev = world.rig.topic("cameras").unwrap()["rev"].as_u64().unwrap();
    world.settings(json!([{"key": "show", "value": {"type": "ordered", "value": ["274"]}}]));
    let topic = world
        .rig
        .wait_topic("cameras", WAIT, |t| {
            t["cameras"].as_array().is_some_and(|c| c.len() == 1)
        })
        .expect("filtered");
    assert_eq!(topic["cameras"][0]["id"], "274");
    assert!(topic["rev"].as_u64().unwrap() > rev);
    // an event of a camera that is not shown is not heard
    assert!(wait_until(WAIT, || world.bridge.events_open() == 1));
    world
        .bridge
        .push_event(world.bridge.event("273", "person", "new"));
    std::thread::sleep(Duration::from_millis(300));
    assert!(world.popup().is_none());
    assert_eq!(
        world.rig.topic("cameras.events").unwrap()["events"],
        json!([])
    );
    assert_eq!(
        world.get("/v1/cameras/273/frame").status,
        404,
        "a camera the TV does not show is not there"
    );
    // the order is the owner's
    world.settings(json!([{"key": "show", "value": {"type": "ordered", "value": ["274", "273"]}}]));
    let topic = world
        .rig
        .wait_topic("cameras", WAIT, |t| {
            t["cameras"].as_array().is_some_and(|c| c.len() == 2)
        })
        .unwrap();
    assert_eq!(
        (
            topic["cameras"][0]["id"].as_str(),
            topic["cameras"][1]["id"].as_str()
        ),
        (Some("274"), Some("273"))
    );
    // Scrypted shares a third camera: the list, and the page with a pick for it, follow within a refresh
    let revision = world.get("/v1/settings/scrypted-cameras").json()["schemaRevision"]
        .as_u64()
        .unwrap();
    world.settings(json!([{"key": "show", "value": {"type": "ordered", "value": []}}]));
    world.bridge.set_cameras(vec![
        FakeBridge::camera("273", "Front Door", "doorbell"),
        FakeBridge::camera("274", "Garage", "camera"),
        FakeBridge::camera("275", "Yard", "camera"),
    ]);
    let topic = world
        .rig
        .wait_topic("cameras", WAIT, |t| {
            t["cameras"].as_array().is_some_and(|c| c.len() == 3)
        })
        .expect("three cameras");
    assert_eq!(topic["cameras"][2]["name"], "Yard");
    assert!(wait_until(WAIT, || {
        let page = world.get("/v1/settings/scrypted-cameras").json();
        page["schemaRevision"].as_u64().unwrap() > revision
            && page["schema"].to_string().contains("notify.275")
    }));
    // what the owner picked for a camera is kept when it leaves and comes back
    world.settings(
        json!([{"key": "notify.275", "value": {"type": "ordered", "value": ["package"]}}]),
    );
    world
        .bridge
        .set_cameras(vec![FakeBridge::camera("273", "Front Door", "doorbell")]);
    assert!(
        world
            .rig
            .wait_topic("cameras", WAIT, |t| t["cameras"]
                .as_array()
                .is_some_and(|c| c.len() == 1))
            .is_some()
    );
    world.bridge.set_cameras(vec![
        FakeBridge::camera("273", "Front Door", "doorbell"),
        FakeBridge::camera("275", "Yard", "camera"),
    ]);
    assert!(
        world
            .rig
            .wait_topic("cameras", WAIT, |t| t["cameras"]
                .as_array()
                .is_some_and(|c| c.len() == 2))
            .is_some()
    );
    // the page follows the list a moment later, with the pick the owner made
    let picked = json!({"type": "ordered", "value": ["package"]});
    assert!(
        wait_until(
            WAIT,
            || world.get("/v1/settings/scrypted-cameras").json()["effective"]["notify.275"]
                == picked
        ),
        "{}",
        world.get("/v1/settings/scrypted-cameras").json()["effective"]
    );
}

// ---- stills ----------------------------------------------------------------------------------------------------------------------

#[test]
fn a_grid_of_asks_costs_one_stream_a_camera_the_newest_picture_is_served_and_the_stream_ends_when_nobody_asks()
 {
    let world = linked_world(json!({}));
    world.bridge.set_stills_fps(20);
    // the first ask waits for the first picture
    let started = Instant::now();
    let first = world.get("/v1/cameras/273/frame?w=480&h=270");
    assert_eq!(
        first.status,
        200,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(
        first.body.starts_with(&STILL[..STILL.len() - 2]),
        "the bridge's picture, as it is"
    );
    assert!(first.header("X-Luma-Camera-Age-Ms").is_some());
    // twenty asks from a grid of tiles: still one stream
    let mut last = 0u32;
    for _ in 0..20 {
        let reply = world.get("/v1/cameras/273/frame?w=480&h=270");
        assert_eq!(reply.status, 200);
        let tail = String::from_utf8_lossy(&reply.body[STILL.len()..]).to_string();
        let n: u32 = tail
            .trim_matches(|c| c == '<' || c == '>')
            .parse()
            .unwrap_or(0);
        assert!(n >= last, "pictures never go back");
        last = n;
        std::thread::sleep(Duration::from_millis(30));
    }
    assert!(
        last >= 3,
        "the newest picture is served as the stream brings new ones: {last}"
    );
    assert_eq!(world.bridge.stills_total(), 1);
    assert_eq!(world.bridge.stills_open(), 1);
    // a second camera is a second stream, at the width the tile asked for
    assert_eq!(world.get("/v1/cameras/274/frame?w=900&h=506").status, 200);
    assert_eq!(world.bridge.stills_open(), 2);
    let queries = world.bridge.stills_queries();
    assert!(
        queries.contains(&"camera=273&w=480&fps=5".to_owned()),
        "{queries:?}"
    );
    assert!(
        queries.contains(&"camera=274&w=960&fps=5".to_owned()),
        "{queries:?}"
    );
    // nobody asks: three seconds later the connections are closed and the bridge stops its ffmpeg
    assert!(
        wait_until(Duration::from_secs(8), || world.bridge.stills_open() == 0),
        "stills still open: {}",
        world.bridge.stills_open()
    );
    // a camera nobody shares, and a bad size
    assert_eq!(world.get("/v1/cameras/999/frame").status, 404);
    assert_eq!(world.get("/v1/cameras/273/frame?w=abc").status, 400);
}

#[test]
fn a_still_older_than_the_hold_is_no_picture_and_never_a_stale_one() {
    let world = linked_world(json!({"frameHoldMs": 700}));
    assert_eq!(world.get("/v1/cameras/273/frame?w=480&h=270").status, 200);
    // the bridge's streams end and it does not answer: no new picture comes
    world.bridge.kill_streams();
    world.bridge.refuse(true);
    std::thread::sleep(Duration::from_millis(900));
    let reply = world.get("/v1/cameras/273/frame?w=480&h=270");
    assert!(matches!(reply.status, 404 | 503), "{}", reply.status);
    let body = reply.json();
    assert!(
        matches!(
            body["error"]["code"].as_str(),
            Some("no_picture" | "not_connected")
        ),
        "{body}"
    );
}

#[test]
fn the_grid_setting_sets_the_streams_rate() {
    let world = linked_world(json!({}));
    world.settings(json!([{"key": "video.grid", "value": {"type": "choice", "value": "smooth"}}]));
    assert_eq!(world.get("/v1/cameras/273/frame?w=480&h=270").status, 200);
    assert!(
        world
            .bridge
            .stills_queries()
            .contains(&"camera=273&w=480&fps=8".to_owned()),
        "{:?}",
        world.bridge.stills_queries()
    );
}

// ---- the timeline ----------------------------------------------------------------------------------------------------------------

#[test]
fn the_timeline_routes_give_the_bridges_own_json_and_pictures_and_its_refusals_in_words() {
    let world = linked_world(json!({}));
    let now = world.bridge.now();
    let from = now - 3_600_000;
    let recordings = world
        .get(&format!("/v1/cameras/273/recordings?from={from}&to={now}"))
        .json();
    assert_eq!(recordings["ok"], true);
    assert_eq!(
        (
            recordings["from"].as_i64(),
            recordings["to"].as_i64(),
            recordings["step_ms"].as_i64(),
            recordings["complete"].clone()
        ),
        (Some(from), Some(now), Some(60000), json!(true))
    );
    assert_eq!(recordings["ranges"][0]["from"].as_i64(), Some(from));
    // defaults are the last hour (24 h for clips)
    assert_eq!(world.get("/v1/cameras/273/recordings").status, 200);
    assert!(
        world
            .bridge
            .requests()
            .iter()
            .any(|r| r.starts_with("GET /v1/recordings?camera=273&from=") && r.contains("&to="))
    );
    let events = world
        .get(&format!(
            "/v1/cameras/273/events?from={from}&to={now}&count=30"
        ))
        .json();
    assert_eq!(events["events"][0]["kind"], "person");
    assert!(
        world
            .bridge
            .requests()
            .iter()
            .any(|r| r.contains("/v1/events?camera=273") && r.contains("count=30")),
        "{:?}",
        world.bridge.requests()
    );
    let clips = world.get("/v1/cameras/273/clips?count=10").json();
    assert_eq!(clips["clips"][0]["id"], "c1.WyIx");
    assert_eq!(clips["clips"][0]["has_video"], true);
    // pictures of a moment and of a clip
    let thumb = world.get(&format!("/v1/cameras/273/thumbnail?at={from}&w=256&h=144"));
    assert_eq!((thumb.status, thumb.body.as_slice()), (200, STILL));
    let clip = world.get("/v1/cameras/273/clip-thumbnail?clip=c1.WyIx&w=352&h=198");
    assert_eq!((clip.status, clip.body.as_slice()), (200, STILL));
    assert!(
        world
            .bridge
            .requests()
            .iter()
            .any(|r| r.contains("/v1/clip-thumbnail?camera=273") && r.contains("id=c1.WyIx"))
    );
    // a bad ask is refused before the bridge is asked
    let before = world.bridge.requests().len();
    assert_eq!(world.get("/v1/cameras/273/thumbnail").status, 400);
    assert_eq!(
        world.get("/v1/cameras/273/recordings?from=5&to=5").status,
        400
    );
    assert_eq!(world.get("/v1/cameras/273/recordings?from=a").status, 400);
    assert_eq!(world.get("/v1/cameras/999/recordings").status, 404);
    // (a request of the helper's own that was already on its way, a retry of the picture above, may land in between: only what the refused asks
    // would have sent is looked for)
    let leaked: Vec<String> = world
        .bridge
        .requests()
        .into_iter()
        .skip(before)
        .filter(|r| {
            r.contains("camera=999")
                || r.contains("from=a")
                || r.contains("from=5")
                || (r.contains("/v1/thumbnail") && !r.contains("at="))
        })
        .collect();
    assert!(leaked.is_empty(), "the bridge was asked: {leaked:?}");
    // what the bridge refuses comes with its word and words for the person
    world
        .bridge
        .fail_next("GET /v1/thumbnail", 404, "no_recording");
    let refused = world
        .get(&format!("/v1/cameras/273/thumbnail?at={from}"))
        .json();
    assert_eq!(
        (
            refused["error"]["code"].as_str(),
            refused["error"]["message"].as_str()
        ),
        (Some("no_recording"), Some("No recording at that time"))
    );
    world
        .bridge
        .fail_next("GET /v1/clips", 501, "not_supported");
    assert_eq!(world.get("/v1/cameras/273/clips").status, 501);
    world.bridge.fail_next("GET /v1/recordings", 429, "busy");
    assert_eq!(
        world.get("/v1/cameras/273/recordings").json()["error"]["code"],
        "busy"
    );
    // the calls share kept-alive connections: seventeen calls did not make seventeen connections
    assert!(
        world.bridge.connections() < 8,
        "{}",
        world.bridge.connections()
    );
}

#[test]
fn a_slow_bridge_is_answered_in_time_in_words() {
    let world = linked_world(json!({}));
    world.bridge.set_answer_delay_ms(9_500);
    let started = Instant::now();
    let reply = world.get("/v1/cameras/273/recordings").json();
    assert_eq!(reply["ok"], false);
    assert!(
        matches!(
            reply["error"]["code"].as_str(),
            Some("timeout" | "unreachable")
        ),
        "{reply}"
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    world.bridge.set_answer_delay_ms(0);
}

// ---- the live path ---------------------------------------------------------------------------------------------------------------

fn hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap())
        .collect()
}

/// A 640x360 baseline stream's parameter sets.
fn sps() -> Vec<u8> {
    hex("6742c016d900a02ff970110000030001000003001e0f162e48")
}

fn pps() -> Vec<u8> {
    hex("68cb83cb20")
}

fn gop(n: usize) -> Vec<Vec<u8>> {
    gop_of(&sps(), n)
}

fn gop_of(sps: &[u8], n: usize) -> Vec<Vec<u8>> {
    let mut first = annexb(&[sps, &pps()]);
    let mut slice = vec![0x65, 0x88, 0x84, 1];
    slice.extend((0..3000).map(|i| (i % 251) as u8 | 0x10));
    first.extend(annexb(&[&slice]));
    let mut units = vec![first];
    units.extend((1..n).map(|i| annexb(&[&[0x41, 0x9a, 0x24, i as u8, 1, 2, 3, 4]])));
    units
}

fn camera(fps: u32, units: Vec<Vec<u8>>, tweak: impl FnOnce(&mut CameraScript)) -> FakeCamera {
    camera_of(sps(), fps, units, tweak)
}

fn camera_of(sps: Vec<u8>, fps: u32, units: Vec<Vec<u8>>, tweak: impl FnOnce(&mut CameraScript)) -> FakeCamera {
    let mut script = CameraScript::new(Codec::H264, vec![sps, pps()], units, fps);
    tweak(&mut script);
    FakeCamera::start(script)
}

fn loopback_dial() -> lumatv_helper::live::Dial {
    Arc::new(|address, _patience| {
        let url = rtsp_client::Url::parse(address)?;
        rtsp_client::Session::connect(
            &url,
            &rtsp_client::Options {
                loopback: true,
                reply_timeout: Duration::from_secs(3),
                first_picture_timeout: Duration::from_secs(3),
                ..rtsp_client::Options::default()
            },
        )
    })
}

fn host_feeder() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../gfn/feeder/build/host");
    let (program, library) = (
        root.join("lumatv-gfn-feeder"),
        root.join("libNDL_directmedia.so.1"),
    );
    (program.is_file() && library.is_file()).then_some((program, library))
}

/// How many lines of what the fake pipeline libraries logged hold `what`: the starfish library's, whose ES player a camera's view loads, and NDL's, in one file.
fn pipeline_lines(log: &std::path::Path, what: &str) -> usize {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.contains(what))
        .count()
}

struct Video {
    world: World,
    /// What the fake libraries logged (`FAKE_SMP_LOG`, `FAKE_NDL_LOG`): a line a call.
    log: std::path::PathBuf,
    /// Every video unit the fake libraries took (`FAKE_SMP_VIDEO_DUMP`, `FAKE_NDL_VIDEO_DUMP`).
    dump: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

/// A linked helper whose live path is the real host feeder on the fake starfish library (the ES player a camera's view loads) and the fake NDL, with `idle` seconds
/// of feeder idle.
fn video(idle: u32) -> Option<Video> {
    let Some((program, library)) = host_feeder() else {
        eprintln!("skipped: the host feeder is not built (sh gfn/feeder/build.sh)");
        return None;
    };
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("pipeline.log");
    let dump = dir.path().join("video.bin");
    let env = vec![
        ("FAKE_NDL_LOG".to_owned(), log.display().to_string()),
        ("FAKE_SMP_LOG".to_owned(), log.display().to_string()),
        ("FAKE_NDL_VIDEO_DUMP".to_owned(), dump.display().to_string()),
        ("FAKE_SMP_VIDEO_DUMP".to_owned(), dump.display().to_string()),
        (
            "LD_LIBRARY_PATH".to_owned(),
            library.parent().unwrap().display().to_string(),
        ),
    ];
    let mut deps = lumatv_helper::scrypted::testing::deps();
    deps.dial = Some(loopback_dial());
    deps.feeder = Some(lumatv_helper::live::FeederSpec {
        program,
        window: "card".into(),
        library: library.display().to_string(),
        extra_args: vec!["--no-window".into()],
        env,
        run_dir: dir.path().to_path_buf(),
        idle: Duration::from_secs(u64::from(idle)),
    });
    let mut world = World::unlinked(json!({"live": true}), deps);
    world.link();
    world.ready();
    Some(Video {
        world,
        log,
        dump,
        _dir: dir,
    })
}

fn live_state(world: &World) -> Value {
    world.rig.topic("cameras.live").unwrap()
}

/// x264's 640x360 baseline set at 25 pictures a second (`-r 25`), and the same set with the rate ffmpeg's `h264_metadata=tick_rate=60/1` writes over
/// it: what the decoder is to be given for a camera that says 25.
const SPS_25: &str = "6742c01ed900a02ff97011000003000100000300320f162e48";
const SPS_25_AS_30: &str = "6742c01ed900a02ff970110000030001000003003c0f162e48";

/// Plays a stream whose parameter set is `sps` through the live path for a while and returns the bytes the fake ES player took of it, or `None` where
/// the host's fake library is older than `FAKE_SMP_VIDEO_DUMP` (`sh gfn/feeder/build.sh`).
fn decoder_got(v: &Video, sps: Vec<u8>, fps: u32) -> Option<Vec<u8>> {
    let cam = camera_of(sps.clone(), fps, gop_of(&sps, 30), |_| {});
    v.world.bridge.set_rtsp("273", &cam.url());
    let reply = v.world.post("/v1/cameras/live", json!({"camera": "273"}));
    assert_eq!(reply.status, 200, "{}", String::from_utf8_lossy(&reply.body));
    assert!(
        wait_until(WAIT, || live_state(&v.world)["fed"].as_u64().unwrap_or(0) >= 20),
        "{:?} / pipeline: {}",
        live_state(&v.world),
        std::fs::read_to_string(&v.log).unwrap_or_default()
    );
    assert_eq!(v.world.post("/v1/cameras/live/stop", json!({})).status, 200);
    let got = std::fs::read(&v.dump).unwrap_or_default();
    if got.is_empty() {
        eprintln!("skipped: the host's fake starfish library does not dump video (sh gfn/feeder/build.sh)");
        return None;
    }
    Some(got)
}

fn has(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn a_camera_that_says_25_fps_is_given_to_the_decoder_as_30_and_nothing_else_of_the_picture_changes() {
    let Some(v) = video(1) else { return };
    let Some(got) = decoder_got(&v, hex(SPS_25), 25) else { return };
    assert!(has(&got, &hex(SPS_25_AS_30)), "the decoder is given the set that says 30");
    assert!(!has(&got, &hex(SPS_25)), "and never the camera's own");
    assert!(has(&got, &pps()) && has(&got, &[0x65, 0x88, 0x84, 1]), "the rest of the key picture is the camera's");
}

#[test]
fn a_camera_that_says_15_fps_is_given_to_the_decoder_as_it_is() {
    let Some(v) = video(1) else { return };
    let Some(got) = decoder_got(&v, sps(), 15) else { return };
    assert!(has(&got, &sps()), "the camera's own set, byte for byte");
}

#[test]
fn live_starts_at_once_plays_on_the_video_plane_and_stops_clean() {
    let Some(v) = video(1) else { return };
    let cam = camera(30, gop(30), |_| {});
    v.world.bridge.set_rtsp("273", &cam.url());
    let started = Instant::now();
    let reply = v.world.post("/v1/cameras/live", json!({"camera": "273"}));
    assert_eq!(
        reply.status,
        200,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    let answer = reply.json();
    assert_eq!(answer["live"]["camera"], "273");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "the call returns when the bridge has answered, not when the stream plays"
    );
    let playing = v
        .world
        .rig
        .wait_topic("cameras.live", WAIT, |t| t["state"] == "playing")
        .unwrap_or_else(|| {
            panic!(
                "never playing: {:?} / pipeline: {}",
                live_state(&v.world),
                std::fs::read_to_string(&v.log).unwrap_or_default()
            )
        });
    println!("first picture {:?} after the call", started.elapsed());
    assert_eq!(
        (
            playing["mode"].as_str(),
            playing["camera"].as_str(),
            playing["codec"].as_str()
        ),
        (Some("live"), Some("273"), Some("h264"))
    );
    assert_eq!(
        (playing["width"].clone(), playing["height"].clone()),
        (json!(640), json!(360))
    );
    assert!(playing["window"].is_string(), "{playing}");
    assert!(
        wait_until(WAIT, || live_state(&v.world)["fed"].as_u64().unwrap_or(0)
            >= 20),
        "{:?}",
        live_state(&v.world)
    );
    // the dial asked the bridge for the highest stream the set takes, in the bridge's words
    let body = v
        .world
        .bridge
        .requests()
        .into_iter()
        .find(|r| r == "POST /v1/live");
    assert!(body.is_some());
    assert_eq!(
        pipeline_lines(&v.log, "VIOLATION"),
        0,
        "{}",
        std::fs::read_to_string(&v.log).unwrap_or_default()
    );
    // the full view loads the ES player and tells it a video, not a game, so the TV keeps the picture mode its owner set (docs/research/vwin-refresh-result.md)
    assert!(pipeline_lines(&v.log, "\"mediaTransportType\":\"DIRECTMEDIA-ES-PLAYER\"") >= 1, "{}", std::fs::read_to_string(&v.log).unwrap_or_default());
    assert_eq!(pipeline_lines(&v.log, "isGameMode"), 0);
    // Back: the state says stopped at once and the pipeline unloads off this path
    let stopped_at = Instant::now();
    let stop = v.world.post("/v1/cameras/live/stop", json!({}));
    assert_eq!(stop.status, 200);
    assert!(
        stopped_at.elapsed() < Duration::from_millis(500),
        "{:?}",
        stopped_at.elapsed()
    );
    assert_eq!(live_state(&v.world)["state"], "stopped");
    assert!(
        wait_until(WAIT, || pipeline_lines(&v.log, "Unload") >= 1),
        "{}",
        std::fs::read_to_string(&v.log).unwrap_or_default()
    );
    assert!(
        wait_until(WAIT, || cam
            .requests()
            .iter()
            .any(|r| r.starts_with("TEARDOWN"))),
        "the camera is told the stream is over"
    );
    // the feeder is kept 1 s for the next open, then ended; no socket is left
    assert!(
        wait_until(Duration::from_secs(6), || std::fs::read_dir(v._dir.path())
            .unwrap()
            .all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".sock"))),
        "the feeder's socket is gone"
    );
    assert_eq!(pipeline_lines(&v.log, "VIOLATION"), 0);
}

#[test]
fn switching_cameras_replaces_the_session_and_a_refused_start_leaves_the_one_that_plays() {
    let Some(v) = video(30) else { return };
    let a = camera(30, gop(30), |_| {});
    let b = camera(30, gop(30), |_| {});
    v.world.bridge.set_rtsp("273", &a.url());
    v.world.bridge.set_rtsp("274", &b.url());
    assert_eq!(
        v.world
            .post("/v1/cameras/live", json!({"camera": "273"}))
            .status,
        200
    );
    v.world
        .rig
        .wait_topic("cameras.live", WAIT, |t| {
            t["state"] == "playing" && t["camera"] == "273"
        })
        .expect("273 plays");
    // the bridge refuses the next one: a refusal in words, and the picture on the plane is still the first
    v.world.bridge.fail_next("POST /v1/live", 409, "offline");
    let refused = v.world.post("/v1/cameras/live", json!({"camera": "274"}));
    assert_eq!(refused.status, 409);
    let body = refused.json();
    assert_eq!(
        (
            body["error"]["code"].as_str(),
            body["error"]["message"].as_str()
        ),
        (Some("offline"), Some("The camera is offline"))
    );
    let state = live_state(&v.world);
    assert_eq!(
        (state["state"].as_str(), state["camera"].as_str()),
        (Some("playing"), Some("273")),
        "{state}"
    );
    // Right: the second camera, a new session on the same feeder
    let before = pipeline_lines(&v.log, "Load");
    assert_eq!(
        v.world
            .post("/v1/cameras/live", json!({"camera": "274"}))
            .status,
        200
    );
    v.world
        .rig
        .wait_topic("cameras.live", WAIT, |t| {
            t["state"] == "playing" && t["camera"] == "274"
        })
        .expect("274 plays");
    assert!(
        wait_until(WAIT, || a
            .requests()
            .iter()
            .any(|r| r.starts_with("TEARDOWN"))),
        "the old stream is torn down"
    );
    assert!(
        pipeline_lines(&v.log, "Load") > before,
        "the pipeline was loaded again for the other camera"
    );
    assert_eq!(
        pipeline_lines(&v.log, "VIOLATION"),
        0,
        "{}",
        std::fs::read_to_string(&v.log).unwrap_or_default()
    );
    // a camera the TV does not show, and a bad body
    assert_eq!(
        v.world
            .post("/v1/cameras/live", json!({"camera": "999"}))
            .status,
        404
    );
    assert_eq!(
        v.world
            .post(
                "/v1/cameras/live",
                json!({"camera": "273", "quality": "ultra"})
            )
            .status,
        400
    );
    assert_eq!(v.world.post("/v1/cameras/live", json!({})).status, 400);
    v.world.post("/v1/cameras/live/stop", json!({}));
}

#[test]
fn a_recording_plays_from_the_asked_moment_pauses_where_it_is_and_plays_on_from_there() {
    let Some(v) = video(30) else { return };
    let cam = camera(30, gop(600), |_| {});
    v.world.bridge.set_rtsp("273", &cam.url());
    let at = v.world.bridge.now() - 600_000;
    let reply = v.world.post(
        "/v1/cameras/playback",
        json!({"camera": "273", "at": at, "rate": 1}),
    );
    assert_eq!(
        reply.status,
        200,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    let state = v
        .world
        .rig
        .wait_topic("cameras.live", WAIT, |t| t["state"] == "playing")
        .expect("playing");
    assert_eq!(
        (
            state["mode"].as_str(),
            state["rate"].as_u64(),
            state["startedAt"].as_i64()
        ),
        (Some("playback"), Some(1), Some(at))
    );
    // the playhead runs from the moment asked for, as the stream's own clock says
    let p1 = wait_position(&v, |p| p > at + 300).expect("the playhead moves");
    // Pause: the picture is held, the dial ends (the bridge is told), the position stops
    assert_eq!(
        v.world
            .post("/v1/cameras/live/pause", json!({"paused": true}))
            .status,
        200
    );
    let held = live_state(&v.world);
    assert_eq!(
        (held["paused"].clone(), held["state"].as_str()),
        (json!(true), Some("playing"))
    );
    assert!(
        wait_until(WAIT, || cam
            .requests()
            .iter()
            .any(|r| r.starts_with("TEARDOWN"))),
        "the dial ends"
    );
    assert!(
        wait_until(WAIT, || v
            .world
            .bridge
            .requests_matching("POST /v1/playback/stop")
            >= 1),
        "the bridge's session is given back"
    );
    let frozen = live_state(&v.world)["positionMs"].as_i64().unwrap();
    assert!(frozen >= p1);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        live_state(&v.world)["positionMs"].as_i64().unwrap(),
        frozen,
        "the position does not run while paused"
    );
    // Play: dialled again where it stopped
    let loads = pipeline_lines(&v.log, "Load");
    assert_eq!(
        v.world
            .post("/v1/cameras/live/pause", json!({"paused": false}))
            .status,
        200
    );
    let resumed = v
        .world
        .rig
        .wait_topic("cameras.live", WAIT, |t| {
            t["paused"] == false
                && t["state"] == "playing"
                && t["positionMs"].as_i64().is_some_and(|p| p > frozen)
        })
        .expect("plays on");
    assert!(resumed["positionMs"].as_i64().unwrap() >= frozen);
    assert_eq!(
        pipeline_lines(&v.log, "Load"),
        loads,
        "the same pipeline went on: the picture was never dropped"
    );
    // live cannot be paused, and nothing paused cannot be resumed
    v.world.post("/v1/cameras/live", json!({"camera": "273"}));
    v.world
        .rig
        .wait_topic("cameras.live", WAIT, |t| {
            t["mode"] == "live" && t["state"] == "playing"
        })
        .expect("back to live on the same pipeline");
    assert_eq!(
        v.world
            .post("/v1/cameras/live/pause", json!({"paused": true}))
            .json()["error"]["code"],
        "not_playback"
    );
    assert_eq!(
        v.world
            .post("/v1/cameras/live/pause", json!({"paused": false}))
            .json()["error"]["code"],
        "not_paused"
    );
    assert_eq!(
        v.world.post("/v1/cameras/live/pause", json!({})).status,
        400
    );
    assert_eq!(
        pipeline_lines(&v.log, "VIOLATION"),
        0,
        "{}",
        std::fs::read_to_string(&v.log).unwrap_or_default()
    );
}

#[test]
fn the_full_views_recording_plays_where_its_live_does_and_play_goes_on_there_on_the_same_pipeline() {
    let Some(v) = video(30) else { return };
    let cam = camera(30, gop(600), |_| {});
    v.world.bridge.set_rtsp("273", &cam.url());
    // live, as the full view asks for it on a video window of the display
    let reply = v.world.post("/v1/cameras/live", json!({"camera": "273", "for": "full", "window": "main"}));
    assert_eq!(reply.status, 200, "{}", String::from_utf8_lossy(&reply.body));
    v.world.rig.wait_topic("cameras.live", WAIT, |t| t["mode"] == "live" && t["state"] == "playing").expect("live plays");
    let loads = pipeline_lines(&v.log, "Load");
    assert!(loads >= 1);
    // a recording from the strip names the same window: the same feeder, the same pipeline, the picture on the plane never dropped
    let at = v.world.bridge.now() - 600_000;
    let reply = v.world.post("/v1/cameras/playback", json!({"camera": "273", "at": at, "window": "main"}));
    assert_eq!(reply.status, 200, "{}", String::from_utf8_lossy(&reply.body));
    v.world.rig.wait_topic("cameras.live", WAIT, |t| t["mode"] == "playback" && t["state"] == "playing").expect("the recording plays");
    assert_eq!(pipeline_lines(&v.log, "Load"), loads, "the seek reused the pipeline");
    // Pause, then Play: dialled again where it stopped, in the window it was in
    assert_eq!(v.world.post("/v1/cameras/live/pause", json!({"paused": true})).status, 200);
    assert!(wait_until(WAIT, || v.world.bridge.requests_matching("POST /v1/playback/stop") >= 1), "the bridge's session is given back");
    assert_eq!(v.world.post("/v1/cameras/live/pause", json!({"paused": false})).status, 200);
    v.world.rig.wait_topic("cameras.live", WAIT, |t| t["paused"] == false && t["mode"] == "playback" && t["state"] == "playing").expect("plays on");
    assert_eq!(pipeline_lines(&v.log, "Load"), loads, "Play went on in the same window and pipeline");
    // the decoder was opened for Home, the app LG's video path connects a window's video for
    let feeder_log = std::fs::read_to_string(v._dir.path().join("feeder.log")).unwrap_or_default();
    assert!(feeder_log.contains("ndl app com.webos.app.home"), "{feeder_log}");
    // a window the shell was not granted is refused, not guessed
    let reply = v.world.post("/v1/cameras/playback", json!({"camera": "273", "at": at, "window": "top"}));
    assert_eq!(reply.status, 400, "{}", String::from_utf8_lossy(&reply.body));
    // without the word (the compositor has no video windows) a recording plays as it did before: in the configured window, with another feeder and its own load
    let reply = v.world.post("/v1/cameras/playback", json!({"camera": "273", "at": at}));
    assert_eq!(reply.status, 200, "{}", String::from_utf8_lossy(&reply.body));
    assert!(wait_until(WAIT, || pipeline_lines(&v.log, "Load") > loads), "another window kind: another feeder loads its own pipeline");
    v.world.post("/v1/cameras/live/stop", json!({}));
    assert_eq!(pipeline_lines(&v.log, "VIOLATION"), 0, "{}", std::fs::read_to_string(&v.log).unwrap_or_default());
}

fn wait_position(v: &Video, check: impl Fn(i64) -> bool) -> Option<i64> {
    let mut seen = None;
    wait_until(WAIT, || {
        seen = live_state(&v.world)["positionMs"].as_i64();
        seen.is_some_and(&check)
    })
    .then_some(seen)
    .flatten()
}

#[test]
fn the_bridge_ending_the_old_recording_for_a_seek_is_not_an_end_of_recording_and_a_refused_seek_says_why()
 {
    let Some(v) = video(30) else { return };
    let first = camera(30, gop(600), |_| {});
    v.world.bridge.set_rtsp("273#playback", &first.url());
    let at = v.world.bridge.now() - 600_000;
    assert_eq!(
        v.world
            .post(
                "/v1/cameras/playback",
                json!({"camera": "273", "at": at, "rate": 1})
            )
            .status,
        200
    );
    v.world
        .rig
        .wait_topic("cameras.live", WAIT, |t| t["state"] == "playing")
        .expect("playing");
    let hub = v.world.rig.service().hub();
    let said_since = |seq: u64| -> Vec<String> {
        hub.events_after(
            Some(seq),
            Some(hub.boot()),
            Some(&["cameras.live".to_owned()]),
        )
        .events
        .iter()
        .map(|e| e.data["state"].as_str().unwrap_or("").to_owned())
        .collect()
    };
    let playbacks = || {
        v.world
            .bridge
            .requests()
            .iter()
            .filter(|r| r.as_str() == "POST /v1/playback")
            .count()
    };
    let seek = |to: i64| {
        let (addr, body) = (
            v.world.rig.addr,
            json!({"camera": "273", "at": to, "rate": 1}).to_string(),
        );
        std::thread::spawn(move || post(addr, "/v1/cameras/playback", &body))
    };
    // A seek: the bridge ends the camera's old playback as it takes the call, and answers a moment later. Nothing in between says the
    // recording is over.
    let second = camera(30, gop(600), |_| {});
    v.world.bridge.set_rtsp("273#playback", &second.url());
    v.world.bridge.set_playback_delay_ms(700);
    let before = hub.events_after(None, None, None).seq;
    let seeking = seek(at + 120_000);
    assert!(wait_until(WAIT, || playbacks() == 2));
    drop(first);
    assert_eq!(seeking.join().unwrap().status, 200);
    let state = v
        .world
        .rig
        .wait_topic("cameras.live", WAIT, |t| {
            t["state"] == "playing" && t["startedAt"].as_i64() == Some(at + 120_000)
        })
        .expect("playing from the new moment");
    assert_eq!(state["mode"], "playback");
    let said = said_since(before);
    assert!(
        said.iter().all(|s| s == "playing" || s == "starting"),
        "{said:?}"
    );
    // A seek the bridge refuses after it ended the recording: the picture stays and the words say why it does not move.
    v.world
        .bridge
        .fail_next("POST /v1/playback", 404, "no_recording");
    let before = hub.events_after(None, None, None).seq;
    let seeking = seek(at + 300_000);
    assert!(wait_until(WAIT, || playbacks() == 3));
    drop(second);
    let reply = seeking.join().unwrap();
    assert_eq!(
        (
            reply.status,
            reply.json()["error"]["code"].as_str().map(str::to_owned)
        ),
        (409, Some("no_recording".to_owned())),
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    let failed = v
        .world
        .rig
        .wait_topic("cameras.live", WAIT, |t| t["state"] == "failed")
        .expect("failed, in words");
    assert_eq!(failed["reason"], "No recording at that time");
    let said = said_since(before);
    assert!(!said.iter().any(|s| s == "stopped"), "{said:?}");
    // and a seek that works plays again
    v.world.bridge.set_playback_delay_ms(0);
    let third = camera(30, gop(600), |_| {});
    v.world.bridge.set_rtsp("273#playback", &third.url());
    assert_eq!(seek(at + 60_000).join().unwrap().status, 200);
    v.world
        .rig
        .wait_topic("cameras.live", WAIT, |t| {
            t["state"] == "playing" && t["startedAt"].as_i64() == Some(at + 60_000)
        })
        .expect("playing again");
}

#[test]
fn a_recording_that_ends_says_so_a_refused_playback_leaves_live_alone_and_a_clip_plays() {
    let Some(v) = video(30) else { return };
    let live = camera(30, gop(300), |_| {});
    let recording = camera(60, gop(40), |script| script.close_after = Some(40));
    v.world.bridge.set_rtsp("273", &live.url());
    v.world.bridge.set_rtsp("273#playback", &recording.url());
    assert_eq!(
        v.world
            .post("/v1/cameras/live", json!({"camera": "273"}))
            .status,
        200
    );
    v.world
        .rig
        .wait_topic("cameras.live", WAIT, |t| t["state"] == "playing")
        .unwrap();
    // a moment with no recording: refused with its words, and live goes on
    v.world
        .bridge
        .fail_next("POST /v1/playback", 404, "no_recording");
    let refused = v.world.post(
        "/v1/cameras/playback",
        json!({"camera": "273", "at": v.world.bridge.now() - 1000}),
    );
    assert_eq!(refused.status, 409);
    assert_eq!(
        refused.json()["error"]["message"],
        "No recording at that time"
    );
    let state = live_state(&v.world);
    assert_eq!(
        (state["state"].as_str(), state["mode"].as_str()),
        (Some("playing"), Some("live")),
        "{state}"
    );
    // a recording that ends by itself: stopped, in words, and the session is there for live to be asked again
    assert_eq!(
        v.world
            .post(
                "/v1/cameras/playback",
                json!({"camera": "273", "at": v.world.bridge.now() - 60_000})
            )
            .status,
        200
    );
    let over = v
        .world
        .rig
        .wait_topic("cameras.live", WAIT, |t| {
            t["state"] == "stopped" && t["mode"] == "playback"
        })
        .expect("the recording ends");
    assert_eq!(over["reason"], "End of recording");
    assert!(wait_until(WAIT, || v
        .world
        .bridge
        .requests_matching("POST /v1/playback/stop")
        >= 1));
    assert_eq!(
        v.world
            .post("/v1/cameras/live", json!({"camera": "273"}))
            .status,
        200
    );
    v.world
        .rig
        .wait_topic("cameras.live", WAIT, |t| {
            t["state"] == "playing" && t["mode"] == "live"
        })
        .unwrap_or_else(|| {
            panic!(
                "live again: {:?} / pipeline: {}",
                live_state(&v.world),
                std::fs::read_to_string(&v.log).unwrap_or_default()
            )
        });
    // a clip: the bridge's clip call, played as a clip
    let clip = camera(30, gop(60), |_| {});
    v.world.bridge.set_rtsp("273#playback", &clip.url());
    assert_eq!(
        v.world
            .post(
                "/v1/cameras/playback",
                json!({"camera": "273", "clip": "c1.WyIx"})
            )
            .status,
        200
    );
    let state = v
        .world
        .rig
        .wait_topic("cameras.live", WAIT, |t| {
            t["mode"] == "clip" && t["state"] == "playing"
        })
        .expect("a clip plays");
    assert_eq!(state["camera"], "273");
    assert!(
        v.world
            .bridge
            .requests()
            .iter()
            .any(|r| r == "POST /v1/clip-playback")
    );
    // bad asks
    assert_eq!(
        v.world
            .post("/v1/cameras/playback", json!({"camera": "273"}))
            .status,
        400
    );
    assert_eq!(
        v.world
            .post(
                "/v1/cameras/playback",
                json!({"camera": "273", "at": 1, "rate": 3})
            )
            .status,
        400
    );
    assert_eq!(
        v.world
            .post("/v1/cameras/playback", json!({"camera": "273", "clip": ""}))
            .status,
        400
    );
}

#[test]
fn a_stream_that_goes_quiet_ends_with_a_reason_and_a_dead_address_fails_in_words() {
    let Some(v) = video(30) else { return };
    // the camera sends five pictures and stays connected, silent: the stream is over after its quiet time
    let quiet = camera(30, gop(30), |script| script.silent_after = Some(5));
    v.world.bridge.set_rtsp("273", &quiet.url());
    assert_eq!(
        v.world
            .post("/v1/cameras/live", json!({"camera": "273"}))
            .status,
        200
    );
    v.world
        .rig
        .wait_topic("cameras.live", WAIT, |t| t["state"] == "playing")
        .expect("it plays first");
    let failed = v
        .world
        .rig
        .wait_topic("cameras.live", Duration::from_secs(20), |t| {
            t["state"] == "failed"
        })
        .expect("quiet for 8 s is over");
    assert!(
        failed["reason"].as_str().unwrap().contains("quiet"),
        "{failed}"
    );
    // an address nobody answers at: failed with words that carry no address
    v.world.bridge.set_rtsp("274", "rtsp://127.0.0.1:1/nothing");
    assert_eq!(
        v.world
            .post("/v1/cameras/live", json!({"camera": "274"}))
            .status,
        200
    );
    let failed = v
        .world
        .rig
        .wait_topic("cameras.live", WAIT, |t| {
            t["state"] == "failed" && t["camera"] == "274"
        })
        .expect("failed");
    let reason = failed["reason"].as_str().unwrap();
    assert!(
        !reason.contains("127.0.0.1") && !reason.contains("rtsp://") && !reason.contains("nothing"),
        "{reason}"
    );
    v.world.post("/v1/cameras/live/stop", json!({}));
}

/// The downstairs door (a Ring doorbell, 2026-11-04): Scrypted has one stream for it and no Rebroadcast, so the bridge answers with a one-shot
/// address on its own machine that nothing else on the LAN can reach: a codec and nothing else known of the video, no prebuffer, and an audio
/// track with no codec named. The TV's dial is refused at once.
#[test]
fn a_doorbell_with_a_one_shot_address_fails_in_words_and_a_refusal_after_it_says_why() {
    let Some(v) = video(30) else { return };
    v.world.bridge.set_doc(
        "live",
        200,
        json!({"rtsp": "rtsp://127.0.0.1:1", "camera": "274", "quality": "native", "video": {"codec": "h264"}, "audio": {},
               "prebuffered": false, "expires_in_ms": 25000}),
    );
    assert_eq!(
        v.world
            .post("/v1/cameras/live", json!({"camera": "274"}))
            .status,
        200
    );
    let failed = v
        .world
        .rig
        .wait_topic("cameras.live", WAIT, |t| {
            t["state"] == "failed" && t["camera"] == "274"
        })
        .expect("the dial is refused");
    let reason = failed["reason"].as_str().unwrap();
    assert_eq!(reason, "Scrypted's stream for this camera could not be reached");
    assert!(!reason.contains("127.0.0.1") && !reason.contains("rtsp://"), "{reason}");
    assert_eq!(
        (failed["width"].as_u64(), failed["height"].as_u64()),
        (Some(0), Some(0)),
        "no size is made up for the shell to fit a window by"
    );
    assert_eq!(
        pipeline_lines(&v.log, "Load"),
        0,
        "no pipeline is loaded for a stream nobody described"
    );
    // the next ask is refused by the bridge (Scrypted says the camera is offline): the words the shell shows, and the view says the same
    v.world.bridge.set_doc("live", 409, json!({"code": "offline"}));
    let refused = v.world.post("/v1/cameras/live", json!({"camera": "274"}));
    assert_eq!(refused.status, 409);
    let body = refused.json();
    assert_eq!(
        (
            body["error"]["code"].as_str(),
            body["error"]["message"].as_str()
        ),
        (Some("offline"), Some("The camera is offline"))
    );
    let state = live_state(&v.world);
    assert_eq!(
        (
            state["state"].as_str(),
            state["camera"].as_str(),
            state["reason"].as_str()
        ),
        (Some("failed"), Some("274"), Some("The camera is offline")),
        "{state}"
    );
}

#[test]
fn live_is_off_in_words_when_the_switch_is_off_and_without_a_feeder_nothing_starts() {
    let mut world = World::unlinked(
        json!({"live": false}),
        lumatv_helper::scrypted::testing::deps(),
    );
    world.link();
    world.ready();
    let cam = camera(30, gop(30), |_| {});
    world.bridge.set_rtsp("273", &cam.url());
    let reply = world.post("/v1/cameras/live", json!({"camera": "273"}));
    assert_eq!(reply.status, 409);
    assert_eq!(reply.json()["error"]["code"], "live_off");
    assert_eq!(
        world.bridge.requests_matching("POST /v1/live"),
        0,
        "the bridge is not asked"
    );
    assert_eq!(world.rig.topic("cameras.live").unwrap()["state"], "idle");
    assert_eq!(cam.connections(), 0);
}

#[test]
fn leaving_the_link_while_playing_ends_the_stream_and_the_feeder_goes() {
    let Some(v) = video(30) else { return };
    let cam = camera(30, gop(60), |_| {});
    v.world.bridge.set_rtsp("273", &cam.url());
    assert_eq!(
        v.world
            .post("/v1/cameras/live", json!({"camera": "273"}))
            .status,
        200
    );
    v.world
        .rig
        .wait_topic("cameras.live", WAIT, |t| t["state"] == "playing")
        .unwrap();
    v.world.bridge.remove_device(&v.world.verifier);
    let state = v
        .world
        .rig
        .wait_topic("cameras.live", WAIT, |t| t["state"] == "stopped")
        .expect("the stream ends with the link");
    assert!(
        state["reason"].as_str().is_some_and(|r| !r.is_empty()),
        "{state}"
    );
    assert!(wait_until(WAIT, || cam
        .requests()
        .iter()
        .any(|r| r.starts_with("TEARDOWN"))));
}
