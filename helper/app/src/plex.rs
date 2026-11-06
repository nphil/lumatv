//! The Plex account: this TV's one sign-in to Plex, kept by the helper and used by everything on the TV that talks to a Plex server: the
//! Guide data plugin's Plex source and the Live TV Tuner's Plex DVR setup.
//! It is the base's connection, as the Home Assistant lease is (`plugins/ha.rs`): no plugin signs in by itself, and none is given the
//! account's token.
//!
//! **Signing in is a code, not typing.** Settings asks for a PIN (`POST https://plex.tv/api/v2/pins`, product `LumaTV`, this TV's client
//! identifier) and shows its four characters with a QR code of Plex's link page for it, the address of the QR code Plex makes for a PIN
//! itself ([`link_url`]). The person confirms on a phone; the helper asks after the PIN every two seconds until it carries the account's
//! token or expires (Plex gives a four-character PIN 15 minutes), then reads who signed in (`/api/v2/user`) and the account's servers
//! (`/api/v2/resources`).
//!
//! **The server** is one the account owns that answers at an address of the home network: a local IPv4 connection, held to the rule the
//! Home Assistant lease is (`ha_link::AddressPolicy::HOME`). The first one Plex lists is taken at once; Settings offers the others.
//!
//! **The tokens are secrets.** The account's token and each server's access token are kept in `plex/account.json` (mode 0600) and
//! nowhere else: never in a log, a topic, a setting or a message to a plugin of the account's own. The client identifier is kept apart
//! (`plex/client-id`) and outlives a sign-out, so a new sign-in is the same device in the account's list. Signing out revokes the token
//! at plex.tv (`DELETE /api/v2/users/signout`) and deletes the file, also when plex.tv cannot be reached.
//!
//! **What it reaches:** `plex.tv` and `clients.plex.tv` over HTTPS, and nothing else ([`CurlNet`]); the servers are reached by what
//! uses them.
//!
//! What the TV is told is the `plex.account` topic: how the sign-in stands, the code and its address while one is shown, the account's
//! name, the server and the others to choose from, never a token.

use crate::config::{FetchConfig, Paths};
use crate::ctx::Ctx;
use crate::fetcher::{CurlFetch, Exchange};
use crate::http::{CLIENT_HEADER, Request, Response, UI_CLIENT, error_response};
use crate::settings::SettingsStore;
use fetch::{FetchError, FetchLimits};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use std::net::{IpAddr, Ipv4Addr};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// The topic the TV's Settings reads.
pub const TOPIC: &str = "plex.account";
/// What Plex shows for this TV in the account's devices.
const PRODUCT: &str = "LumaTV";
const ACCOUNT_FILE: &str = "account.json";
const CLIENT_ID_FILE: &str = "client-id";
/// Plex's link page for a four-character PIN: the address in the QR code plex.tv makes for one (`pins/qr/<code>`).
const LINK: &str = "https://plex.tv/link/?pin=";
const PLEX_TV: &str = "https://plex.tv/api/v2";
const RESOURCES: &str = "https://clients.plex.tv/api/v2/resources?includeHttps=1&includeIPv6=1";
/// How often a PIN is asked after while it is shown (Plex's guide says once a second; every two is plenty for a person and half the
/// requests).
const POLL: Duration = Duration::from_secs(2);
/// The account's name and servers are read again at most this often when Settings opens the page.
const REFRESH_EVERY_MS: i64 = 60_000;
pub const ROUTES: &[(&str, &str, &str)] = &[
    (
        "GET",
        "/v1/plex",
        "the Plex account: how the sign-in stands, the code while one is shown, the account's name and servers; never a token",
    ),
    (
        "POST",
        "/v1/plex/signin",
        "asks Plex for a code to sign this TV in with (the TV's own Settings, the owner only)",
    ),
    (
        "POST",
        "/v1/plex/cancel",
        "stops asking after the code shown",
    ),
    (
        "POST",
        "/v1/plex/refresh",
        "reads the account's name and servers again",
    ),
    (
        "POST",
        "/v1/plex/server",
        "picks the server the TV uses: {\"id\": the server's id from /v1/plex}",
    ),
    (
        "POST",
        "/v1/plex/signout",
        "revokes the account's token at plex.tv and forgets it",
    ),
];

// -- what is kept --------------------------------------------------------------------------------------------------------------------

/// A server of the account that answers on the home network.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    /// Plex's machine identifier.
    pub id: String,
    pub name: String,
    /// `ip:port` of the home network, plain http.
    pub address: String,
    /// What the server takes as `X-Plex-Token`.
    token: String,
}

impl Server {
    pub fn token(&self) -> &str {
        &self.token
    }

    #[cfg(test)]
    pub(crate) fn test(id: &str, name: &str, address: &str, token: &str) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            address: address.into(),
            token: token.into(),
        }
    }
}

impl std::fmt::Debug for Server {
    /// Never says the token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("address", &self.address)
            .finish()
    }
}

