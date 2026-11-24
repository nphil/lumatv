//! One tuner, one stream at a time: the lineup, the lease on the tuner, what a stream is (a PAT naming the one program, then that
//! program's packets) and what the tuner is doing. The hardware is behind two small traits so all of it runs in tests with a scripted fake
//! and a fake clock.
//!
//! A client counts one tuner, so a new request from the client whose stream has the tuner means it gave that stream up: Plex asks for
//! the next channel before it closes the last one. That request takes the tuner over: the old stream is told to end, and the TV is let
//! go before it is tuned again. A stream that is over is only letting go of the TV (a pipeline to close, the channel to put back), so a
//! request then waits for it too. Whoever starts last has the tuner otherwise: another client's running stream (not Plex's, which is a
//! recording) is told to end and who took it, and the TV's own Live TV is asked to give way ([`Hardware::yield_to`]) unless the request is
//! Plex's; `/status.json` says for [`DISPLACED_FOR_MS`] who was displaced and by whom.

use crate::Status;
use crate::docs::{self, Entry, Identity};
use crate::health::{Health, Known, Seen, Tables, Verdict, lost_before};
use crate::mpeg2::Format;
use crate::psi::{self, PACKET};
use crate::slate::{Look, Says};
use crate::slated::Slated;
use crate::transcode::{Breaker, Codec, Fault, Profile, Stage};
use parking_lot::{Condvar, Mutex};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

/// Why a stream could not start, or had to end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The lineup has no such number.
    UnknownChannel,
    /// Another stream has the tuner (`TunerCount` is 1).
    Busy,
    /// The TV's own use has the tuner: the reason, for the owner's log and the client's body.
    TvInUse(String),
    /// The tuner did not lock.
    NoSignal(String),
    /// The tuner locked, but what the channel needs did not come whole in time, or the demodulator counts packet errors on the
    /// multiplex: the signal, not the station.
    WeakSignal(String),
    /// A clean multiplex whose whole tables or packets say the channel is not there.
    OffAir(String),
    /// The program is scrambled or asks not to be passed on.
    Protected,
    /// The tuner, the demux or the luna call said no.
    Hardware(String),
    /// `?transcode=` names a profile the converter does not have (an HDHomeRun's error 802).
    UnknownProfile(String),
    /// The converter failed while the stream ran: the stream ends, and the next request is served as the station sends it.
    Converter(Fault),
}

impl Failure {
    /// The HTTP status of a stream that could not start. A tuner in use is 503, which is what an HDHomeRun answers and what Plex waits on.
    pub fn status(&self) -> u16 {
        match self {
            Self::UnknownChannel => 404,
            Self::Busy
            | Self::TvInUse(_)
            | Self::NoSignal(_)
            | Self::WeakSignal(_)
            | Self::OffAir(_) => 503,
            Self::Protected => 403,
            Self::UnknownProfile(_) => 400,
            Self::Hardware(_) | Self::Converter(_) => 502,
        }
    }

    pub fn reason(&self) -> String {
        match self {
            Self::UnknownChannel => "no such channel".to_owned(),
            Self::Busy => "all tuners in use".to_owned(),
            Self::TvInUse(why) => format!("the TV is in use: {why}"),
            Self::NoSignal(why) => format!("no signal: {why}"),
            Self::WeakSignal(why) => format!("weak signal: {why}"),
            Self::OffAir(why) => format!("not on air: {why}"),
            Self::Protected => "the program is protected".to_owned(),
            Self::Hardware(why) => format!("tuner failure: {why}"),
            Self::UnknownProfile(name) => format!("no such transcode profile: {name}"),
            Self::Converter(fault) => format!("converter failure: {fault}"),
        }
    }

    /// The error an HDHomeRun names in `X-HDHomeRun-Error`, which players read the cause from (SiliconDust's HTTP API, 2026-07-14).
    pub fn hdhomerun_error(&self) -> &'static str {
        match self {
            Self::UnknownChannel => "801 Unknown Channel",
            Self::Busy | Self::TvInUse(_) => "805 All Tuners In Use",
            Self::Hardware(_) | Self::Converter(_) => "806 Tune Failed",
            Self::NoSignal(_) | Self::WeakSignal(_) | Self::OffAir(_) => "807 No Video Data",
            Self::Protected => "811 Content Protection Required",
            Self::UnknownProfile(_) => "802 Unknown Transcode Profile",
        }
    }
}

/// The TV.
pub trait Hardware: Send + Sync + 'static {
    /// LG's channel list (`iepg/getChannelList`'s reply).
    fn channel_list(&self) -> Result<Value, Failure>;
    /// The programmes the TV's guide has for LG's `channel_id` (`iepg/getEventList`'s reply): read only, the tuner is not touched.
    fn programmes(&self, channel_id: &str) -> Result<Value, Failure>;
    /// Who else has the tuner, if anyone: Live TV in front, or a pipeline of its own.
    fn in_use(&self) -> Option<String>;
    /// Tunes LG's `channel_id` and returns the tap of what the antenna carries; the TV's own channel stays as it was. Dropping the tap
    /// lets go of the demux, then of the tuner, which may be kept a moment for the next tune ([`Hardware::release`]).
    fn tune(&self, channel_id: &str) -> Result<Box<dyn Tap>, Failure>;
    /// Lets go at once of a tuner kept for the next tune: it is no longer shared.
    fn release(&self) {}
    /// What the demodulator says of the frequency it is tuned to, if the TV says: read, nothing is touched. A slate shows it.
    fn signal(&self) -> Option<Signal> {
        None
    }
    /// Moves the tuner to LG's `channel_id` for the meter and keeps it there: nothing is routed and the lock is not waited for, so a
    /// frequency that does not lock is read too. Dropping the hold lets go as a tap does.
    fn hold(&self, channel_id: &str) -> Result<Box<dyn Hold>, Failure> {
        let _ = channel_id;
        Err(Failure::Hardware("this TV's tuner cannot be held on a channel".to_owned()))
    }
    /// The TV's last channel, LG's: what the demodulator reads while nothing of this tuner has it.
    fn tuned_to(&self) -> Option<String> {
        None
    }
    /// What the TV is called, for the clients it displaces.
    fn tv_name(&self) -> String {
        "The TV".to_owned()
    }
    /// The device `by` took the tuner while the TV's own Live TV had it: tells the viewer (a toast naming `by`), leaves Live TV for the TV's
    /// home and lets go of the tuner. `true` when the TV has given it up; `false` when it is not Live TV that has it or it cannot.
    fn yield_to(&self, by: &str) -> bool {
        let _ = by;
        false
    }
}

/// What the TV's demodulator says of the frequency it is tuned to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Signal {
    /// It holds a signal.
    pub locked: bool,
    /// Its mean square error against the constellation (`MSE`) while it holds a signal: lower is cleaner. Read on the set on 2026-11-05:
    /// 23 to 37 on clean multiplexes, 345 to 365 on RF 29.
    pub mse: Option<u32>,
    /// Signal to noise in dB, when it says.
    pub snr: Option<f32>,
    /// The packets it could not correct lately (`Packet Error`, 0 to 255 on the set), when it says: 0 on a clean multiplex.
    pub errors: Option<u32>,
    /// The bit errors it counted lately (`BER`), when it says: 0 on a clean multiplex, 320,000 to 585,000 on RF 29.
    pub ber: Option<u32>,
}

/// How a signal would show a picture, for someone aiming the antenna.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Grade {
    /// No lock.
    None,
    /// Packets lost every second: a picture that breaks up.
    Poor,
    /// A picture with little to spare: a few packets lost, or no margin left.
    Fair,
    Good,
}

impl Grade {
    pub fn word(&self) -> &'static str {
        match self {
            Self::None => "No signal",
            Self::Poor => "Poor",
            Self::Fair => "Fair",
            Self::Good => "Good",
        }
    }
}

impl Signal {
    /// How strong the signal is, 0 to 1: from 16 dB, where an ATSC picture starts breaking up, to 30 dB.
    pub fn strength(&self) -> f32 {
        match (self.locked, self.snr) {
            (true, Some(snr)) => ((snr - 16.0) / 14.0).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }

    /// The signal in a word. Measured on the set on 2026-11-05: RF 32 at 28 to 29 dB and RF 25 at 24 dB lost nothing; RF 29 at 18 to 20 dB
    /// lost up to 4% of its packets; RF 21 at 18 dB once brought no whole picture in 12 s.
    pub fn quality(&self) -> &'static str {
        match (self.locked, self.snr) {
            (false, _) => "none",
            (true, None) => "locked",
            (true, Some(s)) if s < 16.0 => "none",
            (true, Some(s)) if s < 20.0 => "weak",
            (true, Some(s)) if s < 24.0 => "fair",
            (true, Some(s)) if s < 28.0 => "good",
            _ => "excellent",
        }
    }

    /// The meter's verdict: good is no packet errors at 23 dB or more (clean multiplexes read 28 to 30 dB on the set); fair is a few
    /// errors, or none with less margin; poor is 20 or more a reading, or under 19 dB (RF 29 read 81 to 255 at 17.8 dB on 2026-11-05).
    pub fn grade(&self) -> Grade {
        let errors = self.errors.unwrap_or(0);
        let snr = self.snr.unwrap_or(f32::MAX);
        match () {
            _ if !self.locked => Grade::None,
            _ if errors >= 20 || snr < 19.0 => Grade::Poor,
            _ if errors > 0 || snr < 23.0 => Grade::Fair,
            _ => Grade::Good,
        }
    }

    /// Whether this reading is better than `other`: its grade first, then its strength.
    pub fn better_than(&self, other: &Signal) -> bool {
        (self.grade(), self.strength()) > (other.grade(), other.strength())
    }

    /// What a person is told: `Signal 18 dB: weak`, or that there is none.
    pub fn words(&self) -> String {
        match (self.locked, self.snr) {
            (false, _) => "No signal on this channel's frequency".to_owned(),
            (true, None) => "The tuner holds the frequency".to_owned(),
            (true, Some(snr)) => format!("Signal {snr:.0} dB: {}", self.quality()),
        }
    }
}

/// The tuner held on a channel for the meter ([`Hardware::hold`]).
pub trait Hold: Send {
    /// Whether the TV has taken the tuner from it, and why.
    fn taken(&mut self) -> Option<String>;
}

/// The antenna's multiplex through the demux.
pub trait Tap: Send {
    /// One whole section (`pid`, `table_id`, `section_number`) within `wait`.
    fn section(&mut self, pid: u16, table_id: u8, number: u8, wait: Duration) -> Option<Vec<u8>>;
    /// Routes these PIDs to the download; `read` gives their packets from here on. Routing again replaces the last routing and what it
    /// brought.
    fn start(&mut self, pids: &[u16]) -> Result<(), Failure>;
    /// Appends the whole 188-byte packets that arrived within `wait` (none is fine) to `out`.
    fn read(&mut self, out: &mut Vec<u8>, wait: Duration) -> Result<(), Failure>;
    /// Starts waiting for one section without blocking; [`Tap::arrived`] gives it once it has come. Asking again for one already asked
    /// for changes nothing.
    fn want(&mut self, pid: u16, table_id: u8, number: u8);
    /// The section asked for with [`Tap::want`], if it has come: it is not waited for.
    fn arrived(&mut self, pid: u16, table_id: u8, number: u8) -> Option<Vec<u8>>;
    /// Stops waiting for what was asked for with [`Tap::want`].
    fn forget(&mut self);
    /// Whether the TV has taken the tuner from this tap, and why. Measured on the set: when Live TV opens, LG's own pipeline manager
    /// unrealizes the helper's at once, and the demux goes on giving whatever the frontend is on (the same PIDs on the next multiplex), so
    /// the packets themselves do not say it.
    fn taken(&mut self) -> Option<String>;
}

/// A guide the host has for `/xmltv.xml` that is better than the TV's own (online listings laid over it): the host decides, per document,
/// whether it has one.
pub trait Guide: Send + Sync {
    /// The XMLTV document for the lineup, given each channel with the TV's own listings for it (`iepg/getEventList`'s reply, `Null` for a
    /// channel the TV could not answer for); `None` serves the TV's own.
    fn xmltv(&self, channels: &[(Entry, Value)]) -> Option<String>;
    /// Changes whenever the documents [`Guide::xmltv`] makes would: a kept document of another revision is made again.
    fn revision(&self) -> u64;
}

/// The time the facade waits and counts, one place so a test can set them small.
#[derive(Clone, Debug)]
pub struct Limits {
    /// How long a read channel list is kept.
    pub lineup_ttl: Duration,
    /// How long a built guide is kept: reading it is one luna call per channel.
    pub guide_ttl: Duration,
    /// How long the tables are read before the channel is given up. A clean multiplex gives them within a second (the PAT comes every 100
    /// ms, the PMT and the virtual channel table every 400 ms at most); a weak one loses whole sections, so they are read again, the
    /// channel table's sections kept across repeats, until this has passed. On the weak RF 29 on 2026-11-05 they took up to 4.8 s, and
    /// at times did not come in 6 s.
    pub section_wait: Duration,
    /// How long the program's video (or clock) PID may be silent before the stream ends: the signal is gone. It is not what notices Live TV
    /// taking the tuner: stations use the same PIDs, so the packets of another multiplex look the same (`Tap::taken` and `in_use` notice).
    pub silence: Duration,
    /// How often a running stream asks whether the TV has been taken for its own use.
    pub owner_every: Duration,
    /// How often the one-program PAT is sent.
    pub pat_every: Duration,
    /// The longest one read waits.
    pub poll: Duration,
    /// How long a request waits for a stream it took over, or one that is over, to let go of the TV.
    pub release: Duration,
    /// How long the program's first whole picture may take once the program is routed before the channel is given up: the 5 s an
    /// HDHomeRun gives a program. Measured on the set on 2026-11-05 over its 59 channels: the first picture packet came 0.1 to 0.7 s after
    /// routing, the first whole picture up to 3.0 s (30.1).
    pub first_picture: Duration,
    /// With slates: how long the tuning slate stands before it says the signal is weak and how strong it is.
    pub weak_after: Duration,
    /// With slates: how long a channel whose picture's shape is not known yet (never streamed nor checked) holds its first slate for its
    /// first sequence header, so that the slate is the picture's size and rate from its first frame: Plex restarts its transcode ("Error
    /// reinitializing filters") when the size changes within a stream. The first whole picture came up to 3.0 s after routing on the set
    /// on 2026-11-05 (30.1), routing about 0.7 s after the request. A shape still unknown then is the fallback's. At most `weak_after`.
    pub shape_wait: Duration,
    /// With slates: how long the programme's picture may stop before the slate stands in for it (its packets come every few
    /// milliseconds).
    pub dropout: Duration,
    /// With slates: the share of the picture's packets lost within a second that brings the slate in (a second of 16.1 that lost a
    /// quarter of them on 2026-11-05 was its last before the signal went), and the share a second may lose for the picture to come back.
    pub loss_weak: f32,
    pub loss_clear: f32,
    /// With slates: how long a channel whose picture has not come is waited for, and one whose picture stopped, before the stream ends.
    pub give_up_tuning: Duration,
    pub give_up: Duration,
    /// With slates: how long the last slate (no signal, the TV using its tuner) stands at the end of the stream.
    pub last_word: Duration,
    /// How long a check of every channel listens to the pictures of each RF channel: several of their whole pictures.
    pub listen: Duration,
    /// The meter: how often it reads the demodulator, and how long it goes on after a reading was last asked for.
    pub meter_every: Duration,
    pub meter_lease: Duration,
    /// With a converter ([`crate::transcode`]): how long pictures may go in without one coming back before the stream ends, and the
    /// longest one may take through it: past either the converter is stuck, or too slow for a live stream.
    pub convert_stall: Duration,
    pub convert_lag: Duration,
    /// How long a converter that failed rests: the requests meanwhile are served as the station sends them.
    pub convert_rest: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            lineup_ttl: Duration::from_secs(60),
            section_wait: Duration::from_secs(8),
            silence: Duration::from_secs(5),
            owner_every: Duration::from_secs(5),
            pat_every: Duration::from_millis(100),
            poll: Duration::from_millis(200),
            guide_ttl: Duration::from_secs(30 * 60),
            release: Duration::from_secs(5),
            first_picture: Duration::from_secs(5),
            weak_after: Duration::from_secs(8),
            shape_wait: Duration::from_secs(4),
            dropout: Duration::from_millis(1500),
            loss_weak: 0.2,
            loss_clear: 0.05,
            give_up_tuning: Duration::from_secs(45),
            give_up: Duration::from_secs(120),
            last_word: Duration::from_secs(6),
            listen: Duration::from_secs(3),
            meter_every: Duration::from_secs(1),
            meter_lease: Duration::from_secs(5),
            convert_stall: Duration::from_secs(2),
            convert_lag: Duration::from_millis(1500),
            convert_rest: Duration::from_secs(60),
        }
    }
}

