//! The TV as the AI module acts on it: apps, inputs, standby and the transport keys.
//!
//! Everything the TV is asked goes through `luna-send` (the helper is a 64-bit static binary and cannot link the set's 32-bit
//! luna library), and only the five calls below can ever be built: each is a private constant, the payload is made from
//! `serde_json` values and handed to the program as one argument (never through a shell), and no method takes a URI. An
//! assistant's input reaches a payload in exactly one place, the `id` of `launch`, and only after it has passed the shape check
//! and been found in the TV's own list of apps it may open.

use crate::ai::types::{
    ActionError, AppInfo, ForegroundApp, InputInfo, KeyInjector, MediaKey, TvControl,
};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const URI_FOREGROUND_APP: &str = "luna://com.webos.applicationManager/getForegroundAppInfo";
const URI_LIST_APPS: &str = "luna://com.webos.applicationManager/listApps";
const URI_LAUNCH: &str = "luna://com.webos.applicationManager/launch";
const URI_INPUT_STATUS: &str = "luna://com.webos.service.eim/getAllInputStatus";
const URI_POWER_OFF: &str = "luna://com.webos.service.tvpower/power2/powerOff";

/// The longest a `luna-send` may take before it is killed (the set answers in well under a second).
const DEADLINE: Duration = Duration::from_secs(4);
/// How long the list of apps is reused (it is read for titles and for the membership check of every launch).
const APPS_CACHE: Duration = Duration::from_secs(30);
/// The most of a reply that is read.
const MAX_REPLY: u64 = 1024 * 1024;
/// The longest text taken from the TV (an error, a title, a label) and shown to an assistant.
const MAX_TEXT: usize = 120;

/// Ids an assistant may never open, by prefix. An assistant must not open the tools that change the TV's own configuration
/// (LG's settings and home screens), the Homebrew tools (which can install or remove anything on the set) or LumaTV itself
/// (whose menus are the place permissions are granted).
const DENIED_PREFIXES: &[&str] = &[
    "org.webosbrew.",
    "org.nphil.",
    "com.amazoff.",
    "com.palm.app.settings",
    "com.webos.app.home",
];

/// Ids an assistant may never open, by a word anywhere in them: the settings, developer, factory, hotel-mode, update and
/// service apps, for the same reason as [`DENIED_PREFIXES`].
const DENIED_FRAGMENTS: &[&str] = &[
    "settings",
    "devmode",
    "factory",
    "hotel",
    "softwareupdate",
    "engineering",
    "servicemenu",
];

