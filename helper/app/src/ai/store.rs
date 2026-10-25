//! Who is paired, what the AI listener's switch says, and the limits only the TV's owner may raise.
//!
//! State lives in `<data dir>/ai/state.json` (mode 0600): the listener switch, the volume limit, and for each assistant its id,
//! its own name for itself, the SHA-256 of its token (the token itself is never stored), and whether it is paused. What each
//! assistant may do lives in the settings layer as the protected owner `ai.<id>` (see `levels`).
//!
//! The rule that shapes this file: **a change that gives more needs a physical remote press, a change that gives less does
//! not.** Giving more is a method that takes `&GateApproval` (see `gate`), a type nothing outside the gate can make, so a code
//! path that widens something without a press does not compile. Giving less (turning the listener off, pausing, revoking,
//! lowering a level, a lock-out) is always possible and is the first thing that applies, even if the state file cannot be
//! written; widening writes the file first and fails if it cannot.
//!
//! A state file that cannot be read means *everything off and nobody paired* (and a warning in the log), never the reverse.

use super::gate::GateApproval;
use super::levels::{Levels, owner_of, schema_for};
use super::tools::clean_text;
use super::types::{AssistantView, Group, PairFailure, Paired};
use crate::settings::SettingsStore;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use settings_schema::{FieldKey, SettingChange, Value as SettingValue};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::fmt;
use std::net::IpAddr;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

/// The pairing code's alphabet: digits 2 to 9 and the letters without I and O, 32 symbols so each carries exactly 5 bits.
const ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
/// Characters in a pairing code (40 bits).
pub const CODE_LEN: usize = 8;
/// How long a pairing window stays open.
pub const PAIRING_TTL_MS: u64 = 5 * 60_000;
/// Wrong codes a window survives.
pub const PAIRING_ATTEMPTS: u8 = 5;
/// Pairing attempts of any kind, from anywhere, in a minute.
pub const PAIRING_RATE_PER_MINUTE: usize = 20;
/// The prefix of every token (so a leaked one is recognisable).
pub const TOKEN_PREFIX: &str = "ltv_";
/// The longest an assistant's name is kept.
pub const NAME_MAX: usize = 40;
/// The volume limit on a fresh TV, on the receiver's 0 to 98 scale.
pub const DEFAULT_MAX_VOLUME: f64 = 50.0;

/// Why a change was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum StoreError {
    NoSuchAssistant,
    /// A narrowing call was given something wider than what is in force: that needs a physical press.
    Widens,
    /// The state could not be saved; nothing was widened.
    Io(String),
    /// The settings layer refused the levels.
    Settings(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchAssistant => f.write_str("no such assistant"),
            Self::Widens => f.write_str("that gives more access and needs the remote"),
            Self::Io(why) => write!(f, "the state could not be saved ({why})"),
            Self::Settings(why) => write!(f, "the permissions could not be saved ({why})"),
        }
    }
}

#[derive(Clone, Debug)]
struct Assistant {
    id: String,
    name: String,
    token_hash: [u8; 32],
    created_at: u64,
    last_seen_at: u64,
    paused: bool,
    calls: u64,
}

#[derive(Clone, Debug)]
struct Pairing {
    code: String,
    expires_at: u64,
    attempts_left: u8,
}

#[derive(Clone, Debug)]
struct Inner {
    listener_enabled: bool,
    enabled_at: Option<u64>,
    max_volume: f64,
    assistants: Vec<Assistant>,
    pairing: Option<Pairing>,
    /// When pairing attempts were made, for the global rate limit.
    attempts: VecDeque<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileDoc {
    version: u32,
    listener: FileListener,
    limits: FileLimits,
    assistants: Vec<FileAssistant>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileListener {
    enabled: bool,
    enabled_at: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileLimits {
    max_volume: f64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileAssistant {
    id: String,
    name: String,
    token_hash: String,
    created_at: u64,
    last_seen_at: u64,
    paused: bool,
}

/// What the TV's UI is shown about one assistant.
#[derive(Clone, Debug, PartialEq)]
pub struct AssistantInfo {
    pub id: String,
    pub name: String,
    pub created_at: u64,
    pub last_seen_at: u64,
    pub paused: bool,
    pub calls: u64,
    pub levels: Levels,
}

impl AssistantInfo {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "createdAt": self.created_at,
            "lastSeenAt": self.last_seen_at,
            "paused": self.paused,
            "calls": self.calls,
            "levels": self.levels.to_json(),
        })
    }
}

/// The open pairing window, as the TV shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairingView {
    /// `ABCD-EFGH`
    pub code: String,
    pub expires_at: u64,
    pub attempts_left: u8,
}

/// What a lock-out changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockOut {
    pub revoked: usize,
    pub listener_was_on: bool,
    pub pairing_was_open: bool,
}

pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;
type OnChange = Arc<dyn Fn() + Send + Sync>;

pub struct Store {
    dir: PathBuf,
    settings: Arc<SettingsStore>,
    clock: Clock,
    inner: Mutex<Inner>,
    on_change: OnceLock<OnChange>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex_32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in text.as_bytes().chunks(2).enumerate() {
        out[i] = u8::from_str_radix(std::str::from_utf8(chunk).ok()?, 16).ok()?;
    }
    Some(out)
}

fn token_hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

/// Equality that takes the same time wherever the first difference is.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    if crate::sys::random_bytes(&mut bytes).is_err() {
        // The kernel's random source failing is not something to paper over with a guessable value.
        crate::log_error!("ai: the system's random source failed; refusing to make a secret");
        return [0u8; N];
    }
    bytes
}

