//! What the owner made of the remote's buttons (Settings > Remote & pointer > Buttons): the theme's `remote.buttons`,
//! `{"<button>": "<action>"}`, applied to a key before anything else sees it, as LG Input Hook did inside LG's input daemon (its
//! disable, replace and launch-an-app entries; running a shell command is not carried over).
//!
//! A button is named by id: the four app buttons (`netflix`, `amazon`, `disneyplus`, `lgchannels`, whose codes are the key table's),
//! the ones of [`BUTTONS`], or `code:N` for any other evdev code (what an import from Input Hook leaves for a button this list lacks).
//! A button the owner never set is absent and keeps what it did. The action is one word: `nothing`, `app:<id>` (an app, or an input,
//! which is the app `com.webos.app.hdmi1` and the like), `home`, `quick`, `settings`, `notifications`, `mute`, `pictureoff`, or
//! `as:<button>` (the remote then sends that button's code instead). Home is never remappable: holding it is the way out to LG's own
//! Home. LG Input Hook, while it is installed, acts inside LG's input daemon, before a key reaches the compositor: a button it disables,
//! launches or runs never arrives here, and one it replaces arrives as the replacement. State only: `input.rs` asks, `system_ui.rs` carries out.

use serde_json::Value;

use crate::input::code::*;
use crate::keys::{AppKey, KeyTable, Logical, SystemRequest};

const KEY_CHANNELUP: u32 = 402;
const KEY_CHANNELDOWN: u32 = 403;
const KEY_LIST: u32 = 787;
const KEY_MORE: u32 = 994;

/// The buttons that are not one of the four app buttons, by id. LG's codes (`network-input-service`'s table, as LG Input Hook's list of the
/// Magic Remote gives them).
pub const BUTTONS: [(&str, u32); 8] = [
    ("red", KEY_RED),
    ("green", KEY_GREEN),
    ("yellow", KEY_YELLOW),
    ("blue", KEY_BLUE),
    ("chup", KEY_CHANNELUP),
    ("chdown", KEY_CHANNELDOWN),
    ("list", KEY_LIST),
    ("more", KEY_MORE),
];

