//! AI access: an MCP server on the TV, with every permission held on the TV and changed only with the physical remote.
//!
//! Principle 0 (Nitin and his family stay in control) shapes all of it: *a confirmation an AI can reach is not a control*, so
//! nothing an AI (or anything on the network, or a script, or this helper's own loopback API) can do is able to grant, widen or
//! approve its own access. The pieces:
//!
//! | Module | What it is |
//! |---|---|
//! | [`types`] | the vocabulary: groups, levels, the typed [`types::Call`]s, and the traits everything acts through |
//! | [`gate`] | the one place "a person with the physical remote agreed" is decided: it watches the TV's evdev devices itself |
//! | [`store`] | who is paired, the listener switch, the volume limit; widening takes a [`gate::GateApproval`], narrowing never does |
//! | [`levels`] | each assistant's level per group, kept in the settings layer as the protected owner `ai.<id>` |
//! | [`tools`] | the catalogue of tools and the strict parsing of their arguments |
//! | [`broker`] | the pipeline every call goes through: validate, pause, level, prompt, act, audit |
//! | [`audit`] | every call and what became of it |
//! | [`mcp`] | the MCP protocol and the LAN endpoints (`/mcp`, `/pair`) |
//! | [`listener`] | the LAN socket, which exists only while the owner has turned AI access on |
//! | [`keys`], [`luna`], [`receiver_ctl`] | the real remote keys, the real TV (luna-send) and the real receiver |
//! | `routes` | the loopback API the TV's own UI uses (`/v1/ai/...`) |
//!
//! What follows from the shape, and is tested:
//! - **Off by default.** With no state file there is no listener, no socket, no thread but the idle one.
//! - **Only the remote turns it on.** Turning the listener on, opening pairing, giving an assistant more, resuming a paused one
//!   and raising the volume limit each open a prompt on the TV and complete only when the helper itself sees a key from a
//!   physical remote's input device. The loopback API can *ask* for these (the TV shows the prompt) but cannot complete them,
//!   and no route answers a prompt. Keys from the network device (`Smart Remote RCU Input`, which is where Home Assistant's and
//!   this module's own key presses arrive) never count.
//! - **Giving less is always allowed, at once:** turning it off, pausing, revoking, lowering a level, lowering the volume limit,
//!   and the lock-out (every assistant unpaired, the listener off) need no prompt.
//! - **Typed tools only.** There is no tool for files, the shell, luna, the network, the firewall, keys, settings, pairing or
//!   permissions, and the media-key tool can press only the five playback keys.
//!
//! The TV's UI (phase 3) renders the hub topics `ai` and `ai.prompt`, swallows the remote's keys while a prompt is up, and
//! calls the `/v1/ai` routes for the changes the owner makes; it has no way to answer a prompt, because there is none.

pub mod audit;
pub mod broker;
pub mod gate;
pub mod keys;
pub mod levels;
pub mod listener;
pub mod luna;
pub mod mcp;
pub mod receiver_ctl;
pub mod routes;
pub mod store;
pub mod tools;
pub mod types;

use crate::config::AiConfig;
use crate::ctx::Ctx;
use crate::timeutil::now_ms;
use audit::Audit;
use broker::{Broker, BrokerConfig, BrokerDeps};
use gate::{Busy, Decision, Gate, GateApproval, GateConfig, PromptKind, PromptSpec};
use levels::Levels;
use listener::{Listener, ListenerConfig, ListenerStatus};
use mcp::{McpHandler, ServerInfo};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;
use store::{LockOut, Store, StoreError};
use types::{AssistantView, Group, KeySource, ReceiverControl, TvControl};

/// The hub topic of the module's state (`ai.prompt` is the open prompt, published by the gate).
pub const TOPIC: &str = "ai";

/// What the module acts through. The real ones are built from the config; a test gives its own.
pub struct Deps {
    pub keys: Arc<dyn KeySource>,
    pub tv: Arc<dyn TvControl>,
    pub receiver: Arc<dyn ReceiverControl>,
}

/// What a request from the TV's UI came to.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// It was a change that gives less (or nothing), and it is done.
    Done { changed: bool },
    /// It gives more: a prompt is open on the TV and the change happens only if a physical remote approves it.
    Pending { prompt_id: String },
}

/// Why a request from the TV's UI was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Another prompt is open.
    Busy,
    NotFound,
    Invalid(String),
    Io(String),
}

impl From<StoreError> for Refusal {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::NoSuchAssistant => Self::NotFound,
            StoreError::Widens => {
                Self::Invalid("that gives more access and needs the remote".to_owned())
            }
            StoreError::Io(why) | StoreError::Settings(why) => Self::Io(why),
        }
    }
}

