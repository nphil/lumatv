//! One client connection: the HTTP request that opens it, then an SSAP session, a pointer-socket proxy, a file download or a
//! byte relay to LG's gateway. Behaviour follows `web-socket-remote-client.js`, `gateway.js` and `http-server.js`.

use crate::adapter::{self, Act, Env, Kind};
use crate::iface::parse_uri;
use crate::luna::Owner;
use crate::net::{self, Conn, Pipe};
use crate::server::Ctx;
use crate::store::{self, Client, RECHECK_SECS};
use crate::util::{self, truthy};
use crate::ws::{self, Assembler, Head, Msg, Parsed};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, VecDeque};
use std::os::fd::{OwnedFd, RawFd};
use std::time::{Duration, Instant, UNIX_EPOCH};

/// LG's `MSG_MAX_SIZE`, in UTF-16 units as JavaScript counts a string.
const MSG_MAX: usize = 2_097_152;
/// LG's ping schedule: the first after 22.5 s of silence, doubling to 90 s; 15 s for the pong.
const PING_FIRST: Duration = Duration::from_millis(22_500);
const PING_MAX: Duration = Duration::from_secs(90);
const PONG_WAIT: Duration = Duration::from_secs(15);
const HEAD_WAIT: Duration = Duration::from_secs(30);
const CLOSE_WAIT: Duration = Duration::from_secs(2);
/// Output queued past this closes the session: the client stopped reading.
const BACKLOG_MAX: usize = 8 * 1024 * 1024;
/// Resource files are read whole into memory; larger ones are refused.
const FILE_MAX: u64 = 4 * 1024 * 1024;
/// Operations one session may have open at once.
const OPS_MAX: usize = 1024;
const GATEWAY: &str = "com.webos.service.secondscreen.gateway";
const APIADAPTER: &str = "com.webos.service.apiadapter";
const APIADAPTER_PUB: &str = "com.webos.service.apiadapter.pub";
const SIDE_REGISTER: &str = "ssgw-reg";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Which {
    Client,
    /// The session's own connection to LG's gateway, for methods this gateway leaves to LG.
    Side,
    /// LG's gateway when the whole connection is relayed.
    Up,
    /// The pointer socket.
    Target,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Head,
    Ssap,
    Pointer,
    /// An SSAP session LG serves, re-framed so this gateway can watch registrations and rewrite resource URLs.
    Relay,
    /// Bytes passed both ways untouched (LG's own resources).
    Raw,
    Closing,
}

struct OpEntry {
    id: Option<Value>,
    key: String,
    sub: bool,
    media: Vec<String>,
    socket: Vec<String>,
    adapter: Option<adapter::Op>,
}

enum Route {
    Op(u32),
    Side(String),
}

/// What LG's answer to a relayed registration teaches the store.
struct RegWatch {
    id: Option<Value>,
    sent_key: Option<String>,
    /// The grant and app id of an unsigned manifest, when LG's acceptance is to be remembered.
    grant: Option<(Vec<String>, Option<String>)>,
}

struct Relay {
    accept: String,
    open: bool,
    asm: Assembler,
    watch: Option<RegWatch>,
}

struct Side {
    pipe: Pipe,
    accept: String,
    open: bool,
    registering: bool,
    /// Frames waiting for the handshake or a registration; `true` marks a register.
    held: VecDeque<(bool, Vec<u8>)>,
    asm: Assembler,
    /// LG-side id to the client's id, whether it is a subscription, and the client's id key.
    pending: HashMap<String, (Option<Value>, bool, String)>,
    next: u64,
}

impl Side {
    fn open(lg_http: &str, host: &str, key: Option<&str>) -> std::io::Result<Side> {
        let mut pipe = Pipe::tcp(lg_http)?;
        let (req, k) = ws::client_handshake(host, "/");
        pipe.send(&req);
        let mut side = Side {
            pipe,
            accept: ws::accept_key(&k),
            open: false,
            registering: false,
            held: VecDeque::new(),
            asm: Assembler::default(),
            pending: HashMap::new(),
            next: 0,
        };
        if let Some(key) = key {
            side.register(key);
        }
        Ok(side)
    }

    fn register(&mut self, key: &str) {
        let msg = json!({"type": "register", "id": SIDE_REGISTER, "payload": {"client-key": key}});
        self.held.push_back((true, masked(&msg)));
        self.release();
    }

    fn send(&mut self, msg: &Value) {
        self.held.push_back((false, masked(msg)));
        self.release();
    }

    /// Sends what was held back, stopping after a register until LG answers it: LG loads the profile asynchronously and
    /// would judge a request that overtook it as unregistered.
    fn release(&mut self) {
        while self.open && !self.registering {
            let Some((reg, frame)) = self.held.pop_front() else { break };
            self.pipe.send(&frame);
            self.registering = reg;
        }
    }
}

fn masked(msg: &Value) -> Vec<u8> {
    let mut f = Vec::new();
    ws::encode(&mut f, ws::OP_TEXT, msg.to_string().as_bytes(), true);
    f
}

pub struct Session {
    pub id: u32,
    conn: Conn,
    rx: Vec<u8>,
    phase: Phase,
    tls: bool,
    host: String,
    peer: String,
    asm: Assembler,
    hello_done: bool,
    key: Option<String>,
    /// The fingerprint of `key`: the stored client and the owner of launch sessions.
    client: String,
    ops: HashMap<u32, OpEntry>,
    routes: HashMap<String, Route>,
    next_op: u32,
    /// Resource paths handed out, by category and file, so a file keeps its URL for the session.
    res: HashMap<(bool, String), String>,
    ping_every: Duration,
    ping_at: Option<Instant>,
    pong_by: Option<Instant>,
    side: Option<Side>,
    up: Option<Pipe>,
    relay: Option<Relay>,
    client_eof: bool,
    up_shut: bool,
    target: Option<Pipe>,
    target_q: Vec<u8>,
    head_by: Option<Instant>,
    close_by: Option<Instant>,
    pub dead: bool,
}

impl Session {
    pub fn new(id: u32, conn: Conn, tls: bool, now: Instant) -> Session {
        let peer = conn.peer();
        Session {
            id,
            conn,
            rx: Vec::new(),
            phase: Phase::Head,
            tls,
            host: String::new(),
            peer,
            asm: Assembler::default(),
            hello_done: false,
            key: None,
            client: String::new(),
            ops: HashMap::new(),
            routes: HashMap::new(),
            next_op: 1,
            res: HashMap::new(),
            ping_every: PING_FIRST,
            ping_at: None,
            pong_by: None,
            side: None,
            up: None,
            relay: None,
            client_eof: false,
            up_shut: false,
            target: None,
            target_q: Vec::new(),
            head_by: Some(now + HEAD_WAIT),
            close_by: None,
            dead: false,
        }
    }

