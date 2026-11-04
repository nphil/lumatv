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
    assert_eq!(ids(&doc), ["light.kitchen_main", "climate.hall", "cover.garage", "sensor.hall_temperature", "scene.movie"], "put back, it is last: nothing of its kind is on the list");
    // order inside a kind by up and down, never across kinds
    r.onhome.pick(&json!({"hide": "cover.garage"})).unwrap();
    let doc = r.onhome.pick(&json!({"up": "climate.hall"})).unwrap();
    assert_eq!(ids(&doc)[..3], ["light.kitchen_main", "climate.hall", "sensor.hall_temperature"], "no other thermostat to pass: it stays");
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
    let near = format!("{}{}", &token[..63], if token.ends_with('0') { '1' } else { '0' });
    for bad in ["", "x", &token[..63], &near, &token.to_uppercase()] {
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

// -- size, order and colour ----------------------------------------------------------------------------------------------------

fn lamps() -> Snapshot {
    Snapshot {
        rev: 1,
        part: 0,
        more: false,
        policy: Policy::default(),
        areas: vec![],
        entities: vec![
            ent("light.strip", "Strip", None, "on", json!({"brightness": 255, "supported_color_modes": ["rgb"], "color_mode": "rgb", "rgb_color": [255, 0, 128], "effect": "Rainbow", "effect_list": ["None", "Rainbow", "Pulse"]}), &["turn_on", "turn_off", "toggle"]),
            ent("light.desk", "Desk", None, "off", json!({"supported_color_modes": ["color_temp"]}), &["turn_on", "turn_off", "toggle"]),
            ent("switch.fan", "Fan switch", None, "off", json!({}), &["turn_on", "turn_off", "toggle"]),
            ent("sensor.hall_temperature", "Hall temperature", None, "21.4", json!({"device_class": "temperature", "unit_of_measurement": "\u{b0}C"}), &[]),
        ],
    }
}

#[test]
fn an_item_is_given_a_size_and_a_place_that_are_kept_and_come_back_on_its_tile() {
    let r = rig(true, Some(KEY));
    let mut it = connect(&r);
    it.send(&Message::Snapshot(lamps())).unwrap();
    wait_for(&r, "shared", |d| d["shared"] == 4);
    let doc = r.onhome.status();
    assert!(doc["tiles"].as_array().unwrap().iter().all(|t| t["size"].is_null()), "no size until the owner gives one: the kind's own");
    // a size is kept beside the list: it does not make the list the owner's own
    let doc = r.onhome.pick(&json!({"size": "light.strip", "to": "wide"})).unwrap();
    assert_eq!((tile(&doc, "light.strip")["size"].clone(), doc["picked"].clone()), (json!("wide"), Value::Null));
    let doc = r.onhome.pick(&json!({"size": "sensor.hall_temperature", "to": "large"})).unwrap();
    assert_eq!(tile(&doc, "sensor.hall_temperature")["size"], json!("large"));
    assert!(r.onhome.pick(&json!({"size": "light.strip", "to": "huge"})).is_err());
    assert!(r.onhome.pick(&json!({"size": "camera.porch", "to": "wide"})).is_err(), "something Home has no tile for has no size");
    // a place: the order crosses kinds, and the list becomes the owner's own
    let before = ids(&r.onhome.status()).into_iter().map(str::to_owned).collect::<Vec<_>>();
    assert_eq!(before, ["light.desk", "light.strip", "switch.fan", "sensor.hall_temperature"]);
    let doc = r.onhome.pick(&json!({"move": "sensor.hall_temperature", "to": 0})).unwrap();
    assert_eq!(ids(&doc), ["sensor.hall_temperature", "light.desk", "light.strip", "switch.fan"]);
    let doc = r.onhome.pick(&json!({"move": "sensor.hall_temperature", "to": 99})).unwrap();
    assert_eq!(ids(&doc), ["light.desk", "light.strip", "switch.fan", "sensor.hall_temperature"], "a place past the end is the end");
    assert!(r.onhome.pick(&json!({"move": "light.nothing", "to": 0})).is_err());
    assert!(r.onhome.pick(&json!({"move": "light.desk"})).is_err(), "a move needs its place");
    // it is kept, and read again by a helper that starts later
    let kept: Value = serde_json::from_str(&std::fs::read_to_string(r.ctx.paths.data_dir.join("onhome").join("picks.json")).unwrap()).unwrap();
    assert_eq!(kept["sizes"], json!({"light.strip": "wide", "sensor.hall_temperature": "large"}));
    let again = lumatv_helper::onhome::Picks::load(&r.ctx.paths.data_dir);
    assert_eq!(again.sizes.get("light.strip").map(String::as_str), Some("wide"));
    assert_eq!(again.list.as_ref().map(Vec::len), Some(4));
    // a size is taken back with null and "all" keeps the sizes, as it keeps nothing of the list
    let doc = r.onhome.pick(&json!({"size": "light.strip", "to": null})).unwrap();
    assert!(tile(&doc, "light.strip")["size"].is_null());
    let doc = r.onhome.pick(&json!({"all": true})).unwrap();
    assert_eq!((doc["picked"].clone(), tile(&doc, "sensor.hall_temperature")["size"].clone()), (Value::Null, json!("large")));
}

#[test]
fn a_lamp_says_its_colour_and_is_asked_for_a_colour_an_effect_and_a_white_in_the_integrations_words() {
    let r = rig(true, Some(KEY));
    let mut it = connect(&r);
    it.send(&Message::Snapshot(lamps())).unwrap();
    wait_for(&r, "shared", |d| d["shared"] == 4);
    let doc = r.onhome.status();
    let strip = tile(&doc, "light.strip");
    assert_eq!((strip["ctl"]["rgb"].clone(), strip["ctl"]["colour"].clone(), strip["ctl"]["effects"].clone()), (json!([255, 0, 128]), json!(true), json!(["None", "Rainbow", "Pulse"])));
    assert!(tile(&doc, "light.desk")["ctl"]["rgb"].is_null(), "off says no colour");
    let (done, call) = act_answered(&r, &mut it, "light.strip", "colour", json!({"rgb": [10, 20, 30]}), true);
    assert!(done.is_ok(), "{done:?}");
    let call = call.expect("a call");
    assert_eq!((call.verb.as_str(), call.args["rgb_color"].clone()), ("turn_on", json!([10, 20, 30])));
    let (done, call) = act_answered(&r, &mut it, "light.strip", "effect", json!("Pulse"), true);
    assert!(done.is_ok() && call.unwrap().args["effect"] == json!("Pulse"));
    let (done, call) = act_answered(&r, &mut it, "light.desk", "colour", json!([1, 2, 3]), true);
    assert!(done.is_err() && call.is_none(), "a white lamp has no colour to give");
    let (done, call) = act_answered(&r, &mut it, "light.strip", "effect", json!("Strobe"), true);
    assert!(done.is_err() && call.is_none(), "an effect it does not list is not asked");
}

// -- history ---------------------------------------------------------------------------------------------------------------------

mod history_rig {
    use ha_link::ws;
    use parking_lot::Mutex;
    use serde_json::{Value, json};
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    /// What the fake answers a command of one type with: the `result`, made from the command.
    pub type Answer = Box<dyn Fn(&Value) -> Value + Send + Sync>;

    pub struct Shared {
        pub token: String,
        pub answers: Mutex<BTreeMap<String, Answer>>,
        pub seen: Mutex<Vec<Value>>,
        pub auths: Mutex<Vec<String>>,
        pub connections: AtomicUsize,
        stop: AtomicBool,
    }

    /// A Home Assistant that speaks the WebSocket API's auth and answers the commands it was given answers for.
    pub struct FakeHa {
        pub addr: SocketAddr,
        pub shared: Arc<Shared>,
        thread: Option<JoinHandle<()>>,
    }

    impl FakeHa {
        pub fn start(token: &str) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let addr = listener.local_addr().unwrap();
            let shared = Arc::new(Shared { token: token.to_owned(), answers: Mutex::new(BTreeMap::new()), seen: Mutex::new(Vec::new()), auths: Mutex::new(Vec::new()), connections: AtomicUsize::new(0), stop: AtomicBool::new(false) });
            let accept = {
                let shared = shared.clone();
                thread::spawn(move || {
                    let mut children = Vec::new();
                    while !shared.stop.load(Ordering::SeqCst) {
                        match listener.accept() {
                            Ok((stream, _)) => {
                                let shared = shared.clone();
                                children.push(thread::spawn(move || serve(stream, &shared)));
                            }
                            Err(_) => thread::sleep(Duration::from_millis(5)),
                        }
                    }
                    for child in children {
                        let _ = child.join();
                    }
                })
            };
            Self { addr, shared, thread: Some(accept) }
        }

        pub fn url(&self) -> String {
            format!("http://{}", self.addr)
        }

        pub fn answer(&self, command: &str, f: impl Fn(&Value) -> Value + Send + Sync + 'static) {
            self.shared.answers.lock().insert(command.to_owned(), Box::new(f));
        }

        pub fn commands(&self) -> Vec<String> {
            self.shared.seen.lock().iter().map(|m| m["type"].as_str().unwrap_or("").to_owned()).collect()
        }
    }

    impl Drop for FakeHa {
        fn drop(&mut self) {
            self.shared.stop.store(true, Ordering::SeqCst);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    fn text_frame(value: &Value) -> Vec<u8> {
        let payload = value.to_string().into_bytes();
        let mut out = vec![0x81];
        match payload.len() {
            n if n < 126 => out.push(n as u8),
            n if n <= u16::MAX as usize => {
                out.push(126);
                out.extend_from_slice(&(n as u16).to_be_bytes());
            }
            n => {
                out.push(127);
                out.extend_from_slice(&(n as u64).to_be_bytes());
            }
        }
        out.extend_from_slice(&payload);
        out
    }

    fn take_frame(buf: &mut Vec<u8>) -> Option<(u8, Vec<u8>)> {
        if buf.len() < 2 {
            return None;
        }
        let opcode = buf[0] & 0x0f;
        let (len, at) = match buf[1] & 0x7f {
            126 if buf.len() >= 4 => (usize::from(u16::from_be_bytes([buf[2], buf[3]])), 4),
            126 => return None,
            127 if buf.len() >= 10 => (u64::from_be_bytes(buf[2..10].try_into().unwrap()) as usize, 10),
            127 => return None,
            n => (usize::from(n), 2),
        };
        if buf.len() < at + 4 + len {
            return None;
        }
        let mask: [u8; 4] = buf[at..at + 4].try_into().unwrap();
        let payload: Vec<u8> = buf[at + 4..at + 4 + len].iter().enumerate().map(|(i, b)| b ^ mask[i % 4]).collect();
        buf.drain(..at + 4 + len);
        Some((opcode, payload))
    }

    fn serve(mut stream: TcpStream, shared: &Shared) {
        stream.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        shared.connections.fetch_add(1, Ordering::SeqCst);
        let mut buf = Vec::new();
        let key = loop {
            if shared.stop.load(Ordering::SeqCst) {
                return;
            }
            let mut chunk = [0u8; 2048];
            match stream.read(&mut chunk) {
                Ok(0) => return,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(_) => {}
            }
            if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..end]).into_owned();
                buf.drain(..end + 4);
                break head.lines().find_map(|l| l.strip_prefix("Sec-WebSocket-Key: ")).expect("a key").to_owned();
            }
        };
        let accept = ws::accept_key(&key);
        stream.write_all(format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n").as_bytes()).unwrap();
        let send = |stream: &mut TcpStream, value: Value| {
            let _ = stream.write_all(&text_frame(&value));
        };
        send(&mut stream, json!({"type": "auth_required", "ha_version": "2026.10.0"}));
        let mut authed = false;
        loop {
            if shared.stop.load(Ordering::SeqCst) {
                return;
            }
            while let Some((opcode, payload)) = take_frame(&mut buf) {
                match opcode {
                    0x8 => return,
                    0x1 => {
                        let message: Value = serde_json::from_slice(&payload).unwrap();
                        if !authed {
                            let token = message["access_token"].as_str().unwrap_or("").to_owned();
                            shared.auths.lock().push(token.clone());
                            if token == shared.token {
                                authed = true;
                                send(&mut stream, json!({"type": "auth_ok", "ha_version": "2026.10.0"}));
                            } else {
                                send(&mut stream, json!({"type": "auth_invalid", "message": "Invalid access token or password"}));
                                return;
                            }
                        } else {
                            shared.seen.lock().push(message.clone());
                            let command = message["type"].as_str().unwrap_or("").to_owned();
                            let result = shared.answers.lock().get(&command).map(|f| f(&message));
                            match result {
                                Some(result) => send(&mut stream, json!({"id": message["id"], "type": "result", "success": true, "result": result})),
                                None => send(&mut stream, json!({"id": message["id"], "type": "result", "success": false, "error": {"code": "unknown_command", "message": "Unknown command."}})),
                            }
                        }
                    }
                    _ => {}
                }
            }
            let mut chunk = [0u8; 8192];
            match stream.read(&mut chunk) {
                Ok(0) => return,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(_) => {}
            }
        }
    }
}