pub struct Ai {
    store: Arc<Store>,
    gate: Arc<Gate>,
    audit: Arc<Audit>,
    listener: Arc<Listener>,
    status: Arc<Mutex<ListenerStatus>>,
    port: u16,
}

/// The state document of the `ai` topic.
fn state_doc(store: &Store, status: &ListenerStatus, port: u16) -> Value {
    let pairing = store.pairing_view().map(
        |p| json!({"code": p.code, "expiresAt": p.expires_at, "attemptsLeft": p.attempts_left}),
    );
    json!({
        "listener": {
            "enabled": store.listener_enabled(),
            "bound": status.bound.is_some(),
            "addr": status.bound.map(|a| a.to_string()),
            "port": status.bound.map_or(port, |a| a.port()),
            "since": store.listener_since(),
            "error": status.error,
        },
        "pairing": pairing,
        "assistants": store.assistants().iter().map(store::AssistantInfo::to_json).collect::<Vec<_>>(),
        "limits": {"maxVolume": store.max_volume()},
        "groups": Group::ALL.map(|g| json!({"id": g.id(), "title": g.title(), "default": g.default_level().id()})),
        "tools": tools::catalogue().iter().filter_map(|d| tools::group_of(d.name).map(|g| json!({"name": d.name, "title": d.title, "group": g.id(), "readOnly": d.read_only}))).collect::<Vec<_>>(),
    })
}

impl Ai {
    /// Starts the module: loads the state (off and unpaired when there is none), makes the gate, the broker and the listener
    /// thread (idle while the switch is off), and publishes the topics. Binds nothing.
    pub fn start(
        ctx: &Ctx,
        deps: Deps,
        nudge: Arc<dyn Fn() + Send + Sync>,
    ) -> std::io::Result<Arc<Self>> {
        let config: &AiConfig = &ctx.config.ai;
        ctx.settings.protect_prefix(levels::OWNER_PREFIX);
        let clock: store::Clock = Arc::new(now_ms);
        let dir: PathBuf = ctx.paths.data_dir.join("ai");
        let store = Store::load(dir.clone(), ctx.settings.clone(), clock.clone());
        let audit = Audit::open(dir, ctx.hub.clone());
        let gate = Gate::new(
            deps.keys,
            ctx.hub.clone(),
            GateConfig {
                prompt_ttl: Duration::from_secs(config.prompt_seconds),
                approve_codes: config.approve_codes.clone(),
                deny_codes: config.deny_codes.clone(),
                physical: config.physical_devices.clone(),
                network: config.network_devices.clone(),
                ..GateConfig::default()
            },
        );
        let broker = Broker::new(BrokerDeps {
            store: store.clone(),
            gate: gate.clone(),
            hub: ctx.hub.clone(),
            audit: audit.clone(),
            tv: deps.tv,
            receiver: deps.receiver,
            nudge,
            clock,
            config: BrokerConfig::default(),
        });
        let handler = Arc::new(McpHandler::new(
            broker.clone(),
            broker,
            ServerInfo {
                name: "lumatv".to_owned(),
                title: "LumaTV".to_owned(),
                version: crate::VERSION.to_owned(),
            },
        ));
        let status = Arc::new(Mutex::new(ListenerStatus::default()));
        let weak_store: Weak<Store> = Arc::downgrade(&store);
        let publish: Arc<dyn Fn() + Send + Sync> = {
            let (hub, status, port) = (ctx.hub.clone(), status.clone(), config.port);
            Arc::new(move || {
                if let Some(store) = weak_store.upgrade() {
                    let current = status.lock().clone();
                    hub.set(TOPIC, state_doc(&store, &current, port));
                }
            })
        };
        let peers = config
            .allowed_nets
            .iter()
            .filter_map(|n| crate::http::Cidr::parse(n))
            .collect();
        let bind = config
            .bind
            .as_deref()
            .and_then(|b| b.parse::<Ipv4Addr>().ok());
        let listener = Listener::start(
            store.clone(),
            handler,
            ListenerConfig {
                port: config.port,
                bind,
                peers,
            },
            status.clone(),
            &ctx.stop,
            publish.clone(),
        )?;
        let weak_listener: Arc<OnceLock<Weak<Listener>>> = Arc::new(OnceLock::new());
        let _ = weak_listener.set(Arc::downgrade(&listener));
        store.on_change({
            let publish = publish.clone();
            move || {
                publish();
                if let Some(listener) = weak_listener.get().and_then(Weak::upgrade) {
                    listener.reconcile();
                }
            }
        });
        let ai = Arc::new(Self {
            store,
            gate: gate.clone(),
            audit,
            listener,
            status,
            port: config.port,
        });
        publish();
        {
            let gate = gate;
            ctx.stop.on_stop(move || {
                gate.cancel();
            });
        }
        Ok(ai)
    }

