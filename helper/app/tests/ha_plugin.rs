//! The Home Assistant plugin on the TV, end to end: LumaShow's own plugin (its library, unchanged), a real helper with its broker and
//! unix socket, and a fake Home Assistant (`ha_link::fake`) on the loopback. What is held to: the plugin connects as an approved
//! plugin and is told where the lease is; lights, scenes and a room's card appear in the topics the TV draws from; a tap on a tile
//! becomes a service call on Home Assistant; a doorbell ring and one of Home Assistant's own notifications arrive as a card and
//! notifications; a page's pickers are answered by the plugin through the choices route; the token reaches nothing the TV draws or
//! says; a token Home Assistant refuses is not tried again until the owner saves a link; forgetting deletes it; a plugin that asks to
//! call a door is not switched on; and the Home Assistant plugin's switch takes the program, its cards and its hold on the lease away and
//! back.

mod support;

use ha_link::fake::{FakeEntity, FakeHa};
use lumashow_sdk::Config as SdkConfig;
use lumatv_helper::ha_plugin::OPT_IN;
use lumatv_helper::ha_store;
use plugin_api::Manifest;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;
use support::*;

const TOKEN: &str = "eyJhbGciOiJIUzI1NiJ9.the-token-nothing-may-say.signature-part";
const WAIT: Duration = Duration::from_secs(20);
/// LumaShow's own manifest of the plugin, read where it lives: the approval is of exactly what the Show approves.
const MANIFEST: &str =
    include_str!("../../../../LumaShow/wt/lumatv/plugins/home-assistant/plugin.json");
const ID: &str = "home-assistant";

fn my_uid() -> u32 {
    // SAFETY: getuid cannot fail.
    unsafe { libc::getuid() }
}

struct Home {
    rig: Rig,
    ha: FakeHa,
    socket: PathBuf,
    stop: Arc<AtomicBool>,
    plugin: Option<JoinHandle<()>>,
}

/// A house with a living room (two lamps, a scene), a doorbell and a lock the plugin never asks about.
fn house() -> FakeHa {
    let ha = FakeHa::start(TOKEN);
    ha.add_area("living_room", "Living room");
    ha.add_area("kitchen", "Kitchen");
    for (id, name, area) in [
        ("light.lamp", "Lamp", "living_room"),
        ("light.shelf", "Shelf light", "living_room"),
        ("light.sink", "Sink light", "kitchen"),
        ("scene.movie", "Movie night", "living_room"),
    ] {
        ha.add_entity(FakeEntity::new(id, name).in_area(area));
    }
    ha.add_entity(FakeEntity::new("event.front_doorbell", "Front doorbell"));
    ha.add_entity(FakeEntity::new("lock.front_door", "Front door").in_area("kitchen"));
    ha.set_state(
        "light.lamp",
        "on",
        json!({"friendly_name": "Lamp", "brightness": 120}),
    );
    ha.set_state(
        "light.shelf",
        "off",
        json!({"friendly_name": "Shelf light"}),
    );
    ha.set_state("light.sink", "off", json!({"friendly_name": "Sink light"}));
    ha.set_state(
        "scene.movie",
        "scening",
        json!({"friendly_name": "Movie night"}),
    );
    ha.set_state(
        "event.front_doorbell",
        "2026-10-08T17:00:00.000+00:00",
        json!({"event_type": "pressed", "friendly_name": "Front doorbell"}),
    );
    ha.set_state(
        "lock.front_door",
        "locked",
        json!({"friendly_name": "Front door"}),
    );
    ha
}

impl Home {
    /// Everything on disk and listening, the plugin not yet running (`run_plugin`).
    fn new(linked: bool) -> Self {
        let ha = house();
        let mut rig = Rig::new();
        let dir = rig.paths.plugins_dir();
        std::fs::create_dir_all(dir.join(ID)).unwrap();
        std::fs::write(dir.join(ID).join("plugin.json"), MANIFEST).unwrap();
        let approve = lumatv_helper::plugins::roster::approval(
            &Manifest::parse(MANIFEST.as_bytes()).unwrap(),
        );
        std::fs::write(
            dir.join("enabled.json"),
            json!({"plugins": [{"id": ID, "uid": my_uid(), "approve": approve}], "firstParty": [switched_on()]}).to_string(),
        )
        .unwrap();
        if linked {
            ha_store::save(&rig.paths.ha_link_file(), &ha.url(), TOKEN).unwrap();
        }
        let socket = rig.dir.path().join("run").join("broker.sock");
        let config = test_config(json!({"plugins": {
            "socket": socket, "rescanSeconds": 1, "handshakeSeconds": 2, "answerSeconds": 3, "firstUid": my_uid()}}));
        rig.start_voice(config, Script::new());
        Self {
            rig,
            ha,
            socket,
            stop: Arc::new(AtomicBool::new(false)),
            plugin: None,
        }
    }

