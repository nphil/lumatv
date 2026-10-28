//! The Home Assistant lease: this TV's one connection to Home Assistant, held by the plugins that require it
//! (LumaShow's `docs/plugin-api.md`, "The Home Assistant lease"; its own is `core-services::plugins::ha`, which this follows).
//!
//! **The connection belongs to the helper.** The lease keeps the address and the access token, runs the link (`ha-link`: one
//! thread, one WebSocket, the home network only) and gives each plugin that holds it only what its manifest asked for and the
//! owner approved: the entities of the domains in `ha.read` (at most 128), the rooms when `areas` is there, persistent notifications
//! when `notifications` is, and calls of the services of the domains in `ha.call` on entities of that domain with a few plain
//! values. A plugin never sees the token, never opens a socket and cannot point the connection anywhere.
//!
//! **Where the address and the token come from: the owner's command line, and nothing else.** `lumatv-helper ha-link` saves them
//! together in one private file (`ha_store`, `<data dir>/ha/link.json`), the same stance as voice search's link: no route of the
//! loopback API, no settings field and no message of a plugin can set either, so nothing that reaches the API can point the token at
//! another machine. The TV's Settings can only narrow: "forget" deletes the file ([`HaLease::forget`]). The helper is told to read
//! the file again ([`HaLease::refresh`]) when the command line has saved a new one.
//!
//! **This TV is stricter than the Show about what a plugin may call.** A manifest may not name a siren, an alarm panel, a
//! script... (the plugin API refuses those); here a lock, a camera, a cover (garage doors, gates) and a valve are added ([`NEVER_CALLED`]):
//! doors, alarms, cameras and water are the owner's to operate (Principle 0). Reading them is another matter and stays allowed:
//! `ha.read:lock` shows a lock's state and nothing more.
//!
//! **It costs nothing when no plugin holds it.** Nothing is read at construction, and there is no link, thread, socket or timer
//! until a holder has an address and a token; when the last holder goes the link is stopped and what it kept is dropped.
//!
//! What it tells the holders: where the link is (`ha.link`), the watched entities in batches no closer than the interval asked for
//! (`ha.states`), the answers to calls and lists, and Home Assistant's own notifications. What it tells the TV: the `ha.link` topic
//! ([`Info`]): the state in words, the address as the owner gave it and whether a token is kept, never the token.

use super::session::Outlet;
use crate::ha_store;
use ha_link::{
    AddressPolicy, Config as LinkConfig, Endpoint, EntityState, Link, Notice, NoticeKind, Sink,
    StateBatch, Status, Timing, Token,
};
use parking_lot::Mutex;
use plugin_api::Manifest;
use plugin_api::Permission;
use plugin_api::ha as limits;
use plugin_api::wire::{
    HaCall, HaLinkKind, HaLinkState, HaList, HaNotice, HaNoticeKind, HaPage, HaPageState, HaRow,
    HaState, HaStates, HaWatch, HaWhat, ToPlugin,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

/// The largest a batch of states is in one frame (the frame limit is 64 KiB; the rest is room for the frame's own words).
const CHUNK_BYTES: usize = 40 * 1024;

/// Domains this TV never lets a plugin call, whatever its manifest says (and the plugin API's own list, which a manifest cannot
/// name at all): an owner's doors, alarms, cameras and water are not for a program on the TV to operate.
pub const NEVER_CALLED: [&str; 6] = [
    "lock",
    "alarm_control_panel",
    "siren",
    "camera",
    "cover",
    "valve",
];

/// What the lease is made with.
#[derive(Clone)]
pub struct LeaseSettings {
    /// Where the saved link is (`ha_store`).
    pub link_file: PathBuf,
    /// Which addresses the token may be sent to.
    pub policy: AddressPolicy,
    pub timing: Timing,
    /// The system's TLS helper, for a secure address.
    pub tls: Option<fetch::TlsFetch>,
    /// Where a line for the log goes (never the token).
    pub log: Arc<dyn Fn(String) + Send + Sync>,
}

impl LeaseSettings {
    /// What the TV runs: the home network only, and a verdict on a new token as quickly as Home Assistant gives one.
    pub fn system(
        link_file: PathBuf,
        policy: AddressPolicy,
        tls: Option<fetch::TlsFetch>,
        log: Arc<dyn Fn(String) + Send + Sync>,
    ) -> Self {
        Self {
            link_file,
            policy,
            timing: Timing {
                connect: Duration::from_secs(4),
                handshake: Duration::from_secs(5),
                ..Timing::default()
            },
            tls,
            log,
        }
    }
}

/// What a plugin may do with the lease: what its approved manifest says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scope {
    /// The domains of entities it may watch and list.
    pub read: BTreeSet<String>,
    /// It may list the rooms.
    pub areas: bool,
    /// It may receive persistent notifications.
    pub notices: bool,
    /// The domains whose services it may call.
    pub call: BTreeSet<String>,
}

impl Scope {
    pub fn of(manifest: &Manifest) -> Self {
        let mut scope = Self::default();
        for permission in &manifest.permissions {
            match permission {
                Permission::HaRead(words) => {
                    for word in words {
                        match word.as_str() {
                            "areas" => scope.areas = true,
                            "notifications" => scope.notices = true,
                            domain => {
                                scope.read.insert(domain.to_owned());
                            }
                        }
                    }
                }
                Permission::HaCall(domains) => scope.call.extend(domains.iter().cloned()),
                _ => {}
            }
        }
        scope
    }
}

/// What the TV is told of the lease (the `ha.link` topic): never the token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Info {
    /// `off` (no plugin holds the lease), `unconfigured`, `connecting`, `connected`, `offline` or `auth_failed`.
    pub state: &'static str,
    /// The state in words for the person.
    pub detail: String,
    /// The address as it was saved (the scheme and port made explicit).
    pub address: Option<String>,
    /// Whether a token is kept.
    pub token: bool,
    /// How many plugins hold the lease.
    pub holders: usize,
}

struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Bucket {
    fn new() -> Self {
        Self {
            tokens: f64::from(2 * limits::CALLS_PER_S),
            at: Instant::now(),
        }
    }

    fn take(&mut self) -> bool {
        let now = Instant::now();
        let refill = now.duration_since(self.at).as_secs_f64() * f64::from(limits::CALLS_PER_S);
        self.at = now;
        self.tokens = (self.tokens + refill).min(f64::from(2 * limits::CALLS_PER_S));
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

struct Holder {
    outlet: Arc<dyn Outlet>,
    /// Not told where the lease is until [`Hold::ready`]: the plugin must have its `registered` first.
    gated: bool,
    scope: Scope,
    watch: BTreeSet<String>,
    notices: bool,
    interval_ms: u32,
    calls: Bucket,
}

struct Running {
    link: Arc<Link>,
    /// The configuration (address and token) it was started with.
    config: u64,
}

#[derive(Default)]
struct State {
    endpoint: Option<Endpoint>,
    token: Option<Token>,
    /// Whether the saved link has been read (it is read the first time a plugin holds the lease).
    loaded: bool,
    /// Why the saved link could not be used (words, never the token).
    unusable: Option<String>,
    /// Rises with every change of address or token.
    config: u64,
    holders: BTreeMap<u64, Holder>,
    next_holder: u64,
    link: Option<Running>,
    /// Which link the news is from: news from another is ignored.
    generation: u64,
    view: Option<HaLinkState>,
    /// The merged state of the watched entities.
    mirror: HashMap<String, EntityState>,
}

/// Where the TV hears of changes of [`Info`].
type InfoSink = Arc<dyn Fn(&Info) + Send + Sync>;

struct Inner {
    settings: LeaseSettings,
    state: Mutex<State>,
    /// Told whenever [`Info`] changes.
    on_info: Mutex<Option<InfoSink>>,
    /// The last [`Info`] told, so that a change that is not one is not told.
    told: Mutex<Option<Info>>,
}

/// This TV's connection to Home Assistant. Shared as an `Arc`.
pub struct HaLease {
    inner: Arc<Inner>,
}

/// Who is to be told what, collected under the lock and sent after it.
type Outgoing = Vec<(Arc<dyn Outlet>, ToPlugin)>;

fn send_all(outgoing: Outgoing) {
    for (outlet, message) in outgoing {
        let _ = outlet.send(message);
    }
}

fn link_state(kind: HaLinkKind, detail: impl Into<String>) -> HaLinkState {
    HaLinkState {
        state: kind,
        detail: Some(detail.into()),
    }
}

fn unconfigured_state(state: &State) -> HaLinkState {
    let words = match (
        &state.unusable,
        state.endpoint.is_some(),
        state.token.is_some(),
    ) {
        (Some(why), _, _) => why.as_str(),
        (None, false, false) => "Home Assistant is not linked yet",
        (None, false, true) => "Give the address of Home Assistant",
        (None, true, false) => "Give an access token",
        (None, true, true) => "Not connected",
    };
    link_state(HaLinkKind::Unconfigured, words)
}

fn kind_word(kind: HaLinkKind) -> &'static str {
    match kind.known() {
        HaLinkKind::Unconfigured => "unconfigured",
        HaLinkKind::Connecting => "connecting",
        HaLinkKind::Connected => "connected",
        HaLinkKind::AuthFailed => "auth_failed",
        _ => "offline",
    }
}

impl HaLease {
    pub fn new(settings: LeaseSettings) -> Self {
        Self {
            inner: Arc::new(Inner {
                settings,
                state: Mutex::new(State::default()),
                on_info: Mutex::new(None),
                told: Mutex::new(None),
            }),
        }
    }

    /// Where the TV hears of changes of [`Info`] (it publishes them as the `ha.link` topic).
    pub fn on_info(&self, tell: InfoSink) {
        *self.inner.on_info.lock() = Some(tell);
    }

    /// A plugin holds the lease: the link starts if there is an address and a token. Dropping the [`Hold`] lets go.
    pub fn acquire(&self, outlet: Arc<dyn Outlet>, scope: Scope) -> Hold {
        let inner = self.inner.clone();
        let mut state = inner.state.lock();
        inner.load(&mut state);
        state.next_holder += 1;
        let id = state.next_holder;
        state.holders.insert(
            id,
            Holder {
                outlet,
                gated: true,
                scope,
                watch: BTreeSet::new(),
                notices: false,
                interval_ms: limits::DEFAULT_INTERVAL_MS,
                calls: Bucket::new(),
            },
        );
        inner.reconcile(&mut state);
        drop(state);
        inner.tell();
        Hold { inner, id }
    }

    /// What the TV shows of the lease now.
    pub fn info(&self) -> Info {
        let state = self.inner.state.lock();
        self.inner.info_of(&state)
    }

    /// Whether a link thread exists now.
    pub fn running(&self) -> bool {
        self.inner.state.lock().link.is_some()
    }

    /// The saved link has been written again (by `ha-link`): read it. A changed address or token makes a new link (and is the one
    /// way out of a token Home Assistant refused); the same one asks the link to try again at once.
    pub fn refresh(&self) {
        {
            let mut state = self.inner.state.lock();
            let (endpoint, token) = (state.endpoint.clone(), state.token.clone());
            self.inner.read_file(&mut state);
            if state.endpoint != endpoint || state.token != token {
                state.config += 1;
            } else if let Some(running) = &state.link {
                running.link.retry();
            }
            self.inner.reconcile(&mut state);
        }
        self.inner.tell();
    }

    /// Forgets the link: the saved file goes and the connection ends. The person's "forget this connection"; it narrows what the TV
    /// may do, so Settings may ask for it.
    pub fn forget(&self) {
        {
            let mut state = self.inner.state.lock();
            ha_store::clear(&self.inner.settings.link_file);
            state.loaded = true;
            state.unusable = None;
            state.endpoint = None;
            state.token = None;
            state.config += 1;
            self.inner.reconcile(&mut state);
        }
        self.inner.tell();
    }
}

impl Inner {
    /// Reads the saved link the first time it is needed.
    fn load(&self, state: &mut State) {
        if state.loaded {
            return;
        }
        state.loaded = true;
        self.read_file(state);
    }

    fn read_file(&self, state: &mut State) {
        state.loaded = true;
        match ha_store::load(&self.settings.link_file) {
            Ok(Some(link)) => {
                state.endpoint = Some(link.endpoint);
                state.token = Some(link.token);
                state.unusable = None;
            }
            Ok(None) => {
                state.endpoint = None;
                state.token = None;
                state.unusable = None;
            }
            Err(e) => {
                (self.settings.log)(format!("home assistant: {e}"));
                state.endpoint = None;
                state.token = None;
                state.unusable = Some("The saved Home Assistant link cannot be used".to_owned());
            }
        }
    }

    fn info_of(&self, state: &State) -> Info {
        let (word, detail) = if state.holders.is_empty() {
            ("off", String::new())
        } else {
            let view = state
                .view
                .clone()
                .unwrap_or_else(|| unconfigured_state(state));
            (kind_word(view.state), view.detail.unwrap_or_default())
        };
        Info {
            state: word,
            detail,
            address: state.endpoint.as_ref().map(Endpoint::display),
            token: state.token.is_some(),
            holders: state.holders.len(),
        }
    }

    /// Tells the TV where the lease is, when that is not what it was last told.
    fn tell(&self) {
        let info = {
            let state = self.state.lock();
            self.info_of(&state)
        };
        {
            let mut told = self.told.lock();
            if told.as_ref() == Some(&info) {
                return;
            }
            *told = Some(info.clone());
        }
        let tell = self.on_info.lock().clone();
        if let Some(tell) = tell {
            tell(&info);
        }
    }

    /// Makes what runs what should run: a link when a plugin holds the lease and there is an address and a token, none otherwise.
    fn reconcile(self: &Arc<Self>, state: &mut State) {
        let want = !state.holders.is_empty() && state.endpoint.is_some() && state.token.is_some();
        let stale = state
            .link
            .as_ref()
            .is_some_and(|l| l.config != state.config);
        if state.link.is_some() && (!want || stale) {
            if let Some(old) = state.link.take() {
                old.link.stop();
            }
            state.generation += 1;
            state.mirror.clear();
        }
        if want && state.link.is_none() {
            self.start(state);
        } else if !want {
            let view = unconfigured_state(state);
            self.set_view(state, view);
        }
    }

    fn start(self: &Arc<Self>, state: &mut State) {
        let (Some(endpoint), Some(token)) = (state.endpoint.clone(), state.token.clone()) else {
            return;
        };
        state.generation += 1;
        let generation = state.generation;
        let config = LinkConfig {
            endpoint,
            token,
            policy: self.settings.policy,
            timing: self.settings.timing,
            tls: self.settings.tls.clone(),
        };
        let sink = LeaseSink {
            inner: Arc::downgrade(self),
            generation,
        };
        match Link::start(config, Box::new(sink)) {
            Ok(link) => {
                state.link = Some(Running {
                    link: Arc::new(link),
                    config: state.config,
                });
                self.push_wishes(state);
                let view = link_state(HaLinkKind::Connecting, "Connecting to Home Assistant");
                self.set_view(state, view);
            }
            Err(e) => {
                (self.settings.log)(format!("home assistant: the link could not start: {e}"));
                let view = link_state(
                    HaLinkKind::Offline,
                    "Home Assistant cannot be reached from here",
                );
                self.set_view(state, view);
            }
        }
    }

    /// Tells the link what the holders together want: the entities, the notifications, how soon.
    fn push_wishes(&self, state: &State) {
        let Some(running) = &state.link else {
            return;
        };
        let union: BTreeSet<String> = state
            .holders
            .values()
            .flat_map(|h| h.watch.iter().cloned())
            .collect();
        running.link.watch(union);
        running
            .link
            .notices(state.holders.values().any(|h| h.notices));
        let shortest = state
            .holders
            .values()
            .map(|h| h.interval_ms)
            .min()
            .unwrap_or(limits::DEFAULT_INTERVAL_MS);
        running
            .link
            .interval(Duration::from_millis(u64::from(shortest)));
    }

    /// The lease is somewhere else now: every holder is told.
    fn set_view(&self, state: &mut State, view: HaLinkState) {
        if state.view.as_ref() == Some(&view) {
            return;
        }
        state.view = Some(view.clone());
        let outgoing: Outgoing = state
            .holders
            .values()
            .filter(|h| !h.gated)
            .map(|h| (h.outlet.clone(), ToPlugin::HaLink(view.clone())))
            .collect();
        send_all(outgoing);
    }

    // -- what the link says -----------------------------------------------------------------------------------------------------

    fn on_status(self: &Arc<Self>, generation: u64, status: Status) {
        {
            let mut state = self.state.lock();
            if generation != state.generation || state.link.is_none() {
                return;
            }
            let view = match status {
                Status::Connecting => {
                    link_state(HaLinkKind::Connecting, "Connecting to Home Assistant")
                }
                Status::Connected { version } => {
                    let words = if version.is_empty() {
                        "Connected to Home Assistant".to_owned()
                    } else {
                        format!("Connected to Home Assistant {version}")
                    };
                    link_state(HaLinkKind::Connected, words)
                }
                Status::AuthInvalid => link_state(
                    HaLinkKind::AuthFailed,
                    "Home Assistant did not accept the access token",
                ),
                Status::Offline { reason } => link_state(
                    HaLinkKind::Offline,
                    format!("Cannot reach Home Assistant: {reason}"),
                ),
                // The TV's lease runs on the token link alone; a bridged status is the Show's and is read as its nearest word.
                Status::Waiting => link_state(HaLinkKind::Connecting, "Waiting for Home Assistant"),
                Status::Bridged { ha_version, .. } => link_state(
                    HaLinkKind::Connected,
                    format!("Connected to Home Assistant {ha_version}"),
                ),
                Status::Stopped => return,
            };
            self.set_view(&mut state, view);
        }
        self.tell();
    }

    fn on_states(self: &Arc<Self>, generation: u64, batch: StateBatch) {
        let mut state = self.state.lock();
        if generation != state.generation || state.link.is_none() {
            return;
        }
        if batch.initial {
            state.mirror.clear();
        }
        for entity in &batch.states {
            state.mirror.insert(entity.id.clone(), entity.clone());
        }
        for id in &batch.removed {
            state.mirror.remove(id);
        }
        let mut outgoing = Vec::new();
        for holder in state.holders.values() {
            if holder.watch.is_empty() {
                continue;
            }
            let states: Vec<&EntityState> = batch
                .states
                .iter()
                .filter(|s| holder.watch.contains(&s.id))
                .collect();
            let removed: Vec<String> = batch
                .removed
                .iter()
                .filter(|id| holder.watch.contains(*id))
                .cloned()
                .collect();
            if states.is_empty() && removed.is_empty() && !batch.initial {
                continue;
            }
            for message in chunks(batch.initial, &states, removed) {
                outgoing.push((holder.outlet.clone(), message));
            }
        }
        drop(state);
        send_all(outgoing);
    }

    fn on_notices(self: &Arc<Self>, generation: u64, notices: Vec<Notice>) {
        let state = self.state.lock();
        if generation != state.generation || state.link.is_none() {
            return;
        }
        let mut outgoing = Vec::new();
        for holder in state
            .holders
            .values()
            .filter(|h| h.notices && h.scope.notices)
        {
            for n in &notices {
                outgoing.push((
                    holder.outlet.clone(),
                    ToPlugin::HaNotice(HaNotice {
                        kind: match n.kind {
                            NoticeKind::Current => HaNoticeKind::Current,
                            NoticeKind::Added => HaNoticeKind::Added,
                            NoticeKind::Updated => HaNoticeKind::Updated,
                            NoticeKind::Removed => HaNoticeKind::Removed,
                        },
                        id: n.id.clone(),
                        title: n.title.chars().take(200).collect(),
                        message: n.message.chars().take(1000).collect(),
                    }),
                ));
            }
        }
        drop(state);
        send_all(outgoing);
    }
}

/// States as frames that fit: each chunk of an initial batch is marked initial, so that nothing in it is taken for an event.
pub(crate) fn chunks(
    initial: bool,
    states: &[&EntityState],
    removed: Vec<String>,
) -> Vec<ToPlugin> {
    let mut out = Vec::new();
    let mut current: Vec<HaState> = Vec::new();
    let mut size = 0;
    for entity in states {
        let wire = HaState {
            id: entity.id.clone(),
            state: entity.state.clone(),
            attrs: entity.attrs.clone(),
            changed_at: entity.changed_at.map(|t| t as i64),
        };
        let bytes = serde_json::to_vec(&wire).map_or(0, |b| b.len());
        if !current.is_empty() && size + bytes > CHUNK_BYTES {
            out.push(ToPlugin::HaStates(HaStates {
                initial,
                states: std::mem::take(&mut current),
                removed: Vec::new(),
            }));
            size = 0;
        }
        size += bytes;
        current.push(wire);
    }
    if !current.is_empty() || !removed.is_empty() || initial {
        out.push(ToPlugin::HaStates(HaStates {
            initial,
            states: current,
            removed,
        }));
    }
    out
}

struct LeaseSink {
    inner: Weak<Inner>,
    generation: u64,
}

impl Sink for LeaseSink {
    fn status(&mut self, status: Status) {
        if let Some(inner) = self.inner.upgrade() {
            inner.on_status(self.generation, status);
        }
    }

    fn states(&mut self, batch: StateBatch) {
        if let Some(inner) = self.inner.upgrade() {
            inner.on_states(self.generation, batch);
        }
    }

    fn notices(&mut self, notices: Vec<Notice>) {
        if let Some(inner) = self.inner.upgrade() {
            inner.on_notices(self.generation, notices);
        }
    }
}

/// What one plugin holds: its place among the holders. Dropping it lets go (the link stops when the last holder does).
pub struct Hold {
    inner: Arc<Inner>,
    id: u64,
}

/// Why a request of a plugin is refused, in words for its author.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub code: &'static str,
    pub reason: String,
}