    /// Stops the listener (closing its socket) and withdraws any open prompt.
    pub fn stop(&self) {
        self.gate.cancel();
        self.listener.stop();
    }

    /// The `ai` topic's document, as of now.
    pub fn doc(&self) -> Value {
        state_doc(&self.store, &self.status.lock().clone(), self.port)
    }

    pub fn recent_audit(&self, limit: usize, assistant: Option<&str>) -> Vec<Value> {
        self.audit.recent(limit, assistant)
    }

    /// Opens a prompt for a change that gives more, and applies the change when (and only when) a physical remote approves it.
    /// The caller does not wait: the change shows up in the `ai` topic.
    fn gated(
        &self,
        spec: PromptSpec,
        apply: impl FnOnce(&GateApproval) + Send + 'static,
    ) -> Result<Action, Refusal> {
        let pending = self.gate.begin(spec).map_err(|Busy| Refusal::Busy)?;
        let prompt_id = pending.id().to_owned();
        let spawned = std::thread::Builder::new()
            .name("ai-gate".to_owned())
            .stack_size(256 * 1024)
            .spawn(move || {
                if let Decision::Approved(approval) = pending.wait().decision {
                    apply(&approval);
                }
            });
        spawned.map_err(|e| Refusal::Io(e.to_string()))?;
        Ok(Action::Pending { prompt_id })
    }

    fn view_of(&self, id: &str) -> Option<AssistantView> {
        self.store.assistant(id).map(|a| AssistantView {
            id: a.id,
            name: a.name,
        })
    }

    /// Turns the listener off at once, or asks (on the TV) to turn it on.
    pub fn set_listener(&self, enabled: bool) -> Result<Action, Refusal> {
        if !enabled {
            return Ok(Action::Done {
                changed: self.store.disable_listener(),
            });
        }
        if self.store.listener_enabled() {
            return Ok(Action::Done { changed: false });
        }
        let store = self.store.clone();
        self.gated(
            PromptSpec {
                kind: PromptKind::EnableListener,
                assistant: None,
                title: "Turn on AI access?".to_owned(),
                detail: "AI assistants on your home network will be able to connect to this TV and use the tools you allow them. Press OK to turn it on, Back to cancel.".to_owned(),
                reason: None,
            },
            move |approval| {
                if let Err(e) = store.enable_listener(approval) {
                    crate::log_error!("ai: could not turn the listener on ({e})");
                }
            },
        )
    }

    /// Asks (on the TV) to open a pairing window; the code appears in the `ai` topic once a physical remote approves.
    pub fn start_pairing(&self) -> Result<Action, Refusal> {
        let store = self.store.clone();
        self.gated(
            PromptSpec {
                kind: PromptKind::StartPairing,
                assistant: None,
                title: "Pair an AI assistant?".to_owned(),
                detail: "A one-time code will be shown on this screen. Give it only to an assistant you trust. Press OK to show it, Back to cancel.".to_owned(),
                reason: None,
            },
            move |approval| {
                store.open_pairing(approval);
            },
        )
    }

    pub fn cancel_pairing(&self) -> bool {
        self.store.close_pairing()
    }

    /// Lowers an assistant's levels at once, or asks (on the TV) to give it more.
    pub fn set_levels(&self, id: &str, new: Levels) -> Result<Action, Refusal> {
        let current = self.store.levels_of(id).ok_or(Refusal::NotFound)?;
        if !current.widened_by(&new) {
            if current == new {
                return Ok(Action::Done { changed: false });
            }
            self.store.lower_levels(id, &new)?;
            return Ok(Action::Done { changed: true });
        }
        let who = self.view_of(id).ok_or(Refusal::NotFound)?;
        let changes: Vec<String> = current
            .changes_to(&new)
            .iter()
            .map(|(g, old, now)| format!("{}: {} to {}", g.title(), old.title(), now.title()))
            .collect();
        let (store, id) = (self.store.clone(), id.to_owned());
        self.gated(
            PromptSpec {
                kind: PromptKind::WidenLevels,
                assistant: Some(who.clone()),
                title: format!("Give {} more access?", who.name),
                detail: format!(
                    "{}. Press OK to allow it, Back to cancel.",
                    changes.join("; ")
                ),
                reason: None,
            },
            move |approval| {
                if let Err(e) = store.widen_levels(&id, &new, approval) {
                    crate::log_error!("ai: could not change an assistant's permissions ({e})");
                }
            },
        )
    }

