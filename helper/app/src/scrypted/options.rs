//! The Cameras page of Settings (design 3.5): a first-party plugin of the helper, `scrypted-cameras`, whose settings page the TV's Settings
//! draws with the rows it draws any plugin's page with (a switch, a number, a choice, an ordered pick, a read-only line, a button).
//!
//! The page is rebuilt, at a higher `revision`, when the camera list changes: the *Notify for* section has one pick of events for each camera
//! the TV shows (`notify.<id>`), and what the owner picked stays in the settings store when a camera leaves and comes back. A secret or an
//! address is never typed here: the link is made on the TV with `scrypted-link`, and the page can only forget it.
//!
//! Defaults: every camera notifies for a person and a doorbell press and nothing else (Nitin's cameras see birds and cats all day, so
//! `animal` on by default would be a pop-up a minute).

use super::policy::{DEFAULT_KINDS, KINDS, Rules};
use crate::plugins::local::{Handler, Local, Spec};
use crate::plugins::session::Shared;
use parking_lot::Mutex;
use plugin_api::wire::ToPlugin;
use serde_json::{Value, json};
use settings_schema::{SettingsSchema, Value as Setting, Values};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The settings owner and the first-party plugin's id.
pub const OWNER: &str = "scrypted-cameras";
pub const SPEC: Spec = Spec {
    id: OWNER,
    name: "Cameras",
    version: "1",
    permissions: &["settings"],
};
/// The providers the page's pickers are served by.
pub const PROVIDER_CAMERAS: &str = "cameras";
pub const PROVIDER_KINDS: &str = "kinds";
/// The action of the button that forgets the link.
pub const FORGET: &str = "forget";
/// The id the first page's pick of events is for, until the cameras are known.
const PLACEHOLDER: &str = "placeholder";
/// The most cameras the page shows or notifies for (the schema allows 128 fields; the other sections use fewer than ten).
pub const MAX_CAMERAS: usize = 32;

/// What the owner types into the bridge's list of devices: this TV's name and the hash of its token. Neither is a secret (a hash of a random
/// 32-byte token says nothing of the token), which is why the page may show them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub device: String,
    pub verifier: String,
}

/// The words of a kind, for the page's pick.
pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "person" => "Person",
        "vehicle" => "Vehicle",
        "animal" => "Animal",
        "package" => "Package",
        "doorbell" => "Doorbell",
        "motion" => "Motion",
        _ => "Other",
    }
}

/// The key of a camera's notify field, when its id is one a field key may be (lowercase letters, digits and `._-`, at most 58 characters).
pub fn notify_key(camera: &str) -> Option<String> {
    let key = format!("notify.{camera}");
    let ok = key.len() <= 64
        && camera.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        })
        && camera
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && camera
            .bytes()
            .last()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    ok.then_some(key)
}

