//! Guide data: the Live TV guide filled from online listings for a zip code (the `lumaguide` engine), as a plugin of the helper's own
//! that asks first (`plugins/local.rs`'s `OptIn`).
//!
//! **A plugin of the helper's own, not a program of its own.** The listings come from the internet (Schedules Direct, the Gracenote grid)
//! and from a Plex server on the home network, and what they make is laid over LG's guide in the TV's guide and channel banner and in the
//! tuner's `/xmltv.xml`. A plugin on the socket has no network at this TV's plugin level, so the engine runs in the helper and the plugin
//! is the helper speaking the plugin API: listed with the other plugins, its page drawn like theirs, switched on by an approval of its
//! words.
//!
//! **What it may reach** ([`Net`], the base's rule for it): the internet service of each source the owner picked, by name and over
//! HTTPS only (`tvlistings.gracenote.com`, `json.schedulesdirect.org`), and the Plex account's server, over plain http and only when it
//! is an address of the home network (`ha_link::AddressPolicy::HOME`, the rule the Home Assistant lease is held to).
//! Whatever the engine asks beyond that is refused before anything is sent.
//!
//! **Secrets.** The Schedules Direct password is a secret field of the page: Settings hands it over once (`secret.submit`), and the
//! plugin keeps it in `guide/secrets.json` (mode 0600) and nowhere else; the page says only whether one is kept.
//!
//! **The Plex server** is the TV's Plex account's (`plex.rs`): the server it signed in with and that server's access token. Nothing of
//! Plex is typed on this page; it says which server it uses. What the page kept before the account is the account's on the first start
//! ([`plex_kept`], [`forget_plex`]).
//!
//! **Off** asks nothing and keeps no listings: switching off forgets them (the page's settings and secrets stay). **On**, the TV's
//! antenna channels are read at the start and hourly (one luna call), the engine refreshes each source at most every six hours, and
//! the page's facts say how each stands. `GET /v1/guide/programmes` serves the online programmes to the TV's guide and banner, which lay
//! them over LG's.

use crate::config::{FetchConfig, Paths};
use crate::ctx::Ctx;
use crate::fetcher::{CurlFetch, Exchange};
use crate::http::{Request as HttpRequest, Response as HttpResponse, error_response};
use crate::plugins::firstparty::{clip, clock_words, local_time};
use crate::plugins::local::{Handler, Local, OptIn, Spec};
use crate::plugins::session::{MAX_FACT_LINE, Shared};
use crate::plex::{PlexAccount, Server};
use crate::settings::SettingsStore;
use crate::timeutil::now_ms;
use fetch::{FetchError, FetchLimits};
use lumaguide::net::Origin;
use lumaguide::time::{HOUR_MS, days_from_civil};
use lumaguide::{
    Channel, DirStore, Guide, Http, Listings, NetError, Programme, Request, Response, Settings,
    Source, SourceSettings, Store,
};
use lumatuner::Hardware;
use lumatuner::docs::Entry;
use parking_lot::{Mutex, RwLock};
use plugin_api::wire::ToPlugin;
use serde_json::{Map, Value as Json, json};
use settings_schema::{Value as Setting, Values};
use std::collections::BTreeMap;
use std::io::Write;
use std::net::{IpAddr, SocketAddrV4};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The plugin's id, which is also its settings owner.
pub const ID: &str = "guide-data";
pub const OPT_IN: OptIn = OptIn {
    spec: Spec {
        id: ID,
        name: "Guide data",
        version: "1",
        permissions: &["settings"],
    },
    consent: "Fills the Live TV guide with a week of listings for a zip code, from the sources picked on its page: a Plex server on the \
              home network, Schedules Direct (a paid account) or the free Gracenote listings. The TV asks only those (the Plex server's \
              address, json.schedulesdirect.org, tvlistings.gracenote.com), at most every six hours, and keeps the listings, the Plex \
              token and the Schedules Direct password on the TV.",
};

const ZIP: &str = "zip";
const SOURCES: &str = "sources";
const SD_USERNAME: &str = "sd.username";
const SD_PASSWORD: &str = "sd.password";
/// The page's action: ask the sources now (the engine still waits an hour after the last try).
const REFRESH: &str = "refresh";
/// What the page kept of a Plex server before the Plex account: an address it typed, a token in its secrets.
const PLEX_ADDRESS: &str = "plex.address";
const PLEX_TOKEN: &str = "plex.token";
/// Plex's own port, when an address was typed without one.
const PLEX_PORT: u16 = 32400;
/// What the page says, as the status and as the Plex source's line, while Plex is picked and the TV is not signed in to Plex.
const SIGN_IN: &str = "Sign in to Plex for listings";
/// How often the TV's channel list is read again, and again soon when it could not be read.
const CHANNELS_EVERY: Duration = Duration::from_secs(3600);
const CHANNELS_RETRY: Duration = Duration::from_secs(300);
/// The longest the thread sleeps, so that a stop and a clock that jumped are noticed.
const LONGEST_SLEEP: Duration = Duration::from_secs(15 * 60);

// -- what readers share ----------------------------------------------------------------------------------------------------------

/// The listings while the plugin is on: what the API serves the TV's guide and banner and what the tuner's `/xmltv.xml` is made with.
#[derive(Default)]
pub struct GuideData {
    listings: RwLock<Option<Arc<Listings>>>,
    revision: AtomicU64,
}

impl GuideData {
    /// The listings, `None` while the plugin is off.
    pub fn listings(&self) -> Option<Arc<Listings>> {
        self.listings.read().clone()
    }

