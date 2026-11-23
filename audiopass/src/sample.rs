//! What a player polls while it streams, kept apart from the route.
//!
//! The route's steps wait on LG for seconds (a luna call, the /proc task scan, a journal write), and an arm or a restore holds the
//! route throughout, so the sample a player polls is not taken from them. During a session the caller runs two threads that never
//! take the route: a reader that calls [`read`] every [`READ_MS`], and a sampler that calls [`Sample::refresh`] every [`SAMPLE_MS`]
//! with the route's last [`Snapshot`] and the reader's last [`Reading`]. A read of LG's driver can wait on the sound card's control
//! lock (WEDGE.md of the TrueHD study); then only the reader waits. The sampler goes on and stamps each sample with the time it
//! publishes it, so a sample's age tells only whether the sampler is alive, and its reading stops saying bypass once it is older
//! than [`READING_FRESH_MS`].

use crate::host::Host;
use crate::route::{DRIVER_BUSY, INSTANCES, Phase, Snapshot, status_text};
use crate::sndout::{self, Sndout};

/// How often the reader reads the kernel during a session.
pub const READ_MS: u64 = 50;
/// How often the sampler publishes during a session. A player polls every 50 ms and stops sending once a sample is 300 ms old
/// (`validForMs`): a live sampler's samples reach it at most about 100 ms old, so one late sample still passes.
pub const SAMPLE_MS: u64 = 50;
/// How old a reading may be and still say bypass. A read of LG's sound status answers in microseconds; one that has not come back
/// for four read periods waits on LG's driver or is starved, and the route may have changed meanwhile. A player polling every
/// 50 ms then stops sending about 300 ms after the last read that showed bypass, close to the 250 ms after which upstream
/// (`lgroute.c`) looks at Pass-Through again.
pub const READING_FRESH_MS: u64 = 200;
/// The reason a sample gives while its reading is older than [`READING_FRESH_MS`].
pub const NOT_READ: &str = "a read of LG's sound status has not come back";

/// What the kernel showed of one session's route at one read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    /// The converter of the session it was read for.
    pub converter: String,
    /// When the read started, on the clock of [`Host::now_ms`].
    pub at: u64,
    pub bypassing: bool,
    pub mixed: bool,
    pub converter_present: bool,
    /// Why not bypassing; empty when bypassing.
    pub why: String,
}

/// One read for the session whose converter is `converter`, with LG's sound status as it was read.
pub fn read(host: &dyn Host, converter: &str) -> (Reading, Option<Sndout>) {
    let at = host.now_ms();
    let snd = host
        .read(sndout::SNDOUT)
        .ok()
        .map(|text| sndout::parse(&text));
    let present = host.is_dir(&format!("{INSTANCES}/{converter}"));
    let why = match &snd {
        None if host.driver_busy() => DRIVER_BUSY,
        None => "LG's sound status cannot be read",
        Some(_) if !present => "LG rebuilt its audio path: the converter is gone",
        Some(s) if s.earc_on != Some(true) => "eARC is off",
        Some(s) if s.mixed() => "LG turned eARC mixing back on",
        Some(s) if !s.bypassing() => "the kernel is not reporting Pass-Through",
        Some(_) => "",
    };
    let reading = Reading {
        converter: converter.to_owned(),
        at,
        bypassing: why.is_empty(),
        mixed: snd.as_ref().is_some_and(Sndout::mixed),
        converter_present: present,
        why: why.to_owned(),
    };
    (reading, snd)
}

/// What a player is answered: the route's last state with the newest reading of the kernel, as at the time it was published.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sample {
    /// When it was published, on the clock of [`Host::now_ms`].
    pub at: u64,
    /// `None` without a session.
    pub phase: Option<Phase>,
    pub adec: Option<u8>,
    pub bypassing: bool,
    pub mixed: bool,
    pub converter_present: bool,
    pub connected: bool,
    pub audible: bool,
    pub reason: String,
}

impl Sample {
    /// Publishes `snapshot` at `now`. A session's reading is the newer of the route's own and `reading`, when that one is of the
    /// session's converter; it says bypass only while it is no older than [`READING_FRESH_MS`] and the session is not ending.
    pub fn refresh(&mut self, snapshot: &Snapshot, reading: Option<&Reading>, now: u64) {
        self.at = now;
        self.reason.clear();
        let Some(s) = &snapshot.session else {
            self.phase = None;
            self.adec = None;
            self.bypassing = false;
            self.mixed = false;
            self.converter_present = false;
            self.connected = false;
            self.audible = false;
            self.reason.push_str(&snapshot.reason);
            return;
        };
        let newer = reading.filter(|r| r.converter == s.converter && r.at > s.at);
        let (at, bypassing, mixed, present, why) = match newer {
            Some(r) => (r.at, r.bypassing, r.mixed, r.converter_present, r.why.as_str()),
            None => (s.at, s.bypassing, s.mixed, s.converter_present, s.why.as_str()),
        };
        let fresh = now.saturating_sub(at) <= READING_FRESH_MS;
        let ending = s.phase == Phase::Ending;
        self.phase = Some(s.phase);
        self.adec = Some(s.adec);
        self.bypassing = bypassing && fresh && !ending;
        self.mixed = mixed;
        self.converter_present = present;
        self.connected = s.connected;
        self.audible = s.audible;
        // An ending session keeps the reason it ended for, however old its last reading.
        self.reason.push_str(if fresh || ending {
            status_text(s.phase, why)
        } else {
            NOT_READ
        });
    }
}
