//! The antenna tuner of an LG webOS TV as an HDHomeRun: Plex's DVR (or Jellyfin, Emby, Channels DVR) finds the TV by its address, reads the
//! lineup and records or watches from it, without the TV being switched to Live TV and without a tuner box. The same lineup is also an IPTV
//! playlist with an XMLTV guide, for players that take those (VLC, Kodi, TiviMate, Jellyfin, Emby, Channels DVR). The stream path was
//! measured on the set with probes and through Plex (`docs/research/live-tv-capture.md`).
//!
//! The engine has no host in it. What a host gives it:
//!
//! | Hook | What |
//! |---|---|
//! | [`Hardware`] | the TV: its channel list and guide, whether its own use has the tuner, a tap on the tuned multiplex; [`webos::Tv`] is LG's |
//! | [`webos::Luna`] | the luna bus [`webos::Tv`] speaks through: a call, and a call that stays connected; [`webos::LunaSend`] runs the set's `luna-send` |
//! | [`Config`] | the name players show, the device id, the address and port to listen on, the networks that are answered |
//! | [`Clock`] | monotonic time: [`real_clock`], or a test's that moves only when told |
//! | the `log` facade | what it served and why it refused, under the target `lumatuner` |
//!
//! It is a network entrance and the tuner is shared with the TV's own use, so a host serves it only when the owner says so, on a private
//! address of the TV. One tuner: one stream at a time, and the TV's own Live TV comes first: a stream does not start while Live TV has the
//! tuner, and a running one ends when Live TV takes it.
//!
//! | Request | Answer |
//! |---|---|
//! | `GET /discover.json` | the device: name, id, `TunerCount` 1, `BaseURL`, `LineupURL` |
//! | `GET /lineup_status.json` | an antenna, not scanning |
//! | `GET /lineup.json` | the TV's antenna channels (`iepg/getChannelList`), one per number: `GuideNumber` `2.4`, `GuideName`, `URL` |
//! | `GET /lineup.m3u` | the same lineup as an M3U playlist: `tvg-id` and `tvg-chno` are the guide number, the URL is `/auto/v<number>`, `url-tvg` names `/xmltv.xml` |
//! | `GET /xmltv.xml` | the TV's own broadcast guide (`iepg/getEventList`, a day back to six days ahead, read once per channel and kept 30 minutes) as XMLTV, UTC, channel ids the guide numbers; a channel the TV has no listings for has no programmes |
//! | `GET /auto/v<number>` | MPEG-TS of that channel: a PAT naming the one program, then its PMT, clock, picture and sound. The client whose stream has the tuner takes it over with its next request; another client gets 503 while it runs, and every client while the TV is in use; 403 for a protected program; 404 for no such channel |
//! | `GET`/`POST /lineup.post` | 200, nothing to start (the lineup is the TV's) |
//!
//! Pieces: [`psi`] reads and writes the transport stream's tables; [`docs`] makes the HDHomeRun documents, the playlist and the guide;
//! [`stream`] holds the lease on the one tuner and says what a stream is, over two small traits, so it runs in tests on a fake; [`webos`]
//! is LG's TV (luna and the DVB nodes); `server` is the listener; [`Tuner`] serves it and says what it is doing.

pub mod docs;
pub mod net;
pub mod psi;
mod server;
pub mod stream;
pub mod webos;

pub use net::Cidr;
pub use stream::{Clock, Failure, Hardware, Limits, Tap, real_clock};

use docs::Identity;
use parking_lot::Mutex;
use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use stream::{Facade, Tuning};

/// How the tuner is offered: the host reads these from wherever it keeps its settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// What players show (`FriendlyName`).
    pub name: String,
    /// What players keep the tuner by, so it must be the same on every start: [`docs::device_id`] of the TV's own hardware address.
    pub device_id: String,
    /// Where to listen: a private address of the TV (never a public one), and the port (5004, the one the other emulators use).
    pub addr: SocketAddr,
    /// The networks whose peers are answered; any other peer is dropped unanswered.
    pub allowed: Vec<Cidr>,
}

/// What the tuner is doing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// Nothing listens.
    #[default]
    Off,
    /// Listening, and the tuner is free.
    Idle,
    /// One channel is streamed: its guide number (`2.4`) and name.
    Streaming { guide: String, name: String },
    /// The TV's own use has the tuner (Live TV in front, or running with the panel off): a stream asked for now is refused.
    Busy(String),
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Off => f.write_str("Off"),
            Self::Idle => f.write_str("Idle"),
            Self::Streaming { guide, name } => write!(f, "Streaming {guide} {name}"),
            Self::Busy(why) => write!(f, "Busy: {why}"),
        }
    }
}

struct Running {
    server: server::Server,
    config: Config,
}

/// The tuner: a listener while it is served, what it is doing, and who is told when that changes. Dropping it closes the listener.
pub struct Tuner {
    hw: Arc<dyn Hardware>,
    clock: Clock,
    limits: Limits,
    tuning: Arc<Tuning>,
    running: Mutex<Option<Running>>,
}

impl Tuner {
    pub fn new(hw: Arc<dyn Hardware>, clock: Clock) -> Self {
        Self::with_limits(hw, clock, Limits::default())
    }

