//! One tuner, one stream at a time: the lineup, the lease on the tuner, and what a stream is (a PAT naming the one program, then that
//! program's packets). The hardware is behind two small traits so all of it runs in tests with a scripted fake and a fake clock.

use super::docs::{self, Entry, Identity};
use super::psi::{self, PACKET};
use parking_lot::Mutex;
use serde_json::Value;
use std::io::{self, Write};
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
}

/// The TV.
pub trait Hardware: Send + Sync + 'static {
    /// LG's channel list (`iepg/getChannelList`'s reply).
    fn channel_list(&self) -> Result<Value, Failure>;
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

/// The time the facade waits and counts, one place so a test can set them small.
#[derive(Clone, Debug)]
pub struct Limits {
    /// How long a read channel list is kept.
    pub lineup_ttl: Duration,
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
        }
    }
}

/// Time since some start; the real one is the monotonic clock.
pub type Clock = Arc<dyn Fn() -> Duration + Send + Sync>;

pub fn real_clock() -> Clock {
    let start = Instant::now();
    Arc::new(move || start.elapsed())
}

/// The tuner, held: dropping it frees it.
struct Lease(Arc<AtomicBool>);

impl Lease {
    fn take(flag: &Arc<AtomicBool>) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
            .then(|| Self(flag.clone()))
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
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

pub struct Facade {
    hw: Arc<dyn Hardware>,
    pub identity: Identity,
    limits: Limits,
    clock: Clock,
    lineup: Mutex<Option<(Duration, Vec<Entry>)>>,
    busy: Arc<AtomicBool>,
}

impl Facade {
    pub fn new(hw: Arc<dyn Hardware>, identity: Identity, limits: Limits, clock: Clock) -> Self {
        Self {
            hw,
            identity,
            limits,
            clock,
            lineup: Mutex::new(None),
            busy: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The TV's antenna channels. Read again after a minute; a TV that does not answer leaves the last list standing.
    pub fn lineup(&self) -> Result<Vec<Entry>, Failure> {
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

    /// Takes the tuner for channel `guide` (`2.4`), tunes it and finds the program. Nothing has been sent to the client yet, so a failure
    /// is still an HTTP status.
    pub fn open(&self, guide: &str) -> Result<Stream, Failure> {
        let entry = self
            .lineup()?
            .into_iter()
            .find(|e| e.guide == guide)
            .ok_or(Failure::UnknownChannel)?;
        let lease = Lease::take(&self.busy).ok_or(Failure::Busy)?;
        if let Some(why) = self.hw.in_use() {
            return Err(Failure::TvInUse(why));
        }
        let mut tap = self.hw.tune(&entry.channel_id)?;
        let wait = self.limits.section_wait;
        let no = |what: &str| Failure::NoSignal(format!("{what} ({} is not on air)", entry.guide));
        let channel = channels_of(tap.as_mut(), wait)
            .and_then(|all| {
                all.into_iter()
                    .find(|c| (c.major, c.minor) == (entry.major, entry.minor))
            })
            .ok_or_else(|| no("the multiplex does not list the channel"))?;
        let pat = tap
            .section(psi::PID_PAT, psi::TID_PAT, 0, wait)
            .and_then(|s| psi::parse_pat(&s))
            .ok_or_else(|| no("no PAT"))?;
        let pmt_pid = pat
            .programs
            .iter()
            .find(|p| p.0 == channel.program)
            .map(|p| p.1)
            .ok_or_else(|| no("the PAT does not list the program"))?;
        let pmt = tap
            .section(pmt_pid, psi::TID_PMT, 0, wait)
            .and_then(|s| psi::parse_pmt(&s))
            .ok_or_else(|| no("no PMT"))?;
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
        let watch = pmt
            .streams
            .iter()
            .find(|e| matches!(e.stream_type, 0x02 | 0x1B | 0x24))
            .map_or(pmt.pcr_pid, |e| e.pid);
        tap.start(&pids)?;
        Ok(Stream {
            tap,
            hw: self.hw.clone(),
            pat: psi::build_pat(pat.tsid, channel.program, pmt_pid),
            watch,
            limits: self.limits.clone(),
            clock: self.clock.clone(),
            _lease: lease,
        })
    }
}

/// How a running stream ended (a client that went away is the `io::Error` of `pump`, not one of these).
#[derive(Debug, PartialEq, Eq)]
pub enum End {
    /// Asked to stop (the helper is going down, or the setting went off).
    Stopped,
    /// The program's picture stopped arriving: the tuner has been moved (Live TV changed channel).
    Silent,
    /// The TV took the tuner for its own use.
    Preempted(String),
    Failed(Failure),
}

/// A stream that is tuned and routed, and has not yet written a byte. Fields drop in order: the tap lets go of the TV, then the lease.
pub struct Stream {
    tap: Box<dyn Tap>,
    hw: Arc<dyn Hardware>,
    pat: Vec<u8>,
    watch: u16,
    limits: Limits,
    clock: Clock,
    _lease: Lease,
}

impl Stream {
    /// Writes the program until the client goes, `stop` is set, or the TV takes the tuner: a PAT that names only this program, ten times a
    /// second, and the packets as they arrive.
    pub fn pump(&mut self, out: &mut dyn Write, stop: &AtomicBool) -> io::Result<End> {
        let lim = &self.limits;
        let mut pat = psi::SectionPackets::default();
        let mut buf = Vec::with_capacity(PACKET * 512);
        let mut last_pat: Option<Duration> = None;
        let mut last_watch = (self.clock)();
        let mut last_owner = last_watch;
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(End::Stopped);
            }
            let now = (self.clock)();
            if last_pat.is_none_or(|t| now.saturating_sub(t) >= lim.pat_every) {
                out.write_all(&pat.packet(psi::PID_PAT, &self.pat))?;
                last_pat = Some(now);
            }
            if now.saturating_sub(last_watch) >= lim.silence {
                return Ok(End::Silent);
            }
            if now.saturating_sub(last_owner) >= lim.owner_every {
                last_owner = now;
                if let Some(why) = self.hw.in_use().or_else(|| self.tap.taken()) {
                    return Ok(End::Preempted(why));
                }
            }
            buf.clear();
            if let Err(f) = self.tap.read(&mut buf, lim.poll) {
                return Ok(End::Failed(f));
            }
            if !buf.is_empty() {
                if buf
                    .chunks_exact(PACKET)
                    .any(|p| u16::from_be_bytes([p[1] & 0x1F, p[2]]) == self.watch)
                {
                    last_watch = (self.clock)();
                }
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
    use std::sync::atomic::AtomicU64;

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
        fn section(
            &mut self,
            pid: u16,
            table_id: u8,
            number: u8,
            _wait: Duration,
        ) -> Option<Vec<u8>> {
            let m = &self.fake.mux;
            match (pid, table_id) {
                (psi::PID_PSIP, psi::TID_TVCT) => m.tvct.get(usize::from(number)).cloned(),
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

    fn facade(fake: &Arc<Fake>, time: &Time) -> Facade {
        Facade::new(
            Arc::new(Shared(fake.clone())),
            Identity {
                name: "LumaTV".into(),
                device_id: "0A1B2C3D".into(),
            },
            Limits::default(),
            time.clock(),
        )
    }

    pub(crate) fn fake_with(time: &Time, mux: Multiplex, scripted: Vec<Vec<u8>>) -> Arc<Fake> {
        Fake::new(time, mux, scripted)
    }

    #[test]
    fn a_stream_is_a_pat_naming_one_program_then_that_programs_packets() {
        let time = Time::default();
        let chunk = [
            packet(0x61, 0),
            packet(0x64, 0),
            packet(0x60, 0),
            packet(0x61, 1),
        ]
        .concat();
        let fake = fake_with(&time, rf32(&[]), vec![chunk.clone()]);
        let mut stream = facade(&fake, &time).open("2.4").unwrap();
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
        let rest: Vec<u8> = packets
            .iter()
            .filter(|p| pid(p) != 0)
            .flat_map(|p| p.iter().copied())
            .collect();
        assert_eq!(rest, chunk, "everything the tuner gave, in order");
        assert!(
            packets.iter().filter(|p| pid(p) == 0).count() > 10,
            "the PAT goes on while the stream runs"
        );
        drop(stream);
        assert_eq!(fake.released.load(Ordering::Relaxed), 1, "the TV is let go");
    }

    #[test]
    fn a_second_stream_waits_for_the_first_and_the_tuner_is_free_again_when_it_ends() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), vec![]);
        let f = facade(&fake, &time);
        let first = f.open("2.4").unwrap();
        assert_eq!(f.open("2.1").err(), Some(Failure::Busy));
        assert_eq!(
            fake.tuned.lock().len(),
            1,
            "the refused one never touched the TV"
        );
        drop(first);
        assert!(f.open("2.1").is_ok());
    }

    #[test]
    fn a_channel_that_is_not_in_the_lineup_is_not_found() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), vec![]);
        assert_eq!(
            facade(&fake, &time).open("9.9").err(),
            Some(Failure::UnknownChannel)
        );
        assert!(fake.tuned.lock().is_empty());
    }

