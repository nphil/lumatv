//! LG's Input Manager queues (`libim`): the handshake lginput2 makes with the compositor at each reconnect of the Magic Remote.
//!
//! lginput2, LG's input daemon, turns the remotes into evdev devices and links `libim`, LG's Input Manager library. The library's other
//! half ran inside LG's compositor (`surface-manager` loads `/usr/lib/libim.so.1.0.0`: its threads `tMQMsgHandler`, `tInput`, `tCursor`
//! and `tTIMER` read the evdev devices, drew the cursor and told lginput2 about it). The halves talk over POSIX message queues of 84-byte
//! messages, little-endian 32-bit words, word 0 the message type (`docs/research/im-queue-result.md` has the protocol in full):
//!
//! - `/IM_MQ_WEBOS`, created by the compositor (128 messages of 84 bytes), carries the requests to it: `{type, pid, argument}`.
//! - `/IM_MQ_MRCU`, created by lginput2, carries the compositor's answer to the cursor registration and its cursor notifications.
//! - `/mq_IMOpenAPI_<pid>`, created by the caller, carries the answers to the library's "open API" calls.
//!
//! lginput2 calls three of the library's functions. At each reconnect of the Magic Remote (it sleeps when laid down) its `mrcu_state_create`
//! calls `IM_GetInputDispatchType` and `IM_RegisterCursorInfoCallback`, each a request and a synchronous wait of 2 s for the reply:
//! `REGISTER_OPENAPI_MQ` answered on the caller's queue, `GET_INPUT_DISPATCH_TYPE` the same way, then `REGISTER_INPUTDEVICE_MQ` answered on
//! `/IM_MQ_MRCU` with the cursor's state. With LumaTV's compositor in LG's place nobody answered: every reconnect sat 4 s in `STAGE_ADDING`
//! before the remote worked (the first press after waking was lost), each timed-out call leaked its queue descriptors (4 per reconnect) and
//! its request stayed unread (round 19: 30 of them). `IM_RequestToCheckInput` sends `REQUEST_TO_CHECK_INPUT` with no reply when lginput2
//! makes a device. This module is the reader: it answers as `libim` does and tells lginput2 when the pointer shows or hides.
//!
//! What the answers do in lginput2 (its `mrcu_state_*`, read off the binary):
//! - The dispatch type must be 3 (`IM_INPUT_DISPATCH_ALL`, LG's default): 0, 2 and 5 put the remote's state machine in its no-pointer state.
//! - The registration's cursor info is `{visibility, support, speed, standby ms}`. Visibility 0 is shown, 1 hidden, 2 hidden with the remote
//!   sent to sleep (`COMMAND_SLEEP`), 3 unknown. Support 1 keeps the pointer enabled, 0 disables it, 2 leaves it. Speed 3 leaves lginput2's
//!   own; 0, 1 and 2 set the motion library's. The standby time is logged and nothing else.
//! - Told the pointer shows, lginput2 sends the wheel's press as `BTN_LEFT` instead of `KEY_ENTER` (entries 0 and 185 of its key table) and
//!   `CUSTOM_KEY_CURSOR_OFF` (1199) before a navigation key; told it is hidden, it sends `CUSTOM_KEY_CURSOR_ON` (1198) when the remote moves
//!   far from the centre or the wheel turns, asking for the pointer. Both keys are the compositor's (`input.rs`).
//!
//! Nothing here blocks: the queues are opened non-blocking, a full queue drops the message, and the requests left from before the compositor
//! started are read and discarded at the start (their callers gave up 2 s after sending; an answer now would be read as the answer to the
//! next call).

use std::ffi::CString;
use std::io;

use serde_json::{Value, json};

/// Every message is this long (`IM_MQ_Open` sets `mq_msgsize` 84, and every send passes 84).
pub const MSG_LEN: usize = 84;
/// LG's queues hold 128 messages. Needs `CAP_SYS_RESOURCE` above `/proc/sys/fs/mqueue/msg_max`; the set's compositor is root.
const LG_MAX_MSG: libc::c_long = 128;
/// What a queue is made with where 128 is refused (a host test without the capability): the kernel's `msg_default`.
const SMALL_MAX_MSG: libc::c_long = 10;
/// `IM_MQ_Open` creates with mode 0777.
const MODE: libc::c_uint = 0o777;

