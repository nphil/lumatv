//! LG Channels' native switch and its app-specific preload policy.
use crate::ai::luna::LunaTv;
use crate::http::{Request, Response, error_response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Mutex;

const APP: &str = "com.webos.app.lgchannels";
const SETTINGS: &str = "luna://com.webos.settingsservice/";
const PRELOAD: &str = "luna://com.webos.service.preloadmanager/";
const MANAGER: &str = "luna://com.webos.applicationManager/";
pub const ROUTES: &[(&str, &str, &str)] = &[
    ("GET", "/v1/lgchannels", "LG Channels native and background status"),
    ("POST", "/v1/lgchannels", "set LG Channels: {enabled: bool}"),
];

trait Platform: Send + Sync {
    fn call(&self, uri: &str, payload: &Value) -> Result<Value, String>;
}
impl Platform for LunaTv {
    fn call(&self, uri: &str, payload: &Value) -> Result<Value, String> {
        LunaTv::call(self, uri, payload).map_err(|e| e.to_string())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Managed {
    desired: bool,
    original_preload: bool,
    params: Value,
}

pub struct Channels {
    platform: Box<dyn Platform>,
    file: PathBuf,
    lock: Mutex<()>,
}
impl Channels {
    pub(crate) fn with_preload_lock<T>(&self, f: impl FnOnce() -> T) -> T {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        f()
    }
    pub fn new(data: PathBuf, luna: &str) -> Self {
        Self { platform: Box::new(LunaTv::new(luna)), file: data.join("lgchannels.json"), lock: Mutex::new(()) }
    }
    fn call(&self, uri: &str, payload: Value) -> Result<Value, String> {
        let v = self.platform.call(uri, &payload)?;
        if v.get("returnValue").and_then(Value::as_bool) != Some(true) {
            return Err(format!("{uri} did not confirm success"));
        }
        Ok(v)
    }
    fn managed(&self) -> Result<Option<Managed>, String> {
        match std::fs::read(&self.file) {
            Ok(data) => serde_json::from_slice(&data).map(Some).map_err(|e| format!("LG Channels settings: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("LG Channels settings: {e}")),
        }
    }
    fn save(&self, m: &Managed) -> Result<(), String> {
        let data = serde_json::to_vec(m).map_err(|e| e.to_string())?;
        lumatv_feeds::cache::atomic_write(&self.file, &data).map_err(|e| format!("LG Channels settings: {e}"))
    }
    fn native(&self) -> Result<bool, String> {
        let v = self.call(&format!("{SETTINGS}getSystemSettings"), json!({"category":"option","keys":["channelplus"]}))?;
        match v.pointer("/settings/channelplus").and_then(Value::as_str) {
            Some("on") => Ok(true), Some("off") => Ok(false), _ => Err("TV did not report the LG Channels switch".into()),
        }
    }
    fn policy(&self) -> Result<(bool, Value), String> {
        let v = self.call(&format!("{PRELOAD}getPreloadPolicy"), json!({}))?;
        let apps = v.get("applications").and_then(Value::as_array).ok_or("TV did not report preload policies")?;
        let mut found = apps.iter().filter(|a| a.get("id").and_then(Value::as_str) == Some(APP));
        let entry = found.next().ok_or("LG Channels preload policy is unavailable")?;
        if found.next().is_some() { return Err("LG Channels preload policy is ambiguous".into()); }
        let enabled = entry.get("isEnabled").and_then(Value::as_bool).ok_or("LG Channels preload state is unavailable")?;
        let params = entry.get("params").filter(|v| v.is_object()).ok_or("LG Channels preload parameters are unavailable")?.clone();
        Ok((enabled, params))
    }
    fn running(&self) -> Result<bool, String> {
        let v = self.call(&format!("{MANAGER}running"), json!({}))?;
        let apps = v.get("running").and_then(Value::as_array).ok_or("TV did not report running apps")?;
        Ok(apps.iter().any(|a| a.get("id").and_then(Value::as_str) == Some(APP)))
    }
    fn set_policy(&self, enabled: bool, params: &Value) -> Result<(), String> {
        self.call(&format!("{PRELOAD}setPreloadPolicy"), json!({"application":{"id":APP,"isEnabled":enabled,"params":params}}))?;
        if self.policy()?.0 != enabled { return Err("TV did not retain the LG Channels preload policy".into()); }
        Ok(())
    }
    fn set_native(&self, enabled: bool) -> Result<(), String> {
        self.call(&format!("{SETTINGS}setSystemSettings"), json!({"category":"option","settings":{"channelplus":if enabled {"on"} else {"off"}}}))?;
        if self.native()? != enabled { return Err("TV did not retain the LG Channels switch".into()); }
        Ok(())
    }
    fn apply(&self, m: &Managed) -> Result<(), String> {
        if m.desired {
            self.set_native(true)?;
            self.set_policy(m.original_preload, &m.params)?;
        } else {
            self.set_policy(false, &m.params)?;
            self.set_native(false)?;
            if self.running()? {
                self.call(&format!("{MANAGER}closeByAppId"), json!({"id":APP,"reason":"windowPolicy"}))?;
                if self.running()? { return Err("LG Channels is still running after the TV was asked to close it".into()); }
            }
        }
        Ok(())
    }
    fn status(&self, managed: Option<&Managed>, reason: Option<String>) -> Value {
        let native = self.native();
        let policy = self.policy();
        let running = self.running();
        let desired = managed.map(|m| m.desired).or_else(|| native.as_ref().ok().copied());
        let issue = reason.or_else(|| native.as_ref().err().cloned()).or_else(|| policy.as_ref().err().cloned()).or_else(|| running.as_ref().err().cloned());
        let state = if issue.is_some() { if native.is_err() || policy.is_err() || running.is_err() { "unavailable" } else { "partial" } }
        else if let Some(m) = managed {
            let expected = if m.desired { m.original_preload } else { false };
            if native.as_ref().ok() != Some(&m.desired) || policy.as_ref().ok().map(|p| p.0) != Some(expected) || (!m.desired && running.as_ref().ok() == Some(&true)) { "partial" } else if m.desired { "enabled" } else { "disabled" }
        } else if native.as_ref().ok() == Some(&true) { "enabled" }
        else if policy.as_ref().ok().map(|p| p.0) == Some(false) && running.as_ref().ok() == Some(&false) { "disabled" }
        else { "partial" };
        let reason = issue.or_else(|| (state == "partial").then(|| "The TV has not applied all LG Channels changes".to_owned())).unwrap_or_default();
        json!({"ok":true,"enabled":native.ok(),"desired":desired,"state":state,"reason":reason,"preloadEnabled":policy.ok().map(|p| p.0),"running":running.ok()})
    }
    pub fn reconcile(&self) {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        match self.managed() {
            Ok(Some(m)) => if let Err(e) = self.apply(&m) { crate::log_warn!("LG Channels: {e}"); },
            Ok(None) => (),
            Err(e) => crate::log_warn!("LG Channels: {e}"),
        }
    }
    fn get(&self) -> Value {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        match self.managed() { Ok(m) => self.status(m.as_ref(), None), Err(e) => self.status(None, Some(e)) }
    }
    fn change(&self, enabled: bool) -> Result<Value, String> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut m = match self.managed()? {
            Some(m) => m,
            None => { let (original_preload, params) = self.policy()?; Managed { desired: enabled, original_preload, params } }
        };
        m.desired = enabled;
        self.save(&m)?;
        let result = self.apply(&m);
        Ok(self.status(Some(&m), result.err()))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Change { enabled: bool }
pub fn handle(channels: &Channels, request: &Request, can_admin: bool) -> Response {
    match request.method.as_str() {
        "GET" => Response::json(200, &channels.get()),
        "POST" => {
            if !can_admin { return error_response(403, "owner_only", "Only the TV's owner can change LG Channels"); }
            let change: Change = match serde_json::from_slice(&request.body) {
                Ok(v) => v, Err(_) => return error_response(400, "bad_request", "expected {enabled: bool}"),
            };
            match channels.change(change.enabled) {
                Ok(v) => Response::json(200, &v),
                Err(e) => error_response(503, "unavailable", &e),
            }
        }
        _ => error_response(405, "method_not_allowed", "that method is not served here").with_header("Allow", "GET, POST"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct State { native: bool, preload: bool, running: bool, fail_close: bool, writes: Vec<(String, Value)> }
    struct Fake(Mutex<State>);
    impl Platform for Arc<Fake> {
        fn call(&self, uri: &str, payload: &Value) -> Result<Value, String> {
            let mut s = self.0.lock().unwrap();
            Ok(match uri.rsplit('/').next().unwrap() {
                "getSystemSettings" => json!({"returnValue":true,"settings":{"channelplus":if s.native {"on"} else {"off"}}}),
                "getPreloadPolicy" => json!({"returnValue":true,"applications":[{"id":APP,"isEnabled":s.preload,"params":{"marker":1}}]}),
                "running" => json!({"returnValue":true,"running":if s.running {vec![json!({"id":APP})]} else {vec![]}}),
                "setSystemSettings" => { s.writes.push((uri.into(), payload.clone())); s.native = payload["settings"]["channelplus"] == "on"; json!({"returnValue":true}) },
                "setPreloadPolicy" => { s.writes.push((uri.into(), payload.clone())); s.preload = payload["application"]["isEnabled"].as_bool().unwrap(); json!({"returnValue":true}) },
                "closeByAppId" => { s.writes.push((uri.into(), payload.clone())); if s.fail_close { json!({"returnValue":false}) } else { s.running = false; json!({"returnValue":true}) } },
                _ => panic!("unexpected URI {uri}"),
            })
        }
    }
    fn setup(dir: &std::path::Path) -> (Channels, Arc<Fake>) {
        let fake = Arc::new(Fake(Mutex::new(State { native:true, preload:true, running:true, fail_close:false, writes:vec![] })));
        let channels = Channels { platform:Box::new(fake.clone()), file:dir.join("lgchannels.json"), lock:Mutex::new(()) };
        (channels, fake)
    }
    #[test]
    fn off_closes_channels_and_on_restores_policy() {
        let dir = tempfile::tempdir().unwrap();
        let (channels, fake) = setup(dir.path());
        let status = channels.change(false).unwrap();
        assert_eq!(status["state"], "disabled");
        assert_eq!(status["reason"], "");
        {
            let s = fake.0.lock().unwrap();
            assert!(s.writes[0].0.ends_with("setPreloadPolicy"));
            assert_eq!(s.writes[0].1["application"], json!({"id":APP,"isEnabled":false,"params":{"marker":1}}));
            assert!(s.writes[1].0.ends_with("setSystemSettings"));
            assert_eq!(s.writes[2].1, json!({"id":APP,"reason":"windowPolicy"}));
        }
        let status = channels.change(true).unwrap();
        assert_eq!(status["state"], "enabled");
        assert_eq!(status["reason"], "");
        let s = fake.0.lock().unwrap();
        assert!(s.native && s.preload && !s.running);
    }
    #[test]
    fn failure_is_reported_and_reconcile_retries_saved_preference() {
        let dir = tempfile::tempdir().unwrap();
        let (channels, fake) = setup(dir.path());
        fake.0.lock().unwrap().fail_close = true;
        let status = channels.change(false).unwrap();
        assert_eq!(status["state"], "partial");
        assert_eq!(status["desired"], false);
        assert_eq!(status["running"], true);
        fake.0.lock().unwrap().fail_close = false;
        channels.reconcile();
        assert_eq!(channels.get()["state"], "disabled");
    }
    #[test]
    fn unmanaged_off_with_background_activity_is_partial() {
        let dir = tempfile::tempdir().unwrap();
        let (channels, fake) = setup(dir.path());
        fake.0.lock().unwrap().native = false;
        assert_eq!(channels.get()["state"], "partial");
    }
}
