//! The plugin broker: LumaShow's plugin API (level 1) on the TV.
//!
//! A plugin is a separate program that talks only to the helper, over a unix socket, and never draws: it declares a settings page,
//! cards, quick tiles and notifications, and the TV's shell draws them (`docs/plugin-api.md` in LumaShow is the protocol; the Rust
//! types are its `plugin-api` crate, used here as it is, and its `lumashow-sdk` is what the tests speak with).
//!
//! | Part | What |
//! |---|---|
//! | [`roster`] | which plugins are on (`plugins/enabled.json`, written by the owner's settings flow, never by the helper) and who each is |
//! | [`session`] | one plugin's conversation without a socket: checks, topics, settings, actions |
//! | [`broker`] | the socket, one reader and one writer thread per connection, rate and size limits |
//! | [`local`] | a plugin of the helper's own, in the same conversation with no socket (the TV's first-party plugins) |
//! | [`firstparty`] | the helper's first-party plugins: Home's weather, news and clock, and the ambient light tile |
//! | [`routes`] | `/v1/plugins`: ask a plugin to run an action |
//!
//! [`Plugins`] is the helper's component: a manager thread that reads the roster every `plugins.rescanSeconds` and keeps the
//! broker (and its socket) only while a plugin is on. With none on, the cost is that sleeping thread.
//!
//! Served at level 1: settings page and values, cards, tiles, notifications, facts. Not served (refused in words, and a plugin that
//! asks for them in its manifest stays inert): timeline sources, choice providers, secrets, images and surfaces, Home Assistant,
//! network and Bluetooth brokering, signals. Plugins run as their own users, but this TV's kernel has no user or network
//! namespaces and no seccomp, so the Show's sandbox is not reproduced: only reviewed plugins belong in `enabled.json`.

pub mod broker;
pub mod firstparty;
pub mod local;
pub mod roster;
pub mod routes;
pub mod session;

use crate::ctx::Ctx;
use parking_lot::{Condvar, Mutex};
use roster::Roster;
use session::Shared;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

struct Wake {
    pending: Mutex<bool>,
    changed: Condvar,
}

impl Wake {
    fn notify(&self) {
        *self.pending.lock() = true;
        self.changed.notify_all();
    }

    /// Sleeps up to `duration` or until notified; returns at once if a notification came since the last wait.
    fn wait(&self, duration: Duration) {
        let mut pending = self.pending.lock();
        if !*pending {
            self.changed.wait_for(&mut pending, duration);
        }
        *pending = false;
    }
}

pub struct Plugins {
    shared: Arc<Shared>,
    wake: Arc<Wake>,
    dir: PathBuf,
    socket: PathBuf,
    rescan: Duration,
    handshake: Duration,
    first_uid: u32,
}

impl Plugins {
    /// The component, not yet running: its routes and topics work, no thread exists.
    pub fn new(ctx: &Ctx) -> Arc<Self> {
        let cfg = &ctx.config.plugins;
        let shared = Shared::new(
            ctx.hub.clone(),
            ctx.settings.clone(),
            Duration::from_secs(cfg.answer_seconds),
            Arc::new(crate::timeutil::now_ms),
        );
        {
            let config = ctx.config.clone();
            shared.set_device_source(Arc::new(move || {
                crate::device::facts(
                    &config.device,
                    std::path::Path::new(&config.ai.sys_input_dir),
                )
                .to_json()
            }));
        }
        let wake = Arc::new(Wake {
            pending: Mutex::new(false),
            changed: Condvar::new(),
        });
        {
            let wake = wake.clone();
            ctx.stop.on_stop(move || wake.notify());
        }
        Arc::new(Self {
            shared,
            wake,
            dir: ctx.paths.plugins_dir(),
            socket: PathBuf::from(&cfg.socket),
            rescan: Duration::from_secs(cfg.rescan_seconds),
            handshake: Duration::from_secs(cfg.handshake_seconds),
            first_uid: cfg.first_uid,
        })
    }

    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    /// Reads the roster again now (the settings flow calls this after it writes `enabled.json`).
    pub fn rescan(&self) {
        self.wake.notify();
    }

    /// Starts the manager thread; it ends when the stop is requested, taking the broker with it.
    pub fn spawn(self: &Arc<Self>, ctx: &Ctx) -> std::io::Result<JoinHandle<()>> {
        let (this, stop) = (self.clone(), ctx.stop.clone());
        thread::Builder::new()
            .name("plugins".into())
            .spawn(move || this.manage(&stop))
    }

    fn manage(&self, stop: &crate::control::Stop) {
        let mut broker: Option<broker::Broker> = None;
        let mut last_note = String::new();
        while !stop.is_set() {
            let (roster, problem) = Roster::load(&self.dir, self.first_uid);
            self.shared.set_roster(&roster, problem.clone());
            let note = format!(
                "{} on, inert: {:?}, problem: {:?}",
                roster.members.len(),
                roster.inert,
                problem
            );
            if note != last_note {
                if roster.members.is_empty() && roster.inert.is_empty() && problem.is_none() {
                    crate::log_debug!("plugins: none switched on");
                } else {
                    crate::log_info!("plugins: {note}");
                }
                last_note = note;
            }
            match (roster.is_empty(), broker.take()) {
                (false, None) => {
                    let started = broker::Broker::start(
                        self.shared.clone(),
                        roster,
                        self.dir.clone(),
                        self.socket.clone(),
                        broker::Timing {
                            handshake: self.handshake,
                            first_uid: self.first_uid,
                        },
                    );
                    match started {
                        Ok(b) => {
                            crate::log_info!(
                                "plugins: broker listening on {}",
                                self.socket.display()
                            );
                            broker = Some(b);
                        }
                        Err(e) => crate::log_warn!(
                            "plugins: cannot listen on {}: {e} (tried again at the next scan)",
                            self.socket.display()
                        ),
                    }
                }
                (false, Some(b)) => {
                    b.set_roster(roster);
                    broker = Some(b);
                }
                (true, Some(b)) => {
                    crate::log_info!("plugins: none switched on, the broker ends");
                    b.stop("disabled");
                }
                (true, None) => {}
            }
            self.wake.wait(self.rescan);
        }
        if let Some(b) = broker {
            b.stop("stopping");
        }
    }
}