const HA_TOKEN: &str = "eyJhbGciOiJIUzI1NiJ9.history-payload.history-signature";

/// A rig whose Home Assistant link (`ha/link.json`) names `ha`, or none.
fn linked(ha: Option<&history_rig::FakeHa>) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let hub = Arc::new(Hub::new(1));
    let settings = Arc::new(SettingsStore::new(dir.path().join("settings"), hub.clone()));
    let config = Config { onhome: OnHomeConfig { enabled: true, port: 0 }, ..Config::default() };
    let paths = Paths::new(dir.path());
    if let Some(ha) = ha {
        lumatv_helper::ha_store::save(&paths.ha_link_file(), &ha.url(), HA_TOKEN).expect("a link");
    }
    let ctx = Ctx { config: Arc::new(config), paths: paths.clone(), hub, settings, stop: Stop::new() };
    let mut deps = testing::keyed_deps(Some(KEY));
    deps.history.links = vec![paths.ha_link_file()];
    let onhome = OnHome::new(&ctx, deps);
    Rig { _dir: dir, ctx, onhome }
}

fn sensors() -> Snapshot {
    Snapshot {
        rev: 1,
        part: 0,
        more: false,
        policy: Policy::default(),
        areas: vec![],
        entities: vec![
            ent("sensor.hall_temperature", "Hall temperature", None, "21.4", json!({"device_class": "temperature", "unit_of_measurement": "\u{b0}C"}), &[]),
            ent("binary_sensor.front_door", "Front door", None, "off", json!({"device_class": "door"}), &[]),
            ent("sensor.house_mode", "House mode", None, "away_mode", json!({}), &[]),
            ent("light.desk", "Desk", None, "on", json!({}), &["toggle", "turn_on", "turn_off"]),
        ],
    }
}

