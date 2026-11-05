//! Local profiles: a person's Home kept apart from the household's settings.
//!
//! A profile is a person's look, cards and dock, quick panel, key choices and popup places (what `compositor/assets/ui/profile-scope.json`
//! lists); everything else on the TV (picture, sound, network, privacy, AI access, panel care, the screensaver, LG's own settings) belongs
//! to the household and to no profile. Nitin is the owner and the administrator of the shared settings; Isabel is the second profile. LG's
//! account is not a profile and is not read for one.
//!
//! # Storage
//!
//! Nothing that already reads `theme/` has to change. `theme/theme.json`, `theme/layout.json` and `theme/tint.json` stay the **working copy**
//! the Home layer, `lumatv theme set` and `lumatv layout` read and write: the household's keys plus the active profile's own. Each profile
//! keeps its own part in `profiles/<id>/` (`profile.json` with the name, colour, picture and the PIN's salted hash, `theme.json` with only its
//! own keys, `layout.json`, `tint.json`, a photo), and `profiles/index.json` says who the owner and the active profile are. Which key is whose
//! is the one table `profile-scope.json` (longest prefix wins, a key in neither list is the household's).
//!
//! * **Switching** writes the new profile's part into the working copy (the household's keys stay, the leaving profile's own keys go) and
//!   the Home layer, which reads those files again on the `profiles` topic's event (and in any case every 2 s), shows the other person's Home.
//!   An intent (`switching`) is written to the index first and cleared last, so a helper that dies half-way finishes the switch at its next
//!   start instead of leaving two people's files mixed.
//! * **Edits** made through the working copy are carried back by [`Store::sync`] to the profile they were made in: the working copy says whose
//!   it was in `_profile` (theme.json) and in the index (`work`: the profile and the hashes of the three files when they were last written
//!   or read), so an edit saved by a screen that had not yet seen a switch lands in the person who made it, and the working copy is made
//!   the new person's again.
//! * **Migration** is the first open with no index: the working copy becomes Nitin (his own keys cut out of it, the rest staying where it is),
//!   and Isabel is made from Nitin's arrangement (cards, dock, quick panel, key choices, popup places; not his look). The working copy is not
//!   changed but for the mark `_profile` that says whose it is, so the way back (delete `profiles/`) is the TV as it was.
//!
//! # Who may do what
//!
//! The acting person is the active profile (the TV has one). Switching to a profile with a PIN needs the PIN unless the owner is active;
//! creating, removing and the options are the owner's; a profile's name, colour and picture are its own or the owner's; a PIN is changed with the
//! old one or by the owner. Five wrong PINs in a row make a profile wait 30 seconds; each further five, twice as long, five minutes at the most. The PIN is a
//! courtesy lock: four digits have ten thousand guesses, whatever is done to them, and the files are the TV's own. It keeps one person out of
//! another's Home; it does not keep anyone out of the TV. Only a salted hash is written; no digit is logged. Nothing here is exposed to AI
//! access (MCP): an assistant cannot create, remove or unlock a profile.
//!
//! # API
//!
//! Topic `profiles`: the public view (`active`, `owner`, `canAdmin`, `pickerOnWake`, `startWith`, the profiles with name, initial, colour,
//! `avatar`, `avatarFile`, `locked`, `owner`, `firstRun`). Never a hash. The routes are in [`ROUTES`].

use crate::ctx::Ctx;
use crate::http::{Request, Response, error_response};
use crate::hub::Hub;
use crate::timeutil::now_ms;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const TOPIC: &str = "profiles";

pub const ROUTES: &[(&str, &str, &str)] = &[
    (
        "GET",
        "/v1/profiles",
        "who the profiles are, who is active and who owns the TV (the `profiles` topic's value); never a PIN",
    ),
    (
        "GET",
        "/v1/profiles/gallery",
        "the bundled avatars: [{id, set, file}], offline",
    ),
    (
        "POST",
        "/v1/profiles/switch",
        "make a profile the active one: {\"id\", \"pin\"}; the PIN is needed when the profile has one and the owner is not the active profile",
    ),
    (
        "POST",
        "/v1/profiles",
        "owner only: a new profile from the owner's arrangement: {\"name\", \"color\", \"avatar\"}",
    ),
    (
        "POST",
        "/v1/profiles/{id}",
        "a profile's name, colour, picture or `firstRun`: {\"name\", \"color\", \"avatar\", \"firstRun\"}; its own or the owner's",
    ),
    (
        "DELETE",
        "/v1/profiles/{id}",
        "owner only: removes a profile and its files; never the owner",
    ),
    (
        "POST",
        "/v1/profiles/{id}/pin",
        "sets {\"pin\", \"current\"} or clears {\"clear\": true, \"current\"} a PIN; the owner may do either for another profile",
    ),
    (
        "POST",
        "/v1/profiles/{id}/avatar",
        "a photo of at most 48 KB for the profile: {\"image\": base64 of a PNG or JPEG}; its own or the owner's",
    ),
    (
        "POST",
        "/v1/profiles/options",
        "owner only: {\"pickerOnWake\": bool, \"startWith\": \"last\" or an id}",
    ),
];

/// The household's names: the TV's owner and the profile made beside him the first time (`helper.json` does not name people).
pub const OWNER_NAME: &str = "Nitin";
pub const OTHER_NAMES: &[&str] = &["Isabel"];
pub const MAX_PROFILES: usize = 6;
pub const NAME_MAX: usize = 16;
/// Words a profile's id may not be: they are routes.
const RESERVED: &[&str] = &["switch", "options", "gallery", "index"];
/// The most a photo is: the API's body is 64 KiB and a base64 body is a third bigger.
const PHOTO_MAX: usize = 48 * 1024;
/// A file path may hold up to this much before it is read: a working copy is a few kilobytes.
const FILE_MAX: u64 = 1024 * 1024;
/// What a new profile takes from the owner: the arrangement, not the look.
pub const COPY: &[&str] = &[
    "apps",
    "quickPanel.tiles",
    "remote.quickAccess",
    "remote.buttons",
    "layout",
    "tiles.icons",
    "tiles.plates",
];
/// The profile's colours, as data of the profile: Rose Pine's accents.
pub const COLORS: &[&str] = &[
    "#A79BF5", "#EB6F92", "#F6C177", "#9CCFD8", "#3E8FB0", "#EA9A97", "#C4A7E7", "#908CAA",
];
const SCOPE_JSON: &str = include_str!("../../../compositor/assets/ui/profile-scope.json");
/// The key of theme.json that says which profile the working copy was written for.
const MARK: &str = "_profile";
const SYNC_EVERY: Duration = Duration::from_millis(1500);

// ============================================================= scope
/// Whose a key is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Shared,
    Profile,
}

/// `profile-scope.json`: which dotted paths of theme.json are a person's, and which paths inside those are the household's anyway.
pub struct Scope {
    profile: Vec<Vec<String>>,
    shared: Vec<Vec<String>>,
}

fn segments(path: &str) -> Vec<String> {
    path.split('.').map(str::to_owned).collect()
}

fn under(path: &[String], prefix: &[String]) -> bool {
    path.len() >= prefix.len() && path[..prefix.len()] == *prefix
}

impl Scope {
    pub fn load() -> Scope {
        let doc: Value =
            serde_json::from_str(SCOPE_JSON).expect("profile-scope.json is valid JSON");
        let list = |name: &str| -> Vec<Vec<String>> {
            doc.get(name)
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(segments).collect())
                .unwrap_or_default()
        };
        Scope {
            profile: list("profile"),
            shared: list("shared"),
        }
    }

    pub fn side_of(&self, path: &str) -> Side {
        self.side_of_segments(&segments(path))
    }

    fn side_of_segments(&self, path: &[String]) -> Side {
        let mut best = 0usize;
        let mut side = Side::Shared;
        for p in &self.profile {
            if under(path, p) && p.len() > best {
                best = p.len();
                side = Side::Profile;
            }
        }
        for p in &self.shared {
            if under(path, p) && p.len() > best {
                best = p.len();
                side = Side::Shared;
            }
        }
        side
    }

    /// Whether a rule of its own lies below `path`: then a value there cannot go whole to one side.
    fn has_deeper(&self, path: &[String]) -> bool {
        self.profile
            .iter()
            .chain(self.shared.iter())
            .any(|p| p.len() > path.len() && under(p, path))
    }

    /// A theme object cut in two: (the household's, the person's).
    pub fn split(&self, value: &Value) -> (Value, Value) {
        let mut shared = Value::Object(Map::new());
        let mut profile = Value::Object(Map::new());
        self.walk(value, &mut Vec::new(), &mut shared, &mut profile);
        (shared, profile)
    }

    fn walk(&self, value: &Value, path: &mut Vec<String>, shared: &mut Value, profile: &mut Value) {
        let Value::Object(map) = value else { return };
        for (key, child) in map {
            path.push(key.clone());
            if child.is_object() && self.has_deeper(path) {
                self.walk(child, path, shared, profile);
            } else {
                let target = match self.side_of_segments(path) {
                    Side::Shared => &mut *shared,
                    Side::Profile => &mut *profile,
                };
                set_path(target, path, child.clone());
            }
            path.pop();
        }
    }
}

fn set_path(target: &mut Value, path: &[String], value: Value) {
    let mut node = target;
    for key in &path[..path.len() - 1] {
        if !node.get(key).is_some_and(Value::is_object) {
            node.as_object_mut()
                .expect("an object")
                .insert(key.clone(), Value::Object(Map::new()));
        }
        node = node.get_mut(key).expect("just made");
    }
    node.as_object_mut()
        .expect("an object")
        .insert(path[path.len() - 1].clone(), value);
}

/// Objects merge key by key, everything else is replaced: the Home layer's `mergeDeep`.
pub fn merge(base: &Value, over: &Value) -> Value {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            let mut out = b.clone();
            for (k, v) in o {
                let merged = b.get(k).map_or_else(|| v.clone(), |bv| merge(bv, v));
                out.insert(k.clone(), merged);
            }
            Value::Object(out)
        }
        _ => over.clone(),
    }
}

/// The parts of `value` at the given dotted paths, as a new object.
pub fn pick(value: &Value, paths: &[&str]) -> Value {
    let mut out = Value::Object(Map::new());
    for p in paths {
        let segs = segments(p);
        let mut node = value;
        let mut found = true;
        for s in &segs {
            match node.get(s) {
                Some(n) => node = n,
                None => {
                    found = false;
                    break;
                }
            }
        }
        if found {
            set_path(&mut out, &segs, node.clone());
        }
    }
    out
}

// ============================================================= names, colours, PINs
pub fn clean_name(name: &str) -> String {
    let joined: Vec<&str> = name.split_whitespace().collect();
    joined
        .join(" ")
        .chars()
        .take(NAME_MAX)
        .collect::<String>()
        .trim()
        .to_owned()
}

pub fn initial_of(name: &str) -> String {
    name.chars()
        .next()
        .map_or_else(|| "?".to_owned(), |c| c.to_uppercase().collect())
}

/// A short lower-case word no profile has and no route is.
pub fn slug(name: &str, taken: &[String]) -> String {
    let mut s: String = name
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .take(NAME_MAX - 4)
        .collect();
    if s.is_empty() {
        s = "user".to_owned();
    }
    let mut id = s.clone();
    let mut n = 2;
    while taken.contains(&id) || RESERVED.contains(&id.as_str()) {
        id = format!("{s}{n}");
        n += 1;
    }
    id
}

