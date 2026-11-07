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
//! | [`Guide`] | optional ([`Tuner::with_guide`]): a better guide than the TV's own for `/xmltv.xml`, made from the TV's own listings |
//! | [`Look`] | optional ([`Tuner::slates`]): the theme slates are drawn in (colours and faces); without one, streams have no slates |
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
//! | `GET /status.json` | who has the tuner, nothing tuned or asked of the TV: `{"holder": {"ip", "name", "channel", "channelName", "since"}}` for a client's stream (`name` as the host knows its address, `Plex` by user agent, else its user agent's product; `since` Unix ms), `{"holder": null}` while no client's stream has it |
//! | `GET /lineup.json` | the TV's antenna channels (`iepg/getChannelList`), one per number: `GuideNumber` `2.4`, `GuideName`, `URL`; a channel the owner left out ([`Tuner::hide`]) is not listed |
//! | `GET /lineup.m3u` | the same lineup as an M3U playlist: `tvg-id` and `tvg-chno` are the guide number, the URL is `/auto/v<number>`, `url-tvg` names `/xmltv.xml` |
//! | `GET /xmltv.xml` | the TV's own broadcast guide (`iepg/getEventList`, a day back to six days ahead, read once per channel and kept 30 minutes, or until the host's [`Guide`] changes) as XMLTV, UTC, channel ids the guide numbers, or the host's guide made from it when it has one; a channel nobody has listings for has no programmes |
//! | `GET /auto/v<number>` | MPEG-TS of that channel. With slates ([`slated`]): answered at once with a slate while the channel is tuned, then the programme from its first whole picture, a slate whenever its picture stops or breaks up, and a last slate before the end, on one program and one clock of the tuner's own ([`splice`]). Without: from its first whole picture, a PAT naming the one program, its PMT, then its clock, picture and sound from that picture on; 503 for a channel that cannot start, the reason saying which: no signal (no lock), weak signal (a locked multiplex whose tables did not come whole within 8 s or picture within 5 s, or that the demodulator counts packet errors on), not on air (a clean multiplex whose whole tables do not list it, or that carries no packets of its picture); 403 for a protected program. Either way the client whose stream has the tuner takes it over with its next request; another client gets 503 while it runs, and every client while the TV is in use (with slates: 200 and `last_word` of a slate saying who has it, `Tuner busy: Plex is recording …` or `The TV is using its tuner`, then the end); 404 for no such channel. Each refusal names its HDHomeRun error in `X-HDHomeRun-Error`; none is remembered |
//! | `GET`/`POST /lineup.post` | 200, nothing to start (the lineup is the TV's) |
//!
//! Pieces: [`psi`] reads and writes the transport stream's tables; [`docs`] makes the HDHomeRun documents, the playlist and the guide;
//! [`stream`] holds the lease on the one tuner and says what a stream is, over two small traits, so it runs in tests on a fake; [`slated`]
//! is a stream with slates, [`slate`] draws them and [`mpeg2`] codes them, [`splice`] is the program they and the programme share;
//! [`health`] is what each channel's streams and checks found; [`meter`] is the reading that helps aim the antenna; [`webos`] is LG's TV
//! (luna, the DVB nodes and the demodulator's reading); `server` is the listener; [`Tuner`] serves it and says what it is doing.

pub mod docs;
pub mod health;
pub mod meter;
pub mod mpeg2;
pub mod net;
pub mod psi;
mod server;
pub mod slate;
pub mod slated;
pub mod splice;
pub mod stream;
pub mod webos;

pub use health::{Health, Reception, Verdict};
pub use meter::Reading;
pub use net::Cidr;
pub use slate::{Look, Type};
pub use stream::{Clock, Failure, Grade, Guide, Hardware, Hold, Limits, Signal, Tap, real_clock};

use docs::Identity;
use health::Seen;
use meter::Meter;
use mpeg2::Format;
use parking_lot::Mutex;
use splice::Layout;
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
    /// Every channel's reception is being checked ([`Tuner::check_reception`]): RF channels done, of how many.
    Checking { done: usize, of: usize },
    /// The meter holds the tuner on an RF channel ([`Tuner::aim`]); anything asked for takes it.
    Aiming { rf: u8 },
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Off => f.write_str("Off"),
            Self::Idle => f.write_str("Idle"),
            Self::Streaming { guide, name } => write!(f, "Streaming {guide} {name}"),
            Self::Busy(why) => write!(f, "Busy: {why}"),
            Self::Checking { done, of } => write!(f, "Checking reception: {done} of {of} frequencies"),
            Self::Aiming { rf } => write!(f, "Measuring the signal on RF {rf}"),
        }
    }
}

