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
//! | `plugin.<id>.notifications` | `{"items": [{id, channel, channelTitle, channelIcon, revision, receivedAt, expiresAt, post}]}`; events `posted` and `ended` |
//! | `plugin.<id>.facts` | `{"facts": {key: line}}` |
//! | `plugin.<id>` | events only: `settings_rejected`, `action_result` |
//! | `signal.occupancy` | `{"state": "present" \| "in_bed" \| "away" \| "unknown", "sources": [{plugin, state, lastMotionS, expiresAt}]}` |
//!
//! A plugin's settings page is registered in the settings store under its id, so `/v1/settings/<id>` serves it and a change of the
//! person's reaches the plugin as `settings.apply`. A page's choice providers are served (`Shared::choices`: the helper asks the
//! plugin a page at a time for whoever asks the API). A plugin that holds the Home Assistant lease (`ha.rs`) has the lease's
//! three keys of its page (`lease.ha.url`, `lease.ha.token`, `lease.ha.forget`) taken off the page the store serves: the address
//! and the token are the owner's command line's alone. What this TV does not serve at level 1 (timeline sources, secrets of a
//! plugin's own, images, surfaces, network, Bluetooth) is refused in words (`not_served`); a plugin that asks for it in its manifest
//! does not get to connect (`roster`).

use super::ha::{HaLease, Hold, Scope};
use super::local::OptIn;
use super::roster::{Approved, Member, Roster};
use crate::hub::Hub;
use crate::settings::SettingsStore;
use parking_lot::{Condvar, Mutex};
use plugin_api::MAX_FRAME;
use plugin_api::convert::{Context, Plan};
use plugin_api::ha as ha_keys;
use plugin_api::level::{self, LevelRefusal};
use plugin_api::limits::MESSAGES_PER_S;
use plugin_api::permission::{Lease, Permission, SignalKind};
use plugin_api::wire::{
    ActionOutcome, Authority, CardPut, ChoicePage, ChoicesBatch, Hello, Importance, NotifyPost,
    PresenceWire, Priority, Register, SettingsOutcome, SignalPut, TileWire, ToBase, ToPlugin,
    Welcome, WelcomeLimits,
};
use serde_json::{Value as Json, json};
use service_api::UtcInstant;
use settings_schema::{FieldKind, SettingsSchema};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// The plugin API level this TV speaks (`docs/research/plugins-slice-result.md`): the one before widgets and the decoder lease.
pub const TV_LEVEL: u32 = 1;
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
            retry_after_ms: None,
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
    /// The Home Assistant lease, for the plugins that require it (`set_lease`); none: they are refused in words.
    lease: Mutex<Option<Arc<HaLease>>>,
    /// What plugins last said of the room's occupancy, by plugin (see `Session::signal_put`).
    signals: Mutex<BTreeMap<String, Occupancy>>,
}

/// One plugin's statement about whether somebody is in the room, and when it stops holding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Occupancy {
    state: PresenceWire,
    last_motion_s: Option<u32>,
    expires: i64,
}

#[derive(Default)]
struct RosterView {
    members: Vec<(String, String, String, String)>,
    inert: Vec<(String, String)>,
    problem: Option<String>,
    /// The helper's own plugins (`local.rs`). Listed apart, in the `plugins` topic's `firstParty`.
    first_party: Vec<FirstParty>,
    /// What the last read of `enabled.json` switched on of them.
    approved: Vec<Approved>,
    /// Why a plugin that is switched on is not running (`runner.rs`), by plugin: what its entry says while it is waiting.
    notes: BTreeMap<String, String>,
}

/// One of the helper's own plugins, as the `plugins` topic lists it.
struct FirstParty {
    id: String,
    name: String,
    version: String,
    /// For one that asks first: its words, their code, whether the owner switched it on, and how it is told of a change.
    asks: Option<Asks>,
}

struct Asks {
    consent: String,
    approve: String,
    on: bool,
    wake: Arc<dyn Fn() + Send + Sync>,
}

fn approved(list: &[Approved], id: &str, code: &str) -> bool {
    list.iter()
        .any(|a| a.id == id && a.approve.eq_ignore_ascii_case(code))
}

/// What other threads need of a registered connection: where to speak to the plugin, and what waits for an answer.
pub struct Live {
    pub conn: u64,
    outlet: Arc<dyn Outlet>,
    actions: Mutex<Vec<String>>,
    /// op -> (action, deadline in ms)
    ops: Mutex<BTreeMap<u64, (String, u64)>>,
    txns: Mutex<VecDeque<u64>>,
    /// Whether the plugin holds the Home Assistant lease (its `lease.ha.forget` is the base's action).
    holds: bool,
    /// The choice providers its page names.
    providers: Vec<String>,
    /// Questions asked of its providers that wait for an answer: query -> where the answer goes.
    queries: Mutex<BTreeMap<u64, Arc<Query>>>,
}

/// A page of a provider's choices that somebody waits for.
struct Query {
    page: Mutex<Option<ChoicePage>>,
    ready: Condvar,
}

