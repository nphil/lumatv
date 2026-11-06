//! The Home Assistant plugin: one switch in Settings for everything on the TV that exists only for Home Assistant. It asks first
//! ([`OPT_IN`]), as the helper's own plugins do, and while it is off none of it runs:
//! - On Home does not listen (`onhome`): no Smart home card and no sheets of it, nothing for Home Assistant's LumaShow integration to dial;
//! - the packaged `home-assistant` program (the room and doorbell of the At home card, Home Assistant's alerts) is left off the roster
//!   (`plugins::roster`), so it is not started, or is stopped, and lets go of the Home Assistant lease;
//! - the weather comes from the built-in source whatever `base.weather` says (`homefeeds`), and the entity list is refused.
//!
//! It has no program of its own and is not in `FIRST_PARTY_IDS`: the packaged program keeps the id, its settings page and its approval
//! in `enabled.json`'s `plugins`. This switch is a second approval, in `firstParty`, and the program runs only with both.
//!
//! Not the plugin's, and left alone by its switch: the Home Assistant link (`ha/link.json`, `voice/ha.json`), core's shared connection;
//! and the BLE proxy and the TV sensors, Home Assistant's ESPHome device, which run on their own switches.

use crate::config::Paths;
use crate::plugins::local::{OptIn, Spec};
use crate::plugins::roster::{self, Approved};
use crate::settings::SettingsStore;
use std::path::Path;

/// The plugin's id: the packaged program's, so its settings page and its topics are the plugin's.
pub const ID: &str = "home-assistant";
pub const OPT_IN: OptIn = OptIn {
    spec: Spec {
        id: ID,
        name: "Home Assistant",
        version: "1",
        permissions: &[],
    },
    consent: "Shows what Home Assistant shares with this TV: the Smart home card and its sheets, the room and doorbell of the At home card \
              and Home Assistant's alerts, and lets the weather come from a Home Assistant weather entity. Home Assistant's LumaShow \
              integration connects to the TV on port 6054. The TV uses the Home Assistant link made from a shell (lumatv-helper ha-link) \
              and keeps its token on the TV.",
};

pub fn off_words() -> &'static str {
    "The Home Assistant plugin is off: switch it on in Settings"
}

/// Whether `enabled.json`'s `firstParty` (`list`) has the plugin switched on with the words it asks with now.
pub fn approved(list: &[Approved]) -> bool {
    let code = OPT_IN.approval();
    list.iter()
        .any(|a| a.id == ID && a.approve.eq_ignore_ascii_case(&code))
}

/// The marker that a TV set up before Home Assistant was a plugin has been carried over (`<data dir>/ha/plugin-migrated`).
const MIGRATED: &str = "plugin-migrated";

/// Carries a TV that used Home Assistant before it was a plugin over, once: On Home switched on in `helper.json`, the packaged program
/// approved in `enabled.json`, or the weather from a Home Assistant entity comes up with the plugin switched on and every setting where it
/// was (the packaged program's own, lamp control among them, are not touched). From then on the switch is the owner's. When
/// `enabled.json` cannot be read or written nothing is marked, to be tried at the next start.
pub fn migrate(paths: &Paths, settings: &SettingsStore) {
    let link = paths.ha_link_file();
    let Some(dir) = link.parent() else {
        return;
    };
    let marker = dir.join(MIGRATED);
    if marker.exists() {
        return;
    }
    let plugins_dir = paths.plugins_dir();
    let mut file = match roster::read_file(&plugins_dir) {
        Ok(file) => file,
        Err(why) => {
            crate::log_warn!("home assistant: not carried over to the {} plugin yet: {why}", OPT_IN.spec.name);
            return;
        }
    };
    let used = file.plugins.iter().any(|p| p.id == ID)
        || onhome_was_on(&paths.config_file())
        || settings.weather().source == "homeassistant";
    if used {
        file.first_party.retain(|a| a.id != ID);
        file.first_party.push(Approved {
            id: ID.to_owned(),
            approve: OPT_IN.approval(),
        });
        if let Err(why) = roster::write_file(&plugins_dir, &file) {
            crate::log_warn!("home assistant: not carried over to the {} plugin yet: {why}", OPT_IN.spec.name);
            return;
        }
    }
    match std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&marker, b"")) {
        Ok(()) if used => crate::log_info!("home assistant: carried over to the {} plugin, on", OPT_IN.spec.name),
        Ok(()) => {}
        Err(e) => crate::log_warn!("home assistant: {}: {e}", marker.display()),
    }
}