    pub fn pause(&self, id: &str) -> Result<Action, Refusal> {
        self.store.assistant(id).ok_or(Refusal::NotFound)?;
        Ok(Action::Done {
            changed: self.store.pause(id),
        })
    }

    /// Asks (on the TV) to let a paused assistant call again.
    pub fn resume(&self, id: &str) -> Result<Action, Refusal> {
        let who = self.view_of(id).ok_or(Refusal::NotFound)?;
        if self.store.is_paused(id) != Some(true) {
            return Ok(Action::Done { changed: false });
        }
        let (store, id) = (self.store.clone(), id.to_owned());
        self.gated(
            PromptSpec {
                kind: PromptKind::Resume,
                assistant: Some(who.clone()),
                title: format!("Let {} use the TV again?", who.name),
                detail: "It was paused after a request was denied. Press OK to resume it, Back to leave it paused.".to_owned(),
                reason: None,
            },
            move |approval| {
                store.resume(&id, approval);
            },
        )
    }

    /// Unpairs one assistant at once.
    pub fn revoke(&self, id: &str) -> Result<Action, Refusal> {
        if self.store.revoke(id) {
            Ok(Action::Done { changed: true })
        } else {
            Err(Refusal::NotFound)
        }
    }

    /// Lowers the receiver volume limit at once, or asks (on the TV) to raise it.
    pub fn set_max_volume(&self, level: f64) -> Result<Action, Refusal> {
        let current = self.store.max_volume();
        if level <= current {
            if (level - current).abs() < f64::EPSILON {
                return Ok(Action::Done { changed: false });
            }
            self.store.lower_max_volume(level)?;
            return Ok(Action::Done { changed: true });
        }
        let store = self.store.clone();
        self.gated(
            PromptSpec {
                kind: PromptKind::RaiseLimit,
                assistant: None,
                title: format!("Raise the volume limit to {level}?"),
                detail: format!("AI assistants will be able to set the receiver up to {level} (now {current}). Press OK to allow it, Back to cancel."),
                reason: None,
            },
            move |approval| {
                if let Err(e) = store.raise_max_volume(level, approval) {
                    crate::log_error!("ai: could not raise the volume limit ({e})");
                }
            },
        )
    }

    /// Everything off now: every assistant unpaired, the listener off, pairing closed, any prompt withdrawn.
    pub fn lock_out(&self) -> LockOut {
        let result = self.store.lock_out();
        self.gate.cancel();
        crate::log_warn!("ai: locked out ({} assistant(s) unpaired)", result.revoked);
        result
    }
}

/// Inert stand-ins for the TV, the receiver and the remote, for tests of other modules that need an [`Ai`] but not its actions.
#[cfg(test)]
pub(crate) mod testing {
    use super::types::{ActionError, AppInfo, ForegroundApp, InputInfo, KeySession, MediaKey};
    use super::*;

    struct NoKeys;
    struct NoSession;
    struct NoTv;
    struct NoReceiver;

    impl KeySession for NoSession {
        fn next(&mut self, timeout: Duration) -> Option<types::KeyEvent> {
            std::thread::sleep(timeout.min(Duration::from_millis(5)));
            None
        }
    }

    impl KeySource for NoKeys {
        fn open(&self) -> Box<dyn KeySession> {
            Box::new(NoSession)
        }
    }

    fn unavailable<T>() -> Result<T, ActionError> {
        Err(ActionError::new("unavailable", "not in a test"))
    }

    impl TvControl for NoTv {
        fn foreground_app(&self) -> Result<ForegroundApp, ActionError> {
            unavailable()
        }
        fn launchable_apps(&self) -> Result<Vec<AppInfo>, ActionError> {
            unavailable()
        }
        fn inputs(&self) -> Result<Vec<InputInfo>, ActionError> {
            unavailable()
        }
        fn launch_app(&self, _: &str) -> Result<AppInfo, ActionError> {
            unavailable()
        }
        fn switch_input(&self, _: &str) -> Result<InputInfo, ActionError> {
            unavailable()
        }
        fn standby(&self) -> Result<(), ActionError> {
            unavailable()
        }
        fn media_key(&self, _: MediaKey) -> Result<(), ActionError> {
            unavailable()
        }
    }

    impl ReceiverControl for NoReceiver {
        fn set_volume(&self, _: f64) -> Result<(), ActionError> {
            unavailable()
        }
        fn set_mute(&self, _: bool) -> Result<(), ActionError> {
            unavailable()
        }
    }

    pub fn inert_deps() -> Deps {
        Deps {
            keys: Arc::new(NoKeys),
            tv: Arc::new(NoTv),
            receiver: Arc::new(NoReceiver),
        }
    }
}
