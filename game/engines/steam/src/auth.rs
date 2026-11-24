//! Signing in by QR code, and keeping the sign-in alive.
//!
//! Steam's authentication service runs the flow. `BeginAuthSessionViaQR` gives an address to show as a QR code,
//! `PollAuthSessionStatus` is asked every few seconds until the owner approves in the Steam phone app, and approval hands back a
//! long-lived refresh token. That token, the account's id and the latest access token are kept, in `token.json`. The web API wants an
//! access token, which is made from the refresh token when none is kept that still has time left, so a start does not ask Steam for a
//! new one. Steam also returns the login name with an approval; it is never read, so it cannot be kept or shown.

use crate::http::{self, Http, HttpError, Resp};
use crate::{Adapter, Clock, files};
use lumagame_core::ids::TargetId;
use lumagame_core::ipc::{AuthFail, AuthState, Event};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::io;
use std::path::Path;

pub const TOKEN_FILE: &str = "token.json";

/// How long the whole attempt lasts, code rotations included.
const ATTEMPT_MS: u64 = 300_000;
/// Polls that may fail one after the other before the attempt gives up; each is tried again at the next interval.
const POLL_RETRIES: u8 = 3;
/// An access token with less than this left is renewed before it is used.
const RENEW_MARGIN_S: u64 = 60;
/// After Steam refuses to renew the access token, no further renewal is asked for this long. The refusal was seen on the TV for a
/// sign-in that Steam had accepted earlier the same day, and every extra request counts against the account.
const RENEW_BACKOFF_S: u64 = 1800;
/// A read refused with a reused access token forces a renewal at most this often.
const FORCED_RENEW_EVERY_S: u64 = 1800;
/// Renewals refused without one that worked for this long retire the saved sign-in. Steam also answers a valid sign-in with a
/// refusal, so only a long unbroken run of them is taken to mean it was revoked.
const REFUSED_FOR_S: u64 = 7 * 86_400;
/// The unix time of the first renewal refused since the last one that worked, kept in the cache directory.
pub const REFUSED_FILE: &str = "renew-refused";
const DEFAULT_INTERVAL_S: u64 = 5;
const APPROVAL_HINT: &str = "Approve the sign-in in the Steam app";
/// What the owner's Steam app calls this sign-in.
const DEVICE_NAME: &str = "LumaGame TV";
/// `k_EAuthTokenPlatformType_MobileApp`, with `k_EOSType_AndroidUnknown` and Steam's gaming-device number for a phone. Steam renews
/// an access token over HTTP only for a refresh token of this kind: a SteamClient one is refused (error 63) unless the request
/// comes over a logged-on client connection, and a WebBrowser one is denied (as of 2025-04-30, per node-steam-session).
const PLATFORM_TYPE: u32 = 3;
const OS_TYPE: i32 = -500;
const GAMING_DEVICE_TYPE: u32 = 528;
const WEBSITE_ID: &str = "Mobile";
/// The audience a mobile app's tokens name besides `web`; a refresh token without it cannot be renewed over HTTP.
const MOBILE_AUDIENCE: &str = "mobile";
/// Steam's result codes that mean the owner (or Steam) said no: access denied, and the request was refused.
const ACCESS_DENIED: i32 = 15;
const REFUSED: i32 = 63;

/// Why Steam gave no usable answer. Said in fixed words and numbers, never with what was sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trouble {
    Network(HttpError),
    Status(u16),
    Eresult(Option<i32>),
    Unreadable,
}

impl Trouble {
    /// What was wrong with a reply that was not a success.
    pub fn of(resp: &Resp) -> Self {
        if resp.status == 200 { Self::Eresult(resp.eresult) } else { Self::Status(resp.status) }
    }
}

impl fmt::Display for Trouble {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(error) => write!(f, "{error}"),
            Self::Status(status) => write!(f, "Steam answered with HTTP {status}"),
            Self::Eresult(Some(code)) => write!(f, "Steam answered with error {code}"),
            Self::Eresult(None) => f.write_str("Steam sent no result code"),
            Self::Unreadable => f.write_str("Steam sent an answer the TV could not read"),
        }
    }
}

/// Why there is no access token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Steam no longer accepts the saved sign-in: the owner has to sign in again.
    Expired,
    /// A data endpoint refused this read, not the saved account sign-in.
    ReadDenied,
    /// Steam refused to renew the access token just now. The saved sign-in is kept and nothing is shown to the owner; a renewal is
    /// tried again later and on the next start.
    Renewal,
    Trouble(Trouble),
}

/// What a token says about itself. Its signature is Steam's to check, not ours.
pub struct Claims {
    /// The account's SteamID64, in decimal.
    pub steamid: String,
    /// Unix seconds.
    pub exp: Option<u64>,
    /// Who the token is for (`web`, `mobile`, `client`, `renew`, `derive`).
    pub aud: Vec<String>,
}

pub fn claims(token: &str) -> Option<Claims> {
    if token.len() > 8192 {
        return None;
    }
    let mut parts = token.split('.');
    let (_header, payload, _signature, None) = (parts.next()?, parts.next()?, parts.next()?, parts.next()) else {
        return None;
    };
    let json: Value = serde_json::from_slice(&base64url(payload)?).ok()?;
    let sub = digits(json.get("sub")?)?.parse::<u64>().ok().filter(|sub| *sub != 0)?;
    let aud = match json.get("aud") {
        Some(Value::String(name)) => vec![name.clone()],
        Some(Value::Array(names)) => names.iter().filter_map(|name| name.as_str().map(str::to_owned)).collect(),
        _ => Vec::new(),
    };
    Some(Claims { steamid: sub.to_string(), exp: json.get("exp").and_then(Value::as_u64), aud })
}

/// A number written as digits, whether Steam sent it as a string or as a JSON number.
fn digits(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() && text.len() <= 20 && text.bytes().all(|byte| byte.is_ascii_digit()) => Some(text.clone()),
        Value::Number(number) => number.as_u64().map(|number| number.to_string()),
        _ => None,
    }
}

/// The URL-safe base64 a token's parts are written in, with or without padding.
fn base64url(text: &str) -> Option<Vec<u8>> {
    let text = text.trim_end_matches('=');
    if text.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut bits, mut held) = (0u32, 0u32);
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        bits = (bits << 6) | u32::from(value);
        held += 6;
        if held >= 8 {
            held -= 8;
            out.push((bits >> held) as u8);
            bits &= (1 << held) - 1;
        }
    }
    Some(out)
}

/// What `token.json` holds. The login name is not in it. The access token is the latest one made, kept so that a start can use it.
#[derive(Deserialize)]
struct TokenIn {
    refresh_token: String,
    steamid: String,
    #[serde(default)]
    persona: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
}

#[derive(Serialize)]
struct TokenOut<'a> {
    refresh_token: &'a str,
    steamid: &'a str,
    persona: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    access_token: Option<&'a str>,
}

/// An access token and its end of life, when it is this account's and has more than the renewal margin left.
fn usable_access(token: String, steamid: &str, now: u64) -> Option<(String, u64)> {
    let claims = claims(&token).filter(|claims| claims.steamid == steamid)?;
    let exp = claims.exp.filter(|exp| *exp > now + RENEW_MARGIN_S)?;
    Some((token, exp))
}