fn refuse(code: &'static str, reason: impl Into<String>) -> Refusal {
    Refusal {
        code,
        reason: reason.into(),
    }
}

impl Hold {
    /// The plugin has been told it is registered: from here on it is told where the lease is, now and at every change. (A plugin
    /// is told nothing before its `registered`, so the session calls this once its reply is on the wire.)
    pub fn ready(&self) {
        let (outlet, view) = {
            let mut state = self.inner.state.lock();
            let view = state
                .view
                .clone()
                .unwrap_or_else(|| unconfigured_state(&state));
            let Some(holder) = state.holders.get_mut(&self.id) else {
                return;
            };
            if !holder.gated {
                return;
            }
            holder.gated = false;
            (holder.outlet.clone(), view)
        };
        let _ = outlet.send(ToPlugin::HaLink(view));
    }

    /// Where the lease is now.
    pub fn view(&self) -> HaLinkState {
        let state = self.inner.state.lock();
        state
            .view
            .clone()
            .unwrap_or_else(|| unconfigured_state(&state))
    }

    /// The plugin's wish to watch these entities (and notifications), replacing the last. Nothing is changed when anything is refused.
    pub fn watch(&self, watch: &HaWatch) -> Result<(), Refusal> {
        let mut state = self.inner.state.lock();
        let Some(holder) = state.holders.get(&self.id) else {
            return Err(refuse("closed", "the lease has been let go"));
        };
        if watch.entities.len() > limits::MAX_WATCHED {
            return Err(refuse(
                "too_many",
                format!("at most {} entities can be watched", limits::MAX_WATCHED),
            ));
        }
        let mut wanted = BTreeSet::new();
        for id in &watch.entities {
            let Some(domain) = limits::entity_domain(id) else {
                return Err(refuse(
                    "invalid",
                    format!("{id:?} is not an entity id (domain.name)"),
                ));
            };
            if !holder.scope.read.contains(domain) {
                return Err(refuse(
                    "not_permitted",
                    format!(
                        "this plugin did not ask to read {domain} entities, so it was not approved ({id})"
                    ),
                ));
            }
            wanted.insert(id.clone());
        }
        if watch.notices && !holder.scope.notices {
            return Err(refuse(
                "not_permitted",
                "this plugin did not ask for Home Assistant's notifications, so it was not approved",
            ));
        }
        let before: BTreeSet<String> = state
            .holders
            .values()
            .flat_map(|h| h.watch.iter().cloned())
            .collect();
        let interval = limits::interval_ms(watch.interval_ms);
        let Some(holder) = state.holders.get_mut(&self.id) else {
            return Err(refuse("closed", "the lease has been let go"));
        };
        holder.watch = wanted.clone();
        holder.notices = watch.notices;
        holder.interval_ms = interval;
        let after: BTreeSet<String> = state
            .holders
            .values()
            .flat_map(|h| h.watch.iter().cloned())
            .collect();
        self.inner.push_wishes(&state);
        // When the union did not change the link will say nothing new: this holder is given what is already known.
        if before == after && !wanted.is_empty() {
            let known: Vec<&EntityState> = wanted
                .iter()
                .filter_map(|id| state.mirror.get(id))
                .collect();
            let connected = matches!(
                state.view.as_ref().map(|v| v.state),
                Some(HaLinkKind::Connected)
            );
            if connected && let Some(holder) = state.holders.get(&self.id) {
                let outgoing: Outgoing = chunks(true, &known, Vec::new())
                    .into_iter()
                    .map(|m| (holder.outlet.clone(), m))
                    .collect();
                drop(state);
                send_all(outgoing);
            }
        }
        Ok(())
    }