/// Codes no choice reaches: Home (its hold is the escape to LG's compositor), Power (nothing here may take it or hold it up) and the pointer's
/// own reports, which are no keys.
pub fn protected(code: u32) -> bool {
    matches!(code, KEY_HOME | KEY_HOMEPAGE | KEY_RF_HOME | KEY_POWER | KEY_POINTER_ALIVE | KEY_CURSOR_OFF)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ButtonAction {
    Nothing,
    App(String),
    Home,
    QuickPanel,
    Settings,
    Notifications,
    Mute,
    PictureOff,
    /// The remote sends this button's code in its place (Input Hook's Replace).
    As(String),
}

impl ButtonAction {
    pub fn parse(word: &str) -> Option<ButtonAction> {
        Some(match word {
            "nothing" => ButtonAction::Nothing,
            "home" => ButtonAction::Home,
            "quick" => ButtonAction::QuickPanel,
            "settings" => ButtonAction::Settings,
            "notifications" => ButtonAction::Notifications,
            "mute" => ButtonAction::Mute,
            "pictureoff" => ButtonAction::PictureOff,
            _ => match word.split_once(':')? {
                ("app", id) if !id.is_empty() => ButtonAction::App(id.to_string()),
                ("as", id) if valid_id(id) => ButtonAction::As(id.to_string()),
                _ => return None,
            },
        })
    }

    pub fn word(&self) -> String {
        match self {
            ButtonAction::Nothing => "nothing".into(),
            ButtonAction::App(id) => format!("app:{id}"),
            ButtonAction::Home => "home".into(),
            ButtonAction::QuickPanel => "quick".into(),
            ButtonAction::Settings => "settings".into(),
            ButtonAction::Notifications => "notifications".into(),
            ButtonAction::Mute => "mute".into(),
            ButtonAction::PictureOff => "pictureoff".into(),
            ButtonAction::As(id) => format!("as:{id}"),
        }
    }
}

/// A button id the map knows how to find a code for.
fn valid_id(id: &str) -> bool {
    AppKey::ALL.iter().any(|k| k.id() == id) || BUTTONS.iter().any(|(b, _)| *b == id) || custom_code(id).is_some()
}

fn custom_code(id: &str) -> Option<u32> {
    id.strip_prefix("code:")?.parse().ok().filter(|c| !protected(*c))
}

/// What a press of a button does now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Not the owner's to change: the key goes on as it was.
    Pass,
    /// The key goes on as this code (press, repeats and release).
    Replace(u32),
    /// Taken here: no app sees it, the request (if any) is asked once, at the press.
    Take(Option<SystemRequest>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ButtonMap {
    chosen: Vec<(String, ButtonAction)>,
}

impl ButtonMap {
    /// The theme's `remote.buttons`. An entry that is no button or no action is no choice.
    pub fn from_theme(theme: &Value) -> ButtonMap {
        let chosen = theme
            .pointer("/remote/buttons")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter(|(id, _)| valid_id(id))
            .filter_map(|(id, v)| Some((id.clone(), ButtonAction::parse(v.as_str()?)?)))
            .collect();
        ButtonMap { chosen }
    }

    /// What the owner chose, as (button id, action word), for the control socket.
    pub fn chosen(&self) -> Vec<(&str, String)> {
        self.chosen.iter().map(|(id, a)| (id.as_str(), a.word())).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.chosen.is_empty()
    }

    /// The id of the button `code` is, when it is one of the listed.
    fn known_id(code: u32, table: &KeyTable) -> Option<&'static str> {
        match table.classify(code) {
            Logical::App(k) => Some(k.id()),
            _ => BUTTONS.iter().find(|(_, c)| *c == code).map(|(id, _)| *id),
        }
    }

    /// What the owner chose for `code`: by the button's id, or by `code:N`, which is how an import leaves a button whose code it could not
    /// tell from the list's (an app button's code is the table's, and a remote may teach a different one).
    fn entry(&self, code: u32, table: &KeyTable) -> Option<&ButtonAction> {
        let known = Self::known_id(code, table);
        self.chosen.iter().find(|(b, _)| Some(b.as_str()) == known || custom_code(b) == Some(code)).map(|(_, a)| a)
    }

    /// The code a button id stands for: an app button's is the table's, which is how a code taught in `keys.conf` is found.
    fn code_of(id: &str, table: &KeyTable) -> Option<u32> {
        if let Some(k) = AppKey::ALL.into_iter().find(|k| k.id() == id) {
            return table.code_of(k);
        }
        BUTTONS.iter().find(|(b, _)| *b == id).map(|(_, c)| *c).or_else(|| custom_code(id))
    }

    /// Whether the owner chose anything for this code: what makes the key worth reading the theme again for.
    pub fn knows(&self, code: u32, table: &KeyTable) -> bool {
        self.entry(code, table).is_some()
    }

    /// Whether this code is one of the buttons Settings lists, which the owner may set at any moment.
    pub fn settable(code: u32, table: &KeyTable) -> bool {
        matches!(table.classify(code), Logical::App(_)) || BUTTONS.iter().any(|(_, c)| *c == code)
    }

    /// What a press of `code` does, over the app in front (`foreground`).
    pub fn decide(&self, code: u32, table: &KeyTable, foreground: &str) -> Decision {
        if protected(code) {
            return Decision::Pass;
        }
        let Some(action) = self.entry(code, table) else { return Decision::Pass };
        match action {
            ButtonAction::Nothing => Decision::Take(None),
            ButtonAction::App(app) if app == foreground => Decision::Take(None),
            ButtonAction::App(app) => Decision::Take(Some(SystemRequest::LaunchApp(app.clone()))),
            ButtonAction::QuickPanel => Decision::Take(Some(SystemRequest::QuickSettings)),
            ButtonAction::Settings => Decision::Take(Some(SystemRequest::OpenSettings)),
            ButtonAction::Notifications => Decision::Take(Some(SystemRequest::OpenNotifications)),
            ButtonAction::PictureOff => Decision::Take(Some(SystemRequest::PictureOff)),
            ButtonAction::Home => Decision::Replace(KEY_HOME),
            ButtonAction::Mute => Decision::Replace(KEY_MUTE),
            // A button this remote has no code for (Disney+ before `keys.conf` teaches it) is no button to act as.
            ButtonAction::As(other) => match Self::code_of(other, table) {
                Some(c) if c != code => Decision::Replace(c),
                _ => Decision::Take(None),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(buttons: Value) -> ButtonMap {
        ButtonMap::from_theme(&json!({"remote": {"buttons": buttons}}))
    }

    #[test]
    fn an_action_is_a_word_and_a_word_that_is_none_is_no_choice() {
        for word in
            ["nothing", "app:plex", "app:com.webos.app.hdmi2", "home", "quick", "settings", "notifications", "mute", "pictureoff", "as:red", "as:code:139"]
        {
            assert_eq!(ButtonAction::parse(word).map(|a| a.word()).as_deref(), Some(word));
        }
        for word in ["", "app:", "as:", "as:nobody", "as:code:x", "launch", "Home"] {
            assert_eq!(ButtonAction::parse(word), None, "{word:?}");
        }
        let m = map(json!({"red": "quick", "nobody": "home", "blue": 5, "green": "launch", "code:77": "nothing"}));
        assert_eq!(m.chosen(), vec![("code:77", "nothing".to_string()), ("red", "quick".to_string())]);
        assert_eq!(ButtonMap::from_theme(&json!({})), ButtonMap::default());
        assert_eq!(ButtonMap::from_theme(&json!({"remote": {"buttons": []}})), ButtonMap::default());
    }

    #[test]
    fn a_button_the_owner_did_not_set_goes_on_as_it_was() {
        let t = KeyTable::default();
        let m = map(json!({"red": "quick"}));
        assert_eq!(m.decide(KEY_GREEN, &t, ""), Decision::Pass);
        assert_eq!(m.decide(KEY_NETFLIX, &t, ""), Decision::Pass);
        assert_eq!(ButtonMap::default().decide(KEY_RED, &t, ""), Decision::Pass);
    }

    #[test]
    fn each_action_is_what_its_word_says() {
        let t = KeyTable::default();
        let m = map(json!({
            "red": "app:plex", "green": "nothing", "yellow": "quick", "blue": "settings", "chup": "notifications", "chdown": "pictureoff",
            "list": "home", "more": "mute", "code:78": "as:blue", "code:77": "as:red"
        }));
        assert_eq!(m.decide(KEY_RED, &t, "x"), Decision::Take(Some(SystemRequest::LaunchApp("plex".into()))));
        assert_eq!(m.decide(KEY_RED, &t, "plex"), Decision::Take(None), "its app is in front already");
        assert_eq!(m.decide(KEY_GREEN, &t, ""), Decision::Take(None));
        assert_eq!(m.decide(KEY_YELLOW, &t, ""), Decision::Take(Some(SystemRequest::QuickSettings)));
        assert_eq!(m.decide(KEY_BLUE, &t, ""), Decision::Take(Some(SystemRequest::OpenSettings)));
        assert_eq!(m.decide(KEY_CHANNELUP, &t, ""), Decision::Take(Some(SystemRequest::OpenNotifications)));
        assert_eq!(m.decide(KEY_CHANNELDOWN, &t, ""), Decision::Take(Some(SystemRequest::PictureOff)));
        assert_eq!(m.decide(KEY_LIST, &t, ""), Decision::Replace(KEY_HOME));
        assert_eq!(m.decide(KEY_MORE, &t, ""), Decision::Replace(KEY_MUTE));
        assert_eq!(m.decide(78, &t, ""), Decision::Replace(KEY_BLUE));
        assert_eq!(m.decide(77, &t, ""), Decision::Replace(KEY_RED), "a code the list lacks, as an import leaves it");
    }

    #[test]
    fn the_app_buttons_are_found_by_the_key_table_and_acting_as_one_needs_its_code() {
        let (t, _) = KeyTable::parse("disneyplus 1041\n");
        let m = map(json!({"disneyplus": "app:plex", "netflix": "as:disneyplus", "amazon": "as:lgchannels", "red": "as:disneyplus"}));
        assert_eq!(m.decide(1041, &t, ""), Decision::Take(Some(SystemRequest::LaunchApp("plex".into()))), "the taught code");
        assert_eq!(m.decide(KEY_NETFLIX, &t, ""), Decision::Replace(1041));
        assert_eq!(m.decide(KEY_AMAZON, &t, ""), Decision::Take(None), "LG Channels has no code here yet: nothing to act as");
        assert_eq!(m.decide(KEY_RED, &KeyTable::default(), ""), Decision::Take(None));
        assert!(m.knows(1041, &t) && m.knows(KEY_NETFLIX, &t) && !m.knows(KEY_BLUE, &t));
        assert!(ButtonMap::settable(KEY_NETFLIX, &t) && ButtonMap::settable(KEY_RED, &t) && !ButtonMap::settable(KEY_VOLUMEUP, &t));
    }

    #[test]
    fn a_code_entry_reaches_a_button_the_table_names_under_another_code() {
        let (t, _) = KeyTable::parse("disneyplus 1041\n");
        let m = map(json!({"code:1042": "nothing", "code:1037": "home"}));
        assert_eq!(m.decide(1042, &t, ""), Decision::Take(None), "no table name for 1042: the code is the button");
        assert_eq!(m.decide(KEY_NETFLIX, &t, ""), Decision::Replace(KEY_HOME), "and Netflix is reached by its code as well");
        assert_eq!(m.decide(1041, &t, ""), Decision::Pass);
    }

    #[test]
    fn home_and_the_pointers_reports_are_never_remapped() {
        let t = KeyTable::default();
        let m = map(json!({"code:102": "nothing", "code:773": "nothing", "code:1199": "nothing", "red": "as:code:773"}));
        assert!(m.chosen().is_empty(), "no entry for them, and none that acts as Home by its code: {:?}", m.chosen());
        for c in [KEY_HOME, KEY_HOMEPAGE, KEY_RF_HOME, KEY_POINTER_ALIVE, KEY_CURSOR_OFF] {
            assert_eq!(m.decide(c, &t, ""), Decision::Pass);
        }
        assert_eq!(m.decide(KEY_RED, &t, ""), Decision::Pass);
    }

    #[test]
    fn power_is_never_remapped_taken_or_acted_as() {
        let t = KeyTable::default();
        let m = map(json!({"code:116": "nothing", "red": "as:code:116", "green": "home"}));
        assert_eq!(m.chosen(), vec![("green", "home".to_string())], "no entry for Power, and none that acts as it by its code");
        assert_eq!(m.decide(KEY_POWER, &t, ""), Decision::Pass);
        assert_eq!(m.decide(KEY_RED, &t, ""), Decision::Pass);
        assert!(!m.knows(KEY_POWER, &t));
    }
}
