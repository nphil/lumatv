//! One tuner, one stream at a time: the lineup, the lease on the tuner, what a stream is (a PAT naming the one program, then that
//! program's packets) and what the tuner is doing. The hardware is behind two small traits so all of it runs in tests with a scripted fake
//! and a fake clock.
//!
//! A client counts one tuner, so a new request from the client whose stream has the tuner means it gave that stream up: Plex asks for
//! the next channel before it closes the last one. That request takes the tuner over: the old stream is told to end, and the TV is let
//! go before it is tuned again. A stream that is over is only letting go of the TV (a pipeline to close, the channel to put back), so a
//! request then waits for it too. Another client's running stream keeps the tuner (503).

use crate::Status;
use crate::docs::{self, Entry, Identity};
use crate::psi::{self, PACKET};
use parking_lot::{Condvar, Mutex};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{self, Write};
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Why a stream could not start, or had to end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The lineup has no such number.
    UnknownChannel,
    /// Another stream has the tuner (`TunerCount` is 1).
    Busy,
    /// The TV's own use has the tuner: the reason, for the owner's log and the client's body.
    TvInUse(String),
    /// The tuner locked nothing that carries the channel.
    NoSignal(String),
    /// The program is scrambled or asks not to be passed on.
    Protected,
    /// The tuner, the demux or the luna call said no.
    Hardware(String),
}

impl Failure {
    /// The HTTP status of a stream that could not start. A tuner in use is 503, which is what an HDHomeRun answers and what Plex waits on.
    pub fn status(&self) -> u16 {
        match self {
            Self::UnknownChannel => 404,
            Self::Busy | Self::TvInUse(_) | Self::NoSignal(_) => 503,
            Self::Protected => 403,
            Self::Hardware(_) => 502,
        }
    }

    pub fn reason(&self) -> String {
        match self {
            Self::UnknownChannel => "no such channel".to_owned(),
            Self::Busy => "all tuners in use".to_owned(),
            Self::TvInUse(why) => format!("the TV is in use: {why}"),
            Self::NoSignal(why) => format!("no signal: {why}"),
            Self::Protected => "the program is protected".to_owned(),
            Self::Hardware(why) => format!("tuner failure: {why}"),
        }
    }

    /// The error an HDHomeRun names in `X-HDHomeRun-Error`, which players read the cause from (SiliconDust's HTTP API, 2026-07-14).
    pub fn hdhomerun_error(&self) -> &'static str {
        match self {
            Self::UnknownChannel => "801 Unknown Channel",
            Self::Busy | Self::TvInUse(_) => "805 All Tuners In Use",
            Self::Hardware(_) => "806 Tune Failed",
            Self::NoSignal(_) => "807 No Video Data",
            Self::Protected => "811 Content Protection Required",
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
    /// Tunes LG's `channel_id` and returns the tap of what the antenna carries. Dropping the tap lets everything go: the demux, the
    /// pipeline, and the TV's last channel, which is put back.
    fn tune(&self, channel_id: &str) -> Result<Box<dyn Tap>, Failure>;
}

/// The antenna's multiplex through the demux.
pub trait Tap: Send {
    /// One whole section (`pid`, `table_id`, `section_number`) within `wait`.
    fn section(&mut self, pid: u16, table_id: u8, number: u8, wait: Duration) -> Option<Vec<u8>>;
    /// Routes these PIDs to the download; `read` gives their packets from here on.
    fn start(&mut self, pids: &[u16]) -> Result<(), Failure>;
    /// Appends the whole 188-byte packets that arrived within `wait` (none is fine) to `out`.
    fn read(&mut self, out: &mut Vec<u8>, wait: Duration) -> Result<(), Failure>;
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
    /// How long a table is waited for: the PSIP and the PAT repeat in well under a second on air.
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
    /// How long the program's first whole picture may take once the program is routed before the channel is taken to be off the air.
    /// Measured on the set on 2026-11-05 over its 59 channels: the first picture packet came 0.1 to 0.7 s after routing, the first whole
    /// picture up to 2.2 s (36.4). An HDHomeRun gives up on a program after 5 s.
    pub first_picture: Duration,
    /// How long a channel found off the air (no lock, tables that do not come whole or do not list it, no whole picture) is left out of the
    /// lineup and refused at once. A weak RF channel comes and goes: 30.1 lost most of its packets at 14:24 that day and none at 14:47.
    pub off_air: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            lineup_ttl: Duration::from_secs(60),
            section_wait: Duration::from_secs(6),
            silence: Duration::from_secs(5),
            owner_every: Duration::from_secs(5),
            pat_every: Duration::from_millis(100),
            poll: Duration::from_millis(200),
            guide_ttl: Duration::from_secs(30 * 60),
            release: Duration::from_secs(5),
            first_picture: Duration::from_secs(4),
            off_air: Duration::from_secs(30 * 60),
        }
    }
}

