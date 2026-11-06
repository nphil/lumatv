//! The Denon receiver plugin: one switch in Settings for everything the TV does with a Denon receiver. It asks first ([`OPT_IN`]), as
//! the helper's own plugins do, and while it is off none of it runs: the receiver is not read and nothing is sent to it (no telnet
//! connection, no power or input sync), and the volume keys change the TV's own volume.
//!
//! The switch is the receiver page (`denon-receiver`, the address and the web port): registered while the plugin is on and taken away
//! while it is off, so [`SettingsStore::receiver`] reads it as off and every part of the receiver client (`receiver`, `receiver_volume`,
//! `ai::receiver_ctl`) stands still. The page is the helper's own, not a plugin session's, so the address keeps its rule.

use crate::plugins::local::{Local, OptIn, Spec};
use crate::plugins::roster;
use crate::plugins::session::Shared;
use crate::settings::{Rule, SettingsStore, receiver_rule};
use parking_lot::Mutex;
use serde_json::json;
use settings_schema::{SettingsSchema, Value as Setting};
use std::path::Path;
use std::sync::Arc;

/// The plugin's id, which is also its settings owner.
pub const ID: &str = "denon-receiver";
pub const OPT_IN: OptIn = OptIn {
    spec: Spec {
        id: ID,
        name: "Denon receiver",
        version: "1",
        permissions: &["settings"],
    },
    consent: "Connects to the Denon receiver on the home network: the volume keys and the volume bar use the receiver's volume, its power \
              follows the TV over HDMI ARC (with the TV speakers for a while when it cannot turn on), and its sound modes and levels \
              are set in Settings.",
};
/// The Settings switch the plugin replaced.
const OLD_OWNER: &str = "base.receiver";

/// Why a receiver request is refused while the plugin is off.
pub fn off_words() -> &'static str {
    "The Denon receiver plugin is off: switch it on in Settings"
}

/// Registers the receiver page with its address rule.
pub(crate) fn register_page(settings: &SettingsStore) -> Result<(), String> {
    let schema = SettingsSchema::parse(include_str!("schemas/denon-receiver.json").as_bytes())
        .map_err(|e| e.to_string())?;
    settings
        .register(schema, Some(receiver_rule as Rule))
        .map_err(|e| e.0)
}

/// Carries `base.receiver` over to the plugin, once: a receiver that was not switched off (the switch was on unless stored off) comes up
/// with the plugin switched on, and its address and port are the plugin's. The old file goes once that is done; when `enabled.json`
/// cannot be written it stays, to be carried over at the next start.
pub fn migrate(settings: &SettingsStore, plugins_dir: &Path) {
    let Some(old) = settings.stored(OLD_OWNER) else {
        return;
    };
    let on = !matches!(old.get("enabled"), Some(Setting::Bool(false)));
    if on {
        let approved = roster::read_file(plugins_dir).and_then(|mut file| {
            file.first_party.retain(|a| a.id != ID);
            file.first_party.push(roster::Approved {
                id: ID.to_owned(),
                approve: OPT_IN.approval(),
            });
            roster::write_file(plugins_dir, &file)
        });
        if let Err(why) = approved {
            crate::log_warn!("receiver: {OLD_OWNER} was on and could not be carried over: {why}");
            return;
        }
    }
    match settings.carry_over(OLD_OWNER, ID, &["host", "port"]) {
        Ok(()) => crate::log_info!(
            "receiver: {OLD_OWNER} carried over to the {} plugin, {}",
            OPT_IN.spec.name,
            if on { "on" } else { "off" }
        ),
        Err(e) => crate::log_warn!("receiver: {OLD_OWNER} could not be carried over: {e}"),
    }
}

/// Follows the plugin's switch: the receiver page and the plugin's session while it is on, neither while it is off. `wake` runs after
/// every change of the switch and of the page's values, so the receiver client looks again at once.
pub struct Denon {
    settings: Arc<SettingsStore>,
    wake: Arc<dyn Fn() + Send + Sync>,
    /// The plugin's session (`Some` while on): it lists the plugin as connected.
    session: Mutex<Option<Arc<Local>>>,
}

