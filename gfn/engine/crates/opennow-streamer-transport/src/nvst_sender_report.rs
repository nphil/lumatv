//! RTCP sender reports (SR, RFC 3550 section 6.4.1), recorded and never acted on.
//!
//! An SR pairs a sender's NTP clock with its RTP timestamp, which is what would place the audio and
//! video tracks on one timeline. Whether the NVST server sends any is what this module finds out:
//! it counts what arrives, per track, and keeps the first and last pair. No packet is accepted,
//! dropped or sent because of it.
//!
//! A report is seen in three ways. On the `rtcp1` data channel the compound packet is plaintext and
//! is read in full. On the bundle socket str0m decrypts the audio track's SRTCP and hands over the
//! sender info. Any other SRTCP, the video socket's included, cannot be decrypted here, but SRTCP
//! leaves the first 8 bytes of a packet unencrypted (RFC 3711 section 3.4): that names the sender
//! and shows a report arrived, without its times.

use std::fmt::Write as _;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// RTCP packet type of a sender report.
const RTCP_SR: u8 = 200;
/// An SR up to its last sender-info word: the 4-byte RTCP header, the sender SSRC and five 32-bit
/// words.
const SR_LEN: usize = 28;
/// Seconds from the NTP epoch (1900-01-01) to the Unix epoch.
const NTP_UNIX_OFFSET_SECS: u64 = 2_208_988_800;

/// The NVST video RTP clock, the one the receiver stamps on its frames.
pub(crate) const VIDEO_RTP_CLOCK_HZ: u32 = 90_000;

/// The sender-info block of a report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SenderInfo {
    /// Microseconds since 1900-01-01. Whether the server's clock is wall time is for the reader to
    /// judge.
    pub(crate) ntp_us: u64,
    /// The sender's RTP timestamp at that instant, as written (32 bits, wrapping).
    pub(crate) rtp: u32,
    pub(crate) packets: u32,
    pub(crate) octets: u32,
}

impl From<&str0m::rtp::rtcp::SenderInfo> for SenderInfo {
    fn from(info: &str0m::rtp::rtcp::SenderInfo) -> Self {
        Self {
            ntp_us: ntp_micros(info.ntp_time),
            // str0m widens the 32-bit RTP timestamp to 64 bits; the low word is the one on the
            // wire.
            rtp: info.rtp_time.numer() as u32,
            packets: info.sender_packet_count,
            octets: info.sender_octet_count,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SenderReport {
    pub(crate) ssrc: u32,
    pub(crate) info: SenderInfo,
}

/// The sender reports in a plaintext compound RTCP packet, in order. Reading stops at the first
/// sub-packet that is not version 2, is shorter than its fixed part or runs past the end; what came
/// before it is kept.
pub(crate) fn read_sender_reports(compound: &[u8]) -> SenderReports<'_> {
    SenderReports { rest: compound }
}

pub(crate) struct SenderReports<'a> {
    rest: &'a [u8],
}

impl Iterator for SenderReports<'_> {
    type Item = SenderReport;

    fn next(&mut self) -> Option<SenderReport> {
        while self.rest.len() >= 4 {
            if self.rest[0] >> 6 != 2 {
                break;
            }
            let length = packet_len(self.rest);
            if length > self.rest.len() {
                break;
            }
            let (packet, rest) = self.rest.split_at(length);
            self.rest = rest;
            if packet[1] == RTCP_SR {
                if length < SR_LEN {
                    break;
                }
                return Some(read_sender_report(packet));
            }
        }
        self.rest = &[];
        None
    }
}

/// The length in bytes that an RTCP packet's header gives (its 16-bit field counts 32-bit words,
/// less one).
fn packet_len(header: &[u8]) -> usize {
    (usize::from(u16::from_be_bytes([header[2], header[3]])) + 1) * 4
}