/// An app id is 1 to `max` characters of letters, digits, dot, underscore and dash: nothing that could change a JSON string,
/// a path or a command line.
fn id_shape_ok(id: &str, max: usize) -> bool {
    (1..=max).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Text from the TV as one plain line of at most [`MAX_TEXT`] characters.
fn one_line(text: &str) -> String {
    let joined = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    joined.chars().take(MAX_TEXT).collect()
}

/// One app as `listApps` reports it.
#[derive(Clone, Debug)]
struct AppRecord {
    id: String,
    title: String,
    /// Shown in the launcher: `visible` is not false and `class.hidden` is not true.
    listed: bool,
}

/// What the module asks of the TV through `luna-send`.
pub struct LunaTv {
    program: String,
    timeout: Duration,
    deny_prefixes: Vec<String>,
    apps: Mutex<Option<(Instant, Vec<AppRecord>)>>,
}

impl LunaTv {
    /// `program` is found with `which` at each call (the helper's `PATH` can be bare) and used as given when it is not found.
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            timeout: DEADLINE,
            deny_prefixes: Vec::new(),
            apps: Mutex::new(None),
        }
    }

    /// More app ids an assistant may not open, by prefix (the brand's own apps).
    pub fn with_deny_prefixes(mut self, prefixes: Vec<String>) -> Self {
        self.deny_prefixes = prefixes
            .into_iter()
            .filter(|p| !p.is_empty())
            .map(|p| p.to_ascii_lowercase())
            .collect();
        self
    }

    /// The deadline of one `luna-send`, after which it is killed.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn denied(&self, id: &str) -> bool {
        let id = id.to_ascii_lowercase();
        DENIED_PREFIXES.iter().any(|p| id.starts_with(p))
            || self
                .deny_prefixes
                .iter()
                .any(|p| id.starts_with(p.as_str()))
            || DENIED_FRAGMENTS.iter().any(|f| id.contains(f))
    }

    /// Runs `luna-send -n 1 <uri> <payload>` and returns the first JSON value it prints. `returnValue: false` is a `failed`
    /// with the TV's own error text; no JSON at all is `unavailable`; a child that does not finish in time is killed.
    fn call(&self, uri: &str, payload: &Value) -> Result<Value, ActionError> {
        let program =
            crate::sys::which(&self.program).unwrap_or_else(|| PathBuf::from(&self.program));
        let body = serde_json::to_string(payload)
            .map_err(|_| ActionError::new("failed", "the request could not be built"))?;
        let mut child = Command::new(program)
            .args(["-n", "1"])
            .arg(uri)
            .arg(body)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| ActionError::new("unavailable", "the TV's luna-send cannot be run"))?;
        let Some(out) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ActionError::new("unavailable", "the TV gave no answer"));
        };
        // The reader ends when the child closes its output, which the kill below guarantees.
        let (tx, rx) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("luna-read".to_owned())
            .stack_size(128 * 1024)
            .spawn(move || {
                let mut text = String::new();
                let _ = out.take(MAX_REPLY).read_to_string(&mut text);
                let _ = tx.send(text);
            });
        if spawned.is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ActionError::new("unavailable", "the TV gave no answer"));
        }
        let received = rx.recv_timeout(self.timeout);
        // Killed on every path: a child that already exited is only reaped, one that hangs on is ended, none is left behind.
        let _ = child.kill();
        let _ = child.wait();
        let text = match received {
            Ok(text) => text,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err(ActionError::new("timeout", "the TV did not answer in time"));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => String::new(),
        };
        let Some(Ok(value)) = serde_json::Deserializer::from_str(&text)
            .into_iter::<Value>()
            .next()
        else {
            return Err(ActionError::new("unavailable", "the TV gave no answer"));
        };
        if value.get("returnValue").and_then(Value::as_bool) == Some(false) {
            let why = value
                .get("errorText")
                .and_then(Value::as_str)
                .map(one_line)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "the TV refused the call".to_owned());
            return Err(ActionError::new("failed", why));
        }
        Ok(value)
    }

    /// Every app the TV lists, hidden ones included, from the cache when it is under 30 s old.
    fn all_apps(&self) -> Result<Vec<AppRecord>, ActionError> {
        if let Some((at, apps)) = &*self.apps.lock()
            && at.elapsed() < APPS_CACHE
        {
            return Ok(apps.clone());
        }
        let reply = self.call(URI_LIST_APPS, &json!({}))?;
        let apps: Vec<AppRecord> = reply
            .get("apps")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(parse_app).collect())
            .ok_or_else(|| ActionError::new("failed", "the TV's list of apps was not usable"))?;
        *self.apps.lock() = Some((Instant::now(), apps.clone()));
        Ok(apps)
    }

    pub fn foreground_app(&self) -> Result<ForegroundApp, ActionError> {
        let reply = self.call(URI_FOREGROUND_APP, &json!({}))?;
        // The surface manager's variant lists the windows; the application manager's has the id at the top.
        let entry = match reply.get("foregroundAppInfo").and_then(Value::as_array) {
            Some(list) => list
                .iter()
                .find(|e| e.get("primary").and_then(Value::as_bool) == Some(true))
                .or_else(|| list.first()),
            None => Some(&reply),
        };
        let app_id = entry
            .and_then(|e| e.get("appId"))
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_owned);
        let title = app_id.as_deref().and_then(|id| {
            self.all_apps()
                .ok()?
                .into_iter()
                .find(|a| a.id == id)
                .map(|a| a.title)
        });
        Ok(ForegroundApp { app_id, title })
    }

    pub fn launchable_apps(&self) -> Result<Vec<AppInfo>, ActionError> {
        let mut apps: Vec<AppInfo> = self
            .all_apps()?
            .into_iter()
            .filter(|a| a.listed && id_shape_ok(&a.id, 128) && !self.denied(&a.id))
            .map(|a| AppInfo {
                id: a.id,
                title: a.title,
            })
            .collect();
        apps.sort_by_cached_key(|a| (a.title.to_lowercase(), a.id.clone()));
        Ok(apps)
    }

    pub fn inputs(&self) -> Result<Vec<InputInfo>, ActionError> {
        let reply = self.call(URI_INPUT_STATUS, &json!({}))?;
        let devices = reply
            .get("devices")
            .and_then(Value::as_array)
            .ok_or_else(|| ActionError::new("failed", "the TV's list of inputs was not usable"))?;
        Ok(devices.iter().filter_map(parse_input).collect())
    }

    pub fn launch_app(&self, app_id: &str) -> Result<AppInfo, ActionError> {
        if !id_shape_ok(app_id, 128) {
            return Err(ActionError::new("refused", "that is not an app id"));
        }
        let not_found = || ActionError::new("not_found", "there is no such app that may be opened");
        if self.denied(app_id) {
            return Err(not_found());
        }
        let app = self
            .launchable_apps()?
            .into_iter()
            .find(|a| a.id == app_id)
            .ok_or_else(not_found)?;
        self.call(URI_LAUNCH, &json!({"id": app.id}))?;
        Ok(app)
    }

    pub fn switch_input(&self, input_id: &str) -> Result<InputInfo, ActionError> {
        let input = self
            .inputs()?
            .into_iter()
            .find(|i| i.id == input_id)
            .ok_or_else(|| ActionError::new("not_found", "there is no such input"))?;
        if !input.connected {
            return Err(ActionError::new(
                "refused",
                "the input has nothing connected",
            ));
        }
        let app_id = input
            .app_id
            .clone()
            .ok_or_else(|| ActionError::new("not_found", "the input has no app to show it"))?;
        if !id_shape_ok(&app_id, 128) || self.denied(&app_id) {
            return Err(ActionError::new(
                "refused",
                "the input's app cannot be opened",
            ));
        }
        self.call(URI_LAUNCH, &json!({"id": app_id}))?;
        Ok(input)
    }

    pub fn standby(&self) -> Result<(), ActionError> {
        self.call(URI_POWER_OFF, &json!({"reason": "remoteKey"}))
            .map(|_| ())
    }
}

