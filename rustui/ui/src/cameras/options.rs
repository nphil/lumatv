//! The per-camera pop-up options page: a switch for each kind that may pop up for one camera (`docs/mocks/cameras-scrypted/settings-notify-cam`,
//! `cameras-ui-spec.md` "Notify for"). It is the page of the helper's schema (`scrypted-cameras`, the `notify.<id>` field of kind
//! `ordered-set` over the provider `kinds`) in the form a person uses it: Person, Vehicle, Animal, Package, Doorbell, Motion, each on or
//! off, defaults person and doorbell. The Cameras app opens it on a tile (hold OK) and Settings > Cameras opens the same page from its
//! "Notify for" tab, both through this model, so the two cannot say different things: a switch is a write of the field's whole list
//! through `POST /v1/settings/scrypted-cameras`, in the order the owner picked, and the page is rebuilt when the helper's answer comes.

use super::data::{Settings, default_kinds};
use super::view::{OptionRow, OptionsView};
use super::words;
use serde_json::{Value, json};

/// The settings owner of the cameras (`scrypted/options.rs` `OWNER`).
pub const OWNER: &str = "scrypted-cameras";
/// The page's group label.
pub const GROUP: &str = "Pop up when it sees";
/// The back chip's word.
pub const BACK: &str = "Cameras";

/// The field key of a camera's pick, when its id is one a field key may be (lowercase letters, digits and `._-`, at most 58
/// characters): the helper's own rule (`scrypted/options.rs` `notify_key`), so a camera without a field has no page.
pub fn field_key(camera: &str) -> Option<String> {
    let ok = !camera.is_empty()
        && camera.len() <= 58
        && camera.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
        && !camera.starts_with(['-', '.', '_']);
    ok.then(|| format!("notify.{camera}"))
}

/// The path the page is read from and written to.
pub fn settings_path() -> String {
    format!("/v1/settings/{OWNER}")
}

/// The body that writes `kinds` as the pick of `camera` (the settings transaction of `POST /v1/settings/{owner}`), against the version the
/// page was read at.
pub fn write_body(camera: &str, kinds: &[String], if_version: Option<u64>) -> Option<Value> {
    let key = field_key(camera)?;
    let mut body = json!({"changes": [{"key": key, "value": {"type": "ordered", "value": kinds}}]});
    if let Some(v) = if_version.filter(|v| *v > 0) {
        body["ifVersion"] = json!(v);
    }
    Some(body)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub camera: String,
    pub name: String,
    /// The kinds that pop up, in the order they were picked.
    pub kinds: Vec<String>,
    pub focus: usize,
    version: Option<u64>,
}

impl Options {
    /// The page of `camera` over what the settings say now; `None` for a camera that has no field.
    pub fn new(camera: &str, name: &str, settings: &Settings) -> Option<Self> {
        field_key(camera)?;
        Some(Self { camera: camera.to_string(), name: name.to_string(), kinds: settings.kinds_for(camera), focus: 0, version: Some(settings.version) })
    }

    /// The rows: a switch for each of the six kinds.
    pub fn rows(&self) -> Vec<OptionRow> {
        words::KINDS.iter().map(|k| OptionRow { kind: k, icon: words::kind_icon(k), name: words::kind_name(k), on: self.kinds.iter().any(|x| x == k) }).collect()
    }

    pub fn view(&self) -> OptionsView {
        OptionsView { title: self.name.clone(), group: GROUP, back: BACK, rows: self.rows(), focus: self.focus }
    }

    pub fn step(&mut self, down: bool) {
        let last = words::KINDS.len() - 1;
        self.focus = if down { (self.focus + 1).min(last) } else { self.focus.saturating_sub(1) };
    }

    pub fn focus_row(&mut self, row: usize) {
        self.focus = row.min(words::KINDS.len() - 1);
    }

    /// Flips the switch of row `row`: the kind is taken out of the pick or put at its end. Returns the write that says so.
    pub fn toggle(&mut self, row: usize) -> Option<Value> {
        let kind = words::KINDS.get(row)?;
        if let Some(i) = self.kinds.iter().position(|k| k == kind) {
            self.kinds.remove(i);
        } else {
            self.kinds.push((*kind).to_string());
        }
        write_body(&self.camera, &self.kinds, self.version)
    }

