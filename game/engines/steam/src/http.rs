//! The network, behind one trait so that every other part of the engine is tried against a fake.
//!
//! Steam reports how a call went in the `x-eresult` header and answers HTTP 200 to most failures, so a reply keeps both. Failing to get
//! an answer at all is named here in a few fixed words: ureq's own error text carries the request URL, and some of these URLs carry an
//! access token, so that text is never passed on, logged or shown.

use std::error::Error as _;
use std::fmt::{self, Write as _};
use std::io::{self, Read};
use std::time::Duration;

/// Steam's web API and its community site: the only two hosts the engine talks to.
pub const API: &str = "https://api.steampowered.com";
pub const COMMUNITY: &str = "https://steamcommunity.com";

/// An address on the web API: `api("/IPlayerService/GetOwnedGames/v1/")`.
pub fn api(path: &str) -> String {
    format!("{API}{path}")
}

/// The most an answer may hold. A library is the one large answer: 5,000 games are about 1.5 MB.
pub const SMALL_BODY: usize = 1 << 20;
pub const LIBRARY_BODY: usize = 8 << 20;
pub const PERSONA_BODY: usize = 256 << 10;

#[derive(Clone, PartialEq, Eq)]
pub struct Resp {
    pub status: u16,
    /// Steam's own result code, from `x-eresult`: 1 is success, 15 is access denied.
    pub eresult: Option<i32>,
    pub body: Vec<u8>,
}

impl Resp {
    /// Steam's own idea of success: HTTP 200 and result code 1.
    pub fn succeeded(&self) -> bool {
        self.status == 200 && self.eresult == Some(1)
    }
}

// A body can hold a token, so it is never printed.
impl fmt::Debug for Resp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Resp").field("status", &self.status).field("eresult", &self.eresult).field("body_bytes", &self.body.len()).finish()
    }
}

/// Why there is no answer. Fixed words only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpError {
    /// The address is not `https`; nothing was sent.
    NotHttps,
    /// No connection: DNS, TCP, TLS or the line dropped.
    Unreachable,
    Timeout,
    /// The answer is longer than the caller allows.
    TooLarge,
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotHttps => "the address is not https",
            Self::Unreachable => "Steam could not be reached",
            Self::Timeout => "Steam did not answer in time",
            Self::TooLarge => "Steam's answer was larger than allowed",
        })
    }
}

impl std::error::Error for HttpError {}

pub trait Http {
    /// A GET. `limit` is the most body bytes accepted.
    fn get(&self, url: &str, limit: usize) -> Result<Resp, HttpError>;
    /// A POST of an already encoded `application/x-www-form-urlencoded` body (see [`form`]), with `headers` added to it.
    fn post_form(&self, url: &str, headers: &[(&str, &str)], body: &str, limit: usize) -> Result<Resp, HttpError>;
}

/// The headers Steam's Android app sends with every authentication call (node-steam-session's `MobileApp` platform data). Steam
/// renews an access token over HTTP only for a refresh token made as a mobile app.
pub const MOBILE_APP: [(&str, &str); 2] = [("User-Agent", "okhttp/4.9.2"), ("Cookie", "mobileClient=android; mobileClientVersion=777777 3.10.3")];

/// `text` as one form or query component: letters, digits and `-._~` stay, every other byte becomes `%XX`. Steam's `request_id` is
/// standard base64, and its `+`, `/` and `=` must not reach the other side as they are.
pub fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// `key=value` pairs joined by `&`, each part encoded. Serves as a form body and as a query string.
pub fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(key, value)| format!("{}={}", percent_encode(key), percent_encode(value))).collect::<Vec<_>>().join("&")
}

/// The real network: https only, no redirects, no proxies, bounded in time and size.
pub struct Ureq {
    agent: ureq::Agent,
}

impl Ureq {
    pub fn new() -> Self {
        let agent = ureq::AgentBuilder::new()
            .https_only(true)
            .redirects(0)
            .timeout_connect(Duration::from_secs(8))
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("LumaGame/", env!("CARGO_PKG_VERSION")))
            .build();
        Self { agent }
    }
}

impl Default for Ureq {
    fn default() -> Self {
        Self::new()
    }
}

impl Http for Ureq {
    fn get(&self, url: &str, limit: usize) -> Result<Resp, HttpError> {
        require_https(url)?;
        finish(self.agent.get(url).call(), limit)
    }

    fn post_form(&self, url: &str, headers: &[(&str, &str)], body: &str, limit: usize) -> Result<Resp, HttpError> {
        require_https(url)?;
        let request = headers.iter().fold(self.agent.post(url), |request, (name, value)| request.set(name, value));
        finish(request.set("Content-Type", "application/x-www-form-urlencoded").send_string(body), limit)
    }
}

fn require_https(url: &str) -> Result<(), HttpError> {
    if url.get(..8).is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://")) { Ok(()) } else { Err(HttpError::NotHttps) }
}

/// Turns what ureq returned into a [`Resp`] or one of the fixed errors. Any HTTP status is an answer: Steam's failures are in its
/// headers, and a 401 means something to the caller.
fn finish(result: Result<ureq::Response, ureq::Error>, limit: usize) -> Result<Resp, HttpError> {
    let response = match result {
        Ok(response) | Err(ureq::Error::Status(_, response)) => response,
        Err(ureq::Error::Transport(transport)) => return Err(classify(&transport)),
    };
    let status = response.status();
    let eresult = response.header("x-eresult").and_then(|value| value.trim().parse().ok());
    let mut body = Vec::new();
    response.into_reader().take(limit as u64 + 1).read_to_end(&mut body).map_err(|error| io_error(&error))?;
    if body.len() > limit {
        return Err(HttpError::TooLarge);
    }
    Ok(Resp { status, eresult, body })
}

