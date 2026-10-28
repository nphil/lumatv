//! The Bluetooth proxy's runner: the TV as a Home Assistant Bluetooth proxy for presence and BLE sensors.
//!
//! One thread. It owns the pieces the others are pure about: the LAN socket Home Assistant connects to ([`ble_proxy`]'s protocol),
//! the mDNS announcement that lets Home Assistant find it, the radio ([`ble_scan`]: LG's `le/startScan` through `luna-send`) and
//! the duty machine ([`ble_duty`]) that decides when the radio may be asked. Advertisements go from LG's service through the
//! proxy to Home Assistant; nothing is stored and no address heard is logged or published.
//!
//! **Off is the default and off means nothing**: with no link file (`lumatv-helper ble-link`, run by the TV's owner) there is no
//! socket, no announcement and no scan, and the thread only looks for the file every ten seconds. With a link:
//!
//! - the socket exists only once LG's Bluetooth service has answered (the adapter's address is part of what Home Assistant is
//!   told), binds the TV's own private address (never `0.0.0.0`), serves peers in private networks only and keeps at most four;
//!   the address is checked every minute, as a DHCP change moves it;
//! - the radio is asked only while Home Assistant has subscribed to advertisements, and then in windows of the link's length with
//!   the rests the duty machine owes (see `ble_duty` for the limits and the guards: panel dark, Bluetooth audio up, adapter off);
//! - the scan child is killed at the end of every window, on every failure, when nobody is subscribed, when the link is removed,
//!   and when the helper stops.
//!
//! The hub topic `ble` says where it stands (`state`, how many are connected, how many advertisements went through, the last
//! problem) and never what was heard.

use crate::ble_duty::{Duty, HOLD_RADIO, HOLD_SCREEN, Phase};
use crate::ble_proxy::{ConnId, Identity, Proxy, Random, ScanState};
use crate::ble_scan::{Bluetooth, Event, LunaBluetooth, Wake};
use crate::ble_store::{self, BleLink};
use crate::ctx::Ctx;
use crate::screen;
use crate::sys;
use esphome_api::message::RawAdvertisement;
use mdns::{Mdns, Service};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The hub topic.
pub const TOPIC: &str = "ble";
const SERVICE: &str = "_esphomelib._tcp";
const ENCRYPTION: &str = "Noise_NNpsk0_25519_ChaChaPoly_SHA256";
/// The most a client's unwritten output may grow to before the client is dropped (a Home Assistant that stopped reading).
const MAX_PENDING: usize = 1024 * 1024;

/// How often the runner looks at its own state, apart from what happens.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    /// How often the link file is looked at (the owner's `ble-link` and `ble-unlink`).
    pub reload: Duration,
    /// How long before a listener that could not be made is tried again.
    pub bind_retry: Duration,
    /// How often a bound listener checks the TV still has its address.
    pub address_check: Duration,
    /// How long a scan may take to say it has started.
    pub subscribe_within: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            reload: Duration::from_secs(10),
            bind_retry: Duration::from_secs(15),
            address_check: Duration::from_secs(60),
            subscribe_within: Duration::from_secs(5),
        }
    }
}

/// The mDNS announcement of the node.
pub trait Announcer: Send {
    fn announce(&mut self, address: Ipv4Addr, service: Service) -> Result<(), String>;
    /// Withdraws it (goodbyes are sent) and closes the multicast socket.
    fn withdraw(&mut self);
}

/// The real announcer: LumaShow's responder on the multicast socket, which exists only while something is announced.
#[derive(Default)]
pub struct MdnsAnnouncer {
    mdns: Option<(Mdns, Ipv4Addr, String)>,
}

impl Announcer for MdnsAnnouncer {
    fn announce(&mut self, address: Ipv4Addr, service: Service) -> Result<(), String> {
        self.withdraw();
        if !service.is_valid() {
            return Err("the node's name is not a valid DNS name".to_owned());
        }
        let mdns = Mdns::start_udp(address).map_err(|e| format!("mDNS: {e}"))?;
        let instance = service.instance.clone();
        mdns.add(service);
        self.mdns = Some((mdns, address, instance));
        Ok(())
    }

    fn withdraw(&mut self) {
        if let Some((mdns, _, instance)) = self.mdns.take() {
            mdns.remove(&instance, SERVICE);
            // Dropping it ends the thread and closes the socket.
            drop(mdns);
        }
    }
}

impl Drop for MdnsAnnouncer {
    fn drop(&mut self) {
        self.withdraw();
    }
}

/// What the runner acts through.
pub struct Deps {
    pub bluetooth: Box<dyn Bluetooth>,
    pub announcer: Box<dyn Announcer>,
    /// Where to listen. `None` is the TV's private LAN address; a test names the loopback.
    pub bind: Option<Ipv4Addr>,

    pub timing: Timing,
}

impl Deps {
    /// The real ones: `luna-send`, the multicast responder, the kernel's random numbers.
    pub fn real(luna_send: &str) -> Self {
        Self {
            bluetooth: Box::new(LunaBluetooth::new(luna_send)),
            announcer: Box::new(MdnsAnnouncer::default()),
            bind: None,

            timing: Timing::default(),
        }
    }
}

/// A pipe whose write end wakes the thread out of `poll`.
struct WakePipe {
    read: OwnedFd,
    write: OwnedFd,
}

impl WakePipe {
    fn new() -> io::Result<Self> {
        let mut fds = [0 as libc::c_int; 2];
        // SAFETY: pipe2 into a two-element array.
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: descriptors pipe2 just made.
        Ok(unsafe {
            Self {
                read: OwnedFd::from_raw_fd(fds[0]),
                write: OwnedFd::from_raw_fd(fds[1]),
            }
        })
    }

    fn wake(&self) {
        // SAFETY: one byte to the pipe; a full pipe means it is already woken.
        unsafe { libc::write(self.write.as_raw_fd(), [1u8].as_ptr().cast(), 1) };
    }

