//! The client against a fake receiver on the loopback: the reply formats the set's receiver uses, and every way a read fails.

use lumatv_denon::{
    Client, ClientError, ParseError, Power, STATUS_LITE_PATH, mute_command, volume_command,
};
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
    let mut reply =
        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
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

#[test]
fn a_host_name_is_resolved_once_and_read_again_after_a_failure() {
    let mut reply = RECEIVER_HEAD.as_bytes().to_vec();
    reply.extend_from_slice(STATUS_ON_THE_SET.as_bytes());
    let (port, request) = serve_once(reply.clone());
    let named = Client::new("localhost", port)
        .with_timeouts(Duration::from_millis(500), Duration::from_secs(2));
    assert_eq!(named.status().unwrap().power, Power::On);
    assert!(
        request
            .join()
            .unwrap()
            .contains(&format!("Host: localhost:{port}"))
    );
    // The server is gone: the failure is reported as unreachable, and the next read looks the name up again.
    assert!(matches!(
        named.status(),
        Err(ClientError::Unreachable(_) | ClientError::Timeout)
    ));
    let listener = TcpListener::bind(("127.0.0.1", port));
    if let Ok(listener) = listener {
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 512];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(&reply);
        });
        assert_eq!(named.status().unwrap().power, Power::On);
        server.join().unwrap();
    }
}

#[test]
fn a_name_that_does_not_resolve_is_unreachable() {
    let result = Client::new("no-such-host.invalid", 8080)
        .with_timeouts(Duration::from_millis(300), Duration::from_secs(1))
        .status();
    assert!(
        matches!(result, Err(ClientError::Unreachable(_))),
        "{result:?}"
    );
}

// ------------------------------------------------------------------------------------------------ commands

#[test]
fn a_command_is_one_get_and_an_empty_reply_is_success() {
    let (port, request) = serve_once(b"HTTP/1.0 200 OK\r\nServer: mongoose2.5\r\n\r\n".to_vec());
    assert_eq!(client(port).send_command("MV355"), Ok(()));
    let request = request.join().unwrap();
    assert!(
        request.starts_with("GET /goform/formiPhoneAppDirect.xml?MV355 HTTP/1.0\r\n"),
        "{request}"
    );
}

#[test]
fn a_command_accepts_any_body_on_a_2xx() {
    let (port, _) = serve_once(b"HTTP/1.0 200 OK\r\n\r\n<html>ok</html>".to_vec());
    assert_eq!(client(port).send_command("MUON"), Ok(()));
}

#[test]
fn a_refused_command_is_an_http_error() {
    let (port, _) = serve_once(b"HTTP/1.0 403 Forbidden\r\n\r\nAccess Forbidden\r\n".to_vec());
    assert_eq!(
        client(port).send_command("MV35"),
        Err(ClientError::Http(403))
    );
}

#[test]
fn a_command_to_a_closed_port_is_unreachable() {
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    assert!(matches!(
        client(port).send_command("MV35"),
        Err(ClientError::Unreachable(_))
    ));
}

#[test]
fn an_invalid_command_sends_nothing() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let long = "A".repeat(17);
    let cases = [
        "",
        "M",
        "mv35",
        "MV 35",
        "MV35\r\n",
        "MV35 HTTP/1.1\r\nX: y",
        "MV35&Z=1",
        "MV35#",
        "MV/35",
        "MV\u{0662}5",
        "MÜ35",
        long.as_str(),
    ];
    for command in cases {
        assert_eq!(
            client(port).send_command(command),
            Err(ClientError::InvalidCommand),
            "{command:?}"
        );
    }
    assert_eq!(ClientError::InvalidCommand.code(), "invalid_command");
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn the_shortest_and_longest_commands_and_queries_are_valid() {
    let longest = "A".repeat(16);
    for command in ["MV", "PW?", longest.as_str()] {
        let (port, request) = serve_once(b"HTTP/1.0 200 OK\r\n\r\n".to_vec());
        assert_eq!(client(port).send_command(command), Ok(()), "{command}");
        assert!(
            request
                .join()
                .unwrap()
                .contains(&format!("?{command} HTTP/1.0"))
        );
    }
}

#[test]
fn volume_commands_round_to_the_nearest_half_with_ties_up() {
    let table: [(f64, Option<&str>); 22] = [
        (0.0, Some("MV00")),
        (-0.0, Some("MV00")),
        (0.24, Some("MV00")),
        (0.25, Some("MV005")),
        (0.5, Some("MV005")),
        (0.74, Some("MV005")),
        (0.75, Some("MV01")),
        (5.0, Some("MV05")),
        (5.5, Some("MV055")),
        (35.0, Some("MV35")),
        (35.24, Some("MV35")),
        (35.25, Some("MV355")),
        (35.5, Some("MV355")),
        (35.74, Some("MV355")),
        (35.75, Some("MV36")),
        (97.5, Some("MV975")),
        (98.0, Some("MV98")),
        (98.01, None),
        (-0.01, None),
        (f64::NAN, None),
        (f64::INFINITY, None),
        (f64::NEG_INFINITY, None),
    ];
    for (level, want) in table {
        assert_eq!(volume_command(level).as_deref(), want, "{level}");
    }
}

#[test]
fn every_volume_command_is_a_valid_command() {
    let mut level = 0.0;
    while level <= 98.0 {
        let command = volume_command(level).unwrap();
        assert!(command.len() == 4 || command.len() == 5, "{command}");
        level += 0.25;
    }
}

#[test]
fn mute_commands() {
    assert_eq!(mute_command(true), "MUON");
    assert_eq!(mute_command(false), "MUOFF");
}