    /// [`Tuner::new`] with the waits and counts of a test.
    pub fn with_limits(hw: Arc<dyn Hardware>, clock: Clock, limits: Limits) -> Self {
        Self {
            hw,
            clock,
            limits,
            tuning: Arc::new(Tuning::default()),
            running: Mutex::new(None),
        }
    }

    /// Listens as `config` says. A listener that is up with the same config is kept; one with another is closed first, which ends its
    /// stream.
    pub fn serve(&self, config: &Config) -> io::Result<SocketAddr> {
        let mut running = self.running.lock();
        if let Some(r) = running.as_ref()
            && r.config == *config
        {
            return Ok(r.server.local_addr());
        }
        if let Some(r) = running.take() {
            r.server.stop();
        }
        let identity = Identity {
            name: config.name.clone(),
            device_id: config.device_id.clone(),
        };
        let facade = Facade::new(
            self.hw.clone(),
            identity,
            self.limits.clone(),
            self.clock.clone(),
            self.tuning.clone(),
        );
        let server = server::Server::start(config.addr, Arc::new(facade), config.allowed.clone())
            .inspect_err(|_| self.tuning.stop())?;
        let addr = server.local_addr();
        log::info!("tuner: sharing the tuner as {:?} at {addr}", config.name);
        self.tuning.serve();
        *running = Some(Running {
            server,
            config: config.clone(),
        });
        Ok(addr)
    }

    /// Closes the listener and ends its stream; the TV is given back as the stream lets go.
    pub fn stop(&self) {
        let running = self.running.lock().take();
        if let Some(r) = running {
            log::info!("tuner: no longer shared; closing {}", r.server.local_addr());
            r.server.stop();
        }
        self.tuning.stop();
    }

    /// Where it listens, while it is served.
    pub fn addr(&self) -> Option<SocketAddr> {
        self.running.lock().as_ref().map(|r| r.server.local_addr())
    }

    pub fn status(&self) -> Status {
        self.tuning.status()
    }

    /// Asks the TV whether its own use has the tuner, unless a stream of this tuner has it (a running stream asks by itself), and says
    /// what the tuner is doing then. The tuner is not touched: it is two questions on the luna bus.
    pub fn check(&self) -> Status {
        if matches!(self.status(), Status::Idle | Status::Busy(_)) {
            self.tuning.checked(self.hw.in_use());
        }
        self.status()
    }

    /// `wake` is called after each change of [`Tuner::status`] (it reads the status itself: two changes close together may come as one).
    pub fn on_change(&self, wake: impl Fn() + Send + Sync + 'static) {
        self.tuning.on_change(Arc::new(wake));
    }
}

impl Drop for Tuner {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::stream::tests::{Shared, Time, fake_with, rf32};
    use super::*;
    use std::io::{Read, Write};
    use std::net::{Ipv4Addr, TcpStream};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn config(name: &str) -> Config {
        Config {
            name: name.to_owned(),
            device_id: "0A1B2C3D".to_owned(),
            addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            allowed: vec![Cidr::parse("127.0.0.0/8").unwrap()],
        }
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

    #[test]
    fn nothing_listens_until_it_is_served_and_the_listener_follows_the_config() {
        let time = Time::default();
        let tuner = Tuner::new(
            Arc::new(Shared(fake_with(&time, rf32(&[]), vec![]))),
            time.clock(),
        );
        let changes = Arc::new(AtomicUsize::new(0));
        {
            let changes = changes.clone();
            tuner.on_change(move || {
                changes.fetch_add(1, Ordering::Relaxed);
            });
        }
        assert_eq!((tuner.addr(), tuner.status()), (None, Status::Off));
        let first = tuner.serve(&config("LumaTV")).unwrap();
        assert_eq!((tuner.addr(), tuner.status()), (Some(first), Status::Idle));
        assert_eq!(name_served(first), "LumaTV");
        assert_eq!(
            tuner.serve(&config("LumaTV")).unwrap(),
            first,
            "nothing changed: the same listener"
        );
        let renamed = tuner.serve(&config("Den")).unwrap();
        assert_eq!(name_served(renamed), "Den");
        tuner.stop();
        assert_eq!((tuner.addr(), tuner.status()), (None, Status::Off));
        assert!(TcpStream::connect(renamed).is_err(), "closed");
        assert_eq!(changes.load(Ordering::Relaxed), 2, "served, then off");
    }

    #[test]
    fn a_check_says_busy_while_the_tvs_own_use_has_the_tuner_and_idle_once_it_lets_go() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), vec![]);
        let tuner = Tuner::new(Arc::new(Shared(fake.clone())), time.clock());
        *fake.owner.lock() = Some("Live TV is in front".into());
        assert_eq!(tuner.check(), Status::Off, "not served: nothing is asked");
        tuner.serve(&config("LumaTV")).unwrap();
        assert_eq!(tuner.check(), Status::Busy("Live TV is in front".into()));
        *fake.owner.lock() = None;
        assert_eq!(tuner.check(), Status::Idle);
        assert_eq!(
            tuner.status().to_string(),
            "Idle",
            "and in words: Off, Idle, Streaming 2.4 WEST, Busy: <why>"
        );
    }
}
