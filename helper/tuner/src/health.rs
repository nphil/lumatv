//! What the tuner knows of each channel: how it came in the last time it was streamed or checked ([`Health`]), so the owner sees which
//! channels the antenna brings in badly and can leave them out of the lineup; and what its picture and program were like, so the slate
//! before its next stream is already that shape.
//!
//! A check of every channel ([`crate::Tuner::check_reception`]) tunes each RF channel of the lineup once, routes the pictures of all its
//! virtual channels together and listens for a few seconds: the demodulator's reading, whether each picture came whole, and the share
//! of its packets lost. Any stream asked for meanwhile takes the tuner, and the TV's own use stops the check.

use crate::docs::{self, Entry};
use crate::mpeg2::Format;
use crate::psi::{self, PACKET};
use crate::splice::Layout;
use crate::stream::{Facade, Failure, Signal, channels_of};
use crate::Status;
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

/// How a channel comes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The picture comes whole.
    Good,
    /// The picture comes, with packets lost now and then (blocks, a skip).
    Glitchy,
    /// The picture breaks up or stops: a tenth of its packets lost, or a gap the slate stood in for.
    Weak,
    /// No picture came.
    NoSignal,
}

impl Verdict {
    /// The verdict of a picture that came (or not), lost `lost` of its packets and stopped `dropouts` times. A twentieth lost is a picture
    /// that breaks up every few seconds (16.1 on 2026-11-05 lost 3% and showed blocks; a second that lost a quarter was its last).
    pub fn of(pictured: bool, lost: f32, dropouts: u32) -> Self {
        match () {
            _ if !pictured => Self::NoSignal,
            _ if dropouts > 0 || lost >= 0.05 => Self::Weak,
            _ if lost >= 0.002 => Self::Glitchy,
            _ => Self::Good,
        }
    }

    pub fn word(&self) -> &'static str {
        match self {
            Self::Good => "Good",
            Self::Glitchy => "Glitchy",
            Self::Weak => "Weak",
            Self::NoSignal => "No signal",
        }
    }
}

/// How a channel came in the last time it was streamed or checked.
#[derive(Clone, Debug, PartialEq)]
pub struct Health {
    /// When.
    pub at: SystemTime,
    pub verdict: Verdict,
    /// The weakest the demodulator read meanwhile, when it says.
    pub signal: Option<Signal>,
    /// The share of the picture's packets lost on the way.
    pub lost: f32,
    /// How long the picture took to come: from the request for a stream, from the tune for a check.
    pub picture: Option<Duration>,
    /// How many times the picture stopped and a slate stood in.
    pub dropouts: u32,
    /// Found by a check of every channel, not by a stream.
    pub checked: bool,
}

/// What a channel's picture and program were like: the slate before its next stream is that shape and names those streams.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Seen {
    pub(crate) format: Format,
    pub(crate) layout: Layout,
    pub(crate) tsid: u16,
}

/// What the tuner remembers of a channel.
#[derive(Clone, Debug, Default)]
pub(crate) struct Known {
    pub(crate) seen: Option<Seen>,
    pub(crate) health: Option<Health>,
    /// The PIDs its last stream routed: the next stream routes them as soon as the tuner locks, ahead of its tables.
    pub(crate) pids: Option<Vec<u16>>,
}

/// A channel as the owner sees it: the lineup's channel, whether it is left out, and how it came in.
#[derive(Clone, Debug, PartialEq)]
pub struct Reception {
    pub guide: String,
    pub name: String,
    /// The RF channel it is broadcast on (what it shares its signal with).
    pub rf: u8,
    pub hidden: bool,
    pub health: Option<Health>,
    /// Its picture's format and how many sounds its program has, once seen, and its picture's stream type (0x02 MPEG-2, 0x1B H.264):
    /// what a host keeps to give back with [`crate::Tuner::recall`]. The format is an MPEG-2 picture's only.
    pub format: Option<Format>,
    pub video: Option<u8>,
    pub sounds: usize,
}