    fn set(&self, listings: Option<Arc<Listings>>) {
        let mut held = self.listings.write();
        let same = match (&*held, &listings) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            *held = listings;
            self.revision.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl lumatuner::Guide for GuideData {
    /// The TV's own listings with the online ones over them, while the plugin is on and has some.
    fn xmltv(&self, channels: &[(Entry, Json)]) -> Option<String> {
        let listings = self.listings().filter(|l| !l.is_empty())?;
        let merged: Vec<Vec<Programme>> = channels
            .iter()
            .map(|(e, reply)| {
                listings.over(
                    &e.channel_id,
                    &broadcast(reply, &e.channel_id),
                    i64::MIN,
                    i64::MAX,
                )
            })
            .collect();
        let documents: Vec<lumaguide::xmltv::Listing<'_>> = channels
            .iter()
            .zip(&merged)
            .map(|((e, _), programmes)| lumaguide::xmltv::Listing {
                id: &e.guide,
                name: &e.name,
                programmes,
            })
            .collect();
        Some(lumaguide::xmltv::document(&documents))
    }

    fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }
}

/// The programmes of an `iepg/getEventList` reply for `channel_id`, by start: its UTC times, a title, one per start.
fn broadcast(reply: &Json, channel_id: &str) -> Vec<Programme> {
    let text = |p: &Json, keys: &[&str]| {
        keys.iter()
            .filter_map(|k| p[*k].as_str())
            .map(str::trim)
            .find(|t| !t.is_empty())
            .unwrap_or("")
            .to_owned()
    };
    let mut out: Vec<Programme> = reply["programList"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["channelId"].as_str().is_none_or(|id| id == channel_id))
        .filter_map(|p| {
            let (start, end) = (
                lg_utc(p["startTime"].as_str()?)?,
                lg_utc(p["endTime"].as_str()?)?,
            );
            let title = text(p, &["programName", "programMainTitle"]);
            (end > start && !title.is_empty()).then(|| Programme {
                id: text(p, &["programId"]),
                channel: channel_id.to_owned(),
                title,
                subtitle: text(p, &["programSubTitle", "secondaryTitle"]),
                text: text(p, &["description"]),
                start,
                end,
                rating: String::new(),
                captions: p["isSubtitle"].as_bool() == Some(true),
                described: false,
            })
        })
        .collect();
    out.sort_by_key(|p| p.start);
    out.dedup_by_key(|p| p.start);
    out
}

/// LG's `YYYY,MM,DD,hh,mm,ss` on the UTC clock, as epoch milliseconds.
fn lg_utc(text: &str) -> Option<i64> {
    let fields: Vec<i64> = text
        .split(',')
        .map(|f| f.trim().parse().ok())
        .collect::<Option<_>>()?;
    let [y, mo, d, h, mi, s] = fields[..] else {
        return None;
    };
    let fits = (1..=12).contains(&mo)
        && (1..=31).contains(&d)
        && (0..24).contains(&h)
        && (0..60).contains(&mi)
        && (0..=60).contains(&s);
    fits.then(|| ((days_from_civil(y, mo as u32, d as u32) * 24 + h) * 60 + mi) * 60_000 + s * 1000)
}

pub const ROUTES: &[(&str, &str, &str)] = &[(
    "GET",
    "/v1/guide/programmes",
    "the online programmes of channels while the Guide data plugin is on: ?channel=<LG id>&channel=...&from=<ms>&to=<ms>",
)];

/// `/v1/guide/...`; `rest` is the path after `guide`.
pub fn handle(data: &GuideData, request: &HttpRequest, rest: &[&str]) -> HttpResponse {
    match rest {
        ["programmes"] if request.method == "GET" => {
            let number = |name: &str| request.param(name).and_then(|v| v.parse::<i64>().ok());
            let (Some(from), Some(to)) = (number("from"), number("to")) else {
                return error_response(400, "bad_request", "from and to are epoch milliseconds");
            };
            if to <= from || to - from > 8 * 24 * HOUR_MS {
                return error_response(
                    400,
                    "bad_request",
                    "from comes before to, at most eight days apart",
                );
            }
            let listings = data.listings();
            let programmes: Vec<&Programme> = match &listings {
                Some(l) => request
                    .query
                    .iter()
                    .filter(|(k, _)| k == "channel")
                    .take(64)
                    .flat_map(|(_, id)| l.programmes(id, from, to))
                    .collect(),
                None => Vec::new(),
            };
            HttpResponse::json(
                200,
                &json!({"ok": true, "api": crate::API_VERSION, "on": listings.is_some(), "programmes": programmes}),
            )
        }
        _ => error_response(404, "not_found", "no such guide route"),
    }
}

// -- the network, as the base allows it ------------------------------------------------------------------------------------------

/// The guide's way out: only to the origins its settings name (see the module), through the set's curl, the TLS child run as the
/// unprivileged fetch user when the helper is root (and as the helper's when the system refuses that, once and from then on).
pub struct Net {
    allowed: Vec<Origin>,
    curl: CurlFetch,
    as_helper: CurlFetch,
    refused: AtomicBool,
}

impl Net {
    pub fn new(config: &FetchConfig, settings: &Settings) -> Self {
        Self {
            allowed: allowed(settings),
            curl: CurlFetch::of(config, true),
            as_helper: CurlFetch::of(config, false),
            refused: AtomicBool::new(false),
        }
    }
}

/// The origins `settings` may reach.
fn allowed(settings: &Settings) -> Vec<Origin> {
    settings
        .sources
        .iter()
        .filter_map(|s| match (s, s.source().host()) {
            (SourceSettings::Plex { address, .. }, _) => plex_origin(address),
            (_, Some(host)) => Some(Origin {
                https: true,
                host: host.to_owned(),
                port: 443,
            }),
            _ => None,
        })
        .collect()
}

/// The Plex server's origin: an IPv4 address of the home network and a port, plain http.
fn plex_origin(address: &str) -> Option<Origin> {
    let addr: SocketAddrV4 = address.parse().ok()?;
    ha_link::AddressPolicy::HOME
        .allows(IpAddr::V4(*addr.ip()))
        .then(|| Origin {
            https: false,
            host: addr.ip().to_string(),
            port: addr.port(),
        })
}

impl Http for Net {
    fn send(&self, r: &Request<'_>) -> Result<Response, NetError> {
        let origin = r
            .origin()
            .ok_or_else(|| NetError::NotAllowed("that is not a web address".into()))?;
        if !self.allowed.contains(&origin) {
            return Err(NetError::NotAllowed(format!(
                "the guide may not reach {}:{}",
                origin.host, origin.port
            )));
        }
        let x = Exchange {
            method: r.method.as_str(),
            url: r.url,
            headers: &r.headers,
            body: r.body,
        };
        let limits = FetchLimits {
            total: r.timeout,
            max_bytes: r.max_bytes,
        };
        let answer = if self.refused.load(Ordering::Relaxed) {
            self.as_helper.exchange(&x, limits)
        } else {
            match self.curl.exchange(&x, limits) {
                Err(FetchError::Denied(why)) => {
                    self.refused.store(true, Ordering::Relaxed);
                    crate::log_warn!(
                        "guide: the system refused curl as the fetch user ({why}); it runs as the helper's from now on"
                    );
                    self.as_helper.exchange(&x, limits)
                }
                other => other,
            }
        };
        match answer {
            Ok((status, body)) => Ok(Response { status, body }),
            Err(FetchError::Offline) => Err(NetError::Offline),
            Err(FetchError::Timeout) => Err(NetError::Timeout),
            Err(FetchError::TooLarge) => Err(NetError::TooLarge),
            Err(e) => Err(NetError::Failed(e.to_string())),
        }
    }
}

// -- the page's secrets ----------------------------------------------------------------------------------------------------------

/// The page's secrets in one file, mode 0600, replaced whole; never in a log, a fact or a setting.
pub struct Secrets {
    path: PathBuf,
    kept: Mutex<BTreeMap<String, String>>,
}

impl Secrets {
    pub fn open(path: PathBuf) -> Self {
        let kept = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self {
            path,
            kept: Mutex::new(kept),
        }
    }

