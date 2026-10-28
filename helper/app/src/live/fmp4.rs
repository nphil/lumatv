//! Fragmented MP4 (the segments of an HLS stream as Home Assistant's stream integration writes them) to whole access units in Annex B.
//!
//! The initialisation segment gives the codec and its parameter sets (`avcC`: SPS and PPS; `hvcC`: VPS, SPS and PPS), the picture size and
//! the track's timescale. Each media segment is one or more `moof` + `mdat` pairs: `tfhd` (the defaults), `tfdt` (the decode time of the
//! first sample), `trun` (the samples: duration, size, flags, composition offset) and the bytes. A sample is a list of NAL units, each
//! behind a length of `nal_length_size` bytes; it becomes one access unit with `00 00 00 01` before every NAL unit, and the parameter sets
//! in front of a key picture (NDL's pipeline wants them with the IDR it starts from).
//!
//! Only what the decoder path needs is read; anything else is skipped by its box size. A box that lies about its size, or a sample that
//! runs past its `mdat`, ends the segment with an error in words rather than a panic.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    H264,
    H265,
}

/// What the initialisation segment says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Init {
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
    /// Ticks per second of the track's times.
    pub timescale: u32,
    /// How many bytes carry each NAL unit's length in a sample.
    pub nal_length_size: usize,
    /// The parameter sets, Annex B, in the order they are sent (VPS, SPS, PPS).
    pub parameter_sets: Vec<u8>,
    pub track_id: u32,
}

/// One access unit, ready for the feeder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessUnit {
    /// Decode time in the track's timescale: rises with every unit, which is what the decoder's pipeline wants its timestamps to do.
    pub dts: i64,
    /// Presentation time (decode time plus the composition offset); behind the decode order where the encoder used B pictures.
    pub pts: i64,
    pub key: bool,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DemuxError(pub String);

impl fmt::Display for DemuxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<T>(why: impl Into<String>) -> Result<T, DemuxError> {
    Err(DemuxError(why.into()))
}

fn be16(b: &[u8], at: usize) -> Result<u16, DemuxError> {
    b.get(at..at + 2)
        .map(|s| u16::from_be_bytes([s[0], s[1]]))
        .ok_or_else(|| DemuxError("short box".into()))
}

fn be32(b: &[u8], at: usize) -> Result<u32, DemuxError> {
    b.get(at..at + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| DemuxError("short box".into()))
}

fn be64(b: &[u8], at: usize) -> Result<u64, DemuxError> {
    b.get(at..at + 8)
        .map(|s| u64::from_be_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
        .ok_or_else(|| DemuxError("short box".into()))
}

/// A box: its type, its whole bytes, and the offset of its payload within them.
type Box<'a> = (&'a [u8; 4], &'a [u8], usize);

/// The boxes directly inside `data`.
fn children(data: &[u8]) -> Result<Vec<Box<'_>>, DemuxError> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 8 <= data.len() {
        let size32 = be32(data, at)? as usize;
        let kind: &[u8; 4] = data[at + 4..at + 8].try_into().expect("four bytes");
        let (size, head) = match size32 {
            0 => (data.len() - at, 8),
            1 => (be64(data, at + 8)? as usize, 16),
            n => (n, 8),
        };
        if size < head || at + size > data.len() {
            return err(format!(
                "box {} says {size} bytes where {} remain",
                String::from_utf8_lossy(kind),
                data.len() - at
            ));
        }
        out.push((kind, &data[at..at + size], head));
        at += size;
    }
    Ok(out)
}

fn find<'a>(data: &'a [u8], kind: &[u8; 4]) -> Result<Option<&'a [u8]>, DemuxError> {
    Ok(children(data)?
        .into_iter()
        .find(|(k, _, _)| *k == kind)
        .map(|(_, b, head)| &b[head..]))
}

fn need<'a>(data: &'a [u8], kind: &[u8; 4]) -> Result<&'a [u8], DemuxError> {
    find(data, kind)?.ok_or_else(|| DemuxError(format!("no {} box", String::from_utf8_lossy(kind))))
}

const START: [u8; 4] = [0, 0, 0, 1];

