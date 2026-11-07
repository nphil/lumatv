//! The program the tuner serves when it shows slates: one program on PIDs of its own and one clock, into which the slate (a picture the
//! tuner makes) and the station's programme take turns, so a player that started on one goes on with the other.
//!
//! What Plex's transcoder tolerates decided the shape (`docs/research/tuner-slates.md`): it keeps the streams it found when it started and
//! drops any that appear later, so the slate and the programme share PIDs; and its grab copies timestamps, so the programme's are moved onto
//! the slate's clock at every splice instead of jumping. The station's continuity counters go on from the slate's, so a packet lost on
//! the air still shows as lost downstream; a packet left out here does not.

use crate::mpeg2::Format;
use crate::psi::{self, PACKET};

/// The program's PMT.
pub const PMT: u16 = 0x100;
/// The picture, which carries the clock.
pub const VIDEO: u16 = 0x101;
/// The first sound; the others follow it.
pub const AUDIO: u16 = 0x102;
/// A station's clock that comes on a PID of its own.
pub const CLOCK: u16 = 0x10F;
/// The sounds carried at most (ATSC stations send one to three).
pub const MAX_AUDIO: usize = 8;

const WRAP: u64 = 1 << 33;
/// How far a slate frame is stamped ahead of its clock: as far as the stations measured on 2026-11-05 (0.46 to 1.0 s), so the clock
/// does not step back when the programme comes in.
const SLATE_DELAY: u64 = 90_000;
/// An AC-3 frame, 1536 samples at 48 kHz, in 90 kHz ticks.
const AUDIO_FRAME: u64 = 2880;
/// How far a slate's silence keeps behind its picture: further than a station's first sound after a whole picture starts before that
/// picture (its PES hold 0.15 to 0.3 s of sound, measured on 2026-11-05), so that sound never starts before the silence has ended; what
/// is left between them is filled with silence when it comes.
const AUDIO_LAG: u64 = 45_000;
/// One AC-3 frame of stereo silence at 48 kHz and 32 kb/s, the same bytes every time (made by `ffmpeg -f lavfi -i
/// anullsrc=r=48000:cl=stereo -c:a ac3 -b:a 32k -f ac3`).
pub const SILENCE: [u8; 128] = [
    0x0b, 0x77, 0x3f, 0x11, 0x00, 0x40, 0x43, 0xe1, 0x06, 0xf0, 0x01, 0x08, 0x08, 0x08, 0x21, 0x01, 0x01, 0x04, 0x57, 0x1f, 0x3e,
    0x7c, 0xf9, 0xf3, 0xe7, 0xcf, 0x9f, 0x3e, 0x7c, 0xfb, 0xfe, 0x75, 0x7c, 0xf9, 0xf3, 0xe7, 0xcf, 0x9f, 0x3e, 0x7c, 0xf8, 0xff,
    0x9d, 0x5f, 0x3e, 0x7c, 0xf9, 0xf3, 0xe7, 0xcf, 0x9f, 0x3e, 0x32, 0xfe, 0xde, 0x4c, 0x99, 0x20, 0x00, 0x00, 0xf1, 0xe3, 0x5a,
    0xd6, 0x80, 0x03, 0x5a, 0xc3, 0x00, 0x00, 0x00, 0x07, 0x8f, 0x1a, 0xd6, 0xb4, 0x00, 0x1a, 0xd6, 0x18, 0x00, 0x00, 0x00, 0x3c,
    0x78, 0xd6, 0xb5, 0xa0, 0x00, 0xd6, 0xb0, 0xc0, 0x00, 0x00, 0x01, 0xe3, 0xc6, 0xb5, 0xad, 0x00, 0x06, 0xb5, 0x86, 0x00, 0x00,
    0x00, 0x0f, 0x1e, 0x35, 0xad, 0x68, 0x00, 0x35, 0xac, 0x30, 0x00, 0x00, 0x00, 0x78, 0xf1, 0xad, 0x6b, 0x40, 0x01, 0xad, 0x60,
    0xe4, 0x8e,
];

/// `a - b` on the 33-bit clock: the short way round.
fn diff(a: u64, b: u64) -> i64 {
    let d = (a.wrapping_sub(b)) & (WRAP - 1);
    if d >= WRAP / 2 { d as i64 - WRAP as i64 } else { d as i64 }
}

fn add(a: u64, b: u64) -> u64 {
    (a + b) & (WRAP - 1)
}

/// What the program carries: its number, its picture, its sounds in order, each with its type and descriptors, and the program's own
/// descriptors. A channel's layout is remembered, so the slate before its next stream already names every sound.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub program: u16,
    pub video_type: u8,
    pub video_descriptors: Vec<u8>,
    pub audio: Vec<(u8, Vec<u8>)>,
    pub descriptors: Vec<u8>,
    /// The station sends its clock on a PID of its own.
    pub clock_apart: bool,
}

impl Layout {
    /// A channel not seen yet: an MPEG-2 picture and one AC-3 sound.
    pub fn fallback(program: u16) -> Self {
        Self {
            program,
            video_type: 0x02,
            video_descriptors: Vec::new(),
            audio: vec![(0x81, Vec::new())],
            descriptors: Vec::new(),
            clock_apart: false,
        }
    }

    /// The layout of a station's PMT, and where its streams are: `None` when it has no picture (a slate is a picture).
    pub fn of(pmt: &psi::Pmt) -> Option<(Self, Routes)> {
        let video = pmt.streams.iter().find(|e| matches!(e.stream_type, 0x02 | 0x1B | 0x24))?;
        let audio: Vec<&psi::Es> = pmt
            .streams
            .iter()
            .filter(|e| matches!(e.stream_type, 0x03 | 0x04 | 0x0F | 0x11 | 0x81 | 0x87))
            .take(MAX_AUDIO)
            .collect();
        let clock_apart = pmt.pcr_pid != video.pid && audio.iter().all(|a| a.pid != pmt.pcr_pid);
        Some((
            Self {
                program: pmt.program,
                video_type: video.stream_type,
                video_descriptors: video.descriptors.clone(),
                audio: audio.iter().map(|a| (a.stream_type, a.descriptors.clone())).collect(),
                // a conditional access descriptor does not come along: a protected program is not served
                descriptors: strip(&pmt.descriptors, 0x09),
                clock_apart,
            },
            Routes {
                video: video.pid,
                audio: audio.iter().map(|a| a.pid).collect(),
                clock: pmt.pcr_pid,
            },
        ))
    }

