//! The phone remote: phones paired with the TV drive it over the home network through `lumaremote`. A plugin of the helper's own, off
//! until the owner switches it on: on, it listens on the TV's private address at port 18733 and announces itself over mDNS; off closes
//! both and ends every session, keeping the pairings. Opening pairing needs OK on the physical remote (the AI gate's prompt).
//!
//! | Route | |
//! |---|---|
//! | `GET /v1/phone-remote` | the state, as the `phone-remote` topic has it |
//! | `POST /v1/phone-remote/pairing` | `202`, a prompt on the TV; the code shows in the state after OK |
//! | `DELETE /v1/phone-remote/pairing` | closes the pairing window |
//! | `DELETE /v1/phone-remote/devices/{id}` | removes one phone and ends its session |
//! | `DELETE /v1/phone-remote/devices` | removes every phone |
//!
//! The topic: `{on, problem, pairing: {code, qr, expires} | null, devices: [{id, name, model, last, live}]}`.

mod host;

pub use host::Deps;

use crate::ai::{Action, Ai, Refusal};
use crate::control::Stop;
use crate::ctx::Ctx;
use crate::http::{CLIENT_HEADER, Request, Response, UI_CLIENT, error_response};
use crate::hub::Hub;
use crate::plugins::local::{Local, OptIn, Spec};
use crate::plugins::session::Shared;
use host::TvHost;
use lumaremote::server::{Config, Host, Server};
use mdns::{Mdns, Service};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::io;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::Duration;

pub const ID: &str = "phone-remote";
pub const TOPIC: &str = "phone-remote";
pub const OPT_IN: OptIn = OptIn {
    spec: Spec {
        id: ID,
        name: "Phone remote",
        version: "1",
        permissions: &[],
    },
    consent: "Lets paired phones control this TV over the home network: the pointer, the remote's keys, typing into text boxes and \
              LumaTV's actions. While on, the TV listens on port 18733.",
};

pub const ROUTES: &[(&str, &str, &str)] = &[
    ("GET", "/v1/phone-remote", "the phone remote's state: the pairing window and the paired phones"),
    ("POST", "/v1/phone-remote/pairing", "opens a prompt on the TV; the code shows after OK (the TV's Settings only)"),
    ("DELETE", "/v1/phone-remote/pairing", "closes the pairing window (the TV's Settings only)"),
    ("DELETE", "/v1/phone-remote/devices/{id}", "removes one phone and ends its session (the TV's Settings only)"),
    ("DELETE", "/v1/phone-remote/devices", "removes every phone (the TV's Settings only)"),
];

/// How often the TV's address is looked at again, and a listener that did not start is tried again.
const RECHECK: Duration = Duration::from_secs(30);

enum Wake {
    Switch,
    Changed,
    Stop,
}

pub struct PhoneRemote {
    hub: Arc<Hub>,
    shared: Arc<Shared>,
    ai: Arc<Ai>,
    /// The listener while the plugin is on and the TV has a private address.
    server: Mutex<Option<Server>>,
    /// Why it is not listening though it is on.
    problem: Mutex<Option<String>>,
}

impl PhoneRemote {
    pub fn new(hub: Arc<Hub>, shared: Arc<Shared>, ai: Arc<Ai>) -> Arc<Self> {
        Arc::new(Self {
            hub,
            shared,
            ai,
            server: Mutex::new(None),
            problem: Mutex::new(None),
        })
    }

    /// Offers the plugin's switch, and follows it on a thread of its own until the helper stops.
    pub fn spawn(self: &Arc<Self>, ctx: &Ctx, deps: Deps) -> io::Result<JoinHandle<()>> {
        let (tx, rx) = mpsc::channel();
        {
            let tx = tx.clone();
            ctx.stop.on_stop(move || {
                let _ = tx.send(Wake::Stop);
            });
        }
        let changed = tx.clone();
        let mut plugin = Plugin {
            remote: self.clone(),
            stop: ctx.stop.clone(),
            host: Arc::new(TvHost::new(ctx, self.shared.clone(), deps, move || {
                let _ = changed.send(Wake::Changed);
            })),
            store: ctx.paths.data_dir.join("phone-remote/devices.json"),
            running: None,
            page: None,
        };
        self.shared.offer(
            &OPT_IN,
            Arc::new(move || {
                let _ = tx.send(Wake::Switch);
            }),
        );
        crate::plugins::guard::spawn("phone-remote", 256 * 1024, self.shared.clone(), &[ID], ctx.stop.clone(), move |_| plugin.run(&rx))
    }

