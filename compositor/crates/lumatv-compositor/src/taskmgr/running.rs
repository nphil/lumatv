//! What the app manager says is running (`luna://com.webos.applicationManager/running`), and which of its life events change that.
//!
//! The windows the compositor holds are not the list of apps: a client releases its buffer when it leaves the front, and an app with
//! nothing mapped is still running. The app manager's list is, and it carries the pids the task manager needs to tell an app that is
//! stopped from one that is busy.

use serde_json::Value;

use super::procs::parse_pid;

pub const URI: &str = "luna://com.webos.applicationManager/running";

/// One running app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningApp {
    pub id: String,
    /// Its processes: `processid`, and `webprocessid` when that is another process (a web app's is the same one).
    pub pids: Vec<i32>,
}

/// The apps with a card in a `running` reply, in the app manager's order; `None` when the reply is not a success. An app whose default
/// window is not a card (an overlay, a popup) is not one the owner switches between, and an entry that says nothing of its window is taken
/// for a card, as the app manager does.
pub fn parse(payload: &Value) -> Option<Vec<RunningApp>> {
    if payload.get("returnValue").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let apps = payload.get("running")?.as_array()?;
    Some(
        apps.iter()
            .filter(|a| a.get("defaultWindowType").and_then(Value::as_str).is_none_or(|t| t == "card"))
            .filter_map(|a| {
                let id = a.get("id").and_then(Value::as_str).filter(|id| !id.is_empty())?;
                let mut pids: Vec<i32> = Vec::new();
                for key in ["processid", "webprocessid"] {
                    if let Some(pid) = a.get(key).and_then(parse_pid).filter(|p| !pids.contains(p)) {
                        pids.push(pid);
                    }
                }
                Some(RunningApp { id: id.to_string(), pids })
            })
            .collect(),
    )
}

/// Whether a life event (`getAppLifeEvents`) can have changed who is running: an app launched or ended. (`foreground`, `background`, `pause`
/// and `splash` change nothing about it.)
pub fn changes_the_list(event: &Value) -> bool {
    matches!(event.get("event").and_then(Value::as_str), Some("launch" | "relaunch" | "close" | "stop"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_apps_of_a_reply_keep_their_order_and_their_pids() {
        let reply = json!({"returnValue": true, "running": [
            {"id": "netflix", "processid": "3754", "webprocessid": "", "defaultWindowType": "card", "appType": "native_mvpd"},
            {"id": "com.webos.app.hdmi2", "processid": "2513", "webprocessid": "", "defaultWindowType": "card", "appType": "native"},
            {"id": "com.webos.app.home", "processid": "3169", "webprocessid": "3169", "defaultWindowType": "card", "appType": "web"}]});
        let apps = parse(&reply).unwrap();
        assert_eq!(apps.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["netflix", "com.webos.app.hdmi2", "com.webos.app.home"]);
        assert_eq!(apps[0].pids, [3754]);
        assert_eq!(apps[2].pids, [3169], "a web app's two ids are one process");
    }

    #[test]
    fn an_overlay_is_not_listed_and_an_entry_without_a_window_type_is_a_card() {
        let reply = json!({"returnValue": true, "running": [
            {"id": "com.webos.app.volume", "processid": "10", "defaultWindowType": "overlay"},
            {"id": "com.example.plain", "processid": "11"},
            {"id": "", "processid": "12"}]});
        let apps = parse(&reply).unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].id, "com.example.plain");
    }

    #[test]
    fn a_failed_or_malformed_reply_is_no_answer() {
        assert_eq!(parse(&json!({"returnValue": false, "errorText": "denied"})), None);
        assert_eq!(parse(&json!({"returnValue": true})), None);
        assert_eq!(parse(&json!({"running": []})), None);
        assert_eq!(parse(&json!({"returnValue": true, "running": []})), Some(Vec::new()));
    }

    #[test]
    fn only_a_launch_or_an_end_changes_the_list() {
        for (event, changes) in [("launch", true), ("relaunch", true), ("close", true), ("stop", true), ("foreground", false), ("background", false), ("pause", false), ("splash", false), ("preload", false)] {
            assert_eq!(changes_the_list(&json!({"event": event, "appId": "x"})), changes, "{event}");
        }
        assert!(!changes_the_list(&json!({"appId": "x"})));
    }
}
