//! The facade's HTTP server: a few GETs on the LAN, one of which does not end. A stream that runs for as long as a recording needs a
//! listener of its own: one thread per connection, no body length, the connection closed when the program is over (what an HDHomeRun
//! does). Only peers in the allowed networks are answered; the rest are dropped.

use crate::consent::{Conflict, Key, TuneRequest};
use crate::docs;
use crate::net::Cidr;
use crate::sendlog::Sent;
use crate::stream::{Admitted, End, Facade, Failure, Lease, Rejected};
use parking_lot::Mutex;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Connections at once: a stream holds one for as long as it runs, and the documents are short.
const MAX_CONNECTIONS: usize = 6;
const REQUEST_LIMIT: u64 = 8 * 1024;
const REQUEST_WAIT: Duration = Duration::from_secs(5);
/// Bound writes of the short HTTP documents and errors; streams have their own stall limit.
const SEND_WAIT: Duration = Duration::from_secs(10);
/// How long the refusals of one channel to one peer for one reason are counted before they are logged together.
const REFUSALS_EVERY: Duration = Duration::from_secs(1);

/// Refusals logged once with how many came: VLC asked again about 25 times in 1.3 s while the tuner was in use (2026-11-05). The first of
/// each peer, channel and reason is logged at once, the ones after it within [`REFUSALS_EVERY`] as a count once that has passed (at the
/// next refusal or the next stream's end).
#[derive(Default)]
struct Refusals(Mutex<HashMap<Refused, Counted>>);

/// A peer, the channel it asked for and the reason it was refused.
type Refused = (IpAddr, String, String);
/// When the first and the last of a second's refusals came, and how many came after the first.
type Counted = (Instant, Instant, u32);

impl Refusals {
    /// Counts a refusal; the lines to log.
    fn refused(&self, peer: IpAddr, guide: &str, reason: String, now: Instant) -> Vec<String> {
        let mut lines = self.flush(now);
        let mut counted = self.0.lock();
        let key = (peer, guide.to_owned(), reason);
        match counted.get_mut(&key) {
            Some((_, last, more)) => {
                *last = now;
                *more += 1;
            }
            None => {
                lines.push(format!("tuner: {peer} asked for {guide}: {}", key.2));
                counted.insert(key, (now, now, 0));
            }
        }
        lines
    }

    /// Forgets the refusals counted for long enough; the lines that say how many more came.
    fn flush(&self, now: Instant) -> Vec<String> {
        let mut lines = Vec::new();
        self.0.lock().retain(|(peer, guide, reason), (first, last, more)| {
            if now.saturating_duration_since(*first) < REFUSALS_EVERY {
                return true;
            }
            if *more > 0 {
                let within = last.saturating_duration_since(*first).as_secs_f64();
                lines.push(format!("tuner: {peer} asked for {guide} {more} more times in {within:.1} s: {reason}"));
            }
            false
        });
        lines
    }
}

