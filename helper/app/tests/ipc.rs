//! The loopback API over real TCP: what a QML `XMLHttpRequest` (and a hostile web page) can and cannot do.

mod support;

use lumatv_helper::http::{Handler, Options, Request, Response, Server};
use serde_json::json;
use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use support::*;

/// A rig with the receiver off and the screen on: only the API under test is alive.
fn quiet_rig() -> Rig {
    let mut rig = Rig::new();
    receiver_off(&rig.paths);
    rig.start(test_config(json!({})), Script::internet());
    rig
}

#[test]
fn health_discovery_and_state_answer_over_tcp() {
    let rig = quiet_rig();
    let health = get(rig.addr, "/v1/health");
    assert_eq!(health.status, 200);
    assert_eq!(
        health.header("content-type"),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(health.header("cache-control"), Some("no-store"));
    assert_eq!(health.json()["ok"], true);

    let root = get(rig.addr, "/");
    assert_eq!(root.status, 200);
    assert_eq!(
        root.json()["endpoints"].as_array().unwrap().len(),
        lumatv_helper::api::ROUTES.len()
            + lumatv_helper::ai::routes::ROUTES.len()
            + lumatv_helper::plugins::routes::ROUTES.len()
            + lumatv_helper::voice::ROUTES.len()
            + lumatv_helper::images::ROUTES.len()
            + lumatv_helper::gfn_boost::ROUTES.len()
            + lumatv_helper::crash::ROUTES.len()
            + lumatv_helper::capture::ROUTES.len()
    );

    let state = get(rig.addr, "/v1/state").json();
    assert_eq!(state["api"], 1);
    assert!(
        state["seq"].as_u64().unwrap() >= 1,
        "the screen topic is published at start"
    );
    assert_eq!(state["topics"]["screen"]["on"], true);
    assert_eq!(state["topics"]["screen"]["source"], "config");
    assert_eq!(state["helper"]["name"], "lumatv-helper");
}

#[test]
fn unknown_paths_and_methods() {
    let rig = quiet_rig();
    assert_eq!(get(rig.addr, "/nope").status, 404);
    assert_eq!(
        get(rig.addr, "/v1/nope").json()["error"]["code"],
        "not_found"
    );
    let r = http(
        rig.addr,
        "POST",
        "/v1/state",
        &[("X-Luma-Client", "t")],
        None,
    );
    assert_eq!(r.status, 405);
    assert_eq!(r.header("allow"), Some("GET"));
    let r = http(
        rig.addr,
        "PATCH",
        "/v1/state",
        &[("X-Luma-Client", "t")],
        None,
    );
    assert_eq!(r.status, 405);
    assert_eq!(http(rig.addr, "HEAD", "/v1/health", &[], None).status, 405);
}

#[test]
fn a_page_in_a_browser_cannot_call_the_api() {
    let rig = quiet_rig();
    // A web page always sends an Origin; QML and curl never do.
    let r = http(
        rig.addr,
        "GET",
        "/v1/state",
        &[("Origin", "http://evil.example")],
        None,
    );
    assert_eq!(r.status, 403);
    assert_eq!(r.json()["error"]["code"], "origin_not_allowed");
    assert!(r.header("access-control-allow-origin").is_none());
    let r = http(rig.addr, "GET", "/v1/state", &[("Origin", "file://")], None);
    assert_eq!(r.status, 403);
    // A preflight is refused the same way, so a browser never sends the write.
    let r = http(
        rig.addr,
        "OPTIONS",
        "/v1/settings/base.weather",
        &[
            ("Origin", "http://evil.example"),
            ("Access-Control-Request-Method", "POST"),
            ("Access-Control-Request-Headers", "x-luma-client"),
        ],
        None,
    );
    assert_eq!(r.status, 403);
}

#[test]
fn a_rebound_name_cannot_reach_the_api() {
    let rig = quiet_rig();
    for host in [
        "evil.example",
        &format!("evil.example:{}", rig.addr.port()),
        "127.0.0.1",
        "127.0.0.1:1",
        &format!("127.0.0.2:{}", rig.addr.port()),
        &format!("[::1]:{}", rig.addr.port()),
        &format!("0.0.0.0:{}", rig.addr.port()),
    ] {
        let request = format!("GET /v1/state HTTP/1.1\r\nHost: {host}\r\n\r\n");
        let r = raw(rig.addr, request.as_bytes());
        assert_eq!(r.status, 403, "{host}");
        assert_eq!(r.json()["error"]["code"], "host_not_allowed", "{host}");
    }
    // The loopback names with the right port are served, and so is a client with no Host (HTTP/1.0).
    for host in [
        format!("127.0.0.1:{}", rig.addr.port()),
        format!("localhost:{}", rig.addr.port()),
    ] {
        let request = format!("GET /v1/health HTTP/1.1\r\nHost: {host}\r\n\r\n");
        assert_eq!(raw(rig.addr, request.as_bytes()).status, 200, "{host}");
    }
    assert_eq!(
        raw(rig.addr, b"GET /v1/health HTTP/1.0\r\n\r\n").status,
        200
    );
}

#[test]
fn writes_need_the_client_header_and_a_json_body() {
    let rig = quiet_rig();
    let body = r#"{"changes":[]}"#;
    let plain = http(
        rig.addr,
        "POST",
        "/v1/settings/base.news",
        &[("Content-Type", "application/json")],
        Some(body),
    );
    assert_eq!(plain.status, 403);
    assert_eq!(plain.json()["error"]["code"], "client_header_required");
    // The classic cross-site form post: no custom header, form content type.
    let form = http(
        rig.addr,
        "POST",
        "/v1/settings/base.news",
        &[("Content-Type", "application/x-www-form-urlencoded")],
        Some("changes=1"),
    );
    assert_eq!(form.status, 403);
    let wrong_type = http(
        rig.addr,
        "POST",
        "/v1/settings/base.news",
        &[("X-Luma-Client", "t"), ("Content-Type", "text/plain")],
        Some(body),
    );
    assert_eq!(wrong_type.status, 415);
    assert_eq!(post(rig.addr, "/v1/settings/base.news", body).status, 200);
}

#[test]
fn malformed_and_oversized_requests_are_refused_without_harm() {
    let rig = quiet_rig();
    assert_eq!(raw(rig.addr, b"GARBAGE\r\n\r\n").status, 400);
    assert_eq!(
        raw(rig.addr, b"GET /v1/state HTTP/9.9\r\nHost: x\r\n\r\n").status,
        400
    );
    assert_eq!(
        raw(rig.addr, b"GET http://example.com/ HTTP/1.1\r\n\r\n").status,
        400
    );

    let long = format!(
        "GET /v1/state HTTP/1.1\r\nX-Pad: {}\r\n\r\n",
        "a".repeat(20_000)
    );
    assert_eq!(raw(rig.addr, long.as_bytes()).status, 431);

    let big = "x".repeat(70_000);
    let r = http(
        rig.addr,
        "POST",
        "/v1/settings/base.news",
        &[("X-Luma-Client", "t"), ("Content-Type", "application/json")],
        Some(&big),
    );
    assert_eq!(r.status, 413);

    let chunked = b"POST /v1/refresh HTTP/1.1\r\nX-Luma-Client: t\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n";
    assert_eq!(raw(rig.addr, chunked).status, 501);

    // A client that connects and says nothing, or hangs up inside its headers, does not wedge the server.
    let mut idle = TcpStream::connect(rig.addr).unwrap();
    idle.write_all(b"GET /v1/st").unwrap();
    drop(idle);
    assert_eq!(get(rig.addr, "/v1/health").status, 200);
}

#[test]
fn a_long_poll_waits_for_an_event_and_times_out_without_one() {
    let rig = quiet_rig();
    let hub = rig.service().hub().clone();
    let seq = get(rig.addr, "/v1/state").json()["seq"].as_u64().unwrap();
    let boot = get(rig.addr, "/v1/state").json()["boot"].as_u64().unwrap();

    let addr = rig.addr;
    let waiter = thread::spawn(move || {
        let started = Instant::now();
        let reply = get(
            addr,
            &format!("/v1/events?after={seq}&boot={boot}&timeout=20&topics=probe"),
        );
        (reply.json(), started.elapsed())
    });
    thread::sleep(Duration::from_millis(150));
    hub.set("probe", json!({"volumeDb": -20.0}));
    let (body, waited) = waiter.join().unwrap();
    let probe = body["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["topic"] == "probe")
        .expect("the probe event");
    assert_eq!(probe["data"]["volumeDb"], -20.0);
    assert!(
        waited >= Duration::from_millis(100) && waited < Duration::from_secs(5),
        "{waited:?}"
    );
    let seq = body["seq"].as_u64().unwrap();

    let started = Instant::now();
    let quiet = get(
        rig.addr,
        &format!("/v1/events?after={seq}&boot={boot}&timeout=0.3&topics=probe"),
    )
    .json();
    assert_eq!(quiet["events"], json!([]));
    assert_eq!(quiet["reset"], false);
    assert!(started.elapsed() >= Duration::from_millis(250));
}

#[test]
fn a_state_then_events_sequence_loses_nothing() {
    let rig = quiet_rig();
    let hub = rig.service().hub().clone();
    let state = get(rig.addr, "/v1/state").json();
    let (seq, boot) = (
        state["seq"].as_u64().unwrap(),
        state["boot"].as_u64().unwrap(),
    );
    for i in 0..5 {
        hub.set("counter", json!(i));
    }
    let batch = get(
        rig.addr,
        &format!("/v1/events?after={seq}&boot={boot}&timeout=1&topics=counter"),
    )
    .json();
    let values: Vec<_> = batch["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["data"].clone())
        .collect();
    assert_eq!(values, [json!(0), json!(1), json!(2), json!(3), json!(4)]);
    assert!(batch["seq"].as_u64().unwrap() >= seq + 5);
    // A client that knows another run of the helper is told to start over.
    let stale = get(
        rig.addr,
        &format!("/v1/events?after={seq}&boot={}&timeout=1", boot + 1),
    )
    .json();
    assert_eq!(stale["reset"], true);
}

#[test]
fn settings_over_http() {
    let rig = quiet_rig();
    let list = get(rig.addr, "/v1/settings").json();
    assert_eq!(list["owners"].as_array().unwrap().len(), 5);
    let change = r#"{"changes":[{"key":"units","value":{"type":"choice","value":"celsius"}}],"ifVersion":0}"#;
    let applied = post(rig.addr, "/v1/settings/base.weather", change);
    assert_eq!(
        applied.status,
        200,
        "{}",
        String::from_utf8_lossy(&applied.body)
    );
    assert_eq!(
        applied.json()["settings"]["effective"]["units"]["value"],
        "celsius"
    );
    // The change is an event, and it is on disk.
    let events = get(rig.addr, "/v1/events?after=0&topics=settings&timeout=1").json();
    let settings_event = events["events"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(
        settings_event["data"],
        json!({"owner": "base.weather", "version": 1})
    );
    assert!(rig.paths.settings_dir().join("base.weather.json").is_file());
    // Refused values say which field.
    let bad = post(
        rig.addr,
        "/v1/settings/base.weather",
        r#"{"changes":[{"key":"zip","value":{"type":"text","value":"12"}}]}"#,
    );
    assert_eq!(bad.status, 422);
    assert!(bad.json()["error"]["fields"]["zip"].is_string());
    assert_eq!(
        post(rig.addr, "/v1/settings/base.weather", change).status,
        409
    );
}

#[test]
fn stopping_ends_a_waiting_long_poll_and_frees_the_port() {
    let mut rig = quiet_rig();
    let addr = rig.addr;
    let waiter = thread::spawn(move || {
        let started = Instant::now();
        let reply = get(addr, "/v1/events?after=0&timeout=30");
        (reply.status, started.elapsed())
    });
    thread::sleep(Duration::from_millis(200));
    let stopping = Instant::now();
    rig.stop();
    assert!(
        stopping.elapsed() < Duration::from_secs(6),
        "{:?}",
        stopping.elapsed()
    );
    let (status, waited) = waiter.join().unwrap();
    assert_eq!(status, 200);
    assert!(waited < Duration::from_secs(8), "{waited:?}");
    assert!(
        TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_err(),
        "the port is still open"
    );
}

struct Slow;

impl Handler for Slow {
    fn handle(&self, _request: &Request) -> Response {
        thread::sleep(Duration::from_millis(600));
        Response::json(200, &json!({"ok": true}))
    }
}

struct Panics;

impl Handler for Panics {
    fn handle(&self, _request: &Request) -> Response {
        panic!("a bug in a handler");
    }
}

#[test]
fn only_loopback_can_be_bound() {
    for ip in ["0.0.0.0", "192.168.1.5", "::"] {
        let addr = SocketAddr::new(ip.parse().unwrap(), 0);
        assert!(
            Server::start(addr, Arc::new(Slow), Options::default()).is_err(),
            "{ip}"
        );
    }
    let ok = Server::start(
        "127.0.0.1:0".parse().unwrap(),
        Arc::new(Slow),
        Options::default(),
    )
    .unwrap();
    assert!(ok.local_addr().ip().is_loopback());
    ok.stop(Duration::from_secs(1));
}

#[test]
fn the_connection_cap_answers_busy() {
    let server = Server::start(
        "127.0.0.1:0".parse().unwrap(),
        Arc::new(Slow),
        Options {
            max_connections: 2,
            ..Options::default()
        },
    )
    .unwrap();
    let addr = server.local_addr();
    let held: Vec<_> = (0..2)
        .map(|_| {
            thread::spawn(move || {
                let r = raw(
                    addr,
                    format!("GET / HTTP/1.1\r\nHost: {addr}\r\n\r\n").as_bytes(),
                );
                r.status
            })
        })
        .collect();
    thread::sleep(Duration::from_millis(200));
    let third = raw(
        addr,
        format!("GET / HTTP/1.1\r\nHost: {addr}\r\n\r\n").as_bytes(),
    );
    assert_eq!(third.status, 503);
    assert_eq!(third.json()["error"]["code"], "busy");
    for h in held {
        assert_eq!(h.join().unwrap(), 200);
    }
    // After they finish there is room again.
    assert_eq!(
        raw(
            addr,
            format!("GET / HTTP/1.1\r\nHost: {addr}\r\n\r\n").as_bytes()
        )
        .status,
        200
    );
    server.stop(Duration::from_secs(2));
}

#[test]
fn a_panicking_handler_is_a_500_and_the_server_lives() {
    let server = Server::start(
        "127.0.0.1:0".parse().unwrap(),
        Arc::new(Panics),
        Options::default(),
    )
    .unwrap();
    let addr = server.local_addr();
    for _ in 0..3 {
        let r = raw(
            addr,
            format!("GET / HTTP/1.1\r\nHost: {addr}\r\n\r\n").as_bytes(),
        );
        assert_eq!(r.status, 500);
        assert_eq!(r.json()["error"]["code"], "internal");
    }
    server.stop(Duration::from_secs(1));
}

#[test]
fn an_allowed_origin_is_answered_with_its_cors_headers_and_others_still_refused() {
    let server = Server::start(
        "127.0.0.1:0".parse().unwrap(),
        Arc::new(Slow),
        Options {
            allowed_origins: vec!["file://".to_owned()],
            ..Options::default()
        },
    )
    .unwrap();
    let addr = server.local_addr();
    let r = http(addr, "GET", "/", &[("Origin", "file://")], None);
    assert_eq!(r.status, 200);
    assert_eq!(r.header("access-control-allow-origin"), Some("file://"));
    assert_eq!(r.header("vary"), Some("Origin"));
    let pre = http(addr, "OPTIONS", "/", &[("Origin", "file://")], None);
    assert_eq!(pre.status, 204);
    assert!(
        pre.header("access-control-allow-headers")
            .unwrap()
            .contains("x-luma-client")
    );
    assert_eq!(
        http(addr, "GET", "/", &[("Origin", "http://evil.example")], None).status,
        403
    );
    server.stop(Duration::from_secs(2));
}

#[test]
fn a_pages_subresource_request_is_refused_by_its_fetch_metadata() {
    let rig = quiet_rig();
    // An <img> or <iframe> on a web page carries no Origin but says where it comes from; it could only hold connections.
    for site in ["cross-site", "same-site", "same-origin"] {
        let r = http(
            rig.addr,
            "GET",
            "/v1/events?after=0&timeout=30",
            &[("Sec-Fetch-Site", site)],
            None,
        );
        assert_eq!(r.status, 403, "{site}");
        assert_eq!(
            r.json()["error"]["code"],
            "cross_site_not_allowed",
            "{site}"
        );
    }
    // Typing the address into a browser is `none`; QML and curl send no such header.
    assert_eq!(
        http(
            rig.addr,
            "GET",
            "/v1/health",
            &[("Sec-Fetch-Site", "none")],
            None
        )
        .status,
        200
    );
    assert_eq!(get(rig.addr, "/v1/health").status, 200);
}

/// The headers Qt 5.12's QML `XMLHttpRequest` (QNetworkAccessManager) sends: a Mozilla user agent with Qt in it, keep-alive,
/// gzip, a language, and no `Origin` and no `Sec-Fetch-*` (QML implements no CORS). The helper must serve exactly this.
const QT_HEADERS: &str = "User-Agent: Mozilla/5.0 (Linux; Qt/5.12.4) AppleWebKit/602.1 (KHTML, like Gecko)\r\nAccept: */*\r\nAccept-Language: en-US,*\r\nAccept-Encoding: gzip, deflate\r\nConnection: Keep-Alive\r\n";

#[test]
fn a_request_as_qt_5_12s_xmlhttprequest_sends_it_is_served() {
    let rig = quiet_rig();
    let host = rig.addr;
    let read = format!("GET /v1/state HTTP/1.1\r\nHost: {host}\r\n{QT_HEADERS}\r\n");
    let reply = raw(rig.addr, read.as_bytes());
    assert_eq!(reply.status, 200);
    assert!(
        reply.header("content-length").is_some(),
        "QML needs a length or a close to know the body is whole"
    );
    assert_eq!(reply.header("connection"), Some("close"));
    assert_eq!(
        reply.header("content-encoding"),
        None,
        "nothing is compressed, whatever Accept-Encoding says"
    );
    assert_eq!(reply.json()["api"], 1);

    let long_poll =
        format!("GET /v1/events?after=0&timeout=0.2 HTTP/1.1\r\nHost: {host}\r\n{QT_HEADERS}\r\n");
    assert_eq!(raw(rig.addr, long_poll.as_bytes()).status, 200);

    let body = r#"{"changes":[{"key":"units","value":{"type":"choice","value":"celsius"}}]}"#;
    let write = format!(
        "POST /v1/settings/base.weather HTTP/1.1\r\nHost: {host}\r\n{QT_HEADERS}X-Luma-Client: home\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let reply = raw(rig.addr, write.as_bytes());
    assert_eq!(
        reply.status,
        200,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    assert_eq!(
        reply.json()["settings"]["effective"]["units"]["value"],
        "celsius"
    );
}

#[test]
fn the_body_is_utf8_with_the_charset_declared_so_a_degree_sign_survives_qml() {
    let rig = quiet_rig();
    rig.service().hub().set(
        "probe",
        json!({"temp": "54\u{b0}", "stats": "Feels like 50\u{b0} \u{b7} 72%"}),
    );
    let reply = get(rig.addr, "/v1/state/probe");
    assert_eq!(
        reply.header("content-type"),
        Some("application/json; charset=utf-8")
    );
    let text = String::from_utf8(reply.body.clone()).expect("the body is UTF-8");
    assert!(
        text.contains("54\u{b0}") && text.contains("\u{b7}"),
        "{text}"
    );
    assert_eq!(reply.json()["value"]["temp"], "54\u{b0}");
}