/// The page, as the schema format writes it. `cameras` are the ones the TV shows (id and name), in the order it shows them.
pub fn schema_json(cameras: &[(String, String)], revision: u32) -> Value {
    let notify: Vec<Value> = cameras
        .iter()
        .take(MAX_CAMERAS)
        .filter_map(|(id, name)| {
            let key = notify_key(id)?;
            Some(json!({
                "key": key,
                "label": if name.is_empty() { id } else { name },
                "kind": {"type": "ordered-set", "options": {"provider": PROVIDER_KINDS}, "max_items": 8},
                "default": {"type": "ordered", "value": DEFAULT_KINDS},
                "apply": "on-commit"
            }))
        })
        .collect();
    let mut sections = vec![
        json!({"id": "link", "title": "Scrypted", "fields": [
            {"key": "link.status", "label": "Connection",
             "kind": {"type": "read-only", "value": {"capability": format!("plugin.{OWNER}.link")}}, "apply": "on-commit"},
            {"key": "link.device", "label": "Device name",
             "kind": {"type": "read-only", "value": {"capability": format!("plugin.{OWNER}.device")}}, "apply": "on-commit"},
            {"key": "link.verifier", "label": "Verifier",
             "kind": {"type": "read-only", "value": {"capability": format!("plugin.{OWNER}.verifier")}}, "apply": "on-commit"},
            {"key": "link.forget", "label": "Forget link",
             "kind": {"type": "action", "action": FORGET, "confirmation": "destructive"}, "apply": "on-commit"}]}),
        json!({"id": "show", "title": "Cameras", "fields": [
            {"key": "show", "label": "Show cameras",
             "help": "Empty shows every camera Scrypted shares, in Scrypted's order.",
             "kind": {"type": "ordered-set", "options": {"provider": PROVIDER_CAMERAS}, "max_items": MAX_CAMERAS},
             "default": {"type": "ordered", "value": []}, "apply": "on-commit"}]}),
        json!({"id": "popups", "title": "Pop-ups", "fields": [
            {"key": "popups.on", "label": "Show event pop-ups", "kind": {"type": "bool"},
             "default": {"type": "bool", "value": true}, "apply": "on-commit"},
            {"key": "popups.seconds", "label": "Stay for",
             "kind": {"type": "integer", "min": 5, "max": 60, "step": 1, "unit": "s"},
             "default": {"type": "integer", "value": 12}, "apply": "on-commit"},
            {"key": "popups.cooldown", "label": "Same event again after",
             "kind": {"type": "integer", "min": 0, "max": 600, "step": 5, "unit": "s"},
             "default": {"type": "integer", "value": 30}, "apply": "on-commit"},
            {"key": "popups.over_apps", "label": "Show pop-ups over playing apps", "kind": {"type": "bool"},
             "default": {"type": "bool", "value": true}, "apply": "on-commit"}]}),
    ];
    if !notify.is_empty() {
        sections.push(json!({"id": "notify", "title": "Notify for", "fields": notify}));
    }
    sections.push(json!({"id": "video", "title": "Video", "fields": [
        {"key": "video.live", "label": "Live quality",
         "kind": {"type": "choice", "options": {"inline": [
            {"id": "native", "label": "Full"}, {"id": "balanced", "label": "Medium"}, {"id": "light", "label": "Low"}]}},
         "default": {"type": "choice", "value": "native"}, "apply": "on-commit"},
        {"key": "video.grid", "label": "Grid",
         "kind": {"type": "choice", "options": {"inline": [
            {"id": "light", "label": "Light"}, {"id": "balanced", "label": "Balanced"}, {"id": "smooth", "label": "Smooth"}]}},
         "default": {"type": "choice", "value": "balanced"}, "apply": "on-commit"}]}));
    json!({"format": 1, "owner": OWNER, "revision": revision, "title": "Cameras", "sections": sections})
}

/// The page as a validated schema (what the settings store registers).
pub fn schema(cameras: &[(String, String)], revision: u32) -> Result<SettingsSchema, String> {
    SettingsSchema::parse(schema_json(cameras, revision).to_string().as_bytes())
        .map_err(|e| e.to_string())
}

/// Pictures a second a grid tile's stream is asked at, for the Grid setting.
pub fn grid_fps(word: &str) -> u32 {
    match word {
        "light" => 3,
        "smooth" => 8,
        _ => 5,
    }
}

/// The owner's choices, as the page holds them now.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub rules: Rules,
    /// The cameras the TV shows, in order; empty is every camera.
    pub show: Vec<String>,
    /// The kinds each camera notifies for, where the owner picked (a camera not here has the defaults).
    pub notify: BTreeMap<String, Vec<String>>,
    /// `native`, `balanced` or `light`.
    pub live: String,
    /// `light`, `balanced` or `smooth`.
    pub grid: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            rules: Rules::default(),
            show: Vec::new(),
            notify: BTreeMap::new(),
            live: "native".into(),
            grid: "balanced".into(),
        }
    }
}

impl Options {
    /// Reads the page's effective values (`Values` hold the stored choices over the page's defaults).
    pub fn read(values: &Values) -> Self {
        let mut out = Self::default();
        let int = |key: &str| match values.get(key) {
            Some(Setting::Integer(n)) => u32::try_from(*n).ok(),
            _ => None,
        };
        let flag = |key: &str| match values.get(key) {
            Some(Setting::Bool(b)) => Some(*b),
            _ => None,
        };
        let word = |key: &str| match values.get(key) {
            Some(Setting::Choice(c)) => Some(c.to_string()),
            _ => None,
        };
        let list = |key: &str| match values.get(key) {
            Some(Setting::Ordered(items)) => {
                Some(items.iter().map(ToString::to_string).collect::<Vec<_>>())
            }
            _ => None,
        };
        out.rules.on = flag("popups.on").unwrap_or(out.rules.on);
        out.rules.seconds = int("popups.seconds").unwrap_or(out.rules.seconds);
        out.rules.cooldown = int("popups.cooldown").unwrap_or(out.rules.cooldown);
        out.rules.over_apps = flag("popups.over_apps").unwrap_or(out.rules.over_apps);
        out.show = list("show").unwrap_or_default();
        out.live = word("video.live").unwrap_or(out.live);
        out.grid = word("video.grid").unwrap_or(out.grid);
        for (key, value) in values.iter() {
            if let Some(camera) = key.as_str().strip_prefix("notify.")
                && let Setting::Ordered(items) = value
            {
                out.notify.insert(
                    camera.to_owned(),
                    items.iter().map(ToString::to_string).collect(),
                );
            }
        }
        out
    }

