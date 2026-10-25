//! The broker: every assistant call goes through here, in this order.
//!
//! 1. the arguments are parsed and validated (`tools`): an unknown tool or a bad argument stops here;
//! 2. the assistant must still be paired, must not be paused, and must be inside its rate limit;
//! 3. its level for the tool's group decides: `never` refuses (and the tool is not even listed), `always` goes on, `ask` opens a
//!    prompt on the TV that only a physical remote can answer (`gate`): OK runs the call, Back refuses it *and pauses the
//!    assistant until the owner resumes it*, no answer within the time refuses it (and the assistant waits a few seconds before
//!    it may ask again);
//! 4. before any prompt the target is checked against what the TV actually has (an app that can be opened, an input that
//!    exists and has something connected, a volume inside the limit), so the prompt only ever asks about something real and its
//!    words come from the TV's own names;
//! 5. the action runs through the narrow traits of `types` (`TvControl`, `ReceiverControl`): there is no way to say anything else;
//! 6. whatever happened is audited.
//!
//! Nothing here can change a level, pair an assistant, turn the listener on or answer a prompt: those need the `&GateApproval`
//! only `gate` can make, and no tool takes one.

use super::audit::{Audit, Entry, Outcome};
use super::gate::{Busy, Decision, Gate, PromptKind, PromptSpec};
use super::store::{Clock, Store};
use super::tools::{self, VOLUME_MAX, VOLUME_STEP};
use super::types::{
    ActionError, AssistantView, AuthFailure, Authenticator, Call, Group, Level, PairFailure,
    Paired, ReceiverControl, ToolDef, ToolHost, ToolOutcome, TvControl,
};
use crate::hub::Hub;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Instant;

/// How the broker limits and paces assistants.
#[derive(Clone, Debug)]
pub struct BrokerConfig {
    /// Reads an assistant may make in a minute.
    pub reads_per_minute: usize,
    /// Actions (anything but a read) an assistant may make in a minute, whether or not they were allowed.
    pub actions_per_minute: usize,
    /// How long an assistant must wait after a prompt that expired or was denied before it can open another.
    pub ask_cooldown_ms: u64,
}

impl Default for BrokerConfig {
    fn default() -> Self {
        Self {
            reads_per_minute: 60,
            actions_per_minute: 12,
            ask_cooldown_ms: 10_000,
        }
    }
}

/// Who is calling what, for the audit trail.
struct CallCtx<'a> {
    who: Option<&'a AssistantView>,
    peer: Option<IpAddr>,
    tool: &'a str,
    args: &'a Value,
    started: Instant,
}

/// What a call needs resolved before it can be prompted about or run.
enum Resolved {
    None,
    App(super::types::AppInfo),
    Input(super::types::InputInfo),
    /// A volume change expressed as the level it ends at.
    Volume(f64),
}

pub struct Broker {
    store: Arc<Store>,
    gate: Arc<Gate>,
    hub: Arc<Hub>,
    audit: Arc<Audit>,
    tv: Arc<dyn TvControl>,
    receiver: Arc<dyn ReceiverControl>,
    /// Asks the receiver poller to read now (after a volume or mute change).
    nudge: Arc<dyn Fn() + Send + Sync>,
    clock: Clock,
    config: BrokerConfig,
    windows: Mutex<HashMap<(String, bool), VecDeque<u64>>>,
    cooldowns: Mutex<HashMap<String, u64>>,
}

/// Everything a broker acts through, bundled so `Ai` can build one.
pub struct BrokerDeps {
    pub store: Arc<Store>,
    pub gate: Arc<Gate>,
    pub hub: Arc<Hub>,
    pub audit: Arc<Audit>,
    pub tv: Arc<dyn TvControl>,
    pub receiver: Arc<dyn ReceiverControl>,
    pub nudge: Arc<dyn Fn() + Send + Sync>,
    pub clock: Clock,
    pub config: BrokerConfig,
}

fn refusal(code: &'static str, message: impl Into<String>) -> ToolOutcome {
    ToolOutcome::Err {
        code,
        message: message.into(),
    }
}

impl From<ActionError> for ToolOutcome {
    fn from(e: ActionError) -> Self {
        refusal(e.code, e.message)
    }
}

/// The receiver topic's reading, when it is current.
struct ReceiverReading {
    power: String,
    input: Option<String>,
    level: Option<f64>,
    db: Option<f64>,
    mute: Option<bool>,
}

impl Broker {
    pub fn new(deps: BrokerDeps) -> Arc<Self> {
        Arc::new(Self {
            store: deps.store,
            gate: deps.gate,
            hub: deps.hub,
            audit: deps.audit,
            tv: deps.tv,
            receiver: deps.receiver,
            nudge: deps.nudge,
            clock: deps.clock,
            config: deps.config,
            windows: Mutex::new(HashMap::new()),
            cooldowns: Mutex::new(HashMap::new()),
        })
    }

    fn reading(&self) -> Option<ReceiverReading> {
        let doc = self.hub.get(crate::receiver::TOPIC)?;
        if doc.get("fresh") != Some(&Value::Bool(true)) {
            return None;
        }
        Some(ReceiverReading {
            power: doc.get("power")?.as_str()?.to_owned(),
            input: doc.get("input").and_then(Value::as_str).map(str::to_owned),
            level: doc.get("volumeAbs").and_then(Value::as_f64),
            db: doc.get("volumeDb").and_then(Value::as_f64),
            mute: doc.get("mute").and_then(Value::as_bool),
        })
    }

    fn allow(&self, assistant: &str, read: bool) -> bool {
        let now = (self.clock)();
        let limit = if read {
            self.config.reads_per_minute
        } else {
            self.config.actions_per_minute
        };
        let mut windows = self.windows.lock();
        if windows.len() > 64 {
            windows.retain(|_, w| w.back().is_some_and(|t| now.saturating_sub(*t) < 60_000));
        }
        let window = windows.entry((assistant.to_owned(), read)).or_default();
        while window
            .front()
            .is_some_and(|t| now.saturating_sub(*t) >= 60_000)
        {
            window.pop_front();
        }
        if window.len() >= limit {
            return false;
        }
        window.push_back(now);
        true
    }

    fn record(&self, c: &CallCtx<'_>, level: Option<Level>, outcome: Outcome, message: &str) {
        self.audit.record(&Entry {
            assistant: c.who.map(|w| (w.id.as_str(), w.name.as_str())),
            peer: c.peer,
            tool: c.tool,
            args: c.args,
            level: level.map(Level::id),
            outcome,
            message,
            took_ms: u64::try_from(c.started.elapsed().as_millis()).unwrap_or(u64::MAX),
        });
    }