/// Reads the initialisation segment (`ftyp` + `moov`).
pub fn parse_init(data: &[u8]) -> Result<Init, DemuxError> {
    let moov = need(data, b"moov")?;
    // the first video track
    for (kind, trak, head) in children(moov)? {
        if kind != b"trak" {
            continue;
        }
        let trak = &trak[head..];
        let tkhd = need(trak, b"tkhd")?;
        let track_id = if tkhd.first() == Some(&1) {
            be32(tkhd, 4 + 8 + 8)?
        } else {
            be32(tkhd, 4 + 4 + 4)?
        };
        let mdia = need(trak, b"mdia")?;
        let hdlr = need(mdia, b"hdlr")?;
        if hdlr.get(8..12) != Some(b"vide") {
            continue;
        }
        let mdhd = need(mdia, b"mdhd")?;
        let timescale = if mdhd.first() == Some(&1) {
            be32(mdhd, 4 + 8 + 8)?
        } else {
            be32(mdhd, 4 + 4 + 4)?
        };
        let stsd = need(need(need(mdia, b"minf")?, b"stbl")?, b"stsd")?;
        // the sample entries follow the full box header and the count
        for (entry_kind, entry, head) in children(&stsd[8..])? {
            let visual = &entry[head..];
            // VisualSampleEntry: 6 reserved + data_reference_index (2) + 16 pre-defined/reserved + width (2) + height (2) ...
            let width = u32::from(be16(visual, 24)?);
            let height = u32::from(be16(visual, 26)?);
            let extensions = visual.get(78..).unwrap_or(&[]);
            match entry_kind {
                b"avc1" | b"avc3" => {
                    let avcc = need(extensions, b"avcC")?;
                    let (nal_length_size, sets) = parse_avcc(avcc)?;
                    return Ok(Init {
                        codec: Codec::H264,
                        width,
                        height,
                        timescale,
                        nal_length_size,
                        parameter_sets: sets,
                        track_id,
                    });
                }
                b"hvc1" | b"hev1" => {
                    let hvcc = need(extensions, b"hvcC")?;
                    let (nal_length_size, sets) = parse_hvcc(hvcc)?;
                    return Ok(Init {
                        codec: Codec::H265,
                        width,
                        height,
                        timescale,
                        nal_length_size,
                        parameter_sets: sets,
                        track_id,
                    });
                }
                _ => continue,
            }
        }
        return err("the video track's sample entry is neither AVC nor HEVC");
    }
    err("no video track")
}

/// `AVCDecoderConfigurationRecord`: the NAL length size and the SPS and PPS as Annex B.
fn parse_avcc(b: &[u8]) -> Result<(usize, Vec<u8>), DemuxError> {
    if b.len() < 7 || b[0] != 1 {
        return err("avcC is not version 1");
    }
    let nal_length_size = usize::from(b[4] & 3) + 1;
    let mut sets = Vec::new();
    let mut at = 6;
    let sps_count = usize::from(b[5] & 0x1f);
    for _ in 0..sps_count {
        let n = usize::from(be16(b, at)?);
        at += 2;
        let sps = b
            .get(at..at + n)
            .ok_or_else(|| DemuxError("avcC: a short SPS".into()))?;
        sets.extend_from_slice(&START);
        sets.extend_from_slice(sps);
        at += n;
    }
    let pps_count = usize::from(
        *b.get(at)
            .ok_or_else(|| DemuxError("avcC: no PPS count".into()))?,
    );
    at += 1;
    for _ in 0..pps_count {
        let n = usize::from(be16(b, at)?);
        at += 2;
        let pps = b
            .get(at..at + n)
            .ok_or_else(|| DemuxError("avcC: a short PPS".into()))?;
        sets.extend_from_slice(&START);
        sets.extend_from_slice(pps);
        at += n;
    }
    if sets.is_empty() {
        return err("avcC carries no parameter sets");
    }
    Ok((nal_length_size, sets))
}

/// `HEVCDecoderConfigurationRecord`: the NAL length size and every array (VPS, SPS, PPS) as Annex B.
fn parse_hvcc(b: &[u8]) -> Result<(usize, Vec<u8>), DemuxError> {
    if b.len() < 23 || b[0] != 1 {
        return err("hvcC is not version 1");
    }
    let nal_length_size = usize::from(b[21] & 3) + 1;
    let arrays = usize::from(b[22]);
    let mut sets = Vec::new();
    let mut at = 23;
    for _ in 0..arrays {
        at += 1; // array_completeness and the NAL unit type
        let count = usize::from(be16(b, at)?);
        at += 2;
        for _ in 0..count {
            let n = usize::from(be16(b, at)?);
            at += 2;
            let nal = b
                .get(at..at + n)
                .ok_or_else(|| DemuxError("hvcC: a short parameter set".into()))?;
            sets.extend_from_slice(&START);
            sets.extend_from_slice(nal);
            at += n;
        }
    }
    if sets.is_empty() {
        return err("hvcC carries no parameter sets");
    }
    Ok((nal_length_size, sets))
}