fn read_sender_report(packet: &[u8]) -> SenderReport {
    let word = |at: usize| {
        u32::from_be_bytes([packet[at], packet[at + 1], packet[at + 2], packet[at + 3]])
    };
    let ntp = (u64::from(word(8)) << 32) | u64::from(word(12));
    SenderReport {
        ssrc: word(4),
        info: SenderInfo {
            ntp_us: ntp_to_micros(ntp),
            rtp: word(16),
            packets: word(20),
            octets: word(24),
        },
    }
}

/// An NTP timestamp (32.32 fixed point seconds since 1900) in whole microseconds.
fn ntp_to_micros(ntp: u64) -> u64 {
    (ntp >> 32) * 1_000_000 + (((ntp & 0xffff_ffff) * 1_000_000) >> 32)
}

/// A `SystemTime` on the 1900 scale of [`SenderInfo::ntp_us`]. str0m hands over a report's NTP time
/// as a `SystemTime`, that is minus the 1900-1970 offset, rounded to about half a microsecond and
/// clamped at 1970 (a server clock that is not wall time reads as 1970).
fn ntp_micros(time: SystemTime) -> u64 {
    let since_unix = micros(time.duration_since(UNIX_EPOCH).unwrap_or_default());
    NTP_UNIX_OFFSET_SECS
        .saturating_mul(1_000_000)
        .saturating_add(since_unix)
}

/// The sender of a datagram's first RTCP packet when that packet is a sender report. Only the clear
/// header is read, so this works on SRTCP without keys; the report's times are not readable here.
pub(crate) fn peeked_sender_report_ssrc(datagram: &[u8]) -> Option<u32> {
    if datagram.len() < 8 || datagram[0] >> 6 != 2 || datagram[1] != RTCP_SR {
        return None;
    }
    (SR_LEN..=datagram.len())
        .contains(&packet_len(datagram))
        .then(|| u32::from_be_bytes([datagram[4], datagram[5], datagram[6], datagram[7]]))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Track {
    Video,
    Audio,
    /// An SSRC that is neither track's: counted, never attributed.
    Unmapped,
}

impl Track {
    const fn name(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Unmapped => "unmapped",
        }
    }

    /// The clock of the track's RTP timestamps. `audio_hz` is the session's declared audio clock.
    pub(crate) const fn clock_hz(self, audio_hz: u32) -> u32 {
        match self {
            Self::Video => VIDEO_RTP_CLOCK_HZ,
            Self::Audio => audio_hz,
            Self::Unmapped => 0,
        }
    }
}

/// The track an SSRC belongs to. `video` is the SSRC the receiver was told to expect or has bound
/// to, if any.
pub(crate) fn track_of(ssrc: u32, video: Option<u32>, is_audio: bool) -> Track {
    if is_audio {
        Track::Audio
    } else if video == Some(ssrc) {
        Track::Video
    } else {
        Track::Unmapped
    }
}

/// How a report's times were read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Via {
    /// Plaintext RTCP on the `rtcp1` data channel.
    DataChannel,
    /// SRTCP that str0m decrypted.
    Srtcp,
}

impl Via {
    const fn name(self) -> &'static str {
        match self {
            Self::DataChannel => "datachannel",
            Self::Srtcp => "srtcp",
        }
    }
}

/// One sender report, as a transport thread saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Observation {
    pub(crate) track: Track,
    pub(crate) ssrc: u32,
    pub(crate) clock_hz: u32,
    pub(crate) arrival: Instant,
    /// The sender info and how it was read; `None` when only the clear header was seen.
    pub(crate) read: Option<(SenderInfo, Via)>,
}

