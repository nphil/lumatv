//! What the TV can launch (`kit/Apps.qml` of the QML shell): the app manager's launch points (`applicationmanager/listLaunchPoints`, the
//! list Home's dock starts from) reduced to the apps a person opens, with the owner's names laid over them (`theme.apps.names`, id to
//! the name he gave it). A launch point is not always an app: the HDMI inputs are launch points of another type and belong to the input
//! picker; LG keeps its own system apps off Home with `hidden`, and the list follows it except for Live TV. LG's store is listed as
//! "LG Content Store" so installs stay reachable. The switcher reads the icon and name of an open app here.
//!
//! A push that carries a change (`change`) and not the list asks for the list again.

use crate::kit::screen::{Effect, Input};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const URI: &str = "luna://com.webos.service.applicationmanager/listLaunchPoints";
pub const STORE: &str = "com.webos.app.discovery";
const KEEP_SYSTEM: [&str; 1] = ["com.webos.app.livetv"];

#[derive(Clone, Debug, PartialEq)]
pub struct App {
    pub id: String,
    pub launch_point_id: String,
    pub title: String,
    /// The icon's file path ("" for none).
    pub icon: String,
    pub system: bool,
}

#[derive(Clone, Debug)]
pub struct Apps {
    /// The tag of the subscription; the re-ask uses the next.
    tag: u64,
    points: Vec<Value>,
    names: BTreeMap<String, String>,
    list: Vec<App>,
}

/// A launch point's icon as a file path: the largest it has, a `file://` prefix dropped, a URL of another scheme left to the caller as "".
pub fn icon_of(p: &Value) -> String {
    for k in ["extraLargeIcon", "largeIcon", "icon", "mediumLargeIcon"] {
        let u = p.get(k).and_then(Value::as_str).unwrap_or("");
        if u.is_empty() {
            continue;
        }
        return match u.strip_prefix("file://") {
            Some(path) => path.to_string(),
            None if u.starts_with('/') => u.to_string(),
            None => String::new(),
        };
    }
    String::new()
}

impl Apps {
    pub fn new(tag: u64) -> Self {
        Self { tag, points: Vec::new(), names: BTreeMap::new(), list: Vec::new() }
    }

    pub fn subscribe(&self) -> Vec<Effect> {
        vec![Effect::Luna { tag: self.tag, uri: URI.into(), payload: json!({"subscribe": true}), subscribe: true }]
    }

    /// The owner's names (`theme.apps.names`).
    pub fn set_names(&mut self, theme: &Value) {
        self.names = theme
            .pointer("/apps/names")
            .and_then(Value::as_object)
            .map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
            .unwrap_or_default();
        self.rebuild();
    }

    fn rebuild(&mut self) {
        let mut out: Vec<App> = Vec::new();
        for p in &self.points {
            let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let id = s("id");
            let lptype = p.get("lptype").and_then(Value::as_str).unwrap_or("default");
            if lptype != "default" || id.is_empty() || out.iter().any(|a| a.id == id) {
                continue;
            }
            let system = p.get("systemApp").and_then(Value::as_bool) == Some(true);
            if system && p.get("hidden").and_then(Value::as_bool) == Some(true) && !KEEP_SYSTEM.contains(&id.as_str()) {
                continue;
            }
            let title = match self.names.get(&id) {
                Some(n) if !n.is_empty() => n.clone(),
                _ if id == STORE => "LG Content Store".to_string(),
                _ => {
                    let t = s("title");
                    if t.is_empty() { id.clone() } else { t }
                }
            };
            let lp = s("launchPointId");
            out.push(App { launch_point_id: if lp.is_empty() { format!("{id}_default") } else { lp }, icon: icon_of(p), title, id, system });
        }
        self.list = out;
    }

    pub fn list(&self) -> &[App] {
        &self.list
    }

    /// The launch points as the app manager gave them.
    pub fn points(&self) -> &[Value] {
        &self.points
    }

    /// Every app the TV can launch as (id, title), hidden system apps included (LG Channels is one: the list has neither it nor its
    /// like); the inputs are launch points of another type (`AppKeys.installed`).
    pub fn launchable(&self) -> Vec<(String, String)> {
        let s = |p: &Value, k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        self.points
            .iter()
            .filter(|p| p.get("lptype").and_then(Value::as_str).unwrap_or("default") == "default" && !s(p, "id").is_empty())
            .map(|p| (s(p, "id"), Some(s(p, "title")).filter(|t| !t.is_empty()).unwrap_or_else(|| s(p, "id"))))
            .collect()
    }

