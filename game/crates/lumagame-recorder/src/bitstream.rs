//! Just enough of H.264, H.265, AV1 and Opus to put coded units in MP4: where the NAL units and OBUs are, the parameter sets a decoder
//! needs up front, and how long an Opus packet is. Nothing here decodes anything.

/// The codecs a stream can carry; the feeder's numbering is mapped by the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    H264,
    H265,
    Av1,
}

/// The NAL units of an Annex B access unit as byte ranges, start codes and trailing zeros left out.
pub fn nal_units(data: &[u8]) -> Vec<(usize, usize)> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::with_capacity(starts.len());
    for (n, &s) in starts.iter().enumerate() {
        let mut e = starts.get(n + 1).map_or(data.len(), |next| next - 3);
        while e > s && data[e - 1] == 0 {
            e -= 1;
        }
        if e > s {
            out.push((s, e));
        }
    }
    out
}

fn h264_type(nal: &[u8]) -> u8 {
    nal[0] & 0x1f
}
fn h265_type(nal: &[u8]) -> u8 {
    (nal[0] >> 1) & 0x3f
}

/// Whether a NAL unit stays out of an MP4 sample: parameter sets live in the sample entry, and access unit delimiters mean nothing there.
pub fn nal_is_out_of_band(codec: Codec, nal: &[u8]) -> bool {
    match codec {
        Codec::H264 => matches!(h264_type(nal), 7..=9),
        Codec::H265 => matches!(h265_type(nal), 32..=35),
        Codec::Av1 => false,
    }
}

/// The parameter sets a decoder needs before the first picture, as the sample entry's configuration record holds them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodecRecord {
    /// `avcC`, `hvcC` or `av1C` payload.
    pub record: Vec<u8>,
}

struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
}
impl<'a> Bits<'a> {
    fn new(d: &'a [u8]) -> Self {
        Self { d, pos: 0 }
    }
    fn bit(&mut self) -> Option<u32> {
        let byte = *self.d.get(self.pos / 8)?;
        let b = (byte >> (7 - self.pos % 8)) & 1;
        self.pos += 1;
        Some(u32::from(b))
    }
    fn u(&mut self, n: u32) -> Option<u32> {
        let mut v = 0u64;
        for _ in 0..n {
            v = (v << 1) | u64::from(self.bit()?);
        }
        u32::try_from(v).ok()
    }
    fn skip(&mut self, n: usize) -> Option<()> {
        self.pos += n;
        (self.pos <= self.d.len() * 8).then_some(())
    }
    /// Exp-Golomb, as H.264 and H.265 write `ue(v)`; AV1's `uvlc()` is the same code.
    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        Some(((1u64 << zeros) - 1 + u64::from(self.u(zeros)?)) as u32)
    }
}