/// `plex/account.json`.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
struct Account {
    token: String,
    #[serde(default)]
    user: String,
    #[serde(default)]
    servers: Vec<Server>,
    /// The id of the server in use.
    #[serde(default)]
    server: String,
}

impl Account {
    fn server(&self) -> Option<&Server> {
        self.servers.iter().find(|s| s.id == self.server)
    }
}

fn load_account(dir: &Path) -> Option<Account> {
    let account: Account =
        serde_json::from_slice(&std::fs::read(dir.join(ACCOUNT_FILE)).ok()?).ok()?;
    (!account.token.is_empty()).then_some(account)
}

fn save_account(dir: &Path, account: &Account) -> std::io::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    crate::hue_store::write_private(
        &dir.join(ACCOUNT_FILE),
        &serde_json::to_vec(account)?,
        0o600,
    )
}

/// This TV's client identifier: made once (`lumatv-` and 16 random hex digits) and kept, so every sign-in is the same device to Plex.
fn client_id(dir: &Path) -> std::io::Result<String> {
    let file = dir.join(CLIENT_ID_FILE);
    if let Ok(kept) = std::fs::read_to_string(&file) {
        let kept = kept.trim();
        if !kept.is_empty() && kept.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Ok(kept.to_owned());
        }
    }
    let mut bytes = [0u8; 8];
    crate::sys::random_bytes(&mut bytes)?;
    let id = format!(
        "lumatv-{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    crate::hue_store::write_private(&file, id.as_bytes(), 0o600)?;
    Ok(id)
}

/// The address of the QR code and the link a person opens for the PIN `code`.
pub fn link_url(code: &str) -> String {
    format!("{LINK}{code}")
}

// -- the network --------------------------------------------------------------------------------------------------------------------

/// The account's way out.
pub trait Net: Send + Sync {
    /// `method` to `url` with `headers`: the status and the body, or what went wrong in words.
    fn send(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<(u16, Vec<u8>), String>;
}

/// plex.tv and clients.plex.tv over HTTPS through the set's curl (the headers, the token among them, go on its standard input), the TLS
/// child run as the unprivileged fetch user when the helper is root, and as the helper's when the system refuses that.
pub struct CurlNet {
    curl: CurlFetch,
    as_helper: CurlFetch,
    refused: AtomicBool,
}

impl CurlNet {
    pub fn new(config: &FetchConfig) -> Self {
        Self {
            curl: CurlFetch::of(config, true),
            as_helper: CurlFetch::of(config, false),
            refused: AtomicBool::new(false),
        }
    }
}

impl Net for CurlNet {
    fn send(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<(u16, Vec<u8>), String> {
        if !(url.starts_with("https://plex.tv/") || url.starts_with("https://clients.plex.tv/")) {
            return Err("the Plex account reaches plex.tv only".into());
        }
        let x = Exchange {
            method,
            url,
            headers,
            body: None,
        };
        let limits = FetchLimits {
            total: Duration::from_secs(15),
            max_bytes: 1 << 20,
        };
        let answer = if self.refused.load(Ordering::Relaxed) {
            self.as_helper.exchange(&x, limits)
        } else {
            match self.curl.exchange(&x, limits) {
                Err(FetchError::Denied(why)) => {
                    self.refused.store(true, Ordering::Relaxed);
                    crate::log_warn!(
                        "plex: the system refused curl as the fetch user ({why}); it runs as the helper's from now on"
                    );
                    self.as_helper.exchange(&x, limits)
                }
                other => other,
            }
        };
        answer.map_err(|e| match e {
            FetchError::Offline => {
                "plex.tv cannot be reached: is the TV on the internet?".to_owned()
            }
            FetchError::Timeout => "plex.tv did not answer in time".to_owned(),
            e => format!("plex.tv could not be asked: {e}"),
        })
    }
}

// -- the sign-in --------------------------------------------------------------------------------------------------------------------

/// How the sign-in stands.
enum Link {
    Out,
    /// A PIN is shown: its id, its four characters and when it expires, and the account signed in before, which stays in use until the
    /// code is linked.
    Code {
        pin: u64,
        code: String,
        expires_ms: i64,
        kept: Option<Account>,
    },
    In(Account),
}

/// The sign-in and what it keeps, run by the account's thread (tests drive it directly, with their own network and clock).
struct Machine {
    dir: PathBuf,
    client_id: String,
    link: Link,
    /// What went wrong last, in words for the page.
    problem: Option<String>,
    /// When the account's name and servers were read last.
    refreshed_ms: Option<i64>,
    /// The TV's own IPv4 addresses, which pick a server's address on the home network ([`servers_of`]).
    own: fn() -> Vec<Ipv4Addr>,
}

/// The TV's IPv4 addresses but loopback.
fn own_addresses() -> Vec<Ipv4Addr> {
    crate::sys::ipv4_interfaces()
        .into_iter()
        .map(|(_, ip)| ip)
        .filter(|ip| !ip.is_loopback())
        .collect()
}

impl Machine {
    fn open(dir: PathBuf) -> std::io::Result<Self> {
        let client_id = client_id(&dir)?;
        let link = load_account(&dir).map_or(Link::Out, Link::In);
        Ok(Self {
            dir,
            client_id,
            link,
            problem: None,
            refreshed_ms: None,
            own: own_addresses,
        })
    }

