//! The antenna tuner as an HDHomeRun: Plex's DVR (or Jellyfin, Emby, Tvheadend) finds the TV by its address, reads the lineup and records or
//! watches from it, without the TV being switched to Live TV and without a tuner box. The same lineup is also an IPTV playlist with an XMLTV
//! guide, for players that take those (VLC, Kodi, TiviMate, Jellyfin, Emby, Channels DVR). The stream path was measured on the set with
//! probes and through Plex (`docs/research/live-tv-capture.md`).
//!
//! It is a network entrance and the tuner is shared with the TV's own use, so both are the owner's: **off by default** (Settings, `base.livetv`),
//! and while off nothing is bound and the tuner is left alone. On, it listens on the TV's private address (`helper.json`, `hdhr`; never a public
//! one) and answers only the private networks. One tuner: one stream at a time, and the TV's own Live TV comes first: a stream does not start
//! while Live TV is in front, and a running one ends when Live TV takes the tuner.
//!
//! | Request | Answer |
//! |---|---|
//! | `GET /discover.json` | the device: name, id, `TunerCount` 1, `BaseURL`, `LineupURL` |
//! | `GET /lineup_status.json` | an antenna, not scanning |
//! | `GET /lineup.json` | the TV's antenna channels (`iepg/getChannelList`), one per number: `GuideNumber` `2.4`, `GuideName`, `URL` |
//! | `GET /lineup.m3u` | the same lineup as an M3U playlist: `tvg-id` and `tvg-chno` are the guide number, the URL is `/auto/v<number>`, `url-tvg` names `/xmltv.xml` |
//! | `GET /xmltv.xml` | the TV's own broadcast guide (`iepg/getEventList`, a day back to six days ahead, read once per channel and kept 30 minutes) as XMLTV, UTC, channel ids the guide numbers; a channel the TV has no listings for has no programmes |
//! | `GET /auto/v<number>` | MPEG-TS of that channel: a PAT naming the one program, then its PMT, clock, picture and sound; 503 while busy or the TV is in use; 403 for a protected program; 404 for no such channel |
//! | `GET`/`POST /lineup.post` | 200, nothing to start (the lineup is the TV's) |
//!
//! Pieces: [`psi`] reads and writes the transport stream's tables; [`docs`] makes the HDHomeRun documents; [`stream`] holds the lease on the one
//! tuner and says what a stream is, over two small traits, so it runs in tests on a fake; `hw` is the TV (luna and the DVB nodes); `server` is
//! the listener.

pub mod docs;
mod hw;
pub mod psi;
mod server;
pub mod stream;

pub use hw::Tv;

use crate::ctx::Ctx;
use crate::http::Cidr;
use crate::settings::wake_on_change;
use crate::sys;
use parking_lot::Mutex;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use stream::{Facade, Hardware, Limits};

/// How often a listener that should be up but is not tries again, and how often an up one checks that the TV still has its address.
const RETRY: Duration = Duration::from_secs(15);
const RECHECK: Duration = Duration::from_secs(60);

struct Running {
    server: server::Server,
    name: String,
    ip: Ipv4Addr,
}

#[derive(Default)]
struct State {
    running: Option<Running>,
    problem: Option<String>,
}

pub struct Hdhr {
    ctx: Ctx,
    hw: Arc<dyn Hardware>,
    state: Mutex<State>,
}

impl Hdhr {
    pub fn new(ctx: &Ctx, hw: Arc<dyn Hardware>) -> Arc<Self> {
        Arc::new(Self {
            ctx: ctx.clone(),
            hw,
            state: Mutex::default(),
        })
    }