fn seconds_of(message: &Value, key: &str) -> f64 {
    lumatv_helper::onhome::history::parse_iso(message[key].as_str().expect("a time")).expect("an ISO time")
}

#[test]
fn a_numbers_history_comes_from_the_recorder_through_the_saved_link_with_its_extremes_and_average() {
    let ha = history_rig::FakeHa::start(HA_TOKEN);
    ha.answer("history/history_during_period", |m| {
        let (from, to) = (seconds_of(m, "start_time"), seconds_of(m, "end_time"));
        let span = to - from;
        json!({"sensor.hall_temperature": [{"s": "20.0", "lu": from}, {"s": "22.0", "lu": from + span * 0.5}, {"s": "unavailable", "lu": from + span * 0.6}, {"s": "21.0", "lu": from + span * 0.75}]})
    });
    let r = linked(Some(&ha));
    let mut it = connect(&r);
    it.send(&Message::Snapshot(sensors())).unwrap();
    wait_for(&r, "shared", |d| d["shared"] == 4);
    let (status, doc) = testing::history_route(&r.onhome, "sensor.hall_temperature", "24h");
    assert_eq!(status, 200, "{doc}");
    let h = &doc["history"];
    assert_eq!((h["kind"].clone(), h["range"].clone(), h["unit"].clone(), h["class"].clone(), h["source"].clone()), (json!("number"), json!("24h"), json!("\u{b0}C"), json!("temperature"), json!("history")));
    assert_eq!(h["points"].as_array().unwrap().len(), 240);
    let (min, max, avg) = (h["stats"]["min"].as_f64().unwrap(), h["stats"]["max"].as_f64().unwrap(), h["stats"]["avg"].as_f64().unwrap());
    assert_eq!((min, max), (20.0, 22.0));
    assert!((avg - 20.75).abs() < 0.01, "20 for half a day, 22 for a quarter (the gap between holds it), 21 for the rest: {avg}");
    // the command is read-only, for this one sensor, in the compressed form
    let seen = ha.shared.seen.lock().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!((seen[0]["entity_ids"].clone(), seen[0]["minimal_response"].clone(), seen[0]["no_attributes"].clone()), (json!(["sensor.hall_temperature"]), json!(true), json!(true)));
    assert!((seconds_of(&seen[0], "end_time") - seconds_of(&seen[0], "start_time") - 86_400.0).abs() < 1.0);
    assert_eq!(ha.shared.auths.lock().as_slice(), [HA_TOKEN.to_owned()], "the saved token, to the saved address");
    // the same range again within the minute is not asked for again
    let (status, _) = testing::history_route(&r.onhome, "sensor.hall_temperature", "24h");
    assert_eq!((status, ha.shared.connections.load(std::sync::atomic::Ordering::SeqCst)), (200, 1));
    // the token is nowhere in the answer, the topic or a log line the helper keeps
    assert!(!doc.to_string().contains(HA_TOKEN) && !r.ctx.hub.get("onhome").unwrap().to_string().contains(HA_TOKEN));
}