fn parse_app(entry: &Value) -> Option<AppRecord> {
    let id = entry.get("id")?.as_str()?;
    if id.is_empty() {
        return None;
    }
    let visible = entry.get("visible").and_then(Value::as_bool) != Some(false);
    let hidden = entry
        .get("class")
        .and_then(|c| c.get("hidden"))
        .and_then(Value::as_bool)
        == Some(true);
    let title = entry
        .get("title")
        .and_then(Value::as_str)
        .map(one_line)
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| one_line(id));
    Some(AppRecord {
        id: id.to_owned(),
        title,
        listed: visible && !hidden,
    })
}

fn parse_input(entry: &Value) -> Option<InputInfo> {
    let id = entry.get("id")?.as_str()?;
    if !id_shape_ok(id, 64) {
        return None;
    }
    let flag = |key: &str| entry.get(key).and_then(Value::as_bool) == Some(true);
    let label = entry
        .get("label")
        .and_then(Value::as_str)
        .map(one_line)
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| id.to_owned());
    Some(InputInfo {
        id: id.to_owned(),
        label,
        connected: flag("connected") || flag("activate"),
        app_id: entry
            .get("appId")
            .and_then(Value::as_str)
            .filter(|a| !a.is_empty())
            .map(str::to_owned),
    })
}

/// The evdev codes of the transport keys. The defaults are the standard Linux codes (`KEY_PLAY`, `KEY_PAUSE`, `KEY_STOP`,
/// `KEY_REWIND`, `KEY_FASTFORWARD`); they have not been verified on the set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaKeyCodes {
    pub play: u16,
    pub pause: u16,
    pub stop: u16,
    pub rewind: u16,
    pub fast_forward: u16,
}

impl Default for MediaKeyCodes {
    fn default() -> Self {
        Self {
            play: 207,
            pause: 119,
            stop: 128,
            rewind: 168,
            fast_forward: 208,
        }
    }
}