pub struct Server {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    pub fn start(addr: SocketAddr, facade: Arc<Facade>, peers: Vec<Cidr>) -> io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            thread::Builder::new()
                .name("hdhr".to_owned())
                .stack_size(256 * 1024)
                .spawn(move || accept(&listener, &facade, &peers, &stop))?
        };
        Ok(Self {
            addr,
            stop,
            thread: Some(thread),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Stops accepting and ends the streams (each sees the flag within a read) and waits for all of it.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(500));
            let _ = thread.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn accept(listener: &TcpListener, facade: &Arc<Facade>, peers: &[Cidr], stop: &Arc<AtomicBool>) {
    let active = Arc::new(AtomicUsize::new(0));
    let refusals = Arc::new(Refusals::default());
    for incoming in listener.incoming() {
        if stop.load(Ordering::Acquire) {
            break;
        }
        let Ok(stream) = incoming else { continue };
        let Ok(peer) = stream.peer_addr().map(|p| p.ip().to_canonical()) else {
            continue;
        };
        if !peers.iter().any(|net| net.contains(peer)) {
            continue;
        }
        if active.load(Ordering::Acquire) >= MAX_CONNECTIONS {
            let mut stream = stream;
            let _ = reply(&mut stream, 503, "text/plain", b"too many connections");
            continue;
        }
        active.fetch_add(1, Ordering::AcqRel);
        let task = {
            let (facade, stop, active, refusals) = (facade.clone(), stop.clone(), active.clone(), refusals.clone());
            move || {
                serve(stream, peer, &facade, &stop, &refusals);
                active.fetch_sub(1, Ordering::AcqRel);
            }
        };
        if thread::Builder::new()
            .name("hdhr-conn".to_owned())
            .stack_size(256 * 1024)
            .spawn(task)
            .is_err()
        {
            active.fetch_sub(1, Ordering::AcqRel);
        }
    }
    // A process another thread is starting holds a copy of every descriptor until it runs its program, which kept the port answering
    // after a drop of the listener (a quarter of the engine's test runs) and would refuse a new listener on the same port. Shutting the
    // socket down closes it for every copy at once, before the streams are waited for.
    // SAFETY: the descriptor is the listener's own and open.
    unsafe { libc::shutdown(listener.as_raw_fd(), libc::SHUT_RDWR) };
    // the streams see the flag within one read; the helper is going down, so a stuck one is not waited for
    let end = Instant::now() + Duration::from_secs(3);
    while active.load(Ordering::Acquire) > 0 && Instant::now() < end {
        thread::sleep(Duration::from_millis(20));
    }
}

struct Request {
    method: String,
    path: String,
    host: Option<String>,
    agent: Option<String>,
    /// The profile `?transcode=` names, as written.
    transcode: Option<String>,
    /// What the client calls itself: `?device=` or the `X-Luma-Device` header, the query first.
    device: Option<String>,
    structured: bool,
    takeover: Option<Key>,
    stream_key: Option<Key>,
}

/// A device name as a client wrote it (percent-escapes and `+` undone): visible text, at most 40 characters, none when empty.
fn device_name(raw: &str) -> Option<String> {
    let bytes = raw.replace('+', " ").into_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = (bytes[i] == b'%')
            .then(|| bytes.get(i + 1..i + 3))
            .flatten()
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match escaped {
            Some(b) => {
                out.push(b);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    let text = String::from_utf8_lossy(&out);
    let name: String = text.chars().filter(|c| !c.is_control()).take(40).collect();
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// Reads one bounded HTTP request. The target's query and the private authorization key never enter the tune log.
fn read_request(stream: &TcpStream) -> Option<Request> {
    stream.set_read_timeout(Some(REQUEST_WAIT)).ok()?;
    let mut lines = BufReader::new(stream.take(REQUEST_LIMIT)).lines();
    let first = lines.next()?.ok()?;
    let mut parts = first.split_whitespace();
    let (method, target, version) = (parts.next()?, parts.next()?, parts.next()?);
    if !version.starts_with("HTTP/1.") {
        return None;
    }
    let (mut host, mut agent, mut header_device, mut stream_key) = (None, None, None, None);
    for line in lines {
        let line = line.ok()?;
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim();
            if name.eq_ignore_ascii_case("host") {
                host = Some(value.trim().to_owned());
            } else if name.eq_ignore_ascii_case("user-agent") {
                agent = Some(value.trim().chars().take(200).collect());
            } else if name.eq_ignore_ascii_case("x-luma-device") {
                header_device = device_name(value);
            } else if name.eq_ignore_ascii_case("authorization")
                && let Some((scheme, key)) = value.trim().split_once(' ')
                && scheme.eq_ignore_ascii_case("bearer")
            {
                stream_key = Key::parse(key);
            }
        }
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let param = |key: &str| {
        query.split('&').filter_map(|pair| pair.split_once('=')).find_map(|(k, v)| (k == key).then_some(v))
    };
    let transcode = param("transcode").map(|name| name.chars().take(40).collect());
    let device = param("device").and_then(device_name).or(header_device);
    Some(Request {
        method: method.to_owned(),
        path: path.to_owned(),
        host,
        agent,
        transcode,
        device,
        structured: param("consent") == Some("1"),
        takeover: param("takeover").and_then(Key::parse),
        stream_key,
    })
}

/// A host the client reached the TV by (`192.168.1.242:5004`), when it is only what a host looks like: it goes into URLs the client follows.
fn plausible(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 100
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b':' | b'-' | b'[' | b']'))
}

fn reply(out: &mut impl Write, status: u16, content_type: &str, body: &[u8]) -> io::Result<()> {
    answer(out, status, content_type, "", body)
}

/// A stream that cannot be served: its status, the `X-HDHomeRun-Error` players read the cause from, and the reason in words.
fn refuse(out: &mut impl Write, f: &Failure) -> io::Result<()> {
    answer(
        out,
        f.status(),
        "text/plain",
        &format!("X-HDHomeRun-Error: {}\r\n", f.hdhomerun_error()),
        f.reason().as_bytes(),
    )
}

fn deny_tune(out: &mut impl Write, conflict: &Conflict, structured: bool) -> io::Result<()> {
    const HEADERS: &str = "X-HDHomeRun-Error: 805 All Tuners In Use\r\nCache-Control: no-store\r\n";
    if structured {
        answer(out, 503, "application/json", HEADERS, conflict.doc().to_string().as_bytes())
    } else {
        answer(out, 503, "text/plain", HEADERS, b"All Tuners In Use")
    }
}

fn stream_head(out: &mut TcpStream, lease: &Lease, stale_after: Duration) -> io::Result<()> {
    // A blocked send times out even if no new tune is requested. Waiting for source data never runs this timer.
    out.set_write_timeout(Some(stale_after))?;
    let key = lease.key.as_ref().expect("admitted stream has a key");
    write!(
        out,
        "HTTP/1.0 200 OK\r\nContent-Type: video/mp2t\r\nCache-Control: no-store\r\nX-Luma-Stream: s{}\r\nX-Luma-Stream-Key: {}\r\n\r\n",
        lease.id, key.text().as_str(),
    )
}

fn answer(
    out: &mut impl Write,
    status: u16,
    content_type: &str,
    headers: &str,
    body: &[u8],
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        502 => "Bad Gateway",
        _ => "Service Unavailable",
    };
    write!(
        out,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    out.write_all(body)?;
    out.flush()
}

fn json(out: &mut impl Write, value: &Value) -> io::Result<()> {
    reply(out, 200, "application/json", value.to_string().as_bytes())
}

fn serve(mut stream: TcpStream, peer: IpAddr, facade: &Facade, stop: &AtomicBool, refusals: &Refusals) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_write_timeout(Some(SEND_WAIT));
    let Some(request) = read_request(&stream) else {
        return;
    };
    let local = stream
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_default();
    let base = format!(
        "http://{}",
        request
            .host
            .as_deref()
            .filter(|h| plausible(h))
            .unwrap_or(&local)
    );
    // a client that went away is not news: what it was told is all that can be done for it
    let _ = match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/discover.json") => json(&mut stream, &docs::discover(&facade.identity, &base)),
        ("GET", "/lineup_status.json") => json(&mut stream, &docs::lineup_status()),
        ("GET", "/status.json") => answer(
            &mut stream, 200, "application/json", "Cache-Control: no-store\r\n",
            facade.status_doc().to_string().as_bytes(),
        ),
        ("GET", "/lineup.json") => match facade.lineup() {
            Ok(entries) => json(&mut stream, &docs::lineup(&entries, &base, |guide| facade.format_of(guide))),
            Err(f) => reply(&mut stream, f.status(), "text/plain", f.reason().as_bytes()),
        },
        ("GET", "/lineup.m3u") => match facade.lineup() {
            Ok(entries) => reply(
                &mut stream,
                200,
                "audio/x-mpegurl; charset=utf-8",
                docs::m3u(&entries, &base).as_bytes(),
            ),
            Err(f) => reply(&mut stream, f.status(), "text/plain", f.reason().as_bytes()),
        },
        ("GET", "/xmltv.xml") => match facade.guide() {
            Ok(xml) => reply(
                &mut stream,
                200,
                "application/xml; charset=utf-8",
                xml.as_bytes(),
            ),
            Err(f) => reply(&mut stream, f.status(), "text/plain", f.reason().as_bytes()),
        },
        // Plex asks for a scan; the lineup is the TV's own list, so there is nothing to start
        ("GET" | "POST", "/lineup.post") => reply(&mut stream, 200, "text/plain", b""),
        ("GET", path) if path.starts_with("/auto/v") => {
            let guide = &path["/auto/v".len()..];
            let asked = Instant::now();
            let fd = stream.as_raw_fd();
            let client = TuneRequest {
                peer, agent: request.agent.as_deref(), device: request.device.as_deref(), profile: request.transcode.as_deref(),
                structured: request.structured, takeover: request.takeover, stream_key: request.stream_key,
            };
            let sent = match facade.admit(guide, client) {
                Err(Rejected::Failed(f)) => {
                    refusals.refused(peer, guide, f.reason(), Instant::now()).iter().for_each(|l| log::info!("{l}"));
                    return drop(refuse(&mut stream, &f));
                }
                Err(Rejected::Busy(conflict)) => {
                    refusals.refused(peer, guide, conflict.failure.reason(), Instant::now()).iter().for_each(|l| log::info!("{l}"));
                    return drop(deny_tune(&mut stream, &conflict, request.structured));
                }
                Ok(Admitted::Plain(mut program)) => {
                    // the PAT and the PMT go out as soon as the head is written: this is the client's wait for its first byte
                    let [tune, tables, picture] = program.took.map(|d| d.as_secs_f64());
                    log::info!(
                        "tuner: {peer} streams {guide} after {:.1} s (tune {tune:.1}, tables {tables:.1}, picture {picture:.1})",
                        asked.elapsed().as_secs_f64()
                    );
                    stream_head(&mut stream, &program.lease, facade.limits.stale_after).and_then(|()| {
                        let mut out = Sent::new(&mut stream, fd, facade.clock.clone(), format!("{peer} {guide}"));
                        let end = program.pump(&mut out, stop);
                        out.finish();
                        end
                    })
                }
                Ok(Admitted::Slated(slated)) => {
                    // answered at once: the slate is the first byte, and the channel is tuned while it shows
                    log::info!("tuner: {peer} streams {guide} with slates (taken in {:.2} s)", asked.elapsed().as_secs_f64());
                    stream_head(&mut stream, &slated.lease, facade.limits.stale_after).and_then(|()| {
                        let mut out = Sent::new(&mut stream, fd, facade.clock.clone(), format!("{peer} {guide}"));
                        let end = facade.play(slated, &mut out, stop);
                        out.finish();
                        end
                    })
                }
            };
            match sent {
                Ok(End::Stopped) => log::info!("tuner: {guide} stopped"),
                Ok(End::Replaced) => log::info!("tuner: {guide} gave the tuner to the next request"),
                Ok(End::Taken(by)) => log::info!("tuner: {guide} gave the tuner to {by}"),
                Ok(end) => log::warn!("tuner: {guide} ended: {end:?}"),
                Err(e) => log::info!("tuner: {peer} left {guide}: {e}"),
            }
            refusals.flush(Instant::now()).iter().for_each(|l| log::info!("{l}"));
            Ok(())
        }
        ("GET" | "POST", _) => reply(&mut stream, 404, "text/plain", b"not found"),
        _ => reply(&mut stream, 405, "text/plain", b"method not allowed"),
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::Identity;
    use crate::stream::Limits;
    use crate::stream::tests::{Fake, OTHER, Shared, Time, fake_with, packet, picture, program, rf32, served, video_type};
    use crate::transcode::tests as converter;
    use std::net::Ipv4Addr;

    fn start(fake: &Arc<Fake>, time: &Time, peers: &[&str]) -> Server {
        let facade = Facade::new(
            Arc::new(Shared(fake.clone())),
            Identity {
                name: "LumaTV".into(),
                device_id: "0A1B2C3D".into(),
            },
            Limits::default(),
            time.clock(),
            served(),
            None,
        );
        let peers = peers.iter().map(|p| Cidr::parse(p).unwrap()).collect();
        Server::start(
            SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            Arc::new(facade),
            peers,
        )
        .unwrap()
    }

    /// One request, the whole answer (the server closes the connection).
    fn get(server: &Server, request: &str) -> Vec<u8> {
        let mut c = TcpStream::connect(server.local_addr()).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        c.write_all(request.as_bytes()).unwrap();
        let mut out = Vec::new();
        let _ = c.read_to_end(&mut out);
        out
    }

    fn split(answer: &[u8]) -> (String, &[u8]) {
        let at = answer
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("a head");
        (
            String::from_utf8_lossy(&answer[..at]).into_owned(),
            &answer[at + 4..],
        )
    }

    const LOCAL: &[&str] = &["127.0.0.0/8"];

    #[test]
    fn the_documents_name_the_tuner_and_follow_the_host_the_client_used() {
        let time = Time::default();
        let server = start(&fake_with(&time, rf32(&[]), vec![]), &time, LOCAL);
        let answer = get(
            &server,
            "GET /discover.json HTTP/1.1\r\nHost: 192.168.1.242:5004\r\n\r\n",
        );
        let (head, body) = split(&answer);
        assert!(
            head.starts_with("HTTP/1.1 200 OK") && head.contains("application/json"),
            "{head}"
        );
        let d: Value = serde_json::from_slice(body).unwrap();
        assert_eq!(
            (d["TunerCount"].clone(), d["BaseURL"].as_str()),
            (serde_json::json!(1), Some("http://192.168.1.242:5004"))
        );
        let answer = get(
            &server,
            "GET /lineup.json HTTP/1.1\r\nHost: tv.local:5004\r\n\r\n",
        );
        let l: Value = serde_json::from_slice(split(&answer).1).unwrap();
        assert_eq!(l[1]["URL"], "http://tv.local:5004/auto/v2.4");
        let answer = get(&server, "GET /lineup_status.json HTTP/1.0\r\n\r\n");
        assert_eq!(
            serde_json::from_slice::<Value>(split(&answer).1).unwrap()["Source"],
            "Antenna"
        );
        assert!(
            get(&server, "POST /lineup.post?scan=start HTTP/1.1\r\n\r\n")
                .starts_with(b"HTTP/1.1 200")
        );
    }

    #[test]
    fn the_lineup_says_how_a_seen_channel_is_scanned_and_leaves_the_rest_unsaid() {
        use crate::health::Seen;
        use crate::mpeg2::Format;
        use crate::splice::Layout;
        let time = Time::default();
        let tuning = served();
        let facade = Facade::new(
            Arc::new(Shared(fake_with(&time, rf32(&[]), vec![]))),
            Identity { name: "LumaTV".into(), device_id: "0A1B2C3D".into() },
            Limits::default(),
            time.clock(),
            tuning.clone(),
            None,
        );
        let server = Server::start(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)), Arc::new(facade), vec![Cidr::parse("127.0.0.0/8").unwrap()]).unwrap();
        let lineup = || serde_json::from_slice::<Value>(split(&get(&server, "GET /lineup.json HTTP/1.1\r\n\r\n")).1).unwrap();
        let scan = |l: &Value, i: usize| (l[i]["interlaced"].clone(), l[i]["height"].clone(), l[i]["fps"].clone());
        let unknown = (Value::Null, Value::Null, Value::Null);
        let before = lineup();
        assert_eq!((scan(&before, 0), scan(&before, 1)), (unknown.clone(), unknown.clone()), "nothing is said until a channel has been tuned");

        let layout = |video_type| Layout { video_type, ..Layout::fallback(1) };
        let hd720 = Format { width: 1280, height: 720, rate: 7, aspect: 3, progressive: true };
        tuning.saw("2.1", Seen { format: hd720, layout: layout(0x02), tsid: 1 });
        tuning.saw("2.4", Seen { format: Format::FALLBACK, layout: layout(0x1B), tsid: 1 });
        let after = lineup();
        let (progressive, height, fps) = (serde_json::json!(false), serde_json::json!(720), serde_json::json!(59.94));
        assert_eq!(scan(&after, 0), (progressive, height, fps), "2.1 is 720p60");
        assert_eq!(scan(&after, 1), unknown, "an H.264 picture's format is not read");
    }

    #[test]
    fn the_playlist_and_the_guide_follow_the_host_and_join_by_guide_number() {
        let time = Time::default();
        let server = start(&fake_with(&time, rf32(&[]), vec![]), &time, LOCAL);
        let answer = get(
            &server,
            "GET /lineup.m3u HTTP/1.1\r\nHost: tv.local:5004\r\n\r\n",
        );
        let (head, body) = split(&answer);
        assert!(
            head.starts_with("HTTP/1.1 200 OK") && head.contains("audio/x-mpegurl"),
            "{head}"
        );
        let playlist = String::from_utf8_lossy(body);
        assert_eq!(
            playlist.lines().take(4).collect::<Vec<_>>(),
            [
                "#EXTM3U url-tvg=\"http://tv.local:5004/xmltv.xml\"",
                "#EXTINF:-1 tvg-id=\"2.1\" tvg-chno=\"2.1\" tvg-name=\"WSB-HD\",2.1 WSB-HD",
                "http://tv.local:5004/auto/v2.1",
                "#EXTINF:-1 tvg-id=\"2.4\" tvg-chno=\"2.4\" tvg-name=\"WEST\",2.4 WEST",
            ]
        );
        let answer = get(&server, "GET /xmltv.xml HTTP/1.1\r\nHost: x\r\n\r\n");
        let (head, body) = split(&answer);
        assert!(
            head.starts_with("HTTP/1.1 200 OK") && head.contains("application/xml"),
            "{head}"
        );
        let xml = String::from_utf8_lossy(body);
        assert!(xml.starts_with("<?xml") && xml.contains("<channel id=\"2.4\">"));
        assert!(xml.contains("channel=\"2.4\"><title>News on 1_32_2_4_0_0_0</title>"));
    }

    fn header<'a>(head: &'a str, prefix: &str) -> &'a str {
        head.lines().find_map(|line| line.strip_prefix(prefix)).expect("required response header")
    }

    #[test]
    fn a_stream_is_mpeg_ts_with_no_length_until_the_program_is_over() {
        let time = Time::default();
        let chunk = [picture(0x61, 0), packet(0x64, 0)].concat();
        let server = start(
            &fake_with(&time, rf32(&[]), vec![chunk.clone()]),
            &time,
            LOCAL,
        );
        let answer = get(&server, "GET /auto/v2.4 HTTP/1.1\r\nHost: x\r\n\r\n");
        let (head, body) = split(&answer);
        assert!(
            head.starts_with("HTTP/1.0 200 OK")
                && head.contains("Content-Type: video/mp2t")
                && !head.contains("Content-Length"),
            "{head}"
        );
        assert_eq!(header(&head, "X-Luma-Stream: "), "s0");
        assert!(Key::parse(header(&head, "X-Luma-Stream-Key: ")).is_some());
        assert_eq!(header(&head, "Cache-Control: "), "no-store");
        assert!(body.len() % 188 == 0 && body[0] == 0x47);
        assert!(
            body.windows(chunk.len()).any(|w| w == &chunk[..]),
            "the tuner's packets arrive whole"
        );
    }

    #[test]
    fn transcode_in_the_query_asks_for_h264_when_a_converter_is_installed_and_a_profile_it_does_not_know_is_error_802() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), program(4));
        let tuning = served();
        let facade = Facade::new(
            Arc::new(Shared(fake.clone())),
            Identity { name: "LumaTV".into(), device_id: "0A1B2C3D".into() },
            Limits::default(),
            time.clock(),
            tuning.clone(),
            None,
        );
        let peers = vec![Cidr::parse("127.0.0.0/8").unwrap()];
        let server = Server::start(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)), Arc::new(facade), peers).unwrap();
        let asked = |query: &str| get(&server, &format!("GET /auto/v2.4{query} HTTP/1.1\r\nHost: x\r\n\r\n"));
        let answer = asked("?transcode=mobile&duration=5");
        let (head, body) = split(&answer);
        assert!(head.starts_with("HTTP/1.0 200 OK"), "{head}");
        assert_eq!(video_type(body), 0x02, "without a converter the query is not read");

        tuning.options.lock().codec = Some(Arc::new(converter::Fake::default()));
        let answer = asked("?transcode=mobile&duration=5");
        let (head, body) = split(&answer);
        assert!(head.starts_with("HTTP/1.0 200 OK") && head.contains("video/mp2t"), "{head}");
        assert_eq!(video_type(body), 0x1B, "with one it is H.264");
        let answer = asked("");
        let (head, body) = split(&answer);
        assert!(head.starts_with("HTTP/1.0 200 OK"), "{head}");
        assert_eq!(video_type(body), 0x02, "and a request that does not ask is as before");

        let answer = asked("?duration=5&transcode=faster");
        let (head, body) = split(&answer);
        assert!(head.starts_with("HTTP/1.1 400 Bad Request"), "{head}");
        assert!(head.contains("X-HDHomeRun-Error: 802 Unknown Transcode Profile"), "{head}");
        assert_eq!(String::from_utf8_lossy(body), "no such transcode profile: faster");
    }

    #[test]
    fn what_cannot_be_served_is_a_status_an_hdhomerun_error_and_a_reason() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), vec![]);
        let server = start(&fake, &time, LOCAL);
        let status = |request: &str| {
            let answer = get(&server, request);
            let (head, body) = split(&answer);
            let error = head
                .lines()
                .find_map(|l| l.strip_prefix("X-HDHomeRun-Error: "))
                .unwrap_or("")
                .to_owned();
            (
                head.lines().next().unwrap().to_owned(),
                error,
                String::from_utf8_lossy(body).into_owned(),
            )
        };
        assert_eq!(
            status("GET /auto/v9.9 HTTP/1.1\r\n\r\n"),
            (
                "HTTP/1.1 404 Not Found".to_owned(),
                "801 Unknown Channel".to_owned(),
                "no such channel".to_owned()
            )
        );
        assert_eq!(
            status("GET /nothing HTTP/1.1\r\n\r\n").0,
            "HTTP/1.1 404 Not Found"
        );
        assert_eq!(
            status("DELETE /discover.json HTTP/1.1\r\n\r\n").0,
            "HTTP/1.1 405 Method Not Allowed"
        );
        *fake.owner.lock() = Some("Live TV is in front".into());
        assert_eq!(
            status("GET /auto/v2.4 HTTP/1.1\r\n\r\n"),
            (
                "HTTP/1.1 503 Service Unavailable".to_owned(),
                "805 All Tuners In Use".to_owned(),
                "All Tuners In Use".to_owned()
            )
        );
        let native = get(&server, "GET /auto/v2.4?consent=1 HTTP/1.1\r\n\r\n");
        let (_, body) = split(&native);
        let value: Value = serde_json::from_slice(body).unwrap();
        assert_eq!(value, serde_json::json!({
            "version": 1, "error": "tuner_busy", "takeover": null,
            "holders": [{"stream": null, "name": "Living Room TV", "kind": "tv", "channel": null, "channelName": null, "since": null, "rf": null}],
        }));
        assert!(fake.tuned.lock().is_empty());
        *fake.owner.lock() = None;
        assert_eq!(
            status("GET /auto/v2.4 HTTP/1.1\r\n\r\n"),
            (
                "HTTP/1.1 503 Service Unavailable".to_owned(),
                "807 No Video Data".to_owned(),
                "weak signal: no packets (2.4)".to_owned()
            ),
            "a channel that sends nothing"
        );
    }

    #[test]
    fn legacy_plex_retries_are_plain_503s_that_leave_the_original_stream_playing_even_with_slates_on() {
        let time = Time::default();
        let picture = [picture(0x61, 0), packet(0x64, 0)].concat();
        let fake = fake_with(&time, rf32(&[]), vec![picture.clone()]);
        fake.rf25.store(true, Ordering::Relaxed);
        let tuning = served();
        tuning.options.lock().names.insert(Ipv4Addr::LOCALHOST.into(), "Plex".into());
        let facade = Arc::new(Facade::new(
            Arc::new(Shared(fake.clone())),
            Identity { name: "LumaTV".into(), device_id: "0A1B2C3D".into() },
            Limits::default(), time.clock(), tuning.clone(), None,
        ));
        let server = Server::start(
            SocketAddr::from((Ipv4Addr::LOCALHOST, 0)), facade.clone(), vec![Cidr::parse(LOCAL[0]).unwrap()],
        ).unwrap();
        let mut original = facade.open("2.4", OTHER).unwrap();
        let key = original.lease.key.unwrap().text();
        tuning.options.lock().look = Some(Arc::new(|| Some(crate::slate::tests::look())));
        let status = || get(&server, "GET /status.json HTTP/1.1\r\n\r\n");
        let request = "GET /auto/v7.1 HTTP/1.1\r\nUser-Agent: PlexMediaServer/1.43.5\r\n\r\n";

        // Exercise the requests between writes, not after the old stream has gone away.
        struct DuringWrite<F>(Option<F>, Vec<u8>);
        impl<F: FnOnce()> Write for DuringWrite<F> {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.1.extend_from_slice(bytes);
                if let Some(check) = self.0.take() { check() }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> { Ok(()) }
        }
        let mut output = DuringWrite(Some(|| {
            let before = status();
            let (_, body) = split(&before);
            let doc: Value = serde_json::from_slice(body).unwrap();
            assert_eq!(doc["consent"], serde_json::json!({"version": 1, "takeoverMs": 30000, "staleMs": 6000}));
            assert!(!body.windows(32).any(|w| w == key.as_str().as_bytes()));
            let inspect = get(&server, "GET /auto/v7.1?consent=1&device=iPad HTTP/1.1\r\n\r\n");
            let (head, body) = split(&inspect);
            assert!(head.starts_with("HTTP/1.1 503") && head.contains("Content-Type: application/json"));
            assert_eq!(header(&head, "Cache-Control: "), "no-store");
            assert!(!body.windows(32).any(|w| w == key.as_str().as_bytes()));
            let offer: Value = serde_json::from_slice(body).unwrap();
            let since = doc["viewers"][0]["since"].as_u64().unwrap();
            assert_eq!(offer["holders"], serde_json::json!([{
                "stream": "s0", "name": OTHER.to_string(), "kind": "client", "channel": "2.4",
                "channelName": "WEST", "since": since, "rf": 32,
            }]));
            assert_eq!(offer["error"], "takeover_required");
            let token = offer["takeover"].as_str().unwrap();
            assert!(Key::parse(token).is_some());
            for seconds in [0, 5, 20] {
                time.advance(Duration::from_secs(seconds));
                let answer = get(&server, request);
                let (head, body) = split(&answer);
                assert_eq!(head.lines().next(), Some("HTTP/1.1 503 Service Unavailable"));
                assert_eq!(header(&head, "X-HDHomeRun-Error: "), "805 All Tuners In Use");
                assert_eq!(header(&head, "Content-Type: "), "text/plain");
                assert_eq!(header(&head, "Cache-Control: "), "no-store");
                assert_eq!(body, b"All Tuners In Use");
            }
            assert_eq!(status(), before, "same generation, holder and no displaced streams");
            let inspect_again = get(&server, "GET /auto/v7.1?consent=1&device=iPad HTTP/1.1\r\n\r\n");
            assert_eq!(inspect_again, inspect, "legacy retries did not invalidate the ticket");
        }), Vec::new());
        assert_eq!(original.pump(&mut output, &AtomicBool::new(false)).unwrap(), End::Silent);
        assert!(output.1.windows(picture.len()).any(|w| w == picture), "original TS continues after refusal");
        assert_eq!(*fake.tuned.lock(), ["1_32_2_4_0_0_0"]);
    }

    #[test]
    fn the_repeat_tune_reads_a_ticket_and_the_private_key_from_different_places() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let key = Key::new().unwrap();
        let ticket = Key::new().unwrap();
        write!(
            client,
            "GET /auto/v7.1?consent=1&device=iPad&takeover={} HTTP/1.1\r\nAuthorization: Bearer {}\r\n\r\n",
            ticket.text().as_str(), key.text().as_str(),
        ).unwrap();
        let (server, _) = listener.accept().unwrap();
        let request = read_request(&server).unwrap();
        assert_eq!(request.path, "/auto/v7.1");
        assert_eq!(request.device.as_deref(), Some("iPad"));
        assert!(request.structured);
        assert_eq!(request.takeover, Some(ticket));
        assert_eq!(request.stream_key, Some(key));
    }

    #[test]
    fn the_stream_send_limit_cuts_off_a_client_that_stops_reading() {
        let time = Time::default();
        let facade = Facade::new(
            Arc::new(Shared(fake_with(&time, rf32(&[]), program(4)))),
            Identity { name: "LumaTV".into(), device_id: "0A1B2C3D".into() },
            Limits::default(), time.clock(), served(), None,
        );
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut socket, _) = listener.accept().unwrap();
        let size: libc::c_int = 4096;
        // SAFETY: the option reads the size from a live c_int and the socket owns this descriptor.
        let set = unsafe {
            libc::setsockopt(socket.as_raw_fd(), libc::SOL_SOCKET, libc::SO_SNDBUF, (&raw const size).cast(), std::mem::size_of_val(&size) as libc::socklen_t)
        };
        assert_eq!(set, 0);
        let mut stream = facade.open("2.4", OTHER).unwrap();
        stream_head(&mut socket, &stream.lease, Duration::from_millis(40)).unwrap();
        struct Backpressure(TcpStream);
        impl Write for Backpressure {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                let fill = [0u8; 16 * 1024];
                for _ in 0..32 {
                    self.0.write_all(&fill)?;
                }
                panic!("the client never drains its receive buffer");
            }
            fn flush(&mut self) -> io::Result<()> { Ok(()) }
        }
        let error = stream.pump(&mut Backpressure(socket), &AtomicBool::new(false)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(facade.status_doc()["viewers"], serde_json::json!([]));
    }

    #[test]
    fn status_json_says_whether_the_converter_is_on_and_the_profiles_it_makes_best_first() {
        /// A converter that makes the two best profiles and nothing else.
        struct Two;
        impl crate::Codec for Two {
            fn open(&self, _: &crate::mpeg2::Format, _: &crate::Want) -> Result<Box<dyn crate::Convert>, crate::Fault> {
                Err(crate::Fault::Unsupported("test".to_owned()))
            }

            fn profiles(&self) -> &'static [crate::Profile] {
                &[crate::Profile::Heavy, crate::Profile::Mobile]
            }
        }
        let time = Time::default();
        let tuning = served();
        let facade = Facade::new(
            Arc::new(Shared(fake_with(&time, rf32(&[]), vec![]))),
            Identity { name: "LumaTV".into(), device_id: "0A1B2C3D".into() },
            Limits::default(),
            time.clock(),
            tuning.clone(),
            None,
        );
        let server = Server::start(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)), Arc::new(facade), vec![Cidr::parse("127.0.0.0/8").unwrap()]).unwrap();
        let transcode = || serde_json::from_slice::<Value>(split(&get(&server, "GET /status.json HTTP/1.1\r\n\r\n")).1).unwrap()["transcode"].clone();
        assert_eq!(transcode(), serde_json::json!({"enabled": false, "profiles": []}), "no converter, nothing is made");

        tuning.options.lock().codec = Some(Arc::new(Two));
        assert_eq!(
            transcode(),
            serde_json::json!({
                "enabled": true,
                "profiles": [
                    {"name": "heavy", "codec": "h264", "maxHeight": null, "progressive": true, "kbps": 9000, "interlacedFps": 30},
                    {"name": "mobile", "codec": "h264", "maxHeight": 720, "progressive": true, "kbps": 3500, "interlacedFps": 30},
                ]
            })
        );

        tuning.options.lock().codec = Some(Arc::new(converter::Fake::default()));
        let all = transcode();
        let names: Vec<&str> = all["profiles"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["heavy", "mobile", "internet540", "internet480", "internet360", "internet240"], "a converter that does not say makes them all");
    }

    #[test]
    fn a_device_name_is_unescaped_cleaned_and_cut_to_forty_characters() {
        assert_eq!(device_name("Living%20Room+iPad").as_deref(), Some("Living Room iPad"));
        assert_eq!(device_name("%0A%09").as_deref(), None);
        assert_eq!(device_name("100%").as_deref(), Some("100%"));
        assert_eq!(device_name(&"a".repeat(60)).map(|n| n.len()), Some(40));
    }

    #[test]
    fn a_peer_outside_the_allowed_networks_gets_no_answer_at_all() {
        let time = Time::default();
        let server = start(&fake_with(&time, rf32(&[]), vec![]), &time, &["10.0.0.0/8"]);
        assert!(get(&server, "GET /discover.json HTTP/1.1\r\n\r\n").is_empty());
    }

    #[test]
    fn refusals_of_one_channel_to_one_peer_are_logged_once_a_second_with_their_count() {
        let r = Refusals::default();
        let (vlc, t0) = (IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)), Instant::now());
        let busy = || "all tuners in use".to_owned();
        assert_eq!(r.refused(vlc, "2.1", busy(), t0), ["tuner: 192.168.1.50 asked for 2.1: all tuners in use"]);
        for n in 1..25u32 {
            assert!(r.refused(vlc, "2.1", busy(), t0 + Duration::from_millis(36) * n).is_empty());
        }
        assert_eq!(r.refused(vlc, "2.2", busy(), t0 + Duration::from_millis(900)).len(), 1, "another channel is its own");
        assert_eq!(
            r.flush(t0 + Duration::from_millis(1_300)),
            ["tuner: 192.168.1.50 asked for 2.1 24 more times in 0.9 s: all tuners in use"],
            "2.2's second is not over"
        );
        assert!(r.flush(t0 + Duration::from_secs(5)).is_empty(), "2.2 came once: nothing more to say");
        assert_eq!(r.refused(vlc, "2.1", busy(), t0 + Duration::from_secs(6)).len(), 1, "a new second starts with a line");
    }
}