    fn drain(&self) {
        let mut buf = [0u8; 64];
        // SAFETY: reads into a buffer of its size.
        unsafe { libc::read(self.read.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
    }
}

struct ClientIo {
    stream: TcpStream,
    /// Bytes the socket would not take yet.
    pending: Vec<u8>,
}

fn set_nonblocking(stream: &TcpStream) -> io::Result<()> {
    stream.set_nonblocking(true)?;
    stream.set_nodelay(true)?;
    // A Home Assistant that vanished (power, Wi-Fi) is noticed by the kernel in about 90 seconds, with no timer of ours.
    let fd = stream.as_raw_fd();
    for (level, name, value) in [
        (libc::SOL_SOCKET, libc::SO_KEEPALIVE, 1),
        (libc::IPPROTO_TCP, libc::TCP_KEEPIDLE, 60),
        (libc::IPPROTO_TCP, libc::TCP_KEEPINTVL, 10),
        (libc::IPPROTO_TCP, libc::TCP_KEEPCNT, 3),
    ] {
        // SAFETY: setsockopt on a live descriptor with an int.
        unsafe {
            libc::setsockopt(
                fd,
                level,
                name,
                (&raw const value).cast(),
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        }
    }
    Ok(())
}

/// The handshakes' ephemeral keys come from the kernel.
fn system_random() -> Random {
    Box::new(|buf: &mut [u8]| {
        if let Err(e) = sys::random_bytes(buf) {
            panic!("no random numbers: {e}");
        }
    })
}

fn pollfd(fd: RawFd, events: libc::c_short) -> libc::pollfd {
    libc::pollfd {
        fd,
        events,
        revents: 0,
    }
}

/// The hardware address of the interface that has `ip`, as `AA:BB:CC:DD:EE:FF` (`None` for loopback or an unreadable one).
pub(crate) fn interface_mac(ip: Ipv4Addr) -> Option<String> {
    let (name, _) = sys::ipv4_interfaces().into_iter().find(|(_, a)| *a == ip)?;
    let text = std::fs::read_to_string(format!("/sys/class/net/{name}/address")).ok()?;
    let text = text.trim().to_ascii_uppercase();
    let valid = text.len() == 17
        && text
            .split(':')
            .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()));
    (valid && text != "00:00:00:00:00:00").then_some(text)
}

/// What a scan window is doing.
struct ScanRun {
    started: Instant,
    /// LG's service has answered the call.
    subscribed: bool,
    heard: u64,
}

struct Bound {
    listener: TcpListener,
    addr: SocketAddr,
    ip: Ipv4Addr,
    checked: Instant,
}

/// Everything that exists only while a link is saved.
struct Active {
    link: BleLink,
    proxy: Proxy,
    duty: Duty,
    bound: Option<Bound>,
    bind_retry_at: Option<Instant>,
    clients: HashMap<ConnId, ClientIo>,
    scan: Option<ScanRun>,
    /// Why the radio is not being asked right now (a guard), and the last thing that went wrong.
    hold: Option<String>,
    error: Option<String>,
    last_window_heard: Option<u64>,
    windows_silent: u32,
}

struct Runner {
    ctx: Ctx,
    deps: Deps,
    wake: Arc<WakePipe>,
    active: Option<Active>,
    reload_at: Instant,
    published: Option<Value>,
    /// A test's clock: the time a turn is given is the time it has, before and after waiting.
    virtual_time: bool,
}

impl Runner {
    fn new(ctx: Ctx, deps: Deps, now: Instant) -> io::Result<Self> {
        let wake = Arc::new(WakePipe::new()?);
        {
            let wake = wake.clone();
            ctx.stop.on_stop(move || wake.wake());
        }
        Ok(Self {
            ctx,
            deps,
            wake,
            active: None,
            reload_at: now,
            published: None,
            virtual_time: false,
        })
    }

    fn scan_wake(&self) -> Wake {
        let wake = self.wake.clone();
        Arc::new(move || wake.wake())
    }

    // ---- the link ------------------------------------------------------------------------------------------------------------------

    /// Looks at the link file: a link that came, went or changed rebuilds everything that depends on it.
    fn reload(&mut self, now: Instant) {
        self.reload_at = now + self.deps.timing.reload;
        let link = match ble_store::load(&self.ctx.paths.data_dir) {
            Ok(link) => link,
            Err(e) => {
                if self.active.is_some() || self.published_error() != Some(e.to_string()) {
                    crate::log_warn!("ble: {e}");
                }
                self.teardown();
                self.set_off(Some(e.to_string()));
                return;
            }
        };
        match (link, self.active.as_ref().map(|a| &a.link)) {
            (None, None) => self.set_off(None),
            (None, Some(_)) => {
                crate::log_info!("ble: unlinked");
                self.teardown();
                self.set_off(None);
            }
            (Some(new), Some(old)) if new == *old => {}
            (Some(new), _) => {
                self.teardown();
                crate::log_info!(
                    "ble: linked as {} on port {} ({} s window, {} s rest, {}% of the time, standby scanning {})",
                    new.name,
                    new.port,
                    new.scan_seconds,
                    new.rest_seconds,
                    new.duty_percent(),
                    if new.standby { "on" } else { "off" }
                );
                let identity = Identity {
                    name: new.name.clone(),
                    friendly_name: new.friendly_name.clone(),
                    mac: String::new(),
                    bluetooth_mac: String::new(),
                };
                self.active = Some(Active {
                    proxy: Proxy::new(new.key, identity, system_random()),
                    duty: Duty::new(new.scan_seconds, new.rest_seconds),
                    link: new,
                    bound: None,
                    bind_retry_at: Some(now),
                    clients: HashMap::new(),
                    scan: None,
                    hold: None,
                    error: None,
                    last_window_heard: None,
                    windows_silent: 0,
                });
            }
        }
    }