    pub fn by_id(&self, id: &str) -> Option<&App> {
        self.list.iter().find(|a| a.id == id)
    }

    /// An answer or a push of the subscription. Returns the effects it asks for (the list again after a change) and whether the list
    /// changed.
    pub fn update(&mut self, input: &Input) -> (Vec<Effect>, bool) {
        let Input::Luna { tag, payload } = input else { return (Vec::new(), false) };
        if *tag != self.tag && *tag != self.tag + 1 {
            return (Vec::new(), false);
        }
        if let Some(points) = payload.get("launchPoints").and_then(Value::as_array) {
            let before = self.list.clone();
            self.points = points.clone();
            self.rebuild();
            return (Vec::new(), before != self.list);
        }
        if payload.get("change").is_some() {
            return (vec![Effect::Luna { tag: self.tag + 1, uri: URI.into(), payload: json!({}), subscribe: false }], false);
        }
        (Vec::new(), false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points() -> Value {
        json!({"launchPoints": [
            {"id": "netflix", "title": "Netflix", "icon": "/media/netflix.png", "largeIcon": "/media/netflix-l.png", "lptype": "default"},
            {"id": "com.webos.app.hdmi1", "title": "HDMI 1", "lptype": "bookmark"},
            {"id": "com.webos.app.gallery", "title": "Gallery+", "systemApp": true, "hidden": true},
            {"id": "com.webos.app.livetv", "title": "Live TV", "systemApp": true, "hidden": true, "icon": "file:///usr/livetv.png"},
            {"id": "com.webos.app.discovery", "title": "Content Store", "icon": "http://x/y.png"},
            {"id": "netflix", "title": "Netflix again"}
        ]})
    }

    #[test]
    fn the_list_is_what_a_person_opens_without_inputs_or_the_hidden_system_apps_and_without_repeats() {
        let mut a = Apps::new(300);
        let (_, changed) = a.update(&Input::Luna { tag: 300, payload: points() });
        assert!(changed);
        assert_eq!(a.list().iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["netflix", "com.webos.app.livetv", "com.webos.app.discovery"]);
        assert_eq!(a.by_id("netflix").map(|x| x.icon.as_str()), Some("/media/netflix-l.png"), "the largest icon");
        assert_eq!(a.by_id("com.webos.app.livetv").map(|x| x.icon.as_str()), Some("/usr/livetv.png"), "a file URL is a path");
        assert_eq!(
            a.by_id("com.webos.app.discovery").map(|x| (x.title.as_str(), x.icon.as_str())),
            Some(("LG Content Store", "")),
            "an icon at another URL is none"
        );
        assert_eq!(a.by_id("netflix").map(|x| x.launch_point_id.as_str()), Some("netflix_default"));
    }

    #[test]
    fn the_owners_names_are_laid_over_the_titles() {
        let mut a = Apps::new(300);
        a.update(&Input::Luna { tag: 300, payload: points() });
        a.set_names(&json!({"apps": {"names": {"netflix": "Nflx"}}}));
        assert_eq!(a.by_id("netflix").map(|x| x.title.as_str()), Some("Nflx"));
    }

    #[test]
    fn a_push_that_is_a_change_asks_for_the_list_again_and_an_equal_list_is_no_change() {
        let mut a = Apps::new(300);
        a.update(&Input::Luna { tag: 300, payload: points() });
        let (e, changed) = a.update(&Input::Luna { tag: 300, payload: json!({"change": "removed", "id": "netflix"}) });
        assert!(!changed);
        assert!(matches!(e.as_slice(), [Effect::Luna { tag: 301, subscribe: false, .. }]));
        let (_, changed) = a.update(&Input::Luna { tag: 301, payload: points() });
        assert!(!changed, "the same list again");
        let (_, changed) = a.update(&Input::Luna { tag: 9, payload: points() });
        assert!(!changed, "another tag is not ours");
        assert!(matches!(a.subscribe().as_slice(), [Effect::Luna { tag: 300, subscribe: true, .. }]));
    }
}