/// The picture packets of one channel while a check listens: its stream type, how many came and how many were lost, by continuity count,
/// and when the first whole picture came (for MPEG-2, with the shape its sequence header gives).
struct Listened {
    stream_type: u8,
    got: u32,
    lost: u32,
    cc: Option<u8>,
    whole: Option<Duration>,
    format: Option<Format>,
}

/// The packets lost before `cc` on a PID whose last packet with a payload counted `last`.
pub(crate) fn lost_before(last: u8, cc: u8) -> u32 {
    u32::from(cc.wrapping_sub(last).wrapping_sub(1) & 0x0F)
}

impl Facade {
    /// Checks every channel of the lineup, one RF channel at a time (the status says how far it is). Gives up when a stream is asked for or
    /// the TV uses its tuner, keeping what it found so far. Returns how many channels it found something for.
    pub(crate) fn check_all(&self) -> Result<usize, Failure> {
        let entries = self.entries()?;
        let mut by_rf: BTreeMap<u8, Vec<Entry>> = BTreeMap::new();
        for e in entries {
            if let Some((rf, _, _)) = docs::parse_channel_id(&e.channel_id) {
                by_rf.entry(rf).or_default().push(e);
            }
        }
        // the check is nobody's stream: any request takes the tuner from it, and waits for it to let go
        let lease = self.tuning.take(IpAddr::V4(Ipv4Addr::UNSPECIFIED), self.limits.release)?;
        lease.over();
        let total = by_rf.len();
        let mut found = 0;
        for (done, group) in by_rf.values().enumerate() {
            if lease.replaced.load(Ordering::Acquire) || self.tuning.status() == Status::Off {
                break;
            }
            if let Some(why) = self.hw.in_use() {
                self.tuning.refused(&why);
                return Err(Failure::TvInUse(why));
            }
            self.tuning.change(|now| (*now != Status::Off).then_some(Status::Checking { done, of: total }));
            found += self.check_group(group);
        }
        self.tuning.change(|now| matches!(now, Status::Checking { .. }).then_some(Status::Idle));
        Ok(found)
    }