/// A name as it is kept: control characters dropped, white space collapsed, cut to [`NAME_MAX`]; `None` when nothing is left.
pub fn clean_name(name: &str) -> Option<String> {
    let cleaned = clean_text(name, NAME_MAX);
    (!cleaned.is_empty()).then_some(cleaned)
}

/// A pairing code as typed: upper-cased, without spaces or dashes.
fn normalise_code(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .flat_map(char::to_uppercase)
        .collect()
}

fn shown_code(code: &str) -> String {
    format!("{}-{}", &code[..CODE_LEN / 2], &code[CODE_LEN / 2..])
}

fn well_formed_id(id: &str) -> bool {
    id.len() == 12
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Inner {
    fn empty() -> Self {
        Self {
            listener_enabled: false,
            enabled_at: None,
            max_volume: DEFAULT_MAX_VOLUME,
            assistants: Vec::new(),
            pairing: None,
            attempts: VecDeque::new(),
        }
    }

    fn to_file(&self) -> FileDoc {
        FileDoc {
            version: 1,
            listener: FileListener {
                enabled: self.listener_enabled,
                enabled_at: self.enabled_at,
            },
            limits: FileLimits {
                max_volume: self.max_volume,
            },
            assistants: self
                .assistants
                .iter()
                .map(|a| FileAssistant {
                    id: a.id.clone(),
                    name: a.name.clone(),
                    token_hash: hex(&a.token_hash),
                    created_at: a.created_at,
                    last_seen_at: a.last_seen_at,
                    paused: a.paused,
                })
                .collect(),
        }
    }

    fn find(&self, id: &str) -> Option<&Assistant> {
        self.assistants.iter().find(|a| a.id == id)
    }
}

impl Store {
    fn state_file(&self) -> PathBuf {
        self.dir.join("state.json")
    }

    /// Reads `<dir>/state.json` and registers each assistant's permissions page. Anything unreadable is dropped: off, unpaired.
    pub fn load(dir: PathBuf, settings: Arc<SettingsStore>, clock: Clock) -> Arc<Self> {
        let _ = std::fs::create_dir_all(&dir);
        let mut inner = Inner::empty();
        let path = dir.join("state.json");
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<FileDoc>(&text) {
                Ok(doc) if doc.version == 1 => {
                    inner.listener_enabled = doc.listener.enabled;
                    inner.enabled_at = doc.listener.enabled_at;
                    if doc.limits.max_volume.is_finite()
                        && (0.0..=98.0).contains(&doc.limits.max_volume)
                    {
                        inner.max_volume = doc.limits.max_volume;
                    }
                    for a in doc.assistants {
                        let (Some(hash), Some(name)) =
                            (unhex_32(&a.token_hash), clean_name(&a.name))
                        else {
                            crate::log_warn!(
                                "ai: an assistant in the state file is damaged and was dropped"
                            );
                            continue;
                        };
                        if !well_formed_id(&a.id) || inner.find(&a.id).is_some() {
                            crate::log_warn!(
                                "ai: an assistant in the state file has a bad id and was dropped"
                            );
                            continue;
                        }
                        inner.assistants.push(Assistant {
                            id: a.id,
                            name,
                            token_hash: hash,
                            created_at: a.created_at,
                            last_seen_at: a.last_seen_at,
                            paused: a.paused,
                            calls: 0,
                        });
                    }
                }
                Ok(doc) => crate::log_warn!(
                    "ai: {} is version {}, which this helper does not know; AI access starts off and unpaired",
                    path.display(),
                    doc.version
                ),
                Err(e) => crate::log_warn!(
                    "ai: {} is not readable ({e}); AI access starts off and unpaired",
                    path.display()
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => crate::log_warn!(
                "ai: {} cannot be read ({e}); AI access starts off and unpaired",
                path.display()
            ),
        }
        let store = Arc::new(Self {
            dir,
            settings,
            clock,
            inner: Mutex::new(inner),
            on_change: OnceLock::new(),
        });
        let registered: Vec<(String, String)> = store
            .inner
            .lock()
            .assistants
            .iter()
            .map(|a| (a.id.clone(), a.name.clone()))
            .collect();
        for (id, name) in registered {
            store.register_page(&id, &name);
        }
        store
    }

    /// Calls `callback` after every change (outside the lock), so the topics and the listener follow.
    pub fn on_change(&self, callback: impl Fn() + Send + Sync + 'static) {
        let _ = self.on_change.set(Arc::new(callback));
    }

    fn changed(&self) {
        if let Some(callback) = self.on_change.get() {
            callback();
        }
    }

    fn register_page(&self, id: &str, name: &str) {
        match schema_for(id, name, 1) {
            Ok(schema) => {
                if let Err(e) = self.settings.register(schema, None) {
                    crate::log_error!(
                        "ai: the permissions page of an assistant could not be registered ({e})"
                    );
                }
            }
            Err(e) => {
                crate::log_error!("ai: the permissions page of an assistant is not valid ({e})")
            }
        }
    }

    /// Saves `next` and makes it the state. With `narrowing` the state is replaced even if saving fails (what gives less applies
    /// first); otherwise a failed save leaves everything as it was.
    fn commit(&self, inner: &mut Inner, next: Inner, narrowing: bool) -> Result<(), StoreError> {
        let result = serde_json::to_vec_pretty(&next.to_file())
            .map_err(|e| e.to_string())
            .and_then(|bytes| write_private(&self.state_file(), &bytes).map_err(|e| e.to_string()));
        match result {
            Ok(()) => {
                *inner = next;
                Ok(())
            }
            Err(why) if narrowing => {
                crate::log_error!(
                    "ai: the state could not be saved ({why}); the change applies now and may not survive a restart"
                );
                *inner = next;
                Ok(())
            }
            Err(why) => Err(StoreError::Io(why)),
        }
    }

    // ------------------------------------------------------------------------------------------------ reads

    pub fn listener_enabled(&self) -> bool {
        self.inner.lock().listener_enabled
    }

    pub fn listener_since(&self) -> Option<u64> {
        let inner = self.inner.lock();
        inner.enabled_at.filter(|_| inner.listener_enabled)
    }

    pub fn max_volume(&self) -> f64 {
        self.inner.lock().max_volume
    }

    pub fn levels_of(&self, id: &str) -> Option<Levels> {
        self.inner.lock().find(id)?;
        let values = self.settings.effective(&owner_of(id))?;
        Some(Levels::from_values(&values))
    }

    fn info(&self, a: &Assistant) -> AssistantInfo {
        AssistantInfo {
            id: a.id.clone(),
            name: a.name.clone(),
            created_at: a.created_at,
            last_seen_at: a.last_seen_at,
            paused: a.paused,
            calls: a.calls,
            levels: self
                .settings
                .effective(&owner_of(&a.id))
                .map_or_else(Levels::defaults, |v| Levels::from_values(&v)),
        }
    }

    pub fn assistants(&self) -> Vec<AssistantInfo> {
        let list: Vec<Assistant> = self.inner.lock().assistants.clone();
        list.iter().map(|a| self.info(a)).collect()
    }

    pub fn assistant(&self, id: &str) -> Option<AssistantInfo> {
        let found = self.inner.lock().find(id).cloned()?;
        Some(self.info(&found))
    }

    pub fn is_paused(&self, id: &str) -> Option<bool> {
        self.inner.lock().find(id).map(|a| a.paused)
    }

    /// The window open now, if any (an expired one is closed here).
    pub fn pairing_view(&self) -> Option<PairingView> {
        let now = (self.clock)();
        let mut inner = self.inner.lock();
        match &inner.pairing {
            Some(p) if now >= p.expires_at => {
                inner.pairing = None;
                None
            }
            Some(p) => Some(PairingView {
                code: shown_code(&p.code),
                expires_at: p.expires_at,
                attempts_left: p.attempts_left,
            }),
            None => None,
        }
    }

    /// The assistant a bearer token belongs to. The token is hashed and compared with every paired assistant's hash in constant
    /// time; a token of the wrong shape never reaches the comparison.
    pub fn authenticate(&self, token: &str) -> Option<AssistantView> {
        if token.len() != TOKEN_PREFIX.len() + 64
            || !token.starts_with(TOKEN_PREFIX)
            || !token[TOKEN_PREFIX.len()..]
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
        {
            return None;
        }
        let presented = token_hash(token);
        let inner = self.inner.lock();
        let mut found = None;
        for a in &inner.assistants {
            if ct_eq(&a.token_hash, &presented) {
                found = Some(AssistantView {
                    id: a.id.clone(),
                    name: a.name.clone(),
                });
            }
        }
        found
    }

    /// Notes that an assistant made a call (not saved on its own: it goes out with the next change).
    pub fn record_call(&self, id: &str) {
        let now = (self.clock)();
        if let Some(a) = self.inner.lock().assistants.iter_mut().find(|a| a.id == id) {
            a.last_seen_at = now;
            a.calls += 1;
        }
    }

    // ------------------------------------------------------------------------------------------------ narrowing: never needs the remote

    pub fn disable_listener(&self) -> bool {
        let mut inner = self.inner.lock();
        if !inner.listener_enabled {
            return false;
        }
        let mut next = inner.clone();
        next.listener_enabled = false;
        next.enabled_at = None;
        let _ = self.commit(&mut inner, next, true);
        drop(inner);
        self.changed();
        true
    }

    pub fn close_pairing(&self) -> bool {
        let mut inner = self.inner.lock();
        if inner.pairing.is_none() {
            return false;
        }
        inner.pairing = None;
        drop(inner);
        self.changed();
        true
    }

    pub fn pause(&self, id: &str) -> bool {
        let mut inner = self.inner.lock();
        let Some(a) = inner.find(id) else {
            return false;
        };
        if a.paused {
            return false;
        }
        let mut next = inner.clone();
        if let Some(a) = next.assistants.iter_mut().find(|a| a.id == id) {
            a.paused = true;
        }
        let _ = self.commit(&mut inner, next, true);
        drop(inner);
        self.changed();
        true
    }

    /// Unpairs one assistant: its token stops working at once and its permissions page is removed.
    pub fn revoke(&self, id: &str) -> bool {
        let mut inner = self.inner.lock();
        if inner.find(id).is_none() {
            return false;
        }
        let mut next = inner.clone();
        next.assistants.retain(|a| a.id != id);
        let _ = self.commit(&mut inner, next, true);
        drop(inner);
        self.settings.unregister(&owner_of(id));
        self.changed();
        true
    }

    /// Everything off: every assistant unpaired, the listener off, pairing closed.
    pub fn lock_out(&self) -> LockOut {
        let mut inner = self.inner.lock();
        let result = LockOut {
            revoked: inner.assistants.len(),
            listener_was_on: inner.listener_enabled,
            pairing_was_open: inner.pairing.is_some(),
        };
        let ids: Vec<String> = inner.assistants.iter().map(|a| a.id.clone()).collect();
        let mut next = inner.clone();
        next.assistants.clear();
        next.listener_enabled = false;
        next.enabled_at = None;
        next.pairing = None;
        let _ = self.commit(&mut inner, next, true);
        drop(inner);
        for id in ids {
            self.settings.unregister(&owner_of(&id));
        }
        self.changed();
        result
    }

    /// Gives an assistant less (or the same). Refused with [`StoreError::Widens`] if any group would have more than it does:
    /// that is [`Store::widen_levels`], which needs the remote.
    pub fn lower_levels(&self, id: &str, new: &Levels) -> Result<(), StoreError> {
        let current = self.levels_of(id).ok_or(StoreError::NoSuchAssistant)?;
        if current.widened_by(new) {
            return Err(StoreError::Widens);
        }
        self.write_levels(id, new)
    }

    /// Lowers the volume limit (or keeps it); raising it is [`Store::raise_max_volume`].
    pub fn lower_max_volume(&self, level: f64) -> Result<(), StoreError> {
        let mut inner = self.inner.lock();
        if level > inner.max_volume {
            return Err(StoreError::Widens);
        }
        let mut next = inner.clone();
        next.max_volume = level;
        self.commit(&mut inner, next, true)?;
        drop(inner);
        self.changed();
        Ok(())
    }

    fn write_levels(&self, id: &str, new: &Levels) -> Result<(), StoreError> {
        let changes: Vec<SettingChange> = Group::ALL
            .into_iter()
            .map(|g| SettingChange {
                key: g
                    .id()
                    .parse::<FieldKey>()
                    .expect("group ids are valid keys"),
                value: SettingValue::Choice(
                    new.get(g)
                        .id()
                        .parse()
                        .expect("level ids are valid choices"),
                ),
            })
            .collect();
        self.settings
            .apply_trusted(&owner_of(id), changes)
            .map_err(|e| StoreError::Settings(e.to_string()))?;
        self.changed();
        Ok(())
    }

    // ------------------------------------------------------------------------------------------------ widening: needs the remote

    pub fn enable_listener(&self, _approval: &GateApproval) -> Result<(), StoreError> {
        let now = (self.clock)();
        let mut inner = self.inner.lock();
        if inner.listener_enabled {
            return Ok(());
        }
        let mut next = inner.clone();
        next.listener_enabled = true;
        next.enabled_at = Some(now);
        self.commit(&mut inner, next, false)?;
        drop(inner);
        self.changed();
        Ok(())
    }

    /// Opens a pairing window and returns its code. Opening one needs the remote: the code is only ever shown after a physical
    /// press, and a window that is already open is replaced (its code stops working).
    pub fn open_pairing(&self, _approval: &GateApproval) -> PairingView {
        let now = (self.clock)();
        let bytes: [u8; CODE_LEN] = random();
        let code: String = bytes
            .iter()
            .map(|b| ALPHABET[usize::from(b & 0x1f)] as char)
            .collect();
        let view = PairingView {
            code: shown_code(&code),
            expires_at: now + PAIRING_TTL_MS,
            attempts_left: PAIRING_ATTEMPTS,
        };
        self.inner.lock().pairing = Some(Pairing {
            code,
            expires_at: view.expires_at,
            attempts_left: PAIRING_ATTEMPTS,
        });
        self.changed();
        view
    }

    pub fn resume(&self, id: &str, _approval: &GateApproval) -> bool {
        let mut inner = self.inner.lock();
        let Some(a) = inner.find(id) else {
            return false;
        };
        if !a.paused {
            return false;
        }
        let mut next = inner.clone();
        if let Some(a) = next.assistants.iter_mut().find(|a| a.id == id) {
            a.paused = false;
        }
        if self.commit(&mut inner, next, false).is_err() {
            return false;
        }
        drop(inner);
        self.changed();
        true
    }

    pub fn widen_levels(
        &self,
        id: &str,
        new: &Levels,
        _approval: &GateApproval,
    ) -> Result<(), StoreError> {
        self.levels_of(id).ok_or(StoreError::NoSuchAssistant)?;
        self.write_levels(id, new)
    }

    pub fn raise_max_volume(&self, level: f64, _approval: &GateApproval) -> Result<(), StoreError> {
        let mut inner = self.inner.lock();
        let mut next = inner.clone();
        next.max_volume = level;
        self.commit(&mut inner, next, false)?;
        drop(inner);
        self.changed();
        Ok(())
    }

    // ------------------------------------------------------------------------------------------------ pairing

    /// Completes a pairing: the code must be the one of the open window. Any wrong answer says the same thing. Five wrong codes
    /// close the window; so do success (a code works once) and the clock. Twenty attempts of any kind in a minute, from anywhere,
    /// stop all pairing for the rest of the minute.
    pub fn complete_pairing(
        &self,
        code: &str,
        name: &str,
        _peer: IpAddr,
    ) -> Result<Paired, PairFailure> {
        let now = (self.clock)();
        let mut inner = self.inner.lock();
        while inner
            .attempts
            .front()
            .is_some_and(|t| now.saturating_sub(*t) >= 60_000)
        {
            inner.attempts.pop_front();
        }
        if inner.attempts.len() >= PAIRING_RATE_PER_MINUTE {
            return Err(PairFailure::TooManyAttempts);
        }
        inner.attempts.push_back(now);
        let Some(window) = inner.pairing.clone() else {
            return Err(PairFailure::Refused);
        };
        if now >= window.expires_at {
            inner.pairing = None;
            drop(inner);
            self.changed();
            return Err(PairFailure::Refused);
        }
        let presented = normalise_code(code);
        if !ct_eq(presented.as_bytes(), window.code.as_bytes()) {
            let left = window.attempts_left.saturating_sub(1);
            inner.pairing = (left > 0).then_some(Pairing {
                attempts_left: left,
                ..window
            });
            drop(inner);
            self.changed();
            return Err(PairFailure::Refused);
        }
        let Some(name) = clean_name(name) else {
            return Err(PairFailure::BadName);
        };
        let id = hex(&random::<6>());
        let token = format!("{TOKEN_PREFIX}{}", hex(&random::<32>()));
        if id == "000000000000" || token.ends_with(&"0".repeat(64)) {
            // The random source failed (see `random`): no secret is made from nothing.
            return Err(PairFailure::Refused);
        }
        let mut next = inner.clone();
        next.pairing = None;
        next.assistants.push(Assistant {
            id: id.clone(),
            name: name.clone(),
            token_hash: token_hash(&token),
            created_at: now,
            last_seen_at: now,
            paused: false,
            calls: 0,
        });
        if self.commit(&mut inner, next, false).is_err() {
            return Err(PairFailure::Refused);
        }
        drop(inner);
        self.register_page(&id, &name);
        crate::log_info!("ai: paired an assistant ({id})");
        self.changed();
        Ok(Paired {
            assistant_id: id,
            token,
        })
    }
}

/// Writes `bytes` to `path` through a temporary file and a rename, readable by the owner only.
fn write_private(path: &PathBuf, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("json.tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    let result = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    match result.and_then(|()| std::fs::rename(&tmp, path)) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::gate::approval_for_tests as approval;
    use super::*;
    use crate::hub::Hub;
    use std::sync::atomic::{AtomicU64, Ordering};

    const PEER: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 9));

    struct Rig {
        dir: tempfile::TempDir,
        settings: Arc<SettingsStore>,
        now: Arc<AtomicU64>,
        store: Arc<Store>,
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let now = Arc::new(AtomicU64::new(1_800_000_000_000));
        let store = open(&dir, &now);
        Rig {
            settings: store.settings.clone(),
            dir,
            now,
            store,
        }
    }

    fn open(dir: &tempfile::TempDir, now: &Arc<AtomicU64>) -> Arc<Store> {
        let hub = Arc::new(Hub::new(1));
        let settings = Arc::new(SettingsStore::new(dir.path().join("settings"), hub));
        settings.register_builtin().unwrap();
        settings.protect_prefix("ai.");
        let clock = now.clone();
        Store::load(
            dir.path().join("ai"),
            settings,
            Arc::new(move || clock.load(Ordering::SeqCst)),
        )
    }

    fn pair(rig: &Rig, name: &str) -> Paired {
        let view = rig.store.open_pairing(&approval());
        rig.store.complete_pairing(&view.code, name, PEER).unwrap()
    }

    #[test]
    fn a_fresh_store_is_off_unpaired_and_limited() {
        let r = rig();
        assert!(!r.store.listener_enabled());
        assert_eq!(r.store.listener_since(), None);
        assert!(r.store.assistants().is_empty());
        assert_eq!(r.store.pairing_view(), None);
        assert_eq!(r.store.max_volume(), DEFAULT_MAX_VOLUME);
        assert!(
            !r.dir.path().join("ai/state.json").exists(),
            "nothing is written until something changes"
        );
    }

    #[test]
    fn the_listener_switch_survives_a_restart_and_off_applies_first() {
        let r = rig();
        r.store.enable_listener(&approval()).unwrap();
        assert!(r.store.listener_enabled());
        assert_eq!(r.store.listener_since(), Some(1_800_000_000_000));
        let again = open(&r.dir, &r.now);
        assert!(again.listener_enabled());
        assert!(again.disable_listener());
        assert!(!again.disable_listener());
        assert!(!open(&r.dir, &r.now).listener_enabled());
    }

    #[test]
    fn a_state_file_that_cannot_be_read_means_off_and_unpaired() {
        let r = rig();
        r.store.enable_listener(&approval()).unwrap();
        let paired = pair(&r, "Cody");
        for damage in [
            "{not json",
            "",
            "[]",
            r#"{"version":2,"listener":{"enabled":true,"enabledAt":1},"limits":{"maxVolume":50.0},"assistants":[]}"#,
        ] {
            std::fs::write(r.dir.path().join("ai/state.json"), damage).unwrap();
            let after = open(&r.dir, &r.now);
            assert!(!after.listener_enabled(), "{damage}");
            assert!(after.assistants().is_empty(), "{damage}");
            assert_eq!(after.authenticate(&paired.token), None, "{damage}");
        }
    }

    #[test]
    fn damaged_assistants_are_dropped_and_good_ones_kept() {
        let r = rig();
        let paired = pair(&r, "Cody");
        let mut doc: Value = serde_json::from_str(
            &std::fs::read_to_string(r.dir.path().join("ai/state.json")).unwrap(),
        )
        .unwrap();
        let good = doc["assistants"][0].clone();
        let mut bad_hash = good.clone();
        bad_hash["tokenHash"] = json!("zz");
        bad_hash["id"] = json!("aaaaaaaaaaaa");
        let mut bad_id = good.clone();
        bad_id["id"] = json!("../../etc/x");
        let mut bad_name = good.clone();
        bad_name["name"] = json!("\u{7}\u{7}");
        bad_name["id"] = json!("bbbbbbbbbbbb");
        let duplicate = good.clone();
        doc["assistants"] = json!([good, bad_hash, bad_id, bad_name, duplicate]);
        std::fs::write(r.dir.path().join("ai/state.json"), doc.to_string()).unwrap();
        let after = open(&r.dir, &r.now);
        assert_eq!(after.assistants().len(), 1);
        assert!(after.authenticate(&paired.token).is_some());
    }

    #[test]
    fn the_state_file_is_private_and_holds_no_token() {
        use std::os::unix::fs::PermissionsExt;
        let r = rig();
        let paired = pair(&r, "Cody");
        let path = r.dir.path().join("ai/state.json");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains(&paired.token),
            "the token is in the state file"
        );
        assert!(!text.contains(&paired.token[TOKEN_PREFIX.len()..]));
        assert!(
            text.contains(&hex(&token_hash(&paired.token))),
            "its hash is"
        );
        assert!(!r.dir.path().join("ai/state.json.tmp").exists());
        for entry in walk(r.dir.path()) {
            let content = std::fs::read(&entry).unwrap_or_default();
            assert!(
                !String::from_utf8_lossy(&content).contains(&paired.token),
                "{} holds the token",
                entry.display()
            );
        }
    }

    fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else {
                out.push(path);
            }
        }
        out
    }

    #[test]
    fn a_pairing_code_has_the_documented_shape() {
        let r = rig();
        for _ in 0..50 {
            let view = r.store.open_pairing(&approval());
            let (a, b) = view.code.split_once('-').unwrap();
            assert_eq!((a.len(), b.len()), (4, 4));
            assert!(
                view.code
                    .replace('-', "")
                    .bytes()
                    .all(|c| ALPHABET.contains(&c)),
                "{}",
                view.code
            );
            assert_eq!(view.attempts_left, PAIRING_ATTEMPTS);
            assert_eq!(view.expires_at, 1_800_000_000_000 + PAIRING_TTL_MS);
        }
        let codes: std::collections::HashSet<String> = (0..50)
            .map(|_| r.store.open_pairing(&approval()).code)
            .collect();
        assert!(codes.len() > 45, "codes repeat: {}", codes.len());
    }

    #[test]
    fn pairing_makes_an_assistant_with_the_default_levels_and_a_working_token() {
        let r = rig();
        let paired = pair(&r, "  Cody   the\tagent ");
        assert!(paired.token.starts_with(TOKEN_PREFIX) && paired.token.len() == 68);
        assert_eq!(paired.assistant_id.len(), 12);
        let view = r.store.authenticate(&paired.token).unwrap();
        assert_eq!(
            (view.id.as_str(), view.name.as_str()),
            (paired.assistant_id.as_str(), "Cody the agent")
        );
        let info = r.store.assistant(&paired.assistant_id).unwrap();
        assert_eq!(info.levels, Levels::defaults());
        assert!(!info.paused);
        assert_eq!(info.created_at, 1_800_000_000_000);
        // The window is spent: a code works once.
        assert_eq!(r.store.pairing_view(), None);
        // The permissions page exists in the settings layer, protected.
        let page = r
            .settings
            .describe(&owner_of(&paired.assistant_id))
            .unwrap();
        assert_eq!(page["effective"]["launch"]["value"], "ask");
        // It survives a restart.
        let again = open(&r.dir, &r.now);
        assert!(again.authenticate(&paired.token).is_some());
        assert_eq!(
            again.assistant(&paired.assistant_id).unwrap().levels,
            Levels::defaults()
        );
    }

    #[test]
    fn a_code_is_forgiving_about_case_dashes_and_spaces_and_nothing_else() {
        let r = rig();
        let view = r.store.open_pairing(&approval());
        let loose = view.code.replace('-', " ").to_lowercase();
        assert!(r.store.complete_pairing(&loose, "Cody", PEER).is_ok());
        let view = r.store.open_pairing(&approval());
        let wrong = view.code.replace('-', "") + "A";
        assert_eq!(
            r.store.complete_pairing(&wrong, "Cody", PEER),
            Err(PairFailure::Refused)
        );
        assert_eq!(
            r.store.complete_pairing(&view.code[..8], "Cody", PEER),
            Err(PairFailure::Refused)
        );
    }

    #[test]
    fn no_window_no_pairing() {
        let r = rig();
        assert_eq!(
            r.store.complete_pairing("ABCD-EFGH", "Cody", PEER),
            Err(PairFailure::Refused)
        );
        assert_eq!(
            r.store.complete_pairing("", "Cody", PEER),
            Err(PairFailure::Refused)
        );
        assert!(r.store.assistants().is_empty());
    }

    #[test]
    fn five_wrong_codes_close_the_window_and_the_right_one_no_longer_works() {
        let r = rig();
        let view = r.store.open_pairing(&approval());
        for left in (1..PAIRING_ATTEMPTS).rev() {
            assert_eq!(
                r.store.complete_pairing("AAAA-AAAA", "x", PEER),
                Err(PairFailure::Refused)
            );
            assert_eq!(r.store.pairing_view().unwrap().attempts_left, left);
        }
        assert_eq!(
            r.store.complete_pairing("AAAA-AAAA", "x", PEER),
            Err(PairFailure::Refused)
        );
        assert_eq!(r.store.pairing_view(), None, "the window closed");
        assert_eq!(
            r.store.complete_pairing(&view.code, "Cody", PEER),
            Err(PairFailure::Refused)
        );
        assert!(r.store.assistants().is_empty());
    }

    #[test]
    fn a_window_expires_after_five_minutes() {
        let r = rig();
        let view = r.store.open_pairing(&approval());
        r.now.fetch_add(PAIRING_TTL_MS - 1, Ordering::SeqCst);
        assert!(r.store.pairing_view().is_some());
        r.now.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            r.store.complete_pairing(&view.code, "Cody", PEER),
            Err(PairFailure::Refused)
        );
        assert_eq!(r.store.pairing_view(), None);
    }

    #[test]
    fn a_new_window_replaces_the_old_and_a_closed_one_is_closed() {
        let r = rig();
        let first = r.store.open_pairing(&approval());
        let second = r.store.open_pairing(&approval());
        assert_ne!(first.code, second.code);
        assert_eq!(
            r.store.complete_pairing(&first.code, "Old", PEER),
            Err(PairFailure::Refused)
        );
        assert!(r.store.close_pairing());
        assert!(!r.store.close_pairing());
        assert_eq!(
            r.store.complete_pairing(&second.code, "New", PEER),
            Err(PairFailure::Refused)
        );
    }

    #[test]
    fn pairing_attempts_are_rate_limited_for_everyone_together() {
        let r = rig();
        r.store.open_pairing(&approval());
        let mut limited = 0;
        for _ in 0..40 {
            if r.store.complete_pairing("AAAA-AAAA", "x", PEER) == Err(PairFailure::TooManyAttempts)
            {
                limited += 1;
            }
            // Keep the window open: it would close after five wrong codes.
            if r.store.pairing_view().is_none() {
                r.store.open_pairing(&approval());
            }
        }
        assert!(limited > 0);
        // Even the right code is refused while the limit holds, and works again once the minute is over.
        let view = r.store.open_pairing(&approval());
        assert_eq!(
            r.store.complete_pairing(&view.code, "Cody", PEER),
            Err(PairFailure::TooManyAttempts)
        );
        r.now.fetch_add(60_000, Ordering::SeqCst);
        let _ = view;
        let fresh = r.store.open_pairing(&approval());
        assert!(r.store.complete_pairing(&fresh.code, "Cody", PEER).is_ok());
    }

    #[test]
    fn a_bad_name_after_the_right_code_does_not_burn_the_window() {
        let r = rig();
        let view = r.store.open_pairing(&approval());
        assert_eq!(
            r.store.complete_pairing(&view.code, " \u{7}\n ", PEER),
            Err(PairFailure::BadName)
        );
        assert!(r.store.pairing_view().is_some());
        assert!(r.store.complete_pairing(&view.code, "Cody", PEER).is_ok());
        // And a bad name with a wrong code is just refused: the name check says nothing about the code.
        let view = r.store.open_pairing(&approval());
        assert_eq!(
            r.store.complete_pairing("AAAA-AAAA", "", PEER),
            Err(PairFailure::Refused)
        );
        let _ = view;
    }

    #[test]
    fn names_are_cleaned_and_cut() {
        assert_eq!(clean_name("Cody").as_deref(), Some("Cody"));
        assert_eq!(
            clean_name(&"n".repeat(100)).unwrap().chars().count(),
            NAME_MAX
        );
        assert_eq!(clean_name("a\u{1b}[2Jb").as_deref(), Some("a[2Jb"));
        assert_eq!(clean_name("   "), None);
        assert_eq!(clean_name(""), None);
    }

    #[test]
    fn authentication_takes_only_well_formed_tokens_that_were_issued() {
        let r = rig();
        let a = pair(&r, "Cody");
        let b = pair(&r, "Home Assistant");
        assert_eq!(r.store.authenticate(&a.token).unwrap().name, "Cody");
        assert_eq!(
            r.store.authenticate(&b.token).unwrap().name,
            "Home Assistant"
        );
        let flipped = format!(
            "{}{}",
            &a.token[..67],
            if a.token.ends_with('0') { '1' } else { '0' }
        );
        for bad in [
            "",
            "ltv_",
            &a.token[..67],
            &format!("{}0", a.token),
            &flipped,
            &a.token.to_uppercase(),
            &a.token[TOKEN_PREFIX.len()..],
            "Bearer x",
            &"a".repeat(68),
        ] {
            assert_eq!(r.store.authenticate(bad), None, "{bad}");
        }
        // The hash is not a token: knowing the state file does not help.
        let hash = hex(&token_hash(&a.token));
        assert_eq!(r.store.authenticate(&format!("{TOKEN_PREFIX}{hash}")), None);
    }

    #[test]
    fn revoking_ends_one_assistant_and_removes_its_page() {
        let r = rig();
        let a = pair(&r, "Cody");
        let b = pair(&r, "Other");
        assert!(r.store.revoke(&a.assistant_id));
        assert!(!r.store.revoke(&a.assistant_id));
        assert_eq!(r.store.authenticate(&a.token), None);
        assert!(r.store.authenticate(&b.token).is_some());
        assert!(r.settings.describe(&owner_of(&a.assistant_id)).is_none());
        assert!(
            !r.dir
                .path()
                .join(format!("settings/ai.{}.json", a.assistant_id))
                .exists()
        );
        assert_eq!(open(&r.dir, &r.now).authenticate(&a.token), None);
    }

    #[test]
    fn a_lock_out_removes_everything_at_once() {
        let r = rig();
        let a = pair(&r, "Cody");
        let b = pair(&r, "Other");
        r.store.enable_listener(&approval()).unwrap();
        r.store.open_pairing(&approval());
        let result = r.store.lock_out();
        assert_eq!(
            result,
            LockOut {
                revoked: 2,
                listener_was_on: true,
                pairing_was_open: true
            }
        );
        assert!(!r.store.listener_enabled());
        assert!(r.store.assistants().is_empty());
        assert_eq!(r.store.pairing_view(), None);
        assert_eq!(r.store.authenticate(&a.token), None);
        assert_eq!(r.store.authenticate(&b.token), None);
        assert!(r.settings.describe(&owner_of(&a.assistant_id)).is_none());
        // It is persisted: a restart does not bring anything back.
        let again = open(&r.dir, &r.now);
        assert!(!again.listener_enabled() && again.assistants().is_empty());
        assert_eq!(
            r.store.lock_out(),
            LockOut {
                revoked: 0,
                listener_was_on: false,
                pairing_was_open: false
            }
        );
    }

    #[test]
    fn a_lock_out_applies_even_when_the_state_cannot_be_saved() {
        let r = rig();
        let a = pair(&r, "Cody");
        r.store.enable_listener(&approval()).unwrap();
        // The state directory goes read-only: saving fails.
        std::fs::remove_file(r.dir.path().join("ai/state.json")).unwrap();
        std::fs::create_dir(r.dir.path().join("ai/state.json")).unwrap();
        let result = r.store.lock_out();
        assert_eq!(result.revoked, 1);
        assert!(
            !r.store.listener_enabled(),
            "the in-memory state is what counts now"
        );
        assert_eq!(r.store.authenticate(&a.token), None);
    }

    #[test]
    fn widening_that_cannot_be_saved_does_not_happen() {
        let r = rig();
        std::fs::create_dir_all(r.dir.path().join("ai/state.json")).unwrap();
        assert!(matches!(
            r.store.enable_listener(&approval()),
            Err(StoreError::Io(_))
        ));
        assert!(!r.store.listener_enabled());
        assert!(matches!(
            r.store.raise_max_volume(70.0, &approval()),
            Err(StoreError::Io(_))
        ));
        assert_eq!(r.store.max_volume(), DEFAULT_MAX_VOLUME);
    }

    #[test]
    fn lowering_levels_needs_no_remote_and_widening_cannot_be_done_without_one() {
        let r = rig();
        let a = pair(&r, "Cody");
        let id = &a.assistant_id;
        let mut lower = Levels::defaults();
        lower.set(Group::State, Level::Ask);
        lower.set(Group::Playback, Level::Never);
        r.store.lower_levels(id, &lower).unwrap();
        assert_eq!(r.store.levels_of(id), Some(lower));
        // Anything wider is refused here, whatever else the change lowers.
        let mut wider = lower;
        wider.set(Group::Launch, Level::Always);
        wider.set(Group::Power, Level::Never);
        assert_eq!(r.store.lower_levels(id, &wider), Err(StoreError::Widens));
        assert_eq!(r.store.levels_of(id), Some(lower), "nothing changed");
        assert_eq!(
            r.store.lower_levels("nobody0000000", &lower),
            Err(StoreError::NoSuchAssistant)
        );
        // The same change with the proof goes through, and persists in the settings layer.
        r.store.widen_levels(id, &wider, &approval()).unwrap();
        assert_eq!(open(&r.dir, &r.now).levels_of(id), Some(wider));
        assert_eq!(
            r.settings.describe(&owner_of(id)).unwrap()["effective"]["launch"]["value"],
            "always"
        );
    }

    #[test]
    fn the_generic_settings_path_cannot_touch_an_assistants_permissions() {
        let r = rig();
        let a = pair(&r, "Cody");
        let change = SettingChange {
            key: "launch".parse().unwrap(),
            value: SettingValue::Choice("always".parse().unwrap()),
        };
        assert!(matches!(
            r.settings
                .apply(&owner_of(&a.assistant_id), vec![change], None),
            Err(crate::settings::ApplyError::Protected)
        ));
        assert_eq!(
            r.store.levels_of(&a.assistant_id).unwrap().launch,
            Level::Ask
        );
    }

    #[test]
    fn pausing_is_free_and_resuming_needs_the_remote() {
        let r = rig();
        let a = pair(&r, "Cody");
        assert!(r.store.pause(&a.assistant_id));
        assert!(!r.store.pause(&a.assistant_id));
        assert_eq!(r.store.is_paused(&a.assistant_id), Some(true));
        assert_eq!(open(&r.dir, &r.now).is_paused(&a.assistant_id), Some(true));
        assert!(r.store.resume(&a.assistant_id, &approval()));
        assert!(!r.store.resume(&a.assistant_id, &approval()));
        assert_eq!(r.store.is_paused(&a.assistant_id), Some(false));
        assert!(!r.store.pause("nobody0000000"));
        assert_eq!(r.store.is_paused("nobody0000000"), None);
    }

    #[test]
    fn the_volume_limit_goes_down_freely_and_up_only_with_the_remote() {
        let r = rig();
        r.store.lower_max_volume(40.0).unwrap();
        assert_eq!(r.store.max_volume(), 40.0);
        r.store.lower_max_volume(40.0).unwrap();
        assert_eq!(r.store.lower_max_volume(40.5), Err(StoreError::Widens));
        assert_eq!(r.store.max_volume(), 40.0);
        r.store.raise_max_volume(60.0, &approval()).unwrap();
        assert_eq!(open(&r.dir, &r.now).max_volume(), 60.0);
    }

    #[test]
    fn calls_are_counted_in_memory() {
        let r = rig();
        let a = pair(&r, "Cody");
        r.now.fetch_add(5_000, Ordering::SeqCst);
        r.store.record_call(&a.assistant_id);
        r.store.record_call(&a.assistant_id);
        let info = r.store.assistant(&a.assistant_id).unwrap();
        assert_eq!(info.calls, 2);
        assert_eq!(info.last_seen_at, 1_800_000_005_000);
        r.store.record_call("nobody0000000");
    }

    #[test]
    fn every_change_calls_back_so_the_topics_follow() {
        let r = rig();
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c = count.clone();
        r.store.on_change(move || {
            c.fetch_add(1, Ordering::SeqCst);
        });
        let a = pair(&r, "Cody");
        let after_pairing = count.load(Ordering::SeqCst);
        assert!(
            after_pairing >= 2,
            "opening and completing a pairing both announce"
        );
        r.store.pause(&a.assistant_id);
        r.store.revoke(&a.assistant_id);
        r.store.lock_out();
        assert!(count.load(Ordering::SeqCst) >= after_pairing + 3);
    }

    #[test]
    fn constant_time_equality_is_equality() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"abcd"));
        assert!(ct_eq(b"", b""));
    }

    use super::super::types::Level;
}