/// The message types (`IM_MQMSG_*`, the switch in libim's handler).
pub mod msg {
    pub const REGISTER_INPUTDEVICE_MQ: u32 = 0;
    pub const REQUEST_TO_CHECK_INPUT: u32 = 1;
    pub const SEND_CURSOR_VISIBILITY: u32 = 2;
    pub const SEND_CURSOR_SUPPORT: u32 = 3;
    pub const SEND_CURSOR_SPEED: u32 = 4;
    pub const REGISTER_OPENAPI_MQ: u32 = 5;
    pub const DEREGISTER_OPENAPI_MQ: u32 = 6;
    pub const SET_INPUT_DISPATCH_TYPE: u32 = 7;
    pub const GET_INPUT_DISPATCH_TYPE: u32 = 8;
    pub const SEND_CURSOR_POSITION: u32 = 9;
    pub const SEND_CURSOR_STANDBY_TIME: u32 = 10;
    pub const STOP_WEBOS_TASK: u32 = 11;
    pub const SET_DEACTIVATED_DEVICE_TYPE: u32 = 12;
    pub const ALLOW_TO_SET_DISPATCH_TYPE: u32 = 13;
    pub const SEND_MRCU_CURSOR_ON: u32 = 14;

    pub fn name(kind: u32) -> &'static str {
        match kind {
            REGISTER_INPUTDEVICE_MQ => "REGISTER_INPUTDEVICE_MQ",
            REQUEST_TO_CHECK_INPUT => "REQUEST_TO_CHECK_INPUT",
            SEND_CURSOR_VISIBILITY => "SEND_CURSOR_VISIBILITY",
            SEND_CURSOR_SUPPORT => "SEND_CURSOR_SUPPORT",
            SEND_CURSOR_SPEED => "SEND_CURSOR_SPEED",
            REGISTER_OPENAPI_MQ => "REGISTER_OPENAPI_MQ",
            DEREGISTER_OPENAPI_MQ => "DEREGISTER_OPENAPI_MQ",
            SET_INPUT_DISPATCH_TYPE => "SET_INPUT_DISPATCH_TYPE",
            GET_INPUT_DISPATCH_TYPE => "GET_INPUT_DISPATCH_TYPE",
            SEND_CURSOR_POSITION => "SEND_CURSOR_POSITION",
            SEND_CURSOR_STANDBY_TIME => "SEND_CURSOR_STANDBY_TIME",
            STOP_WEBOS_TASK => "STOP_WEBOS_TASK",
            SET_DEACTIVATED_DEVICE_TYPE => "SET_DEACTIVATED_DEVICE_TYPE",
            ALLOW_TO_SET_DISPATCH_TYPE => "ALLOW_TO_SET_DISPATCH_TYPE",
            SEND_MRCU_CURSOR_ON => "SEND_MRCU_CURSOR_ON",
            _ => "unknown",
        }
    }
}

/// `IM_INPUT_DISPATCH_ALL`: keys and the pointer, LG's default (`_gInputDispatchType`).
pub const DISPATCH_ALL: u32 = 3;
/// The cursor is supported (`_gSupportCursor`, LG's default): lginput2 keeps the remote's pointer on.
pub const SUPPORT_CURSOR: u32 = 1;
/// The speed lginput2 does not apply (`mrcu_state_registerCursorCallback` skips 3): the motion library keeps its own.
pub const SPEED_KEEP: u32 = 3;
/// The visibility words: `IM_INPUT_SetCursorVisibility`'s states.
pub const VISIBLE: u32 = 0;
pub const HIDDEN: u32 = 1;

/// The compositor's side of the pointer, as the registration answers it and the notifications report it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorInfo {
    pub visible: bool,
    /// After how long a still pointer hides (`seat::CURSOR_TIMEOUT`), what LG's `IM_SetMrcuStandbytimer` carries.
    pub standby_ms: u32,
}

