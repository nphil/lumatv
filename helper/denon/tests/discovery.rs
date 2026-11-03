//! Discovery against a receiver that ignores AppCommand XML without a declaration.
use lumatv_denon::Client;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

const INFO: &str = include_str!("fixtures/denon-device-capabilities.xml");
const BASIC: &str = include_str!("fixtures/denon-basic-discovery.xml");
const AUDYSSEY: &str = include_str!("fixtures/denon-audyssey-discovery.xml");

#[test]
fn declared_appcommand_envelope_discovers_receiver_capabilities() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let mut accepted = Vec::new();
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut request = Vec::new();
            let (head_len, body_len) = loop {
                let mut buf = [0; 1024];
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buf[..n]);
                if let Some(pos) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head_len = pos + 4;
                    let head = std::str::from_utf8(&request[..pos]).unwrap();
                    let length = head.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().unwrap())
                    }).unwrap_or(0);
                    break (head_len, length);
                }
            };
            while request.len() < head_len + body_len {
                let mut buf = [0; 1024];
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buf[..n]);
            }
            assert_eq!(request.len(), head_len + body_len);
            let head = std::str::from_utf8(&request[..head_len]).unwrap();
            let body = std::str::from_utf8(&request[head_len..]).unwrap();
            let response = if head.starts_with("GET /goform/Deviceinfo.xml ") {
                INFO
            } else {
                let declaration = body.starts_with("<?xml ")
                    && body.split_once("?>\r\n").is_some_and(|(declaration, xml)| {
                        declaration.contains("utf-8") && xml.starts_with("<tx>")
                    });
                let charset = head.lines().any(|line| line.eq_ignore_ascii_case("Content-Type: text/xml; charset=utf-8"));
                accepted.push(declaration && charset);
                if !(declaration && charset) {
                    "<rx></rx>"
                } else if head.starts_with("POST /goform/AppCommand.xml ") {
                    assert!(body.contains("GetDeletedSource"));
                    BASIC
                } else {
                    assert!(head.starts_with("POST /goform/AppCommand0300.xml "));
                    assert!(body.contains("GetAudyssey"));
                    AUDYSSEY
                }
            };
            write!(stream, "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{}", response.len(), response).unwrap();
        }
        accepted
    });
    let result = Client::new("127.0.0.1", port)
        .with_timeouts(Duration::from_millis(500), Duration::from_secs(2))
        .settings_prerequisites().unwrap();
    assert_eq!(server.join().unwrap(), vec![true, true]);
    let sources = result.main_sources.unwrap();
    let names: Vec<_> = sources.iter().map(|s| (s.id.as_str(), s.name.as_str())).collect();
    assert_eq!(names, vec![("MPLAY", "Apple TV"), ("BD", "MAGNAVOX 4K BD"), ("GAME", "PS4"), ("TV", "TV Audio"), ("NET", "HEOS Music"), ("BT", "Bluetooth")]);
    assert_eq!(result.tone_available, Some(false));
    assert_eq!(result.tone_enabled, None);
    // An unrecognized explicit control code is not proof of availability.
    assert_eq!(result.dynamic_eq_available, None);
    assert_eq!(result.reference_level_available, None);
    assert_eq!(result.dynamic_volume_available, None);
    assert_eq!(result.mult_eq_available, None);
    assert_eq!(result.zone2_present, None);
    assert_eq!(result.zone2_sources, None);
    assert_eq!(result.zone2_volume_variable, Some(true));
    assert_eq!(result.zone2_volume_limit, Some(70.0));
}
