//! Hue Entertainment streaming end to end, against a fake bridge on the loopback: HTTPS (CLIP v2 and the v1 registration) from python's
//! `ssl`, and DTLS 1.2 PSK from OpenSSL's `s_server`, started when the area is started and killed when it is stopped
//! (`support/fake_hue_bridge.py`). A real helper runs in a temporary data dir with the real screen watcher (a fake `luna-send`) and
//! the capture socket. Nothing here goes beyond 127.0.0.1; no real bridge is touched.
//!
//! What is held to: the link is pinned to the certificate the bridge showed and nothing is sent to one that shows another; the
//! keys are used (the fake refuses a request without the application key) and never printed; the area is read, started over HTTPS,
//! and only then streamed to over DTLS with the application id as identity and the client key as the key; the stream carries the
//! still colour or the picture's colours on the area's channels, mapped by where each sits; the screen going off (or the lights
//! fading) sends black, closes the session and stops the area; an area in use is not taken; a stream the bridge ends is started
//! again; and the keys on the command line side stay out of every output.

mod support;

use fetch::tls_fixture::{Identity, Pki};
use lumatv_ambient::huestream::{self, Parsed};
use lumatv_ambient::plan::Still;
use lumatv_ambient::zones::{MAX_ZONES, Preset, ZoneFrame};
use lumatv_ambient::{Rgb, wire};
use lumatv_helper::hue_rest::{self, Tls};
use lumatv_helper::hue_store::{self, HueLink};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, UdpSocket};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};
use support::*;

const BIN: &str = env!("CARGO_BIN_EXE_lumatv-helper");
const WAIT: Duration = Duration::from_secs(20);

const APP_KEY: &str = "0123456789abcdef0123456789abcdef01234567";
const APP_ID: &str = "0b9d5c1e-3f6a-4d2b-8a11-6e0c2f9b7a44";
const CLIENT_KEY: &str = "00112233445566778899aabbccddeeff";
const AREA: &str = "7c6ab1f2-3e44-4a5e-9d1b-0c8f6a2d9e10";
const OTHER_AREA: &str = "5a1d7e90-2b3c-4f64-8e55-0d9c1a7b3f22";
const OTHER_APP: &str = "99999999-3f6a-4d2b-8a11-6e0c2f9b7a44";
/// Three channels: left, top middle, right.
const CHANNELS: usize = 3;
const PACKET: usize = 16 + 36 + 7 * CHANNELS;

