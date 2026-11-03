//! On Home against a fake LumaShow integration (`ha_bridge::fake`): the integration dials the TV with the pairing key, shares entities, and
//! the TV draws tiles from them and asks for controls through the integration. What the tests hold the TV to: it holds no Home Assistant
//! address or token; it keeps only what is shared and drops what Home has no tile for (cameras); it asks only for what the integration said
//! it may; and it never asks for a lock, an alarm, a camera, a garage door or a gate, whatever is shared and however it is asked.

use ha_bridge::fake::FakeIntegration;
use ha_bridge::wire::{AreaRec, CallResult, EntityRec, Message, Policy, Snapshot, StateRec, Update};
use lumatv_helper::config::{Config, OnHomeConfig, Paths};
use lumatv_helper::control::Stop;
use lumatv_helper::ctx::Ctx;
use lumatv_helper::hub::Hub;
use lumatv_helper::onhome::{OnHome, testing};
use lumatv_helper::settings::SettingsStore;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};

const KEY: [u8; 32] = [7; 32];
const WITHIN: Duration = Duration::from_secs(5);

struct Rig {
    _dir: tempfile::TempDir,
    ctx: Ctx,
    onhome: OnHome,
}

fn rig(enabled: bool, key: Option<[u8; 32]>) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let hub = Arc::new(Hub::new(1));
    let settings = Arc::new(SettingsStore::new(dir.path().join("settings"), hub.clone()));
    let config = Config { onhome: OnHomeConfig { enabled, port: 0 }, ..Config::default() };
    let ctx = Ctx { config: Arc::new(config), paths: Paths::new(dir.path()), hub, settings, stop: Stop::new() };
    let onhome = OnHome::new(&ctx, testing::keyed_deps(key));
    Rig { _dir: dir, ctx, onhome }
}

fn ent(id: &str, name: &str, area: Option<&str>, state: &str, attrs: Value, ctl: &[&str]) -> EntityRec {
    EntityRec {
        id: id.into(),
        name: name.into(),
        area: area.map(Into::into),
        state: state.into(),
        attrs: attrs.as_object().cloned().unwrap_or_default(),
        changed: Some(1.0),
        minor: false,
        ctl: ctl.iter().map(|s| (*s).into()).collect(),
        ctl_pin: if id.starts_with("lock.") { vec!["unlock".into()] } else { vec![] },
    }
}

fn snapshot() -> Snapshot {
    Snapshot {
        rev: 1,
        part: 0,
        more: false,
        policy: Policy::default(),
        areas: vec![AreaRec { id: "kitchen".into(), name: "Kitchen".into() }, AreaRec { id: "hall".into(), name: "Hall".into() }],
        entities: vec![
            ent("light.kitchen_main", "Kitchen main light", Some("kitchen"), "on", json!({"brightness": 128, "supported_color_modes": ["color_temp"], "friendly_name": "Kitchen main light"}), &["turn_on", "turn_off", "toggle"]),
            ent("climate.hall", "Hall thermostat", Some("hall"), "heat", json!({"current_temperature": 68.0, "temperature": 70.0, "hvac_action": "heating", "hvac_modes": ["off", "heat"]}), &["set_temperature", "set_hvac_mode"]),
            ent("lock.front_door", "Front door", Some("hall"), "locked", json!({}), &["lock"]),
            ent("cover.garage", "Garage door", None, "closed", json!({"device_class": "garage"}), &["close_cover"]),
            ent("sensor.hall_temperature", "Hall temperature", Some("hall"), "21.4", json!({"device_class": "temperature", "unit_of_measurement": "\u{b0}C"}), &[]),
            ent("camera.porch", "Porch camera", None, "idle", json!({}), &["turn_off"]),
            ent("scene.movie", "Movie", None, "scening", json!({}), &["turn_on"]),
        ],
    }
}