    /// Whether the session is a live SSAP or pointer connection (what the control socket counts).
    pub fn is_open(&self) -> bool {
        matches!(self.phase, Phase::Ssap | Phase::Pointer | Phase::Relay)
    }

    pub fn fds(&self, out: &mut Vec<(Which, RawFd, i16)>) {
        let ev = |eof: bool, write: bool| (if eof { 0 } else { libc::POLLIN }) | if write { libc::POLLOUT } else { 0 };
        out.push((Which::Client, self.conn.fd(), ev(self.conn.eof, self.conn.wants_write())));
        if let Some(s) = &self.side {
            out.push((Which::Side, s.pipe.fd(), ev(s.pipe.eof, s.pipe.wants_write())));
        }
        if let Some(u) = &self.up {
            out.push((Which::Up, u.fd(), ev(u.eof, u.wants_write())));
        }
        if let Some(t) = &self.target {
            out.push((Which::Target, t.fd(), ev(t.eof, t.wants_write())));
        }
    }

    pub fn on_ready(&mut self, ctx: &mut Ctx, which: Which, now: Instant) {
        match which {
            Which::Client => self.client_ready(ctx, now),
            Which::Side => self.side_ready(ctx, now),
            Which::Up => self.up_ready(ctx, now),
            Which::Target => self.target_ready(ctx, now),
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        [self.head_by, self.ping_at, self.pong_by, self.close_by].into_iter().flatten().min()
    }

    pub fn tick(&mut self, ctx: &mut Ctx, now: Instant) {
        if self.head_by.is_some_and(|t| t <= now) || self.close_by.is_some_and(|t| t <= now) {
            self.finish(ctx);
            return;
        }
        if self.pong_by.is_some_and(|t| t <= now) {
            crate::info!("session {}: no pong from {}", self.id, self.peer);
            self.close(ctx, Some(1000), "client did not respond to ping", now);
            return;
        }
        if self.ping_at.is_some_and(|t| t <= now) {
            let mut f = Vec::new();
            ws::encode(&mut f, ws::OP_PING, b"", false);
            self.conn.queue(&f);
            self.ping_every = (self.ping_every * 2).min(PING_MAX);
            self.ping_at = None;
            self.pong_by = Some(now + PONG_WAIT);
        }
    }

    /// The gateway is stopping or mobile access was turned off.
    pub fn shut(&mut self, ctx: &mut Ctx, now: Instant) {
        match self.phase {
            Phase::Ssap | Phase::Pointer | Phase::Relay => self.close(ctx, Some(1001), "server shutting down", now),
            Phase::Closing => {}
            Phase::Head | Phase::Raw => self.finish(ctx),
        }
    }

    /// Writes what is queued, everywhere; ends a session whose close went out or whose peers stopped reading.
    pub fn flush(&mut self, ctx: &mut Ctx, now: Instant) {
        if let Some(s) = self.side.as_mut()
            && (s.pipe.flush().is_err() || s.pipe.backlog() > BACKLOG_MAX)
        {
            self.side_failed();
        }
        if let Some(t) = self.target.as_mut()
            && (t.flush().is_err() || t.backlog() > BACKLOG_MAX)
        {
            self.close(ctx, Some(1011), "error in target socket", now);
        }
        if let Some(u) = self.up.as_mut() {
            if u.flush().is_err() || u.backlog() > BACKLOG_MAX {
                self.finish(ctx);
                return;
            }
            if self.client_eof && !self.up_shut && !u.wants_write() {
                let _ = u.stream.shutdown(std::net::Shutdown::Write);
                self.up_shut = true;
            }
        }
        if let Err(e) = self.conn.flush() {
            crate::debug!("session {}: {e}", self.id);
            self.finish(ctx);
            return;
        }
        if self.conn.backlog() > BACKLOG_MAX {
            crate::warn!("session {}: {} stopped reading; closing", self.id, self.peer);
            self.finish(ctx);
            return;
        }
        if self.phase == Phase::Closing && !self.conn.wants_write() {
            self.conn.close_notify();
            let _ = self.conn.flush();
            self.finish(ctx);
        }
    }

    /// Ends the session at once, dropping whatever it was doing.
    fn finish(&mut self, ctx: &mut Ctx) {
        self.release(ctx);
        self.dead = true;
    }

    fn release(&mut self, ctx: &mut Ctx) {
        ctx.luna.cancel_session(self.id);
        self.ops.clear();
        self.routes.clear();
        self.side = None;
        self.up = None;
        self.relay = None;
        self.target = None;
        self.ping_at = None;
        self.pong_by = None;
    }

    fn close(&mut self, ctx: &mut Ctx, code: Option<u16>, reason: &str, now: Instant) {
        if self.phase == Phase::Closing || self.dead {
            return;
        }
        let mut f = Vec::new();
        ws::encode(&mut f, ws::OP_CLOSE, &ws::close_body(code, reason), false);
        self.conn.queue(&f);
        self.closing(ctx, now);
    }

    fn closing(&mut self, ctx: &mut Ctx, now: Instant) {
        self.release(ctx);
        self.phase = Phase::Closing;
        self.head_by = None;
        self.close_by = Some(now + CLOSE_WAIT);
    }

    fn touch(&mut self, now: Instant) {
        self.ping_at = Some(now + self.ping_every);
        self.pong_by = None;
    }

    fn send(&mut self, msg: &Value) {
        if !matches!(self.phase, Phase::Ssap | Phase::Relay) {
            return;
        }
        let text = msg.to_string();
        let mut f = Vec::with_capacity(text.len() + 10);
        ws::encode(&mut f, ws::OP_TEXT, text.as_bytes(), false);
        self.conn.queue(&f);
    }

    // ---- client input ----

    fn client_ready(&mut self, ctx: &mut Ctx, now: Instant) {
        if let Err(e) = self.conn.read(&mut self.rx) {
            crate::debug!("session {}: {e}", self.id);
            self.finish(ctx);
            return;
        }
        self.process(ctx, now);
        if self.conn.eof && !self.dead {
            match self.phase {
                Phase::Relay | Phase::Raw => self.client_eof = true,
                _ => self.finish(ctx),
            }
        }
    }

    fn process(&mut self, ctx: &mut Ctx, now: Instant) {
        loop {
            match self.phase {
                Phase::Head => {
                    match ws::parse_head(&self.rx) {
                        Ok(Some(head)) => self.on_head(ctx, head, now),
                        Ok(None) => return,
                        Err(()) => {
                            self.finish(ctx);
                            return;
                        }
                    }
                    if self.dead {
                        return;
                    }
                }
                Phase::Ssap | Phase::Pointer => {
                    self.take_frames(ctx, now);
                    if self.phase != Phase::Relay {
                        return;
                    }
                }
                Phase::Relay | Phase::Raw => {
                    if let Some(up) = self.up.as_mut() {
                        up.send(&self.rx);
                    }
                    self.rx.clear();
                    return;
                }
                Phase::Closing => {
                    self.rx.clear();
                    return;
                }
            }
        }
    }

    fn on_head(&mut self, ctx: &mut Ctx, head: Head, now: Instant) {
        self.head_by = None;
        self.host = head.header("host").unwrap_or_default().to_owned();
        if head.is_upgrade() {
            self.upgrade(ctx, head, now);
        } else {
            self.http(ctx, head, now);
        }
    }

    fn upgrade(&mut self, ctx: &mut Ctx, head: Head, now: Instant) {
        let Some(key) = head.header("sec-websocket-key").map(str::to_owned) else {
            self.conn.queue(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            self.closing(ctx, now);
            return;
        };
        let resource = head.target.starts_with("/resources/");
        if resource && !ctx.resources.contains(&head.target) {
            // a resource LG's gateway handed out (a relayed method's reply)
            self.start_raw(ctx);
            return;
        }
        self.rx.drain(..head.len);
        self.conn.queue(&ws::server_handshake(&key));
        self.phase = Phase::Ssap;
        if ctx.standby {
            self.close(ctx, Some(1008), "Try Again Later (EWS)", now);
            return;
        }
        if resource {
            match ctx.resources.take_socket(&head.target) {
                Some(path) => self.start_pointer(ctx, path, now),
                None => self.close(ctx, Some(1008), "", now),
            }
            return;
        }
        if let Some(origin) = head.header("origin")
            && origin != "null"
            && !origin.starts_with("file://")
            && !origin.starts_with("chrome-extension://")
        {
            self.close(ctx, Some(1008), "invalid origin", now);
            return;
        }
        self.touch(now);
        crate::info!("session {}: {} connected", self.id, self.peer);
    }

    fn http(&mut self, ctx: &mut Ctx, head: Head, now: Instant) {
        if head.target == "/" {
            self.respond(&head, "200 OK", b"Hello world\r\n", &[]);
        } else if let Some(file) = ctx.resources.media(&head.target) {
            let file = file.to_owned();
            self.send_file(&head, &file);
        } else if head.target.starts_with("/resources/") {
            self.start_raw(ctx);
            return;
        } else {
            // LG's gateway never answers other paths
            crate::debug!("session {}: no handler for {} {}", self.id, head.method, head.target);
        }
        self.rx.clear();
        self.closing(ctx, now);
    }

    fn respond(&mut self, head: &Head, status: &str, body: &[u8], extra: &[(&str, String)]) {
        let mut out = format!("HTTP/1.1 {status}\r\n");
        for (k, v) in extra {
            out.push_str(&format!("{k}: {v}\r\n"));
        }
        let date = util::http_date((util::now_ms() / 1000) as i64);
        let length = if status.starts_with("304") { String::new() } else { format!("Content-Length: {}\r\n", body.len()) };
        out.push_str(&format!("Date: {date}\r\n{length}Connection: close\r\n\r\n"));
        self.conn.queue(out.as_bytes());
        if head.method != "HEAD" {
            self.conn.queue(body);
        }
    }

    fn send_file(&mut self, head: &Head, file: &str) {
        let meta = match std::fs::metadata(file) {
            Ok(m) if m.is_file() && m.len() <= FILE_MAX => m,
            _ => return self.respond(head, "404 Not Found", b"", &[]),
        };
        let mtime_ms = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64);
        let since = head.header("if-modified-since").and_then(util::parse_http_date);
        if let (Some(m), Some(s)) = (mtime_ms, since)
            && m <= s
        {
            return self.respond(head, "304 Not Modified", b"", &[]);
        }
        let Ok(body) = std::fs::read(file) else {
            return self.respond(head, "404 Not Found", b"", &[]);
        };
        let mut extra = vec![("Cache-Control", "private".to_owned())];
        if let Some(m) = mtime_ms {
            extra.push(("Last-Modified", util::http_date(m.div_euclid(1000))));
        }
        self.respond(head, "200 OK", &body, &extra);
    }

    fn start_raw(&mut self, ctx: &mut Ctx) {
        match Pipe::tcp(&ctx.lg_http) {
            Ok(mut p) => {
                p.send(&self.rx);
                self.rx.clear();
                self.up = Some(p);
                self.phase = Phase::Raw;
            }
            Err(e) => {
                crate::warn!("session {}: LG's gateway at {} is unreachable: {e}", self.id, ctx.lg_http);
                self.finish(ctx);
            }
        }
    }

    fn take_frames(&mut self, ctx: &mut Ctx, now: Instant) {
        while matches!(self.phase, Phase::Ssap | Phase::Pointer) {
            match ws::parse_frame(&self.rx) {
                Parsed::Incomplete => return,
                Parsed::Fail(code) => {
                    self.close(ctx, Some(code), "", now);
                    return;
                }
                Parsed::Frame(f, used) => {
                    self.rx.drain(..used);
                    match self.asm.push(f) {
                        Err(code) => self.close(ctx, Some(code), "", now),
                        Ok(None) => {}
                        Ok(Some(m)) => self.on_msg(ctx, m, now),
                    }
                }
            }
        }
    }

    fn on_msg(&mut self, ctx: &mut Ctx, m: Msg, now: Instant) {
        match m {
            Msg::Ping(p) => {
                self.touch(now);
                let mut f = Vec::new();
                ws::encode(&mut f, ws::OP_PONG, &p, false);
                self.conn.queue(&f);
            }
            Msg::Pong => self.touch(now),
            Msg::Close(body) => {
                // 1005, 1006 and 1015 describe a close and may not be sent in one
                let code = (body.len() >= 2).then(|| u16::from_be_bytes([body[0], body[1]])).filter(|c| !matches!(c, 1005 | 1006 | 1015));
                self.close(ctx, code, "", now);
            }
            Msg::Text(data) | Msg::Binary(data) if self.phase == Phase::Pointer => {
                self.touch(now);
                let mut f = Vec::with_capacity(data.len() + 4);
                f.extend_from_slice(&(data.len() as u32).to_be_bytes());
                f.extend_from_slice(&data);
                match self.target.as_mut() {
                    Some(t) => t.send(&f),
                    None => self.target_q.extend_from_slice(&f),
                }
            }
            Msg::Binary(_) => {
                self.touch(now);
                self.send(&error_msg(&None, "400 binary messages not supported", json!({})));
            }
            Msg::Text(data) => {
                self.touch(now);
                self.on_text(ctx, &data, now);
            }
        }
    }

    fn on_text(&mut self, ctx: &mut Ctx, data: &[u8], now: Instant) {
        let text = String::from_utf8_lossy(data);
        if text.len() > MSG_MAX && text.chars().map(char::len_utf16).sum::<usize>() > MSG_MAX {
            self.send(&error_msg(&None, "413 message size limit exceeded", json!({})));
            return;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&text) else {
            self.close(ctx, Some(1008), "invalid message", now);
            return;
        };
        if msg.is_null() {
            // LG's handler throws on `null` and answers nothing
            return;
        }
        let ty = msg.get("type").and_then(Value::as_str).unwrap_or("");
        match ty {
            "request" | "subscribe" => {
                let sub = ty == "subscribe";
                self.on_request(ctx, msg, sub);
            }
            "unsubscribe" => self.on_unsubscribe(ctx, &msg),
            "p2p" => {
                if let Some(side) = self.side.as_mut() {
                    side.send(&msg);
                }
            }
            "register" => self.on_register(ctx, msg, &text),
            "hello" => self.on_hello(ctx, &msg),
            "verification" => crate::debug!("session {}: verification message ignored", self.id),
            _ => self.send(&error_msg(&msg.get("id").cloned(), "400 unknown message type", json!({}))),
        }
    }

    fn on_hello(&mut self, ctx: &Ctx, msg: &Value) {
        if self.hello_done || matches!(msg.get("payload"), None | Some(Value::Null)) {
            return;
        }
        self.hello_done = true;
        self.send(&json!({"type": "hello", "payload": ctx.hello}));
    }

    fn on_unsubscribe(&mut self, ctx: &mut Ctx, msg: &Value) {
        match self.routes.remove(&id_key(msg.get("id"))) {
            Some(Route::Op(op)) => self.end_op(ctx, op),
            Some(Route::Side(sid)) => {
                if let Some(side) = self.side.as_mut() {
                    side.pending.remove(&sid);
                    side.send(&json!({"type": "unsubscribe", "id": sid}));
                }
            }
            None => {}
        }
    }

    // ---- requests ----

    fn on_request(&mut self, ctx: &mut Ctx, msg: Value, sub: bool) {
        let id = msg.get("id").cloned();
        let idk = id_key(id.as_ref());
        let uri = match msg.get("uri") {
            Some(Value::String(s)) => s.clone(),
            other => id_key(other),
        };
        ctx.note(&uri);
        if truthy(msg.get("client-key")) {
            // a key of its own: LG judges it against that key's profile
            self.relay_side(ctx, msg, id, idk, sub);
            return;
        }
        let Some((service, path)) = parse_uri(&uri) else {
            self.send(&error_msg(&id, "404 no such service or method", json!({})));
            return;
        };
        let (service, path) =
            if service == "com.webos.applicationManager" && path == "/launch" { (APIADAPTER_PUB, "/application_manager/launch") } else { (service, path) };
        let Some(found) = ctx.catalog.find(service, path) else {
            self.send(&error_msg(&id, "404 no such service or method", json!({})));
            return;
        };
        let required = &found.method.required;
        let denied = match &self.key {
            // LG's `isAllowed`: any one of the required permissions; a method needing none is closed to registered clients
            Some(_) => {
                let ok = ctx.store.get(&self.client).is_some_and(|c| required.iter().any(|p| c.grant.contains(p)));
                (!ok).then_some("401 insufficient permissions")
            }
            None => (!required.is_empty()).then_some("401 insufficient permissions (not registered)"),
        };
        if let Some(e) = denied {
            self.send(&error_msg(&id, e, json!({})));
            return;
        }
        let (service, path) = (found.service.to_owned(), found.path.to_owned());
        let (sci, media, socket) = (found.method.send_client_info, found.method.media.clone(), found.method.socket.clone());
        let payload = msg.get("payload").filter(|v| truthy(Some(v))).cloned().unwrap_or_else(|| json!({}));

        if service == GATEWAY {
            match path.as_str() {
                "/getServiceList" => {
                    let mut list = ctx.catalog.service_list();
                    if let Some(m) = list.as_object_mut() {
                        m.insert("returnValue".into(), Value::Bool(true));
                    }
                    self.send(&response(&id, list));
                }
                // pairing prompts run in LG's gateway, so no PIN is ever pending here
                "/pairing/setPin" => self.send(&error_msg(&id, "403 Access denied", json!({}))),
                _ => self.relay_side(ctx, msg, id, idk, sub),
            }
            return;
        }
        if self.ops.len() >= OPS_MAX {
            crate::warn!("session {}: {OPS_MAX} operations open; refusing {uri}", self.id);
            self.send(&error_msg(&id, "500 Gateway error", json!({})));
            return;
        }
        if let Some(kind) = Kind::of(&service, &path, sub) {
            if kind == Kind::Close && ctx.launches.find(&payload, &self.client).is_none() {
                // the app was launched through LG's gateway, which holds its session
                self.relay_side(ctx, msg, id, idk, sub);
                return;
            }
            let op_id = self.add_op(id, idk, sub, media, socket);
            let mut env = Env { client: &self.client, launches: &mut ctx.launches, icons: &mut ctx.icons };
            let (op, acts) = adapter::Op::start(kind, payload, sub, &mut env);
            if let Some(entry) = self.ops.get_mut(&op_id) {
                entry.adapter = Some(op);
            }
            self.apply(ctx, op_id, acts);
            return;
        }
        if service == APIADAPTER || service == APIADAPTER_PUB || sci {
            self.relay_side(ctx, msg, id, idk, sub);
            return;
        }
        let mut payload = payload;
        if let Some(m) = payload.as_object_mut() {
            for k in ["_socketClientId", "_remoteClientId", "_remoteClientName", "_remoteClientBaseUrl"] {
                m.remove(k);
            }
            if sub {
                m.insert("subscribe".into(), Value::Bool(true));
            }
        }
        let op_id = self.add_op(id, idk, sub, media, socket);
        let owner = Owner { sess: self.id, op: op_id, tag: 0 };
        let uri = format!("luna://{service}{path}");
        if sub {
            ctx.luna.subscribe(owner, &uri, &payload);
        } else {
            ctx.luna.call(owner, &uri, &payload);
        }
    }

    fn add_op(&mut self, id: Option<Value>, key: String, sub: bool, media: Vec<String>, socket: Vec<String>) -> u32 {
        let op_id = self.next_op;
        self.next_op = self.next_op.wrapping_add(1).max(1);
        // a reused id names the newest request; the older one keeps running, as in LG
        self.routes.insert(key.clone(), Route::Op(op_id));
        self.ops.insert(op_id, OpEntry { id, key, sub, media, socket, adapter: None });
        op_id
    }

    fn end_op(&mut self, ctx: &mut Ctx, op_id: u32) {
        if let Some(op) = self.ops.remove(&op_id) {
            ctx.luna.cancel_op(self.id, op_id);
            if matches!(self.routes.get(&op.key), Some(Route::Op(o)) if *o == op_id) {
                self.routes.remove(&op.key);
            }
        }
    }

    fn apply(&mut self, ctx: &mut Ctx, op_id: u32, acts: Vec<Act>) {
        let sess = self.id;
        for act in acts {
            if !self.ops.contains_key(&op_id) {
                return;
            }
            match act {
                Act::Call(tag, uri, p) => ctx.luna.call(Owner { sess, op: op_id, tag }, uri, &p),
                Act::Sub(tag, uri, p) => {
                    ctx.luna.subscribe(Owner { sess, op: op_id, tag }, uri, &p);
                }
                Act::Cancel(tag) => ctx.luna.cancel(Owner { sess, op: op_id, tag }),
                Act::Emit(v) => self.deliver(ctx, op_id, &v),
                Act::Finish => self.end_op(ctx, op_id),
            }
        }
    }

    /// A reply from the bus (or an end, `None`) for one of this session's operations.
    pub fn on_luna(&mut self, ctx: &mut Ctx, owner: Owner, reply: Option<&Value>) {
        let Some(op) = self.ops.get_mut(&owner.op) else { return };
        if let Some(a) = op.adapter.as_mut() {
            let mut env = Env { client: &self.client, launches: &mut ctx.launches, icons: &mut ctx.icons };
            let acts = match reply {
                Some(v) => a.on_reply(owner.tag, v, &mut env),
                None => a.on_end(owner.tag, &mut env),
            };
            self.apply(ctx, owner.op, acts);
            return;
        }
        match reply {
            Some(v) => self.deliver(ctx, owner.op, v),
            None => {
                let id = op.id.clone();
                self.send(&error_msg(&id, "500 Gateway error", json!({})));
                self.end_op(ctx, owner.op);
            }
        }
    }

    /// `sendRequest`'s handling of a reply: an error ends the operation, a success is passed on with its resources
    /// rewritten, and a request ends after its first answer.
    fn deliver(&mut self, ctx: &mut Ctx, op_id: u32, v: &Value) {
        let Some(op) = self.ops.get(&op_id) else { return };
        let id = op.id.clone();
        if v.get("returnValue") == Some(&Value::Bool(false)) {
            let code = v.get("errorCode").filter(|c| c.as_f64().is_some_and(|n| n > 0.0));
            match (code, v.get("errorText").filter(|t| truthy(Some(t)))) {
                (Some(c), Some(t)) => self.send(&error_msg(&id, &format!("{} {}", id_key(Some(c)), id_key(Some(t))), json!({}))),
                _ => self.send(&error_msg(&id, "500 Application error", v.clone())),
            }
            self.end_op(ctx, op_id);
            return;
        }
        let sub = op.sub;
        let payload = if op.media.is_empty() && op.socket.is_empty() {
            v.clone()
        } else {
            let (media, socket) = (op.media.clone(), op.socket.clone());
            self.rewrite(ctx, v.clone(), &media, &socket)
        };
        self.send(&response(&id, payload));
        if !sub {
            self.end_op(ctx, op_id);
        }
    }

    /// Replaces file and socket paths in a reply with URLs this gateway serves (`addResourceHandler`).
    fn rewrite(&mut self, ctx: &mut Ctx, mut v: Value, media: &[String], socket: &[String]) -> Value {
        let (http, ws) = if self.tls { ("https", "wss") } else { ("http", "ws") };
        let host = self.host.clone();
        let res = &mut self.res;
        let resources = &mut ctx.resources;
        for (props, is_socket) in [(media, false), (socket, true)] {
            for prop in props {
                let pats: Vec<&str> = prop.split('.').collect();
                json_match(&mut v, &pats, &mut |value: &str| {
                    let file = if is_socket || value.starts_with("file://") { value.to_owned() } else { value.replacen("file://", "", 1) };
                    let path = res
                        .entry((is_socket, file.clone()))
                        .or_insert_with(|| {
                            let hash = util::sha1_hex(format!("{}{}", util::now_ms(), file).as_bytes());
                            util::path_join("/resources/", &format!("{hash}/{}", escape_name(util::basename(&file))))
                        })
                        .clone();
                    resources.add(&path, &file, is_socket);
                    format!("{}://{host}{path}", if is_socket { ws } else { http })
                });
            }
        }
        v
    }

    // ---- registration ----

    fn on_register(&mut self, ctx: &mut Ctx, msg: Value, text: &str) {
        let id = msg.get("id").cloned();
        let empty = Map::new();
        let p = msg.get("payload").and_then(Value::as_object).unwrap_or(&empty);
        let key = p.get("client-key").filter(|k| truthy(Some(k))).and_then(Value::as_str).map(str::to_owned);
        let manifest = p.get("manifest");
        let signed = manifest.is_some_and(|m| truthy(m.get("signed")) && truthy(m.get("signatures")));
        let plain = !truthy(p.get("companion-client-key")) && !truthy(p.get("proxyRegister"));
        let secs = util::now_ms() / 1000;
        let fp = key.as_deref().map(store::fingerprint);
        let fresh = fp.as_deref().and_then(|f| ctx.store.get(f)).is_some_and(|c| secs.saturating_sub(c.checked) < RECHECK_SECS);
        let manifest_ok = manifest.is_none_or(|m| m.is_object() && !signed);
        if fresh && plain && manifest_ok && !truthy(p.get("forcePairing")) && ctx.first_use_done() {
            let (key, fp) = (key.unwrap_or_default(), fp.unwrap_or_default());
            if let Some(m) = manifest
                && let Some(stored) = ctx.store.get(&fp)
            {
                // LG takes a known key's new manifest without a prompt
                let updated = Client {
                    grant: ctx.catalog.grant(m.get("permissions")),
                    app_id: m.get("appId").and_then(Value::as_str).map(str::to_owned),
                    ..stored.clone()
                };
                if updated != *stored {
                    ctx.store.put(updated);
                }
            }
            self.key = Some(key.clone());
            self.client = fp;
            if let Some(side) = self.side.as_mut() {
                side.register(&key);
            }
            let mut reply = Map::new();
            reply.insert("type".into(), "registered".into());
            if let Some(id) = &id {
                reply.insert("id".into(), id.clone());
            }
            reply.insert("payload".into(), json!({"client-key": key}));
            self.send(&Value::Object(reply));
            crate::info!("session {}: registered {}", self.id, ctx.store.get(&self.client).and_then(|c| c.app_id.as_deref()).unwrap_or("client"));
            return;
        }
        let grant = (plain && !signed)
            .then(|| manifest.filter(|m| m.is_object()))
            .flatten()
            .map(|m| (ctx.catalog.grant(m.get("permissions")), m.get("appId").and_then(Value::as_str).map(str::to_owned)));
        self.start_relay(ctx, text, RegWatch { id, sent_key: key, grant });
    }

    fn start_relay(&mut self, ctx: &mut Ctx, register: &str, watch: RegWatch) {
        let mut pipe = match Pipe::tcp(&ctx.lg_http) {
            Ok(p) => p,
            Err(e) => {
                crate::warn!("session {}: LG's gateway at {} is unreachable: {e}", self.id, ctx.lg_http);
                self.send(&error_msg(&watch.id, "500 internal error", json!({})));
                return;
            }
        };
        let (req, key) = ws::client_handshake(&self.host, "/");
        pipe.send(&req);
        let mut f = Vec::new();
        ws::encode(&mut f, ws::OP_TEXT, register.as_bytes(), true);
        pipe.send(&f);
        self.up = Some(pipe);
        self.relay = Some(Relay { accept: ws::accept_key(&key), open: false, asm: Assembler::default(), watch: Some(watch) });
        self.phase = Phase::Relay;
        // LG pings the client from here on
        self.ping_at = None;
        self.pong_by = None;
        self.close_by = None;
        crate::info!("session {}: {} registers with LG's gateway", self.id, self.peer);
    }

    // ---- LG's side ----

    fn up_ready(&mut self, ctx: &mut Ctx, now: Instant) {
        let Some(up) = self.up.as_mut() else { return };
        if let Err(e) = up.read() {
            crate::debug!("session {}: LG's gateway: {e}", self.id);
            self.finish(ctx);
            return;
        }
        let eof = up.eof;
        if self.phase == Phase::Raw {
            let data = std::mem::take(&mut up.rx);
            self.conn.queue(&data);
        } else {
            self.relay_frames(ctx, now);
        }
        if eof && !self.dead && self.phase != Phase::Closing {
            // whatever LG sent last (its close frame) is on its way to the client
            self.closing(ctx, now);
        }
    }

    fn relay_frames(&mut self, ctx: &mut Ctx, now: Instant) {
        let (Some(up), Some(relay)) = (self.up.as_mut(), self.relay.as_mut()) else { return };
        if !relay.open {
            match ws::parse_head(&up.rx) {
                Ok(None) => return,
                Ok(Some(h)) if h.status == 101 && h.header("sec-websocket-accept") == Some(relay.accept.as_str()) => {
                    up.rx.drain(..h.len);
                    relay.open = true;
                }
                _ => {
                    crate::warn!("session {}: LG's gateway refused the relayed connection", self.id);
                    self.close(ctx, Some(1011), "", now);
                    return;
                }
            }
        }
        let mut out: Vec<(u8, Vec<u8>)> = Vec::new();
        let mut learned: Vec<Value> = Vec::new();
        let mut fail = None;
        loop {
            match ws::parse_frame(&up.rx) {
                Parsed::Incomplete => break,
                Parsed::Fail(code) => {
                    fail = Some(code);
                    break;
                }
                Parsed::Frame(f, used) => {
                    up.rx.drain(..used);
                    match relay.asm.push(f) {
                        Err(code) => {
                            fail = Some(code);
                            break;
                        }
                        Ok(None) => {}
                        Ok(Some(Msg::Text(t))) => {
                            if relay.watch.is_some()
                                && let Ok(v) = serde_json::from_slice::<Value>(&t)
                            {
                                learned.push(v);
                            }
                            out.push((ws::OP_TEXT, t));
                        }
                        Ok(Some(Msg::Binary(b))) => out.push((ws::OP_BINARY, b)),
                        Ok(Some(Msg::Ping(p))) => out.push((ws::OP_PING, p)),
                        Ok(Some(Msg::Pong)) => out.push((ws::OP_PONG, Vec::new())),
                        Ok(Some(Msg::Close(b))) => out.push((ws::OP_CLOSE, b)),
                    }
                }
            }
        }
        for v in learned {
            self.learn(ctx, &v);
        }
        for (op, data) in out {
            let data = if op == ws::OP_TEXT { secure_urls(self.tls, &self.host, data) } else { data };
            let mut f = Vec::with_capacity(data.len() + 10);
            ws::encode(&mut f, op, &data, false);
            self.conn.queue(&f);
            if op == ws::OP_CLOSE {
                self.closing(ctx, now);
                return;
            }
        }
        if let Some(code) = fail {
            self.close(ctx, Some(code), "", now);
        }
    }

    /// Records what LG answered to a relayed registration.
    fn learn(&mut self, ctx: &mut Ctx, v: &Value) {
        let Some(relay) = self.relay.as_mut() else { return };
        let Some(watch) = relay.watch.as_ref() else { return };
        if v.get("id") != watch.id.as_ref() {
            return;
        }
        let secs = util::now_ms() / 1000;
        match v.get("type").and_then(Value::as_str) {
            Some("registered") => {
                if let Some(new) = v.pointer("/payload/client-key").and_then(Value::as_str) {
                    let fp = store::fingerprint(new);
                    if let Some(old) = &watch.sent_key
                        && old != new
                    {
                        ctx.store.remove(&store::fingerprint(old));
                    }
                    match &watch.grant {
                        Some((grant, app_id)) => ctx.store.put(Client { fp, grant: grant.clone(), app_id: app_id.clone(), checked: secs }),
                        None if watch.sent_key.as_deref() == Some(new) => {
                            if let Some(c) = ctx.store.get(&fp) {
                                ctx.store.put(Client { checked: secs, ..c.clone() });
                            }
                        }
                        None => {}
                    }
                }
                relay.watch = None;
            }
            Some("error") => {
                if v.get("error").and_then(Value::as_str).is_some_and(|e| e.starts_with("401"))
                    && let Some(old) = &watch.sent_key
                    && ctx.store.remove(&store::fingerprint(old))
                {
                    crate::info!("session {}: LG no longer knows a stored client; forgotten", self.id);
                }
                relay.watch = None;
            }
            _ => {}
        }
    }

    fn relay_side(&mut self, ctx: &mut Ctx, mut msg: Value, id: Option<Value>, idk: String, sub: bool) {
        if self.side.is_none() {
            match Side::open(&ctx.lg_http, &self.host, self.key.as_deref()) {
                Ok(s) => self.side = Some(s),
                Err(e) => {
                    crate::warn!("session {}: LG's gateway at {} is unreachable: {e}", self.id, ctx.lg_http);
                    self.send(&error_msg(&id, "500 Gateway error", json!({})));
                    return;
                }
            }
        }
        let Some(side) = self.side.as_mut() else { return };
        side.next += 1;
        let sid = format!("ssgw-{}", side.next);
        if let Some(m) = msg.as_object_mut() {
            m.insert("id".into(), Value::String(sid.clone()));
        }
        side.pending.insert(sid.clone(), (id, sub, idk.clone()));
        side.send(&msg);
        self.routes.insert(idk, Route::Side(sid));
    }

    fn side_ready(&mut self, ctx: &mut Ctx, _now: Instant) {
        let Some(side) = self.side.as_mut() else { return };
        if side.pipe.read().is_err() {
            self.side_failed();
            return;
        }
        if !side.open {
            match ws::parse_head(&side.pipe.rx) {
                Ok(None) => {
                    if side.pipe.eof {
                        self.side_failed();
                    }
                    return;
                }
                Ok(Some(h)) if h.status == 101 && h.header("sec-websocket-accept") == Some(side.accept.as_str()) => {
                    side.pipe.rx.drain(..h.len);
                    side.open = true;
                    side.release();
                }
                _ => {
                    self.side_failed();
                    return;
                }
            }
        }
        let mut out: Vec<Value> = Vec::new();
        let mut forget = false;
        let mut failed = side.pipe.eof;
        loop {
            match ws::parse_frame(&side.pipe.rx) {
                Parsed::Incomplete => break,
                Parsed::Fail(_) => {
                    failed = true;
                    break;
                }
                Parsed::Frame(f, used) => {
                    side.pipe.rx.drain(..used);
                    match side.asm.push(f) {
                        Err(_) | Ok(Some(Msg::Close(_))) => {
                            failed = true;
                            break;
                        }
                        Ok(Some(Msg::Ping(p))) => {
                            let mut pong = Vec::new();
                            ws::encode(&mut pong, ws::OP_PONG, &p, true);
                            side.pipe.send(&pong);
                        }
                        Ok(Some(Msg::Text(t))) => {
                            let Ok(mut v) = serde_json::from_slice::<Value>(&t) else { continue };
                            let sid = v.get("id").and_then(Value::as_str).map(str::to_owned);
                            if sid.as_deref() == Some(SIDE_REGISTER) {
                                forget |= v.get("type").and_then(Value::as_str) == Some("error")
                                    && v.get("error").and_then(Value::as_str).is_some_and(|e| e.starts_with("401"));
                                side.registering = false;
                                side.release();
                                continue;
                            }
                            if let Some(sid) = sid
                                && let Some((cid, sub, _)) = side.pending.get(&sid)
                            {
                                let ty = v.get("type").and_then(Value::as_str);
                                let last = ty == Some("error") || (!*sub && ty == Some("response"));
                                if let Some(m) = v.as_object_mut() {
                                    match cid {
                                        Some(c) => m.insert("id".into(), c.clone()),
                                        None => m.remove("id"),
                                    };
                                }
                                if last
                                    && let Some((_, _, idk)) = side.pending.remove(&sid)
                                    && matches!(self.routes.get(&idk), Some(Route::Side(s)) if *s == sid)
                                {
                                    self.routes.remove(&idk);
                                }
                            }
                            out.push(v);
                        }
                        Ok(_) => {}
                    }
                }
            }
        }
        if forget && let Some(key) = &self.key {
            ctx.store.remove(&store::fingerprint(key));
            crate::info!("session {}: LG no longer knows this client; forgotten", self.id);
        }
        for v in out {
            let text = secure_urls(self.tls, &self.host, v.to_string().into_bytes());
            if matches!(self.phase, Phase::Ssap | Phase::Relay) {
                let mut f = Vec::with_capacity(text.len() + 10);
                ws::encode(&mut f, ws::OP_TEXT, &text, false);
                self.conn.queue(&f);
            }
        }
        if failed {
            self.side_failed();
        }
    }

    /// LG's side went away: every request waiting on it fails as a gateway error.
    fn side_failed(&mut self) {
        let Some(side) = self.side.take() else { return };
        crate::debug!("session {}: connection to LG's gateway closed", self.id);
        self.routes.retain(|_, r| !matches!(r, Route::Side(_)));
        for (_, (id, _, _)) in side.pending {
            self.send(&error_msg(&id, "500 Gateway error", json!({})));
        }
    }

    // ---- pointer socket ----

    fn start_pointer(&mut self, ctx: &mut Ctx, path: String, now: Instant) {
        self.phase = Phase::Pointer;
        self.touch(now);
        crate::info!("session {}: {} opened {path}", self.id, self.peer);
        match ctx.broker.as_mut() {
            Some(b) => {
                if !b.request(self.id, &path) {
                    self.close(ctx, Some(1011), "error in target socket", now);
                }
            }
            None => {
                let r = net::unix_connect(&path).map_err(|e| e.raw_os_error().unwrap_or(libc::EIO));
                self.on_target(ctx, r, now);
            }
        }
    }

    /// The pointer socket's descriptor (or the errno that kept it).
    pub fn on_target(&mut self, ctx: &mut Ctx, r: Result<OwnedFd, i32>, now: Instant) {
        if self.phase != Phase::Pointer {
            return;
        }
        match r.and_then(|fd| Pipe::unix(fd).map_err(|e| e.raw_os_error().unwrap_or(libc::EIO))) {
            Ok(mut p) => {
                p.send(&self.target_q);
                self.target_q = Vec::new();
                self.target = Some(p);
            }
            Err(errno) => {
                crate::warn!("session {}: pointer socket: {}", self.id, std::io::Error::from_raw_os_error(errno));
                self.close(ctx, Some(1011), "error in target socket", now);
            }
        }
    }

    fn target_ready(&mut self, ctx: &mut Ctx, now: Instant) {
        let Some(t) = self.target.as_mut() else { return };
        match t.read() {
            Ok(()) => {
                // the input service has nothing to say to the client
                t.rx.clear();
                if t.eof {
                    self.close(ctx, None, "", now);
                }
            }
            Err(_) => self.close(ctx, Some(1011), "error in target socket", now),
        }
    }
}

/// `String(id)` as a JavaScript object key: how LG files requests by id.
fn id_key(id: Option<&Value>) -> String {
    match id {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => i.to_string(),
            (None, Some(f)) if f.fract() == 0.0 && f.abs() < 1e21 => format!("{f:.0}"),
            _ => n.to_string(),
        },
        Some(Value::Array(a)) => a.iter().map(|x| if x.is_null() { String::new() } else { id_key(Some(x)) }).collect::<Vec<_>>().join(","),
        Some(Value::Object(_)) => "[object Object]".into(),
    }
}