fn have_tools() -> bool {
    let ok = |p: &str, a: &str| {
        Command::new(p)
            .arg(a)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    let both = ok("openssl", "version") && ok("python3", "--version");
    if !both {
        assert!(
            std::env::var_os("LUMATV_REQUIRE_OPENSSL").is_none(),
            "openssl and python3 are required here and missing"
        );
        eprintln!("SKIP: openssl or python3 is missing, the fake bridge cannot run");
    }
    both
}

/// The fake bridge: a python process, and the files it writes.
struct Fake {
    child: Child,
    stdin: ChildStdin,
    port: u16,
    udp: u16,
    dir: tempfile::TempDir,
    _pki: Pki,
}

impl Fake {
    fn start(shows: impl FnOnce(&Pki) -> Identity, link_button: bool, extra_area: bool) -> Fake {
        let pki = Pki::new();
        let identity = shows(&pki);
        let dir = tempfile::tempdir().unwrap();
        let udp = UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let config = json!({
            "cert": identity.cert, "key": identity.key, "dir": dir.path(), "udp_port": udp,
            "app_key": APP_KEY, "app_id": APP_ID, "client_key": CLIENT_KEY, "area": AREA, "name": "TV",
            "channels": [
                {"channel_id": 0, "x": -0.8, "y": 1.0, "z": 0.5},
                {"channel_id": 1, "x": 0.0, "y": 1.0, "z": 0.9},
                {"channel_id": 2, "x": 0.8, "y": 1.0, "z": 0.5},
            ],
            "link_button": link_button,
            "extra_area": if extra_area { json!(OTHER_AREA) } else { Value::Null },
        });
        let config_path = dir.path().join("config.json");
        std::fs::write(&config_path, config.to_string()).unwrap();
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/fake_hue_bridge.py");
        let mut child = Command::new("python3")
            .arg(script)
            .arg(&config_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("python3 starts the fake bridge");
        let stdin = child.stdin.take().unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let port = line
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("the fake bridge did not start: {line:?}"));
        Fake {
            child,
            stdin,
            port,
            udp,
            dir,
            _pki: pki,
        }
    }

    fn addr(&self) -> SocketAddr {
        ([127, 0, 0, 1], self.port).into()
    }

    /// A command to the fake (see the script).
    fn say(&mut self, line: &str) {
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// Every HTTPS request that reached the application layer.
    fn requests(&self) -> Vec<Value> {
        std::fs::read_to_string(self.dir.path().join("requests.log"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn count(&self, method: &str, action: &str) -> usize {
        self.requests()
            .iter()
            .filter(|r| {
                r["method"] == method
                    && r["path"].as_str().unwrap().ends_with(AREA)
                    && r["body"].as_str().unwrap().contains(action)
            })
            .count()
    }

    fn starts(&self) -> usize {
        self.count("PUT", "start")
    }

    fn stops(&self) -> usize {
        self.count("PUT", "stop")
    }

    /// Every whole HueStream packet the DTLS side has received.
    fn packets(&self) -> Vec<Parsed> {
        let bytes = std::fs::read(self.dir.path().join("dtls.out")).unwrap_or_default();
        let magic = b"HueStream";
        let mut out = Vec::new();
        let mut from = 0;
        while let Some(at) = bytes[from..].windows(9).position(|w| w == magic) {
            let start = from + at;
            if let Some(p) = bytes.get(start..start + PACKET).and_then(huestream::parse) {
                out.push(p);
            }
            from = start + 9;
        }
        out
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// What `hue-link` does, without the command line: pin the certificate the bridge shows and save the link.
fn link(data_dir: &Path, fake: &Fake, area: Option<&str>) {
    let tls = Tls::new("openssl", None);
    let pem = hue_rest::scan_certificate(&tls, fake.addr()).unwrap();
    hue_store::stage_certificate(data_dir, &pem).unwrap();
    hue_store::settle_certificate(data_dir, true).unwrap();
    let link = HueLink::new("127.0.0.1", fake.port, fake.udp, area, APP_KEY, CLIENT_KEY).unwrap();
    hue_store::save(data_dir, &link).unwrap();
}

/// A helper in a temporary data dir with the screen read from a fake `luna-send`, ambient light on, and the link saved.
struct Setup {
    rig: Rig,
    luna: FakeLuna,
}

fn setup(fake: &Fake, mode: &str, area: Option<&str>) -> Setup {
    let mut rig = Rig::new();
    receiver_off(&rig.paths);
    seed_settings(
        &rig.paths,
        "base.ambient",
        json!({
            "enabled": {"type": "bool", "value": true},
            "mode": {"type": "choice", "value": mode},
            "brightness": {"type": "integer", "value": 100},
            "delay_ms": {"type": "integer", "value": 0},
        }),
    );
    link(&rig.paths.data_dir, fake, area);
    let luna = FakeLuna::new(rig.dir.path(), LunaMode::PollOnly, "Active");
    rig.start(
        test_config(json!({
            "screen": {"source": "luna", "pollMs": 40},
            "luna": {"send": luna.program},
            // The TLS child runs as the helper does: the pinned certificate sits in a private temporary directory.
            "fetch": {"user": null},
        })),
        Script::new(),
    );
    Setup { rig, luna }
}

fn hue_state(rig: &Rig, state: &str) -> bool {
    rig.wait_topic("ambient", WAIT, |v| v["hueState"] == state)
        .is_some()
}

fn the_stream_is_idle(rig: &Rig) -> bool {
    hue_state(rig, "idle")
}

#[test]
fn the_still_colour_streams_to_the_started_area_and_the_screen_going_off_ends_it() {
    if !have_tools() {
        return;
    }
    let fake = Fake::start(|pki| pki.good(), false, false);
    let s = setup(&fake, "colour", Some(AREA));
    let rig = &s.rig;

    assert!(hue_state(rig, "streaming"), "{:?}", rig.topic("ambient"));
    let status = rig.topic("ambient").unwrap();
    assert_eq!(
        (status["hue"].clone(), status["available"].clone()),
        (json!(true), json!(true))
    );
    assert!(wait_until(WAIT, || fake.packets().len() >= 3));

    // The stream is the still colour, at full brightness, on the area's three channels, for the area's id.
    let still = Still::Warm.rgb().to_u16();
    let packets = fake.packets();
    for p in &packets {
        assert_eq!(p.config_id, AREA);
        assert_eq!(
            p.channels,
            vec![(0, still), (1, still), (2, still)],
            "{p:?}"
        );
    }
    let seqs: Vec<u8> = packets.iter().map(|p| p.seq).collect();
    assert!(
        seqs.windows(2).all(|w| w[1] == w[0].wrapping_add(1)),
        "{seqs:?}"
    );

    // The order on the wire: the key proves itself on every request, the area is read, then started, and only then streamed to.
    let requests = fake.requests();
    assert!(requests.iter().all(|r| r["key_ok"] == true), "{requests:?}");
    assert_eq!(requests[0]["path"], "/auth/v1");
    let first_put = requests.iter().position(|r| r["method"] == "PUT").unwrap();
    assert!(
        requests[..first_put]
            .iter()
            .any(|r| r["method"] == "GET" && r["path"].as_str().unwrap().ends_with(AREA)),
        "the area was not read before it was started: {requests:?}"
    );
    assert_eq!((fake.starts(), fake.stops()), (1, 0));

    // Screen off: black twice, the session closed, the area stopped. The helper's own status goes quiet.
    s.luna.set_state("Active Standby");
    assert!(
        wait_until(WAIT, || fake.stops() == 1),
        "{:?}",
        fake.requests()
    );
    assert!(the_stream_is_idle(rig), "{:?}", rig.topic("ambient"));
    let after = fake.packets();
    let last: Vec<_> = after.iter().rev().take(2).collect();
    assert!(
        last.iter()
            .all(|p| p.channels.iter().all(|(_, rgb)| *rgb == [0, 0, 0])),
        "the lights were not sent black before the stream ended: {last:?}"
    );

    // Screen on again: the stream is started afresh.
    s.luna.set_state("Active");
    assert!(
        wait_until(WAIT, || fake.starts() == 2),
        "{:?}",
        fake.requests()
    );
    assert!(hue_state(rig, "streaming"));

    // The helper ending ends the stream too: black, closed, stopped.
    let mut s = s;
    s.rig.stop();
    assert_eq!(fake.stops(), 2, "{:?}", fake.requests());
}

/// The capture helper's side of the socket: frames of one colour on the left of the picture and another on the right.
fn left_red_right_blue() -> ZoneFrame {
    let mut f = ZoneFrame::blank(Preset::Plain);
    for (i, rect) in Preset::Plain.rects().iter().enumerate().take(MAX_ZONES) {
        f.color[i] = if (rect.x0 + rect.x1) / 2.0 < 0.5 {
            Rgb::new(0.9, 0.05, 0.05)
        } else {
            Rgb::new(0.05, 0.05, 0.9)
        };
        f.count[i] = 1000;
    }
    f
}

#[test]
fn the_pictures_colours_reach_each_channel_by_where_it_sits_and_the_stream_ends_when_the_picture_goes()
 {
    if !have_tools() {
        return;
    }
    let fake = Fake::start(|pki| pki.good(), false, false);
    let s = setup(&fake, "picture", Some(AREA));
    let socket = s.rig.paths.data_dir.join("ambient.sock");
    assert!(wait_until(WAIT, || socket.exists()), "no capture socket");
    let mut capture = UnixStream::connect(&socket).unwrap();
    let bytes = wire::encode(&left_red_right_blue(), 1000);

    // Frames at the capture rate until a packet shows the picture on the channels.
    let deadline = Instant::now() + WAIT;
    let mut seen = None;
    while Instant::now() < deadline && seen.is_none() {
        capture.write_all(&bytes).unwrap();
        std::thread::sleep(Duration::from_millis(30));
        seen = fake.packets().into_iter().rev().find(|p| {
            p.channels.len() == CHANNELS
                && p.channels.iter().any(|(_, c)| c[0] > 20000 || c[2] > 20000)
        });
    }
    let p =
        seen.unwrap_or_else(|| panic!("no picture on the channels: {:?}", fake.packets().last()));
    let by_id = |id: u8| p.channels.iter().find(|(c, _)| *c == id).unwrap().1;
    let (left, right) = (by_id(0), by_id(2));
    assert!(
        left[0] > left[2],
        "the left channel is not red: {left:?} {right:?}"
    );
    assert!(
        right[2] > right[0],
        "the right channel is not blue: {left:?} {right:?}"
    );
    assert!(hue_state(&s.rig, "streaming"));

    // The picture goes (no more frames): the lights fade, the stream is sent black, closed and the area stopped.
    assert!(
        wait_until(Duration::from_secs(30), || fake.stops() == 1),
        "{:?}",
        fake.requests()
    );
    assert!(the_stream_is_idle(&s.rig));
    let packets = fake.packets();
    let last = packets.last().unwrap();
    assert!(
        last.channels.iter().all(|(_, rgb)| *rgb == [0, 0, 0]),
        "{last:?}"
    );
}

#[test]
fn an_area_that_is_streaming_for_another_application_is_left_alone_until_it_is_free() {
    if !have_tools() {
        return;
    }
    let mut fake = Fake::start(|pki| pki.good(), false, false);
    fake.say(&format!("takeover {OTHER_APP}"));
    std::thread::sleep(Duration::from_millis(200));
    let s = setup(&fake, "colour", Some(AREA));
    let status = s
        .rig
        .wait_topic("ambient", WAIT, |v| v["hueState"] == "error")
        .expect("no complaint");
    assert!(
        status["problem"]
            .as_str()
            .unwrap()
            .contains("another application"),
        "{status}"
    );
    // Nothing was started and nothing was streamed.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        (fake.starts(), fake.stops()),
        (0, 0),
        "{:?}",
        fake.requests()
    );
    assert!(fake.packets().is_empty());

    // The other application lets go: the next look (the pause is 5 s) starts the stream.
    fake.say("release");
    assert!(
        hue_state(&s.rig, "streaming"),
        "{:?}",
        s.rig.topic("ambient")
    );
    assert_eq!(fake.starts(), 1);
}

#[test]
fn a_bridge_that_shows_another_certificate_than_the_linked_one_is_sent_nothing() {
    if !have_tools() {
        return;
    }
    // Linked to one bridge, then the address is answered by another (a different certificate, from another authority).
    let first = Fake::start(|pki| pki.good(), false, false);
    let mut rig = Rig::new();
    receiver_off(&rig.paths);
    seed_settings(
        &rig.paths,
        "base.ambient",
        json!({"enabled": {"type": "bool", "value": true}, "mode": {"type": "choice", "value": "colour"}}),
    );
    link(&rig.paths.data_dir, &first, Some(AREA));
    drop(first);
    let impostor = Fake::start(|pki| pki.self_signed(), false, false);
    // The saved link names the first bridge's port; the impostor is on its own, so the link is rewritten to point at it with the
    // first bridge's certificate still pinned (what an address that changed hands looks like).
    let stored = hue_store::load(&rig.paths.data_dir).unwrap().unwrap();
    let moved = HueLink::new(
        "127.0.0.1",
        impostor.port,
        impostor.udp,
        stored.area.as_deref(),
        APP_KEY,
        CLIENT_KEY,
    )
    .unwrap();
    hue_store::save(&rig.paths.data_dir, &moved).unwrap();
    let luna = FakeLuna::new(rig.dir.path(), LunaMode::PollOnly, "Active");
    rig.start(
        test_config(json!({
            "screen": {"source": "luna", "pollMs": 40},
            "luna": {"send": luna.program},
            "fetch": {"user": null},
        })),
        Script::new(),
    );
    let status = rig
        .wait_topic("ambient", WAIT, |v| v["hueState"] == "error")
        .expect("no complaint");
    assert!(
        status["problem"].as_str().unwrap().contains("certificate"),
        "{status}"
    );
    // The handshake failed before a request existed: not the key, not a request at all.
    assert!(impostor.requests().is_empty(), "{:?}", impostor.requests());
    assert!(impostor.packets().is_empty());
}

#[test]
fn a_stream_the_bridge_ends_is_noticed_and_started_again() {
    if !have_tools() {
        return;
    }
    let mut fake = Fake::start(|pki| pki.good(), false, false);
    let s = setup(&fake, "colour", Some(AREA));
    assert!(hue_state(&s.rig, "streaming"));
    assert!(wait_until(WAIT, || !fake.packets().is_empty()));
    fake.say("close_stream");
    // Noticed within a moment (the status says the stream ended), the area is stopped to put it right, and after the pause it is
    // started again and streams.
    assert!(
        s.rig
            .wait_topic("ambient", WAIT, |v| v["hueState"] == "error"
                && v["problem"]
                    .as_str()
                    .is_some_and(|p| p.contains("stream ended")))
            .is_some(),
        "{:?}",
        s.rig.topic("ambient")
    );
    assert!(
        wait_until(WAIT, || fake.starts() == 2),
        "{:?}",
        fake.requests()
    );
    assert!(hue_state(&s.rig, "streaming"));
    assert!(fake.stops() >= 1);
    let before = fake.packets().len();
    assert!(wait_until(WAIT, || fake.packets().len() > before));
}

// ------------------------------------------------------------------------------------------------- the command line

struct Cli {
    data_dir: PathBuf,
}

impl Cli {
    fn new(dir: &Path) -> Cli {
        // The pinned certificate is in a private temporary directory: the TLS child runs as the command does.
        std::fs::write(dir.join("helper.json"), r#"{"fetch":{"user":null}}"#).unwrap();
        Cli {
            data_dir: dir.to_owned(),
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut c = Command::new(BIN);
        c.args(args).arg("--data-dir").arg(&self.data_dir);
        c
    }

    /// Runs a command, giving it `input` on standard input; returns the exit code, the one JSON object it printed, and everything
    /// it wrote on either stream.
    fn run(&self, args: &[&str], input: &str) -> (i32, Value, String) {
        let mut child = self
            .command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {text}"));
        (out.status.code().unwrap(), value, text)
    }
}

fn nothing_secret(text: &str) {
    for secret in [APP_KEY, CLIENT_KEY, &CLIENT_KEY.to_uppercase()] {
        assert!(!text.contains(secret), "a key was printed: {text}");
    }
}

#[test]
fn hue_link_pairs_after_the_link_button_pins_the_certificate_and_saves_the_keys_privately() {
    if !have_tools() {
        return;
    }
    let mut fake = Fake::start(|pki| pki.good(), false, false);
    let dir = tempfile::tempdir().unwrap();
    let cli = Cli::new(dir.path());
    let port = fake.port.to_string();
    let udp = fake.udp.to_string();
    let args = [
        "hue-link",
        "--host",
        "127.0.0.1",
        "--port",
        &port,
        "--stream-port",
        &udp,
        "--pair",
    ];
    // The button is pressed a moment after the command starts waiting for it.
    let child = cli
        .command(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        fake.requests().iter().any(|r| r["path"] == "/api"),
        "the command never asked for keys"
    );
    fake.say("press");
    let out = child.wait_with_output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(0), "{text}");
    nothing_secret(&text);
    let said: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(said["linked"], true);
    // The only area there is was chosen.
    assert_eq!(said["area"]["id"], AREA);
    assert_eq!(said["area"]["channels"], 3);

    // What was saved: private keys, a world-readable certificate, and a link that loads.
    let mode = |p: &Path| {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    };
    assert_eq!(mode(&hue_store::file(dir.path())), 0o600);
    assert!(hue_store::certificate_file(dir.path()).is_file());
    let saved = hue_store::load(dir.path()).unwrap().unwrap();
    assert_eq!(saved.area.as_deref(), Some(AREA));
    assert_eq!(saved.app_key.expose(), APP_KEY);
    assert_eq!(saved.client_key.expose(), CLIENT_KEY);
    // Every request after the registration carried the key (and the registration itself did not need one).
    let requests = fake.requests();
    // The first try, before the button, was refused; every request after the one that got the keys carried them.
    let api = requests.iter().rposition(|r| r["path"] == "/api").unwrap();
    assert!(
        requests[api + 1..].iter().all(|r| r["key_ok"] == true),
        "{requests:?}"
    );
    assert!(requests[api + 1..].len() >= 2, "{requests:?}");

    // `hue-area` lists, and unlink takes it all away.
    let (code, listed, text) = cli.run(&["hue-area"], "");
    assert_eq!(code, 0, "{text}");
    nothing_secret(&text);
    assert_eq!(listed["chosen"], AREA);
    assert_eq!(listed["areas"][0]["name"], "TV");
    let (code, gone, _) = cli.run(&["hue-unlink"], "");
    assert_eq!((code, gone["removed"].clone()), (0, json!(true)));
    assert_eq!(hue_store::load(dir.path()), Ok(None));
    assert!(!hue_store::certificate_file(dir.path()).exists());
}

#[test]
fn hue_link_takes_the_bridges_answer_on_standard_input_and_leaves_the_area_to_hue_area_when_there_are_two()
 {
    if !have_tools() {
        return;
    }
    let fake = Fake::start(|pki| pki.good(), true, true);
    let dir = tempfile::tempdir().unwrap();
    let cli = Cli::new(dir.path());
    let port = fake.port.to_string();
    let udp = fake.udp.to_string();
    // What `curl` printed after the button: the registration's answer, pasted as it is.
    let answer = json!([{"success": {"username": APP_KEY, "clientkey": CLIENT_KEY}}]).to_string();
    let args = [
        "hue-link",
        "--host",
        "127.0.0.1",
        "--port",
        &port,
        "--stream-port",
        &udp,
    ];
    let (code, said, text) = cli.run(&args, &answer);
    assert_eq!(code, 0, "{text}");
    nothing_secret(&text);
    assert_eq!(said["linked"], true);
    assert_eq!(
        said["area"],
        Value::Null,
        "two areas: none is chosen for the person"
    );
    assert_eq!(said["areas"].as_array().unwrap().len(), 2);
    assert!(said["next"].as_str().unwrap().contains("hue-area"));
    assert_eq!(hue_store::load(dir.path()).unwrap().unwrap().area, None);

    // A wrong area is refused with the list; the right one is saved.
    let (code, refused, _) = cli.run(
        &["hue-area", "--area", "11111111-2222-4333-8444-555555555555"],
        "",
    );
    assert_eq!(code, 2);
    assert!(
        refused["error"].as_str().unwrap().contains(OTHER_AREA),
        "{refused}"
    );
    let (code, chosen, text) = cli.run(&["hue-area", "--area", OTHER_AREA], "");
    assert_eq!(code, 0, "{text}");
    assert_eq!(chosen["area"]["name"], "Desk");
    assert_eq!(
        hue_store::load(dir.path())
            .unwrap()
            .unwrap()
            .area
            .as_deref(),
        Some(OTHER_AREA)
    );

    // Rubbish on standard input is refused without being echoed, and a refused link leaves the saved one alone.
    let (code, bad, text) = cli.run(&args, "not json, my key is hunter2hunter2");
    assert_eq!(code, 2);
    assert!(!text.contains("hunter2"), "{bad}");
    assert!(hue_store::load(dir.path()).unwrap().is_some());
}

#[test]
fn a_link_without_an_area_is_reported_by_ambient_light_with_the_command_that_fixes_it() {
    if !have_tools() {
        return;
    }
    let fake = Fake::start(|pki| pki.good(), false, false);
    let s = setup(&fake, "colour", None);
    let status = s.rig.topic("ambient").unwrap();
    assert_eq!(status["hue"], false);
    assert!(
        status["problem"].as_str().unwrap().contains("hue-area"),
        "{status}"
    );
    // Nothing was said to the bridge.
    assert!(fake.requests().is_empty());
}
