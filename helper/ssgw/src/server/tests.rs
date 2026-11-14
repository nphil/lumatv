//! The gateway end to end in plain mode: connections handed over the way `ssgw-accept` hands them, the bus played by a
//! script, LG's gateway by a small WebSocket server.

use super::*;
use crate::luna::tests::{answer, fake_bus};
use crate::store::{Client, fingerprint};
use crate::ws::{self, Parsed};
use std::net::{TcpListener, TcpStream};
use std::os::fd::FromRawFd;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::JoinHandle;

const WAIT: Duration = Duration::from_secs(5);
const NO_LG: &str = "127.0.0.1:9";
/// A client the store knows, granted `READ_AUDIO`.
const KEY: &str = "stored-key";

fn interfaces() -> Vec<(&'static str, Value)> {
    vec![
        (
            "api.interface",
            json!({"virtualService": "api", "methods": [
                {"path": "/getServiceList", "provider": "com.webos.service.secondscreen.gateway/getServiceList"}]}),
        ),
        (
            "audio.interface",
            json!({"virtualService": "audio", "version": 1, "methods": [
                {"path": "/getVolume", "provider": "com.webos.service.apiadapter/audio/getStatus"},
                {"path": "/other", "provider": "com.webos.service.apiadapter/audio/other"}]}),
        ),
        (
            "gateway.interface",
            json!({"service": "com.webos.service.secondscreen.gateway", "methods": [
                {"path": "/getServiceList"},
                {"path": "/pairing/setPin", "sendClientInfo": true}]}),
        ),
        (
            "apiadapter.interface",
            json!({"service": "com.webos.service.apiadapter", "methods": [
                {"path": "/audio/getStatus", "requiredPermissions": ["CONTROL_AUDIO", "READ_AUDIO"]},
                {"path": "/audio/other", "requiredPermissions": "READ_AUDIO"}]}),
        ),
        (
            "test.interface",
            json!({"service": "com.example.test", "methods": [
                {"path": "/open"},
                {"path": "/read", "requiredPermissions": "READ_AUDIO"},
                {"path": "/fails", "requiredPermissions": "READ_AUDIO"},
                {"path": "/secret", "requiredPermissions": "SECRET"},
                {"path": "/files", "requiredPermissions": "READ_AUDIO",
                 "resources": {"media": {"responseProperties": ["icon"]}, "socket": {"responseProperties": ["socketPath"]}}}]}),
        ),
    ]
}

struct Gw {
    dir: tempfile::TempDir,
    stop: Option<OwnedFd>,
    thread: Option<JoinHandle<i32>>,
}

impl Gw {
    /// LG's files, a bus and a client store holding [`KEY`].
    fn prepare() -> Gw {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let gw = root.join("lg").join(GATEWAY_DIR);
        for sub in ["interfaces", "permission-types"] {
            std::fs::create_dir_all(gw.join(sub)).unwrap();
        }
        for (name, doc) in interfaces() {
            std::fs::write(gw.join("interfaces").join(name), doc.to_string()).unwrap();
        }
        let types = json!({"permissionTypes": [
            {"id": "READ_AUDIO", "accessType": "PROTECTED"},
            {"id": "CONTROL_AUDIO", "accessType": "PROTECTED"},
            {"id": "SECRET", "accessType": "SECURE"}]});
        std::fs::write(gw.join("permission-types/test.permissions"), types.to_string()).unwrap();
        for d in ["lg/var/luna/preferences", "lg/etc", "bus", "state"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("lg/var/luna/preferences/ran-firstuse"), "").unwrap();
        std::fs::write(root.join("lg/etc/starfish-release"), "webOS TV Rockhopper release 10.2.1-5202 (ombre-x)\n").unwrap();
        fake_bus(&root.join("bus"));
        let g = Gw { dir, stop: None, thread: None };
        g.store(KEY, &["READ_AUDIO"]);
        g
    }

    fn store(&self, key: &str, grant: &[&str]) {
        let mut store = Store::load(self.dir.path().join("state/clients.json"));
        let grant = grant.iter().map(|g| g.to_string()).collect();
        store.put(Client { fp: fingerprint(key), grant, app_id: Some("com.example.app".into()), checked: crate::util::now_ms() / 1000 });
    }

