//! What the policy asks of LG's services: SAM's running apps, the media pipelines that make an app perceptible, SAM's close and a
//! toast.

use serde_json::{Value, json};
use std::collections::BTreeSet;

const SAM: &str = "luna://com.webos.applicationManager/";
pub const FRONT_URI: &str = "luna://com.webos.applicationManager/getForegroundAppInfo";
pub const FRONT_PAYLOAD: &str = r#"{"extraInfo":true,"subscribe":true}"#;
pub const LIFE_URI: &str = "luna://com.webos.applicationManager/getAppLifeStatus";
pub const LIFE_PAYLOAD: &str = r#"{"subscribe":true}"#;

pub trait Lg: Send + Sync {
    /// SAM's running apps; `None` when SAM did not answer.
    fn running(&self) -> Option<Vec<String>>;
    /// Apps holding an audio decoder or a sound output: playing, or paused with their media loaded.
    fn playing(&self) -> Option<BTreeSet<String>>;
    fn close(&self, app: &str) -> bool;
    fn title(&self, app: &str) -> Option<String>;
    fn toast(&self, message: &str);
}

pub struct Luna {
    send: String,
}

impl Luna {
    pub fn new(send: &str) -> Self {
        Self { send: send.to_owned() }
    }

    fn call(&self, method: &str, payload: &Value) -> Option<Value> {
        crate::luna::once(&self.send, &format!("{SAM}{method}"), &payload.to_string())
    }
}

impl Lg for Luna {
    fn running(&self) -> Option<Vec<String>> {
        running(&self.call("running", &json!({}))?)
    }

    fn playing(&self) -> Option<BTreeSet<String>> {
        let reply = crate::luna::once(&self.send, "luna://com.webos.media/getActivePipelines", "{}")?;
        Some(playing(&reply))
    }

    fn close(&self, app: &str) -> bool {
        self.call("closeByAppId", &json!({"id": app, "reason": "memoryReclaim"}))
            .is_some_and(|r| r.get("returnValue").and_then(Value::as_bool) == Some(true))
    }

    fn title(&self, app: &str) -> Option<String> {
        let reply = self.call("getAppInfo", &json!({"id": app}))?;
        reply.pointer("/appInfo/title").and_then(Value::as_str).map(str::to_owned)
    }

    fn toast(&self, message: &str) {
        let payload = json!({"sourceId": "org.nphil.lumatv", "message": message});
        let _ = crate::luna::once(&self.send, "luna://com.webos.notification/createToast", &payload.to_string());
    }
}

/// SAM's `running`: `{running: [{id, processid, ...}]}`.
pub fn running(reply: &Value) -> Option<Vec<String>> {
    let list = reply.get("running")?.as_array()?;
    Some(
        list.iter()
            .filter_map(|a| a.get("id")?.as_str())
            .filter(|id| valid_id(id))
            .map(str::to_owned)
            .collect(),
    )
}

/// The media server's `getActivePipelines`: a list of `{currentAppId, resource: [{resource}]}`.
pub fn playing(reply: &Value) -> BTreeSet<String> {
    let audible = |p: &Value| {
        p.get("resource").and_then(Value::as_array).is_some_and(|rs| {
            rs.iter().filter_map(|r| r.get("resource")?.as_str()).any(|r| r == "ADEC" || r.ends_with("_SOUND"))
        })
    };
    reply
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| audible(p))
        .filter_map(|p| p.get("currentAppId")?.as_str())
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `getForegroundAppInfo` with `extraInfo`: the apps on screen, the primary (or first by `order`) first. The flat answer of an older
/// SAM is one app.
pub fn front(reply: &Value) -> Option<Vec<String>> {
    if reply.get("returnValue").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let Some(list) = reply.get("foregroundAppInfo").and_then(Value::as_array) else {
        let id = reply.get("appId")?.as_str()?;
        return Some(if id.is_empty() { vec![] } else { vec![id.to_owned()] });
    };
    let mut entries: Vec<(bool, i64, &str)> = list
        .iter()
        .filter_map(|e| {
            let id = e.get("appId")?.as_str().filter(|id| !id.is_empty())?;
            let primary = e.get("primary").and_then(Value::as_bool) == Some(true);
            Some((!primary, e.get("order").and_then(Value::as_i64).unwrap_or(i64::MAX), id))
        })
        .collect();
    entries.sort();
    let mut ids: Vec<String> = Vec::new();
    for (_, _, id) in entries {
        if !ids.iter().any(|i| i == id) {
            ids.push(id.to_owned());
        }
    }
    Some(ids)
}

/// One `getAppLifeStatus` reply: the app and its status (`launching`, `relaunching`, `foreground`, `background`, `closing`, `stop`...).
pub fn life(reply: &Value) -> Option<(String, String)> {
    let app = reply.get("appId")?.as_str().filter(|id| valid_id(id))?;
    let status = reply.get("status")?.as_str()?;
    Some((app.to_owned(), status.to_owned()))
}

/// An app id is used as a directory name under the cgroup roots.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('.')
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sams_and_the_media_servers_answers_read_as_the_tv_gives_them() {
        let r: Value = serde_json::from_str(r#"{"running":[{"webprocessid":"","id":"com.webos.app.home","processid":"15229","appType":"native"},{"id":"com.beb.plxnative","processid":"32428"},{"id":"../x"}],"returnValue":true}"#).unwrap();
        assert_eq!(running(&r).unwrap(), vec!["com.webos.app.home", "com.beb.plxnative"]);
        let p: Value = serde_json::from_str(r#"[{"currentAppId":"netflix","type":"media","resource":[{"index":1,"resource":"ADEC"},{"index":3,"resource":"VDEC"}]},
            {"currentAppId":"tv.twitch","type":"avconnector","resource":[{"index":0,"resource":"MAIN_SOUND"}]},
            {"currentAppId":"youtube.leanback.v4","type":"media","resource":[{"index":0,"resource":"VDEC"}]},
            {"type":"vtclient","resource":[]}]"#).unwrap();
        assert_eq!(playing(&p), BTreeSet::from(["netflix".to_owned(), "tv.twitch".to_owned()]));
    }

    #[test]
    fn the_primary_app_comes_first_then_the_others_by_order() {
        let r: Value = serde_json::from_str(r#"{"returnValue":true,"foregroundAppInfo":[{"order":1,"appId":"b"},{"order":0,"appId":"a"},{"order":2,"appId":"c","primary":true}]}"#).unwrap();
        assert_eq!(front(&r).unwrap(), vec!["c", "a", "b"]);
        assert_eq!(front(&serde_json::json!({"returnValue":true,"foregroundAppInfo":[]})).unwrap(), Vec::<String>::new());
        assert_eq!(front(&serde_json::json!({"appId":"netflix"})).unwrap(), vec!["netflix"]);
        assert_eq!(front(&serde_json::json!({"returnValue":false})), None);
    }
}
