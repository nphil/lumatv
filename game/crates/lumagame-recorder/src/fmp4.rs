//! A fragmented MP4 writer for one coded video track and, when the stream has it, one Opus track. The output is `ftyp`, `moov` with empty
//! sample tables, then a `moof` and `mdat` about every second, so it streams through a pipe to a NAS and a file cut short anywhere still
//! plays up to the last whole fragment. Sample data is written from the shared units, with H.264/H.265 start codes turned into length
//! prefixes on the way out; the only copy held is the list of unit references of the fragment being built.
//!
//! Timestamps are the arrival times the sender gave (`pts_ns`); the picture count of a clip is exact, its timing is the stream as it
//! reached the TV. B-frame reordering is not handled (the three streaming codecs are used without it).
use std::io::{self, Write};
use std::sync::Arc;

use crate::bitstream::{self, Codec};
use crate::ring::Unit;
use crate::{AudioConfig, Config, Track};

const VIDEO_TIMESCALE: u32 = 90_000;
const AUDIO_TIMESCALE: u32 = 48_000;
/// A fragment closes after this much video, or this many bytes, whichever comes first.
const FRAGMENT_NS: i64 = 1_000_000_000;
pub const FRAGMENT_BYTES: usize = 4 << 20;

const SYNC: u32 = 0x0200_0000;
const NON_SYNC: u32 = 0x0101_0000;

/// A sample's payload: byte ranges of one unit, each written as is or, for NAL units, behind a 4-byte length.
struct Sample {
    dts: u64,
    duration: u32,
    flags: u32,
    data: Arc<[u8]>,
    ranges: Vec<(u32, u32)>,
    length_prefixed: bool,
}

impl Sample {
    fn size(&self) -> u32 {
        self.ranges.iter().map(|&(s, e)| e - s + if self.length_prefixed { 4 } else { 0 }).sum()
    }
}

struct Writer {
    out: Vec<u8>,
}

impl Writer {
    fn new() -> Self {
        Self { out: Vec::new() }
    }
    fn u8(&mut self, v: u8) {
        self.out.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.out.extend_from_slice(&v.to_be_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_be_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.out.extend_from_slice(&v.to_be_bytes());
    }
    fn bytes(&mut self, v: &[u8]) {
        self.out.extend_from_slice(v);
    }
    fn zeros(&mut self, n: usize) {
        self.out.resize(self.out.len() + n, 0);
    }
    fn matrix(&mut self) {
        for v in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
            self.u32(v);
        }
    }
    /// A box whose size is filled in after `body` has written its contents.
    fn boxed(&mut self, ty: &[u8; 4], body: impl FnOnce(&mut Self)) {
        let at = self.out.len();
        self.u32(0);
        self.bytes(ty);
        body(self);
        let size = (self.out.len() - at) as u32;
        self.out[at..at + 4].copy_from_slice(&size.to_be_bytes());
    }
    fn full_box(&mut self, ty: &[u8; 4], version: u8, flags: u32, body: impl FnOnce(&mut Self)) {
        self.boxed(ty, |w| {
            w.u32(u32::from(version) << 24 | flags);
            body(w);
        });
    }
}

fn ftyp() -> Vec<u8> {
    let mut w = Writer::new();
    w.boxed(b"ftyp", |w| {
        w.bytes(b"isom");
        w.u32(512);
        for brand in [b"isom", b"iso6", b"mp41"] {
            w.bytes(brand);
        }
    });
    w.out
}

