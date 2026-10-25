//! The LAN listener of the MCP server: it exists only while the TV's owner has turned AI access on.
//!
//! Off is the default and off means *no socket*: nothing is bound, nothing accepts, nothing is waiting. The switch is
//! [`Store::listener_enabled`], which only a physical remote press can turn on (see `gate` and `store`); this thread just makes
//! the world match it. While it is on, the listener binds the TV's own private address (never `0.0.0.0`, never a public
//! address), serves only peers in the allowed private networks, and re-checks the address every minute (a DHCP change moves it).
//! When the switch goes off, or a lock-out happens, the server is stopped and the port closes within a moment.
//!
//! One thread, blocked on a channel while the switch is off.

use super::store::Store;
use crate::control::Stop;
use crate::http::{Cidr, Handler, Mode, Options, Server};
use crate::sys;
use parking_lot::Mutex;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// How often a listener that is meant to be on but is not bound tries again.
const RETRY: Duration = Duration::from_secs(15);
/// How often a bound listener checks that the TV still has the address it is bound to.
const RECHECK: Duration = Duration::from_secs(60);
/// Connections served at once on the LAN (the per-address cap in the server is lower).
const MAX_CONNECTIONS: usize = 8;

/// How the listener is set up.
#[derive(Clone, Debug)]
pub struct ListenerConfig {
    pub port: u16,
    pub bind: Option<Ipv4Addr>,
    pub peers: Vec<Cidr>,
}

/// Where the listener stands, for the TV's page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListenerStatus {
    pub bound: Option<SocketAddr>,
    pub error: Option<String>,
}

enum Cmd {
    Reconcile,
    Stop,
}

pub struct Listener {
    tx: Sender<Cmd>,
    status: Arc<Mutex<ListenerStatus>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Listener {
    /// Starts the thread. `status` is where it reports (the owner reads it for the TV's page); `on_status` is called after every
    /// change of it, outside any lock.
    pub fn start(
        store: Arc<Store>,
        handler: Arc<dyn Handler>,
        config: ListenerConfig,
        status: Arc<Mutex<ListenerStatus>>,
        stop: &Stop,
        on_status: Arc<dyn Fn() + Send + Sync>,
    ) -> std::io::Result<Arc<Self>> {
        let (tx, rx) = channel();
        let thread = {
            let status = status.clone();
            thread::Builder::new()
                .name("ai-listener".to_owned())
                .stack_size(256 * 1024)
                .spawn(move || run(&store, handler, &config, &rx, &status, on_status.as_ref()))?
        };
        let listener = Arc::new(Self {
            tx,
            status,
            thread: Mutex::new(Some(thread)),
        });
        let tx = listener.tx.clone();
        stop.on_stop(move || {
            let _ = tx.send(Cmd::Stop);
        });
        Ok(listener)
    }

    /// Looks at the switch again (call after every change to it).
    pub fn reconcile(&self) {
        let _ = self.tx.send(Cmd::Reconcile);
    }

    pub fn status(&self) -> ListenerStatus {
        self.status.lock().clone()
    }

    /// Stops the server and the thread and waits for them.
    pub fn stop(&self) {
        let _ = self.tx.send(Cmd::Stop);
        if let Some(handle) = self.thread.lock().take() {
            let _ = handle.join();
        }
    }
}

fn set_status(
    status: &Mutex<ListenerStatus>,
    new: ListenerStatus,
    on_status: &(dyn Fn() + Send + Sync),
) {
    let changed = {
        let mut current = status.lock();
        let changed = *current != new;
        *current = new;
        changed
    };
    if changed {
        on_status();
    }
}

fn run(
    store: &Store,
    handler: Arc<dyn Handler>,
    config: &ListenerConfig,
    rx: &Receiver<Cmd>,
    status: &Mutex<ListenerStatus>,
    on_status: &(dyn Fn() + Send + Sync),
) {
    let mut server: Option<(Server, SocketAddr)> = None;
    loop {
        let wanted = store.listener_enabled();
        let mut wait = None;
        match (wanted, server.is_some()) {
            (true, false) => match bind(config, handler.clone()) {
                Ok((s, addr)) => {
                    crate::log_info!("ai: listening on {addr}");
                    server = Some((s, addr));
                    set_status(
                        status,
                        ListenerStatus {
                            bound: Some(addr),
                            error: None,
                        },
                        on_status,
                    );
                    wait = Some(RECHECK);
                }
                Err(why) => {
                    crate::log_warn!("ai: cannot listen: {why}");
                    set_status(
                        status,
                        ListenerStatus {
                            bound: None,
                            error: Some(why),
                        },
                        on_status,
                    );
                    wait = Some(RETRY);
                }
            },
            (false, true) => {
                if let Some((s, addr)) = server.take() {
                    s.stop(Duration::from_secs(2));
                    crate::log_info!("ai: stopped listening on {addr}");
                }
                set_status(status, ListenerStatus::default(), on_status);
            }
            (true, true) => {
                // The address may have moved (DHCP): bind again to wherever the TV is now.
                let moved = server.as_ref().is_some_and(|(_, addr)| {
                    match (sys::lan_ipv4(config.bind), addr) {
                        (Some(now), SocketAddr::V4(bound)) => now != *bound.ip(),
                        (None, _) => true,
                        _ => false,
                    }
                });
                if moved && let Some((s, addr)) = server.take() {
                    s.stop(Duration::from_secs(2));
                    crate::log_info!("ai: the TV's address changed from {addr}; binding again");
                    set_status(status, ListenerStatus::default(), on_status);
                    continue;
                }
                wait = Some(RECHECK);
            }
            (false, false) => {
                set_status(status, ListenerStatus::default(), on_status);
            }
        }
        let command = match wait {
            Some(timeout) => match rx.recv_timeout(timeout) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => Some(Cmd::Stop),
            },
            // Off: nothing runs and nothing times out; only a command wakes the thread.
            None => rx.recv().ok().or(Some(Cmd::Stop)),
        };
        if matches!(command, Some(Cmd::Stop)) {
            if let Some((s, addr)) = server.take() {
                s.stop(Duration::from_secs(2));
                crate::log_info!("ai: stopped listening on {addr}");
            }
            set_status(status, ListenerStatus::default(), on_status);
            return;
        }
    }
}

fn bind(
    config: &ListenerConfig,
    handler: Arc<dyn Handler>,
) -> Result<(Server, SocketAddr), String> {
    let ip = sys::lan_ipv4(config.bind)
        .ok_or_else(|| "the TV has no private network address to listen on".to_owned())?;
    let options = Options {
        max_connections: MAX_CONNECTIONS,
        allowed_origins: Vec::new(),
        mode: Mode::Lan {
            peers: config.peers.clone(),
        },
    };
    let server = Server::start(SocketAddr::new(ip.into(), config.port), handler, options)
        .map_err(|e| format!("{e}"))?;
    let addr = server.local_addr();
    Ok((server, addr))
}
