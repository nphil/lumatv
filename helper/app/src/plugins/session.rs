//! One plugin's conversation with the helper, without a socket in it: messages in, replies out, topics published.
//!
//! What a plugin says is checked by `plugin-api`'s own conversions (`Plan`, `Context`), which build the records of the service
//! contract through their `validate()`, so a card, a tile or a notification the TV draws is one the same checks passed on the Show.
//! The TV then publishes the plugin's own wire message (that is what QML needs) under topics `plugin.<id>.<name>`:
//!
//! | Topic | Value |
//! |---|---|
//! | `plugins` | `{"plugins": [{id, name, version, class, state: "connected" \| "waiting" \| "inert", registered, reason?}], "problem"}` |
//! | `plugin.<id>.cards` | `{"items": [{id, revision, createdAt, expiresAt, card}]}`, `card` is the plugin's `card.put` |
//! | `plugin.<id>.tiles` | `{"items": [<tile.put>]}` |
//! | `plugin.<id>.notifications` | `{"items": [{id, channel, revision, receivedAt, expiresAt, post}]}`; events `posted` and `ended` |
//! | `plugin.<id>.facts` | `{"facts": {key: line}}` |
//! | `plugin.<id>` | events only: `settings_rejected`, `action_result` |
//!
//! A plugin's settings page is registered in the settings store under its id, so `/v1/settings/<id>` serves it and a change of the
//! person's reaches the plugin as `settings.apply`. What this TV does not serve at level 1 (timeline sources, choice providers,
//! secrets, images, surfaces, Home Assistant, network, signals) is refused in words (`not_served`); a plugin that asks for it in its
//! manifest does not get to connect (`roster`).

use super::roster::{Member, Roster};
use crate::hub::Hub;
use crate::settings::SettingsStore;
use parking_lot::Mutex;
use plugin_api::MAX_FRAME;
use plugin_api::convert::{Context, Plan};
use plugin_api::level::{self, LevelRefusal};
use plugin_api::limits::MESSAGES_PER_S;
use plugin_api::permission::Permission;
use plugin_api::wire::{
    ActionOutcome, Authority, CardPut, Hello, Importance, NotifyPost, Priority, Register,
    SettingsOutcome, TileWire, ToBase, ToPlugin, Welcome, WelcomeLimits,
};
use serde_json::{Value as Json, json};
use service_api::UtcInstant;
use settings_schema::{FieldKind, SettingsSchema};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub const MAX_CARDS: usize = service_api::limits::MAX_CARDS_PER_OWNER;
pub const MAX_TILES: usize = service_api::limits::MAX_QUICK_TILES_PER_OWNER;
pub const MAX_NOTES: usize = service_api::limits::MAX_NOTIFICATIONS_PER_OWNER;
pub const MAX_FACTS: usize = 16;
pub const MAX_FACT_LINE: usize = 160;
/// Settings transactions and actions waiting for an answer, at most, per plugin.
const MAX_OUTSTANDING: usize = plugin_api::limits::MAX_OUTSTANDING;

/// Where the facts about this device come from (`device.rs`): asked each time a plugin says hello, because a controller comes and
/// goes.
pub type DeviceSource = Arc<dyn Fn() -> Json + Send + Sync>;

/// What goes on the wire for `message`: the contract's own JSON, and for `welcome` the device facts as the additive field
/// `device`. LumaShow's plugin API changes only by addition inside a level (a message may gain optional fields and a receiver
/// skips one it does not know), so a plugin built on the shared crate as it is reads the welcome it always did, and one that wants
/// to adapt to the device finds the facts beside it.
pub fn wire_body(message: &ToPlugin, device: Option<Json>) -> Vec<u8> {
    match (message, device) {
        (ToPlugin::Welcome(_), Some(device)) => {
            let mut value = serde_json::to_value(message).expect("plain data serializes");
            if let Some(map) = value.as_object_mut() {
                map.insert("device".to_owned(), device);
            }
            serde_json::to_vec(&value).expect("plain data serializes")
        }
        _ => serde_json::to_vec(message).expect("plain data serializes"),
    }
}

/// How the helper speaks to a plugin from any thread. `false`: the plugin cannot be reached (the connection is going away).
pub trait Outlet: Send + Sync {
    fn send(&self, message: ToPlugin) -> bool;
    /// Wakes the connection's own thread: something with a deadline (a question to the plugin) was added behind its back.
    fn poke(&self);
}

/// What one message of a plugin makes the connection do.
#[derive(Debug, Default)]
pub struct Reply {
    pub messages: Vec<ToPlugin>,
    pub hangup: bool,
}

impl Reply {
    fn one(message: ToPlugin) -> Self {
        Self {
            messages: vec![message],
            hangup: false,
        }
    }

    fn nothing() -> Self {
        Self::default()
    }

    fn refused(what: &str, code: &str, reason: impl std::fmt::Display) -> Self {
        Self::one(ToPlugin::Refused {
            what: what.to_owned(),
            code: code.to_owned(),
            reason: reason.to_string(),
        })
    }