/// One request off `/IM_MQ_WEBOS`: `{type, pid, argument}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub kind: u32,
    pub pid: u32,
    pub arg: u32,
}

/// The request in `bytes` (a message of at least three words); `None` for anything shorter.
pub fn parse(bytes: &[u8]) -> Option<Request> {
    if bytes.len() < 12 {
        return None;
    }
    let w = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    Some(Request { kind: w(0), pid: w(4), arg: w(8) })
}

/// A message of `words` followed by zeros, as libim builds them (`memset` then the words).
pub fn message(words: &[u32]) -> [u8; MSG_LEN] {
    let mut m = [0u8; MSG_LEN];
    for (i, w) in words.iter().enumerate() {
        m[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    m
}

/// Where the answer to a request goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// On the caller's `/mq_IMOpenAPI_<pid>`.
    OpenApi(u32, [u8; MSG_LEN]),
    /// On `/IM_MQ_MRCU`.
    Mrcu([u8; MSG_LEN]),
    None,
}

/// What libim's handler sends back for `req`: `{type, 1}` for the open API calls (`1` is the success the caller checks), the dispatch type
/// after it, and the cursor info after the registration.
pub fn answer(req: &Request, cursor: CursorInfo, dispatch_type: u32) -> Answer {
    match req.kind {
        msg::REGISTER_INPUTDEVICE_MQ => {
            Answer::Mrcu(message(&[req.kind, 1, if cursor.visible { VISIBLE } else { HIDDEN }, SUPPORT_CURSOR, SPEED_KEEP, cursor.standby_ms]))
        }
        msg::REGISTER_OPENAPI_MQ | msg::SET_INPUT_DISPATCH_TYPE | msg::SET_DEACTIVATED_DEVICE_TYPE | msg::ALLOW_TO_SET_DISPATCH_TYPE => {
            Answer::OpenApi(req.pid, message(&[req.kind, 1]))
        }
        msg::GET_INPUT_DISPATCH_TYPE => Answer::OpenApi(req.pid, message(&[req.kind, 1, dispatch_type])),
        _ => Answer::None,
    }
}

/// The notification that the pointer showed or hid (`IM_INPUT_SendCursorVisibility`).
pub fn visibility_notice(visible: bool) -> [u8; MSG_LEN] {
    message(&[msg::SEND_CURSOR_VISIBILITY, if visible { VISIBLE } else { HIDDEN }])
}

/// The queues' names: LG's on the set, private ones in the host tests.
#[derive(Clone, Debug)]
pub struct Names {
    pub webos: String,
    pub mrcu: String,
    /// The open API queues' name before the pid.
    pub openapi: String,
}

impl Names {
    pub fn lg() -> Names {
        Names { webos: "/IM_MQ_WEBOS".into(), mrcu: "/IM_MQ_MRCU".into(), openapi: "/mq_IMOpenAPI_".into() }
    }

    pub fn openapi(&self, pid: u32) -> String {
        format!("{}{pid}", self.openapi)
    }
}

/// One POSIX message queue descriptor, non-blocking.
struct Mq {
    fd: libc::mqd_t,
    /// The queue's `mq_msgsize`: a receive needs a buffer at least this long.
    msgsize: usize,
}

impl Mq {
    /// Opens `name` with `oflag` (`O_NONBLOCK` added), making it with LG's attributes when `create` and it does not exist.
    fn open(name: &str, oflag: libc::c_int, create: bool) -> io::Result<Mq> {
        let cname = CString::new(name).map_err(|_| io::Error::other("queue name"))?;
        let oflag = oflag | libc::O_NONBLOCK;
        // SAFETY: a NUL-terminated name; without O_CREAT the remaining arguments are not read.
        let mut fd = unsafe { libc::mq_open(cname.as_ptr(), oflag) };
        if fd < 0 && create && io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
            for maxmsg in [LG_MAX_MSG, SMALL_MAX_MSG] {
                // SAFETY: mq_attr is plain data; only the two sizes are read at creation.
                let mut attr: libc::mq_attr = unsafe { std::mem::zeroed() };
                attr.mq_maxmsg = maxmsg;
                attr.mq_msgsize = MSG_LEN as libc::c_long;
                // SAFETY: with O_CREAT the mode and a valid attribute pointer follow.
                fd = unsafe { libc::mq_open(cname.as_ptr(), oflag | libc::O_CREAT, MODE, &attr as *const libc::mq_attr) };
                if fd >= 0 || io::Error::last_os_error().raw_os_error() != Some(libc::EINVAL) {
                    break;
                }
            }
        }
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut mq = Mq { fd, msgsize: MSG_LEN };
        // SAFETY: a live descriptor and a stack attribute struct.
        let mut attr: libc::mq_attr = unsafe { std::mem::zeroed() };
        if unsafe { libc::mq_getattr(fd, &mut attr) } < 0 {
            return Err(io::Error::last_os_error());
        }
        mq.msgsize = attr.mq_msgsize.max(MSG_LEN as libc::c_long) as usize;
        Ok(mq)
    }

    /// Messages waiting in the queue.
    fn waiting(&self) -> usize {
        // SAFETY: a live descriptor and a stack attribute struct.
        let mut attr: libc::mq_attr = unsafe { std::mem::zeroed() };
        if unsafe { libc::mq_getattr(self.fd, &mut attr) } < 0 { 0 } else { attr.mq_curmsgs.max(0) as usize }
    }

    /// One message into `buf` (at least `msgsize` long): its length, or `None` when the queue is empty.
    fn receive(&self, buf: &mut [u8]) -> io::Result<Option<usize>> {
        debug_assert!(buf.len() >= self.msgsize);
        // SAFETY: the buffer's length is passed with its pointer; no priority is wanted.
        let n = unsafe { libc::mq_receive(self.fd, buf.as_mut_ptr().cast(), buf.len(), std::ptr::null_mut()) };
        if n < 0 {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::WouldBlock { Ok(None) } else { Err(e) };
        }
        Ok(Some(n as usize))
    }

    /// Sends `msg` at priority 0 (libim's); `false` when the queue is full (nobody reads it).
    fn send(&self, msg: &[u8]) -> io::Result<bool> {
        // SAFETY: the message's length is passed with its pointer.
        if unsafe { libc::mq_send(self.fd, msg.as_ptr().cast(), msg.len(), 0) } < 0 {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::WouldBlock { Ok(false) } else { Err(e) };
        }
        Ok(true)
    }

    /// Reads and discards what waits, so that a stale answer is not read as the answer to the next call.
    fn purge(&self, buf: &mut [u8]) -> usize {
        let mut n = 0;
        while let Ok(Some(_)) = self.receive(buf) {
            n += 1;
        }
        n
    }
}

