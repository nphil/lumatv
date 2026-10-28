//! A fragmented MP4 writer for the encoder's H.264: one video track, an access unit in Annex B in, `moof`/`mdat` fragments out.
//!
//! Fragmented, because a recording on a TV can end at any moment (the power, the compositor, a full disk): every fragment is whole
//! in itself, the file is playable up to the last one written, and nothing is rewritten at the end. A fragment closes at each keyframe
//! (the encoder's GOP is about a second) or when it holds two seconds. Timestamps are the compositor's own clock, so a recording of
//! a screen that did not change for ten seconds says ten seconds there, not a frame.
//!
//! The encoder's stream has SPS and PPS in front of every keyframe; they go into the track's `avcC` from the first one and out of the
//! samples (which are length-prefixed as MP4 wants them). Samples before the first keyframe are dropped: nothing can show them.

use std::io::{self, Write};

/// Ticks a second has in the track's clock.
pub const TIMESCALE: u64 = 90_000;
/// The longest a fragment stays open without a keyframe.
const FRAGMENT_TICKS: u64 = 2 * TIMESCALE;
/// The duration given to a last sample whose real one is unknown (the stream ended on it): one frame at 30 fps.
const ONE_FRAME: u64 = TIMESCALE / 30;

struct Sample {
    /// Length-prefixed NAL units.
    data: Vec<u8>,
    ticks: u64,
    key: bool,
}

pub struct Mp4Writer<W: Write> {
    out: W,
    width: u16,
    height: u16,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    started: bool,
    base_us: u64,
    sequence: u32,
    /// Where the track's clock stands at the end of what has been written: the next fragment's decode time.
    written_ticks: u64,
    pending: Vec<Sample>,
    bytes: u64,
    samples: u64,
}