    fn fatal(code: &str, message: impl std::fmt::Display) -> Self {
        Self {
            messages: vec![ToPlugin::Error {
                code: code.to_owned(),
                message: message.to_string(),
                fatal: true,
            }],
            hangup: true,
        }
    }
}

/// What every connection of the broker shares.
pub struct Shared {
    pub hub: Arc<Hub>,
    pub settings: Arc<SettingsStore>,
    now_ms: Arc<dyn Fn() -> u64 + Send + Sync>,
    answer: Duration,
    /// Settings owners that are plugins' pages (a plugin may not take over an owner of the helper).
    page_owners: Mutex<BTreeSet<String>>,
    live: Mutex<BTreeMap<String, Arc<Live>>>,
    counter: AtomicU64,
    roster: Mutex<RosterView>,
    device: Mutex<Option<DeviceSource>>,
}

#[derive(Default)]
struct RosterView {
    members: Vec<(String, String, String, String)>,
    inert: Vec<(String, String)>,
    problem: Option<String>,
}

/// What other threads need of a registered connection: where to speak to the plugin, and what waits for an answer.
pub struct Live {
    pub conn: u64,
    outlet: Arc<dyn Outlet>,
    actions: Mutex<Vec<String>>,
    /// op -> (action, deadline in ms)
    ops: Mutex<BTreeMap<u64, (String, u64)>>,
    txns: Mutex<VecDeque<u64>>,
}

/// Why an action could not be sent to a plugin.
#[derive(Debug, PartialEq, Eq)]
pub enum InvokeError {
    NoSuchPlugin,
    NoSuchAction,
    TooMany,
    Unreachable,
}

impl InvokeError {
    pub fn words(&self) -> &'static str {
        match self {
            Self::NoSuchPlugin => "that plugin is not connected",
            Self::NoSuchAction => "that plugin declared no such action",
            Self::TooMany => "that plugin has too many requests waiting",
            Self::Unreachable => "that plugin cannot be reached",
        }
    }
}

fn topic(id: &str, name: &str) -> String {
    if name.is_empty() {
        format!("plugin.{id}")
    } else {
        format!("plugin.{id}.{name}")
    }
}

impl Shared {
    pub fn new(
        hub: Arc<Hub>,
        settings: Arc<SettingsStore>,
        answer: Duration,
        now_ms: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Arc<Self> {
        Arc::new(Self {
            hub,
            settings,
            now_ms,
            answer,
            page_owners: Mutex::new(BTreeSet::new()),
            live: Mutex::new(BTreeMap::new()),
            counter: AtomicU64::new(1),
            roster: Mutex::new(RosterView::default()),
            device: Mutex::new(None),
        })
    }

    pub fn now_ms(&self) -> u64 {
        (self.now_ms)()
    }

    /// Says where the device facts for `welcome` come from.
    pub fn set_device_source(&self, source: DeviceSource) {
        *self.device.lock() = Some(source);
    }

    /// The device facts now, if a source was given.
    pub fn device_facts(&self) -> Option<Json> {
        let source = self.device.lock().clone()?;
        Some(source())
    }

    fn next(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::Relaxed)
    }

    /// Records what the roster is (and why a plugin is inert) and publishes the `plugins` topic.
    pub fn set_roster(&self, roster: &Roster, problem: Option<String>) {
        {
            let mut view = self.roster.lock();
            view.members = roster
                .members
                .iter()
                .map(|m| {
                    (
                        m.id.clone(),
                        m.manifest.name.clone(),
                        m.manifest.version.clone(),
                        m.manifest.class.name().to_owned(),
                    )
                })
                .collect();
            view.inert = roster
                .inert
                .iter()
                .map(|i| (i.id.clone(), i.reason.clone()))
                .collect();
            view.problem = problem;
        }
        self.publish_plugins();
    }

    fn publish_plugins(&self) {
        // A connection is in `live` only once it has registered (see `Session::register`).
        let live: BTreeSet<String> = self.live.lock().keys().cloned().collect();
        let view = self.roster.lock();
        let mut plugins: Vec<Json> = view
            .members
            .iter()
            .map(|(id, name, version, class)| {
                let connected = live.contains(id);
                let state = if connected { "connected" } else { "waiting" };
                json!({"id": id, "name": name, "version": version, "class": class, "state": state,
                       "registered": connected})
            })
            .collect();
        plugins.extend(view.inert.iter().map(|(id, reason)| {
            json!({"id": id, "state": "inert", "registered": false, "reason": reason})
        }));
        self.hub.set(
            "plugins",
            json!({"plugins": plugins, "problem": view.problem}),
        );
    }