    fn stored(&self, key: &str) -> Option<Client> {
        Store::load(self.dir.path().join("state/clients.json")).get(&fingerprint(key)).cloned()
    }

    fn answer(&self, uri: &str, lines: &[Value]) {
        answer(&self.dir.path().join("bus"), uri, lines);
    }

    fn calls(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("bus/calls.log")).unwrap_or_default()
    }

    fn run_dir(&self) -> PathBuf {
        self.dir.path().join("run")
    }

    fn config(&self, lg_http: &str, broker: Option<OwnedFd>) -> Config {
        let root = self.dir.path();
        Config {
            state_dir: root.join("state"),
            run_dir: self.run_dir(),
            lg_root: root.join("lg"),
            lg_http: lg_http.into(),
            luna_send: root.join("bus/luna-send"),
            tls: false,
            check_pins: false,
            broker,
            version: "test".into(),
        }
    }

    fn run(mut self, lg_http: &str, broker: Option<OwnedFd>) -> Gw {
        let cfg = self.config(lg_http, broker);
        let (r, w) = pipe();
        let (tx, rx) = std::sync::mpsc::channel();
        self.thread = Some(std::thread::spawn(move || run_notify(cfg, r, move || tx.send(()).unwrap())));
        rx.recv_timeout(WAIT).expect("the gateway starts");
        self.stop = Some(w);
        self
    }

    fn stop(&mut self) -> i32 {
        let w = self.stop.take().expect("running");
        // SAFETY: writes one byte from a static buffer to the stop pipe.
        unsafe { libc::write(w.as_raw_fd(), b"s".as_ptr().cast(), 1) };
        self.thread.take().expect("running").join().expect("the gateway thread ends")
    }
}

impl Drop for Gw {
    fn drop(&mut self) {
        if let Some(w) = self.stop.take() {
            // SAFETY: as in `stop`.
            unsafe { libc::write(w.as_raw_fd(), b"s".as_ptr().cast(), 1) };
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }
}

fn pipe() -> (OwnedFd, OwnedFd) {
    let mut fds = [0i32; 2];
    // SAFETY: pipe2 fills the array; each end is owned once.
    unsafe {
        assert_eq!(libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC), 0);
        (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1]))
    }
}

fn read_more(s: &mut impl Read, buf: &mut Vec<u8>) -> bool {
    let mut chunk = [0u8; 4096];
    match s.read(&mut chunk) {
        Ok(n) if n > 0 => {
            buf.extend_from_slice(&chunk[..n]);
            true
        }
        _ => false,
    }
}

/// The client end of a connection handed to the gateway as `ssgw-accept` hands it.
fn hand_over(gw: &Gw) -> UnixStream {
    let (ours, theirs) = UnixStream::pair().unwrap();
    let acc = UnixStream::connect(gw.run_dir().join("accept.sock")).unwrap();
    acc.set_read_timeout(Some(WAIT)).unwrap();
    let mut b = [0u8];
    (&acc).read_exact(&mut b).unwrap();
    assert_eq!(&b, b"R");
    net::send_fd(acc.as_raw_fd(), b'P', theirs.as_raw_fd()).unwrap();
    (&acc).read_exact(&mut b).unwrap();
    assert_eq!(&b, b"T");
    ours.set_read_timeout(Some(WAIT)).unwrap();
    ours
}

/// A plain request; the status, then the body.
fn http_get(gw: &Gw, path: &str, headers: &str) -> (u16, Vec<u8>) {
    let mut s = hand_over(gw);
    write!(s, "GET {path} HTTP/1.1\r\nHost: tv:3001\r\n{headers}\r\n").unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).unwrap();
    let end = out.windows(4).position(|w| w == b"\r\n\r\n").expect("a complete head") + 4;
    let status = std::str::from_utf8(&out[9..12]).unwrap().parse().unwrap();
    (status, out[end..].to_vec())
}

struct Ws {
    s: UnixStream,
    buf: Vec<u8>,
}

impl Ws {
    fn open(gw: &Gw, path: &str) -> Ws {
        Ws::open_with(gw, path, "")
    }