    fn get(&self, field: &str) -> String {
        self.kept.lock().get(field).cloned().unwrap_or_default()
    }

    fn has(&self, field: &str) -> bool {
        self.kept.lock().contains_key(field)
    }

    /// Keeps what was typed in `field`; nothing typed forgets it.
    fn put(&self, field: &str, secret: &str) -> Result<(), String> {
        let secret = secret.trim();
        match field {
            SD_PASSWORD if secret.chars().any(char::is_control) => {
                return Err("that password has a character it cannot have".into());
            }
            SD_PASSWORD => {}
            _ => return Err("this page has no such field".into()),
        }
        let mut kept = self.kept.lock();
        let before = kept.clone();
        if secret.is_empty() {
            kept.remove(field);
        } else {
            kept.insert(field.to_owned(), secret.to_owned());
        }
        if let Err(e) = write_private(&self.path, &serde_json::to_vec(&*kept).unwrap_or_default()) {
            *kept = before;
            return Err(format!("it could not be kept ({e})"));
        }
        Ok(())
    }

    /// Forgets `field`, a secret the page no longer has.
    fn forget(&self, field: &str) -> std::io::Result<()> {
        let mut kept = self.kept.lock();
        if kept.remove(field).is_some() {
            write_private(&self.path, &serde_json::to_vec(&*kept).unwrap_or_default())?;
        }
        Ok(())
    }
}

/// `bytes` as the file at `path`, mode 0600, whole: written beside it, then renamed.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("new");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(&tmp, path)
}

// -- the settings ----------------------------------------------------------------------------------------------------------------

fn text_of(values: &Values, key: &str) -> String {
    match values.get(key) {
        Some(Setting::Text(t)) => t.trim().to_owned(),
        _ => String::new(),
    }
}

/// A typed Plex address as `ip:port`: a scheme and a trailing slash dropped, Plex's port added when there is none.
fn plex_address(typed: &str) -> String {
    let bare = typed
        .trim()
        .trim_start_matches("http://")
        .trim_end_matches('/');
    if bare.is_empty() || bare.contains(':') {
        bare.to_owned()
    } else {
        format!("{bare}:{PLEX_PORT}")
    }
}

/// The plugin's files: its secrets and the listings it keeps.
fn dir_of(paths: &Paths) -> PathBuf {
    paths.data_dir.join("guide")
}

/// The Plex server the page kept before the Plex account (`plex::migrate`): its address as `ip:port` and its token, when it has both.
pub fn plex_kept(paths: &Paths, settings: &SettingsStore) -> Option<(String, String)> {
    let address = match settings.stored(ID)?.get(PLEX_ADDRESS)? {
        Setting::Text(a) if !a.trim().is_empty() => plex_address(a),
        _ => return None,
    };
    let token = Secrets::open(dir_of(paths).join("secrets.json")).get(PLEX_TOKEN);
    (!token.is_empty()).then_some((address, token))
}

/// Forgets the Plex server the page kept before the Plex account: the address in its settings and the token in its secrets.
pub fn forget_plex(paths: &Paths, settings: &SettingsStore) {
    if let Err(e) = settings.take_keys(ID, &[PLEX_ADDRESS]) {
        crate::log_warn!("guide: the Plex address kept before the Plex account could not be forgotten: {e}");
    }
    if let Err(e) = Secrets::open(dir_of(paths).join("secrets.json")).forget(PLEX_TOKEN) {
        crate::log_warn!("guide: the Plex token kept before the Plex account could not be forgotten: {e}");
    }
}

/// The engine's settings from the page's values and secrets and the Plex account's server: the sources in the order picked.
fn settings_of(values: &Values, secrets: &Secrets, plex: Option<&Server>) -> Settings {
    let order: Vec<String> = match values.get(SOURCES) {
        Some(Setting::Ordered(ids)) => ids.iter().map(ToString::to_string).collect(),
        _ => Vec::new(),
    };
    let mut sources = Vec::new();
    for id in order {
        match Source::from_id(&id) {
            Some(Source::Plex) => sources.push(SourceSettings::Plex {
                address: plex.map(|s| s.address.clone()).unwrap_or_default(),
                token: plex.map(|s| s.token().to_owned()).unwrap_or_default(),
            }),
            Some(Source::SchedulesDirect) => sources.push(SourceSettings::SchedulesDirect {
                username: text_of(values, SD_USERNAME),
                password: secrets.get(SD_PASSWORD),
            }),
            Some(Source::Gracenote) => sources.push(SourceSettings::Gracenote),
            None => {}
        }
    }
    Settings {
        zip: text_of(values, ZIP),
        sources,
    }
}