fn response(id: &Option<Value>, payload: Value) -> Value {
    let mut m = Map::new();
    m.insert("type".into(), "response".into());
    if let Some(id) = id {
        m.insert("id".into(), id.clone());
    }
    m.insert("payload".into(), payload);
    Value::Object(m)
}

fn error_msg(id: &Option<Value>, error: &str, payload: Value) -> Value {
    let mut m = Map::new();
    m.insert("type".into(), "error".into());
    if let Some(id) = id {
        m.insert("id".into(), id.clone());
    }
    m.insert("error".into(), error.into());
    m.insert("payload".into(), payload);
    Value::Object(m)
}

/// LG builds resource URLs for its plain port; the client reached it through the TLS one.
fn secure_urls(tls: bool, host: &str, text: Vec<u8>) -> Vec<u8> {
    if !tls || host.is_empty() {
        return text;
    }
    let (http, ws) = (format!("\"http://{host}/resources/"), format!("\"ws://{host}/resources/"));
    let s = match String::from_utf8(text) {
        Ok(s) => s,
        Err(e) => return e.into_bytes(),
    };
    if !s.contains(&http) && !s.contains(&ws) {
        return s.into_bytes();
    }
    s.replace(&http, &format!("\"https://{host}/resources/")).replace(&ws, &format!("\"wss://{host}/resources/")).into_bytes()
}

