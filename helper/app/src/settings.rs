//! The settings data layer, on LumaShow's `settings-schema`.
//!
//! An *owner* (a base component now, a plugin in phase 9) describes its settings page as data: sections of fields with
//! kinds, defaults, bounds and predicates. This store keeps every owner's schema and stored values, checks every change against
//! the schema (`SettingsSchema::validate_changes`: type, range, step, listed choices, no duplicates, nothing a predicate
//! disables) plus the owner's own rule, writes the values atomically, and says that something changed (a hub event and the
//! listeners components register, so the receiver client reconnects and the weather card refetches at once).
//!
//! The same JSON document that describes a page here is the one the TV's Settings will draw and the one LumaShow's plugins
//! publish, so a plugin owner needs nothing but `register`.
//!
//! Nothing is clamped or repaired: a stored value that its field no longer accepts is reported (`invalid`), left in the file
//! as the person wrote it, and replaced in `effective` by the schema default.
//!
//! Values live in `<dir>/<owner>.json`: `{"owner","schemaRevision","version","values":{key: {"type","value"}}}`. `version`
//! counts accepted transactions, so a client can say "apply this only to the version I read".

use crate::control::Stop;
use crate::hub::Hub;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use settings_schema::{
    CapabilityKey, FieldError, FieldErrors, FieldKey, FieldKind, SettingChange, SettingsSchema,
    Value, Values,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

/// The owner's own check, run after the schema's, against the values the change would leave (stored over defaults).
pub type Rule = fn(&Values) -> Result<(), FieldErrors>;

/// Why a change was not applied.
#[derive(Debug)]
pub enum ApplyError {
    UnknownOwner,
    /// The client asked to apply to a version that is no longer current.
    Conflict {
        current: u64,
    },
    /// The schema or the owner's rule refused one or more fields.
    Invalid(FieldErrors),
    /// The owner is protected: only the module that owns it may change its values (the AI permissions, which only a physical
    /// remote press may widen).
    Protected,
    /// The values could not be written; nothing changed.
    Io(String),
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownOwner => f.write_str("no such settings owner"),
            Self::Conflict { current } => write!(f, "the settings are at version {current} now"),
            Self::Invalid(errors) => write!(f, "{errors}"),
            Self::Protected => f.write_str("these settings can only be changed on the TV"),
            Self::Io(why) => write!(f, "the settings could not be saved ({why})"),
        }
    }
}

/// Why a schema could not be registered.
#[derive(Debug, PartialEq, Eq)]
pub struct RegisterError(pub String);

impl fmt::Display for RegisterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RegisterError {}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredFile {
    owner: String,
    schema_revision: u32,
    version: u64,
    values: Values,
}

struct Owner {
    schema: SettingsSchema,
    rule: Option<Rule>,
    /// What is in the file: valid or not, known keys or not.
    stored: Values,
    version: u64,
}

impl Owner {
    /// Stored values that their fields accept, over the schema's defaults.
    fn effective(&self) -> Values {
        effective_of(&self.schema, &self.stored)
    }
}

fn effective_of(schema: &SettingsSchema, stored: &Values) -> Values {
    let audit = schema.audit_stored(stored);
    let mut values = schema.defaults();
    for (key, value) in stored.iter() {
        let usable = schema.field(key).is_some() && audit.invalid.get(key).is_none();
        if usable {
            values.insert(key.clone(), value.clone());
        }
    }
    values
}

type Listener = Arc<dyn Fn() + Send + Sync>;

/// What the settings store holds, by owner.
pub struct SettingsStore {
    dir: PathBuf,
    hub: Arc<Hub>,
    owners: Mutex<BTreeMap<String, Owner>>,
    listeners: Mutex<Vec<(String, Listener)>>,
    /// Owners whose name starts with one of these cannot be changed through [`SettingsStore::apply`].
    protected: Mutex<Vec<String>>,
}

/// One owner in the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub owner: String,
    pub title: String,
    pub schema_revision: u32,
    pub version: u64,
}

/// A successful transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    pub version: u64,
    /// Whether anything was written: a transaction of no changes, or of values already held, is accepted and changes nothing.
    pub changed: bool,
}

impl SettingsStore {
    /// A store that keeps its files in `dir` and reports changes to `hub`. Nothing is read until an owner registers.
    pub fn new(dir: impl Into<PathBuf>, hub: Arc<Hub>) -> Self {
        Self {
            dir: dir.into(),
            hub,
            owners: Mutex::new(BTreeMap::new()),
            listeners: Mutex::new(Vec::new()),
            protected: Mutex::new(Vec::new()),
        }
    }

    /// Makes every owner whose name starts with `prefix` (now or later) refuse [`SettingsStore::apply`]; the module that owns
    /// them changes them with [`SettingsStore::apply_trusted`]. The generic settings API goes through `apply`, so it can never
    /// change them, whatever it is sent.
    pub fn protect_prefix(&self, prefix: &str) {
        self.protected.lock().push(prefix.to_owned());
    }