    /// A service call. The answer is a `ha.result` for `op`, now or when Home Assistant has answered.
    pub fn call(&self, call: &HaCall) -> Result<(), Refusal> {
        let (outlet, link) = {
            let mut state = self.inner.state.lock();
            let link = state.link.as_ref().map(|r| r.link.clone());
            let Some(holder) = state.holders.get_mut(&self.id) else {
                return Err(refuse("closed", "the lease has been let go"));
            };
            if NEVER_CALLED.contains(&call.domain.as_str()) {
                return Err(refuse(
                    "not_permitted",
                    format!(
                        "this TV does not let a plugin call {} services: doors, alarms, cameras and water are the owner's to operate",
                        call.domain
                    ),
                ));
            }
            if !holder.scope.call.contains(&call.domain) {
                return Err(refuse(
                    "not_permitted",
                    format!(
                        "this plugin did not ask to call {} services, so it was not approved",
                        call.domain
                    ),
                ));
            }
            if !limits::service_ok(&call.service) {
                return Err(refuse(
                    "invalid",
                    format!("{:?} is not a service name", call.service),
                ));
            }
            if call.entity_ids.is_empty() || call.entity_ids.len() > limits::MAX_CALL_ENTITIES {
                return Err(refuse(
                    "invalid",
                    format!(
                        "a call names between 1 and {} entities",
                        limits::MAX_CALL_ENTITIES
                    ),
                ));
            }
            for id in &call.entity_ids {
                if limits::entity_domain(id) != Some(call.domain.as_str()) {
                    return Err(refuse(
                        "invalid",
                        format!("{id:?} is not an entity of the {} domain", call.domain),
                    ));
                }
            }
            limits::check_data(&call.data).map_err(|why| refuse("invalid", why))?;
            if !holder.calls.take() {
                return Err(refuse(
                    "rate",
                    format!("at most {} calls a second", limits::CALLS_PER_S),
                ));
            }
            (holder.outlet.clone(), link)
        };
        let op = call.op;
        let answer = move |result: Result<(), String>| {
            let message = match result {
                Ok(()) => ToPlugin::HaResult {
                    op,
                    ok: true,
                    error: None,
                },
                Err(why) => ToPlugin::HaResult {
                    op,
                    ok: false,
                    error: Some(why),
                },
            };
            let _ = outlet.send(message);
        };
        match link {
            Some(link) => link.call(
                ha_link::Call {
                    domain: call.domain.clone(),
                    service: call.service.clone(),
                    entity_ids: call.entity_ids.clone(),
                    data: call.data.clone(),
                },
                Box::new(answer),
            ),
            None => answer(Err("Home Assistant is not connected".into())),
        }
        Ok(())
    }