/// `jsonMatch` with replacement: arrays are walked, `*` takes every key, a falsy value stops the walk, strings at the end of
/// the path are replaced.
fn json_match(v: &mut Value, pats: &[&str], f: &mut dyn FnMut(&str) -> String) {
    match v {
        Value::Array(items) => {
            for x in items {
                json_match(x, pats, f);
            }
        }
        Value::Object(m) => {
            let Some((first, rest)) = pats.split_first() else { return };
            if *first == "*" {
                for x in m.values_mut() {
                    json_match(x, rest, f);
                }
            } else if let Some(x) = m.get_mut(*first)
                && truthy(Some(x))
            {
                json_match(x, rest, f);
            }
        }
        Value::String(s) if pats.is_empty() => *s = f(s),
        _ => {}
    }
}

/// A file name as it may appear in a URL path: Node's auto-escaped characters, `#`, `%`, `?`, controls and non-ASCII bytes
/// percent-encoded, so the URL a client is given is the path it asks for.
fn escape_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for b in name.bytes() {
        if b.is_ascii_graphic() && !b"\"#%'<>?\\^`{|}".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_key_as_javascript_strings() {
        assert_eq!(id_key(None), "undefined");
        assert_eq!(id_key(Some(&json!(7))), "7");
        assert_eq!(id_key(Some(&json!(7.0))), "7");
        assert_eq!(id_key(Some(&json!([1, null, "a"]))), "1,,a");
        assert_eq!(id_key(Some(&json!({"a": 1}))), "[object Object]");
    }

    #[test]
    fn json_match_replaces_leaves_on_the_path_only() {
        let mut v = json!({"apps": [{"icon": "/a.png", "title": "/t"}, {"icon": ""}], "icon": 5});
        json_match(&mut v, &["apps", "icon"], &mut |s| format!("<{s}>"));
        json_match(&mut v, &["icon"], &mut |s| format!("<{s}>"));
        assert_eq!(v, json!({"apps": [{"icon": "</a.png>", "title": "/t"}, {"icon": ""}], "icon": 5}));
        let mut w = json!({"launchPoints": {"x": {"icon": "/b"}}});
        json_match(&mut w, &["launchPoints", "*", "icon"], &mut |s| format!("<{s}>"));
        assert_eq!(w["launchPoints"]["x"]["icon"], "</b>");
    }

    #[test]
    fn names_escape_what_a_url_cannot_carry() {
        assert_eq!(escape_name("icon 80x80.png"), "icon%2080x80.png");
        assert_eq!(escape_name("a#b?c%é"), "a%23b%3Fc%25%C3%A9");
        assert_eq!(escape_name("plain-name_1.png"), "plain-name_1.png");
    }

    #[test]
    fn relayed_resource_urls_follow_the_client_onto_tls() {
        let text = br#"{"icon":"http://tv:3001/resources/a/i.png","pointer":"ws://tv:3001/resources/b/s","x":"http://other/resources/c"}"#;
        let out = String::from_utf8(secure_urls(true, "tv:3001", text.to_vec())).unwrap();
        assert_eq!(out, r#"{"icon":"https://tv:3001/resources/a/i.png","pointer":"wss://tv:3001/resources/b/s","x":"http://other/resources/c"}"#);
        assert_eq!(secure_urls(false, "tv:3001", text.to_vec()), text.to_vec());
        assert_eq!(secure_urls(true, "tv:3001", vec![0xff, b'"']), vec![0xff, b'"'], "bytes that are not UTF-8 pass untouched");
    }
}