impl MediaKeyCodes {
    pub fn code(&self, key: MediaKey) -> u16 {
        match key {
            MediaKey::Play => self.play,
            MediaKey::Pause => self.pause,
            MediaKey::Stop => self.stop,
            MediaKey::Rewind => self.rewind,
            MediaKey::FastForward => self.fast_forward,
        }
    }
}

/// The whole of [`TvControl`]: luna for apps, inputs and standby, the key injector for the transport keys.
pub struct RealTv {
    luna: LunaTv,
    injector: Arc<dyn KeyInjector>,
    key_codes: MediaKeyCodes,
}

impl RealTv {
    pub fn new(luna: LunaTv, injector: Arc<dyn KeyInjector>, key_codes: MediaKeyCodes) -> Self {
        Self {
            luna,
            injector,
            key_codes,
        }
    }
}

impl TvControl for RealTv {
    fn foreground_app(&self) -> Result<ForegroundApp, ActionError> {
        self.luna.foreground_app()
    }

    fn launchable_apps(&self) -> Result<Vec<AppInfo>, ActionError> {
        self.luna.launchable_apps()
    }

    fn inputs(&self) -> Result<Vec<InputInfo>, ActionError> {
        self.luna.inputs()
    }

    fn launch_app(&self, app_id: &str) -> Result<AppInfo, ActionError> {
        self.luna.launch_app(app_id)
    }

    fn switch_input(&self, input_id: &str) -> Result<InputInfo, ActionError> {
        self.luna.switch_input(input_id)
    }

    fn standby(&self) -> Result<(), ActionError> {
        self.luna.standby()
    }