    /// A page of rooms or entities. The answer is a `ha.page` for the query, now or when Home Assistant has answered.
    pub fn list(&self, list: &HaList) -> Result<(), Refusal> {
        let (outlet, link, scope, view) = {
            let state = self.inner.state.lock();
            let Some(holder) = state.holders.get(&self.id) else {
                return Err(refuse("closed", "the lease has been let go"));
            };
            (
                holder.outlet.clone(),
                state.link.as_ref().map(|r| r.link.clone()),
                holder.scope.clone(),
                state.view.clone(),
            )
        };
        let query = list.query;
        let page =
            move |state: HaPageState, rows: Vec<HaRow>, more: bool, error: Option<String>| {
                ToPlugin::HaPage(HaPage {
                    query,
                    state,
                    rows,
                    more,
                    error,
                })
            };
        let domains: BTreeSet<String> = match list.what {
            HaWhat::Areas => {
                if !scope.areas {
                    return Err(refuse(
                        "not_permitted",
                        "this plugin did not ask to read the rooms, so it was not approved",
                    ));
                }
                BTreeSet::new()
            }
            HaWhat::Entities => {
                if list.search.as_ref().is_some_and(|s| s.chars().count() > 64) {
                    return Err(refuse("invalid", "a search is at most 64 characters"));
                }
                let wanted: BTreeSet<String> = if list.domains.is_empty() {
                    scope.read.clone()
                } else {
                    list.domains.iter().cloned().collect()
                };
                if let Some(domain) = wanted.iter().find(|d| !scope.read.contains(*d)) {
                    return Err(refuse(
                        "not_permitted",
                        format!(
                            "this plugin did not ask to read {domain} entities, so it was not approved"
                        ),
                    ));
                }
                wanted
            }
        };
        let offset = list.offset as usize;
        let limit = usize::from(list.limit.clamp(1, limits::MAX_PAGE_ROWS));
        let not_ready = |view: Option<HaLinkState>| -> (HaPageState, Option<String>) {
            match view {
                Some(HaLinkState {
                    state: HaLinkKind::AuthFailed,
                    detail,
                }) => (HaPageState::AuthFailed, detail),
                Some(HaLinkState {
                    state: HaLinkKind::Unconfigured,
                    detail,
                }) => (HaPageState::Unconfigured, detail),
                Some(HaLinkState { detail, .. }) => (HaPageState::Offline, detail),
                None => (HaPageState::Unconfigured, None),
            }
        };
        let connected = matches!(view.as_ref().map(|v| v.state), Some(HaLinkKind::Connected));
        let Some(link) = link.filter(|_| connected) else {
            let (state, error) = not_ready(view);
            let _ = outlet.send(page(state, Vec::new(), false, error));
            return Ok(());
        };
        match list.what {
            HaWhat::Areas => link.areas(Box::new(move |result| {
                let message = match result {
                    Ok(areas) => {
                        let more = areas.len() > offset + limit;
                        let rows = areas
                            .into_iter()
                            .skip(offset)
                            .take(limit)
                            .map(|a| HaRow {
                                id: a.id,
                                name: a.name,
                                area: None,
                            })
                            .collect();
                        page(HaPageState::Ready, rows, more, None)
                    }
                    Err(why) => page(HaPageState::Failed, Vec::new(), false, Some(why)),
                };
                let _ = outlet.send(message);
            })),
            HaWhat::Entities => {
                let area = list.area.clone();
                let minor = list.minor;
                // Any of the words, in the name or the id, in any case.
                let words: Vec<String> = list
                    .search
                    .as_deref()
                    .map(|s| s.split_whitespace().map(str::to_lowercase).collect())
                    .unwrap_or_default();
                link.entities(Box::new(move |result| {
                    let message = match result {
                        Ok(all) => {
                            let wanted: Vec<&ha_link::EntityInfo> = all
                                .iter()
                                .filter(|e| {
                                    limits::entity_domain(&e.id)
                                        .is_some_and(|d| domains.contains(d))
                                        && (minor || !e.minor)
                                        && area.as_ref().is_none_or(|a| e.area.as_ref() == Some(a))
                                        && (words.is_empty() || {
                                            let hay = format!("{} {}", e.name.to_lowercase(), e.id);
                                            words.iter().any(|w| hay.contains(w.as_str()))
                                        })
                                })
                                .collect();
                            let more = wanted.len() > offset + limit;
                            let rows = wanted
                                .into_iter()
                                .skip(offset)
                                .take(limit)
                                .map(|e| HaRow {
                                    id: e.id.clone(),
                                    name: e.name.clone(),
                                    area: e.area.clone(),
                                })
                                .collect();
                            page(HaPageState::Ready, rows, more, None)
                        }
                        Err(why) => page(HaPageState::Failed, Vec::new(), false, Some(why)),
                    };
                    let _ = outlet.send(message);
                }));
            }
        }
        Ok(())
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        {
            let mut state = self.inner.state.lock();
            state.holders.remove(&self.id);
            if state.holders.is_empty() {
                // The last one let go: nothing keeps the link or the state.
                state.mirror = HashMap::new();
                state.view = None;
            }
            self.inner.reconcile(&mut state);
            if !state.holders.is_empty() {
                self.inner.push_wishes(&state);
            }
        }
        self.inner.tell();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ha_link::fake::{FakeEntity, FakeHa};
    use serde_json::json;
    use std::time::Duration;

    const TOKEN: &str = "eyJhbGciOiJIUzI1NiJ9.lease-test-token.signature";

    /// A plugin's end of the connection: what the lease said to it.
    #[derive(Default)]
    struct Recorder(Mutex<Vec<ToPlugin>>);

    impl Outlet for Recorder {
        fn send(&self, message: ToPlugin) -> bool {
            self.0.lock().push(message);
            true
        }

        fn poke(&self) {}
    }

    impl Recorder {
        fn all(&self) -> Vec<ToPlugin> {
            self.0.lock().clone()
        }

        fn wait(&self, what: &str, mut pick: impl FnMut(&ToPlugin) -> bool) -> ToPlugin {
            let end = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(found) = self.0.lock().iter().find(|m| pick(m)) {
                    return found.clone();
                }
                assert!(Instant::now() < end, "no {what}: {:?}", self.all());
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        fn link_states(&self) -> Vec<HaLinkKind> {
            self.all()
                .into_iter()
                .filter_map(|m| match m {
                    ToPlugin::HaLink(l) => Some(l.state),
                    _ => None,
                })
                .collect()
        }
    }

    fn lease_in(dir: &std::path::Path) -> (HaLease, PathBuf) {
        let file = dir.join("ha").join("link.json");
        let lease = HaLease::new(LeaseSettings {
            link_file: file.clone(),
            policy: AddressPolicy::TESTING,
            timing: Timing {
                backoff_min: Duration::from_millis(30),
                backoff_max: Duration::from_millis(200),
                coalesce: Duration::from_millis(20),
                ..Timing::default()
            },
            tls: None,
            log: Arc::new(|_| {}),
        });
        (lease, file)
    }

    fn scope(read: &[&str], call: &[&str]) -> Scope {
        Scope {
            read: read.iter().map(|s| (*s).to_owned()).collect(),
            areas: true,
            notices: true,
            call: call.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    fn house() -> FakeHa {
        let ha = FakeHa::start(TOKEN);
        ha.add_area("kitchen", "Kitchen");
        ha.add_entity(FakeEntity::new("light.sink", "Sink light").in_area("kitchen"));
        ha.add_entity(FakeEntity::new("lock.front", "Front door").in_area("kitchen"));
        ha.set_state("light.sink", "on", json!({"friendly_name": "Sink light"}));
        ha.set_state(
            "lock.front",
            "locked",
            json!({"friendly_name": "Front door"}),
        );
        ha
    }

    fn watch(entities: &[&str]) -> HaWatch {
        HaWatch::new()
            .entities(entities.iter().copied())
            .interval_ms(250)
    }

    fn call(domain: &str, service: &str, entity: &str) -> HaCall {
        HaCall::new(1, domain, service, [entity])
    }

    #[test]
    fn nothing_runs_until_a_plugin_holds_the_lease_and_there_is_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let (lease, file) = lease_in(dir.path());
        assert!(!lease.running());
        assert_eq!(lease.info().state, "off");
        let plugin = Arc::new(Recorder::default());
        let hold = lease.acquire(plugin.clone(), scope(&["light"], &["light"]));
        assert!(!lease.running(), "no link, no thread");
        let view = hold.view();
        assert_eq!(view.state, HaLinkKind::Unconfigured);
        assert_eq!(
            view.detail.as_deref(),
            Some("Home Assistant is not linked yet")
        );
        assert_eq!(lease.info().state, "unconfigured");
        assert!(lease.info().address.is_none() && !lease.info().token);

        // The owner's command line saves a link and tells the helper.
        let ha = house();
        ha_store::save(&file, &ha.url(), TOKEN).unwrap();
        lease.refresh();
        assert!(lease.running());
        hold.ready();
        plugin.wait(
            "a connected lease",
            |m| matches!(m, ToPlugin::HaLink(l) if l.state == HaLinkKind::Connected),
        );
        let info = lease.info();
        assert_eq!(info.state, "connected");
        assert_eq!(info.address.as_deref(), Some(ha.url().as_str()));
        assert!(info.token && info.holders == 1);
        assert!(
            !format!("{info:?}").contains(TOKEN),
            "the token is nowhere in what the TV is told"
        );

        drop(hold);
        assert!(!lease.running(), "the last holder letting go ends the link");
        assert_eq!(lease.info().state, "off");
    }

    #[test]
    fn a_plugin_hears_of_the_lease_only_after_it_is_ready_and_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let (lease, file) = lease_in(dir.path());
        let ha = house();
        ha_store::save(&file, &ha.url(), TOKEN).unwrap();
        let plugin = Arc::new(Recorder::default());
        let hold = lease.acquire(plugin.clone(), scope(&["light"], &[]));
        // The link starts at once, and the plugin has not had its `registered`: it is told nothing.
        std::thread::sleep(Duration::from_millis(300));
        assert!(plugin.all().is_empty(), "{:?}", plugin.all());
        hold.ready();
        hold.ready();
        plugin.wait("the link", |m| matches!(m, ToPlugin::HaLink(_)));
        plugin.wait(
            "connected",
            |m| matches!(m, ToPlugin::HaLink(l) if l.state == HaLinkKind::Connected),
        );
        let states = plugin.link_states();
        assert_eq!(
            states
                .iter()
                .filter(|s| **s == HaLinkKind::Connected)
                .count(),
            1,
            "ready twice says it once: {states:?}"
        );
    }

    #[test]
    fn a_plugin_watches_only_what_its_manifest_names_and_a_refusal_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (lease, file) = lease_in(dir.path());
        let ha = house();
        ha_store::save(&file, &ha.url(), TOKEN).unwrap();
        let plugin = Arc::new(Recorder::default());
        let hold = lease.acquire(plugin.clone(), scope(&["light"], &[]));
        hold.ready();
        plugin.wait(
            "connected",
            |m| matches!(m, ToPlugin::HaLink(l) if l.state == HaLinkKind::Connected),
        );
        let refused = hold
            .watch(&watch(&["light.sink", "lock.front"]))
            .unwrap_err();
        assert_eq!(refused.code, "not_permitted", "{refused:?}");
        assert_eq!(
            hold.watch(&watch(&["Light.sink"])).unwrap_err().code,
            "invalid"
        );
        let mut notices = watch(&["light.sink"]);
        let mut open = scope(&["light"], &[]);
        open.notices = false;
        let other = Arc::new(Recorder::default());
        let closed = lease.acquire(other, open);
        notices.notices = true;
        assert_eq!(closed.watch(&notices).unwrap_err().code, "not_permitted");
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            !plugin
                .all()
                .iter()
                .any(|m| matches!(m, ToPlugin::HaStates(_))),
            "a refused watch watched nothing"
        );
        hold.watch(&watch(&["light.sink"])).unwrap();
        let first = plugin.wait("the first states", |m| matches!(m, ToPlugin::HaStates(_)));
        match first {
            ToPlugin::HaStates(batch) => {
                assert!(batch.initial);
                assert_eq!(batch.states.len(), 1);
                assert_eq!(batch.states[0].id, "light.sink");
                assert_eq!(batch.states[0].state, "on");
            }
            other => panic!("{other:?}"),
        }
        // A change is a later batch, not marked initial, and a lock it never asked for is not in it.
        ha.set_state(
            "lock.front",
            "unlocked",
            json!({"friendly_name": "Front door"}),
        );
        ha.set_state("light.sink", "off", json!({"friendly_name": "Sink light"}));
        let later = plugin.wait("a change", |m| {
            matches!(m, ToPlugin::HaStates(b) if !b.initial && b.states.iter().any(|s| s.state == "off"))
        });
        if let ToPlugin::HaStates(batch) = later {
            assert!(batch.states.iter().all(|s| s.id == "light.sink"));
        }
    }