/// The page, as the schema format writes it.
fn schema_json() -> Json {
    let fact = |key: &str, label: &str| {
        json!({"key": key, "label": label, "kind": {"type": "read-only", "value": {"capability": format!("plugin.{ID}.{key}")}},
               "apply": "on-commit"})
    };
    let text = |key: &str, label: &str, help: &str, max: u16| {
        json!({"key": key, "label": label, "help": help, "kind": {"type": "text", "max_chars": max, "secret": false},
               "default": {"type": "text", "value": ""}, "apply": "on-commit"})
    };
    let secret = |key: &str, label: &str, help: &str, max: u16| json!({"key": key, "label": label, "help": help, "kind": {"type": "text", "max_chars": max, "secret": true}, "apply": "on-commit"});
    let options: Vec<Json> = Source::ALL
        .iter()
        .map(|s| json!({"id": s.id(), "label": s.label()}))
        .collect();
    json!({"format": 1, "owner": ID, "revision": 2, "title": OPT_IN.spec.name, "sections": [
        {"id": "listings", "title": "Listings", "fields": [
            fact("status", "Status"),
            text(ZIP, "Zip code", "The US zip code whose antenna listings the guide takes, five digits.", 5),
            {"key": SOURCES, "label": "Sources", "help": "In the order they are trusted: the first that has a programme for a time wins \
                it, and the TV's own listings fill what none has.",
             "kind": {"type": "ordered-set", "options": {"inline": options}, "max_items": 3},
             "default": {"type": "ordered", "value": []}, "apply": "on-commit"}]},
        {"id": "plex", "title": "Plex", "fields": [
            fact("plex", "Listings"),
            fact("plex-server", "Server")]},
        {"id": "schedules-direct", "title": "Schedules Direct", "fields": [
            fact("schedules-direct", "Listings"),
            text(SD_USERNAME, "User name", "The Schedules Direct account's user name (schedulesdirect.org, about $35 a year).", 64),
            secret(SD_PASSWORD, "Password", "It stays on this TV; Schedules Direct is sent only its SHA-1.", 128)]},
        {"id": "gracenote", "title": "Free listings", "fields": [fact("gracenote", "Listings")]}]})
}

fn register() -> Json {
    json!({"title": OPT_IN.spec.name, "settings": schema_json(), "actions": [{"key": REFRESH, "label": "Refresh now"}]})
}

// -- the plugin ------------------------------------------------------------------------------------------------------------------

/// Makes the network for a set of settings: [`Net`] on the TV, a fake in a test.
pub type MakeNet = Arc<dyn Fn(&Settings) -> Arc<dyn Http> + Send + Sync>;

/// What the plugin's thread is told.
enum Event {
    /// The owner's switch may have changed: [`Shared::switched_on`] says how it stands.
    Switch,
    Settings(Values),
    /// A secret was kept or forgotten.
    Secret,
    /// The Plex account's server changed.
    Plex,
    Refresh,
    Stop,
}

/// Offers the plugin (off until the owner switches it on) and starts its thread. `hw` is the TV whose channel list the listings are
/// matched to; `plex` the TV's Plex account, whose server is the Plex source's.
pub fn spawn(
    ctx: &Ctx,
    shared: Arc<Shared>,
    hw: Arc<dyn Hardware>,
    data: Arc<GuideData>,
    plex: Arc<PlexAccount>,
) -> std::io::Result<JoinHandle<()>> {
    let fetch = ctx.config.fetch.clone();
    spawn_with(
        ctx,
        shared,
        hw,
        data,
        plex,
        Arc::new(move |s: &Settings| Arc::new(Net::new(&fetch, s)) as Arc<dyn Http>),
    )
}

fn spawn_with(
    ctx: &Ctx,
    shared: Arc<Shared>,
    hw: Arc<dyn Hardware>,
    data: Arc<GuideData>,
    plex: Arc<PlexAccount>,
    net: MakeNet,
) -> std::io::Result<JoinHandle<()>> {
    let (tx, rx) = mpsc::channel();
    {
        let tx = tx.clone();
        ctx.stop.on_stop(move || {
            let _ = tx.send(Event::Stop);
        });
    }
    {
        let tx = tx.clone();
        plex.on_change(move || {
            let _ = tx.send(Event::Plex);
        });
    }
    let dir = dir_of(&ctx.paths);
    let mut plugin = Plugin {
        ctx: ctx.clone(),
        shared: shared.clone(),
        hw,
        data,
        net,
        tx: tx.clone(),
        guide: Guide::new(Arc::new(DirStore::new(dir.join("listings"))) as Arc<dyn Store>),
        secrets: Arc::new(Secrets::open(dir.join("secrets.json"))),
        plex,
        page: None,
        on: false,
        values: Values::new(),
        channels_due: None,
        said: Json::Null,
    };
    shared.offer(
        &OPT_IN,
        Arc::new(move || {
            let _ = tx.send(Event::Switch);
        }),
    );
    crate::plugins::guard::spawn("guide", 256 * 1024, shared, &[ID], ctx.stop.clone(), move |again| {
        if again {
            plugin.follow_afresh();
        }
        plugin.run(&rx)
    })
}

struct Plugin {
    ctx: Ctx,
    shared: Arc<Shared>,
    hw: Arc<dyn Hardware>,
    data: Arc<GuideData>,
    net: MakeNet,
    tx: Sender<Event>,
    guide: Guide,
    secrets: Arc<Secrets>,
    plex: Arc<PlexAccount>,
    /// Its conversation with the broker's session while it is on: its page, its facts, its action.
    page: Option<Arc<Local>>,
    on: bool,
    values: Values,
    /// When the TV's channels are read next.
    channels_due: Option<Instant>,
    /// The facts last put: unchanged ones are not put again.
    said: Json,
}