impl Observation {
    pub(crate) fn trace_line(&self) -> String {
        let mut line = format!(
            "track={} ssrc={} clock_hz={}",
            self.track.name(),
            self.ssrc,
            self.clock_hz
        );
        match self.read {
            Some((info, via)) => {
                let _ = write!(
                    line,
                    " via={} ntp_us={} rtp={} packets={} octets={}",
                    via.name(),
                    info.ntp_us,
                    info.rtp,
                    info.packets,
                    info.octets
                );
            }
            None => line.push_str(" via=header times=encrypted"),
        }
        line
    }
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    ssrc: u32,
    info: SenderInfo,
    arrival: Instant,
    via: Via,
}

impl Sample {
    fn to_json(self, at: &impl Fn(Instant) -> i64) -> Value {
        json!({
            "ssrc": self.ssrc,
            "ntp_us": self.info.ntp_us,
            "rtp": self.info.rtp,
            "packets": self.info.packets,
            "octets": self.info.octets,
            "arrival_ns": at(self.arrival),
            "via": self.via.name(),
        })
    }
}

/// How often one kind of sighting arrived.
#[derive(Debug, Default)]
struct Cadence {
    count: u32,
    first_at: Option<Instant>,
    last_at: Option<Instant>,
    max_gap: Duration,
}

impl Cadence {
    fn record(&mut self, at: Instant) {
        self.count = self.count.saturating_add(1);
        if let Some(last) = self.last_at {
            self.max_gap = self.max_gap.max(at.saturating_duration_since(last));
        }
        self.first_at.get_or_insert(at);
        self.last_at = Some(at);
    }

    /// The mean gap between sightings; none before a second one.
    fn every(&self) -> Option<Duration> {
        let span = self.last_at?.saturating_duration_since(self.first_at?);
        (self.count > 1).then(|| span / (self.count - 1))
    }

    fn to_json(&self, at: &impl Fn(Instant) -> i64) -> Value {
        json!({
            "count": self.count,
            "every_us": self.every().map(micros),
            "max_gap_us": (self.count > 1).then(|| micros(self.max_gap)),
            "first_ns": self.first_at.map(at),
            "last_ns": self.last_at.map(at),
        })
    }
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

#[derive(Debug, Default)]
struct TrackReports {
    clock_hz: u32,
    /// The sender of the latest report, read or not.
    ssrc: Option<u32>,
    /// Reports whose clear header was seen on a socket.
    wire: Cadence,
    /// Reports whose times were read.
    read: Cadence,
    first: Option<Sample>,
    last: Option<Sample>,
}

impl TrackReports {
    fn record(&mut self, observation: &Observation) {
        if observation.clock_hz != 0 {
            self.clock_hz = observation.clock_hz;
        }
        self.ssrc = Some(observation.ssrc);
        match observation.read {
            None => self.wire.record(observation.arrival),
            Some((info, via)) => {
                self.read.record(observation.arrival);
                let sample = Sample {
                    ssrc: observation.ssrc,
                    info,
                    arrival: observation.arrival,
                    via,
                };
                self.first.get_or_insert(sample);
                self.last = Some(sample);
            }
        }
    }

    fn heard(&self) -> bool {
        self.wire.count > 0 || self.read.count > 0
    }

    fn to_json(&self, at: &impl Fn(Instant) -> i64) -> Value {
        let mut read = self.read.to_json(at);
        read["via"] = json!(self.last.map(|sample| sample.via.name()));
        read["first"] = json!(self.first.map(|sample| sample.to_json(at)));
        read["last"] = json!(self.last.map(|sample| sample.to_json(at)));
        json!({
            "clock_hz": (self.clock_hz != 0).then_some(self.clock_hz),
            "ssrc": self.ssrc,
            "wire": self.wire.to_json(at),
            "read": read,
        })
    }