    #[test]
    fn a_call_is_held_to_the_manifest_to_plain_values_to_a_rate_and_is_never_a_door() {
        let dir = tempfile::tempdir().unwrap();
        let (lease, file) = lease_in(dir.path());
        let ha = house();
        ha_store::save(&file, &ha.url(), TOKEN).unwrap();
        let plugin = Arc::new(Recorder::default());
        // The plugin names a cover and a light; the TV never lets a plugin call a cover.
        let hold = lease.acquire(plugin.clone(), scope(&["light"], &["light", "cover"]));
        hold.ready();
        plugin.wait(
            "connected",
            |m| matches!(m, ToPlugin::HaLink(l) if l.state == HaLinkKind::Connected),
        );
        for (domain, entity, code) in [
            ("scene", "scene.movie", "not_permitted"),
            ("lock", "lock.front", "not_permitted"),
            ("cover", "cover.garage", "not_permitted"),
            ("light", "switch.fan", "invalid"),
        ] {
            let e = hold.call(&call(domain, "turn_on", entity)).unwrap_err();
            assert_eq!(e.code, code, "{domain} {entity}: {e:?}");
        }
        let mut loud = call("light", "turn_on", "light.sink");
        loud.data.insert("shell".into(), json!("rm -rf /"));
        assert_eq!(hold.call(&loud).unwrap_err().code, "invalid");
        assert_eq!(
            hold.call(&call("light", "turn on", "light.sink"))
                .unwrap_err()
                .code,
            "invalid"
        );
        assert!(
            ha.calls().is_empty(),
            "nothing refused reached Home Assistant"
        );

        let mut quiet = call("light", "turn_off", "light.sink");
        quiet.data.insert("transition".into(), json!(2));
        hold.call(&quiet).unwrap();
        let answer = plugin.wait("an answer", |m| matches!(m, ToPlugin::HaResult { .. }));
        assert!(
            matches!(
                answer,
                ToPlugin::HaResult {
                    op: 1,
                    ok: true,
                    ..
                }
            ),
            "{answer:?}"
        );
        assert_eq!(ha.calls().len(), 1);
        assert_eq!(ha.calls()[0].service, "turn_off");

        // Five a second, a burst of twice that: the rest are refused for rate.
        let mut refused = 0;
        for _ in 0..30 {
            if hold.call(&call("light", "toggle", "light.sink")).is_err() {
                refused += 1;
            }
        }
        assert!(refused >= 15, "{refused} refused");
    }

