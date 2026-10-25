//! The plugin broker end to end: a real helper, a real unix socket, and LumaShow's own SDK as the plugin.
//!
//! The plugin runs as the test's own user, so these tests set `plugins.firstUid` to it; what is held to is the contract of
//! `docs/plugin-api.md`: only a switched-on, approved plugin connects; what it declares and publishes is checked by the contract's
//! own conversions and lands in topics; a plugin cannot take more than a plugin may (ringing cards, ongoing notifications,
//! other surfaces); the helper's settings and actions reach it; and everything of it goes when it goes.

mod support;

use lumashow_sdk::{Config as SdkConfig, Error, Plugin, Wait};
use plugin_api::Manifest;
use plugin_api::wire::{Register, ToBase, ToPlugin};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;
use support::*;

const WAIT: Duration = Duration::from_secs(10);

fn my_uid() -> u32 {
    // SAFETY: getuid cannot fail.
    unsafe { libc::getuid() }
}

fn manifest(id: &str, permissions: &str) -> String {
    format!(
        r#"{{"schema":1,"id":"{id}","name":"Hello","version":"0.1.0","api":{{"min":1,"target":1}},"run":{{"exe":"bin/hello"}},"class":"S","permissions":[{permissions}]}}"#
    )
}

struct PluginRig {
    rig: Rig,
    socket: PathBuf,
}

impl PluginRig {
    /// A helper with the broker's socket in the test's directory, scanning for plugins every second.
    fn prepare() -> (Rig, PathBuf) {
        let rig = Rig::new();
        receiver_off(&rig.paths);
        let socket = rig.dir.path().join("run").join("broker.sock");
        (rig, socket)
    }

    fn start(mut rig: Rig, socket: PathBuf) -> Self {
        let config = test_config(json!({"plugins": {
            "socket": socket, "rescanSeconds": 1, "handshakeSeconds": 2, "answerSeconds": 1, "firstUid": my_uid()}}));
        rig.start(config, Script::new());
        Self { rig, socket }
    }

    fn plugins_dir(&self) -> PathBuf {
        self.rig.paths.plugins_dir()
    }

    /// Installs a plugin the way the owner's flow will: its package, then the entry in `enabled.json` with the approval.
    fn switch_on(&self, id: &str, permissions: &str) {
        install(&self.plugins_dir(), &[(id, permissions, my_uid())]);
        post(self.rig.addr, "/v1/plugins/rescan", "{}");
    }

    fn sdk(&self) -> SdkConfig {
        SdkConfig::at(&self.socket)
    }

