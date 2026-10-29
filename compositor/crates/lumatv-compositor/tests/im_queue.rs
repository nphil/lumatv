//! LG's Input Manager queue (`im_queue`): a stand-in lginput2 makes the calls libim makes at a reconnect of the Magic Remote, over real
//! POSIX message queues with private names, and gets the answers LG's compositor gave: at once, on the right queue, with the cursor's state.
//! Then the compositor's pointer: told it shows, the wheel's press arrives as BTN_LEFT and is the click; the notices follow the pointer.

mod harness;

use std::ffi::CString;
use std::io;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use harness::{Client, Server};
use lumatv_compositor::im_queue::{CursorInfo, HIDDEN, ImQueue, MSG_LEN, Names, SPEED_KEEP, SUPPORT_CURSOR, VISIBLE, message, msg};
use lumatv_compositor::server::seat::{BTN_LEFT, remote_motion, wheel_turn};

const WAIT: Duration = Duration::from_secs(3);
/// libim waits this long for an answer (`IM_CallMethodSyncWithMq`); the fix is that none takes a measurable part of it.
const LIBIM_TIMEOUT: Duration = Duration::from_secs(2);
const CURSOR: CursorInfo = CursorInfo { visible: false, standby_ms: 10_000 };

static NEXT: AtomicU32 = AtomicU32::new(0);

/// Private names for one test, so that tests run beside each other and beside anything the host has.
fn names() -> Names {
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let base = format!("/lumatv-im-test-{}-{n}", std::process::id());
    Names { webos: format!("{base}-webos"), mrcu: format!("{base}-mrcu"), openapi: format!("{base}-openapi-") }
}

/// What libim does with a queue, on lginput2's side.
struct StandIn(libc::mqd_t);

impl StandIn {
    /// `IM_MQ_Open` with `O_CREAT | O_EXCL` as `IM_MQ_Connect` and `_IM_GetDeviceMqFd` make their queues; 10 messages where the host refuses 128.
    fn create(name: &str) -> StandIn {
        let cname = CString::new(name).unwrap();
        for maxmsg in [128, 10] {
            // SAFETY: mq_attr is plain data; the name is NUL-terminated.
            let mut attr: libc::mq_attr = unsafe { std::mem::zeroed() };
            attr.mq_maxmsg = maxmsg;
            attr.mq_msgsize = MSG_LEN as libc::c_long;
            let fd =
                unsafe { libc::mq_open(cname.as_ptr(), libc::O_RDWR | libc::O_CREAT | libc::O_EXCL, 0o777 as libc::c_uint, &attr as *const libc::mq_attr) };
            if fd >= 0 {
                return StandIn(fd);
            }
            assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::EINVAL), "{name}: {}", io::Error::last_os_error());
        }
        panic!("{name} does not open");
    }

    /// `O_RDWR` without creation, as libim opens `/IM_MQ_WEBOS`.
    fn open(name: &str) -> StandIn {
        let cname = CString::new(name).unwrap();
        // SAFETY: a NUL-terminated name; without O_CREAT nothing else is read.
        let fd = unsafe { libc::mq_open(cname.as_ptr(), libc::O_RDWR) };
        assert!(fd >= 0, "{name}: {}", io::Error::last_os_error());
        StandIn(fd)
    }

    /// `IM_SendMsgWithMq`: 84 bytes at priority 0, with libim's 2 s timeout.
    fn send(&self, m: &[u8; MSG_LEN]) -> bool {
        let abs = deadline(LIBIM_TIMEOUT);
        // SAFETY: the message's length goes with its pointer, the timeout is a stack struct.
        unsafe { libc::mq_timedsend(self.0, m.as_ptr().cast(), m.len(), 0, &abs) == 0 }
    }

    /// `IM_ReceiveMsgWithMq`: one message within libim's 2 s, or `None` as the timeout libim logged.
    fn receive(&self) -> Option<[u8; MSG_LEN]> {
        let mut buf = [0u8; 8192];
        let abs = deadline(LIBIM_TIMEOUT);
        // SAFETY: the buffer's length goes with its pointer; no priority is wanted.
        let n = unsafe { libc::mq_timedreceive(self.0, buf.as_mut_ptr().cast(), buf.len(), std::ptr::null_mut(), &abs) };
        if n < 0 {
            return None;
        }
        assert_eq!(n as usize, MSG_LEN, "every message is 84 bytes");
        let mut m = [0u8; MSG_LEN];
        m.copy_from_slice(&buf[..MSG_LEN]);
        Some(m)
    }

    fn waiting(&self) -> i64 {
        // SAFETY: a live descriptor and a stack attribute struct.
        let mut attr: libc::mq_attr = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { libc::mq_getattr(self.0, &mut attr) }, 0);
        attr.mq_curmsgs as i64
    }
}