#[test]
fn beyond_a_day_the_hourly_statistics_are_asked_first_and_the_states_when_there_are_none() {
    let ha = history_rig::FakeHa::start(HA_TOKEN);
    ha.answer("recorder/statistics_during_period", |m| {
        let from = seconds_of(m, "start_time");
        let rows: Vec<Value> = (0..168).map(|h| json!({"start": (from + h as f64 * 3600.0) * 1000.0, "end": (from + (h + 1) as f64 * 3600.0) * 1000.0, "mean": 20.0 + (h % 24) as f64 / 4.0, "min": 19.0, "max": 30.0 - (h % 5) as f64})).collect();
        json!({"sensor.hall_temperature": rows})
    });
    ha.answer("history/history_during_period", |m| {
        let from = seconds_of(m, "start_time");
        json!({"binary_sensor.front_door": [{"s": "off", "lu": from}, {"s": "on", "lu": from + 3600.0}, {"s": "off", "lu": from + 3700.0}], "sensor.house_mode": [{"s": "home", "lu": from}, {"s": "away_mode", "lu": from + 100.0}]})
    });
    let r = linked(Some(&ha));
    let mut it = connect(&r);
    it.send(&Message::Snapshot(sensors())).unwrap();
    wait_for(&r, "shared", |d| d["shared"] == 4);
    let (status, doc) = testing::history_route(&r.onhome, "sensor.hall_temperature", "7d");
    assert_eq!(status, 200, "{doc}");
    assert_eq!((doc["history"]["source"].clone(), doc["history"]["points"].as_array().unwrap().len()), (json!("statistics"), 168));
    assert_eq!((doc["history"]["stats"]["min"].clone(), doc["history"]["stats"]["max"].clone()), (json!(19.0), json!(30.0)));
    assert_eq!(ha.commands(), ["recorder/statistics_during_period"]);
    // no statistics for a sensor: the states of the range
    ha.answer("recorder/statistics_during_period", |_| json!({}));
    ha.answer("history/history_during_period", |m| {
        let from = seconds_of(m, "start_time");
        json!({"sensor.hall_temperature": [{"s": "18.5", "lu": from}, {"s": "19.5", "lu": from + 1000.0}]})
    });
    let (status, doc) = testing::history_route(&r.onhome, "sensor.hall_temperature", "30d");
    assert_eq!((status, doc["history"]["source"].clone()), (200, json!("history")), "{doc}");
    assert_eq!(ha.commands()[1..], ["recorder/statistics_during_period", "history/history_during_period"]);
}