    /// Asks a connected plugin to run one of the actions it declared (a tile was tapped, a button of its page pressed). The
    /// answer comes later as an `action_result` event; one that does not come in time is reported as a failure.
    pub fn invoke_action(
        &self,
        plugin: &str,
        action: &str,
        target: Option<String>,
    ) -> Result<u64, InvokeError> {
        let live = self
            .live
            .lock()
            .get(plugin)
            .cloned()
            .ok_or(InvokeError::NoSuchPlugin)?;
        if !live.actions.lock().iter().any(|a| a == action) {
            return Err(InvokeError::NoSuchAction);
        }
        let op = self.next();
        {
            let mut ops = live.ops.lock();
            if ops.len() >= MAX_OUTSTANDING {
                return Err(InvokeError::TooMany);
            }
            ops.insert(
                op,
                (
                    action.to_owned(),
                    self.now_ms() + self.answer.as_millis() as u64,
                ),
            );
        }
        let sent = live.outlet.send(ToPlugin::ActionInvoke {
            op,
            action: action.to_owned(),
            target,
        });
        if sent {
            live.outlet.poke();
            Ok(op)
        } else {
            live.ops.lock().remove(&op);
            Err(InvokeError::Unreachable)
        }
    }

    /// Sends the plugin the values the person has chosen. A change in the settings store comes here.
    fn send_settings(&self, plugin: &str, authority: Authority) {
        let Some(live) = self.live.lock().get(plugin).cloned() else {
            return;
        };
        let Some(values) = self.settings.effective(plugin) else {
            return;
        };
        let txn = self.next();
        {
            let mut txns = live.txns.lock();
            if txns.len() >= MAX_OUTSTANDING {
                txns.pop_front();
            }
            txns.push_back(txn);
        }
        live.outlet.send(ToPlugin::SettingsApply {
            txn,
            authority,
            values,
        });
    }

    /// Plugin ids with a registered connection.
    pub fn connected(&self) -> Vec<String> {
        self.live.lock().keys().cloned().collect()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Fresh,
    Welcomed,
    Registered,
}

struct CardEntry {
    revision: u64,
    created: i64,
    expires: Option<i64>,
    put: CardPut,
}

struct NoteEntry {
    revision: u64,
    received: i64,
    expires: Option<i64>,
    post: NotifyPost,
}

pub struct Session {
    member: Member,
    shared: Arc<Shared>,
    conn: u64,
    outlet: Arc<dyn Outlet>,
    phase: Phase,
    live: Option<Arc<Live>>,
    context: Option<Context>,
    page: bool,
    cards: BTreeMap<String, CardEntry>,
    tiles: BTreeMap<String, TileWire>,
    notes: BTreeMap<String, NoteEntry>,
    facts: BTreeMap<String, String>,
    declared_actions: Vec<String>,
}

fn secs(ms: u64) -> i64 {
    (ms / 1000) as i64
}

/// What a plugin's page may not use on this TV, in words (or nothing).
fn page_problem(schema: &SettingsSchema, declared_actions: Option<&[String]>) -> Option<String> {
    for field in schema.fields() {
        if field.key.as_str().starts_with("lease.") {
            return Some(format!(
                "{}: leases are not served by this TV",
                field.key.as_str()
            ));
        }
        match &field.kind {
            FieldKind::Text { secret: true, .. } => {
                return Some(format!(
                    "{}: secret fields are not served by this TV yet",
                    field.key.as_str()
                ));
            }
            FieldKind::Action { action, .. } => {
                if let Some(declared) = declared_actions
                    && !declared.iter().any(|d| d == action.as_str())
                {
                    return Some(format!(
                        "{}: the action {} was not declared at registration",
                        field.key.as_str(),
                        action.as_str()
                    ));
                }
            }
            _ => {}
        }
    }
    None
}

impl Session {
    pub fn new(member: Member, shared: Arc<Shared>, conn: u64, outlet: Arc<dyn Outlet>) -> Self {
        Self {
            member,
            shared,
            conn,
            outlet,
            phase: Phase::Fresh,
            live: None,
            context: None,
            page: false,
            cards: BTreeMap::new(),
            tiles: BTreeMap::new(),
            notes: BTreeMap::new(),
            facts: BTreeMap::new(),
            declared_actions: Vec::new(),
        }
    }

    pub fn id(&self) -> &str {
        &self.member.id
    }

    pub fn messages_per_s(&self) -> u32 {
        MESSAGES_PER_S
    }

    fn needs(&self, permission: Permission, what: &str) -> Result<(), Reply> {
        if self.member.manifest.permissions.contains(&permission) {
            Ok(())
        } else {
            Err(Reply::refused(
                what,
                "not_permitted",
                format!(
                    "this plugin did not ask for the {permission} permission, so it was not approved"
                ),
            ))
        }
    }