    fn open_with(gw: &Gw, path: &str, headers: &str) -> Ws {
        let mut s = hand_over(gw);
        write!(
            s,
            "GET {path} HTTP/1.1\r\nHost: tv:3001\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n{headers}\r\n"
        )
        .unwrap();
        let mut buf = Vec::new();
        let head = loop {
            if let Ok(Some(h)) = ws::parse_head(&buf) {
                break h;
            }
            assert!(read_more(&mut s, &mut buf), "no answer to the upgrade");
        };
        assert_eq!(head.status, 101);
        buf.drain(..head.len);
        Ws { s, buf }
    }

    fn send(&mut self, v: Value) {
        self.send_frame(ws::OP_TEXT, v.to_string().as_bytes());
    }

    fn send_frame(&mut self, op: u8, data: &[u8]) {
        let mut f = Vec::new();
        ws::encode(&mut f, op, data, true);
        self.s.write_all(&f).unwrap();
    }

    fn frame(&mut self) -> (u8, Vec<u8>) {
        loop {
            match ws::parse_frame(&self.buf) {
                Parsed::Frame(f, used) => {
                    self.buf.drain(..used);
                    return (f.op, f.payload);
                }
                Parsed::Incomplete => assert!(read_more(&mut self.s, &mut self.buf), "the gateway sent nothing more"),
                Parsed::Fail(code) => panic!("a bad frame from the gateway ({code})"),
            }
        }
    }

    fn recv(&mut self) -> Value {
        let (op, p) = self.frame();
        assert_eq!(op, ws::OP_TEXT, "{}", String::from_utf8_lossy(&p));
        serde_json::from_slice(&p).unwrap()
    }

    fn ask(&mut self, v: Value) -> Value {
        self.send(v);
        self.recv()
    }

    fn request(&mut self, uri: &str) -> Value {
        self.ask(json!({"type": "request", "id": 1, "uri": uri}))
    }

    fn register(&mut self, key: &str) -> Value {
        self.ask(json!({"type": "register", "id": "reg", "payload": {"client-key": key}}))
    }

    fn closed(&mut self) -> (Option<u16>, String) {
        let (op, p) = self.frame();
        assert_eq!(op, ws::OP_CLOSE, "{}", String::from_utf8_lossy(&p));
        let code = (p.len() >= 2).then(|| u16::from_be_bytes([p[0], p[1]]));
        (code, String::from_utf8_lossy(p.get(2..).unwrap_or_default()).into_owned())
    }
}