    /// The kinds a camera notifies for.
    pub fn notify_for(&self, camera: &str) -> Vec<String> {
        self.notify
            .get(camera)
            .cloned()
            .unwrap_or_else(|| DEFAULT_KINDS.iter().map(|k| (*k).to_owned()).collect())
    }

    /// Whether any camera asks for motion (the bridge listens to it only when the push channel says so).
    pub fn wants_motion(&self, cameras: &[String]) -> bool {
        cameras
            .iter()
            .any(|camera| self.notify_for(camera).iter().any(|k| k == "motion"))
    }
}

// ---- the plugin ---------------------------------------------------------------------------------------------------------------------

/// What the page's handlers act on: the cameras the pickers list, and what the forget button does.
pub struct PageHooks {
    /// Every camera Scrypted shares with the TV (id, name, room), whether the TV shows it or not: what the *Show cameras* pick offers.
    pub all: Mutex<Vec<(String, String, Option<String>)>>,
    /// Whether the TV is linked (the pickers say "not linked" otherwise).
    pub linked: Mutex<bool>,
    pub forget: Box<dyn Fn() + Send + Sync>,
}

/// The first-party plugin: registered once, its page replaced when the cameras change.
pub struct Page {
    local: Arc<Local>,
    hooks: Arc<PageHooks>,
    revision: Mutex<(u32, Vec<(String, String)>)>,
}

fn page_json(state: &str, rows: Vec<Value>, offset: u32, more: bool) -> Value {
    json!({"offset": offset, "rows": rows, "more": more, "state": state})
}

impl Page {
    pub fn start(shared: &Arc<Shared>, hooks: Arc<PageHooks>) -> Result<Arc<Self>, String> {
        // The page names its providers when it is registered, and a provider named only by a field that is not there yet is not served: the first
        // page has a pick of events for one camera that is not (the placeholder), which `show` replaces before anybody reads it.
        let first = [(PLACEHOLDER.to_owned(), String::new())];
        let register = json!({
            "title": "Cameras",
            "settings": schema_json(&first, 1),
            "actions": [{
                "key": FORGET, "label": "Forget link", "confirmation": "destructive",
                "confirm_text": {"title": "Forget Scrypted?", "confirm_label": "Forget",
                                 "body": "The TV forgets its token for Scrypted and shows no cameras until it is linked again. Nothing is changed in Scrypted."}
            }],
        });
        let handler: Handler = {
            let hooks = hooks.clone();
            Arc::new(move |local: &Local, message: ToPlugin| match message {
                ToPlugin::ChoicesQuery {
                    query,
                    generation,
                    provider,
                    offset,
                    limit,
                } => {
                    let page = choices(&hooks, &provider, offset, limit);
                    let _ = local.say(json!({"t": "choices.batch", "query": query, "generation": generation, "page": page}));
                }
                ToPlugin::ActionInvoke { op, action, .. } => {
                    if action == FORGET {
                        (hooks.forget)();
                        let _ = local.answer(op, Ok(()));
                    } else {
                        let _ = local.answer(op, Err("no such action"));
                    }
                }
                // the owner's choices are read from the store whenever they matter: nothing here waits on them
                ToPlugin::SettingsApply { txn, values, .. } => {
                    let _ = local.say(json!({"t": "settings.result", "txn": txn, "outcome": {"type": "applied", "values": values}}));
                }
                _ => {}
            })
        };
        let local = Local::start(shared, &SPEC, register, handler)?;
        let page = Arc::new(Self {
            local,
            hooks,
            revision: Mutex::new((1, first.to_vec())),
        });
        page.show(&[]);
        Ok(page)
    }

    pub fn hooks(&self) -> &Arc<PageHooks> {
        &self.hooks
    }