impl Drop for Mq {
    fn drop(&mut self) {
        // SAFETY: closing a descriptor this struct owns.
        unsafe { libc::mq_close(self.fd) };
    }
}

#[derive(Default, Clone, Copy, Debug)]
pub struct Stats {
    pub requests: u64,
    pub answered: u64,
    /// Answers that went nowhere: no queue to send on, or a full one.
    pub unanswered: u64,
    /// Messages found waiting when the compositor started (requests, and answers nobody read), read and discarded.
    pub stale: u64,
    /// Answers nobody read, discarded before a new one was sent on the same queue.
    pub purged: u64,
    /// Visibility notices sent.
    pub notices: u64,
    /// The pid whose registration was last answered.
    pub registered_pid: u32,
}

/// The reader of `/IM_MQ_WEBOS` and the writer of what lginput2 waits for.
pub struct ImQueue {
    names: Names,
    webos: Mq,
    buf: Vec<u8>,
    mrcu: Option<Mq>,
    /// The open API queues by pid, kept as libim keeps them (`interface_input_InsertSendMqId`).
    openapi: Vec<(u32, Mq)>,
    dispatch_type: u32,
    /// lginput2 reads `/IM_MQ_MRCU` (its reader thread is up): visibility notices are sent. True once a registration was answered, and from
    /// the start while the queue exists (lginput2 registered with LG's compositor at boot and never again in its life: libim keeps a flag), until
    /// a notice finds the queue full or gone.
    registered: bool,
    /// The visibility last told, so that a repeat is not sent.
    told_visible: Option<bool>,
    pub stats: Stats,
}