#[test]
fn an_on_off_sensor_is_bands_and_a_sensor_of_words_is_its_recent_changes() {
    let ha = history_rig::FakeHa::start(HA_TOKEN);
    ha.answer("history/history_during_period", |m| {
        let from = seconds_of(m, "start_time");
        json!({"binary_sensor.front_door": [{"s": "off", "lu": from}, {"s": "on", "lu": from + 3600.0}, {"s": "off", "lu": from + 3700.0}], "sensor.house_mode": [{"s": "home", "lu": from}, {"s": "away_mode", "lu": from + 100.0}]})
    });
    let r = linked(Some(&ha));
    let mut it = connect(&r);
    it.send(&Message::Snapshot(sensors())).unwrap();
    wait_for(&r, "shared", |d| d["shared"] == 4);
    let (status, doc) = testing::history_route(&r.onhome, "binary_sensor.front_door", "24h");
    assert_eq!(status, 200, "{doc}");
    let h = &doc["history"];
    assert_eq!((h["kind"].clone(), h["changes"].clone(), h["now"].clone()), (json!("binary"), json!(2), json!("off")));
    assert_eq!(h["bands"].as_array().unwrap().iter().map(|b| b[2].as_str().unwrap()).collect::<Vec<_>>(), ["off", "on", "off"]);
    assert!(h["last_change"].as_f64().unwrap() > h["from"].as_f64().unwrap());
    assert!(ha.shared.seen.lock()[0]["significant_changes_only"] == json!(true), "a changing state is all a switch needs");
    let (status, doc) = testing::history_route(&r.onhome, "sensor.house_mode", "6h");
    assert_eq!(status, 200, "{doc}");
    assert_eq!(doc["history"]["kind"], json!("text"));
    assert_eq!(doc["history"]["changes"].as_array().unwrap()[0][1], json!("Away mode"));
}