/// The NAL units of an Annex B access unit, without their start codes.
pub fn nal_units(annexb: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= annexb.len() {
        if annexb[i] == 0 && annexb[i + 1] == 0 && annexb[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut units = Vec::with_capacity(starts.len());
    for (n, &s) in starts.iter().enumerate() {
        let mut end = starts.get(n + 1).map_or(annexb.len(), |&next| next - 3);
        // A four-byte start code leaves a zero at the end of the unit before it, and so do the zero bytes the encoder pads with.
        while end > s && annexb[end - 1] == 0 {
            end -= 1;
        }
        if end > s {
            units.push(&annexb[s..end]);
        }
    }
    units
}

impl<W: Write> Mp4Writer<W> {
    pub fn new(out: W, width: u32, height: u32) -> Self {
        Mp4Writer {
            out,
            width: width as u16,
            height: height as u16,
            sps: None,
            pps: None,
            started: false,
            base_us: 0,
            sequence: 0,
            written_ticks: 0,
            pending: Vec::new(),
            bytes: 0,
            samples: 0,
        }
    }

    /// Bytes written so far plus what the open fragment will take: what the file will hold once it is flushed.
    pub fn size_so_far(&self) -> u64 {
        self.bytes + self.pending.iter().map(|s| s.data.len() as u64 + 16).sum::<u64>()
    }

    pub fn samples(&self) -> u64 {
        self.samples + self.pending.len() as u64
    }

    /// Adds one access unit taken at `pts_us` (the recording's clock, microseconds).
    pub fn push(&mut self, annexb: &[u8], pts_us: u64) -> io::Result<()> {
        let mut data = Vec::with_capacity(annexb.len() + 8);
        let mut key = false;
        for nal in nal_units(annexb) {
            match nal[0] & 0x1f {
                7 => {
                    if self.sps.is_none() {
                        self.sps = Some(nal.to_vec());
                    }
                }
                8 => {
                    if self.pps.is_none() {
                        self.pps = Some(nal.to_vec());
                    }
                }
                9 => {}
                t => {
                    key |= t == 5;
                    data.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                    data.extend_from_slice(nal);
                }
            }
        }
        if data.is_empty() {
            return Ok(());
        }
        if !self.started {
            if !key || self.sps.is_none() || self.pps.is_none() {
                return Ok(());
            }
            self.base_us = pts_us;
            self.write_init()?;
            self.started = true;
        }
        let ticks = (pts_us.saturating_sub(self.base_us) as u128 * TIMESCALE as u128 / 1_000_000) as u64;
        let last = self.pending.last().map_or(0, |s| s.ticks);
        // A clock that did not move (or went back): one tick on, so decode times stay strictly increasing.
        let ticks = if self.pending.is_empty() { ticks.max(self.written_ticks) } else { ticks.max(last + 1) };
        if !self.pending.is_empty() && (key || ticks - self.pending[0].ticks >= FRAGMENT_TICKS) {
            self.flush(ticks)?;
        }
        self.pending.push(Sample { data, ticks, key });
        Ok(())
    }

    /// Closes the open fragment with the last sample lasting until `end_us`, and flushes the output. Returns the file's size.
    pub fn finish(&mut self, end_us: u64) -> io::Result<u64> {
        if let Some(last) = self.pending.last() {
            let end = (end_us.saturating_sub(self.base_us) as u128 * TIMESCALE as u128 / 1_000_000) as u64;
            let end = if end > last.ticks { end } else { last.ticks + ONE_FRAME };
            self.flush(end)?;
        }
        self.out.flush()?;
        Ok(self.bytes)
    }

    /// The output, once the file is finished (to make it durable before it is named).
    pub fn into_inner(self) -> W {
        self.out
    }

    /// Writes the open fragment; its last sample ends at `next_ticks`.
    fn flush(&mut self, next_ticks: u64) -> io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let samples = std::mem::take(&mut self.pending);
        self.sequence += 1;
        let payload: usize = samples.iter().map(|s| s.data.len()).sum();
        // moof: mfhd (16) + traf { tfhd (16) + tfdt (20) + trun (20 + 12 per sample) }
        let trun_len = 20 + 12 * samples.len();
        let moof_len = 8 + 16 + 8 + 16 + 20 + trun_len;
        let mut b = Vec::with_capacity(moof_len + 8 + payload);
        b.extend_from_slice(&(moof_len as u32).to_be_bytes());
        b.extend_from_slice(b"moof");
        full_box(&mut b, b"mfhd", 0, 0, &self.sequence.to_be_bytes());
        b.extend_from_slice(&((8 + 16 + 20 + trun_len) as u32).to_be_bytes());
        b.extend_from_slice(b"traf");
        // default-base-is-moof: the data offset below counts from the start of this moof
        full_box(&mut b, b"tfhd", 0, 0x02_0000, &1u32.to_be_bytes());
        full_box(&mut b, b"tfdt", 1, 0, &samples[0].ticks.to_be_bytes());
        b.extend_from_slice(&(trun_len as u32).to_be_bytes());
        b.extend_from_slice(b"trun");
        // version 0, flags: data offset, sample duration, sample size, sample flags
        b.extend_from_slice(&0x0000_0701u32.to_be_bytes());
        b.extend_from_slice(&(samples.len() as u32).to_be_bytes());
        b.extend_from_slice(&((moof_len + 8) as u32).to_be_bytes());
        for (i, s) in samples.iter().enumerate() {
            let next = samples.get(i + 1).map_or(next_ticks, |n| n.ticks);
            b.extend_from_slice(&((next.saturating_sub(s.ticks)).max(1) as u32).to_be_bytes());
            b.extend_from_slice(&(s.data.len() as u32).to_be_bytes());
            // sync sample: depends on nothing; otherwise depends on others and is a non-sync sample
            b.extend_from_slice(&(if s.key { 0x0200_0000u32 } else { 0x0101_0000u32 }).to_be_bytes());
        }
        debug_assert_eq!(b.len(), moof_len);
        b.extend_from_slice(&((8 + payload) as u32).to_be_bytes());
        b.extend_from_slice(b"mdat");
        for s in &samples {
            b.extend_from_slice(&s.data);
        }
        self.out.write_all(&b)?;
        self.bytes += b.len() as u64;
        self.samples += samples.len() as u64;
        self.written_ticks = next_ticks;
        Ok(())
    }

    fn write_init(&mut self) -> io::Result<()> {
        let (sps, pps) = (self.sps.clone().unwrap_or_default(), self.pps.clone().unwrap_or_default());
        let mut b = Vec::new();
        let mut ftyp = Vec::new();
        ftyp.extend_from_slice(b"isom");
        ftyp.extend_from_slice(&0x200u32.to_be_bytes());
        for brand in [b"isom", b"iso6", b"avc1", b"mp41"] {
            ftyp.extend_from_slice(brand);
        }
        plain_box(&mut b, b"ftyp", &ftyp);

        let mut moov = Vec::new();
        // mvhd (version 0): times, timescale 1000, duration unknown (0), rate 1.0, volume 1.0, the identity matrix, next track id 2
        let mut mvhd = Vec::new();
        mvhd.extend_from_slice(&[0; 8]);
        mvhd.extend_from_slice(&1000u32.to_be_bytes());
        mvhd.extend_from_slice(&0u32.to_be_bytes());
        mvhd.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        mvhd.extend_from_slice(&0x0100u16.to_be_bytes());
        mvhd.extend_from_slice(&[0; 10]);
        mvhd.extend_from_slice(&MATRIX);
        mvhd.extend_from_slice(&[0; 24]);
        mvhd.extend_from_slice(&2u32.to_be_bytes());
        full_box(&mut moov, b"mvhd", 0, 0, &mvhd);

        let mut trak = Vec::new();
        let mut tkhd = Vec::new();
        tkhd.extend_from_slice(&[0; 8]);
        tkhd.extend_from_slice(&1u32.to_be_bytes());
        tkhd.extend_from_slice(&[0; 4]);
        tkhd.extend_from_slice(&0u32.to_be_bytes());
        tkhd.extend_from_slice(&[0; 8]);
        tkhd.extend_from_slice(&[0; 4]);
        tkhd.extend_from_slice(&[0; 4]);
        tkhd.extend_from_slice(&MATRIX);
        tkhd.extend_from_slice(&((self.width as u32) << 16).to_be_bytes());
        tkhd.extend_from_slice(&((self.height as u32) << 16).to_be_bytes());
        // enabled, in the movie
        full_box(&mut trak, b"tkhd", 0, 3, &tkhd);

        let mut mdia = Vec::new();
        let mut mdhd = Vec::new();
        mdhd.extend_from_slice(&[0; 8]);
        mdhd.extend_from_slice(&(TIMESCALE as u32).to_be_bytes());
        mdhd.extend_from_slice(&0u32.to_be_bytes());
        mdhd.extend_from_slice(&0x55C4u16.to_be_bytes());
        mdhd.extend_from_slice(&0u16.to_be_bytes());
        full_box(&mut mdia, b"mdhd", 0, 0, &mdhd);
        let mut hdlr = Vec::new();
        hdlr.extend_from_slice(&[0; 4]);
        hdlr.extend_from_slice(b"vide");
        hdlr.extend_from_slice(&[0; 12]);
        hdlr.extend_from_slice(b"LumaTV\0");
        full_box(&mut mdia, b"hdlr", 0, 0, &hdlr);

        let mut minf = Vec::new();
        full_box(&mut minf, b"vmhd", 0, 1, &[0; 8]);
        let mut dref = Vec::new();
        dref.extend_from_slice(&1u32.to_be_bytes());
        full_box(&mut dref, b"url ", 0, 1, &[]);
        let mut dinf = Vec::new();
        full_box(&mut dinf, b"dref", 0, 0, &dref);
        plain_box(&mut minf, b"dinf", &dinf);

        let mut avc1 = Vec::new();
        avc1.extend_from_slice(&[0; 6]);
        avc1.extend_from_slice(&1u16.to_be_bytes());
        avc1.extend_from_slice(&[0; 16]);
        avc1.extend_from_slice(&self.width.to_be_bytes());
        avc1.extend_from_slice(&self.height.to_be_bytes());
        avc1.extend_from_slice(&0x0048_0000u32.to_be_bytes());
        avc1.extend_from_slice(&0x0048_0000u32.to_be_bytes());
        avc1.extend_from_slice(&[0; 4]);
        avc1.extend_from_slice(&1u16.to_be_bytes());
        avc1.extend_from_slice(&[0; 32]);
        avc1.extend_from_slice(&0x0018u16.to_be_bytes());
        avc1.extend_from_slice(&0xFFFFu16.to_be_bytes());
        let mut avcc = vec![1, sps.get(1).copied().unwrap_or(100), sps.get(2).copied().unwrap_or(0), sps.get(3).copied().unwrap_or(40), 0xFF, 0xE1];
        avcc.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        avcc.extend_from_slice(&sps);
        avcc.push(1);
        avcc.extend_from_slice(&(pps.len() as u16).to_be_bytes());
        avcc.extend_from_slice(&pps);
        // The High profile's extension: 4:2:0, 8 bits, no extended SPS. The encoder makes nothing else.
        if matches!(sps.get(1), Some(100 | 110 | 122 | 144)) {
            avcc.extend_from_slice(&[0xFD, 0xF8, 0xF8, 0]);
        }
        plain_box(&mut avc1, b"avcC", &avcc);
        let mut stsd = Vec::new();
        stsd.extend_from_slice(&1u32.to_be_bytes());
        plain_box(&mut stsd, b"avc1", &avc1);
        let mut stbl = Vec::new();
        full_box(&mut stbl, b"stsd", 0, 0, &stsd);
        full_box(&mut stbl, b"stts", 0, 0, &0u32.to_be_bytes());
        full_box(&mut stbl, b"stsc", 0, 0, &0u32.to_be_bytes());
        full_box(&mut stbl, b"stsz", 0, 0, &[0; 8]);
        full_box(&mut stbl, b"stco", 0, 0, &0u32.to_be_bytes());
        plain_box(&mut minf, b"stbl", &stbl);
        plain_box(&mut mdia, b"minf", &minf);
        plain_box(&mut trak, b"mdia", &mdia);
        plain_box(&mut moov, b"trak", &trak);

        let mut mvex = Vec::new();
        let mut trex = Vec::new();
        trex.extend_from_slice(&1u32.to_be_bytes());
        trex.extend_from_slice(&1u32.to_be_bytes());
        trex.extend_from_slice(&[0; 12]);
        full_box(&mut mvex, b"trex", 0, 0, &trex);
        plain_box(&mut moov, b"mvex", &mvex);
        plain_box(&mut b, b"moov", &moov);
        self.out.write_all(&b)?;
        self.bytes += b.len() as u64;
        Ok(())
    }
}

const MATRIX: [u8; 36] = [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x40, 0, 0, 0];

fn plain_box(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
}

fn full_box(out: &mut Vec<u8>, kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) {
    out.extend_from_slice(&((body.len() + 12) as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.push(version);
    out.extend_from_slice(&flags.to_be_bytes()[1..]);
    out.extend_from_slice(body);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny parser of the boxes the writer makes, enough to check the structure and read the samples back.
    #[derive(Debug)]
    struct B<'a> {
        kind: [u8; 4],
        body: &'a [u8],
        start: usize,
    }

    fn boxes(data: &[u8], base: usize) -> Vec<B<'_>> {
        let mut v = Vec::new();
        let mut i = 0;
        while i + 8 <= data.len() {
            let size = u32::from_be_bytes(data[i..i + 4].try_into().unwrap()) as usize;
            assert!(size >= 8 && i + size <= data.len(), "box at {} has size {size} in {} bytes", base + i, data.len());
            v.push(B { kind: data[i + 4..i + 8].try_into().unwrap(), body: &data[i + 8..i + size], start: base + i });
            i += size;
        }
        assert_eq!(i, data.len(), "boxes cover the data exactly");
        v
    }

    fn find<'a>(v: &'a [B<'a>], kind: &[u8; 4]) -> &'a B<'a> {
        v.iter().find(|b| &b.kind == kind).unwrap_or_else(|| panic!("no {}", String::from_utf8_lossy(kind)))
    }

    fn sps(profile: u8) -> Vec<u8> {
        vec![0, 0, 0, 1, 0x67, profile, 0, 31, 0xAC, 0xD9]
    }

    fn au(key: bool, with_params: bool, payload: usize) -> Vec<u8> {
        let mut v = Vec::new();
        if with_params {
            v.extend(sps(100));
            v.extend([0, 0, 0, 1, 0x68, 0xEB, 0xE3, 0xCB]);
        }
        v.extend([0, 0, 1, if key { 0x65 } else { 0x41 }]);
        v.extend((0..payload).map(|i| (i % 200 + 1) as u8));
        v
    }

    /// The samples of every fragment: (decode time, duration, size, key).
    fn fragments(file: &[u8]) -> Vec<(u64, u32, u32, bool)> {
        let top = boxes(file, 0);
        let mut out = Vec::new();
        let mut i = 0;
        while i < top.len() {
            if &top[i].kind == b"moof" {
                let inner = boxes(top[i].body, top[i].start + 8);
                let traf = boxes(find(&inner, b"traf").body, 0);
                let tfdt = find(&traf, b"tfdt");
                assert_eq!(tfdt.body[0], 1);
                let t0 = u64::from_be_bytes(tfdt.body[4..12].try_into().unwrap());
                let trun = find(&traf, b"trun").body;
                let count = u32::from_be_bytes(trun[4..8].try_into().unwrap()) as usize;
                let offset = i32::from_be_bytes(trun[8..12].try_into().unwrap()) as usize;
                assert_eq!(top[i].start + offset, top[i + 1].start + 8, "the data offset points at the mdat's first byte");
                assert_eq!(&top[i + 1].kind, b"mdat");
                let mut t = t0;
                let mut total = 0usize;
                for n in 0..count {
                    let e = &trun[12 + n * 12..24 + n * 12];
                    let (d, s, f) = (
                        u32::from_be_bytes(e[0..4].try_into().unwrap()),
                        u32::from_be_bytes(e[4..8].try_into().unwrap()),
                        u32::from_be_bytes(e[8..12].try_into().unwrap()),
                    );
                    out.push((t, d, s, f == 0x0200_0000));
                    t += d as u64;
                    total += s as usize;
                }
                assert_eq!(total, top[i + 1].body.len(), "the sizes add up to the mdat");
                i += 2;
            } else {
                i += 1;
            }
        }
        out
    }

    fn write(units: &[(Vec<u8>, u64)], end: u64) -> Vec<u8> {
        let mut w = Mp4Writer::new(Vec::new(), 1280, 720);
        for (a, t) in units {
            w.push(a, *t).unwrap();
        }
        let n = w.finish(end).unwrap();
        let out = w.out.clone();
        assert_eq!(n as usize, out.len());
        out
    }

    #[test]
    fn the_nal_walker_splits_three_and_four_byte_start_codes_and_drops_padding() {
        let mut v = vec![0, 0, 0, 1, 0x67, 1, 2, 0, 0, 0, 1, 0x68, 3, 0, 0, 0, 0, 0, 1, 0x65, 4, 5];
        let units = nal_units(&v);
        assert_eq!(units, vec![&[0x67, 1, 2][..], &[0x68, 3][..], &[0x65, 4, 5][..]]);
        v.clear();
        assert!(nal_units(&v).is_empty());
        assert!(nal_units(&[1, 2, 3, 4]).is_empty());
    }

    #[test]
    fn the_init_segment_has_the_track_the_sizes_and_the_parameter_sets() {
        let file = write(&[(au(true, true, 50), 1_000_000)], 1_033_333);
        let top = boxes(&file, 0);
        assert_eq!(&top[0].kind, b"ftyp");
        assert_eq!(&top[0].body[..4], b"isom");
        let moov = boxes(find(&top, b"moov").body, 0);
        assert!(moov.iter().any(|b| &b.kind == b"mvex"));
        let trak = boxes(find(&moov, b"trak").body, 0);
        let tkhd = find(&trak, b"tkhd").body;
        assert_eq!(&tkhd[tkhd.len() - 8..], &[5, 0, 0, 0, 2, 0xD0, 0, 0], "1280 x 720 as 16.16");
        let mdia = boxes(find(&trak, b"mdia").body, 0);
        let mdhd = find(&mdia, b"mdhd").body;
        assert_eq!(u32::from_be_bytes(mdhd[12..16].try_into().unwrap()), 90_000);
        let minf = boxes(find(&mdia, b"minf").body, 0);
        let stbl = boxes(find(&minf, b"stbl").body, 0);
        let stsd = find(&stbl, b"stsd").body;
        let avc1 = boxes(&stsd[8..], 0);
        let a = find(&avc1, b"avc1").body;
        assert_eq!((u16::from_be_bytes([a[24], a[25]]), u16::from_be_bytes([a[26], a[27]])), (1280, 720));
        let avc_children = boxes(&a[78..], 0);
        let avcc = find(&avc_children, b"avcC").body;
        // version 1, the SPS's profile/compatibility/level, 4-byte lengths, one SPS, one PPS, the High profile's tail
        assert_eq!(&avcc[..6], &[1, 100, 0, 31, 0xFF, 0xE1]);
        assert_eq!(&avcc[6..8], &[0, 6]);
        assert_eq!(&avcc[8..14], &[0x67, 100, 0, 31, 0xAC, 0xD9]);
        assert_eq!(&avcc[14..19], &[1, 0, 4, 0x68, 0xEB][..5]);
        assert_eq!(&avcc[avcc.len() - 4..], &[0xFD, 0xF8, 0xF8, 0]);
    }

    #[test]
    fn samples_are_length_prefixed_without_the_parameter_sets_and_timed_by_the_recordings_clock() {
        let units: Vec<(Vec<u8>, u64)> = (0..5).map(|i| (au(i == 0 || i == 3, i == 0 || i == 3, 40 + i), 5_000_000 + i as u64 * 33_333)).collect();
        let file = write(&units, 5_000_000 + 5 * 33_333);
        let frags = fragments(&file);
        assert_eq!(frags.len(), 5);
        // Decode times follow the pts from the first sample's, in 90 kHz ticks, and the durations are the gaps.
        assert_eq!(frags[0].0, 0);
        assert_eq!(frags[1].0, 2999, "33.333 ms is 2999.97 ticks, truncated");
        assert_eq!(frags[4].0 + frags[4].1 as u64, 14_999, "the last sample lasts until the recording ended (166.665 ms)");
        assert_eq!(frags.iter().map(|f| f.3).collect::<Vec<_>>(), [true, false, false, true, false]);
        // The first sample: one NAL of 1 + 40 bytes behind its 4-byte length, and neither SPS nor PPS in it.
        assert_eq!(frags[0].2, 4 + 1 + 40);
        let top = boxes(&file, 0);
        let mdat = top.iter().find(|b| &b.kind == b"mdat").unwrap();
        assert_eq!(&mdat.body[..5], &[0, 0, 0, 41, 0x65]);
    }

    #[test]
    fn a_keyframe_closes_the_fragment_and_two_seconds_without_one_do_too() {
        let mut units = vec![(au(true, true, 30), 0u64)];
        for i in 1..=5u64 {
            units.push((au(false, false, 30), i * 700_000));
        }
        units.push((au(true, true, 30), 4_000_000));
        let file = write(&units, 4_033_333);
        let top = boxes(&file, 0);
        let moofs = top.iter().filter(|b| &b.kind == b"moof").count();
        // [0, 0.7, 1.4] then 2.1 is 2.1 s after the first: a second fragment [2.1, 2.8, 3.5], then the keyframe at 4.0: a third.
        assert_eq!(moofs, 3, "{:?}", fragments(&file));
        let seq: Vec<u32> = top.iter().filter(|b| &b.kind == b"moof").map(|m| u32::from_be_bytes(boxes(m.body, 0)[0].body[4..8].try_into().unwrap())).collect();
        assert_eq!(seq, [1, 2, 3]);
    }

    #[test]
    fn nothing_is_written_until_a_keyframe_with_parameter_sets_arrives_and_a_stream_without_one_is_an_empty_file() {
        let mut w = Mp4Writer::new(Vec::new(), 640, 360);
        w.push(&au(false, false, 20), 0).unwrap();
        w.push(&au(true, false, 20), 10).unwrap();
        assert_eq!(w.size_so_far(), 0, "a P frame and a keyframe without SPS start nothing");
        w.push(&au(true, true, 20), 20).unwrap();
        assert!(w.size_so_far() > 100);
        assert_eq!(w.samples(), 1);
        let mut empty = Mp4Writer::new(Vec::new(), 640, 360);
        assert_eq!(empty.finish(1000).unwrap(), 0);
    }

    #[test]
    fn a_clock_that_stands_still_keeps_decode_times_strictly_increasing() {
        let file = write(&[(au(true, true, 10), 100), (au(false, false, 10), 100), (au(false, false, 10), 100)], 100);
        let frags = fragments(&file);
        assert_eq!(frags.iter().map(|f| f.0).collect::<Vec<_>>(), [0, 1, 2]);
        assert!(frags.iter().all(|f| f.1 >= 1));
    }

    /// The real thing: an x264 stream (320x240, 30 frames, a keyframe every 10) through the writer, then ffmpeg decodes the MP4 to the
    /// last frame without a complaint. Skipped where there is no ffmpeg.
    #[test]
    fn a_real_h264_stream_makes_an_mp4_that_ffmpeg_decodes_whole() {
        let stream: &[u8] = include_bytes!("testdata/x264-320x240-30f.h264");
        let mut aus: Vec<Vec<u8>> = Vec::new();
        let mut cur: Vec<u8> = Vec::new();
        let starts: Vec<usize> = {
            let mut s = Vec::new();
            let mut i = 0;
            while i + 3 <= stream.len() {
                if stream[i..i + 3] == [0, 0, 1] {
                    s.push(i);
                    i += 3;
                } else {
                    i += 1;
                }
            }
            s
        };
        for (n, &s) in starts.iter().enumerate() {
            let e = starts.get(n + 1).copied().unwrap_or(stream.len());
            cur.extend_from_slice(&stream[s..e]);
            if matches!(stream[s + 3] & 0x1f, 1 | 5) {
                aus.push(std::mem::take(&mut cur));
            }
        }
        assert_eq!(aus.len(), 30);
        let mut w = Mp4Writer::new(Vec::new(), 320, 240);
        for (i, a) in aus.iter().enumerate() {
            w.push(a, i as u64 * 33_333).unwrap();
        }
        w.finish(30 * 33_333).unwrap();
        let file = w.out.clone();
        assert_eq!(fragments(&file).len(), 30);
        let path = std::env::temp_dir().join(format!("lumatv-mp4-{}.mp4", std::process::id()));
        std::fs::write(&path, &file).unwrap();
        let Ok(out) = std::process::Command::new("ffmpeg").args(["-v", "error", "-i"]).arg(&path).args(["-f", "null", "-"]).output() else {
            let _ = std::fs::remove_file(&path);
            return;
        };
        let probe = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-count_frames",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_name,profile,width,height,nb_read_frames",
                "-of",
                "default=nw=1",
            ])
            .arg(&path)
            .output()
            .unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(out.status.success() && out.stderr.is_empty(), "ffmpeg: {}", String::from_utf8_lossy(&out.stderr));
        let text = String::from_utf8_lossy(&probe.stdout);
        assert!(text.contains("codec_name=h264") && text.contains("width=320") && text.contains("height=240") && text.contains("nb_read_frames=30"), "{text}");
    }
}