    fn line(&self, name: &str) -> String {
        let mut line = format!("{name}: heard={} read={}", self.wire.count, self.read.count);
        if let Some(every) = self.read.every().or_else(|| self.wire.every()) {
            let _ = write!(line, " every_ms={}", every.as_millis());
        }
        match self.last {
            Some(last) => {
                let _ = write!(
                    line,
                    " ssrc={} ntp_us={} rtp={}",
                    last.ssrc, last.info.ntp_us, last.info.rtp
                );
            }
            None => {
                if let Some(ssrc) = self.ssrc {
                    let _ = write!(line, " ssrc={ssrc}");
                }
            }
        }
        line
    }
}

/// Everything seen for one session, in a fixed amount of memory.
#[derive(Debug, Default)]
pub(crate) struct SenderReportMonitor {
    video: TrackReports,
    audio: TrackReports,
    unmapped: TrackReports,
}

impl SenderReportMonitor {
    pub(crate) fn record(&mut self, observation: Observation) {
        match observation.track {
            Track::Video => &mut self.video,
            Track::Audio => &mut self.audio,
            Track::Unmapped => &mut self.unmapped,
        }
        .record(&observation);
    }

    /// Both tracks have reports whose times were read: the condition for anchoring audio to video.
    fn both_read(&self) -> bool {
        self.video.read.count > 0 && self.audio.read.count > 0
    }

    /// The summary for the timing capture. Arrivals are converted to the caller's clock: `now_ns`
    /// is that clock's reading at `now`.
    pub(crate) fn to_json(&self, now: Instant, now_ns: i64) -> Value {
        let at = |when: Instant| {
            now_ns.saturating_sub(
                i64::try_from(now.saturating_duration_since(when).as_nanos()).unwrap_or(i64::MAX),
            )
        };
        json!({
            "video": self.video.to_json(&at),
            "audio": self.audio.to_json(&at),
            "unmapped": self.unmapped.to_json(&at),
            "coverage": {
                "both_heard": self.video.heard() && self.audio.heard(),
                "both_read": self.both_read(),
            },
            "definitions": {
                "scope": "every report since the transport started, up to the write; not limited \
                    to the capture window",
                "wire": "reports whose clear RTCP header was seen on the bundle or video socket; \
                    their times are encrypted",
                "read": "reports whose NTP and RTP times were read: plaintext on the rtcp1 data \
                    channel, or decrypted by str0m",
                "arrival_ns": "when the report reached the engine, on the capture's \
                    CLOCK_MONOTONIC (also first_ns and last_ns)",
                "every_us": "mean gap between sightings of that kind; max_gap_us is the longest",
                "ntp_us": "microseconds since 1900-01-01; str0m's reading is clamped at 1970",
                "rtp": "the sender's 32-bit RTP timestamp, as written",
                "clock_hz": "video: the fixed NVST 90 kHz; audio: audioTrack.clockRateHz from the \
                    session handoff; the NVST DESCRIBE SDP has no rtpmap",
                "unmapped": "SSRCs that are neither track's, counted and never attributed",
                "coverage": "both_heard: audio and video each had a report in either form; \
                    both_read: each had one whose times were read, the condition for placing \
                    audio against video",
            },
        })
    }