    /// Runs the real plugin, as a thread of this test (it speaks to the broker's socket as any plugin does, as this user).
    fn run_plugin(&mut self) {
        let (config, stop) = (SdkConfig::at(&self.socket), self.stop.clone());
        self.plugin = Some(std::thread::spawn(move || {
            let _ = plugin_home_assistant::run(&config, &|| stop.load(Ordering::SeqCst));
        }));
    }

    fn topic(&self, name: &str) -> Value {
        self.rig.topic(name).unwrap_or(Value::Null)
    }

    fn until(&self, name: &str, what: &str, check: impl Fn(&Value) -> bool) -> Value {
        self.rig
            .wait_topic(name, WAIT, check)
            .unwrap_or_else(|| panic!("{name}: no {what}; it is {}", self.topic(name)))
    }

    fn settings(&self, changes: Value) -> Reply {
        post(
            self.rig.addr,
            &format!("/v1/settings/{ID}"),
            &json!({"changes": changes}).to_string(),
        )
    }

    fn act(&self, action: &str, target: Option<&str>) -> Reply {
        let body = target.map_or("{}".to_owned(), |t| json!({"target": t}).to_string());
        post(
            self.rig.addr,
            &format!("/v1/plugins/{ID}/actions/{action}"),
            &body,
        )
    }

    /// Sets the room the way the TV's Settings does.
    fn choose_room(&self) {
        let r = self.settings(
            json!([{"key": "room", "value": {"type": "choice", "value": "living_room"}}]),
        );
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    }

    /// Switches the Home Assistant plugin as the TV's Settings does.
    fn switch(&self, on: bool) -> Reply {
        let body = if on { json!({"on": true, "approve": OPT_IN.approval()}) } else { json!({"on": false}) };
        http(
            self.rig.addr,
            "POST",
            &format!("/v1/plugins/{ID}/switch"),
            &[("X-Luma-Client", "lumatv-ui"), ("Content-Type", "application/json")],
            Some(&body.to_string()),
        )
    }