    fn is_protected(&self, owner: &str) -> bool {
        self.protected
            .lock()
            .iter()
            .any(|p| owner.starts_with(p.as_str()))
    }

    fn file_of(&self, owner: &str) -> PathBuf {
        self.dir.join(format!("{owner}.json"))
    }

    /// Registers an owner's page and loads its stored values. A second registration of the same owner replaces the schema
    /// when its revision is not lower (a changed page), keeping the stored values.
    pub fn register(
        &self,
        schema: SettingsSchema,
        rule: Option<Rule>,
    ) -> Result<(), RegisterError> {
        schema
            .validate()
            .map_err(|e| RegisterError(format!("{}: {e}", schema.owner)))?;
        let name = schema.owner.as_str().to_owned();
        let mut owners = self.owners.lock();
        if let Some(existing) = owners.get(&name)
            && existing.schema.revision > schema.revision
        {
            return Err(RegisterError(format!(
                "{name}: revision {} is older than the registered {}",
                schema.revision, existing.schema.revision
            )));
        }
        let (stored, version) = match owners.remove(&name) {
            Some(existing) => (existing.stored, existing.version),
            None => self.load(&name),
        };
        let audit = schema.audit_stored(&stored);
        for (key, error) in audit.invalid.iter() {
            crate::log_warn!(
                "settings: {name}: stored {key} is not accepted ({error}); the default is used"
            );
        }
        owners.insert(
            name,
            Owner {
                schema,
                rule,
                stored,
                version,
            },
        );
        Ok(())
    }

    /// Removes an owner and its stored values (an assistant that was revoked). Nothing is left of it on disk.
    pub fn unregister(&self, owner: &str) -> bool {
        let removed = self.owners.lock().remove(owner).is_some();
        let _ = std::fs::remove_file(self.file_of(owner));
        self.listeners.lock().retain(|(o, _)| o != owner);
        if removed {
            self.hub
                .emit("settings", "removed", json!({"owner": owner}));
        }
        removed
    }

    /// Takes an owner's page off the list but keeps its stored values on disk and in the store's files: a plugin that disconnected
    /// has no page until it comes back, and its settings are what the person chose when it does.
    pub fn retire(&self, owner: &str) -> bool {
        let removed = self.owners.lock().remove(owner).is_some();
        self.listeners.lock().retain(|(o, _)| o != owner);
        if removed {
            self.hub
                .emit("settings", "retired", json!({"owner": owner}));
        }
        removed
    }

    /// Registers the helper's own pages: `base.weather`, `base.news`, `base.logging`, `base.ambient` and `base.voice`.
    pub fn register_builtin(&self) -> Result<(), RegisterError> {
        for (json, rule) in [
            (
                include_str!("schemas/base.weather.json"),
                Some(weather_rule as Rule),
            ),
            (include_str!("schemas/base.news.json"), None),
            (include_str!("schemas/base.logging.json"), Some(logging_rule as Rule)),
            (include_str!("schemas/base.ambient.json"), None),
            (
                include_str!("schemas/base.voice.json"),
                Some(voice_rule as Rule),
            ),
        ] {
            let schema = SettingsSchema::parse(json.as_bytes())
                .map_err(|e| RegisterError(format!("built-in schema: {e}")))?;
            self.register(schema, rule)?;
        }
        Ok(())
    }