#[test]
fn a_history_is_only_asked_for_a_shown_sensor_and_says_why_in_words_when_it_cannot_be_had() {
    let ha = history_rig::FakeHa::start(HA_TOKEN);
    ha.answer("history/history_during_period", |_| json!({}));
    let r = linked(Some(&ha));
    let mut it = connect(&r);
    it.send(&Message::Snapshot(sensors())).unwrap();
    wait_for(&r, "shared", |d| d["shared"] == 4);
    // a light is not a sensor, a thing that is not shared is nothing, a range that is not one of the five is refused
    for (id, range, code) in [("light.desk", "24h", "not_shown"), ("sensor.nothing", "24h", "not_shown"), ("camera.porch", "24h", "not_shown"), ("sensor.hall_temperature", "90d", "invalid")] {
        let (status, doc) = testing::history_route(&r.onhome, id, range);
        assert_eq!((status, doc["error"]["code"].clone()), (409, json!(code)), "{id} {range}: {doc}");
    }
    assert!(ha.commands().is_empty(), "nothing was asked of Home Assistant for any of them");
    // one that the owner took off Home is not asked for either
    r.onhome.pick(&json!({"hide": "sensor.hall_temperature"})).unwrap();
    assert_eq!(testing::history_route(&r.onhome, "sensor.hall_temperature", "24h").0, 409);
    // a Home Assistant that sends nothing for the sensor: an empty chart, said so by its being empty
    let (status, doc) = testing::history_route(&r.onhome, "binary_sensor.front_door", "1h");
    assert_eq!((status, doc["history"]["bands"].clone()), (200, json!([])), "{doc}");
    // no link, a token Home Assistant refuses, and Home Assistant gone
    let none = linked(None);
    let mut it2 = connect(&none);
    it2.send(&Message::Snapshot(sensors())).unwrap();
    wait_for(&none, "shared", |d| d["shared"] == 4);
    let (status, doc) = testing::history_route(&none.onhome, "sensor.hall_temperature", "24h");
    assert_eq!((status, doc["error"]["code"].clone()), (409, json!("no_link")));
    assert!(doc["error"]["message"].as_str().unwrap().contains("link"), "{doc}");
    let wrong = history_rig::FakeHa::start("another-token");
    let r3 = linked(Some(&wrong));
    let mut it3 = connect(&r3);
    it3.send(&Message::Snapshot(sensors())).unwrap();
    wait_for(&r3, "shared", |d| d["shared"] == 4);
    let (status, doc) = testing::history_route(&r3.onhome, "sensor.hall_temperature", "24h");
    assert_eq!((status, doc["error"]["code"].clone()), (409, json!("unavailable")), "{doc}");
    assert!(!doc.to_string().contains(HA_TOKEN));
    let gone = history_rig::FakeHa::start(HA_TOKEN);
    let r4 = linked(Some(&gone));
    let mut it4 = connect(&r4);
    it4.send(&Message::Snapshot(sensors())).unwrap();
    wait_for(&r4, "shared", |d| d["shared"] == 4);
    drop(gone);
    let (status, doc) = testing::history_route(&r4.onhome, "sensor.hall_temperature", "24h");
    assert_eq!((status, doc["error"]["code"].clone()), (409, json!("unavailable")), "{doc}");
}