    fn headers<'a>(&'a self, token: Option<&'a str>) -> Vec<(&'a str, &'a str)> {
        let mut h = vec![
            ("Accept", "application/json"),
            ("X-Plex-Product", PRODUCT),
            ("X-Plex-Version", env!("CARGO_PKG_VERSION")),
            ("X-Plex-Platform", "webOS"),
            ("X-Plex-Device-Name", PRODUCT),
            ("X-Plex-Client-Identifier", &self.client_id),
        ];
        if let Some(token) = token {
            h.push(("X-Plex-Token", token));
        }
        h
    }

    /// Asks Plex for a code to show; the account kept, if any, stays until the code is linked or given up.
    fn sign_in(&mut self, net: &dyn Net, now_ms: i64) {
        self.problem = None;
        let answer = net.send("POST", &format!("{PLEX_TV}/pins"), &self.headers(None));
        let pin = match answer {
            Ok((201 | 200, body)) => serde_json::from_slice::<Json>(&body).ok(),
            Ok((status, _)) => {
                self.problem = Some(format!("Plex did not give a code (HTTP {status})"));
                return;
            }
            Err(why) => {
                self.problem = Some(why);
                return;
            }
        };
        let (Some(id), Some(code)) = (
            pin.as_ref().and_then(|p| p["id"].as_u64()),
            pin.as_ref()
                .and_then(|p| p["code"].as_str())
                .filter(|c| !c.is_empty() && c.bytes().all(|b| b.is_ascii_alphanumeric())),
        ) else {
            self.problem = Some("Plex's answer had no code".into());
            return;
        };
        let lasts = pin
            .as_ref()
            .and_then(|p| p["expiresIn"].as_i64())
            .unwrap_or(900)
            .clamp(60, 3600);
        crate::log_info!("plex: a sign-in code is shown, for {lasts} s");
        let kept = match std::mem::replace(&mut self.link, Link::Out) {
            Link::In(account) => Some(account),
            Link::Code { kept, .. } => kept,
            Link::Out => None,
        };
        self.link = Link::Code {
            pin: id,
            code: code.to_owned(),
            expires_ms: now_ms + lasts * 1000,
            kept,
        };
    }

    /// Gives up the code shown; the account signed in before, if any, is the one in force.
    fn cancel(&mut self) {
        if matches!(self.link, Link::Code { .. })
            && let Link::Code { kept, .. } = std::mem::replace(&mut self.link, Link::Out)
        {
            self.link = kept.map_or(Link::Out, Link::In);
            self.problem = None;
        }
    }

    /// Asks after the code shown: still waiting, linked (the account is kept and read), or expired.
    fn poll(&mut self, net: &dyn Net, now_ms: i64) {
        let Link::Code {
            pin, expires_ms, ..
        } = self.link
        else {
            return;
        };
        if now_ms >= expires_ms {
            self.expired();
            return;
        }
        let answer = net.send("GET", &format!("{PLEX_TV}/pins/{pin}"), &self.headers(None));
        let token = match answer {
            Ok((200, body)) => serde_json::from_slice::<Json>(&body)
                .ok()
                .and_then(|p| p["authToken"].as_str().map(str::to_owned)),
            Ok((404, _)) => {
                self.expired();
                return;
            }
            // a request that failed is asked again at the next turn, until the code expires
            Ok(_) | Err(_) => None,
        };
        let Some(token) = token.filter(|t| plain(t)) else {
            return;
        };
        crate::log_info!("plex: the code was linked to an account");
        self.link = Link::In(Account {
            token,
            ..Account::default()
        });
        self.refresh(net, now_ms);
    }

    fn expired(&mut self) {
        self.cancel();
        self.problem = Some("The code expired before it was linked: ask for a new one".into());
    }