    /// The helper's answer to a write (or a read): the page shows what the owner's values are now, and the version to write against.
    pub fn answered(&mut self, settings: &Settings) {
        self.kinds = settings.kinds_for(&self.camera);
        self.version = Some(settings.version);
    }

    /// The defaults a camera that was never picked for starts from (a person and the doorbell), for a caller that has no settings yet.
    pub fn defaults() -> Vec<String> {
        default_kinds()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn settings(kinds: &[(&str, &[&str])], version: u64) -> Settings {
        let mut s = Settings { version, ..Settings::default() };
        for (cam, list) in kinds {
            s.notify.insert((*cam).to_string(), list.iter().map(|k| (*k).to_string()).collect());
        }
        s
    }

    #[test]
    fn a_field_key_is_the_helpers_rule() {
        assert_eq!(field_key("273").as_deref(), Some("notify.273"));
        assert_eq!(field_key("ok-1").as_deref(), Some("notify.ok-1"));
        assert_eq!(field_key("Camera One"), None);
        assert_eq!(field_key(""), None);
        assert_eq!(field_key("-a"), None);
        assert_eq!(field_key(&"a".repeat(60)), None);
    }

    #[test]
    fn the_page_has_a_switch_for_each_kind_with_the_defaults_on_for_a_camera_never_picked_for() {
        let o = Options::new("273", "Front Door", &Settings::default()).unwrap();
        let on: Vec<(&str, bool)> = o.rows().iter().map(|r| (r.name, r.on)).collect();
        assert_eq!(on, [("Person", true), ("Vehicle", false), ("Animal", false), ("Package", false), ("Doorbell", true), ("Motion", false)]);
        assert_eq!(o.rows()[0].icon, "user");
        assert_eq!(o.rows()[4].icon, "bell-ring");
        assert_eq!(o.view().title, "Front Door");
        assert_eq!((o.view().group, o.view().back), ("Pop up when it sees", "Cameras"));
        assert!(Options::new("Camera One", "x", &Settings::default()).is_none(), "a camera that has no field has no page");
    }

    #[test]
    fn a_switch_writes_the_whole_pick_in_the_order_it_was_made_against_the_version_read() {
        let mut o = Options::new("273", "Front Door", &settings(&[("273", &["person", "doorbell"])], 4)).unwrap();
        let body = o.toggle(2).expect("a write");
        assert_eq!(body, json!({"changes": [{"key": "notify.273", "value": {"type": "ordered", "value": ["person", "doorbell", "animal"]}}], "ifVersion": 4}));
        let body = o.toggle(0).expect("a write");
        assert_eq!(body["changes"][0]["value"]["value"], json!(["doorbell", "animal"]), "off takes it out and keeps the order of the rest");
        o.toggle(1);
        assert_eq!(o.kinds, ["doorbell", "animal", "vehicle"], "on puts it at the end");
    }

    #[test]
    fn the_answer_rebuilds_the_page_and_moves_the_version_on() {
        let mut o = Options::new("273", "Front Door", &Settings::default()).unwrap();
        o.toggle(1);
        o.answered(&settings(&[("273", &["motion"])], 9));
        assert_eq!(o.kinds, ["motion"]);
        let body = o.toggle(5).expect("a write");
        assert_eq!(body["ifVersion"], 9);
        assert_eq!(body["changes"][0]["value"]["value"], json!([]), "everything off is a pick");
    }

    #[test]
    fn the_focus_walks_the_six_rows_and_stops_at_the_ends() {
        let mut o = Options::new("273", "A", &Settings::default()).unwrap();
        o.step(false);
        assert_eq!(o.focus, 0);
        for _ in 0..9 {
            o.step(true);
        }
        assert_eq!(o.focus, 5);
        o.focus_row(99);
        assert_eq!(o.focus, 5);
        assert!(o.toggle(9).is_none(), "no such row");
    }

    #[test]
    fn the_settings_path_names_the_owner() {
        assert_eq!(settings_path(), "/v1/settings/scrypted-cameras");
        assert_eq!(Options::defaults(), ["person", "doorbell"]);
    }
}