    /// Checks the call's target against what the TV has. A refusal here never reaches a prompt.
    fn resolve(&self, call: &Call) -> Result<Resolved, ToolOutcome> {
        match call {
            Call::LaunchApp { app_id } => {
                let apps = self.tv.launchable_apps().map_err(ToolOutcome::from)?;
                apps.into_iter()
                    .find(|a| &a.id == app_id)
                    .map(Resolved::App)
                    .ok_or_else(|| {
                        refusal(
                            "not_found",
                            "that app cannot be opened (see tv_list_apps for the ones that can)",
                        )
                    })
            }
            Call::SwitchInput { input_id } => {
                let inputs = self.tv.inputs().map_err(ToolOutcome::from)?;
                let input = inputs
                    .into_iter()
                    .find(|i| &i.id == input_id)
                    .ok_or_else(|| {
                        refusal("not_found", "the TV has no such input (see tv_list_inputs)")
                    })?;
                if !input.connected {
                    return Err(refusal("refused", "nothing is connected to that input"));
                }
                Ok(Resolved::Input(input))
            }
            Call::SetVolume { level } => {
                self.check_limit(*level)?;
                Ok(Resolved::Volume(*level))
            }
            Call::ChangeVolume { up, steps } => {
                let current = self
                    .reading()
                    .and_then(|r| r.level)
                    .ok_or_else(|| refusal("unavailable", "the receiver's volume is not known right now (it is read only while the screen is on and the receiver answers)"))?;
                let delta = f64::from(*steps) * VOLUME_STEP;
                let target = if *up {
                    current + delta
                } else {
                    (current - delta).max(0.0)
                };
                let target = tools::round_half(target.min(VOLUME_MAX));
                if *up {
                    self.check_limit(target)?;
                }
                Ok(Resolved::Volume(target))
            }
            _ => Ok(Resolved::None),
        }
    }

    fn check_limit(&self, level: f64) -> Result<(), ToolOutcome> {
        let limit = self.store.max_volume();
        if level > limit {
            return Err(refusal(
                "over_limit",
                format!("the volume limit set on the TV is {limit}; it can only be raised there"),
            ));
        }
        Ok(())
    }

    fn execute(&self, call: &Call, resolved: Resolved) -> ToolOutcome {
        match (call, resolved) {
            (Call::GetPower, _) => match self.hub.get(crate::screen::TOPIC) {
                Some(doc) if doc.get("on").is_some() => {
                    let on = doc["on"].as_bool().unwrap_or(false);
                    ToolOutcome::Ok {
                        text: format!("The TV's screen is {}.", if on { "on" } else { "off" }),
                        data: json!({"on": on, "state": doc["state"], "since": doc["since"]}),
                    }
                }
                _ => refusal("unavailable", "the TV's power state is not known yet"),
            },
            (Call::GetForegroundApp, _) => match self.tv.foreground_app() {
                Ok(app) => ToolOutcome::Ok {
                    text: match &app.title {
                        Some(title) => format!("{title} is in front."),
                        None => "Nothing the TV names is in front.".to_owned(),
                    },
                    data: json!({"appId": app.app_id, "title": app.title}),
                },
                Err(e) => e.into(),
            },
            (Call::ListInputs, _) => match self.tv.inputs() {
                Ok(inputs) => ToolOutcome::Ok {
                    text: format!("The TV has {} inputs.", inputs.len()),
                    data: json!({"inputs": inputs.iter().map(|i| json!({"id": i.id, "label": i.label, "connected": i.connected})).collect::<Vec<_>>()}),
                },
                Err(e) => e.into(),
            },
            (Call::ListApps, _) => match self.tv.launchable_apps() {
                Ok(apps) => ToolOutcome::Ok {
                    text: format!("{} apps can be opened.", apps.len()),
                    data: json!({"apps": apps.iter().map(|a| json!({"id": a.id, "title": a.title})).collect::<Vec<_>>()}),
                },
                Err(e) => e.into(),
            },
            (Call::GetReceiver, _) => ToolOutcome::Ok {
                text: "The receiver's status as last read.".to_owned(),
                data: self.receiver_data(),
            },
            (Call::GetNowPlaying, _) => {
                let app = self.tv.foreground_app().ok();
                let inputs = self.tv.inputs().unwrap_or_default();
                let input = app
                    .as_ref()
                    .and_then(|a| a.app_id.as_deref())
                    .and_then(|id| inputs.iter().find(|i| i.app_id.as_deref() == Some(id)))
                    .map(|i| json!({"id": i.id, "label": i.label}));
                ToolOutcome::Ok {
                    text: "What the TV is showing and what the receiver is set to. What an app plays is not available.".to_owned(),
                    data: json!({
                        "app": app.as_ref().map(|a| json!({"id": a.app_id, "title": a.title})),
                        "input": input,
                        "receiver": self.receiver_data(),
                        "note": "Titles of what an app is playing are not available to the TV's services.",
                    }),
                }
            }
            (Call::SetVolume { .. } | Call::ChangeVolume { .. }, Resolved::Volume(level)) => {
                match self.receiver.set_volume(level) {
                    Ok(()) => {
                        (self.nudge)();
                        ToolOutcome::Ok {
                            text: format!("Set the receiver volume to {level}."),
                            data: json!({"level": level}),
                        }
                    }
                    Err(e) => e.into(),
                }
            }
            (Call::SetMute { muted }, _) => match self.receiver.set_mute(*muted) {
                Ok(()) => {
                    (self.nudge)();
                    ToolOutcome::Ok {
                        text: if *muted {
                            "Muted the receiver."
                        } else {
                            "Unmuted the receiver."
                        }
                        .to_owned(),
                        data: json!({"muted": muted}),
                    }
                }
                Err(e) => e.into(),
            },
            (Call::MediaKey { key }, _) => match self.tv.media_key(*key) {
                Ok(()) => ToolOutcome::Ok {
                    text: format!("Pressed {} on the TV.", key.id().replace('_', " ")),
                    data: json!({"key": key.id()}),
                },
                Err(e) => e.into(),
            },
            (Call::LaunchApp { app_id }, _) => match self.tv.launch_app(app_id) {
                Ok(app) => ToolOutcome::Ok {
                    text: format!("Opened {}.", tools::clean_text(&app.title, 60)),
                    data: json!({"appId": app.id, "title": app.title}),
                },
                Err(e) => e.into(),
            },
            (Call::SwitchInput { input_id }, _) => match self.tv.switch_input(input_id) {
                Ok(input) => ToolOutcome::Ok {
                    text: format!(
                        "Switched the TV to {}.",
                        tools::clean_text(&input.label, 60)
                    ),
                    data: json!({"id": input.id, "label": input.label}),
                },
                Err(e) => e.into(),
            },
            (Call::Standby, _) => match self.tv.standby() {
                Ok(()) => ToolOutcome::Ok {
                    text: "The TV is going to standby.".to_owned(),
                    data: json!({"standby": true}),
                },
                Err(e) => e.into(),
            },
            (Call::SetVolume { .. } | Call::ChangeVolume { .. }, _) => {
                refusal("failed", "the volume was not worked out")
            }
        }
    }

