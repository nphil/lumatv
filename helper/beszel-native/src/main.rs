//! A Beszel agent for small Linux devices. It answers the hub over its WebSocket as the official agent
//! of the same version does, on one thread, and exits with [`NEWER_HUB`] when the hub has moved to a
//! protocol it cannot read, so a supervisor can run the official agent instead.

mod agent;
mod auth;
mod cbor;
mod cpu;
mod disk;
mod filter;
mod gpu;
mod host;
mod mem;
mod net;
mod proto;
mod sensors;
mod slog;
mod system;
mod wifi;
mod ws;

use std::fs;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use agent::Agent;
use auth::Keys;
use proto::{Identity, Session, Step};
use ws::{ConnectError, Endpoint, Message};

/// The exit status for a hub this agent cannot serve; the official agent may.
const NEWER_HUB: u8 = 76;
/// The Go agent retries a failed connection on a 10 s ticker.
const RETRY: Duration = Duration::from_secs(10);
/// After a lost connection the Go agent redials at once, or after 5 s when the last attempt was that recent.
const RECONNECT: Duration = Duration::from_secs(5);

fn env(key: &str) -> Option<String> {
    std::env::var(format!("BESZEL_AGENT_{key}")).ok().or_else(|| std::env::var(key).ok())
}

fn load_keys() -> Result<Keys, String> {
    if let Some(key) = env("KEY").filter(|key| !key.is_empty()) {
        return Keys::parse(&key);
    }
    let Some(path) = env("KEY_FILE") else {
        return Err("no key provided: must set -key flag, KEY env var, or KEY_FILE env var. Use 'beszel-agent help' for usage".into());
    };
    let text = fs::read_to_string(&path).map_err(|err| format!("failed to read key file: {err}"))?;
    Keys::parse(&text)
}

fn load_token() -> Result<String, String> {
    if let Some(token) = env("TOKEN").filter(|token| !token.is_empty()) {
        return Ok(token);
    }
    let path = env("TOKEN_FILE").filter(|path| !path.is_empty()).ok_or("must set TOKEN or TOKEN_FILE")?;
    let text = fs::read_to_string(&path).map_err(|err| err.to_string())?;
    parse_token_file(&text, &path)
}

/// One token; blank lines and `#` comments are skipped.
fn parse_token_file(contents: &str, path: &str) -> Result<String, String> {
    let mut token = "";
    for line in contents.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if !token.is_empty() {
            return Err(format!("{path} must contain a single token"));
        }
        token = line;
    }
    Ok(token.to_owned())
}

/// `HUB_HEADERS`: `Name: value` pairs split by commas or newlines, names in canonical form.
fn hub_headers(raw: &str) -> Result<Vec<(String, Vec<String>)>, String> {
    let token_char = |c: char| c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c);
    let mut headers: Vec<(String, Vec<String>)> = Vec::new();
    for (i, entry) in raw.split([',', '\n', '\r']).filter(|entry| !entry.is_empty()).enumerate() {
        let (name, value) = entry
            .split_once(':')
            .map(|(name, value)| (name.trim(), value.trim()))
            .filter(|(name, _)| !name.is_empty() && name.chars().all(token_char))
            .ok_or_else(|| format!("invalid HUB_HEADERS: entry {} is not a valid 'Name: value' pair", i + 1))?;
        if value.bytes().any(|b| (b < b' ' && b != b'\t') || b == 0x7f) {
            return Err(format!("invalid HUB_HEADERS: invalid value for header {name:?}"));
        }
        let mut upper = true;
        let name: String = name
            .chars()
            .map(|c| {
                let out = if upper { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() };
                upper = c == '-';
                out
            })
            .collect();
        match headers.iter_mut().find(|(known, _)| *known == name) {
            Some((_, values)) => values.push(value.to_owned()),
            None => headers.push((name, vec![value.to_owned()])),
        }
    }
    Ok(headers)
}

/// The Go agent's browser-like user agent, picked once per run.
fn user_agent() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.subsec_nanos());
    let platform = if nanos % 2 == 0 { "Windows NT 11.0; Win64; x64" } else { "Macintosh; Intel Mac OS X 14_0_0" };
    format!("Mozilla/5.0 ({platform}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
}