    fn load(&self, owner: &str) -> (Values, u64) {
        let path = self.file_of(owner);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return (Values::new(), 0);
        };
        match serde_json::from_str::<StoredFile>(&text) {
            Ok(file) if file.owner == owner => (file.values, file.version),
            Ok(_) => {
                crate::log_warn!(
                    "settings: {} belongs to another owner; ignored",
                    path.display()
                );
                (Values::new(), 0)
            }
            Err(e) => {
                crate::log_warn!(
                    "settings: {} is not readable ({e}); the defaults are used",
                    path.display()
                );
                (Values::new(), 0)
            }
        }
    }

    /// What `owner`'s file holds, registered or not: `None` when it has none.
    pub fn stored(&self, owner: &str) -> Option<Values> {
        self.file_of(owner).exists().then(|| self.load(owner).0)
    }

    /// Moves what an owner the helper no longer has (`from`) stored to the owner that took its place (`to`, not registered yet): the keys
    /// in `keep`, and only when `to` has no file of its own. `from`'s file is removed once that is done.
    pub fn carry_over(&self, from: &str, to: &str, keep: &[&str]) -> std::io::Result<()> {
        let target = self.file_of(to);
        if !target.exists() {
            let mut kept = Values::new();
            for (key, value) in self.load(from).0.iter() {
                if keep.contains(&key.as_str()) {
                    kept.insert(key.clone(), value.clone());
                }
            }
            if kept.iter().next().is_some() {
                let file = StoredFile {
                    owner: to.to_owned(),
                    schema_revision: 0,
                    version: 1,
                    values: kept,
                };
                let bytes = serde_json::to_vec_pretty(&file).map_err(std::io::Error::other)?;
                lumatv_feeds::cache::atomic_write(&target, &bytes)?;
            }
        }
        match std::fs::remove_file(self.file_of(from)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Takes `keys` out of what `owner` keeps, once another part of the helper has taken their place (Guide data's Plex server became the
    /// Plex account's). Its listeners are not told: nothing in force changes.
    pub(crate) fn take_keys(&self, owner: &str, keys: &[&str]) -> std::io::Result<()> {
        let path = self.file_of(owner);
        let Ok(text) = std::fs::read_to_string(&path) else { return Ok(()) };
        let mut file: StoredFile = serde_json::from_str(&text).map_err(std::io::Error::other)?;
        let before = file.values.len();
        for key in keys {
            file.values.remove(*key);
        }
        if file.owner != owner || file.values.len() == before {
            return Ok(());
        }
        file.version += 1;
        let bytes = serde_json::to_vec_pretty(&file).map_err(std::io::Error::other)?;
        lumatv_feeds::cache::atomic_write(&path, &bytes)?;
        if let Some(o) = self.owners.lock().get_mut(owner) {
            for key in keys {
                o.stored.remove(*key);
            }
            o.version = file.version;
        }
        Ok(())
    }

    /// The registered owners.
    pub fn owners(&self) -> Vec<Summary> {
        self.owners
            .lock()
            .values()
            .map(|o| Summary {
                owner: o.schema.owner.as_str().to_owned(),
                title: o.schema.title.clone(),
                schema_revision: o.schema.revision,
                version: o.version,
            })
            .collect()
    }

    /// One owner as the API shows it: the schema, the stored values, the effective values and anything stored that no longer fits.
    pub fn describe(&self, owner: &str) -> Option<Json> {
        let owners = self.owners.lock();
        let o = owners.get(owner)?;
        let audit = o.schema.audit_stored(&o.stored);
        let invalid: BTreeMap<String, String> = audit
            .invalid
            .iter()
            .map(|(k, e)| (k.as_str().to_owned(), e.to_string()))
            .collect();
        Some(json!({
            "owner": owner,
            "version": o.version,
            "schemaRevision": o.schema.revision,
            "schema": serde_json::to_value(&o.schema).unwrap_or(Json::Null),
            "values": serde_json::to_value(&o.stored).unwrap_or(Json::Null),
            "effective": serde_json::to_value(o.effective()).unwrap_or(Json::Null),
            "invalid": invalid,
            "unknown": audit.unknown.iter().map(|k| k.as_str().to_owned()).collect::<Vec<_>>(),
        }))
    }

    /// The values that are in force for `owner`.
    pub fn effective(&self, owner: &str) -> Option<Values> {
        self.owners.lock().get(owner).map(Owner::effective)
    }

    /// The most characters `owner`'s secret field `key` takes, when its page has that field and it is a secret. A secret is never
    /// stored here: it goes to its owner alone (`plugins::session::Shared::submit_secret`).
    pub fn secret_field(&self, owner: &str, key: &str) -> Option<u16> {
        let owners = self.owners.lock();
        let field = owners
            .get(owner)?
            .schema
            .fields()
            .find(|f| f.key.as_str() == key)?;
        match field.kind {
            FieldKind::Text {
                max_chars,
                secret: true,
            } => Some(max_chars),
            _ => None,
        }
    }

    /// Applies one transaction: all of `changes` or none. With `expect_version`, only to that version of the owner's values.
    /// A protected owner is refused: see [`SettingsStore::protect_prefix`].
    pub fn apply(
        &self,
        owner: &str,
        changes: Vec<SettingChange>,
        expect_version: Option<u64>,
    ) -> Result<Applied, ApplyError> {
        if self.is_protected(owner) {
            return Err(ApplyError::Protected);
        }
        self.apply_inner(owner, changes, expect_version)
    }

    /// [`SettingsStore::apply`] for the module that owns a protected owner. Only code inside this crate can call it, and the
    /// loopback API does not.
    pub(crate) fn apply_trusted(
        &self,
        owner: &str,
        changes: Vec<SettingChange>,
    ) -> Result<Applied, ApplyError> {
        self.apply_inner(owner, changes, None)
    }

    fn apply_inner(
        &self,
        owner: &str,
        changes: Vec<SettingChange>,
        expect_version: Option<u64>,
    ) -> Result<Applied, ApplyError> {
        let version = {
            let mut owners = self.owners.lock();
            let o = owners.get_mut(owner).ok_or(ApplyError::UnknownOwner)?;
            if let Some(expected) = expect_version
                && expected != o.version
            {
                return Err(ApplyError::Conflict { current: o.version });
            }
            let capabilities: BTreeSet<CapabilityKey> = BTreeSet::new();
            o.schema
                .validate_changes(&changes, &o.stored, &capabilities)
                .map_err(ApplyError::Invalid)?;
            let mut after = o.stored.clone();
            for change in &changes {
                after.insert(change.key.clone(), change.value.clone());
            }
            if let Some(rule) = o.rule {
                rule(&effective_of(&o.schema, &after)).map_err(ApplyError::Invalid)?;
            }
            if after == o.stored {
                return Ok(Applied {
                    version: o.version,
                    changed: false,
                });
            }
            let file = StoredFile {
                owner: owner.to_owned(),
                schema_revision: o.schema.revision,
                version: o.version + 1,
                values: after.clone(),
            };
            let bytes =
                serde_json::to_vec_pretty(&file).map_err(|e| ApplyError::Io(e.to_string()))?;
            lumatv_feeds::cache::atomic_write(&self.file_of(owner), &bytes)
                .map_err(|e| ApplyError::Io(e.to_string()))?;
            o.stored = after;
            o.version += 1;
            o.version
        };
        self.hub.emit(
            "settings",
            "changed",
            json!({"owner": owner, "version": version}),
        );
        let to_call: Vec<Listener> = self
            .listeners
            .lock()
            .iter()
            .filter(|(o, _)| o == owner)
            .map(|(_, l)| l.clone())
            .collect();
        for listener in to_call {
            listener();
        }
        Ok(Applied {
            version,
            changed: true,
        })
    }

    /// Calls `listener` after every change to `owner`'s values.
    pub fn on_change(&self, owner: &str, listener: impl Fn() + Send + Sync + 'static) {
        self.listeners
            .lock()
            .push((owner.to_owned(), Arc::new(listener)));
    }

    /// Takes the temperature unit that older builds kept in theme.json as the weather's, unless one is already stored. `false` when the
    /// unit could not be taken, so the caller keeps its copy.
    pub(crate) fn adopt_weather_units(&self, units: &str) -> bool {
        const OWNER: &str = "base.weather";
        let stored = self.owners.lock().get(OWNER).is_none_or(|o| o.stored.get("units").is_some());
        if stored {
            return true;
        }
        let Ok(choice) = units.parse() else { return false };
        let change = SettingChange { key: field("units"), value: Value::Choice(choice) };
        self.apply_trusted(OWNER, vec![change]).is_ok()
    }

    pub(crate) fn text(&self, owner: &str, key: &str) -> Option<String> {
        match self.effective(owner)?.get(key)? {
            Value::Text(t) => Some(t.clone()),
            Value::Choice(c) => Some(c.to_string()),
            _ => None,
        }
    }

    pub(crate) fn integer(&self, owner: &str, key: &str) -> Option<i64> {
        match self.effective(owner)?.get(key)? {
            Value::Integer(n) => Some(*n),
            _ => None,
        }
    }

    pub(crate) fn flag(&self, owner: &str, key: &str) -> Option<bool> {
        match self.effective(owner)?.get(key)? {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The weather card's settings (the schema's defaults when nothing is stored).
    pub fn weather(&self) -> WeatherSettings {
        WeatherSettings {
            zip: self
                .text("base.weather", "zip")
                .unwrap_or_else(|| "30311".to_owned()),
            units: self
                .text("base.weather", "units")
                .unwrap_or_else(|| "fahrenheit".to_owned()),
            source: self
                .text("base.weather", "source")
                .unwrap_or_else(|| "builtin".to_owned()),
            entity: self.text("base.weather", "entity").unwrap_or_default(),
        }
    }

    /// The news card's settings.
    pub fn news(&self) -> NewsSettings {
        NewsSettings {
            edition: self
                .text("base.news", "edition")
                .unwrap_or_else(|| "bbc".to_owned()),
            refresh_minutes: self
                .integer("base.news", "refresh_minutes")
                .and_then(|n| u64::try_from(n).ok())
                .unwrap_or(15),
        }
    }

    /// Voice search's settings: off, and the Local (Kokoro + Whisper) pipeline, until the owner says otherwise.
    pub fn voice(&self) -> VoiceSettings {
        VoiceSettings {
            enabled: self.flag("base.voice", "enabled").unwrap_or(false),
            pipeline: self
                .text("base.voice", "pipeline")
                .unwrap_or_else(|| DEFAULT_PIPELINE.to_owned()),
        }
    }

    /// The receiver client's settings. Its page (`denon-receiver`) is registered exactly while the Denon receiver plugin is switched on
    /// (`receiver_plugin`), so without it the receiver is off: nothing is read from it or sent to it.
    pub fn receiver(&self) -> ReceiverSettings {
        let Some(values) = self.effective(crate::receiver_plugin::ID) else {
            return ReceiverSettings {
                enabled: false,
                host: String::new(),
                port: 0,
            };
        };
        ReceiverSettings {
            enabled: true,
            host: match values.get("host") {
                Some(Value::Text(t)) => t.clone(),
                _ => "192.168.1.93".to_owned(),
            },
            port: match values.get("port") {
                Some(Value::Integer(n)) => u16::try_from(*n).unwrap_or(8080),
                _ => 8080,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceSettings {
    pub enabled: bool,
    /// An Assist pipeline id; empty is Home Assistant's preferred one.
    pub pipeline: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeatherSettings {
    pub zip: String,
    /// `fahrenheit` or `celsius`.
    pub units: String,
    /// `builtin` or `homeassistant`.
    pub source: String,
    /// The Home Assistant `weather.*` entity, used when `source` is `homeassistant`.
    pub entity: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewsSettings {
    /// `bbc` or `nyt`.
    pub edition: String,
    pub refresh_minutes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiverSettings {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
}

fn field(key: &str) -> FieldKey {
    key.parse().expect("built-in keys are valid")
}

fn text_of<'a>(values: &'a Values, key: &str) -> Option<&'a str> {
    match values.get(&field(key))? {
        Value::Text(t) => Some(t),
        _ => None,
    }
}

/// A Home Assistant entity id of the weather domain: `weather.` and a lowercase object id.
fn weather_entity_ok(entity: &str) -> bool {
    entity.strip_prefix("weather.").is_some_and(|object| {
        !object.is_empty() && object.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    })
}

/// A ZIP is five digits: the geocoding service takes nothing else and the text goes into a URL. The Home Assistant source needs a
/// weather entity, and one that is saved must be well formed whatever the source is.
fn weather_rule(values: &Values) -> Result<(), FieldErrors> {
    let mut errors = FieldErrors::default();
    let entity = text_of(values, "entity").unwrap_or("");
    let homeassistant = matches!(values.get(&field("source")), Some(Value::Choice(c)) if c.to_string() == "homeassistant");
    if (homeassistant || !entity.is_empty()) && !weather_entity_ok(entity) {
        errors.insert(field("entity"), FieldError::refused("choose a Home Assistant weather entity, such as weather.home"));
    }
    if let Some(zip) = text_of(values, "zip")
        && !(zip.len() == 5 && zip.bytes().all(|b| b.is_ascii_digit()))
    {
        errors.insert(
            field("zip"),
            FieldError::refused("a ZIP code is five digits"),
        );
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}


fn logging_rule(values: &Values) -> Result<(), FieldErrors> {
    let host = text_of(values, "host").unwrap_or("");
    let enabled = matches!(values.get("enabled"), Some(Value::Bool(true)));
    if (!host.is_empty() && !crate::logging::valid_host(host)) || (enabled && host.is_empty()) {
        let mut errors = FieldErrors::default();
        errors.insert(field("host"), FieldError::refused("use an IP address or DNS hostname without a scheme or port"));
        return Err(errors);
    }
    Ok(())
}

/// An address is a name or an IPv4 number: letters, digits, dots and dashes, as the text goes into a request line.
pub(crate) fn receiver_rule(values: &Values) -> Result<(), FieldErrors> {
    let mut errors = FieldErrors::default();
    if let Some(host) = text_of(values, "host")
        && (host.is_empty()
            || host.starts_with(['-', '.'])
            || !host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'))
    {
        errors.insert(
            field("host"),
            FieldError::refused(
                "use the receiver's IP address or name (letters, digits, dots and dashes)",
            ),
        );
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// The pipeline voice search uses until the owner says otherwise: Local (Kokoro + Whisper) on Nitin's Home Assistant (2026-10-25).
pub const DEFAULT_PIPELINE: &str = "01m3xamd8xzkdv52syapw0cmf2";

/// An Assist pipeline id is Home Assistant's ULID-like word: letters, digits, dashes and underscores, so it can be neither a path nor
/// a second message.
fn voice_rule(values: &Values) -> Result<(), FieldErrors> {
    let mut errors = FieldErrors::default();
    if text_of(values, "pipeline").is_some_and(|id| {
        !id.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    }) {
        errors.insert(
            field("pipeline"),
            FieldError::refused("an Assist pipeline id is letters, digits, dashes and underscores"),
        );
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Wires the settings store to a component: `wake` runs after every change to `owner`, and never after the stop.
pub fn wake_on_change(
    store: &SettingsStore,
    owner: &str,
    stop: &Stop,
    wake: impl Fn() + Send + Sync + 'static,
) {
    let stop = stop.clone();
    store.on_change(owner, move || {
        if !stop.is_set() {
            wake();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn store(dir: &std::path::Path) -> (Arc<Hub>, SettingsStore) {
        let hub = Arc::new(Hub::new(1));
        let store = SettingsStore::new(dir, hub.clone());
        store.register_builtin().unwrap();
        (hub, store)
    }

    fn text(key: &str, value: &str) -> SettingChange {
        SettingChange {
            key: field(key),
            value: Value::Text(value.to_owned()),
        }
    }

    fn choice(key: &str, value: &str) -> SettingChange {
        SettingChange {
            key: field(key),
            value: Value::Choice(value.parse().unwrap()),
        }
    }

    fn int(key: &str, value: i64) -> SettingChange {
        SettingChange {
            key: field(key),
            value: Value::Integer(value),
        }
    }

    fn flag(key: &str, value: bool) -> SettingChange {
        SettingChange {
            key: field(key),
            value: Value::Bool(value),
        }
    }

    #[test]
    fn every_built_in_schema_parses_and_registers() {
        let dir = tempfile::tempdir().unwrap();
        let (_, store) = store(dir.path());
        let owners: Vec<_> = store.owners().into_iter().map(|s| s.owner).collect();
        assert_eq!(
            owners,
            [
                "base.ambient",
                "base.logging",
                "base.news",
                "base.voice",
                "base.weather"
            ]
        );
    }

    #[test]
    fn defaults_are_in_force_until_something_is_stored() {
        let dir = tempfile::tempdir().unwrap();
        let (_, store) = store(dir.path());
        assert_eq!(
            store.weather(),
            WeatherSettings {
                zip: "30311".into(),
                units: "fahrenheit".into(),
                source: "builtin".into(),
                entity: String::new()
            }
        );
        assert_eq!(
            store.news(),
            NewsSettings {
                edition: "bbc".into(),
                refresh_minutes: 15
            }
        );
        assert!(!dir.path().join("base.weather.json").exists());
    }

    #[test]
    fn a_valid_transaction_is_applied_saved_announced_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let (hub, s) = store(dir.path());
        let woken = Arc::new(AtomicUsize::new(0));
        let w = woken.clone();
        s.on_change("base.weather", move || {
            w.fetch_add(1, Ordering::SeqCst);
        });
        let applied = s
            .apply(
                "base.weather",
                vec![text("zip", "10001"), choice("units", "celsius")],
                None,
            )
            .unwrap();
        assert_eq!(
            applied,
            Applied {
                version: 1,
                changed: true
            }
        );
        assert_eq!(s.weather().zip, "10001");
        assert_eq!(s.weather().units, "celsius");
        assert_eq!(woken.load(Ordering::SeqCst), 1);
        let events = hub.events_after(Some(0), None, None).events;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].topic, "settings");
        assert_eq!(
            events[0].data,
            json!({"owner": "base.weather", "version": 1})
        );

        let (_, again) = store(dir.path());
        assert_eq!(again.weather().zip, "10001");
        assert_eq!(
            again
                .owners()
                .iter()
                .find(|o| o.owner == "base.weather")
                .unwrap()
                .version,
            1
        );
    }

    #[test]
    fn the_schema_refuses_what_its_field_cannot_hold() {
        let dir = tempfile::tempdir().unwrap();
        let (_, s) = store(dir.path());
        for bad in [choice("units", "kelvin"), int("zip", 5), text("nope", "x")] {
            let Err(ApplyError::Invalid(errors)) = s.apply("base.weather", vec![bad], None) else {
                panic!("accepted");
            };
            assert_eq!(errors.len(), 1);
        }
        // Off the step, and out of range.
        assert!(matches!(
            s.apply("base.news", vec![int("refresh_minutes", 7)], None),
            Err(ApplyError::Invalid(_))
        ));
        assert!(matches!(
            s.apply("base.news", vec![int("refresh_minutes", 500)], None),
            Err(ApplyError::Invalid(_))
        ));
        // Twice in one transaction.
        assert!(matches!(
            s.apply(
                "base.weather",
                vec![text("zip", "10001"), text("zip", "10002")],
                None
            ),
            Err(ApplyError::Invalid(_))
        ));
        assert_eq!(s.weather().zip, "30311");
    }

    #[test]
    fn a_transaction_is_all_or_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (hub, s) = store(dir.path());
        let result = s.apply(
            "base.weather",
            vec![text("zip", "10001"), choice("units", "kelvin")],
            None,
        );
        assert!(matches!(result, Err(ApplyError::Invalid(_))));
        assert_eq!(s.weather().zip, "30311");
        assert!(hub.events_after(Some(0), None, None).events.is_empty());
        assert!(!dir.path().join("base.weather.json").exists());
    }

    #[test]
    fn the_home_assistant_source_needs_a_well_formed_weather_entity() {
        let dir = tempfile::tempdir().unwrap();
        let (_, s) = store(dir.path());
        let refused = |changes: Vec<SettingChange>| match s.apply("base.weather", changes, None) {
            Err(ApplyError::Invalid(errors)) => errors.iter().next().map(|(k, _)| k.as_str().to_owned()),
            other => panic!("accepted: {:?}", other.is_ok()),
        };
        assert_eq!(refused(vec![choice("source", "homeassistant")]).as_deref(), Some("entity"), "no entity chosen yet");
        for bad in ["sensor.temperature", "weather.", "weather.Home", "weather.a b", "home"] {
            assert_eq!(refused(vec![text("entity", bad)]).as_deref(), Some("entity"), "{bad}");
        }
        assert_eq!(s.weather().source, "builtin", "nothing was stored");
        // the entity first, then the source: each step is valid on its own
        s.apply("base.weather", vec![text("entity", "weather.pirate_weather")], None).unwrap();
        s.apply("base.weather", vec![choice("source", "homeassistant")], None).unwrap();
        let w = s.weather();
        assert_eq!((w.source.as_str(), w.entity.as_str(), w.zip.as_str()), ("homeassistant", "weather.pirate_weather", "30311"));
        // a source that is built in does not care about the entity left behind
        s.apply("base.weather", vec![choice("source", "builtin")], None).unwrap();
        assert_eq!(s.weather().entity, "weather.pirate_weather");
        assert!(matches!(s.apply("base.weather", vec![choice("source", "tomorrow")], None), Err(ApplyError::Invalid(_))));
    }

    #[test]
    fn the_owners_own_rule_runs_after_the_schema() {
        let dir = tempfile::tempdir().unwrap();
        let (_, s) = store(dir.path());
        let Err(ApplyError::Invalid(errors)) =
            s.apply("base.weather", vec![text("zip", "3031a")], None)
        else {
            panic!("accepted");
        };
        let (key, error) = errors.iter().next().unwrap();
        assert_eq!(key.as_str(), "zip");
        assert!(error.to_string().contains("five digits"), "{error}");
        assert!(matches!(
            s.apply("base.weather", vec![text("zip", "1234")], None),
            Err(ApplyError::Invalid(_))
        ));
    }

    #[test]
    fn a_field_that_another_setting_disables_cannot_change() {
        let dir = tempfile::tempdir().unwrap();
        let (_, s) = store(dir.path());
        let Err(ApplyError::Invalid(errors)) =
            s.apply("base.voice", vec![text("pipeline", "other")], None)
        else {
            panic!("accepted");
        };
        assert!(errors.get(&field("pipeline")).is_some());
        // Turning it on and changing the pipeline in one transaction is judged against the values it leaves.
        assert!(
            s.apply(
                "base.voice",
                vec![flag("enabled", true), text("pipeline", "other")],
                None
            )
            .is_ok()
        );
        assert_eq!(s.voice().pipeline, "other");
    }

    #[test]
    fn a_version_precondition_guards_against_a_stale_client() {
        let dir = tempfile::tempdir().unwrap();
        let (_, s) = store(dir.path());
        s.apply("base.news", vec![choice("edition", "nyt")], Some(0))
            .unwrap();
        let Err(ApplyError::Conflict { current }) =
            s.apply("base.news", vec![choice("edition", "bbc")], Some(0))
        else {
            panic!("accepted");
        };
        assert_eq!(current, 1);
        assert_eq!(s.news().edition, "nyt");
        assert!(
            s.apply("base.news", vec![choice("edition", "bbc")], Some(1))
                .is_ok()
        );
    }

    #[test]
    fn a_unit_from_the_theme_is_adopted_unless_one_is_already_stored() {
        let dir = tempfile::tempdir().unwrap();
        let (_hub, s) = store(dir.path());
        assert!(s.adopt_weather_units("celsius"));
        assert_eq!(s.weather().units, "celsius");
        assert!(s.adopt_weather_units("fahrenheit"), "the stored unit stays, and the theme's copy may go");
        assert_eq!(s.weather().units, "celsius");
        let other_dir = tempfile::tempdir().unwrap();
        let (_hub, other) = store(other_dir.path());
        assert!(!other.adopt_weather_units("kelvin"));
    }

    #[test]
    fn values_already_held_change_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (hub, s) = store(dir.path());
        s.apply("base.weather", vec![text("zip", "10001")], None)
            .unwrap();
        let applied = s
            .apply("base.weather", vec![text("zip", "10001")], None)
            .unwrap();
        assert_eq!(
            applied,
            Applied {
                version: 1,
                changed: false
            }
        );
        let none = s.apply("base.weather", vec![], None).unwrap();
        assert!(!none.changed);
        assert_eq!(hub.events_after(Some(0), None, None).events.len(), 1);
    }

    #[test]
    fn a_protected_owner_refuses_the_generic_path_and_serves_the_trusted_one() {
        let dir = tempfile::tempdir().unwrap();
        let (_, s) = store(dir.path());
        s.protect_prefix("ai.");
        let schema = SettingsSchema::parse(
            br#"{"format":1,"owner":"ai.abc","revision":1,"title":"Cody","sections":[{"id":"p","title":"P","fields":[
                {"key":"state","label":"State","kind":{"type":"choice","options":{"inline":[{"id":"ask","label":"Ask"},{"id":"always","label":"Always"}]}},"default":{"type":"choice","value":"ask"},"apply":"on-commit"}]}]}"#,
        )
        .unwrap();
        s.register(schema, None).unwrap();
        let change = choice("state", "always");
        assert!(matches!(
            s.apply("ai.abc", vec![change.clone()], None),
            Err(ApplyError::Protected)
        ));
        assert!(matches!(
            s.apply("ai.abc", vec![], Some(0)),
            Err(ApplyError::Protected)
        ));
        // The unprotected owners are untouched by the rule, and the owner can read.
        assert!(
            s.apply("base.news", vec![choice("edition", "nyt")], None)
                .is_ok()
        );
        assert_eq!(
            s.describe("ai.abc").unwrap()["effective"]["state"]["value"],
            "ask"
        );
        s.apply_trusted("ai.abc", vec![change]).unwrap();
        assert_eq!(
            s.describe("ai.abc").unwrap()["effective"]["state"]["value"],
            "always"
        );
        // A prefix is a prefix of the owner's name, not a substring: `nai.x` is not protected.
        assert!(!s.is_protected("nai.x") && s.is_protected("ai.x"));
    }

    #[test]
    fn an_owner_can_be_removed_with_its_values() {
        let dir = tempfile::tempdir().unwrap();
        let (hub, s) = store(dir.path());
        s.apply("base.news", vec![choice("edition", "nyt")], None)
            .unwrap();
        assert!(dir.path().join("base.news.json").is_file());
        assert!(s.unregister("base.news"));
        assert!(!dir.path().join("base.news.json").exists());
        assert!(s.describe("base.news").is_none());
        assert!(!s.unregister("base.news"));
        let last = hub
            .events_after(Some(0), None, Some(&["settings".to_owned()]))
            .events
            .pop()
            .unwrap();
        assert_eq!(
            (last.kind.as_str(), last.data["owner"].clone()),
            ("removed", json!("base.news"))
        );
    }

    #[test]
    fn an_unknown_owner_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let (_, s) = store(dir.path());
        assert!(matches!(
            s.apply("base.nope", vec![], None),
            Err(ApplyError::UnknownOwner)
        ));
        assert!(s.describe("base.nope").is_none());
    }

    #[test]
    fn a_stored_value_the_schema_no_longer_accepts_is_reported_not_repaired() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("base.weather.json"),
            r#"{"owner":"base.weather","schemaRevision":1,"version":4,"values":{
                "units":{"type":"choice","value":"kelvin"},
                "zip":{"type":"text","value":"60601"},
                "old_key":{"type":"bool","value":true}}}"#,
        )
        .unwrap();
        let (_, s) = store(dir.path());
        // The bad choice falls back to the default, the good value is used.
        assert_eq!(s.weather().units, "fahrenheit");
        assert_eq!(s.weather().zip, "60601");
        let view = s.describe("base.weather").unwrap();
        assert_eq!(view["version"], 4);
        assert!(view["invalid"]["units"].is_string());
        assert_eq!(view["unknown"], json!(["old_key"]));
        assert_eq!(view["effective"]["units"]["value"], "fahrenheit");
        // The file keeps what the person wrote until a transaction replaces it.
        assert!(
            std::fs::read_to_string(dir.path().join("base.weather.json"))
                .unwrap()
                .contains("kelvin")
        );
    }

    #[test]
    fn an_unreadable_file_means_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("base.news.json"), "{not json").unwrap();
        let (_, s) = store(dir.path());
        assert_eq!(s.news().edition, "bbc");
        s.apply("base.news", vec![choice("edition", "nyt")], None)
            .unwrap();
        let (_, again) = store(dir.path());
        assert_eq!(again.news().edition, "nyt");
    }

    #[test]
    fn a_file_for_another_owner_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("base.news.json"),
            r#"{"owner":"base.weather","schemaRevision":1,"version":9,"values":{}}"#,
        )
        .unwrap();
        let (_, s) = store(dir.path());
        assert_eq!(
            s.owners()
                .iter()
                .find(|o| o.owner == "base.news")
                .unwrap()
                .version,
            0
        );
    }

    #[test]
    fn describe_carries_the_documented_schema_and_values() {
        let dir = tempfile::tempdir().unwrap();
        let (_, s) = store(dir.path());
        s.apply("base.news", vec![choice("edition", "nyt")], None)
            .unwrap();
        let view = s.describe("base.news").unwrap();
        assert_eq!(view["owner"], "base.news");
        assert_eq!(view["schema"]["format"], 1);
        assert_eq!(view["schema"]["sections"][0]["fields"][0]["key"], "edition");
        assert_eq!(
            view["values"]["edition"],
            json!({"type": "choice", "value": "nyt"})
        );
        assert_eq!(
            view["effective"]["refresh_minutes"],
            json!({"type": "integer", "value": 15})
        );
    }

    #[test]
    fn a_plugin_owner_registers_with_nothing_but_a_schema() {
        let dir = tempfile::tempdir().unwrap();
        let (_, s) = store(dir.path());
        let schema = SettingsSchema::parse(
            br#"{"format":1,"owner":"hello","revision":1,"title":"Hello","sections":[{"id":"main","title":"Main","fields":[
                {"key":"name","label":"Name","kind":{"type":"text","max_chars":20,"secret":false},"default":{"type":"text","value":"Luma"},"apply":"on-commit"}]}]}"#,
        )
        .unwrap();
        s.register(schema.clone(), None).unwrap();
        s.apply("hello", vec![text("name", "Nitin")], None).unwrap();
        assert_eq!(s.owners().len(), 6);
        // An older revision of the same owner is refused; a newer one replaces the page and keeps the values.
        let mut older = schema.clone();
        older.revision = 0;
        assert!(s.register(older, None).is_err());
        let mut newer = schema;
        newer.revision = 2;
        s.register(newer, None).unwrap();
        assert_eq!(
            s.describe("hello").unwrap()["values"]["name"]["value"],
            "Nitin"
        );
    }

    #[test]
    fn a_schema_that_does_not_validate_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let hub = Arc::new(Hub::new(1));
        let s = SettingsStore::new(dir.path(), hub);
        let mut schema =
            SettingsSchema::parse(include_str!("schemas/base.news.json").as_bytes()).unwrap();
        schema.title = "   ".into();
        assert!(s.register(schema, None).is_err());
    }
}