/// Time since some start; the real one is the monotonic clock.
pub type Clock = Arc<dyn Fn() -> Duration + Send + Sync>;

pub fn real_clock() -> Clock {
    let start = Instant::now();
    Arc::new(move || start.elapsed())
}

/// Who has the tuner: the client whose stream it is, the flag that ends that stream, whether the stream is over and only letting go
/// of the TV, what the stream is (none for the meter and the reception check) and, once another client has displaced it, who.
struct Holding {
    peer: IpAddr,
    replaced: Arc<AtomicBool>,
    over: bool,
    claim: Option<Claim>,
    taker: Option<String>,
}

/// How long `/status.json` keeps saying a client was displaced.
pub(crate) const DISPLACED_FOR_MS: u64 = 60_000;

/// A client whose stream another device (or the TV's own Live TV) took the tuner from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Displaced {
    pub(crate) peer: IpAddr,
    /// The displaced client, as a player knows it.
    pub(crate) name: String,
    pub(crate) by: String,
    /// `tv` for the TV's own Live TV, `app` for a device on the network.
    pub(crate) by_kind: &'static str,
    pub(crate) at_ms: u64,
    pub(crate) guide: String,
    pub(crate) channel_name: String,
}

/// A client's stream that has the tuner, as `/status.json` and a refused request's slate say it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Claim {
    pub(crate) peer: IpAddr,
    /// The request's `User-Agent`.
    pub(crate) agent: Option<String>,
    /// The device name the client gave (`?device=` or `X-Luma-Device`).
    pub(crate) device: Option<String>,
    /// A recording (Plex's stream): another client does not take the tuner from it.
    pub(crate) recording: bool,
    pub(crate) guide: String,
    pub(crate) name: String,
    pub(crate) channel_id: String,
    /// Unix milliseconds.
    pub(crate) since_ms: u64,
}

/// The tuner, held: dropping it frees it and wakes whoever waits for it.
pub(crate) struct Lease {
    tuning: Arc<Tuning>,
    /// Set when another request takes the tuner over.
    pub(crate) replaced: Arc<AtomicBool>,
}

impl Lease {
    /// The stream is over: what is left is letting go of the TV, which a new request waits for instead of being refused.
    pub(crate) fn over(&self) {
        if let Some(h) = self.tuning.held.lock().as_mut() {
            h.over = true;
        }
    }

    /// The device that displaced this stream, when another client did (the same client asking again is not that).
    pub(crate) fn taker(&self) -> Option<String> {
        self.tuning.held.lock().as_ref().and_then(|h| h.taker.clone())
    }

    /// The TV's own Live TV took the tuner from this stream: noted for the client to read.
    pub(crate) fn taken_by_tv(&self, tv: &str) {
        let claim = self.tuning.held.lock().as_ref().and_then(|h| h.claim.clone());
        if let Some(claim) = claim {
            self.tuning.displaced(&claim, tv, "tv");
        }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        *self.tuning.held.lock() = None;
        self.tuning.freed.notify_all();
    }
}

/// What every facade of one tuner shares: who holds it (a listener started again cannot tune while the last one's stream still lets go)
/// and the wake-up of whoever waits for it, what it is doing, with the host's wake-up for a change, how its streams are served and what it
/// remembers of each channel.
#[derive(Default)]
pub(crate) struct Tuning {
    held: Mutex<Option<Holding>>,
    freed: Condvar,
    status: Mutex<Status>,
    wake: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    pub(crate) options: Mutex<Options>,
    pub(crate) breaker: Breaker,
    known: Mutex<HashMap<String, Known>>,
    taken: Mutex<Vec<Displaced>>,
}

/// Where a stream's slates get their look: asked once for each request, so a change of the host's theme reaches the next stream; `None`
/// serves it without slates.
pub(crate) type LookSource = Arc<dyn Fn() -> Option<Look> + Send + Sync>;

/// How streams are served: with slates or without (no source); the channels the owner left out of the lineup; the names of clients the
/// host knows by their address (its Plex server); and what makes H.264 for `?transcode=` (none serves every request as the station sends it).
#[derive(Clone, Default)]
pub(crate) struct Options {
    pub(crate) look: Option<LookSource>,
    pub(crate) hidden: HashSet<String>,
    pub(crate) names: HashMap<IpAddr, String>,
    pub(crate) codec: Option<Arc<dyn Codec>>,
}

impl Tuning {
    /// How channel `guide` came in, and the host told.
    pub(crate) fn record(&self, guide: &str, health: Health) {
        self.known.lock().entry(guide.to_owned()).or_default().health = Some(health);
        self.tell();
    }

    /// What channel `guide`'s picture and program were like.
    pub(crate) fn saw(&self, guide: &str, seen: Seen) {
        self.known.lock().entry(guide.to_owned()).or_default().seen = Some(seen);
    }

    pub(crate) fn seen(&self, guide: &str) -> Option<Seen> {
        self.known.lock().get(guide).and_then(|k| k.seen.clone())
    }

    /// Notes channel `guide`'s program as its tables say; the host is told when that is not what was noted.
    pub(crate) fn learned(&self, guide: &str, found: &Found) {
        let tables = Tables::of(found);
        let changed = {
            let mut known = self.known.lock();
            let noted = &mut known.entry(guide.to_owned()).or_default().tables;
            (noted.as_ref() != Some(&tables)).then(|| *noted = Some(tables)).is_some()
        };
        if changed {
            self.tell();
        }
    }

    /// Takes in the tables a host kept of channel `guide`.
    pub(crate) fn noted(&self, guide: &str, tables: Tables) {
        self.known.lock().entry(guide.to_owned()).or_default().tables = Some(tables);
    }

    /// Forgets channel `guide`'s tables: they led to no picture.
    pub(crate) fn unlearn(&self, guide: &str) {
        let forgot = self.known.lock().get_mut(guide).is_some_and(|k| k.tables.take().is_some());
        if forgot {
            self.tell();
        }
    }

    fn tables(&self, guide: &str) -> Option<Tables> {
        self.known.lock().get(guide).and_then(|k| k.tables.clone())
    }

    pub(crate) fn known(&self) -> HashMap<String, Known> {
        self.known.lock().clone()
    }

    fn tell(&self) {
        let wake = self.wake.lock().clone();
        if let Some(wake) = wake {
            wake();
        }
    }
}

impl Tuning {
    /// Takes the tuner for `peer`, who is nobody's stream (the meter, the reception check): a stream of the same client, or one that is
    /// over, is told to end and waited for, at most `wait`; a client's running stream keeps it.
    pub(crate) fn take(self: &Arc<Self>, peer: IpAddr, wait: Duration) -> Result<Lease, Failure> {
        self.acquire(peer, None, wait)
    }

    /// Takes the tuner for a client's stream, `by` being what it is called. As [`Tuning::take`], and the last to start wins: another
    /// client's running stream that is not a recording is told to end, who took it noted, and waited for.
    pub(crate) fn take_stream(self: &Arc<Self>, claim: Claim, by: String, wait: Duration) -> Result<Lease, Failure> {
        self.acquire(claim.peer, Some((claim, by)), wait)
    }

    fn acquire(self: &Arc<Self>, peer: IpAddr, taker: Option<(Claim, String)>, wait: Duration) -> Result<Lease, Failure> {
        let deadline = Instant::now() + wait;
        let mut held = self.held.lock();
        loop {
            match held.as_mut() {
                None => {
                    let replaced = Arc::new(AtomicBool::new(false));
                    *held = Some(Holding {
                        peer,
                        replaced: replaced.clone(),
                        over: false,
                        claim: taker.as_ref().map(|(c, _)| c.clone()),
                        taker: None,
                    });
                    if taker.is_some() {
                        self.taken.lock().retain(|d| d.peer != peer);
                    }
                    return Ok(Lease {
                        tuning: self.clone(),
                        replaced,
                    });
                }
                Some(h) if h.over || h.peer == peer => h.replaced.store(true, Ordering::Release),
                Some(h) if taker.is_some() && h.claim.as_ref().is_some_and(|c| !c.recording) => {
                    if !h.replaced.swap(true, Ordering::AcqRel) {
                        let (_, by) = taker.as_ref().expect("a stream's taker");
                        h.taker = Some(by.clone());
                        if let Some(old) = &h.claim {
                            self.displaced(old, by, "app");
                        }
                    }
                }
                Some(_) => return Err(Failure::Busy),
            }
            if self.freed.wait_until(&mut held, deadline).timed_out() && held.is_some() {
                return Err(Failure::Busy);
            }
        }
    }

    /// Notes that `by` (of `kind`) took the tuner from `old`'s stream.
    fn displaced(&self, old: &Claim, by: &str, kind: &'static str) {
        let name = client_name(old, &self.options.lock().names).unwrap_or_else(|| old.peer.to_string());
        let mut taken = self.taken.lock();
        taken.retain(|d| d.peer != old.peer);
        taken.push(Displaced {
            peer: old.peer,
            name,
            by: by.to_owned(),
            by_kind: kind,
            at_ms: crate::webos::unix_ms(),
            guide: old.guide.clone(),
            channel_name: old.name.clone(),
        });
    }

    /// The clients displaced within [`DISPLACED_FOR_MS`] of `now_ms`.
    pub(crate) fn displaced_since(&self, now_ms: u64) -> Vec<Displaced> {
        let mut taken = self.taken.lock();
        taken.retain(|d| now_ms.saturating_sub(d.at_ms) < DISPLACED_FOR_MS);
        taken.clone()
    }

    /// The client's stream that has the tuner, if one has it and is not letting go of it.
    pub(crate) fn holder(&self) -> Option<Claim> {
        self.held.lock().as_ref().filter(|h| !h.over).and_then(|h| h.claim.clone())
    }

    pub(crate) fn status(&self) -> Status {
        self.status.lock().clone()
    }

    pub(crate) fn on_change(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        *self.wake.lock() = Some(wake);
    }