/// Removes the emulation prevention bytes (`00 00 03`) of a NAL unit's payload.
fn unescape(nal: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(nal.len());
    let mut zeros = 0;
    for &b in nal {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}

/// The `avcC` record from a keyframe's SPS and PPS, or `None` when it lacks either.
pub fn h264_record(au: &[u8]) -> Option<CodecRecord> {
    let nals = nal_units(au);
    let find = |t: u8| nals.iter().map(|&(s, e)| &au[s..e]).find(|n| h264_type(n) == t);
    let (sps, pps) = (find(7)?, find(8)?);
    if sps.len() < 4 {
        return None;
    }
    let mut r = vec![1, sps[1], sps[2], sps[3], 0xff, 0xe1];
    r.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    r.extend_from_slice(sps);
    r.push(1);
    r.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    r.extend_from_slice(pps);
    if matches!(sps[1], 100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135) {
        let rbsp = unescape(&sps[4..]);
        let mut b = Bits::new(&rbsp);
        let ext = (|| {
            b.ue()?; // seq_parameter_set_id
            let chroma = b.ue()?;
            if chroma == 3 {
                b.bit()?;
            }
            Some((chroma, b.ue()?, b.ue()?))
        })();
        let (chroma, luma, chroma_depth) = ext?;
        r.extend_from_slice(&[0xfc | chroma as u8, 0xf8 | luma as u8, 0xf8 | chroma_depth as u8, 0]);
    }
    Some(CodecRecord { record: r })
}

/// The `hvcC` record from a keyframe's VPS, SPS and PPS, or `None` when it lacks one or the SPS cannot be read.
pub fn h265_record(au: &[u8]) -> Option<CodecRecord> {
    let nals = nal_units(au);
    let find = |t: u8| nals.iter().map(|&(s, e)| &au[s..e]).find(|n| n.len() > 2 && h265_type(n) == t);
    let (vps, sps, pps) = (find(32)?, find(33)?, find(34)?);
    let rbsp = unescape(&sps[2..]);
    let mut b = Bits::new(&rbsp);
    b.u(4)?; // vps id
    let sub_layers = b.u(3)?;
    let nesting = b.u(1)?;
    let ptl = rbsp.get(1..13)?.to_vec(); // profile space/tier/idc, 32 compatibility flags, 48 constraint bits, level
    b.skip(96)?;
    let (mut profile_present, mut level_present) = ([false; 8], [false; 8]);
    for i in 0..sub_layers as usize {
        profile_present[i] = b.bit()? == 1;
        level_present[i] = b.bit()? == 1;
    }
    if sub_layers > 0 {
        b.skip(2 * (8 - sub_layers as usize))?;
    }
    for i in 0..sub_layers as usize {
        if profile_present[i] {
            b.skip(88)?;
        }
        if level_present[i] {
            b.skip(8)?;
        }
    }
    b.ue()?; // sps id
    let chroma = b.ue()?;
    if chroma == 3 {
        b.bit()?;
    }
    b.ue()?;
    b.ue()?; // width, height
    if b.bit()? == 1 {
        for _ in 0..4 {
            b.ue()?;
        }
    }
    let (luma, chroma_depth) = (b.ue()?, b.ue()?);
    let mut r = vec![1];
    r.extend_from_slice(&ptl);
    r.extend_from_slice(&[0xf0, 0x00, 0xfc, 0xfc | chroma as u8, 0xf8 | luma as u8, 0xf8 | chroma_depth as u8, 0, 0]);
    r.push(((sub_layers + 1).min(7) << 3) as u8 | (nesting << 2) as u8 | 3);
    r.push(3);
    for (t, nal) in [(32u8, vps), (33, sps), (34, pps)] {
        r.push(0x80 | t);
        r.extend_from_slice(&1u16.to_be_bytes());
        r.extend_from_slice(&(nal.len() as u16).to_be_bytes());
        r.extend_from_slice(nal);
    }
    Some(CodecRecord { record: r })
}

/// One OBU of a temporal unit: its type and its whole bytes (header, size field, payload).
pub(crate) fn obus(tu: &[u8]) -> Vec<(u8, &[u8], usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < tu.len() {
        let h = tu[i];
        let (ty, ext, sized) = ((h >> 3) & 15, (h >> 2) & 1, (h >> 1) & 1);
        let mut p = i + 1 + usize::from(ext);
        let len = if sized == 1 {
            let (mut v, mut shift) = (0usize, 0);
            loop {
                let Some(&b) = tu.get(p) else { return out };
                p += 1;
                v |= usize::from(b & 0x7f) << shift;
                shift += 7;
                if b & 0x80 == 0 || shift > 28 {
                    break;
                }
            }
            v
        } else {
            tu.len().saturating_sub(p)
        };
        let end = p + len;
        if end > tu.len() {
            return out;
        }
        out.push((ty, &tu[i..end], p - i));
        i = end;
    }
    out
}

/// The byte ranges of a temporal unit's OBUs without its temporal delimiters, which an MP4 sample leaves out.
pub fn av1_sample(tu: &[u8]) -> Vec<(usize, usize)> {
    let base = tu.as_ptr() as usize;
    obus(tu).into_iter().filter(|(ty, _, _)| *ty != 2).map(|(_, o, _)| (o.as_ptr() as usize - base, o.as_ptr() as usize - base + o.len())).collect()
}

/// The `av1C` record from a keyframe's sequence header, or `None` when it has none or it cannot be read.
pub fn av1_record(tu: &[u8]) -> Option<CodecRecord> {
    let (_, obu, header) = obus(tu).into_iter().find(|(ty, _, _)| *ty == 1)?;
    let mut b = Bits::new(&obu[header..]);
    let profile = b.u(3)?;
    b.bit()?; // still picture
    let reduced = b.bit()? == 1;
    let (mut level, mut tier) = (0, 0);
    if reduced {
        level = b.u(5)?;
    } else {
        let timing = b.bit()? == 1;
        let (mut model, mut delay_bits) = (false, 0);
        if timing {
            b.skip(64)?;
            if b.bit()? == 1 {
                b.ue()?;
            }
            model = b.bit()? == 1;
            if model {
                delay_bits = b.u(5)? + 1;
                b.skip(32)?;
                b.skip(10)?;
            }
        }
        let initial_delay = b.bit()? == 1;
        let ops = b.u(5)? + 1;
        for i in 0..ops {
            b.skip(12)?;
            let l = b.u(5)?;
            let t = if l > 7 { b.bit()? } else { 0 };
            if i == 0 {
                (level, tier) = (l, t);
            }
            if model && b.bit()? == 1 {
                b.skip(2 * delay_bits as usize + 1)?;
            }
            if initial_delay && b.bit()? == 1 {
                b.skip(4)?;
            }
        }
    }
    let (wb, hb) = (b.u(4)? + 1, b.u(4)? + 1);
    b.skip((wb + hb) as usize)?;
    if !reduced && b.bit()? == 1 {
        b.skip(7)?;
    }
    b.skip(3)?; // 128x128 superblock, filter intra, intra edge
    if !reduced {
        b.skip(4)?;
        let order_hint = b.bit()? == 1;
        if order_hint {
            b.skip(2)?;
        }
        let force_sct = if b.bit()? == 1 { 2 } else { b.u(1)? };
        if force_sct > 0 && b.bit()? == 0 {
            b.bit()?;
        }
        if order_hint {
            b.skip(3)?;
        }
    }
    b.skip(3)?; // superres, cdef, restoration
    let high = b.bit()?;
    let twelve = if profile == 2 && high == 1 { b.bit()? } else { 0 };
    let mono = if profile == 1 { 0 } else { b.bit()? };
    let (mut cp, mut tc, mut mc) = (2, 2, 2);
    if b.bit()? == 1 {
        (cp, tc, mc) = (b.u(8)?, b.u(8)?, b.u(8)?);
    }
    let (mut sx, mut sy, mut pos) = (1, 1, 0);
    if mono == 1 {
        b.bit()?;
    } else if (cp, tc, mc) == (1, 13, 0) {
        (sx, sy) = (0, 0);
    } else {
        b.bit()?;
        match profile {
            0 => {}
            1 => (sx, sy) = (0, 0),
            _ => {
                if high == 1 && twelve == 1 {
                    sx = b.bit()?;
                    sy = if sx == 1 { b.bit()? } else { 0 };
                } else {
                    (sx, sy) = (1, 0);
                }
            }
        }
        if sx == 1 && sy == 1 {
            pos = b.u(2)?;
        }
    }
    let mut r = vec![
        0x81,
        ((profile << 5) | level) as u8,
        ((tier << 7) | (high << 6) | (twelve << 5) | (mono << 4) | (sx << 3) | (sy << 2) | pos) as u8,
        0,
    ];
    r.extend_from_slice(obu);
    Some(CodecRecord { record: r })
}

/// The decoder record a keyframe carries for `codec`.
pub fn record(codec: Codec, key_unit: &[u8]) -> Option<CodecRecord> {
    match codec {
        Codec::H264 => h264_record(key_unit),
        Codec::H265 => h265_record(key_unit),
        Codec::Av1 => av1_record(key_unit),
    }
}

/// Samples (at 48 kHz) in one Opus packet, from its table-of-contents byte and, for code 3, its frame count.
pub fn opus_samples(packet: &[u8]) -> Option<u32> {
    let toc = *packet.first()?;
    let config = toc >> 3;
    let frame = match config {
        0..=11 => [480, 960, 1920, 2880][usize::from(config % 4)],
        12..=15 => [480, 960][usize::from(config % 2)],
        _ => [120, 240, 480, 960][usize::from(config % 4)],
    };
    let frames = match toc & 3 {
        0 => 1,
        1 | 2 => 2,
        _ => u32::from(*packet.get(1)? & 0x3f),
    };
    Some(frame * frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SC: [u8; 4] = [0, 0, 0, 1];
    // x264's SPS for 1280x720 High profile, level 3.1, and a PPS.
    const SPS: [u8; 21] = [0x67, 0x64, 0x00, 0x1f, 0xac, 0xd9, 0x40, 0x50, 0x05, 0xbb, 0x01, 0x10, 0x00, 0x00, 0x03, 0x00, 0x10, 0x00, 0x00, 0x03, 0x01];
    const PPS: [u8; 4] = [0x68, 0xeb, 0xe3, 0xcb];

    #[test]
    fn nal_units_split_on_both_start_code_lengths_and_trim_padding() {
        let mut au = Vec::new();
        au.extend_from_slice(&SC);
        au.extend_from_slice(&[0x09, 0xf0]);
        au.extend_from_slice(&[0, 0, 1, 0x65, 1, 2, 3, 0, 0]);
        au.extend_from_slice(&SC);
        au.extend_from_slice(&[0x41, 9]);
        let n = nal_units(&au);
        assert_eq!(n.iter().map(|&(s, e)| au[s..e].to_vec()).collect::<Vec<_>>(), [vec![0x09, 0xf0], vec![0x65, 1, 2, 3], vec![0x41, 9]]);
    }

    #[test]
    fn h264_record_carries_profile_level_and_both_sets_with_the_high_profile_extension() {
        let mut au = SC.to_vec();
        au.extend_from_slice(&SPS);
        au.extend_from_slice(&SC);
        au.extend_from_slice(&PPS);
        au.extend_from_slice(&SC);
        au.extend_from_slice(&[0x65, 0x88]);
        let r = h264_record(&au).unwrap().record;
        assert_eq!(&r[..6], &[1, 0x64, 0x00, 0x1f, 0xff, 0xe1]);
        assert_eq!(r[6..8], (SPS.len() as u16).to_be_bytes());
        // 4:2:0, 8-bit, no extended sets.
        assert_eq!(&r[r.len() - 4..], &[0xfd, 0xf8, 0xf8, 0]);
        assert!(h264_record(&SC).is_none());
    }

    #[test]
    fn opus_packet_lengths_follow_the_toc() {
        assert_eq!(opus_samples(&[0xfc, 0]), Some(960)); // CELT 20 ms, one frame
        assert_eq!(opus_samples(&[0x80, 0]), Some(120)); // CELT 2.5 ms
        assert_eq!(opus_samples(&[0xfd, 0]), Some(1920)); // two frames of 20 ms
        assert_eq!(opus_samples(&[0xff, 3]), Some(2880)); // three frames
        assert_eq!(opus_samples(&[0x00, 0]), Some(480)); // SILK 10 ms
        assert_eq!(opus_samples(&[]), None);
    }

    /// An AV1 sequence header for 1920x1080, profile 0, level index 8, 8-bit 4:2:0, written field by field from the spec's syntax.
    fn av1_sequence_header() -> Vec<u8> {
        let mut bits = Vec::new();
        let mut put = |v: u32, n: u32| (0..n).rev().for_each(|i| bits.push((v >> i) & 1));
        put(0, 3); // profile
        put(0, 2); // still picture, reduced header
        put(0, 1); // timing info
        put(0, 1); // initial display delay
        put(0, 5); // one operating point
        put(0, 12);
        put(8, 5);
        put(0, 1); // tier
        put(11, 4);
        put(11, 4);
        put(1919, 12);
        put(1079, 12);
        put(0, 1); // frame ids
        put(0b111, 3); // 128 superblock, filter intra, intra edge
        put(0b11111, 5); // interintra, masked, warped, dual filter, order hint
        put(0b11, 2); // jnt comp, ref frame mvs
        put(1, 1); // choose screen content tools
        put(1, 1); // choose integer mv
        put(6, 3); // order hint bits
        put(0b011, 3); // superres off, cdef, restoration
        put(0, 3); // 8-bit, not mono, no colour description
        put(0, 1); // colour range
        put(0, 2); // chroma sample position
        put(0, 2); // separate uv delta q, film grain
        put(1, 1); // trailing bit
        while bits.len() % 8 != 0 {
            bits.push(0);
        }
        let payload: Vec<u8> = bits.chunks(8).map(|c| c.iter().fold(0, |a, b| a << 1 | *b as u8)).collect();
        let mut obu = vec![0x0a, payload.len() as u8];
        obu.extend(payload);
        obu
    }

    #[test]
    fn av1_temporal_delimiters_are_dropped_and_the_sequence_header_is_read() {
        let seq = av1_sequence_header();
        let mut tu = vec![0x12, 0x00];
        tu.extend_from_slice(&seq);
        tu.extend_from_slice(&[0x32, 0x02, 0xaa, 0xbb]);
        let kept = av1_sample(&tu);
        assert_eq!(kept, [(2, 2 + seq.len()), (2 + seq.len(), 6 + seq.len())]);
        let r = av1_record(&tu).unwrap().record;
        // version 1, level index 8; 4:2:0 (both subsampling bits), 8-bit, colour; then the header itself.
        assert_eq!(&r[..4], &[0x81, 0x08, 0x0c, 0x00]);
        assert_eq!(&r[4..], &seq[..]);
    }
}