impl Denon {
    /// Offers the switch. The roster keeps only a weak hold on the plugin, so dropping it ends the following.
    pub fn offer(
        shared: &Arc<Shared>,
        settings: Arc<SettingsStore>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Arc<Self> {
        let denon = Arc::new(Self {
            settings,
            wake,
            session: Mutex::new(None),
        });
        let (me, roster) = (Arc::downgrade(&denon), Arc::downgrade(shared));
        shared.offer(
            &OPT_IN,
            Arc::new(move || {
                if let (Some(me), Some(shared)) = (me.upgrade(), roster.upgrade()) {
                    me.follow(&shared);
                }
            }),
        );
        denon
    }

    fn follow(&self, shared: &Arc<Shared>) {
        let mut session = self.session.lock();
        let on = shared.switched_on(ID);
        if on == session.is_some() {
            return;
        }
        if on {
            if let Err(e) = register_page(&self.settings) {
                crate::log_warn!("receiver: the page was not registered: {e}");
                return;
            }
            let wake = self.wake.clone();
            self.settings.on_change(ID, move || wake());
            match Local::start(
                shared,
                &OPT_IN.spec,
                json!({"title": OPT_IN.spec.name}),
                Arc::new(|_, _| {}),
            ) {
                Ok(local) => *session = Some(local),
                Err(e) => {
                    crate::log_warn!("receiver: the plugin did not start: {e}");
                    self.settings.retire(ID);
                    return;
                }
            }
        } else {
            *session = None;
            self.settings.retire(ID);
        }
        drop(session);
        (self.wake)();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::Hub;
    use settings_schema::{SettingChange, Value as Change};

    fn store(dir: &Path) -> SettingsStore {
        let store = SettingsStore::new(dir.join("settings"), Arc::new(Hub::new(1)));
        store.register_builtin().unwrap();
        store
    }

    fn old(dir: &Path, values: serde_json::Value) {
        std::fs::create_dir_all(dir.join("settings")).unwrap();
        let doc = json!({"owner": OLD_OWNER, "schemaRevision": 1, "version": 2, "values": values});
        std::fs::write(dir.join("settings/base.receiver.json"), doc.to_string()).unwrap();
    }

    fn approved(dir: &Path) -> bool {
        roster::read_file(&dir.join("plugins"))
            .unwrap()
            .first_party
            .iter()
            .any(|a| a.id == ID && a.approve == OPT_IN.approval())
    }

    #[test]
    fn a_receiver_that_was_in_use_comes_up_switched_on_with_its_address() {
        let dir = tempfile::tempdir().unwrap();
        old(
            dir.path(),
            json!({"host": {"type": "text", "value": "10.0.0.7"}, "port": {"type": "integer", "value": 8081}}),
        );
        let s = store(dir.path());
        assert!(!s.receiver().enabled, "off until the plugin's page is registered");
        migrate(&s, &dir.path().join("plugins"));
        assert!(approved(dir.path()));
        assert!(!dir.path().join("settings/base.receiver.json").exists());
        register_page(&s).unwrap();
        let r = s.receiver();
        assert!(r.enabled);
        assert_eq!((r.host.as_str(), r.port), ("10.0.0.7", 8081));
        // once carried over, a second start finds nothing to do
        std::fs::remove_file(dir.path().join("plugins/enabled.json")).unwrap();
        migrate(&s, &dir.path().join("plugins"));
        assert!(!dir.path().join("plugins/enabled.json").exists());
    }

    #[test]
    fn a_receiver_switched_off_stays_off_and_a_new_tv_has_none() {
        let dir = tempfile::tempdir().unwrap();
        old(
            dir.path(),
            json!({"enabled": {"type": "bool", "value": false}, "host": {"type": "text", "value": "10.0.0.7"}}),
        );
        let s = store(dir.path());
        migrate(&s, &dir.path().join("plugins"));
        assert!(!dir.path().join("plugins/enabled.json").exists());
        assert!(!dir.path().join("settings/base.receiver.json").exists());
        register_page(&s).unwrap();
        assert_eq!(s.receiver().host, "10.0.0.7", "the address is kept for when it is switched on");

        let fresh = tempfile::tempdir().unwrap();
        let s = store(fresh.path());
        migrate(&s, &fresh.path().join("plugins"));
        assert!(!fresh.path().join("plugins/enabled.json").exists());
        assert!(!s.receiver().enabled);
    }

    #[test]
    fn the_address_keeps_its_rule() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        register_page(&s).unwrap();
        let host = |v: &str| {
            vec![SettingChange {
                key: "host".parse().unwrap(),
                value: Change::Text(v.into()),
            }]
        };
        assert!(s.apply(ID, host("evil host/../x"), None).is_err());
        s.apply(ID, host("avr.local"), None).unwrap();
        assert_eq!(s.receiver().host, "avr.local");
        s.retire(ID);
        assert!(!s.receiver().enabled, "the page taken away is the receiver off");
    }
}