impl ImQueue {
    /// Opens LG's queue (making it when LG's compositor never did) and discards what waits in it and in `/IM_MQ_MRCU`: requests whose callers
    /// gave up, and answers nobody read, which the next caller would take for its own.
    pub fn open() -> io::Result<ImQueue> {
        Self::open_named(Names::lg())
    }

    pub fn open_named(names: Names) -> io::Result<ImQueue> {
        let webos = Mq::open(&names.webos, libc::O_RDWR, true)?;
        let mut buf = vec![0u8; webos.msgsize];
        let mut stale = webos.purge(&mut buf) as u64;
        let mrcu = Mq::open(&names.mrcu, libc::O_RDWR, false).ok();
        if let Some(q) = &mrcu {
            if q.msgsize > buf.len() {
                buf.resize(q.msgsize, 0);
            }
            stale += q.purge(&mut buf) as u64;
        }
        Ok(ImQueue {
            names,
            webos,
            buf,
            registered: mrcu.is_some(),
            mrcu,
            openapi: Vec::new(),
            dispatch_type: DISPATCH_ALL,
            told_visible: None,
            stats: Stats { stale, ..Stats::default() },
        })
    }

    /// The request queue's descriptor: readable while a request waits.
    pub fn fd(&self) -> libc::c_int {
        self.webos.fd
    }

    pub fn registered(&self) -> bool {
        self.registered
    }

    /// Reads every waiting request and answers it; `log` gets one line per request.
    pub fn poll(&mut self, cursor: CursorInfo, log: &mut dyn FnMut(String)) {
        loop {
            let n = match self.webos.receive(&mut self.buf) {
                Ok(Some(n)) => n,
                Ok(None) => return,
                Err(e) => {
                    log(format!("im: reading {}: {e}", self.names.webos));
                    return;
                }
            };
            let Some(req) = parse(&self.buf[..n]) else {
                log(format!("im: a {n}-byte message on {} is no request", self.names.webos));
                continue;
            };
            self.stats.requests += 1;
            self.handle(req, cursor, log);
        }
    }

    fn handle(&mut self, req: Request, cursor: CursorInfo, log: &mut dyn FnMut(String)) {
        let name = msg::name(req.kind);
        match req.kind {
            msg::SET_INPUT_DISPATCH_TYPE => self.dispatch_type = req.arg,
            msg::DEREGISTER_OPENAPI_MQ => self.openapi.retain(|(p, _)| *p != req.pid),
            _ => {}
        }
        match answer(&req, cursor, self.dispatch_type) {
            Answer::OpenApi(pid, m) => match self.openapi_queue(pid) {
                Ok(i) => {
                    let q = &self.openapi[i].1;
                    let purged = q.purge(&mut self.buf);
                    let sent = q.send(&m);
                    self.count(&sent, purged);
                    log(format!(
                        "im: {name} from pid {pid} (arg {}): answered {} on {}{}",
                        req.arg,
                        describe(&sent),
                        self.names.openapi(pid),
                        purged_note(purged)
                    ));
                }
                Err(e) => {
                    self.stats.unanswered += 1;
                    log(format!("im: {name} from pid {pid}: {} does not open ({e}); not answered", self.names.openapi(pid)));
                }
            },
            Answer::Mrcu(m) => {
                // Whatever waits in the queue now is nobody's: the caller's reader thread starts after this answer, so it would read what
                // waits before it as the answer (an old notice `{2, 1}` passes for success with support 0, which disables the pointer).
                let purged = match self.open_mrcu() {
                    Ok(()) => self.mrcu.as_ref().map_or(0, |q| q.purge(&mut self.buf)),
                    Err(_) => 0,
                };
                let sent = self.send_mrcu(&m);
                self.count(&sent, purged);
                if matches!(sent, Ok(true)) {
                    self.registered = true;
                    self.told_visible = Some(cursor.visible);
                    self.stats.registered_pid = req.pid;
                }
                log(format!(
                    "im: {name} from pid {}: cursor info (visibility {}, support {SUPPORT_CURSOR}, speed {SPEED_KEEP}, standby {} ms) {} on {}{}",
                    req.pid,
                    if cursor.visible { VISIBLE } else { HIDDEN },
                    cursor.standby_ms,
                    describe(&sent),
                    self.names.mrcu,
                    purged_note(purged)
                ));
            }
            Answer::None => log(match req.kind {
                msg::REQUEST_TO_CHECK_INPUT => format!("im: {name} from pid {}: lginput2 made or removed a device", req.pid),
                msg::DEREGISTER_OPENAPI_MQ => format!("im: {name} from pid {}: its queue is forgotten", req.pid),
                msg::STOP_WEBOS_TASK => format!("im: {name} from pid {}: ignored", req.pid),
                _ => format!("im: {name} ({}) from pid {} (arg {}): not a request to the compositor; ignored", req.kind, req.pid, req.arg),
            }),
        }
    }