    #[test]
    fn the_tv_in_use_is_refused_before_the_tuner_is_touched_and_frees_the_lease() {
        let time = Time::default();
        let fake = fake_with(&time, rf32(&[]), vec![]);
        *fake.owner.lock() = Some("Live TV is in front".into());
        let f = facade(&fake, &time);
        assert_eq!(
            f.open("2.4").err(),
            Some(Failure::TvInUse("Live TV is in front".into()))
        );
        assert!(fake.tuned.lock().is_empty());
        *fake.owner.lock() = None;
        assert!(f.open("2.4").is_ok(), "the refusal did not keep the tuner");
    }

    #[test]
    fn the_pipeline_unrealized_by_the_tv_ends_the_stream_though_the_packets_go_on() {
        let time = Time::default();
        let chunk = [packet(0x61, 0)].concat();
        let fake = fake_with(&time, rf32(&[]), vec![chunk]);
        let f = Facade::new(
            Arc::new(Shared(fake.clone())),
            Identity {
                name: "LumaTV".into(),
                device_id: "0A1B2C3D".into(),
            },
            Limits {
                owner_every: Duration::from_secs(1),
                ..Limits::default()
            },
            time.clock(),
        );
        let mut stream = f.open("2.4").unwrap();
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
        let chunk = [packet(0x61, 0)].concat();
        let fake = fake_with(&time, rf32(&[]), vec![chunk]);
        let f = Facade::new(
            Arc::new(Shared(fake.clone())),
            Identity {
                name: "LumaTV".into(),
                device_id: "0A1B2C3D".into(),
            },
            Limits {
                owner_every: Duration::from_secs(1),
                ..Limits::default()
            },
            time.clock(),
        );
        let mut stream = f.open("2.4").unwrap();
        *fake.owner.lock() = Some("Live TV is in front".into());
        let end = stream
            .pump(&mut Vec::new(), &AtomicBool::new(false))
            .unwrap();
        assert_eq!(end, End::Preempted("Live TV is in front".into()));
    }

    #[test]
    fn a_program_that_asks_not_to_be_passed_on_is_not_served_and_the_tv_is_let_go() {
        for tags in [&[0x09][..], &[0xAA][..]] {
            let time = Time::default();
            let fake = fake_with(&time, rf32(tags), vec![]);
            assert_eq!(
                facade(&fake, &time).open("2.4").err(),
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
        let fake = fake_with(&time, mux, vec![]);
        assert!(
            facade(&fake, &time).open("2.4").is_ok(),
            "2.4 is in the second section"
        );
    }

    #[test]
    fn a_multiplex_that_does_not_carry_the_channel_is_no_signal() {
        let time = Time::default();
        let mut mux = rf32(&[]);
        mux.tvct = vec![tvct(739, &[("WSB-HD", 2, 1, 1, false)])];
        let fake = fake_with(&time, mux, vec![]);
        let got = facade(&fake, &time).open("2.4").err();
        assert!(matches!(got, Some(Failure::NoSignal(_))), "{got:?}");
        assert_eq!(fake.released.load(Ordering::Relaxed), 1);
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