fn id_ok(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= NAME_MAX
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

fn color_ok(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

fn sha_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn pin_valid(pin: &str) -> bool {
    pin.len() == 4 && pin.bytes().all(|b| b.is_ascii_digit())
}

fn new_salt() -> String {
    let mut bytes = [0u8; 8];
    let got = fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes))
        .is_ok();
    if !got {
        bytes = now_ms().to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PinRec {
    salt: String,
    hash: String,
}

impl PinRec {
    fn make(pin: &str) -> PinRec {
        let salt = new_salt();
        let hash = sha_hex(format!("{salt}:{pin}").as_bytes());
        PinRec { salt, hash }
    }

    fn check(&self, pin: &str) -> bool {
        if !pin_valid(pin) {
            return false;
        }
        let want = sha_hex(format!("{}:{pin}", self.salt).as_bytes());
        want.len() == self.hash.len()
            && want
                .bytes()
                .zip(self.hash.bytes())
                .fold(0u8, |d, (a, b)| d | (a ^ b))
                == 0
    }

    fn sane(&self) -> bool {
        self.salt.len() == 16
            && self.salt.bytes().all(|b| b.is_ascii_hexdigit())
            && self.hash.len() == 64
            && self.hash.bytes().all(|b| b.is_ascii_hexdigit())
    }
}

/// How long a profile waits after its `round`-th run of five wrong PINs, in seconds: 30, then twice as long each round, five minutes at most.
pub fn wait_seconds(round: u32) -> u64 {
    if round == 0 {
        return 0;
    }
    300.min(30u64 << (round - 1).min(8))
}

/// Wrong PINs: `count` in this round of five, `rounds` that have made the profile wait, and until when it waits.
struct Fails {
    count: u32,
    rounds: u32,
    until: Option<Instant>,
}

// ============================================================= the files
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Avatar {
    Gallery { id: String },
    Photo { file: String },
    Initial,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    #[serde(default = "one")]
    v: u32,
    name: String,
    #[serde(default)]
    color: String,
    #[serde(default = "initial_avatar")]
    avatar: Avatar,
    #[serde(default)]
    pin: Option<PinRec>,
    #[serde(default)]
    created: u64,
    #[serde(default)]
    first_run: bool,
}

fn one() -> u32 {
    1
}

fn initial_avatar() -> Avatar {
    Avatar::Initial
}

fn last() -> String {
    "last".to_owned()
}

/// What each of the working copy's files was when it was last written here or read here: a sha-256 hex, empty for a file that is not there.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Work {
    #[serde(default)]
    id: String,
    #[serde(default)]
    theme: String,
    #[serde(default)]
    layout: String,
    #[serde(default)]
    tint: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Index {
    #[serde(default = "one")]
    v: u32,
    owner: String,
    active: String,
    order: Vec<String>,
    #[serde(default)]
    picker_on_wake: bool,
    #[serde(default = "last")]
    start_with: String,
    /// A switch begun and not finished: the profile it was going to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    switching: Option<String>,
    #[serde(default)]
    work: Work,
}

fn read_text(path: &Path) -> Option<String> {
    use std::io::Read;
    let file = fs::File::open(path).ok()?;
    let mut text = String::new();
    file.take(FILE_MAX).read_to_string(&mut text).ok()?;
    Some(text)
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&read_text(path)?).ok()
}

/// A file made whole: written beside it and renamed over it, so a reader sees the old file or the new one.
fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode)
        .open(&tmp)?;
    file.write_all(bytes)?;
    file.flush()?;
    drop(file);
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

fn pretty(value: &Value) -> Vec<u8> {
    let mut text = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_owned());
    text.push('\n');
    text.into_bytes()
}

fn hash_file(path: &Path) -> String {
    fs::read(path).map(|b| sha_hex(&b)).unwrap_or_default()
}

// ============================================================= the store
#[derive(Debug, PartialEq)]
pub enum Error {
    /// The store could not be opened (the data directory is not writable): nothing works, the TV does as it did.
    NotReady,
    NoProfile,
    OwnerOnly,
    PinRequired,
    WrongPin {
        tries_left: u32,
    },
    Locked {
        retry_after_s: u64,
    },
    Invalid(String),
    Full,
    Io(String),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}

fn invalid(why: &str) -> Error {
    Error::Invalid(why.to_owned())
}

#[derive(Clone, Debug)]
pub struct GalleryItem {
    pub id: String,
    pub set: String,
    pub file: PathBuf,
}

/// What a profile is made of when it is first written: its record, its own keys, its layout and, for the owner, the wallpaper's sample.
struct Seed {
    id: String,
    record: Record,
    theme: Value,
    layout: Value,
    tint: Option<Vec<u8>>,
}

pub struct Store {
    root: PathBuf,
    theme: PathBuf,
    avatars: PathBuf,
    scope: Scope,
    index: Index,
    records: BTreeMap<String, Record>,
    fails: HashMap<String, Fails>,
    ready: bool,
    bad_reads: u32,
    switched_at: u64,
}

impl Store {
    /// Opens `<data_dir>/profiles`, makes it (the owner from the working copy, and the others from the owner) when there is none, and finishes a
    /// switch that was cut short. Never fails: a store that cannot be had is `ready() == false` and does nothing.
    pub fn open(data_dir: &Path) -> Store {
        let mut s = Store {
            root: data_dir.join("profiles"),
            theme: data_dir.join("theme"),
            avatars: data_dir.join("avatars"),
            scope: Scope::load(),
            index: Index {
                v: 1,
                owner: String::new(),
                active: String::new(),
                order: Vec::new(),
                picker_on_wake: false,
                start_with: last(),
                switching: None,
                work: Work::default(),
            },
            records: BTreeMap::new(),
            fails: HashMap::new(),
            ready: false,
            bad_reads: 0,
            switched_at: 0,
        };
        s.retry();
        s
    }

    /// Opens the store if it is not open: reads it, or makes it from the working copy; finishes a switch that was cut short. Called at the start and,
    /// while the store is not ready (a working copy that could not be read just then), at every look of the helper's thread.
    pub fn retry(&mut self) {
        if self.ready {
            return;
        }
        if !self.load() {
            match self.migrate() {
                Ok(()) => {
                    self.ready = self.load();
                }
                Err(e) => crate::log_warn!("profiles: not made yet: {e:?}"),
            }
        }
        if self.ready && self.index.switching.is_some() {
            self.recover();
        }
    }

    pub fn ready(&self) -> bool {
        self.ready
    }

    fn theme_file(&self) -> PathBuf {
        self.theme.join("theme.json")
    }
    fn layout_file(&self) -> PathBuf {
        self.theme.join("layout.json")
    }
    fn tint_file(&self) -> PathBuf {
        self.theme.join("tint.json")
    }
    fn dir_of(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }
    fn index_file(&self) -> PathBuf {
        self.root.join("index.json")
    }

    /// Reads the index and every profile it lists. A profile whose file cannot be read leaves the list; the owner is then the first one left.
    fn load(&mut self) -> bool {
        let Some(text) = read_text(&self.index_file()) else {
            return false;
        };
        let Ok(mut index) = serde_json::from_str::<Index>(&text) else {
            return false;
        };
        let mut records = BTreeMap::new();
        let mut order = Vec::new();
        for id in &index.order {
            if !id_ok(id) || records.contains_key(id) || order.len() >= MAX_PROFILES {
                continue;
            }
            let rec = fs::read_to_string(self.dir_of(id).join("profile.json"))
                .ok()
                .and_then(|t| serde_json::from_str::<Record>(&t).ok());
            if let Some(mut rec) = rec {
                rec.name = clean_name(&rec.name);
                if rec.name.is_empty() {
                    continue;
                }
                if !color_ok(&rec.color) {
                    rec.color = COLORS[order.len() % COLORS.len()].to_owned();
                }
                if rec.pin.as_ref().is_some_and(|p| !p.sane()) {
                    rec.pin = None;
                }
                rec.avatar = self.clean_avatar(id, &rec.avatar);
                records.insert(id.clone(), rec);
                order.push(id.clone());
            }
        }
        if order.is_empty() {
            return false;
        }
        index.order = order;
        if !records.contains_key(&index.owner) {
            index.owner = index.order[0].clone();
        }
        if !records.contains_key(&index.active) {
            index.active = index.owner.clone();
        }
        if index.start_with != "last" && !records.contains_key(&index.start_with) {
            index.start_with = last();
        }
        if index
            .switching
            .as_ref()
            .is_some_and(|id| !records.contains_key(id))
        {
            index.switching = None;
        }
        if !records.contains_key(&index.work.id) {
            index.work = Work {
                id: index.active.clone(),
                theme: hash_file(&self.theme_file()),
                layout: hash_file(&self.layout_file()),
                tint: hash_file(&self.tint_file()),
            };
        }
        self.index = index;
        self.records = records;
        self.ready = true;
        true
    }

    fn write_index(&self) -> Result<(), Error> {
        let value = serde_json::to_value(&self.index).map_err(|e| Error::Io(e.to_string()))?;
        write_atomic(&self.index_file(), &pretty(&value), 0o644)?;
        Ok(())
    }

    fn write_record(&self, id: &str) -> Result<(), Error> {
        let rec = self.records.get(id).ok_or(Error::NoProfile)?;
        let value = serde_json::to_value(rec).map_err(|e| Error::Io(e.to_string()))?;
        // the PIN's hash is in this file: the owner of the TV reads it, nothing else needs to
        write_atomic(
            &self.dir_of(id).join("profile.json"),
            &pretty(&value),
            0o600,
        )?;
        Ok(())
    }

    /// A file of the working copy as an object: `{}` when it is not there, an error when it is there and cannot be read as one (the Home layer's write is
    /// not atomic, and a hand-edited file may be broken): it is never taken for an empty one, which would be overwritten.
    fn working(&self, path: &Path) -> Result<Value, Error> {
        match read_text(path) {
            None if !path.exists() => Ok(json!({})),
            Some(text) => serde_json::from_str::<Value>(&text)
                .ok()
                .filter(Value::is_object)
                .ok_or_else(|| Error::Io(format!("{} cannot be read just now", path.display()))),
            None => Err(Error::Io(format!(
                "{} cannot be read just now",
                path.display()
            ))),
        }
    }

    // ---- migration
    fn migrate(&mut self) -> Result<(), Error> {
        let working = self.working(&self.theme_file())?;
        let layout = self.working(&self.layout_file())?;
        fs::create_dir_all(&self.root)?;
        let (_, own) = self.scope.split(&working);
        let gallery = self.gallery();
        let avatar_at = |i: usize| {
            gallery
                .get(i)
                .map_or(Avatar::Initial, |g| Avatar::Gallery { id: g.id.clone() })
        };
        let created = now_ms();
        let owner_id = slug(OWNER_NAME, &[]);
        let mut order = vec![owner_id.clone()];
        let mut makes = vec![Seed {
            id: owner_id.clone(),
            record: Record {
                v: 1,
                name: OWNER_NAME.to_owned(),
                color: COLORS[0].to_owned(),
                avatar: avatar_at(0),
                pin: None,
                created,
                first_run: false,
            },
            theme: own.clone(),
            layout: layout.clone(),
            tint: fs::read(self.tint_file()).ok(),
        }];
        for (i, name) in OTHER_NAMES.iter().enumerate() {
            let id = slug(name, &order);
            order.push(id.clone());
            let record = Record {
                v: 1,
                name: (*name).to_owned(),
                color: COLORS[(i + 1) % COLORS.len()].to_owned(),
                avatar: avatar_at(i + 1),
                pin: None,
                created,
                first_run: true,
            };
            makes.push(Seed {
                id,
                record,
                theme: pick(&own, COPY),
                layout: layout.clone(),
                tint: None,
            });
        }
        for seed in &makes {
            let dir = self.dir_of(&seed.id);
            fs::create_dir_all(&dir)?;
            write_atomic(
                &dir.join("profile.json"),
                &pretty(&serde_json::to_value(&seed.record).map_err(|e| Error::Io(e.to_string()))?),
                0o600,
            )?;
            write_atomic(&dir.join("theme.json"), &pretty(&seed.theme), 0o644)?;
            write_atomic(&dir.join("layout.json"), &pretty(&seed.layout), 0o644)?;
            if let Some(tint) = &seed.tint {
                write_atomic(&dir.join("tint.json"), tint, 0o644)?;
            }
        }
        // The index is written last: a migration cut short starts again from the same working copy. The working copy is then put right for the
        // owner, which only adds the mark that says whose it is (the household's keys and the owner's stay as they were): an edit saved by a
        // screen that read the file before this is then known for the owner's, and the first switch is as safe as the hundredth.
        self.index = Index {
            v: 1,
            owner: owner_id.clone(),
            active: owner_id.clone(),
            order,
            picker_on_wake: false,
            start_with: last(),
            switching: None,
            work: Work::default(),
        };
        self.materialize(&owner_id)?;
        self.write_index()
    }

    // ---- the working copy
    /// Writes a profile's part into the working copy: the household's keys stay, the profile's own replace the leaving profile's.
    fn materialize(&mut self, id: &str) -> Result<(), Error> {
        let dir = self.dir_of(id);
        let working = self.working(&self.theme_file())?;
        let (shared, _) = self.scope.split(&working);
        let own = read_json(&dir.join("theme.json"))
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        let (_, own) = self.scope.split(&own);
        let mut out = merge(&shared, &own);
        out.as_object_mut()
            .expect("an object")
            .insert(MARK.to_owned(), json!(id));
        let layout = fs::read(dir.join("layout.json")).unwrap_or_else(|_| b"{}\n".to_vec());
        // the layout first: the theme is what a reader looks at to know a switch has happened
        write_atomic(&self.layout_file(), &layout, 0o644)?;
        match fs::read(dir.join("tint.json")) {
            Ok(tint) => write_atomic(&self.tint_file(), &tint, 0o644)?,
            Err(_) => {
                let _ = fs::remove_file(self.tint_file());
            }
        }
        write_atomic(&self.theme_file(), &pretty(&out), 0o644)?;
        self.index.work = Work {
            id: id.to_owned(),
            theme: hash_file(&self.theme_file()),
            layout: hash_file(&self.layout_file()),
            tint: hash_file(&self.tint_file()),
        };
        Ok(())
    }