fn visual_entry(w: &mut Writer, c: &Config) {
    let fourcc = match c.codec {
        Codec::H264 => b"avc1",
        Codec::H265 => b"hvc1",
        Codec::Av1 => b"av01",
    };
    w.boxed(fourcc, |w| {
        w.zeros(6);
        w.u16(1);
        w.zeros(16);
        w.u16(c.width as u16);
        w.u16(c.height as u16);
        w.u32(0x0048_0000);
        w.u32(0x0048_0000);
        w.u32(0);
        w.u16(1);
        w.zeros(32);
        w.u16(0x0018);
        w.u16(0xffff);
        let config = match c.codec {
            Codec::H264 => b"avcC",
            Codec::H265 => b"hvcC",
            Codec::Av1 => b"av1C",
        };
        w.boxed(config, |w| w.bytes(&c.record));
        if let Some(h) = &c.hdr {
            w.boxed(b"colr", |w| {
                w.bytes(b"nclx");
                w.u16(u16::from(h.colour_primaries));
                w.u16(u16::from(h.transfer));
                w.u16(u16::from(h.matrix));
                w.u8(0);
            });
            if let Some(m) = &h.mastering {
                w.boxed(b"mdcv", |w| {
                    m.primaries.iter().chain(&m.white).for_each(|&v| w.u16(v));
                    w.u32(m.max_luminance);
                    w.u32(m.min_luminance);
                });
                w.boxed(b"clli", |w| {
                    w.u16(m.max_cll);
                    w.u16(m.max_fall);
                });
            }
        }
    });
}

fn audio_entry(w: &mut Writer, a: &AudioConfig) {
    w.boxed(b"Opus", |w| {
        w.zeros(6);
        w.u16(1);
        w.zeros(8);
        w.u16(u16::from(a.channels));
        w.u16(16);
        w.u32(0);
        w.u32(AUDIO_TIMESCALE << 16);
        w.boxed(b"dOps", |w| {
            w.u8(0);
            w.u8(a.channels);
            w.u16(312);
            w.u32(a.sample_rate);
            w.u16(0);
            match &a.mapping {
                Some(m) => {
                    w.u8(1);
                    w.u8(m.streams);
                    w.u8(m.coupled);
                    w.bytes(&m.map[..usize::from(a.channels).min(6)]);
                }
                None => w.u8(0),
            }
        });
    });
}

fn track(w: &mut Writer, id: u32, c: &Config, audio: bool) {
    w.boxed(b"trak", |w| {
        w.full_box(b"tkhd", 0, 3, |w| {
            w.u32(0);
            w.u32(0);
            w.u32(id);
            w.u32(0);
            w.u32(0);
            w.zeros(8);
            w.u16(0);
            w.u16(0);
            w.u16(if audio { 0x0100 } else { 0 });
            w.u16(0);
            w.matrix();
            if audio {
                w.u32(0);
                w.u32(0);
            } else {
                w.u32(c.width << 16);
                w.u32(c.height << 16);
            }
        });
        w.boxed(b"mdia", |w| {
            w.full_box(b"mdhd", 0, 0, |w| {
                w.u32(0);
                w.u32(0);
                w.u32(if audio { AUDIO_TIMESCALE } else { VIDEO_TIMESCALE });
                w.u32(0);
                w.u16(0x55c4);
                w.u16(0);
            });
            w.full_box(b"hdlr", 0, 0, |w| {
                w.u32(0);
                w.bytes(if audio { b"soun" } else { b"vide" });
                w.zeros(12);
                w.bytes(if audio { b"SoundHandler\0" } else { b"VideoHandler\0" });
            });
            w.boxed(b"minf", |w| {
                if audio {
                    w.full_box(b"smhd", 0, 0, |w| w.u32(0));
                } else {
                    w.full_box(b"vmhd", 0, 1, |w| w.zeros(8));
                }
                w.boxed(b"dinf", |w| {
                    w.full_box(b"dref", 0, 0, |w| {
                        w.u32(1);
                        w.full_box(b"url ", 0, 1, |_| {});
                    });
                });
                w.boxed(b"stbl", |w| {
                    w.full_box(b"stsd", 0, 0, |w| {
                        w.u32(1);
                        match (audio, &c.audio) {
                            (true, Some(a)) => audio_entry(w, a),
                            _ => visual_entry(w, c),
                        }
                    });
                    for ty in [b"stts", b"stsc", b"stco"] {
                        w.full_box(ty, 0, 0, |w| w.u32(0));
                    }
                    w.full_box(b"stsz", 0, 0, |w| w.zeros(8));
                });
            });
        });
    });
}