impl Plugin {
    /// After a panic: follows the switch from scratch, the page and the listings being as the panic left them.
    fn follow_afresh(&mut self) {
        self.on = !self.shared.switched_on(ID);
        self.switch();
    }

    fn run(&mut self, rx: &Receiver<Event>) {
        loop {
            match rx.recv_timeout(self.sleep()) {
                Ok(Event::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(Event::Switch) => self.switch(),
                Ok(Event::Settings(values)) => {
                    self.values = values;
                    self.configure();
                }
                Ok(Event::Secret | Event::Plex) => self.configure(),
                Ok(Event::Refresh) => self.guide.refresh_soon(now_ms() as i64),
                Err(RecvTimeoutError::Timeout) => {}
            }
            if self.ctx.stop.is_set() {
                break;
            }
            if self.on {
                self.work();
            }
            self.publish();
        }
        self.page = None;
    }

    /// Until the channels are due again or a source is, at most [`LONGEST_SLEEP`].
    fn sleep(&self) -> Duration {
        if !self.on {
            return LONGEST_SLEEP;
        }
        let now = Instant::now();
        let channels = self
            .channels_due
            .map_or(Duration::ZERO, |due| due.saturating_duration_since(now));
        let source = self.guide.due().map_or(LONGEST_SLEEP, |due| {
            Duration::from_millis(due.saturating_sub(now_ms() as i64).max(0) as u64)
        });
        channels.min(source).min(LONGEST_SLEEP)
    }

    /// Follows the owner's switch: on registers the page and starts the work; off takes the page away and forgets the listings.
    fn switch(&mut self) {
        let on = self.shared.switched_on(ID);
        if on == self.on {
            return;
        }
        self.on = on;
        if on {
            match Local::start(
                &self.shared,
                &OPT_IN.spec,
                register(),
                handler(self.tx.clone(), self.secrets.clone()),
            ) {
                Ok(page) => self.page = Some(page),
                Err(e) => crate::log_warn!("guide: the plugin's page was not registered: {e}"),
            }
            self.values = self.ctx.settings.effective(ID).unwrap_or_default();
            self.channels_due = None;
            self.configure();
        } else {
            self.page = None;
            self.said = Json::Null;
            self.guide.clear();
            self.data.set(None);
        }
    }

    fn configure(&mut self) {
        if !self.on {
            return;
        }
        let settings = settings_of(&self.values, &self.secrets, self.plex.server().as_ref());
        self.guide.set_settings(settings);
        self.data.set(Some(self.guide.listings()));
    }

    /// The channels when they are due, then the sources that are.
    fn work(&mut self) {
        if self.channels_due.is_none_or(|due| Instant::now() >= due) {
            let next = match self.hw.channel_list() {
                Ok(reply) => {
                    self.guide.set_channels(tv_channels(&reply));
                    CHANNELS_EVERY
                }
                Err(e) => {
                    crate::log_warn!(
                        "guide: the TV's channel list could not be read: {}",
                        e.reason()
                    );
                    CHANNELS_RETRY
                }
            };
            self.channels_due = Some(Instant::now() + next);
        }
        let now = now_ms() as i64;
        if self.guide.due().is_some_and(|due| due <= now) {
            // a week of a source took 25 to 60 s on the set: the page says so first, not what it said before the settings changed
            self.publish();
            let net = (self.net)(self.guide.settings());
            if self.guide.refresh(&*net, now) {
                crate::log_info!(
                    "guide: listings for {} channels",
                    self.guide.listings().channels()
                );
            }
        }
        self.data.set(Some(self.guide.listings()));
    }

    /// Puts the page's facts when they changed.
    fn publish(&mut self) {
        let Some(page) = &self.page else { return };
        let facts = self.facts(now_ms() as i64);
        let said = json!({"t": "facts.put", "facts": facts});
        if said != self.said {
            if let Err(e) = page.say(said.clone()) {
                crate::log_warn!("guide: the page's facts were not put: {e}");
            }
            self.said = said;
        }
    }

    fn facts(&self, now: i64) -> Map<String, Json> {
        let line = |text: String| json!(clip(&text.replace(char::is_control, " "), MAX_FACT_LINE));
        let mut facts = Map::new();
        let settings = self.guide.settings();
        let channels = self.guide.channels().len();
        let listed = self.guide.listings().channels();
        let status = self.guide.status();
        // a source that is not set up is passed over: the status is the others', and only when every source is unset its word
        let asked = status.iter().filter(|s| !s.skipped);
        let first_problem = asked.clone().find_map(|s| s.problem.clone());
        // Plex is set up by signing in, not on this page
        let unset = status.iter().find(|s| s.skipped && s.problem.is_some()).map(|s| match s.source {
            Source::Plex => SIGN_IN.to_owned(),
            _ => s.problem.clone().unwrap_or_default(),
        });
        facts.insert(
            "status".into(),
            line(if !lumaguide::guide::zip_ok(&settings.zip) {
                "Type the zip code".to_owned()
            } else if settings.sources.is_empty() {
                "Pick a source".to_owned()
            } else if channels == 0 {
                "The TV has no antenna channels: scan for channels first".to_owned()
            } else if listed > 0 {
                format!("{listed} of {channels} channels have online listings")
            } else if asked.count() == 0 {
                unset.unwrap_or_default()
            } else {
                first_problem.unwrap_or_else(|| "Fetching the listings".to_owned())
            }),
        );
        for source in Source::ALL {
            let words = match status.iter().find(|s| s.source == source) {
                None => "Not picked".to_owned(),
                Some(s) => {
                    let mut words = match s.refreshed {
                        Some(at) => format!(
                            "Updated {}: {} of {channels} channels",
                            when(at, now),
                            s.matched
                        ),
                        None => "Not fetched yet".to_owned(),
                    };
                    if !s.unmatched.is_empty() {
                        words.push_str(&format!(" (not listed: {})", s.unmatched.join(", ")));
                    }
                    if let Some(problem) = &s.problem {
                        words = if s.skipped && source == Source::Plex {
                            SIGN_IN.to_owned()
                        } else if s.skipped {
                            let mut c = problem.chars();
                            let typed: String = c
                                .next()
                                .map(|f| f.to_lowercase().chain(c).collect())
                                .unwrap_or_default();
                            format!("Skipped until set up: {typed}")
                        } else {
                            format!("{problem}. {words}")
                        };
                    }
                    words
                }
            };
            facts.insert(source.id().into(), line(words));
        }
        facts.insert(
            "plex-server".into(),
            line(match self.plex.server() {
                Some(s) if s.name.is_empty() => s.address,
                Some(s) => format!("{} ({})", s.name, s.address),
                None => "Not signed in to Plex".to_owned(),
            }),
        );
        // a secret field's row reads the fact of its name, dots as dashes (`plugin_rows` in the TV's Settings)
        let kept = if self.secrets.has(SD_PASSWORD) { "Kept on this TV" } else { "Not set" };
        facts.insert(SD_PASSWORD.replace('.', "-"), json!(kept));
        facts
    }
}

/// `2:05 PM` today, `Tuesday, November 3 2:05 PM` before.
fn when(at_ms: i64, now_ms: i64) -> String {
    let (Some(at), Some(now)) = (local_time(at_ms / 1000), local_time(now_ms / 1000)) else {
        return String::new();
    };
    let (time, date) = clock_words(at, true);
    if (at.year, at.month, at.day) == (now.year, now.month, now.day) {
        time
    } else {
        format!("{date} {time}")
    }
}

/// The TV's antenna channels in an `iepg/getChannelList` reply, with their RF channels.
fn tv_channels(reply: &Json) -> Vec<Channel> {
    lumatuner::docs::entries(reply)
        .into_iter()
        .map(|e| Channel {
            rf: lumatuner::docs::parse_channel_id(&e.channel_id).map(|(rf, _, _)| u16::from(rf)),
            id: e.channel_id,
            major: e.major,
            minor: e.minor,
            name: e.name,
        })
        .collect()
}

/// Takes what the base says to the thread: the page's values, a secret (kept here, at once, and answered), the action.
fn handler(tx: Sender<Event>, secrets: Arc<Secrets>) -> Handler {
    Arc::new(move |local: &Local, message: ToPlugin| match message {
        ToPlugin::SettingsApply { txn, values, .. } => {
            let applied = json!({"t": "settings.result", "txn": txn, "outcome": {"type": "applied", "values": values}});
            let _ = tx.send(Event::Settings(values));
            let _ = local.say(applied);
        }
        ToPlugin::SecretSubmit(s) => {
            let outcome = match secrets.put(&s.field, &s.secret) {
                Ok(()) => {
                    let _ = tx.send(Event::Secret);
                    json!({"type": "accepted"})
                }
                Err(reason) => json!({"type": "rejected", "reason": reason}),
            };
            let _ = local.say(json!({"t": "secret.result", "op": s.op, "outcome": outcome}));
        }
        ToPlugin::ActionInvoke { op, action, .. } => {
            let known = action == REFRESH;
            if known {
                let _ = tx.send(Event::Refresh);
            }
            let _ = local.answer(op, if known { Ok(()) } else { Err("no such action") });
        }
        _ => {}
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Paths};
    use crate::control::Stop;
    use crate::hub::Hub;
    use crate::plugins::Plugins;
    use crate::plugins::session::SecretError;
    use crate::settings::SettingsStore;
    use lumatuner::{Failure, Tap};
    use std::os::unix::fs::PermissionsExt;

    const WSB: &str = "1_39_2_1_0_0_0";
    const ESPANOL: &str = "1_21_35_2_0_0_0";

    /// A TV with two antenna channels and no listings of its own.
    struct TwoChannels;

    impl Hardware for TwoChannels {
        fn channel_list(&self) -> Result<Json, Failure> {
            Ok(json!({"channelList": [
                {"channelId": WSB, "channelNumber": "2-1", "channelName": "WSB-HD"},
                {"channelId": ESPANOL, "channelNumber": "35-2", "channelName": "ESPANOL"}]}))
        }

        fn programmes(&self, _: &str) -> Result<Json, Failure> {
            Ok(json!({"programList": []}))
        }

        fn in_use(&self) -> Option<String> {
            None
        }

        fn tune(&self, _: &str) -> Result<Box<dyn Tap>, Failure> {
            Err(Failure::Hardware("no tuner in a test".into()))
        }
    }

    /// The free listings, noting the page's status as each request goes out.
    struct Watched(Arc<Hub>, Arc<parking_lot::Mutex<Vec<Json>>>);

    impl Http for Watched {
        fn send(&self, r: &Request<'_>) -> Result<Response, NetError> {
            let facts = self
                .0
                .get(&format!("plugin.{ID}.facts"))
                .unwrap_or_default();
            self.1.lock().push(facts["facts"]["status"].clone());
            Site.send(r)
        }
    }

    /// The free listings site, for 2.1 only: an hour-long show at the start of every grid it is asked for.
    struct Site;

    impl Http for Site {
        fn send(&self, r: &Request<'_>) -> Result<Response, NetError> {
            let body = if r.url.contains("getPostalCodeProviders") {
                json!({"Providers": [{"type": "OTA", "device": "", "lineupId": "USA-lineupId-DEFAULT", "headendId": "lineupId",
                    "name": "Local Over the Air Broadcast"}]})
            } else {
                let at: i64 = r
                    .url
                    .split("&time=")
                    .nth(1)
                    .and_then(|t| t.split('&').next()?.parse().ok())
                    .unwrap_or(0);
                let stamp = |s: i64| lumaguide::time::xmltv(s * 1000);
                let iso = |s: i64| {
                    let t = stamp(s);
                    format!(
                        "{}-{}-{}T{}:{}:{}Z",
                        &t[0..4],
                        &t[4..6],
                        &t[6..8],
                        &t[8..10],
                        &t[10..12],
                        &t[12..14]
                    )
                };
                json!({"channels": [{"callSign": "WSBDT", "channelId": "19586", "channelNo": "2.1", "events": [
                    {"startTime": iso(at), "endTime": iso(at + 3600), "rating": "TV-PG", "tags": ["CC"],
                     "program": {"title": "Online show", "tmsId": "EP1", "shortDesc": "From the grid"}}]}]})
            };
            Ok(Response {
                status: 200,
                body: body.to_string().into_bytes(),
            })
        }
    }

    fn ctx(dir: &Path) -> Ctx {
        let hub = Arc::new(Hub::new(1));
        let paths = Paths::new(dir);
        let settings = Arc::new(SettingsStore::new(paths.settings_dir(), hub.clone()));
        settings.register_builtin().unwrap();
        Ctx {
            config: Arc::new(Config::default()),
            paths,
            hub,
            settings,
            stop: Stop::new(),
        }
    }

    fn soon(what: &str, done: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < end, "{what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn facts(ctx: &Ctx) -> Json {
        ctx.hub
            .get(&format!("plugin.{ID}.facts"))
            .unwrap_or_default()["facts"]
            .clone()
    }

    fn values(pairs: Json) -> Values {
        serde_json::from_value(pairs).unwrap()
    }

    #[test]
    fn switched_on_it_fills_the_guide_from_the_picked_source_and_says_how_each_stands() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(dir.path());
        let plugins = Plugins::new(&ctx);
        let data = Arc::new(GuideData::default());
        let said = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let (hub, heard) = (ctx.hub.clone(), said.clone());
        let net: MakeNet = Arc::new(move |_: &Settings| {
            Arc::new(Watched(hub.clone(), heard.clone())) as Arc<dyn Http>
        });
        let plex = PlexAccount::fixed(None);
        let threads = [
            plugins.spawn(&ctx).unwrap(),
            spawn_with(
                &ctx,
                plugins.shared().clone(),
                Arc::new(TwoChannels),
                data.clone(),
                plex.clone(),
                net,
            )
            .unwrap(),
        ];
        let approve = || plugins.shared().approval_of(ID);
        soon("offered, off", || approve().is_some());
        assert!(data.listings().is_none(), "off: no listings");
        plugins.switch(ID, true, approve().as_deref()).unwrap();
        soon("its page", || ctx.settings.effective(ID).is_some());
        // Plex first but not set up: passed over, and the next source fills the guide
        let changes = serde_json::from_value(json!([
            {"key": "zip", "value": {"type": "text", "value": "30311"}},
            {"key": "sources", "value": {"type": "ordered", "value": ["plex", "gracenote"]}}]))
        .unwrap();
        ctx.settings.apply(ID, changes, None).unwrap();
        soon("the listings", || {
            data.listings().is_some_and(|l| l.channels() == 1)
        });
        assert_eq!(
            said.lock().first(),
            Some(&json!("Fetching the listings")),
            "the page says it fetches before the first request, not the unset source's words"
        );
        soon("the page says so", || {
            facts(&ctx)["status"] == "1 of 2 channels have online listings"
        });
        let f = facts(&ctx);
        assert!(
            f["gracenote"]
                .as_str()
                .unwrap()
                .contains("1 of 2 channels (not listed: 35.2)"),
            "{f}"
        );
        assert_eq!(
            (f["plex"].as_str(), f["plex-server"].as_str()),
            (Some(SIGN_IN), Some("Not signed in to Plex"))
        );
        // signing the TV in to Plex gives the source the account's server
        plex.set_server(Some(Server::test("beast", "BeastNAS", "192.168.1.69:32400", "tk-beast")));
        soon("the account's server", || facts(&ctx)["plex-server"] == "BeastNAS (192.168.1.69:32400)");

        // the TV's guide and banner ask the API; the tuner's guide lays the listings over the TV's own
        let now = now_ms() as i64;
        let request = HttpRequest {
            method: "GET".into(),
            path: "/v1/guide/programmes".into(),
            query: vec![
                ("channel".into(), WSB.into()),
                ("from".into(), (now - 7 * HOUR_MS).to_string()),
                ("to".into(), now.to_string()),
            ],
            headers: Vec::new(),
            body: Vec::new(),
            peer: IpAddr::from([127, 0, 0, 1]),
        };
        let answer: Json =
            serde_json::from_slice(&handle(&data, &request, &["programmes"]).body).unwrap();
        assert_eq!(answer["on"], true);
        assert_eq!(answer["programmes"][0]["title"], "Online show");
        assert_eq!(answer["programmes"][0]["channel"], WSB);
        let entries = lumatuner::docs::entries(&TwoChannels.channel_list().unwrap());
        let espanol_own = json!({"programList": [{"channelId": ESPANOL, "programName": "Noticias", "startTime": "2026,11,05,18,00,00",
            "endTime": "2026,11,05,19,00,00"}]});
        let xml = lumatuner::Guide::xmltv(
            &*data,
            &[
                (entries[0].clone(), json!({})),
                (entries[1].clone(), espanol_own),
            ],
        )
        .unwrap();
        assert!(
            xml.contains("channel=\"2.1\"><title>Online show</title>")
                && xml.contains("channel=\"35.2\"><title>Noticias</title>"),
            "{xml}"
        );

        // a secret typed in Settings is kept by the plugin alone, privately, and the page says only that it is
        assert_eq!(
            plugins.shared().submit_secret(ID, SD_PASSWORD, "pw\u{7}"),
            Err(SecretError::Rejected(
                "that password has a character it cannot have".into()
            ))
        );
        plugins
            .shared()
            .submit_secret(ID, SD_PASSWORD, "pw-1")
            .unwrap();
        assert_eq!(
            plugins.shared().submit_secret(ID, ZIP, "x"),
            Err(SecretError::NoSuchField)
        );
        soon("kept", || facts(&ctx)["sd-password"] == "Kept on this TV");
        let file = dir.path().join("guide/secrets.json");
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(
            !serde_json::to_string(&facts(&ctx))
                .unwrap()
                .contains("pw-1")
        );

        plugins.switch(ID, false, None).unwrap();
        soon("off: the listings are forgotten", || {
            data.listings().is_none()
        });
        assert!(
            DirStore::new(dir.path().join("guide/listings"))
                .keys("")
                .is_empty()
        );
        assert_eq!(
            Secrets::open(file).get(SD_PASSWORD),
            "pw-1",
            "the secrets stay with the page's settings"
        );
        ctx.stop.request();
        for t in threads {
            t.join().unwrap();
        }
    }

    #[test]
    fn the_sources_come_in_the_order_picked_and_plex_is_the_accounts_server() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Secrets::open(dir.path().join("secrets.json"));
        secrets.put(SD_PASSWORD, "pw").unwrap();
        let picked = values(json!({"zip": {"type": "text", "value": " 30311 "},
            "sources": {"type": "ordered", "value": ["schedules-direct", "gracenote", "plex"]},
            "sd.username": {"type": "text", "value": "nitin"}}));
        let settings = settings_of(&picked, &secrets, None);
        assert_eq!(settings.zip, "30311");
        assert_eq!(
            settings.sources,
            [
                SourceSettings::SchedulesDirect {
                    username: "nitin".into(),
                    password: "pw".into()
                },
                SourceSettings::Gracenote,
                SourceSettings::Plex {
                    address: String::new(),
                    token: String::new()
                }
            ]
        );
        let server = Server::test("beast", "BeastNAS", "192.168.1.69:32400", "tk-beast");
        assert_eq!(
            settings_of(&picked, &secrets, Some(&server)).sources[2],
            SourceSettings::Plex {
                address: "192.168.1.69:32400".into(),
                token: "tk-beast".into()
            }
        );
    }