    pub fn hello(&mut self, hello: Hello) -> Reply {
        if self.phase != Phase::Fresh {
            return Reply::fatal("hello_twice", "hello was already said on this connection");
        }
        let api = match level::negotiate(hello.min, hello.target) {
            Ok(api) => api,
            Err(refusal) => {
                let code = match refusal {
                    LevelRefusal::BaseTooOld { .. } => "api_too_new",
                    LevelRefusal::PluginTooOld { .. } => "api_too_old",
                    LevelRefusal::Malformed { .. } => "api_malformed",
                };
                return Reply::fatal(code, refusal);
            }
        };
        let manifest = &self.member.manifest;
        let mut features: Vec<String> = [
            (Permission::Settings, "settings:1"),
            (Permission::Cards, "cards:1"),
            (Permission::Tiles, "tiles:1"),
            (Permission::Notify, "notify:1"),
        ]
        .iter()
        .filter(|(p, _)| manifest.permissions.contains(p))
        .map(|(_, word)| (*word).to_owned())
        .collect();
        if manifest.permissions.contains(&Permission::Settings) {
            features.push("facts:1".to_owned());
        }
        self.phase = Phase::Welcomed;
        Reply::one(ToPlugin::Welcome(Welcome {
            api,
            id: self.member.id.clone(),
            class: manifest.class,
            features,
            permissions: manifest
                .permission_order
                .iter()
                .map(Permission::to_string)
                .collect(),
            limits: WelcomeLimits {
                max_frame: MAX_FRAME as u32,
                messages_per_s: MESSAGES_PER_S,
                max_assets: 0,
                max_asset_bytes: 0,
            },
        }))
    }

    /// What the plugin said.
    pub fn message(&mut self, message: ToBase) -> Reply {
        let name = message.name();
        match message {
            ToBase::Hello(hello) => return self.hello(hello),
            ToBase::Bye { .. } => {
                return Reply {
                    messages: Vec::new(),
                    hangup: true,
                };
            }
            _ if self.phase == Phase::Fresh => {
                return Reply::fatal("hello_first", "the first message must be hello");
            }
            ToBase::Register(register) => return self.register(register),
            _ if self.phase != Phase::Registered => {
                return Reply::refused(
                    name,
                    "not_registered",
                    "register first: nothing is accepted before the declaration",
                );
            }
            _ => {}
        }
        match message {
            ToBase::SettingsPage { schema } => self.settings_page(schema),
            ToBase::SettingsResult { txn, outcome } => self.settings_result(txn, outcome),
            ToBase::ActionResult { op, outcome } => self.action_result(op, outcome),
            ToBase::CardPut(put) => self.card_put(put),
            ToBase::CardRemove { id } => {
                if self.cards.remove(&id).is_some() {
                    self.publish_cards();
                }
                Reply::nothing()
            }
            ToBase::TilePut(tile) => self.tile_put(tile),
            ToBase::TileRemove { id } => {
                if self.tiles.remove(&id).is_some() {
                    self.publish_tiles();
                }
                Reply::nothing()
            }
            ToBase::NotifyPost(post) => self.notify_post(post),
            ToBase::NotifyEnd { id } => {
                if self.notes.remove(&id).is_some() {
                    self.publish_notes();
                    self.shared.hub.emit(
                        &topic(self.id(), "notifications"),
                        "ended",
                        json!({"id": id}),
                    );
                }
                Reply::nothing()
            }
            ToBase::FactsPut { facts } => self.facts_put(facts),
            ToBase::Hello(_) | ToBase::Register(_) | ToBase::Bye { .. } => Reply::nothing(),
            // Everything else belongs to a surface this TV does not serve at level 1.
            other => Reply::refused(
                other.name(),
                "not_served",
                "this TV does not serve that at plugin API level 1 (it serves settings, cards, tiles, notifications and facts)",
            ),
        }
    }