fn moov(c: &Config) -> Vec<u8> {
    let mut w = Writer::new();
    let tracks = 1 + u32::from(c.audio.is_some());
    w.boxed(b"moov", |w| {
        w.full_box(b"mvhd", 0, 0, |w| {
            w.u32(0);
            w.u32(0);
            w.u32(1000);
            w.u32(0);
            w.u32(0x0001_0000);
            w.u16(0x0100);
            w.zeros(10);
            w.matrix();
            w.zeros(24);
            w.u32(tracks + 1);
        });
        track(w, 1, c, false);
        if c.audio.is_some() {
            track(w, 2, c, true);
        }
        w.boxed(b"mvex", |w| {
            for id in 1..=tracks {
                w.full_box(b"trex", 0, 0, |w| {
                    w.u32(id);
                    w.u32(1);
                    w.zeros(12);
                });
            }
        });
    });
    w.out
}

/// Smooths arrival times into frame times: each picture advances by one frame period plus a sixteenth of how far the arrival is from
/// that, so bursts and gaps in delivery do not show as uneven frames while the clip's length still follows the clock. A stall of more than
/// four periods restarts the line at the arrival.
struct FrameClock {
    period_ns: Option<f64>,
    last_ns: Option<i64>,
}

impl FrameClock {
    fn next(&mut self, arrival_ns: i64) -> i64 {
        let t = match (self.last_ns, self.period_ns) {
            (None, _) => 0,
            (Some(last), Some(p)) => {
                let expected = last as f64 + p;
                let err = arrival_ns as f64 - expected;
                if err.abs() > 4.0 * p { arrival_ns } else { (expected + (err / 16.0).clamp(-p / 2.0, p / 2.0)).round() as i64 }
            }
            (Some(last), None) => arrival_ns.max(last + 1_000_000),
        };
        self.last_ns = Some(t);
        t
    }
}

fn ticks(ns: i64, scale: u32) -> u64 {
    ((i128::from(ns.max(0)) * i128::from(scale) + 500_000_000) / 1_000_000_000) as u64
}

pub struct Muxer<W: Write> {
    out: W,
    config: Arc<Config>,
    t0: Option<i64>,
    clock: FrameClock,
    /// The last video sample waits for the next one to learn its duration.
    held: Option<Sample>,
    last_video_duration: u32,
    video: Vec<Sample>,
    audio: Vec<Sample>,
    audio_dts: Option<u64>,
    pending_bytes: usize,
    sequence: u32,
    pub fragments: u32,
    pub video_samples: u64,
    pub audio_samples: u64,
    pub bytes: u64,
}

impl<W: Write> Muxer<W> {
    /// Writes the header; the first unit pushed must be the keyframe the clip starts on.
    pub fn new(mut out: W, config: Arc<Config>) -> io::Result<Self> {
        let header = [ftyp(), moov(&config)].concat();
        out.write_all(&header)?;
        let period_ns = (config.fps > 0).then(|| 1e9 / f64::from(config.fps));
        Ok(Self {
            out,
            config,
            t0: None,
            clock: FrameClock { period_ns, last_ns: None },
            held: None,
            last_video_duration: 0,
            video: Vec::new(),
            audio: Vec::new(),
            audio_dts: None,
            pending_bytes: 0,
            sequence: 0,
            fragments: 0,
            video_samples: 0,
            audio_samples: 0,
            bytes: header.len() as u64,
        })
    }

    pub fn config(&self) -> &Arc<Config> {
        &self.config
    }