    fn receiver_data(&self) -> Value {
        match self.reading() {
            Some(r) => json!({
                "fresh": true,
                "power": r.power,
                "input": r.input,
                "volume": {"level": r.level, "db": r.db},
                "mute": r.mute,
                "limit": self.store.max_volume(),
            }),
            None => json!({
                "fresh": false,
                "reason": "the receiver is not being read now (the screen is off, the receiver is switched off in the settings, or it does not answer)",
                "limit": self.store.max_volume(),
            }),
        }
    }

    fn cooldown_left(&self, assistant: &str) -> Option<u64> {
        let now = (self.clock)();
        let mut cooldowns = self.cooldowns.lock();
        cooldowns.retain(|_, until| *until > now);
        cooldowns
            .get(assistant)
            .map(|until| (until - now).div_ceil(1000))
    }

    fn start_cooldown(&self, assistant: &str) {
        let until = (self.clock)() + self.config.ask_cooldown_ms;
        self.cooldowns.lock().insert(assistant.to_owned(), until);
    }

    /// Runs the prompt for an `ask` call. `Ok(note)` means a physical OK; `Err` is what to answer the assistant.
    fn ask(
        &self,
        who: &AssistantView,
        call: &Call,
        resolved: &Resolved,
        reason: Option<String>,
    ) -> Result<String, (Outcome, ToolOutcome, String)> {
        if let Some(wait) = self.cooldown_left(&who.id) {
            let message = format!(
                "a request of yours was just refused or went unanswered on the TV; wait {wait} s before asking again"
            );
            return Err((
                Outcome::Cooldown,
                refusal("cooldown", message.clone()),
                message,
            ));
        }
        let (app, input) = match resolved {
            Resolved::App(a) => (Some(a), None),
            Resolved::Input(i) => (None, Some(i)),
            _ => (None, None),
        };
        let (title, detail) = tools::describe(call, app, input);
        let spec = PromptSpec {
            kind: PromptKind::Call,
            assistant: Some(who.clone()),
            title,
            detail,
            reason,
        };
        let pending = match self.gate.begin(spec) {
            Ok(p) => p,
            Err(Busy) => {
                let message =
                    "another request is waiting for an answer on the TV; try again shortly"
                        .to_owned();
                return Err((Outcome::Busy, refusal("busy", message.clone()), message));
            }
        };
        let verdict = pending.wait();
        let ignored = verdict.ignored_network_keys + verdict.ignored_unknown_keys;
        let ignored_note = if ignored > 0 {
            format!(" ({ignored} key press(es) from network devices were ignored)")
        } else {
            String::new()
        };
        match verdict.decision {
            Decision::Approved(_) => Ok("allowed on the TV".to_owned()),
            Decision::Denied => {
                self.store.pause(&who.id);
                self.start_cooldown(&who.id);
                let message = "the request was denied on the TV; this assistant is paused until the owner resumes it there".to_owned();
                Err((Outcome::Denied, refusal("denied", message.clone()), message))
            }
            Decision::Expired => {
                self.start_cooldown(&who.id);
                let message = format!("nobody answered on the TV in time{ignored_note}");
                Err((
                    Outcome::Expired,
                    refusal("expired", message.clone()),
                    message,
                ))
            }
            Decision::Cancelled => {
                let message = "the request was withdrawn".to_owned();
                Err((
                    Outcome::Cancelled,
                    refusal("cancelled", message.clone()),
                    message,
                ))
            }
        }
    }
}

impl ToolHost for Broker {
    fn list_tools(&self, who: &AssistantView) -> Vec<ToolDef> {
        let Some(levels) = self.store.levels_of(&who.id) else {
            return Vec::new();
        };
        tools::catalogue()
            .into_iter()
            .filter_map(|mut def| {
                let level = levels.get(tools::group_of(def.name)?);
                match level {
                    Level::Never => None,
                    Level::Ask => {
                        def.description.push_str(
                            " Asks for approval on the TV first and waits up to 30 seconds.",
                        );
                        Some(def)
                    }
                    Level::Always => Some(def),
                }
            })
            .collect()
    }

    fn call_tool(
        &self,
        who: &AssistantView,
        peer: IpAddr,
        name: &str,
        arguments: &Value,
    ) -> ToolOutcome {
        let c = CallCtx {
            who: Some(who),
            peer: Some(peer),
            tool: name,
            args: arguments,
            started: Instant::now(),
        };
        let parsed = match tools::parse(name, arguments) {
            Ok(p) => p,
            Err(e) => {
                self.record(&c, None, Outcome::Invalid, &e.message);
                return refusal(e.code, e.message);
            }
        };
        let call = parsed.call;
        let group: Group = call.group();
        let Some(info) = self.store.assistant(&who.id) else {
            self.record(&c, None, Outcome::Unauthorized, "no longer paired");
            return refusal(
                "unauthorized",
                "this assistant is no longer paired with the TV",
            );
        };
        if !self.allow(&who.id, call.is_read_only()) {
            let message = "too many requests; wait a minute";
            self.record(&c, None, Outcome::RateLimited, message);
            return refusal("rate_limited", message);
        }
        let level = info.levels.get(group);
        if info.paused {
            let message = "this assistant is paused until the owner resumes it on the TV";
            self.record(&c, Some(level), Outcome::Paused, message);
            return refusal("paused", message);
        }
        if level == Level::Never {
            let message = format!("this assistant has no access to \"{}\"", group.title());
            self.record(&c, Some(level), Outcome::Never, &message);
            return refusal("never", message);
        }
        let resolved = match self.resolve(&call) {
            Ok(r) => r,
            Err(outcome) => {
                let (code, message) = match &outcome {
                    ToolOutcome::Err { code, message } => (*code, message.clone()),
                    ToolOutcome::Ok { .. } => ("failed", String::new()),
                };
                let kind = if code == "unavailable" {
                    Outcome::Failed
                } else {
                    Outcome::Invalid
                };
                self.record(&c, Some(level), kind, &message);
                return outcome;
            }
        };
        let mut note = String::new();
        if level == Level::Ask {
            match self.ask(who, &call, &resolved, parsed.reason.clone()) {
                Ok(text) => note = text,
                Err((kind, outcome, message)) => {
                    self.record(&c, Some(level), kind, &message);
                    return outcome;
                }
            }
            // The prompt can outlast a revoke or a lock-out: check again before acting.
            if self.store.assistant(&who.id).is_none_or(|a| a.paused) {
                let message = "this assistant was paused or unpaired while the TV was asking";
                self.record(&c, Some(level), Outcome::Unauthorized, message);
                return refusal("unauthorized", message);
            }
        }
        self.store.record_call(&who.id);
        let outcome = self.execute(&call, resolved);
        let (kind, message) = match &outcome {
            ToolOutcome::Ok { text, .. } => (
                Outcome::Ok,
                if note.is_empty() {
                    text.clone()
                } else {
                    format!("{text} ({note})")
                },
            ),
            ToolOutcome::Err { message, .. } => (Outcome::Failed, message.clone()),
        };
        self.record(&c, Some(level), kind, &message);
        outcome
    }
}