    fn register(&mut self, register: Register) -> Reply {
        if self.phase == Phase::Registered {
            return Reply::refused(
                "register",
                "registered",
                "this plugin is already registered on this connection",
            );
        }
        if !register.sources.is_empty() {
            return Reply::refused(
                "register",
                "not_served",
                "timeline sources are not served by this TV",
            );
        }
        let needs = [
            (register.settings.is_some(), Permission::Settings),
            (!register.channels.is_empty(), Permission::Notify),
            (!register.tiles.is_empty(), Permission::Tiles),
        ];
        for (declared, permission) in needs {
            if declared && let Err(reply) = self.needs(permission, "register") {
                return reply;
            }
        }
        if register
            .channels
            .iter()
            .any(|c| c.importance == Importance::Urgent)
        {
            return Reply::refused(
                "register",
                "not_permitted",
                "a plugin's channel may be at most high: urgent is for the base",
            );
        }
        let declared_actions: Vec<String> = register
            .actions
            .iter()
            .map(|a| a.key.clone())
            .chain(
                register
                    .settings
                    .iter()
                    .flat_map(SettingsSchema::fields)
                    .filter_map(|f| match &f.kind {
                        FieldKind::Action { action, .. } => Some(action.as_str().to_owned()),
                        _ => None,
                    }),
            )
            .collect();
        if let Some(schema) = &register.settings
            && let Some(why) = page_problem(schema, None)
        {
            return Reply::refused("register", "not_served", why);
        }
        let key = self.member.manifest.owner_key();
        let tiles = register.tiles.clone();
        let has_page = register.settings.is_some();
        let schema = register.settings.clone();
        let plan = match Plan::new(register, &key) {
            Ok(plan) => plan,
            Err(e) => return Reply::refused("register", "invalid", e),
        };
        if !plan.declared().providers.is_empty() {
            return Reply::refused(
                "register",
                "not_served",
                "choice providers are not served by this TV",
            );
        }
        let owner = match service_api::OwnerTable::new().register(key.clone()) {
            Ok(owner) => owner,
            Err(e) => return Reply::refused("register", "refused", e),
        };
        if let Err(e) = plan.contributions(owner) {
            return Reply::refused("register", "invalid", e);
        }
        let context = Context {
            owner,
            plugin: key,
            declared: plan.declared().clone(),
        };

        let id = self.member.id.clone();
        let live = Arc::new(Live {
            conn: self.conn,
            outlet: self.outlet.clone(),
            actions: Mutex::new(declared_actions.clone()),
            ops: Mutex::new(BTreeMap::new()),
            txns: Mutex::new(VecDeque::new()),
        });
        {
            // Taking over from an earlier connection of the same plugin and publishing happen together, so that the earlier one's
            // cleanup (which checks it is still the live one) cannot interleave.
            let mut table = self.shared.live.lock();
            if let Some(schema) = schema {
                let mut pages = self.shared.page_owners.lock();
                let taken = self.shared.settings.owners().iter().any(|o| o.owner == id);
                if taken && !pages.contains(&id) {
                    return Reply::refused(
                        "register",
                        "refused",
                        format!("the settings owner {id} belongs to the helper"),
                    );
                }
                self.shared.settings.retire(&id);
                if let Err(e) = self.shared.settings.register(schema, None) {
                    return Reply::refused("register", "invalid", e.0);
                }
                pages.insert(id.clone());
                let shared = Arc::downgrade(&self.shared);
                let plugin = id.clone();
                self.shared.settings.on_change(&id, move || {
                    if let Some(shared) = shared.upgrade() {
                        shared.send_settings(&plugin, Authority::User);
                    }
                });
            }
            table.insert(id.clone(), live.clone());
        }
        self.live = Some(live);
        self.context = Some(context);
        self.page = has_page;
        self.declared_actions = declared_actions;
        self.phase = Phase::Registered;
        for tile in tiles {
            self.tiles.insert(tile.id.clone(), tile);
        }
        self.publish_all();
        self.shared.publish_plugins();
        let mut reply = Reply::one(ToPlugin::Registered { owner: id.clone() });
        if self.page {
            // Bring the plugin to the values the person chose before it started (`reconcile`: converge, repeat nothing).
            if let Some(values) = self.shared.settings.effective(&id) {
                let txn = self.shared.next();
                if let Some(live) = &self.live {
                    live.txns.lock().push_back(txn);
                }
                reply.messages.push(ToPlugin::SettingsApply {
                    txn,
                    authority: Authority::Reconcile,
                    values,
                });
            }
        }
        reply
    }

    fn settings_page(&mut self, schema: SettingsSchema) -> Reply {
        if let Err(reply) = self.needs(Permission::Settings, "settings.page") {
            return reply;
        }
        if !self.page {
            return Reply::refused(
                "settings.page",
                "invalid",
                "this plugin registered without a settings page",
            );
        }
        if schema.owner != self.member.manifest.owner_key() {
            return Reply::refused(
                "settings.page",
                "invalid",
                "the page's owner is not this plugin",
            );
        }
        if let Some(why) = page_problem(&schema, Some(&self.declared_actions)) {
            return Reply::refused("settings.page", "not_served", why);
        }
        match self.shared.settings.register(schema, None) {
            Ok(()) => Reply::nothing(),
            Err(e) => Reply::refused("settings.page", "refused", e.0),
        }
    }

    fn settings_result(&mut self, txn: u64, outcome: SettingsOutcome) -> Reply {
        let known = self.live.as_ref().is_some_and(|live| {
            let mut txns = live.txns.lock();
            match txns.iter().position(|t| *t == txn) {
                Some(at) => {
                    txns.remove(at);
                    true
                }
                None => false,
            }
        });
        if !known {
            return Reply::refused(
                "settings.result",
                "unknown_txn",
                format!("no settings.apply {txn} is waiting for an answer"),
            );
        }
        match outcome {
            SettingsOutcome::Applied { .. } => {}
            SettingsOutcome::Rejected { errors, .. } => {
                self.shared.hub.emit(
                    &topic(self.id(), ""),
                    "settings_rejected",
                    json!({"txn": txn, "errors": errors}),
                );
            }
            SettingsOutcome::RestartRequired { .. } => {
                self.shared.hub.emit(
                    &topic(self.id(), ""),
                    "settings_restart_required",
                    json!({"txn": txn}),
                );
            }
        }
        Reply::nothing()
    }

    fn action_result(&mut self, op: u64, outcome: ActionOutcome) -> Reply {
        let action = self
            .live
            .as_ref()
            .and_then(|live| live.ops.lock().remove(&op))
            .map(|(action, _)| action);
        let Some(action) = action else {
            return Reply::refused(
                "action.result",
                "unknown_op",
                format!("no action.invoke {op} is waiting for an answer"),
            );
        };
        let outcome = match outcome {
            ActionOutcome::Ok => json!({"type": "ok"}),
            ActionOutcome::Failed { reason } => json!({"type": "failed", "reason": reason}),
        };
        self.shared.hub.emit(
            &topic(self.id(), ""),
            "action_result",
            json!({"op": op, "action": action, "outcome": outcome}),
        );
        Reply::nothing()
    }