    /// Tunes the first channel of one RF channel, routes the pictures of all of them, listens, and records each channel's health.
    fn check_group(&self, group: &[Entry]) -> usize {
        let started = (self.clock)();
        let record = |e: &Entry, listened: Option<&Listened>, signal: Option<Signal>| {
            let (got, lost, whole) = listened.map_or((0, 0, None), |l| (l.got, l.lost, l.whole));
            let share = if got + lost == 0 { 0.0 } else { lost as f32 / (got + lost) as f32 };
            self.tuning.record(
                &e.guide,
                Health {
                    at: SystemTime::now(),
                    verdict: Verdict::of(whole.is_some(), share, 0),
                    signal,
                    lost: share,
                    picture: whole,
                    dropouts: 0,
                    checked: true,
                },
            );
        };
        let mut tap = match self.hw.tune(&group[0].channel_id) {
            Ok(tap) => tap,
            Err(_) => {
                let signal = self.hw.signal();
                group.iter().for_each(|e| record(e, None, signal));
                return group.len();
            }
        };
        let wait = self.limits.section_wait;
        let mut sections = Vec::new();
        let tables = channels_of(tap.as_mut(), &mut sections, wait).zip(tap.section(psi::PID_PAT, psi::TID_PAT, 0, wait).and_then(|s| psi::parse_pat(&s)));
        // each channel's picture PID, with what was heard of it, and its program's layout
        let mut pictures: BTreeMap<u16, (usize, Listened)> = BTreeMap::new();
        let mut layouts: Vec<Option<(Layout, u16)>> = vec![None; group.len()];
        if let Some((channels, pat)) = &tables {
            for (i, e) in group.iter().enumerate() {
                let pmt = channels
                    .iter()
                    .find(|c| (c.major, c.minor) == (e.major, e.minor))
                    .and_then(|c| pat.programs.iter().find(|p| p.0 == c.program))
                    .and_then(|p| tap.section(p.1, psi::TID_PMT, 0, wait))
                    .and_then(|s| psi::parse_pmt(&s));
                if let Some((layout, routes)) = pmt.as_ref().and_then(Layout::of) {
                    let listened = Listened { stream_type: layout.video_type, got: 0, lost: 0, cc: None, whole: None, format: None };
                    pictures.insert(routes.video, (i, listened));
                    layouts[i] = Some((layout, pat.tsid));
                }
            }
        }
        let pids: Vec<u16> = pictures.keys().copied().collect();
        if !pids.is_empty() && tap.start(&pids).is_ok() {
            let routed = (self.clock)();
            let mut buf = Vec::with_capacity(PACKET * 512);
            while (self.clock)().saturating_sub(routed) < self.limits.listen {
                buf.clear();
                if tap.read(&mut buf, self.limits.poll).is_err() {
                    break;
                }
                let now = (self.clock)();
                for p in buf.chunks_exact(PACKET) {
                    let Some((_, l)) = pictures.get_mut(&psi::pid_of_packet(p)) else {
                        continue;
                    };
                    if p[3] & 0x10 == 0 {
                        continue;
                    }
                    let cc = p[3] & 0x0F;
                    l.lost += l.cc.map_or(0, |last| lost_before(last, cc));
                    l.cc = Some(cc);
                    l.got += 1;
                    if l.whole.is_none() && psi::access_point(p, l.stream_type) {
                        l.whole = Some(now.saturating_sub(started));
                        if l.stream_type == 0x02 {
                            l.format = Format::of_sequence(&p[4..]);
                        }
                    }
                }
            }
        }
        let signal = self.hw.signal();
        drop(tap);
        for (i, e) in group.iter().enumerate() {
            let listened = pictures.values().find(|(at, _)| *at == i).map(|(_, l)| l);
            // what the channel's next stream is like: an MPEG-2 picture's shape for its slate; another picture streams without slates
            let format = listened.filter(|l| l.whole.is_some()).and_then(|l| if l.stream_type == 0x02 { l.format } else { Some(Format::FALLBACK) });
            if let (Some(format), Some((layout, tsid))) = (format, layouts[i].take()) {
                self.tuning.saw(&e.guide, Seen { format, layout, tsid });
            }
            record(e, listened, signal);
        }
        group.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::Identity;
    use crate::slated::tests::{Script, Tv, avc, h264_mux, pes};
    use crate::stream::Limits;
    use crate::stream::tests::{Time, served};
    use std::sync::Arc;

    #[test]
    fn a_check_hears_an_h264_picture_as_it_hears_an_mpeg2_one() {
        let time = Time::default();
        let (mut a, mut b) = (0u8, 0u8);
        let mpeg2 = [0, 0, 1, 0xB3, 0x2D, 0x01, 0xE0, 0x34, 0xFF, 0xFF, 0xE0, 0x18];
        let chunks = vec![(400, [pes(0x31, &mut a, 1_003_003, Some(1_000_000), &mpeg2), pes(0x61, &mut b, 1_003_003, Some(1_000_000), &avc(true, b""))].concat())];
        let script = Script::with_mux(&time, h264_mux(), chunks);
        let identity = Identity { name: "LumaTV".into(), device_id: "0A1B2C3D".into() };
        let f = Facade::new(Arc::new(Tv(script)), identity, Limits::default(), time.clock(), served(), None);
        assert_eq!(f.check_all(), Ok(2));
        let known = f.tuning.known();
        let verdict = |g: &str| known[g].health.as_ref().map(|h| h.verdict);
        assert_eq!((verdict("2.1"), verdict("2.4")), (Some(Verdict::Good), Some(Verdict::Good)));
        let kind = |g: &str| known[g].seen.as_ref().map(|s| s.layout.video_type);
        assert_eq!((kind("2.1"), kind("2.4")), (Some(0x02), Some(0x1B)), "2.4's next stream goes without slates");
    }
}