/// The port of the address the Go agent would listen on (`LISTEN`, else `PORT`, else 45876), or
/// nothing for a unix socket. A hub that registers the system itself records it.
fn listen_port() -> String {
    let set = |key: &str| env(key).filter(|value| !value.is_empty());
    let mut addr = set("LISTEN").or_else(|| set("PORT")).unwrap_or_else(|| ":45876".into());
    let unix = set("NETWORK").map_or(addr.starts_with('/'), |network| network == "unix");
    if !unix && !addr.contains(':') {
        addr.insert(0, ':');
    }
    split_port(&addr).unwrap_or_default().to_owned()
}

/// The port half of Go's `net.SplitHostPort`, `None` where it fails.
fn split_port(addr: &str) -> Option<&str> {
    let colon = addr.rfind(':')?;
    let (host_from, port_from) = if addr.starts_with('[') {
        let end = addr.find(']')?;
        if end + 1 != colon {
            return None;
        }
        (1, colon)
    } else {
        if addr[..colon].contains(':') {
            return None;
        }
        (0, 0)
    };
    if addr[host_from..].contains('[') || addr[port_from..].contains(']') {
        return None;
    }
    Some(&addr[colon + 1..])
}

struct Client {
    endpoint: Endpoint,
    headers: Vec<(String, String)>,
    tls: Option<Arc<rustls::ClientConfig>>,
    identity: Identity,
}

/// The hub connection's settings, read in the Go agent's order. `None` when the hub is not
/// configured; the agent then has nothing to do.
fn client(agent: &Agent, keys: Keys, port: String) -> Option<Client> {
    let unusable = |err: &str| slog::warn("Error creating WebSocket client", &[("err", &err)]);
    let Some(hub_url) = env("HUB_URL") else {
        let err = "HUB_URL environment variable not set";
        if env("DISABLE_SSH").as_deref() == Some("true") {
            unusable(err);
        } else {
            slog::debug("WebSocket client not configured", &[("err", &err)]);
        }
        return None;
    };
    let Some(endpoint) = Endpoint::parse(&hub_url) else {
        unusable(&format!("invalid HUB_URL {hub_url:?}: must include scheme and host (e.g. http://hub.example.com:8090)"));
        return None;
    };
    let token = match load_token() {
        Ok(token) => token,
        Err(err) => {
            unusable(&err);
            return None;
        }
    };
    let fail = |err: String| -> ! { slog::fatal(&format!("Failed to start: {err}")) };
    let ca_cert_file = env("CA_CERT_FILE").filter(|path| !path.is_empty());
    let tls = (endpoint.secure || ca_cert_file.is_some()).then(|| ws::tls_config(ca_cert_file.as_deref()).unwrap_or_else(|err| fail(err)));
    let extra = hub_headers(&env("HUB_HEADERS").unwrap_or_default()).unwrap_or_else(|err| fail(err));

    let mut headers = vec![
        ("User-Agent".to_owned(), vec![user_agent()]),
        ("X-Token".to_owned(), vec![token.clone()]),
        ("X-Beszel".to_owned(), vec![system::UPSTREAM.to_owned()]),
    ];
    for (name, values) in extra {
        if name == "X-Token" || name == "X-Beszel" {
            slog::warn("Ignoring reserved header in HUB_HEADERS", &[("header", &name)]);
            continue;
        }
        match headers.iter_mut().find(|(known, _)| *known == name) {
            Some(entry) => entry.1 = values,
            None => headers.push((name, values)),
        }
    }
    let headers = headers.into_iter().flat_map(|(name, values)| values.into_iter().map(move |value| (name.clone(), value))).collect();

    let details = &agent.details;
    let identity = Identity {
        keys,
        token,
        fingerprint: host::fingerprint(agent.data_dir.as_deref(), &details.hostname, &details.cpu_model),
        hostname: details.hostname.clone(),
        port,
        name: env("SYSTEM_NAME").unwrap_or_default(),
    };
    Some(Client { endpoint, headers, tls, identity })
}