    fn context(&self) -> &Context {
        self.context
            .as_ref()
            .expect("a registered session has a context")
    }

    fn card_put(&mut self, put: CardPut) -> Reply {
        if let Err(reply) = self.needs(Permission::Cards, "card.put") {
            return reply;
        }
        if put.priority == Priority::Ringing {
            return Reply::refused(
                "card.put",
                "not_permitted",
                "a plugin's card may be at most running: ringing is for the base",
            );
        }
        let existing = self.cards.get(&put.id);
        if existing.is_none() && self.cards.len() >= MAX_CARDS {
            return Reply::refused(
                "card.put",
                "too_many",
                format!("a plugin may have {MAX_CARDS} cards at once"),
            );
        }
        let now = secs(self.shared.now_ms());
        let (revision, created) = existing.map_or((1, now), |e| (e.revision + 1, e.created));
        if let Err(e) = self
            .context()
            .card(&put, revision, UtcInstant::from_unix_secs(created))
        {
            return Reply::refused("card.put", "invalid", e);
        }
        let expires = put.expires_in_secs.map(|s| now + i64::from(s));
        self.cards.insert(
            put.id.clone(),
            CardEntry {
                revision,
                created,
                expires,
                put,
            },
        );
        self.publish_cards();
        Reply::nothing()
    }

    fn tile_put(&mut self, tile: TileWire) -> Reply {
        if let Err(reply) = self.needs(Permission::Tiles, "tile.put") {
            return reply;
        }
        if !self.tiles.contains_key(&tile.id) && self.tiles.len() >= MAX_TILES {
            return Reply::refused(
                "tile.put",
                "too_many",
                format!("a plugin may have {MAX_TILES} tiles at once"),
            );
        }
        if let Err(e) = self.context().tile(&tile) {
            return Reply::refused("tile.put", "invalid", e);
        }
        self.tiles.insert(tile.id.clone(), tile);
        self.publish_tiles();
        Reply::nothing()
    }

    fn notify_post(&mut self, post: NotifyPost) -> Reply {
        if let Err(reply) = self.needs(Permission::Notify, "notify.post") {
            return reply;
        }
        if post.ongoing {
            return Reply::refused(
                "notify.post",
                "not_permitted",
                "a plugin's notification cannot stay until the plugin ends it (ongoing)",
            );
        }
        let existing = self.notes.get(&post.id);
        if existing.is_none() && self.notes.len() >= MAX_NOTES {
            return Reply::refused(
                "notify.post",
                "too_many",
                format!("a plugin may have {MAX_NOTES} notifications at once"),
            );
        }
        let now = secs(self.shared.now_ms());
        let revision = existing.map_or(1, |e| e.revision + 1);
        if let Err(e) =
            self.context()
                .notification(&post, revision, UtcInstant::from_unix_secs(now))
        {
            return Reply::refused("notify.post", "invalid", e);
        }
        let entry = NoteEntry {
            revision,
            received: now,
            expires: post.expires_in_secs.map(|s| now + i64::from(s)),
            post,
        };
        let event = note_json(&entry);
        self.notes.insert(entry.post.id.clone(), entry);
        self.publish_notes();
        self.shared
            .hub
            .emit(&topic(self.id(), "notifications"), "posted", event);
        Reply::nothing()
    }