    fn pmt(&self, version: u8) -> Vec<u8> {
        let pids: Vec<u16> = (0..self.audio.len()).map(|i| AUDIO + i as u16).collect();
        let mut streams: Vec<(u8, u16, &[u8])> = vec![(self.video_type, VIDEO, &self.video_descriptors)];
        streams.extend(self.audio.iter().zip(&pids).map(|((t, d), pid)| (*t, *pid, d.as_slice())));
        psi::build_pmt(self.program, version, if self.clock_apart { CLOCK } else { VIDEO }, &self.descriptors, &streams)
    }
}

/// A descriptor loop without the descriptors tagged `tag`.
fn strip(mut d: &[u8], tag: u8) -> Vec<u8> {
    let mut out = Vec::new();
    while d.len() >= 2 {
        let n = 2 + usize::from(d[1]);
        let one = &d[..n.min(d.len())];
        if one[0] != tag {
            out.extend_from_slice(one);
        }
        d = &d[n.min(d.len())..];
    }
    out
}

/// Where a station's streams are: the PIDs the program's picture, sounds and clock come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Routes {
    pub video: u16,
    pub audio: Vec<u16>,
    pub clock: u16,
}

impl Routes {
    /// The program's PID for a station's PID, if it is the program's.
    fn map(&self, pid: u16) -> Option<u16> {
        if pid == self.video {
            return Some(VIDEO);
        }
        if let Some(i) = self.audio.iter().position(|&a| a == pid) {
            return Some(AUDIO + i as u16);
        }
        (pid == self.clock).then_some(CLOCK)
    }

    /// The PIDs to route from the demux.
    pub fn pids(&self) -> Vec<u16> {
        let mut p: Vec<u16> = [self.video, self.clock].into_iter().chain(self.audio.iter().copied()).collect();
        p.sort_unstable();
        p.dedup();
        p
    }
}

/// The PES head of a packet that starts one: where its PTS and DTS are, and their values.
pub(crate) struct Head {
    pub(crate) pts: Option<(usize, u64)>,
    pub(crate) dts: Option<(usize, u64)>,
    /// Where the PES payload starts in the packet.
    pub(crate) payload: usize,
}

fn payload_at(p: &[u8]) -> Option<usize> {
    let control = (p[3] >> 4) & 3;
    let at = match control {
        1 => 4,
        3 => 5 + usize::from(p[4]),
        _ => return None,
    };
    (at < PACKET).then_some(at)
}

pub(crate) fn head(p: &[u8]) -> Option<Head> {
    if p[1] & 0x40 == 0 {
        return None;
    }
    let at = payload_at(p)?;
    let h = p.get(at..at + 9)?;
    if h[..3] != [0, 0, 1] {
        return None;
    }
    let flags = h[7] >> 6;
    let read = |o: usize| -> Option<(usize, u64)> {
        let b = p.get(o..o + 5)?;
        let v = (u64::from(b[0] >> 1 & 7) << 30) | (u64::from(b[1]) << 22) | (u64::from(b[2] >> 1) << 15) | (u64::from(b[3]) << 7) | u64::from(b[4] >> 1);
        Some((o, v))
    };
    Some(Head {
        pts: if flags & 2 != 0 { read(at + 9) } else { None },
        dts: if flags == 3 { read(at + 14) } else { None },
        payload: at + 9 + usize::from(h[8]),
    })
}

pub(crate) fn write_stamp(p: &mut [u8], at: usize, value: u64) {
    let prefix = p[at] & 0xF0;
    let v = value & (WRAP - 1);
    p[at] = prefix | ((v >> 29) & 0x0E) as u8 | 1;
    p[at + 1] = (v >> 22) as u8;
    p[at + 2] = ((v >> 14) & 0xFE) as u8 | 1;
    p[at + 3] = (v >> 7) as u8;
    p[at + 4] = ((v << 1) & 0xFE) as u8 | 1;
}

/// The clock base of a packet that carries one.
pub(crate) fn pcr(p: &[u8]) -> Option<u64> {
    if (p[3] >> 4) & 2 == 0 || p[4] < 7 || p[5] & 0x10 == 0 {
        return None;
    }
    let b = &p[6..12];
    Some((u64::from(b[0]) << 25) | (u64::from(b[1]) << 17) | (u64::from(b[2]) << 9) | (u64::from(b[3]) << 1) | u64::from(b[4] >> 7))
}

fn write_pcr(p: &mut [u8], base: u64) {
    let b = base & (WRAP - 1);
    p[6] = (b >> 25) as u8;
    p[7] = (b >> 17) as u8;
    p[8] = (b >> 9) as u8;
    p[9] = (b >> 1) as u8;
    p[10] = ((b & 1) << 7) as u8 | 0x7E | (p[10] & 1);
}

/// The picture coding type of an MPEG-2 picture whose header is in `payload` (1 I, 2 P, 3 B).
fn picture_type(payload: &[u8]) -> Option<u8> {
    let at = payload.windows(4).position(|w| w == [0, 0, 1, 0])?;
    payload.get(at + 5).map(|b| (b >> 3) & 7)
}

/// Whether the GOP whose header is in `payload` is closed (its first B pictures need nothing before it).
fn closed_gop(payload: &[u8]) -> bool {
    payload
        .windows(4)
        .position(|w| w == [0, 0, 1, 0xB8])
        .and_then(|at| payload.get(at + 7))
        .is_some_and(|b| b & 0x40 != 0)
}

/// The program as it is written: its tables, the counters and the clock of each of its PIDs, and how the station's programme is laid onto
/// it while it plays.
pub struct Program {
    layout: Layout,
    version: u8,
    pat: Vec<u8>,
    pmt: Vec<u8>,
    pat_out: psi::SectionPackets,
    pmt_out: psi::SectionPackets,
    /// The next continuity counter of the picture, the clock and each sound.
    cc: [u8; 2 + MAX_AUDIO],
    /// The picture's next decoding time and the latest presentation time written.
    next_dts: u64,
    shown: u64,
    /// Each sound's next presentation time.
    audio_next: Vec<u64>,
    last_pcr: Option<u64>,
    /// The duration of one picture.
    frame: u64,
    live: Option<Live>,
}