struct Running {
    server: server::Server,
    facade: Arc<Facade>,
    config: Config,
}

/// The tuner: a listener while it is served, what it is doing, and who is told when that changes. Dropping it closes the listener.
pub struct Tuner {
    hw: Arc<dyn Hardware>,
    clock: Clock,
    limits: Limits,
    tuning: Arc<Tuning>,
    guide: Option<Arc<dyn Guide>>,
    running: Mutex<Option<Running>>,
    meter: Arc<Meter>,
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
            guide: None,
            running: Mutex::new(None),
            meter: Arc::default(),
        }
    }

    /// `/xmltv.xml` is `guide`'s document whenever it has one, the TV's own otherwise.
    pub fn with_guide(mut self, guide: Arc<dyn Guide>) -> Self {
        self.guide = Some(guide);
        self
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
        let facade = Arc::new(Facade::new(
            self.hw.clone(),
            identity,
            self.limits.clone(),
            self.clock.clone(),
            self.tuning.clone(),
            self.guide.clone(),
        ));
        let server = server::Server::start(config.addr, facade.clone(), config.allowed.clone())
            .inspect_err(|_| self.tuning.stop())?;
        let addr = server.local_addr();
        log::info!("tuner: sharing the tuner as {:?} at {addr}", config.name);
        self.tuning.serve();
        *running = Some(Running {
            server,
            facade,
            config: config.clone(),
        });
        Ok(addr)
    }

    /// Closes the listener and ends its stream and the meter; the TV is given back as they let go.
    pub fn stop(&self) {
        let running = self.running.lock().take();
        if let Some(r) = running {
            log::info!("tuner: no longer shared; closing {}", r.server.local_addr());
            r.server.stop();
        }
        self.tuning.stop();
        self.meter.stop();
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

    /// `wake` is called after each change of [`Tuner::status`] and of a channel's [`Health`] (it reads them itself: two changes close
    /// together may come as one).
    pub fn on_change(&self, wake: impl Fn() + Send + Sync + 'static) {
        self.tuning.on_change(Arc::new(wake));
    }

    /// Streams asked for from now on are served with slates drawn in the look `look` gives when each is asked for, or without them when
    /// it gives none.
    pub fn slates(&self, look: impl Fn() -> Option<Look> + Send + Sync + 'static) {
        self.tuning.options.lock().look = Some(Arc::new(look));
    }

    /// The channels (guide numbers) the owner leaves out of the lineup: the playlist and the documents do not list them.
    pub fn hide(&self, guides: impl IntoIterator<Item = String>) {
        self.tuning.options.lock().hidden = guides.into_iter().collect();
    }

    /// The names clients at these addresses go by in `/status.json` and on a refused request's slate (the host's Plex server).
    pub fn name_peers(&self, names: impl IntoIterator<Item = (std::net::IpAddr, String)>) {
        self.tuning.options.lock().names = names.into_iter().collect();
    }

    /// Starts the meter that helps aim the antenna ([`meter`]) on RF channel `rf`, or on the frequency the tuner is on (`None`), or moves
    /// it there: a reading about once a second ([`Tuner::aiming`]) until it is stopped, or nobody asked for one for a few seconds.
    /// Refused while the tuner is not shared.
    pub fn aim(&self, rf: Option<u8>) -> Result<(), Failure> {
        let facade = self.facade().ok_or_else(|| Failure::Hardware("the tuner is not shared".to_owned()))?;
        self.meter.aim(facade, rf);
        Ok(())
    }

    /// The meter's latest reading while it runs; asking keeps it running.
    pub fn aiming(&self) -> Option<Reading> {
        self.meter.reading((self.clock)())
    }

    /// The meter's best reading starts over from its next one.
    pub fn aim_afresh(&self) {
        self.meter.fresh();
    }

    /// Stops the meter: the tuner is let go of at once.
    pub fn stop_aiming(&self) {
        self.meter.stop();
    }

    fn facade(&self) -> Option<Arc<Facade>> {
        self.running.lock().as_ref().map(|r| r.facade.clone())
    }

    /// Every antenna channel of the TV, left out of the lineup or not, with how it came in the last time it was streamed or checked. Empty
    /// while the tuner is not served.
    pub fn reception(&self) -> Vec<Reception> {
        let Some(entries) = self.facade().and_then(|f| f.entries().ok()) else {
            return Vec::new();
        };
        let known = self.tuning.known();
        let hidden = self.tuning.options.lock().hidden.clone();
        entries
            .into_iter()
            .map(|e| {
                let k = known.get(&e.guide);
                let seen = k.and_then(|k| k.seen.as_ref());
                Reception {
                    rf: docs::parse_channel_id(&e.channel_id).map_or(0, |(rf, _, _)| rf),
                    hidden: hidden.contains(&e.guide),
                    health: k.and_then(|k| k.health.clone()),
                    format: seen.filter(|s| s.layout.video_type == 0x02).map(|s| s.format),
                    video: seen.map(|s| s.layout.video_type),
                    sounds: seen.map_or(0, |s| s.layout.audio.len()),
                    guide: e.guide,
                    name: e.name,
                }
            })
            .collect()
    }

    /// Gives back what a host kept of a channel from [`Tuner::reception`]: its health, its picture's stream type and, for an MPEG-2
    /// picture, its format and how many sounds its program has, which its next slate takes (a picture of another type is streamed
    /// without slates).
    pub fn recall(&self, guide: &str, health: Option<Health>, format: Option<Format>, sounds: usize, video: Option<u8>) {
        if let Some(health) = health {
            self.tuning.record(guide, health);
        }
        let video = video.unwrap_or(0x02);
        if format.is_some() || video != 0x02 {
            let mut layout = Layout::fallback(1);
            layout.video_type = video;
            layout.audio = vec![(0x81, Vec::new()); sounds.clamp(1, splice::MAX_AUDIO)];
            self.tuning.saw(guide, Seen { format: format.unwrap_or(Format::FALLBACK), layout, tsid: 1 });
        }
    }

    /// Checks how every channel comes in, one RF channel at a time, on a thread of its own: the status says how far it is, a stream asked
    /// for meanwhile takes the tuner, and the TV's own use or the tuner no longer being shared stops it. It takes the tuner from the meter.
    /// Refused while the tuner is not served, streams or checks already.
    pub fn check_reception(&self) -> Result<(), Failure> {
        let facade = self.facade().ok_or_else(|| Failure::Hardware("the tuner is not shared".to_owned()))?;
        if !matches!(self.status(), Status::Idle | Status::Busy(_) | Status::Aiming { .. }) {
            return Err(Failure::Busy);
        }
        std::thread::Builder::new()
            .name("tuner-check".to_owned())
            .spawn(move || match facade.check_all() {
                Ok(n) => log::info!("tuner: checked the reception of {n} channels"),
                Err(f) => log::info!("tuner: the reception check stopped: {}", f.reason()),
            })
            .map(drop)
            .map_err(|e| Failure::Hardware(e.to_string()))
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

    #[test]
    fn the_meter_runs_while_readings_are_asked_for_and_lets_go_of_the_tuner_when_they_stop() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), vec![]);
        let limits = Limits { meter_every: std::time::Duration::from_millis(2), ..Limits::default() };
        let tuner = Tuner::with_limits(Arc::new(Shared(fake.clone())), time.clock(), limits);
        assert!(tuner.aim(Some(32)).is_err(), "not shared, not aimed");
        tuner.serve(&config("LumaTV")).unwrap();
        tuner.aim(Some(32)).unwrap();
        let soon = |done: &dyn Fn() -> bool| {
            (0..1000).any(|_| {
                std::thread::sleep(std::time::Duration::from_millis(1));
                done()
            })
        };
        assert!(soon(&|| tuner.aiming().is_some_and(|r| r.tuned)));
        assert_eq!(tuner.status(), Status::Aiming { rf: 32 });
        // nobody asks for six seconds: the page went away without a word
        time.advance(std::time::Duration::from_secs(6));
        assert!(soon(&|| fake.released.load(Ordering::Relaxed) == 1));
        assert!(soon(&|| tuner.aiming().is_none()));
        assert_eq!(tuner.status(), Status::Idle);
    }
}