    pub fn push(&mut self, unit: &Unit) -> io::Result<()> {
        match unit.track {
            Track::Video => self.push_video(unit),
            Track::Audio => self.push_audio(unit),
        }
    }

    fn push_video(&mut self, unit: &Unit) -> io::Result<()> {
        let t0 = match self.t0 {
            Some(t) => t,
            None if unit.key => *self.t0.insert(unit.pts_ns),
            None => return Ok(()),
        };
        let data = &unit.data;
        let (ranges, length_prefixed) = match self.config.codec {
            Codec::Av1 => (bitstream::av1_sample(data).into_iter().map(|(s, e)| (s as u32, e as u32)).collect::<Vec<_>>(), false),
            codec => (
                bitstream::nal_units(data).into_iter().filter(|&(s, e)| !bitstream::nal_is_out_of_band(codec, &data[s..e])).map(|(s, e)| (s as u32, e as u32)).collect(),
                true,
            ),
        };
        if ranges.is_empty() {
            return Ok(());
        }
        let dts = ticks(self.clock.next(unit.pts_ns - t0), VIDEO_TIMESCALE);
        let sample = Sample { dts, duration: 0, flags: if unit.key { SYNC } else { NON_SYNC }, data: unit.data.clone(), ranges, length_prefixed };
        if let Some(mut prev) = self.held.replace(sample) {
            prev.duration = u32::try_from(dts.saturating_sub(prev.dts)).unwrap_or(u32::MAX).max(1);
            self.last_video_duration = prev.duration;
            self.pending_bytes += prev.data.len();
            self.video.push(prev);
        }
        let span = self.video.last().map_or(0, |s| s.dts + u64::from(s.duration)) - self.video.first().map_or(0, |s| s.dts);
        // A keyframe opens a fragment, so the receiver can start a clip or a recording on a fragment's first sample.
        if !self.video.is_empty() && (unit.key || span >= ticks(FRAGMENT_NS, VIDEO_TIMESCALE) || self.pending_bytes >= FRAGMENT_BYTES) {
            self.flush()?;
        }
        Ok(())
    }

    fn push_audio(&mut self, unit: &Unit) -> io::Result<()> {
        let Some(t0) = self.t0 else { return Ok(()) };
        if self.config.audio.is_none() || unit.pts_ns < t0 - 10_000_000 {
            return Ok(());
        }
        let Some(samples) = bitstream::opus_samples(&unit.data) else { return Ok(()) };
        let dts = *self.audio_dts.get_or_insert_with(|| ticks(unit.pts_ns - t0, AUDIO_TIMESCALE));
        self.audio_dts = Some(dts + u64::from(samples));
        self.pending_bytes += unit.data.len();
        self.audio.push(Sample { dts, duration: samples, flags: SYNC, data: unit.data.clone(), ranges: vec![(0, unit.data.len() as u32)], length_prefixed: false });
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        let video = std::mem::take(&mut self.video);
        let audio = std::mem::take(&mut self.audio);
        self.pending_bytes = 0;
        if video.is_empty() && audio.is_empty() {
            return Ok(());
        }
        self.sequence += 1;
        let trafs: Vec<(u32, u64, &[Sample])> = [(1, &video[..]), (2, &audio[..])].into_iter().filter(|(_, s)| !s.is_empty()).map(|(id, s)| (id, s[0].dts, s)).collect();
        let sizes: Vec<u32> = trafs.iter().map(|(_, _, s)| s.iter().map(Sample::size).sum()).collect();
        let moof_len = moof(self.sequence, &trafs, &vec![0; trafs.len()]).len() as u32;
        let mut offsets = Vec::with_capacity(trafs.len());
        let mut at = moof_len + 8;
        for size in &sizes {
            offsets.push(at);
            at += size;
        }
        let moof = moof(self.sequence, &trafs, &offsets);
        let payload: u64 = sizes.iter().map(|&s| u64::from(s)).sum();
        self.out.write_all(&moof)?;
        let mut head = Vec::with_capacity(16);
        if payload + 8 > u64::from(u32::MAX) {
            head.extend_from_slice(&1u32.to_be_bytes());
            head.extend_from_slice(b"mdat");
            head.extend_from_slice(&(payload + 16).to_be_bytes());
        } else {
            head.extend_from_slice(&((payload + 8) as u32).to_be_bytes());
            head.extend_from_slice(b"mdat");
        }
        self.out.write_all(&head)?;
        for s in video.iter().chain(&audio) {
            for &(a, b) in &s.ranges {
                if s.length_prefixed {
                    self.out.write_all(&(b - a).to_be_bytes())?;
                }
                self.out.write_all(&s.data[a as usize..b as usize])?;
            }
        }
        self.bytes += (moof.len() + head.len()) as u64 + payload;
        self.fragments += 1;
        self.video_samples += video.len() as u64;
        self.audio_samples += audio.len() as u64;
        Ok(())
    }