fn wait_for(rig: &Rig, what: &str, check: impl Fn(&Value) -> bool) -> Value {
    let until = Instant::now() + WITHIN;
    loop {
        let doc = rig.ctx.hub.get("onhome").unwrap_or(Value::Null);
        if check(&doc) {
            return doc;
        }
        assert!(Instant::now() < until, "{what}: the topic is {doc}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn tile<'a>(doc: &'a Value, id: &str) -> &'a Value {
    doc["tiles"].as_array().unwrap().iter().find(|t| t["id"] == id).unwrap_or_else(|| panic!("no tile {id} in {doc}"))
}

/// The listener's address, once it listens; a connected fake integration.
fn connect(rig: &Rig) -> FakeIntegration {
    rig.onhome.look();
    let addr = rig.onhome.local_addr().expect("the listener listens");
    FakeIntegration::open(addr, &KEY).expect("the integration is let in with the pairing key")
}

#[test]
fn nothing_listens_until_it_is_switched_on_and_paired() {
    let off = rig(false, Some(KEY));
    off.onhome.look();
    assert!(off.onhome.local_addr().is_none());
    assert_eq!(off.ctx.hub.get("onhome").unwrap()["state"], "off");
    let unpaired = rig(true, None);
    unpaired.onhome.look();
    assert!(unpaired.onhome.local_addr().is_none());
    let doc = unpaired.ctx.hub.get("onhome").unwrap();
    assert_eq!(doc["state"], "unpaired");
    assert!(doc["detail"].as_str().unwrap().contains("ble-link"));
}

fn ids(doc: &Value) -> Vec<&str> {
    doc["tiles"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect()
}

/// The call the integration is asked for by `worker`, or none: a worker that is refused ends without one, so no wait is spent on a call that
/// is not coming; a worker that is waiting for the integration's answer is waited for up to 800 ms.
fn call_until_done<T>(it: &mut FakeIntegration, worker: &std::thread::JoinHandle<T>) -> Option<ha_bridge::wire::Call> {
    let ask = |it: &mut FakeIntegration, within| it.recv_where(within, |m| if let Message::Call(c) = m { Some(c.clone()) } else { None });
    let until = Instant::now() + Duration::from_millis(800);
    while Instant::now() < until {
        if let Some(call) = ask(it, Duration::from_millis(20)) {
            return Some(call);
        }
        if worker.is_finished() {
            return ask(it, Duration::from_millis(20));
        }
    }
    None
}

/// Runs `act` on its own thread (it waits for the integration's answer) and gives the integration's side what it needs to answer.
fn act_answered(r: &Rig, it: &mut FakeIntegration, id: &str, verb: &str, value: Value, answer: bool) -> (Result<(), (&'static str, String)>, Option<ha_bridge::wire::Call>) {
    let (onhome, id, verb) = (r.onhome.clone(), id.to_owned(), verb.to_owned());
    let worker = std::thread::spawn(move || onhome.act(&id, &verb, &value).map_err(|f| (f.code, f.words)));
    let call = call_until_done(it, &worker);
    if let Some(call) = &call {
        let message = if answer { None } else { Some("the bulb is stuck".to_owned()) };
        it.send(&Message::CallResult(CallResult { op: call.op, ok: answer, code: None, message, results: vec![] })).unwrap();
    }
    (worker.join().unwrap(), call)
}

#[test]
fn what_is_shared_becomes_tiles_by_kind_and_a_camera_is_not_one() {
    let r = rig(true, Some(KEY));
    let mut it = connect(&r);
    wait_for(&r, "the link is up before anything is shared", |d| d["state"] == "empty");
    it.send(&Message::Snapshot(snapshot())).unwrap();
    let doc = wait_for(&r, "tiles", |d| d["shared"] == 6);
    assert_eq!((doc["state"].as_str(), doc["words"].as_str(), doc["controllable"].clone()), (Some("connected"), Some("6 things"), json!(5)));
    assert_eq!(ids(&doc), ["light.kitchen_main", "scene.movie", "climate.hall", "cover.garage", "sensor.hall_temperature", "lock.front_door"], "lights, scenes, climate, covers, sensors, locks; no camera");
    let light = tile(&doc, "light.kitchen_main");
    assert_eq!((light["words"].as_str(), light["area"].as_str(), light["can"].clone()), (Some("On \u{b7} 50%"), Some("Kitchen"), json!(["toggle"])));
    assert_eq!(tile(&doc, "climate.hall")["words"], json!("68\u{b0} \u{b7} Heating to 70\u{b0}"));
    assert_eq!(tile(&doc, "climate.hall")["ctl"]["unit"], json!("\u{b0}C"), "the house's temperature sensors are in Celsius");
    assert_eq!(tile(&doc, "sensor.hall_temperature")["words"], json!("21.4\u{b0}C"));
    // shown, never operated
    assert_eq!((tile(&doc, "lock.front_door")["words"].as_str(), tile(&doc, "lock.front_door")["can"].clone()), (Some("Locked"), json!(["unlock"])), "a locked door asks before it unlocks");
    assert_eq!((tile(&doc, "cover.garage")["can"].clone(), tile(&doc, "cover.garage")["verbs"].clone()), (json!([]), json!([])));
    // a change is a change of the tile
    it.send(&Message::Update(Update { rev: 2, entities: vec![StateRec { id: "light.kitchen_main".into(), state: "off".into(), attrs: Default::default(), changed: Some(2.0) }], removed: vec![] })).unwrap();
    wait_for(&r, "the light is off", |d| d["tiles"][0]["words"] == "Off");
    // the picker lists what is shared, by name, with the areas and kinds it can be narrowed by
    let shared = lumatv_helper::onhome::testing::shared_of(&r.onhome, "q=hall");
    assert_eq!(shared["total"], 3);
    assert_eq!(shared["areas"], json!([{"id": "hall", "name": "Hall", "count": 3}]));
    assert!(shared["rows"].as_array().unwrap().iter().all(|row| row["shown"] == true), "until the owner chooses, all of it is shown");
}

#[test]
fn a_control_is_asked_of_the_integration_and_answered_in_words() {
    let r = rig(true, Some(KEY));
    let mut it = connect(&r);
    it.send(&Message::Snapshot(snapshot())).unwrap();
    wait_for(&r, "shared", |d| d["shared"] == 6);
    let (done, call) = act_answered(&r, &mut it, "light.kitchen_main", "brightness", json!(30), true);
    assert_eq!(done, Ok(()));
    let call = call.expect("the integration was asked");
    assert_eq!((call.entity_ids, call.verb.as_str(), call.args["brightness_pct"].clone()), (vec!["light.kitchen_main".to_owned()], "turn_on", json!(30)));
    assert_eq!(call.origin, "plugin");
    // Home Assistant said no: the words come back
    let (done, _) = act_answered(&r, &mut it, "light.kitchen_main", "toggle", json!(null), false);
    assert_eq!(done, Err(("failed", "the bulb is stuck".to_owned())));
    // a service the integration did not list is not asked: set_temperature is there, hvac mode off is a service it has
    let (done, call) = act_answered(&r, &mut it, "climate.hall", "temperature", json!(21.5), true);
    assert_eq!((done, call.map(|c| c.verb)), (Ok(()), Some("set_temperature".to_owned())));
    let (done, call) = act_answered(&r, &mut it, "scene.movie", "run", json!(null), true);
    assert_eq!((done, call.map(|c| c.verb)), (Ok(()), Some("turn_on".to_owned())));
}

#[test]
fn nothing_is_asked_for_an_unlock_a_garage_door_a_camera_or_something_that_is_not_shared() {
    let r = rig(true, Some(KEY));
    let mut it = connect(&r);
    it.send(&Message::Snapshot(snapshot())).unwrap();
    wait_for(&r, "shared", |d| d["shared"] == 6);
    for (id, verb) in [("lock.front_door", "unlock"), ("lock.front_door", "open"), ("cover.garage", "close"), ("cover.garage", "open"), ("camera.porch", "toggle"), ("light.nowhere", "toggle"), ("sensor.hall_temperature", "toggle")] {
        let (done, call) = act_answered(&r, &mut it, id, verb, json!(null), true);
        assert!(done.is_err(), "{id} {verb}");
        assert!(call.is_none(), "{id} {verb} must not reach the integration");
    }
    // not even a service the integration listed, for a thing the TV does not operate
    assert_eq!(act_answered(&r, &mut it, "cover.garage", "close", json!(null), true).0.unwrap_err().0, "not_allowed");
    // control paused in Home Assistant: tiles are shown and nothing is asked
    let mut paused = snapshot();
    paused.rev = 3;
    paused.policy = Policy { paused: true, pin_set: false };
    for e in &mut paused.entities {
        e.ctl.clear();
    }
    it.send(&Message::Snapshot(paused)).unwrap();
    let doc = wait_for(&r, "paused", |d| d["paused"] == true);
    assert_eq!(doc["detail"], "Control is paused in Home Assistant");
    assert_eq!(tile(&doc, "light.kitchen_main")["can"], json!([]));
    assert!(act_answered(&r, &mut it, "light.kitchen_main", "toggle", json!(null), true).1.is_none());
}

#[test]
fn the_owners_list_narrows_what_home_shows_and_is_kept() {
    let r = rig(true, Some(KEY));
    let mut it = connect(&r);
    it.send(&Message::Snapshot(snapshot())).unwrap();
    wait_for(&r, "shared", |d| d["shared"] == 6);
    let doc = r.onhome.pick(&json!({"hide": "lock.front_door"})).unwrap();
    assert_eq!(doc["tiles"].as_array().unwrap().len(), 5, "hiding one makes the list the owner's own");
    assert_eq!(doc["picked"].as_array().unwrap().len(), 5);
    r.onhome.pick(&json!({"hide": "scene.movie"})).unwrap();
    let doc = r.onhome.pick(&json!({"show": "scene.movie"})).unwrap();
    assert_eq!(ids(&doc), ["light.kitchen_main", "scene.movie", "climate.hall", "cover.garage", "sensor.hall_temperature"], "put back, it is last of its kind");
    // order inside a kind, never across kinds
    r.onhome.pick(&json!({"hide": "cover.garage"})).unwrap();
    let doc = r.onhome.pick(&json!({"up": "climate.hall"})).unwrap();
    assert_eq!(ids(&doc)[..3], ["light.kitchen_main", "scene.movie", "climate.hall"]);
    // a thing that is not shared is not shown, and something Home has no tile for is not accepted
    assert!(r.onhome.pick(&json!({"show": "camera.porch"})).is_err());
    assert!(r.onhome.pick(&json!({"explode": true})).is_err());
    // it is kept
    let kept = std::fs::read_to_string(r.ctx.paths.data_dir.join("onhome").join("picks.json")).unwrap();
    assert!(kept.contains("light.kitchen_main") && !kept.contains("lock.front_door"));
    // a thing that is not on the list is not controlled
    let (done, call) = act_answered(&r, &mut it, "lock.front_door", "lock", json!(null), true);
    assert!(done.is_err() && call.is_none(), "a thing that is not on the list is not controlled, a lock included");
    // and "all" puts it back
    let doc = r.onhome.pick(&json!({"all": true})).unwrap();
    assert_eq!(doc["picked"], Value::Null);
    assert!(!r.ctx.paths.data_dir.join("onhome").join("picks.json").exists());
}

#[test]
fn a_change_of_the_list_while_nothing_is_shared_is_refused_not_taken_as_hide_everything() {
    let r = rig(true, Some(KEY));
    r.onhome.look();
    let why = r.onhome.pick(&json!({"hide": "light.kitchen_main"})).unwrap_err();
    assert!(why.contains("not connected"), "{why}");
    assert!(!r.ctx.paths.data_dir.join("onhome").join("picks.json").exists());
    assert!(r.onhome.pick(&json!({"all": true})).is_ok(), "putting everything back is always possible");
}

#[test]
fn when_home_assistant_goes_the_tiles_say_so_in_their_own_names() {
    let r = rig(true, Some(KEY));
    let mut it = connect(&r);
    it.send(&Message::Snapshot(snapshot())).unwrap();
    wait_for(&r, "shared", |d| d["shared"] == 6);
    r.onhome.pick(&json!({"hide": "lock.front_door"})).unwrap();
    it.bye("unloaded").unwrap();
    let doc = wait_for(&r, "waiting", |d| d["state"] == "waiting");
    assert_eq!(doc["words"], "Waiting for Home Assistant");
    let light = tile(&doc, "light.kitchen_main");
    assert_eq!((light["name"].as_str(), light["words"].as_str(), light["available"].clone(), light["can"].clone()), (Some("Kitchen main light"), Some("Not reachable"), json!(false), json!([])));
    // and it is not controlled while it is away
    assert_eq!(r.onhome.act("light.kitchen_main", "toggle", &Value::Null).unwrap_err().code, "unavailable");
    // it comes back
    let mut again = connect(&r);
    again.send(&Message::Snapshot(snapshot())).unwrap();
    wait_for(&r, "back", |d| d["state"] == "connected");
}

#[test]
fn only_the_paired_integration_is_let_in_and_the_cameras_and_notifications_modules_are_not_offered() {
    let r = rig(true, Some(KEY));
    r.onhome.look();
    let addr = r.onhome.local_addr().unwrap();
    // another key: refused at the handshake
    let wrong = FakeIntegration::open(addr, &[9; 32]);
    assert!(wrong.is_err(), "a key that is not the TV's is not let in");
    let it = FakeIntegration::open(addr, &KEY).unwrap();
    let modules: Vec<&String> = it.welcome.as_ref().unwrap().modules.keys().collect();
    assert_eq!(modules, ["actions", "states"]);
}

#[test]
fn the_topic_never_carries_a_key_an_address_or_a_picture_address() {
    let r = rig(true, Some(KEY));
    let mut it = connect(&r);
    it.send(&Message::Snapshot(snapshot())).unwrap();
    let doc = wait_for(&r, "shared", |d| d["shared"] == 6);
    let text = doc.to_string();
    for forbidden in ["token", "access_token", "127.0.0.1", "entity_picture", "http"] {
        assert!(!text.contains(forbidden), "{forbidden} in {text}");
    }
}

#[test]
fn stopping_the_listener_ends_the_session_and_does_not_wait_on_itself() {
    let r = rig(true, Some(KEY));
    let mut it = connect(&r);
    it.send(&Message::Snapshot(snapshot())).unwrap();
    wait_for(&r, "shared", |d| d["shared"] == 6);
    let onhome = r.onhome.clone();
    let stopper = std::thread::spawn(move || onhome.shut_down());
    let until = Instant::now() + WITHIN;
    while !stopper.is_finished() {
        assert!(Instant::now() < until, "the listener did not stop");
        std::thread::sleep(Duration::from_millis(10));
    }
    stopper.join().unwrap();
    assert!(it.closed_within(WITHIN), "the integration is told it is over");
    assert!(r.onhome.local_addr().is_none());
}


// -- locks ---------------------------------------------------------------------------------------------------------------------

fn locks() -> Snapshot {
    let mut front = ent("lock.front_door", "Front door", Some("hall"), "locked", json!({}), &["lock"]);
    front.ctl_pin = vec!["unlock".into()];
    let mut back = ent("lock.back_door", "Back door", Some("hall"), "unlocked", json!({}), &["lock"]);
    back.ctl_pin = vec!["unlock".into()];
    // the integration listing unlock and open as ordinary services must not make them ordinary
    let mut loose = ent("lock.shed", "Shed", None, "locked", json!({}), &["lock", "unlock", "open"]);
    loose.ctl_pin = vec!["unlock".into(), "open".into()];
    let mut look_only = ent("lock.cellar", "Cellar", None, "locked", json!({}), &[]);
    look_only.ctl_pin = vec![];
    let mut no_unlock = ent("lock.gate", "Gate lock", None, "locked", json!({}), &["lock"]);
    no_unlock.ctl_pin = vec![];
    Snapshot { rev: 1, part: 0, more: false, policy: Policy::default(), areas: vec![AreaRec { id: "hall".into(), name: "Hall".into() }], entities: vec![front, back, loose, look_only, no_unlock, ent("light.lamp", "Lamp", None, "on", json!({}), &["turn_on", "turn_off", "toggle"])] }
}

fn shell_token(r: &Rig) -> String {
    let text = std::fs::read_to_string(r.ctx.paths.data_dir.join("onhome").join("shell.json")).expect("the shell's file");
    serde_json::from_str::<Value>(&text).unwrap()["token"].as_str().unwrap().to_owned()
}

fn journal(r: &Rig) -> Vec<Value> {
    std::fs::read_to_string(r.ctx.paths.data_dir.join("onhome").join("journal.jsonl")).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

/// `unlock` on its own thread, the integration's side answering `answer` (`None`: nothing is expected to arrive).
fn unlock_answered(r: &Rig, it: &mut FakeIntegration, id: &str, token: &str, answer: Option<(bool, Option<&str>)>) -> (Result<(), (&'static str, String)>, Option<ha_bridge::wire::Call>) {
    let (onhome, id, token) = (r.onhome.clone(), id.to_owned(), token.to_owned());
    let worker = std::thread::spawn(move || onhome.unlock(&id, &token).map_err(|f| (f.code, f.words)));
    let call = call_until_done(it, &worker);
    if let (Some(call), Some((ok, message))) = (&call, answer) {
        it.send(&Message::CallResult(CallResult { op: call.op, ok, code: message.map(|_| "pin_required".to_owned()), message: message.map(str::to_owned), results: vec![] })).unwrap();
    }
    (worker.join().unwrap(), call)
}

#[test]
fn a_lock_is_locked_from_the_api_in_one_call_and_shows_its_status() {
    let r = rig(true, Some(KEY));
    let mut it = connect(&r);
    it.send(&Message::Snapshot(locks())).unwrap();
    let doc = wait_for(&r, "locks", |d| d["shared"] == 6);
    assert_eq!((tile(&doc, "lock.back_door")["words"].clone(), tile(&doc, "lock.back_door")["can"].clone()), (json!("Unlocked"), json!(["lock"])));
    assert_eq!((tile(&doc, "lock.front_door")["words"].clone(), tile(&doc, "lock.front_door")["can"].clone()), (json!("Locked"), json!(["unlock"])));
    let (done, call) = act_answered(&r, &mut it, "lock.back_door", "lock", json!(null), true);
    assert_eq!(done, Ok(()));
    let call = call.expect("the integration was asked");
    assert_eq!((call.entity_ids, call.verb.as_str(), call.origin.as_str(), call.confirmed, call.pin), (vec!["lock.back_door".to_owned()], "lock", "plugin", None, None));
    // shown only: nothing is offered or asked
    assert_eq!(tile(&doc, "lock.cellar")["can"], json!([]));
    assert!(act_answered(&r, &mut it, "lock.cellar", "lock", json!(null), true).1.is_none());
}

#[test]
fn no_api_call_unlocks_or_unlatches_whatever_the_integration_lists_and_each_try_is_written_down() {
    let r = rig(true, Some(KEY));
    let mut it = connect(&r);
    it.send(&Message::Snapshot(locks())).unwrap();
    wait_for(&r, "locks", |d| d["shared"] == 6);
    for id in ["lock.front_door", "lock.back_door", "lock.shed"] {
        for verb in ["unlock", "open"] {
            let (done, call) = act_answered(&r, &mut it, id, verb, json!(null), true);
            assert_eq!(done.unwrap_err().0, "needs_shell", "{id} {verb}");
            assert!(call.is_none(), "{id} {verb} must not reach the integration");
        }
    }
    let lines = journal(&r);
    assert_eq!(lines.len(), 6);
    assert!(lines.iter().all(|l| l["result"] == "refused" && l["viaShell"] == false));
    // the route says the same, with the status of a refusal
    let (status, body) = lumatv_helper::onhome::testing::route(&r.onhome, "POST", "act", json!({"id": "lock.front_door", "verb": "unlock"}), "127.0.0.1");
    assert_eq!((status, body["error"]["code"].clone()), (409, json!("needs_shell")));
    let (status, body) = lumatv_helper::onhome::testing::route(&r.onhome, "POST", "act", json!({"id": "lock.front_door", "verb": "open"}), "127.0.0.1");
    assert_eq!((status, body["error"]["code"].clone()), (409, json!("needs_shell")));
    assert!(it.recv(Duration::from_millis(300)).is_none_or(|m| !matches!(m, Message::Call(_))));
}

#[test]
fn only_the_shell_with_its_token_on_the_loopback_unlocks_and_the_call_is_a_confirmed_touch() {
    let r = rig(true, Some(KEY));
    let mut it = connect(&r);
    it.send(&Message::Snapshot(locks())).unwrap();
    wait_for(&r, "locks", |d| d["shared"] == 6);
    let token = shell_token(&r);
    assert_eq!(token.len(), 64);
    // the file is the owner's alone
    use std::os::unix::fs::PermissionsExt;
    let dir = r.ctx.paths.data_dir.join("onhome");
    assert_eq!(std::fs::metadata(dir.join("shell.json")).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
    // no token, another token, a near token: refused, nothing reaches the integration
    for bad in ["", "x", &token[..63], &format!("{}0", &token[..63]), &token.to_uppercase()] {
        let (done, call) = unlock_answered(&r, &mut it, "lock.front_door", bad, None);
        assert_eq!(done.unwrap_err().0, "needs_shell", "{bad:?}");
        assert!(call.is_none());
    }
    // the right token: the integration is asked for a confirmed touch, with no PIN of the TV's
    let (done, call) = unlock_answered(&r, &mut it, "lock.front_door", &token, Some((true, None)));
    assert_eq!(done, Ok(()));
    let call = call.expect("asked");
    assert_eq!((call.verb.as_str(), call.origin.as_str(), call.confirmed, call.pin.clone(), call.entity_ids), ("unlock", "touch", Some(true), None, vec!["lock.front_door".to_owned()]));
    // Home Assistant's own rule is the last word, in words that say where the setting is
    let (done, _) = unlock_answered(&r, &mut it, "lock.back_door", &token, Some((false, Some("This action needs a confirmation and the PIN on the Show"))));
    let (code, words) = done.unwrap_err();
    assert_eq!(code, "failed");
    assert!(words.contains("LumaTV entry"), "{words}");
    // what it is not offered for, a thing that is not a lock, and what is not shared: not asked
    for id in ["lock.gate", "lock.cellar", "light.lamp", "lock.nowhere"] {
        let (done, call) = unlock_answered(&r, &mut it, id, &token, None);
        assert!(done.is_err(), "{id}");
        assert!(call.is_none(), "{id}");
    }
    // the unlatch is never asked, even with the token, and "shed" lists it as ordinary
    assert!(act_answered(&r, &mut it, "lock.shed", "open", json!(null), true).1.is_none());
    // every try is written down: who, which lock, how it ended
    let lines = journal(&r);
    assert!(lines.iter().any(|l| l["id"] == "lock.front_door" && l["result"] == "done" && l["viaShell"] == true));
    assert!(lines.iter().any(|l| l["id"] == "lock.back_door" && l["result"] == "failed"));
    assert!(lines.iter().filter(|l| l["why"].as_str().is_some_and(|w| w.contains("wrong token"))).count() >= 5);
    // the token is nowhere a client can read it
    assert!(!r.ctx.hub.get("onhome").unwrap().to_string().contains(&token));
    assert!(!r.onhome.status().to_string().contains(&token));
    assert!(!journal(&r).iter().any(|l| l.to_string().contains(&token)));
}

#[test]
fn the_unlock_route_is_the_loopbacks_alone_and_asks_for_the_token_in_the_body() {
    let r = rig(true, Some(KEY));
    let mut it = connect(&r);
    it.send(&Message::Snapshot(locks())).unwrap();
    wait_for(&r, "locks", |d| d["shared"] == 6);
    let token = shell_token(&r);
    let body = json!({"id": "lock.front_door", "token": token});
    // from another machine: refused before the token is looked at
    for peer in ["192.168.1.50", "10.0.0.7", "8.8.8.8"] {
        let (status, doc) = lumatv_helper::onhome::testing::route(&r.onhome, "POST", "unlock", body.clone(), peer);
        assert_eq!((status, doc["error"]["code"].clone()), (403, json!("needs_shell")), "{peer}");
    }
    assert!(it.recv(Duration::from_millis(300)).is_none_or(|m| !matches!(m, Message::Call(_))), "nothing was asked of the integration");
    // from the loopback with no token, or a wrong one
    for b in [json!({"id": "lock.front_door"}), json!({"token": token}), json!({"id": "lock.front_door", "token": "nope"})] {
        let (status, _) = lumatv_helper::onhome::testing::route(&r.onhome, "POST", "unlock", b, "127.0.0.1");
        assert!(status == 400 || status == 403);
    }
    // GET is not served
    let (status, _) = lumatv_helper::onhome::testing::route(&r.onhome, "GET", "unlock", json!({}), "127.0.0.1");
    assert_eq!(status, 405);
}

#[test]
fn every_other_door_to_home_assistant_still_refuses_locks() {
    // the plugins' lease (and the Show's plugin API under it) never calls a lock, whatever a manifest says
    assert!(lumatv_helper::plugins::ha::NEVER_CALLED.contains(&"lock"));
    assert!(lumatv_helper::plugins::ha::NEVER_CALLED.contains(&"alarm_control_panel"));
    // On Home's planner has no row at all for an unlock, for any entity, state or list of services
    let ent = lumatv_helper::onhome::tile::Ent { state: "locked".into(), attrs: Default::default() };
    for verb in ["unlock", "open"] {
        let all: Vec<String> = ["lock", "unlock", "open", "turn_on", "toggle"].map(String::from).to_vec();
        assert!(lumatv_helper::onhome::tile::plan("lock.x", &ent, &all, "", verb, &Value::Null).is_err());
    }
}