/// Whether `helper.json` switched On Home on the way it was switched before the plugin (`onhome.enabled`, no longer read).
fn onhome_was_on(file: &Path) -> bool {
    std::fs::read(file)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|v| v.pointer("/onhome/enabled") == Some(&serde_json::Value::Bool(true)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::Hub;
    use std::sync::Arc;

    /// The store, made once the files are in place: it reads them when the owners register.
    fn settings_of(paths: &Paths) -> SettingsStore {
        let settings = SettingsStore::new(paths.settings_dir(), Arc::new(Hub::new(1)));
        settings.register_builtin().unwrap();
        settings
    }

    fn on(paths: &Paths) -> bool {
        approved(&roster::read_file(&paths.plugins_dir()).unwrap().first_party)
    }

    fn package() -> roster::Enabled {
        roster::Enabled {
            id: ID.to_owned(),
            uid: 20000,
            approve: "ec66".to_owned(),
            program: None,
        }
    }

    #[test]
    fn a_tv_that_used_home_assistant_comes_up_with_the_plugin_on_once_and_a_new_one_with_it_off() {
        for used in ["helper.json", "package", "weather"] {
            let dir = tempfile::tempdir().unwrap();
            let paths = Paths::new(dir.path());
            match used {
                "helper.json" => std::fs::write(paths.config_file(), br#"{"onhome":{"enabled":true}}"#).unwrap(),
                "package" => {
                    let file = roster::EnabledFile { plugins: vec![package()], first_party: Vec::new() };
                    roster::write_file(&paths.plugins_dir(), &file).unwrap();
                }
                _ => {
                    std::fs::create_dir_all(paths.settings_dir()).unwrap();
                    let values = r#"{"source":{"type":"choice","value":"homeassistant"},"entity":{"type":"text","value":"weather.home"}}"#;
                    let doc = format!(r#"{{"owner":"base.weather","schemaRevision":1,"version":1,"values":{values}}}"#);
                    std::fs::write(paths.settings_dir().join("base.weather.json"), doc).unwrap();
                }
            }
            let settings = settings_of(&paths);
            migrate(&paths, &settings);
            assert!(on(&paths), "{used}");
            if used == "package" {
                let file = roster::read_file(&paths.plugins_dir()).unwrap();
                assert_eq!((file.plugins.len(), file.plugins[0].approve.as_str()), (1, "ec66"), "the package's approval is its own");
            }
            // switched off later by the owner, it stays off
            roster::write_file(&paths.plugins_dir(), &roster::EnabledFile::default()).unwrap();
            migrate(&paths, &settings);
            assert!(!on(&paths), "{used}");
        }
        // a TV that never used Home Assistant: marked, and off
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let settings = settings_of(&paths);
        migrate(&paths, &settings);
        assert!(!on(&paths) && paths.ha_link_file().with_file_name(MIGRATED).exists());
    }

    #[test]
    fn a_damaged_enabled_json_is_left_alone_and_tried_again_at_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let settings = settings_of(&paths);
        std::fs::write(paths.config_file(), br#"{"onhome":{"enabled":true}}"#).unwrap();
        std::fs::create_dir_all(paths.plugins_dir()).unwrap();
        std::fs::write(paths.plugins_dir().join("enabled.json"), b"{").unwrap();
        migrate(&paths, &settings);
        assert_eq!(std::fs::read(paths.plugins_dir().join("enabled.json")).unwrap(), b"{");
        assert!(!paths.ha_link_file().with_file_name(MIGRATED).exists());
        std::fs::remove_file(paths.plugins_dir().join("enabled.json")).unwrap();
        migrate(&paths, &settings);
        assert!(on(&paths));
    }
}