    /// Closes the last fragment, with the last picture lasting as long as the one before it; the counters are final afterwards.
    pub fn close(&mut self) -> io::Result<()> {
        if let Some(mut last) = self.held.take() {
            last.duration = self.last_video_duration.max(1);
            self.video.push(last);
        }
        self.flush()?;
        self.out.flush()
    }

    pub fn finish(mut self) -> io::Result<W> {
        self.close()?;
        Ok(self.out)
    }
}

fn moof(sequence: u32, trafs: &[(u32, u64, &[Sample])], offsets: &[u32]) -> Vec<u8> {
    let mut w = Writer::new();
    w.boxed(b"moof", |w| {
        w.full_box(b"mfhd", 0, 0, |w| w.u32(sequence));
        for (i, (id, base, samples)) in trafs.iter().enumerate() {
            w.boxed(b"traf", |w| {
                w.full_box(b"tfhd", 0, 0x02_0000, |w| w.u32(*id));
                w.full_box(b"tfdt", 1, 0, |w| w.u64(*base));
                w.full_box(b"trun", 0, 0x0701, |w| {
                    w.u32(samples.len() as u32);
                    w.u32(offsets[i]);
                    for s in samples.iter() {
                        w.u32(s.duration);
                        w.u32(s.size());
                        w.u32(s.flags);
                    }
                });
            });
        }
    });
    w.out
}

#[cfg(test)]
pub(crate) mod parse {
    //! A reader for the boxes the writer makes, for tests that check the output without a player.
    #[derive(Debug)]
    pub struct BoxRef<'a> {
        pub ty: [u8; 4],
        pub start: usize,
        pub body: &'a [u8],
    }

    pub fn boxes(data: &[u8], base: usize) -> Vec<BoxRef<'_>> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < data.len() {
            assert!(i + 8 <= data.len(), "a box header runs past the end");
            let size = u32::from_be_bytes(data[i..i + 4].try_into().unwrap()) as usize;
            assert!(size >= 8 && i + size <= data.len(), "box at {} has size {size}", base + i);
            out.push(BoxRef { ty: data[i + 4..i + 8].try_into().unwrap(), start: base + i, body: &data[i + 8..i + size] });
            i += size;
        }
        out
    }

    pub fn child<'a>(b: &BoxRef<'a>, ty: &[u8; 4], skip: usize) -> Option<BoxRef<'a>> {
        boxes(&b.body[skip..], b.start + 8 + skip).into_iter().find(|c| &c.ty == ty)
    }
}

#[cfg(test)]
mod tests {
    use super::parse::*;
    use super::*;
    use crate::tests::{h264_config, h264_key, h264_delta, opus_unit};

    fn unit(seq: u64, track: Track, pts_ms: i64, key: bool, data: Vec<u8>) -> Unit {
        Unit { seq, track, pts_ns: pts_ms * 1_000_000, key, data: data.into() }
    }