    /// Reads who signed in and the account's servers, keeps them, and picks the server: the one in use while Plex still lists it, the
    /// one at the address taken over from Guide data, else the first. A token Plex turns down is forgotten.
    fn refresh(&mut self, net: &dyn Net, now_ms: i64) {
        let Link::In(account) = &self.link else {
            return;
        };
        let mut account = account.clone();
        self.refreshed_ms = Some(now_ms);
        self.problem = None;
        let answer = net.send(
            "GET",
            &format!("{PLEX_TV}/user"),
            &self.headers(Some(&account.token)),
        );
        match answer {
            Ok((200, body)) => {
                let user = serde_json::from_slice::<Json>(&body).unwrap_or_default();
                let name = ["username", "title", "friendlyName"]
                    .iter()
                    .find_map(|k| user[k].as_str().filter(|n| !n.is_empty()));
                if let Some(name) = name {
                    account.user = name.to_owned();
                }
            }
            Ok((401, _)) => {
                crate::log_warn!("plex: plex.tv turned the account's token down; it is forgotten");
                self.forget();
                self.problem =
                    Some("Plex no longer accepts this TV's sign-in: sign in again".into());
                return;
            }
            Ok((status, _)) => {
                self.problem = Some(format!(
                    "plex.tv did not say who is signed in (HTTP {status})"
                ))
            }
            Err(why) => self.problem = Some(why),
        }
        let answer = net.send("GET", RESOURCES, &self.headers(Some(&account.token)));
        match answer {
            // a list with none of the account's own servers on the home network keeps the server in use
            Ok((200, body)) => {
                match servers_of(
                    &serde_json::from_slice::<Json>(&body).unwrap_or_default(),
                    &(self.own)(),
                ) {
                    found if found.is_empty() => {
                        self.problem =
                            Some("Plex lists no server of this account on the home network".into());
                    }
                    found => {
                        let kept = account.server().cloned();
                        account.server = kept
                            .as_ref()
                            .and_then(|k| {
                                found
                                    .iter()
                                    .find(|s| s.id == k.id)
                                    .or_else(|| found.iter().find(|s| s.address == k.address))
                            })
                            .unwrap_or(&found[0])
                            .id
                            .clone();
                        account.servers = found;
                    }
                }
            }
            Ok((status, _)) => {
                self.problem.get_or_insert(format!(
                    "plex.tv did not list the account's servers (HTTP {status})"
                ));
            }
            Err(why) => {
                self.problem.get_or_insert(why);
            }
        }
        self.keep(account);
    }

    fn refresh_if_old(&mut self, net: &dyn Net, now_ms: i64) {
        if self
            .refreshed_ms
            .is_none_or(|at| now_ms - at >= REFRESH_EVERY_MS)
        {
            self.refresh(net, now_ms);
        }
    }

    fn choose(&mut self, id: &str) -> bool {
        let Link::In(account) = &self.link else {
            return false;
        };
        if !account.servers.iter().any(|s| s.id == id) {
            return false;
        }
        let mut account = account.clone();
        account.server = id.to_owned();
        self.keep(account);
        true
    }

    /// Keeps `account` on disk and in force; a file that cannot be written leaves the old one and says so.
    fn keep(&mut self, account: Account) {
        if let Err(e) = save_account(&self.dir, &account) {
            crate::log_warn!("plex: the account could not be kept: {e}");
            self.problem = Some(format!("The sign-in could not be kept on this TV ({e})"));
        }
        self.link = Link::In(account);
    }

    /// Revokes the token at plex.tv and forgets the account, also when plex.tv cannot be told (the page says so).
    fn sign_out(&mut self, net: &dyn Net) {
        let Link::In(account) = &self.link else {
            self.cancel();
            return;
        };
        let revoked = net.send(
            "DELETE",
            &format!("{PLEX_TV}/users/signout"),
            &self.headers(Some(&account.token)),
        );
        self.forget();
        self.problem = match revoked {
            Ok((200..=299 | 401, _)) => None,
            Ok((status, _)) => Some(format!(
                "Signed out on this TV; plex.tv refused to end the sign-in (HTTP {status}), so remove LumaTV at plex.tv/devices"
            )),
            Err(why) => Some(format!(
                "Signed out on this TV; {why}, so remove LumaTV at plex.tv/devices"
            )),
        };
        crate::log_info!("plex: signed out");
    }

    fn forget(&mut self) {
        match std::fs::remove_file(self.dir.join(ACCOUNT_FILE)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                crate::log_warn!("plex: the account file could not be deleted: {e}")
            }
            _ => {}
        }
        self.link = Link::Out;
    }

    /// The server in use and its token, for what uses it.
    fn server(&self) -> Option<Server> {
        match &self.link {
            Link::In(account)
            | Link::Code {
                kept: Some(account),
                ..
            } => account.server().cloned(),
            _ => None,
        }
    }

    /// The `plex.account` topic: never a token.
    fn view(&self) -> Json {
        let server = |s: &Server| json!({"id": s.id, "name": s.name, "address": s.address});
        let mut view = match &self.link {
            Link::Out => json!({"state": "signed-out"}),
            Link::Code {
                code, expires_ms, ..
            } => {
                json!({"state": "code", "code": code, "url": link_url(code), "expiresAt": expires_ms})
            }
            Link::In(a) => {
                json!({"state": "signed-in", "user": a.user, "server": a.server().map(server),
                                  "servers": a.servers.iter().map(server).collect::<Vec<_>>()})
            }
        };
        if let Some(problem) = &self.problem {
            view["problem"] = json!(problem);
        }
        view
    }

