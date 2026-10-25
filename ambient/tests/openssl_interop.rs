//! The DTLS client against OpenSSL's own `s_server`, a second, independent implementation of DTLS 1.2 PSK
//! (`PSK-AES128-GCM-SHA256`), which is what Bifrost builds on. Loopback only; nothing leaves the machine.
//!
//! The test needs the `openssl` command; without it the test says so and passes, unless LUMATV_REQUIRE_OPENSSL is
//! set (a gate that must not skip sets it).

use lumatv_ambient::dtls::{PskClient, PskConfig};
use lumatv_ambient::huestream::{self, ColorSpace, Session, SessionConfig};
use std::io::Read;
use std::net::{SocketAddr, UdpSocket};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const PSK_HEX: &str = "00112233445566778899aabbccddeeff";
const IDENTITY: &str = "0b9d5c1e-3f6a-4d2b-8a11-6e0c2f9b7a44";
const CONFIG: &str = "7c6ab1f2-3e44-4a5e-9d1b-0c8f6a2d9e10";

fn have_openssl() -> bool {
    let ok = Command::new("openssl")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ok {
        assert!(
            std::env::var_os("LUMATV_REQUIRE_OPENSSL").is_none(),
            "openssl is required here and missing"
        );
        eprintln!("SKIP: no `openssl` command, interop with s_server not exercised");
    }
    ok
}

struct Server {
    child: Child,
    _stdin: ChildStdin, // s_server exits when its stdin closes
    out: Arc<Mutex<Vec<u8>>>,
    addr: SocketAddr,
}

impl Server {
    fn start(listen: bool) -> Server {
        let port = UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut cmd = Command::new("openssl");
        cmd.args([
            "s_server",
            "-dtls1_2",
            "-accept",
            &format!("127.0.0.1:{port}"),
            "-nocert",
            "-psk",
            PSK_HEX,
            "-psk_identity",
            IDENTITY,
        ])
        .args(["-cipher", "PSK-AES128-GCM-SHA256"]);
        if listen {
            cmd.arg("-listen");
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn openssl s_server");
        let stdin = child.stdin.take().unwrap();
        let out = Arc::new(Mutex::new(Vec::new()));
        for mut pipe in [
            Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>,
            Box::new(child.stderr.take().unwrap()),
        ] {
            let out = out.clone();
            thread::spawn(move || {
                let mut b = [0u8; 4096];
                while let Ok(n) = pipe.read(&mut b) {
                    if n == 0 {
                        break;
                    }
                    out.lock().unwrap().extend_from_slice(&b[..n]);
                }
            });
        }
        let s = Server {
            child,
            _stdin: stdin,
            out,
            addr: ([127, 0, 0, 1], port).into(),
        };
        assert!(
            s.wait_for(b"ACCEPT", Duration::from_secs(5)),
            "s_server did not start: {}",
            s.dump()
        );
        s
    }

    fn wait_for(&self, needle: &[u8], within: Duration) -> bool {
        let t = Instant::now();
        while t.elapsed() < within {
            if self
                .out
                .lock()
                .unwrap()
                .windows(needle.len())
                .any(|w| w == needle)
            {
                return true;
            }
            thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn dump(&self) -> String {
        String::from_utf8_lossy(&self.out.lock().unwrap()).into_owned()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn psk() -> Vec<u8> {
    huestream::decode_client_key(PSK_HEX).unwrap()
}

#[test]
fn handshake_and_data_against_openssl_s_server() {
    if !have_openssl() {
        return;
    }
    let s = Server::start(false);
    let mut c = PskClient::connect(s.addr, &PskConfig::new(IDENTITY.as_bytes(), &psk()))
        .unwrap_or_else(|e| panic!("{e}\n{}", s.dump()));
    // Distinct, binary-safe payloads, several records, so sequence numbers and nonces advance.
    for i in 0..5u8 {
        c.send(format!("lumatv-ambient-record-{i}\n").as_bytes())
            .unwrap();
    }
    assert!(
        s.wait_for(b"lumatv-ambient-record-4\n", Duration::from_secs(3)),
        "{}",
        s.dump()
    );
    let all = s.dump();
    for i in 0..5 {
        assert!(
            all.contains(&format!("lumatv-ambient-record-{i}\n")),
            "record {i} missing:\n{all}"
        );
    }
    assert!(
        all.contains("PSK-AES128-GCM-SHA256") || all.contains("Cipher"),
        "{all}"
    );
    c.close();
}

#[test]
fn handshake_with_a_hello_verify_cookie_against_openssl() {
    if !have_openssl() {
        return;
    }
    let s = Server::start(true);
    let mut c = PskClient::connect(s.addr, &PskConfig::new(IDENTITY.as_bytes(), &psk()))
        .unwrap_or_else(|e| panic!("{e}\n{}", s.dump()));
    c.send(b"after-the-cookie-round\n").unwrap();
    assert!(
        s.wait_for(b"after-the-cookie-round\n", Duration::from_secs(3)),
        "{}",
        s.dump()
    );
}

#[test]
fn a_wrong_key_is_refused_by_openssl() {
    if !have_openssl() {
        return;
    }
    let s = Server::start(false);
    let mut cfg = PskConfig::new(IDENTITY.as_bytes(), &[0xAA; 16]);
    cfg.handshake_timeout = Duration::from_secs(2);
    assert!(PskClient::connect(s.addr, &cfg).is_err());
}

#[test]
fn the_psk_identity_reaches_openssl_exactly() {
    if !have_openssl() {
        return;
    }
    // s_server only warns about an identity it did not expect (it does not refuse), so the log shows what arrived.
    let right = Server::start(false);
    let _c = PskClient::connect(right.addr, &PskConfig::new(IDENTITY.as_bytes(), &psk()))
        .unwrap_or_else(|e| panic!("{e}\n{}", right.dump()));
    assert!(!right.dump().contains("PSK warning"), "{}", right.dump());

    let wrong = Server::start(false);
    let _c = PskClient::connect(wrong.addr, &PskConfig::new(b"someone-else", &psk()))
        .unwrap_or_else(|e| panic!("{e}\n{}", wrong.dump()));
    assert!(
        wrong.wait_for(b"got 'someone-else'", Duration::from_secs(2)),
        "{}",
        wrong.dump()
    );
}

#[test]
fn a_huestream_session_reaches_openssl_as_whole_packets() {
    if !have_openssl() {
        return;
    }
    let s = Server::start(false);
    let mut cfg = SessionConfig::new(s.addr, IDENTITY, PSK_HEX, CONFIG);
    cfg.colour_space = ColorSpace::Rgb;
    let mut session = Session::connect(&cfg).unwrap_or_else(|e| panic!("{e}\n{}", s.dump()));
    let frame = [(0u8, [0xFFFFu16, 0x0000, 0x8000]), (3, [1, 2, 3])];
    session.send(&frame).unwrap();
    let expected = huestream::encode(0, ColorSpace::Rgb, CONFIG, &frame).unwrap();
    assert!(
        s.wait_for(&expected, Duration::from_secs(3)),
        "the packet did not arrive intact:\n{}",
        s.dump()
    );
    session.close();
}