/// Why a provider's page could not be had.
#[derive(Debug, PartialEq, Eq)]
pub enum ChoicesError {
    NoSuchPlugin,
    NoSuchProvider,
    TooMany,
    Unreachable,
    /// The plugin did not answer in time.
    Timeout,
}

impl ChoicesError {
    pub fn words(&self) -> &'static str {
        match self {
            Self::NoSuchPlugin => "that plugin is not connected",
            Self::NoSuchProvider => "that plugin's page names no such provider",
            Self::TooMany => "that plugin has too many questions waiting",
            Self::Unreachable => "that plugin cannot be reached",
            Self::Timeout => "that plugin did not answer in time",
        }
    }
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
            lease: Mutex::new(None),
            signals: Mutex::new(BTreeMap::new()),
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

    /// Gives the plugins that require `lease:ha` the lease to hold.
    pub fn set_lease(&self, lease: Arc<HaLease>) {
        *self.lease.lock() = Some(lease);
    }

    /// The Home Assistant lease, if this helper has one.
    pub fn lease(&self) -> Option<Arc<HaLease>> {
        self.lease.lock().clone()
    }

    fn next(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::Relaxed)
    }

    /// Asks a connected plugin for a page of the choices of one of the providers its settings page names, and waits for the answer
    /// (at most the time a question to a plugin has). The caller is a thread of the API, never the plugin's own connection.
    pub fn choices(
        &self,
        plugin: &str,
        provider: &str,
        offset: u32,
        limit: u16,
    ) -> Result<ChoicePage, ChoicesError> {
        let live = self
            .live
            .lock()
            .get(plugin)
            .cloned()
            .ok_or(ChoicesError::NoSuchPlugin)?;
        if !live.providers.iter().any(|p| p == provider) {
            return Err(ChoicesError::NoSuchProvider);
        }
        let query = self.next();
        let waiting = Arc::new(Query {
            page: Mutex::new(None),
            ready: Condvar::new(),
        });
        {
            let mut queries = live.queries.lock();
            if queries.len() >= MAX_OUTSTANDING {
                return Err(ChoicesError::TooMany);
            }
            queries.insert(query, waiting.clone());
        }
        let sent = live.outlet.send(ToPlugin::ChoicesQuery {
            query,
            generation: 1,
            provider: provider.to_owned(),
            offset,
            limit: limit.clamp(1, ha_keys::MAX_PAGE_ROWS),
        });
        if !sent {
            live.queries.lock().remove(&query);
            return Err(ChoicesError::Unreachable);
        }
        live.outlet.poke();
        let deadline = std::time::Instant::now() + self.answer;
        let mut page = waiting.page.lock();
        while page.is_none() {
            if waiting.ready.wait_until(&mut page, deadline).timed_out() {
                break;
            }
        }
        let answer = page.take();
        drop(page);
        live.queries.lock().remove(&query);
        match answer {
            Some(answer) => Ok(answer),
            None => {
                let _ = live.outlet.send(ToPlugin::ChoicesCancel { query });
                live.outlet.poke();
                Err(ChoicesError::Timeout)
            }
        }
    }

    /// Records (or, with `None`, withdraws) what `plugin` says of the room's occupancy and publishes the merged answer: somebody is
    /// near if any source says so, nobody is only if every source that holds says so.
    fn set_occupancy(&self, plugin: &str, said: Option<Occupancy>) {
        let now = secs(self.now_ms());
        let mut signals = self.signals.lock();
        match said {
            Some(said) => {
                signals.insert(plugin.to_owned(), said);
            }
            None => {
                signals.remove(plugin);
            }
        }
        let valid: Vec<(&String, &Occupancy)> =
            signals.iter().filter(|(_, o)| o.expires > now).collect();
        let state = if valid.iter().any(|(_, o)| o.state == PresenceWire::Present) {
            "present"
        } else if valid.iter().any(|(_, o)| o.state == PresenceWire::InBed) {
            "in_bed"
        } else if valid.is_empty() {
            "unknown"
        } else {
            "away"
        };
        let sources: Vec<Json> = valid
            .iter()
            .map(|(plugin, o)| {
                json!({"plugin": plugin, "state": match o.state {
                    PresenceWire::Away => "away",
                    PresenceWire::Present => "present",
                    PresenceWire::InBed => "in_bed",
                }, "lastMotionS": o.last_motion_s, "expiresAt": o.expires})
            })
            .collect();
        self.hub.set(
            "signal.occupancy",
            json!({"state": state, "sources": sources}),
        );
    }

    /// A connection number nobody else has: what a plugin of the helper's own (`local.rs`) connects as.
    pub fn connection_id(&self) -> u64 {
        self.next()
    }

    /// Lists a plugin of the helper's own in the `plugins` topic (`firstParty`), connected once it has registered.
    pub fn add_first_party(&self, id: &str, name: &str, version: &str) {
        {
            let mut view = self.roster.lock();
            match view.first_party.iter_mut().find(|p| p.id == id) {
                Some(known) => {
                    known.name = name.to_owned();
                    known.version = version.to_owned();
                }
                None => view.first_party.push(FirstParty {
                    id: id.to_owned(),
                    name: name.to_owned(),
                    version: version.to_owned(),
                    asks: None,
                }),
            }
        }
        self.publish_plugins();
    }

    /// Lists a plugin of the helper's own that asks first: off, with its words and their code, until `enabled.json` approves them. `wake`
    /// is called once now and after each change of that; the plugin reads where it stands with [`Shared::switched_on`].
    pub fn offer(&self, plugin: &OptIn, wake: Arc<dyn Fn() + Send + Sync>) {
        let id = plugin.spec.id;
        let approve = plugin.approval();
        {
            let mut view = self.roster.lock();
            let on = approved(&view.approved, id, &approve);
            view.first_party.retain(|p| p.id != id);
            view.first_party.push(FirstParty {
                id: id.to_owned(),
                name: plugin.spec.name.to_owned(),
                version: plugin.spec.version.to_owned(),
                asks: Some(Asks {
                    consent: plugin.consent.to_owned(),
                    approve,
                    on,
                    wake: wake.clone(),
                }),
            });
        }
        self.publish_plugins();
        wake();
    }

    /// Whether the owner has switched on `id`, one of the helper's own plugins that ask first, with the words it asks with now.
    pub fn switched_on(&self, id: &str) -> bool {
        self.roster
            .lock()
            .first_party
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| p.asks.as_ref())
            .is_some_and(|a| a.on)
    }

    /// The code of the words `id` asks with, when it is one of the helper's own plugins that ask first.
    pub fn approval_of(&self, id: &str) -> Option<String> {
        self.roster
            .lock()
            .first_party
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| p.asks.as_ref())
            .map(|a| a.approve.clone())
    }

    /// Says why a plugin that is switched on has no program running (or, with `None`, that nothing is wrong), and publishes it.
    pub fn set_run_note(&self, plugin: &str, note: Option<String>) {
        {
            let mut view = self.roster.lock();
            match note {
                Some(note) => {
                    if view.notes.insert(plugin.to_owned(), note.clone()) == Some(note) {
                        return;
                    }
                }
                None => {
                    if view.notes.remove(plugin).is_none() {
                        return;
                    }
                }
            }
        }
        self.publish_plugins();
    }

    /// Records what the roster is (and why a plugin is inert) and publishes the `plugins` topic; the helper's own plugins that ask first
    /// are woken when their approval came or went.
    pub fn set_roster(&self, roster: &Roster, problem: Option<String>) {
        let woken: Vec<Arc<dyn Fn() + Send + Sync>> = {
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
            view.approved = roster.first_party.clone();
            let RosterView {
                first_party,
                approved: list,
                ..
            } = &mut *view;
            first_party
                .iter_mut()
                .filter_map(|p| {
                    let asks = p.asks.as_mut()?;
                    let on = approved(list, &p.id, &asks.approve);
                    (on != asks.on).then(|| {
                        asks.on = on;
                        asks.wake.clone()
                    })
                })
                .collect()
        };
        self.publish_plugins();
        for wake in woken {
            wake();
        }
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
                let mut entry = json!({"id": id, "name": name, "version": version, "class": class, "state": state,
                                       "registered": connected});
                if !connected && let Some(note) = view.notes.get(id) {
                    entry["reason"] = json!(note);
                }
                entry
            })
            .collect();
        plugins.extend(view.inert.iter().map(|(id, reason)| {
            json!({"id": id, "state": "inert", "registered": false, "reason": reason})
        }));
        let first_party: Vec<Json> = view
            .first_party
            .iter()
            .map(|p| {
                let connected = live.contains(&p.id);
                let state = match &p.asks {
                    Some(asks) if !asks.on => "off",
                    _ if connected => "connected",
                    _ => "waiting",
                };
                let mut entry = json!({"id": p.id, "name": p.name, "version": p.version, "class": "S", "state": state,
                                       "registered": connected, "firstParty": true});
                if let Some(asks) = &p.asks {
                    entry["consent"] = json!(asks.consent);
                    entry["approve"] = json!(asks.approve);
                }
                entry
            })
            .collect();
        self.hub.set(
            "plugins",
            json!({"plugins": plugins, "firstParty": first_party, "problem": view.problem}),
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
        if live.holds
            && action == ha_keys::ACTION_FORGET
            && let Some(lease) = self.lease()
        {
            // The base's own action: the plugin only has the button. It narrows what the TV may do, so the TV's Settings may ask.
            lease.forget();
            self.hub.emit(
                &topic(plugin, ""),
                "action_result",
                json!({"op": op, "action": action, "outcome": {"type": "ok"}}),
            );
            return Ok(op);
        }
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
            gesture: None,
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
    /// The channels the plugin declared (id, title, icon): a notification is drawn with its channel's icon.
    channels: Vec<(String, String, String)>,
    /// The Home Assistant lease, while the plugin holds it (it requires `lease:ha`): dropping it lets go.
    hold: Option<Hold>,
    /// What the plugin last said of the room's occupancy (`signal.put`).
    occupancy: Option<Occupancy>,
}