impl Drop for StandIn {
    fn drop(&mut self) {
        // SAFETY: closing a descriptor this struct owns.
        unsafe { libc::mq_close(self.0) };
    }
}

fn deadline(after: Duration) -> libc::timespec {
    // SAFETY: a stack timespec filled by the kernel.
    let mut now: libc::timespec = unsafe { std::mem::zeroed() };
    unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut now) };
    let ns = now.tv_nsec as u64 + after.subsec_nanos() as u64;
    libc::timespec { tv_sec: now.tv_sec + after.as_secs() as i64 + (ns / 1_000_000_000) as i64, tv_nsec: (ns % 1_000_000_000) as i64 }
}

fn unlink(name: &str) {
    let cname = CString::new(name).unwrap();
    // SAFETY: a NUL-terminated name; a missing queue is fine.
    unsafe { libc::mq_unlink(cname.as_ptr()) };
}

/// Unlinks every queue of `names` and the open API queue of `pid`, when the test is over.
struct Cleanup(Names, u32);

impl Drop for Cleanup {
    fn drop(&mut self) {
        unlink(&self.0.webos);
        unlink(&self.0.mrcu);
        unlink(&self.0.openapi(self.1));
    }
}

/// The compositor's side: answers what waits, as the loop does when the descriptor polls readable.
fn serve(im: &mut ImQueue, cursor: CursorInfo) -> Vec<String> {
    let mut p = libc::pollfd { fd: im.fd(), events: libc::POLLIN, revents: 0 };
    // SAFETY: one pollfd on the queue's descriptor.
    assert_eq!(unsafe { libc::poll(&mut p, 1, 1000) }, 1, "a request waits");
    let mut lines = Vec::new();
    im.poll(cursor, &mut |l| lines.push(l));
    lines
}