    #[test]
    fn a_call_while_home_assistant_is_away_is_answered_not_lost() {
        let dir = tempfile::tempdir().unwrap();
        let (lease, _file) = lease_in(dir.path());
        let plugin = Arc::new(Recorder::default());
        let hold = lease.acquire(plugin.clone(), scope(&["light"], &["light"]));
        hold.ready();
        hold.call(&call("light", "toggle", "light.sink")).unwrap();
        match plugin.wait("an answer", |m| matches!(m, ToPlugin::HaResult { .. })) {
            ToPlugin::HaResult { ok, error, .. } => {
                assert!(!ok);
                assert_eq!(error.as_deref(), Some("Home Assistant is not connected"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rooms_and_entities_are_listed_only_as_far_as_the_manifest_reaches() {
        let dir = tempfile::tempdir().unwrap();
        let (lease, file) = lease_in(dir.path());
        let ha = house();
        ha_store::save(&file, &ha.url(), TOKEN).unwrap();
        let plugin = Arc::new(Recorder::default());
        let hold = lease.acquire(plugin.clone(), scope(&["light"], &[]));
        hold.ready();
        plugin.wait(
            "connected",
            |m| matches!(m, ToPlugin::HaLink(l) if l.state == HaLinkKind::Connected),
        );
        let ask = |query, what, domains: &[&str]| {
            HaList::new(query, what)
                .domains(domains.iter().copied())
                .limit(24)
        };
        hold.list(&ask(1, HaWhat::Areas, &[])).unwrap();
        hold.list(&ask(2, HaWhat::Entities, &[])).unwrap();
        assert_eq!(
            hold.list(&ask(3, HaWhat::Entities, &["lock"]))
                .unwrap_err()
                .code,
            "not_permitted"
        );
        let rooms = plugin.wait(
            "rooms",
            |m| matches!(m, ToPlugin::HaPage(p) if p.query == 1),
        );
        match rooms {
            ToPlugin::HaPage(p) => {
                assert_eq!(p.state, HaPageState::Ready);
                assert_eq!(p.rows.len(), 1);
                assert_eq!(p.rows[0].name, "Kitchen");
            }
            other => panic!("{other:?}"),
        }
        match plugin.wait(
            "entities",
            |m| matches!(m, ToPlugin::HaPage(p) if p.query == 2),
        ) {
            ToPlugin::HaPage(p) => {
                let ids: Vec<&str> = p.rows.iter().map(|r| r.id.as_str()).collect();
                assert_eq!(ids, vec!["light.sink"], "the lock is not a light");
            }
            other => panic!("{other:?}"),
        }
        // Not connected: the page says why instead of staying silent.
        let (lease2, _) = lease_in(dir.path().join("other").as_path());
        let bare = Arc::new(Recorder::default());
        let hold2 = lease2.acquire(bare.clone(), scope(&["light"], &[]));
        hold2.list(&ask(9, HaWhat::Areas, &[])).unwrap();
        match bare.wait("a page", |m| matches!(m, ToPlugin::HaPage(_))) {
            ToPlugin::HaPage(p) => assert_eq!(p.state, HaPageState::Unconfigured),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_token_home_assistant_refuses_is_tried_once_and_a_saved_link_asks_again() {
        let dir = tempfile::tempdir().unwrap();
        let (lease, file) = lease_in(dir.path());
        let ha = house();
        ha.set_token("another-token");
        ha_store::save(&file, &ha.url(), TOKEN).unwrap();
        let plugin = Arc::new(Recorder::default());
        let hold = lease.acquire(plugin.clone(), scope(&["light"], &[]));
        hold.ready();
        plugin.wait(
            "a refused token",
            |m| matches!(m, ToPlugin::HaLink(l) if l.state == HaLinkKind::AuthFailed),
        );
        assert_eq!(lease.info().state, "auth_failed");
        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(
            ha.auth_attempts(),
            1,
            "Home Assistant bans an address that keeps sending a wrong token"
        );
        // The owner gives the token Home Assistant takes, and says so.
        ha_store::save(&file, &ha.url(), "another-token").unwrap();
        lease.refresh();
        plugin.wait(
            "connected",
            |m| matches!(m, ToPlugin::HaLink(l) if l.state == HaLinkKind::Connected),
        );
        assert_eq!(ha.auth_attempts(), 2);
        // The same link again is a request to try again, not a new link.
        ha.set_token(TOKEN);
        ha_store::save(&file, &ha.url(), "another-token").unwrap();
        plugin.0.lock().clear();
        ha.drop_connections();
        plugin.wait(
            "a refused token again",
            |m| matches!(m, ToPlugin::HaLink(l) if l.state == HaLinkKind::AuthFailed),
        );
        let before = ha.auth_attempts();
        ha.set_token("another-token");
        lease.refresh();
        let end = Instant::now() + Duration::from_secs(10);
        while lease.info().state != "connected" {
            assert!(Instant::now() < end, "{:?}", lease.info());
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(ha.auth_attempts() > before);
    }

    #[test]
    fn forgetting_deletes_the_saved_link_and_ends_the_connection() {
        let dir = tempfile::tempdir().unwrap();
        let (lease, file) = lease_in(dir.path());
        let ha = house();
        ha_store::save(&file, &ha.url(), TOKEN).unwrap();
        let plugin = Arc::new(Recorder::default());
        let hold = lease.acquire(plugin.clone(), scope(&["light"], &[]));
        hold.ready();
        plugin.wait(
            "connected",
            |m| matches!(m, ToPlugin::HaLink(l) if l.state == HaLinkKind::Connected),
        );
        lease.forget();
        assert!(!file.exists(), "the token is deleted");
        assert!(!lease.running());
        let info = lease.info();
        assert_eq!(
            (info.state, info.token, info.address),
            ("unconfigured", false, None)
        );
        plugin.wait(
            "unconfigured",
            |m| matches!(m, ToPlugin::HaLink(l) if l.state == HaLinkKind::Unconfigured),
        );
    }

    #[test]
    fn a_link_file_that_cannot_be_used_says_so_without_quoting_it() {
        let dir = tempfile::tempdir().unwrap();
        let (lease, file) = lease_in(dir.path());
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            format!("{{\"url\":\"ftp://x\",\"token\":\"{TOKEN}\"}}"),
        )
        .unwrap();
        let plugin = Arc::new(Recorder::default());
        let hold = lease.acquire(plugin, scope(&["light"], &[]));
        let view = hold.view();
        assert_eq!(view.state, HaLinkKind::Unconfigured);
        assert_eq!(
            view.detail.as_deref(),
            Some("The saved Home Assistant link cannot be used")
        );
        assert!(!format!("{:?}", lease.info()).contains(TOKEN));
    }

    #[test]
    fn states_that_do_not_fit_one_frame_come_as_several_initial_ones() {
        let big: Vec<EntityState> = (0..400)
            .map(|n| EntityState {
                id: format!("light.l{n}"),
                state: "on".into(),
                attrs: {
                    let mut a = serde_json::Map::new();
                    a.insert("friendly_name".into(), json!("x".repeat(120)));
                    a
                },
                changed_at: Some(1.0),
            })
            .collect();
        let refs: Vec<&EntityState> = big.iter().collect();
        let frames = chunks(true, &refs, Vec::new());
        assert!(frames.len() > 1, "{}", frames.len());
        let mut seen = 0;
        for frame in &frames {
            let ToPlugin::HaStates(batch) = frame else {
                panic!("{frame:?}");
            };
            assert!(batch.initial, "each chunk of an initial batch is initial");
            seen += batch.states.len();
            assert!(serde_json::to_vec(frame).unwrap().len() < plugin_api::MAX_FRAME);
        }
        assert_eq!(seen, 400);
    }
}