    /// Connects (waiting for the manager to open the socket) and returns the plugin after the welcome.
    fn connect(&self) -> Plugin {
        let mut last = None;
        for _ in 0..200 {
            match Plugin::connect(&self.sdk()) {
                Ok(plugin) => return plugin,
                Err(e) => last = Some(e),
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("the plugin did not connect: {:?}", last);
    }

    fn topic(&self, name: &str) -> Option<Value> {
        self.rig.topic(name)
    }

    fn wait_topic(&self, name: &str, check: impl Fn(&Value) -> bool) -> Option<Value> {
        self.rig.wait_topic(name, WAIT, check)
    }
}

/// Writes the packages and `enabled.json` of `plugins` (id, permissions, uid).
fn install(dir: &Path, plugins: &[(&str, &str, u32)]) {
    std::fs::create_dir_all(dir).unwrap();
    let mut listed = Vec::new();
    for (id, permissions, uid) in plugins {
        let text = manifest(id, permissions);
        std::fs::create_dir_all(dir.join(id)).unwrap();
        std::fs::write(dir.join(id).join("plugin.json"), &text).unwrap();
        let approve =
            lumatv_helper::plugins::roster::approval(&Manifest::parse(text.as_bytes()).unwrap());
        listed.push(json!({"id": id, "uid": uid, "approve": approve}));
    }
    std::fs::write(
        dir.join("enabled.json"),
        json!({"plugins": listed}).to_string(),
    )
    .unwrap();
}

fn register_json() -> Register {
    serde_json::from_value(json!({
        "title": "Hello",
        "settings": {"format": 1, "owner": "hello", "revision": 1, "title": "Hello", "sections": [{"id": "greeting", "title": "Greeting", "fields": [
            {"key": "name", "label": "Name", "kind": {"type": "text", "max_chars": 32, "secret": false},
             "default": {"type": "text", "value": "friend"}, "apply": "on-commit"}]}]},
        "actions": [{"key": "greet", "label": "Greet"}],
        "views": [{"id": "main", "title": "Hello", "presentation": "sheet"}],
        "channels": [{"id": "hi", "title": "Hi", "icon": "base.icons/message", "importance": "normal"}],
        "tiles": [{"id": "hello", "label": "Hello", "icon": "base.icons/message", "state": {"type": "off"}, "action": "greet"}]
    }))
    .unwrap()
}

fn message(value: Value) -> ToBase {
    serde_json::from_value(value).unwrap()
}

/// Waits up to `WAIT` for a message of the base that `pick` takes, keeping the rest of what arrives out of the way.
fn next_matching<T>(plugin: &mut Plugin, mut pick: impl FnMut(&ToPlugin) -> Option<T>) -> T {
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        assert!(
            !left.is_zero(),
            "the base did not send what the test waits for"
        );
        match plugin.wait(Some(left)).unwrap() {
            Wait::Message(m) => {
                if let Some(found) = pick(&m) {
                    return found;
                }
            }
            Wait::Timeout => panic!("timed out"),
            Wait::Closed => panic!("closed"),
        }
    }
}

fn refusal(plugin: &mut Plugin, what: &'static str) -> (String, String) {
    next_matching(plugin, |m| match m {
        ToPlugin::Refused {
            what: w,
            code,
            reason,
        } if w == what => Some((code.clone(), reason.clone())),
        _ => None,
    })
}

// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn with_no_plugin_on_there_is_no_socket_and_the_topic_says_so() {
    let (rig, socket) = PluginRig::prepare();
    let rig = PluginRig::start(rig, socket);
    let topic = rig
        .wait_topic("plugins", |v| v["plugins"] == json!([]))
        .expect("the plugins topic");
    assert_eq!(topic["problem"], Value::Null);
    assert!(!rig.socket.exists(), "no broker, no socket");
    assert!(matches!(
        Plugin::connect(&rig.sdk()),
        Err(Error::Connect(_))
    ));
    let listed = get(rig.rig.addr, "/v1/plugins").json();
    assert_eq!(listed["plugins"], json!([]));
}