fn secs(ms: u64) -> i64 {
    (ms / 1000) as i64
}

/// Whether `key` is one of the Home Assistant lease's, which a plugin that holds the lease puts on its page for the Show's sake and
/// this TV does not draw from the plugin's page (the address and the token are the owner's command line's: `ha.rs`).
fn lease_key(key: &str) -> bool {
    ha_keys::LEASE_FIELDS.contains(&key) || key == ha_keys::ACTION_FORGET
}

/// What a plugin's page may not use on this TV, in words (or nothing). `holds`: the plugin holds the Home Assistant lease, so the
/// lease's own fields are allowed (and are not served, see [`served`]).
fn page_problem(
    schema: &SettingsSchema,
    declared_actions: Option<&[String]>,
    holds: bool,
) -> Option<String> {
    for field in schema.fields() {
        let key = field.key.as_str();
        let action_key = match &field.kind {
            FieldKind::Action { action, .. } => Some(action.as_str()),
            _ => None,
        };
        let is_lease = ha_keys::LEASE_FIELDS.contains(&key)
            || action_key.is_some_and(|a| a == ha_keys::ACTION_FORGET);
        if key.starts_with("lease.") && !(holds && is_lease) {
            return Some(format!("{key}: leases are not served by this TV"));
        }
        if is_lease && holds {
            continue;
        }
        match &field.kind {
            FieldKind::Text { secret: true, .. } => {
                return Some(format!(
                    "{key}: secret fields are not served by this TV yet"
                ));
            }
            FieldKind::Action { action, .. } => {
                if let Some(declared) = declared_actions
                    && !declared.iter().any(|d| d == action.as_str())
                {
                    return Some(format!(
                        "{key}: the action {} was not declared at registration",
                        action.as_str()
                    ));
                }
            }
            _ => {}
        }
    }
    None
}