    /// Carries the edits made in the working copy since it was written back to the profile they were made in. True when a file was written.
    pub fn sync(&mut self) -> bool {
        if !self.ready {
            return false;
        }
        if self.index.switching.is_some() {
            self.recover();
            return true;
        }
        let mut wrote = false;
        let mut stale = false;
        let work_id = self.index.work.id.clone();
        // whose the edits of this look are: the working copy's person, unless theme.json says a screen that had not seen a switch saved them
        let mut edit_of = work_id.clone();
        // theme.json
        let theme_hash = hash_file(&self.theme_file());
        if theme_hash != self.index.work.theme {
            match read_json(&self.theme_file()).filter(Value::is_object) {
                Some(v) => {
                    // an edit saved by a screen that had not yet seen a switch carries the old profile's mark: it is that person's
                    let whose = v
                        .get(MARK)
                        .and_then(Value::as_str)
                        .filter(|m| self.records.contains_key(*m))
                        .map_or_else(|| work_id.clone(), str::to_owned);
                    let (_, own) = self.scope.split(&v);
                    if self.records.contains_key(&whose)
                        && write_atomic(
                            &self.dir_of(&whose).join("theme.json"),
                            &pretty(&own),
                            0o644,
                        )
                        .is_ok()
                    {
                        self.index.work.theme = theme_hash;
                        self.bad_reads = 0;
                        wrote = true;
                        stale = whose != self.index.active;
                        edit_of = whose;
                    }
                }
                None => {
                    // a file being written (the Home layer's PUT is not atomic) reads as cut short: the next look finds it whole
                    self.bad_reads += 1;
                    if self.bad_reads == 20 {
                        crate::log_warn!(
                            "profiles: theme/theme.json has not been valid JSON for a while; left as it is"
                        );
                    }
                }
            }
        }
        // layout.json: all of it is a person's
        let layout_hash = hash_file(&self.layout_file());
        if layout_hash != self.index.work.layout {
            let bytes = fs::read(self.layout_file()).ok();
            let valid = bytes
                .as_ref()
                .is_some_and(|b| serde_json::from_slice::<Value>(b).is_ok_and(|v| v.is_object()));
            if (valid || bytes.is_none())
                && self.records.contains_key(&edit_of)
                && write_atomic(
                    &self.dir_of(&edit_of).join("layout.json"),
                    &bytes.unwrap_or_else(|| b"{}\n".to_vec()),
                    0o644,
                )
                .is_ok()
            {
                self.index.work.layout = layout_hash;
                wrote = true;
            }
        }
        // tint.json: the wallpaper's sample, the Home layer's own
        let tint_hash = hash_file(&self.tint_file());
        if tint_hash != self.index.work.tint {
            if let Ok(bytes) = fs::read(self.tint_file()) {
                if serde_json::from_slice::<Value>(&bytes).is_ok()
                    && self.records.contains_key(&edit_of)
                    && write_atomic(&self.dir_of(&edit_of).join("tint.json"), &bytes, 0o644).is_ok()
                {
                    self.index.work.tint = tint_hash;
                    wrote = true;
                }
            } else {
                self.index.work.tint = tint_hash;
            }
        }
        if stale {
            // the stale edit is kept for the person it was made by; the working copy is the active person's again
            let active = self.index.active.clone();
            if self.materialize(&active).is_err() {
                crate::log_warn!(
                    "profiles: the working copy could not be put right after a late edit"
                );
            }
        }
        if wrote {
            let _ = self.write_index();
        }
        wrote
    }

    /// Finishes a switch that was begun and not ended.
    fn recover(&mut self) {
        let Some(to) = self.index.switching.clone() else {
            return;
        };
        if !self.records.contains_key(&to) {
            self.index.switching = None;
            let _ = self.write_index();
            return;
        }
        if self.materialize(&to).is_ok() {
            self.index.active = to;
            self.index.switching = None;
            if self.write_index().is_ok() {
                crate::log_info!("profiles: finished a switch that was cut short");
            }
        }
    }

    // ---- who is acting
    pub fn owner_active(&self) -> bool {
        self.ready && self.index.active == self.index.owner
    }
    /// Whether the shared settings may be changed now: the owner's profile is active, or there are no profiles to say otherwise (a store that
    /// could not be opened must not lock the TV's owner out of his own TV).
    pub fn can_admin(&self) -> bool {
        !self.ready || self.owner_active()
    }
    pub fn active_id(&self) -> &str {
        &self.index.active
    }
    pub fn owner_id(&self) -> &str {
        &self.index.owner
    }

    fn need_ready(&self) -> Result<(), Error> {
        if self.ready {
            Ok(())
        } else {
            Err(Error::NotReady)
        }
    }

    fn need_owner(&self) -> Result<(), Error> {
        if self.owner_active() {
            Ok(())
        } else {
            Err(Error::OwnerOnly)
        }
    }

    fn own_or_owner(&self, id: &str) -> Result<(), Error> {
        if self.index.active == id || self.owner_active() {
            Ok(())
        } else {
            Err(Error::OwnerOnly)
        }
    }

    // ---- PINs
    fn check_pin(&mut self, id: &str, pin: Option<&str>) -> Result<(), Error> {
        let Some(rec) = self.records.get(id).and_then(|r| r.pin.clone()) else {
            return Ok(());
        };
        if let Some(f) = self.fails.get(id) {
            if let Some(until) = f.until {
                let now = Instant::now();
                if until > now {
                    return Err(Error::Locked {
                        retry_after_s: (until - now).as_secs() + 1,
                    });
                }
            }
        }
        let Some(pin) = pin else {
            return Err(Error::PinRequired);
        };
        if rec.check(pin) {
            self.fails.remove(id);
            return Ok(());
        }
        let f = self.fails.entry(id.to_owned()).or_insert(Fails {
            count: 0,
            rounds: 0,
            until: None,
        });
        f.count += 1;
        if f.count >= 5 {
            f.count = 0;
            f.rounds += 1;
            let wait = wait_seconds(f.rounds);
            f.until = Some(Instant::now() + Duration::from_secs(wait));
            return Err(Error::Locked {
                retry_after_s: wait,
            });
        }
        Err(Error::WrongPin {
            tries_left: 5 - f.count,
        })
    }

    // ---- the operations
    /// Makes `id` the active profile. The PIN is needed when it has one and the owner is not the active profile.
    pub fn switch(&mut self, id: &str, pin: Option<&str>) -> Result<(), Error> {
        self.need_ready()?;
        if !self.records.contains_key(id) {
            return Err(Error::NoProfile);
        }
        if id == self.index.active && self.index.switching.is_none() {
            return Ok(());
        }
        if !self.owner_active() {
            self.check_pin(id, pin)?;
        }
        // a working copy that is being written cannot be switched just now: nothing is begun, the caller may ask again
        self.working(&self.theme_file())?;
        // what was edited in the leaving profile's Home is its own
        self.sync();
        self.index.switching = Some(id.to_owned());
        self.write_index()?;
        self.materialize(id)?;
        self.index.active = id.to_owned();
        self.index.switching = None;
        self.switched_at = now_ms();
        self.write_index()?;
        Ok(())
    }

    pub fn create(
        &mut self,
        name: &str,
        color: Option<&str>,
        avatar: Option<&Value>,
    ) -> Result<String, Error> {
        self.need_ready()?;
        self.need_owner()?;
        if self.index.order.len() >= MAX_PROFILES {
            return Err(Error::Full);
        }
        let name = clean_name(name);
        if name.is_empty() {
            return Err(invalid("a profile needs a name"));
        }
        self.sync();
        let id = slug(&name, &self.index.order);
        let color = match color {
            Some(c) if color_ok(c) => c.to_owned(),
            Some(_) => return Err(invalid("the colour is #rrggbb")),
            None => COLORS[self.index.order.len() % COLORS.len()].to_owned(),
        };
        let avatar = match avatar {
            Some(a) => self.parse_avatar(&id, a)?,
            None => Avatar::Initial,
        };
        let owner_dir = self.dir_of(&self.index.owner.clone());
        let own = read_json(&owner_dir.join("theme.json")).unwrap_or_else(|| json!({}));
        let layout = fs::read(owner_dir.join("layout.json")).unwrap_or_else(|_| b"{}\n".to_vec());
        let rec = Record {
            v: 1,
            name,
            color,
            avatar,
            pin: None,
            created: now_ms(),
            first_run: true,
        };
        let dir = self.dir_of(&id);
        fs::create_dir_all(&dir)?;
        write_atomic(
            &dir.join("profile.json"),
            &pretty(&serde_json::to_value(&rec).map_err(|e| Error::Io(e.to_string()))?),
            0o600,
        )?;
        write_atomic(&dir.join("theme.json"), &pretty(&pick(&own, COPY)), 0o644)?;
        write_atomic(&dir.join("layout.json"), &layout, 0o644)?;
        self.records.insert(id.clone(), rec);
        self.index.order.push(id.clone());
        self.write_index()?;
        Ok(id)
    }

    pub fn update(&mut self, id: &str, patch: &Value) -> Result<(), Error> {
        self.need_ready()?;
        if !self.records.contains_key(id) {
            return Err(Error::NoProfile);
        }
        self.own_or_owner(id)?;
        let mut rec = self.records[id].clone();
        if let Some(n) = patch.get("name") {
            let name = clean_name(n.as_str().ok_or_else(|| invalid("name is text"))?);
            if name.is_empty() {
                return Err(invalid("a profile needs a name"));
            }
            rec.name = name;
        }
        if let Some(c) = patch.get("color") {
            let c = c
                .as_str()
                .filter(|c| color_ok(c))
                .ok_or_else(|| invalid("the colour is #rrggbb"))?;
            rec.color = c.to_owned();
        }
        if let Some(a) = patch.get("avatar") {
            rec.avatar = self.parse_avatar(id, a)?;
        }
        if let Some(f) = patch.get("firstRun") {
            rec.first_run = f
                .as_bool()
                .ok_or_else(|| invalid("firstRun is true or false"))?;
        }
        self.records.insert(id.to_owned(), rec);
        self.write_record(id)
    }

    pub fn remove(&mut self, id: &str) -> Result<(), Error> {
        self.need_ready()?;
        self.need_owner()?;
        if !self.records.contains_key(id) {
            return Err(Error::NoProfile);
        }
        if id == self.index.owner {
            return Err(invalid("the owner stays"));
        }
        self.sync();
        self.records.remove(id);
        self.index.order.retain(|x| x != id);
        if self.index.active == id {
            self.index.active = self.index.owner.clone();
        }
        if self.index.start_with == id {
            self.index.start_with = last();
        }
        self.fails.remove(id);
        // the index first: a person who is off the list is gone even if the files cannot be removed
        self.write_index()?;
        if let Err(e) = fs::remove_dir_all(self.dir_of(id)) {
            crate::log_warn!("profiles: the files of {id} were left: {e}");
        }
        Ok(())
    }

    /// Sets (`pin` is Some) or clears (None) a PIN. The owner may change another profile's without knowing it; anyone else needs the old one.
    pub fn set_pin(
        &mut self,
        id: &str,
        pin: Option<&str>,
        current: Option<&str>,
    ) -> Result<(), Error> {
        self.need_ready()?;
        if !self.records.contains_key(id) {
            return Err(Error::NoProfile);
        }
        self.own_or_owner(id)?;
        if let Some(p) = pin {
            if !pin_valid(p) {
                return Err(invalid("a PIN is four digits"));
            }
        }
        let overrides = self.owner_active() && id != self.index.owner;
        if !overrides {
            self.check_pin(id, current)?;
        }
        self.records.get_mut(id).expect("checked").pin = pin.map(PinRec::make);
        self.fails.remove(id);
        self.write_record(id)
    }

    pub fn set_options(&mut self, patch: &Value) -> Result<(), Error> {
        self.need_ready()?;
        self.need_owner()?;
        let mut next = self.index.clone();
        if let Some(v) = patch.get("pickerOnWake") {
            next.picker_on_wake = v
                .as_bool()
                .ok_or_else(|| invalid("pickerOnWake is true or false"))?;
        }
        if let Some(v) = patch.get("startWith") {
            let v = v.as_str().ok_or_else(|| invalid("startWith is text"))?;
            if v != "last" && !self.records.contains_key(v) {
                return Err(invalid("startWith is \"last\" or a profile"));
            }
            next.start_with = v.to_owned();
        }
        self.index = next;
        self.write_index()
    }