    fn facts_put(&mut self, facts: BTreeMap<String, String>) -> Reply {
        if let Err(reply) = self.needs(Permission::Settings, "facts.put") {
            return reply;
        }
        if facts.len() > MAX_FACTS {
            return Reply::refused(
                "facts.put",
                "too_many",
                format!("at most {MAX_FACTS} facts"),
            );
        }
        for (name, line) in &facts {
            let plain = name.len() <= 32
                && name
                    .bytes()
                    .next()
                    .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                && name.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'
                });
            if !plain {
                return Reply::refused(
                    "facts.put",
                    "invalid",
                    format!("{name:?} is not a fact's name (lowercase letters, digits, - and _)"),
                );
            }
            if line.chars().count() > MAX_FACT_LINE || line.chars().any(char::is_control) {
                return Reply::refused(
                    "facts.put",
                    "invalid",
                    format!(
                        "the line of {name:?} is one line of at most {MAX_FACT_LINE} characters"
                    ),
                );
            }
        }
        self.facts = facts;
        self.publish_facts();
        Reply::nothing()
    }

    // -- topics -----------------------------------------------------------------------------------------------------------------

    fn publish_all(&self) {
        self.publish_cards();
        self.publish_tiles();
        self.publish_notes();
        self.publish_facts();
    }

    fn publish_cards(&self) {
        let items: Vec<Json> = self
            .cards
            .values()
            .map(|c| {
                json!({"id": c.put.id, "revision": c.revision, "createdAt": c.created,
                       "expiresAt": c.expires, "card": c.put})
            })
            .collect();
        self.shared
            .hub
            .set(&topic(self.id(), "cards"), json!({"items": items}));
    }

    fn publish_tiles(&self) {
        let items: Vec<&TileWire> = self.tiles.values().collect();
        self.shared
            .hub
            .set(&topic(self.id(), "tiles"), json!({"items": items}));
    }

    fn publish_notes(&self) {
        let items: Vec<Json> = self.notes.values().map(note_json).collect();
        self.shared
            .hub
            .set(&topic(self.id(), "notifications"), json!({"items": items}));
    }

    fn publish_facts(&self) {
        self.shared
            .hub
            .set(&topic(self.id(), "facts"), json!({"facts": self.facts}));
    }

    // -- time -------------------------------------------------------------------------------------------------------------------

    /// When this connection next has something to do on its own (a card or notification expires, a question runs out of time), in
    /// milliseconds on the shared clock.
    pub fn next_deadline_ms(&self) -> Option<u64> {
        let expiries = self
            .cards
            .values()
            .filter_map(|c| c.expires)
            .chain(self.notes.values().filter_map(|n| n.expires))
            .map(|s| (s.max(0) as u64) * 1000);
        let asks = self.live.iter().flat_map(|live| {
            live.ops
                .lock()
                .values()
                .map(|(_, d)| *d)
                .collect::<Vec<_>>()
        });
        expiries.chain(asks).min()
    }

    /// Takes away what has expired and reports the questions that were not answered in time.
    pub fn tick(&mut self) {
        let now_ms = self.shared.now_ms();
        let now = secs(now_ms);
        let before = (self.cards.len(), self.notes.len());
        self.cards.retain(|_, c| c.expires.is_none_or(|e| e > now));
        self.notes.retain(|_, n| n.expires.is_none_or(|e| e > now));
        if self.cards.len() != before.0 {
            self.publish_cards();
        }
        if self.notes.len() != before.1 {
            self.publish_notes();
        }
        if let Some(live) = &self.live {
            let late: Vec<(u64, String)> = {
                let mut ops = live.ops.lock();
                let late: Vec<u64> = ops
                    .iter()
                    .filter(|(_, (_, deadline))| *deadline <= now_ms)
                    .map(|(op, _)| *op)
                    .collect();
                late.into_iter()
                    .filter_map(|op| ops.remove(&op).map(|(action, _)| (op, action)))
                    .collect()
            };
            for (op, action) in late {
                self.shared.hub.emit(
                    &topic(self.id(), ""),
                    "action_result",
                    json!({"op": op, "action": action,
                           "outcome": {"type": "failed", "reason": "the plugin did not answer in time"}}),
                );
            }
        }
    }
}

fn note_json(n: &NoteEntry) -> Json {
    json!({"id": n.post.id, "channel": n.post.channel, "revision": n.revision,
           "receivedAt": n.received, "expiresAt": n.expires, "post": n.post})
}