    /// Where the listener is, while it is up.
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.state
            .lock()
            .running
            .as_ref()
            .map(|r| r.server.local_addr())
    }

    /// Why it is not up though the switch is on, if that is so.
    pub fn problem(&self) -> Option<String> {
        self.state.lock().problem.clone()
    }

    /// Makes the world match the switch: a listener exists exactly while the owner has the tuner shared, under the name they gave it, on the
    /// address the TV has now.
    pub fn reconcile(&self) {
        let settings = self.ctx.settings.livetv();
        let cfg = &self.ctx.config.hdhr;
        let mut state = self.state.lock();
        if !settings.enabled {
            if let Some(running) = state.running.take() {
                crate::log_info!(
                    "hdhr: the tuner is no longer shared; closing {}",
                    running.server.local_addr()
                );
                running.server.stop();
            }
            state.problem = None;
            return;
        }
        let ip = sys::lan_ipv4(cfg.bind.as_deref().and_then(|b| b.parse().ok()));
        if let (Some(running), Some(ip)) = (&state.running, ip)
            && running.name == settings.name
            && running.ip == ip
        {
            return;
        }
        if let Some(running) = state.running.take() {
            running.server.stop();
        }
        let Some(ip) = ip else {
            state.problem = Some("the TV has no private network address to listen on".to_owned());
            return;
        };
        match self.start(ip, &settings.name) {
            Ok(running) => {
                crate::log_info!(
                    "hdhr: sharing the tuner as {:?} at {}",
                    running.name,
                    running.server.local_addr()
                );
                state.running = Some(running);
                state.problem = None;
            }
            Err(why) => {
                if state.problem.as_deref() != Some(why.as_str()) {
                    crate::log_warn!("hdhr: cannot listen: {why}");
                }
                state.problem = Some(why);
            }
        }
    }

    fn start(&self, ip: Ipv4Addr, name: &str) -> Result<Running, String> {
        let cfg = &self.ctx.config.hdhr;
        let mut peers = Vec::new();
        for net in &cfg.allowed_nets {
            match Cidr::parse(net) {
                Some(cidr) => peers.push(cidr),
                None => {
                    crate::log_warn!("hdhr: allowedNets: {net:?} is not a network and is ignored")
                }
            }
        }
        let mac = std::fs::read_to_string(&cfg.mac_file).unwrap_or_default();
        let identity = docs::Identity {
            name: name.to_owned(),
            device_id: docs::device_id(&mac),
        };
        let facade = Facade::new(
            self.hw.clone(),
            identity,
            Limits::default(),
            stream::real_clock(),
        );
        let server = server::Server::start(
            SocketAddr::new(ip.into(), cfg.port),
            Arc::new(facade),
            peers,
        )
        .map_err(|e| e.to_string())?;
        Ok(Running {
            server,
            name: name.to_owned(),
            ip,
        })
    }

    fn shut_down(&self) {
        if let Some(running) = self.state.lock().running.take() {
            running.server.stop();
        }
    }

    /// The thread: looks at the switch after every change of it, and again every minute (the TV's address can move).
    pub fn spawn(self: &Arc<Self>) -> io::Result<JoinHandle<()>> {
        let (tx, rx) = mpsc::channel::<()>();
        {
            let tx = tx.clone();
            wake_on_change(
                &self.ctx.settings,
                "base.livetv",
                &self.ctx.stop,
                move || {
                    let _ = tx.send(());
                },
            );
        }
        self.ctx.stop.on_stop(move || {
            let _ = tx.send(());
        });
        let me = self.clone();
        thread::Builder::new()
            .name("hdhr-switch".to_owned())
            .stack_size(256 * 1024)
            .spawn(move || {
                loop {
                    me.reconcile();
                    let wait = if me.problem().is_some() {
                        RETRY
                    } else {
                        RECHECK
                    };
                    if matches!(rx.recv_timeout(wait), Err(RecvTimeoutError::Disconnected))
                        || me.ctx.stop.is_set()
                    {
                        break;
                    }
                }
                me.shut_down();
            })
    }
}

impl Drop for Hdhr {
    fn drop(&mut self) {
        self.shut_down();
    }
}

#[cfg(test)]
mod tests {
    use super::stream::tests::{Shared, Time, fake_with, rf32};
    use super::*;
    use crate::config::{Config, HdhrConfig, Paths};
    use crate::control::Stop;
    use crate::hub::Hub;
    use crate::settings::SettingsStore;
    use settings_schema::{SettingChange, Value};
    use std::io::{Read, Write};
    use std::net::TcpStream;

    fn ctx(dir: &std::path::Path) -> Ctx {
        let hub = Arc::new(Hub::new(1));
        let settings = Arc::new(SettingsStore::new(dir.join("settings"), hub.clone()));
        settings.register_builtin().unwrap();
        let hdhr = HdhrConfig {
            port: 0,
            bind: Some("127.0.0.1".to_owned()),
            allowed_nets: vec!["127.0.0.0/8".to_owned()],
            mac_file: dir.join("mac").display().to_string(),
            ..HdhrConfig::default()
        };
        Ctx {
            config: Arc::new(Config {
                hdhr,
                ..Config::default()
            }),
            paths: Paths::new(dir),
            hub,
            settings,
            stop: Stop::new(),
        }
    }

    fn set(ctx: &Ctx, key: &str, value: Value) {
        ctx.settings
            .apply(
                "base.livetv",
                vec![SettingChange {
                    key: key.parse().unwrap(),
                    value,
                }],
                None,
            )
            .unwrap();
    }

    fn name_served(addr: SocketAddr) -> String {
        let mut c = TcpStream::connect(addr).unwrap();
        c.write_all(b"GET /discover.json HTTP/1.1\r\n\r\n").unwrap();
        let mut answer = String::new();
        c.read_to_string(&mut answer).unwrap();
        let body = answer.split("\r\n\r\n").nth(1).unwrap();
        serde_json::from_str::<serde_json::Value>(body).unwrap()["FriendlyName"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn hdhr(ctx: &Ctx) -> Arc<Hdhr> {
        let time = Time::default();
        Hdhr::new(ctx, Arc::new(Shared(fake_with(&time, rf32(&[]), vec![]))))
    }

    #[test]
    fn it_is_off_until_the_owner_turns_it_on_and_nothing_listens_while_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(dir.path());
        assert!(!ctx.settings.livetv().enabled);
        let hdhr = hdhr(&ctx);
        hdhr.reconcile();
        assert_eq!((hdhr.local_addr(), hdhr.problem()), (None, None));
    }

    #[test]
    fn the_switch_opens_and_closes_the_listener_and_the_name_follows_the_setting() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(dir.path());
        let hdhr = hdhr(&ctx);
        set(&ctx, "enabled", Value::Bool(true));
        hdhr.reconcile();
        let first = hdhr.local_addr().expect("listening once switched on");
        assert_eq!(name_served(first), "LumaTV");
        hdhr.reconcile();
        assert_eq!(
            hdhr.local_addr(),
            Some(first),
            "nothing changed: the same listener"
        );
        set(&ctx, "name", Value::Text("Den".to_owned()));
        hdhr.reconcile();
        assert_eq!(name_served(hdhr.local_addr().unwrap()), "Den");
        set(&ctx, "enabled", Value::Bool(false));
        hdhr.reconcile();
        assert_eq!(hdhr.local_addr(), None);
    }
}