/// Time since some start; the real one is the monotonic clock.
pub type Clock = Arc<dyn Fn() -> Duration + Send + Sync>;

pub fn real_clock() -> Clock {
    let start = Instant::now();
    Arc::new(move || start.elapsed())
}

/// Who has the tuner: the client whose stream it is, the flag that ends that stream, and whether the stream is over and only letting go
/// of the TV.
struct Holding {
    peer: IpAddr,
    replaced: Arc<AtomicBool>,
    over: bool,
}

/// The tuner, held: dropping it frees it and wakes whoever waits for it.
struct Lease {
    tuning: Arc<Tuning>,
    /// Set when another request takes the tuner over.
    replaced: Arc<AtomicBool>,
}

impl Lease {
    /// The stream is over: what is left is letting go of the TV, which a new request waits for instead of being refused.
    fn over(&self) {
        if let Some(h) = self.tuning.held.lock().as_mut() {
            h.over = true;
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
/// and the wake-up of whoever waits for it, and what it is doing, with the host's wake-up for a change.
#[derive(Default)]
pub(crate) struct Tuning {
    held: Mutex<Option<Holding>>,
    freed: Condvar,
    status: Mutex<Status>,
    wake: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl Tuning {
    /// Takes the tuner for `peer`. A stream of the same client, or one that is over, is told to end and waited for, at most `wait`;
    /// another client's running stream keeps it.
    fn take(self: &Arc<Self>, peer: IpAddr, wait: Duration) -> Result<Lease, Failure> {
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
                    });
                    return Ok(Lease {
                        tuning: self.clone(),
                        replaced,
                    });
                }
                Some(h) if h.over || h.peer == peer => h.replaced.store(true, Ordering::Release),
                Some(_) => return Err(Failure::Busy),
            }
            if self.freed.wait_until(&mut held, deadline).timed_out() && held.is_some() {
                return Err(Failure::Busy);
            }
        }
    }

    pub(crate) fn status(&self) -> Status {
        self.status.lock().clone()
    }

    pub(crate) fn on_change(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        *self.wake.lock() = Some(wake);
    }

    /// Moves to what `next` makes of the status now (`None`: it stays) and wakes the host when that is a change. The host is woken outside
    /// the lock and reads the status itself, so wake-ups that cross still leave it with the last one.
    fn change(&self, next: impl FnOnce(&Status) -> Option<Status>) {
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
        let wake = changed.then(|| self.wake.lock().clone()).flatten();
        if let Some(wake) = wake {
            wake();
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

    fn refused(&self, why: &str) {
        self.change(|now| (*now != Status::Off).then(|| Status::Busy(why.to_owned())));
    }

    fn streaming(&self, entry: &Entry) {
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
struct Over {
    tuning: Arc<Tuning>,
    then: Status,
}

impl Drop for Over {
    fn drop(&mut self) {
        let then = std::mem::replace(&mut self.then, Status::Idle);
        self.tuning
            .change(|now| matches!(now, Status::Streaming { .. }).then_some(then));
    }
}

/// The channels of a multiplex: every section of the virtual channel table (the first says how many there are).
fn channels_of(tap: &mut dyn Tap, wait: Duration) -> Option<Vec<psi::Channel>> {
    let first = tap.section(psi::PID_PSIP, psi::TID_TVCT, 0, wait)?;
    let last = *first.get(7)?;
    let mut all = psi::parse_tvct(&first)?;
    for n in 1..=last {
        all.extend(
            tap.section(psi::PID_PSIP, psi::TID_TVCT, n, wait)
                .and_then(|s| psi::parse_tvct(&s))?,
        );
    }
    Some(all)
}

pub(crate) struct Facade {
    hw: Arc<dyn Hardware>,
    pub(crate) identity: Identity,
    limits: Limits,
    clock: Clock,
    lineup: Mutex<Option<(Duration, Vec<Entry>)>>,
    /// The host's guide, when it has one: `/xmltv.xml` is its document when it gives one.
    source: Option<Arc<dyn Guide>>,
    /// The document served, when it was made and from which revision of the host's guide.
    guide: Mutex<Option<(Duration, u64, Arc<str>)>>,
    /// Channels found off the air, by guide number: when, and why.
    off_air: Mutex<HashMap<String, (Duration, String)>>,
    tuning: Arc<Tuning>,
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
            off_air: Mutex::new(HashMap::new()),
            tuning,
        }
    }

    /// The TV's antenna channels. Read again after a minute; a TV that does not answer leaves the last list standing.
    fn entries(&self) -> Result<Vec<Entry>, Failure> {
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

    /// The lineup the documents list: the TV's antenna channels less those found off the air within `off_air`.
    pub(crate) fn lineup(&self) -> Result<Vec<Entry>, Failure> {
        let mut entries = self.entries()?;
        let now = (self.clock)();
        let mut off = self.off_air.lock();
        off.retain(|_, (at, _)| now.saturating_sub(*at) < self.limits.off_air);
        entries.retain(|e| !off.contains_key(&e.guide));
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

    /// Takes the tuner for channel `guide` (`2.4`) for the client at `peer`, tunes it, finds the program and waits for its picture.
    /// Nothing has been sent to the client yet, so a failure is still an HTTP status. A channel found off the air (no lock, not in its
    /// multiplex, no picture) is left out of the lineup and refused at once for `off_air`, then tried again.
    pub(crate) fn open(&self, guide: &str, peer: IpAddr) -> Result<Stream, Failure> {
        let entry = self
            .entries()?
            .into_iter()
            .find(|e| e.guide == guide)
            .ok_or(Failure::UnknownChannel)?;
        let now = (self.clock)();
        if let Some((at, why)) = self.off_air.lock().get(guide)
            && now.saturating_sub(*at) < self.limits.off_air
        {
            return Err(Failure::NoSignal(format!(
                "{why}, {} min ago",
                now.saturating_sub(*at).as_secs() / 60
            )));
        }
        let lease = self.tuning.take(peer, self.limits.release)?;
        if let Some(why) = self.hw.in_use() {
            self.tuning.refused(&why);
            return Err(Failure::TvInUse(why));
        }
        let found = self.find(&entry, lease);
        if let Err(Failure::NoSignal(why)) = &found {
            self.off_air
                .lock()
                .insert(entry.guide.clone(), ((self.clock)(), why.clone()));
        }
        found
    }

    /// Tunes `entry`'s channel, finds its program in the multiplex, routes it and waits for its picture. A request of the same client that
    /// comes meanwhile takes the tuner over: this one gives up between steps (Plex asks for the next channel while the last is still tuned
    /// when channels are flipped, and its transcoder asks again for a stream that ended).
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
        let wait = self.limits.section_wait;
        let no = |what: &str| Failure::NoSignal(format!("{what} ({})", entry.guide));
        // The tables are read as soon as the frontend has locked. The previous multiplex's could still be on the demux for a moment (not
        // seen on the set on 2026-11-05: the first PAT after the lock was always the new multiplex's), so tables that do not name the
        // channel are read again until `wait` has passed.
        let (channel, pat, pmt_pid) = loop {
            if let Some(f) = given_up() {
                return Err(f);
            }
            // a table that is not read whole within `wait` is a weak signal (on RF 29 the same afternoon, with the demod counting packet
            // errors, five 16.x channels' tables did not come whole in 6 s); one read whole without the channel is a multiplex without it
            let found = channels_of(tap.as_mut(), wait)
                .ok_or("no whole channel table")
                .and_then(|all| {
                    all.into_iter()
                        .find(|c| (c.major, c.minor) == (entry.major, entry.minor))
                        .ok_or("the multiplex does not list the channel")
                })
                .and_then(|channel| {
                    let pat = tap
                        .section(psi::PID_PAT, psi::TID_PAT, 0, wait)
                        .and_then(|s| psi::parse_pat(&s))
                        .ok_or("no PAT")?;
                    let pmt_pid = pat
                        .programs
                        .iter()
                        .find(|p| p.0 == channel.program)
                        .map(|p| p.1)
                        .ok_or("the PAT does not list the program")?;
                    Ok((channel, pat, pmt_pid))
                });
            match found {
                Ok(found) => break found,
                Err(why) if (self.clock)().saturating_sub(tuned) >= wait => return Err(no(why)),
                Err(_) => {}
            }
        };
        let pmt_section = tap
            .section(pmt_pid, psi::TID_PMT, 0, wait)
            .and_then(|s| psi::checked(&s, psi::TID_PMT).map(<[u8]>::to_vec))
            .ok_or_else(|| no("no PMT"))?;
        let pmt = psi::parse_pmt(&pmt_section).ok_or_else(|| no("no PMT"))?;
        if psi::protected(&pmt, Some(&channel)) {
            return Err(Failure::Protected);
        }
        let mut pids: Vec<u16> = [pmt_pid, pmt.pcr_pid]
            .into_iter()
            .chain(pmt.streams.iter().map(|e| e.pid))
            .collect();
        pids.sort_unstable();
        pids.dedup();
        // the program's picture tells whether the tuner is still where it was; a program without one is watched by its clock
        let picture = pmt
            .streams
            .iter()
            .find(|e| matches!(e.stream_type, 0x02 | 0x1B | 0x24))
            .map(|e| (e.pid, e.stream_type));
        let watch = picture.map_or(pmt.pcr_pid, |(pid, _)| pid);
        tap.start(&pids)?;
        let routed = (self.clock)();
        // The picture has to come through whole before the client is told yes. A station can list a channel whose PMT names a picture and
        // a sound that carry no packets (16.9 on 2026-11-05), and a weak RF channel can bring the picture too broken to start (30.1 the same
        // day, 559 packets lost in 12 s); a player takes either for a stream that has not started yet and keeps asking for it. What comes
        // meanwhile is kept for the client, which counts those frames too.
        let whole = |p: &[u8]| {
            psi::pid_of_packet(p) == watch
                && picture.is_none_or(|(_, stream_type)| psi::access_point(p, stream_type))
        };
        let mut first = Vec::with_capacity(PACKET * 512);
        let mut seen = 0;
        while !first[seen..].chunks_exact(PACKET).any(whole) {
            if let Some(f) = given_up() {
                return Err(f);
            }
            if (self.clock)().saturating_sub(routed) >= self.limits.first_picture {
                let broken = first
                    .chunks_exact(PACKET)
                    .any(|p| psi::pid_of_packet(p) == watch);
                return Err(no(if broken {
                    "no whole picture"
                } else {
                    "no picture"
                }));
            }
            seen = first.len();
            tap.read(&mut first, self.limits.poll)?;
        }
        let pictured = (self.clock)();
        self.tuning.streaming(entry);
        Ok(Stream {
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
            over: Over {
                tuning: self.tuning.clone(),
                then: Status::Idle,
            },
            lease,
        })
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
    /// The TV took the tuner for its own use.
    Preempted(String),
    Failed(Failure),
}

/// A stream that is tuned and routed, its picture arriving, and has not yet written a byte. Fields drop in order: the tap lets go of the TV,
/// the tuner says it is free, then the lease lets the next request have it (which then says what it streams).
pub(crate) struct Stream {
    tap: Box<dyn Tap>,
    hw: Arc<dyn Hardware>,
    pat: Vec<u8>,
    pmt_pid: u16,
    /// The program's PMT as read, sent first.
    pmt: Vec<u8>,
    /// The packets that came while the picture was waited for.
    first: Vec<u8>,
    watch: u16,
    /// How long the tune, the tables and the first picture took.
    pub(crate) took: [Duration; 3],
    limits: Limits,
    clock: Clock,
    over: Over,
    lease: Lease,
}

impl Stream {
    /// Writes the program until the client goes, `stop` is set, another request takes the tuner or the TV does: a PAT that names only
    /// this program, ten times a second, the program's PMT once at the start, and the packets as they arrive. However it ends, the stream
    /// is over from then on.
    pub(crate) fn pump(&mut self, out: &mut dyn Write, stop: &AtomicBool) -> io::Result<End> {
        let end = self.run(out, stop);
        self.lease.over();
        end
    }

    fn run(&mut self, out: &mut dyn Write, stop: &AtomicBool) -> io::Result<End> {
        let lim = &self.limits;
        let (mut pat, mut pmt) = (
            psi::SectionPackets::default(),
            psi::SectionPackets::default(),
        );
        // the PAT and the PMT lead, so a player knows the program from the first packet (the station's PMT comes up to 0.4 s later), then
        // the packets that came while the picture was waited for; the station's PMT packets go on from the count of the one written here
        let mut buf = Vec::with_capacity(PACKET * 512);
        pat.write(psi::PID_PAT, &self.pat, &mut buf);
        pmt.write(self.pmt_pid, &self.pmt, &mut buf);
        let mut first = std::mem::take(&mut self.first);
        for p in first.chunks_exact_mut(PACKET) {
            if psi::pid_of_packet(p) == self.pmt_pid {
                pmt.renumber(p);
            }
        }
        out.write_all(&buf)?;
        out.write_all(&first)?;
        drop(first);
        let mut last_pat = (self.clock)();
        let mut last_watch = last_pat;
        let mut last_owner = last_pat;
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(End::Stopped);
            }
            if self.lease.replaced.load(Ordering::Acquire) {
                return Ok(End::Replaced);
            }
            let now = (self.clock)();
            if now.saturating_sub(last_pat) >= lim.pat_every {
                buf.clear();
                pat.write(psi::PID_PAT, &self.pat, &mut buf);
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
                    return Ok(End::Preempted(why));
                }
            }
            buf.clear();
            if let Err(f) = self.tap.read(&mut buf, lim.poll) {
                return Ok(End::Failed(f));
            }
            let mut pictured = false;
            for p in buf.chunks_exact_mut(PACKET) {
                match psi::pid_of_packet(p) {
                    pid if pid == self.watch => pictured = true,
                    pid if pid == self.pmt_pid => pmt.renumber(p),
                    _ => {}
                }
            }
            if pictured {
                last_watch = (self.clock)();
            }
            if !buf.is_empty() {
                out.write_all(&buf)?;
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::psi::tests::{pat, pmt, tvct, tvct_part};
    use super::*;
    use serde_json::json;
    use std::collections::{HashMap, VecDeque};
    use std::net::Ipv4Addr;
    use std::sync::atomic::AtomicU64;
    use std::thread;

    /// Plex at home, and another client.
    const PLEX: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 69));
    const OTHER: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 70));

    /// The fake clock: reads wait by moving it, so a test of five silent seconds takes no time.
    #[derive(Clone, Default)]
    pub(crate) struct Time(Arc<AtomicU64>);

    impl Time {
        pub(crate) fn clock(&self) -> Clock {
            let ms = self.0.clone();
            Arc::new(move || Duration::from_millis(ms.load(Ordering::Relaxed)))
        }

        fn advance(&self, by: Duration) {
            self.0.fetch_add(by.as_millis() as u64, Ordering::Relaxed);
        }
    }

    #[derive(Clone)]
    pub(crate) struct Multiplex {
        tvct: Vec<Vec<u8>>,
        pat: Vec<u8>,
        pmts: HashMap<u16, Vec<u8>>,
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
        taken: Mutex<Option<String>>,
        pub(crate) tuned: Mutex<Vec<String>>,
        started: Mutex<Vec<Vec<u16>>>,
        released: AtomicU64,
        scripted: Vec<Vec<u8>>,
        lists: AtomicU64,
        list_fails: AtomicBool,
        /// The channels `programmes` was asked for, and the ones it fails for.
        programmes_asked: Mutex<Vec<String>>,
        programmes_fail: Mutex<Vec<String>>,
        /// A virtual channel table the next read gives instead of the multiplex's own (the last multiplex's, still on the demux).
        stale: Mutex<Option<Vec<u8>>>,
    }

    impl Fake {
        fn new(time: &Time, mux: Multiplex, scripted: Vec<Vec<u8>>) -> Arc<Self> {
            Arc::new(Self {
                time: time.clone(),
                mux,
                owner: Mutex::default(),
                taken: Mutex::default(),
                tuned: Mutex::default(),
                started: Mutex::default(),
                released: AtomicU64::new(0),
                scripted,
                lists: AtomicU64::new(0),
                list_fails: AtomicBool::new(false),
                programmes_asked: Mutex::default(),
                programmes_fail: Mutex::default(),
                stale: Mutex::default(),
            })
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

        fn tune(&self, channel_id: &str) -> Result<Box<dyn Tap>, Failure> {
            self.0.tuned.lock().push(channel_id.to_owned());
            Ok(Box::new(FakeTap {
                fake: self.0.clone(),
                queue: self.0.scripted.iter().cloned().collect(),
            }))
        }
    }

    impl Tap for FakeTap {
        /// A table takes a moment to come round; the first virtual channel table can be the last multiplex's.
        fn section(
            &mut self,
            pid: u16,
            table_id: u8,
            number: u8,
            _wait: Duration,
        ) -> Option<Vec<u8>> {
            self.fake.time.advance(Duration::from_millis(50));
            let m = &self.fake.mux;
            match (pid, table_id) {
                (psi::PID_PSIP, psi::TID_TVCT) => self
                    .fake
                    .stale
                    .lock()
                    .take()
                    .or_else(|| m.tvct.get(usize::from(number)).cloned()),
                (psi::PID_PAT, psi::TID_PAT) => Some(m.pat.clone()),
                (pid, psi::TID_PMT) => m.pmts.get(&pid).cloned(),
                _ => None,
            }
        }

        fn start(&mut self, pids: &[u16]) -> Result<(), Failure> {
            self.fake.started.lock().push(pids.to_vec());
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
    fn a_stream_is_a_pat_naming_one_program_then_its_pmt_then_that_programs_packets() {
        let time = Time::default();
        let chunk = [
            picture(0x61, 0),
            packet(0x64, 0),
            packet(0x60, 0),
            packet(0x61, 1),
        ]
        .concat();
        let fake = fake_with(&time, rf32(&[]), vec![chunk.clone()]);
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
            "everything the tuner gave, in order, the station's PMT counted on from the one written first"
        );
        assert!(
            packets.iter().filter(|p| pid(p) == 0).count() > 10,
            "the PAT goes on while the stream runs"
        );
        drop(stream);
        assert_eq!(fake.released.load(Ordering::Relaxed), 1, "the TV is let go");
    }

    #[test]
    fn a_channel_whose_picture_never_comes_is_off_the_air_refused_at_once_and_out_of_the_lineup_for_a_while()
     {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), vec![packet(0x60, 0)]);
        let f = facade(&fake, &time);
        let failed = f.open("2.4", PLEX).err();
        assert!(
            matches!(&failed, Some(Failure::NoSignal(why)) if why.contains("no picture")),
            "{failed:?}"
        );
        assert_eq!(fake.released.load(Ordering::Relaxed), 1, "the TV is let go");
        assert_eq!(f.tuning.status(), Status::Idle);
        let listed = || {
            f.lineup()
                .unwrap()
                .into_iter()
                .map(|e| e.guide)
                .collect::<Vec<_>>()
        };
        assert_eq!(listed(), ["2.1"]);
        time.advance(Duration::from_secs(60));
        assert!(matches!(f.open("2.4", PLEX), Err(Failure::NoSignal(_))));
        assert_eq!(fake.tuned.lock().len(), 1, "refused without tuning again");
        time.advance(Limits::default().off_air);
        assert_eq!(listed(), ["2.1", "2.4"]);
        assert!(f.open("2.4", PLEX).is_err());
        assert_eq!(fake.tuned.lock().len(), 2, "then tried again");
    }

    #[test]
    fn a_picture_that_never_comes_through_whole_is_refused() {
        let time = Time::default();
        let fake = fake_with(
            &time,
            rf32(&[]),
            vec![[packet(0x61, 0), packet(0x61, 2)].concat()],
        );
        let failed = facade(&fake, &time).open("2.4", PLEX).err();
        assert!(
            matches!(&failed, Some(Failure::NoSignal(why)) if why.contains("no whole picture")),
            "{failed:?}"
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
        let entry = f.entries().unwrap().remove(1);
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
    fn a_channel_table_in_two_sections_is_read_whole() {
        let time = Time::default();
        let mut mux = rf32(&[]);
        mux.tvct = vec![
            tvct_part(739, 0, 1, &[("WSB-HD", 2, 1, 1, false)]),
            tvct_part(739, 1, 1, &[("WEST", 2, 4, 4, false)]),
        ];
        let fake = fake_with(&time, mux, on_air());
        assert!(
            facade(&fake, &time).open("2.4", PLEX).is_ok(),
            "2.4 is in the second section"
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
    fn a_multiplex_that_does_not_carry_the_channel_or_whose_table_does_not_come_whole_is_no_signal()
    {
        for (tables, why) in [
            (
                vec![tvct(739, &[("WSB-HD", 2, 1, 1, false)])],
                "the multiplex does not list the channel (2.4)",
            ),
            (vec![], "no whole channel table (2.4)"),
        ] {
            let time = Time::default();
            let mut mux = rf32(&[]);
            mux.tvct = tables;
            let fake = fake_with(&time, mux, vec![]);
            assert_eq!(
                facade(&fake, &time).open("2.4", PLEX).err(),
                Some(Failure::NoSignal(why.into()))
            );
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
                Failure::Protected,
                Failure::Hardware("x".into())
            ]
            .map(|f| f.status()),
            [404, 503, 503, 503, 403, 502]
        );
        assert_eq!(Failure::Busy.reason(), "all tuners in use");
    }
}