    fn media_key(&self, key: MediaKey) -> Result<(), ActionError> {
        self.injector.tap(self.key_codes.code(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const LIST_APPS: &str = include_str!("../../tests/fixtures/luna/listApps.json");
    const INPUTS: &str = include_str!("../../tests/fixtures/luna/getAllInputStatus.json");
    const FOREGROUND: &str = include_str!("../../tests/fixtures/luna/getForegroundAppInfo.json");
    const FOREGROUND_APPMANAGER: &str =
        include_str!("../../tests/fixtures/luna/getForegroundAppInfo-appmanager.json");
    const OK: &str = include_str!("../../tests/fixtures/luna/ok.json");
    const DENIED: &str = include_str!("../../tests/fixtures/luna/denied.json");

    /// Runs a script that was just written until the system lets it run: another test thread's fork can hold the script's
    /// write descriptor for a moment, and exec of a file open for writing fails with "text file busy".
    fn settle(program: &Path) {
        for _ in 0..300 {
            let result = Command::new(program)
                .arg("probe")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            match result {
                Err(e) if e.raw_os_error() == Some(26) => thread::sleep(Duration::from_millis(10)),
                _ => return,
            }
        }
    }

    /// A `luna-send` that records `uri<TAB>payload` of every call to `calls` and answers with the file named like the last
    /// part of the URI in `answers/` (no file, no output). With a file `hang` it records its pid and sleeps instead.
    struct FakeLuna {
        dir: tempfile::TempDir,
        program: PathBuf,
    }

    impl FakeLuna {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir(dir.path().join("answers")).unwrap();
            let program = dir.path().join("luna-send");
            let script = "#!/bin/sh\n[ \"$1\" = probe ] && exit 0\nD=DIR\nprintf '%s\\t%s\\n' \"$3\" \"$4\" >> \"$D/calls\"\nif [ -f \"$D/hang\" ]; then echo $$ > \"$D/pid\"; exec sleep 30; fi\nf=\"$D/answers/$(basename \"$3\")\"\n[ -f \"$f\" ] && cat \"$f\"\nexit 0\n"
                .replace("DIR", &dir.path().display().to_string());
            std::fs::write(&program, script).unwrap();
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            settle(&program);
            let fake = Self { dir, program };
            fake.answer("listApps", LIST_APPS);
            fake.answer("getAllInputStatus", INPUTS);
            fake.answer("getForegroundAppInfo", FOREGROUND);
            fake.answer("launch", OK);
            fake.answer("powerOff", OK);
            fake
        }

        fn answer(&self, name: &str, text: &str) {
            std::fs::write(self.dir.path().join("answers").join(name), text).unwrap();
        }

        fn tv(&self) -> LunaTv {
            LunaTv::new(self.program.display().to_string())
        }

        fn calls_file(&self) -> PathBuf {
            self.dir.path().join("calls")
        }

        /// Every call as `(uri, payload)`, in order.
        fn calls(&self) -> Vec<(String, String)> {
            std::fs::read_to_string(self.calls_file())
                .unwrap_or_default()
                .lines()
                .map(|l| {
                    let (uri, payload) = l.split_once('\t').unwrap();
                    (uri.to_owned(), payload.to_owned())
                })
                .collect()
        }

        fn launches(&self) -> Vec<String> {
            self.calls()
                .into_iter()
                .filter(|(uri, _)| uri.ends_with("/launch"))
                .map(|(_, p)| p)
                .collect()
        }
    }

    fn ids(apps: &[AppInfo]) -> Vec<&str> {
        apps.iter().map(|a| a.id.as_str()).collect()
    }

    #[test]
    fn the_launchable_list_keeps_the_real_apps_and_drops_the_tools_and_hidden_ones() {
        let fake = FakeLuna::new();
        let apps = fake.tv().launchable_apps().unwrap();
        let listed = ids(&apps);
        for want in ["netflix", "youtube.leanback.v4", "amazon"] {
            assert!(listed.contains(&want), "{want} missing from {listed:?}");
        }
        for dropped in [
            "org.webosbrew.hbchannel",
            "org.webosbrew.safeupdate",
            "org.nphil.lumaglass",
            "org.nphil.lumaglass.frame",
            "com.amazoff.patcher",
            "com.palm.app.settings",
            "com.webos.app.home",
            "com.webos.app.sportsteamsettings",
            "com.webos.app.multiviewsettings",
            "com.webos.app.hdmi2",
            "com.webos.app.acrhdmi1",
            "airplay",
        ] {
            assert!(!listed.contains(&dropped), "{dropped} is in {listed:?}");
        }
        let titles: Vec<String> = apps.iter().map(|a| a.title.to_lowercase()).collect();
        let mut sorted = titles.clone();
        sorted.sort();
        assert_eq!(titles, sorted);
        assert_eq!(
            apps.iter().find(|a| a.id == "netflix").unwrap().title,
            "Netflix"
        );
        assert_eq!(fake.calls(), [(URI_LIST_APPS.to_owned(), "{}".to_owned())]);
    }

    #[test]
    fn the_brand_prefixes_are_denied_too() {
        let fake = FakeLuna::new();
        let tv = fake.tv().with_deny_prefixes(vec!["com.apple.".to_owned()]);
        assert!(!ids(&tv.launchable_apps().unwrap()).contains(&"com.apple.appletv"));
        assert_eq!(
            tv.launch_app("com.apple.appletv").unwrap_err().code,
            "not_found"
        );
        assert!(fake.launches().is_empty());
    }

    #[test]
    fn launch_sends_exactly_the_id() {
        let fake = FakeLuna::new();
        let app = fake.tv().launch_app("netflix").unwrap();
        assert_eq!(
            app,
            AppInfo {
                id: "netflix".to_owned(),
                title: "Netflix".to_owned()
            }
        );
        assert_eq!(fake.launches(), [r#"{"id":"netflix"}"#]);
        assert_eq!(
            fake.calls().last().unwrap().0,
            "luna://com.webos.applicationManager/launch"
        );
    }

    #[test]
    fn malformed_and_excluded_ids_are_refused_before_any_luna_call() {
        let fake = FakeLuna::new();
        let tv = fake.tv();
        let long = "a".repeat(300);
        let malformed = [
            "",
            "net flix",
            " netflix",
            "netflix ",
            "net\"flix",
            "netflix\"}",
            "../netflix",
            "net/flix",
            "netflix\n",
            "netflix\0",
            "n\u{00e9}tflix",
            "\u{4e2d}\u{6587}",
            long.as_str(),
        ];
        for id in malformed {
            assert_eq!(tv.launch_app(id).unwrap_err().code, "refused", "{id:?}");
        }
        for id in [
            "org.webosbrew.hbchannel",
            "ORG.NPHIL.lumaglass",
            "com.palm.app.settings",
            "com.webos.app.home",
            "com.webos.app.sportsteamsettings",
            "x.devmode.y",
            "a.factory",
            "a.hotel.b",
            "softwareupdate",
            "engineering.app",
            "servicemenu",
        ] {
            assert_eq!(tv.launch_app(id).unwrap_err().code, "not_found", "{id}");
        }
        assert!(!fake.calls_file().exists(), "{:?}", fake.calls());
    }

    #[test]
    fn an_unknown_app_is_not_found_and_never_launched() {
        let fake = FakeLuna::new();
        assert_eq!(
            fake.tv().launch_app("no.such.app").unwrap_err().code,
            "not_found"
        );
        // A hidden app is in the list but not launchable.
        assert_eq!(
            fake.tv()
                .launch_app("com.webos.app.acrhdmi1")
                .unwrap_err()
                .code,
            "not_found"
        );
        assert!(fake.launches().is_empty());
    }

    #[test]
    fn inputs_come_from_the_real_reply() {
        let fake = FakeLuna::new();
        let inputs = fake.tv().inputs().unwrap();
        assert_eq!(inputs.len(), 4);
        let hdmi2 = inputs.iter().find(|i| i.id == "HDMI_2").unwrap();
        assert_eq!(hdmi2.label, "AVR-S760H");
        assert!(hdmi2.connected);
        assert_eq!(hdmi2.app_id.as_deref(), Some("com.webos.app.hdmi2"));
        assert!(!inputs.iter().find(|i| i.id == "HDMI_1").unwrap().connected);
    }

    #[test]
    fn an_activated_input_counts_as_connected_and_odd_entries_are_skipped() {
        let fake = FakeLuna::new();
        fake.answer(
            "getAllInputStatus",
            r#"{"returnValue":true,"devices":[
                {"id":"HDMI_3","label":"Box","connected":false,"activate":true,"appId":"com.webos.app.hdmi3"},
                {"label":"no id"},
                {"id":"bad id","connected":true},
                {"id":"HDMI_4","connected":true,"appId":""}]}"#,
        );
        let inputs = fake.tv().inputs().unwrap();
        assert_eq!(
            inputs,
            [
                InputInfo {
                    id: "HDMI_3".to_owned(),
                    label: "Box".to_owned(),
                    connected: true,
                    app_id: Some("com.webos.app.hdmi3".to_owned())
                },
                InputInfo {
                    id: "HDMI_4".to_owned(),
                    label: "HDMI_4".to_owned(),
                    connected: true,
                    app_id: None
                },
            ]
        );
    }

    #[test]
    fn switching_launches_the_inputs_own_app() {
        let fake = FakeLuna::new();
        let tv = fake.tv();
        let input = tv.switch_input("HDMI_2").unwrap();
        assert_eq!(input.id, "HDMI_2");
        assert_eq!(fake.launches(), [r#"{"id":"com.webos.app.hdmi2"}"#]);
        assert_eq!(tv.switch_input("HDMI_1").unwrap_err().code, "refused");
        assert_eq!(
            tv.switch_input("HDMI_1").unwrap_err().message,
            "the input has nothing connected"
        );
        assert_eq!(tv.switch_input("HDMI_9").unwrap_err().code, "not_found");
        assert_eq!(tv.switch_input("HDMI_2\"}").unwrap_err().code, "not_found");
        assert_eq!(fake.launches().len(), 1);
    }

    #[test]
    fn an_input_without_an_app_or_with_a_hostile_app_is_not_launched() {
        let fake = FakeLuna::new();
        fake.answer(
            "getAllInputStatus",
            r#"{"returnValue":true,"devices":[
                {"id":"A","connected":true},
                {"id":"B","connected":true,"appId":"x\"}"},
                {"id":"C","connected":true,"appId":"com.palm.app.settings"}]}"#,
        );
        let tv = fake.tv();
        assert_eq!(tv.switch_input("A").unwrap_err().code, "not_found");
        assert_eq!(tv.switch_input("B").unwrap_err().code, "refused");
        assert_eq!(tv.switch_input("C").unwrap_err().code, "refused");
        assert!(fake.launches().is_empty());
    }

    #[test]
    fn standby_sends_the_remote_key_reason() {
        let fake = FakeLuna::new();
        fake.tv().standby().unwrap();
        assert_eq!(
            fake.calls(),
            [(
                URI_POWER_OFF.to_owned(),
                r#"{"reason":"remoteKey"}"#.to_owned()
            )]
        );
    }

    #[test]
    fn the_foreground_app_is_read_from_either_reply_and_titled_from_the_cached_list() {
        let fake = FakeLuna::new();
        let tv = fake.tv();
        assert_eq!(
            tv.foreground_app().unwrap(),
            ForegroundApp {
                app_id: Some("com.webos.app.home".to_owned()),
                title: Some("Home".to_owned())
            }
        );
        fake.answer("getForegroundAppInfo", FOREGROUND_APPMANAGER);
        assert_eq!(
            tv.foreground_app().unwrap(),
            ForegroundApp {
                app_id: Some("netflix".to_owned()),
                title: Some("Netflix".to_owned())
            }
        );
        let list_calls = fake
            .calls()
            .iter()
            .filter(|(uri, _)| uri == URI_LIST_APPS)
            .count();
        assert_eq!(list_calls, 1);
    }

    #[test]
    fn the_primary_window_wins_and_an_unknown_or_empty_app_has_no_title() {
        let fake = FakeLuna::new();
        let tv = fake.tv();
        fake.answer(
            "getForegroundAppInfo",
            r#"{"returnValue":true,"foregroundAppInfo":[{"appId":"x.second"},{"appId":"netflix","primary":true}]}"#,
        );
        assert_eq!(
            tv.foreground_app().unwrap().app_id.as_deref(),
            Some("netflix")
        );
        fake.answer(
            "getForegroundAppInfo",
            r#"{"returnValue":true,"foregroundAppInfo":[{"appId":"x.second"},{"appId":"netflix"}]}"#,
        );
        assert_eq!(
            tv.foreground_app().unwrap(),
            ForegroundApp {
                app_id: Some("x.second".to_owned()),
                title: None
            }
        );
        fake.answer("getForegroundAppInfo", r#"{"returnValue":true,"appId":""}"#);
        assert_eq!(
            tv.foreground_app().unwrap(),
            ForegroundApp {
                app_id: None,
                title: None
            }
        );
    }

    #[test]
    fn a_refused_call_carries_the_tvs_error_text_on_one_line() {
        let fake = FakeLuna::new();
        fake.answer("launch", DENIED);
        let e = fake.tv().launch_app("netflix").unwrap_err();
        assert_eq!(
            (e.code, e.message.as_str()),
            ("failed", "Denied method call")
        );

        fake.answer(
            "powerOff",
            &format!(
                r#"{{"returnValue":false,"errorText":"line one\n\tline two {}"}}"#,
                "x".repeat(300)
            ),
        );
        let e = fake.tv().standby().unwrap_err();
        assert_eq!(e.code, "failed");
        assert!(e.message.starts_with("line one line two x"));
        assert!(!e.message.contains(['\n', '\t']));
        assert_eq!(e.message.chars().count(), 120);

        fake.answer("powerOff", r#"{"returnValue":false}"#);
        assert_eq!(
            fake.tv().standby().unwrap_err().message,
            "the TV refused the call"
        );
    }

    #[test]
    fn no_output_or_no_program_is_unavailable() {
        let fake = FakeLuna::new();
        std::fs::remove_file(fake.dir.path().join("answers/powerOff")).unwrap();
        assert_eq!(fake.tv().standby().unwrap_err().code, "unavailable");
        fake.answer("powerOff", "not json at all");
        assert_eq!(fake.tv().standby().unwrap_err().code, "unavailable");
        let missing = LunaTv::new("/nonexistent/dir/luna-send");
        assert_eq!(missing.standby().unwrap_err().code, "unavailable");
    }

    #[test]
    fn only_the_first_json_value_is_used() {
        let fake = FakeLuna::new();
        fake.answer(
            "powerOff",
            "{\"returnValue\":true}\n{\"returnValue\":false}\n",
        );
        assert!(fake.tv().standby().is_ok());
    }

    #[test]
    fn a_luna_send_that_hangs_is_killed_at_the_deadline_and_reaped() {
        let fake = FakeLuna::new();
        std::fs::write(fake.dir.path().join("hang"), "").unwrap();
        let started = Instant::now();
        let e = fake
            .tv()
            .with_timeout(Duration::from_millis(300))
            .standby()
            .unwrap_err();
        assert_eq!(e.code, "timeout");
        assert!(started.elapsed() < Duration::from_secs(3));
        let pid = std::fs::read_to_string(fake.dir.path().join("pid")).unwrap();
        let pid = pid.trim();
        assert!(!pid.is_empty());
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "process {pid} is still there"
        );
    }

    #[test]
    fn every_payload_has_exactly_the_expected_keys() {
        let fake = FakeLuna::new();
        let tv = fake.tv();
        let _ = tv.foreground_app();
        let _ = tv.launch_app("netflix");
        let _ = tv.switch_input("HDMI_2");
        let _ = tv.standby();
        for hostile in ["netflix\"}", "x\",\"extra\":\"1", "net flix", "../x"] {
            let _ = tv.launch_app(hostile);
            let _ = tv.switch_input(hostile);
        }
        let calls = fake.calls();
        assert!(!calls.is_empty());
        for (uri, payload) in calls {
            let value: Value = serde_json::from_str(&payload).unwrap();
            let keys: Vec<&str> = value
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            let want: &[&str] = match uri.as_str() {
                URI_LAUNCH => &["id"],
                URI_POWER_OFF => &["reason"],
                URI_LIST_APPS | URI_INPUT_STATUS | URI_FOREGROUND_APP => &[],
                other => panic!("unexpected uri {other}"),
            };
            assert_eq!(keys, want, "{uri} {payload}");
        }
        for payload in fake.launches() {
            let value: Value = serde_json::from_str(&payload).unwrap();
            let id = value["id"].as_str().unwrap();
            assert!(id == "netflix" || id == "com.webos.app.hdmi2", "{id}");
        }
    }

    struct RecordingInjector(Mutex<Vec<u16>>, Option<ActionError>);

    impl KeyInjector for RecordingInjector {
        fn tap(&self, code: u16) -> Result<(), ActionError> {
            self.0.lock().push(code);
            match &self.1 {
                Some(e) => Err(e.clone()),
                None => Ok(()),
            }
        }
    }

    #[test]
    fn media_keys_go_to_the_injector_with_their_codes() {
        let fake = FakeLuna::new();
        let injector = Arc::new(RecordingInjector(Mutex::new(Vec::new()), None));
        let tv = RealTv::new(fake.tv(), injector.clone(), MediaKeyCodes::default());
        for key in MediaKey::ALL {
            tv.media_key(key).unwrap();
        }
        assert_eq!(*injector.0.lock(), [207, 119, 128, 168, 208]);
        assert!(!fake.calls_file().exists());
    }

    #[test]
    fn configured_codes_and_injector_errors_pass_through() {
        let fake = FakeLuna::new();
        let codes = MediaKeyCodes {
            play: 1,
            pause: 2,
            stop: 3,
            rewind: 4,
            fast_forward: 5,
        };
        let injector = Arc::new(RecordingInjector(
            Mutex::new(Vec::new()),
            Some(ActionError::new("unavailable", "no input device")),
        ));
        let tv = RealTv::new(fake.tv(), injector.clone(), codes);
        let e = tv.media_key(MediaKey::FastForward).unwrap_err();
        assert_eq!(e, ActionError::new("unavailable", "no input device"));
        assert_eq!(*injector.0.lock(), [5]);
        assert_eq!(codes.code(MediaKey::Play), 1);
        assert_eq!(codes.code(MediaKey::Stop), 3);
    }

    #[test]
    fn the_real_tv_delegates_apps_inputs_and_standby() {
        let fake = FakeLuna::new();
        let injector = Arc::new(RecordingInjector(Mutex::new(Vec::new()), None));
        let tv = RealTv::new(fake.tv(), injector, MediaKeyCodes::default());
        assert!(!tv.launchable_apps().unwrap().is_empty());
        assert_eq!(tv.inputs().unwrap().len(), 4);
        assert_eq!(tv.launch_app("netflix").unwrap().id, "netflix");
        assert_eq!(tv.switch_input("HDMI_2").unwrap().id, "HDMI_2");
        tv.standby().unwrap();
        assert!(tv.foreground_app().unwrap().app_id.is_some());
    }
}