    /// Moves to what `next` makes of the status now (`None`: it stays) and wakes the host when that is a change. The host is woken outside
    /// the lock and reads the status itself, so wake-ups that cross still leave it with the last one.
    pub(crate) fn change(&self, next: impl FnOnce(&Status) -> Option<Status>) {
        let changed = {
            let mut now = self.status.lock();
            match next(&now) {
                Some(status) if status != *now => {
                    *now = status;
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.tell();
        }
    }

    pub(crate) fn serve(&self) {
        self.change(|now| (*now == Status::Off).then_some(Status::Idle));
    }

    pub(crate) fn stop(&self) {
        self.change(|_| Some(Status::Off));
    }

    /// What the TV said of its own use while no stream of this tuner has the tuner.
    pub(crate) fn checked(&self, in_use: Option<String>) {
        self.change(|now| {
            matches!(now, Status::Idle | Status::Busy(_))
                .then(|| in_use.map_or(Status::Idle, Status::Busy))
        });
    }

    pub(crate) fn refused(&self, why: &str) {
        self.change(|now| (*now != Status::Off).then(|| Status::Busy(why.to_owned())));
    }

    pub(crate) fn streaming(&self, entry: &Entry) {
        self.change(|now| {
            (*now != Status::Off).then(|| Status::Streaming {
                guide: entry.guide.clone(),
                name: entry.name.clone(),
            })
        });
    }
}

/// Says what the tuner does once a stream is over: idle, or busy when the TV taking it back is what ended it. A tuner switched off
/// meanwhile stays off.
pub(crate) struct Over {
    pub(crate) tuning: Arc<Tuning>,
    pub(crate) then: Status,
}

impl Drop for Over {
    fn drop(&mut self) {
        let then = std::mem::replace(&mut self.then, Status::Idle);
        self.tuning
            .change(|now| matches!(now, Status::Streaming { .. }).then_some(then));
    }
}

/// The channels of a multiplex from its virtual channel table. Sections that come whole go into `have` (the first says how many there are)
/// and stay there across calls, so on a weak multiplex a repeat only has to bring the sections the last one lost; `None` until all are in.
pub(crate) fn channels_of(
    tap: &mut dyn Tap,
    have: &mut Vec<Option<Vec<u8>>>,
    wait: Duration,
) -> Option<Vec<psi::Channel>> {
    if have.is_empty() {
        let first = tap.section(psi::PID_PSIP, psi::TID_TVCT, 0, wait)?;
        *have = vec![None; usize::from(*first.get(7)?) + 1];
        have[0] = Some(first);
    }
    for (n, section) in (0..=u8::MAX).zip(have.iter_mut()) {
        if section.is_none() {
            *section = tap.section(psi::PID_PSIP, psi::TID_TVCT, n, wait);
        }
    }
    let mut all = Vec::new();
    for section in have.iter() {
        all.extend(psi::parse_tvct(section.as_deref()?)?);
    }
    Some(all)
}

/// Why the program of a channel was not found: something that did not come whole in time, or something read whole that says it is not
/// there.
pub(crate) enum Unfound {
    Missing(&'static str),
    Absent(&'static str),
}

pub(crate) struct Facade {
    pub(crate) hw: Arc<dyn Hardware>,
    pub(crate) identity: Identity,
    pub(crate) limits: Limits,
    pub(crate) clock: Clock,
    lineup: Mutex<Option<(Duration, Vec<Entry>)>>,
    /// The host's guide, when it has one: `/xmltv.xml` is its document when it gives one.
    source: Option<Arc<dyn Guide>>,
    /// The document served, when it was made and from which revision of the host's guide.
    guide: Mutex<Option<(Duration, u64, Arc<str>)>>,
    pub(crate) tuning: Arc<Tuning>,
}

impl Facade {
    pub(crate) fn new(
        hw: Arc<dyn Hardware>,
        identity: Identity,
        limits: Limits,
        clock: Clock,
        tuning: Arc<Tuning>,
        source: Option<Arc<dyn Guide>>,
    ) -> Self {
        Self {
            hw,
            identity,
            limits,
            clock,
            lineup: Mutex::new(None),
            source,
            guide: Mutex::new(None),
            tuning,
        }
    }

    /// The TV's antenna channels. Read again after a minute; a TV that does not answer leaves the last list standing.
    pub(crate) fn entries(&self) -> Result<Vec<Entry>, Failure> {
        let now = (self.clock)();
        let mut kept = self.lineup.lock();
        if let Some((at, entries)) = kept.as_ref()
            && now.saturating_sub(*at) < self.limits.lineup_ttl
        {
            return Ok(entries.clone());
        }
        match self.hw.channel_list() {
            Ok(reply) => {
                let entries = docs::entries(&reply);
                *kept = Some((now, entries.clone()));
                Ok(entries)
            }
            Err(f) => kept.as_ref().map(|(_, e)| e.clone()).ok_or(f),
        }
    }

    /// The lineup the documents list: the TV's antenna channels less those the owner left out.
    pub(crate) fn lineup(&self) -> Result<Vec<Entry>, Failure> {
        let mut entries = self.entries()?;
        let hidden = &self.tuning.options.lock().hidden;
        entries.retain(|e| !hidden.contains(&e.guide));
        Ok(entries)
    }
    /// The XMLTV guide of the lineup, from the TV's own listings, one read per channel, or the host's guide made from them when it has one.
    /// Kept for `guide_ttl`, or until the host's guide changes: a client that asks again gets the same document, and one that asks while it
    /// is being read waits for it. A channel the TV cannot answer for is listed without its own programmes; only when no channel could be
    /// read at all is it a failure (and then the last guide stands, if there is one).
    pub(crate) fn guide(&self) -> Result<Arc<str>, Failure> {
        let mut kept = self.guide.lock();
        let now = (self.clock)();
        let revision = self.source.as_ref().map_or(0, |g| g.revision());
        if let Some((at, made_from, xml)) = kept.as_ref()
            && now.saturating_sub(*at) < self.limits.guide_ttl
            && *made_from == revision
        {
            return Ok(xml.clone());
        }
        let built = self.lineup().and_then(|entries| {
            let mut first_failure = None;
            let mut read = 0;
            let channels: Vec<_> = entries
                .into_iter()
                .map(|e| {
                    let reply = match self.hw.programmes(&e.channel_id) {
                        Ok(reply) => {
                            read += 1;
                            reply
                        }
                        Err(f) => {
                            first_failure.get_or_insert(f);
                            Value::Null
                        }
                    };
                    (e, reply)
                })
                .collect();
            match first_failure {
                Some(f) if read == 0 => Err(f),
                _ => Ok(Arc::<str>::from(
                    self.source
                        .as_ref()
                        .and_then(|g| g.xmltv(&channels))
                        .unwrap_or_else(|| docs::xmltv(&channels)),
                )),
            }
        });
        match built {
            Ok(xml) => {
                *kept = Some((now, revision, xml.clone()));
                Ok(xml)
            }
            Err(f) => kept.as_ref().map(|(_, _, x)| x.clone()).ok_or(f),
        }
    }

    fn entry(&self, guide: &str) -> Result<Entry, Failure> {
        self.lineup()?
            .into_iter()
            .find(|e| e.guide == guide)
            .ok_or(Failure::UnknownChannel)
    }

    /// Takes the tuner for `entry` for the client at `peer`: what a refusal answers is checked here, while nothing has been sent to the
    /// client and a failure is still an HTTP status. The last to start has the tuner: another client's stream gives way (told who took it),
    /// and so does the TV's own Live TV, to a device but not to Plex, whose streams are recordings.
    fn take_for(&self, entry: &Entry, peer: IpAddr, agent: Option<&str>, device: Option<&str>) -> Result<Lease, Failure> {
        let mut claim = Claim {
            peer,
            agent: agent.map(str::to_owned),
            device: device.map(str::to_owned),
            recording: false,
            guide: entry.guide.clone(),
            name: entry.name.clone(),
            channel_id: entry.channel_id.clone(),
            since_ms: crate::webos::unix_ms(),
        };
        let who = client_name(&claim, &self.tuning.options.lock().names);
        claim.recording = who.as_deref() == Some("Plex");
        let by = who.unwrap_or_else(|| peer.to_string());
        let lease = self.tuning.take_stream(claim.clone(), by.clone(), self.limits.release)?;
        if let Some(why) = self.hw.in_use()
            && (claim.recording || !self.hw.yield_to(&by))
        {
            self.tuning.refused(&why);
            return Err(Failure::TvInUse(why));
        }
        Ok(lease)
    }

    /// The stream for a request. With the owner's slates on, and a channel whose picture a slate can stand in for (MPEG-2, or not seen
    /// yet), it is answered at once and tuned while the slate shows ([`Facade::play`]), and a tuner in use is answered with a short slate
    /// that says who has it ([`Facade::refuse`]); otherwise it is tuned here and answered from its first whole picture, and a channel that
    /// cannot start is still an HTTP status. A request for H.264 (`transcode`, with a converter installed) is not answered with a slate,
    /// whose pictures are MPEG-2. Nothing of a failure is kept but the channel's health: the next request tunes again.
    pub(crate) fn admit(&self, guide: &str, peer: IpAddr, agent: Option<&str>, device: Option<&str>, transcode: Option<&str>) -> Result<Admitted, Failure> {
        let entry = self.entry(guide)?;
        let convert = self.conversion(transcode)?;
        let source = self.tuning.options.lock().look.clone();
        let look = source
            .and_then(|look| look())
            .filter(|_| self.tuning.seen(guide).is_none_or(|s| s.layout.video_type == 0x02));
        match (self.take_for(&entry, peer, agent, device), look) {
            (Ok(lease), Some(look)) if convert.is_none() => Ok(Admitted::Slated(Slated { entry, lease, look })),
            (Ok(lease), _) => self.found(entry, lease).map(|mut stream| {
                if let Some(Conversion { codec, profile }) = convert {
                    self.engage(&mut stream, codec.as_ref(), profile);
                }
                Admitted::Plain(Box::new(stream))
            }),
            (Err(failure @ (Failure::Busy | Failure::TvInUse(_))), Some(look)) => {
                let says = Says::Busy(matches!(failure, Failure::Busy).then(|| self.busy_line()));
                Ok(Admitted::Refused(Refusal { entry, look, says, failure }))
            }
            (Err(failure), _) => Err(failure),
        }
    }

    /// What a request's `?transcode=` asks for: nothing without one, or while no converter is installed (the request is served as the
    /// station sends it, as before); a profile that is not one of SiliconDust's is an error 802.
    fn conversion(&self, transcode: Option<&str>) -> Result<Option<Conversion>, Failure> {
        let Some(name) = transcode else { return Ok(None) };
        let Some(codec) = self.tuning.options.lock().codec.clone() else {
            return Ok(None);
        };
        let profile = Profile::parse(name).ok_or_else(|| Failure::UnknownProfile(name.to_owned()))?;
        Ok(Some(Conversion { codec, profile }))
    }

    /// Makes `stream` H.264 unless the converter is resting or cannot start: the stream is then as the station sends it.
    fn engage(&self, stream: &mut Stream, codec: &dyn Codec, profile: Profile) {
        let now = (self.clock)();
        if !self.tuning.breaker.allows(now) {
            log::info!("tuner: {} is served as the station sends it: the converter is resting", stream.guide);
            return;
        }
        match stream.convert(codec, profile) {
            Ok(()) => log::info!("tuner: {} is converted to H.264 ({})", stream.guide, profile.name()),
            Err(Fault::Unsupported(why)) => log::info!("tuner: {} is served as the station sends it: {why}", stream.guide),
            Err(fault) => {
                log::warn!("tuner: {} is served as the station sends it: the converter did not start: {fault}", stream.guide);
                self.tuning.breaker.failed(&fault, now, self.limits.convert_rest);
            }
        }
    }

    /// What a refused request's slate says of the client whose stream has the tuner.
    fn busy_line(&self) -> String {
        let Some(claim) = self.tuning.holder() else {
            return "In use by another player".to_owned();
        };
        let names = self.tuning.options.lock().names.clone();
        let who = client_name(&claim, &names);
        let programme = (who.as_deref() == Some("Plex"))
            .then(|| self.hw.programmes(&claim.channel_id).ok())
            .flatten()
            .and_then(|reply| docs::showing(&reply, &claim.channel_id, &crate::webos::lg_stamp(crate::webos::unix_ms())));
        busy_line(&claim, who.as_deref(), programme.as_deref())
    }

    /// `/status.json`: the client whose stream has the tuner, or `null` while none has it, and the clients whose stream another device or
    /// the TV took the tuner from in the last [`DISPLACED_FOR_MS`]. Nothing is asked of the TV.
    pub(crate) fn status_doc(&self) -> Value {
        self.status_at(crate::webos::unix_ms())
    }

    fn status_at(&self, now_ms: u64) -> Value {
        let names = self.tuning.options.lock().names.clone();
        let holder = self.tuning.holder().map(|c| {
            serde_json::json!({
                "ip": c.peer.to_string(),
                "name": client_name(&c, &names),
                "channel": c.guide,
                "channelName": c.name,
                "since": c.since_ms,
            })
        });
        let displaced: Vec<Value> = self
            .tuning
            .displaced_since(now_ms)
            .into_iter()
            .map(|d| {
                serde_json::json!({
                    "ip": d.peer.to_string(),
                    "name": d.name,
                    "by": d.by,
                    "byKind": d.by_kind,
                    "at": d.at_ms,
                    "channel": d.guide,
                    "channelName": d.channel_name,
                })
            })
            .collect();
        serde_json::json!({ "holder": holder, "displaced": displaced })
    }

    /// A request without slates: the tuner taken, the channel tuned, its program found and its first whole picture waited for.
    #[cfg(test)]
    pub(crate) fn open(&self, guide: &str, peer: IpAddr) -> Result<Stream, Failure> {
        let entry = self.entry(guide)?;
        let lease = self.take_for(&entry, peer, None, None)?;
        self.found(entry, lease)
    }

    fn found(&self, entry: Entry, lease: Lease) -> Result<Stream, Failure> {
        let found = self.find(&entry, lease);
        if let Err(f) = &found {
            self.not_found(&entry.guide, f);
        }
        found
    }

    /// A channel that could not start for its signal or its station is recorded as having no signal; the next request tunes again.
    pub(crate) fn not_found(&self, guide: &str, failure: &Failure) {
        if matches!(failure, Failure::NoSignal(_) | Failure::WeakSignal(_) | Failure::OffAir(_)) {
            self.tuning.record(
                guide,
                Health {
                    at: SystemTime::now(),
                    verdict: Verdict::NoSignal,
                    signal: None,
                    lost: 0.0,
                    picture: None,
                    dropouts: 0,
                    checked: false,
                },
            );
        }
    }

    /// Starts channel `entry` from the tables its last stream (or a check) read, as soon as the tuner has locked: `pids` of its program
    /// are routed at once, so its picture gathers while the live tables are read and checked against these ([`Verify`]). `None` when
    /// nothing is known of the channel.
    pub(crate) fn cached_start(&self, entry: &Entry, tap: &mut dyn Tap, pids: fn(&Found) -> Option<Vec<u16>>) -> Option<(Found, Start)> {
        let tables = self.tuning.tables(&entry.guide)?;
        let found = tables.found(entry)?;
        let pids = pids(&found)?;
        tap.start(&pids).ok()?;
        let verify = Verify::new(entry.clone(), tables, (self.clock)() + self.limits.section_wait);
        Some((found, Start { early: Some(pids), verify: Some(verify) }))
    }

    /// Routes `pids`, unless they are the ones routed `early`: what came since is kept.
    pub(crate) fn route_found(tap: &mut dyn Tap, early: Option<&[u16]>, pids: &[u16]) -> Result<(), Failure> {
        if early != Some(pids) {
            tap.start(pids)?;
        }
        Ok(())
    }

    /// Tunes `entry`'s channel, finds its program in the multiplex, routes it and waits for its first whole picture. A request of the same
    /// client that comes meanwhile takes the tuner over: this one gives up between steps (Plex asks for the next channel while the last is
    /// still tuned when channels are flipped, and its transcoder asks again for a stream that ended).
    fn find(&self, entry: &Entry, lease: Lease) -> Result<Stream, Failure> {
        let given_up = || {
            lease
                .replaced
                .load(Ordering::Acquire)
                .then_some(Failure::Busy)
        };
        if let Some(f) = given_up() {
            return Err(f);
        }
        let asked = (self.clock)();
        let mut tap = self.hw.tune(&entry.channel_id)?;
        let tuned = (self.clock)();
        let (found, start) = match self.cached_start(entry, tap.as_mut(), |f| Some(plain_pids(f))) {
            Some(cached) => cached,
            None => (self.read_program(entry, tap.as_mut(), tuned, &given_up)?, Start::default()),
        };
        if psi::protected(&found.pmt, Some(&found.channel)) {
            return Err(Failure::Protected);
        }
        self.route(entry, tap, found, lease, (asked, tuned), start)
    }

    /// Reads `entry`'s tables in rounds from the lock on until they name the channel or `section_wait` has passed: a weak multiplex loses
    /// whole sections (on RF 29 on 2026-11-05 they took up to 4.8 s, and at times did not come in 6 s), and the previous multiplex's
    /// could still be on the demux for a moment (not seen on the set: the first PAT after the lock was always the new multiplex's).
    fn read_program(&self, entry: &Entry, tap: &mut dyn Tap, tuned: Duration, given_up: &dyn Fn() -> Option<Failure>) -> Result<Found, Failure> {
        let mut sections = Vec::new();
        let mut why = Unfound::Missing("no whole channel table");
        let found = loop {
            if let Some(f) = given_up() {
                return Err(f);
            }
            let left = self
                .limits
                .section_wait
                .saturating_sub((self.clock)().saturating_sub(tuned));
            if left.is_zero() {
                return Err(self.unfound(entry, why));
            }
            match program(tap, entry, &mut sections, left) {
                Ok(found) => break found,
                Err(not) => why = not,
            }
        };
        self.tuning.learned(&entry.guide, &found);
        Ok(found)
    }

    /// Routes the program `found` on `tap` and waits for its first whole picture: the stream without slates from there. `tuned` is when
    /// the stream was asked for and when the tuner locked, `begun` what was routed then and the tables still being checked
    /// ([`Facade::cached_start`]): a program that turns out to be another before the picture is in is routed again.
    pub(crate) fn route(
        &self,
        entry: &Entry,
        mut tap: Box<dyn Tap>,
        found: Found,
        lease: Lease,
        (asked, tuned): (Duration, Duration),
        begun: Start,
    ) -> Result<Stream, Failure> {
        let given_up = || {
            lease
                .replaced
                .load(Ordering::Acquire)
                .then_some(Failure::Busy)
        };
        let Start { early, mut verify } = begun;
        let mut found = found;
        let mut pids = plain_pids(&found);
        // the program's picture tells whether the tuner is still where it was; a program without one is watched by its clock
        let (mut picture, mut watch) = picture_of(&found.pmt);
        Self::route_found(tap.as_mut(), early.as_deref(), &pids)?;
        let mut routed = (self.clock)();
        // The client is told yes once the picture has come through whole, and its stream starts there: a station can list a channel whose
        // PMT names a picture and a sound that carry no packets (16.9 on 2026-11-05), a weak RF channel can bring the picture too broken to
        // start (30.1 the same day, 559 continuity gaps in 12 s), and a player takes either for a stream that has not started yet and keeps
        // asking for it; the pictures before the first whole one only made Plex's transcoder log "Invalid frame dimensions 0x0".
        let is_whole = |p: &[u8], watch: u16, picture: Option<(u16, u8)>| {
            psi::pid_of_packet(p) == watch && picture.is_none_or(|(_, stream_type)| psi::access_point(p, stream_type))
        };
        let mut first = Vec::with_capacity(PACKET * 512);
        let mut seen = 0;
        let start = loop {
            if let Some(new) = Verify::check(&mut verify, tap.as_mut(), &self.tuning, (self.clock)()) {
                if psi::protected(&new.pmt, Some(&new.channel)) {
                    return Err(Failure::Protected);
                }
                let next = plain_pids(&new);
                if next != pids {
                    tap.start(&next)?;
                    first.clear();
                    seen = 0;
                    routed = (self.clock)();
                }
                (picture, watch) = picture_of(&new.pmt);
                pids = next;
                found = new;
            }
            if let Some(at) = first[seen..].chunks_exact(PACKET).position(|p| is_whole(p, watch, picture)) {
                break seen + at * PACKET;
            }
            if let Some(f) = given_up() {
                return Err(f);
            }
            if (self.clock)().saturating_sub(routed) >= self.limits.first_picture {
                if verify.is_some() {
                    self.tuning.unlearn(&entry.guide);
                }
                let pictured = first
                    .chunks_exact(PACKET)
                    .any(|p| psi::pid_of_packet(p) == watch);
                return Err(self.unfound(
                    entry,
                    match (pictured, first.is_empty()) {
                        (true, _) => Unfound::Missing("no whole picture"),
                        (false, true) => Unfound::Missing("no packets"),
                        (false, false) => Unfound::Absent("no picture"),
                    },
                ));
            }
            seen = first.len();
            tap.read(&mut first, self.limits.poll)?;
        };
        first.drain(..start);
        let pictured = (self.clock)();
        self.tuning.streaming(entry);
        let Found { channel, pat, pmt_pid, pmt_section, .. } = found;
        Ok(Stream {
            stage: None,
            tap,
            hw: self.hw.clone(),
            pat: psi::build_pat(pat.tsid, channel.program, pmt_pid),
            pmt_pid,
            pmt: pmt_section,
            first,
            watch,
            took: [
                tuned.saturating_sub(asked),
                routed.saturating_sub(tuned),
                pictured.saturating_sub(routed),
            ],
            limits: self.limits.clone(),
            clock: self.clock.clone(),
            guide: entry.guide.clone(),
            cc: None,
            counted: (0, 0),
            over: Over {
                tuning: self.tuning.clone(),
                then: Status::Idle,
            },
            lease,
            verify,
        })
    }

    /// The refusal of a channel whose program was not found: only a clean multiplex (the demodulator counts no packet errors) is taken at
    /// its word when something read whole says the channel is not there; anything else is the signal.
    pub(crate) fn unfound(&self, entry: &Entry, why: Unfound) -> Failure {
        let signal = self.hw.signal();
        match why {
            Unfound::Absent(what) if signal.is_some_and(|s| s.locked && s.errors == Some(0)) => {
                Failure::OffAir(format!("{what} ({})", entry.guide))
            }
            Unfound::Absent(what) | Unfound::Missing(what) => {
                Failure::WeakSignal(match signal.and_then(|s| s.errors) {
                    Some(errors) => format!("{what} ({}, {errors} packet errors)", entry.guide),
                    None => format!("{what} ({})", entry.guide),
                })
            }
        }
    }
}

/// A channel's program as its tables say: its virtual channel, the PAT, the PMT's PID and section as read, and what the PMT says.
pub(crate) struct Found {
    pub(crate) channel: psi::Channel,
    pub(crate) pat: psi::Pat,
    pub(crate) pmt_pid: u16,
    pub(crate) pmt_section: Vec<u8>,
    pub(crate) pmt: psi::Pmt,
}

/// What a stream started from the cached tables of its channel has routed already and is still checking.
#[derive(Default)]
pub(crate) struct Start {
    pub(crate) early: Option<Vec<u16>>,
    pub(crate) verify: Option<Verify>,
}

/// The PIDs a stream without slates routes for the program: the PMT's, the clock and every stream.
pub(crate) fn plain_pids(found: &Found) -> Vec<u16> {
    let mut pids: Vec<u16> = [found.pmt_pid, found.pmt.pcr_pid].into_iter().chain(found.pmt.streams.iter().map(|e| e.pid)).collect();
    pids.sort_unstable();
    pids.dedup();
    pids
}

/// The program's picture (its PID and stream type), and the PID that tells whether the tuner is still where it was: the picture's, or the
/// clock's for a program without one.
fn picture_of(pmt: &psi::Pmt) -> (Option<(u16, u8)>, u16) {
    let picture = pmt.streams.iter().find(|e| matches!(e.stream_type, 0x02 | 0x1B | 0x24)).map(|e| (e.pid, e.stream_type));
    (picture, picture.map_or(pmt.pcr_pid, |(pid, _)| pid))
}

/// A stream started from its channel's cached tables, reading the live ones meanwhile without waiting ([`Tap::want`]): the sections are
/// asked for as the stream starts and taken as they come, the same tables [`program`] reads one after another.
pub(crate) struct Verify {
    entry: Entry,
    cached: Tables,
    /// When it gives up: a multiplex that does not bring its tables whole by then is left noted as it was.
    until: Duration,
    channels: Vec<Option<Vec<u8>>>,
    pat: Vec<Option<Vec<u8>>>,
}

enum Checked {
    Waiting,
    Same,
    Changed(Box<Found>),
    GaveUp,
}

/// Asks for the sections of one table and keeps those that have come in `have` (the first says how many there are).
fn gather(tap: &mut dyn Tap, pid: u16, table_id: u8, have: &mut Vec<Option<Vec<u8>>>) {
    if have.is_empty() {
        tap.want(pid, table_id, 0);
        if let Some(first) = tap.arrived(pid, table_id, 0)
            && let Some(last) = first.get(7).copied()
        {
            *have = vec![None; usize::from(last) + 1];
            have[0] = Some(first);
        }
    }
    for (n, section) in (0..=u8::MAX).zip(have.iter_mut()) {
        if section.is_none() {
            tap.want(pid, table_id, n);
            *section = tap.arrived(pid, table_id, n);
        }
    }
}

/// The sections of a table, once all of them are in.
fn whole(have: &[Option<Vec<u8>>]) -> Option<Vec<&[u8]>> {
    if have.is_empty() {
        return None;
    }
    have.iter().map(Option::as_deref).collect()
}

impl Verify {
    pub(crate) fn new(entry: Entry, cached: Tables, until: Duration) -> Self {
        Self { entry, cached, until, channels: Vec::new(), pat: Vec::new() }
    }

    /// Takes what has come of the live tables and compares them with the cached ones, once they name the channel's program whole.
    fn step(&mut self, tap: &mut dyn Tap, now: Duration) -> Checked {
        if now >= self.until {
            return Checked::GaveUp;
        }
        gather(tap, psi::PID_PSIP, psi::TID_TVCT, &mut self.channels);
        gather(tap, psi::PID_PAT, psi::TID_PAT, &mut self.pat);
        let channel = whole(&self.channels).and_then(|sections| {
            let all: Option<Vec<Vec<psi::Channel>>> = sections.into_iter().map(psi::parse_tvct).collect();
            all.and_then(|all| all.into_iter().flatten().find(|c| (c.major, c.minor) == (self.entry.major, self.entry.minor)))
        });
        if channel.is_none() && whole(&self.channels).is_some() {
            // a table read whole without the channel may be the last multiplex's: it is read again
            self.channels.clear();
        }
        let pat = whole(&self.pat).and_then(|sections| {
            let all: Option<Vec<psi::Pat>> = sections.into_iter().map(psi::parse_pat).collect();
            all.and_then(|all| {
                let tsid = all.first()?.tsid;
                Some(psi::Pat { tsid, programs: all.into_iter().flat_map(|p| p.programs).collect() })
            })
        });
        if pat.is_none() && whole(&self.pat).is_some() {
            self.pat.clear();
        }
        let (Some(channel), Some(pat)) = (channel, pat) else {
            return Checked::Waiting;
        };
        let Some(pmt_pid) = pat.programs.iter().find(|p| p.0 == channel.program).map(|p| p.1) else {
            self.pat.clear();
            return Checked::Waiting;
        };
        tap.want(pmt_pid, psi::TID_PMT, 0);
        let Some(pmt_section) = tap.arrived(pmt_pid, psi::TID_PMT, 0).and_then(|s| psi::checked(&s, psi::TID_PMT).map(<[u8]>::to_vec)) else {
            return Checked::Waiting;
        };
        let Some(pmt) = psi::parse_pmt(&pmt_section) else {
            return Checked::Waiting;
        };
        let found = Found { channel, pat, pmt_pid, pmt_section, pmt };
        if Tables::of(&found) == self.cached {
            Checked::Same
        } else {
            Checked::Changed(Box::new(found))
        }
    }

    /// Steps the check in `slot` and ends it when it has an answer or gave up. The live program, when it is not the cached one (it is
    /// noted as the channel's from then on).
    pub(crate) fn check(slot: &mut Option<Self>, tap: &mut dyn Tap, tuning: &Tuning, now: Duration) -> Option<Found> {
        let verify = slot.as_mut()?;
        let found = match verify.step(tap, now) {
            Checked::Waiting => return None,
            Checked::Changed(found) => Some(*found),
            Checked::Same | Checked::GaveUp => None,
        };
        if let Some(found) = &found {
            tuning.learned(&verify.entry.guide, found);
        }
        *slot = None;
        tap.forget();
        found
    }
}

/// One round of reading `entry`'s tables within `wait`: its virtual channel (the channel table's sections kept in `sections` across
/// rounds), the PAT's entry for its program and the program's PMT.
pub(crate) fn program(
    tap: &mut dyn Tap,
    entry: &Entry,
    sections: &mut Vec<Option<Vec<u8>>>,
    wait: Duration,
) -> Result<Found, Unfound> {
    let all = channels_of(tap, sections, wait).ok_or(Unfound::Missing("no whole channel table"))?;
    let Some(channel) = all
        .into_iter()
        .find(|c| (c.major, c.minor) == (entry.major, entry.minor))
    else {
        // a table read whole without the channel may be the last multiplex's: the next round reads it new
        sections.clear();
        return Err(Unfound::Absent("the multiplex does not list the channel"));
    };
    let pat = pat_of(tap, wait).ok_or(Unfound::Missing("no whole PAT"))?;
    let pmt_pid = pat
        .programs
        .iter()
        .find(|p| p.0 == channel.program)
        .map(|p| p.1)
        .ok_or(Unfound::Absent("the PAT does not list the program"))?;
    let pmt_section = tap
        .section(pmt_pid, psi::TID_PMT, 0, wait)
        .and_then(|s| psi::checked(&s, psi::TID_PMT).map(<[u8]>::to_vec))
        .ok_or(Unfound::Missing("no PMT"))?;
    let pmt = psi::parse_pmt(&pmt_section).ok_or(Unfound::Missing("no PMT"))?;
    Ok(Found { channel, pat, pmt_pid, pmt_section, pmt })
}

/// The whole PAT: every section of it (the first says how many there are), so a program missing from it is missing from the multiplex.
fn pat_of(tap: &mut dyn Tap, wait: Duration) -> Option<psi::Pat> {
    let first = tap.section(psi::PID_PAT, psi::TID_PAT, 0, wait)?;
    let last = *first.get(7)?;
    let mut pat = psi::parse_pat(&first)?;
    for n in 1..=last {
        let more = tap
            .section(psi::PID_PAT, psi::TID_PAT, n, wait)
            .and_then(|s| psi::parse_pat(&s))?;
        pat.programs.extend(more.programs);
    }
    Some(pat)
}

/// A request the tuner took: a stream tuned without slates, or one to answer at once with them; or one it refused while slates are on.
pub(crate) enum Admitted {
    Plain(Box<Stream>),
    Slated(Slated),
    Refused(Refusal),
}

/// A request for a tuner in use, answered with a few seconds of a slate in the channel's shape that says who has it, instead of 503.
pub(crate) struct Refusal {
    pub(crate) entry: Entry,
    pub(crate) look: Look,
    pub(crate) says: Says,
    pub(crate) failure: Failure,
}

/// A request for H.264: the converter that makes it and what it is to make.
struct Conversion {
    codec: Arc<dyn Codec>,
    profile: Profile,
}

/// What a client is called: the device name it gave, else the name the host knows its address by, `Plex` for a Plex user agent, else its
/// user agent's product.
pub(crate) fn client_name(claim: &Claim, names: &HashMap<IpAddr, String>) -> Option<String> {
    if let Some(device) = claim.device.as_deref().filter(|d| !d.is_empty()) {
        return Some(device.to_owned());
    }
    if let Some(name) = names.get(&claim.peer) {
        return Some(name.clone());
    }
    let agent = claim.agent.as_deref()?.trim();
    if agent.to_ascii_lowercase().contains("plex") {
        return Some("Plex".to_owned());
    }
    let product = agent.split(['/', ' ']).next().unwrap_or("");
    (!product.is_empty()).then(|| product.chars().take(40).collect())
}

/// The line under "Tuner busy": Plex recording what the guide shows on the channel, else the client by name or address.
pub(crate) fn busy_line(claim: &Claim, who: Option<&str>, programme: Option<&str>) -> String {
    let channel = format!("{} {}", claim.guide, claim.name);
    match (who, programme) {
        (Some("Plex"), Some(title)) => format!("Plex is recording {title} on {channel}"),
        (Some("Plex"), None) => format!("Plex is recording {channel}"),
        (Some(who), _) => format!("In use by {who}"),
        (None, _) => format!("In use by {}", claim.peer),
    }
}

/// How a running stream ended (a client that went away is the `io::Error` of `pump`, not one of these).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum End {
    /// Asked to stop (the host is going down, or the tuner is no longer shared).
    Stopped,
    /// The same client asked for another channel, or the TV is waited for: the next request has the tuner.
    Replaced,
    /// The program's picture stopped arriving: the tuner has been moved (Live TV changed channel).
    Silent,
    /// Another device took the tuner: its name. The stream ends and the client is told who took it.
    Taken(String),
    /// The TV took the tuner for its own use.
    Preempted(String),
    Failed(Failure),
}

/// A stream that is tuned and routed, its picture arriving, and has not yet written a byte. Fields drop in order: the converter, if
/// there is one, is given back first (the next request may want it), the tap lets go of the TV, the tuner says it is free, then the lease
/// lets the next request have it (which then says what it streams).
pub(crate) struct Stream {
    /// The picture as H.264 and the program's PMT of it, when the request asked for it and the converter started.
    stage: Option<Stage>,
    tap: Box<dyn Tap>,
    hw: Arc<dyn Hardware>,
    pat: Vec<u8>,
    pmt_pid: u16,
    /// The program's PMT as read, sent first.
    pmt: Vec<u8>,
    /// The packets read since the first whole picture, that picture's first.
    first: Vec<u8>,
    watch: u16,
    /// How long the tune, the tables and the first picture took.
    pub(crate) took: [Duration; 3],
    limits: Limits,
    clock: Clock,
    /// The channel, and its picture's packets that came and that were lost, by continuity count, for its health.
    guide: String,
    cc: Option<u8>,
    counted: (u64, u64),
    over: Over,
    lease: Lease,
    /// The cached tables it started from, while the live ones are still being checked against them.
    verify: Option<Verify>,
}

impl Stream {
    /// Makes the stream H.264 ([`crate::transcode`]): `codec` is opened for the picture's size and rate to make what `profile` asks.
    /// The stream is left as it is when that cannot be done.
    pub(crate) fn convert(&mut self, codec: &dyn Codec, profile: Profile) -> Result<(), Fault> {
        let unsupported = |why: &str| Fault::Unsupported(why.to_owned());
        let pmt = psi::parse_pmt(&self.pmt).ok_or_else(|| unsupported("no PMT"))?;
        if !pmt.streams.iter().any(|e| e.pid == self.watch && e.stream_type == 0x02) {
            return Err(unsupported("the picture is not MPEG-2"));
        }
        let format = self
            .first
            .chunks_exact(PACKET)
            .filter(|p| psi::pid_of_packet(p) == self.watch)
            .find_map(|p| Format::of_sequence(&p[4..]))
            .ok_or_else(|| unsupported("no sequence header in the first picture"))?;
        let convert = codec.open(&format, &profile.want(&format))?;
        self.stage = Some(Stage::new(convert, &pmt, self.pmt_pid, self.watch, &format, &self.limits));
        Ok(())
    }

    /// Writes the program until the client goes, `stop` is set, another request takes the tuner or the TV does: a PAT that names only
    /// this program, ten times a second, the program's PMT once at the start, and the packets as they arrive. A converted stream sends
    /// its own PMT with every PAT, and its picture as H.264. However it ends, the stream is over from then on.
    pub(crate) fn pump(&mut self, out: &mut dyn Write, stop: &AtomicBool) -> io::Result<End> {
        let end = self.run(out, stop);
        if let Ok(End::Failed(Failure::Converter(fault))) = &end {
            self.over.tuning.breaker.failed(fault, (self.clock)(), self.limits.convert_rest);
        }
        self.lease.over();
        let (got, lost) = self.counted;
        let share = if got + lost == 0 { 0.0 } else { lost as f32 / (got + lost) as f32 };
        let dropouts = u32::from(matches!(end, Ok(End::Silent)));
        self.over.tuning.record(
            &self.guide,
            Health {
                at: SystemTime::now(),
                verdict: Verdict::of(true, share, dropouts),
                signal: None,
                lost: share,
                picture: Some(self.took.iter().sum()),
                dropouts,
                checked: false,
            },
        );
        end
    }

    fn run(&mut self, out: &mut dyn Write, stop: &AtomicBool) -> io::Result<End> {
        let lim = &self.limits;
        let mut stage = self.stage.take();
        let (mut pat, mut pmt) = (
            psi::SectionPackets::default(),
            psi::SectionPackets::default(),
        );
        // the PAT and the PMT lead, so a player knows the program from the first packet (the station's PMT comes up to 0.4 s later), then
        // the first whole picture and what came after it; the station's PMT packets go on from the count of the one written here
        let mut buf = Vec::with_capacity(PACKET * 512);
        let mut staged = Vec::new();
        pat.write(psi::PID_PAT, &self.pat, &mut buf);
        pmt.write(self.pmt_pid, stage.as_ref().map_or(&self.pmt[..], Stage::pmt), &mut buf);
        let mut first = std::mem::take(&mut self.first);
        for p in first.chunks_exact_mut(PACKET) {
            if psi::pid_of_packet(p) == self.pmt_pid {
                pmt.renumber(p);
            }
        }
        out.write_all(&buf)?;
        match stage.as_mut() {
            Some(stage) => {
                if let Err(fault) = stage.feed(&first, (self.clock)(), &mut staged) {
                    return Ok(End::Failed(Failure::Converter(fault)));
                }
                out.write_all(&staged)?;
            }
            None => out.write_all(&first)?,
        }
        drop(first);
        let mut last_pat = (self.clock)();
        let mut last_watch = last_pat;
        let mut last_owner = last_pat;
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(End::Stopped);
            }
            if self.lease.replaced.load(Ordering::Acquire) {
                return Ok(self.lease.taker().map_or(End::Replaced, End::Taken));
            }
            let now = (self.clock)();
            if now.saturating_sub(last_pat) >= lim.pat_every {
                buf.clear();
                pat.write(psi::PID_PAT, &self.pat, &mut buf);
                if let Some(stage) = &stage {
                    pmt.write(self.pmt_pid, stage.pmt(), &mut buf);
                }
                out.write_all(&buf)?;
                last_pat = now;
            }
            if now.saturating_sub(last_watch) >= lim.silence {
                return Ok(End::Silent);
            }
            if now.saturating_sub(last_owner) >= lim.owner_every {
                last_owner = now;
                if let Some(why) = self.hw.in_use().or_else(|| self.tap.taken()) {
                    self.over.then = Status::Busy(why.clone());
                    self.lease.taken_by_tv(&self.hw.tv_name());
                    return Ok(End::Preempted(why));
                }
            }
            if self.verify.is_some() {
                // a change in the tables only corrects what is noted for the next stream: this one is going out as it started
                Verify::check(&mut self.verify, self.tap.as_mut(), &self.over.tuning, now);
            }
            buf.clear();
            if let Err(f) = self.tap.read(&mut buf, lim.poll) {
                return Ok(End::Failed(f));
            }
            let mut pictured = false;
            for p in buf.chunks_exact_mut(PACKET) {
                match psi::pid_of_packet(p) {
                    pid if pid == self.watch => {
                        pictured = true;
                        if p[3] & 0x10 != 0 {
                            let cc = p[3] & 0x0F;
                            self.counted.1 += self.cc.map_or(0, |last| u64::from(lost_before(last, cc)));
                            self.counted.0 += 1;
                            self.cc = Some(cc);
                        }
                    }
                    pid if pid == self.pmt_pid => pmt.renumber(p),
                    _ => {}
                }
            }
            if pictured {
                last_watch = (self.clock)();
            }
            if let Some(stage) = stage.as_mut() {
                staged.clear();
                if let Err(fault) = stage.feed(&buf, (self.clock)(), &mut staged) {
                    return Ok(End::Failed(Failure::Converter(fault)));
                }
                if !staged.is_empty() {
                    out.write_all(&staged)?;
                }
            } else if !buf.is_empty() {
                out.write_all(&buf)?;
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::psi::tests::{pat, pmt, tvct, tvct_part};
    use super::*;
    use crate::transcode::{Want, tests as converter};
    use serde_json::json;
    use std::collections::{HashMap, VecDeque};
    use std::net::Ipv4Addr;
    use std::sync::atomic::AtomicU64;
    use std::thread;

    /// Plex at home, and another client.
    pub(crate) const PLEX: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 69));
    const OTHER: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 70));
    const THIRD: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 71));

    /// The fake clock: reads wait by moving it, so a test of five silent seconds takes no time.
    #[derive(Clone, Default)]
    pub(crate) struct Time(Arc<AtomicU64>);

    impl Time {
        pub(crate) fn clock(&self) -> Clock {
            let ms = self.0.clone();
            Arc::new(move || Duration::from_millis(ms.load(Ordering::Relaxed)))
        }

        pub(crate) fn advance(&self, by: Duration) {
            self.0.fetch_add(by.as_millis() as u64, Ordering::Relaxed);
        }

        pub(crate) fn now(&self) -> Duration {
            Duration::from_millis(self.0.load(Ordering::Relaxed))
        }
    }

    #[derive(Clone)]
    pub(crate) struct Multiplex {
        pub(crate) tvct: Vec<Vec<u8>>,
        pub(crate) pat: Vec<u8>,
        pub(crate) pmts: HashMap<u16, Vec<u8>>,
    }

    /// RF 32's multiplex as the set carries it, cut down: 2.1 is program 1 (PMT 0x30), 2.4 is program 4 (PMT 0x60: clock and picture 0x61, AC-3 0x64).
    pub(crate) fn rf32(program_tags: &[u8]) -> Multiplex {
        Multiplex {
            tvct: vec![tvct(
                739,
                &[("WSB-HD", 2, 1, 1, false), ("WEST", 2, 4, 4, false)],
            )],
            pat: pat(739, &[(1, 0x30), (4, 0x60)]),
            pmts: HashMap::from([
                (
                    0x30,
                    pmt(1, 0x31, &[], &[(0x02, 0x31, &[]), (0x81, 0x34, &[])]),
                ),
                (
                    0x60,
                    pmt(
                        4,
                        0x61,
                        program_tags,
                        &[(0x02, 0x61, &[]), (0x81, 0x64, &[])],
                    ),
                ),
            ]),
        }
    }

    pub(crate) struct Fake {
        time: Time,
        mux: Multiplex,
        pub(crate) owner: Mutex<Option<String>>,
        pub(crate) taken: Mutex<Option<String>>,
        pub(crate) tuned: Mutex<Vec<String>>,
        started: Mutex<Vec<Vec<u16>>>,
        pub(crate) released: AtomicU64,
        scripted: Vec<Vec<u8>>,
        lists: AtomicU64,
        list_fails: AtomicBool,
        /// The devices Live TV was asked to give way to, and whether it does.
        pub(crate) yielded: Mutex<Vec<String>>,
        pub(crate) yields: AtomicBool,
        /// The channels `programmes` was asked for, and the ones it fails for.
        programmes_asked: Mutex<Vec<String>>,
        programmes_fail: Mutex<Vec<String>>,
        /// A virtual channel table the next read gives instead of the multiplex's own (the last multiplex's, still on the demux).
        stale: Mutex<Option<Vec<u8>>>,
        /// Sections (`pid`, `table_id`, `section_number`) the next read of each loses, and every section read.
        lost: Mutex<Vec<(u16, u8, u8)>>,
        reads: Mutex<Vec<(u16, u8, u8)>>,
        /// Sections asked for with `want` and when each comes, and the ones that have come.
        wanted: Mutex<Vec<(Section, Duration)>>,
        arrivals: Mutex<Vec<Section>>,
        /// What a tap gets to read once these PIDs are routed on it.
        on_start: Mutex<Vec<(Vec<u16>, Vec<u8>)>>,
        /// What the demodulator says.
        pub(crate) signal: Mutex<Option<Signal>>,
        /// The channels held for the meter, and the TV's last channel.
        pub(crate) held: Mutex<Vec<String>>,
        pub(crate) last: Mutex<Option<String>>,
    }

    impl Fake {
        fn new(time: &Time, mux: Multiplex, scripted: Vec<Vec<u8>>) -> Arc<Self> {
            Arc::new(Self {
                time: time.clone(),
                mux,
                owner: Mutex::default(),
                taken: Mutex::default(),
                yielded: Mutex::default(),
                yields: AtomicBool::new(false),
                tuned: Mutex::default(),
                started: Mutex::default(),
                released: AtomicU64::new(0),
                scripted,
                lists: AtomicU64::new(0),
                list_fails: AtomicBool::new(false),
                programmes_asked: Mutex::default(),
                programmes_fail: Mutex::default(),
                stale: Mutex::default(),
                lost: Mutex::default(),
                reads: Mutex::default(),
                wanted: Mutex::default(),
                arrivals: Mutex::default(),
                on_start: Mutex::default(),
                signal: Mutex::default(),
                held: Mutex::default(),
                last: Mutex::default(),
            })
        }
    }

    /// A section a tap is asked for: `pid`, `table_id`, `section_number`.
    type Section = (u16, u8, u8);

    /// The section as the multiplex gives it: the first virtual channel table can be the last multiplex's, and a section can be lost.
    fn serve(fake: &Fake, pid: u16, table_id: u8, number: u8) -> Option<Vec<u8>> {
        let mut lost = fake.lost.lock();
        if let Some(at) = lost.iter().position(|l| *l == (pid, table_id, number)) {
            lost.remove(at);
            return None;
        }
        let m = &fake.mux;
        match (pid, table_id) {
            (psi::PID_PSIP, psi::TID_TVCT) => fake.stale.lock().take().or_else(|| m.tvct.get(usize::from(number)).cloned()),
            (psi::PID_PAT, psi::TID_PAT) => Some(m.pat.clone()),
            (pid, psi::TID_PMT) => m.pmts.get(&pid).cloned(),
            _ => None,
        }
    }

    struct FakeTap {
        fake: Arc<Fake>,
        queue: VecDeque<Vec<u8>>,
    }

    /// The fake as the hardware: a tap needs the fake's shared state, so the hardware is the `Arc`.
    pub(crate) struct Shared(pub(crate) Arc<Fake>);

    impl Hardware for Shared {
        fn channel_list(&self) -> Result<Value, Failure> {
            self.0.lists.fetch_add(1, Ordering::Relaxed);
            if self.0.list_fails.load(Ordering::Relaxed) {
                return Err(Failure::Hardware("luna-send gave no answer".into()));
            }
            Ok(json!({"channelList": [
                {"channelId": "1_32_2_1_0_0_0", "channelName": "WSB-HD"},
                {"channelId": "1_32_2_4_0_0_0", "channelName": "WEST"},
            ]}))
        }

        fn programmes(&self, channel_id: &str) -> Result<Value, Failure> {
            self.0.programmes_asked.lock().push(channel_id.to_owned());
            if self
                .0
                .programmes_fail
                .lock()
                .iter()
                .any(|c| c == channel_id)
            {
                return Err(Failure::Hardware("luna-send gave no answer".into()));
            }
            Ok(json!({"returnValue": true, "programList": [
                {"channelId": channel_id, "programName": format!("News on {channel_id}"),
                 "startTime": "2026,11,05,23,00,00", "endTime": "2026,11,06,00,00,00"},
            ]}))
        }

        fn in_use(&self) -> Option<String> {
            self.0.owner.lock().clone()
        }

        fn signal(&self) -> Option<Signal> {
            *self.0.signal.lock()
        }

        fn tune(&self, channel_id: &str) -> Result<Box<dyn Tap>, Failure> {
            self.0.tuned.lock().push(channel_id.to_owned());
            Ok(Box::new(FakeTap {
                fake: self.0.clone(),
                queue: self.0.scripted.iter().cloned().collect(),
            }))
        }

        fn hold(&self, channel_id: &str) -> Result<Box<dyn Hold>, Failure> {
            self.0.held.lock().push(channel_id.to_owned());
            Ok(Box::new(FakeTap {
                fake: self.0.clone(),
                queue: VecDeque::new(),
            }))
        }

        fn tuned_to(&self) -> Option<String> {
            self.0.last.lock().clone()
        }

        fn tv_name(&self) -> String {
            "Living Room TV".to_owned()
        }

        fn yield_to(&self, by: &str) -> bool {
            self.0.yielded.lock().push(by.to_owned());
            let gives_way = self.0.yields.load(Ordering::Relaxed);
            if gives_way {
                *self.0.owner.lock() = None;
            }
            gives_way
        }
    }

    impl Hold for FakeTap {
        fn taken(&mut self) -> Option<String> {
            self.fake.taken.lock().clone()
        }
    }

    /// A clean multiplex's reading.
    pub(crate) const CLEAN: Signal = Signal { locked: true, mse: Some(22), snr: Some(30.0), errors: Some(0), ber: Some(0) };

    impl Tap for FakeTap {
        /// A table takes a moment to come round.
        fn section(&mut self, pid: u16, table_id: u8, number: u8, _wait: Duration) -> Option<Vec<u8>> {
            self.fake.time.advance(Duration::from_millis(50));
            self.fake.reads.lock().push((pid, table_id, number));
            serve(&self.fake, pid, table_id, number)
        }

        fn want(&mut self, pid: u16, table_id: u8, number: u8) {
            let mut wanted = self.fake.wanted.lock();
            if wanted.iter().all(|(key, _)| *key != (pid, table_id, number)) {
                wanted.push(((pid, table_id, number), self.fake.time.now() + Duration::from_millis(50)));
            }
        }

        fn arrived(&mut self, pid: u16, table_id: u8, number: u8) -> Option<Vec<u8>> {
            let mut wanted = self.fake.wanted.lock();
            let at = wanted.iter().position(|(key, due)| *key == (pid, table_id, number) && *due <= self.fake.time.now())?;
            wanted.remove(at);
            self.fake.arrivals.lock().push((pid, table_id, number));
            serve(&self.fake, pid, table_id, number)
        }

        fn forget(&mut self) {
            self.fake.wanted.lock().clear();
        }

        fn start(&mut self, pids: &[u16]) -> Result<(), Failure> {
            self.fake.started.lock().push(pids.to_vec());
            let mut on_start = self.fake.on_start.lock();
            if let Some(at) = on_start.iter().position(|(routed, _)| routed == pids) {
                self.queue.push_back(on_start.remove(at).1);
            }
            Ok(())
        }

        fn taken(&mut self) -> Option<String> {
            self.fake.taken.lock().clone()
        }

        fn read(&mut self, out: &mut Vec<u8>, wait: Duration) -> Result<(), Failure> {
            match self.queue.pop_front() {
                Some(chunk) => out.extend_from_slice(&chunk),
                None => self.fake.time.advance(wait),
            }
            Ok(())
        }
    }

    impl Drop for FakeTap {
        fn drop(&mut self) {
            self.fake.released.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn packet(pid: u16, cc: u8) -> Vec<u8> {
        let mut p = vec![0x47, (pid >> 8) as u8, pid as u8, 0x10 | cc];
        p.resize(PACKET, 0xAB);
        p
    }

    /// A packet that starts a picture that decodes on its own: an MPEG-2 sequence header at the start of its payload.
    pub(crate) fn picture(pid: u16, cc: u8) -> Vec<u8> {
        let mut p = vec![
            0x47,
            0x40 | (pid >> 8) as u8,
            pid as u8,
            0x10 | cc,
            0,
            0,
            1,
            0xB3,
        ];
        p.resize(PACKET, 0xAB);
        p
    }

    /// The state of a tuner that is served.
    pub(crate) fn served() -> Arc<Tuning> {
        let tuning = Arc::new(Tuning::default());
        tuning.serve();
        tuning
    }

    fn facade_with(fake: &Arc<Fake>, time: &Time, limits: Limits) -> Facade {
        facade_guided(fake, time, limits, None)
    }

    fn facade_guided(
        fake: &Arc<Fake>,
        time: &Time,
        limits: Limits,
        source: Option<Arc<dyn Guide>>,
    ) -> Facade {
        Facade::new(
            Arc::new(Shared(fake.clone())),
            Identity {
                name: "LumaTV".into(),
                device_id: "0A1B2C3D".into(),
            },
            limits,
            time.clock(),
            served(),
            source,
        )
    }

    fn facade(fake: &Arc<Fake>, time: &Time) -> Facade {
        facade_with(fake, time, Limits::default())
    }

    /// Limits under which a running stream asks every second whether the TV has taken the tuner.
    fn asking_often() -> Limits {
        Limits {
            owner_every: Duration::from_secs(1),
            ..Limits::default()
        }
    }

    pub(crate) fn fake_with(time: &Time, mux: Multiplex, scripted: Vec<Vec<u8>>) -> Arc<Fake> {
        Fake::new(time, mux, scripted)
    }

    /// What the tap gives first on a channel that is on the air: the start of a whole picture of 2.1 and of 2.4.
    pub(crate) fn on_air() -> Vec<Vec<u8>> {
        vec![[picture(0x31, 0), picture(0x61, 0)].concat()]
    }

    #[test]
    fn a_stream_is_a_pat_naming_one_program_then_its_pmt_then_that_programs_packets_from_its_first_whole_picture()
     {
        let time = Time::default();
        let before = [packet(0x61, 15), packet(0x64, 15)].concat();
        let chunk = [
            picture(0x61, 0),
            packet(0x64, 0),
            packet(0x60, 0),
            packet(0x61, 1),
        ]
        .concat();
        let fake = fake_with(&time, rf32(&[]), vec![[before, chunk.clone()].concat()]);
        let mut stream = facade(&fake, &time).open("2.4", PLEX).unwrap();
        assert_eq!(*fake.tuned.lock(), ["1_32_2_4_0_0_0"]);
        assert_eq!(
            *fake.started.lock(),
            [vec![0x60, 0x61, 0x64]],
            "the PMT, the clock and picture, the audio; not the PAT, which is rewritten"
        );
        let mut out = Vec::new();
        let end = stream.pump(&mut out, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            end,
            End::Silent,
            "after the script the picture stops and the stream ends by itself"
        );
        assert!(out.len() % PACKET == 0);
        let packets: Vec<&[u8]> = out.chunks(PACKET).collect();
        let pid = |p: &[u8]| u16::from_be_bytes([p[1] & 0x1F, p[2]]);
        assert_eq!(pid(packets[0]), 0, "a PAT first");
        assert_eq!(
            psi::parse_pat(&packets[0][5..21]),
            Some(psi::Pat {
                tsid: 739,
                programs: vec![(4, 0x60)]
            }),
            "which names only program 4"
        );
        assert_eq!(
            (pid(packets[1]), psi::parse_pmt(&packets[1][5..])),
            (0x60, psi::parse_pmt(&fake.mux.pmts[&0x60])),
            "then its PMT, as the multiplex has it"
        );
        let rest: Vec<u8> = packets[2..]
            .iter()
            .filter(|p| pid(p) != 0)
            .flat_map(|p| p.iter().copied())
            .collect();
        let mut renumbered = chunk.clone();
        renumbered[2 * PACKET + 3] = 0x11;
        assert_eq!(
            rest, renumbered,
            "from the first whole picture on, everything the tuner gave, in order, the station's PMT counted on from the one written first"
        );
        assert!(
            packets.iter().filter(|p| pid(p) == 0).count() > 10,
            "the PAT goes on while the stream runs"
        );
        drop(stream);
        assert_eq!(fake.released.load(Ordering::Relaxed), 1, "the TV is let go");
    }

    #[test]
    fn a_channel_whose_picture_never_comes_is_not_on_air_only_on_a_clean_multiplex_and_is_tuned_again_when_asked_again()
     {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), vec![packet(0x60, 0)]);
        let f = facade(&fake, &time);
        let clean = CLEAN;
        *fake.signal.lock() = Some(clean);
        assert_eq!(
            f.open("2.4", PLEX).err(),
            Some(Failure::OffAir("no picture (2.4)".into()))
        );
        assert_eq!(fake.released.load(Ordering::Relaxed), 1, "the TV is let go");
        assert_eq!(f.tuning.status(), Status::Idle);
        *fake.signal.lock() = Some(Signal {
            errors: Some(81),
            ..clean
        });
        assert_eq!(
            f.open("2.4", PLEX).err(),
            Some(Failure::WeakSignal(
                "no picture (2.4, 81 packet errors)".into()
            ))
        );
        *fake.signal.lock() = None;
        assert_eq!(
            f.open("2.4", PLEX).err(),
            Some(Failure::WeakSignal("no picture (2.4)".into()))
        );
        assert_eq!(fake.tuned.lock().len(), 3, "each request tunes again");
        assert_eq!(f.lineup().unwrap().len(), 2, "and the lineup keeps it");
    }

    #[test]
    fn a_picture_that_never_comes_through_whole_is_a_weak_signal_even_on_a_clean_multiplex() {
        let time = Time::default();
        let fake = fake_with(
            &time,
            rf32(&[]),
            vec![[packet(0x61, 0), packet(0x61, 2)].concat()],
        );
        *fake.signal.lock() = Some(CLEAN);
        assert_eq!(
            facade(&fake, &time).open("2.4", PLEX).err(),
            Some(Failure::WeakSignal(
                "no whole picture (2.4, 0 packet errors)".into()
            ))
        );
    }

    /// Lets another thread get as far as asking for the tuner: it tells the stream that holds it to end before it waits, and the stream's
    /// lease can only be let go once it waits.
    fn until_asked(stream: &Stream) {
        let end = Instant::now() + Duration::from_secs(5);
        while !stream.lease.replaced.load(Ordering::Acquire) {
            assert!(Instant::now() < end, "nobody asked for the tuner");
            thread::yield_now();
        }
    }

    #[test]
    fn another_client_is_refused_while_a_stream_runs_and_the_tuner_is_free_again_when_it_ends() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), on_air());
        let f = facade(&fake, &time);
        f.tuning.options.lock().names.insert(PLEX, "Plex".to_owned());
        let first = f.open("2.4", PLEX).unwrap();
        assert_eq!(f.open("2.1", OTHER).err(), Some(Failure::Busy));
        assert_eq!(
            fake.tuned.lock().len(),
            1,
            "the refused one never touched the TV"
        );
        assert!(
            !first.lease.replaced.load(Ordering::Acquire),
            "the running stream goes on"
        );
        assert_eq!(
            f.tuning.status(),
            Status::Streaming {
                guide: "2.4".into(),
                name: "WEST".into()
            }
        );
        drop(first);
        assert_eq!(f.tuning.status(), Status::Idle);
        assert!(f.open("2.1", OTHER).is_ok());
    }

    /// A client's stream as a request without slates is served, the client calling itself `device`.
    fn started(f: &Facade, guide: &str, peer: IpAddr, device: &str) -> Result<Box<Stream>, Failure> {
        match f.admit(guide, peer, None, Some(device), None)? {
            Admitted::Plain(stream) => Ok(stream),
            _ => panic!("not a plain stream"),
        }
    }

    fn displaced_of(f: &Facade, now_ms: u64) -> Vec<Value> {
        f.status_at(now_ms)["displaced"].as_array().cloned().unwrap()
    }

    #[test]
    fn the_last_client_to_start_has_the_tuner_and_the_displaced_one_is_told_who_took_it_for_a_minute() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), on_air());
        let f = Arc::new(facade(&fake, &time));
        let mut phone = started(&f, "2.4", OTHER, "Pixel").unwrap();
        let next = {
            let f = f.clone();
            thread::spawn(move || started(&f, "2.1", THIRD, "Living Room iPad").map(drop))
        };
        until_asked(&phone);
        let end = phone.pump(&mut Vec::new(), &AtomicBool::new(false)).unwrap();
        assert_eq!(end, End::Taken("Living Room iPad".into()), "the stream ends, not fails");
        drop(phone);
        assert_eq!(next.join().unwrap(), Ok(()));
        let now = crate::webos::unix_ms();
        let [told] = displaced_of(&f, now).try_into().unwrap();
        let at = told["at"].as_u64().unwrap();
        assert_eq!(
            told,
            serde_json::json!({"ip": "192.168.1.70", "name": "Pixel", "by": "Living Room iPad", "byKind": "app", "at": at, "channel": "2.4", "channelName": "WEST"})
        );
        assert_eq!(displaced_of(&f, at + DISPLACED_FOR_MS - 1).len(), 1);
        assert!(displaced_of(&f, at + DISPLACED_FOR_MS).is_empty(), "the record expires");
    }

    #[test]
    fn the_same_client_changing_channel_is_not_displaced_and_a_recording_is_not_taken_from() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), on_air());
        let f = Arc::new(facade(&fake, &time));
        let mut old = started(&f, "2.4", OTHER, "Pixel").unwrap();
        let next = {
            let f = f.clone();
            thread::spawn(move || started(&f, "2.1", OTHER, "Pixel").map(drop))
        };
        until_asked(&old);
        assert_eq!(old.pump(&mut Vec::new(), &AtomicBool::new(false)).unwrap(), End::Replaced);
        drop(old);
        assert_eq!(next.join().unwrap(), Ok(()));
        assert!(displaced_of(&f, crate::webos::unix_ms()).is_empty());

        f.tuning.options.lock().names.insert(PLEX, "Plex".to_owned());
        let recording = f.open("2.4", PLEX).unwrap();
        assert_eq!(started(&f, "2.1", THIRD, "Pixel").err(), Some(Failure::Busy), "a recording keeps the tuner");
        assert!(!recording.lease.replaced.load(Ordering::Acquire));
        assert!(displaced_of(&f, crate::webos::unix_ms()).is_empty());
    }

    #[test]
    fn a_device_takes_the_tuner_from_the_tvs_live_tv_by_asking_it_to_give_way_but_plex_does_not() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), on_air());
        *fake.owner.lock() = Some("Live TV is in front".into());
        let f = facade(&fake, &time);
        f.tuning.options.lock().names.insert(PLEX, "Plex".to_owned());
        assert_eq!(f.open("2.4", PLEX).err(), Some(Failure::TvInUse("Live TV is in front".into())));
        assert!(fake.yielded.lock().is_empty(), "the TV does not give way to a recording");

        assert_eq!(started(&f, "2.4", OTHER, "Pixel").err(), Some(Failure::TvInUse("Live TV is in front".into())), "Live TV that cannot give way keeps it");
        assert_eq!(*fake.yielded.lock(), ["Pixel"]);
        assert!(fake.tuned.lock().is_empty());

        fake.yields.store(true, Ordering::Relaxed);
        assert!(started(&f, "2.4", OTHER, "Pixel").is_ok());
        assert_eq!(*fake.yielded.lock(), ["Pixel", "Pixel"], "the TV is told who took the tuner");
    }

    #[test]
    fn the_tv_taking_the_tuner_from_a_client_is_noted_for_it_as_the_tv() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), on_air());
        let f = facade_with(&fake, &time, asking_often());
        let mut stream = started(&f, "2.4", OTHER, "Pixel").unwrap();
        *fake.owner.lock() = Some("Live TV is in front".into());
        assert_eq!(
            stream.pump(&mut Vec::new(), &AtomicBool::new(false)).unwrap(),
            End::Preempted("Live TV is in front".into())
        );
        let [told] = displaced_of(&f, crate::webos::unix_ms()).try_into().unwrap();
        assert_eq!((told["name"].as_str(), told["by"].as_str(), told["byKind"].as_str()), (Some("Pixel"), Some("Living Room TV"), Some("tv")));
    }

    #[test]
    fn a_channel_is_routed_from_its_cached_tables_as_soon_as_it_is_tuned_while_the_live_ones_are_read_and_checked() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), on_air());
        let f = facade(&fake, &time);
        drop(f.open("2.4", PLEX).unwrap());
        assert_eq!(*fake.started.lock(), [vec![0x60, 0x61, 0x64]]);
        let read = fake.reads.lock().len();
        let mut stream = f.open("2.4", PLEX).unwrap();
        assert_eq!(fake.reads.lock().len(), read, "no table was waited for");
        assert_eq!(stream.took[1], Duration::ZERO);
        stream.pump(&mut Vec::new(), &AtomicBool::new(false)).unwrap();
        assert_eq!(*fake.arrivals.lock(), [(psi::PID_PSIP, psi::TID_TVCT, 0), (psi::PID_PAT, psi::TID_PAT, 0), (0x60, psi::TID_PMT, 0)]);
        assert_eq!(fake.started.lock().len(), 2, "routed once on this tune: the tables said the same");
    }

    #[test]
    fn a_program_that_moved_since_it_was_cached_is_routed_again_before_its_picture_and_noted_as_it_is() {
        let time = Time::default();
        let mux = rf32(&[]);
        let fake = fake_with(&time, mux.clone(), on_air());
        let f = facade(&fake, &time);
        let moved = pmt(4, 0x62, &[], &[(0x02, 0x62, &[]), (0x81, 0x64, &[])]);
        f.tuning.noted("2.4", Tables { tsid: 739, program: 4, access_controlled: false, pmt_pid: 0x60, pmt: moved });
        fake.on_start.lock().push((vec![0x60, 0x61, 0x64], picture(0x61, 0)));
        let stream = f.open("2.4", PLEX).unwrap();
        assert_eq!(*fake.started.lock(), [vec![0x60, 0x62, 0x64], vec![0x60, 0x61, 0x64]]);
        assert_eq!(stream.pmt, mux.pmts[&0x60], "the stream goes out as the live tables say");
        assert_eq!(f.tuning.tables("2.4").map(|t| t.pmt), Some(mux.pmts[&0x60].clone()));
    }

    #[test]
    fn a_cached_program_that_brings_no_picture_and_that_the_multiplex_does_not_list_is_forgotten() {
        let time = Time::default();
        let mut mux = rf32(&[]);
        mux.tvct = vec![tvct(739, &[("WSB-HD", 2, 1, 1, false)])];
        let fake = fake_with(&time, mux, vec![packet(0x61, 0)]);
        let f = facade(&fake, &time);
        let gone = pmt(4, 0x61, &[], &[(0x02, 0x61, &[]), (0x81, 0x64, &[])]);
        f.tuning.noted("2.4", Tables { tsid: 739, program: 4, access_controlled: false, pmt_pid: 0x60, pmt: gone });
        assert!(f.open("2.4", PLEX).is_err());
        assert_eq!(f.tuning.tables("2.4"), None);
    }

    #[test]
    fn the_same_client_asking_again_takes_the_tuner_over_once_its_old_stream_lets_go() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), on_air());
        let f = Arc::new(facade(&fake, &time));
        let mut old = f.open("2.4", PLEX).unwrap();
        let next = {
            let f = f.clone();
            thread::spawn(move || f.open("2.1", PLEX).map(drop))
        };
        until_asked(&old);
        assert_eq!(
            old.pump(&mut Vec::new(), &AtomicBool::new(false)).unwrap(),
            End::Replaced
        );
        assert_eq!(
            fake.tuned.lock().len(),
            1,
            "not tuned again while the old stream has the TV"
        );
        drop(old);
        assert_eq!(next.join().unwrap(), Ok(()));
        assert_eq!(*fake.tuned.lock(), ["1_32_2_4_0_0_0", "1_32_2_1_0_0_0"]);
    }

    #[test]
    fn a_request_while_a_stream_that_ended_lets_go_of_the_tv_waits_for_it() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), on_air());
        let f = Arc::new(facade(&fake, &time));
        let mut ended = f.open("2.4", PLEX).unwrap();
        assert_eq!(
            ended
                .pump(&mut Vec::new(), &AtomicBool::new(false))
                .unwrap(),
            End::Silent
        );
        let next = {
            let f = f.clone();
            thread::spawn(move || f.open("2.1", OTHER).map(drop))
        };
        until_asked(&ended);
        drop(ended);
        assert_eq!(
            next.join().unwrap(),
            Ok(()),
            "it waited for the TV instead of being refused"
        );
    }

    #[test]
    fn a_request_still_being_tuned_gives_way_to_the_same_clients_next_one() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), on_air());
        let f = Arc::new(facade(&fake, &time));
        let tuning = f.tuning.take(PLEX, Duration::ZERO).unwrap();
        let next = {
            let f = f.clone();
            thread::spawn(move || f.open("2.1", PLEX).map(drop))
        };
        let end = Instant::now() + Duration::from_secs(5);
        while !tuning.replaced.load(Ordering::Acquire) {
            assert!(Instant::now() < end, "nobody asked for the tuner");
            thread::yield_now();
        }
        let entry = f.lineup().unwrap().remove(1);
        assert_eq!(f.find(&entry, tuning).err(), Some(Failure::Busy));
        assert_eq!(next.join().unwrap(), Ok(()));
        assert_eq!(
            *fake.tuned.lock(),
            ["1_32_2_1_0_0_0"],
            "only the next one was tuned"
        );
    }

    #[test]
    fn a_channel_that_is_not_in_the_lineup_is_not_found() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), vec![]);
        assert_eq!(
            facade(&fake, &time).open("9.9", PLEX).err(),
            Some(Failure::UnknownChannel)
        );
        assert!(fake.tuned.lock().is_empty());
    }

    #[test]
    fn the_tv_in_use_is_refused_before_the_tuner_is_touched_and_frees_the_lease() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), on_air());
        *fake.owner.lock() = Some("Live TV is in front".into());
        let f = facade(&fake, &time);
        assert_eq!(
            f.open("2.4", PLEX).err(),
            Some(Failure::TvInUse("Live TV is in front".into()))
        );
        assert!(fake.tuned.lock().is_empty());
        assert_eq!(
            f.tuning.status(),
            Status::Busy("Live TV is in front".into())
        );
        *fake.owner.lock() = None;
        assert!(
            f.open("2.4", PLEX).is_ok(),
            "the refusal did not keep the tuner"
        );
    }

    #[test]
    fn the_pipeline_unrealized_by_the_tv_ends_the_stream_though_the_packets_go_on() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), on_air());
        let f = facade_with(&fake, &time, asking_often());
        let mut stream = f.open("2.4", PLEX).unwrap();
        *fake.taken.lock() = Some("the TV took the tuner (BROADCAST_STATE_UNREALIZED)".into());
        let end = stream
            .pump(&mut Vec::new(), &AtomicBool::new(false))
            .unwrap();
        assert_eq!(
            end,
            End::Preempted("the TV took the tuner (BROADCAST_STATE_UNREALIZED)".into())
        );
    }

    #[test]
    fn the_tv_taking_the_tuner_back_ends_a_running_stream_with_the_reason() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), on_air());
        let f = facade_with(&fake, &time, asking_often());
        let mut stream = f.open("2.4", PLEX).unwrap();
        *fake.owner.lock() = Some("Live TV is in front".into());
        let end = stream
            .pump(&mut Vec::new(), &AtomicBool::new(false))
            .unwrap();
        assert_eq!(end, End::Preempted("Live TV is in front".into()));
        drop(stream);
        assert_eq!(
            f.tuning.status(),
            Status::Busy("Live TV is in front".into()),
            "over, and the tuner is the TV's"
        );
    }

    #[test]
    fn a_program_that_asks_not_to_be_passed_on_is_not_served_and_the_tv_is_let_go() {
        for tags in [&[0x09][..], &[0xAA][..]] {
            let time = Time::default();
            let fake = fake_with(&time, rf32(tags), vec![]);
            assert_eq!(
                facade(&fake, &time).open("2.4", PLEX).err(),
                Some(Failure::Protected)
            );
            assert_eq!(fake.released.load(Ordering::Relaxed), 1);
            assert!(fake.started.lock().is_empty(), "nothing was routed");
        }
    }

    #[test]
    fn a_channel_table_in_two_sections_is_read_whole_and_a_lost_section_is_read_again_alone() {
        let time = Time::default();
        let mut mux = rf32(&[]);
        mux.tvct = vec![
            tvct_part(739, 0, 1, &[("WSB-HD", 2, 1, 1, false)]),
            tvct_part(739, 1, 1, &[("WEST", 2, 4, 4, false)]),
        ];
        let fake = fake_with(&time, mux, on_air());
        fake.lost.lock().push((psi::PID_PSIP, psi::TID_TVCT, 1));
        assert!(
            facade(&fake, &time).open("2.4", PLEX).is_ok(),
            "2.4 is in the second section"
        );
        let read: Vec<u8> = fake
            .reads
            .lock()
            .iter()
            .filter(|r| r.0 == psi::PID_PSIP)
            .map(|r| r.2)
            .collect();
        assert_eq!(
            read,
            [0, 1, 1],
            "the first section is kept while the lost one comes round again"
        );
    }

    #[test]
    fn a_channel_table_of_the_last_multiplex_is_read_again_until_the_new_one_comes() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), on_air());
        *fake.stale.lock() = Some(tvct(741, &[("WAGA-HD", 5, 1, 1, false)]));
        assert!(facade(&fake, &time).open("2.4", PLEX).is_ok());
    }

    #[test]
    fn a_channel_missing_from_a_clean_multiplex_is_not_on_air_and_otherwise_a_weak_signal() {
        let clean = CLEAN;
        let weak = Signal {
            errors: Some(12),
            ..clean
        };
        let only_2_1 = || vec![tvct(739, &[("WSB-HD", 2, 1, 1, false)])];
        let full = rf32(&[]);
        for (tables, programs, signal, failure) in [
            (
                only_2_1(),
                full.pat.clone(),
                clean,
                Failure::OffAir("the multiplex does not list the channel (2.4)".into()),
            ),
            (
                only_2_1(),
                full.pat.clone(),
                weak,
                Failure::WeakSignal(
                    "the multiplex does not list the channel (2.4, 12 packet errors)".into(),
                ),
            ),
            (
                full.tvct.clone(),
                pat(739, &[(1, 0x30)]),
                clean,
                Failure::OffAir("the PAT does not list the program (2.4)".into()),
            ),
            (
                vec![],
                full.pat.clone(),
                clean,
                Failure::WeakSignal("no whole channel table (2.4, 0 packet errors)".into()),
            ),
        ] {
            let time = Time::default();
            let mut mux = rf32(&[]);
            (mux.tvct, mux.pat) = (tables, programs);
            let fake = fake_with(&time, mux, vec![]);
            *fake.signal.lock() = Some(signal);
            assert_eq!(facade(&fake, &time).open("2.4", PLEX).err(), Some(failure));
            assert_eq!(fake.released.load(Ordering::Relaxed), 1);
        }
    }

    #[test]
    fn the_lineup_is_asked_for_once_a_minute_and_a_silent_tv_leaves_the_last_one() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), vec![]);
        let f = facade(&fake, &time);
        assert_eq!(f.lineup().unwrap().len(), 2);
        time.advance(Duration::from_secs(59));
        f.lineup().unwrap();
        assert_eq!(fake.lists.load(Ordering::Relaxed), 1);
        fake.list_fails.store(true, Ordering::Relaxed);
        time.advance(Duration::from_secs(2));
        assert_eq!(f.lineup().unwrap().len(), 2, "stale is better than nothing");
        assert_eq!(fake.lists.load(Ordering::Relaxed), 2);
        let first_boot = facade(&fake, &time);
        assert_eq!(
            first_boot.lineup().err(),
            Some(Failure::Hardware("luna-send gave no answer".into()))
        );
    }

    #[test]
    fn the_guide_is_read_once_per_channel_and_kept_for_its_ttl() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), vec![]);
        let f = facade(&fake, &time);
        let xml = f.guide().unwrap();
        assert_eq!(
            *fake.programmes_asked.lock(),
            ["1_32_2_1_0_0_0", "1_32_2_4_0_0_0"]
        );
        assert!(xml.contains("<programme start=\"20261105230000 +0000\" stop=\"20261106000000 +0000\" channel=\"2.4\"><title>News on 1_32_2_4_0_0_0</title>"));
        time.advance(Duration::from_secs(29 * 60));
        assert_eq!(f.guide().unwrap(), xml);
        assert_eq!(fake.programmes_asked.lock().len(), 2);
        time.advance(Duration::from_secs(2 * 60));
        f.guide().unwrap();
        assert_eq!(fake.programmes_asked.lock().len(), 4);
    }

    #[test]
    fn a_channel_the_tv_cannot_answer_for_is_listed_bare_and_a_dead_guide_leaves_the_last() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), vec![]);
        let f = facade(&fake, &time);
        fake.programmes_fail.lock().push("1_32_2_1_0_0_0".into());
        let xml = f.guide().unwrap();
        assert!(xml.contains("<channel id=\"2.1\">") && !xml.contains("channel=\"2.1\""));
        assert!(xml.contains("channel=\"2.4\"><title>"));
        fake.programmes_fail.lock().push("1_32_2_4_0_0_0".into());
        time.advance(Duration::from_secs(31 * 60));
        assert_eq!(f.guide().unwrap(), xml, "stale is better than nothing");
        assert_eq!(
            facade(&fake, &time).guide().err(),
            Some(Failure::Hardware("luna-send gave no answer".into()))
        );
    }

    /// A host guide that names how many channels it was given, and has a revision a test moves.
    struct HostGuide {
        revision: std::sync::atomic::AtomicU64,
        own: AtomicBool,
    }

    impl Guide for HostGuide {
        fn xmltv(&self, channels: &[(Entry, Value)]) -> Option<String> {
            (!self.own.load(Ordering::Relaxed))
                .then(|| format!("<tv>{} channels</tv>", channels.len()))
        }

        fn revision(&self) -> u64 {
            self.revision.load(Ordering::Relaxed)
        }
    }

    #[test]
    fn the_hosts_guide_is_served_when_it_has_one_and_made_again_when_it_changes() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), vec![]);
        let host = Arc::new(HostGuide {
            revision: 1.into(),
            own: AtomicBool::new(false),
        });
        let f = facade_guided(&fake, &time, Limits::default(), Some(host.clone()));
        assert_eq!(&*f.guide().unwrap(), "<tv>2 channels</tv>");
        assert_eq!(
            fake.programmes_asked.lock().len(),
            2,
            "the TV's own listings are what it is made from"
        );
        f.guide().unwrap();
        assert_eq!(fake.programmes_asked.lock().len(), 2, "kept");
        host.own.store(true, Ordering::Relaxed);
        host.revision.store(2, Ordering::Relaxed);
        assert!(
            f.guide()
                .unwrap()
                .contains("<title>News on 1_32_2_4_0_0_0</title>"),
            "no host guide now: the TV's own"
        );
        assert_eq!(fake.programmes_asked.lock().len(), 4);
    }

    #[test]
    fn a_failure_says_its_status_and_why() {
        assert_eq!(
            [
                Failure::UnknownChannel,
                Failure::Busy,
                Failure::TvInUse("x".into()),
                Failure::NoSignal("x".into()),
                Failure::WeakSignal("x".into()),
                Failure::OffAir("x".into()),
                Failure::Protected,
                Failure::Hardware("x".into()),
                Failure::UnknownProfile("x".into()),
                Failure::Converter(Fault::Failed("x".into()))
            ]
            .map(|f| f.status()),
            [404, 503, 503, 503, 503, 503, 403, 502, 400, 502]
        );
        assert_eq!(Failure::Busy.reason(), "all tuners in use");
        assert_eq!(
            Failure::WeakSignal("no PMT (16.4)".into()).reason(),
            "weak signal: no PMT (16.4)"
        );
        assert_eq!(
            Failure::OffAir("no picture (16.9)".into()).reason(),
            "not on air: no picture (16.9)"
        );
        assert_eq!(Failure::UnknownProfile("fast".into()).reason(), "no such transcode profile: fast");
        assert_eq!(Failure::Converter(Fault::Stalled("no picture came back".into())).reason(), "converter failure: no picture came back");
    }

    #[test]
    fn the_meter_grades_a_reading_by_its_lost_packets_and_its_margin() {
        let fair = Signal { mse: Some(141), snr: Some(21.8), ..CLEAN };
        let rf29 = Signal { mse: Some(355), snr: Some(17.8), errors: Some(81), ber: Some(584_211), ..CLEAN };
        let unlocked = Signal { locked: false, mse: None, snr: None, ..CLEAN };
        assert_eq!([CLEAN, fair, rf29, unlocked].map(|s| s.grade()), [Grade::Good, Grade::Fair, Grade::Poor, Grade::None]);
        assert!(CLEAN.better_than(&fair) && fair.better_than(&rf29) && !rf29.better_than(&fair));
    }

    #[test]
    fn a_refused_request_is_told_who_has_the_tuner() {
        let claim = |agent: Option<&str>| Claim {
            peer: OTHER,
            agent: agent.map(str::to_owned),
            device: None,
            recording: false,
            guide: "2.1".into(),
            name: "WSB-HD".into(),
            channel_id: "1_32_2_1_0_0_0".into(),
            since_ms: 0,
        };
        let line = |agent: Option<&str>, names: &HashMap<IpAddr, String>, programme: Option<&str>| {
            let c = claim(agent);
            busy_line(&c, client_name(&c, names).as_deref(), programme)
        };
        let none = HashMap::new();
        assert_eq!(line(Some("PlexMediaServer/1.41"), &none, Some("Jeopardy!")), "Plex is recording Jeopardy! on 2.1 WSB-HD");
        assert_eq!(line(Some("Lavf/60.3"), &HashMap::from([(OTHER, "Plex".into())]), None), "Plex is recording 2.1 WSB-HD");
        assert_eq!(line(Some("VLC/3.0.20 LibVLC/3.0.20"), &none, None), "In use by VLC");
        assert_eq!(line(None, &none, None), "In use by 192.168.1.70");
    }

    /// 2.4 as the antenna sends it for a converter to take: `n` MPEG-2 pictures (a PES with a sequence header, a PTS and a PCR), each
    /// followed by a packet of its AC-3.
    pub(crate) fn program(n: u8) -> Vec<Vec<u8>> {
        let mut cc = 0;
        (0..n)
            .map(|i| {
                let pts = 90_000 + u64::from(i) * 3003;
                let pcr = 1_000_000 + u64::from(i) * 2970;
                [converter::picture(&mut cc, pts, i, Some(pcr)), converter::sound(i, 0x10 + i)].concat()
            })
            .collect()
    }

    /// One request for 2.4, pumped until it ends: how it ended and what it sent.
    fn ask(f: &Facade, transcode: Option<&str>) -> (End, Vec<u8>) {
        let Ok(Admitted::Plain(mut stream)) = f.admit("2.4", PLEX, None, None, transcode) else {
            panic!("not a plain stream");
        };
        let mut out = Vec::new();
        let end = stream.pump(&mut out, &AtomicBool::new(false)).unwrap();
        (end, out)
    }

    /// What the first PMT in `out` says the picture is.
    pub(crate) fn video_type(out: &[u8]) -> u8 {
        let pmt = out.chunks(PACKET).find(|p| psi::pid_of_packet(p) == 0x60).and_then(|p| psi::parse_pmt(&p[5..]));
        pmt.expect("a PMT").streams[0].stream_type
    }

    #[test]
    fn a_request_for_h264_is_the_converters_pictures_under_a_pmt_that_says_so_and_the_sound_goes_as_it_was() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), program(6));
        let codec = converter::Fake::default();
        let f = facade(&fake, &time);
        f.tuning.options.lock().codec = Some(Arc::new(codec.clone()));
        let Ok(Admitted::Plain(mut stream)) = f.admit("2.4", PLEX, None, None, Some("MOBILE")) else {
            panic!("not a plain stream");
        };
        let source = Format { width: 1920, height: 1080, rate: 4, aspect: 3, progressive: false };
        let made = Want { width: 1280, height: 720, rate: (30000, 1001), progressive: true, kbps: 3500 };
        assert_eq!(codec.0.lock().opened, [(source, made)], "opened for the station's picture and what mobile is");
        let mut out = Vec::new();
        assert_eq!(stream.pump(&mut out, &AtomicBool::new(false)).unwrap(), End::Silent);
        let ts = converter::read(&out);
        assert_eq!(ts.iter().take(2).map(|p| p.pid).collect::<Vec<_>>(), [0, 0x60], "a PAT, then a PMT");
        let pmt = psi::parse_pmt(&out[PACKET + 5..PACKET * 2]).unwrap();
        assert_eq!(
            pmt.streams.iter().map(|e| (e.stream_type, e.pid)).collect::<Vec<_>>(),
            [(0x1B, 0x61), (0x81, 0x64)],
            "which says the picture is H.264 and keeps the sound"
        );
        let pts: Vec<u64> = converter::pictures(&ts).iter().map(|p| p.0).collect();
        assert_eq!(pts, (0..5).map(|i| 90_000 + i * 3003).collect::<Vec<u64>>(), "each picture keeps its PTS; the last waits for the next to start");
        let heard: Vec<u8> = ts.iter().filter(|p| p.pid == 0x64).map(|p| p.payload[0]).collect();
        assert_eq!(heard, [0x10, 0x11, 0x12, 0x13, 0x14, 0x15], "the sound is as it was");
        let pats = ts.iter().filter(|p| p.pid == 0).count();
        assert!(pats > 10 && ts.iter().filter(|p| p.pid == 0x60).count() == pats, "every PAT comes with the PMT while the stream runs");
        drop(stream);
        assert_eq!((codec.0.lock().closed, fake.released.load(Ordering::Relaxed)), (1, 1), "the converter and the TV are let go");
    }

    #[test]
    fn a_request_for_h264_is_served_as_the_station_sends_it_without_a_converter_or_when_it_cannot_start_and_a_failed_one_rests() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), program(6));
        let f = facade(&fake, &time);
        let (_, plain) = ask(&f, None);
        assert_eq!(video_type(&plain), 0x02);
        assert_eq!(ask(&f, Some("mobile")).1, plain, "no converter is installed: the query changes nothing");
        assert_eq!(ask(&f, Some("faster")).1, plain, "not even one that names a profile there is none of");

        let codec = converter::Fake::default();
        f.tuning.options.lock().codec = Some(Arc::new(codec.clone()));
        codec.0.lock().open = Some(Fault::Unsupported("more than the hardware takes".into()));
        for _ in 0..2 {
            assert_eq!(ask(&f, Some("heavy")).1, plain, "a picture the converter cannot take goes out as it is");
        }
        assert_eq!(codec.0.lock().opens, 2, "that is no failure: it is asked again");

        codec.0.lock().open = Some(Fault::Failed("the encoder did not open".into()));
        assert_eq!(ask(&f, Some("mobile")).1, plain);
        assert_eq!(ask(&f, Some("mobile")).1, plain);
        assert_eq!(codec.0.lock().opens, 3, "one that failed rests, and is not asked meanwhile");
        time.advance(Duration::from_secs(61));
        codec.0.lock().open = None;
        assert_eq!(video_type(&ask(&f, Some("mobile")).1), 0x1B, "and is tried again once it has rested");
        assert_eq!(codec.0.lock().opens, 4);
    }

    #[test]
    fn a_converter_that_fails_while_the_stream_runs_ends_it_and_rests_so_the_next_request_is_as_the_station_sends_it() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), program(6));
        let f = facade(&fake, &time);
        let codec = converter::Fake::default();
        codec.0.lock().fail_at = Some(3);
        f.tuning.options.lock().codec = Some(Arc::new(codec.clone()));
        let (end, out) = ask(&f, Some("mobile"));
        assert_eq!(end, End::Failed(Failure::Converter(Fault::Failed("the encoder said no".into()))));
        assert_eq!(video_type(&out), 0x1B, "it was H.264 as far as it went");
        let (end, out) = ask(&f, Some("mobile"));
        assert_eq!((end, video_type(&out)), (End::Silent, 0x02), "the converter rests: the next request is the station's own");
        time.advance(Duration::from_secs(61));
        let (end, out) = ask(&f, Some("mobile"));
        assert_eq!((end, video_type(&out)), (End::Silent, 0x1B), "and it is tried again once it has rested");
    }

    #[test]
    fn a_station_that_already_sends_h264_is_served_as_it_is_and_the_converter_is_not_opened() {
        let time = Time::default();
        let mut mux = rf32(&[]);
        mux.pmts.insert(0x60, pmt(4, 0x61, &[], &[(0x1B, 0x61, &[]), (0x81, 0x64, &[])]));
        let mut sps = vec![0x47, 0x40, 0x61, 0x10, 0, 0, 1, 0x67];
        sps.resize(PACKET, 0xAB);
        let fake = fake_with(&time, mux, vec![sps]);
        let f = facade(&fake, &time);
        let (_, plain) = ask(&f, None);
        let codec = converter::Fake::default();
        f.tuning.options.lock().codec = Some(Arc::new(codec.clone()));
        assert_eq!(ask(&f, Some("mobile")).1, plain);
        assert_eq!((codec.0.lock().opens, video_type(&plain)), (0, 0x1B));
    }
}