impl Drop for Session {
    fn drop(&mut self) {
        let Some(live) = self.live.take() else {
            return;
        };
        let id = self.member.id.clone();
        {
            let mut table = self.shared.live.lock();
            // A newer connection of the same plugin has taken over: what it published is its own.
            if table
                .get(&id)
                .is_none_or(|current| current.conn != live.conn)
            {
                return;
            }
            table.remove(&id);
            if self.page {
                self.shared.settings.retire(&id);
            }
            let hub = &self.shared.hub;
            hub.set(&topic(&id, "cards"), json!({"items": []}));
            hub.set(&topic(&id, "tiles"), json!({"items": []}));
            hub.set(&topic(&id, "notifications"), json!({"items": []}));
            hub.set(&topic(&id, "facts"), json!({"facts": {}}));
        }
        self.shared.publish_plugins();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    struct Quiet;

    impl Outlet for Quiet {
        fn send(&self, _: ToPlugin) -> bool {
            true
        }

        fn poke(&self) {}
    }

    fn member() -> Member {
        let manifest = plugin_api::Manifest::parse(
            br#"{"schema":1,"id":"hello","name":"Hello","version":"1","api":{"min":1,"target":1},"run":{"exe":"bin/x"},"class":"S","permissions":["cards","notify"]}"#,
        )
        .unwrap();
        Member {
            id: "hello".into(),
            uid: 1,
            manifest,
        }
    }

    fn say(session: &mut Session, value: serde_json::Value) -> Reply {
        session.message(serde_json::from_value(value).unwrap())
    }

    #[test]
    fn cards_and_notifications_expire_on_the_clock_and_the_session_says_when() {
        let dir = tempfile::tempdir().unwrap();
        let hub = Arc::new(Hub::new(1));
        let settings = Arc::new(SettingsStore::new(dir.path(), hub.clone()));
        let now = Arc::new(AtomicU64::new(1_800_000_000_000));
        let clock = now.clone();
        let shared = Shared::new(
            hub.clone(),
            settings,
            Duration::from_secs(10),
            Arc::new(move || clock.load(Ordering::SeqCst)),
        );
        let mut session = Session::new(member(), shared, 1, Arc::new(Quiet));
        say(&mut session, json!({"t": "hello", "min": 1, "target": 1}));
        let registered = say(
            &mut session,
            json!({"t": "register", "title": "Hello", "views": [{"id": "main", "title": "Hello", "presentation": "sheet"}],
                   "channels": [{"id": "hi", "title": "Hi", "icon": "base.icons/message", "importance": "normal"}]}),
        );
        assert!(
            matches!(registered.messages[0], ToPlugin::Registered { .. }),
            "{registered:?}"
        );
        assert_eq!(session.next_deadline_ms(), None);
        say(
            &mut session,
            json!({"t": "card.put", "id": "c", "size": "half", "priority": "new", "expires_in_secs": 60, "content": {"type": "facts",
                "title": "C", "icon": "base.icons/message", "role": "accent", "facts": [{"label": "a", "value": "b"}]},
                "open": {"view": "main"}}),
        );
        say(
            &mut session,
            json!({"t": "notify.post", "id": "n", "channel": "hi", "title": "N", "expires_in_secs": 30}),
        );
        assert_eq!(session.next_deadline_ms(), Some(1_800_000_030_000));
        let items = |topic: &str| hub.get(topic).unwrap()["items"].as_array().unwrap().len();
        assert_eq!(
            (
                items("plugin.hello.cards"),
                items("plugin.hello.notifications")
            ),
            (1, 1)
        );

        now.store(1_800_000_031_000, Ordering::SeqCst);
        session.tick();
        assert_eq!(
            (
                items("plugin.hello.cards"),
                items("plugin.hello.notifications")
            ),
            (1, 0)
        );
        assert_eq!(session.next_deadline_ms(), Some(1_800_000_060_000));
        now.store(1_800_000_061_000, Ordering::SeqCst);
        session.tick();
        assert_eq!(items("plugin.hello.cards"), 0);
        assert_eq!(session.next_deadline_ms(), None);
    }

    #[test]
    fn the_first_message_must_be_hello_and_a_level_the_base_does_not_speak_is_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let hub = Arc::new(Hub::new(1));
        let shared = Shared::new(
            hub.clone(),
            Arc::new(SettingsStore::new(dir.path(), hub)),
            Duration::from_secs(10),
            Arc::new(|| 0),
        );
        let mut early = Session::new(member(), shared.clone(), 1, Arc::new(Quiet));
        let reply = say(&mut early, json!({"t": "card.remove", "id": "x"}));
        assert!(reply.hangup, "{reply:?}");
        let mut newer = Session::new(member(), shared.clone(), 2, Arc::new(Quiet));
        let reply = say(&mut newer, json!({"t": "hello", "min": 2, "target": 3}));
        assert!(reply.hangup);
        assert!(
            matches!(&reply.messages[0], ToPlugin::Error { code, fatal: true, .. } if code == "api_too_new"),
            "{reply:?}"
        );
        let mut twice = Session::new(member(), shared, 3, Arc::new(Quiet));
        assert!(!say(&mut twice, json!({"t": "hello", "min": 1, "target": 1})).hangup);
        assert!(say(&mut twice, json!({"t": "hello", "min": 1, "target": 1})).hangup);
    }

    #[test]
    fn only_the_welcome_gains_the_device_and_a_plugin_on_the_shared_crate_still_reads_it() {
        let dir = tempfile::tempdir().unwrap();
        let hub = Arc::new(Hub::new(1));
        let shared = Shared::new(
            hub.clone(),
            Arc::new(SettingsStore::new(dir.path(), hub)),
            Duration::from_secs(10),
            Arc::new(|| 0),
        );
        assert_eq!(shared.device_facts(), None, "no source, no facts");
        shared.set_device_source(Arc::new(
            || json!({"screen": {"class": "tv", "width": 1920, "height": 1080}, "input": ["remote"]}),
        ));
        let mut session = Session::new(member(), shared.clone(), 1, Arc::new(Quiet));
        let welcome = say(&mut session, json!({"t": "hello", "min": 1, "target": 1}))
            .messages
            .remove(0);
        let on_the_wire = wire_body(&welcome, shared.device_facts());
        let doc: Json = serde_json::from_slice(&on_the_wire).unwrap();
        assert_eq!(doc["t"], "welcome");
        assert_eq!(doc["device"]["screen"]["class"], "tv");
        assert_eq!(doc["device"]["input"], json!(["remote"]));
        // The shared crate's own types do not know the field and read the message as they always did (the contract's
        // change rule: a receiver skips a field it does not know).
        let read: ToPlugin = serde_json::from_slice(&on_the_wire).unwrap();
        assert_eq!(read, welcome);
        // Nothing else gains it, and a welcome with no source is the contract's own JSON.
        let refused = ToPlugin::Refused {
            what: "x".into(),
            code: "y".into(),
            reason: "z".into(),
        };
        assert_eq!(
            wire_body(&refused, shared.device_facts()),
            serde_json::to_vec(&refused).unwrap()
        );
        assert_eq!(
            wire_body(&welcome, None),
            serde_json::to_vec(&welcome).unwrap()
        );
    }
}