    pub(crate) fn log_line(&self) -> String {
        format!(
            "{} | {} | {} | both_read={}",
            self.video.line("video"),
            self.audio.line("audio"),
            self.unmapped.line("unmapped"),
            self.both_read()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sender_report(
        ssrc: u32,
        ntp: u64,
        rtp: u32,
        packets: u32,
        octets: u32,
        blocks: u8,
    ) -> Vec<u8> {
        let words = 6 + 6 * u16::from(blocks);
        let mut packet = vec![0x80 | blocks, RTCP_SR];
        packet.extend_from_slice(&words.to_be_bytes());
        packet.extend_from_slice(&ssrc.to_be_bytes());
        packet.extend_from_slice(&ntp.to_be_bytes());
        for word in [rtp, packets, octets] {
            packet.extend_from_slice(&word.to_be_bytes());
        }
        packet.resize(packet.len() + 24 * usize::from(blocks), 0);
        packet
    }

    fn receiver_report(ssrc: u32) -> Vec<u8> {
        let mut packet = vec![0x80, 201, 0, 1];
        packet.extend_from_slice(&ssrc.to_be_bytes());
        packet
    }

    fn source_description(ssrc: u32) -> Vec<u8> {
        let mut packet = vec![0x81, 202, 0, 2];
        packet.extend_from_slice(&ssrc.to_be_bytes());
        packet.extend_from_slice(&[1, 1, b'a', 0]);
        packet
    }

    fn info(ntp_us: u64, rtp: u32) -> SenderInfo {
        SenderInfo {
            ntp_us,
            rtp,
            packets: 1,
            octets: 2,
        }
    }

    fn observation(track: Track, arrival: Instant, read: Option<(SenderInfo, Via)>) -> Observation {
        Observation {
            track,
            ssrc: 7,
            clock_hz: track.clock_hz(48_000),
            arrival,
            read,
        }
    }

    #[test]
    fn reads_the_sender_info_of_a_report() {
        let ntp = (3_900_000_000_u64 << 32) | 0x8000_0000;
        let reports: Vec<_> =
            read_sender_reports(&sender_report(0xaabb_ccdd, ntp, 123_456, 77, 9_999, 1)).collect();
        let ntp_us = 3_900_000_000 * 1_000_000 + 500_000;
        assert_eq!(
            reports,
            [SenderReport {
                ssrc: 0xaabb_ccdd,
                info: SenderInfo {
                    ntp_us,
                    rtp: 123_456,
                    packets: 77,
                    octets: 9_999
                }
            }]
        );
    }

    #[test]
    fn finds_every_report_in_a_compound_packet() {
        let mut compound = receiver_report(1);
        compound.extend(source_description(1));
        compound.extend(sender_report(10, 1 << 32, 1_000, 5, 50, 0));
        compound.extend(sender_report(20, 2 << 32, 2_000, 6, 60, 2));
        compound.extend([0x81, 203, 0, 1, 0, 0, 0, 30]);
        let found: Vec<_> = read_sender_reports(&compound)
            .map(|report| (report.ssrc, report.info.rtp, report.info.ntp_us))
            .collect();
        assert_eq!(found, [(10, 1_000, 1_000_000), (20, 2_000, 2_000_000)]);
    }

    #[test]
    fn stops_at_a_truncated_or_malformed_packet() {
        let whole = sender_report(10, 0, 1, 2, 3, 0);
        let first = |bytes: &[u8]| read_sender_reports(bytes).next();
        assert_eq!(
            first(&whole[..whole.len() - 1]),
            None,
            "the length field runs past the end"
        );
        assert_eq!(first(&whole[..3]), None);
        assert_eq!(first(&[]), None);
        let mut version_one = whole.clone();
        version_one[0] = 0x40;
        assert_eq!(first(&version_one), None);
        assert_eq!(
            first(&[0x80, RTCP_SR, 0, 1, 0, 0, 0, 9]),
            None,
            "an SR too short for its sender info"
        );

        let mut tail = whole.clone();
        tail.extend(&whole[..10]);
        assert_eq!(
            read_sender_reports(&tail).count(),
            1,
            "what precedes the truncated packet is kept"
        );
        let mut behind_garbage = receiver_report(1);
        behind_garbage.extend([0x40, 0, 0, 0]);
        behind_garbage.extend(&whole);
        assert_eq!(
            read_sender_reports(&behind_garbage).count(),
            0,
            "nothing after a packet that is not RTCP is trusted"
        );
    }

    #[test]
    fn an_unknown_ssrc_is_unmapped_and_never_counted_for_a_track() {
        assert_eq!(track_of(5, Some(5), false), Track::Video);
        assert_eq!(track_of(6, Some(5), false), Track::Unmapped);
        assert_eq!(
            track_of(6, None, false),
            Track::Unmapped,
            "video not bound yet"
        );
        assert_eq!(track_of(6, Some(5), true), Track::Audio);
        assert_eq!(
            track_of(5, Some(5), true),
            Track::Audio,
            "the audio track wins an SSRC both claim"
        );
        assert_eq!(Track::Unmapped.clock_hz(48_000), 0);

        let now = Instant::now();
        let mut monitor = SenderReportMonitor::default();
        monitor.record(observation(
            Track::Unmapped,
            now,
            Some((info(1, 2), Via::DataChannel)),
        ));
        monitor.record(observation(Track::Unmapped, now, None));
        let json = monitor.to_json(now, 1_000);
        assert_eq!(
            (
                json["unmapped"]["read"]["count"].as_u64(),
                json["unmapped"]["wire"]["count"].as_u64()
            ),
            (Some(1), Some(1))
        );
        assert_eq!(
            (
                json["video"]["wire"]["count"].as_u64(),
                json["audio"]["read"]["count"].as_u64()
            ),
            (Some(0), Some(0))
        );
        assert_eq!(json["unmapped"]["clock_hz"], Value::Null);
    }

    #[test]
    fn a_clear_header_names_the_sender_of_an_encrypted_report() {
        let mut srtcp = sender_report(0x0102_0304, 0, 0, 0, 0, 0);
        srtcp[8..].fill(0xa5);
        srtcp.extend([0x80, 0, 0, 1, 0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(
            peeked_sender_report_ssrc(&srtcp),
            Some(0x0102_0304),
            "index and tag after the packet are fine"
        );
        assert_eq!(
            peeked_sender_report_ssrc(&srtcp[..27]),
            None,
            "shorter than its own length field"
        );
        assert_eq!(peeked_sender_report_ssrc(&receiver_report(9)), None);
        assert_eq!(
            peeked_sender_report_ssrc(&[0x80, RTCP_SR, 0, 6, 0, 0, 0]),
            None
        );
        let mut other_version = srtcp.clone();
        other_version[0] = 0x40;
        assert_eq!(peeked_sender_report_ssrc(&other_version), None);
        assert_eq!(
            peeked_sender_report_ssrc(&[0x80, RTCP_SR, 0, 1, 0, 0, 0, 1]),
            None,
            "too short to be a report"
        );
    }

    #[test]
    fn counts_cadence_keeps_the_first_and_last_pair_and_reports_coverage() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut monitor = SenderReportMonitor::default();
        for (ms, rtp) in [(0, 100), (1_000, 48_100), (3_000, 144_100)] {
            monitor.record(observation(
                Track::Audio,
                at(ms),
                Some((info(5_000 + ms, rtp), Via::Srtcp)),
            ));
        }
        monitor.record(observation(Track::Video, at(2_000), None));

        let json = monitor.to_json(at(4_000), 9_000_000_000);
        let audio = &json["audio"];
        assert_eq!(
            (audio["clock_hz"].as_u64(), audio["ssrc"].as_u64()),
            (Some(48_000), Some(7))
        );
        assert_eq!(audio["read"]["count"], 3);
        assert_eq!(
            (
                audio["read"]["every_us"].as_u64(),
                audio["read"]["max_gap_us"].as_u64()
            ),
            (Some(1_500_000), Some(2_000_000))
        );
        assert_eq!(
            (
                audio["read"]["first"]["rtp"].as_u64(),
                audio["read"]["last"]["rtp"].as_u64()
            ),
            (Some(100), Some(144_100))
        );
        assert_eq!(
            audio["read"]["first"]["arrival_ns"], 5_000_000_000_i64,
            "arrival is on the caller's clock"
        );
        assert_eq!(audio["read"]["last"]["arrival_ns"], 8_000_000_000_i64);
        assert_eq!(audio["read"]["via"], "srtcp");
        assert_eq!(
            (
                audio["wire"]["count"].as_u64(),
                audio["wire"]["first_ns"].clone()
            ),
            (Some(0), Value::Null)
        );

        let video = &json["video"];
        assert_eq!(
            (
                video["clock_hz"].as_u64(),
                video["wire"]["count"].as_u64(),
                video["read"]["count"].as_u64()
            ),
            (Some(90_000), Some(1), Some(0))
        );
        assert_eq!(
            (
                video["read"]["first"].clone(),
                video["wire"]["every_us"].clone()
            ),
            (Value::Null, Value::Null)
        );
        assert_eq!(
            (
                json["coverage"]["both_heard"].clone(),
                json["coverage"]["both_read"].clone()
            ),
            (json!(true), json!(false))
        );

        monitor.record(observation(
            Track::Video,
            at(3_500),
            Some((info(8_500, 315_000), Via::DataChannel)),
        ));
        assert_eq!(monitor.to_json(at(4_000), 0)["coverage"]["both_read"], true);
        assert!(
            monitor
                .log_line()
                .contains("audio: heard=0 read=3 every_ms=1500 ssrc=7 ntp_us=8000 rtp=144100"),
            "{}",
            monitor.log_line()
        );
        assert!(monitor.log_line().ends_with("both_read=true"));
    }

    #[test]
    fn nothing_seen_is_a_recorded_result() {
        let json = SenderReportMonitor::default().to_json(Instant::now(), 5);
        assert_eq!(
            (
                json["video"]["wire"]["count"].as_u64(),
                json["audio"]["read"]["count"].as_u64()
            ),
            (Some(0), Some(0))
        );
        assert_eq!(
            (
                json["coverage"]["both_heard"].clone(),
                json["coverage"]["both_read"].clone()
            ),
            (json!(false), json!(false))
        );
        assert_eq!(
            SenderReportMonitor::default().log_line(),
            "video: heard=0 read=0 | audio: heard=0 read=0 | unmapped: heard=0 read=0 \
                | both_read=false"
        );
    }

    #[test]
    fn ntp_is_microseconds_since_1900() {
        assert_eq!(ntp_to_micros(1 << 32), 1_000_000);
        assert_eq!(ntp_to_micros(0x8000_0000), 500_000);
        assert_eq!(
            ntp_micros(UNIX_EPOCH + Duration::from_millis(1_500)),
            2_208_988_801_500_000
        );
        assert_eq!(
            ntp_micros(UNIX_EPOCH - Duration::from_secs(1)),
            2_208_988_800_000_000,
            "before 1970 reads as 1970"
        );
    }

    #[test]
    fn str0m_sender_info_keeps_the_wire_rtp_word_and_the_counts() {
        use str0m::media::{Frequency, MediaTime};
        let info = str0m::rtp::rtcp::SenderInfo {
            ssrc: 7.into(),
            ntp_time: UNIX_EPOCH + Duration::from_millis(1_500),
            rtp_time: MediaTime::new(0x1_0000_0005, Frequency::FORTY_EIGHT_KHZ),
            sender_packet_count: 11,
            sender_octet_count: 22,
        };
        assert_eq!(
            SenderInfo::from(&info),
            SenderInfo {
                ntp_us: 2_208_988_801_500_000,
                rtp: 5,
                packets: 11,
                octets: 22
            }
        );
    }

    #[test]
    fn a_trace_line_says_how_the_times_were_read() {
        let now = Instant::now();
        assert_eq!(
            observation(
                Track::Audio,
                now,
                Some((
                    SenderInfo {
                        ntp_us: 9,
                        rtp: 8,
                        packets: 7,
                        octets: 6
                    },
                    Via::Srtcp
                ))
            )
            .trace_line(),
            "track=audio ssrc=7 clock_hz=48000 via=srtcp ntp_us=9 rtp=8 packets=7 octets=6"
        );
        assert_eq!(
            observation(Track::Video, now, None).trace_line(),
            "track=video ssrc=7 clock_hz=90000 via=header times=encrypted"
        );
    }
}