    /// The state, as the topic and `GET /v1/phone-remote` have it.
    fn doc(&self) -> Value {
        let status = self.server.lock().as_ref().map(Server::status).unwrap_or_else(|| json!({"pairing": null, "devices": []}));
        json!({
            "on": self.shared.switched_on(ID),
            "problem": self.problem.lock().clone(),
            "pairing": status["pairing"],
            "devices": status["devices"],
        })
    }

    fn publish(&self) {
        self.hub.set(TOPIC, self.doc());
    }

    fn set_problem(&self, problem: Option<String>) {
        let mut held = self.problem.lock();
        if *held != problem
            && let Some(why) = &problem
        {
            crate::log_warn!("phone remote: {why}");
        }
        *held = problem;
    }

    fn answer(&self) -> Response {
        let mut doc = self.doc();
        doc["ok"] = json!(true);
        doc["api"] = json!(crate::API_VERSION);
        Response::json(200, &doc)
    }

    /// `change` done to the listener, answered with the state.
    fn with_server(&self, change: impl FnOnce(&Server) -> Result<(), Response>) -> Response {
        let done = match self.server.lock().as_ref() {
            Some(server) => change(server),
            None => return not_listening(),
        };
        match done {
            Ok(()) => self.answer(),
            Err(refused) => refused,
        }
    }

    /// Asks on the TV to open a pairing window, which opens once the physical remote approves.
    fn pair(self: &Arc<Self>) -> Response {
        match self.server.lock().as_ref() {
            None => return not_listening(),
            Some(server) if server.status()["devices"].as_array().map_or(0, Vec::len) >= lumaremote::store::MAX_DEVICES => {
                return error_response(409, "full", "16 phones are paired: remove one to pair another");
            }
            Some(_) => {}
        }
        let me = Arc::downgrade(self);
        let asked = self.ai.pair_phone(move || {
            if let Some(me) = me.upgrade()
                && let Some(server) = me.server.lock().as_ref()
                && let Err(why) = server.open_pairing()
            {
                crate::log_warn!("phone remote: pairing did not open: {why}");
            }
        });
        match asked {
            Ok(Action::Pending { prompt_id }) => {
                Response::json(202, &json!({"ok": true, "api": crate::API_VERSION, "pending": true, "promptId": prompt_id}))
            }
            Ok(Action::Done { .. }) => self.answer(),
            Err(Refusal::Busy) => error_response(409, "busy", "another prompt is open on the TV"),
            Err(refusal) => error_response(500, "failed", &format!("{refusal:?}")),
        }
    }
}

/// `/v1/phone-remote/...`; `rest` is the path after `phone-remote`. Pairing and removing phones are the TV's own Settings' to do.
pub fn handle(remote: &Arc<PhoneRemote>, request: &Request, rest: &[&str]) -> Response {
    let method = request.method.as_str();
    if method != "GET" && request.header(CLIENT_HEADER) != Some(UI_CLIENT) {
        return error_response(403, "not_the_ui", "pairing and removing phones are the TV's Settings' own");
    }
    match (method, rest) {
        ("GET", []) => remote.answer(),
        ("POST", ["pairing"]) => remote.pair(),
        ("DELETE", ["pairing"]) => remote.with_server(|server| {
            server.close_pairing();
            Ok(())
        }),
        ("DELETE", ["devices"]) => remote.with_server(|server| server.forget_all().map_err(io_failure)),
        ("DELETE", ["devices", id]) => remote.with_server(|server| match server.forget(id) {
            Ok(true) => Ok(()),
            Ok(false) => Err(error_response(404, "not_found", "no such phone")),
            Err(e) => Err(io_failure(e)),
        }),
        _ => error_response(404, "not_found", "no such phone remote route"),
    }
}

fn not_listening() -> Response {
    error_response(409, "not_listening", "the phone remote is not listening: it is off, or the TV has no address on the home network")
}

fn io_failure(e: io::Error) -> Response {
    error_response(500, "io", &e.to_string())
}

/// The plugin's state, held by its thread.
struct Plugin {
    remote: Arc<PhoneRemote>,
    stop: Stop,
    host: Arc<TvHost>,
    store: PathBuf,
    running: Option<Running>,
    /// The plugin's session with the broker while it is on: it lists the plugin as connected.
    page: Option<Arc<Local>>,
}

struct Running {
    ip: Ipv4Addr,
    name: String,
    /// The announcement (none when the responder did not start), and the TXT record it carries.
    mdns: Option<Mdns>,
    txt: Vec<(String, String)>,
}