    #[test]
    fn the_network_reaches_only_the_picked_services_and_a_plex_server_of_the_home_network() {
        let settings = |sources| Settings {
            zip: "30311".into(),
            sources,
        };
        let plex = |address: &str| SourceSettings::Plex {
            address: address.into(),
            token: "tk".into(),
        };
        let net = |s: &Settings| Net {
            allowed: allowed(s),
            curl: CurlFetch::new("/nonexistent/curl", None),
            as_helper: CurlFetch::new("/nonexistent/curl", None),
            refused: AtomicBool::new(false),
        };
        let reach = |n: &Net, url: &str| n.send(&Request::get(url));
        let n = net(&settings(vec![
            SourceSettings::Gracenote,
            plex("192.168.1.69:32400"),
        ]));
        // allowed: refused by nothing but the missing curl
        for url in [
            "https://tvlistings.gracenote.com/api/grid?time=1",
            "http://192.168.1.69:32400/livetv/dvrs",
        ] {
            assert!(matches!(reach(&n, url), Err(NetError::Failed(_))), "{url}");
        }
        for url in [
            "https://json.schedulesdirect.org/20141201/token",
            "http://tvlistings.gracenote.com/api/grid",
            "https://192.168.1.69:32400/",
            "http://192.168.1.69:8080/",
            "https://example.com/",
        ] {
            assert!(
                matches!(reach(&n, url), Err(NetError::NotAllowed(_))),
                "{url}"
            );
        }
        // a Plex server off the home network is never reached
        let public = net(&settings(vec![plex("8.8.8.8:32400")]));
        assert!(matches!(
            reach(&public, "http://8.8.8.8:32400/livetv/dvrs"),
            Err(NetError::NotAllowed(_))
        ));
    }