/// The signed-in account. It has no `Debug`: it holds the keys to the account.
pub struct Account {
    pub refresh_token: String,
    /// The account's SteamID64 in decimal, which is also its target id.
    pub steamid: String,
    /// The profile name, when Steam's community page gave one.
    pub persona: Option<String>,
    /// The refresh token's own end of life, from its claims.
    exp: Option<u64>,
    /// Whether the refresh token was made for the mobile app, the only kind Steam renews over HTTP.
    mobile: bool,
    /// Steam no longer accepts the refresh token. The file is kept: the library saved with it is still worth showing.
    pub expired: bool,
    /// An access token and its end of life; kept in `token.json` with the refresh token.
    access: Option<(String, u64)>,
    /// No renewal is asked for before this unix time, after Steam refused one.
    renew_after: u64,
    /// A refused read forces no renewal before this unix time.
    forced_after: u64,
}

impl Account {
    fn new(refresh_token: String, claims: Claims, persona: Option<String>, now: u64) -> Self {
        let expired = claims.exp.is_some_and(|exp| exp <= now);
        let mobile = claims.aud.iter().any(|name| name == MOBILE_AUDIENCE);
        Self { refresh_token, steamid: claims.steamid, persona, exp: claims.exp, mobile, expired, access: None, renew_after: 0, forced_after: 0 }
    }

    /// The saved sign-in, when there is one that can be used: its token must say it belongs to the account the file names.
    pub fn load(dir: &Path, now: u64) -> Option<Self> {
        let file: TokenIn = serde_json::from_slice(&files::read(dir, TOKEN_FILE, 64 << 10)?).ok()?;
        let claims = claims(&file.refresh_token).filter(|claims| claims.steamid == file.steamid)?;
        let access = file.access_token.and_then(|token| usable_access(token, &file.steamid, now));
        let mut account = Self::new(file.refresh_token, claims, file.persona.as_deref().and_then(clean_name), now);
        account.access = access;
        Some(account)
    }

    pub fn save(&self, dir: &Path) -> io::Result<()> {
        let file = TokenOut {
            refresh_token: &self.refresh_token,
            steamid: &self.steamid,
            persona: self.persona.as_deref(),
            access_token: self.access.as_ref().map(|(token, _)| token.as_str()),
        };
        files::write(dir, TOKEN_FILE, &serde_json::to_vec(&file).map_err(io::Error::other)?)
    }

    pub fn name(&self) -> String {
        self.persona.clone().unwrap_or_else(|| "Steam account".to_owned())
    }

    pub fn target_id(&self) -> TargetId {
        TargetId::new(self.steamid.clone()).expect("a Steam id is a valid target id")
    }
}