/// Reads one media segment (`moof` + `mdat`, possibly several) into access units. `pts` is in the track's timescale.
pub fn parse_segment(init: &Init, data: &[u8]) -> Result<Vec<AccessUnit>, DemuxError> {
    let boxes = children(data)?;
    let mut out = Vec::new();
    let mut moof_at: Option<(usize, &[u8])> = None;
    let mut offset = 0usize;
    for (kind, whole, head) in &boxes {
        match *kind {
            b"moof" => moof_at = Some((offset, &whole[*head..])),
            b"mdat" => {
                let Some((moof_offset, moof)) = moof_at.take() else {
                    return err("mdat before any moof");
                };
                fragment(init, moof, moof_offset, data, offset, &mut out)?;
            }
            _ => {}
        }
        offset += whole.len();
    }
    Ok(out)
}

fn fragment(
    init: &Init,
    moof: &[u8],
    moof_offset: usize,
    file: &[u8],
    mdat_offset: usize,
    out: &mut Vec<AccessUnit>,
) -> Result<(), DemuxError> {
    for (kind, traf, head) in children(moof)? {
        if kind != b"traf" {
            continue;
        }
        let traf = &traf[head..];
        let tfhd = need(traf, b"tfhd")?;
        let flags = be32(tfhd, 0)? & 0x00ff_ffff;
        let track_id = be32(tfhd, 4)?;
        if track_id != init.track_id {
            continue;
        }
        let mut at = 8;
        let mut base_offset: Option<u64> = None;
        if flags & 0x1 != 0 {
            base_offset = Some(be64(tfhd, at)?);
            at += 8;
        }
        if flags & 0x2 != 0 {
            at += 4; // sample description index
        }
        let default_duration = if flags & 0x8 != 0 {
            let v = be32(tfhd, at)?;
            at += 4;
            Some(v)
        } else {
            None
        };
        let default_size = if flags & 0x10 != 0 {
            let v = be32(tfhd, at)?;
            at += 4;
            Some(v)
        } else {
            None
        };
        let default_flags = if flags & 0x20 != 0 {
            Some(be32(tfhd, at)?)
        } else {
            None
        };
        let base_is_moof = flags & 0x0002_0000 != 0;
        let tfdt = need(traf, b"tfdt")?;
        let mut decode_time: i64 = if tfdt.first() == Some(&1) {
            be64(tfdt, 4)? as i64
        } else {
            i64::from(be32(tfdt, 4)?)
        };
        for (kind, trun, head) in children(traf)? {
            if kind != b"trun" {
                continue;
            }
            let trun = &trun[head..];
            let tflags = be32(trun, 0)? & 0x00ff_ffff;
            let count = be32(trun, 4)? as usize;
            let mut at = 8;
            let data_offset = if tflags & 0x1 != 0 {
                let v = be32(trun, at)? as i32;
                at += 4;
                Some(v)
            } else {
                None
            };
            let first_flags = if tflags & 0x4 != 0 {
                let v = be32(trun, at)?;
                at += 4;
                Some(v)
            } else {
                None
            };
            // where the samples start in the file: an explicit base, else the moof's own start (default-base-is-moof, which is what
            // every writer of CMAF segments sets; a segment with several trafs and no flag is read the same, as it is one track here)
            let base = match base_offset {
                Some(b) => b as usize,
                None => moof_offset,
            };
            let _ = (base_is_moof, mdat_offset);
            let mut cursor = (base as i64 + i64::from(data_offset.unwrap_or(0))) as usize;
            for i in 0..count {
                let duration = if tflags & 0x100 != 0 {
                    let v = be32(trun, at)?;
                    at += 4;
                    v
                } else {
                    default_duration
                        .ok_or_else(|| DemuxError("a sample without a duration".into()))?
                };
                let size = if tflags & 0x200 != 0 {
                    let v = be32(trun, at)?;
                    at += 4;
                    v
                } else {
                    default_size.ok_or_else(|| DemuxError("a sample without a size".into()))?
                } as usize;
                let sflags = if tflags & 0x400 != 0 {
                    let v = be32(trun, at)?;
                    at += 4;
                    v
                } else if i == 0 && first_flags.is_some() {
                    first_flags.unwrap_or(0)
                } else {
                    default_flags.unwrap_or(0)
                };
                // a version-1 trun carries signed offsets; a version-0 one unsigned values that never reach the sign bit
                let composition = if tflags & 0x800 != 0 {
                    let v = be32(trun, at)? as i32;
                    at += 4;
                    v
                } else {
                    0
                };
                let sample = file
                    .get(cursor..cursor + size)
                    .ok_or_else(|| DemuxError(format!("sample {i} runs past the segment")))?;
                let key = sflags & 0x0001_0000 == 0; // sample_is_non_sync_sample is bit 16
                out.push(AccessUnit {
                    dts: decode_time,
                    pts: decode_time + i64::from(composition),
                    key,
                    data: annex_b(init, sample, key)?,
                });
                decode_time += i64::from(duration);
                cursor += size;
            }
        }
    }
    Ok(())
}

