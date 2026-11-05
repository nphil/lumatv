//! The facade's HTTP server: a few GETs on the LAN, one of which does not end. A stream that runs for as long as a recording needs a
//! listener of its own: one thread per connection, no body length, the connection closed when the program is over (what an HDHomeRun
//! does). Only peers in the allowed networks are answered; the rest are dropped.

use crate::docs;
use crate::net::Cidr;
use crate::stream::{End, Facade, Failure};
use serde_json::Value;
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
/// A client that stops reading for this long loses its stream.
const SEND_WAIT: Duration = Duration::from_secs(10);

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
            let (facade, stop, active) = (facade.clone(), stop.clone(), active.clone());
            move || {
                serve(stream, peer, &facade, &stop);
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
}

/// The request line and the Host header; a request that is not read whole within a moment, or is not HTTP, is nothing.
fn read_request(stream: &TcpStream) -> Option<Request> {
    stream.set_read_timeout(Some(REQUEST_WAIT)).ok()?;
    let mut lines = BufReader::new(stream.take(REQUEST_LIMIT)).lines();
    let first = lines.next()?.ok()?;
    let mut parts = first.split_whitespace();
    let (method, target, version) = (parts.next()?, parts.next()?, parts.next()?);
    if !version.starts_with("HTTP/1.") {
        return None;
    }
    let mut host = None;
    for line in lines {
        let line = line.ok()?;
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("host")
        {
            host = Some(value.trim().to_owned());
        }
    }
    let path = target.split('?').next().unwrap_or(target).to_owned();
    Some(Request {
        method: method.to_owned(),
        path,
        host,
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

fn answer(
    out: &mut impl Write,
    status: u16,
    content_type: &str,
    headers: &str,
    body: &[u8],
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
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

fn serve(mut stream: TcpStream, peer: IpAddr, facade: &Facade, stop: &AtomicBool) {
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
        ("GET", "/lineup.json") => match facade.lineup() {
            Ok(entries) => json(&mut stream, &docs::lineup(&entries, &base)),
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
            match facade.open(guide, peer) {
                Err(f) => {
                    log::info!("tuner: {peer} asked for {guide}: {}", f.reason());
                    refuse(&mut stream, &f)
                }
                Ok(mut program) => {
                    // the PAT and the PMT go out as soon as the head is written: this is the client's wait for its first byte
                    let [tune, tables, picture] = program.took.map(|d| d.as_secs_f64());
                    log::info!(
                        "tuner: {peer} streams {guide} after {:.1} s (tune {tune:.1}, tables {tables:.1}, picture {picture:.1})",
                        asked.elapsed().as_secs_f64()
                    );
                    let sent = write!(stream, "HTTP/1.0 200 OK\r\nContent-Type: video/mp2t\r\nCache-Control: no-cache\r\n\r\n").and_then(|()| program.pump(&mut stream, stop));
                    match sent {
                        Ok(End::Stopped) => log::info!("tuner: {guide} stopped"),
                        Ok(End::Replaced) => {
                            log::info!("tuner: {guide} gave the tuner to the next request")
                        }
                        Ok(end) => log::warn!("tuner: {guide} ended: {end:?}"),
                        Err(e) => log::info!("tuner: {peer} left {guide}: {e}"),
                    }
                    Ok(())
                }
            }
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
    use crate::stream::tests::{Fake, Shared, Time, fake_with, packet, picture, rf32, served};
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
        assert!(body.len() % 188 == 0 && body[0] == 0x47);
        assert!(
            body.windows(chunk.len()).any(|w| w == &chunk[..]),
            "the tuner's packets arrive whole"
        );
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
                "the TV is in use: Live TV is in front".to_owned()
            )
        );
        assert!(fake.tuned.lock().is_empty());
        *fake.owner.lock() = None;
        assert_eq!(
            status("GET /auto/v2.4 HTTP/1.1\r\n\r\n"),
            (
                "HTTP/1.1 503 Service Unavailable".to_owned(),
                "807 No Video Data".to_owned(),
                "no signal: no picture (2.4)".to_owned()
            ),
            "a channel that sends no picture"
        );
    }

    #[test]
    fn a_peer_outside_the_allowed_networks_gets_no_answer_at_all() {
        let time = Time::default();
        let server = start(&fake_with(&time, rf32(&[]), vec![]), &time, &["10.0.0.0/8"]);
        assert!(get(&server, "GET /discover.json HTTP/1.1\r\n\r\n").is_empty());
    }
}