impl Authenticator for Broker {
    fn authenticate(&self, bearer: &str, _peer: IpAddr) -> Result<AssistantView, AuthFailure> {
        self.store.authenticate(bearer).ok_or(AuthFailure::Unknown)
    }

    fn pair(&self, code: &str, name: &str, peer: IpAddr) -> Result<Paired, PairFailure> {
        let started = Instant::now();
        match self.store.complete_pairing(code, name, peer) {
            Ok(paired) => {
                let who = AssistantView {
                    id: paired.assistant_id.clone(),
                    name: super::store::clean_name(name).unwrap_or_default(),
                };
                let c = CallCtx {
                    who: Some(&who),
                    peer: Some(peer),
                    tool: "pair",
                    args: &Value::Null,
                    started,
                };
                self.record(&c, None, Outcome::Ok, "paired");
                Ok(paired)
            }
            Err(failure) => {
                let message = match failure {
                    PairFailure::Refused => "pairing refused",
                    PairFailure::TooManyAttempts => "pairing refused: too many attempts",
                    PairFailure::BadName => "pairing refused: bad name",
                };
                let c = CallCtx {
                    who: None,
                    peer: Some(peer),
                    tool: "pair",
                    args: &Value::Null,
                    started,
                };
                self.record(&c, None, Outcome::Unauthorized, message);
                Err(failure)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::gate::fake::FakeKeys;
    use super::super::gate::{GateConfig, approval_for_tests as approval};
    use super::super::levels::Levels;
    use super::super::types::{AppInfo, ForegroundApp, InputInfo, MediaKey};
    use super::*;
    use crate::settings::SettingsStore;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    const IR: &str = "LGE RCU";
    const PEER: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 9));

    #[derive(Default)]
    struct FakeTv {
        calls: Mutex<Vec<String>>,
        fail: Mutex<Option<ActionError>>,
    }

    impl FakeTv {
        fn log(&self, what: impl Into<String>) {
            self.calls.lock().push(what.into());
        }

        fn acted(&self) -> Vec<String> {
            self.calls
                .lock()
                .iter()
                .filter(|c| !c.starts_with("read:"))
                .cloned()
                .collect()
        }

        fn fail_with(&self, e: ActionError) {
            *self.fail.lock() = Some(e);
        }

        fn check(&self) -> Result<(), ActionError> {
            self.fail.lock().clone().map_or(Ok(()), Err)
        }
    }

    impl TvControl for FakeTv {
        fn foreground_app(&self) -> Result<ForegroundApp, ActionError> {
            self.log("read:foreground");
            Ok(ForegroundApp {
                app_id: Some("netflix".into()),
                title: Some("Netflix".into()),
            })
        }

        fn launchable_apps(&self) -> Result<Vec<AppInfo>, ActionError> {
            self.log("read:apps");
            Ok(vec![
                AppInfo {
                    id: "netflix".into(),
                    title: "Netflix".into(),
                },
                AppInfo {
                    id: "youtube.leanback.v4".into(),
                    title: "YouTube".into(),
                },
            ])
        }

        fn inputs(&self) -> Result<Vec<InputInfo>, ActionError> {
            self.log("read:inputs");
            Ok(vec![
                InputInfo {
                    id: "HDMI_1".into(),
                    label: "HDMI 1".into(),
                    connected: false,
                    app_id: Some("com.webos.app.hdmi1".into()),
                },
                InputInfo {
                    id: "HDMI_2".into(),
                    label: "AVR-S760H".into(),
                    connected: true,
                    app_id: Some("com.webos.app.hdmi2".into()),
                },
            ])
        }

        fn launch_app(&self, app_id: &str) -> Result<AppInfo, ActionError> {
            self.check()?;
            self.log(format!("launch {app_id}"));
            Ok(AppInfo {
                id: app_id.into(),
                title: "Netflix".into(),
            })
        }

        fn switch_input(&self, input_id: &str) -> Result<InputInfo, ActionError> {
            self.check()?;
            self.log(format!("switch {input_id}"));
            Ok(InputInfo {
                id: input_id.into(),
                label: "AVR-S760H".into(),
                connected: true,
                app_id: None,
            })
        }

        fn standby(&self) -> Result<(), ActionError> {
            self.check()?;
            self.log("standby");
            Ok(())
        }

        fn media_key(&self, key: MediaKey) -> Result<(), ActionError> {
            self.check()?;
            self.log(format!("key {}", key.id()));
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeReceiver {
        calls: Mutex<Vec<String>>,
    }

    impl ReceiverControl for FakeReceiver {
        fn set_volume(&self, level: f64) -> Result<(), ActionError> {
            self.calls.lock().push(format!("volume {level}"));
            Ok(())
        }

        fn set_mute(&self, muted: bool) -> Result<(), ActionError> {
            self.calls.lock().push(format!("mute {muted}"));
            Ok(())
        }
    }

    struct Rig {
        broker: Arc<Broker>,
        store: Arc<Store>,
        hub: Arc<Hub>,
        keys: Arc<FakeKeys>,
        tv: Arc<FakeTv>,
        receiver: Arc<FakeReceiver>,
        audit: Arc<Audit>,
        nudges: Arc<AtomicUsize>,
        now: Arc<AtomicU64>,
        who: AssistantView,
        _dir: tempfile::TempDir,
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let hub = Arc::new(Hub::new(1));
        let settings = Arc::new(SettingsStore::new(dir.path().join("settings"), hub.clone()));
        settings.register_builtin().unwrap();
        settings.protect_prefix("ai.");
        let now = Arc::new(AtomicU64::new(1_800_000_000_000));
        let clock: Clock = {
            let now = now.clone();
            Arc::new(move || now.load(Ordering::SeqCst))
        };
        let store = Store::load(dir.path().join("ai"), settings, clock.clone());
        let keys = Arc::new(FakeKeys::default());
        let gate = Gate::new(
            keys.clone(),
            hub.clone(),
            GateConfig {
                prompt_ttl: Duration::from_millis(400),
                min_display: Duration::from_millis(30),
                slice: Duration::from_millis(10),
                ..GateConfig::default()
            },
        );
        let audit = Audit::open(dir.path().join("audit"), hub.clone());
        let tv = Arc::new(FakeTv::default());
        let receiver = Arc::new(FakeReceiver::default());
        let nudges = Arc::new(AtomicUsize::new(0));
        let broker = Broker::new(BrokerDeps {
            store: store.clone(),
            gate,
            hub: hub.clone(),
            audit: audit.clone(),
            tv: tv.clone(),
            receiver: receiver.clone(),
            nudge: {
                let n = nudges.clone();
                Arc::new(move || {
                    n.fetch_add(1, Ordering::SeqCst);
                })
            },
            clock,
            config: BrokerConfig::default(),
        });
        let view = store.open_pairing(&approval());
        let paired = store.complete_pairing(&view.code, "Cody", PEER).unwrap();
        let who = store.authenticate(&paired.token).unwrap();
        hub.set("screen", json!({"on": true, "state": "Active", "since": 5, "source": "luna-poll", "error": null}));
        hub.set(
            "receiver",
            json!({"fresh": true, "power": "on", "input": "TV", "volumeAbs": 35.5, "volumeDb": -44.5, "mute": false, "volumeText": "35.5"}),
        );
        Rig {
            broker,
            store,
            hub,
            keys,
            tv,
            receiver,
            audit,
            nudges,
            now,
            who,
            _dir: dir,
        }
    }

    impl Rig {
        fn set_levels(&self, levels: Levels) {
            self.store
                .widen_levels(&self.who.id, &levels, &approval())
                .unwrap();
        }

        fn all(&self, level: Level) {
            self.set_levels(Levels {
                state: level,
                playback: level,
                launch: level,
                power: level,
            });
        }

        fn call(&self, name: &str, args: Value) -> ToolOutcome {
            self.broker.call_tool(&self.who, PEER, name, &args)
        }

        /// Calls on a thread, waits for the prompt to be on the hub, then does `answer` with the fake remote.
        fn ask_and(
            &self,
            name: &'static str,
            args: Value,
            answer: impl FnOnce(&FakeKeys),
        ) -> ToolOutcome {
            let broker = self.broker.clone();
            let who = self.who.clone();
            let worker = thread::spawn(move || broker.call_tool(&who, PEER, name, &args));
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.hub.get("ai.prompt").is_none_or(|v| v.is_null()) && Instant::now() < deadline
            {
                thread::sleep(Duration::from_millis(5));
            }
            thread::sleep(Duration::from_millis(60));
            answer(&self.keys);
            worker.join().unwrap()
        }

        fn last_audit(&self) -> Value {
            self.audit.recent(1, None).pop().unwrap()
        }
    }

    fn code_of(outcome: &ToolOutcome) -> &'static str {
        match outcome {
            ToolOutcome::Err { code, .. } => code,
            ToolOutcome::Ok { .. } => "ok",
        }
    }

    fn is_ok(outcome: &ToolOutcome) -> bool {
        matches!(outcome, ToolOutcome::Ok { .. })
    }

    /// One representative call per tool.
    fn samples() -> Vec<(&'static str, Value, Group)> {
        vec![
            ("tv_get_power", json!({}), Group::State),
            ("tv_get_foreground_app", json!({}), Group::State),
            ("tv_list_inputs", json!({}), Group::State),
            ("tv_list_apps", json!({}), Group::State),
            ("tv_get_now_playing", json!({}), Group::State),
            ("receiver_get_status", json!({}), Group::State),
            ("receiver_set_volume", json!({"level": 30}), Group::Playback),
            (
                "receiver_change_volume",
                json!({"direction": "down"}),
                Group::Playback,
            ),
            ("receiver_set_mute", json!({"muted": true}), Group::Playback),
            ("tv_media_key", json!({"key": "pause"}), Group::Playback),
            ("tv_launch_app", json!({"app_id": "netflix"}), Group::Launch),
            (
                "tv_switch_input",
                json!({"input_id": "HDMI_2"}),
                Group::Launch,
            ),
            ("tv_standby", json!({}), Group::Power),
        ]
    }

    fn acted(r: &Rig) -> usize {
        r.tv.acted().len() + r.receiver.calls.lock().len()
    }

    #[test]
    fn the_permission_matrix_never_always_and_ask() {
        for (name, args, group) in samples() {
            // Never: refused, not listed, nothing acted on, no prompt.
            let r = rig();
            r.all(Level::Never);
            assert_eq!(code_of(&r.call(name, args.clone())), "never", "{name}");
            assert!(
                r.broker.list_tools(&r.who).iter().all(|d| d.name != name),
                "{name} is listed at never"
            );
            assert_eq!(acted(&r), 0, "{name}");
            assert_eq!(r.keys.opens(), 0, "{name} opened a prompt at never");

            // Always: runs, no prompt.
            let r = rig();
            r.all(Level::Always);
            assert!(
                is_ok(&r.call(name, args.clone())),
                "{name}: {:?}",
                r.call(name, args.clone())
            );
            assert_eq!(r.keys.opens(), 0, "{name} prompted at always");
            assert!(
                r.broker.list_tools(&r.who).iter().any(|d| d.name == name),
                "{name}"
            );

            // Ask: a physical OK runs it.
            let r = rig();
            r.all(Level::Ask);
            let outcome = r.ask_and(name, args.clone(), |k| k.tap(IR, 28));
            assert!(is_ok(&outcome), "{name} ask+OK: {outcome:?}");
            assert_eq!(r.keys.opens(), 1, "{name}");
            if group != Group::State {
                assert_eq!(acted(&r), 1, "{name}");
            }
            assert!(r.store.assistant(&r.who.id).is_some_and(|a| !a.paused));

            // Ask: Back refuses, pauses the assistant, and nothing is done.
            let r = rig();
            r.all(Level::Ask);
            let outcome = r.ask_and(name, args.clone(), |k| k.tap(IR, 412));
            assert_eq!(code_of(&outcome), "denied", "{name}");
            assert_eq!(acted(&r), 0, "{name}");
            assert_eq!(
                r.store.is_paused(&r.who.id),
                Some(true),
                "{name}: a denial pauses the assistant"
            );
            assert_eq!(r.last_audit()["outcome"], "denied");

            // Ask: nobody answers -> refused, not paused.
            let r = rig();
            r.all(Level::Ask);
            let outcome = r.call(name, args.clone());
            assert_eq!(code_of(&outcome), "expired", "{name}");
            assert_eq!(acted(&r), 0, "{name}");
            assert_eq!(
                r.store.is_paused(&r.who.id),
                Some(false),
                "{name}: an expiry does not pause"
            );

            // Ask: an OK from the network device never runs it.
            let r = rig();
            r.all(Level::Ask);
            let outcome = r.ask_and(name, args.clone(), |k| {
                k.tap("Smart Remote RCU Input", 28);
                k.tap("LGE Network Input", 28);
            });
            assert_eq!(code_of(&outcome), "expired", "{name}");
            assert_eq!(acted(&r), 0, "{name}");
            assert!(
                r.last_audit()["message"]
                    .as_str()
                    .unwrap()
                    .contains("network devices were ignored"),
                "{name}"
            );
        }
    }

    #[test]
    fn the_default_levels_read_and_play_freely_and_ask_before_opening_or_turning_off() {
        let r = rig();
        assert!(is_ok(&r.call("tv_get_power", json!({}))));
        assert!(is_ok(&r.call("tv_media_key", json!({"key": "play"}))));
        assert!(is_ok(&r.call("receiver_set_mute", json!({"muted": false}))));
        assert_eq!(r.keys.opens(), 0);
        assert_eq!(
            code_of(&r.call("tv_launch_app", json!({"app_id": "netflix"}))),
            "expired"
        );
        assert_eq!(r.keys.opens(), 1);
        r.now.fetch_add(60_000, Ordering::SeqCst);
        assert_eq!(code_of(&r.call("tv_standby", json!({}))), "expired");
        assert_eq!(r.keys.opens(), 2);
    }

    #[test]
    fn a_paused_assistant_is_refused_everything_even_reads() {
        let r = rig();
        r.store.pause(&r.who.id);
        for (name, args, _) in samples() {
            assert_eq!(code_of(&r.call(name, args)), "paused", "{name}");
        }
        assert_eq!(acted(&r), 0);
        assert_eq!(r.keys.opens(), 0);
        // Resuming (with the remote) puts it back.
        assert!(r.store.resume(&r.who.id, &approval()));
        assert!(is_ok(&r.call("tv_get_power", json!({}))));
    }

    #[test]
    fn after_a_denial_the_next_call_is_refused_as_paused_until_resumed() {
        let r = rig();
        r.all(Level::Ask);
        assert_eq!(
            code_of(&r.ask_and("tv_standby", json!({}), |k| k.tap(IR, 412))),
            "denied"
        );
        assert_eq!(code_of(&r.call("tv_standby", json!({}))), "paused");
        assert_eq!(code_of(&r.call("tv_get_power", json!({}))), "paused");
        assert_eq!(r.keys.opens(), 1, "a paused assistant opens no prompt");
    }

    #[test]
    fn an_unanswered_prompt_makes_the_assistant_wait_before_asking_again() {
        let r = rig();
        r.all(Level::Ask);
        assert_eq!(code_of(&r.call("tv_standby", json!({}))), "expired");
        assert_eq!(code_of(&r.call("tv_standby", json!({}))), "cooldown");
        assert_eq!(r.keys.opens(), 1);
        r.now.fetch_add(10_001, Ordering::SeqCst);
        assert_eq!(code_of(&r.call("tv_standby", json!({}))), "expired");
        assert_eq!(r.keys.opens(), 2);
    }

    #[test]
    fn a_second_request_while_a_prompt_is_open_is_busy() {
        let r = rig();
        r.all(Level::Ask);
        let broker = r.broker.clone();
        let who = r.who.clone();
        let first = thread::spawn(move || broker.call_tool(&who, PEER, "tv_standby", &json!({})));
        let deadline = Instant::now() + Duration::from_secs(5);
        while r.hub.get("ai.prompt").is_none_or(|v| v.is_null()) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        let second = r.call("tv_media_key", json!({"key": "stop"}));
        assert_eq!(code_of(&second), "busy");
        r.keys.tap(IR, 412);
        first.join().unwrap();
        assert_eq!(acted(&r), 0);
    }

    #[test]
    fn the_prompt_says_what_the_tv_calls_things_and_carries_the_assistants_reason_apart() {
        let r = rig();
        r.all(Level::Ask);
        let broker = r.broker.clone();
        let who = r.who.clone();
        let worker = thread::spawn(move || {
            broker.call_tool(
                &who,
                PEER,
                "tv_launch_app",
                &json!({"app_id": "netflix", "reason": "Nitin asked for a movie"}),
            )
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while r.hub.get("ai.prompt").is_none_or(|v| v.is_null()) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        let prompt = r.hub.get("ai.prompt").unwrap();
        assert_eq!(prompt["title"], "Open Netflix");
        assert_eq!(prompt["assistant"], json!({"id": r.who.id, "name": "Cody"}));
        assert_eq!(prompt["reason"], "Nitin asked for a movie");
        assert_eq!(prompt["kind"], "call");
        thread::sleep(Duration::from_millis(60));
        r.keys.tap(IR, 28);
        assert!(is_ok(&worker.join().unwrap()));
        assert_eq!(r.tv.acted(), ["launch netflix"]);
    }

    #[test]
    fn a_target_the_tv_does_not_have_is_refused_before_any_prompt() {
        let r = rig();
        r.all(Level::Ask);
        assert_eq!(
            code_of(&r.call(
                "tv_launch_app",
                json!({"app_id": "org.webosbrew.hbchannel"})
            )),
            "not_found"
        );
        assert_eq!(
            code_of(&r.call("tv_switch_input", json!({"input_id": "HDMI_9"}))),
            "not_found"
        );
        assert_eq!(
            code_of(&r.call("tv_switch_input", json!({"input_id": "HDMI_1"}))),
            "refused"
        );
        assert_eq!(r.keys.opens(), 0, "nothing impossible was asked about");
        assert_eq!(acted(&r), 0);
    }

    #[test]
    fn arguments_are_validated_before_anything_else() {
        let r = rig();
        r.all(Level::Always);
        for (name, args) in [
            ("tv_launch_app", json!({"app_id": "net flix"})),
            ("tv_launch_app", json!({"app_id": "../etc"})),
            ("tv_launch_app", json!({})),
            ("tv_media_key", json!({"key": "home"})),
            ("tv_media_key", json!({"key": "ok"})),
            ("receiver_set_volume", json!({"level": 120})),
            ("tv_standby", json!({"now": true})),
        ] {
            assert_eq!(
                code_of(&r.call(name, args.clone())),
                "invalid_arguments",
                "{name} {args}"
            );
        }
        assert_eq!(code_of(&r.call("tv_shell", json!({}))), "not_found");
        assert_eq!(
            code_of(&r.call("luna_call", json!({"uri": "luna://x"}))),
            "not_found"
        );
        assert_eq!(acted(&r), 0);
        assert_eq!(r.last_audit()["outcome"], "invalid");
    }

    #[test]
    fn volume_is_held_to_the_limit_set_on_the_tv() {
        let r = rig();
        r.all(Level::Always);
        assert_eq!(
            code_of(&r.call("receiver_set_volume", json!({"level": 50.5}))),
            "over_limit"
        );
        assert!(is_ok(&r.call("receiver_set_volume", json!({"level": 50}))));
        assert_eq!(r.receiver.calls.lock().as_slice(), ["volume 50"]);
        // Lowering the limit applies at once; the assistant cannot raise it (no tool does).
        r.store.lower_max_volume(40.0).unwrap();
        assert_eq!(
            code_of(&r.call("receiver_set_volume", json!({"level": 41}))),
            "over_limit"
        );
        assert!(is_ok(&r.call("receiver_set_volume", json!({"level": 40}))));
        // Steps: from 35.5, ten steps up is 40.5, past the limit.
        assert_eq!(
            code_of(&r.call(
                "receiver_change_volume",
                json!({"direction": "up", "steps": 10})
            )),
            "over_limit"
        );
        assert!(is_ok(&r.call(
            "receiver_change_volume",
            json!({"direction": "up", "steps": 9})
        )));
        assert_eq!(r.receiver.calls.lock().last().unwrap(), "volume 40");
        // Down is always allowed, and stops at zero.
        r.hub.set("receiver", json!({"fresh": true, "power": "on", "input": "TV", "volumeAbs": 2.0, "volumeDb": -78.0, "mute": false}));
        assert!(is_ok(&r.call(
            "receiver_change_volume",
            json!({"direction": "down", "steps": 10})
        )));
        assert_eq!(r.receiver.calls.lock().last().unwrap(), "volume 0");
    }

    #[test]
    fn a_volume_step_needs_a_current_reading() {
        let r = rig();
        r.all(Level::Always);
        r.hub
            .set("receiver", json!({"fresh": false, "power": null}));
        assert_eq!(
            code_of(&r.call("receiver_change_volume", json!({"direction": "up"}))),
            "unavailable"
        );
        // An absolute volume does not need one, and a status read says it is stale.
        assert!(is_ok(&r.call("receiver_set_volume", json!({"level": 20}))));
        let ToolOutcome::Ok { data, .. } = r.call("receiver_get_status", json!({})) else {
            panic!()
        };
        assert_eq!(data["fresh"], false);
        assert_eq!(data["limit"], 50.0);
    }

    #[test]
    fn reads_report_what_the_tv_says() {
        let r = rig();
        r.all(Level::Always);
        let ToolOutcome::Ok { data, text } = r.call("tv_get_power", json!({})) else {
            panic!()
        };
        assert_eq!(data["on"], true);
        assert!(text.contains("on"));
        let ToolOutcome::Ok { data, .. } = r.call("tv_get_foreground_app", json!({})) else {
            panic!()
        };
        assert_eq!(data, json!({"appId": "netflix", "title": "Netflix"}));
        let ToolOutcome::Ok { data, .. } = r.call("tv_list_inputs", json!({})) else {
            panic!()
        };
        assert_eq!(
            data["inputs"][1],
            json!({"id": "HDMI_2", "label": "AVR-S760H", "connected": true})
        );
        let ToolOutcome::Ok { data, .. } = r.call("tv_list_apps", json!({})) else {
            panic!()
        };
        assert_eq!(
            data["apps"][0],
            json!({"id": "netflix", "title": "Netflix"})
        );
        let ToolOutcome::Ok { data, .. } = r.call("receiver_get_status", json!({})) else {
            panic!()
        };
        assert_eq!(data["volume"], json!({"level": 35.5, "db": -44.5}));
        assert_eq!(data["mute"], false);
        assert_eq!(data["fresh"], true);
        let ToolOutcome::Ok { data, .. } = r.call("tv_get_now_playing", json!({})) else {
            panic!()
        };
        assert_eq!(data["app"]["title"], "Netflix");
        assert_eq!(data["receiver"]["input"], "TV");
        assert_eq!(data["input"], Value::Null, "Netflix is not an input");
    }

    #[test]
    fn what_is_on_names_the_input_when_an_input_is_in_front() {
        struct OnHdmi(FakeTv);
        impl TvControl for OnHdmi {
            fn foreground_app(&self) -> Result<ForegroundApp, ActionError> {
                Ok(ForegroundApp {
                    app_id: Some("com.webos.app.hdmi2".into()),
                    title: Some("HDMI2".into()),
                })
            }
            fn launchable_apps(&self) -> Result<Vec<AppInfo>, ActionError> {
                self.0.launchable_apps()
            }
            fn inputs(&self) -> Result<Vec<InputInfo>, ActionError> {
                self.0.inputs()
            }
            fn launch_app(&self, a: &str) -> Result<AppInfo, ActionError> {
                self.0.launch_app(a)
            }
            fn switch_input(&self, i: &str) -> Result<InputInfo, ActionError> {
                self.0.switch_input(i)
            }
            fn standby(&self) -> Result<(), ActionError> {
                self.0.standby()
            }
            fn media_key(&self, k: MediaKey) -> Result<(), ActionError> {
                self.0.media_key(k)
            }
        }
        let mut r = rig();
        let broker = Broker::new(BrokerDeps {
            store: r.store.clone(),
            gate: Gate::new(r.keys.clone(), r.hub.clone(), GateConfig::default()),
            hub: r.hub.clone(),
            audit: r.audit.clone(),
            tv: Arc::new(OnHdmi(FakeTv::default())),
            receiver: r.receiver.clone(),
            nudge: Arc::new(|| {}),
            clock: {
                let now = r.now.clone();
                Arc::new(move || now.load(Ordering::SeqCst))
            },
            config: BrokerConfig::default(),
        });
        r.broker = broker;
        let ToolOutcome::Ok { data, .. } = r.call("tv_get_now_playing", json!({})) else {
            panic!()
        };
        assert_eq!(data["input"], json!({"id": "HDMI_2", "label": "AVR-S760H"}));
    }

    #[test]
    fn actions_nudge_the_receiver_reader_and_failures_are_passed_on() {
        let r = rig();
        r.all(Level::Always);
        assert!(is_ok(&r.call("receiver_set_mute", json!({"muted": true}))));
        assert!(is_ok(&r.call("receiver_set_volume", json!({"level": 20}))));
        assert_eq!(r.nudges.load(Ordering::SeqCst), 2);
        r.tv.fail_with(ActionError::new("timeout", "luna-send did not answer"));
        let outcome = r.call("tv_standby", json!({}));
        assert_eq!(code_of(&outcome), "timeout");
        assert_eq!(r.last_audit()["outcome"], "failed");
    }

    #[test]
    fn the_tool_list_is_what_the_assistant_may_use_and_says_which_ones_ask() {
        let r = rig();
        let tools = r.broker.list_tools(&r.who);
        assert_eq!(tools.len(), 13);
        let launch = tools.iter().find(|d| d.name == "tv_launch_app").unwrap();
        assert!(launch.description.contains("Asks for approval on the TV"));
        assert!(
            !tools
                .iter()
                .find(|d| d.name == "tv_get_power")
                .unwrap()
                .description
                .contains("Asks for approval")
        );
        let mut levels = Levels::defaults();
        levels.set(Group::Launch, Level::Never);
        levels.set(Group::Power, Level::Never);
        r.store.lower_levels(&r.who.id, &levels).unwrap();
        let names: Vec<_> = r.broker.list_tools(&r.who).iter().map(|d| d.name).collect();
        assert_eq!(names.len(), 10);
        assert!(
            !names.contains(&"tv_launch_app")
                && !names.contains(&"tv_standby")
                && !names.contains(&"tv_switch_input")
        );
        // Someone who is not paired sees nothing.
        let stranger = AssistantView {
            id: "ffffffffffff".into(),
            name: "x".into(),
        };
        assert!(r.broker.list_tools(&stranger).is_empty());
    }

    #[test]
    fn an_assistant_that_was_unpaired_cannot_call() {
        let r = rig();
        r.store.revoke(&r.who.id);
        assert_eq!(code_of(&r.call("tv_get_power", json!({}))), "unauthorized");
        assert_eq!(r.last_audit()["outcome"], "unauthorized");
    }

    #[test]
    fn a_lock_out_during_a_prompt_withdraws_it_and_nothing_runs() {
        let r = rig();
        r.all(Level::Ask);
        let outcome = r.ask_and("tv_standby", json!({}), |_| {
            r.broker.gate.cancel();
            r.store.lock_out();
        });
        assert!(
            matches!(code_of(&outcome), "cancelled" | "unauthorized"),
            "{outcome:?}"
        );
        assert_eq!(acted(&r), 0);
    }

    #[test]
    fn being_unpaired_while_a_prompt_is_answered_still_stops_the_action() {
        let r = rig();
        r.all(Level::Ask);
        let who = r.who.id.clone();
        let outcome = r.ask_and("tv_standby", json!({}), |k| {
            r.store.revoke(&who);
            k.tap(IR, 28);
        });
        assert_eq!(code_of(&outcome), "unauthorized");
        assert_eq!(acted(&r), 0);
    }

    #[test]
    fn calls_are_rate_limited_per_assistant_and_kind() {
        let r = rig();
        r.all(Level::Always);
        for _ in 0..12 {
            assert!(is_ok(&r.call("tv_media_key", json!({"key": "play"}))));
        }
        assert_eq!(
            code_of(&r.call("tv_media_key", json!({"key": "play"}))),
            "rate_limited"
        );
        // Reads have their own, larger allowance.
        assert!(is_ok(&r.call("tv_get_power", json!({}))));
        r.now.fetch_add(60_000, Ordering::SeqCst);
        assert!(is_ok(&r.call("tv_media_key", json!({"key": "play"}))));
        for _ in 0..60 {
            assert!(is_ok(&r.call("tv_get_power", json!({}))));
        }
        assert_eq!(code_of(&r.call("tv_get_power", json!({}))), "rate_limited");
    }

    #[test]
    fn every_call_is_audited_with_its_outcome() {
        let r = rig();
        r.all(Level::Always);
        r.call("tv_get_power", json!({}));
        r.call("tv_launch_app", json!({"app_id": "netflix"}));
        r.call(
            "tv_media_key",
            json!({"key": "play", "reason": "movie night"}),
        );
        let entries = r.audit.recent(10, Some(&r.who.id));
        let tools: Vec<_> = entries
            .iter()
            .map(|e| e["tool"].as_str().unwrap())
            .collect();
        assert_eq!(tools, ["tv_get_power", "tv_launch_app", "tv_media_key"]);
        assert!(
            entries
                .iter()
                .all(|e| e["outcome"] == "ok" && e["peer"] == "192.168.1.9")
        );
        assert_eq!(entries[2]["args"]["reason"], "movie night");
        assert_eq!(entries[1]["level"], "always");
    }

    #[test]
    fn the_listener_pairing_and_levels_are_out_of_reach_of_every_tool() {
        let r = rig();
        r.all(Level::Always);
        let before_levels = r.store.levels_of(&r.who.id);
        // No tool name or argument reaches the store's widening methods: there is no such tool, and the arguments are checked.
        for (name, args) in [
            ("ai_enable_listener", json!({})),
            ("ai_set_levels", json!({"levels": {"launch": "always"}})),
            ("ai_pair", json!({})),
            ("ai_resume", json!({})),
            (
                "tv_standby",
                json!({"enable_listener": true, "levels": {"power": "always"}}),
            ),
            (
                "tv_launch_app",
                json!({"app_id": "netflix", "listener": true}),
            ),
        ] {
            let outcome = r.call(name, args);
            assert!(
                matches!(code_of(&outcome), "not_found" | "invalid_arguments"),
                "{name}: {outcome:?}"
            );
        }
        assert!(!r.store.listener_enabled());
        assert_eq!(r.store.levels_of(&r.who.id), before_levels);
        assert_eq!(r.store.pairing_view(), None);
    }

    #[test]
    fn pairing_through_the_authenticator_audits_success_and_refusal() {
        let r = rig();
        let view = r.store.open_pairing(&approval());
        let ok = r.broker.pair(&view.code, "Home Assistant", PEER).unwrap();
        assert!(r.broker.authenticate(&ok.token, PEER).is_ok());
        assert_eq!(
            r.broker.authenticate("ltv_nope", PEER),
            Err(AuthFailure::Unknown)
        );
        assert_eq!(r.last_audit()["tool"], "pair");
        assert_eq!(r.last_audit()["outcome"], "ok");
        assert_eq!(
            r.broker.pair("AAAA-AAAA", "x", PEER),
            Err(PairFailure::Refused)
        );
        assert_eq!(r.last_audit()["outcome"], "unauthorized");
    }
}