    /// 45 pictures at 30 fps (a keyframe first) and 20 ms Opus packets, muxed to bytes.
    fn mux_h264() -> Vec<u8> {
        let mut m = Muxer::new(Vec::new(), h264_config()).unwrap();
        let mut seq = 0;
        for i in 0..45 {
            m.push(&unit(seq, Track::Video, 1000 + i * 33, i == 0, if i == 0 { h264_key() } else { h264_delta(i as u8) })).unwrap();
            seq += 1;
            for k in 0..2 {
                m.push(&unit(seq, Track::Audio, 1000 + i * 33 + k * 16, false, opus_unit())).unwrap();
                seq += 1;
            }
        }
        m.finish().unwrap()
    }

    #[test]
    fn the_output_is_ftyp_moov_then_whole_moof_mdat_pairs_whose_offsets_land_in_the_mdat() {
        let file = mux_h264();
        let top = boxes(&file, 0);
        let names: Vec<_> = top.iter().map(|b| std::str::from_utf8(&b.ty).unwrap()).collect();
        assert_eq!(&names[..2], ["ftyp", "moov"]);
        assert!(names[2..].chunks(2).all(|c| c == ["moof", "mdat"]), "{names:?}");
        let moov = &top[1];
        let traks: Vec<_> = boxes(moov.body, 0).into_iter().filter(|b| &b.ty == b"trak").collect();
        assert_eq!(traks.len(), 2);
        // 45 pictures at 33 ms are 1.5 s.
        let (mut video, mut audio, mut seen_audio_dts) = (0, 0, 0u64);
        for pair in top[2..].chunks(2) {
            let (moof, mdat) = (&pair[0], &pair[1]);
            for traf in boxes(moof.body, 0).into_iter().filter(|b| &b.ty == b"traf") {
                let tfhd = child(&traf, b"tfhd", 0).unwrap();
                let id = u32::from_be_bytes(tfhd.body[4..8].try_into().unwrap());
                let tfdt = child(&traf, b"tfdt", 0).unwrap();
                let base = u64::from_be_bytes(tfdt.body[4..12].try_into().unwrap());
                let trun = child(&traf, b"trun", 0).unwrap();
                let count = u32::from_be_bytes(trun.body[4..8].try_into().unwrap()) as usize;
                let offset = i32::from_be_bytes(trun.body[8..12].try_into().unwrap()) as usize;
                let size: usize = (0..count).map(|i| u32::from_be_bytes(trun.body[16 + i * 12..20 + i * 12].try_into().unwrap()) as usize).sum();
                assert!(moof.start + offset >= mdat.start + 8 && moof.start + offset + size <= mdat.start + 8 + mdat.body.len(), "track {id} data is inside its mdat");
                if id == 1 {
                    video += count;
                    let first_flags = u32::from_be_bytes(trun.body[20..24].try_into().unwrap());
                    assert_eq!(first_flags == SYNC, base == 0, "only the first fragment starts on the keyframe here");
                } else {
                    audio += count;
                    assert!(base >= seen_audio_dts, "audio time only moves forward");
                    seen_audio_dts = base + count as u64 * 960;
                }
            }
        }
        assert_eq!((video, audio), (45, 90));
        assert_eq!(top[2..].len(), 4, "a second of video closes the first fragment, the rest the second");
    }

    #[test]
    fn every_keyframe_opens_a_fragment() {
        let mut m = Muxer::new(Vec::new(), h264_config()).unwrap();
        for i in 0..20i64 {
            let key = i % 10 == 0;
            m.push(&unit(i as u64, Track::Video, 1000 + i * 33, key, if key { h264_key() } else { h264_delta(i as u8) })).unwrap();
        }
        let file = m.finish().unwrap();
        let mut firsts = Vec::new();
        for moof in boxes(&file, 0).into_iter().filter(|b| &b.ty == b"moof") {
            for traf in boxes(moof.body, 0).into_iter().filter(|b| &b.ty == b"traf") {
                let trun = child(&traf, b"trun", 0).unwrap();
                let word = |at: usize| u32::from_be_bytes(trun.body[at..at + 4].try_into().unwrap());
                if u32::from_be_bytes(child(&traf, b"tfhd", 0).unwrap().body[4..8].try_into().unwrap()) == 1 {
                    firsts.push((word(4), word(20)));
                }
            }
        }
        assert_eq!(firsts, [(10, SYNC), (10, SYNC)], "a cut at any fragment starts on a keyframe");
    }

