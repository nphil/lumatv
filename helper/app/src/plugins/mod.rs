//! The plugin broker: LumaShow's plugin API (level 1) on the TV.
//!
//! A plugin is a separate program that talks only to the helper, over a unix socket, and never draws: it declares a settings page,
//! cards, quick tiles and notifications, and the TV's shell draws them (`docs/plugin-api.md` in LumaShow is the protocol; the Rust
//! types are its `plugin-api` crate, used here as it is, and its `lumashow-sdk` is what the tests speak with).
//!
//! | Part | What |
//! |---|---|
//! | [`roster`] | which plugins are on (`plugins/enabled.json`, written by the owner's command line, and its `firstParty` by Settings) and who each is |
//! | [`session`] | one plugin's conversation without a socket: checks, topics, settings, actions |
//! | [`broker`] | the socket, one reader and one writer thread per connection, rate and size limits |
//! | [`local`] | a plugin of the helper's own, in the same conversation with no socket (the TV's first-party plugins), and one that asks first |
//! | [`firstparty`] | the helper's first-party plugins: Home's weather, news and clock, and the ambient light tile |
//! | [`ha`] | the Home Assistant lease: the one connection, held by the plugins that require it, and what each may watch and call |
//! | [`install`] | switching a plugin on and off and linking Home Assistant: the owner's command line (`plugin-enable`, `ha-link`) |
//! | [`runner`] | starts the programs of the plugins that are on, as their own users, and again when they stop |
//! | [`routes`] | `/v1/plugins`: ask a plugin to run an action, page through the choices of its settings, refresh the lease's link, switch one of the helper's own that asks first |
//!
//! [`Plugins`] is the helper's component: a manager thread that reads the roster every `plugins.rescanSeconds` and keeps the
//! broker (and its socket) only while a plugin is on. With none on, the cost is that sleeping thread.
//!
//! Served at level 1: settings page and values (with choice providers), cards, tiles, notifications, facts, the Home Assistant lease
//! (`lease:ha`, `ha.read`, `ha.call`) and the occupancy signal. Not served (refused in words, and a plugin that asks for them in its
//! manifest stays inert): timeline sources, secrets of a plugin's own, images and surfaces, network and Bluetooth brokering, and
//! any call on a lock, an alarm, a camera, a cover or a valve. Plugins run as their own users, but this TV's kernel has no user or
//! network namespaces and no seccomp, so the Show's sandbox is not reproduced: only reviewed plugins belong in `enabled.json`.

pub mod broker;
pub mod firstparty;
pub mod ha;
pub mod install;
pub mod local;
pub mod roster;
pub mod routes;
pub mod runner;
pub mod session;

use crate::ctx::Ctx;
use ha::{HaLease, LeaseSettings};
use parking_lot::{Condvar, Mutex};
use roster::Roster;
use session::Shared;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Why one of the helper's own plugins was not switched.
#[derive(Debug, PartialEq, Eq)]
pub enum SwitchError {
    /// No plugin of the helper's own asks first by that id.
    NotOffered,
    /// The code is not the one of the words it asks with now: they changed since they were shown.
    Consent,
    /// `enabled.json` could not be read or written, in words.
    File(String),
}

struct Wake {
    pending: Mutex<bool>,
    changed: Condvar,
}

impl Wake {
    fn notify(&self) {
        *self.pending.lock() = true;
        self.changed.notify_all();
    }

    /// Sleeps up to `duration` or until notified; returns at once if a notification came since the last wait. True when it was
    /// notified.
    fn wait(&self, duration: Duration) -> bool {
        let mut pending = self.pending.lock();
        if !*pending {
            self.changed.wait_for(&mut pending, duration);
        }
        std::mem::take(&mut *pending)
    }
}

pub struct Plugins {
    shared: Arc<Shared>,
    lease: Arc<HaLease>,
    wake: Arc<Wake>,
    dir: PathBuf,
    socket: PathBuf,
    rescan: Duration,
    handshake: Duration,
    first_uid: u32,
    /// One write of `enabled.json` at a time.
    writing: Mutex<()>,
}