    fn count(&mut self, sent: &io::Result<bool>, purged: usize) {
        match sent {
            Ok(true) => self.stats.answered += 1,
            _ => self.stats.unanswered += 1,
        }
        self.stats.purged += purged as u64;
    }

    /// The pointer showed or hid: lginput2 hears of it while it reads its queue (`registered`).
    pub fn cursor_visibility(&mut self, visible: bool, log: &mut dyn FnMut(String)) {
        if !self.registered || self.told_visible == Some(visible) {
            return;
        }
        self.told_visible = Some(visible);
        match self.send_mrcu(&visibility_notice(visible)) {
            Ok(true) => self.stats.notices += 1,
            sent => {
                // Nobody reads the queue: lginput2's reader thread is gone with it. The next registration starts the notices again.
                self.registered = false;
                log(format!("im: the cursor notice {} on {}: no more until lginput2 registers again", describe(&sent), self.names.mrcu));
            }
        }
    }

    fn send_mrcu(&mut self, m: &[u8; MSG_LEN]) -> io::Result<bool> {
        let r = self.open_mrcu().and_then(|()| self.mrcu.as_ref().map_or(Ok(false), |q| q.send(m)));
        if r.is_err() {
            self.mrcu = None;
        }
        r
    }

    /// Opens `/IM_MQ_MRCU` for reading and writing (a purge reads it) at the first need; it is kept.
    fn open_mrcu(&mut self) -> io::Result<()> {
        if self.mrcu.is_none() {
            let q = Mq::open(&self.names.mrcu, libc::O_RDWR, true)?;
            if q.msgsize > self.buf.len() {
                self.buf.resize(q.msgsize, 0);
            }
            self.mrcu = Some(q);
        }
        Ok(())
    }

    /// The index of `pid`'s open API queue in `openapi`, opened at the first answer to it (the caller made it before asking). Queues of
    /// callers that are gone are dropped first (libim's `interface_input_CheckPIDValidation`).
    fn openapi_queue(&mut self, pid: u32) -> io::Result<usize> {
        if let Some(i) = self.openapi.iter().position(|(p, _)| *p == pid) {
            return Ok(i);
        }
        self.openapi.retain(|(p, _)| alive(*p));
        let q = Mq::open(&self.names.openapi(pid), libc::O_RDWR, true)?;
        if q.msgsize > self.buf.len() {
            self.buf.resize(q.msgsize, 0);
        }
        self.openapi.push((pid, q));
        Ok(self.openapi.len() - 1)
    }

    /// For `ctl status`.
    pub fn report(&self) -> Value {
        json!({
            "queue": self.names.webos,
            "waiting": self.webos.waiting(),
            "registered": self.registered,
            "registeredPid": self.stats.registered_pid,
            "requests": self.stats.requests,
            "answered": self.stats.answered,
            "unanswered": self.stats.unanswered,
            "stale": self.stats.stale,
            "purged": self.stats.purged,
            "notices": self.stats.notices,
            "dispatchType": self.dispatch_type,
        })
    }
}