/// A length-prefixed sample as Annex B, with the parameter sets in front of a key picture.
fn annex_b(init: &Init, sample: &[u8], key: bool) -> Result<Vec<u8>, DemuxError> {
    let mut out =
        Vec::with_capacity(sample.len() + 16 + if key { init.parameter_sets.len() } else { 0 });
    if key {
        out.extend_from_slice(&init.parameter_sets);
    }
    let mut at = 0;
    while at < sample.len() {
        let n = init.nal_length_size;
        let len_bytes = sample
            .get(at..at + n)
            .ok_or_else(|| DemuxError("a NAL unit length runs past the sample".into()))?;
        let len = len_bytes
            .iter()
            .fold(0usize, |acc, b| (acc << 8) | usize::from(*b));
        at += n;
        let nal = sample
            .get(at..at + len)
            .ok_or_else(|| DemuxError("a NAL unit runs past the sample".into()))?;
        out.extend_from_slice(&START);
        out.extend_from_slice(nal);
        at += len;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INIT: &[u8] = include_bytes!("../../tests/fixtures/hls/init.mp4");
    const SEG0: &[u8] = include_bytes!("../../tests/fixtures/hls/seg0.m4s");
    const SEG1: &[u8] = include_bytes!("../../tests/fixtures/hls/seg1.m4s");

    #[test]
    fn the_init_segment_gives_the_codec_the_size_the_timescale_and_the_parameter_sets() {
        let init = parse_init(INIT).unwrap();
        assert_eq!(
            (init.codec, init.width, init.height),
            (Codec::H264, 320, 180)
        );
        assert_eq!(init.nal_length_size, 4);
        assert_eq!(init.track_id, 1);
        assert!(init.timescale > 0);
        // an SPS (type 7) and a PPS (type 8), each behind a start code
        assert_eq!(&init.parameter_sets[..4], &START);
        assert_eq!(init.parameter_sets[4] & 0x1f, 7);
        let second = init.parameter_sets[4..]
            .windows(4)
            .position(|w| w == START)
            .map(|p| p + 4)
            .unwrap();
        assert_eq!(init.parameter_sets[second + 4] & 0x1f, 8);
    }

    #[test]
    fn a_segment_is_whole_access_units_in_annex_b_with_the_sets_before_the_key_picture_and_rising_times()
     {
        let init = parse_init(INIT).unwrap();
        let units = parse_segment(&init, SEG0).unwrap();
        assert_eq!(units.len(), 10, "one second at ten pictures a second");
        assert!(units[0].key && units[1..].iter().all(|u| !u.key));
        assert!(units[0].data.starts_with(&init.parameter_sets));
        for u in &units {
            assert_eq!(&u.data[..4], &START);
            assert!(
                !u.data.windows(4).any(|w| w == [0, 0, 0, 0]),
                "no length prefix left"
            );
        }
        // decode order with composition offsets (the encoder used B pictures): the decode times rise one step at a time, the
        // presentation times are the same steps once sorted
        let dts: Vec<i64> = units.iter().map(|u| u.dts).collect();
        let step = dts[1] - dts[0];
        assert!(
            step > 0 && dts.windows(2).all(|w| w[1] - w[0] == step),
            "{dts:?}"
        );
        let mut times: Vec<i64> = units.iter().map(|u| u.pts).collect();
        assert_ne!(
            times, dts,
            "this fixture has B pictures: presentation differs from decode order"
        );
        times.sort_unstable();
        assert!(times.windows(2).all(|w| w[1] - w[0] == step), "{times:?}");
        let next = parse_segment(&init, SEG1).unwrap();
        assert!(
            next[0].dts > units[9].dts,
            "the next segment continues the clock"
        );
        assert_eq!(
            (next[0].dts - units[0].dts) as f64 / f64::from(init.timescale),
            1.0,
            "one second apart"
        );
    }

    #[test]
    fn a_segment_that_lies_is_an_error_in_words() {
        let init = parse_init(INIT).unwrap();
        assert!(parse_segment(&init, &SEG0[..100]).is_err());
        let mut cut = SEG0.to_vec();
        cut.truncate(SEG0.len() - 2000);
        let why = parse_segment(&init, &cut).unwrap_err().0;
        assert!(why.contains("runs past") || why.contains("says"), "{why}");
        assert!(parse_init(b"\0\0\0\x08free").is_err());
    }
}