impl Plugins {
    /// The component, not yet running: its routes and topics work, no thread exists. Home Assistant is dialled only on the home
    /// network.
    pub fn new(ctx: &Ctx) -> Arc<Self> {
        Self::with_policy(ctx, ha_link::AddressPolicy::HOME)
    }

    /// [`Plugins::new`] with the addresses the Home Assistant lease may dial (a test's fake Home Assistant is on the loopback).
    pub fn with_policy(ctx: &Ctx, policy: ha_link::AddressPolicy) -> Arc<Self> {
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
        let lease = Arc::new(HaLease::new(LeaseSettings::system(
            ctx.paths.ha_link_file(),
            policy,
            crate::voice::tls_for(&ctx.config.fetch),
            Arc::new(|line| crate::log_info!("{line}")),
        )));
        {
            // Where the lease is, for the TV's Settings and Home: the saved address and whether a token is kept, never the token.
            let hub = ctx.hub.clone();
            lease.on_info(Arc::new(move |info| {
                hub.set(
                    "ha.link",
                    serde_json::json!({"state": info.state, "detail": info.detail, "address": info.address,
                                       "token": info.token, "holders": info.holders}),
                );
            }));
        }
        shared.set_lease(lease.clone());
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
            lease,
            wake,
            dir: ctx.paths.plugins_dir(),
            socket: PathBuf::from(&cfg.socket),
            rescan: Duration::from_secs(cfg.rescan_seconds),
            handshake: Duration::from_secs(cfg.handshake_seconds),
            first_uid: cfg.first_uid,
            writing: Mutex::new(()),
        })
    }

    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    pub fn lease(&self) -> &Arc<HaLease> {
        &self.lease
    }

    /// Reads the roster again now (the settings flow calls this after it writes `enabled.json`).
    pub fn rescan(&self) {
        self.wake.notify();
    }

    /// Switches `id`, one of the helper's own plugins that ask first, as the owner did in Settings: on takes the code of the words the
    /// owner was shown (`approve`), off takes nothing. `enabled.json`'s `firstParty` keeps it, and the roster is read again.
    pub fn switch(&self, id: &str, on: bool, approve: Option<&str>) -> Result<(), SwitchError> {
        let code = self.shared.approval_of(id).ok_or(SwitchError::NotOffered)?;
        if on && !approve.is_some_and(|a| a.eq_ignore_ascii_case(&code)) {
            return Err(SwitchError::Consent);
        }
        {
            let _one = self.writing.lock();
            let mut file = roster::read_file(&self.dir).map_err(SwitchError::File)?;
            file.first_party.retain(|a| a.id != id);
            if on {
                file.first_party.push(roster::Approved {
                    id: id.to_owned(),
                    approve: code,
                });
            }
            roster::write_file(&self.dir, &file).map_err(SwitchError::File)?;
        }
        crate::log_info!("plugins: {id} switched {} in Settings", if on { "on" } else { "off" });
        self.rescan();
        Ok(())
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
        let mut runner = runner::Runner::new(self.dir.clone(), self.shared.clone());
        let mut last_note = String::new();
        let mut scanned: Option<Instant> = None;
        let mut woken = false;
        while !stop.is_set() {
            // The roster is read again when asked and every `rescan`; between, the runner only looks at its programs.
            if !woken && scanned.is_some_and(|at| at.elapsed() < self.rescan) {
                runner.tick();
                let left = self
                    .rescan
                    .saturating_sub(scanned.map_or(Duration::ZERO, |at| at.elapsed()));
                woken = self.wake.wait(runner.wait_hint(left));
                continue;
            }
            scanned = Some(Instant::now());
            let (roster, problem) = Roster::load(&self.dir, self.first_uid);
            let members = roster.clone();
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
            // The programs follow the roster once the broker's socket is there for them to connect to (none is started for a
            // broker that could not listen: they would only wait at a door that is not there).
            runner.sync(&if broker.is_some() {
                members
            } else {
                Roster::default()
            });
            woken = self.wake.wait(runner.wait_hint(self.rescan));
        }
        if let Some(b) = broker {
            b.stop("stopping");
        }
        runner.stop_all(Duration::from_secs(2));
    }
}