/// A name for display: no control characters, at most 64 of them, and something left.
pub fn clean_name(raw: &str) -> Option<String> {
    let name: String = raw.chars().filter(|c| !c.is_control()).take(64).collect();
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// The profile name in a community profile's XML: `<steamID><![CDATA[Name]]></steamID>`. Not `<steamID64>`, which is the number.
pub fn persona_name(body: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(body);
    let rest = &text[text.find("<steamID>")? + "<steamID>".len()..];
    let inner = rest[..rest.find("</steamID>")?].trim();
    match inner.strip_prefix("<![CDATA[").and_then(|cdata| cdata.strip_suffix("]]>")) {
        Some(name) => clean_name(name),
        None => clean_name(&inner.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")),
    }
}

/// A sign-in in progress.
pub struct Attempt {
    /// The `auth` request, which every event of the attempt answers.
    seq: u64,
    client_id: String,
    request_id: String,
    interval_ms: u64,
    next_poll: u64,
    deadline: u64,
    /// Polls that failed one after the other.
    failures: u8,
    approval_shown: bool,
}

impl Attempt {
    /// Seconds left of the whole attempt, not of the code on screen: the app takes its own deadline from every code, and
    /// a rotation must not give the sign-in a fresh five minutes.
    fn expires_s(&self, now: u64) -> u32 {
        let left = self.deadline.saturating_sub(now).div_ceil(1000).clamp(1, ATTEMPT_MS / 1000);
        u32::try_from(left).unwrap_or(u32::MAX)
    }
}

struct Begun {
    client_id: String,
    challenge_url: String,
    request_id: String,
    interval_s: u64,
}

fn read_begin(result: Result<Resp, HttpError>) -> Result<Begun, Trouble> {
    #[derive(Deserialize)]
    struct Body {
        response: Fields,
    }
    #[derive(Deserialize)]
    struct Fields {
        client_id: Option<Value>,
        challenge_url: Option<String>,
        request_id: Option<String>,
        interval: Option<f64>,
    }
    let resp = result.map_err(Trouble::Network)?;
    if !resp.succeeded() {
        return Err(Trouble::of(&resp));
    }
    let Body { response } = serde_json::from_slice(&resp.body).map_err(|_| Trouble::Unreadable)?;
    let client_id = response.client_id.as_ref().and_then(digits).ok_or(Trouble::Unreadable)?;
    let challenge_url = response.challenge_url.filter(|url| shown_url(url)).ok_or(Trouble::Unreadable)?;
    let request_id = response.request_id.filter(|id| !id.is_empty() && id.len() <= 256).ok_or(Trouble::Unreadable)?;
    // A float or an int, and never so small that polling is a flood or so large that the code lapses between polls.
    let interval_s = response.interval.map_or(DEFAULT_INTERVAL_S, |seconds| (seconds.round() as u64).clamp(2, 30));
    Ok(Begun { client_id, challenge_url, request_id, interval_s })
}

/// An address fit to put in a QR code: https, one line, not absurdly long.
fn shown_url(url: &str) -> bool {
    url.starts_with("https://") && url.len() <= 512 && !url.chars().any(|c| c.is_control() || c.is_whitespace())
}

enum Poll {
    /// Steam can replace the poll id and the displayed challenge independently.
    Pending {
        client_id: Option<String>,
        challenge_url: Option<String>,
        scanned: bool,
    },
    Approved { refresh: String, access: Option<String> },
    Denied,
    Trouble(Trouble),
}

fn read_poll(result: Result<Resp, HttpError>) -> Poll {
    #[derive(Deserialize)]
    struct Body {
        response: Fields,
    }
    #[derive(Deserialize)]
    struct Fields {
        new_client_id: Option<Value>,
        new_challenge_url: Option<String>,
        had_remote_interaction: Option<bool>,
        refresh_token: Option<String>,
        access_token: Option<String>,
    }
    let resp = match result {
        Ok(resp) => resp,
        Err(error) => return Poll::Trouble(Trouble::Network(error)),
    };
    if resp.status != 200 {
        return Poll::Trouble(Trouble::Status(resp.status));
    }
    match resp.eresult {
        Some(1) => {}
        Some(ACCESS_DENIED | REFUSED) => return Poll::Denied,
        other => return Poll::Trouble(Trouble::Eresult(other)),
    }
    let Ok(Body { response }) = serde_json::from_slice(&resp.body) else {
        return Poll::Trouble(Trouble::Unreadable);
    };
    if let Some(refresh) = response.refresh_token.filter(|token| !token.is_empty()) {
        return Poll::Approved { refresh, access: response.access_token };
    }
    Poll::Pending {
        client_id: response.new_client_id.as_ref().and_then(digits),
        challenge_url: response.new_challenge_url.filter(|url| shown_url(url)),
        scanned: response.had_remote_interaction == Some(true),
    }
}

impl<H: Http, C: Clock> Adapter<H, C> {
    /// Starts a sign-in. One already running is dropped without a word: the app asked for a new one.
    pub(crate) fn begin_auth(&mut self, seq: u64) {
        self.attempt = None;
        let started = self.clock.mono_ms();
        // Steam reads the device from `device_details` and the older flat fields alike; a form cannot carry the nested one.
        let request = serde_json::json!({
            "device_friendly_name": DEVICE_NAME,
            "platform_type": PLATFORM_TYPE,
            "website_id": WEBSITE_ID,
            "device_details": {
                "device_friendly_name": DEVICE_NAME,
                "platform_type": PLATFORM_TYPE,
                "os_type": OS_TYPE,
                "gaming_device_type": GAMING_DEVICE_TYPE,
            },
        });
        let body = http::form(&[("input_json", &request.to_string())]);
        let reply = self.http.post_form(&http::api("/IAuthenticationService/BeginAuthSessionViaQR/v1/"), &http::MOBILE_APP, &body, http::SMALL_BODY);
        match read_begin(reply) {
            Ok(begun) => {
                let now = self.clock.mono_ms();
                let interval_ms = begun.interval_s * 1000;
                let attempt = Attempt {
                    seq,
                    client_id: begun.client_id,
                    request_id: begun.request_id,
                    interval_ms,
                    next_poll: now + interval_ms,
                    deadline: started + ATTEMPT_MS,
                    failures: 0,
                    approval_shown: false,
                };
                let expires_s = attempt.expires_s(now);
                self.answer(seq, Event::Auth { state: AuthState::Qr { url: begun.challenge_url, expires_s } });
                self.attempt = Some(attempt);
                self.note("sign-in: code shown");
            }
            Err(trouble) => self.fail_auth(seq, AuthFail::Network, &trouble.to_string()),
        }
    }

    /// Stops the sign-in the app is showing. With none running there is nothing to say.
    pub(crate) fn cancel_auth(&mut self, seq: u64) {
        if self.attempt.take().is_some() {
            self.note("sign-in: cancelled");
            self.answer(seq, Event::Auth { state: AuthState::Failed { reason: AuthFail::Cancelled, message: "Sign-in cancelled".to_owned() } });
        }
    }

    /// Milliseconds until the sign-in in progress next has to be looked at.
    pub(crate) fn auth_wake(&self) -> Option<u64> {
        let attempt = self.attempt.as_ref()?;
        Some(attempt.next_poll.min(attempt.deadline).saturating_sub(self.clock.mono_ms()))
    }

    /// A final check at the deadline can collect an approval made since the last poll.
    pub(crate) fn poll_due(&mut self) {
        let Some(attempt) = &self.attempt else { return };
        let now = self.clock.mono_ms();
        if now >= attempt.next_poll.min(attempt.deadline) {
            self.poll_once();
        }
    }

    fn poll_once(&mut self) {
        let Some(mut attempt) = self.attempt.take() else { return };
        let body = http::form(&[("client_id", &attempt.client_id), ("request_id", &attempt.request_id)]);
        let reply = self.http.post_form(&http::api("/IAuthenticationService/PollAuthSessionStatus/v1/"), &http::MOBILE_APP, &body, http::SMALL_BODY);
        match read_poll(reply) {
            Poll::Approved { refresh, access } => self.approved(attempt.seq, refresh, access),
            Poll::Denied => self.fail_auth(attempt.seq, AuthFail::Denied, "Sign-in was declined"),
            Poll::Trouble(trouble) => {
                attempt.failures += 1;
                if attempt.failures > POLL_RETRIES || self.clock.mono_ms() >= attempt.deadline {
                    self.fail_auth(attempt.seq, AuthFail::Network, &trouble.to_string());
                } else {
                    self.note(&format!("sign-in: a check failed ({trouble}), {} of {POLL_RETRIES} retries used", attempt.failures));
                    attempt.next_poll = self.clock.mono_ms() + attempt.interval_ms;
                    self.attempt = Some(attempt);
                }
            }
            Poll::Pending { client_id, challenge_url, scanned } => {
                let now = self.clock.mono_ms();
                if now >= attempt.deadline {
                    return self.fail_auth(attempt.seq, AuthFail::Expired, "The sign-in code expired");
                }
                attempt.failures = 0;
                if let Some(client_id) = client_id {
                    attempt.client_id = client_id;
                }
                if let Some(url) = challenge_url {
                    let expires_s = attempt.expires_s(now);
                    self.answer(attempt.seq, Event::Auth { state: AuthState::Qr { url, expires_s } });
                    self.note("sign-in: new code shown");
                }
                if scanned && !attempt.approval_shown {
                    attempt.approval_shown = true;
                    self.answer(attempt.seq, Event::Auth { state: AuthState::Approval { hint: Some(APPROVAL_HINT.to_owned()) } });
                    self.note("sign-in: scanned, waiting for approval");
                }
                attempt.next_poll = now + attempt.interval_ms;
                self.attempt = Some(attempt);
            }
        }
    }

    /// The owner approved: keeps the refresh token, looks up the profile name, and tells the app.
    fn approved(&mut self, seq: u64, refresh_token: String, access: Option<String>) {
        let Some(claims) = claims(&refresh_token) else {
            return self.fail_auth(seq, AuthFail::Network, &Trouble::Unreadable.to_string());
        };
        let now = self.clock.unix_s();
        let mut account = Account::new(refresh_token, claims, None, now);
        // The approval's own access token serves the first reads; later ones are renewed from the refresh token.
        account.access = access.and_then(|token| usable_access(token, &account.steamid, now));
        if let Err(error) = account.save(&self.data) {
            self.note(&format!("sign-in: the token could not be saved ({:?})", error.kind()));
            return self.fail_auth(seq, AuthFail::Network, "The Steam sign-in could not be saved on the TV");
        }
        // A new sign-in starts without the old one's refusals.
        let _ = files::remove(&self.cache, REFUSED_FILE);
        // A saved library belongs to the account it came from.
        if self.account.as_ref().is_none_or(|old| old.steamid != account.steamid) {
            self.snapshot = None;
            self.forget_avatar();
            if let Err(error) = files::remove(&self.cache, crate::library::CACHE_FILE) {
                self.note(&format!("sign-in: the earlier account's library could not be deleted ({:?})", error.kind()));
            }
        }
        let page = self.profile_page(&account.steamid);
        account.persona = page.as_deref().and_then(persona_name);
        if account.persona.is_some()
            && let Err(error) = account.save(&self.data)
        {
            self.note(&format!("sign-in: the profile name could not be saved ({:?})", error.kind()));
        }
        let target = account.target_id();
        self.account = Some(account);
        if let Some(page) = page {
            self.keep_avatar(&page);
        }
        self.answer(seq, Event::Auth { state: AuthState::Done { target } });
        self.targets_changed();
        self.note("sign-in: approved");
    }

    fn fail_auth(&mut self, seq: u64, reason: AuthFail, message: &str) {
        self.attempt = None;
        self.note(&format!("sign-in: failed ({})", reason.as_str()));
        self.answer(seq, Event::Auth { state: AuthState::Failed { reason, message: message.to_owned() } });
    }

    /// Steam no longer accepts the saved sign-in. The file stays; the app is told the account needs signing in again.
    pub(crate) fn expire(&mut self) {
        let Some(account) = self.account.as_mut().filter(|account| !account.expired) else { return };
        account.expired = true;
        account.access = None;
        self.targets_changed();
        self.note("sign-in: Steam no longer accepts the saved sign-in");
    }

    /// An access token for the web API: the one in memory while it has time left, else a new one from the refresh token.
    pub(crate) fn access_token(&mut self) -> Result<String, Refusal> {
        let now = self.clock.unix_s();
        let account = self.account.as_ref().ok_or(Refusal::Expired)?;
        if account.expired {
            return Err(Refusal::Expired);
        }
        if account.exp.is_some_and(|exp| exp <= now) {
            self.expire();
            return Err(Refusal::Expired);
        }
        if let Some((token, exp)) = &account.access
            && *exp > now + RENEW_MARGIN_S
        {
            return Ok(token.clone());
        }
        if !account.mobile {
            // Made by the earlier sign-in as a desktop client: Steam would refuse every renewal, so say so once and ask for a new QR.
            self.expire();
            return Err(Refusal::Expired);
        }
        if now < account.renew_after {
            return Err(Refusal::Renewal);
        }
        let body = http::form(&[("refresh_token", &account.refresh_token), ("steamid", &account.steamid)]);
        let reply = self.http.post_form(&http::api("/IAuthenticationService/GenerateAccessTokenForApp/v1/"), &http::MOBILE_APP, &body, http::SMALL_BODY);
        let resp = reply.map_err(|error| Refusal::Trouble(Trouble::Network(error)))?;
        if resp.status == 200 && matches!(resp.eresult, Some(ACCESS_DENIED | REFUSED)) {
            return Err(self.renewal_refused(now, resp.eresult));
        }
        if !resp.succeeded() {
            return Err(Refusal::Trouble(Trouble::of(&resp)));
        }
        #[derive(Deserialize)]
        struct Body {
            response: Fields,
        }
        #[derive(Deserialize)]
        struct Fields {
            access_token: Option<String>,
        }
        let token = serde_json::from_slice(&resp.body).ok().and_then(|body: Body| body.response.access_token).filter(|token| !token.is_empty());
        let token = token.ok_or(Refusal::Trouble(Trouble::Unreadable))?;
        // A token that does not say when it ends is used once.
        let exp = claims(&token).and_then(|claims| claims.exp).unwrap_or(now);
        if let Some(account) = self.account.as_mut() {
            account.access = Some((token.clone(), exp));
        }
        if let Some(account) = &self.account
            && let Err(error) = account.save(&self.data)
        {
            self.note(&format!("sign-in: the access token could not be saved ({:?})", error.kind()));
        }
        if let Err(error) = files::remove(&self.cache, REFUSED_FILE) {
            self.note(&format!("sign-in: the refusal record could not be deleted ({:?})", error.kind()));
        }
        self.note("sign-in: access token renewed");
        Ok(token)
    }

    /// Steam answered a renewal with access denied or refused. That is also what it said, on the TV, to a sign-in it accepted before and
    /// after, so the saved sign-in stays and is shown as it was; it is retired only when every renewal for a week has been refused.
    fn renewal_refused(&mut self, now: u64, code: Option<i32>) -> Refusal {
        if let Some(account) = self.account.as_mut() {
            account.renew_after = now + RENEW_BACKOFF_S;
        }
        let since = files::read(&self.cache, REFUSED_FILE, 32)
            .and_then(|bytes| String::from_utf8(bytes).ok()?.trim().parse::<u64>().ok())
            .filter(|since| *since <= now);
        let since = since.unwrap_or_else(|| {
            if let Err(error) = files::write(&self.cache, REFUSED_FILE, now.to_string().as_bytes()) {
                self.note(&format!("sign-in: the refusal could not be recorded ({:?})", error.kind()));
            }
            now
        });
        let code = code.map_or_else(|| "none".to_owned(), |code| code.to_string());
        if now - since >= REFUSED_FOR_S {
            self.note(&format!("sign-in: Steam refused every renewal for a week (error {code})"));
            self.expire();
            return Refusal::Expired;
        }
        self.note(&format!("sign-in: Steam refused to renew access (error {code}); the saved sign-in is kept"));
        Refusal::Renewal
    }

    /// A read of the web API with the account's access token. An access token that was in memory and is refused gets one renewal and
    /// one new try, since Steam may have ended it early; a refusal of a token just made is the answer for this read alone. The reply
    /// is returned as Steam gave it: judging a refusal is the caller's.
    pub(crate) fn read_with_access(&mut self, url: impl Fn(&str) -> String, limit: usize) -> Result<Resp, Refusal> {
        let now = self.clock.unix_s();
        let reused = self.account.as_ref().is_some_and(|account| account.access.as_ref().is_some_and(|(_, exp)| *exp > now + RENEW_MARGIN_S));
        let token = self.access_token()?;
        let resp = self.http.get(&url(&token), limit).map_err(|error| Refusal::Trouble(Trouble::Network(error)))?;
        let forced = reused
            && denied(&resp)
            && self.account.as_mut().is_some_and(|account| {
                let allowed = now >= account.forced_after;
                if allowed {
                    account.forced_after = now + FORCED_RENEW_EVERY_S;
                    account.access = None;
                }
                allowed
            });
        if !forced {
            return Ok(resp);
        }
        let token = self.access_token()?;
        self.http.get(&url(&token), limit).map_err(|error| Refusal::Trouble(Trouble::Network(error)))
    }
}

/// Whether a data endpoint refused the read: HTTP 401, or Steam's access-denied result code. For a per-game read this is also what
/// Steam answers for a game without stats or with private details, so it says nothing about the sign-in.
pub(crate) fn denied(resp: &Resp) -> bool {
    resp.status == 401 || (resp.status == 200 && resp.eresult == Some(ACCESS_DENIED))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{self, Rig, STEAMID, UNIX0};
    use lumagame_core::engine::AuthMethod;
    use lumagame_core::ipc::{ErrorCode, FromEngine, Link, Reach, Request, Target};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    const CODE: &str = "https://s.team/q/1/1111";
    const BEGIN: &str = r#"{"response":{"client_id":"1111","challenge_url":"https://s.team/q/1/1111","request_id":"cmVx+/==","interval":5.0,"version":1}}"#;
    const PENDING: &str = r#"{"response":{"had_remote_interaction":false}}"#;
    const SCANNED: &str = r#"{"response":{"had_remote_interaction":true}}"#;

    fn id() -> TargetId {
        TargetId::new(STEAMID).unwrap()
    }

    fn auth(re: u64, state: AuthState) -> FromEngine {
        FromEngine::answer(re, Event::Auth { state })
    }

    fn qr(re: u64, url: &str, expires_s: u32) -> FromEngine {
        auth(re, AuthState::Qr { url: url.to_owned(), expires_s })
    }

    fn failed(re: u64, reason: AuthFail, message: &str) -> FromEngine {
        auth(re, AuthState::Failed { reason, message: message.to_owned() })
    }

    fn rotated(client_id: &str) -> String {
        format!(r#"{{"response":{{"new_client_id":"{client_id}","new_challenge_url":"https://s.team/q/1/{client_id}","had_remote_interaction":false}}}}"#)
    }

    fn approved(refresh_token: &str) -> String {
        // What Steam really sends with an approval, the login name and an access token included.
        format!(r#"{{"response":{{"refresh_token":"{refresh_token}","access_token":"ACCESS","account_name":"login","had_remote_interaction":true}}}}"#)
    }

    fn refresh() -> String {
        fake::jwt(STEAMID, Some(UNIX0 + 86_400))
    }

    fn with_begin() -> Rig {
        let rig = Rig::ready();
        rig.http.on("BeginAuthSessionViaQR", [fake::ok(BEGIN)]);
        rig
    }

    fn start(rig: &mut Rig) -> Vec<FromEngine> {
        rig.send(Request::Auth { target: None, method: AuthMethod::Qr })
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn persona_target(name: &str) -> Target {
        Target { id: id(), name: name.to_owned(), detail: None, avatar: None, link: Link::Linked, reach: Reach::Online }
    }

    #[test]
    fn a_sign_in_runs_from_the_code_through_a_new_code_and_the_scan_to_the_account() {
        let mut rig = with_begin();
        rig.http.on(
            "PollAuthSessionStatus",
            [fake::ok(PENDING), fake::ok(&rotated("2222")), fake::ok(SCANNED), fake::ok(SCANNED), fake::ok(&approved(&refresh()))],
        );
        rig.http.on("steamcommunity.com", [fake::ok("<profile><steamID64>1</steamID64><steamID><![CDATA[Persona Name]]></steamID></profile>")]);

        assert_eq!(start(&mut rig), [qr(2, CODE, 300)]);
        let begin = &rig.http.to("BeginAuthSessionViaQR")[0];
        let sent: Value = serde_json::from_str(&percent_decode(begin.body.strip_prefix("input_json=").unwrap())).unwrap();
        let device = serde_json::json!({"device_friendly_name": "LumaGame TV", "platform_type": 3, "os_type": -500, "gaming_device_type": 528});
        assert_eq!(sent, serde_json::json!({"device_friendly_name": "LumaGame TV", "platform_type": 3, "website_id": "Mobile", "device_details": device}));
        let mobile: Vec<_> = http::MOBILE_APP.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())).collect();
        assert_eq!(begin.headers, mobile);
        assert_eq!(rig.adapter.next_wake(), Some(5_000));
        rig.clock.advance_ms(4_999);
        assert!(rig.tick().is_empty() && rig.http.count("PollAuthSessionStatus") == 0, "nothing is asked before the interval is up");

        rig.clock.advance_ms(1);
        assert!(rig.tick().is_empty(), "a poll that is still pending says nothing");
        assert_eq!(rig.http.to("PollAuthSessionStatus")[0].body, "client_id=1111&request_id=cmVx%2B%2F%3D%3D");

        assert_eq!(rig.wake(), [qr(2, "https://s.team/q/1/2222", 290)]);
        assert_eq!(rig.wake(), [auth(2, AuthState::Approval { hint: Some("Approve the sign-in in the Steam app".to_owned()) })]);
        assert!(rig.wake().is_empty(), "the approval is shown once");
        let polls = rig.http.to("PollAuthSessionStatus");
        assert_eq!(polls[1].body, "client_id=1111&request_id=cmVx%2B%2F%3D%3D");
        assert_eq!(polls[2].body, "client_id=2222&request_id=cmVx%2B%2F%3D%3D", "polling follows the new code and keeps the request id");

        let events = rig.wake();
        let persona = persona_target("Persona Name");
        assert_eq!(events, [auth(2, AuthState::Done { target: id() }), FromEngine::told(Event::Targets { targets: vec![persona] })]);
        assert_eq!(rig.adapter.next_wake(), None);
        assert!(rig.tick().is_empty());

        let file = fs::read_to_string(rig.data().join("token.json")).unwrap();
        let saved: Value = serde_json::from_str(&file).unwrap();
        assert_eq!(saved, serde_json::json!({ "refresh_token": refresh(), "steamid": STEAMID, "persona": "Persona Name" }));
        assert!(!file.contains("ACCESS") && !file.contains("login"), "only the refresh token is kept");
        assert_eq!((mode(&rig.data().join("token.json")), mode(&rig.data())), (0o600, 0o700));
        assert_eq!(rig.http.to("steamcommunity.com")[0].url, format!("https://steamcommunity.com/profiles/{STEAMID}/?xml=1"));

        // The account survives a restart.
        rig.restart();
        rig.hello();
        assert_eq!(rig.send(Request::Targets), [FromEngine::answer(2, Event::Targets { targets: vec![persona_target("Persona Name")] })]);
    }

    #[test]
    fn approval_without_a_profile_name_is_still_an_account() {
        let mut rig = with_begin();
        rig.http.on("PollAuthSessionStatus", [fake::ok(&approved(&refresh()))]);
        rig.http.on("steamcommunity.com", [fake::down()]);
        start(&mut rig);
        let events = rig.wake();
        assert_eq!(events[0], auth(2, AuthState::Done { target: id() }));
        assert_eq!(events[1], FromEngine::told(Event::Targets { targets: vec![persona_target("Steam account")] }));
        assert_eq!(serde_json::from_str::<Value>(&fs::read_to_string(rig.data().join("token.json")).unwrap()).unwrap()["persona"], Value::Null);
    }

    #[test]
    fn the_code_lapses_five_minutes_after_the_start() {
        let mut rig = with_begin();
        rig.http.on("PollAuthSessionStatus", [fake::ok(PENDING)]);
        start(&mut rig);
        rig.clock.advance_s(295);
        // Polls were due every five seconds; one tick sees only the last of them.
        assert!(rig.tick().is_empty());
        assert_eq!(rig.http.count("PollAuthSessionStatus"), 1);
        assert_eq!(rig.adapter.next_wake(), Some(5_000));
        rig.clock.advance_s(5);
        assert_eq!(rig.tick(), [failed(2, AuthFail::Expired, "The sign-in code expired")]);
        assert_eq!(rig.http.count("PollAuthSessionStatus"), 2, "the final check found no approval");
        assert_eq!(rig.adapter.next_wake(), None);
        assert!(rig.tick().is_empty());
    }

    #[test]
    fn final_poll_collects_approval_and_the_first_library_uses_its_access_token() {
        let mut rig = with_begin();
        let access = fake::jwt(STEAMID, Some(UNIX0 + 3_600));
        rig.http.on("PollAuthSessionStatus", [fake::ok(&approved(&refresh()).replace("ACCESS", &access))]);
        rig.http.on("GetOwnedGames", [fake::ok(&fake::owned(&[]))]);
        rig.http.on("steamcommunity.com", [fake::down()]);
        rig.http.on("IStoreService", [fake::down()]);
        start(&mut rig);
        rig.clock.advance_s(300);
        assert!(matches!(rig.tick()[0].event, Event::Auth { state: AuthState::Done { .. } }));
        assert!(matches!(library(&mut rig)[0].event, Event::Library { .. }));
        assert_eq!(rig.http.count("GenerateAccessTokenForApp"), 0);
        let saved: Value = serde_json::from_slice(&fs::read(rig.data().join(TOKEN_FILE)).unwrap()).unwrap();
        assert_eq!(saved["access_token"], access.as_str(), "the approval's access token is kept for the next start");
    }

    #[test]
    fn the_wake_is_whichever_comes_first_of_the_next_check_and_the_end() {
        let mut rig = with_begin();
        rig.http.on("BeginAuthSessionViaQR", [fake::ok(&BEGIN.replace("5.0", "30"))]);
        rig.http.on("PollAuthSessionStatus", [fake::ok(PENDING)]);
        start(&mut rig);
        rig.clock.advance_s(285);
        rig.tick();
        assert_eq!(rig.adapter.next_wake(), Some(15_000), "the next check would fall after the end");
    }

    #[test]
    fn cancelling_stops_the_checks_and_answers_the_cancel() {
        let mut rig = with_begin();
        rig.http.on("PollAuthSessionStatus", [fake::ok(PENDING)]);
        start(&mut rig);
        assert_eq!(rig.send(Request::AuthCancel), [failed(3, AuthFail::Cancelled, "Sign-in cancelled")]);
        assert_eq!(rig.adapter.next_wake(), None);
        rig.clock.advance_s(600);
        assert!(rig.tick().is_empty());
        assert_eq!(rig.http.count("PollAuthSessionStatus"), 0);
        assert!(rig.send(Request::AuthCancel).is_empty(), "cancelling nothing says nothing");
    }

    #[test]
    fn a_refusal_ends_the_sign_in_and_the_next_one_starts_clean() {
        for code in [15, 63] {
            let mut rig = with_begin();
            rig.http.on("PollAuthSessionStatus", [fake::eresult(code, r#"{"response":{}}"#)]);
            start(&mut rig);
            assert_eq!(rig.wake(), [failed(2, AuthFail::Denied, "Sign-in was declined")]);
            assert_eq!(rig.adapter.next_wake(), None);
            assert_eq!(start(&mut rig), [qr(3, CODE, 300)], "idle again after a failure");
        }
    }

    #[test]
    fn a_few_failed_checks_in_a_row_are_retried_and_one_more_ends_the_attempt() {
        let mut rig = with_begin();
        let bad_body = fake::ok("<html>");
        rig.http.on("PollAuthSessionStatus", [fake::eresult(3, "{}"), fake::down(), fake::status(503), bad_body]);
        start(&mut rig);
        assert!(rig.wake().is_empty() && rig.wake().is_empty() && rig.wake().is_empty());
        assert_eq!(rig.adapter.next_wake(), Some(5_000), "a failed check is asked again at the next interval");
        assert_eq!(rig.wake(), [failed(2, AuthFail::Network, "Steam sent an answer the TV could not read")]);
        assert_eq!(rig.http.count("PollAuthSessionStatus"), 4);
        assert_eq!(rig.adapter.next_wake(), None);
    }

    #[test]
    fn a_check_that_works_resets_the_count_of_failures() {
        let mut rig = with_begin();
        let steps = [fake::down(), fake::down(), fake::down(), fake::ok(PENDING), fake::down(), fake::down(), fake::down(), fake::eresult(2, "{}")];
        rig.http.on("PollAuthSessionStatus", steps);
        start(&mut rig);
        for _ in 0..7 {
            assert!(rig.wake().is_empty());
        }
        assert_eq!(rig.wake(), [failed(2, AuthFail::Network, "Steam answered with error 2")]);
    }

    #[test]
    fn each_kind_of_trouble_is_named_in_fixed_words() {
        let cases = [
            (fake::down(), "Steam could not be reached"),
            (Err(HttpError::Timeout), "Steam did not answer in time"),
            (fake::status(429), "Steam answered with HTTP 429"),
            (fake::eresult(29, "{}"), "Steam answered with error 29"),
            (Ok(Resp { status: 200, eresult: None, body: Vec::new() }), "Steam sent no result code"),
            (fake::ok(r#"{"response":{"client_id":"abc","challenge_url":"https://s.team/q","request_id":"r"}}"#), "Steam sent an answer the TV could not read"),
            (fake::ok(r#"{"response":{"client_id":"1","challenge_url":"http://s.team/q","request_id":"r"}}"#), "Steam sent an answer the TV could not read"),
            (fake::ok(r#"{"response":{"client_id":"1","challenge_url":"https://s.team/q","request_id":""}}"#), "Steam sent an answer the TV could not read"),
        ];
        for (reply, message) in cases {
            let mut rig = Rig::ready();
            rig.http.on("BeginAuthSessionViaQR", [reply]);
            assert_eq!(start(&mut rig), [failed(2, AuthFail::Network, message)]);
            assert_eq!(rig.adapter.next_wake(), None);
        }
    }

    #[test]
    fn the_interval_is_read_as_a_number_of_either_kind_and_kept_sensible() {
        for (interval, expected) in [("5", 5_000), ("5.0", 5_000), ("0.2", 2_000), ("600", 30_000), ("-3", 2_000), ("2.6", 3_000), ("null", 5_000)] {
            let mut rig = Rig::ready();
            rig.http.on("BeginAuthSessionViaQR", [fake::ok(&BEGIN.replace("5.0", interval))]);
            start(&mut rig);
            assert_eq!(rig.adapter.next_wake(), Some(expected), "{interval}");
        }
        let mut rig = Rig::ready();
        rig.http.on("BeginAuthSessionViaQR", [fake::ok(&BEGIN.replace(r#""interval":5.0,"#, ""))]);
        start(&mut rig);
        assert_eq!(rig.adapter.next_wake(), Some(5_000));
    }

    #[test]
    fn a_new_request_replaces_the_one_running_and_only_it_is_checked() {
        let mut rig = with_begin();
        rig.http.on("PollAuthSessionStatus", [fake::ok(PENDING), fake::ok(&approved(&refresh()))]);
        rig.http.on("steamcommunity.com", [fake::down()]);
        start(&mut rig);
        rig.http.on("BeginAuthSessionViaQR", [fake::ok(&BEGIN.replace("1111", "3333"))]);
        assert_eq!(start(&mut rig), [qr(3, "https://s.team/q/1/3333", 300)], "the first is dropped without a word");
        assert!(rig.wake().is_empty());
        assert_eq!(rig.http.to("PollAuthSessionStatus")[0].body, "client_id=3333&request_id=cmVx%2B%2F%3D%3D");
        assert_eq!(rig.http.count("PollAuthSessionStatus"), 1);
        assert_eq!(rig.wake()[0], auth(3, AuthState::Done { target: id() }));
    }

    #[test]
    fn replacement_poll_id_and_display_code_can_arrive_separately() {
        let mut rig = with_begin();
        let only_id = r#"{"response":{"new_client_id":"9999"}}"#;
        let only_url = r#"{"response":{"new_challenge_url":"https://s.team/q/1/9999"}}"#;
        rig.http.on("PollAuthSessionStatus", [fake::ok(only_id), fake::ok(only_url), fake::ok(PENDING)]);
        start(&mut rig);
        assert!(rig.wake().is_empty());
        let events = rig.wake();
        assert!(matches!(&events[..], [FromEngine { event: Event::Auth { state: AuthState::Qr { .. } }, .. }]));
        assert!(rig.wake().is_empty());
        assert!(rig.http.to("PollAuthSessionStatus")[1..].iter().all(|poll| poll.body.starts_with("client_id=9999&")));
    }

    #[test]
    fn an_approval_that_cannot_be_read_or_saved_fails_the_attempt_and_keeps_nothing() {
        let mut rig = with_begin();
        rig.http.on("PollAuthSessionStatus", [fake::ok(&approved("not-a-token"))]);
        start(&mut rig);
        assert_eq!(rig.wake(), [failed(2, AuthFail::Network, "Steam sent an answer the TV could not read")]);
        assert!(!rig.data().join("token.json").exists());
        assert_eq!(rig.adapter.next_wake(), None);

        let mut rig = with_begin();
        rig.http.on("PollAuthSessionStatus", [fake::ok(&approved(&refresh()))]);
        fs::write(rig.data(), b"in the way").unwrap();
        start(&mut rig);
        assert_eq!(rig.wake(), [failed(2, AuthFail::Network, "The Steam sign-in could not be saved on the TV")]);
        assert_eq!(rig.http.count("steamcommunity.com"), 0);
        assert_eq!(rig.send(Request::Targets), [FromEngine::answer(3, Event::Targets { targets: Vec::new() })]);
    }

    #[test]
    fn signing_in_as_someone_else_replaces_the_account_and_the_library_saved_for_the_first() {
        let mut rig = Rig::signed_in();
        files::write(&rig.cache(), "library.json", b"{}").unwrap();
        let other = "76561198000000002";
        rig.http.on("BeginAuthSessionViaQR", [fake::ok(BEGIN)]);
        rig.http.on("PollAuthSessionStatus", [fake::ok(&approved(&fake::jwt(other, Some(UNIX0 + 86_400))))]);
        rig.http.on("steamcommunity.com", [fake::ok("<profile><steamID>Other</steamID></profile>")]);
        start(&mut rig);
        let events = rig.wake();
        assert_eq!(events[0], auth(2, AuthState::Done { target: TargetId::new(other).unwrap() }));
        assert!(!rig.cache().join("library.json").exists());
        let saved: Value = serde_json::from_slice(&fs::read(rig.data().join("token.json")).unwrap()).unwrap();
        assert_eq!((saved["steamid"].as_str(), saved["persona"].as_str()), (Some(other), Some("Other")));
    }

    #[test]
    fn a_sign_in_again_clears_an_expired_account_and_keeps_the_saved_library() {
        let mut rig = Rig::signed_in_until(UNIX0 - 5);
        files::write(&rig.cache(), "library.json", b"{}").unwrap();
        let events = rig.send(Request::Targets);
        let [FromEngine { event: Event::Targets { targets }, .. }] = events.as_slice() else { panic!("{events:?}") };
        assert_eq!(targets[0].link, Link::Expired);
        rig.http.on("BeginAuthSessionViaQR", [fake::ok(BEGIN)]);
        rig.http.on("PollAuthSessionStatus", [fake::ok(&approved(&refresh()))]);
        rig.http.on("steamcommunity.com", [fake::down()]);
        start(&mut rig);
        let events = rig.wake();
        let FromEngine { event: Event::Targets { targets }, .. } = &events[1] else { panic!("{events:?}") };
        assert_eq!(targets[0].link, Link::Linked);
        assert!(rig.cache().join("library.json").exists(), "the same account keeps its saved library");
    }

    #[test]
    fn a_token_is_read_for_its_account_and_its_end_and_nothing_else() {
        let token = fake::jwt("76561198000000001", Some(1_900_000_000));
        let claims = claims(&token).unwrap();
        assert_eq!((claims.steamid.as_str(), claims.exp), ("76561198000000001", Some(1_900_000_000)));
        assert_eq!(super::claims(&fake::jwt("76561198000000001", None)).unwrap().exp, None);
        let payload = |json: &str| format!("e30.{}.c2ln", fake::b64url(json.as_bytes()));
        assert_eq!(super::claims(&payload(r#"{"sub":76561198000000001}"#)).unwrap().steamid, "76561198000000001");
        assert_eq!(super::claims(&payload(r#"{"sub":"0076561198000000001"}"#)).unwrap().steamid, "76561198000000001");
        assert_eq!(super::claims(&payload(r#"{"sub":"5","exp":"soon"}"#)).unwrap().exp, None);
        assert_eq!(claims.aud, ["web", "mobile"]);
        assert_eq!(super::claims(&payload(r#"{"sub":"5","aud":"web"}"#)).unwrap().aud, ["web"]);
        assert!(super::claims(&payload(r#"{"sub":"5","aud":7}"#)).unwrap().aud.is_empty());
        let padded = format!("e30.{}==.c2ln", fake::b64url(br#"{"sub":"5"}"#));
        assert_eq!(super::claims(&padded).unwrap().steamid, "5");

        let long = "9".repeat(21);
        let huge = format!("e30.{}.c2ln", fake::b64url(format!(r#"{{"sub":"{long}"}}"#).as_bytes()));
        let too_long = "a".repeat(9_000);
        for bad in [
            "",
            "x",
            "a.b",
            "a.b.c.d",
            "a..c",
            "a.!!!.c",
            "a.A.c",
            &too_long,
            &huge,
            &payload("not json"),
            &payload("[]"),
            &payload("{}"),
            &payload(r#"{"sub":null}"#),
            &payload(r#"{"sub":"abc"}"#),
            &payload(r#"{"sub":"0"}"#),
            &payload(r#"{"sub":"+5"}"#),
            &payload(r#"{"sub":-5}"#),
            &payload(r#"{"sub":1.5}"#),
            &payload(r#"{"sub":""}"#),
        ] {
            assert!(super::claims(bad).is_none(), "{}", &bad[..bad.len().min(40)]);
        }
    }

    #[test]
    fn a_profile_name_is_taken_from_the_name_element_and_cleaned() {
        let name = |xml: &str| persona_name(xml.as_bytes());
        assert_eq!(name("<profile><steamID64>7656</steamID64><steamID><![CDATA[Rabscuttle]]></steamID></profile>").as_deref(), Some("Rabscuttle"));
        assert_eq!(name("<steamID>  Plain &amp; &lt;Simple&gt; </steamID>").as_deref(), Some("Plain & <Simple>"));
        assert_eq!(name("<steamID><![CDATA[Tab\there\u{7}bell]]></steamID>").as_deref(), Some("Tabherebell"));
        assert_eq!(name(&format!("<steamID><![CDATA[{}]]></steamID>", "n".repeat(100))).map(|name| name.len()), Some(64));
        for none in ["", "<steamID64>7656</steamID64>", "<steamID><![CDATA[   ]]></steamID>", "<steamID>unclosed", "not xml"] {
            assert_eq!(name(none), None, "{none}");
        }
        assert_eq!(persona_name(&[0xff, 0xfe]), None);
    }

    fn percent_decode(text: &str) -> String {
        let bytes = text.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                out.push(u8::from_str_radix(&text[i + 1..i + 3], 16).unwrap());
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        String::from_utf8(out).unwrap()
    }

    fn library_rig(exp: u64) -> Rig {
        let rig = Rig::signed_in_until(exp);
        rig.http.on("GetOwnedGames", [fake::ok(&fake::owned(&[(10, "Ten", 1, 0)]))]);
        rig.http.on("IStoreService", [fake::down()]);
        rig
    }

    fn library(rig: &mut Rig) -> Vec<FromEngine> {
        rig.send(Request::Library { target: id(), cursor: None })
    }

    /// The owner's Refresh, then the library: Steam is asked whatever the age of the saved copy.
    fn refreshed(rig: &mut Rig) -> Vec<FromEngine> {
        rig.send(Request::Action { key: "refresh".into() });
        library(rig)
    }

    #[test]
    fn a_saved_access_token_is_used_by_the_next_start_until_a_minute_before_it_ends() {
        let mut rig = library_rig(UNIX0 + 30 * 86_400);
        let access = fake::jwt(STEAMID, Some(UNIX0 + 86_400));
        rig.http.on("GenerateAccessTokenForApp", [fake::ok(&format!(r#"{{"response":{{"access_token":"{access}"}}}}"#))]);
        library(&mut rig);
        let path = rig.data().join(TOKEN_FILE);
        let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["access_token"], access.as_str());
        assert_eq!(mode(&path), 0o600);
        assert!(rig.log.lines().iter().all(|line| !line.contains(&access)), "never logged");

        // Later starts, with the library due each time, renew nothing while the saved one has time.
        rig.restart();
        rig.hello();
        rig.clock.advance_s(7_200);
        library(&mut rig);
        assert_eq!(rig.http.count("GenerateAccessTokenForApp"), 1);
        assert!(rig.http.to("GetOwnedGames")[1].url.contains(&format!("access_token={access}")));

        // Under a minute from its end it is renewed instead.
        rig.restart();
        rig.hello();
        rig.clock.advance_s(86_400 - 7_200 - 30);
        library(&mut rig);
        assert_eq!(rig.http.count("GenerateAccessTokenForApp"), 2);
    }

    #[test]
    fn a_saved_access_token_of_another_account_is_not_used() {
        let mut rig = library_rig(UNIX0 + 30 * 86_400);
        let renewed = fake::jwt(STEAMID, Some(UNIX0 + 86_400));
        rig.http.on("GenerateAccessTokenForApp", [fake::ok(&format!(r#"{{"response":{{"access_token":"{renewed}"}}}}"#))]);
        let mut file: Value = serde_json::from_slice(&fs::read(rig.data().join(TOKEN_FILE)).unwrap()).unwrap();
        file["access_token"] = Value::from(fake::jwt("76561198000000009", Some(UNIX0 + 86_400)));
        fs::write(rig.data().join(TOKEN_FILE), file.to_string()).unwrap();
        rig.restart();
        rig.hello();
        library(&mut rig);
        assert_eq!(rig.http.count("GenerateAccessTokenForApp"), 1);
    }

    #[test]
    fn an_access_token_is_made_when_needed_and_used_until_a_minute_before_it_ends() {
        let mut rig = library_rig(UNIX0 + 30 * 86_400);
        let access = fake::jwt(STEAMID, Some(UNIX0 + 3_600));
        rig.http.on("GenerateAccessTokenForApp", [fake::ok(&format!(r#"{{"response":{{"access_token":"{access}"}}}}"#))]);
        library(&mut rig);
        let renew = rig.http.to("GenerateAccessTokenForApp");
        assert_eq!(renew.len(), 1);
        assert_eq!(renew[0].body, format!("refresh_token={}&steamid={STEAMID}", fake::jwt(STEAMID, Some(UNIX0 + 30 * 86_400))));
        assert!(
            rig.http.to("GetOwnedGames")[0]
                .url
                .contains(&format!("access_token={access}&steamid={STEAMID}&include_appinfo=1&include_played_free_games=1&format=json"))
        );

        rig.clock.advance_s(1_000);
        refreshed(&mut rig);
        assert_eq!(rig.http.count("GenerateAccessTokenForApp"), 1, "still good");
        rig.clock.advance_s(2_000);
        refreshed(&mut rig);
        assert_eq!(rig.http.count("GenerateAccessTokenForApp"), 1, "still good with more than a minute left");
        rig.clock.advance_s(560);
        refreshed(&mut rig);
        assert_eq!(rig.http.count("GenerateAccessTokenForApp"), 2, "under a minute left: renewed");
    }

    #[test]
    fn a_refused_renewal_keeps_the_sign_in_untouched_and_is_not_asked_again_for_half_an_hour() {
        for code in [15, 63] {
            let mut rig = library_rig(UNIX0 + 30 * 86_400);
            rig.http.on("GenerateAccessTokenForApp", [fake::eresult(code, r#"{"response":{}}"#)]);
            let events = library(&mut rig);
            let [FromEngine { re: Some(2), event: Event::Error { code: error, .. } }] = events.as_slice() else { panic!("{events:?}") };
            assert_ne!(*error, ErrorCode::SignIn, "nothing tells the owner to sign in");
            assert!(rig.log.lines().iter().any(|line| line.contains(&format!("(error {code})"))), "the code is logged");
            let events = rig.send(Request::Targets);
            let [FromEngine { event: Event::Targets { targets }, .. }] = events.as_slice() else { panic!("{events:?}") };
            assert_eq!(targets[0].link, Link::Linked);
            assert!(rig.data().join("token.json").exists() && rig.cache().join(REFUSED_FILE).exists());
            assert_eq!(rig.http.to("GenerateAccessTokenForApp")[0].headers, http::MOBILE_APP.map(|(name, value)| (name.to_owned(), value.to_owned())));
            rig.http.forget();
            library(&mut rig);
            assert!(rig.http.sent().is_empty(), "no request while Steam is given time");

            // Half an hour on Steam is asked again, and an answer clears the record.
            rig.clock.advance_s(RENEW_BACKOFF_S + 1);
            let access = fake::jwt(STEAMID, Some(UNIX0 + 86_400));
            rig.http.on("GenerateAccessTokenForApp", [fake::ok(&format!(r#"{{"response":{{"access_token":"{access}"}}}}"#))]);
            let events = library(&mut rig);
            assert!(matches!(events.as_slice(), [FromEngine { event: Event::Library { .. }, .. }]), "{events:?}");
            assert!(!rig.cache().join(REFUSED_FILE).exists());
        }
    }

    #[test]
    fn a_refusal_at_every_start_is_asked_again_at_the_next_one() {
        let mut rig = library_rig(UNIX0 + 30 * 86_400);
        rig.http.on("GenerateAccessTokenForApp", [fake::eresult(15, r#"{"response":{}}"#)]);
        library(&mut rig);
        rig.restart();
        rig.hello();
        library(&mut rig);
        assert_eq!(rig.http.count("GenerateAccessTokenForApp"), 2);
        let events = rig.send(Request::Targets);
        let [FromEngine { event: Event::Targets { targets }, .. }] = events.as_slice() else { panic!("{events:?}") };
        assert_eq!(targets[0].link, Link::Linked);
    }

    #[test]
    fn only_a_week_of_refused_renewals_retires_the_sign_in_and_the_file_stays() {
        let mut rig = library_rig(UNIX0 + 30 * 86_400);
        rig.http.on("GenerateAccessTokenForApp", [fake::eresult(15, r#"{"response":{}}"#)]);
        library(&mut rig);
        rig.clock.advance_s(REFUSED_FOR_S - 3_600);
        let events = library(&mut rig);
        assert!(matches!(events.as_slice(), [FromEngine { event: Event::Error { code, .. }, .. }] if *code != ErrorCode::SignIn), "{events:?}");
        rig.clock.advance_s(3_600);
        let events = library(&mut rig);
        let [FromEngine { re: Some(_), event: Event::Error { code: ErrorCode::SignIn, .. } }, FromEngine { re: None, event: Event::Targets { targets } }] =
            events.as_slice()
        else {
            panic!("{events:?}")
        };
        assert_eq!(targets[0].link, Link::Expired);
        assert!(rig.data().join("token.json").exists());
        // Once, not at every request; and with no more requests to Steam.
        rig.http.forget();
        library(&mut rig);
        assert!(rig.http.sent().is_empty());
    }

    #[test]
    fn a_saved_sign_in_made_as_a_desktop_client_is_retired_without_asking_steam() {
        let mut rig = Rig::new();
        rig.save_token(&fake::jwt_for(STEAMID, Some(UNIX0 + 30 * 86_400), &["client", "web"]), STEAMID, Some("Rig"));
        rig.hello();
        let events = library(&mut rig);
        let [FromEngine { re: Some(2), event: Event::Error { code: ErrorCode::SignIn, .. } }, FromEngine { re: None, event: Event::Targets { targets } }] =
            events.as_slice()
        else {
            panic!("{events:?}")
        };
        assert_eq!(targets[0].link, Link::Expired);
        assert!(rig.http.sent().is_empty());
    }

    #[test]
    fn a_refresh_token_past_its_own_end_is_expired_without_asking_steam() {
        let mut rig = library_rig(UNIX0 + 500);
        rig.clock.advance_s(600);
        let events = library(&mut rig);
        assert!(
            matches!(
                events.as_slice(),
                [FromEngine { event: Event::Error { code: ErrorCode::SignIn, .. }, .. }, FromEngine { event: Event::Targets { .. }, .. }]
            ),
            "{events:?}"
        );
        assert!(rig.http.sent().is_empty());
    }

    #[test]
    fn trouble_reaching_steam_for_a_token_is_not_an_expiry() {
        let mut rig = library_rig(UNIX0 + 30 * 86_400);
        for reply in [fake::down(), fake::status(500), fake::eresult(3, "{}"), fake::ok("{}"), fake::ok(r#"{"response":{"access_token":""}}"#)] {
            rig.http.on("GenerateAccessTokenForApp", [reply]);
            let events = library(&mut rig);
            assert!(matches!(events.as_slice(), [FromEngine { event: Event::Error { .. }, .. }]), "no Targets event for plain trouble: {events:?}");
        }
        let events = rig.send(Request::Targets);
        let [FromEngine { event: Event::Targets { targets }, .. }] = events.as_slice() else { panic!("{events:?}") };
        assert_eq!(targets[0].link, Link::Linked);
    }

    #[test]
    fn a_token_that_does_not_say_when_it_ends_is_used_once() {
        let mut rig = library_rig(UNIX0 + 30 * 86_400);
        rig.http.on("GenerateAccessTokenForApp", [fake::ok(r#"{"response":{"access_token":"opaque"}}"#)]);
        library(&mut rig);
        rig.clock.advance_s(301);
        refreshed(&mut rig);
        assert_eq!(rig.http.count("GenerateAccessTokenForApp"), 2);
    }
}