    #[test]
    fn the_tvs_own_listings_and_channels_are_read_with_their_utc_times_and_rf_channels() {
        let reply = json!({"programList": [
            {"channelId": WSB, "programName": "News", "programSubTitle": "Late", "description": "d", "isSubtitle": true,
             "startTime": "2026,11,05,23,00,00", "endTime": "2026,11,06,00,00,00", "programId": "p1"},
            {"channelId": WSB, "programMainTitle": "", "startTime": "2026,11,06,00,00,00", "endTime": "2026,11,06,01,00,00"},
            {"channelId": "1_39_2_2_0_0_0", "programName": "Other", "startTime": "2026,11,05,23,00,00", "endTime": "2026,11,06,00,00,00"}]});
        let list = broadcast(&reply, WSB);
        assert_eq!(
            list.len(),
            1,
            "untitled and other channels' programmes are left out"
        );
        assert_eq!(
            (
                list[0].title.as_str(),
                list[0].subtitle.as_str(),
                list[0].captions
            ),
            ("News", "Late", true)
        );
        assert_eq!(
            list[0].start,
            lumaguide::time::parse_utc("2026-11-05T23:00:00Z").unwrap()
        );
        assert_eq!(lg_utc("2026,13,05,23,00,00"), None);
        let channels = tv_channels(&TwoChannels.channel_list().unwrap());
        assert_eq!(
            channels[0],
            Channel {
                id: WSB.into(),
                major: 2,
                minor: 1,
                name: "WSB-HD".into(),
                rf: Some(39)
            }
        );
    }

    #[test]
    fn the_page_is_one_the_settings_store_takes() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path(), Arc::new(Hub::new(1)));
        let schema =
            settings_schema::SettingsSchema::parse(schema_json().to_string().as_bytes()).unwrap();
        store.register(schema, None).unwrap();
        assert_eq!(store.secret_field(ID, SD_PASSWORD), Some(128));
        assert_eq!(store.secret_field(ID, ZIP), None);
        assert!(serde_json::from_value::<plugin_api::wire::Register>(register()).is_ok());
    }
}
