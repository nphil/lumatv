//! A long-poll feed over HTTP/1.1 on the loopback, the shape both the compositor's UI channel (`shell.rs`: `GET /v1/state`,
//! `GET /v1/events?after=`) and the helper's API (`kit/Helper.qml`) have: one thread polls and hands topics and events to the loop over
//! an mpsc channel, another makes the requests the loop asks for (`POST /v1/ui`, `POST /v1/luna`, the helper's posts and gets) and
//! hands their answers back the same way. Nothing here blocks the loop; a feed that stops answering says `Up(false)` and retries at
//! 2, 4, 8 and then 10 s, as the QML client does; a `reset` or a new `boot` starts the session again from the state.

use serde_json::Value;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::Duration;

/// What a feed tells the loop.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The feed answers (true) or stopped answering (false).
    Up(bool),
    /// The whole state at the start of a session: every topic.
    State(Vec<(String, Value)>),
    /// One event of the long poll.
    Topic { topic: String, data: Value },
    /// The answer to a request made with a tag.
    Answer { tag: u64, status: u16, body: Value },
    /// The answer to a request made with a tag whose body is not JSON (a still, a thumbnail: `kit::picture::decode`).
    Bytes { tag: u64, status: u16, body: Vec<u8> },
}

/// A request the loop asks for.
#[derive(Clone, Debug, PartialEq)]
pub struct Outbound {
    pub method: &'static str,
    pub path: String,
    pub body: Option<Value>,
    /// The answer comes back as [`Event::Answer`] with this tag; `None` drops it.
    pub tag: Option<u64>,
}

/// One HTTP exchange on `stream`; the status and the body. `Host` is the peer's address with its port, which the helper requires
/// (`helper/app/src/http.rs` `host_is_local`; refused 403 `host_not_allowed` otherwise); writes carry `X-Luma-Client`.
pub fn exchange(stream: &mut TcpStream, method: &str, path: &str, token: Option<&str>, body: Option<&[u8]>) -> io::Result<(u16, Vec<u8>)> {
    let host = stream.peer_addr()?;
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: keep-alive\r\nAccept: application/json, image/*;q=0.9, */*;q=0.1\r\n");
    if let Some(t) = token {
        req.push_str(&format!("X-Luma-Token: {t}\r\n"));
    }
    // The helper refuses a write without it (`CLIENT_HEADER`); the compositor's channel ignores it.
    req.push_str("X-Luma-Client: lumatv-ui\r\n");
    if let Some(b) = body {
        req.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", b.len()));
    } else if method != "GET" {
        req.push_str("Content-Length: 0\r\n");
    }
    req.push_str("\r\n");
    stream.write_all(req.as_bytes())?;
    if let Some(b) = body {
        stream.write_all(b)?;
    }
    stream.flush()?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let status: u16 = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or_else(|| io::Error::other(format!("status line {line:?}")))?;
    let mut length = 0usize;
    let mut chunked = false;
    loop {
        line.clear();
        reader.read_line(&mut line)?;
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        let lower = l.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            length = v.trim().parse().unwrap_or(0);
        } else if lower.starts_with("transfer-encoding:") && lower.contains("chunked") {
            chunked = true;
        }
    }
    let mut out = Vec::new();
    if chunked {
        loop {
            line.clear();
            reader.read_line(&mut line)?;
            let n = usize::from_str_radix(line.trim(), 16).map_err(|_| io::Error::other("chunk size"))?;
            if n == 0 {
                line.clear();
                reader.read_line(&mut line)?;
                break;
            }
            let mut chunk = vec![0u8; n + 2];
            reader.read_exact(&mut chunk)?;
            out.extend_from_slice(&chunk[..n]);
        }
    } else {
        out.resize(length, 0);
        reader.read_exact(&mut out)?;
    }
    Ok((status, out))
}

fn connect(host: &str, port: u16, read_timeout: Duration) -> io::Result<TcpStream> {
    let stream = TcpStream::connect_timeout(&format!("{host}:{port}").parse().map_err(|_| io::Error::other("address"))?, Duration::from_secs(3))?;
    stream.set_read_timeout(Some(read_timeout))?;
    stream.set_nodelay(true)?;
    Ok(stream)
}