    fn items(&self, topic: &str) -> Vec<Value> {
        self.topic(topic)["items"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    fn tile(&self, id: &str) -> Option<Value> {
        self.items(&format!("plugin.{ID}.tiles"))
            .into_iter()
            .find(|t| t["id"] == id)
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.rig.stop();
        if let Some(plugin) = self.plugin.take() {
            let _ = plugin.join();
        }
    }
}

fn connected(v: &Value) -> bool {
    v["state"] == "connected"
}

/// The Home Assistant plugin's switch, on, as `enabled.json` keeps it: the packaged program runs only with it.
fn switched_on() -> Value {
    json!({"id": ID, "approve": OPT_IN.approval()})
}

fn has_items(v: &Value) -> bool {
    v["items"].as_array().is_some_and(|t| !t.is_empty())
}

#[test]
fn the_plugin_is_approved_connects_and_is_told_where_the_lease_is() {
    let mut home = Home::new(true);
    home.run_plugin();
    let roster = home.until("plugins", "the plugin connected", |v| {
        v["plugins"].as_array().is_some_and(|p| {
            p.iter()
                .any(|x| x["id"] == ID && x["state"] == "connected" && x["registered"] == true)
        })
    });
    let me = roster["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == ID)
        .unwrap();
    // the manifest's class (L since the plugin's widgets of 0.2.0); the ceilings follow it (`runner.rs`)
    assert_eq!(
        (me["name"].as_str(), me["class"].as_str()),
        (Some("Home Assistant"), Some("L"))
    );
    let link = home.until("ha.link", "a connected lease", connected);
    assert_eq!(link["address"], home.ha.url());
    assert_eq!(
        (link["token"].clone(), link["holders"].clone()),
        (json!(true), json!(1))
    );
    let facts = home.until(
        &format!("plugin.{ID}.facts"),
        "the connection's words",
        |v| {
            v["facts"]["link"]
                .as_str()
                .is_some_and(|l| l.starts_with("Connected to Home Assistant"))
        },
    );
    assert!(facts["facts"]["link"].is_string());
    assert_eq!(
        home.ha.connections(),
        1,
        "one connection, whatever the plugin does"
    );
}

#[test]
fn a_room_becomes_tiles_and_a_card_and_a_tap_becomes_a_service_call() {
    let mut home = Home::new(true);
    home.run_plugin();
    home.until("ha.link", "a connected lease", connected);
    home.choose_room();
    let tiles = home.until(
        &format!("plugin.{ID}.tiles"),
        "the room's lights and scene",
        |v| v["items"].as_array().is_some_and(|t| t.len() == 3),
    );
    let ids: Vec<&str> = tiles["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["id"].as_str())
        .collect();
    assert_eq!(
        ids,
        vec!["light.lamp", "light.shelf", "scene.movie"],
        "the room's, and not the kitchen's"
    );
    // The tiles come first and what Home Assistant says of the lights a moment after.
    home.until(&format!("plugin.{ID}.tiles"), "the lamp on", |v| {
        v["items"].as_array().is_some_and(|t| {
            t.iter()
                .any(|t| t["id"] == "light.lamp" && t["state"]["type"] == "on")
        })
    });
    let lamp = home.tile("light.lamp").unwrap();
    assert_eq!(lamp["state"]["type"], "on");
    assert_eq!(
        (lamp["action"].as_str(), lamp["target"].as_str()),
        (Some("toggle"), Some("light.lamp"))
    );
    assert_eq!(home.tile("light.shelf").unwrap()["state"]["type"], "off");
    assert_eq!(home.tile("scene.movie").unwrap()["action"], "activate");

    let cards = home.until(&format!("plugin.{ID}.cards"), "the room's card", |v| {
        v["items"]
            .as_array()
            .is_some_and(|c| c.iter().any(|c| c["id"] == "room"))
    });
    let room = cards["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "room")
        .unwrap()["card"]
        .clone();
    assert_eq!(room["content"]["title"], "Living room");
    assert_eq!(
        room["content"]["facts"][0],
        json!({"label": "Lights", "value": "1 of 2 on"})
    );
    assert_eq!(
        room["actions"][0]["action"], "all_off",
        "it can turn the room off while something is on"
    );

    // A tap on the shelf light is the broker's action route, which the plugin turns into a service call.
    assert_eq!(home.act("toggle", Some("light.shelf")).status, 202);
    assert!(wait_until(WAIT, || home.ha.calls().iter().any(|c| c
        .domain
        == "light"
        && c.service == "toggle"
        && c.entity_ids == ["light.shelf"])));
    home.until(&format!("plugin.{ID}.tiles"), "the shelf light on", |v| {
        v["items"].as_array().is_some_and(|t| {
            t.iter()
                .any(|t| t["id"] == "light.shelf" && t["state"]["type"] == "on")
        })
    });

    // "All off" is one call on the lights of the room, and the card follows.
    assert_eq!(home.act("all_off", None).status, 202);
    assert!(wait_until(WAIT, || home
        .ha
        .calls()
        .iter()
        .any(|c| c.service == "turn_off" && c.entity_ids.len() == 2)));
    home.until(&format!("plugin.{ID}.cards"), "no light on", |v| {
        v["items"].as_array().is_some_and(|c| {
            c.iter().any(|c| {
                c["id"] == "room" && c["card"]["content"]["facts"][0]["value"] == "0 of 2 on"
            })
        })
    });

    // The scene is set with `scene.turn_on`.
    assert_eq!(home.act("activate", Some("scene.movie")).status, 202);
    assert!(wait_until(WAIT, || home
        .ha
        .calls()
        .iter()
        .any(|c| c.domain == "scene" && c.service == "turn_on")));
    // Nothing else happened to the house, and the lock was never asked about.
    assert!(
        home.ha
            .calls()
            .iter()
            .all(|c| c.domain == "light" || c.domain == "scene")
    );
    assert!(
        home.ha
            .subscriptions()
            .iter()
            .all(|s| !s.contains("lock.front_door"))
    );
}

#[test]
fn a_doorbell_ring_and_a_home_assistant_notification_arrive_and_can_be_cleared() {
    let mut home = Home::new(true);
    home.run_plugin();
    home.until("ha.link", "a connected lease", connected);
    home.settings(json!([{"key": "doorbell", "value": {"type": "ordered", "value": ["event.front_doorbell"]}}]));
    home.choose_room();
    home.until(&format!("plugin.{ID}.tiles"), "tiles", |v| {
        v["items"].as_array().is_some_and(|t| !t.is_empty())
    });
    std::thread::sleep(Duration::from_millis(1500));
    home.ha.ring("event.front_doorbell", "pressed");
    let cards = home.until(&format!("plugin.{ID}.cards"), "the ring's card", |v| {
        v["items"].as_array().is_some_and(|c| {
            c.iter()
                .any(|c| c["id"].as_str().is_some_and(|i| i.starts_with("ring-")))
        })
    });
    let ring = cards["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"].as_str().unwrap().starts_with("ring-"))
        .unwrap()["card"]
        .clone();
    assert_eq!(
        (ring["size"].as_str(), ring["priority"].as_str()),
        (Some("full"), Some("running"))
    );
    assert_eq!(ring["content"]["title"], "Doorbell");
    assert_eq!(ring["content"]["role"], "danger");
    let notes = home.until(
        &format!("plugin.{ID}.notifications"),
        "the ring's notification",
        |v| {
            v["items"]
                .as_array()
                .is_some_and(|n| n.iter().any(|n| n["channel"] == "doorbell"))
        },
    );
    let note = notes["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["channel"] == "doorbell")
        .unwrap()
        .clone();
    assert_eq!(note["channelTitle"], "Doorbell");
    assert_eq!(note["post"]["actions"][0]["action"], "clear_doorbell");

    home.ha
        .notify("battery", "Low battery", "Door sensor is at 5 %");
    home.until(
        &format!("plugin.{ID}.notifications"),
        "Home Assistant's own",
        |v| {
            v["items"].as_array().is_some_and(|n| {
                n.iter()
                    .any(|n| n["channel"] == "ha" && n["post"]["body"] == "Door sensor is at 5 %")
            })
        },
    );

    // "Clear" on the ring takes the card and the notification away.
    let target = note["post"]["actions"][0]["target"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(home.act("clear_doorbell", Some(&target)).status, 202);
    home.until(&format!("plugin.{ID}.cards"), "the ring cleared", |v| {
        v["items"].as_array().is_some_and(|c| {
            !c.iter()
                .any(|c| c["id"].as_str().is_some_and(|i| i.starts_with("ring-")))
        })
    });
    home.until(
        &format!("plugin.{ID}.notifications"),
        "the ring's notification gone",
        |v| {
            v["items"]
                .as_array()
                .is_some_and(|n| !n.iter().any(|n| n["channel"] == "doorbell"))
        },
    );
}

#[test]
fn the_pickers_of_the_page_are_answered_by_the_plugin() {
    let mut home = Home::new(true);
    home.run_plugin();
    home.until("ha.link", "a connected lease", connected);
    let page = |provider: &str| {
        get(
            home.rig.addr,
            &format!("/v1/plugins/{ID}/choices/{provider}?limit=24"),
        )
    };
    let rooms = wait_for_ready(|| page("rooms"));
    let names: Vec<&str> = rooms["page"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["label"].as_str())
        .collect();
    assert_eq!(
        names,
        vec!["Kitchen", "Living room"]
            .into_iter()
            .filter(|n| names.contains(n))
            .collect::<Vec<_>>()
    );
    assert!(
        names.contains(&"Living room") && names.contains(&"Kitchen"),
        "{names:?}"
    );
    // Lights and scenes are offered for a room only once there is one.
    let none = wait_for_ready_or_state(|| page("controls"), "unconfigured");
    assert_eq!(none["page"]["rows"], json!([]));
    home.choose_room();
    let controls = wait_for_ready(|| page("controls"));
    let ids: Vec<&str> = controls["page"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    assert!(
        ids.contains(&"light.lamp") && ids.contains(&"scene.movie") && !ids.contains(&"light.sink"),
        "{ids:?}"
    );
    // A doorbell is looked for among every event and sensor by its name.
    let bells = wait_for_ready(|| page("doorbells"));
    let ids: Vec<&str> = bells["page"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    assert_eq!(ids, vec!["event.front_doorbell"]);
    // A provider the page does not name is not asked; a plugin that is not there is not either.
    assert_eq!(
        get(home.rig.addr, &format!("/v1/plugins/{ID}/choices/secrets")).status,
        404
    );
    assert_eq!(
        get(home.rig.addr, "/v1/plugins/nothing/choices/rooms").status,
        404
    );
    // A value the person picks is in the settings the plugin and the TV share.
    home.settings(json!([{"key": "controls", "value": {"type": "ordered", "value": ["light.shelf", "light.lamp"]}}]));
    let doc = get(home.rig.addr, &format!("/v1/settings/{ID}")).json();
    assert_eq!(
        doc["effective"]["controls"]["value"],
        json!(["light.shelf", "light.lamp"])
    );
    home.until(&format!("plugin.{ID}.tiles"), "the chosen order", |v| {
        v["items"].as_array().is_some_and(|t| t.len() == 2)
    });
}

fn wait_for_ready(ask: impl Fn() -> Reply) -> Value {
    wait_for_ready_or_state(ask, "ready")
}

fn wait_for_ready_or_state(ask: impl Fn() -> Reply, state: &str) -> Value {
    let mut last = Value::Null;
    assert!(
        wait_until(WAIT, || {
            let r = ask();
            last = r.json();
            r.status == 200 && last["page"]["state"] == state
        }),
        "never {state}: {last}"
    );
    last
}

#[test]
fn the_token_is_in_nothing_the_tv_draws_or_says() {
    let mut home = Home::new(true);
    home.run_plugin();
    home.until("ha.link", "a connected lease", connected);
    home.choose_room();
    home.until(&format!("plugin.{ID}.tiles"), "tiles", |v| {
        v["items"].as_array().is_some_and(|t| !t.is_empty())
    });
    for path in [
        "/v1/state".to_owned(),
        "/v1/plugins".to_owned(),
        format!("/v1/settings/{ID}"),
        "/v1/settings".to_owned(),
        "/v1".to_owned(),
    ] {
        let text = String::from_utf8_lossy(&get(home.rig.addr, &path).body).into_owned();
        assert!(
            !text.contains(TOKEN) && !text.contains("never-say"),
            "{path}"
        );
    }
    // The page the TV serves has no field for the address or the token: they are the owner's command line's.
    let doc = get(home.rig.addr, &format!("/v1/settings/{ID}")).json();
    let keys: Vec<String> = doc["schema"]["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["fields"].as_array().unwrap().iter())
        .filter_map(|f| f["key"].as_str().map(str::to_owned))
        .collect();
    assert!(keys.iter().all(|k| !k.starts_with("lease.")), "{keys:?}");
    let actions: Vec<String> = doc["schema"]["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["fields"].as_array().unwrap().iter())
        .filter_map(|f| f["kind"]["action"].as_str().map(str::to_owned))
        .collect();
    assert!(actions.is_empty(), "{actions:?}");
    // And no route sets either: a write of the page's lease keys is not a setting.
    for key in ["lease.ha.url", "lease.ha.token"] {
        let r = home.settings(
            json!([{"key": key, "value": {"type": "text", "value": "http://192.168.1.50:8123"}}]),
        );
        assert_eq!(r.status, 422, "{key}: {}", String::from_utf8_lossy(&r.body));
    }
    let file = std::fs::read_to_string(home.rig.paths.ha_link_file()).unwrap();
    assert!(
        file.contains(&home.ha.url()),
        "the saved link is the one the CLI wrote"
    );
}

#[test]
fn a_refused_token_is_not_tried_again_until_the_owner_saves_a_link_and_says_so() {
    let mut home = Home::new(true);
    home.ha.set_token("a-different-token");
    home.run_plugin();
    let link = home.until("ha.link", "a refused token", |v| {
        v["state"] == "auth_failed"
    });
    assert_eq!(
        link["detail"],
        "Home Assistant did not accept the access token"
    );
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        home.ha.auth_attempts(),
        1,
        "tried once, because Home Assistant bans an address that keeps sending a wrong token"
    );
    // The plugin says so on its page.
    home.until(&format!("plugin.{ID}.facts"), "the plugin's words", |v| {
        v["facts"]["link"]
            .as_str()
            .is_some_and(|l| l.starts_with("Home Assistant did not accept"))
    });
    // The owner gives Home Assistant's token and the helper is told.
    ha_store::save(
        &home.rig.paths.ha_link_file(),
        &home.ha.url(),
        "a-different-token",
    )
    .unwrap();
    assert_eq!(
        post(home.rig.addr, "/v1/plugins/lease/ha/refresh", "{}").status,
        202
    );
    home.until("ha.link", "connected again", connected);
    assert_eq!(home.ha.auth_attempts(), 2);
}

#[test]
fn forgetting_the_connection_deletes_the_link_and_stops_the_lights() {
    let mut home = Home::new(true);
    home.run_plugin();
    home.until("ha.link", "a connected lease", connected);
    home.choose_room();
    home.until(&format!("plugin.{ID}.tiles"), "tiles", |v| {
        v["items"].as_array().is_some_and(|t| !t.is_empty())
    });
    // The plugin declares the action; the base runs it.
    assert_eq!(home.act("lease.ha.forget", None).status, 202);
    let gone = home.until("ha.link", "an unlinked lease", |v| {
        v["state"] == "unconfigured"
    });
    assert_eq!(
        (gone["token"].clone(), gone["address"].clone()),
        (json!(false), Value::Null)
    );
    assert!(
        !home.rig.paths.ha_link_file().exists(),
        "the token is deleted"
    );
    assert!(wait_until(WAIT, || home.ha.connections() == 0));
    let tiles = home.until(
        &format!("plugin.{ID}.tiles"),
        "tiles that cannot be used",
        |v| {
            v["items"]
                .as_array()
                .is_some_and(|t| t.iter().all(|t| t["state"]["type"] == "unavailable"))
        },
    );
    assert!(!tiles["items"].as_array().unwrap().is_empty());
}

#[test]
fn with_no_link_the_plugin_connects_and_says_what_is_missing() {
    let mut home = Home::new(false);
    home.run_plugin();
    let link = home.until("ha.link", "an unlinked lease", |v| {
        v["state"] == "unconfigured"
    });
    assert_eq!(
        (link["token"].clone(), link["address"].clone()),
        (json!(false), Value::Null)
    );
    home.until(&format!("plugin.{ID}.facts"), "the plugin's words", |v| {
        v["facts"]["link"]
            .as_str()
            .is_some_and(|l| l.contains("not linked"))
    });
    assert_eq!(
        home.ha.total_connections(),
        0,
        "nothing is dialled without a link"
    );
}

#[test]
fn a_plugin_that_asks_to_call_a_door_is_not_switched_on() {
    let rig_dir = tempfile::tempdir().unwrap();
    for (permission, why) in [
        ("ha.call:light,cover", "cover"),
        ("ha.call:camera", "camera"),
        ("ha.call:valve", "valve"),
    ] {
        let text = MANIFEST.replace("ha.call:light,scene", permission);
        let manifest = Manifest::parse(text.as_bytes());
        let Ok(manifest) = manifest else {
            continue;
        };
        let dir = rig_dir.path().join(why);
        std::fs::create_dir_all(dir.join(ID)).unwrap();
        std::fs::write(dir.join(ID).join("plugin.json"), &text).unwrap();
        std::fs::write(
            dir.join("enabled.json"),
            json!({"plugins": [{"id": ID, "uid": 20000, "approve": lumatv_helper::plugins::roster::approval(&manifest)}], "firstParty": [switched_on()]})
                .to_string(),
        )
        .unwrap();
        let (roster, problem) = lumatv_helper::plugins::roster::Roster::load(&dir, 20000);
        assert!(problem.is_none() && roster.is_empty(), "{why}: {roster:?}");
        assert!(
            roster.inert[0].reason.contains("never lets a plugin call"),
            "{why}: {:?}",
            roster.inert
        );
    }
}

#[test]
fn switched_off_the_program_goes_with_its_cards_and_its_hold_on_the_lease_and_switched_on_it_comes_back() {
    let mut home = Home::new(true);
    home.run_plugin();
    home.until("ha.link", "a connected lease", connected);
    home.choose_room();
    home.until(&format!("plugin.{ID}.tiles"), "tiles", has_items);
    home.until(&format!("plugin.{ID}.cards"), "the room's card", has_items);
    assert_eq!(home.switch(false).status, 200);
    home.until(&format!("plugin.{ID}.tiles"), "no tiles", |v| !has_items(v));
    home.until(&format!("plugin.{ID}.cards"), "no cards", |v| !has_items(v));
    home.until("ha.link", "no holder", |v| v["holders"] == 0);
    let roster = home.until("plugins", "the switch off", |v| {
        v["firstParty"].as_array().is_some_and(|l| l.iter().any(|p| p["id"] == ID && p["state"] == "off"))
    });
    assert!(roster["plugins"].as_array().is_none_or(|l| l.iter().all(|p| p["id"] != ID)), "{roster}");
    // the broker told the program to finish; the runner starts it again on the TV, the test does here
    assert!(wait_until(WAIT, || home.plugin.as_ref().is_some_and(JoinHandle::is_finished)), "the program was told to finish");
    assert_eq!(home.switch(true).status, 200);
    home.run_plugin();
    home.until(&format!("plugin.{ID}.tiles"), "the tiles again", has_items);
    home.until("ha.link", "the lease again", connected);
}

/// The program that `lumatv-plugin-home-assistant` builds, in the workspace's target directory (and built if it is not there yet).
fn built_program() -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("target")
        });
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let program = target.join(profile).join("home-assistant");
    if !program.is_file() {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let mut build = std::process::Command::new(cargo);
        build.args(["build", "-p", "lumatv-plugin-home-assistant"]);
        if profile == "release" {
            build.arg("--release");
        }
        assert!(
            build.status().unwrap().success(),
            "the plugin's program did not build"
        );
    }
    assert!(program.is_file(), "{} is not there", program.display());
    program
}

/// The whole path of a plugin on the TV, with nothing stood in for but Home Assistant: the owner's `plugin-enable` installs LumaShow's
/// own `plugin.json` and the built program, the helper's runner starts that program as the plugin's user (20000, group 2000, no
/// other), the program finds the broker at the Show's socket path as every plugin does, is told where the lease is, and lights appear
/// in the topics. It needs what a TV has and a workstation may not: root (to become another user) and `/run/lumashow-plugins`, a
/// path that is global to the machine, so it is run on purpose: `cargo test -p lumatv-helper --test ha_plugin -- --ignored`.
#[test]
#[ignore = "needs root and the real socket path /run/lumashow-plugins/broker.sock; run on purpose with --ignored"]
fn the_installed_program_runs_as_its_own_user_and_works() {
    // SAFETY: geteuid has no preconditions.
    assert_eq!(unsafe { libc::geteuid() }, 0, "this test needs root");
    let ha = house();
    let mut rig = Rig::new();
    // The plugin runs as another user: every directory down to its program must be one it can enter.
    for dir in [rig.dir.path().to_owned(), rig.paths.data_dir.clone()] {
        std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
    }
    let package = rig.dir.path().join("package");
    std::fs::create_dir_all(package.join("bin")).unwrap();
    std::fs::write(package.join("plugin.json"), MANIFEST).unwrap();
    std::fs::copy(built_program(), package.join("bin").join(ID)).unwrap();
    let config = test_config(json!({"plugins": {"rescanSeconds": 1}}));
    let approve =
        lumatv_helper::plugins::roster::approval(&Manifest::parse(MANIFEST.as_bytes()).unwrap());
    let (code, doc) =
        lumatv_helper::plugins::install::run_enable(&rig.paths, &config, &package, &approve);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["uid"], 20000);
    ha_store::save(&rig.paths.ha_link_file(), &ha.url(), TOKEN).unwrap();
    rig.start_voice(config, Script::new());

    let wait_topic = |name: &str, check: &dyn Fn(&Value) -> bool| {
        rig.wait_topic(name, WAIT, check).unwrap_or_else(|| {
            let log = std::fs::read_to_string(rig.paths.plugins_dir().join(ID).join("log"))
                .unwrap_or_default();
            panic!(
                "{name}: nothing; the plugin said {log:?}; plugins are {:?}",
                rig.topic("plugins")
            )
        })
    };
    wait_topic("ha.link", &connected);
    let roster = wait_topic("plugins", &|v| v["plugins"][0]["state"] == "connected");
    assert_eq!(roster["plugins"][0]["id"], ID);
    // It really is the plugin's own user that is at the other end: the process is not root and is not this test.
    let owner = std::process::Command::new("ps")
        .args(["-eo", "uid,gid,comm"])
        .output()
        .unwrap();
    let ps = String::from_utf8_lossy(&owner.stdout).into_owned();
    assert!(
        ps.lines()
            .any(|l| l.split_whitespace().collect::<Vec<_>>() == ["20000", "2000", ID]),
        "{ps}"
    );
    let r = post(
        rig.addr,
        &format!("/v1/settings/{ID}"),
        &json!({"changes": [{"key": "room", "value": {"type": "choice", "value": "living_room"}}]})
            .to_string(),
    );
    assert_eq!(r.status, 200);
    wait_topic(&format!("plugin.{ID}.tiles"), &|v| {
        v["items"].as_array().is_some_and(|t| t.len() == 3)
    });
    // Switched off, the helper ends the program.
    let (code, _) = lumatv_helper::plugins::install::run_disable(
        &rig.paths,
        &lumatv_helper::config::Config::parse("{}").unwrap(),
        ID,
    );
    assert_eq!(code, 0);
    post(rig.addr, "/v1/plugins/rescan", "{}");
    assert!(
        wait_until(WAIT, || {
            let ps = std::process::Command::new("ps")
                .args(["-eo", "uid,comm"])
                .output()
                .unwrap();
            !String::from_utf8_lossy(&ps.stdout)
                .lines()
                .any(|l| l.split_whitespace().collect::<Vec<_>>() == ["20000", ID])
        }),
        "the program still runs"
    );
    rig.stop();
}

/// What the plugin and the helper put in the topics and on the routes the TV's screens read, for a house with a room, a doorbell that
/// rang and a notification: the fixture of the QML host's fake helper (`tests/qmlhost/fixtures/ha-plugin.json`). The screens are
/// drawn from this and not from a guess at it: this test fails when the fixture is not what the real plugin and helper make now, and
/// `LUMATV_WRITE_FIXTURES=1 cargo test -p lumatv-helper --test ha_plugin the_fixture` writes it again.
#[test]
fn the_fixture_of_the_tvs_screens_is_what_the_plugin_and_the_helper_make() {
    let mut home = Home::new(true);
    home.run_plugin();
    home.until("ha.link", "a connected lease", connected);
    home.settings(json!([
        {"key": "room", "value": {"type": "choice", "value": "living_room"}},
        {"key": "doorbell", "value": {"type": "ordered", "value": ["event.front_doorbell"]}}
    ]));
    home.until(&format!("plugin.{ID}.tiles"), "tiles", |v| {
        v["items"].as_array().is_some_and(|t| t.len() == 3)
    });
    home.until(&format!("plugin.{ID}.tiles"), "the lamp on", |v| {
        v["items"].as_array().is_some_and(|t| {
            t.iter()
                .any(|t| t["id"] == "light.lamp" && t["state"]["type"] == "on")
        })
    });
    std::thread::sleep(Duration::from_millis(1500));
    home.ha.ring("event.front_doorbell", "pressed");
    home.ha
        .notify("battery", "Low battery", "Door sensor is at 5 %");
    home.until(
        &format!("plugin.{ID}.notifications"),
        "both notifications",
        |v| {
            let items = v["items"].as_array().cloned().unwrap_or_default();
            items.iter().any(|n| n["channel"] == "doorbell")
                && items.iter().any(|n| n["channel"] == "ha")
        },
    );
    home.until(
        &format!("plugin.{ID}.cards"),
        "the room and the ring",
        |v| v["items"].as_array().is_some_and(|c| c.len() == 2),
    );
    home.until(&format!("plugin.{ID}.facts"), "the summary", |v| {
        v["facts"]["summary"].is_string()
    });
    // the helper's own plugins connect a moment after the start (ambient, clock, news and weather; the guide data, the tuner and the
    // cameras, which ask first, stay off; the Home Assistant plugin's switch is on); the document is of all of them
    home.until("plugins", "every first-party plugin connected", |v| {
        let list = v["firstParty"].as_array().cloned().unwrap_or_default();
        let connected = list.iter().filter(|p| p["state"] == "connected").count();
        connected == 4
            && list
                .iter()
                .all(|p| matches!(p["state"].as_str(), Some("connected" | "off" | "on")))
    });

    let page = |provider: &str| {
        wait_for_ready(|| {
            get(
                home.rig.addr,
                &format!("/v1/plugins/{ID}/choices/{provider}?limit=64"),
            )
        })["page"]
            .clone()
    };
    let settings = get(home.rig.addr, &format!("/v1/settings/{ID}")).json();
    let topics = [
        "ha.link",
        "plugins",
        "plugin.home-assistant.cards",
        "plugin.home-assistant.tiles",
        "plugin.home-assistant.notifications",
        "plugin.home-assistant.facts",
    ]
    .iter()
    .map(|t| ((*t).to_owned(), home.topic(t)))
    .collect::<serde_json::Map<_, _>>();
    let mut doc = json!({
        "settings": {"owner": settings["owner"], "schema": settings["schema"], "effective": settings["effective"]},
        "choices": {"rooms": page("rooms"), "controls": page("controls"), "doorbells": page("doorbells"), "presence": page("presence")},
        "topics": topics,
    });
    normalize(&mut doc, &home.ha.url());
    // the helper's own plugins connect in no fixed order
    if let Some(list) = doc["topics"]["plugins"]["firstParty"].as_array_mut() {
        list.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    }
    let text = serde_json::to_string_pretty(&doc).unwrap() + "\n";
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/qmlhost/fixtures/ha-plugin.json");
    if std::env::var_os("LUMATV_WRITE_FIXTURES").is_some() {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, &text).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&file).unwrap_or_default();
    assert!(
        committed == text,
        "{} is not what the plugin and the helper make now: LUMATV_WRITE_FIXTURES=1 cargo test -p lumatv-helper --test ha_plugin the_fixture",
        file.display()
    );
}

/// The parts of a document that differ from one run to another, made the same: times, the fake's address.
fn normalize(value: &mut Value, url: &str) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map.iter_mut() {
                if matches!(
                    key.as_str(),
                    "createdAt" | "receivedAt" | "expiresAt" | "at"
                ) && inner.is_number()
                {
                    *inner = json!(0);
                } else {
                    normalize(inner, url);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|v| normalize(v, url)),
        Value::String(text) if text.contains(url) => {
            *text = text.replace(url, "http://192.168.1.146:8123")
        }
        _ => {}
    }
}