    /// Says the connection's state in words (`plugin.scrypted-cameras.link`) and, while a link file exists, what the owner types into the
    /// bridge's list of devices (`.device` and `.verifier`). One `facts.put` carries all of it: a fact left out is gone.
    pub fn say_link(&self, words: &str, identity: Option<&Identity>) {
        let line: String = words
            .chars()
            .filter(|c| !c.is_control())
            .take(150)
            .collect();
        let mut facts = serde_json::Map::new();
        facts.insert("link".to_owned(), Value::String(line));
        if let Some(identity) = identity {
            facts.insert("device".to_owned(), Value::String(identity.device.clone()));
            facts.insert(
                "verifier".to_owned(),
                Value::String(identity.verifier.clone()),
            );
        }
        let _ = self.local.say(json!({"t": "facts.put", "facts": facts}));
    }

    /// Replaces the page when the cameras it lists are not the ones it lists now.
    pub fn show(&self, cameras: &[(String, String)]) {
        let revision = {
            let mut state = self.revision.lock();
            if state.1 == cameras {
                return;
            }
            state.0 += 1;
            state.1 = cameras.to_vec();
            state.0
        };
        if let Err(e) = self
            .local
            .say(json!({"t": "settings.page", "schema": schema_json(cameras, revision)}))
        {
            crate::log_warn!("cameras: the settings page was not replaced: {e}");
        }
    }
}