/// Which topics a poll asks the helper for.
enum Topics {
    /// The helper's `topics=` as given, or every topic.
    Fixed(Option<String>),
    /// The base list and, for each connected plugin of the `plugins` roster, its cards, tiles, notifications and facts; past the 16
    /// names a poll may carry, every topic (`kit/Helper.qml` `filter()`).
    Follow(&'static str),
}

/// The topic names a poll asks for under the roster as it stands: `None` for every topic.
pub fn follow_filter(base: &str, roster: &Value) -> Option<String> {
    let mut names: Vec<String> = base.split(',').map(str::to_string).collect();
    let plugins =
        roster.get("plugins").and_then(Value::as_array).into_iter().flatten().chain(roster.get("firstParty").and_then(Value::as_array).into_iter().flatten());
    for p in plugins {
        if p.get("state").and_then(Value::as_str) != Some("connected") {
            continue;
        }
        let Some(id) = p.get("id").and_then(Value::as_str) else { continue };
        for kind in ["cards", "tiles", "notifications", "facts"] {
            names.push(format!("plugin.{id}.{kind}"));
        }
    }
    if names.len() > 16 { None } else { Some(names.join(",")) }
}

/// A feed: the requests it makes for the loop; its events come on the receiver [`Feed::start`] returns beside it.
pub struct Feed {
    requests: Sender<Outbound>,
}

impl Feed {
    /// Starts the poller and the requester for `port` (always `127.0.0.1`); `token` goes in `X-Luma-Token`; `topics` narrows the poll to
    /// those topics (the helper's `topics=`), `None` takes them all.
    pub fn start(name: &'static str, port: u16, token: Option<String>, topics: Option<String>) -> io::Result<(Self, Receiver<Event>)> {
        Self::spawn(name, port, token, Topics::Fixed(topics))
    }

    /// The helper's feed: `base` (a comma list) and the topics of every connected plugin, as the QML client's poll names them.
    pub fn start_following(name: &'static str, port: u16, token: Option<String>, base: &'static str) -> io::Result<(Self, Receiver<Event>)> {
        Self::spawn(name, port, token, Topics::Follow(base))
    }

    fn spawn(name: &'static str, port: u16, token: Option<String>, topics: Topics) -> io::Result<(Self, Receiver<Event>)> {
        let (etx, events) = channel();
        let (requests, rrx) = channel::<Outbound>();
        let poll_tx = etx.clone();
        let poll_token = token.clone();
        thread::Builder::new().name(format!("{name}-poll")).spawn(move || poll_loop(port, poll_token, topics, poll_tx))?;
        thread::Builder::new().name(format!("{name}-send")).spawn(move || send_loop(port, token, rrx, etx))?;
        Ok((Self { requests }, events))
    }

    /// Asks for a request; it goes out in order with the others.
    pub fn request(&self, out: Outbound) {
        let _ = self.requests.send(out);
    }

    pub fn post(&self, path: &str, body: Value) {
        self.request(Outbound { method: "POST", path: path.to_string(), body: Some(body), tag: None });
    }