#[test]
fn a_plugin_that_is_on_registers_and_what_it_publishes_becomes_topics() {
    let (rig, socket) = PluginRig::prepare();
    let rig = PluginRig::start(rig, socket);
    rig.switch_on("hello", r#""settings","cards","tiles","notify""#);
    let mut plugin = rig.connect();
    assert_eq!(plugin.welcome().id, "hello");
    assert_eq!(plugin.welcome().api, 1);
    assert_eq!(
        plugin.welcome().features,
        ["settings:1", "cards:1", "tiles:1", "notify:1", "facts:1"]
    );
    assert_eq!(plugin.register(register_json()).unwrap(), "hello");

    // The declared tile is there at once, and the plugin is connected.
    let tiles = rig
        .wait_topic("plugin.hello.tiles", |v| {
            v["items"].as_array().is_some_and(|a| a.len() == 1)
        })
        .unwrap();
    assert_eq!(tiles["items"][0]["id"], "hello");
    let plugins = rig
        .wait_topic("plugins", |v| v["plugins"][0]["state"] == "connected")
        .unwrap();
    assert_eq!(plugins["plugins"][0]["id"], "hello");
    assert_eq!(plugins["plugins"][0]["class"], "S");

    // A card, a notification, a fact.
    plugin
        .send(&message(json!({"t": "card.put", "id": "hello", "size": "half", "priority": "new", "content": {"type": "facts",
            "title": "Hello", "icon": "base.icons/message", "role": "accent", "facts": [{"label": "Greeting", "value": "Hello"}]},
            "open": {"view": "main"}, "actions": [{"action": "greet", "label": "Greet", "icon": "base.icons/message"}]})))
        .unwrap();
    plugin
        .send(&message(json!({"t": "notify.post", "id": "hi-1", "channel": "hi", "title": "Hello!", "body": "Somebody said hello.", "expires_in_secs": 600})))
        .unwrap();
    plugin
        .send(&message(
            json!({"t": "facts.put", "facts": {"link": "Connected"}}),
        ))
        .unwrap();
    let cards = rig
        .wait_topic("plugin.hello.cards", |v| {
            v["items"].as_array().is_some_and(|a| a.len() == 1)
        })
        .unwrap();
    assert_eq!(cards["items"][0]["revision"], 1);
    assert_eq!(
        cards["items"][0]["card"]["content"]["facts"][0]["value"],
        "Hello"
    );
    // Put again with the same id: the same card, a higher revision, the creation time kept.
    let created = cards["items"][0]["createdAt"].clone();
    plugin
        .send(&message(json!({"t": "card.put", "id": "hello", "size": "half", "priority": "new", "content": {"type": "facts",
            "title": "Hello", "icon": "base.icons/message", "role": "accent", "facts": [{"label": "Greeting", "value": "Hi again"}]},
            "open": {"view": "main"}})))
        .unwrap();
    let again = rig
        .wait_topic("plugin.hello.cards", |v| v["items"][0]["revision"] == 2)
        .unwrap();
    assert_eq!(again["items"][0]["createdAt"], created);
    let notes = rig
        .wait_topic("plugin.hello.notifications", |v| {
            v["items"].as_array().is_some_and(|a| a.len() == 1)
        })
        .unwrap();
    assert_eq!(notes["items"][0]["post"]["title"], "Hello!");
    assert!(
        notes["items"][0]["expiresAt"].as_i64().unwrap()
            > notes["items"][0]["receivedAt"].as_i64().unwrap()
    );
    let facts = rig
        .wait_topic("plugin.hello.facts", |v| v["facts"]["link"] == "Connected")
        .unwrap();
    assert_eq!(facts["facts"]["link"], "Connected");

    // Taking things away.
    plugin
        .send(&message(json!({"t": "card.remove", "id": "hello"})))
        .unwrap();
    plugin
        .send(&message(json!({"t": "notify.end", "id": "hi-1"})))
        .unwrap();
    plugin
        .send(&message(json!({"t": "tile.remove", "id": "hello"})))
        .unwrap();
    for topic in [
        "plugin.hello.cards",
        "plugin.hello.notifications",
        "plugin.hello.tiles",
    ] {
        assert!(
            rig.wait_topic(topic, |v| v["items"] == json!([])).is_some(),
            "{topic}"
        );
    }
}

#[test]
fn the_page_is_served_by_the_settings_api_and_the_persons_choice_reaches_the_plugin() {
    let (rig, socket) = PluginRig::prepare();
    let rig = PluginRig::start(rig, socket);
    rig.switch_on("hello", r#""settings","tiles","notify""#);
    let mut plugin = rig.connect();
    plugin.register(register_json()).unwrap();

    // After the registration the plugin is brought to the chosen values (the defaults on a first run).
    let first = next_matching(&mut plugin, |m| match m {
        ToPlugin::SettingsApply {
            txn,
            authority,
            values,
        } => Some((*txn, *authority, serde_json::to_value(values).unwrap())),
        _ => None,
    });
    assert_eq!(format!("{:?}", first.1), "Reconcile");
    assert_eq!(first.2["name"]["value"], "friend");
    plugin.send(&message(json!({"t": "settings.result", "txn": first.0, "outcome": {"type": "applied", "values": {}}}))).unwrap();

    let page = get(rig.rig.addr, "/v1/settings/hello");
    assert_eq!(page.status, 200);
    assert_eq!(page.json()["schema"]["owner"], "hello");

    let changed = post(
        rig.rig.addr,
        "/v1/settings/hello",
        r#"{"changes":[{"key":"name","value":{"type":"text","value":"Nitin"}}]}"#,
    );
    assert_eq!(
        changed.status,
        200,
        "{}",
        String::from_utf8_lossy(&changed.body)
    );
    let apply = next_matching(&mut plugin, |m| match m {
        ToPlugin::SettingsApply {
            txn,
            authority,
            values,
        } => Some((*txn, *authority, serde_json::to_value(values).unwrap())),
        _ => None,
    });
    assert_eq!(format!("{:?}", apply.1), "User");
    assert_eq!(apply.2["name"]["value"], "Nitin");

    // A rejection is an event, not a silent loss; an answer to nothing is refused.
    let hub = rig.rig.service().hub().clone();
    let seq = hub.events_after(None, None, None).seq;
    plugin
        .send(&message(json!({"t": "settings.result", "txn": apply.0, "outcome": {"type": "rejected", "errors": {"name": "I cannot greet by that name"}, "values": {}}})))
        .unwrap();
    plugin.send(&message(json!({"t": "settings.result", "txn": 999_999, "outcome": {"type": "applied", "values": {}}}))).unwrap();
    assert_eq!(refusal(&mut plugin, "settings.result").0, "unknown_txn");
    let topics = ["plugin.hello".to_owned()];
    let events = hub.events_after(Some(seq), None, Some(&topics)).events;
    assert!(
        events.iter().any(|e| e.kind == "settings_rejected"
            && e.data["errors"]["name"] == "I cannot greet by that name"),
        "{events:?}"
    );

    // The page goes with the connection; the choice stays and is what the plugin is brought to when it comes back.
    plugin.bye("done");
    assert!(wait_until(WAIT, || get(rig.rig.addr, "/v1/settings/hello")
        .status
        == 404));
    let mut again = rig.connect();
    again.register(register_json()).unwrap();
    let reconcile = next_matching(&mut again, |m| match m {
        ToPlugin::SettingsApply { values, .. } => Some(serde_json::to_value(values).unwrap()),
        _ => None,
    });
    assert_eq!(reconcile["name"]["value"], "Nitin");
}

#[test]
fn an_action_of_the_ui_reaches_the_plugin_and_its_answer_or_its_silence_is_reported() {
    let (rig, socket) = PluginRig::prepare();
    let rig = PluginRig::start(rig, socket);
    rig.switch_on("hello", r#""settings","tiles","notify""#);
    let mut plugin = rig.connect();
    plugin.register(register_json()).unwrap();
    let hub = rig.rig.service().hub().clone();
    let seq = hub.events_after(None, None, None).seq;
    let topics = ["plugin.hello".to_owned()];

    let asked = post(
        rig.rig.addr,
        "/v1/plugins/hello/actions/greet",
        r#"{"target":"somebody"}"#,
    );
    assert_eq!(
        asked.status,
        202,
        "{}",
        String::from_utf8_lossy(&asked.body)
    );
    let op = asked.json()["op"].as_u64().unwrap();
    let invoked = next_matching(&mut plugin, |m| match m {
        ToPlugin::ActionInvoke { op, action, target } => {
            Some((*op, action.clone(), target.clone()))
        }
        _ => None,
    });
    assert_eq!(
        invoked,
        (op, "greet".to_owned(), Some("somebody".to_owned()))
    );
    plugin
        .send(&message(
            json!({"t": "action.result", "op": op, "outcome": {"type": "ok"}}),
        ))
        .unwrap();
    assert!(wait_until(WAIT, || hub
        .events_after(Some(seq), None, Some(&topics))
        .events
        .iter()
        .any(|e| e.kind == "action_result"
            && e.data["op"] == op
            && e.data["outcome"]["type"] == "ok")));

    // Silence: reported as a failure when the answer time (1 s here) runs out.
    let silent = post(rig.rig.addr, "/v1/plugins/hello/actions/greet", "").json()["op"]
        .as_u64()
        .unwrap();
    assert!(wait_until(WAIT, || hub
        .events_after(Some(seq), None, Some(&topics))
        .events
        .iter()
        .any(|e| e.kind == "action_result"
            && e.data["op"] == silent
            && e.data["outcome"]["type"] == "failed")));

    // Only declared actions of connected plugins; only the plain body.
    assert_eq!(
        post(
            rig.rig.addr,
            "/v1/plugins/hello/actions/launch-missiles",
            ""
        )
        .status,
        404
    );
    assert_eq!(
        post(rig.rig.addr, "/v1/plugins/nobody/actions/greet", "").status,
        404
    );
    assert_eq!(
        post(
            rig.rig.addr,
            "/v1/plugins/hello/actions/greet",
            r#"{"op":1}"#
        )
        .status,
        400
    );
}

#[test]
fn a_plugin_cannot_take_more_than_a_plugin_may() {
    let (rig, socket) = PluginRig::prepare();
    let rig = PluginRig::start(rig, socket);
    rig.switch_on("hello", r#""settings","cards","tiles","notify""#);
    let mut plugin = rig.connect();

    // Before the declaration nothing is taken.
    plugin
        .send(&message(json!({"t": "tile.remove", "id": "x"})))
        .unwrap();
    assert_eq!(refusal(&mut plugin, "tile.remove").0, "not_registered");

    // A page that wants a secret, a lease, a provider or a timeline source is refused whole, in words.
    for (what, patch) in [
        (
            "secret",
            json!({"settings": {"format": 1, "owner": "hello", "revision": 1, "title": "Hello", "sections": [{"id": "s", "title": "S", "fields": [
            {"key": "token", "label": "Token", "kind": {"type": "text", "max_chars": 32, "secret": true}, "default": {"type": "text", "value": ""}, "apply": "on-commit"}]}]}}),
        ),
        (
            "lease",
            json!({"settings": {"format": 1, "owner": "hello", "revision": 1, "title": "Hello", "sections": [{"id": "s", "title": "S", "fields": [
            {"key": "lease.ha.url", "label": "URL", "kind": {"type": "text", "max_chars": 64, "secret": false}, "default": {"type": "text", "value": ""}, "apply": "on-commit"}]}]}}),
        ),
        (
            "source",
            json!({"sources": [{"id": "g", "title": "G", "icon": "base.icons/message"}]}),
        ),
    ] {
        let mut declaration = serde_json::to_value(register_json()).unwrap();
        declaration
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let declaration: Register = serde_json::from_value(declaration).unwrap();
        let error = plugin.register(declaration).unwrap_err();
        assert!(
            matches!(&error, Error::Refused(why) if !why.is_empty()),
            "{what}: {error:?}"
        );
    }
    // An urgent channel is the base's.
    let mut urgent = serde_json::to_value(register_json()).unwrap();
    urgent["channels"][0]["importance"] = json!("urgent");
    assert!(
        plugin
            .register(serde_json::from_value(urgent).unwrap())
            .is_err()
    );

    // After all of that, a proper declaration still works on the same connection.
    assert_eq!(plugin.register(register_json()).unwrap(), "hello");

    plugin
        .send(&message(json!({"t": "card.put", "id": "alarm", "size": "half", "priority": "ringing", "content": {"type": "facts",
            "title": "Alarm", "icon": "base.icons/message", "role": "danger", "facts": [{"label": "a", "value": "b"}]}, "open": {"view": "main"}})))
        .unwrap();
    assert_eq!(refusal(&mut plugin, "card.put").0, "not_permitted");
    plugin
        .send(&message(json!({"t": "card.put", "id": "c", "size": "half", "priority": "new", "content": {"type": "facts",
            "title": "C", "icon": "base.icons/message", "role": "accent", "facts": [{"label": "a", "value": "b"}]}, "open": {"view": "not-declared"}})))
        .unwrap();
    assert_eq!(refusal(&mut plugin, "card.put").0, "invalid");
    plugin
        .send(&message(json!({"t": "notify.post", "id": "n", "channel": "hi", "title": "Stays", "ongoing": true})))
        .unwrap();
    assert_eq!(refusal(&mut plugin, "notify.post").0, "not_permitted");
    plugin
        .send(&message(
            json!({"t": "notify.post", "id": "n", "channel": "not-declared", "title": "X"}),
        ))
        .unwrap();
    assert_eq!(refusal(&mut plugin, "notify.post").0, "invalid");
    plugin
        .send(&message(
            json!({"t": "ha.watch", "entities": ["light.kitchen"]}),
        ))
        .unwrap();
    assert_eq!(refusal(&mut plugin, "ha.watch").0, "not_served");
    plugin
        .send(&message(json!({"t": "net.listen", "op": 1, "port": 6053})))
        .unwrap();
    assert_eq!(refusal(&mut plugin, "net.listen").0, "not_served");
    plugin
        .send(&message(
            json!({"t": "signal.put", "kind": "occupancy", "state": "present", "ttl_s": 60}),
        ))
        .unwrap();
    assert_eq!(refusal(&mut plugin, "signal.put").0, "not_served");

    // Only so many cards at once.
    for n in 0..9 {
        plugin
            .send(&message(json!({"t": "card.put", "id": format!("k{n}"), "size": "half", "priority": "new", "content": {"type": "facts",
                "title": "K", "icon": "base.icons/message", "role": "accent", "facts": [{"label": "a", "value": "b"}]}, "open": {"view": "main"}})))
            .unwrap();
    }
    assert_eq!(refusal(&mut plugin, "card.put").0, "too_many");
    // Nothing that was refused is on the screen.
    let cards = rig
        .wait_topic("plugin.hello.cards", |v| {
            v["items"].as_array().is_some_and(|a| a.len() == 8)
        })
        .unwrap();
    assert!(
        cards["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["id"].as_str().unwrap().starts_with('k'))
    );
}

#[test]
fn a_user_that_is_not_a_plugin_and_a_plugin_that_is_not_approved_do_not_get_in() {
    let (rig, socket) = PluginRig::prepare();
    let rig = PluginRig::start(rig, socket);
    // One plugin is on, but for another user: this process is not it.
    install(&rig.plugins_dir(), &[("other", r#""cards""#, my_uid() + 1)]);
    post(rig.rig.addr, "/v1/plugins/rescan", "{}");
    assert!(
        rig.wait_topic("plugins", |v| v["plugins"][0]["id"] == "other"
            && v["plugins"][0]["state"] == "waiting")
            .is_some()
    );
    let mut tries = 0;
    loop {
        match Plugin::connect(&rig.sdk()) {
            Err(Error::NotSwitchedOn(_)) => break,
            Err(Error::Connect(_)) if tries < 100 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            other => panic!("{:?}", other.map(|_| ())),
        }
    }

    // A manifest that was changed after the owner approved it stays inert, and with nothing else on there is no broker at all.
    let text = manifest("tampered", r#""cards""#);
    std::fs::create_dir_all(rig.plugins_dir().join("tampered")).unwrap();
    std::fs::write(
        rig.plugins_dir().join("tampered").join("plugin.json"),
        manifest("tampered", r#""cards","tiles""#),
    )
    .unwrap();
    let approve =
        lumatv_helper::plugins::roster::approval(&Manifest::parse(text.as_bytes()).unwrap());
    std::fs::write(
        rig.plugins_dir().join("enabled.json"),
        json!({"plugins": [{"id": "tampered", "uid": my_uid(), "approve": approve}]}).to_string(),
    )
    .unwrap();
    post(rig.rig.addr, "/v1/plugins/rescan", "{}");
    let topic = rig
        .wait_topic("plugins", |v| v["plugins"][0]["state"] == "inert")
        .expect("inert");
    assert!(
        topic["plugins"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("not the one that was approved"),
        "{topic}"
    );
    assert!(
        wait_until(WAIT, || !rig.socket.exists()),
        "no member, no socket"
    );
}

#[test]
fn a_damaged_enabled_file_switches_nothing_on_and_says_why() {
    let (rig, socket) = PluginRig::prepare();
    let rig = PluginRig::start(rig, socket);
    std::fs::create_dir_all(rig.plugins_dir()).unwrap();
    std::fs::write(rig.plugins_dir().join("enabled.json"), "{").unwrap();
    post(rig.rig.addr, "/v1/plugins/rescan", "{}");
    let topic = rig
        .wait_topic("plugins", |v| v["problem"].is_string())
        .expect("a problem");
    assert!(
        topic["problem"]
            .as_str()
            .unwrap()
            .starts_with("enabled.json")
    );
    assert!(!rig.socket.exists());
}

#[test]
fn a_new_connection_replaces_the_old_one_and_its_leaving_takes_nothing_of_the_new_ones() {
    let (rig, socket) = PluginRig::prepare();
    let rig = PluginRig::start(rig, socket);
    rig.switch_on("hello", r#""settings","tiles","notify""#);
    let mut first = rig.connect();
    first.register(register_json()).unwrap();
    assert!(
        rig.wait_topic("plugin.hello.tiles", |v| v["items"]
            .as_array()
            .is_some_and(|a| a.len() == 1))
            .is_some()
    );
    let mut second = rig.connect();
    second.register(register_json()).unwrap();
    // The first is closed by the base.
    assert!(wait_until(WAIT, || matches!(
        first.try_message(),
        Ok(Wait::Closed) | Err(_)
    )));
    std::thread::sleep(Duration::from_millis(200));
    // What the old connection's end cleaned up did not take the new one's page and tile with it.
    assert_eq!(get(rig.rig.addr, "/v1/settings/hello").status, 200);
    assert_eq!(
        rig.topic("plugin.hello.tiles").unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let plugins = rig.topic("plugins").unwrap();
    assert_eq!(plugins["plugins"][0]["state"], "connected");
}

#[test]
fn the_end_of_a_connection_takes_everything_of_the_plugin_with_it() {
    let (rig, socket) = PluginRig::prepare();
    let rig = PluginRig::start(rig, socket);
    rig.switch_on("hello", r#""settings","cards","tiles","notify""#);
    let mut plugin = rig.connect();
    plugin.register(register_json()).unwrap();
    plugin
        .send(&message(json!({"t": "card.put", "id": "hello", "size": "half", "priority": "new", "content": {"type": "facts",
            "title": "Hello", "icon": "base.icons/message", "role": "accent", "facts": [{"label": "a", "value": "b"}]}, "open": {"view": "main"}})))
        .unwrap();
    assert!(
        rig.wait_topic("plugin.hello.cards", |v| v["items"]
            .as_array()
            .is_some_and(|a| a.len() == 1))
            .is_some()
    );
    drop(plugin); // a dropped connection is the same as `bye`
    assert!(
        rig.wait_topic("plugin.hello.cards", |v| v["items"] == json!([]))
            .is_some()
    );
    assert!(
        rig.wait_topic("plugin.hello.tiles", |v| v["items"] == json!([]))
            .is_some()
    );
    assert!(wait_until(WAIT, || get(rig.rig.addr, "/v1/settings/hello")
        .status
        == 404));
    assert!(
        rig.wait_topic("plugins", |v| v["plugins"][0]["state"] == "waiting")
            .is_some()
    );
}

#[test]
fn switching_a_plugin_off_asks_it_to_finish_and_closes_the_socket() {
    let (rig, socket) = PluginRig::prepare();
    let rig = PluginRig::start(rig, socket);
    rig.switch_on("hello", r#""cards""#);
    let mut plugin = rig.connect();
    plugin
        .register(serde_json::from_value(json!({"title": "Hello"})).unwrap())
        .unwrap();
    std::fs::write(rig.plugins_dir().join("enabled.json"), r#"{"plugins":[]}"#).unwrap();
    post(rig.rig.addr, "/v1/plugins/rescan", "{}");
    let reason = next_matching(&mut plugin, |m| match m {
        ToPlugin::Shutdown { reason, .. } => Some(reason.clone()),
        _ => None,
    });
    assert_eq!(reason, "disabled");
    assert!(wait_until(WAIT, || !rig.socket.exists()));
}

#[test]
fn the_helper_stopping_asks_plugins_to_finish_and_removes_the_socket() {
    let (rig, socket) = PluginRig::prepare();
    let mut rig = PluginRig::start(rig, socket);
    rig.switch_on("hello", r#""cards""#);
    let mut plugin = rig.connect();
    plugin
        .register(serde_json::from_value(json!({"title": "Hello"})).unwrap())
        .unwrap();
    rig.rig.stop();
    let reason = next_matching(&mut plugin, |m| match m {
        ToPlugin::Shutdown { reason, .. } => Some(reason.clone()),
        _ => None,
    });
    assert_eq!(reason, "stopping");
    assert!(!rig.socket.exists());
}

#[test]
fn bad_frames_unknown_types_and_floods_are_answered_and_bounded() {
    let (rig, socket) = PluginRig::prepare();
    let rig = PluginRig::start(rig, socket);
    rig.switch_on("hello", r#""cards""#);
    let mut plugin = rig.connect();
    plugin
        .register(serde_json::from_value(json!({"title": "Hello"})).unwrap())
        .unwrap();

    // A type of a later level: an error that is not fatal, and the connection goes on.
    plugin
        .send(&message(json!({"t": "card.remove", "id": "x"})))
        .unwrap();
    let mut raw = UnixStream::connect(&rig.socket).unwrap();
    let send = |stream: &mut UnixStream, body: &[u8]| {
        let mut frame = (body.len() as u32).to_le_bytes().to_vec();
        frame.extend_from_slice(body);
        stream.write_all(&frame).unwrap();
    };
    send(&mut raw, br#"{"t":"hello","min":1,"target":1}"#);
    send(&mut raw, br#"{"t":"settings.pgae"}"#);
    let mut text = Vec::new();
    raw.set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let mut buf = [0u8; 4096];
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline
        && !String::from_utf8_lossy(&text).contains("unknown_type")
    {
        if let Ok(n) = raw.read(&mut buf) {
            text.extend_from_slice(&buf[..n]);
        }
    }
    let text = String::from_utf8_lossy(&text).into_owned();
    assert!(
        text.contains(r#""t":"welcome""#)
            && text.contains("unknown_type")
            && text.contains(r#""fatal":false"#),
        "{text}"
    );
    // ...and still answers: a frame that is not an object ends it, in words.
    send(&mut raw, b"[1,2]");
    let mut rest = Vec::new();
    let _ = raw.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = raw.read_to_end(&mut rest);
    assert!(
        String::from_utf8_lossy(&rest).contains("bad_frame"),
        "{}",
        String::from_utf8_lossy(&rest)
    );

    // A length prefix of more than 64 KiB is refused before a byte of the body is kept.
    let mut huge = UnixStream::connect(&rig.socket).unwrap();
    huge.write_all(&(70_000u32).to_le_bytes()).unwrap();
    let mut answer = Vec::new();
    let _ = huge.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = huge.read_to_end(&mut answer);
    assert!(
        String::from_utf8_lossy(&answer).contains("bad_stream")
            || String::from_utf8_lossy(&answer).contains("bad_frame"),
        "{}",
        String::from_utf8_lossy(&answer)
    );

    // A flood: told off (`rate`), then hung up on, and the helper is none the worse.
    let mut flood = UnixStream::connect(&rig.socket).unwrap();
    send(&mut flood, br#"{"t":"hello","min":1,"target":1}"#);
    let body = br#"{"t":"card.remove","id":"x"}"#;
    let mut all = Vec::new();
    for _ in 0..700 {
        all.extend_from_slice(&(body.len() as u32).to_le_bytes());
        all.extend_from_slice(body);
    }
    let _ = flood.write_all(&all);
    let mut answer = Vec::new();
    let _ = flood.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = flood.read_to_end(&mut answer);
    let answer = String::from_utf8_lossy(&answer);
    assert!(
        answer.contains(r#""code":"rate""#) && answer.contains("rate_exceeded"),
        "{}",
        &answer[..answer.len().min(400)]
    );
    assert_eq!(get(rig.rig.addr, "/v1/health").status, 200);
    assert!(
        plugin.try_message().is_ok(),
        "the well-behaved connection was not touched"
    );
}

/// Reads one frame (a `u32` little-endian length, then that many bytes of JSON) from a plugin's socket.
fn read_frame(stream: &mut UnixStream) -> Value {
    let mut length = [0u8; 4];
    stream.read_exact(&mut length).unwrap();
    let mut body = vec![0u8; u32::from_le_bytes(length) as usize];
    stream.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[test]
fn the_welcome_tells_the_plugin_what_device_it_is_on_and_the_api_says_the_same() {
    let (mut rig, socket) = PluginRig::prepare();
    let config = test_config(json!({
        "plugins": {"socket": socket, "rescanSeconds": 1, "handshakeSeconds": 2, "answerSeconds": 1, "firstUid": my_uid()},
        "device": {"pointer": false, "detectControllers": false}
    }));
    rig.start(config, Script::new());
    let rig = PluginRig { rig, socket };
    rig.switch_on("hello", r#""cards""#);
    // A plugin built on the shared crate reads the welcome as it always did, whatever is added beside it.
    let plugin = rig.connect();
    assert_eq!(plugin.welcome().id, "hello");

    let mut raw = UnixStream::connect(&rig.socket).unwrap();
    let hello = br#"{"t":"hello","min":1,"target":1}"#;
    let mut frame = (hello.len() as u32).to_le_bytes().to_vec();
    frame.extend_from_slice(hello);
    raw.write_all(&frame).unwrap();
    raw.set_read_timeout(Some(WAIT)).unwrap();
    let welcome = read_frame(&mut raw);
    assert_eq!(welcome["t"], "welcome");
    assert_eq!(welcome["api"], 1);
    let device = json!({"screen": {"class": "tv", "width": 1920, "height": 1080}, "input": ["remote"]});
    assert_eq!(welcome["device"], device);

    let served = get(rig.rig.addr, "/v1/device").json();
    assert_eq!(served["ok"], true);
    assert_eq!(served["device"], device);
}