fn words(m: &[u8; MSG_LEN]) -> Vec<u32> {
    m.chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

/// `IM_MQ_Connect`'s and `IM_RegisterCursorInfoCallback`'s requests as lginput2 (pid 3737) sent them on the set: the type, the pid, zeros.
fn captured(kind: u32, pid: u32) -> [u8; MSG_LEN] {
    message(&[kind, pid])
}

#[test]
fn the_reconnects_three_calls_are_answered_at_once_on_their_queues_and_the_notices_follow() {
    let names = names();
    let pid = std::process::id();
    let _cleanup = Cleanup(names.clone(), pid);
    let mut im = ImQueue::open_named(names.clone()).unwrap();
    assert_eq!(im.stats.stale, 0);
    assert!(!im.registered());

    // IM_GetInputDispatchType: IM_MQ_Connect makes the caller's queue, sends REGISTER_OPENAPI_MQ and waits for {5, 1} on it ...
    let webos = StandIn::open(&names.webos);
    let openapi = StandIn::create(&names.openapi(pid));
    assert!(webos.send(&captured(msg::REGISTER_OPENAPI_MQ, pid)));
    let t0 = Instant::now();
    let lines = serve(&mut im, CURSOR);
    let reply = openapi.receive().expect("the connect is answered");
    assert!(t0.elapsed() < Duration::from_millis(200), "answered in {:?}, not after libim's 2 s", t0.elapsed());
    assert_eq!(&words(&reply)[..3], &[msg::REGISTER_OPENAPI_MQ, 1, 0]);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("REGISTER_OPENAPI_MQ") && lines[0].contains("sent"), "{lines:?}");

    // ... then sends GET_INPUT_DISPATCH_TYPE and reads the type after the success word: 3, keys and the pointer.
    assert!(webos.send(&captured(msg::GET_INPUT_DISPATCH_TYPE, pid)));
    serve(&mut im, CURSOR);
    let reply = openapi.receive().expect("the dispatch type is answered");
    assert_eq!(&words(&reply)[..4], &[msg::GET_INPUT_DISPATCH_TYPE, 1, 3, 0]);

    // IM_RegisterCursorInfoCallback: lginput2 makes /IM_MQ_MRCU, sends REGISTER_INPUTDEVICE_MQ and reads the cursor info there.
    let mrcu = StandIn::create(&names.mrcu);
    assert!(webos.send(&captured(msg::REGISTER_INPUTDEVICE_MQ, pid)));
    let t0 = Instant::now();
    serve(&mut im, CURSOR);
    let reply = mrcu.receive().expect("the registration is answered");
    assert!(t0.elapsed() < Duration::from_millis(200), "{:?}", t0.elapsed());
    assert_eq!(&words(&reply)[..7], &[msg::REGISTER_INPUTDEVICE_MQ, 1, HIDDEN, SUPPORT_CURSOR, SPEED_KEEP, 10_000, 0]);
    assert!(im.registered());
    assert_eq!(im.stats.registered_pid, pid);
    assert_eq!((im.stats.requests, im.stats.answered, im.stats.unanswered), (3, 3, 0));
    assert_eq!(openapi.waiting(), 0, "nothing left unread on the caller's queue");

    // Registered, lginput2 hears each change of the pointer's visibility and nothing for a repeat.
    let mut lines = Vec::new();
    im.cursor_visibility(true, &mut |l| lines.push(l));
    im.cursor_visibility(true, &mut |l| lines.push(l));
    im.cursor_visibility(false, &mut |l| lines.push(l));
    assert!(lines.is_empty(), "{lines:?}");
    assert_eq!(&words(&mrcu.receive().unwrap())[..2], &[msg::SEND_CURSOR_VISIBILITY, VISIBLE]);
    assert_eq!(&words(&mrcu.receive().unwrap())[..2], &[msg::SEND_CURSOR_VISIBILITY, HIDDEN]);
    assert_eq!(mrcu.waiting(), 0, "the repeat was not sent");
    assert_eq!(im.stats.notices, 2);

    // A later registration from the same lginput2 (its next reconnect, were libim to send one) is answered with the visibility of the moment.
    assert!(webos.send(&captured(msg::REGISTER_INPUTDEVICE_MQ, pid)));
    serve(&mut im, CursorInfo { visible: true, ..CURSOR });
    assert_eq!(&words(&mrcu.receive().unwrap())[..3], &[msg::REGISTER_INPUTDEVICE_MQ, 1, VISIBLE]);
    let report = im.report();
    assert_eq!(report["registered"], true);
    assert_eq!(report["requests"], 4);
}

#[test]
fn messages_waiting_from_before_the_start_are_read_and_discarded() {
    let names = names();
    let pid = std::process::id();
    let _cleanup = Cleanup(names.clone(), pid);
    // The queues outlive LG's compositor; the 30 requests of round 19 sat in one. Here: two reconnects' worth, from before the compositor
    // started, and a notice LG's compositor sent that lginput2 never read.
    let maker = ImQueue::open_named(names.clone()).unwrap();
    assert!(!maker.registered(), "no queue of lginput2's yet");
    drop(maker);
    let webos = StandIn::open(&names.webos);
    let openapi = StandIn::create(&names.openapi(pid));
    let mrcu = StandIn::create(&names.mrcu);
    for _ in 0..2 {
        assert!(webos.send(&captured(msg::REGISTER_OPENAPI_MQ, pid)));
        assert!(webos.send(&captured(msg::REGISTER_INPUTDEVICE_MQ, pid)));
    }
    assert!(mrcu.send(&message(&[msg::SEND_CURSOR_VISIBILITY, HIDDEN])));
    assert_eq!((webos.waiting(), mrcu.waiting()), (4, 1));
    let mut im = ImQueue::open_named(names.clone()).unwrap();
    assert_eq!(im.stats.stale, 5);
    assert_eq!((webos.waiting(), mrcu.waiting()), (0, 0));
    assert_eq!(openapi.waiting(), 0, "an answer now would be read as the answer to the next call");
    assert_eq!(im.report()["stale"], 5);
    // lginput2's queue exists: it registered with LG's compositor at boot and keeps reading, so it is told where the pointer stands.
    assert!(im.registered());
    im.cursor_visibility(true, &mut |_| {});
    assert_eq!(&words(&mrcu.receive().unwrap())[..2], &[msg::SEND_CURSOR_VISIBILITY, VISIBLE]);
}