    /// A photo for the profile (a PNG or JPEG of at most [`PHOTO_MAX`] bytes; the screen crops it to the shape it draws).
    pub fn set_photo(&mut self, id: &str, bytes: &[u8]) -> Result<(), Error> {
        self.need_ready()?;
        if !self.records.contains_key(id) {
            return Err(Error::NoProfile);
        }
        self.own_or_owner(id)?;
        if bytes.is_empty() || bytes.len() > PHOTO_MAX {
            return Err(invalid("a photo is at most 48 KB"));
        }
        let ext = if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
            "png"
        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            "jpg"
        } else {
            return Err(invalid("a photo is a PNG or a JPEG"));
        };
        let dir = self.dir_of(id);
        let file = format!("avatar-{}.{ext}", now_ms() / 1000);
        write_atomic(&dir.join(&file), bytes, 0o644)?;
        if let Ok(entries) = fs::read_dir(&dir) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with("avatar-") && name != file {
                    let _ = fs::remove_file(e.path());
                }
            }
        }
        self.records.get_mut(id).expect("checked").avatar = Avatar::Photo { file };
        self.write_record(id)
    }

    // ---- pictures
    /// The bundled avatars (`<data>/avatars/gallery.json`, staged by `lumatv apply`; the files are PNGs beside it). No manifest, no gallery.
    pub fn gallery(&self) -> Vec<GalleryItem> {
        let Some(doc) = read_json(&self.avatars.join("gallery.json")) else {
            return Vec::new();
        };
        doc.get("avatars")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|a| {
                        let id = a.get("id")?.as_str()?;
                        let file = a.get("file")?.as_str()?;
                        let plain = |s: &str| {
                            !s.is_empty()
                                && s.len() <= 48
                                && s.bytes().all(|b| {
                                    b.is_ascii_lowercase()
                                        || b.is_ascii_digit()
                                        || b == b'-'
                                        || b == b'.'
                                })
                        };
                        if !plain(id) || !plain(file) || file.contains("..") {
                            return None;
                        }
                        let path = self.avatars.join(file);
                        path.is_file().then(|| GalleryItem {
                            id: id.to_owned(),
                            set: a
                                .get("set")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_owned(),
                            file: path,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn clean_avatar(&self, id: &str, a: &Avatar) -> Avatar {
        match a {
            Avatar::Gallery { id: g } if self.gallery().iter().any(|x| x.id == *g) => a.clone(),
            Avatar::Photo { file }
                if file_is_photo(file) && self.dir_of(id).join(file).is_file() =>
            {
                a.clone()
            }
            _ => Avatar::Initial,
        }
    }

    fn parse_avatar(&self, id: &str, a: &Value) -> Result<Avatar, Error> {
        let parsed: Avatar = serde_json::from_value(a.clone())
            .map_err(|_| invalid("avatar is {kind: gallery|photo|initial, ...}"))?;
        match &parsed {
            Avatar::Gallery { id: g } if !self.gallery().iter().any(|x| x.id == *g) => {
                Err(invalid("that avatar is not in the gallery"))
            }
            Avatar::Photo { file }
                if !(file_is_photo(file) && self.dir_of(id).join(file).is_file()) =>
            {
                Err(invalid("that photo is not this profile's"))
            }
            _ => Ok(parsed),
        }
    }

    // ---- the public view
    pub fn view(&self) -> Value {
        if !self.ready {
            return json!({"ready": false, "canAdmin": true});
        }
        let gallery = self.gallery();
        let list: Vec<Value> = self
            .index
            .order
            .iter()
            .filter_map(|id| {
                let r = self.records.get(id)?;
                let (avatar, file) = match &r.avatar {
                    Avatar::Gallery { id: g } => (json!({"kind": "gallery", "id": g}), gallery.iter().find(|x| x.id == *g).map(|x| x.file.display().to_string())),
                    Avatar::Photo { file } => (json!({"kind": "photo", "file": file}), Some(self.dir_of(id).join(file).display().to_string())),
                    Avatar::Initial => (json!({"kind": "initial"}), None),
                };
                Some(json!({
                    "id": id, "name": r.name, "initial": initial_of(&r.name), "color": r.color, "avatar": avatar, "avatarFile": file,
                    "owner": *id == self.index.owner, "active": *id == self.index.active, "locked": r.pin.is_some(), "firstRun": r.first_run,
                }))
            })
            .collect();
        json!({
            "ready": true, "active": self.index.active, "owner": self.index.owner, "canAdmin": self.owner_active(),
            "pickerOnWake": self.index.picker_on_wake, "startWith": self.index.start_with, "max": MAX_PROFILES,
            "switchedAt": self.switched_at, "profiles": list,
        })
    }
}

fn file_is_photo(file: &str) -> bool {
    let Some(rest) = file.strip_prefix("avatar-") else {
        return false;
    };
    let Some((digits, ext)) = rest.split_once('.') else {
        return false;
    };
    !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (ext == "png" || ext == "jpg")
}

// ============================================================= the helper's side: the topic and the routes
pub struct Profiles {
    store: Mutex<Store>,
    hub: Arc<Hub>,
}

fn base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for b in text.bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn respond(e: &Error) -> Response {
    match e {
        Error::NotReady => error_response(
            503,
            "not_ready",
            "profiles are not available: the data directory could not be used",
        ),
        Error::NoProfile => error_response(404, "no_profile", "no such profile"),
        Error::OwnerOnly => {
            error_response(403, "owner_only", "only the owner's profile may do that")
        }
        Error::PinRequired => error_response(403, "pin_required", "this profile has a PIN"),
        Error::WrongPin { tries_left } => {
            let mut r = error_response(403, "wrong_pin", "that is not the PIN");
            r.body = serde_json::to_vec(&json!({"ok": false, "api": crate::API_VERSION, "error": {"code": "wrong_pin", "message": "that is not the PIN", "triesLeft": tries_left}})).unwrap_or(r.body);
            r
        }
        Error::Locked { retry_after_s } => {
            let mut r = error_response(429, "pin_locked", "too many wrong PINs: wait");
            r.body = serde_json::to_vec(&json!({"ok": false, "api": crate::API_VERSION, "error": {"code": "pin_locked", "message": "too many wrong PINs: wait", "retryAfterS": retry_after_s}})).unwrap_or(r.body);
            r.with_header("Retry-After", retry_after_s.to_string())
        }
        Error::Invalid(why) => error_response(400, "bad_request", why),
        Error::Full => error_response(409, "full", "no room for another profile"),
        Error::Io(why) => error_response(500, "io", why),
    }
}

fn body_object(request: &Request) -> Result<Value, Response> {
    if request.body.iter().all(u8::is_ascii_whitespace) {
        return Ok(json!({}));
    }
    match serde_json::from_slice::<Value>(&request.body) {
        Ok(v) if v.is_object() => Ok(v),
        Ok(_) => Err(error_response(
            400,
            "bad_request",
            "the body must be a JSON object",
        )),
        Err(e) => Err(error_response(
            400,
            "bad_request",
            &format!("the body is not JSON: {e}"),
        )),
    }
}

impl Profiles {
    /// Opens the store under the data directory. The topic is set at once.
    pub fn open(ctx: &Ctx) -> Arc<Profiles> {
        let p = Arc::new(Profiles {
            store: Mutex::new(Store::open(&ctx.paths.data_dir)),
            hub: ctx.hub.clone(),
        });
        p.publish();
        p
    }

    /// For another module's rule: is the owner's profile the active one?
    pub fn owner_active(&self) -> bool {
        self.store.lock().owner_active()
    }

    /// For another module's rule, "may the shared settings change now": the owner's profile is active, or profiles are not there at all.
    pub fn can_admin(&self) -> bool {
        self.store.lock().can_admin()
    }

    /// `Ok` when [`Profiles::can_admin`]; else the 403 `owner_only` answer, to be returned as it is, so every module refuses the same way.
    pub fn require_owner(&self) -> Result<(), Response> {
        if self.can_admin() {
            Ok(())
        } else {
            Err(respond(&Error::OwnerOnly))
        }
    }

    pub fn active_id(&self) -> String {
        self.store.lock().active_id().to_owned()
    }

    fn publish(&self) {
        let view = self.store.lock().view();
        self.hub.set(TOPIC, view);
    }

    /// The thread that carries the edits made in the working copy back to the profile they were made in, and keeps the topic true.
    pub fn spawn(self: &Arc<Self>, ctx: &Ctx) -> std::io::Result<JoinHandle<()>> {
        let me = self.clone();
        let stop = ctx.stop.clone();
        thread::Builder::new()
            .name("profiles".to_owned())
            .stack_size(256 * 1024)
            .spawn(move || {
                loop {
                    if stop.sleep(SYNC_EVERY) {
                        return;
                    }
                    {
                        let mut store = me.store.lock();
                        store.retry();
                        store.sync();
                    }
                    me.publish();
                }
            })
    }

    /// Runs `f` on the store and answers with the new view (and `ms`, what it took), or the error's own response.
    fn act(&self, f: impl FnOnce(&mut Store) -> Result<Value, Error>) -> Response {
        let started = Instant::now();
        let out = {
            let mut store = self.store.lock();
            f(&mut store).map(|extra| (extra, store.view()))
        };
        match out {
            Ok((extra, view)) => {
                self.hub.set(TOPIC, view.clone());
                let mut body = json!({"ok": true, "api": crate::API_VERSION, "now": now_ms(), "profiles": view, "ms": started.elapsed().as_secs_f64() * 1000.0});
                if let (Some(map), Some(more)) = (body.as_object_mut(), extra.as_object()) {
                    map.extend(more.clone());
                }
                Response::json(200, &body)
            }
            Err(e) => respond(&e),
        }
    }

    pub fn handle(&self, request: &Request, rest: &[&str]) -> Response {
        let method = request.method.as_str();
        let wrong_method = |allow: &'static str| {
            error_response(405, "method_not_allowed", "that method is not served here")
                .with_header("Allow", allow)
        };
        let body = || body_object(request);
        match rest {
            [] => match method {
                "GET" => {
                    let view = {
                        let mut s = self.store.lock();
                        s.sync();
                        s.view()
                    };
                    Response::json(
                        200,
                        &json!({"ok": true, "api": crate::API_VERSION, "now": now_ms(), "profiles": view}),
                    )
                }
                "POST" => {
                    let b = match body() {
                        Ok(b) => b,
                        Err(r) => return r,
                    };
                    let Some(name) = b.get("name").and_then(Value::as_str) else {
                        return error_response(400, "bad_request", "name is text");
                    };
                    self.act(|s| {
                        let id = s.create(
                            name,
                            b.get("color").and_then(Value::as_str),
                            b.get("avatar"),
                        )?;
                        Ok(json!({"id": id}))
                    })
                }
                _ => wrong_method("GET, POST"),
            },
            ["gallery"] => {
                if method != "GET" {
                    return wrong_method("GET");
                }
                let items: Vec<Value> = self
                    .store
                    .lock()
                    .gallery()
                    .iter()
                    .map(
                        |g| json!({"id": g.id, "set": g.set, "file": g.file.display().to_string()}),
                    )
                    .collect();
                Response::json(
                    200,
                    &json!({"ok": true, "api": crate::API_VERSION, "now": now_ms(), "gallery": items}),
                )
            }
            ["switch"] => {
                if method != "POST" {
                    return wrong_method("POST");
                }
                let b = match body() {
                    Ok(b) => b,
                    Err(r) => return r,
                };
                let Some(id) = b.get("id").and_then(Value::as_str) else {
                    return error_response(400, "bad_request", "id is text");
                };
                let pin = b.get("pin").and_then(Value::as_str);
                self.act(|s| s.switch(id, pin).map(|()| json!({})))
            }
            ["options"] => {
                if method != "POST" {
                    return wrong_method("POST");
                }
                let b = match body() {
                    Ok(b) => b,
                    Err(r) => return r,
                };
                self.act(|s| s.set_options(&b).map(|()| json!({})))
            }
            [id] => match method {
                "POST" => {
                    let b = match body() {
                        Ok(b) => b,
                        Err(r) => return r,
                    };
                    self.act(|s| s.update(id, &b).map(|()| json!({})))
                }
                "DELETE" => self.act(|s| s.remove(id).map(|()| json!({}))),
                _ => wrong_method("POST, DELETE"),
            },
            [id, "pin"] => {
                if method != "POST" {
                    return wrong_method("POST");
                }
                let b = match body() {
                    Ok(b) => b,
                    Err(r) => return r,
                };
                let current = b.get("current").and_then(Value::as_str);
                let clear = b.get("clear").and_then(Value::as_bool) == Some(true);
                let pin = b.get("pin").and_then(Value::as_str);
                if clear == pin.is_some() {
                    return error_response(
                        400,
                        "bad_request",
                        "send either {\"pin\"} or {\"clear\": true}",
                    );
                }
                self.act(|s| {
                    s.set_pin(id, if clear { None } else { pin }, current)
                        .map(|()| json!({}))
                })
            }
            [id, "avatar"] => {
                if method != "POST" {
                    return wrong_method("POST");
                }
                let b = match body() {
                    Ok(b) => b,
                    Err(r) => return r,
                };
                let Some(bytes) = b.get("image").and_then(Value::as_str).and_then(base64) else {
                    return error_response(400, "bad_request", "image is base64");
                };
                self.act(|s| s.set_photo(id, &bytes).map(|()| json!({})))
            }
            _ => error_response(404, "not_found", "no such endpoint"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEGACY: &str = r##"{
        "wallpaper": "wall_aurora.png", "material": "dark", "accent": "#EB6F92", "clock": {"style": "digital", "twelveHour": false},
        "wallpaperMotion": {"enabled": true, "amplitude": 7, "source": "video"}, "weather": {"zip": "30311", "units": "celsius"},
        "apps": {"hidden": ["com.webos.app.browser"], "names": {"netflix": "Films"}}, "quickPanel": {"own": true, "tiles": ["sleep", "picture"]},
        "remote": {"numberKeys": true, "recents": true, "quickAccess": {"1": "netflix"}}, "type": {"body": {"size": 34}},
        "oledcare": {"enabled": true, "dimAfterMin": 4}, "screensaver": {"style": "art"}, "layout": {"overlays": {"volume": {"anchor": "right"}}},
        "overlays": {"enabled": true, "classes": {"doorbell": {"dwellMs": 20000}}}, "cameras": {"liveOverApps": true}
    }"##;
    const LAYOUT: &str = r#"{"widgets": [{"id": "clock", "type": "clock", "col": 0, "row": 0, "span": 4, "rows": 1}]}"#;
    const TINT: &str = r##"{"wallpaper": "file:///x/wall_aurora.png", "tint": "#112233", "accent": "#445566", "light": false, "luma": 0.2}"##;
    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];

    struct Rig {
        dir: tempfile::TempDir,
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        fs::create_dir_all(d.join("theme")).unwrap();
        fs::write(d.join("theme/theme.json"), LEGACY).unwrap();
        fs::write(d.join("theme/layout.json"), LAYOUT).unwrap();
        fs::write(d.join("theme/tint.json"), TINT).unwrap();
        fs::create_dir_all(d.join("avatars")).unwrap();
        fs::write(d.join("avatars/gallery.json"), r#"{"v": 1, "avatars": [{"id": "lorelei-01", "set": "lorelei", "file": "lorelei-01.png"}, {"id": "thumbs-01", "set": "thumbs", "file": "thumbs-01.png"}, {"id": "gone-01", "set": "x", "file": "gone-01.png"}, {"id": "bad", "set": "x", "file": "../up.png"}]}"#).unwrap();
        fs::write(d.join("avatars/lorelei-01.png"), PNG).unwrap();
        fs::write(d.join("avatars/thumbs-01.png"), PNG).unwrap();
        Rig { dir }
    }

    impl Rig {
        fn p(&self) -> &Path {
            self.dir.path()
        }
        fn open(&self) -> Store {
            Store::open(self.p())
        }
        fn theme(&self) -> Value {
            read_json(&self.p().join("theme/theme.json")).unwrap()
        }
        fn set_theme(&self, v: &Value) {
            fs::write(
                self.p().join("theme/theme.json"),
                serde_json::to_string_pretty(v).unwrap(),
            )
            .unwrap();
        }
        fn layout(&self) -> Value {
            read_json(&self.p().join("theme/layout.json")).unwrap()
        }
        fn own(&self, id: &str) -> Value {
            read_json(&self.p().join("profiles").join(id).join("theme.json")).unwrap()
        }
        fn index(&self) -> Value {
            read_json(&self.p().join("profiles/index.json")).unwrap()
        }
    }

    fn legacy() -> Value {
        serde_json::from_str(LEGACY).unwrap()
    }

    #[test]
    fn the_scope_table_names_a_key_by_its_longest_prefix_and_defaults_to_the_household() {
        let s = Scope::load();
        for (path, side) in [
            ("wallpaper", Side::Profile),
            ("clock.style", Side::Profile),
            ("wallpaperMotion.amplitude", Side::Profile),
            ("wallpaperMotion.source", Side::Shared),
            ("weather.zip", Side::Shared),
            ("weather.units", Side::Profile),
            ("remote.numberKeys", Side::Shared),
            ("remote.quickAccess.1", Side::Profile),
            ("layout.overlays.volume", Side::Profile),
            ("overlays.enabled", Side::Shared),
            ("overlays.classes.alarm.dwellMs", Side::Profile),
            ("cameras.liveOverApps", Side::Shared),
            ("somethingNew.key", Side::Shared),
            ("wallpaperfoo", Side::Shared),
        ] {
            assert_eq!(s.side_of(path), side, "{path}");
        }
    }

    #[test]
    fn a_theme_is_cut_in_two_with_nothing_lost_and_nothing_twice() {
        let s = Scope::load();
        let (shared, own) = s.split(&legacy());
        assert_eq!(own["wallpaper"], "wall_aurora.png");
        assert_eq!(
            own["wallpaperMotion"],
            json!({"enabled": true, "amplitude": 7})
        );
        assert_eq!(shared["wallpaperMotion"], json!({"source": "video"}));
        assert_eq!(own["weather"], json!({"units": "celsius"}));
        assert_eq!(shared["weather"], json!({"zip": "30311"}));
        assert_eq!(own["quickPanel"], json!({"tiles": ["sleep", "picture"]}));
        assert_eq!(shared["quickPanel"], json!({"own": true}));
        assert_eq!(own["remote"], json!({"quickAccess": {"1": "netflix"}}));
        assert_eq!(
            shared["remote"],
            json!({"numberKeys": true, "recents": true})
        );
        assert_eq!(
            shared["oledcare"],
            json!({"enabled": true, "dimAfterMin": 4})
        );
        assert_eq!(shared["cameras"], json!({"liveOverApps": true}));
        assert_eq!(
            own["overlays"],
            json!({"classes": {"doorbell": {"dwellMs": 20000}}})
        );
        assert_eq!(shared["overlays"], json!({"enabled": true}));
        assert_eq!(
            own["layout"],
            json!({"overlays": {"volume": {"anchor": "right"}}})
        );
        assert!(own.get("oledcare").is_none() && shared.get("wallpaper").is_none());
        assert_eq!(merge(&shared, &own), legacy());
        assert_eq!(s.split(&json!({})), (json!({}), json!({})));
        let (shared, own) =
            s.split(&json!({"apps": {"hidden": []}, "clock": null, "news": {"feeds": [1, 2]}}));
        assert_eq!(
            own,
            json!({"apps": {"hidden": []}, "clock": null, "news": {"feeds": [1, 2]}})
        );
        assert_eq!(shared, json!({}));
    }

    #[test]
    fn the_shipped_theme_splits_and_composes_with_itself_and_the_table_names_keys_that_exist() {
        let shipped: Value =
            serde_json::from_str(include_str!("../../../payload/theme/theme.json")).unwrap();
        let s = Scope::load();
        let (shared, own) = s.split(&shipped);
        assert_eq!(merge(&shared, &own), shipped);
        let layout: Value =
            serde_json::from_str(include_str!("../../../payload/theme/layout.json")).unwrap();
        let bar = layout["statusBar"]["items"].as_array().unwrap();
        assert_eq!(bar.last().unwrap()["type"], "profile", "the chip ends the status bar");
        // every profile rule that is not a key the owner adds himself is in the shipped file (a rule that names nothing is a typo)
        let added = [
            "tiles.icons",
            "tiles.plates",
            "apps",
            "quickPanel.tiles",
            "remote.quickAccess",
            "remote.buttons",
            "search",
            "layout",
            "overlays.classes",
            "weather.units",
            "weather.days",
            "wallpaperMotion.source",
            "wallpaperMotion.glass",
        ];
        let doc: Value = serde_json::from_str(SCOPE_JSON).unwrap();
        for rule in doc["profile"]
            .as_array()
            .unwrap()
            .iter()
            .chain(doc["shared"].as_array().unwrap())
        {
            let rule = rule.as_str().unwrap();
            let first = rule.split('.').next().unwrap();
            assert!(
                added.contains(&rule)
                    || pick(&shipped, &[rule])
                        .as_object()
                        .is_some_and(|m| !m.is_empty()),
                "{rule} (top key {first})"
            );
        }
        for path in COPY {
            assert_eq!(
                s.side_of(path),
                Side::Profile,
                "{path} is copied from the owner, so it must be a person's"
            );
        }
    }

    #[test]
    fn merge_and_pick() {
        let base = json!({"a": 1, "o": {"x": 1, "y": 2}, "l": [1, 2]});
        assert_eq!(
            merge(&base, &json!({"o": {"y": 3, "z": 4}, "l": [9], "a": null})),
            json!({"a": null, "o": {"x": 1, "y": 3, "z": 4}, "l": [9]})
        );
        assert_eq!(
            pick(&legacy(), COPY),
            json!({"apps": legacy()["apps"], "quickPanel": {"tiles": ["sleep", "picture"]}, "remote": {"quickAccess": {"1": "netflix"}}, "layout": legacy()["layout"]})
        );
        assert_eq!(pick(&json!({}), COPY), json!({}));
    }

    #[test]
    fn an_id_is_a_short_lower_case_word_never_taken_and_never_a_route() {
        assert_eq!(slug("Nitin", &[]), "nitin");
        assert_eq!(slug("Nitin", &["nitin".into()]), "nitin2");
        assert_eq!(slug("Nitin", &["nitin".into(), "nitin2".into()]), "nitin3");
        assert_eq!(slug("  Zo\u{eb} & Co.  ", &[]), "zoco");
        assert_eq!(slug("***", &[]), "user");
        assert_eq!(slug("Options", &[]), "options2");
        assert_eq!(slug("Switch", &[]), "switch2");
        assert!(slug(&"a".repeat(40), &[]).len() <= NAME_MAX - 3);
        assert_eq!(clean_name("  Isabel   Rose \n"), "Isabel Rose");
        assert_eq!(clean_name(&"x".repeat(40)).chars().count(), NAME_MAX);
        assert_eq!(initial_of("isabel"), "I");
        assert_eq!(initial_of(""), "?");
        assert!(
            id_ok("nitin")
                && !id_ok("Nitin")
                && !id_ok("a.b")
                && !id_ok("")
                && !id_ok(&"a".repeat(17))
        );
        assert!(color_ok("#12ab9F") && !color_ok("#12ab9") && !color_ok("red"));
    }

    #[test]
    fn the_first_open_makes_nitin_from_the_working_copy_and_isabel_from_his_arrangement() {
        let r = rig();
        let before = r.theme();
        let s = r.open();
        assert!(s.ready());
        assert_eq!(s.index.order, ["nitin", "isabel"]);
        assert_eq!(
            (s.index.owner.as_str(), s.index.active.as_str()),
            ("nitin", "nitin")
        );
        // the working copy is what it was, plus the mark that says it is Nitin's
        let mut expect = before.clone();
        expect["_profile"] = json!("nitin");
        assert_eq!(r.theme(), expect);
        assert_eq!(r.layout(), serde_json::from_str::<Value>(LAYOUT).unwrap());
        assert_eq!(
            fs::read_to_string(r.p().join("theme/tint.json")).unwrap(),
            TINT
        );
        // Nitin has his own keys and all of them
        let (shared, own) = Scope::load().split(&legacy());
        assert_eq!(r.own("nitin"), own);
        assert!(shared.get("oledcare").is_some() && r.own("nitin").get("oledcare").is_none());
        // Isabel has his arrangement, none of his look
        assert_eq!(r.own("isabel"), pick(&own, COPY));
        assert!(
            r.own("isabel").get("wallpaper").is_none() && r.own("isabel").get("accent").is_none()
        );
        assert_eq!(
            read_json(&r.p().join("profiles/isabel/layout.json")).unwrap(),
            serde_json::from_str::<Value>(LAYOUT).unwrap()
        );
        assert!(!r.p().join("profiles/isabel/tint.json").exists());
        assert_eq!(
            read_json(&r.p().join("profiles/nitin/tint.json")).unwrap()["tint"],
            "#112233"
        );
        let nitin = &s.records["nitin"];
        let isabel = &s.records["isabel"];
        assert_eq!(
            (nitin.name.as_str(), isabel.name.as_str()),
            ("Nitin", "Isabel")
        );
        assert!(!nitin.first_run && isabel.first_run);
        assert_eq!(
            nitin.avatar,
            Avatar::Gallery {
                id: "lorelei-01".into()
            }
        );
        assert_eq!(
            isabel.avatar,
            Avatar::Gallery {
                id: "thumbs-01".into()
            }
        );
        assert_ne!(nitin.color, isabel.color);
        assert_eq!(r.index()["work"]["id"], "nitin");
    }

    #[test]
    fn opening_again_changes_nothing_and_a_migration_cut_short_is_made_again() {
        let r = rig();
        drop(r.open());
        let theme = r.theme();
        let index = fs::read(r.p().join("profiles/index.json")).unwrap();
        let again = r.open();
        assert_eq!(again.index.order.len(), 2);
        assert_eq!(r.theme(), theme);
        assert_eq!(fs::read(r.p().join("profiles/index.json")).unwrap(), index);
        // no index (the migration died before it): the working copy still has everything, and the same two come out
        fs::remove_file(r.p().join("profiles/index.json")).unwrap();
        let third = r.open();
        assert!(third.ready());
        assert_eq!(third.index.order, ["nitin", "isabel"]);
        assert_eq!(r.own("nitin"), Scope::load().split(&legacy()).1);
    }

    #[test]
    fn no_working_copy_at_all_is_a_fresh_tv_with_both_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path());
        assert!(s.ready());
        assert_eq!(s.index.order, ["nitin", "isabel"]);
        assert_eq!(
            read_json(&dir.path().join("theme/theme.json")).unwrap(),
            json!({"_profile": "nitin"})
        );
        assert_eq!(s.records["nitin"].avatar, Avatar::Initial); // no gallery staged: the colour and the letter
    }

    #[test]
    fn a_switch_puts_the_other_persons_home_in_the_working_copy_and_back_again_loses_nothing() {
        let r = rig();
        let mut s = r.open();
        let nitin_home = r.theme();
        let started = Instant::now();
        s.switch("isabel", None).unwrap();
        let took = started.elapsed();
        let t = r.theme();
        assert_eq!(t["_profile"], "isabel");
        assert_eq!(t["oledcare"]["enabled"], true); // the household's stay
        assert_eq!(t["wallpaperMotion"], json!({"source": "video"}));
        assert_eq!(t["weather"], json!({"zip": "30311"}));
        assert_eq!(t["overlays"], json!({"enabled": true}));
        assert!(
            t.get("wallpaper").is_none()
                && t.get("accent").is_none()
                && t.get("clock").is_none()
                && t.get("type").is_none()
        );
        assert_eq!(t["apps"], legacy()["apps"]); // hers, from his arrangement
        assert!(
            !r.p().join("theme/tint.json").exists(),
            "her wallpaper has not been sampled yet"
        );
        assert_eq!(s.index.active, "isabel");
        assert!(s.index.switching.is_none());
        let idx = r.index();
        assert_eq!(idx["active"], "isabel");
        assert!(idx.get("switching").is_none());
        assert_eq!(
            idx["work"]["theme"],
            hash_file(&r.p().join("theme/theme.json"))
        );
        assert_eq!(
            idx["work"]["layout"],
            hash_file(&r.p().join("theme/layout.json"))
        );
        assert_eq!(idx["work"]["tint"], "");
        // and back: Nitin's Home is exactly what it was
        s.switch("nitin", None).unwrap();
        assert_eq!(r.theme(), nitin_home);
        assert_eq!(
            fs::read_to_string(r.p().join("theme/tint.json")).unwrap(),
            TINT
        );
        assert_eq!(r.index()["active"], "nitin");
        assert!(took < Duration::from_millis(250), "a switch took {took:?}");
    }

    #[test]
    fn a_switch_to_the_active_profile_or_to_nobody_does_nothing() {
        let r = rig();
        let mut s = r.open();
        let before = fs::read(r.p().join("profiles/index.json")).unwrap();
        s.switch("nitin", None).unwrap();
        assert_eq!(fs::read(r.p().join("profiles/index.json")).unwrap(), before);
        assert_eq!(s.switch("ghost", None), Err(Error::NoProfile));
        assert_eq!(s.switch("../etc", None), Err(Error::NoProfile));
    }

    #[test]
    fn what_the_home_layer_writes_is_carried_to_the_profile_it_was_written_in() {
        let r = rig();
        let mut s = r.open();
        assert!(!s.sync(), "nothing has changed");
        // the Home layer saves the file it read plus the change: an accent (his) and a screensaver choice (the household's)
        let mut t = r.theme();
        t["accent"] = json!("#00FF00");
        t["screensaver"]["style"] = json!("clock");
        r.set_theme(&t);
        fs::write(r.p().join("theme/layout.json"), r#"{"widgets": []}"#).unwrap();
        fs::write(r.p().join("theme/tint.json"), r#"{"luma": 0.9}"#).unwrap();
        assert!(s.sync());
        assert_eq!(r.own("nitin")["accent"], "#00FF00");
        assert!(
            r.own("nitin").get("screensaver").is_none() && r.own("nitin").get("_profile").is_none()
        );
        assert_eq!(
            read_json(&r.p().join("profiles/nitin/layout.json")).unwrap(),
            json!({"widgets": []})
        );
        assert_eq!(
            read_json(&r.p().join("profiles/nitin/tint.json")).unwrap(),
            json!({"luma": 0.9})
        );
        assert_eq!(
            read_json(&r.p().join("profiles/isabel/layout.json")).unwrap(),
            serde_json::from_str::<Value>(LAYOUT).unwrap(),
            "Isabel's is untouched"
        );
        assert_eq!(
            r.theme()["screensaver"]["style"],
            "clock",
            "the household's key stays in the working copy"
        );
        assert!(!s.sync(), "the second look finds nothing new");
        // the edit survives a switch and a return
        s.switch("isabel", None).unwrap();
        assert_eq!(r.theme()["screensaver"]["style"], "clock");
        assert!(r.theme().get("accent").is_none());
        s.switch("nitin", None).unwrap();
        assert_eq!(r.theme()["accent"], "#00FF00");
    }

    #[test]
    fn an_edit_saved_by_a_screen_that_had_not_seen_the_switch_belongs_to_the_one_who_made_it() {
        let r = rig();
        let mut s = r.open();
        let nitins_text = r.theme(); // what the Home layer holds: his, with his mark
        s.switch("isabel", None).unwrap();
        // the layer saves what it held plus a change, before it has read Isabel's files
        let mut late = nitins_text.clone();
        late["accent"] = json!("#112233");
        late["screensaver"]["style"] = json!("clock");
        r.set_theme(&late);
        assert!(s.sync());
        assert_eq!(r.own("nitin")["accent"], "#112233", "the edit is Nitin's");
        assert!(r.own("isabel").get("accent").is_none(), "and not Isabel's");
        let t = r.theme();
        assert_eq!(
            t["_profile"], "isabel",
            "the working copy is Isabel's again"
        );
        assert!(t.get("accent").is_none());
        assert_eq!(
            t["screensaver"]["style"], "clock",
            "the household's part of the late edit stands"
        );
        assert_eq!(s.index.active, "isabel");
    }

    #[test]
    fn a_file_caught_half_written_is_left_alone_until_it_is_whole() {
        let r = rig();
        let mut s = r.open();
        let own = r.own("nitin");
        fs::write(r.p().join("theme/theme.json"), r##"{"accent": "#12"##).unwrap();
        assert!(!s.sync());
        assert_eq!(r.own("nitin"), own);
        let mut t = legacy();
        t["accent"] = json!("#ABCDEF");
        t[MARK] = json!("nitin");
        r.set_theme(&t);
        assert!(s.sync());
        assert_eq!(r.own("nitin")["accent"], "#ABCDEF");
    }

    #[test]
    fn a_working_copy_that_cannot_be_read_is_never_taken_for_an_empty_one() {
        let r = rig();
        let broken = r##"{"accent": "#12"##;
        fs::write(r.p().join("theme/theme.json"), broken).unwrap();
        let mut s = r.open();
        assert!(
            !s.ready(),
            "nothing is made from a file that cannot be read"
        );
        assert!(!r.p().join("profiles/index.json").exists());
        assert_eq!(
            fs::read_to_string(r.p().join("theme/theme.json")).unwrap(),
            broken,
            "and the file is as it was"
        );
        // the Home layer's write lands whole: the next look makes the profiles from what is there
        fs::write(r.p().join("theme/theme.json"), LEGACY).unwrap();
        s.retry();
        assert!(s.ready());
        assert_eq!(
            r.own("nitin"),
            Scope::load().split(&legacy()).1,
            "Nitin has his own keys"
        );
        // a layout file that cannot be read stops it the same way
        let r2 = rig();
        fs::write(r2.p().join("theme/layout.json"), "{\"widgets\": [").unwrap();
        assert!(!r2.open().ready());
    }

    #[test]
    fn a_switch_asked_while_the_working_theme_is_half_written_begins_nothing() {
        let r = rig();
        let mut s = r.open();
        let before = r.index();
        fs::write(r.p().join("theme/theme.json"), r##"{"accent": "#12"##).unwrap();
        assert!(matches!(s.switch("isabel", None), Err(Error::Io(_))));
        assert_eq!(r.index(), before, "no intent was written");
        assert!(s.index.switching.is_none());
        assert_eq!(s.index.active, "nitin");
        let mut t = legacy();
        t[MARK] = json!("nitin");
        r.set_theme(&t);
        s.switch("isabel", None).unwrap();
        assert_eq!(r.theme()[MARK], "isabel");
    }

    #[test]
    fn a_late_layout_saved_with_a_late_theme_is_the_old_persons_too() {
        let r = rig();
        let mut s = r.open();
        let nitins_text = r.theme();
        s.switch("isabel", None).unwrap();
        let hers = read_json(&r.p().join("profiles/isabel/layout.json")).unwrap();
        // a screen that still held Nitin's Home saves its layout and its theme, after the switch
        fs::write(r.p().join("theme/layout.json"), r#"{"widgets": [{"id": "news", "type": "news", "col": 0, "row": 0, "span": 4, "rows": 1}]}"#).unwrap();
        let mut late = nitins_text;
        late["accent"] = json!("#112233");
        r.set_theme(&late);
        assert!(s.sync());
        assert_eq!(
            read_json(&r.p().join("profiles/nitin/layout.json")).unwrap()["widgets"][0]["id"],
            "news",
            "the layout is Nitin's"
        );
        assert_eq!(
            read_json(&r.p().join("profiles/isabel/layout.json")).unwrap(),
            hers,
            "and Isabel's is as it was"
        );
        assert_eq!(r.layout(), hers, "the working copy is hers again");
    }

    #[test]
    fn an_index_that_names_nobody_for_the_working_copy_starts_from_the_files_as_they_are() {
        let r = rig();
        drop(r.open());
        let mut index = r.index();
        index.as_object_mut().unwrap().remove("work");
        fs::write(
            r.p().join("profiles/index.json"),
            serde_json::to_vec(&index).unwrap(),
        )
        .unwrap();
        let mut s = r.open();
        assert_eq!(s.index.work.id, "nitin");
        let before = r.own("nitin");
        assert!(!s.sync(), "nothing is taken for an edit");
        assert_eq!(r.own("nitin"), before);
    }

    #[test]
    fn a_switch_cut_short_is_finished_at_the_next_open() {
        let r = rig();
        let mut s = r.open();
        // the intent is written, the layout is Isabel's, the theme is still Nitin's: the helper died here
        s.sync();
        s.index.switching = Some("isabel".into());
        s.write_index().unwrap();
        fs::copy(
            r.p().join("profiles/isabel/layout.json"),
            r.p().join("theme/layout.json"),
        )
        .unwrap();
        drop(s);
        let s = r.open();
        assert_eq!(s.index.active, "isabel");
        assert!(s.index.switching.is_none());
        assert_eq!(r.theme()["_profile"], "isabel");
        assert!(r.theme().get("accent").is_none());
        assert!(r.index().get("switching").is_none());
        // and Nitin's own file was not given Isabel's keys or the other way round
        assert_eq!(r.own("nitin"), Scope::load().split(&legacy()).1);
    }

    #[test]
    fn an_unfinished_switch_to_a_profile_that_is_gone_is_dropped() {
        let r = rig();
        let mut s = r.open();
        s.index.switching = Some("ghost".into());
        s.write_index().unwrap();
        drop(s);
        let s = r.open();
        assert!(s.index.switching.is_none());
        assert_eq!(s.index.active, "nitin");
    }

    #[test]
    fn a_profile_whose_file_is_gone_leaves_the_list_and_the_owner_is_the_first_left() {
        let r = rig();
        drop(r.open());
        fs::remove_file(r.p().join("profiles/isabel/profile.json")).unwrap();
        let s = r.open();
        assert_eq!(s.index.order, ["nitin"]);
        fs::write(r.p().join("profiles/nitin/profile.json"), "not json").unwrap();
        let s = r.open(); // nobody can be read: made again from the working copy
        assert!(s.ready());
        assert_eq!(s.index.order, ["nitin", "isabel"]);
    }

    #[test]
    fn a_data_directory_that_cannot_be_written_is_not_ready_and_locks_nobody_out() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("profiles"),
            "a file where the directory goes",
        )
        .unwrap();
        let mut s = Store::open(dir.path());
        assert!(!s.ready());
        assert!(s.can_admin() && !s.owner_active());
        assert_eq!(s.switch("isabel", None), Err(Error::NotReady));
        assert_eq!(s.view(), json!({"ready": false, "canAdmin": true}));
        assert!(!s.sync());
    }

    fn with_pin(s: &mut Store, id: &str, pin: &str) {
        s.set_pin(id, Some(pin), None).unwrap();
    }

    #[test]
    fn the_owner_opens_any_home_without_its_pin() {
        let r = rig();
        let mut s = r.open();
        with_pin(&mut s, "isabel", "4821"); // the owner sets it for her, not knowing hers
        assert!(s.records["isabel"].pin.is_some());
        s.switch("isabel", None).unwrap(); // and may open her Home without it
        assert_eq!(s.index.active, "isabel");
        assert_eq!(
            s.set_pin("nitin", Some("1357"), None),
            Err(Error::OwnerOnly),
            "she may not set his"
        );
    }

    #[test]
    fn only_the_owner_or_the_person_may_set_a_pin_and_the_old_one_is_needed() {
        let r = rig();
        let mut s = r.open();
        s.switch("isabel", None).unwrap();
        assert_eq!(
            s.set_pin("nitin", Some("1357"), None),
            Err(Error::OwnerOnly)
        );
        s.set_pin("isabel", Some("4821"), None).unwrap(); // her own, with none yet
        assert_eq!(
            s.set_pin("isabel", Some("1111"), None),
            Err(Error::PinRequired)
        );
        assert_eq!(
            s.set_pin("isabel", Some("1111"), Some("0000")),
            Err(Error::WrongPin { tries_left: 4 })
        );
        s.set_pin("isabel", Some("1111"), Some("4821")).unwrap();
        assert_eq!(
            s.set_pin("isabel", Some("12"), Some("1111")),
            Err(Error::Invalid("a PIN is four digits".into()))
        );
        assert_eq!(
            s.set_pin("isabel", Some("abcd"), Some("1111")),
            Err(Error::Invalid("a PIN is four digits".into()))
        );
        s.set_pin("isabel", None, Some("1111")).unwrap();
        assert!(s.records["isabel"].pin.is_none());
        // the owner's own PIN is his to change with the old one; the owner may clear hers without it
        s.switch("nitin", None).unwrap();
        s.set_pin("nitin", Some("2468"), None).unwrap();
        assert_eq!(s.set_pin("nitin", None, None), Err(Error::PinRequired));
        s.set_pin("isabel", Some("9999"), None).unwrap();
        s.set_pin("isabel", None, None).unwrap();
    }

    #[test]
    fn switching_to_a_profile_with_a_pin_needs_it_and_five_wrong_ones_make_it_wait() {
        let r = rig();
        let mut s = r.open();
        s.switch("isabel", None).unwrap();
        s.set_pin("isabel", Some("4821"), None).unwrap();
        s.set_pin("nitin", Some("2468"), None).unwrap_err(); // not hers to set
        s.switch("nitin", None).unwrap(); // no PIN on him yet
        s.set_pin("nitin", Some("2468"), None).unwrap();
        s.switch("isabel", None).unwrap(); // the owner is active: no PIN needed
        // now Isabel is active and Nitin is locked
        assert_eq!(s.switch("nitin", None), Err(Error::PinRequired));
        assert_eq!(
            s.switch("nitin", Some("0000")),
            Err(Error::WrongPin { tries_left: 4 })
        );
        assert_eq!(
            s.switch("nitin", Some("1111")),
            Err(Error::WrongPin { tries_left: 3 })
        );
        assert_eq!(
            s.switch("nitin", Some("2222")),
            Err(Error::WrongPin { tries_left: 2 })
        );
        assert_eq!(
            s.switch("nitin", Some("3333")),
            Err(Error::WrongPin { tries_left: 1 })
        );
        assert!(matches!(
            s.switch("nitin", Some("4444")),
            Err(Error::Locked { retry_after_s: 30 })
        ));
        // the right PIN does not open it while it waits
        assert!(matches!(
            s.switch("nitin", Some("2468")),
            Err(Error::Locked { .. })
        ));
        assert_eq!(s.index.active, "isabel");
        s.fails.get_mut("nitin").unwrap().until = Some(Instant::now() - Duration::from_secs(1));
        assert_eq!(
            s.switch("nitin", Some("0000")),
            Err(Error::WrongPin { tries_left: 4 }),
            "after the wait there are five more tries"
        );
        for pin in ["1111", "2222", "3333"] {
            assert!(matches!(
                s.switch("nitin", Some(pin)),
                Err(Error::WrongPin { .. })
            ));
        }
        assert!(
            matches!(
                s.switch("nitin", Some("4444")),
                Err(Error::Locked { retry_after_s: 60 })
            ),
            "and the next wait is twice as long"
        );
        s.fails.get_mut("nitin").unwrap().until = Some(Instant::now() - Duration::from_secs(1));
        s.switch("nitin", Some("2468")).unwrap();
        assert_eq!(s.index.active, "nitin");
        assert!(
            !s.fails.contains_key("nitin"),
            "the right PIN clears the count"
        );
    }

    #[test]
    fn a_pin_is_never_written_or_shown_and_lock_times_grow() {
        let r = rig();
        let mut s = r.open();
        s.set_pin("isabel", Some("4821"), None).unwrap();
        let file = fs::read_to_string(r.p().join("profiles/isabel/profile.json")).unwrap();
        assert!(!file.contains("4821"));
        let rec: Record = serde_json::from_str(&file).unwrap();
        let pin = rec.pin.unwrap();
        assert!(pin.check("4821") && !pin.check("4822") && !pin.check("48") && !pin.check(""));
        assert_ne!(
            PinRec::make("4821").hash,
            pin.hash,
            "another salt, another hash"
        );
        let view = s.view().to_string();
        assert!(
            !view.contains(&pin.hash)
                && !view.contains(&pin.salt)
                && !view.contains("4821")
                && !view.contains("\"pin\"")
        );
        assert_eq!(s.view()["profiles"][1]["locked"], true);
        assert_eq!(
            [0, 1, 2, 3, 4, 5, 9, 100].map(wait_seconds),
            [0, 30, 60, 120, 240, 300, 300, 300]
        );
        let mode = fs::metadata(r.p().join("profiles/isabel/profile.json")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            mode.permissions().mode() & 0o777,
            0o600,
            "the hash is for the TV's owner to read"
        );
    }

    #[test]
    fn only_the_owner_adds_and_removes_and_a_new_profile_starts_from_his_arrangement() {
        let r = rig();
        let mut s = r.open();
        let id = s
            .create(
                "Mia",
                Some("#EB6F92"),
                Some(&json!({"kind": "gallery", "id": "thumbs-01"})),
            )
            .unwrap();
        assert_eq!(id, "mia");
        assert_eq!(s.index.order, ["nitin", "isabel", "mia"]);
        assert_eq!(r.own("mia"), pick(&Scope::load().split(&legacy()).1, COPY));
        assert!(s.records["mia"].first_run);
        assert_eq!(s.records["mia"].color, "#EB6F92");
        assert_eq!(s.create("Mia", None, None).unwrap(), "mia2");
        assert_eq!(s.create("Options", None, None).unwrap(), "options2");
        assert_eq!(
            s.create("  ", None, None),
            Err(Error::Invalid("a profile needs a name".into()))
        );
        assert!(matches!(
            s.create("X", Some("red"), None),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            s.create("X", None, Some(&json!({"kind": "gallery", "id": "nope"}))),
            Err(Error::Invalid(_))
        ));
        assert_eq!(s.create("Six", None, None).unwrap(), "six");
        assert_eq!(s.create("Seven", None, None), Err(Error::Full));
        s.switch("isabel", None).unwrap();
        assert_eq!(s.create("Y", None, None), Err(Error::OwnerOnly));
        assert_eq!(s.remove("mia"), Err(Error::OwnerOnly));
        s.switch("nitin", None).unwrap();
        assert_eq!(
            s.remove("nitin"),
            Err(Error::Invalid("the owner stays".into()))
        );
        s.remove("mia").unwrap();
        assert_eq!(
            s.index.order,
            ["nitin", "isabel", "mia2", "options2", "six"]
        );
        assert!(!r.p().join("profiles/mia").exists());
        assert!(r.p().join("profiles/isabel").exists());
        assert_eq!(s.remove("mia"), Err(Error::NoProfile));
        assert!(s.index_valid_on_disk());
    }

    impl Store {
        fn index_valid_on_disk(&self) -> bool {
            read_json(&self.index_file())
                .and_then(|v| serde_json::from_value::<Index>(v).ok())
                .is_some_and(|i| i.order == self.index.order && i.active == self.index.active)
        }
    }

    #[test]
    fn removing_the_active_profile_falls_back_to_the_owner_and_start_with_forgets_it() {
        let r = rig();
        let mut s = r.open();
        s.set_options(&json!({"startWith": "isabel", "pickerOnWake": true}))
            .unwrap();
        s.switch("isabel", None).unwrap();
        s.switch("nitin", None).unwrap();
        s.index.active = "isabel".into(); // as if she were the one in front
        s.index.work.id = "isabel".into();
        s.index.active = "nitin".into();
        s.remove("isabel").unwrap();
        assert_eq!(s.index.start_with, "last");
        assert!(s.index.picker_on_wake);
        assert_eq!(s.index.active, "nitin");
    }

    #[test]
    fn a_profile_may_change_its_own_name_colour_and_picture_and_the_owner_any() {
        let r = rig();
        let mut s = r.open();
        s.switch("isabel", None).unwrap();
        s.update("isabel", &json!({"name": "  Isa  bel ", "color": "#9CCFD8", "avatar": {"kind": "gallery", "id": "lorelei-01"}, "firstRun": false})).unwrap();
        let rec = &s.records["isabel"];
        assert_eq!(
            (rec.name.as_str(), rec.color.as_str(), rec.first_run),
            ("Isa bel", "#9CCFD8", false)
        );
        assert_eq!(
            rec.avatar,
            Avatar::Gallery {
                id: "lorelei-01".into()
            }
        );
        assert_eq!(
            s.update("nitin", &json!({"name": "X"})),
            Err(Error::OwnerOnly)
        );
        assert!(matches!(
            s.update("isabel", &json!({"name": ""})),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            s.update("isabel", &json!({"name": 5})),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            s.update("isabel", &json!({"color": "blue"})),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            s.update(
                "isabel",
                &json!({"avatar": {"kind": "gallery", "id": "../x"}})
            ),
            Err(Error::Invalid(_))
        ));
        assert!(
            matches!(
                s.update(
                    "isabel",
                    &json!({"avatar": {"kind": "photo", "file": "avatar-1.png"}})
                ),
                Err(Error::Invalid(_))
            ),
            "a photo that is not there"
        );
        assert_eq!(s.update("ghost", &json!({})), Err(Error::NoProfile));
        assert_eq!(
            read_json(&r.p().join("profiles/isabel/profile.json")).unwrap()["name"],
            "Isa bel"
        );
        s.switch("nitin", None).unwrap();
        s.update("isabel", &json!({"name": "Isabel"})).unwrap();
        assert_eq!(s.records["isabel"].name, "Isabel");
    }

    #[test]
    fn a_photo_is_a_png_or_jpeg_of_a_few_kilobytes_kept_under_a_new_name_each_time() {
        let r = rig();
        let mut s = r.open();
        s.set_photo("nitin", PNG).unwrap();
        let first = match &s.records["nitin"].avatar {
            Avatar::Photo { file } => file.clone(),
            other => panic!("{other:?}"),
        };
        assert!(file_is_photo(&first) && r.p().join("profiles/nitin").join(&first).is_file());
        assert_eq!(s.view()["profiles"][0]["avatar"]["kind"], "photo");
        assert_eq!(
            s.view()["profiles"][0]["avatarFile"],
            r.p()
                .join("profiles/nitin")
                .join(&first)
                .display()
                .to_string()
        );
        let jpeg = [0xff, 0xd8, 0xff, 0xe0, 1, 2, 3];
        std::thread::sleep(Duration::from_millis(1100));
        s.set_photo("nitin", &jpeg).unwrap();
        let second = match &s.records["nitin"].avatar {
            Avatar::Photo { file } => file.clone(),
            other => panic!("{other:?}"),
        };
        assert!(second.ends_with(".jpg") && second != first);
        assert!(
            !r.p().join("profiles/nitin").join(&first).exists(),
            "the old photo is not kept"
        );
        assert!(matches!(
            s.set_photo("nitin", b"GIF89a"),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(s.set_photo("nitin", &[]), Err(Error::Invalid(_))));
        assert!(matches!(
            s.set_photo("nitin", &vec![0xff; PHOTO_MAX + 1]),
            Err(Error::Invalid(_))
        ));
        assert_eq!(s.set_photo("ghost", PNG), Err(Error::NoProfile));
        s.switch("isabel", None).unwrap();
        assert_eq!(s.set_photo("nitin", PNG), Err(Error::OwnerOnly));
        // a photo file that is gone is the letter again at the next open
        drop(s);
        fs::remove_file(r.p().join("profiles/nitin").join(&second)).unwrap();
        assert_eq!(r.open().records["nitin"].avatar, Avatar::Initial);
    }

    #[test]
    fn the_gallery_lists_only_files_that_are_there_and_names_that_are_plain() {
        let r = rig();
        let s = r.open();
        let ids: Vec<String> = s.gallery().into_iter().map(|g| g.id).collect();
        assert_eq!(ids, ["lorelei-01", "thumbs-01"]);
        fs::remove_file(r.p().join("avatars/gallery.json")).unwrap();
        assert!(s.gallery().is_empty());
    }

    #[test]
    fn the_options_are_the_owners_and_start_with_must_be_a_profile() {
        let r = rig();
        let mut s = r.open();
        s.set_options(&json!({"pickerOnWake": true})).unwrap();
        assert!(s.index.picker_on_wake);
        assert!(matches!(
            s.set_options(&json!({"pickerOnWake": "yes"})),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            s.set_options(&json!({"startWith": "ghost"})),
            Err(Error::Invalid(_))
        ));
        s.set_options(&json!({"startWith": "isabel"})).unwrap();
        assert_eq!(s.view()["startWith"], "isabel");
        s.switch("isabel", None).unwrap();
        assert_eq!(
            s.set_options(&json!({"pickerOnWake": false})),
            Err(Error::OwnerOnly)
        );
        assert_eq!(r.index()["pickerOnWake"], true);
    }

    #[test]
    fn the_view_says_who_is_who_and_can_admin_follows_the_owner() {
        let r = rig();
        let mut s = r.open();
        let v = s.view();
        assert_eq!(
            (
                v["active"].as_str(),
                v["owner"].as_str(),
                v["canAdmin"].as_bool()
            ),
            (Some("nitin"), Some("nitin"), Some(true))
        );
        assert_eq!(v["profiles"][0]["initial"], "N");
        assert_eq!(v["profiles"][0]["owner"], true);
        assert_eq!(v["profiles"][1]["owner"], false);
        assert_eq!(v["profiles"][1]["firstRun"], true);
        assert_eq!(v["pickerOnWake"], false);
        assert_eq!(v["startWith"], "last");
        assert_eq!(v["max"], MAX_PROFILES);
        assert!(
            v["profiles"][0]["avatarFile"]
                .as_str()
                .unwrap()
                .ends_with("lorelei-01.png")
        );
        s.switch("isabel", None).unwrap();
        let v = s.view();
        assert_eq!(
            (v["active"].as_str(), v["canAdmin"].as_bool()),
            (Some("isabel"), Some(false))
        );
        assert!(!s.can_admin() && !s.owner_active());
    }

    #[test]
    fn a_switch_of_a_realistic_home_is_quick() {
        let r = rig();
        // a few hundred keys, as a well-used theme.json is
        let mut t = legacy();
        for n in 0..300 {
            t["apps"]["names"][format!("com.example.app{n}")] = json!(format!("App {n}"));
            t["tiles"]["plates"][format!("com.example.app{n}")] = json!("#223344");
        }
        r.set_theme(&t);
        let mut s = r.open();
        let mut best = Duration::MAX;
        for _ in 0..5 {
            let started = Instant::now();
            s.switch("isabel", None).unwrap();
            s.switch("nitin", None).unwrap();
            best = best.min(started.elapsed() / 2);
        }
        eprintln!(
            "profiles: a switch of a {} byte theme takes {best:?} (debug build, host disk)",
            serde_json::to_string(&t).unwrap().len()
        );
        assert!(best < Duration::from_millis(100), "{best:?}");
    }

    // ---- the routes
    fn profiles(r: &Rig) -> Profiles {
        Profiles {
            store: Mutex::new(r.open()),
            hub: Arc::new(Hub::new(1)),
        }
    }

    fn req(method: &str, path: &str, body: &str) -> Request {
        Request {
            method: method.into(),
            path: path.into(),
            query: vec![],
            headers: vec![],
            body: body.as_bytes().to_vec(),
            peer: "127.0.0.1".parse().unwrap(),
        }
    }

    fn call(p: &Profiles, method: &str, path: &str, body: &str) -> (u16, Value) {
        let trimmed = path.trim_start_matches("/v1/profiles").trim_matches('/');
        let rest: Vec<&str> = if trimmed.is_empty() {
            vec![]
        } else {
            trimmed.split('/').collect()
        };
        let r = p.handle(&req(method, path, body), &rest);
        (r.status, serde_json::from_slice(&r.body).unwrap())
    }

    #[test]
    fn the_routes_switch_create_remove_and_tell_the_topic() {
        let r = rig();
        let p = profiles(&r);
        let (s, v) = call(&p, "GET", "/v1/profiles", "");
        assert_eq!((s, v["profiles"]["active"].as_str()), (200, Some("nitin")));
        let (s, v) = call(&p, "POST", "/v1/profiles/switch", r#"{"id": "isabel"}"#);
        assert_eq!(
            (s, v["profiles"]["active"].as_str(), v["ok"].as_bool()),
            (200, Some("isabel"), Some(true))
        );
        assert!(v["ms"].as_f64().unwrap() < 250.0);
        assert_eq!(
            p.hub.get(TOPIC).unwrap()["active"],
            "isabel",
            "the topic follows at once"
        );
        let (s, v) = call(&p, "POST", "/v1/profiles", r#"{"name": "Mia"}"#);
        assert_eq!((s, v["error"]["code"].as_str()), (403, Some("owner_only")));
        let (s, _) = call(&p, "POST", "/v1/profiles/switch", r#"{"id": "nitin"}"#);
        assert_eq!(s, 200);
        let (s, v) = call(
            &p,
            "POST",
            "/v1/profiles",
            r##"{"name": "Mia", "color": "#EB6F92"}"##,
        );
        assert_eq!((s, v["id"].as_str()), (200, Some("mia")));
        assert_eq!(
            p.hub.get(TOPIC).unwrap()["profiles"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        let (s, _) = call(&p, "POST", "/v1/profiles/mia", r#"{"name": "Mimi"}"#);
        assert_eq!(s, 200);
        let (s, v) = call(&p, "DELETE", "/v1/profiles/mia", "");
        assert_eq!(
            (s, v["profiles"]["profiles"].as_array().unwrap().len()),
            (200, 2)
        );
        let (s, v) = call(&p, "DELETE", "/v1/profiles/nitin", "");
        assert_eq!((s, v["error"]["code"].as_str()), (400, Some("bad_request")));
        let (s, v) = call(&p, "DELETE", "/v1/profiles/ghost", "");
        assert_eq!((s, v["error"]["code"].as_str()), (404, Some("no_profile")));
        let (s, v) = call(
            &p,
            "POST",
            "/v1/profiles/options",
            r#"{"pickerOnWake": true}"#,
        );
        assert_eq!(
            (s, v["profiles"]["pickerOnWake"].as_bool()),
            (200, Some(true))
        );
    }

    #[test]
    fn the_routes_answer_pins_with_statuses_a_screen_can_tell_apart() {
        let r = rig();
        let p = profiles(&r);
        let (s, _) = call(&p, "POST", "/v1/profiles/nitin/pin", r#"{"pin": "2468"}"#);
        assert_eq!(s, 200);
        call(&p, "POST", "/v1/profiles/switch", r#"{"id": "isabel"}"#);
        let (s, v) = call(&p, "POST", "/v1/profiles/switch", r#"{"id": "nitin"}"#);
        assert_eq!(
            (s, v["error"]["code"].as_str()),
            (403, Some("pin_required"))
        );
        let (s, v) = call(
            &p,
            "POST",
            "/v1/profiles/switch",
            r#"{"id": "nitin", "pin": "1111"}"#,
        );
        assert_eq!(
            (
                s,
                v["error"]["code"].as_str(),
                v["error"]["triesLeft"].as_u64()
            ),
            (403, Some("wrong_pin"), Some(4))
        );
        for pin in ["1111", "2222", "3333"] {
            call(
                &p,
                "POST",
                "/v1/profiles/switch",
                &format!(r#"{{"id": "nitin", "pin": "{pin}"}}"#),
            );
        }
        let r5 = p.handle(
            &req(
                "POST",
                "/v1/profiles/switch",
                r#"{"id": "nitin", "pin": "4444"}"#,
            ),
            &["switch"],
        );
        assert_eq!(r5.status, 429);
        assert!(
            r5.headers
                .iter()
                .any(|(n, v)| *n == "Retry-After" && v == "30")
        );
        let body: Value = serde_json::from_slice(&r5.body).unwrap();
        assert_eq!(
            (
                body["error"]["code"].as_str(),
                body["error"]["retryAfterS"].as_u64()
            ),
            (Some("pin_locked"), Some(30))
        );
        let topic = p.hub.get(TOPIC).unwrap().to_string();
        assert!(!topic.contains("salt") && !topic.contains("hash") && !topic.contains("2468"));
        let (s, v) = call(
            &p,
            "POST",
            "/v1/profiles/isabel/pin",
            r#"{"pin": "1234", "clear": true}"#,
        );
        assert_eq!((s, v["error"]["code"].as_str()), (400, Some("bad_request")));
        let (s, _) = call(&p, "POST", "/v1/profiles/isabel/pin", r#"{}"#);
        assert_eq!(s, 400);
    }

    #[test]
    fn the_routes_take_a_photo_as_base64_and_refuse_what_is_not_one() {
        let r = rig();
        let p = profiles(&r);
        let png = "iVBORw0KGgoAAAAA"; // the PNG signature and four zero bytes
        let (s, v) = call(
            &p,
            "POST",
            "/v1/profiles/nitin/avatar",
            &format!(r#"{{"image": "{png}"}}"#),
        );
        assert_eq!(
            (s, v["profiles"]["profiles"][0]["avatar"]["kind"].as_str()),
            (200, Some("photo"))
        );
        let (s, _) = call(
            &p,
            "POST",
            "/v1/profiles/nitin/avatar",
            r#"{"image": "not base64!"}"#,
        );
        assert_eq!(s, 400);
        let (s, _) = call(
            &p,
            "POST",
            "/v1/profiles/nitin/avatar",
            r#"{"image": "R0lGODlh"}"#,
        );
        assert_eq!(s, 400);
        let (s, _) = call(&p, "POST", "/v1/profiles/nitin/avatar", r#"{}"#);
        assert_eq!(s, 400);
        assert_eq!(base64("aGk=").unwrap(), b"hi");
        assert_eq!(base64("aGk").unwrap(), b"hi");
        assert_eq!(base64("a G\nk=").unwrap(), b"hi");
        assert!(base64("a*k=").is_none());
    }

    #[test]
    fn the_routes_have_the_right_methods_and_unknown_ones_are_not_found() {
        let r = rig();
        let p = profiles(&r);
        assert_eq!(
            call(&p, "GET", "/v1/profiles/gallery", "").1["gallery"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(call(&p, "POST", "/v1/profiles/gallery", "").0, 405);
        assert_eq!(call(&p, "GET", "/v1/profiles/switch", "").0, 405);
        assert_eq!(call(&p, "PUT", "/v1/profiles", "").0, 405);
        assert_eq!(call(&p, "GET", "/v1/profiles/nitin", "").0, 405);
        assert_eq!(call(&p, "GET", "/v1/profiles/nitin/pin", "").0, 405);
        assert_eq!(call(&p, "POST", "/v1/profiles/a/b/c", "").0, 404);
        assert_eq!(call(&p, "POST", "/v1/profiles/switch", "{").0, 400);
        assert_eq!(call(&p, "POST", "/v1/profiles/switch", "[]").0, 400);
        assert_eq!(
            call(&p, "POST", "/v1/profiles/switch", r#"{"id": 5}"#).0,
            400
        );
        assert_eq!(
            call(&p, "POST", "/v1/profiles/switch", r#"{"id": "ghost"}"#).0,
            404
        );
        for (method, path, _) in ROUTES {
            let concrete = path.replace("{id}", "nitin");
            assert_ne!(call(&p, method, &concrete, "{}").0, 404, "{method} {path}");
        }
    }

    #[test]
    fn another_modules_rule_is_a_call_away() {
        let r = rig();
        let p = profiles(&r);
        assert!(p.owner_active() && p.can_admin() && p.require_owner().is_ok());
        assert_eq!(p.active_id(), "nitin");
        call(&p, "POST", "/v1/profiles/switch", r#"{"id": "isabel"}"#);
        assert!(!p.can_admin());
        let refused = p.require_owner().unwrap_err();
        assert_eq!(refused.status, 403);
        assert_eq!(
            serde_json::from_slice::<Value>(&refused.body).unwrap()["error"]["code"],
            "owner_only"
        );
    }

    #[test]
    fn the_sync_thread_carries_a_late_edit_back_without_a_request() {
        let r = rig();
        let p = Arc::new(profiles(&r));
        let mut t = r.theme();
        t["accent"] = json!("#00AA00");
        r.set_theme(&t);
        p.store.lock().sync();
        p.publish();
        assert_eq!(r.own("nitin")["accent"], "#00AA00");
    }
}
