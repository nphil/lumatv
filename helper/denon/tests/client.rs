//! The client against a fake receiver on the loopback: the reply formats the set's receiver uses, and every way a read fails.

use lumatv_denon::{Client, ClientError, ParseError, Power, STATUS_LITE_PATH};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

const STATUS_ON_THE_SET: &str = include_str!("fixtures/status-lite-on-tv.xml");

/// The headers the set's receiver (mongoose 2.5) sends: HTTP/1.0, no `Content-Length`.
const RECEIVER_HEAD: &str = "HTTP/1.0 200 OK\r\nServer: mongoose2.5\r\nPragma: no-cache\r\nCache-control: no-cache\r\nContent-Type: text/xml; charset=utf-8\r\n\r\n";

/// A one-shot server: accepts one connection, reads the request, writes `reply`, closes. The handle yields the request text.
fn serve_once(reply: Vec<u8>) -> (u16, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buf = [0u8; 512];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            request.extend_from_slice(&buf[..n]);
        }
        let _ = stream.write_all(&reply);
        String::from_utf8_lossy(&request).into_owned()
    });
    (port, handle)
}

fn client(port: u16) -> Client {
    Client::new("127.0.0.1", port).with_timeouts(Duration::from_millis(500), Duration::from_secs(2))
}

#[test]
fn reads_the_reply_as_the_set_sends_it() {
    let mut reply = RECEIVER_HEAD.as_bytes().to_vec();
    reply.extend_from_slice(STATUS_ON_THE_SET.as_bytes());
    let (port, request) = serve_once(reply);
    let status = client(port).status().unwrap();
    assert_eq!(status.power, Power::On);
    assert_eq!(status.input.as_deref(), Some("TV"));
    assert_eq!(status.volume_db, Some(-44.5));
    assert_eq!(status.mute, Some(false));
    let request = request.join().unwrap();
    assert!(
        request.starts_with(&format!("GET {STATUS_LITE_PATH} HTTP/1.0\r\n")),
        "{request}"
    );
    assert!(request.contains(&format!("Host: 127.0.0.1:{port}\r\n")));
}

#[test]
fn a_declared_length_ends_the_read_without_waiting_for_the_close() {
    let body = STATUS_ON_THE_SET.as_bytes();
    let mut reply = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    reply.extend_from_slice(body);
    // The server keeps the connection open after the reply.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 512];
        let _ = stream.read(&mut buf);
        stream.write_all(&reply).unwrap();
        thread::sleep(Duration::from_millis(1500));
    });
    let started = Instant::now();
    assert!(client(port).status().is_ok());
    assert!(started.elapsed() < Duration::from_millis(1200));
    server.join().unwrap();
}

#[test]
fn a_forbidden_page_is_an_http_error() {
    let (port, _) = serve_once(
        b"HTTP/1.0 403 Forbidden\r\n\r\nError 403: Forbidden\r\nAccess Forbidden\r\n".to_vec(),
    );
    assert_eq!(client(port).status(), Err(ClientError::Http(403)));
}

#[test]
fn a_page_that_is_not_the_status_is_a_parse_error() {
    let mut reply = RECEIVER_HEAD.as_bytes().to_vec();
    reply.extend_from_slice(b"<html><body>hello</body></html>");
    let (port, _) = serve_once(reply);
    assert_eq!(
        client(port).status(),
        Err(ClientError::Parse(ParseError::NotStatus))
    );
}

#[test]
fn a_body_shorter_than_declared_is_refused() {
    let (port, _) = serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 500\r\n\r\n<item>".to_vec());
    assert_eq!(
        client(port).status(),
        Err(ClientError::BadResponse("body shorter than declared"))
    );
}

#[test]
fn a_reply_far_larger_than_a_status_is_refused() {
    let mut reply = RECEIVER_HEAD.as_bytes().to_vec();
    reply.extend(std::iter::repeat_n(b'x', 80 * 1024));
    let (port, _) = serve_once(reply);
    assert_eq!(client(port).status(), Err(ClientError::TooLarge));
}

#[test]
fn a_receiver_that_never_answers_times_out_at_the_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        thread::sleep(Duration::from_millis(1500));
    });
    let started = Instant::now();
    let result = Client::new("127.0.0.1", port)
        .with_timeouts(Duration::from_millis(200), Duration::from_millis(300))
        .status();
    assert_eq!(result, Err(ClientError::Timeout));
    assert!(started.elapsed() < Duration::from_millis(1200));
    server.join().unwrap();
}

#[test]
fn a_trickle_cannot_outlast_the_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 512];
        let _ = stream.read(&mut buf);
        for _ in 0..40 {
            if stream.write_all(b"x").is_err() {
                return;
            }
            thread::sleep(Duration::from_millis(40));
        }
    });
    let started = Instant::now();
    let result = Client::new("127.0.0.1", port)
        .with_timeouts(Duration::from_millis(200), Duration::from_millis(400))
        .status();
    assert_eq!(result, Err(ClientError::Timeout));
    assert!(started.elapsed() < Duration::from_millis(1200));
    drop(server);
}

#[test]
fn a_closed_port_is_unreachable() {
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    match client(port).status() {
        Err(ClientError::Unreachable(why)) => assert_eq!(why, "connection refused"),
        other => panic!("{other:?}"),
    }
}