#[test]
fn a_stale_answer_on_the_callers_queue_goes_before_the_new_one_and_a_full_queue_drops_without_blocking() {
    let names = names();
    let pid = std::process::id();
    let _cleanup = Cleanup(names.clone(), pid);
    let mut im = ImQueue::open_named(names.clone()).unwrap();
    let webos = StandIn::open(&names.webos);
    let openapi = StandIn::create(&names.openapi(pid));
    // An answer the caller never read (its call timed out 2 s after the request, while the compositor was away) must not be what its next
    // call reads: GET_INPUT_DISPATCH_TYPE would take the connect's `{5, 1, 0}` as "type 0", which puts the remote's state machine in its
    // no-pointer state.
    assert!(openapi.send(&message(&[msg::REGISTER_OPENAPI_MQ, 1])));
    assert!(webos.send(&captured(msg::GET_INPUT_DISPATCH_TYPE, pid)));
    let lines = serve(&mut im, CURSOR);
    assert!(lines[0].contains("1 stale answer(s) discarded first"), "{lines:?}");
    assert_eq!(&words(&openapi.receive().unwrap())[..3], &[msg::GET_INPUT_DISPATCH_TYPE, 1, 3]);
    assert_eq!(openapi.waiting(), 0);

    // lginput2 registered, then stopped reading (its thread is gone): the notices fill its queue and the loop is never held by one.
    let mrcu = StandIn::create(&names.mrcu);
    assert!(webos.send(&captured(msg::REGISTER_INPUTDEVICE_MQ, pid)));
    serve(&mut im, CURSOR);
    assert!(im.registered());
    let t0 = Instant::now();
    let mut lines = Vec::new();
    for i in 0..200 {
        im.cursor_visibility(i % 2 == 0, &mut |l| lines.push(l));
    }
    assert!(t0.elapsed() < Duration::from_millis(500), "{:?}", t0.elapsed());
    assert!(!im.registered(), "a full queue means nobody reads it");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("dropped") && lines[0].contains("registers again"), "{lines:?}");
    assert!(mrcu.waiting() >= 2 && mrcu.waiting() <= 128, "{}", mrcu.waiting());
    // lginput2 registers again (it started again): what its queue holds is discarded before the answer, which it reads first. An old notice
    // `{2, 1}` read as the answer would pass for success with support 0, and the pointer would be off.
    let unread = mrcu.waiting();
    assert!(webos.send(&captured(msg::REGISTER_INPUTDEVICE_MQ, pid)));
    let lines = serve(&mut im, CURSOR);
    assert!(lines[0].contains(&format!("{unread} stale answer(s) discarded first")), "{lines:?}");
    assert!(im.registered());
    assert_eq!(&words(&mrcu.receive().unwrap())[..2], &[msg::REGISTER_INPUTDEVICE_MQ, 1]);
    assert_eq!(mrcu.waiting(), 0);
    assert_eq!(im.stats.purged, unread as u64 + 1);
    im.cursor_visibility(true, &mut |_| {});
    assert_eq!(&words(&mrcu.receive().unwrap())[..2], &[msg::SEND_CURSOR_VISIBILITY, VISIBLE]);

    // What needs no answer is read, counted and logged, and nothing is sent anywhere for it.
    for kind in [msg::REQUEST_TO_CHECK_INPUT, msg::DEREGISTER_OPENAPI_MQ, msg::SEND_CURSOR_POSITION, msg::STOP_WEBOS_TASK] {
        assert!(webos.send(&captured(kind, pid)));
    }
    let lines = serve(&mut im, CURSOR);
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(lines[0].contains("made or removed a device"), "{lines:?}");
    assert_eq!(webos.waiting(), 0);
    assert_eq!(openapi.waiting(), 0);
    assert_eq!(mrcu.waiting(), 0);
}