/// The station's programme while it plays: where its streams are, what is added to its times, what is added to its counters, and what is
/// left out.
struct Live {
    routes: Routes,
    offset: u64,
    /// Per program PID, what turns the station's counter into this side's, once its first packet went on.
    delta: [Option<u8>; 2 + MAX_AUDIO],
    /// The PES being left out on each program PID.
    dropping: [bool; 2 + MAX_AUDIO],
    /// The PES on its way on each program PID.
    held: [Held; 2 + MAX_AUDIO],
    /// The B pictures of an open GOP that follow its first picture predict from before the splice: they are left out until the next
    /// I or P picture.
    leading_b: bool,
    /// Each sound's last PES passed on, for where a slate's silence goes on after it.
    audio_last: Vec<Option<LastSound>>,
}

/// A sound's last PES: its PTS, its length in bytes, and the time a byte of it holds, from the PES before; and, when its AC-3 frames
/// were found, how many start in it and how many bytes its last one still needs from the next PES (a station's PES do not end with a
/// frame).
#[derive(Clone, Copy)]
struct LastSound {
    pts: u64,
    bytes: Option<usize>,
    per_byte: Option<f64>,
    frames: Option<(u64, usize)>,
}

/// The length of the AC-3 frame `b` starts with, when it starts with one at 48 kHz: four bytes a kb/s (A/52 table 5.18). 44.1 and 32 kHz
/// are not broadcast, and E-AC-3 (`bsid` 16) is not read.
fn ac3_frame(b: &[u8]) -> Option<usize> {
    const KBPS: [usize; 19] = [32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 448, 512, 576, 640];
    match b {
        [0x0B, 0x77, _, _, code, bsid, ..] if code >> 6 == 0 && bsid >> 3 <= 8 => KBPS.get(usize::from(code & 0x3F) / 2).map(|k| k * 4),
        _ => None,
    }
}

/// Where the first AC-3 frame in `data` starts: a sync word whose frame is followed by another, or runs past the end.
fn first_frame(data: &[u8]) -> Option<usize> {
    (0..data.len()).find(|&i| {
        ac3_frame(&data[i..]).is_some_and(|size| data.get(i + size..i + size + 2).is_none_or(|next| next == [0x0B, 0x77]))
    })
}

/// The AC-3 frames of a PES's `data`, the first at `start`: how many start in it, and how many bytes the last still needs.
fn frames_in(data: &[u8], start: usize) -> Option<(u64, usize)> {
    let (mut at, mut n) = (start, 0);
    while at < data.len() {
        let size = ac3_frame(&data[at..])?;
        n += 1;
        if at + size > data.len() {
            return Some((n, at + size - data.len()));
        }
        at += size;
    }
    Some((n, 0))
}

/// The PES that `packets` carry, from its head on.
fn pes_of(packets: &[[u8; PACKET]]) -> Vec<u8> {
    packets.iter().filter_map(|p| payload_at(p).map(|at| &p[at..])).flatten().copied().collect()
}

/// A PES on its way: its packets are kept until the next PES on its PID starts, so one that the stream breaks off in is never passed on
/// (a decoder that gets a cut AC-3 frame takes the silence after it as the rest of that frame, 0.3 s of it in Plex's transcoder).
#[derive(Default)]
struct Held {
    packets: Vec<[u8; PACKET]>,
    /// What passing it on moves on.
    stamps: Option<Stamps>,
}

enum Stamps {
    Picture { dts: Option<u64>, pts: Option<u64> },
    Sound { pts: u64, bytes: Option<usize> },
}

fn slot(pid: u16) -> usize {
    match pid {
        VIDEO => 0,
        CLOCK => 1,
        _ => 2 + usize::from(pid - AUDIO),
    }
}

