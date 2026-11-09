//! First-party plugins: the helper's own components speaking LumaShow's plugin API like any plugin, with no socket.
//!
//! "Through the broker" is the same conversation. A [`Local`] says hello and registers with a [`Session`], the very type a
//! connection on the socket gets, so everything a first-party plugin publishes (a card, a tile, a notification) is checked by
//! the contract's own conversions, lands in the same topics (`plugins`, `plugin.<id>.cards`, ...), keeps the same limits, expires
//! on the same clock and is reached by the same action route (`POST /v1/plugins/<id>/actions/<action>`). The TV draws it with
//! the code that draws a plugin's, and the contributions are what another Luma device would draw of the same plugin.
//!
//! What is different, and nothing else:
//! - there is no program, user id or `enabled.json` entry: the manifest is in the code and the plugin is on, so the owner's approval
//!   (which is about installing software) is not asked of what is part of the helper. A plugin that opens something to the network asks
//!   first all the same ([`OptIn`]): it is listed, off, with its words, until the owner switches it on in Settings;
//! - messages are calls, not frames, so a plugin's work is a function call away from the topics it publishes;
//! - a first-party plugin is listed apart in the `plugins` topic (`firstParty`), and the ids it uses are not free for a plugin
//!   on the socket ([`FIRST_PARTY_IDS`]).

use super::roster::Member;
use super::session::{Outlet, Session, Shared};
use parking_lot::Mutex;
use plugin_api::Manifest;
use plugin_api::wire::{Hello, Register, ToBase, ToPlugin};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Weak};

/// The ids of the plugins the helper carries itself. A plugin on the socket may not take one of them. The Home Assistant plugin's
/// switch (`ha_plugin`) is not here: its id is the packaged `home-assistant` program's, which must keep it.
pub const FIRST_PARTY_IDS: &[&str] = &[
    "weather",
    "news",
    "clock",
    "ambient",
    "scrypted-cameras",
    "live-tv-tuner",
    "guide-data",
    "ble-proxy",
    "tv-sensors",
    "denon-receiver",
    "phone-remote",
];

/// What a first-party plugin is, in the words of a manifest.
pub struct Spec {
    pub id: &'static str,
    pub name: &'static str,
    pub version: &'static str,
    /// Manifest permissions (`cards`, `tiles`, `notify`, `settings`, `background`).
    pub permissions: &'static [&'static str],
}

/// A first-party plugin that asks first: it runs only once the owner has switched it on in Settings, approving exactly these words
/// (`enabled.json`'s `firstParty` keeps the code of them, [`OptIn::approval`]).
pub struct OptIn {
    pub spec: Spec,
    /// What switching it on does, in plain words: what the owner is shown and approves.
    pub consent: &'static str,
}