/// A client with a keyboard, a pointer and a card at the origin that has been drawn once (as `pointer_wheel.rs` has it).
fn pointed(server: &mut Server) -> (Client, Box<dyn std::any::Any>) {
    let mut client = Client::connect(server);
    let kb = client.seat.get_keyboard(&client.qh, ());
    let ptr = client.seat.get_pointer(&client.qh, ());
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "Pointed");
    let buf = client.shm_buffer(64, 64, 0xffff_0000);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.keyboard_enters >= 1 && s.frames_done >= 1));
    (client, Box::new((win, buf, kb, ptr)))
}

#[test]
fn the_compositor_tells_lginput2_of_the_pointer_and_takes_btn_left_as_the_wheels_press() {
    let names = names();
    let pid = std::process::id();
    let _cleanup = Cleanup(names.clone(), pid);
    let mut server = Server::start();
    let (mut client, _keep) = pointed(&mut server);
    let n = names.clone();
    server.eval(move |s| s.im = Some(ImQueue::open_named(n).unwrap()));
    let webos = StandIn::open(&names.webos);
    let mrcu = StandIn::create(&names.mrcu);
    assert!(webos.send(&captured(msg::REGISTER_INPUTDEVICE_MQ, pid)));
    server.eval(|s| s.im_poll());
    let reply = mrcu.receive().expect("the registration is answered from the compositor's state");
    assert_eq!(&words(&reply)[..6], &[0, 1, HIDDEN, SUPPORT_CURSOR, SPEED_KEEP, 10_000], "the pointer is hidden and hides after 10 s");

    // Hidden, the wheel's press is the key: lginput2 sends KEY_ENTER then, and the app hears it as such.
    server.eval(|s| remote_motion(s, 10, 20));
    server.eval(|s| {
        s.key_event(lumatv_compositor::server::seat::SeatId(0), 28, true, false);
        s.key_event(lumatv_compositor::server::seat::SeatId(0), 28, false, false);
    });
    assert!(client.wait_until(WAIT, |s| s.keys.len() >= 2));
    assert_eq!(client.state.keys, vec![(28, 1), (28, 0)]);
    assert_eq!(mrcu.waiting(), 0, "nothing changed for lginput2");

    // The wheel shows the pointer: lginput2 is told (and from then on sends the wheel's press as BTN_LEFT).
    assert!(server.eval(wheel_turn));
    assert!(client.wait_until(WAIT, |s| !s.pointer_enters.is_empty()));
    assert_eq!(&words(&mrcu.receive().expect("shown is told"))[..2], &[msg::SEND_CURSOR_VISIBILITY, VISIBLE]);
    server.eval(move |s| {
        s.key_event(lumatv_compositor::server::seat::SeatId(0), BTN_LEFT, true, false);
        s.key_event(lumatv_compositor::server::seat::SeatId(0), BTN_LEFT, false, false);
    });
    assert!(client.wait_until(WAIT, |s| s.buttons.len() >= 2));
    assert_eq!(client.state.buttons, vec![(0x110, 1), (0x110, 0)], "the click on what the pointer points at");
    assert_eq!(client.state.keys.len(), 2, "no key for it");

    // An arrow hides the pointer: lginput2 is told; a BTN_LEFT that still arrives after that (lginput2 had not read the notice) is OK, the key.
    server.eval(|s| {
        s.key_event(lumatv_compositor::server::seat::SeatId(0), 106, true, false);
        s.key_event(lumatv_compositor::server::seat::SeatId(0), 106, false, false);
    });
    assert!(client.wait_until(WAIT, |s| s.keys.len() >= 4));
    assert_eq!(&words(&mrcu.receive().expect("hidden is told"))[..2], &[msg::SEND_CURSOR_VISIBILITY, HIDDEN]);
    server.eval(move |s| {
        s.key_event(lumatv_compositor::server::seat::SeatId(0), BTN_LEFT, true, false);
        s.key_event(lumatv_compositor::server::seat::SeatId(0), BTN_LEFT, false, false);
    });
    assert!(client.wait_until(WAIT, |s| s.keys.len() >= 6));
    assert_eq!(&client.state.keys[4..], &[(28, 1), (28, 0)]);
    assert_eq!(client.state.buttons.len(), 2, "no click without a pointer");
    let status = server.eval(|s| lumatv_compositor::control::handle(s, "status").text());
    let status: serde_json::Value = serde_json::from_str(&status).unwrap();
    assert_eq!(status["im"]["registered"], true);
    assert_eq!(status["im"]["notices"], 2);
}