    fn published_error(&self) -> Option<String> {
        self.published
            .as_ref()
            .and_then(|v| v.get("error"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    }

    /// Hands everything back: the scan ends, the announcement is withdrawn, the sockets close.
    fn teardown(&mut self) {
        self.deps.bluetooth.stop_scan();
        self.deps.announcer.withdraw();
        self.active = None;
    }

    fn set_off(&mut self, error: Option<String>) {
        let value = json!({
            "state": "off", "listening": null, "clients": 0, "subscribed": false,
            "advertisements": 0, "dropped": 0, "windows": 0, "error": error,
        });
        self.ctx.hub.set(TOPIC, value.clone());
        self.published = Some(value);
    }

    // ---- the socket and the announcement -------------------------------------------------------------------------------------------

    fn ensure_listener(&mut self, now: Instant) {
        let timing = self.deps.timing;
        let bind = self.deps.bind;
        let Some(a) = self.active.as_mut() else {
            return;
        };
        // The address a DHCP lease moved away from is not served any more: bind again.
        if let Some(b) = &a.bound
            && now >= b.checked + timing.address_check
        {
            if sys::lan_ipv4(bind) == Some(b.ip) {
                if let Some(b) = a.bound.as_mut() {
                    b.checked = now;
                }
            } else {
                crate::log_info!("ble: the TV's address changed; binding again");
                self.drop_listener();
                return self.ensure_listener(now);
            }
        }
        if a.bound.is_some() || a.bind_retry_at.is_some_and(|t| now < t) {
            return;
        }
        let retry = now + timing.bind_retry;
        let Some(ip) = sys::lan_ipv4(bind) else {
            a.bind_retry_at = Some(retry);
            a.error = Some("the TV has no private network address".to_owned());
            return;
        };
        // LG's service has to answer before anyone can connect: the controller's address is part of what Home Assistant is told.
        let adapter = match self.deps.bluetooth.adapter() {
            Ok(adapter) => adapter,
            Err(why) => {
                a.bind_retry_at = Some(retry);
                a.error = Some(format!("LG's Bluetooth service: {why}"));
                return;
            }
        };
        let listener = match TcpListener::bind(SocketAddr::new(ip.into(), a.link.port))
            .and_then(|l| l.set_nonblocking(true).map(|()| l))
        {
            Ok(l) => l,
            Err(e) => {
                a.bind_retry_at = Some(retry);
                a.error = Some(format!("could not listen on {ip}:{}: {e}", a.link.port));
                return;
            }
        };
        let addr = listener
            .local_addr()
            .unwrap_or_else(|_| SocketAddr::new(ip.into(), a.link.port));
        let mac = interface_mac(ip).unwrap_or_else(|| adapter.address.clone());
        let identity = Identity {
            name: a.link.name.clone(),
            friendly_name: a.link.friendly_name.clone(),
            mac: mac.clone(),
            bluetooth_mac: adapter.address.clone(),
        };
        // Clients that connect from now on are told the identity; a proxy built before it was known has none.
        a.proxy.set_identity(identity);
        let service = Service {
            instance: a.link.name.clone(),
            kind: SERVICE.to_owned(),
            port: addr.port(),
            txt: [
                ("mac", mac.replace(':', "").to_ascii_lowercase()),
                ("version", crate::ble_proxy::ESPHOME_VERSION.to_owned()),
                ("friendly_name", a.link.friendly_name.clone()),
                ("network", "wifi".to_owned()),
                ("project_name", "lumatv.ble-proxy".to_owned()),
                ("project_version", crate::VERSION.to_owned()),
                ("api_encryption", ENCRYPTION.to_owned()),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect(),
        };
        if let Err(why) = self.deps.announcer.announce(ip, service) {
            // Home Assistant can still be given the address by hand; the socket stays.
            crate::log_warn!("ble: not announced: {why}");
            a.error = Some(format!("not announced on the network: {why}"));
        } else {
            a.error = None;
        }
        crate::log_info!("ble: listening on {addr} as {}", a.link.name);
        a.bound = Some(Bound {
            listener,
            addr,
            ip,
            checked: now,
        });
        a.bind_retry_at = None;
    }

    fn drop_listener(&mut self) {
        self.deps.bluetooth.stop_scan();
        self.deps.announcer.withdraw();
        if let Some(a) = self.active.as_mut() {
            a.bound = None;
            a.scan = None;
            for (id, _) in a.clients.drain() {
                a.proxy.client_gone(id);
            }
            if a.duty.is_scanning() {
                a.duty.end_window(Instant::now());
            }
            a.bind_retry_at = None;
        }
    }

    // ---- the radio -----------------------------------------------------------------------------------------------------------------

    fn drive_radio(&mut self, now: Instant) {
        let Some(a) = self.active.as_mut() else {
            return;
        };
        if !a.proxy.wanted() {
            // Nobody listens: the radio is handed back at once.
            if a.duty.is_scanning() {
                self.deps.bluetooth.stop_scan();
                a.duty.end_window(now);
                a.scan = None;
            }
            a.hold = None;
            a.proxy.set_scan_state(ScanState::Idle);
            return;
        }
        self.pump_events(now);
        let Some(a) = self.active.as_mut() else {
            return;
        };
        let standby = a.link.standby;
        let screen_ok = standby || screen::is_on(&self.ctx.hub);
        if a.duty.is_scanning() {
            if a.duty.window_over(now) {
                self.deps.bluetooth.stop_scan();
                a.duty.end_window(now);
                if let Some(run) = a.scan.take() {
                    a.last_window_heard = Some(run.heard);
                    a.windows_silent = if run.heard == 0 {
                        a.windows_silent + 1
                    } else {
                        0
                    };
                }
            } else if !screen_ok {
                self.deps.bluetooth.stop_scan();
                a.duty.hold(now, HOLD_SCREEN);
                a.scan = None;
                a.hold = Some("the screen is off".to_owned());
            } else if a.scan.as_ref().is_some_and(|s| {
                !s.subscribed && now >= s.started + self.deps.timing.subscribe_within
            }) {
                self.fail_scan(now, "LG's Bluetooth service did not answer le/startScan");
            }
        }
        let Some(a) = self.active.as_mut() else {
            return;
        };
        if !a.duty.is_scanning() && a.duty.due(now) {
            self.try_start(now);
        }
        self.reflect_scan_state();
    }

    /// The guards, then the scan.
    fn try_start(&mut self, now: Instant) {
        let wake = self.scan_wake();
        let Some(a) = self.active.as_mut() else {
            return;
        };
        if !(a.link.standby || screen::is_on(&self.ctx.hub)) {
            a.duty.hold(now, HOLD_SCREEN);
            a.hold = Some("the screen is off".to_owned());
            return;
        }
        match self.deps.bluetooth.adapter() {
            Ok(adapter) if !adapter.powered => {
                a.duty.hold(now, HOLD_RADIO);
                a.hold = Some("Bluetooth is switched off on the TV".to_owned());
                return;
            }
            Ok(_) => {}
            Err(why) => return self.fail_scan(now, &why),
        }
        match self.deps.bluetooth.audio_busy() {
            Ok(false) => {}
            Ok(true) => {
                a.duty.hold(now, HOLD_RADIO);
                a.hold = Some("a Bluetooth audio link is up".to_owned());
                return;
            }
            Err(why) => return self.fail_scan(now, &why),
        }
        match self.deps.bluetooth.start_scan(a.link.always, wake) {
            Ok(()) => {
                a.duty.begin(now);
                a.hold = None;
                a.scan = Some(ScanRun {
                    started: now,
                    subscribed: false,
                    heard: 0,
                });
            }
            Err(why) => self.fail_scan(now, &why),
        }
    }

    fn fail_scan(&mut self, now: Instant, why: &str) {
        self.deps.bluetooth.stop_scan();
        let Some(a) = self.active.as_mut() else {
            return;
        };
        a.scan = None;
        a.hold = None;
        let gave_up = a.duty.failed(now);
        crate::log_warn!(
            "ble: scan failed ({why}); {}",
            if gave_up {
                "giving up until the link is made again".to_owned()
            } else {
                format!("failure {} of the last five", a.duty.failures())
            }
        );
        a.error = Some(why.to_owned());
    }

    fn pump_events(&mut self, now: Instant) {
        let events = self.deps.bluetooth.take_events();
        let dropped = self.deps.bluetooth.take_dropped();
        let Some(a) = self.active.as_mut() else {
            return;
        };
        a.proxy.add_dropped(dropped);
        for event in events {
            match event {
                Event::Subscribed(_) => {
                    if let Some(run) = a.scan.as_mut() {
                        run.subscribed = true;
                    }
                    a.error = None;
                }
                Event::Heard(h) => {
                    if !a.duty.is_scanning() {
                        continue;
                    }
                    if let Some(run) = a.scan.as_mut() {
                        run.heard += 1;
                    }
                    a.proxy.heard(
                        RawAdvertisement {
                            address: h.address,
                            rssi: h.rssi,
                            address_type: h.address_type,
                            data: h.data,
                        },
                        now,
                    );
                }
                Event::Failed { code, text } => {
                    if a.duty.is_scanning() {
                        let why = format!("le/startScan refused: {text} (code {code})");
                        return self.fail_scan(now, &why);
                    }
                }
                Event::Ended => {
                    if a.duty.is_scanning() {
                        return self.fail_scan(now, "LG's service ended the scan");
                    }
                }
            }
        }
    }

    /// What Home Assistant is told the scanner is doing.
    fn reflect_scan_state(&mut self) {
        let Some(a) = self.active.as_mut() else {
            return;
        };
        let state = if !a.proxy.wanted() {
            ScanState::Idle
        } else {
            match a.duty.phase() {
                Phase::Idle => ScanState::Starting,
                Phase::Window { .. } | Phase::Rest { .. } | Phase::Held { .. } => {
                    ScanState::Running
                }
                Phase::Backoff { .. } | Phase::GaveUp => ScanState::Failed,
            }
        };
        a.proxy.set_scan_state(state);
    }

    // ---- the sockets -------------------------------------------------------------------------------------------------------------

    fn accept(&mut self) {
        let Some(a) = self.active.as_mut() else {
            return;
        };
        let Some(bound) = &a.bound else {
            return;
        };
        loop {
            let (stream, peer) = match bound.listener.accept() {
                Ok(pair) => pair,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return,
            };
            // Only the home network: a peer that is not in a private range is dropped before it is answered.
            let lan = match peer.ip() {
                std::net::IpAddr::V4(ip) => sys::is_lan_address(ip),
                std::net::IpAddr::V6(_) => false,
            };
            if !lan || set_nonblocking(&stream).is_err() {
                continue;
            }
            match a.proxy.add_client() {
                Some(id) => {
                    a.clients.insert(
                        id,
                        ClientIo {
                            stream,
                            pending: Vec::new(),
                        },
                    );
                }
                None => continue,
            }
        }
    }

    fn read_client(&mut self, id: ConnId, now: Instant) {
        let mut buf = [0u8; 4096];
        loop {
            let Some(a) = self.active.as_mut() else {
                return;
            };
            let Some(client) = a.clients.get_mut(&id) else {
                return;
            };
            match client.stream.read(&mut buf) {
                Ok(0) => return self.drop_client(id),
                Ok(n) => a.proxy.client_input(id, &buf[..n], now),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return self.drop_client(id),
            }
        }
    }

    fn drop_client(&mut self, id: ConnId) {
        if let Some(a) = self.active.as_mut() {
            a.clients.remove(&id);
            a.proxy.client_gone(id);
        }
    }

    /// Writes what the proxy has for a client, and closes it if that was the end.
    fn service_client(&mut self, id: ConnId) {
        let Some(a) = self.active.as_mut() else {
            return;
        };
        let out = a.proxy.client_output(id);
        let Some(client) = a.clients.get_mut(&id) else {
            return;
        };
        client.pending.extend(out);
        let mut dead = false;
        while !client.pending.is_empty() {
            match client.stream.write(&client.pending) {
                Ok(0) => {
                    dead = true;
                    break;
                }
                Ok(n) => {
                    client.pending.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => {
                    dead = true;
                    break;
                }
            }
        }
        let done = client.pending.is_empty() && a.proxy.client_closing(id);
        if dead || done || client.pending.len() > MAX_PENDING {
            self.drop_client(id);
        }
    }

    fn service_all(&mut self) {
        let ids = match self.active.as_mut() {
            Some(a) => {
                // A close the proxy asked for is carried out when the client's output is written (`service_client`).
                a.proxy.take_actions();
                a.proxy.client_ids()
            }
            None => return,
        };
        for id in ids {
            self.service_client(id);
        }
    }

    // ---- what the owner sees ---------------------------------------------------------------------------------------------------------

    fn state_of(a: &Active) -> &'static str {
        if a.bound.is_none() {
            return "waiting";
        }
        if !a.proxy.wanted() {
            return "listening";
        }
        match a.duty.phase() {
            Phase::Window { .. } => "scanning",
            Phase::Held { .. } => "held",
            Phase::Backoff { .. } | Phase::GaveUp => "failed",
            Phase::Rest { .. } | Phase::Idle => "resting",
        }
    }

    /// Writes the topic; a change of the counters alone raises no event.
    fn publish(&mut self) {
        let Some(a) = self.active.as_ref() else {
            return;
        };
        let status = a.proxy.status();
        let value = json!({
            "state": Self::state_of(a),
            "listening": a.bound.as_ref().map(|b| b.addr.to_string()),
            "name": a.link.name,
            "clients": status.clients,
            "subscribed": status.subscribed,
            "advertisements": status.advertisements,
            "dropped": status.dropped,
            "windows": a.duty.windows(),
            "lastWindowHeard": a.last_window_heard,
            "silentWindows": a.windows_silent,
            "scanSeconds": a.link.scan_seconds,
            "restSeconds": a.link.rest_seconds,
            "dutyPercent": a.link.duty_percent(),
            "standby": a.link.standby,
            "hold": if a.duty.is_scanning() { None } else { a.hold.clone() },
            "error": a.error,
        });
        self.ctx.hub.set_ignoring(
            TOPIC,
            value.clone(),
            &["advertisements", "dropped", "lastWindowHeard"],
        );
        self.published = Some(value);
    }

    // ---- the loop ------------------------------------------------------------------------------------------------------------------

    /// When the thread must next look at something, apart from the sockets and its wake pipe.
    fn next_deadline(&self, now: Instant) -> Instant {
        let mut next = self.reload_at;
        let mut consider = |t: Option<Instant>| {
            if let Some(t) = t {
                next = next.min(t);
            }
        };
        if let Some(a) = &self.active {
            consider(a.proxy.next_deadline());
            consider(a.bind_retry_at);
            consider(
                a.bound
                    .as_ref()
                    .map(|b| b.checked + self.deps.timing.address_check),
            );
            if a.proxy.wanted() {
                consider(a.duty.next_deadline(now));
                consider(
                    a.scan
                        .as_ref()
                        .filter(|s| !s.subscribed)
                        .map(|s| s.started + self.deps.timing.subscribe_within),
                );
                // The panel going dark ends a window that is on (the screen watcher has no wake-up for this thread).
                if a.duty.is_scanning() && !a.link.standby {
                    consider(Some(now + HOLD_SCREEN));
                }
            }
        }
        next
    }

    /// One turn: look at the file and the radio, wait up to `max_wait` for a socket or a wake-up, serve what came.
    fn tick(&mut self, now: Instant, max_wait: Duration) {
        if now >= self.reload_at {
            self.reload(now);
        }
        self.ensure_listener(now);
        self.drive_radio(now);
        self.service_all();
        let wait = max_wait
            .min(self.next_deadline(now).saturating_duration_since(now))
            .min(Duration::from_secs(3600));
        let timeout_ms = i32::try_from(wait.as_micros().div_ceil(1000)).unwrap_or(i32::MAX);

        let mut fds = vec![pollfd(self.wake.read.as_raw_fd(), libc::POLLIN)];
        let mut ids = Vec::new();
        let has_listener = match self.active.as_ref() {
            Some(a) => {
                if let Some(b) = &a.bound {
                    fds.push(pollfd(b.listener.as_raw_fd(), libc::POLLIN));
                }
                for (id, client) in &a.clients {
                    let mut events = libc::POLLIN;
                    if !client.pending.is_empty() {
                        events |= libc::POLLOUT;
                    }
                    ids.push(*id);
                    fds.push(pollfd(client.stream.as_raw_fd(), events));
                }
                a.bound.is_some()
            }
            None => false,
        };
        // SAFETY: poll over a vector of pollfds of the length given.
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
        let now = if self.virtual_time {
            now
        } else {
            Instant::now().max(now)
        };
        if ready > 0 {
            if fds[0].revents != 0 {
                self.wake.drain();
            }
            let first_client = if has_listener { 2 } else { 1 };
            if has_listener && fds[1].revents & libc::POLLIN != 0 {
                self.accept();
            }
            for (i, id) in ids.iter().enumerate() {
                let revents = fds[first_client + i].revents;
                if revents & libc::POLLOUT != 0 {
                    self.service_client(*id);
                }
                if revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
                    self.read_client(*id, now);
                }
            }
        }
        if let Some(a) = self.active.as_mut() {
            a.proxy.handle_timeout(now);
        }
        self.drive_radio(now);
        self.service_all();
        self.publish();
    }

    fn finish(&mut self) {
        self.teardown();
        self.set_off(None);
    }
}

/// Starts the runner. With no link saved it costs this thread and a look at one file every ten seconds.
pub fn spawn(ctx: &Ctx, deps: Deps) -> io::Result<JoinHandle<()>> {
    let ctx = ctx.clone();
    let mut runner = Runner::new(ctx, deps, Instant::now())?;
    thread::Builder::new()
        .name("ble".to_owned())
        .stack_size(256 * 1024)
        .spawn(move || {
            while !runner.ctx.stop.is_set() {
                runner.tick(Instant::now(), Duration::from_secs(3600));
            }
            runner.finish();
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ble_scan;
    use crate::ble_scan::Heard;
    use crate::ble_scan::fake::FakeBluetooth;
    use crate::config::{Config, Paths};
    use crate::control::Stop;
    use crate::hub::Hub;
    use crate::settings::SettingsStore;
    use esphome_api::message::{self as m, Message, scanner};
    use esphome_api::session::Client;
    use parking_lot::Mutex;

    const KEY: [u8; 32] = [0x42; 32];

    #[derive(Default)]
    struct Told {
        announced: Vec<(Ipv4Addr, Service)>,
        withdrawn: u32,
    }

    struct FakeAnnouncer(Arc<Mutex<Told>>);

    impl Announcer for FakeAnnouncer {
        fn announce(&mut self, address: Ipv4Addr, service: Service) -> Result<(), String> {
            self.0.lock().announced.push((address, service));
            Ok(())
        }

        fn withdraw(&mut self) {
            self.0.lock().withdrawn += 1;
        }
    }

    /// Home Assistant, over a real loopback socket, with the real client end of the protocol.
    struct Ha {
        stream: TcpStream,
        client: Client,
        inbox: Vec<Message>,
        /// What is said before the handshake is over waits for it.
        queued: Vec<Message>,
    }

    impl Ha {
        fn connect(port: u16, key: [u8; 32]) -> Self {
            let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream.set_nonblocking(true).unwrap();
            let mut ha = Self {
                stream,
                client: Client::new(key, [7; 32]),
                inbox: Vec::new(),
                queued: Vec::new(),
            };
            ha.flush();
            ha
        }

        fn send(&mut self, message: impl Into<Message>) {
            self.queued.push(message.into());
            self.release();
        }

        fn release(&mut self) {
            if self.client.is_ready() {
                for message in std::mem::take(&mut self.queued) {
                    self.client.send(message.id(), &message.encode()).unwrap();
                }
                self.flush();
            }
        }

        fn flush(&mut self) {
            let out = self.client.take_output();
            if !out.is_empty() {
                let _ = self.stream.write_all(&out);
            }
        }

        /// Reads what has arrived; whether the connection is still up.
        fn read(&mut self) -> bool {
            let mut buf = [0u8; 8192];
            loop {
                match self.stream.read(&mut buf) {
                    Ok(0) => return false,
                    Ok(n) => {
                        let _ = self.client.input(&buf[..n]);
                        self.flush();
                        self.release();
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => return false,
                }
            }
            while let Some((kind, payload)) = self.client.next_message() {
                if let Some(message) = Message::decode(kind, &payload).unwrap() {
                    self.inbox.push(message);
                }
            }
            true
        }
    }

    struct Rig {
        runner: Runner,
        bt: FakeBluetooth,
        told: Arc<Mutex<Told>>,
        ctx: Ctx,
        dir: tempfile::TempDir,
        now: Instant,
        port: u16,
    }

    fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    impl Rig {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let hub = Arc::new(Hub::new(0));
            let ctx = Ctx {
                config: Arc::new(Config::default()),
                paths: Paths::new(dir.path()),
                settings: Arc::new(SettingsStore::new(
                    Paths::new(dir.path()).settings_dir(),
                    hub.clone(),
                )),
                hub,
                stop: Stop::new(),
            };
            ctx.hub.set("screen", json!({"on": true}));
            let bt = FakeBluetooth::new();
            let told = Arc::new(Mutex::new(Told::default()));
            let now = Instant::now();
            let deps = Deps {
                bluetooth: Box::new(bt.clone()),
                announcer: Box::new(FakeAnnouncer(told.clone())),
                bind: Some(Ipv4Addr::LOCALHOST),
                timing: Timing {
                    reload: Duration::from_secs(10),
                    bind_retry: Duration::from_secs(15),
                    address_check: Duration::from_secs(60),
                    subscribe_within: Duration::from_secs(5),
                },
            };
            let mut runner = Runner::new(ctx.clone(), deps, now).unwrap();
            runner.virtual_time = true;
            Self {
                runner,
                bt,
                told,
                ctx,
                dir,
                now,
                port: free_port(),
            }
        }

        fn link(&self) -> BleLink {
            let mut link = BleLink::with_key(KEY);
            link.port = self.port;
            link
        }

        fn save(&self, link: &BleLink) {
            ble_store::save(self.dir.path(), link).unwrap();
        }

        /// Time passes (`by`), the runner takes a turn without waiting, and real sockets get a moment to deliver.
        fn step(&mut self, by: Duration) {
            self.now += by;
            self.runner.tick(self.now, Duration::ZERO);
            std::thread::sleep(Duration::from_millis(2));
        }

        /// Turns until `done` holds (virtual time stands still; sockets are real).
        fn until(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) {
            for _ in 0..400 {
                self.step(Duration::ZERO);
                if done(self) {
                    return;
                }
            }
            panic!("never: {what}; topic {:?}", self.ctx.hub.get(TOPIC));
        }

        fn topic(&mut self) -> Value {
            self.runner.publish();
            self.ctx.hub.get(TOPIC).expect("the topic")
        }

        /// Linked, listening, and Home Assistant connected and subscribed.
        fn subscribed(&mut self) -> Ha {
            self.save(&self.link());
            self.step(Duration::ZERO);
            let mut ha = Ha::connect(self.port, KEY);
            ha.send(m::HelloRequest {
                client_info: "Home Assistant".into(),
                api_version_major: 1,
                api_version_minor: 19,
            });
            ha.send(m::SubscribeBluetoothLEAdvertisementsRequest { flags: 1 });
            self.until("subscribed", |r| {
                ha.read();
                r.runner.active.as_ref().is_some_and(|a| a.proxy.wanted())
            });
            ha
        }
    }

    fn heard(address: u64, rssi: i32) -> Heard {
        Heard {
            address,
            rssi,
            address_type: 0,
            data: vec![2, 1, 6],
        }
    }

    #[test]
    fn with_no_link_nothing_listens_nothing_is_announced_and_the_radio_is_never_asked() {
        let mut rig = Rig::new();
        for _ in 0..3 {
            rig.step(Duration::from_secs(11));
        }
        assert!(rig.runner.active.is_none());
        let s = rig.bt.state.lock();
        assert_eq!((s.starts, s.stops), (0, 0));
        drop(s);
        assert!(rig.told.lock().announced.is_empty());
        assert_eq!(rig.ctx.hub.get(TOPIC).unwrap()["state"], "off");
        assert!(TcpStream::connect(("127.0.0.1", rig.port)).is_err());
    }

    #[test]
    fn a_link_opens_the_socket_and_the_announcement_and_home_assistant_is_told_who_this_is() {
        let mut rig = Rig::new();
        rig.save(&rig.link());
        rig.step(Duration::ZERO);
        let told = rig.told.lock();
        assert_eq!(told.announced.len(), 1);
        let (address, service) = &told.announced[0];
        assert_eq!(
            (*address, service.instance.as_str(), service.port),
            (Ipv4Addr::LOCALHOST, "lumatv-ble", rig.port)
        );
        let txt: HashMap<_, _> = service.txt.iter().cloned().collect();
        assert_eq!(txt["api_encryption"], ENCRYPTION);
        assert_eq!(txt["friendly_name"], "LumaTV Bluetooth");
        assert_eq!(txt["mac"].len(), 12);
        drop(told);
        let mut ha = Ha::connect(rig.port, KEY);
        ha.send(m::HelloRequest {
            client_info: "Home Assistant".into(),
            api_version_major: 1,
            api_version_minor: 19,
        });
        ha.send(m::DeviceInfoRequest::default());
        rig.until("device info", |_| {
            ha.read();
            ha.inbox.len() >= 2
        });
        let Some(Message::DeviceInfoResponse(info)) = ha.inbox.pop() else {
            panic!("no device info: {:?}", ha.inbox);
        };
        assert_eq!(info.name, "lumatv-ble");
        // The adapter's address came from LG's service.
        assert_eq!(info.bluetooth_mac_address, "84:A3:29:B9:4D:43");
        assert_eq!(rig.topic()["state"], "listening");
        assert_eq!(rig.topic()["listening"], format!("127.0.0.1:{}", rig.port));
    }

    #[test]
    fn no_socket_until_lgs_bluetooth_service_answers() {
        let mut rig = Rig::new();
        rig.bt.state.lock().adapter = Some(Err("luna-send did not answer".into()));
        rig.save(&rig.link());
        rig.step(Duration::ZERO);
        assert!(TcpStream::connect(("127.0.0.1", rig.port)).is_err());
        assert_eq!(rig.topic()["state"], "waiting");
        assert!(
            rig.topic()["error"]
                .as_str()
                .unwrap()
                .contains("did not answer")
        );
        // It comes up; the next try is after the retry time.
        rig.bt.state.lock().adapter = Some(Ok(ble_scan::Adapter {
            address: "84:A3:29:B9:4D:43".into(),
            powered: true,
        }));
        rig.step(Duration::from_secs(14));
        assert!(TcpStream::connect(("127.0.0.1", rig.port)).is_err());
        rig.step(Duration::from_secs(2));
        assert!(TcpStream::connect(("127.0.0.1", rig.port)).is_ok());
        assert_eq!(rig.topic()["error"], Value::Null);
    }

    #[test]
    fn a_subscription_starts_a_window_and_advertisements_reach_home_assistant() {
        let mut rig = Rig::new();
        let mut ha = rig.subscribed();
        {
            let s = rig.bt.state.lock();
            assert_eq!((s.starts, s.scanning, s.always), (1, true, Some(false)));
        }
        for i in 0..16u64 {
            rig.bt.hear(heard(0xaabb_ccdd_ee00 + i, -60 - i as i32));
        }
        rig.until("a batch", |_| {
            ha.read();
            ha.inbox
                .iter()
                .any(|m| matches!(m, Message::BluetoothLERawAdvertisementsResponse(_)))
        });
        let batch = ha
            .inbox
            .iter()
            .find_map(|m| match m {
                Message::BluetoothLERawAdvertisementsResponse(b) => Some(b.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(batch.advertisements.len(), 16);
        assert_eq!(batch.advertisements[0].address, 0xaabb_ccdd_ee00);
        assert_eq!(batch.advertisements[1].rssi, -61);
        assert_eq!(batch.advertisements[0].data, vec![2, 1, 6]);
        // The scanner was reported as running, passive.
        let state = ha
            .inbox
            .iter()
            .filter_map(|m| match m {
                Message::BluetoothScannerStateResponse(s) => Some(s.clone()),
                _ => None,
            })
            .next_back()
            .unwrap();
        assert_eq!(
            (state.state, state.mode),
            (scanner::RUNNING, scanner::PASSIVE)
        );
        let topic = rig.topic();
        assert_eq!(
            (topic["state"].as_str(), topic["advertisements"].as_u64()),
            (Some("scanning"), Some(16))
        );
        // Nothing about what was heard is published.
        assert!(!topic.to_string().to_lowercase().contains("aabbccdd"));
    }

    #[test]
    fn a_window_ends_on_time_and_the_next_waits_for_the_rest() {
        let mut rig = Rig::new();
        let _ha = rig.subscribed();
        assert_eq!(rig.bt.state.lock().starts, 1);
        rig.step(Duration::from_secs(9));
        assert!(rig.bt.state.lock().scanning);
        rig.step(Duration::from_secs(1));
        {
            let s = rig.bt.state.lock();
            assert!(!s.scanning);
            assert_eq!((s.starts, s.stops), (1, 1));
        }
        rig.step(Duration::from_secs(49));
        assert_eq!(rig.bt.state.lock().starts, 1, "the rest is 50 s");
        rig.step(Duration::from_secs(1));
        assert_eq!(rig.bt.state.lock().starts, 2);
        assert_eq!(rig.topic()["windows"], 2);
    }

    #[test]
    fn home_assistant_reconnecting_in_a_loop_gets_one_window_per_rest() {
        let mut rig = Rig::new();
        let mut ha = rig.subscribed();
        rig.step(Duration::from_secs(2));
        // It goes away mid-window and comes straight back, three times.
        for _ in 0..3 {
            drop(ha);
            rig.until("the client is gone", |r| {
                r.runner.active.as_ref().is_some_and(|a| !a.proxy.wanted())
            });
            assert!(
                !rig.bt.state.lock().scanning,
                "the radio is handed back at once"
            );
            ha = Ha::connect(rig.port, KEY);
            ha.send(m::HelloRequest {
                client_info: "Home Assistant".into(),
                api_version_major: 1,
                api_version_minor: 19,
            });
            ha.send(m::SubscribeBluetoothLEAdvertisementsRequest { flags: 1 });
            rig.until("subscribed again", |r| {
                ha.read();
                r.runner.active.as_ref().is_some_and(|a| a.proxy.wanted())
            });
            rig.step(Duration::from_secs(1));
        }
        assert_eq!(rig.bt.state.lock().starts, 1);
        rig.step(Duration::from_secs(60));
        assert_eq!(rig.bt.state.lock().starts, 2);
    }

    #[test]
    fn an_unsubscribe_hands_the_radio_back_at_once() {
        let mut rig = Rig::new();
        let mut ha = rig.subscribed();
        assert!(rig.bt.state.lock().scanning);
        ha.send(m::UnsubscribeBluetoothLEAdvertisementsRequest::default());
        rig.until("the scan is off", |r| !r.bt.state.lock().scanning);
        assert_eq!(rig.topic()["state"], "listening");
    }

    #[test]
    fn a_dark_screen_holds_the_radio_unless_standby_scanning_is_on() {
        let mut rig = Rig::new();
        rig.ctx.hub.set("screen", json!({"on": false}));
        let _ha = rig.subscribed();
        assert_eq!(rig.bt.state.lock().starts, 0);
        assert_eq!(rig.topic()["state"], "held");
        assert_eq!(rig.topic()["hold"], "the screen is off");
        rig.ctx.hub.set("screen", json!({"on": true}));
        rig.step(Duration::from_secs(5));
        assert_eq!(rig.bt.state.lock().starts, 1);
        // The panel going dark ends a window that is on.
        rig.ctx.hub.set("screen", json!({"on": false}));
        rig.step(Duration::from_secs(5));
        assert!(!rig.bt.state.lock().scanning);

        let mut rig = Rig::new();
        rig.ctx.hub.set("screen", json!({"on": false}));
        let mut link = rig.link();
        link.standby = true;
        rig.save(&link);
        rig.step(Duration::ZERO);
        let mut ha = Ha::connect(rig.port, KEY);
        ha.send(m::HelloRequest {
            client_info: "Home Assistant".into(),
            api_version_major: 1,
            api_version_minor: 19,
        });
        ha.send(m::SubscribeBluetoothLEAdvertisementsRequest { flags: 1 });
        rig.until("scanning in standby", |r| {
            ha.read();
            r.bt.state.lock().scanning
        });
    }

    #[test]
    fn bluetooth_audio_and_a_switched_off_adapter_hold_the_radio() {
        let mut rig = Rig::new();
        rig.bt.state.lock().audio = true;
        let _ha = rig.subscribed();
        assert_eq!(rig.bt.state.lock().starts, 0);
        assert_eq!(rig.topic()["hold"], "a Bluetooth audio link is up");
        rig.bt.state.lock().audio = false;
        rig.step(Duration::from_secs(29));
        assert_eq!(rig.bt.state.lock().starts, 0);
        rig.step(Duration::from_secs(1));
        assert_eq!(rig.bt.state.lock().starts, 1);

        let mut rig = Rig::new();
        rig.bt.state.lock().adapter = Some(Ok(ble_scan::Adapter {
            address: "84:A3:29:B9:4D:43".into(),
            powered: false,
        }));
        let _ha = rig.subscribed();
        assert_eq!(rig.bt.state.lock().starts, 0);
        assert_eq!(rig.topic()["hold"], "Bluetooth is switched off on the TV");
    }

    #[test]
    fn a_refused_scan_backs_off_and_five_in_a_row_end_the_trying() {
        let mut rig = Rig::new();
        rig.bt.state.lock().refuse_start = Some("luna-send: permission denied".into());
        let mut ha = rig.subscribed();
        let mut waits = vec![];
        for wait in [60, 120, 240, 480] {
            assert_eq!(rig.bt.state.lock().starts, 0);
            waits.push(wait);
            rig.step(Duration::from_secs(wait - 1));
            let before = rig.topic()["error"].clone();
            rig.step(Duration::from_secs(1));
            assert!(before.as_str().unwrap().contains("permission denied"));
        }
        assert_eq!(rig.topic()["state"], "failed");
        // Home Assistant was told the scanner failed.
        ha.read();
        assert!(ha.inbox.iter().any(
            |m| matches!(m, Message::BluetoothScannerStateResponse(s) if s.state == scanner::FAILED)
        ));
        // The fifth was the last: a day later still nothing.
        rig.bt.state.lock().refuse_start = None;
        rig.step(Duration::from_secs(86_400));
        assert_eq!(rig.bt.state.lock().starts, 0);
        // The owner linking again starts afresh.
        let mut link = rig.link();
        link.friendly_name = "LumaTV Bluetooth 2".into();
        rig.save(&link);
        rig.step(Duration::from_secs(11));
        let mut ha = Ha::connect(rig.port, KEY);
        ha.send(m::HelloRequest {
            client_info: "Home Assistant".into(),
            api_version_major: 1,
            api_version_minor: 19,
        });
        ha.send(m::SubscribeBluetoothLEAdvertisementsRequest { flags: 1 });
        rig.until("a window after a fresh link", |r| {
            ha.read();
            r.bt.state.lock().scanning
        });
    }

    #[test]
    fn lgs_service_ending_the_scan_is_a_failure_and_a_refusal_carries_its_words() {
        let mut rig = Rig::new();
        let _ha = rig.subscribed();
        rig.step(Duration::ZERO);
        rig.bt.say(Event::Failed {
            code: 143,
            text: "Not permitted".into(),
        });
        rig.step(Duration::ZERO);
        assert!(
            !rig.bt.state.lock().scanning,
            "the child is ended on a failure"
        );
        let topic = rig.topic();
        assert_eq!(topic["state"], "failed");
        assert!(topic["error"].as_str().unwrap().contains("Not permitted"));
        assert!(topic["error"].as_str().unwrap().contains("143"));
    }

    #[test]
    fn a_scan_that_never_says_it_has_started_is_ended() {
        let mut rig = Rig::new();
        rig.bt.state.lock().silent_start = true;
        let _ha = rig.subscribed();
        rig.step(Duration::from_secs(4));
        assert!(rig.bt.state.lock().scanning);
        rig.step(Duration::from_secs(1));
        assert!(!rig.bt.state.lock().scanning);
        assert!(
            rig.topic()["error"]
                .as_str()
                .unwrap()
                .contains("did not answer")
        );
    }

    #[test]
    fn removing_the_link_hands_everything_back() {
        let mut rig = Rig::new();
        let mut ha = rig.subscribed();
        assert!(rig.bt.state.lock().scanning);
        assert!(ble_store::clear(rig.dir.path()));
        rig.step(Duration::from_secs(11));
        assert!(!rig.bt.state.lock().scanning);
        assert!(rig.told.lock().withdrawn >= 1);
        assert!(rig.runner.active.is_none());
        assert!(!ha.read(), "Home Assistant's connection is closed");
        assert!(TcpStream::connect(("127.0.0.1", rig.port)).is_err());
        assert_eq!(rig.topic()["state"], "off");
    }

    #[test]
    fn a_changed_link_is_taken_up_and_a_wrong_key_is_refused() {
        let mut rig = Rig::new();
        rig.save(&rig.link());
        rig.step(Duration::ZERO);
        let mut wrong = Ha::connect(rig.port, [0x11; 32]);
        wrong.send(m::HelloRequest {
            client_info: "x".into(),
            api_version_major: 1,
            api_version_minor: 19,
        });
        rig.until("refused", |_| {
            wrong.read();
            wrong.client.rejected().is_some()
        });
        assert_eq!(wrong.client.rejected(), Some("Handshake MAC failure"));
        // The key rotated: the old one no longer works.
        let mut link = rig.link();
        link.key = [0x99; 32];
        rig.save(&link);
        rig.step(Duration::from_secs(11));
        let mut old = Ha::connect(rig.port, KEY);
        old.send(m::HelloRequest {
            client_info: "x".into(),
            api_version_major: 1,
            api_version_minor: 19,
        });
        rig.until("the old key is refused", |_| {
            old.read();
            old.client.rejected().is_some()
        });
    }

    #[test]
    fn a_port_in_use_is_reported_and_tried_again() {
        let mut rig = Rig::new();
        let squatter = TcpListener::bind(("127.0.0.1", rig.port)).unwrap();
        rig.save(&rig.link());
        rig.step(Duration::ZERO);
        assert!(
            rig.topic()["error"]
                .as_str()
                .unwrap()
                .contains("could not listen")
        );
        drop(squatter);
        rig.step(Duration::from_secs(16));
        assert!(TcpStream::connect(("127.0.0.1", rig.port)).is_ok());
    }

    #[test]
    fn stopping_the_helper_ends_the_scan() {
        let mut rig = Rig::new();
        let _ha = rig.subscribed();
        assert!(rig.bt.state.lock().scanning);
        rig.runner.finish();
        assert!(!rig.bt.state.lock().scanning);
        assert_eq!(rig.ctx.hub.get(TOPIC).unwrap()["state"], "off");
    }
}