    #[test]
    fn nal_units_become_length_prefixed_and_parameter_sets_stay_in_the_sample_entry() {
        let file = mux_h264();
        let top = boxes(&file, 0);
        let mdat = top.iter().find(|b| &b.ty == b"mdat").unwrap();
        // The keyframe's SPS, PPS and delimiter are gone; its IDR slice is the first video sample, behind a 4-byte length.
        assert_eq!(&mdat.body[..4], &(h264_key_slice().len() as u32).to_be_bytes());
        assert_eq!(&mdat.body[4..4 + h264_key_slice().len()], &h264_key_slice()[..]);
        let moov = &top[1];
        let stsd = &file[moov.start..moov.start + 8 + moov.body.len()];
        assert!(stsd.windows(4).any(|w| w == b"avcC") && stsd.windows(4).any(|w| w == b"dOps"));
    }

    fn h264_key_slice() -> Vec<u8> {
        crate::tests::H264_IDR.to_vec()
    }

    #[test]
    fn the_frame_clock_evens_out_bursts_follows_the_rate_and_restarts_after_a_stall() {
        let mut c = FrameClock { period_ns: Some(16_666_667.0), last_ns: None };
        let mut last = c.next(0);
        let mut gaps = Vec::new();
        // Frames arrive in bursts of three every 50 ms: 60 fps on average, never evenly.
        for i in 1..120i64 {
            let t = c.next((i / 3) * 50_000_000 + (i % 3) * 1_000_000);
            gaps.push(t - last);
            last = t;
        }
        assert!(gaps.iter().all(|g| (8_000_000..=25_000_000).contains(g)), "{gaps:?}");
        let end = i64::from(39) * 50_000_000;
        assert!((last - end).abs() < 25_000_000, "the line tracks the clock: {last} vs {end}");
        assert_eq!(c.next(last + 2_000_000_000), last + 2_000_000_000, "a stall restarts the line");
    }

    #[test]
    fn a_stream_without_a_frame_rate_keeps_arrival_times_in_order() {
        let mut c = FrameClock { period_ns: None, last_ns: None };
        assert_eq!([c.next(5), c.next(5), c.next(40_000_000)], [0, 1_000_000, 40_000_000]);
    }

    #[test]
    fn audio_before_the_keyframe_and_non_key_starts_are_left_out() {
        let mut m = Muxer::new(Vec::new(), h264_config()).unwrap();
        m.push(&unit(0, Track::Video, 0, false, h264_delta(1))).unwrap();
        m.push(&unit(1, Track::Audio, 0, false, opus_unit())).unwrap();
        m.push(&unit(2, Track::Video, 1000, true, h264_key())).unwrap();
        m.push(&unit(3, Track::Audio, 900, false, opus_unit())).unwrap();
        m.push(&unit(4, Track::Audio, 1005, false, opus_unit())).unwrap();
        let file = m.finish().unwrap();
        let top = boxes(&file, 0);
        let moof = top.iter().find(|b| &b.ty == b"moof").unwrap();
        let counts: Vec<u32> = boxes(moof.body, 0)
            .into_iter()
            .filter(|b| &b.ty == b"traf")
            .map(|t| u32::from_be_bytes(child(&t, b"trun", 0).unwrap().body[4..8].try_into().unwrap()))
            .collect();
        assert_eq!(counts, [1, 1]);
    }
}