    pub fn get(&self, path: &str, tag: u64) {
        self.request(Outbound { method: "GET", path: path.to_string(), body: None, tag: Some(tag) });
    }
}

fn poll_loop(port: u16, token: Option<String>, topics: Topics, tx: Sender<Event>) {
    let host = "127.0.0.1";
    let mut failures = 0u32;
    'session: loop {
        let mut stream = match connect(host, port, Duration::from_secs(35)) {
            Ok(s) => s,
            Err(_) => {
                failures += 1;
                let _ = tx.send(Event::Up(false));
                thread::sleep(Duration::from_millis((2000 * (1u64 << failures.saturating_sub(1).min(3))).min(10_000)));
                continue;
            }
        };
        let state = exchange(&mut stream, "GET", "/v1/state", token.as_deref(), None)
            .ok()
            .and_then(|(status, body)| (status == 200).then(|| serde_json::from_slice::<Value>(&body).ok()).flatten());
        let Some(state) = state.filter(|s| s.get("ok").and_then(Value::as_bool) == Some(true)) else {
            failures += 1;
            let _ = tx.send(Event::Up(false));
            thread::sleep(Duration::from_millis((2000 * (1u64 << failures.saturating_sub(1).min(3))).min(10_000)));
            continue;
        };
        failures = 0;
        let boot = state.get("boot").cloned().unwrap_or(Value::Null);
        let mut seq = state.get("seq").and_then(Value::as_u64).unwrap_or(0);
        let topics_now: Vec<(String, Value)> =
            state.get("topics").and_then(Value::as_object).map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
        // the roster the following poll names plugin topics from: the state's, then every event of `plugins`
        let mut roster = topics_now.iter().find(|(k, _)| k == "plugins").map_or(Value::Null, |(_, v)| v.clone());
        if tx.send(Event::Up(true)).is_err() || tx.send(Event::State(topics_now)).is_err() {
            return;
        }
        loop {
            let mut path = format!("/v1/events?after={seq}&timeout=25");
            if let Some(b) = boot.as_u64() {
                path.push_str(&format!("&boot={b}"));
            }
            let names = match &topics {
                Topics::Fixed(t) => t.clone(),
                Topics::Follow(base) => follow_filter(base, &roster),
            };
            if let Some(t) = &names {
                path.push_str(&format!("&topics={t}"));
            }
            let began = std::time::Instant::now();
            let answer = exchange(&mut stream, "GET", &path, token.as_deref(), None);
            let doc = match answer {
                Ok((200, body)) => serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null),
                _ => {
                    let _ = tx.send(Event::Up(false));
                    thread::sleep(Duration::from_secs(2));
                    continue 'session;
                }
            };
            if doc.get("ok").and_then(Value::as_bool) != Some(true) {
                let _ = tx.send(Event::Up(false));
                thread::sleep(Duration::from_secs(2));
                continue 'session;
            }
            if doc.get("reset").and_then(Value::as_bool) == Some(true) || doc.get("boot") != Some(&boot) {
                continue 'session;
            }
            seq = doc.get("seq").and_then(Value::as_u64).unwrap_or(seq);
            let events = doc.get("events").and_then(Value::as_array).cloned().unwrap_or_default();
            let n = events.len();
            for e in events {
                let topic = e.get("topic").and_then(Value::as_str).unwrap_or("").to_string();
                let data = e.get("data").cloned().unwrap_or(Value::Null);
                if topic == "plugins" {
                    roster = data.clone();
                }
                if tx.send(Event::Topic { topic, data }).is_err() {
                    return;
                }
            }
            // An answer with nothing in it that came back at once is not a long poll: wait a second.
            if n == 0 && began.elapsed() < Duration::from_millis(500) {
                thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

fn send_loop(port: u16, token: Option<String>, rx: Receiver<Outbound>, tx: Sender<Event>) {
    let host = "127.0.0.1";
    let mut stream: Option<TcpStream> = None;
    for out in rx {
        let body = out.body.as_ref().map(|b| serde_json::to_vec(b).unwrap_or_default());
        let mut answer = None;
        for attempt in 0..2 {
            if stream.is_none() {
                stream = connect(host, port, Duration::from_secs(10)).ok();
            }
            let Some(s) = stream.as_mut() else { break };
            match exchange(s, out.method, &out.path, token.as_deref(), body.as_deref()) {
                Ok(a) => {
                    answer = Some(a);
                    break;
                }
                Err(_) => {
                    // the connection went (the server closed it between requests): once more on a new one
                    stream = None;
                    if attempt == 1 {
                        break;
                    }
                }
            }
        }
        if let Some(tag) = out.tag {
            // a body that is JSON is the answer; one that is not (a still, a thumbnail) comes as bytes
            let event = match answer {
                None => Event::Answer { tag, status: 0, body: Value::Null },
                Some((status, bytes)) => match serde_json::from_slice::<Value>(&bytes) {
                    Ok(body) => Event::Answer { tag, status, body },
                    Err(_) if !bytes.is_empty() => Event::Bytes { tag, status, body: bytes },
                    Err(_) => Event::Answer { tag, status, body: Value::Null },
                },
            };
            if tx.send(event).is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_poll_names_the_topics_of_the_connected_plugins_and_asks_for_all_past_sixteen() {
        let roster = json!({"plugins": [{"id": "home-assistant", "state": "connected"}, {"id": "idle", "state": "waiting"}], "firstParty": []});
        let names = follow_filter("screen,receiver,plugins", &roster).expect("a list");
        assert_eq!(
            names,
            "screen,receiver,plugins,plugin.home-assistant.cards,plugin.home-assistant.tiles,plugin.home-assistant.notifications,plugin.home-assistant.facts"
        );
        assert_eq!(follow_filter("screen", &Value::Null).as_deref(), Some("screen"), "no roster yet: the base list");
        let many = json!({"plugins": [{"id": "a", "state": "connected"}, {"id": "b", "state": "connected"}, {"id": "c", "state": "connected"}, {"id": "d", "state": "connected"}]});
        assert_eq!(follow_filter("screen,receiver,plugins", &many), None, "past the helper's 16 names: every topic");
    }
}