impl Program {
    /// A program of `layout`, its TS id `tsid`, starting on its own clock.
    pub fn new(tsid: u16, layout: Layout, format: &Format) -> Self {
        let pat = psi::build_pat(tsid, layout.program, PMT);
        let pmt = layout.pmt(0);
        let start = 10 * 90_000;
        Self {
            audio_next: vec![start; layout.audio.len()],
            layout,
            version: 0,
            pat,
            pmt,
            pat_out: psi::SectionPackets::default(),
            pmt_out: psi::SectionPackets::default(),
            cc: [0; 2 + MAX_AUDIO],
            next_dts: start,
            shown: start,
            last_pcr: None,
            frame: format.ticks(1),
            live: None,
        }
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// The PAT and the PMT.
    pub fn tables(&mut self, out: &mut Vec<u8>) {
        self.pat_out.write(psi::PID_PAT, &self.pat, out);
        self.pmt_out.write(PMT, &self.pmt, out);
    }

    /// A slate frame: `es` (a whole picture) on the picture's PID at the next frame time, with the clock, then silence on every sound up
    /// to that time.
    pub fn slate_frame(&mut self, es: &[u8], out: &mut Vec<u8>) {
        let dts = self.next_dts;
        let pcr = add(dts, WRAP - SLATE_DELAY);
        let pcr = match self.last_pcr {
            Some(last) if diff(pcr, last) < 0 => last,
            _ => pcr,
        };
        self.last_pcr = Some(pcr);
        let mut pes = vec![0, 0, 1, 0xE0, 0, 0, 0x84, 0x80, 5, 0, 0, 0, 0, 0];
        pes[9] = 0x20;
        write_stamp(&mut pes, 9, dts);
        pes.extend_from_slice(es);
        // FFmpeg's mpegts demuxer holds a video PES of length 0 until the next one starts; with its length it hands the picture on at
        // its last byte. 0 stays for a picture too long to say, which only video may send.
        if let Ok(len) = u16::try_from(pes.len() - 6) {
            pes[4..6].copy_from_slice(&len.to_be_bytes());
        }
        if self.layout.clock_apart {
            self.clock_packet(pcr, out);
            self.packetize(VIDEO, &pes, None, out);
        } else {
            self.packetize(VIDEO, &pes, Some(pcr), out);
        }
        self.shown = dts;
        self.next_dts = add(dts, self.frame);
        let behind = add(dts, WRAP - AUDIO_LAG);
        for i in 0..self.audio_next.len() {
            while diff(self.audio_next[i], behind) <= 0 {
                self.silence(i, out);
            }
        }
    }

    /// One frame of silence on sound `i` at its next time.
    fn silence(&mut self, i: usize, out: &mut Vec<u8>) {
        let pts = self.audio_next[i];
        let mut pes = vec![0, 0, 1, 0xBD, 0, 0, 0x84, 0x80, 5, 0x20, 0, 0, 0, 0];
        let len = pes.len() - 6 + SILENCE.len();
        pes[4..6].copy_from_slice(&(len as u16).to_be_bytes());
        write_stamp(&mut pes, 9, pts);
        pes.extend_from_slice(&SILENCE);
        self.packetize(AUDIO + i as u16, &pes, None, out);
        self.audio_next[i] = add(pts, AUDIO_FRAME);
    }

    /// Where the slate goes on from when the programme stops: after the last picture written and after the last one shown, so no
    /// time is used twice; and each sound after its last frame.
    pub fn splice_out(&mut self, format: &Format, out: &mut Vec<u8>) {
        self.frame = format.ticks(1);
        if diff(self.shown, self.next_dts) >= 0 {
            self.next_dts = add(self.shown, self.frame);
        }
        let Some(live) = self.live.take() else {
            return;
        };
        for (i, last) in live.audio_last.iter().enumerate() {
            let Some(last) = last else {
                continue;
            };
            self.audio_next[i] = match last.frames {
                Some((n, owed)) => {
                    if owed > 0 {
                        // the last frame passed on ends in the PES the stream broke off in: that end goes on (made up with zeros as
                        // far as it did not come), so the silence after it is not taken for the rest of the frame
                        let held = pes_of(&live.held[2 + i].packets);
                        let data = held.get(9 + usize::from(held.get(8).copied().unwrap_or(0))..).unwrap_or_default();
                        let mut pes = vec![0, 0, 1, 0xBD, 0, 0, 0x80, 0, 0];
                        pes.extend_from_slice(&data[..owed.min(data.len())]);
                        pes.resize(9 + owed, 0);
                        let length = (pes.len() - 6).min(0xFFFF) as u16;
                        pes[4..6].copy_from_slice(&length.to_be_bytes());
                        self.packetize(AUDIO + i as u16, &pes, None, out);
                    }
                    add(last.pts, n * AUDIO_FRAME)
                }
                // otherwise the last PES lasts as long as its bytes do at the rate of the one before it
                None => add(last.pts, last.bytes.zip(last.per_byte).map_or(AUDIO_FRAME, |(b, r)| (b as f64 * r).round() as u64)),
            };
        }
    }

    /// The station's programme starts at its packet `first`, the head of a picture that decodes on its own: from here its times are moved
    /// so that picture is decoded one frame after the last slate frame, and its PMT becomes the program's (a new version when it differs).
    pub fn splice_in(&mut self, layout: Layout, routes: Routes, first: &[u8], format: &Format) {
        let (dts, payload) = match head(first) {
            Some(h) => (h.dts.or(h.pts).map_or(0, |(_, v)| v), first[h.payload.min(PACKET)..].to_vec()),
            None => (0, Vec::new()),
        };
        self.frame = format.ticks(1);
        let offset = add(self.next_dts, WRAP - dts);
        if layout != self.layout {
            self.version = (self.version + 1) & 0x1F;
            self.pmt = layout.pmt(self.version);
            self.audio_next.resize(layout.audio.len(), self.next_dts);
            self.layout = layout;
        }
        let sounds = routes.audio.len();
        self.live = Some(Live {
            routes,
            offset,
            delta: [None; 2 + MAX_AUDIO],
            // a sound's packets before its first PES head are the tail of a frame from before the picture: left out
            dropping: std::array::from_fn(|s| s >= 2),
            held: Default::default(),
            leading_b: !closed_gop(&payload),
            audio_last: vec![None; sounds],
        });
    }

    pub fn is_live(&self) -> bool {
        self.live.is_some()
    }

    /// Passes the station's packets in `packets` on as the program's (`splice_in` first): the program's PIDs, the counters carried on, the
    /// times moved; the rest is left out. Each PES goes on once the next one on its PID starts. A sound's PES that would start before the
    /// slate's silence has ended is left out whole, and so are the B pictures that lead an open GOP.
    pub fn live(&mut self, packets: &[u8], out: &mut Vec<u8>) {
        let Some(mut live) = self.live.take() else {
            return;
        };
        for p in packets.chunks_exact(PACKET) {
            let Some(pid) = live.routes.map(psi::pid_of_packet(p)) else {
                continue;
            };
            let s = slot(pid);
            let mut p: [u8; PACKET] = p.try_into().expect("a whole packet");
            p[1] = (p[1] & 0xE0) | (pid >> 8) as u8;
            p[2] = pid as u8;
            if let Some(h) = head(&p) {
                // the PES before it on its PID is whole
                self.pass(&mut live, s, out);
                live.dropping[s] = false;
                let mut stamps = [h.pts, h.dts];
                for (at, v) in stamps.iter_mut().flatten() {
                    *v = add(*v, live.offset);
                    write_stamp(&mut p, *at, *v);
                }
                let (pts, dts) = (stamps[0].map(|(_, v)| v), stamps[1].map(|(_, v)| v));
                if pid == VIDEO {
                    if live.leading_b {
                        match picture_type(&p[h.payload.min(PACKET)..]) {
                            Some(3) => live.dropping[s] = true,
                            Some(_) => live.leading_b = false,
                            None => {}
                        }
                    }
                    live.held[s].stamps = Some(Stamps::Picture { dts, pts });
                } else if pid >= AUDIO
                    && let Some(pts) = pts
                {
                    let i = usize::from(pid - AUDIO);
                    if live.audio_last[i].is_none() && diff(pts, self.audio_next[i]) < 0 {
                        live.dropping[s] = true;
                    }
                    let at = payload_at(&p).unwrap_or(PACKET - 1);
                    let length = usize::from(u16::from_be_bytes([p[at + 4], p[at + 5]]));
                    let bytes = (length > 0).then(|| length.saturating_sub(3 + usize::from(p[at + 8])));
                    live.held[s].stamps = Some(Stamps::Sound { pts, bytes });
                }
            }
            if let Some(base) = pcr(&p) {
                let moved = add(base, live.offset);
                write_pcr(&mut p, moved);
                self.last_pcr = Some(moved);
            }
            if live.dropping[s] {
                live.held[s].stamps = None;
                if p[3] & 0x10 != 0
                    && let Some(d) = live.delta[s].as_mut()
                {
                    // the counters after it close up over what was left out here
                    *d = d.wrapping_sub(1) & 0x0F;
                }
                continue;
            }
            live.held[s].packets.push(p);
            if pid == CLOCK {
                // the clock's own packets carry no PES
                self.pass(&mut live, s, out);
            }
        }
        self.live = Some(live);
    }

    /// Passes the PES held on slot `s` on: its counters carried on from the program's, and what it moves on (a picture's times, a
    /// sound's, the silence that fills up to the first sound after a slate).
    fn pass(&mut self, live: &mut Live, s: usize, out: &mut Vec<u8>) {
        let held = std::mem::take(&mut live.held[s]);
        if held.packets.is_empty() {
            return;
        }
        match held.stamps {
            Some(Stamps::Picture { dts, pts }) => {
                if let Some(dts) = dts.or(pts) {
                    self.next_dts = add(dts, self.frame);
                }
                if let Some(pts) = pts
                    && diff(pts, self.shown) > 0
                {
                    self.shown = pts;
                }
            }
            Some(Stamps::Sound { pts, bytes }) => {
                let i = s - 2;
                let last = live.audio_last[i];
                let pes = pes_of(&held.packets);
                let data_at = (9 + usize::from(pes.get(8).copied().unwrap_or(0))).min(pes.len());
                let data = &pes[data_at..];
                // its first frame starts where the last PES's last frame ended; after a slate, or when that is not a frame, at the first
                // frame found
                let start = last
                    .and_then(|l| l.frames)
                    .map(|(_, owed)| owed)
                    .filter(|&o| data.get(o..).and_then(ac3_frame).is_some())
                    .or_else(|| first_frame(data));
                let frames = start.and_then(|st| frames_in(data, st));
                let per_byte = match last {
                    None => {
                        while diff(add(self.audio_next[i], AUDIO_FRAME), pts) <= 0 {
                            self.silence(i, out);
                        }
                        None
                    }
                    Some(last) => {
                        let gap = diff(pts, last.pts);
                        let rate = last.bytes.filter(|&b| b > 0 && gap > 0 && gap < 90_000).map(|b| gap as f64 / b as f64);
                        rate.or(last.per_byte)
                    }
                };
                live.audio_last[i] = Some(LastSound { pts, bytes, per_byte, frames });
                if last.is_none()
                    && let Some(st) = start.filter(|&st| st > 0)
                {
                    // the bytes before its first frame are the end of one from before the slate: it goes on without them
                    let mut cut = pes[..data_at].to_vec();
                    cut.extend_from_slice(&data[st..]);
                    if bytes.is_some() {
                        let length = (cut.len() - 6).min(0xFFFF) as u16;
                        cut[4..6].copy_from_slice(&length.to_be_bytes());
                    }
                    self.packetize(AUDIO + i as u16, &cut, None, out);
                    return;
                }
            }
            None => {}
        }
        for mut p in held.packets {
            let cc = if p[3] & 0x10 != 0 {
                let delta = *live.delta[s].get_or_insert(self.cc[s].wrapping_sub(p[3] & 0x0F) & 0x0F);
                let cc = ((p[3] & 0x0F) + delta) & 0x0F;
                self.cc[s] = (cc + 1) & 0x0F;
                cc
            } else {
                // a packet without payload does not take a count: it carries the last one
                self.cc[s].wrapping_sub(1) & 0x0F
            };
            p[3] = (p[3] & 0xF0) | cc;
            out.extend_from_slice(&p);
        }
    }

    /// A packet of the clock alone, on the clock's own PID.
    fn clock_packet(&mut self, pcr: u64, out: &mut Vec<u8>) {
        let mut p = [0xFF; PACKET];
        p[0] = 0x47;
        p[1] = (CLOCK >> 8) as u8;
        p[2] = CLOCK as u8;
        p[3] = 0x20 | (self.cc[1].wrapping_sub(1) & 0x0F);
        p[4] = 183;
        p[5] = 0x10;
        write_pcr(&mut p, pcr);
        p[10] = 0x7E | ((pcr & 1) << 7) as u8;
        p[11] = 0;
        out.extend_from_slice(&p);
    }

    /// Cuts `pes` into packets of `pid`, the first with the clock when there is one, the last filled out by its adaptation field.
    fn packetize(&mut self, pid: u16, pes: &[u8], pcr: Option<u64>, out: &mut Vec<u8>) {
        let s = slot(pid);
        let mut rest = pes;
        let mut first = true;
        while !rest.is_empty() {
            let mut p = [0xFF; PACKET];
            p[0] = 0x47;
            p[1] = if first { 0x40 } else { 0 } | (pid >> 8) as u8;
            p[2] = pid as u8;
            let clock = if first { pcr } else { None };
            // the adaptation field: the clock, and as much stuffing as the last packet's payload leaves room for
            let fixed = if clock.is_some() { 8 } else { 0 };
            let room = PACKET - 4 - fixed;
            let n = rest.len().min(room);
            let af = if fixed > 0 || n < PACKET - 4 { PACKET - 4 - n } else { 0 };
            p[3] = if af > 0 { 0x30 } else { 0x10 } | self.cc[s];
            self.cc[s] = (self.cc[s] + 1) & 0x0F;
            if af > 0 {
                p[4] = (af - 1) as u8;
                if af > 1 {
                    p[5] = if clock.is_some() { 0x10 } else { 0 };
                }
                if let Some(base) = clock {
                    write_pcr(&mut p, base);
                    p[10] = 0x7E | ((base & 1) << 7) as u8;
                    p[11] = 0;
                }
            }
            p[4 + af..4 + af + n].copy_from_slice(&rest[..n]);
            rest = &rest[n..];
            first = false;
            out.extend_from_slice(&p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SD: Format = Format { width: 720, height: 480, rate: 4, aspect: 3, progressive: false };

    fn packets(b: &[u8]) -> Vec<[u8; PACKET]> {
        b.chunks_exact(PACKET).map(|p| p.try_into().unwrap()).collect()
    }

    /// A station's packet of `pid`: a PES head with `pts` and `dts` when given (a sound's PES saying it is 768 bytes, one 192 kb/s AC-3
    /// frame), the continuity counter `cc`.
    fn station(pid: u16, cc: u8, stamps: Option<(u64, Option<u64>)>, body: &[u8]) -> [u8; PACKET] {
        let mut p = [0xFFu8; PACKET];
        p[0] = 0x47;
        p[1] = (pid >> 8) as u8 | if stamps.is_some() { 0x40 } else { 0 };
        p[2] = pid as u8;
        p[3] = 0x10 | cc;
        let mut at = 4;
        if let Some((pts, dts)) = stamps {
            let length: u16 = if pid == 0x31 { 0 } else { 3 + 5 + 768 };
            let [hi, lo] = length.to_be_bytes();
            let mut h = vec![0, 0, 1, if pid == 0x31 { 0xE0 } else { 0xBD }, hi, lo, 0x80, if dts.is_some() { 0xC0 } else { 0x80 }];
            h.push(if dts.is_some() { 10 } else { 5 });
            h.extend([0x31, 0, 1, 0, 1]);
            if dts.is_some() {
                h.extend([0x11, 0, 1, 0, 1]);
            }
            p[at..at + h.len()].copy_from_slice(&h);
            write_stamp(&mut p, at + 9, pts);
            if let Some(d) = dts {
                write_stamp(&mut p, at + 14, d);
            }
            at += h.len();
        }
        p[at..at + body.len()].copy_from_slice(body);
        p
    }

    fn stamps_of(p: &[u8]) -> (Option<u64>, Option<u64>) {
        let h = head(p).unwrap();
        (h.pts.map(|x| x.1), h.dts.map(|x| x.1))
    }

    /// The PTS of every sound PES head among `packets`.
    fn sounds(packets: &[[u8; PACKET]]) -> Vec<u64> {
        packets.iter().filter(|p| psi::pid_of_packet(&p[..]) == AUDIO && head(&p[..]).is_some()).map(|p| stamps_of(&p[..]).0.unwrap()).collect()
    }

    fn station_pmt() -> psi::Pmt {
        psi::parse_pmt(&psi::build_pmt(3, 7, 0x31, &[0x05, 4, b'G', b'A', b'9', b'4'], &[(0x02, 0x31, &[]), (0x81, 0x34, &[0x0A, 4, b'e', b'n', b'g', 0]), (0x81, 0x35, &[])]))
            .unwrap()
    }

    #[test]
    fn a_slate_frame_is_a_picture_with_the_clock_and_its_silence_keeps_half_a_second_behind() {
        let mut p = Program::new(1, Layout::fallback(3), &SD);
        let mut out = Vec::new();
        p.tables(&mut out);
        p.slate_frame(&[0, 0, 1, 0xB3, 1, 2, 3], &mut out);
        for _ in 1..17 {
            p.slate_frame(&[0, 0, 1, 0x00, 4, 5], &mut out);
        }
        let all = packets(&out);
        let pmt = psi::parse_pmt(&all[1][5..]).unwrap();
        assert_eq!((pmt.pcr_pid, pmt.streams[0].pid, pmt.streams[1].pid), (VIDEO, VIDEO, AUDIO));
        let video: Vec<_> = all.iter().filter(|p| psi::pid_of_packet(&p[..]) == VIDEO).collect();
        assert_eq!(stamps_of(&video[0][..]).0, Some(900_000));
        assert_eq!(stamps_of(&video[1][..]).0, Some(903_003));
        assert_eq!(pcr(&video[0][..]), Some(810_000), "a second behind");
        assert_eq!((video[0][3] & 0x0F, video[1][3] & 0x0F), (0, 1));
        // the last frame is at 948048: silence up to 903048, frames at 900000 and 902880
        assert_eq!(sounds(&all), [900_000, 902_880]);
        let first = all.iter().position(|p| psi::pid_of_packet(&p[..]) == AUDIO).unwrap();
        assert!(all[..first].iter().filter(|p| psi::pid_of_packet(&p[..]) == VIDEO).count() >= 16);
        let audio = all[first];
        let h = head(&audio[..]).unwrap();
        assert_eq!(&audio[h.payload..h.payload + 2], &[0x0b, 0x77], "an AC-3 frame");
    }

    #[test]
    fn a_slate_picture_says_its_length_unless_it_is_too_long_to_say() {
        let mut p = Program::new(1, Layout::fallback(3), &SD);
        // the length the PES head says, and the bytes the PES has after it
        let mut lengths = |es: &[u8]| {
            let mut out = Vec::new();
            p.slate_frame(es, &mut out);
            let video: Vec<_> = packets(&out).into_iter().filter(|q| psi::pid_of_packet(&q[..]) == VIDEO).collect();
            let at = payload_at(&video[0]).unwrap();
            let rest: usize = video[1..].iter().map(|q| PACKET - payload_at(&q[..]).unwrap()).sum();
            (u16::from_be_bytes([video[0][at + 4], video[0][at + 5]]), PACKET - at - 6 + rest)
        };
        assert_eq!(lengths(&[0, 0, 1, 0xB3, 1, 2, 3]), (8 + 7, 8 + 7));
        assert_eq!(lengths(&vec![0; 70_000]), (0, 8 + 70_000));
    }

    #[test]
    fn the_programme_goes_on_from_the_slate_on_its_pids_its_clock_and_its_counters_the_silence_filled_up_to_its_sound() {
        let pmt = station_pmt();
        let (layout, routes) = Layout::of(&pmt).unwrap();
        assert_eq!((routes.video, routes.audio.clone(), routes.clock), (0x31, vec![0x34, 0x35], 0x31));
        let mut p = Program::new(1, Layout::fallback(3), &SD);
        let mut out = Vec::new();
        p.slate_frame(&[0, 0, 1, 0xB3], &mut out);
        p.slate_frame(&[0, 0, 1, 0x00], &mut out);
        let slate_next = p.next_dts;
        // the station: a keyframe of a closed GOP at DTS 5_000_000, then a sound at 5_010_000
        let key = {
            let mut k = station(0x31, 9, Some((5_006_006, Some(5_000_000))), &[]);
            // a clock in the keyframe's packet: the adaptation field takes 8 bytes
            k.copy_within(4..PACKET - 8, 12);
            k[3] = 0x30 | 9;
            k[4] = 7;
            k[5] = 0x10;
            write_pcr(&mut k, 4_920_000);
            let gop = [0, 0, 1, 0xB3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0xB8, 0, 8, 0, 0x40];
            let at = head(&k).unwrap().payload;
            k[at..at + gop.len()].copy_from_slice(&gop);
            k
        };
        p.splice_in(layout.clone(), routes, &key, &SD);
        assert_eq!(p.layout(), &layout);
        let tail = station(0x34, 2, None, &[0]);
        let audio = station(0x34, 3, Some((5_010_000, None)), &[0x0b, 0x77]);
        let other = station(0x44, 0, Some((1, None)), &[]);
        let more = station(0x31, 10, None, &[1, 2, 3]);
        // the next PES on each PID: what was before it is whole and goes on, and they wait for theirs
        let next_audio = station(0x34, 4, Some((5_010_000 + 8640, None)), &[0x0b, 0x77]);
        let next_picture = station(0x31, 11, Some((5_009_009, Some(5_003_003))), &[]);
        let mut live = Vec::new();
        p.live(&[key, tail, audio, other, more, next_audio, next_picture].concat(), &mut live);
        let got = packets(&live);
        let video: Vec<_> = got.iter().filter(|q| psi::pid_of_packet(&q[..]) == VIDEO).collect();
        assert_eq!(video.len(), 2, "another program's packet is left out");
        assert_eq!(stamps_of(&video[0][..]), (Some(slate_next + 6006), Some(slate_next)), "decoded right after the slate");
        assert_eq!(pcr(&video[0][..]), Some(slate_next - 80_000), "the clock moved with the times");
        assert_eq!((video[0][3] & 0x0F, video[1][3] & 0x0F), (2, 3), "the picture's counter goes on from the slate's");
        // no silence went out before (it keeps half a second behind): it is filled from 900000 up to the sound, which goes on from it
        let sound = sounds(&got);
        assert_eq!(*sound.last().unwrap(), slate_next + 10_000);
        assert!(sound.windows(2).all(|w| w[1] > w[0]));
        assert_eq!(sound[0], 900_000);
        assert!(slate_next + 10_000 - sound[sound.len() - 2] <= 2 * 2880);
        let counts: Vec<u8> = got.iter().filter(|q| psi::pid_of_packet(&q[..]) == AUDIO).map(|q| q[3] & 0x0F).collect();
        assert!(counts.windows(2).all(|w| w[1] == (w[0] + 1) & 0x0F), "the tail of a frame from before is left out: {counts:?}");
    }

    #[test]
    fn leading_b_pictures_of_an_open_gop_and_a_sound_from_before_the_silence_ended_are_left_out_without_a_gap_in_the_counters() {
        let pmt = station_pmt();
        let (layout, routes) = Layout::of(&pmt).unwrap();
        let mut p = Program::new(1, layout.clone(), &SD);
        let mut out = Vec::new();
        for _ in 0..20 {
            p.slate_frame(&[0, 0, 1, 0x00], &mut out);
        }
        let silence_reached = p.audio_next[0];
        let picture = |cc: u8, dts: u64, kind: u8| {
            let body = [0, 0, 1, 0x00, 0, kind << 3, 0, 0];
            station(0x31, cc, Some((dts + 3003, Some(dts))), &body)
        };
        let mut key = station(0x31, 0, Some((2_006_006, Some(2_000_000))), &[0, 0, 1, 0xB3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0xB8, 0, 8, 0, 0]);
        key[1] |= 0x40;
        p.splice_in(layout, routes, &key, &SD);
        // a sound from 0.6 s before the picture, which the silence (half a second behind the slate) has passed
        let early_audio = station(0x34, 5, Some((2_000_000 - 54_000, None)), &[0x0b, 0x77]);
        let early_audio_rest = station(0x34, 6, None, &[0]);
        let audio = station(0x34, 7, Some((2_004_000, None)), &[0x0b, 0x77]);
        let b1 = picture(1, 1_996_997, 3);
        let b1_rest = station(0x31, 2, None, &[]);
        let next_p = picture(3, 2_009_009, 2);
        let after = [picture(4, 2_012_012, 2), station(0x34, 8, Some((2_004_000 + 8640, None)), &[0x0b, 0x77])];
        let mut live = Vec::new();
        p.live(&[&[key, early_audio, early_audio_rest, audio, b1, b1_rest, next_p][..], &after].concat().concat(), &mut live);
        let got = packets(&live);
        let video: Vec<_> = got.iter().filter(|q| psi::pid_of_packet(&q[..]) == VIDEO).collect();
        assert_eq!(video.len(), 2, "the B picture's two packets are left out");
        assert_eq!(((video[1][3] & 0x0F) + 16 - (video[0][3] & 0x0F)) % 16, 1, "and leave no gap");
        let sound = sounds(&got);
        assert!(sound.iter().all(|&s| diff(s, silence_reached) >= 0), "the early sound is left out: {sound:?}");
        let counts: Vec<u8> = got.iter().filter(|q| psi::pid_of_packet(&q[..]) == AUDIO).map(|q| q[3] & 0x0F).collect();
        assert!(counts.windows(2).all(|w| w[1] == (w[0] + 1) & 0x0F), "{counts:?}");
    }

    #[test]
    fn a_slate_after_the_programme_starts_after_the_last_picture_shown_and_when_the_last_sound_ends() {
        let pmt = station_pmt();
        let (layout, routes) = Layout::of(&pmt).unwrap();
        let mut p = Program::new(1, layout.clone(), &SD);
        let mut out = Vec::new();
        p.slate_frame(&[0, 0, 1, 0x00], &mut out);
        let key = station(0x31, 0, Some((1_000_000 + 9009, Some(1_000_000))), &[0, 0, 1, 0xB3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0xB8, 0, 8, 0, 0x40]);
        p.splice_in(layout, routes, &key, &SD);
        let start = p.next_dts;
        let a1 = station(0x34, 0, Some((1_000_000 + 9000, None)), &[]);
        let a2 = station(0x34, 1, Some((1_000_000 + 9000 + 8640, None)), &[]);
        let a3 = station(0x34, 2, Some((1_000_000 + 9000 + 2 * 8640, None)), &[]);
        let p2 = station(0x31, 1, Some((1_000_000 + 6006, Some(1_000_000 + 3003))), &[0, 0, 1, 0x00, 0, 2 << 3, 0, 0]);
        p.live(&[key, a1, a2, a3, p2].concat(), &mut out);
        p.splice_out(&SD, &mut out);
        // the stream broke off in the second picture and the third sound, which are left out: the first was shown up to start + 9009,
        // so the slate goes on a frame after it; the sound when the second PES ends, 768 bytes at the pace of the one before
        assert_eq!(p.next_dts, start + 9009 + 3003);
        assert_eq!(p.audio_next[0], start + 9000 + 8640 + 8640);
        assert!(!p.is_live());
    }

    /// A station's sound PES at `pts` on PID 0x34 holding `body`, packetized as a station does (the last packet's room taken by
    /// stuffing), its counters from `cc`.
    fn station_sound(cc: u8, pts: u64, body: &[u8]) -> Vec<[u8; PACKET]> {
        let mut pes = vec![0, 0, 1, 0xBD, 0, 0, 0x84, 0x80, 5, 0x20, 0, 0, 0, 0];
        write_stamp(&mut pes, 9, pts);
        pes.extend_from_slice(body);
        let length = (pes.len() - 6) as u16;
        pes[4..6].copy_from_slice(&length.to_be_bytes());
        let mut src = Program::new(1, Layout::fallback(1), &SD);
        src.cc[2] = cc;
        let mut out = Vec::new();
        src.packetize(AUDIO, &pes, None, &mut out);
        packets(&out)
            .into_iter()
            .map(|mut p| {
                p[1] &= 0xE0;
                p[2] = 0x34;
                p
            })
            .collect()
    }

    #[test]
    fn an_ac3_frame_a_splice_cuts_is_left_out_before_the_programme_and_made_whole_after_it() {
        let pmt = station_pmt();
        let (layout, routes) = Layout::of(&pmt).unwrap();
        // 64 kb/s frames, 256 bytes: a PES of one and a bit takes two packets
        let frame = |n: u8| {
            let mut f = [n; 256];
            f[..6].copy_from_slice(&[0x0B, 0x77, 0, 0, 0x08, 0x40]);
            f
        };
        // the first PES: the end of a frame from before the picture, a whole frame and the start of the next; the second: that one's
        // end and the start of a third; the third: the third's end
        let first = [&[9u8; 10][..], &frame(1), &frame(2)[..20]].concat();
        let second = [&frame(2)[20..], &frame(3)[..30]].concat();
        let third = [&frame(3)[30..], &frame(4)[..50]].concat();
        // how much of the third PES came before the stream broke off: all of it, its first packet (170 bytes of the 226 the third frame
        // still needed), or none (the second PES is then the one broken off in)
        for (came, whole, ends) in [
            (2, [&frame(1)[..], &frame(2), &frame(3)].concat(), 3),
            (1, [&frame(1)[..], &frame(2), &frame(3)[..200], &[0; 56]].concat(), 3),
            (0, [&frame(1)[..], &frame(2)].concat(), 2),
        ] {
            let mut p = Program::new(1, layout.clone(), &SD);
            let mut out = Vec::new();
            p.slate_frame(&[0, 0, 1, 0x00], &mut out);
            let key = station(0x31, 0, Some((2_006_006, Some(2_000_000))), &[0, 0, 1, 0xB3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0xB8, 0, 8, 0, 0x40]);
            p.splice_in(layout.clone(), routes.clone(), &key, &SD);
            let offset = add(p.next_dts, WRAP - 2_000_000);
            let mut station = vec![key];
            station.extend(station_sound(0, 2_050_000, &first));
            station.extend(station_sound(2, 2_050_000 + 2 * 2880, &second));
            station.extend(station_sound(4, 2_050_000 + 3 * 2880, &third).into_iter().take(came));
            let mut live = Vec::new();
            p.live(&station.concat(), &mut live);
            p.splice_out(&SD, &mut live);
            let mut pes: Vec<Vec<[u8; PACKET]>> = Vec::new();
            for q in packets(&live).into_iter().filter(|q| psi::pid_of_packet(&q[..]) == AUDIO) {
                match pes.last_mut() {
                    Some(last) if q[1] & 0x40 == 0 => last.push(q),
                    _ => pes.push(vec![q]),
                }
            }
            let real: Vec<Vec<u8>> = pes.iter().map(|q| pes_of(q)).filter(|e| e.len() != 9 + 5 + SILENCE.len()).collect();
            let data: Vec<u8> = real.iter().flat_map(|e| e[9 + usize::from(e[8])..].to_vec()).collect();
            assert_eq!(data, whole, "{came} packets came");
            assert_eq!(usize::from(u16::from_be_bytes([real[0][4], real[0][5]])), real[0].len() - 6, "its length says so");
            assert_eq!(p.audio_next[0], add(2_050_000 + ends * 2880, offset), "the silence goes on where the last whole frame ends");
        }
    }

    #[test]
    fn times_wrap_at_33_bits() {
        assert_eq!(diff(1, WRAP - 1), 2);
        assert_eq!(diff(WRAP - 1, 1), -2);
        assert_eq!(add(WRAP - 1, 2), 1);
        let mut p = [0u8; PACKET];
        p[3] = 0x30;
        p[4] = 7;
        p[5] = 0x10;
        write_pcr(&mut p, WRAP + 5);
        assert_eq!(pcr(&p), Some(5));
    }

    #[test]
    fn a_layout_leaves_out_conditional_access_and_knows_a_clock_of_its_own() {
        let pmt = psi::parse_pmt(&psi::build_pmt(1, 0, 0x1FF0, &[0x09, 2, 0, 0, 0x05, 0], &[(0x02, 0x31, &[]), (0x81, 0x34, &[])])).unwrap();
        let (layout, routes) = Layout::of(&pmt).unwrap();
        assert_eq!(layout.descriptors, [0x05, 0]);
        assert!(layout.clock_apart);
        assert_eq!(routes.map(0x1FF0), Some(CLOCK));
        assert_eq!(routes.pids(), [0x31, 0x34, 0x1FF0]);
        let none = psi::parse_pmt(&psi::build_pmt(1, 0, 0x34, &[], &[(0x81, 0x34, &[])])).unwrap();
        assert!(Layout::of(&none).is_none(), "no picture, no slate");
    }
}