impl Plugin {
    fn run(&mut self, rx: &Receiver<Wake>) {
        // after a panic, from nothing
        self.down();
        self.follow();
        loop {
            self.remote.publish();
            match rx.recv_timeout(RECHECK) {
                Ok(Wake::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(Wake::Switch) | Err(RecvTimeoutError::Timeout) => self.follow(),
                Ok(Wake::Changed) => self.changed(),
            }
            if self.stop.is_set() {
                break;
            }
        }
        self.down();
        self.page = None;
    }

    /// Follows the owner's switch and the TV's address: on, the listener runs on the address the TV has now; off, nothing runs.
    fn follow(&mut self) {
        if !self.remote.shared.switched_on(ID) {
            self.down();
            self.page = None;
            self.remote.set_problem(None);
            return;
        }
        if self.page.is_none() {
            match Local::start(&self.remote.shared, &OPT_IN.spec, json!({"title": OPT_IN.spec.name}), Arc::new(|_, _| {})) {
                Ok(page) => self.page = Some(page),
                Err(e) => crate::log_warn!("phone remote: the plugin's session did not start: {e}"),
            }
        }
        let ip = crate::sys::lan_ipv4(None);
        if ip.is_some() && ip == self.running.as_ref().map(|r| r.ip) {
            return;
        }
        self.down();
        let started = match ip {
            Some(ip) => self.up(ip),
            None => Err("the TV has no address on the home network".to_owned()),
        };
        self.remote.set_problem(started.err());
    }

    fn up(&mut self, ip: Ipv4Addr) -> Result<(), String> {
        let name = self.host.tv_name();
        let mac = crate::ble::interface_mac(ip).and_then(|mac| lumaremote::unhex(&mac.replace(':', "")));
        let host: Arc<dyn Host> = self.host.clone();
        let config = Config {
            bind: ip,
            port: lumaremote::PORT,
            name: name.clone(),
            mac,
            store: self.store.clone(),
        };
        let server = Server::start(config, host).map_err(|e| format!("not listening on {ip}:{}: {e}", lumaremote::PORT))?;
        let txt = server.txt();
        *self.remote.server.lock() = Some(server);
        let mdns = match Mdns::start_udp(ip) {
            Ok(mdns) => {
                mdns.add(service(&name, txt.clone()));
                Some(mdns)
            }
            Err(e) => {
                crate::log_warn!("phone remote: mDNS: {e}: phones find the TV only by its QR code");
                None
            }
        };
        crate::log_info!("phone remote: listening on {ip}:{}", lumaremote::PORT);
        self.running = Some(Running { ip, name, mdns, txt });
        Ok(())
    }

    /// Ends every session (the keys phones held are let go), withdraws the announcement and closes the compositor's connections.
    fn down(&mut self) {
        let server = self.remote.server.lock().take();
        drop(server);
        self.running = None;
        self.host.close();
    }

    /// A session, the pairing window or the paired phones changed: without a live session the compositor's phone session ends, and the
    /// TXT record follows the window.
    fn changed(&mut self) {
        let Some((live, txt)) = self.remote.server.lock().as_ref().map(|s| (s.live(), s.txt())) else {
            return;
        };
        if !live {
            self.host.session_off();
        }
        if let Some(running) = self.running.as_mut()
            && running.txt != txt
        {
            if let Some(mdns) = &running.mdns {
                mdns.add(service(&running.name, txt.clone()));
            }
            running.txt = txt;
        }
    }
}

/// The announcement. Its instance is the TV's name, which the responder also makes its host name: so no dot, and one DNS label at most.
fn service(name: &str, txt: Vec<(String, String)>) -> Service {
    let mut instance = name.replace('.', " ");
    while instance.len() > 63 {
        instance.pop();
    }
    let instance = instance.trim();
    Service {
        instance: if instance.is_empty() { "LumaTV" } else { instance }.to_owned(),
        kind: lumaremote::SERVICE.to_owned(),
        port: lumaremote::PORT,
        txt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_announcement_is_named_after_the_tv_as_one_dns_label() {
        assert_eq!(service("Living room", Vec::new()).instance, "Living room");
        assert_eq!(service("[LG] webOS TV OLED65C1.2", Vec::new()).instance, "[LG] webOS TV OLED65C1 2");
        assert_eq!(service(&"é".repeat(40), Vec::new()).instance.len(), 62);
        assert_eq!(service(" . ", Vec::new()).instance, "LumaTV");
        assert!(service(&"x".repeat(80), Vec::new()).is_valid());
    }
}