    fn polling(&self) -> bool {
        matches!(self.link, Link::Code { .. })
    }
}

/// A token, an id: printable, no space.
fn plain(text: &str) -> bool {
    !text.is_empty() && text.len() <= 256 && text.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

/// The servers of a `resources` answer that the account owns and that answer at an IPv4 address of the home network, in Plex's order.
/// A server in Docker lists its bridges' gateways (172.17.0.1 and the like) as local connections too, beside its LAN address and in no
/// fixed order: the address taken is the one nearest one of `own`, the TV's addresses (the longest common prefix), Plex's first on a tie.
fn servers_of(resources: &Json, own: &[Ipv4Addr]) -> Vec<Server> {
    let near = |ip: Ipv4Addr| {
        own.iter()
            .map(|o| (u32::from(*o) ^ u32::from(ip)).leading_zeros())
            .max()
            .unwrap_or(0)
    };
    let mut out = Vec::new();
    for r in resources.as_array().into_iter().flatten() {
        let provides = r["provides"].as_str().unwrap_or("");
        if !provides.split(',').any(|p| p.trim() == "server") || r["owned"].as_bool() != Some(true)
        {
            continue;
        }
        let (Some(id), Some(token)) = (
            r["clientIdentifier"].as_str().filter(|i| plain(i)),
            r["accessToken"].as_str().filter(|t| plain(t)),
        ) else {
            continue;
        };
        let local = r["connections"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| {
                if c["local"].as_bool() != Some(true) || c["relay"].as_bool() == Some(true) {
                    return None;
                }
                let ip: Ipv4Addr = c["address"].as_str()?.parse().ok()?;
                let port = u16::try_from(c["port"].as_u64()?).ok().filter(|p| *p > 0)?;
                ha_link::AddressPolicy::HOME
                    .allows(IpAddr::V4(ip))
                    .then_some((ip, port))
            })
            .enumerate()
            .max_by_key(|(i, (ip, _))| (near(*ip), std::cmp::Reverse(*i)))
            .map(|(_, (ip, port))| format!("{ip}:{port}"));
        let Some(address) = local else { continue };
        let name = r["name"]
            .as_str()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or("Plex Media Server");
        out.push(Server {
            id: id.to_owned(),
            name: name.chars().take(64).collect(),
            address,
            token: token.to_owned(),
        });
    }
    out
}

// -- the account the helper runs ----------------------------------------------------------------------------------------------------

enum Cmd {
    SignIn,
    Cancel,
    Refresh,
    Choose(String),
    SignOut,
    Stop,
}

type Listener = Arc<dyn Fn() + Send + Sync>;

/// What the rest of the helper holds: the server in use, and the way to the account's thread.
pub struct PlexAccount {
    server: RwLock<Option<Server>>,
    tx: Sender<Cmd>,
    listeners: Mutex<Vec<Listener>>,
}

impl PlexAccount {
    /// The server in use and its access token, while the TV is signed in and has one.
    pub fn server(&self) -> Option<Server> {
        self.server.read().clone()
    }

    /// Calls `listener` (on the account's thread) whenever the server in use changes, its token included.
    pub fn on_change(&self, listener: impl Fn() + Send + Sync + 'static) {
        self.listeners.lock().push(Arc::new(listener));
    }

    fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    /// Puts `server` in force and tells the listeners when it is another (the account's thread; a test stands in for it).
    pub(crate) fn set_server(&self, server: Option<Server>) {
        let changed = {
            let mut held = self.server.write();
            let changed = *held != server;
            *held = server;
            changed
        };
        if changed {
            let listeners: Vec<Listener> = self.listeners.lock().clone();
            for listener in listeners {
                listener();
            }
        }
    }

    /// An account with `server` in force and no thread behind it.
    #[cfg(test)]
    pub(crate) fn fixed(server: Option<Server>) -> Arc<Self> {
        Arc::new(Self {
            server: RwLock::new(server),
            tx: mpsc::channel().0,
            listeners: Mutex::new(Vec::new()),
        })
    }
}

/// Opens the account (its file, its client identifier) and starts its thread; it reaches the network only when asked.
pub fn spawn(
    ctx: &Ctx,
    net: Arc<dyn Net>,
) -> std::io::Result<(Arc<PlexAccount>, JoinHandle<()>)> {
    let mut machine = Machine::open(ctx.paths.plex_dir())?;
    let (tx, rx) = mpsc::channel();
    let account = Arc::new(PlexAccount {
        server: RwLock::new(machine.server()),
        tx: tx.clone(),
        listeners: Mutex::new(Vec::new()),
    });
    ctx.stop.on_stop(move || {
        let _ = tx.send(Cmd::Stop);
    });
    let hub = ctx.hub.clone();
    hub.set(TOPIC, machine.view());
    let held = account.clone();
    let handle = thread::Builder::new()
        .name("plex".to_owned())
        .stack_size(256 * 1024)
        .spawn(move || {
            // an account taken over from Guide data has no name and no server list yet
            if let Link::In(a) = &machine.link
                && (a.user.is_empty() || a.servers.is_empty())
            {
                machine.refresh(&*net, now());
                publish(&machine, &hub, &held);
            }
            run(&mut machine, &rx, &*net, &hub, &held);
        })?;
    Ok((account, handle))
}

fn now() -> i64 {
    crate::timeutil::now_ms() as i64
}

fn publish(machine: &Machine, hub: &crate::hub::Hub, account: &PlexAccount) {
    hub.set(TOPIC, machine.view());
    account.set_server(machine.server());
}

fn run(
    machine: &mut Machine,
    rx: &Receiver<Cmd>,
    net: &dyn Net,
    hub: &crate::hub::Hub,
    account: &PlexAccount,
) {
    loop {
        let cmd = if machine.polling() {
            rx.recv_timeout(POLL)
        } else {
            rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };
        match cmd {
            Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Ok(Cmd::SignIn) => machine.sign_in(net, now()),
            Ok(Cmd::Cancel) => machine.cancel(),
            Ok(Cmd::Refresh) => machine.refresh_if_old(net, now()),
            Ok(Cmd::Choose(id)) => {
                machine.choose(&id);
            }
            Ok(Cmd::SignOut) => machine.sign_out(net),
            Err(RecvTimeoutError::Timeout) => machine.poll(net, now()),
        }
        publish(machine, hub, account);
    }
}

/// Guide data kept a Plex server of its own (an address on its page, a token in its secrets) before the account. On the first start
/// without an account they become the account's, signed in, and Guide data keeps neither; the account's name and servers are read
/// once the thread runs. Guide data's copy goes only once the account is kept.
pub fn migrate(paths: &Paths, settings: &SettingsStore) {
    let Some((address, token)) = crate::guidedata::plex_kept(paths, settings) else {
        return;
    };
    let dir = paths.plex_dir();
    if load_account(&dir).is_none() && plain(&token) {
        let server = Server {
            id: String::new(),
            name: String::new(),
            address,
            token: token.clone(),
        };
        let account = Account {
            token,
            user: String::new(),
            servers: vec![server],
            server: String::new(),
        };
        if let Err(e) = save_account(&dir, &account) {
            crate::log_warn!("plex: Guide data's Plex token could not become the account's: {e}");
            return;
        }
        crate::log_info!("plex: Guide data's Plex server and token became the Plex account's");
    }
    crate::guidedata::forget_plex(paths, settings);
}

// -- the routes ---------------------------------------------------------------------------------------------------------------------

/// `/v1/plex/...`; `rest` is the path after `plex`. A change is the TV's own Settings' (`X-Luma-Client`) and the owner's.
pub fn handle(
    account: &PlexAccount,
    hub: &crate::hub::Hub,
    request: &Request,
    rest: &[&str],
    owner: Result<(), Response>,
) -> Response {
    let ok = |account: Option<Json>| {
        let mut body = json!({"ok": true, "api": crate::API_VERSION});
        match account {
            Some(a) => body["account"] = a,
            None => body["requested"] = json!(true),
        }
        Response::json(200, &body)
    };
    if request.method == "GET" && rest.is_empty() {
        return ok(Some(hub.get(TOPIC).unwrap_or(Json::Null)));
    }
    if request.method != "POST" {
        return error_response(404, "not_found", "no such Plex route");
    }
    if request.header(CLIENT_HEADER) != Some(UI_CLIENT) {
        return error_response(
            403,
            "not_the_ui",
            "only the TV's own Settings signs the TV in to Plex",
        );
    }
    if let Err(refused) = owner {
        return refused;
    }
    let body = serde_json::from_slice::<Json>(&request.body).unwrap_or(Json::Null);
    let cmd = match rest {
        ["signin"] => Cmd::SignIn,
        ["cancel"] => Cmd::Cancel,
        ["refresh"] => Cmd::Refresh,
        ["signout"] => Cmd::SignOut,
        ["server"] => match body["id"].as_str().filter(|id| plain(id)) {
            Some(id) => Cmd::Choose(id.to_owned()),
            None => {
                return error_response(400, "bad_request", "the body is {\"id\": a server's id}");
            }
        },
        _ => return error_response(404, "not_found", "no such Plex route"),
    };
    account.send(cmd);
    ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::os::unix::fs::PermissionsExt;

    /// A request plex.tv was sent.
    #[derive(Clone)]
    struct Seen {
        method: String,
        url: String,
        headers: Vec<(String, String)>,
    }

    /// plex.tv as a list of answers in the order they are asked for, with every request kept.
    #[derive(Default)]
    struct Plex {
        answers: Mutex<VecDeque<(u16, String)>>,
        seen: Mutex<Vec<Seen>>,
    }

    impl Plex {
        fn answers(list: &[(u16, &str)]) -> Self {
            let p = Self::default();
            p.answers
                .lock()
                .extend(list.iter().map(|(s, b)| (*s, (*b).to_owned())));
            p
        }

        fn header(&self, i: usize, name: &str) -> Option<String> {
            self.seen.lock()[i]
                .headers
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
        }
    }

    impl Net for Plex {
        fn send(
            &self,
            method: &str,
            url: &str,
            headers: &[(&str, &str)],
        ) -> Result<(u16, Vec<u8>), String> {
            self.seen.lock().push(Seen {
                method: method.into(),
                url: url.into(),
                headers: headers
                    .iter()
                    .map(|(n, v)| ((*n).into(), (*v).into()))
                    .collect(),
            });
            self.answers
                .lock()
                .pop_front()
                .map(|(s, b)| (s, b.into_bytes()))
                .ok_or_else(|| "nothing answered".into())
        }
    }

    const PIN: &str = r#"{"id": 260805706, "code": "WBVQ", "expiresIn": 900, "authToken": null}"#;
    const LINKED: &str = r#"{"id": 260805706, "code": "WBVQ", "authToken": "acct-tk"}"#;
    const USER: &str = r#"{"username": "nitin", "title": "Nitin"}"#;

    /// The account's resources as plex.tv lists them: a server in Docker names its bridge's gateway as a local connection before its LAN
    /// address, as BeastNAS's did (2026-11-05).
    fn resources() -> String {
        let server = |id: &str, name: &str, owned: bool, ip: &str| {
            json!({"name": name, "provides": "server", "owned": owned, "clientIdentifier": id, "accessToken": format!("tk-{id}"),
                   "connections": [{"protocol": "https", "address": "203.0.113.9", "port": 32400, "local": false, "relay": false},
                                   {"protocol": "https", "address": "172.17.0.1", "port": 32400, "local": true, "relay": false},
                                   {"protocol": "https", "address": ip, "port": 32400, "local": true, "relay": false}]})
        };
        json!([{"name": "Phone", "provides": "client", "owned": true, "clientIdentifier": "p", "accessToken": "tk-p", "connections": []},
               server("beast", "BeastNAS", true, "192.168.1.69"),
               server("friend", "Friend's", false, "192.168.1.70"),
               server("den", "Den", true, "192.168.1.80")])
        .to_string()
    }

    fn machine(dir: &Path) -> Machine {
        Machine {
            own: || vec![Ipv4Addr::new(192, 168, 1, 242)],
            ..Machine::open(dir.join("plex")).unwrap()
        }
    }

    #[test]
    fn a_code_is_linked_by_a_phone_and_the_owned_home_servers_are_kept_with_no_token_in_the_topic()
    {
        let dir = tempfile::tempdir().unwrap();
        let mut m = machine(dir.path());
        let res = resources();
        let plex = Plex::answers(&[
            (201, PIN),
            (200, PIN),
            (200, LINKED),
            (200, USER),
            (200, &res),
        ]);
        m.sign_in(&plex, 1_000);
        let view = m.view();
        assert_eq!(
            (view["state"].as_str(), view["code"].as_str()),
            (Some("code"), Some("WBVQ"))
        );
        assert_eq!(view["url"], "https://plex.tv/link/?pin=WBVQ");
        assert_eq!(view["expiresAt"], 901_000);
        let id = std::fs::read_to_string(dir.path().join("plex/client-id")).unwrap();
        assert!(
            id.starts_with("lumatv-") && id.len() == 23,
            "a client identifier of its own: {id}"
        );
        assert_eq!(
            (
                plex.header(0, "X-Plex-Product").as_deref(),
                plex.header(0, "X-Plex-Client-Identifier")
            ),
            (Some("LumaTV"), Some(id.clone()))
        );

        m.poll(&plex, 3_000);
        assert!(m.polling(), "not linked yet");
        m.poll(&plex, 5_000);
        let view = m.view();
        assert_eq!(
            (view["state"].as_str(), view["user"].as_str()),
            (Some("signed-in"), Some("nitin"))
        );
        assert_eq!(
            view["server"],
            json!({"id": "beast", "name": "BeastNAS", "address": "192.168.1.69:32400"})
        );
        assert_eq!(
            view["servers"].as_array().unwrap().len(),
            2,
            "owned servers only: {view}"
        );
        assert!(
            !view.to_string().contains("tk"),
            "no token in the topic: {view}"
        );
        let seen = plex.seen.lock().clone();
        assert_eq!(seen[1].url, "https://plex.tv/api/v2/pins/260805706");
        assert_eq!(plex.header(3, "X-Plex-Token").as_deref(), Some("acct-tk"));
        assert!(
            seen[4]
                .url
                .starts_with("https://clients.plex.tv/api/v2/resources")
        );

        let server = m.server().unwrap();
        assert_eq!(
            (server.address.as_str(), server.token()),
            ("192.168.1.69:32400", "tk-beast")
        );
        assert!(
            !format!("{server:?}").contains("tk"),
            "Debug never says the token"
        );
        let file = dir.path().join("plex/account.json");
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );

        // the chooser: another of the account's servers, kept across a restart; a server it does not have is refused
        assert!(m.choose("den") && !m.choose("friend"));
        let again = machine(dir.path());
        assert_eq!(again.server().map(|s| s.name), Some("Den".into()));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("plex/client-id")).unwrap(),
            id,
            "the same device every time"
        );

        // a new code while signed in: the server stays in use until the code is linked, and Cancel keeps the account
        m.sign_in(&Plex::answers(&[(201, PIN)]), 0);
        assert_eq!(
            (m.view()["state"].as_str(), m.server().map(|s| s.name)),
            (Some("code"), Some("Den".into()))
        );
        m.cancel();
        assert_eq!(
            (m.view()["state"].as_str(), m.server().map(|s| s.name)),
            (Some("signed-in"), Some("Den".into()))
        );
        m.cancel();
        assert_eq!(
            m.view()["state"],
            "signed-in",
            "Cancel with no code shown changes nothing"
        );
    }

    #[test]
    fn a_code_that_expires_says_so_and_signs_nothing_in() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = machine(dir.path());
        let plex = Plex::answers(&[(201, PIN), (404, "")]);
        m.sign_in(&plex, 0);
        m.poll(&plex, 2_000);
        let view = m.view();
        assert_eq!(view["state"], "signed-out");
        assert!(
            view["problem"]
                .as_str()
                .unwrap()
                .starts_with("The code expired")
        );
        // a code past its time is not asked after at all
        m.sign_in(&Plex::answers(&[(201, PIN)]), 0);
        m.poll(&Plex::default(), 900_000);
        assert_eq!(m.view()["state"], "signed-out");
        assert!(!dir.path().join("plex/account.json").exists());
    }

    #[test]
    fn signing_out_revokes_the_token_and_forgets_it_even_when_plex_tv_cannot_be_reached() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = machine(dir.path());
        m.keep(Account {
            token: "acct-tk".into(),
            user: "nitin".into(),
            ..Account::default()
        });
        let plex = Plex::answers(&[(204, "")]);
        m.sign_out(&plex);
        let seen = plex.seen.lock().clone();
        assert_eq!(
            (seen[0].method.as_str(), seen[0].url.as_str()),
            ("DELETE", "https://plex.tv/api/v2/users/signout")
        );
        assert_eq!(plex.header(0, "X-Plex-Token").as_deref(), Some("acct-tk"));
        assert_eq!(m.view(), json!({"state": "signed-out"}));
        assert!(!dir.path().join("plex/account.json").exists());

        m.keep(Account {
            token: "acct-tk".into(),
            ..Account::default()
        });
        m.sign_out(&Plex::default());
        assert_eq!(m.view()["state"], "signed-out");
        assert!(
            m.view()["problem"]
                .as_str()
                .unwrap()
                .contains("plex.tv/devices")
        );
        assert!(!dir.path().join("plex/account.json").exists());
    }

    #[test]
    fn guide_datas_server_and_token_become_the_accounts_and_its_server_keeps_its_address() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let settings = SettingsStore::new(paths.settings_dir(), Arc::new(crate::hub::Hub::new(1)));
        std::fs::create_dir_all(paths.settings_dir()).unwrap();
        std::fs::write(
            paths.settings_dir().join("guide-data.json"),
            r#"{"owner": "guide-data", "schemaRevision": 1, "version": 3, "values": {"plex.address": {"type": "text", "value": "http://192.168.1.80:32400"},
               "zip": {"type": "text", "value": "30311"}}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("guide")).unwrap();
        std::fs::write(
            dir.path().join("guide/secrets.json"),
            r#"{"plex.token": "acct-tk", "sd.password": "pw"}"#,
        )
        .unwrap();

        migrate(&paths, &settings);
        let secrets = std::fs::read_to_string(dir.path().join("guide/secrets.json")).unwrap();
        assert!(
            !secrets.contains("acct-tk") && secrets.contains("pw"),
            "Guide data keeps its other secrets: {secrets}"
        );
        let stored = settings.stored("guide-data").unwrap();
        assert!(stored.get("plex.address").is_none() && stored.get("zip").is_some());
        let mut m = machine(dir.path());
        assert_eq!(
            m.server().map(|s| (s.address, s.token)),
            Some(("192.168.1.80:32400".into(), "acct-tk".into()))
        );

        // the first read names the account and finds the server at that address
        let res = resources();
        m.refresh(&Plex::answers(&[(200, USER), (200, &res)]), 0);
        assert_eq!(m.view()["server"]["name"], "Den");
        assert_eq!(m.view()["user"], "nitin");
        // nothing to take over a second time
        migrate(&paths, &settings);
        assert_eq!(
            machine(dir.path()).server().map(|s| s.id),
            Some("den".into())
        );
    }
}