fn control(gw: &Gw, cmd: &str) -> Value {
    let mut s = UnixStream::connect(gw.run_dir().join("control.sock")).unwrap();
    s.set_read_timeout(Some(WAIT)).unwrap();
    s.write_all(format!("{cmd}\n").as_bytes()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    serde_json::from_str(&out).unwrap()
}

fn wait_for(gw: &Gw, field: &str, want: bool) {
    let until = Instant::now() + WAIT;
    while control(gw, "ping")[field] != want {
        assert!(Instant::now() < until, "{field} never became {want}");
    }
}

/// LG's gateway on its plain port: registration hands out `lg-key` (or keeps the key it is shown, unless that is
/// `revoked`); every request is answered with its URI and a resource URL of LG's; plain GETs get `LG <path>`.
struct Lg {
    addr: String,
    seen: Arc<Mutex<Vec<Value>>>,
    connections: Arc<AtomicUsize>,
}

impl Lg {
    fn start() -> Lg {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(AtomicUsize::new(0));
        let (s, c) = (seen.clone(), connections.clone());
        std::thread::spawn(move || {
            for stream in l.incoming().flatten() {
                c.fetch_add(1, Ordering::SeqCst);
                let seen = s.clone();
                std::thread::spawn(move || lg_serve(stream, &seen));
            }
        });
        Lg { addr, seen, connections }
    }

    fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

fn lg_serve(mut s: TcpStream, seen: &Mutex<Vec<Value>>) {
    s.set_read_timeout(Some(WAIT)).unwrap();
    let mut buf = Vec::new();
    let head = loop {
        if let Ok(Some(h)) = ws::parse_head(&buf) {
            break h;
        }
        if !read_more(&mut s, &mut buf) {
            return;
        }
    };
    buf.drain(..head.len);
    if !head.is_upgrade() {
        let body = format!("LG {}", head.target);
        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        return;
    }
    s.write_all(&ws::server_handshake(head.header("sec-websocket-key").unwrap())).unwrap();
    loop {
        match ws::parse_frame(&buf) {
            Parsed::Frame(f, used) => {
                buf.drain(..used);
                let mut out = Vec::new();
                if f.op == ws::OP_CLOSE {
                    ws::encode(&mut out, ws::OP_CLOSE, &f.payload, false);
                    let _ = s.write_all(&out);
                    return;
                }
                let Ok(msg) = serde_json::from_slice::<Value>(&f.payload) else { continue };
                seen.lock().unwrap().push(msg.clone());
                for reply in lg_answers(&msg) {
                    ws::encode(&mut out, ws::OP_TEXT, reply.to_string().as_bytes(), false);
                }
                if s.write_all(&out).is_err() {
                    return;
                }
            }
            Parsed::Incomplete => {
                if !read_more(&mut s, &mut buf) {
                    return;
                }
            }
            Parsed::Fail(_) => return,
        }
    }
}

fn lg_answers(msg: &Value) -> Vec<Value> {
    let id = msg["id"].clone();
    match msg["type"].as_str() {
        Some("register") => match msg["payload"]["client-key"].as_str() {
            Some("revoked") => vec![json!({"type": "error", "id": id, "error": "401 key not recognised", "payload": {}})],
            key => vec![json!({"type": "registered", "id": id, "payload": {"client-key": key.unwrap_or("lg-key")}})],
        },
        Some("request" | "subscribe") => {
            vec![json!({"type": "response", "id": id, "payload": {"returnValue": true, "uri": msg["uri"], "icon": "http://tv:3001/resources/lg/i.png"}})]
        }
        _ => Vec::new(),
    }
}

#[test]
fn hello_and_malformed_messages_get_lgs_answers() {
    let gw = Gw::prepare().run(NO_LG, None);
    let mut c = Ws::open(&gw, "/");
    let hello = c.ask(json!({"type": "hello", "id": "h", "payload": {"sdkVersion": "1"}}));
    let payload = json!({"protocolVersion": 1, "deviceType": "tv", "deviceOS": "webOS", "deviceOSVersion": "4.1.0",
        "deviceOSReleaseVersion": "10.2.1", "pairingTypes": ["PIN", "PROMPT", "COMBINED", "LGSWITCH-PIN"]});
    assert_eq!(hello, json!({"type": "hello", "payload": payload}));
    c.send_frame(ws::OP_TEXT, b"null");
    c.send_frame(ws::OP_BINARY, b"x");
    assert_eq!(c.recv(), json!({"type": "error", "error": "400 binary messages not supported", "payload": {}}), "null is not answered");
    assert_eq!(c.ask(json!({"type": "nonsense", "id": 4})), json!({"type": "error", "id": 4, "error": "400 unknown message type", "payload": {}}));
    assert_eq!(c.request("ssap://nowhere/at/all"), json!({"type": "error", "id": 1, "error": "404 no such service or method", "payload": {}}));
    c.send_frame(ws::OP_TEXT, b"{not json");
    assert_eq!(c.closed(), (Some(1008), "invalid message".into()));

    let mut web = Ws::open_with(&gw, "/", "Origin: http://evil.example\r\n");
    assert_eq!(web.closed(), (Some(1008), "invalid origin".into()));
}

#[test]
fn requests_are_checked_and_answered_like_lg() {
    let gw = Gw::prepare();
    gw.answer("luna://com.example.test/open", &[json!({"returnValue": true, "open": 1})]);
    gw.answer("luna://com.example.test/read", &[json!({"returnValue": false, "errorCode": 7, "errorText": "busy"})]);
    gw.answer("luna://com.example.test/fails", &[json!({"returnValue": false, "reason": "x"})]);
    gw.answer(
        "luna://com.webos.service.audio/master/getVolume",
        &[json!({"returnValue": true, "volumeStatus": {"muteStatus": false, "volume": 9}})],
    );
    let gw = gw.run(NO_LG, None);
    let mut c = Ws::open(&gw, "/");
    let open = c.ask(json!({"type": "request", "id": 1, "uri": "ssap://com.example.test/open", "payload": {"_remoteClientId": "x", "a": 1}}));
    assert_eq!(open, json!({"type": "response", "id": 1, "payload": {"returnValue": true, "open": 1}}), "open methods need no registration");
    assert_eq!(c.request("ssap://com.example.test/read")["error"], "401 insufficient permissions (not registered)");
    let services = c.request("ssap://api/getServiceList");
    assert_eq!(services["payload"], json!({"services": [{"name": "api", "version": 1}, {"name": "audio", "version": 1}], "returnValue": true}));
    assert_eq!(c.request("ssap://com.webos.service.secondscreen.gateway/pairing/setPin")["error"], "403 Access denied");

    assert_eq!(c.register(KEY), json!({"type": "registered", "id": "reg", "payload": {"client-key": KEY}}));
    assert_eq!(c.request("ssap://com.example.test/open")["error"], "401 insufficient permissions", "registered clients need a permission");
    assert_eq!(c.request("ssap://com.example.test/secret")["error"], "401 insufficient permissions");
    assert_eq!(c.request("ssap://com.example.test/read"), json!({"type": "error", "id": 1, "error": "7 busy", "payload": {}}));
    let fails = c.request("ssap://com.example.test/fails");
    assert_eq!((&fails["error"], &fails["payload"]), (&json!("500 Application error"), &json!({"returnValue": false, "reason": "x"})));
    assert_eq!(c.request("ssap://audio/getVolume")["payload"]["volume"], 9, "the adapter's methods run here");
    c.send(json!({"type": "subscribe", "id": "s", "uri": "ssap://com.example.test/read", "payload": {"_remoteClientName": "x"}}));
    assert_eq!(c.recv()["id"], "s");

    let calls = gw.calls();
    assert!(calls.contains("luna://com.example.test/open {\"a\":1}\n"), "{calls}");
    assert!(calls.contains("luna://com.example.test/read {\"subscribe\":true}\n"), "{calls}");
}

#[test]
fn unknown_clients_register_through_lg_and_are_remembered() {
    let lg = Lg::start();
    let gw = Gw::prepare().run(&lg.addr, None);
    let mut c = Ws::open(&gw, "/");
    let manifest = json!({"manifestVersion": 1, "appId": "com.new.app", "permissions": ["READ_AUDIO", "SECRET", "UNKNOWN"]});
    let reg = c.ask(json!({"type": "register", "id": "r1", "payload": {"manifest": manifest}}));
    assert_eq!(reg, json!({"type": "registered", "id": "r1", "payload": {"client-key": "lg-key"}}));
    assert_eq!(c.request("ssap://com.example.test/open")["payload"]["uri"], "ssap://com.example.test/open", "LG serves the rest");
    let stored = gw.stored("lg-key").expect("LG's key is remembered");
    assert_eq!((stored.grant, stored.app_id.as_deref()), (vec!["READ_AUDIO".to_owned()], Some("com.new.app")));

    let mut d = Ws::open(&gw, "/");
    assert_eq!(d.register("lg-key")["type"], "registered");
    assert_eq!(d.request("ssap://com.example.test/secret")["error"], "401 insufficient permissions", "served here, by the grant");
    assert_eq!(lg.connections(), 1, "a known client registers without LG");

    let mut e = Ws::open(&gw, "/");
    let forced = e.ask(json!({"type": "register", "id": "r2", "payload": {"client-key": KEY, "forcePairing": true}}));
    assert_eq!(forced["payload"]["client-key"], KEY);
    assert_eq!(lg.connections(), 2, "forced pairing is LG's");
}

#[test]
fn methods_left_to_lg_go_over_the_sessions_own_link() {
    let lg = Lg::start();
    let gw = Gw::prepare();
    gw.store("revoked", &["READ_AUDIO"]);
    let gw = gw.run(&lg.addr, None);
    let mut c = Ws::open(&gw, "/");
    c.register(KEY);
    let other = c.ask(json!({"type": "request", "id": "o", "uri": "ssap://audio/other"}));
    let lg_reply = json!({"returnValue": true, "uri": "ssap://audio/other", "icon": "http://tv:3001/resources/lg/i.png"});
    assert_eq!(other, json!({"type": "response", "id": "o", "payload": lg_reply}));
    let seen: Vec<(Value, Value)> = lg.seen.lock().unwrap().iter().map(|m| (m["type"].clone(), m["id"].clone())).collect();
    assert_eq!(seen, [(json!("register"), json!("ssgw-reg")), (json!("request"), json!("ssgw-1"))], "the request waits for the registration");

    let mut r = Ws::open(&gw, "/");
    r.register("revoked");
    r.request("ssap://audio/other");
    assert!(gw.stored("revoked").is_none(), "a key LG refuses is forgotten");
    assert!(gw.stored(KEY).is_some());
}

#[test]
fn resources_are_served_under_the_urls_handed_out() {
    let lg = Lg::start();
    let gw = Gw::prepare();
    let icon = gw.dir.path().join("icon a.png");
    std::fs::write(&icon, b"PNG!").unwrap();
    gw.answer("luna://com.example.test/files", &[json!({"returnValue": true, "icon": icon})]);
    let gw = gw.run(&lg.addr, None);
    let mut c = Ws::open(&gw, "/");
    c.register(KEY);
    let reply = c.request("ssap://com.example.test/files");
    let url = reply["payload"]["icon"].as_str().unwrap();
    let path = url.strip_prefix("http://tv:3001").unwrap_or_else(|| panic!("{url}"));
    assert!(path.starts_with("/resources/") && path.ends_with("/icon%20a.png"), "{path}");

    assert_eq!(http_get(&gw, path, ""), (200, b"PNG!".to_vec()));
    assert_eq!(http_get(&gw, path, "If-Modified-Since: Fri, 01 Jan 2100 00:00:00 GMT\r\n"), (304, Vec::new()));
    assert_eq!(http_get(&gw, "/", ""), (200, b"Hello world\r\n".to_vec()));
    assert_eq!(http_get(&gw, "/resources/lg/i.png", ""), (200, b"LG /resources/lg/i.png".to_vec()), "LG's own resources are LG's");
    assert_eq!(c.request("ssap://com.example.test/files")["payload"]["icon"], url, "a file keeps its URL in a session");
}

#[test]
fn pointer_sockets_are_opened_by_the_broker_and_framed() {
    let target_dir = tempfile::tempdir().unwrap();
    let target = target_dir.path().join("input.sock");
    let input = std::os::unix::net::UnixListener::bind(&target).unwrap();
    let mut fds = [0i32; 2];
    // SAFETY: socketpair fills the array; each end is owned once.
    let (broker, theirs) = unsafe {
        assert_eq!(libc::socketpair(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0, fds.as_mut_ptr()), 0);
        (UnixStream::from(OwnedFd::from_raw_fd(fds[0])), OwnedFd::from_raw_fd(fds[1]))
    };
    broker.set_read_timeout(Some(WAIT)).unwrap();
    let gw = Gw::prepare();
    gw.answer("luna://com.example.test/files", &[json!({"returnValue": true, "socketPath": target})]);
    let gw = gw.run(NO_LG, Some(theirs));
    let mut c = Ws::open(&gw, "/");
    c.register(KEY);
    let url = c.request("ssap://com.example.test/files")["payload"]["socketPath"].as_str().unwrap().to_owned();
    let path = url.strip_prefix("ws://tv:3001").unwrap_or_else(|| panic!("{url}")).to_owned();

    let mut pointer = Ws::open(&gw, &path);
    let event = "type:move\ndx:1\ndy:é\n\n";
    pointer.send_frame(ws::OP_TEXT, event.as_bytes());
    let mut asked = [0u8; 256];
    let n = (&broker).read(&mut asked).unwrap();
    assert_eq!(&asked[..n], target.to_str().unwrap().as_bytes());
    let opened = UnixStream::connect(&target).unwrap();
    net::send_fd(broker.as_raw_fd(), b'K', opened.as_raw_fd()).unwrap();
    let (mut seen, _) = input.accept().unwrap();
    seen.set_read_timeout(Some(WAIT)).unwrap();
    let mut got = vec![0u8; 4 + event.len()];
    seen.read_exact(&mut got).unwrap();
    assert_eq!(got[..4], (event.len() as u32).to_be_bytes(), "the length counts bytes");
    assert_eq!(&got[4..], event.as_bytes());

    let mut again = hand_over(&gw);
    write!(again, "GET {path} HTTP/1.1\r\nHost: tv:3001\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: a2V5\r\n\r\n").unwrap();
    let mut out = Vec::new();
    again.read_to_end(&mut out).unwrap();
    assert!(out.is_empty(), "a socket URL is good for one connection; the next is LG's to answer");

    let url = c.request("ssap://com.example.test/files")["payload"]["socketPath"].as_str().unwrap().to_owned();
    let mut refused = Ws::open(&gw, url.strip_prefix("ws://tv:3001").unwrap());
    let n = (&broker).read(&mut asked).unwrap();
    assert!(n > 0);
    (&broker).write_all(b"E13").unwrap();
    assert_eq!(refused.closed(), (Some(1011), "error in target socket".into()));
}

#[test]
fn standby_turns_new_connections_away() {
    let gw = Gw::prepare();
    gw.answer(POWER, &[json!({"returnValue": true, "state": "Active Standby"})]);
    let gw = gw.run(NO_LG, None);
    wait_for(&gw, "standby", true);
    let mut c = Ws::open(&gw, "/");
    assert_eq!(c.closed(), (Some(1008), "Try Again Later (EWS)".into()));
}

#[test]
fn with_mobile_access_off_connections_are_taken_and_closed() {
    let gw = Gw::prepare();
    gw.answer(SETTINGS, &[json!({"returnValue": true, "settings": {"allowMobileDeviceAccess": false}})]);
    let gw = gw.run(NO_LG, None);
    wait_for(&gw, "access", false);
    let mut s = hand_over(&gw);
    assert_eq!(s.read(&mut [0u8; 16]).unwrap(), 0);
}

#[test]
fn the_control_socket_reports_and_forgets() {
    let lg = Lg::start();
    let gw = Gw::prepare().run(&lg.addr, None);
    let mut c = Ws::open(&gw, "/");
    c.register(KEY);
    c.request("ssap://com.example.test/open");
    let ping = control(&gw, "ping");
    assert_eq!(
        (&ping["ok"], &ping["version"], &ping["sessions"], &ping["standby"], &ping["access"]),
        (&json!(true), &json!("test"), &json!(1), &json!(false), &json!(true))
    );
    assert_eq!(ping["recent"], json!(["ssap://com.example.test/open"]));
    assert!(ping["uptimeS"].is_u64());
    assert_eq!(control(&gw, "forget"), json!({"ok": true, "forgotten": 1}));
    assert_eq!(control(&gw, "what"), json!({"ok": false, "error": "unknown command"}));
    Ws::open(&gw, "/").register(KEY);
    assert_eq!(lg.connections(), 1, "a forgotten client registers through LG");
}

#[test]
fn a_full_gateway_leaves_connections_to_lg_and_stopping_closes_sessions() {
    let mut gw = Gw::prepare().run(NO_LG, None);
    let mut open: Vec<Ws> = (0..MAX_SESSIONS).map(|_| Ws::open(&gw, "/")).collect();
    let acc = UnixStream::connect(gw.run_dir().join("accept.sock")).unwrap();
    acc.set_read_timeout(Some(WAIT)).unwrap();
    assert_eq!((&acc).read(&mut [0u8; 1]).unwrap(), 0, "no ready byte");
    assert_eq!(gw.stop(), 0);
    for c in &mut open {
        assert_eq!(c.closed(), (Some(1001), "server shutting down".into()));
    }
    assert!(!gw.run_dir().join("accept.sock").exists());
}

#[test]
fn a_second_gateway_and_an_unusable_environment_refuse_to_start() {
    let gw = Gw::prepare().run(NO_LG, None);
    assert_eq!(run(gw.config(NO_LG, None), pipe().0), 1, "one gateway per run directory");

    let other = Gw::prepare();
    let pinned = Config { check_pins: true, ..other.config(NO_LG, None) };
    assert_eq!(run(pinned, pipe().0), EXIT_UNUSABLE, "LG's files are not the pinned ones");
    let keyless = Config { tls: true, ..other.config(NO_LG, None) };
    assert_eq!(run(keyless, pipe().0), EXIT_UNUSABLE, "no key from the bus");
}

#[test]
fn the_release_version_is_read_as_lg_reads_it() {
    assert_eq!(release_version("webOS TV Rockhopper release 10.2.1-5202 (ombre-x)\n").as_deref(), Some("10.2.1"));
    assert_eq!(release_version("release 10.2.1 (x)"), None, "a word must come before `release`");
    assert_eq!(release_version("a release 10 x"), None);
}