fn describe(sent: &io::Result<bool>) -> String {
    match sent {
        Ok(true) => "sent".into(),
        Ok(false) => "dropped (the queue is full, nobody reads it)".into(),
        Err(e) => format!("not sent ({e})"),
    }
}

fn purged_note(purged: usize) -> String {
    if purged > 0 { format!(", {purged} stale answer(s) discarded first") } else { String::new() }
}

/// Whether a process with `pid` exists (`kill(pid, 0)`: anything but ESRCH says it does).
fn alive(pid: u32) -> bool {
    // SAFETY: signal 0 delivers nothing; it only checks for the process.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 || io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURSOR: CursorInfo = CursorInfo { visible: false, standby_ms: 10_000 };

    /// `IM_MQ_Connect`'s request as lginput2 (pid 3737) sends it: word 0 the type, word 1 the pid, 76 zero bytes.
    fn captured(kind: u32, pid: u32) -> [u8; MSG_LEN] {
        let mut m = [0u8; MSG_LEN];
        m[0..4].copy_from_slice(&kind.to_le_bytes());
        m[4..8].copy_from_slice(&pid.to_le_bytes());
        m
    }

    #[test]
    fn the_connect_request_is_answered_with_ok_on_the_callers_queue() {
        let req = parse(&captured(5, 3737)).unwrap();
        assert_eq!(req, Request { kind: msg::REGISTER_OPENAPI_MQ, pid: 3737, arg: 0 });
        let Answer::OpenApi(pid, m) = answer(&req, CURSOR, DISPATCH_ALL) else { panic!() };
        assert_eq!(pid, 3737);
        assert_eq!(m.len(), MSG_LEN);
        assert_eq!(&m[..12], &[5, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
        assert!(m[12..].iter().all(|b| *b == 0));
    }

    #[test]
    fn the_dispatch_type_is_lgs_default_until_a_caller_sets_it() {
        let req = parse(&captured(8, 3737)).unwrap();
        assert_eq!(answer(&req, CURSOR, DISPATCH_ALL), Answer::OpenApi(3737, message(&[8, 1, 3])));
        assert_eq!(answer(&req, CURSOR, 4), Answer::OpenApi(3737, message(&[8, 1, 4])));
        let set = parse(&message(&[7, 3737, 4])).unwrap();
        assert_eq!(set.arg, 4);
        assert_eq!(answer(&set, CURSOR, DISPATCH_ALL), Answer::OpenApi(3737, message(&[7, 1])));
    }

    #[test]
    fn the_registration_is_answered_on_the_mrcu_queue_with_the_cursor_info() {
        let req = parse(&captured(0, 3737)).unwrap();
        assert_eq!(answer(&req, CURSOR, DISPATCH_ALL), Answer::Mrcu(message(&[0, 1, HIDDEN, 1, 3, 10_000])));
        let shown = CursorInfo { visible: true, ..CURSOR };
        assert_eq!(answer(&req, shown, DISPATCH_ALL), Answer::Mrcu(message(&[0, 1, VISIBLE, 1, 3, 10_000])));
    }

    #[test]
    fn notifications_and_the_rest_get_no_answer() {
        for kind in [1, 2, 3, 4, 6, 9, 10, 11, 14, 99] {
            let req = parse(&captured(kind, 1)).unwrap();
            assert_eq!(answer(&req, CURSOR, DISPATCH_ALL), Answer::None, "type {kind}");
        }
        for kind in [12, 13] {
            let req = parse(&captured(kind, 1)).unwrap();
            assert_eq!(answer(&req, CURSOR, DISPATCH_ALL), Answer::OpenApi(1, message(&[kind, 1])));
        }
        assert!(parse(&[0; 8]).is_none());
    }

    #[test]
    fn the_visibility_notice_is_lgs_show_and_hide_states() {
        assert_eq!(&visibility_notice(true)[..8], &[2, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&visibility_notice(false)[..8], &[2, 0, 0, 0, 1, 0, 0, 0]);
    }
}