/// The page as this TV serves it: a plugin that holds the Home Assistant lease has its lease's fields and button taken off.
fn served(mut schema: SettingsSchema, holds: bool) -> SettingsSchema {
    if holds {
        for section in &mut schema.sections {
            section.fields.retain(|field| {
                let action = match &field.kind {
                    FieldKind::Action { action, .. } => Some(action.as_str()),
                    _ => None,
                };
                !(ha_keys::LEASE_FIELDS.contains(&field.key.as_str())
                    || action.is_some_and(lease_key))
            });
        }
    }
    schema
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
            channels: Vec::new(),
            hold: None,
            occupancy: None,
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
        // This TV speaks level 1 whatever the crate's own level: it has no widgets, no decoder lease and no pane host. A plugin whose
        // `min` is 1 runs with the level-1 permissions of its manifest; the rest are not granted, as the plugin API's rule says.
        let api = match level::negotiate_with(hello.min, hello.target, TV_LEVEL, TV_LEVEL) {
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
        if manifest.requires.contains(&Lease::Ha) {
            features.push("ha:1".to_owned());
        }
        if manifest
            .permissions
            .contains(&Permission::SignalProvide(SignalKind::Occupancy))
        {
            features.push("signal:1".to_owned());
        }
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
            zone: None,
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
            ToBase::ChoicesBatch(batch) => self.choices_batch(batch),
            // A provider's rows changed: the next page asked for is the new one, and nothing is kept between pages.
            ToBase::ChoicesChanged { .. } => Reply::nothing(),
            ToBase::HaWatch(watch) => self.lease_call("ha.watch", |hold| hold.watch(&watch)),
            ToBase::HaCall(call) => self.lease_call("ha.call", |hold| hold.call(&call)),
            ToBase::HaList(list) => self.lease_call("ha.list", |hold| hold.list(&list)),
            ToBase::SignalPut(put) => self.signal_put(put),
            ToBase::Hello(_) | ToBase::Register(_) | ToBase::Bye { .. } => Reply::nothing(),
            // Everything else belongs to a surface this TV does not serve at level 1.
            other => Reply::refused(
                other.name(),
                "not_served",
                "this TV does not serve that at plugin API level 1 (it serves settings, cards, tiles, notifications, facts, the Home Assistant lease and the occupancy signal)",
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
        let holds = self.member.manifest.requires.contains(&Lease::Ha);
        let lease = self.shared.lease();
        if holds && lease.is_none() {
            return Reply::refused(
                "register",
                "not_served",
                "this TV has no Home Assistant lease to give",
            );
        }
        // The lease's three keys are the base's: what the store serves is the page without them (the struct is non-exhaustive, so the
        // page is swapped in place).
        let mut register = register;
        let page = register.settings.take().map(|schema| served(schema, holds));
        let register = register.maybe_settings(page);
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
            && let Some(why) = page_problem(schema, None, holds)
        {
            return Reply::refused("register", "not_served", why);
        }
        let key = self.member.manifest.owner_key();
        let channels: Vec<(String, String, String)> = register
            .channels
            .iter()
            .map(|c| (c.id.clone(), c.title.clone(), c.icon.clone()))
            .collect();
        let tiles = register.tiles.clone();
        let has_page = register.settings.is_some();
        let schema = register.settings.clone();
        // the permissions this level grants: a level-2 one in the manifest (a widget) is not held on this TV
        let held = self
            .member
            .manifest
            .permissions
            .iter()
            .filter(|p| p.available_at(TV_LEVEL))
            .cloned()
            .collect();
        let plan = match Plan::new(register, &key, &held) {
            Ok(plan) => plan,
            Err(e) => return Reply::refused("register", "invalid", e),
        };
        let providers: Vec<String> = plan
            .declared()
            .providers
            .iter()
            .map(ToString::to_string)
            .collect();
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
            holds,
            providers,
            queries: Mutex::new(BTreeMap::new()),
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
        self.channels = channels;
        if holds && let Some(lease) = lease {
            // The plugin is told where the lease is once its `registered` has gone (`tick`).
            self.hold = Some(lease.acquire(self.outlet.clone(), Scope::of(&self.member.manifest)));
        }
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
        let holds = self.hold.is_some();
        if let Some(why) = page_problem(&schema, Some(&self.declared_actions), holds) {
            return Reply::refused("settings.page", "not_served", why);
        }
        match self.shared.settings.register(served(schema, holds), None) {
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
        let event = note_json(&entry, &self.channels);
        self.notes.insert(entry.post.id.clone(), entry);
        self.publish_notes();
        crate::saver::note_posted(&self.shared.hub, self.id(), &event);
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

    /// A page of a provider's choices, answered: whoever asked for it through the API (`Shared::choices`) is waiting. An answer to a
    /// question that was given up on is dropped unseen.
    fn choices_batch(&mut self, batch: ChoicesBatch) -> Reply {
        let waiting = self
            .live
            .as_ref()
            .and_then(|live| live.queries.lock().remove(&batch.query));
        if let Some(waiting) = waiting {
            *waiting.page.lock() = Some(batch.page);
            waiting.ready.notify_all();
        }
        Reply::nothing()
    }

    /// A request of the Home Assistant lease, which the plugin holds only if its manifest says so (`lease:ha`).
    fn lease_call(
        &mut self,
        what: &str,
        ask: impl FnOnce(&Hold) -> Result<(), super::ha::Refusal>,
    ) -> Reply {
        let Some(hold) = &self.hold else {
            return Reply::refused(
                what,
                "not_permitted",
                "this plugin did not ask for the Home Assistant lease (lease:ha), so it was not approved",
            );
        };
        match ask(hold) {
            Ok(()) => Reply::nothing(),
            Err(refusal) => Reply::refused(what, refusal.code, refusal.reason),
        }
    }

    /// What the plugin believes of the room's occupancy, for as long as it says (5 to 900 seconds): it runs out by itself, so a plugin
    /// that dies cannot hold the room occupied or empty. Published as `signal.occupancy`; this TV has nothing that reads it yet.
    fn signal_put(&mut self, put: SignalPut) -> Reply {
        if let Err(reply) = self.needs(
            Permission::SignalProvide(SignalKind::Occupancy),
            "signal.put",
        ) {
            return reply;
        }
        if !(5..=900).contains(&put.ttl_s) {
            return Reply::refused(
                "signal.put",
                "invalid",
                "a signal holds for 5 to 900 seconds",
            );
        }
        let said = Occupancy {
            state: put.state,
            last_motion_s: put.last_motion_s,
            expires: secs(self.shared.now_ms()) + i64::from(put.ttl_s),
        };
        self.occupancy = Some(said);
        self.shared.set_occupancy(self.id(), Some(said));
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
        let items: Vec<Json> = self
            .notes
            .values()
            .map(|n| note_json(n, &self.channels))
            .collect();
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
            .chain(self.occupancy.map(|o| o.expires))
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
        // The broker ticks right after it has sent a reply: the plugin has its `registered`, and is told where the lease is.
        if let Some(hold) = &self.hold {
            hold.ready();
        }
        if self.occupancy.is_some_and(|o| o.expires <= now) {
            self.occupancy = None;
            self.shared.set_occupancy(self.id(), None);
        }
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

fn note_json(n: &NoteEntry, channels: &[(String, String, String)]) -> Json {
    let channel = channels.iter().find(|(id, _, _)| *id == n.post.channel);
    json!({"id": n.post.id, "channel": n.post.channel, "channelTitle": channel.map(|c| c.1.as_str()),
           "channelIcon": channel.map(|c| c.2.as_str()), "revision": n.revision,
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
            // The lease is let go (the link ends if this was the last holder) and what the plugin said of the room is withdrawn.
            self.hold = None;
            if self.occupancy.take().is_some() {
                self.shared.set_occupancy(&id, None);
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
            program: None,
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
        // the notification is drawn with its channel's icon and name
        let note = &hub.get("plugin.hello.notifications").unwrap()["items"][0];
        assert_eq!(note["channelIcon"], "base.icons/message");
        assert_eq!(note["channelTitle"], "Hi");

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
        let too_new = TV_LEVEL + 1;
        let reply = say(
            &mut newer,
            json!({"t": "hello", "min": too_new, "target": too_new + 1}),
        );
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
            retry_after_ms: None,
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

    /// The plugin's whole conversation as the tests see it: what the base said to it.
    #[derive(Default)]
    struct Said(Mutex<Vec<ToPlugin>>);

    impl Outlet for Said {
        fn send(&self, message: ToPlugin) -> bool {
            self.0.lock().push(message);
            true
        }

        fn poke(&self) {}
    }

    fn holder(permissions: &str, requires: &str) -> Member {
        let text = format!(
            r#"{{"schema":1,"id":"ha-test","name":"HA","version":"1","api":{{"min":1,"target":1}},"run":{{"exe":"bin/x"}},"class":"S","requires":[{requires}],"permissions":[{permissions}]}}"#
        );
        Member {
            id: "ha-test".into(),
            uid: 1,
            manifest: plugin_api::Manifest::parse(text.as_bytes()).unwrap(),
            program: None,
        }
    }

    struct Rig {
        hub: Arc<Hub>,
        shared: Arc<Shared>,
        now: Arc<AtomicU64>,
        _dir: tempfile::TempDir,
        link: std::path::PathBuf,
    }

    fn rig(with_lease: bool) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let hub = Arc::new(Hub::new(1));
        let settings = Arc::new(SettingsStore::new(dir.path().join("settings"), hub.clone()));
        let now = Arc::new(AtomicU64::new(1_800_000_000_000));
        let clock = now.clone();
        let shared = Shared::new(
            hub.clone(),
            settings,
            Duration::from_secs(2),
            Arc::new(move || clock.load(Ordering::SeqCst)),
        );
        let link = dir.path().join("ha").join("link.json");
        if with_lease {
            shared.set_lease(Arc::new(HaLease::new(super::super::ha::LeaseSettings {
                link_file: link.clone(),
                policy: ha_link::AddressPolicy::TESTING,
                timing: link_timing(),
                tls: None,
                log: Arc::new(|_| {}),
            })));
        }
        Rig {
            hub,
            shared,
            now,
            _dir: dir,
            link,
        }
    }

    fn link_timing() -> ha_link::Timing {
        ha_link::Timing::default()
    }

    /// The Home Assistant plugin's page, as LumaShow writes it, with a picker served by the plugin and the lease's three keys.
    fn page() -> serde_json::Value {
        json!({"format": 1, "owner": "ha-test", "revision": 1, "title": "HA", "sections": [
            {"id": "connection", "title": "Connection", "fields": [
                {"key": "state", "label": "Connection", "kind": {"type": "read-only", "value": {"capability": "plugin.ha-test.link"}}, "apply": "on-commit"},
                {"key": "lease.ha.url", "label": "Address", "kind": {"type": "text", "max_chars": 200, "secret": false},
                 "default": {"type": "text", "value": ""}, "apply": "on-commit"},
                {"key": "lease.ha.token", "label": "Access token", "kind": {"type": "text", "max_chars": 1024, "secret": true}, "apply": "on-commit"},
                {"key": "forget", "label": "Forget", "kind": {"type": "action", "action": "lease.ha.forget", "confirmation": "destructive"}, "apply": "on-commit"}]},
            {"id": "room", "title": "Room", "fields": [
                {"key": "room", "label": "Room", "kind": {"type": "choice", "options": {"provider": "rooms"}}, "apply": "on-commit"}]}]})
    }

    fn register(page: serde_json::Value) -> serde_json::Value {
        json!({"t": "register", "title": "HA", "settings": page,
               "actions": [{"key": "lease.ha.forget", "label": "Forget", "confirmation": "destructive",
                            "confirm_text": {"title": "Forget?", "confirm_label": "Forget"}}]})
    }

    const HA_PERMISSIONS: &str =
        r#""settings","cards","ha.read:areas,light","ha.call:light","signal.provide:occupancy""#;

    #[test]
    fn a_plugin_that_holds_the_lease_has_the_leases_keys_taken_off_the_page_the_tv_serves() {
        let rig = rig(true);
        let said = Arc::new(Said::default());
        let mut session = Session::new(
            holder(HA_PERMISSIONS, r#""lease:ha""#),
            rig.shared.clone(),
            1,
            said.clone(),
        );
        let welcome = say(&mut session, json!({"t": "hello", "min": 1, "target": 1}));
        let ToPlugin::Welcome(welcome) = &welcome.messages[0] else {
            panic!("{welcome:?}")
        };
        assert!(
            welcome.features.contains(&"ha:1".to_owned())
                && welcome.features.contains(&"signal:1".to_owned())
        );
        let registered = say(&mut session, register(page()));
        assert!(
            matches!(registered.messages[0], ToPlugin::Registered { .. }),
            "{registered:?}"
        );
        let doc = rig.shared.settings.describe("ha-test").unwrap();
        let keys: Vec<String> = doc["schema"]["sections"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|s| s["fields"].as_array().unwrap().iter())
            .map(|f| f["key"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            keys,
            vec!["state", "room"],
            "no address, no token, no forget on the TV's page"
        );
        assert!(!doc.to_string().contains("lease."), "{doc}");
        // Told where the lease is only after its `registered` has been sent: the broker ticks right after a reply.
        assert!(said.0.lock().is_empty());
        session.tick();
        assert!(
            matches!(said.0.lock().as_slice(), [ToPlugin::HaLink(l)] if l.state == plugin_api::wire::HaLinkKind::Unconfigured)
        );
        assert_eq!(rig.shared.lease().unwrap().info().holders, 1);
        drop(session);
        assert_eq!(
            rig.shared.lease().unwrap().info().holders,
            0,
            "the lease is let go with the connection"
        );
    }

    #[test]
    fn the_lease_forgets_when_the_base_is_asked_to_run_its_action_and_not_the_plugin() {
        let rig = rig(true);
        let said = Arc::new(Said::default());
        let mut session = Session::new(
            holder(HA_PERMISSIONS, r#""lease:ha""#),
            rig.shared.clone(),
            1,
            said.clone(),
        );
        say(&mut session, json!({"t": "hello", "min": 1, "target": 1}));
        say(&mut session, register(page()));
        super::super::super::ha_store::save(&rig.link, "http://192.168.1.50:8123", "tkn-forget")
            .unwrap();
        rig.shared.lease().unwrap().refresh();
        assert!(rig.link.exists());
        let op = rig
            .shared
            .invoke_action("ha-test", "lease.ha.forget", None)
            .unwrap();
        assert!(op > 0);
        assert!(!rig.link.exists(), "the saved link is deleted");
        assert!(
            said.0
                .lock()
                .iter()
                .all(|m| !matches!(m, ToPlugin::ActionInvoke { .. })),
            "the plugin was not asked: it only has the button"
        );
    }

    #[test]
    fn a_plugin_that_does_not_hold_the_lease_cannot_use_its_keys_or_its_messages_and_one_needs_a_lease_to_be_given()
     {
        let rig = rig(true);
        // lease keys on the page of a plugin that has no `lease:ha`
        let mut session = Session::new(
            holder(r#""settings""#, ""),
            rig.shared.clone(),
            1,
            Arc::new(Quiet),
        );
        say(&mut session, json!({"t": "hello", "min": 1, "target": 1}));
        let reply = say(&mut session, register(page()));
        assert!(
            matches!(&reply.messages[0], ToPlugin::Refused { code, reason, .. } if code == "not_served" && reason.contains("lease.ha.url")),
            "{reply:?}"
        );
        // messages of the lease from a plugin that does not hold it
        let mut plain = Session::new(
            holder(r#""cards""#, ""),
            rig.shared.clone(),
            2,
            Arc::new(Quiet),
        );
        say(&mut plain, json!({"t": "hello", "min": 1, "target": 1}));
        say(&mut plain, json!({"t": "register", "title": "P"}));
        for message in [
            json!({"t": "ha.watch", "entities": ["light.a"]}),
            json!({"t": "ha.call", "op": 1, "domain": "light", "service": "turn_on", "entity_ids": ["light.a"]}),
            json!({"t": "ha.list", "query": 1, "what": "areas"}),
            json!({"t": "signal.put", "kind": "occupancy", "state": "present", "ttl_s": 60}),
        ] {
            let reply = say(&mut plain, message.clone());
            assert!(
                matches!(&reply.messages[0], ToPlugin::Refused { code, .. } if code == "not_permitted"),
                "{message}: {reply:?}"
            );
        }
        // a plugin that holds the lease where the helper has none to give
        let bare = self::rig(false);
        let mut holder_session = Session::new(
            holder(HA_PERMISSIONS, r#""lease:ha""#),
            bare.shared.clone(),
            3,
            Arc::new(Quiet),
        );
        say(
            &mut holder_session,
            json!({"t": "hello", "min": 1, "target": 1}),
        );
        let reply = say(&mut holder_session, register(page()));
        assert!(
            matches!(&reply.messages[0], ToPlugin::Refused { code, .. } if code == "not_served"),
            "{reply:?}"
        );
    }

    #[test]
    fn the_occupancy_signal_runs_out_by_itself_merges_and_goes_with_the_plugin() {
        let rig = rig(true);
        let mut session = Session::new(
            holder(HA_PERMISSIONS, r#""lease:ha""#),
            rig.shared.clone(),
            1,
            Arc::new(Quiet),
        );
        say(&mut session, json!({"t": "hello", "min": 1, "target": 1}));
        say(&mut session, register(page()));
        let state = || {
            rig.hub
                .get("signal.occupancy")
                .map(|v| v["state"].as_str().unwrap().to_owned())
        };
        for bad in [4, 901, 0] {
            let r = say(
                &mut session,
                json!({"t": "signal.put", "kind": "occupancy", "state": "present", "ttl_s": bad}),
            );
            assert!(
                matches!(&r.messages[0], ToPlugin::Refused { code, .. } if code == "invalid"),
                "{bad}: {r:?}"
            );
        }
        assert_eq!(state(), None);
        say(
            &mut session,
            json!({"t": "signal.put", "kind": "occupancy", "state": "present", "last_motion_s": 12, "ttl_s": 120}),
        );
        assert_eq!(state().as_deref(), Some("present"));
        assert_eq!(
            rig.hub.get("signal.occupancy").unwrap()["sources"][0]["lastMotionS"],
            12
        );
        assert_eq!(session.next_deadline_ms(), Some(1_800_000_120_000));
        say(
            &mut session,
            json!({"t": "signal.put", "kind": "occupancy", "state": "away", "ttl_s": 60}),
        );
        assert_eq!(state().as_deref(), Some("away"));
        // Said again before it ran out is said; not said again, it runs out and nobody is known to be there or not.
        rig.now.store(1_800_000_061_000, Ordering::SeqCst);
        session.tick();
        assert_eq!(
            state().as_deref(),
            Some("unknown"),
            "a plugin that dies cannot hold the room occupied or empty"
        );
        assert_eq!(session.next_deadline_ms(), None);
        say(
            &mut session,
            json!({"t": "signal.put", "kind": "occupancy", "state": "in_bed", "ttl_s": 600}),
        );
        assert_eq!(state().as_deref(), Some("in_bed"));
        drop(session);
        assert_eq!(
            state().as_deref(),
            Some("unknown"),
            "withdrawn when the plugin goes"
        );
        // A plugin that did not ask to provide it is refused.
        let mut other = Session::new(
            holder(r#""cards""#, ""),
            rig.shared.clone(),
            2,
            Arc::new(Quiet),
        );
        say(&mut other, json!({"t": "hello", "min": 1, "target": 1}));
        say(&mut other, json!({"t": "register", "title": "P"}));
        let r = say(
            &mut other,
            json!({"t": "signal.put", "kind": "occupancy", "state": "present", "ttl_s": 60}),
        );
        assert!(
            matches!(&r.messages[0], ToPlugin::Refused { code, .. } if code == "not_permitted"),
            "{r:?}"
        );
    }

    #[test]
    fn a_page_of_choices_is_asked_of_the_plugin_answered_by_it_and_given_up_on_in_time() {
        let rig = rig(true);
        let said = Arc::new(Said::default());
        let mut session = Session::new(
            holder(HA_PERMISSIONS, r#""lease:ha""#),
            rig.shared.clone(),
            1,
            said.clone(),
        );
        say(&mut session, json!({"t": "hello", "min": 1, "target": 1}));
        say(&mut session, register(page()));
        // The plugin answers from its own thread, as the broker's connection does.
        let session = Arc::new(Mutex::new(session));
        let answering = {
            let (session, said) = (session.clone(), said.clone());
            std::thread::spawn(move || {
                let end = std::time::Instant::now() + Duration::from_secs(5);
                loop {
                    let asked = said.0.lock().iter().find_map(|m| match m {
                        ToPlugin::ChoicesQuery {
                            query,
                            generation,
                            provider,
                            offset,
                            limit,
                        } => Some((*query, *generation, provider.clone(), *offset, *limit)),
                        _ => None,
                    });
                    if let Some((query, generation, provider, offset, limit)) = asked {
                        assert_eq!((provider.as_str(), offset, limit), ("rooms", 0, 24));
                        say(
                            &mut session.lock(),
                            json!({"t": "choices.batch", "query": query, "generation": generation,
                            "page": {"offset": 0, "rows": [{"id": "kitchen", "label": "Kitchen"}], "more": false, "state": "ready"}}),
                        );
                        return;
                    }
                    assert!(std::time::Instant::now() < end);
                    std::thread::sleep(Duration::from_millis(5));
                }
            })
        };
        let page = rig.shared.choices("ha-test", "rooms", 0, 24).unwrap();
        answering.join().unwrap();
        assert_eq!(page.rows[0].id, "kitchen");
        assert_eq!(page.state, plugin_api::wire::ChoiceState::Ready);
        // A provider the page does not name, a plugin that is not there, and one that does not answer.
        assert_eq!(
            rig.shared.choices("ha-test", "secrets", 0, 24).unwrap_err(),
            ChoicesError::NoSuchProvider
        );
        assert_eq!(
            rig.shared.choices("nobody", "rooms", 0, 24).unwrap_err(),
            ChoicesError::NoSuchPlugin
        );
        said.0.lock().clear();
        let started = std::time::Instant::now();
        assert_eq!(
            rig.shared.choices("ha-test", "rooms", 0, 24).unwrap_err(),
            ChoicesError::Timeout
        );
        assert!(
            started.elapsed() >= Duration::from_secs(2)
                && started.elapsed() < Duration::from_secs(6)
        );
        assert!(
            said.0
                .lock()
                .iter()
                .any(|m| matches!(m, ToPlugin::ChoicesCancel { .. })),
            "the plugin is told it need not answer"
        );
        // The answer that comes after is dropped unseen.
        let late = say(
            &mut session.lock(),
            json!({"t": "choices.batch", "query": 99, "generation": 1,
            "page": {"offset": 0, "rows": [], "more": false, "state": "ready"}}),
        );
        assert!(late.messages.is_empty());
    }
}