impl OptIn {
    /// The code of the consent: the hash of the id and the words, so that new words ask again.
    pub fn approval(&self) -> String {
        let digest = Sha256::digest(format!("{}\n{}", self.spec.id, self.consent));
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl Spec {
    fn manifest(&self) -> Result<Manifest, String> {
        let permissions: Vec<String> = self.permissions.iter().map(|p| format!("{p:?}")).collect();
        let text = format!(
            r#"{{"schema":1,"id":"{id}","name":"{name}","version":"{version}","api":{{"min":1,"target":1}},"run":{{"exe":"builtin/{id}"}},"class":"S","permissions":[{permissions}]}}"#,
            id = self.id,
            name = self.name,
            version = self.version,
            permissions = permissions.join(","),
        );
        Manifest::parse(text.as_bytes()).map_err(|e| format!("{}: {e}", self.id))
    }
}

/// What the base says to a first-party plugin that is not an answer: an action to run, mostly. The handler answers an action with
/// [`Local::answer`].
pub type Handler = Arc<dyn Fn(&Local, ToPlugin) + Send + Sync>;

/// The plugin's end of the connection: what the base sends it goes to its handler.
struct Hook {
    local: Weak<Local>,
    handler: Handler,
}

impl Outlet for Hook {
    fn send(&self, message: ToPlugin) -> bool {
        match self.local.upgrade() {
            Some(local) => {
                (self.handler)(&local, message);
                true
            }
            None => false,
        }
    }

    fn poke(&self) {}
}

/// One first-party plugin's side of its conversation with the broker's session.
pub struct Local {
    id: String,
    session: Mutex<Session>,
}

impl Local {
    /// Says hello and registers (`register` is the contract's `register` message, as JSON). Fails, in words, when the base refuses.
    pub fn start(
        shared: &Arc<Shared>,
        spec: &Spec,
        register: Json,
        handler: Handler,
    ) -> Result<Arc<Self>, String> {
        let member = Member {
            id: spec.id.to_owned(),
            uid: u32::MAX,
            manifest: spec.manifest()?,
            program: None,
        };
        let register: Register = serde_json::from_value(register)
            .map_err(|e| format!("{}: its register message is not valid: {e}", spec.id))?;
        shared.add_first_party(spec.id, spec.name, spec.version);
        let conn = shared.connection_id();
        let local = Arc::new_cyclic(|weak: &Weak<Local>| {
            let hook = Arc::new(Hook {
                local: weak.clone(),
                handler,
            });
            Local {
                id: spec.id.to_owned(),
                session: Mutex::new(Session::new(member, shared.clone(), conn, hook)),
            }
        });
        {
            let mut session = local.session.lock();
            let hello = session.message(ToBase::Hello(
                Hello::new(1, 1).sdk("lumatv-helper/first-party"),
            ));
            if hello.hangup {
                return Err(format!(
                    "{}: the base refused hello: {:?}",
                    spec.id, hello.messages
                ));
            }
            let registered = session.message(ToBase::Register(register));
            if !registered
                .messages
                .iter()
                .any(|m| matches!(m, ToPlugin::Registered { .. }))
            {
                return Err(format!(
                    "{}: registration was refused: {}",
                    spec.id,
                    words(&registered.messages)
                ));
            }
        }
        Ok(local)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// What the plugin says: a message of the contract as JSON (`{"t": "card.put", ...}`). A refusal (the same checks as over a
    /// socket: limits, permissions, what the contract's records accept) comes back as its words.
    pub fn say(&self, message: Json) -> Result<(), String> {
        let message: ToBase = serde_json::from_value(message)
            .map_err(|e| format!("{}: not a message of the contract: {e}", self.id))?;
        let reply = self.session.lock().message(message);
        if reply.messages.is_empty() {
            Ok(())
        } else {
            Err(format!("{}: {}", self.id, words(&reply.messages)))
        }
    }

    /// Answers an action the base asked for (`op` is the one in the `action.invoke`).
    pub fn answer(&self, op: u64, outcome: Result<(), &str>) -> Result<(), String> {
        let outcome = match outcome {
            Ok(()) => json!({"type": "ok"}),
            Err(reason) => json!({"type": "failed", "reason": reason}),
        };
        self.say(json!({"t": "action.result", "op": op, "outcome": outcome}))
    }

    /// Takes away what has expired.
    pub fn tick(&self) {
        self.session.lock().tick();
    }

    /// When the plugin next has something to do on its own (a card or notification expires), in Unix milliseconds.
    pub fn next_deadline_ms(&self) -> Option<u64> {
        self.session.lock().next_deadline_ms()
    }
}

/// A reply of the base to a plugin, in words: what was refused and why.
fn words(messages: &[ToPlugin]) -> String {
    messages
        .iter()
        .map(|m| match m {
            ToPlugin::Refused { what, reason, .. } => format!("{what} refused: {reason}"),
            ToPlugin::Error { message, .. } => message.clone(),
            other => other.name().to_owned(),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::Hub;
    use crate::settings::SettingsStore;
    use parking_lot::Mutex as PlMutex;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    const TOY: Spec = Spec {
        id: "toy",
        name: "Toy",
        version: "1",
        permissions: &["cards", "tiles", "notify"],
    };

    fn register() -> Json {
        json!({"title": "Toy",
               "actions": [{"key": "poke", "label": "Poke"}],
               "views": [{"id": "main", "title": "Toy", "presentation": "sheet"}],
               "channels": [{"id": "hi", "title": "Hi", "icon": "base.icons/message", "importance": "normal"}],
               "tiles": [{"id": "toy", "label": "Toy", "icon": "base.icons/lamp", "state": {"type": "off"}, "action": "poke"}]})
    }

    fn card(title: &str) -> Json {
        json!({"t": "card.put", "id": "c", "size": "half", "priority": "new", "expires_in_secs": 60,
               "content": {"type": "facts", "title": title, "icon": "base.icons/sun", "role": "info",
                           "facts": [{"label": "a", "value": "b"}]},
               "open": {"view": "main"}})
    }

    struct Fixture {
        hub: Arc<Hub>,
        shared: Arc<Shared>,
        clock: Arc<AtomicU64>,
        heard: Arc<PlMutex<Vec<ToPlugin>>>,
        _dir: tempfile::TempDir,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let hub = Arc::new(Hub::new(1));
        let clock = Arc::new(AtomicU64::new(1_800_000_000_000));
        let c = clock.clone();
        let shared = Shared::new(
            hub.clone(),
            Arc::new(SettingsStore::new(dir.path(), hub.clone())),
            Duration::from_secs(10),
            Arc::new(move || c.load(Ordering::SeqCst)),
        );
        Fixture {
            hub,
            shared,
            clock,
            heard: Arc::new(PlMutex::new(Vec::new())),
            _dir: dir,
        }
    }

    fn start(f: &Fixture) -> Arc<Local> {
        let heard = f.heard.clone();
        Local::start(
            &f.shared,
            &TOY,
            register(),
            Arc::new(move |local: &Local, message: ToPlugin| {
                if let ToPlugin::ActionInvoke { op, .. } = &message {
                    local.answer(*op, Ok(())).unwrap();
                }
                heard.lock().push(message);
            }),
        )
        .unwrap()
    }

    #[test]
    fn a_first_party_plugin_registers_through_the_broker_and_is_listed_apart() {
        let f = fixture();
        assert!(f.hub.get("plugins").is_none());
        let local = start(&f);
        let plugins = f.hub.get("plugins").unwrap();
        assert_eq!(
            plugins["plugins"],
            json!([]),
            "the owner's plugins are not mixed with ours"
        );
        assert_eq!(plugins["firstParty"][0]["id"], "toy");
        assert_eq!(plugins["firstParty"][0]["state"], "connected");
        assert_eq!(plugins["firstParty"][0]["firstParty"], true);
        // Its declared tile is published at once, as for any plugin.
        assert_eq!(
            f.hub.get("plugin.toy.tiles").unwrap()["items"][0]["id"],
            "toy"
        );

        local.say(card("Hello")).unwrap();
        let cards = f.hub.get("plugin.toy.cards").unwrap();
        assert_eq!(cards["items"][0]["card"]["content"]["title"], "Hello");
        assert_eq!(cards["items"][0]["revision"], 1);
        assert_eq!(cards["items"][0]["expiresAt"], 1_800_000_060);

        // It goes with its owner: nothing of it is left on the hub.
        drop(local);
        assert_eq!(f.hub.get("plugin.toy.cards").unwrap()["items"], json!([]));
        assert_eq!(
            f.hub.get("plugins").unwrap()["firstParty"][0]["state"],
            "waiting"
        );
    }

    #[test]
    fn what_a_first_party_plugin_says_is_held_to_the_contract() {
        let f = fixture();
        let local = start(&f);
        // The contract's own limits: a title of 129 characters, a card put at ringing priority, a view nobody declared.
        let long = local.say(card(&"t".repeat(129))).unwrap_err();
        assert!(long.contains("card.put refused"), "{long}");
        let ringing = local
            .say(json!({"t": "card.put", "id": "r", "size": "half", "priority": "ringing",
                        "content": {"type": "facts", "title": "R", "icon": "base.icons/sun", "role": "info",
                                    "facts": [{"label": "a", "value": "b"}]},
                        "open": {"view": "main"}}))
            .unwrap_err();
        assert!(ringing.contains("ringing is for the base"), "{ringing}");
        let view = local
            .say(json!({"t": "card.put", "id": "v", "size": "half", "priority": "new",
                        "content": {"type": "facts", "title": "V", "icon": "base.icons/sun", "role": "info",
                                    "facts": [{"label": "a", "value": "b"}]},
                        "open": {"view": "elsewhere"}}))
            .unwrap_err();
        assert!(view.contains("card.put refused"), "{view}");
        // Nothing of it got through, and a permission it did not ask for is refused too (`settings`).
        assert_eq!(f.hub.get("plugin.toy.cards").unwrap()["items"], json!([]));
        let facts = local
            .say(json!({"t": "facts.put", "facts": {"a": "b"}}))
            .unwrap_err();
        assert!(facts.contains("did not ask"), "{facts}");
        // A message that is not the contract's at all is said in words, not a panic.
        assert!(local.say(json!({"t": "nonsense"})).is_err());
    }

    #[test]
    fn an_action_of_the_ui_reaches_the_handler_and_its_answer_is_reported() {
        let f = fixture();
        let _local = start(&f);
        // The same path the UI takes: the shared state invokes the declared action.
        let op = f
            .shared
            .invoke_action("toy", "poke", Some("x".into()))
            .unwrap();
        let heard = f.heard.lock();
        assert!(
            matches!(&heard[0], ToPlugin::ActionInvoke { op: o, action, target, .. }
            if *o == op && action == "poke" && target.as_deref() == Some("x")),
            "{heard:?}"
        );
        // ...and answered at once, so there is nothing for the clock to time out.
        let events = f
            .hub
            .events_after(Some(0), None, Some(&["plugin.toy".to_owned()]));
        assert!(
            events
                .events
                .iter()
                .any(|e| e.kind == "action_result" && e.data["outcome"]["type"] == "ok"),
            "{events:?}"
        );
        assert_eq!(
            f.shared.invoke_action("toy", "nothing", None),
            Err(super::super::session::InvokeError::NoSuchAction)
        );
    }

    #[test]
    fn cards_expire_on_the_clock_like_any_plugins() {
        let f = fixture();
        let local = start(&f);
        local.say(card("Soon")).unwrap();
        assert_eq!(local.next_deadline_ms(), Some(1_800_000_060_000));
        f.clock.store(1_800_000_061_000, Ordering::SeqCst);
        local.tick();
        assert_eq!(f.hub.get("plugin.toy.cards").unwrap()["items"], json!([]));
        assert_eq!(local.next_deadline_ms(), None);
    }

    #[test]
    fn a_bad_register_message_is_reported_not_swallowed() {
        let f = fixture();
        let wrong = Local::start(
            &f.shared,
            &TOY,
            json!({"title": "Toy", "tiles": [{"id": "t", "label": "T", "icon": "base.icons/lamp", "state": {"type": "off"}, "action": "undeclared"}]}),
            Arc::new(|_, _| {}),
        );
        assert!(wrong.is_err());
        assert!(
            Local::start(&f.shared, &TOY, json!({"nonsense": 1}), Arc::new(|_, _| {})).is_err()
        );
    }
}