/// A page of one picker's rows.
fn choices(hooks: &PageHooks, provider: &str, offset: u32, limit: u16) -> Value {
    let rows: Vec<Value> = match provider {
        PROVIDER_KINDS => KINDS
            .iter()
            .map(|k| json!({"id": k, "label": kind_label(k)}))
            .collect(),
        PROVIDER_CAMERAS => {
            if !*hooks.linked.lock() {
                return page_json("unconfigured", Vec::new(), offset, false);
            }
            hooks
                .all
                .lock()
                .iter()
                .filter(|(id, ..)| notify_key(id).is_some())
                .map(|(id, name, room)| match room {
                    Some(room) => json!({"id": id, "label": name, "detail": room}),
                    None => json!({"id": id, "label": name}),
                })
                .collect()
        }
        _ => return page_json("failed", Vec::new(), offset, false),
    };
    let total = rows.len();
    let from = (offset as usize).min(total);
    let to = (from + usize::from(limit.max(1))).min(total);
    page_json("ready", rows[from..to].to_vec(), offset, to < total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cams(n: usize) -> Vec<(String, String)> {
        (0..n)
            .map(|i| (format!("{}", 270 + i), format!("Camera {i}")))
            .collect()
    }

    #[test]
    fn the_page_is_a_valid_schema_with_a_notify_pick_for_each_camera_and_the_defaults_of_the_design()
     {
        let schema = schema(&cams(3), 4).unwrap();
        assert_eq!(schema.owner.as_str(), OWNER);
        assert_eq!(schema.revision, 4);
        let keys: Vec<String> = schema.fields().map(|f| f.key.to_string()).collect();
        for key in [
            "link.status",
            "link.device",
            "link.verifier",
            "link.forget",
            "show",
            "popups.on",
            "popups.seconds",
            "popups.cooldown",
            "popups.over_apps",
            "notify.270",
            "notify.272",
            "video.live",
            "video.grid",
        ] {
            assert!(keys.contains(&key.to_owned()), "{key}: {keys:?}");
        }
        assert_eq!(keys.iter().filter(|k| k.starts_with("notify.")).count(), 3);
        // the defaults: a person and a doorbell for every camera, 12 s, 30 s, over apps, full quality, a balanced grid
        let defaults = schema.defaults();
        let options = Options::read(&defaults);
        assert_eq!(
            options.rules,
            Rules {
                on: true,
                seconds: 12,
                cooldown: 30,
                over_apps: true
            }
        );
        assert_eq!(options.notify_for("270"), ["person", "doorbell"]);
        assert_eq!(options.notify_for("271"), ["person", "doorbell"]);
        assert_eq!(
            (
                options.live.as_str(),
                options.grid.as_str(),
                options.show.is_empty()
            ),
            ("native", "balanced", true)
        );
        // a page with no cameras has no notify section, and a bridge's odd id has no field
        let none = schema_json(&[], 1);
        assert!(
            !none["sections"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["id"] == "notify")
        );
        let odd = schema_json(
            &[
                ("Camera One".into(), "x".into()),
                ("ok-1".into(), "Y".into()),
            ],
            1,
        );
        let notify = odd["sections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == "notify")
            .unwrap();
        assert_eq!(notify["fields"].as_array().unwrap().len(), 1);
        assert_eq!(notify_key("ok-1").as_deref(), Some("notify.ok-1"));
        assert_eq!(
            (
                notify_key("Camera One"),
                notify_key("-a"),
                notify_key(&"a".repeat(60))
            ),
            (None, None, None)
        );
    }

    #[test]
    fn thirty_two_cameras_fit_the_pages_limits() {
        let page = schema(&cams(40), 1).unwrap();
        assert_eq!(
            page.fields()
                .filter(|f| f.key.as_str().starts_with("notify."))
                .count(),
            MAX_CAMERAS
        );
        assert!(page.fields().count() <= 128);
    }

    #[test]
    fn the_owners_choices_are_read_from_the_stored_values_over_the_defaults() {
        let page = schema(&cams(2), 1).unwrap();
        let mut values = page.defaults();
        let set = |values: &mut Values, key: &str, value: Setting| {
            values.insert(key.parse().unwrap(), value);
        };
        set(&mut values, "popups.on", Setting::Bool(false));
        set(&mut values, "popups.seconds", Setting::Integer(30));
        set(&mut values, "popups.cooldown", Setting::Integer(0));
        set(
            &mut values,
            "show",
            Setting::Ordered(vec!["271".parse().unwrap(), "270".parse().unwrap()]),
        );
        set(
            &mut values,
            "notify.271",
            Setting::Ordered(vec!["animal".parse().unwrap(), "motion".parse().unwrap()]),
        );
        set(
            &mut values,
            "video.live",
            Setting::Choice("light".parse().unwrap()),
        );
        set(
            &mut values,
            "video.grid",
            Setting::Choice("smooth".parse().unwrap()),
        );
        let options = Options::read(&values);
        assert_eq!(
            (
                options.rules.on,
                options.rules.seconds,
                options.rules.cooldown
            ),
            (false, 30, 0)
        );
        assert_eq!(options.show, ["271", "270"]);
        assert_eq!(options.notify_for("271"), ["animal", "motion"]);
        assert_eq!(options.notify_for("270"), ["person", "doorbell"]);
        assert_eq!(
            (options.live.as_str(), options.grid.as_str()),
            ("light", "smooth")
        );
        assert!(options.wants_motion(&["270".into(), "271".into()]));
        assert!(!options.wants_motion(&["270".into()]));
        assert_eq!(
            (
                grid_fps("light"),
                grid_fps("balanced"),
                grid_fps("smooth"),
                grid_fps("x")
            ),
            (3, 5, 8, 5)
        );
    }

    #[test]
    fn the_pickers_list_the_kinds_always_and_the_cameras_only_while_linked_in_pages() {
        let hooks = PageHooks {
            all: Mutex::new(vec![
                ("273".into(), "Front Door".into(), Some("Entry".into())),
                ("274".into(), "Garage".into(), None),
                ("Weird Id".into(), "Skipped".into(), None),
            ]),
            linked: Mutex::new(false),
            forget: Box::new(|| {}),
        };
        let kinds = choices(&hooks, PROVIDER_KINDS, 0, 24);
        assert_eq!(kinds["state"], "ready");
        let ids: Vec<&str> = kinds["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            [
                "person", "vehicle", "animal", "package", "doorbell", "motion"
            ]
        );
        assert_eq!(kinds["rows"][0]["label"], "Person");
        assert_eq!(
            choices(&hooks, PROVIDER_CAMERAS, 0, 24)["state"],
            "unconfigured"
        );
        *hooks.linked.lock() = true;
        let cameras = choices(&hooks, PROVIDER_CAMERAS, 0, 24);
        assert_eq!(cameras["state"], "ready");
        assert_eq!(cameras["rows"].as_array().unwrap().len(), 2);
        assert_eq!(
            (
                cameras["rows"][0]["label"].as_str(),
                cameras["rows"][0]["detail"].as_str()
            ),
            (Some("Front Door"), Some("Entry"))
        );
        let paged = choices(&hooks, PROVIDER_CAMERAS, 1, 1);
        assert_eq!(
            (paged["rows"][0]["id"].as_str(), paged["more"].as_bool()),
            (Some("274"), Some(false))
        );
        assert_eq!(choices(&hooks, PROVIDER_CAMERAS, 0, 1)["more"], true);
        assert_eq!(choices(&hooks, "secrets", 0, 24)["state"], "failed");
    }
}