enum Ended {
    Verified,
    Unverified,
    NewerHub,
}

fn session(agent: &mut Agent, client: &Client, mut conn: ws::Conn) -> Ended {
    let mut session = Session { identity: &client.identity, verified: false };
    let mut gather = |options| agent.gather(options);
    loop {
        let msg = match conn.read() {
            Ok(Message::Binary(msg)) => msg,
            Ok(Message::Closed(reason)) => {
                slog::warn("Connection closed", &[("err", &reason)]);
                break;
            }
            Err(err) => {
                slog::warn("Connection closed", &[("err", &ws::describe(&err))]);
                break;
            }
        };
        let was_verified = session.verified;
        let reply = match session.handle(&msg, &mut gather) {
            Step::Reply(reply) => {
                if session.verified && !was_verified {
                    slog::info("WebSocket connected", &[("host", &client.endpoint.authority)]);
                }
                Some(reply)
            }
            Step::Failed { err, reply } => {
                slog::error("Error handling message", &[("err", &err)]);
                reply
            }
            Step::NewerHub(err) => {
                slog::error("Hub protocol is newer than this agent", &[("version", &system::UPSTREAM), ("err", &err)]);
                conn.close();
                return Ended::NewerHub;
            }
        };
        if let Some(reply) = reply
            && let Err(err) = conn.send(&reply)
        {
            slog::warn("Connection closed", &[("err", &ws::describe(&err))]);
            break;
        }
    }
    if session.verified { Ended::Verified } else { Ended::Unverified }
}

fn serve(agent: &mut Agent, client: &Client) -> ExitCode {
    let mut next = Instant::now();
    loop {
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
        let attempt = Instant::now();
        let conn = match ws::connect(&client.endpoint, &client.headers, client.tls.as_ref()) {
            Ok(conn) => conn,
            Err(ConnectError::Refused { status, body }) if proto::refused_for_version(status, &body) => {
                slog::error("Hub refused this agent version", &[("version", &system::UPSTREAM), ("status", &status), ("err", &body)]);
                return ExitCode::from(NEWER_HUB);
            }
            Err(err) => {
                slog::warn("WebSocket connection failed", &[("err", &err)]);
                next = attempt + RETRY;
                continue;
            }
        };
        next = match session(agent, client, conn) {
            Ended::NewerHub => return ExitCode::from(NEWER_HUB),
            Ended::Verified => {
                slog::warn("Disconnected from hub", &[]);
                if attempt.elapsed() < RECONNECT { Instant::now() + RECONNECT } else { Instant::now() }
            }
            Ended::Unverified => attempt + RETRY,
        };
    }
}

fn main() -> ExitCode {
    let keys = load_keys().unwrap_or_else(|err| slog::fatal(&format!("Failed to load public keys:{err}")));
    let port = listen_port();
    let mut agent = Agent::new(&env);
    match client(&agent, keys, port) {
        Some(client) => serve(&mut agent, &client),
        None => loop {
            std::thread::park();
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_settings_files_like_the_go_agent() {
        assert_eq!(parse_token_file("# hub token\n\n  abc-123 \n", "t").unwrap(), "abc-123");
        assert_eq!(parse_token_file("a\nb\n", "/t").unwrap_err(), "/t must contain a single token");

        let headers = hub_headers("cf-access-client-id: abc, X-TOKEN: x\nx-a: 1,x-a: 2").unwrap();
        let names: Vec<(&str, usize)> = headers.iter().map(|(name, values)| (name.as_str(), values.len())).collect();
        assert_eq!(names, [("Cf-Access-Client-Id", 1), ("X-Token", 1), ("X-A", 2)]);
        assert_eq!(hub_headers("a: 1,,b").unwrap_err(), "invalid HUB_HEADERS: entry 2 is not a valid 'Name: value' pair");
        assert_eq!(hub_headers("a: 1\u{7f}").unwrap_err(), "invalid HUB_HEADERS: invalid value for header \"a\"");
    }
}