fn classify(transport: &ureq::Transport) -> HttpError {
    match transport.kind() {
        ureq::ErrorKind::InvalidUrl | ureq::ErrorKind::UnknownScheme | ureq::ErrorKind::InsecureRequestHttpsOnly => HttpError::NotHttps,
        _ if timed_out(transport.source()) => HttpError::Timeout,
        _ => HttpError::Unreachable,
    }
}

fn io_error(error: &io::Error) -> HttpError {
    if matches!(error.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) { HttpError::Timeout } else { HttpError::Unreachable }
}

/// Whether an `io::Error` that timed out sits anywhere in the chain of causes.
fn timed_out(mut cause: Option<&(dyn std::error::Error + 'static)>) -> bool {
    while let Some(error) = cause {
        if error.downcast_ref::<io::Error>().is_some_and(|error| matches!(error.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock)) {
            return true;
        }
        cause = error.source();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    const SECRET_URL: &str = "/IPlayerService/GetOwnedGames/v1/?access_token=SECRET&steamid=1";

    fn response(raw: &str) -> ureq::Response {
        raw.parse().unwrap()
    }

    fn assert_clean(error: HttpError) {
        let shown = format!("{error} {error:?}");
        assert!(!shown.contains("SECRET") && !shown.contains("127.0.0.1") && !shown.contains("steamid"), "{shown}");
    }

    #[test]
    fn components_are_encoded_so_base64_survives_a_form() {
        assert_eq!(percent_encode("aZ09-._~"), "aZ09-._~");
        assert_eq!(percent_encode("a+b/c=="), "a%2Bb%2Fc%3D%3D");
        assert_eq!(percent_encode("LumaGame TV"), "LumaGame%20TV");
        assert_eq!(percent_encode("é&"), "%C3%A9%26");
        assert_eq!(form(&[("device_friendly_name", "LumaGame TV"), ("platform_type", "3")]), "device_friendly_name=LumaGame%20TV&platform_type=3");
        assert_eq!(form(&[("a b", "c&d=e")]), "a%20b=c%26d%3De");
    }

    #[test]
    fn the_result_code_comes_from_the_header_and_any_status_is_an_answer() {
        let ok = finish(Ok(response("HTTP/1.1 200 OK\r\nX-eresult: 1\r\n\r\n{}")), 16).unwrap();
        assert_eq!((ok.status, ok.eresult, ok.body.as_slice(), ok.succeeded()), (200, Some(1), &b"{}"[..], true));
        let denied = finish(Ok(response("HTTP/1.1 200 OK\r\nx-eresult: 15\r\n\r\n{}")), 16).unwrap();
        assert_eq!((denied.eresult, denied.succeeded()), (Some(15), false));
        let none = finish(Ok(response("HTTP/1.1 200 OK\r\n\r\n{}")), 16).unwrap();
        assert_eq!((none.eresult, none.succeeded()), (None, false));
        let status = finish(Err(ureq::Error::Status(401, response("HTTP/1.1 401 Unauthorized\r\n\r\nnope"))), 16).unwrap();
        assert_eq!((status.status, status.body.as_slice(), status.succeeded()), (401, &b"nope"[..], false));
    }

    #[test]
    fn a_body_over_the_cap_is_refused_and_one_at_the_cap_is_not() {
        assert_eq!(finish(Ok(response("HTTP/1.1 200 OK\r\n\r\n12345")), 5).unwrap().body, b"12345");
        assert_eq!(finish(Ok(response("HTTP/1.1 200 OK\r\n\r\n123456")), 5), Err(HttpError::TooLarge));
    }

    #[test]
    fn a_resp_never_prints_its_body() {
        let resp = Resp { status: 200, eresult: Some(1), body: b"{\"access_token\":\"SECRET\"}".to_vec() };
        assert!(!format!("{resp:?}").contains("SECRET"));
    }

    #[test]
    fn ureq_errors_become_fixed_words_without_the_address() {
        // Not through `Ureq`, which refuses first: ureq's own refusal, whose text does carry the address.
        let url = format!("http://example.invalid{SECRET_URL}");
        let error = ureq::AgentBuilder::new().https_only(true).build().get(&url).call().unwrap_err();
        let mapped = finish(Err(error), 16).unwrap_err();
        assert_eq!(mapped, HttpError::NotHttps);
        assert_clean(mapped);
    }

    #[test]
    fn the_real_client_refuses_http_before_connecting() {
        assert_eq!(Ureq::new().get(&format!("http://127.0.0.1:9{SECRET_URL}"), 16), Err(HttpError::NotHttps));
        assert_eq!(Ureq::new().post_form("ftp://example.invalid/", &[], "a=b", 16), Err(HttpError::NotHttps));
        assert_eq!(Ureq::new().get("", 16), Err(HttpError::NotHttps));
    }

    #[test]
    fn a_refused_connection_is_unreachable_and_names_nothing() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let error = Ureq::new().get(&format!("https://127.0.0.1:{port}{SECRET_URL}"), 16).unwrap_err();
        assert_eq!(error, HttpError::Unreachable);
        assert_clean(error);
    }
}
