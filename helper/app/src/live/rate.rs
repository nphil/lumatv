//! The rate a camera's stream tells the TV's decoder it has, and the panel's refresh with it.
//!
//! LG's kernel sets the panel's rate from the frame rate the decoder reports for the main window's picture (`_vsc_check_fr`,
//! `v4l2_vsc_event.c`): 25, 50 and 100 fps give 100 Hz, every other rate 120 Hz, and the panel stays at 100 Hz after the window closes
//! until another video's first picture. The decoder's rate is chosen in `vrate.c` from the stream's own (the sequence parameter set's
//! VUI timing) and the container's (the NDL pipeline sets none); only with neither does it use the intervals of the pictures' times,
//! and the pace the pictures arrive at is never read. The rate the video sink is told (`setVideoData.frameRate`) is the decoder's.
//! A camera whose parameter set says 25 therefore takes the panel to 100 Hz at its first picture, however the pictures are paced
//! (`docs/research/vwin-refresh-result.md`).
//!
//! [`keep_panel_rate`] writes another rate into the parameter sets of a key picture before it goes to the feeder: 30 for 25, 60 for
//! 50, 120 for 100. It keeps the camera's own tick and changes the 32 bits of `time_scale`, in an H.264 sequence parameter set, an H.265
//! one and an H.265 video parameter set that has timing; nothing else of the stream changes, and a set that cannot be read the way the
//! helper's own parser reads it, or says no rate, is left as the camera sent it.

use rtsp_client::depay::{START_CODE, nal_units};
use rtsp_client::{Codec, sps};

/// What a parameter set said the rate was, and what it says now, in pictures a second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Redeclared {
    pub was: f32,
    pub now: u32,
}

/// The rate to declare in place of a rate that makes 100 Hz, from the rate in tenths of a Hz as the kernel reads it (`None`: the panel
/// keeps 120 Hz with it). `_vsc_check_fr` takes 24.5 to 25.5 and 49.5 to 50.5 as 50, and halves a rate over 90 first.
fn declared_instead(tenths: u32) -> Option<u32> {
    match tenths {
        245..=255 => Some(30),
        495..=505 => Some(60),
        990..=1011 => Some(120),
        _ => None,
    }
}

/// A rate `scale / ticks` as the kernel counts it: hundredths of a Hz (`res * 100 / div`), then tenths.
fn tenths(scale: u32, ticks: u64) -> u32 {
    (u64::from(scale) * 100 / ticks.max(1) / 10).min(u64::from(u32::MAX)) as u32
}

/// The pictures a second a rate that makes 100 Hz is declared as instead (`None` for any other).
pub fn presented(fps: f32) -> Option<u32> {
    declared_instead((fps * 10.0) as u32)
}

/// The bits of a parameter set's payload, most significant first.
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Bits { data, pos: 0 }
    }

    fn bit(&mut self) -> Option<u32> {
        let byte = *self.data.get(self.pos / 8)?;
        let bit = (byte >> (7 - self.pos % 8)) & 1;
        self.pos += 1;
        Some(u32::from(bit))
    }

    fn flag(&mut self) -> Option<bool> {
        Some(self.bit()? == 1)
    }

    fn bits(&mut self, n: u32) -> Option<u32> {
        let mut v = 0u64;
        for _ in 0..n {
            v = (v << 1) | u64::from(self.bit()?);
        }
        Some(v as u32)
    }

    fn skip(&mut self, n: usize) -> Option<()> {
        (self.pos + n <= self.data.len() * 8).then(|| self.pos += n)
    }

    /// An unsigned Exp-Golomb code.
    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        if zeros == 0 {
            return Some(0);
        }
        Some(((1u64 << zeros) - 1 + u64::from(self.bits(zeros)?)) as u32)
    }

    /// A signed Exp-Golomb code.
    fn se(&mut self) -> Option<i32> {
        let k = self.ue()?;
        Some(if k % 2 == 1 {
            k.div_ceil(2) as i32
        } else {
            -((k / 2) as i32)
        })
    }
}

/// Writes `value` over the 32 bits that begin at bit `at`.
fn put32(data: &mut [u8], at: usize, value: u32) {
    for i in 0..32 {
        let (byte, shift) = ((at + i) / 8, 7 - (at + i) % 8);
        let bit = ((value >> (31 - i)) & 1) as u8;
        data[byte] = (data[byte] & !(1 << shift)) | (bit << shift);
    }
}

/// A NAL unit's payload without its emulation prevention bytes (every `00 00 03` is `00 00`).
fn unescape(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len());
    let mut zeros = 0;
    for &b in payload {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}

/// The payload as a NAL unit carries it: a `03` between two zero bytes and a byte of 3 or less.
fn escape(rbsp: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rbsp.len() + 8);
    let mut zeros = 0;
    for &b in rbsp {
        if zeros >= 2 && b <= 3 {
            out.push(3);
            zeros = 0;
        }
        out.push(b);
        zeros = if b == 0 { zeros + 1 } else { 0 };
    }
    out
}

/// Where a parameter set's timing is: the bit its `num_units_in_tick` begins at (`time_scale` follows), and both values.
struct Timing {
    at: usize,
    units: u32,
    scale: u32,
}

/// A scaling list (7.3.2.1.1.1), read to get past it.
fn scaling_list(r: &mut Bits, size: usize) -> Option<()> {
    let (mut last, mut next) = (8i32, 8i32);
    for _ in 0..size {
        if next != 0 {
            next = (last + r.se()?).rem_euclid(256);
        }
        if next != 0 {
            last = next;
        }
    }
    Some(())
}

/// The timing of an H.264 sequence parameter set's payload (7.3.2.1.1 up to `vui_parameters()`, E.1.1), or `None` where it has none.
fn h264_timing(rbsp: &[u8]) -> Option<Timing> {
    let mut r = Bits::new(rbsp);
    let profile = r.bits(8)?;
    r.skip(16)?;
    r.ue()?;
    if matches!(
        profile,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    ) {
        let chroma = r.ue()?;
        if chroma == 3 {
            r.skip(1)?;
        }
        r.ue()?;
        r.ue()?;
        r.skip(1)?;
        if r.flag()? {
            for i in 0..(if chroma != 3 { 8 } else { 12 }) {
                if r.flag()? {
                    scaling_list(&mut r, if i < 6 { 16 } else { 64 })?;
                }
            }
        }
    }
    r.ue()?;
    match r.ue()? {
        0 => {
            r.ue()?;
        }
        1 => {
            r.skip(1)?;
            r.se()?;
            r.se()?;
            let cycle = r.ue()?;
            if cycle > 255 {
                return None;
            }
            for _ in 0..cycle {
                r.se()?;
            }
        }
        _ => {}
    }
    r.ue()?;
    r.skip(1)?;
    r.ue()?;
    r.ue()?;
    if !r.flag()? {
        r.skip(1)?;
    }
    r.skip(1)?;
    if r.flag()? {
        for _ in 0..4 {
            r.ue()?;
        }
    }
    if !r.flag()? {
        return None;
    }
    if r.flag()? && r.bits(8)? == 255 {
        r.skip(32)?;
    }
    if r.flag()? {
        r.skip(1)?;
    }
    if r.flag()? {
        r.skip(4)?;
        if r.flag()? {
            r.skip(24)?;
        }
    }
    if r.flag()? {
        r.ue()?;
        r.ue()?;
    }
    if !r.flag()? {
        return None;
    }
    let at = r.pos;
    Some(Timing {
        at,
        units: r.bits(32)?,
        scale: r.bits(32)?,
    })
}

/// A parameter set NAL unit (`header` bytes of NAL header) with `time_scale` written so that its rate is one the panel keeps 120 Hz for: the
/// payload is walked by `find`, and `per_picture` is how many ticks of `time_scale` a picture takes (2 in H.264, 1 in H.265). `None` when
/// the set says no rate, a rate that does not matter, or one whose new scale does not fit 32 bits.
fn redeclare(
    nal: &[u8],
    header: usize,
    find: fn(&[u8]) -> Option<Timing>,
    per_picture: u32,
) -> Option<(Vec<u8>, Redeclared, Timing)> {
    let mut rbsp = unescape(nal.get(header..)?);
    let timing = find(&rbsp)?;
    if timing.units == 0 || timing.scale == 0 {
        return None;
    }
    let ticks = u64::from(per_picture) * u64::from(timing.units);
    let now = declared_instead(tenths(timing.scale, ticks))?;
    let scale = now.checked_mul(timing.units)?.checked_mul(per_picture)?;
    put32(&mut rbsp, timing.at + 32, scale);
    let mut out = nal[..header].to_vec();
    out.extend_from_slice(&escape(&rbsp));
    let was = timing.scale as f32 / ticks as f32;
    Some((out, Redeclared { was, now }, Timing { scale, ..timing }))
}

/// An H.264 sequence parameter set (7.3.2.1.1, annex E.1.1) re-declared; `None` too when the parser that reads every stream's size
/// (`rtsp_client::sps`) does not find the rate that was there, the same picture and the new rate.
fn h264_sps(nal: &[u8]) -> Option<(Vec<u8>, Redeclared)> {
    let (out, said, _) = redeclare(nal, 1, h264_timing, 2)?;
    let (before, after) = (sps::parse_h264(nal).ok()?, sps::parse_h264(&out).ok()?);
    let same = sps::Info {
        fps: None,
        ..before
    } == sps::Info { fps: None, ..after };
    (same && before.fps == Some(said.was) && after.fps == Some(said.now as f32))
        .then_some((out, said))
}

/// `profile_tier_level(1, max_sub_layers_minus1)` (7.3.3), read to get past it.
fn profile_tier_level(r: &mut Bits, max_sub_layers_minus1: u32) -> Option<()> {
    r.skip(96)?;
    let (mut profile, mut level) = ([false; 8], [false; 8]);
    for i in 0..max_sub_layers_minus1 as usize {
        profile[i] = r.flag()?;
        level[i] = r.flag()?;
    }
    if max_sub_layers_minus1 > 0 {
        r.skip(2 * (8 - max_sub_layers_minus1 as usize))?;
    }
    for i in 0..max_sub_layers_minus1 as usize {
        if profile[i] {
            r.skip(88)?;
        }
        if level[i] {
            r.skip(8)?;
        }
    }
    Some(())
}

/// `scaling_list_data()` (7.3.4), read to get past it.
fn h265_scaling_list_data(r: &mut Bits) -> Option<()> {
    for size_id in 0..4 {
        for _ in (0..6).step_by(if size_id == 3 { 3 } else { 1 }) {
            if !r.flag()? {
                r.ue()?;
                continue;
            }
            if size_id > 1 {
                r.se()?;
            }
            for _ in 0..(1 << (4 + (size_id << 1))).min(64) {
                r.se()?;
            }
        }
    }
    Some(())
}

/// `st_ref_pic_set(idx)` (7.3.7) of an SPS, read to get past it; `deltas` are the `NumDeltaPocs` of the sets before it, and the answer is its own.
fn h265_st_ref_pic_set(r: &mut Bits, idx: usize, deltas: &[u32]) -> Option<u32> {
    if idx != 0 && r.flag()? {
        // predicted from the set before it (an SPS has no `delta_idx_minus1`)
        r.skip(1)?;
        r.ue()?;
        let mut kept = 0;
        for _ in 0..=*deltas.get(idx - 1)? {
            let used = r.flag()?;
            if used || r.flag()? {
                kept += 1;
            }
        }
        return Some(kept);
    }
    let (negative, positive) = (r.ue()?, r.ue()?);
    if negative > 16 || positive > 16 {
        return None;
    }
    for _ in 0..negative + positive {
        r.ue()?;
        r.skip(1)?;
    }
    Some(negative + positive)
}

/// The `vui_timing_info` of a `vui_parameters()` (E.2.1), `r` at its start, or `None` where it has none.
fn h265_vui_timing(r: &mut Bits) -> Option<Timing> {
    if r.flag()? && r.bits(8)? == 255 {
        r.skip(32)?;
    }
    if r.flag()? {
        r.skip(1)?;
    }
    if r.flag()? {
        r.skip(4)?;
        if r.flag()? {
            r.skip(24)?;
        }
    }
    if r.flag()? {
        r.ue()?;
        r.ue()?;
    }
    r.skip(3)?;
    if r.flag()? {
        for _ in 0..4 {
            r.ue()?;
        }
    }
    if !r.flag()? {
        return None;
    }
    let at = r.pos;
    Some(Timing {
        at,
        units: r.bits(32)?,
        scale: r.bits(32)?,
    })
}

/// The timing of an H.265 sequence parameter set's payload (7.3.2.2.1 up to `vui_parameters()`), or `None` where it has none.
fn h265_sps_timing(rbsp: &[u8]) -> Option<Timing> {
    let mut r = Bits::new(rbsp);
    r.skip(4)?;
    let max_sub_layers_minus1 = r.bits(3)?;
    if max_sub_layers_minus1 > 6 {
        return None;
    }
    r.skip(1)?;
    profile_tier_level(&mut r, max_sub_layers_minus1)?;
    r.ue()?;
    if r.ue()? == 3 {
        r.skip(1)?;
    }
    r.ue()?;
    r.ue()?;
    if r.flag()? {
        for _ in 0..4 {
            r.ue()?;
        }
    }
    r.ue()?;
    r.ue()?;
    let poc_lsb_bits = r.ue()?.checked_add(4)?;
    if poc_lsb_bits > 16 {
        return None;
    }
    let ordering_info = r.flag()?;
    for _ in (if ordering_info {
        0
    } else {
        max_sub_layers_minus1
    })..=max_sub_layers_minus1
    {
        for _ in 0..3 {
            r.ue()?;
        }
    }
    for _ in 0..6 {
        r.ue()?;
    }
    if r.flag()? && r.flag()? {
        h265_scaling_list_data(&mut r)?;
    }
    r.skip(2)?;
    if r.flag()? {
        r.skip(8)?;
        r.ue()?;
        r.ue()?;
        r.skip(1)?;
    }
    let sets = r.ue()? as usize;
    if sets > 64 {
        return None;
    }
    let mut deltas = Vec::with_capacity(sets);
    for idx in 0..sets {
        let n = h265_st_ref_pic_set(&mut r, idx, &deltas)?;
        deltas.push(n);
    }
    if r.flag()? {
        let long_term = r.ue()?;
        if long_term > 32 {
            return None;
        }
        for _ in 0..long_term {
            r.skip(poc_lsb_bits as usize + 1)?;
        }
    }
    r.skip(2)?;
    if !r.flag()? {
        return None;
    }
    h265_vui_timing(&mut r)
}

/// The timing of an H.265 video parameter set's payload (7.3.2.1), or `None` where it has none.
fn h265_vps_timing(rbsp: &[u8]) -> Option<Timing> {
    let mut r = Bits::new(rbsp);
    r.skip(12)?;
    let max_sub_layers_minus1 = r.bits(3)?;
    if max_sub_layers_minus1 > 6 {
        return None;
    }
    r.skip(17)?;
    profile_tier_level(&mut r, max_sub_layers_minus1)?;
    let ordering_info = r.flag()?;
    for _ in (if ordering_info {
        0
    } else {
        max_sub_layers_minus1
    })..=max_sub_layers_minus1
    {
        for _ in 0..3 {
            r.ue()?;
        }
    }
    let max_layer_id = r.bits(6)? as usize;
    let layer_sets = r.ue()? as usize;
    if layer_sets > 1023 {
        return None;
    }
    r.skip(layer_sets * (max_layer_id + 1))?;
    if !r.flag()? {
        return None;
    }
    let at = r.pos;
    Some(Timing {
        at,
        units: r.bits(32)?,
        scale: r.bits(32)?,
    })
}

/// An H.265 sequence parameter set re-declared; `None` too when the parser that reads every stream's size does not read the same picture from it.
fn h265_sps(nal: &[u8]) -> Option<(Vec<u8>, Redeclared)> {
    let (out, said, _) = redeclare(nal, 2, h265_sps_timing, 1)?;
    (sps::parse_h265(nal).ok()? == sps::parse_h265(&out).ok()?).then_some((out, said))
}

/// An H.265 video parameter set with timing re-declared; `None` too when the walk does not find the new scale where it was written.
fn h265_vps(nal: &[u8]) -> Option<(Vec<u8>, Redeclared)> {
    let (out, said, timing) = redeclare(nal, 2, h265_vps_timing, 1)?;
    let again = h265_vps_timing(&unescape(&out[2..]))?;
    (again.at == timing.at && again.units == timing.units && again.scale == timing.scale)
        .then_some((out, said))
}

/// `annexb` (a key picture, its parameter sets in front) with every sequence parameter set whose rate makes 100 Hz re-declared, and what it
/// said; `None` when there is nothing to change, which is almost always. The NAL units keep their bytes and get four-byte start codes, as the
/// depayloader writes them.
pub fn keep_panel_rate(codec: Codec, annexb: &[u8]) -> Option<(Vec<u8>, Redeclared)> {
    let nals = nal_units(annexb);
    let mut first = None;
    let rewritten: Vec<Option<Vec<u8>>> = nals
        .iter()
        .map(|nal| {
            let (new, said) = match codec {
                Codec::H264 if nal.first().is_some_and(|b| b & 0x1f == 7) => h264_sps(nal)?,
                Codec::H265 => match nal.first().map(|b| (b >> 1) & 0x3f) {
                    Some(33) => h265_sps(nal)?,
                    Some(32) => h265_vps(nal)?,
                    _ => return None,
                },
                _ => return None,
            };
            first.get_or_insert(said);
            Some(new)
        })
        .collect();
    let said = first?;
    let mut out = Vec::with_capacity(annexb.len() + 8 * nals.len());
    for (nal, new) in nals.iter().zip(&rewritten) {
        out.extend_from_slice(&START_CODE);
        out.extend_from_slice(new.as_deref().unwrap_or(nal));
    }
    Some((out, said))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// x264's parameter sets at 25 fps with what `ffmpeg -bsf:v h264_metadata=tick_rate=60/1` writes over them (the same bytes but for the
    /// timing): an independent writer of the same bits. `tests/fixtures/rate-vectors.py x264` makes them.
    const X264_25_TO_30: &[(&str, &str, &str)] = &[
        (
            "baseline 640x360",
            "6742c01ed900a02ff97011000003000100000300320f162e48",
            "6742c01ed900a02ff970110000030001000003003c0f162e48",
        ),
        (
            "main 1280x720",
            "674d401feca02802dd80880000030008000003019078c18cb0",
            "674d401feca02802dd8088000003000800000301e078c18cb0",
        ),
        (
            "high 1920x1080, cropped",
            "67640028acd940780227e5c044000003000400000300c83c60c658",
            "67640028acd940780227e5c044000003000400000300f03c60c658",
        ),
        (
            "main with HRD",
            "674d401feca02802dd808800000300080000030197130007a12000f427261807c60c6580",
            "674d401feca02802dd8088000003000800000301e7130007a12000f427261807c60c6580",
        ),
        (
            "high with scaling lists",
            "6764001eacd940a02ff97011000003000100000300320f162d96",
            "6764001eacd940a02ff970110000030001000003003c0f162d96",
        ),
        (
            "high interlaced 720x576",
            "6764001eacd940b424d8088000000300800000190f8a14cb",
            "6764001eacd940b424d80880000003008000001e0f8a14cb",
        ),
    ];

    #[test]
    fn the_kernels_100_hz_family_is_a_25_a_50_or_a_100_and_gets_a_rate_the_panel_keeps_120_for() {
        // `_vsc_check_fr`: tenths of a Hz, 25 and 50 to a half Hz either way, a rate over 90 halved first
        for (tenths, now) in [
            (250, 30),
            (245, 30),
            (255, 30),
            (500, 60),
            (495, 60),
            (505, 60),
            (1000, 120),
            (990, 120),
            (1011, 120),
        ] {
            assert_eq!(declared_instead(tenths), Some(now), "{tenths}");
        }
        for tenths in [
            0, 100, 150, 200, 240, 244, 256, 300, 400, 494, 506, 550, 600, 900, 989, 1012, 1200,
            2400,
        ] {
            assert_eq!(declared_instead(tenths), None, "{tenths}");
        }
        assert_eq!(
            (
                presented(25.0),
                presented(24.99),
                presented(50.0),
                presented(30.0),
                presented(23.976)
            ),
            (Some(30), Some(30), Some(60), None, None)
        );
        // the kernel's own arithmetic: hundredths from the rate's fraction, then tenths
        assert_eq!(
            (
                tenths(50, 2),
                tenths(60, 2),
                tenths(5000, 200),
                tenths(1, 2),
                tenths(50, 0)
            ),
            (250, 300, 250, 5, 500)
        );
    }

    #[test]
    fn x264_sequence_parameter_sets_at_25_and_50_fps_are_written_as_ffmpegs_bitstream_filter_writes_them()
     {
        for (name, from, to) in X264_25_TO_30 {
            let (out, said) =
                h264_sps(&hex(from)).unwrap_or_else(|| panic!("{name}: not re-declared"));
            assert_eq!(hex(to), out, "{name}");
            assert_eq!(said, Redeclared { was: 25.0, now: 30 }, "{name}");
        }
        // 50 fps: the same filter's 120/1
        let (out, said) =
            h264_sps(&hex("67640020acd9405005bb0110000003001000000640f1831960")).unwrap();
        assert_eq!(
            out,
            hex("67640020acd9405005bb0110000003001000000780f1831960")
        );
        assert_eq!(said, Redeclared { was: 50.0, now: 60 });
    }

    #[test]
    fn a_rate_the_panel_already_keeps_120_for_and_a_set_without_timing_are_left_as_they_are() {
        // x264 at 30 fps
        assert_eq!(
            h264_sps(&hex("674d401eeca05017fcb80880000003008000001e078b16cb")),
            None
        );
        // no VUI, and a VUI without timing
        assert_eq!(
            h264_sps(
                &Sps {
                    vui: Vui::None,
                    ..Sps::default()
                }
                .nal()
            ),
            None
        );
        assert_eq!(
            h264_sps(
                &Sps {
                    vui: Vui::NoTiming,
                    ..Sps::default()
                }
                .nal()
            ),
            None
        );
        // a timing of no ticks or no scale, and 29.97
        assert_eq!(
            h264_sps(
                &Sps {
                    vui: Vui::Timing(0, 50),
                    ..Sps::default()
                }
                .nal()
            ),
            None
        );
        assert_eq!(
            h264_sps(
                &Sps {
                    vui: Vui::Timing(1, 0),
                    ..Sps::default()
                }
                .nal()
            ),
            None
        );
        assert_eq!(
            h264_sps(
                &Sps {
                    vui: Vui::Timing(1001, 60000),
                    ..Sps::default()
                }
                .nal()
            ),
            None
        );
    }

    /// The shapes of set a camera's encoder may write that x264 does not, each with the timing `(units, scale)`.
    fn shapes((units, scale): (u32, u32)) -> Vec<Sps> {
        let vui = Vui::Timing(units, scale);
        let base = Sps {
            vui,
            ..Sps::default()
        };
        vec![
            base.clone(),
            Sps {
                profile: 77,
                lead: 3,
                ..base.clone()
            },
            Sps {
                profile: 100,
                ..base.clone()
            },
            Sps {
                profile: 100,
                chroma: 0,
                scaling: true,
                ..base.clone()
            },
            Sps {
                profile: 100,
                chroma: 3,
                scaling: true,
                ..base.clone()
            },
            Sps {
                poc: 1,
                ..base.clone()
            },
            Sps {
                poc: 2,
                frame_mbs_only: false,
                crop: true,
                ..base.clone()
            },
            Sps {
                extended_sar: true,
                overscan: true,
                signal: true,
                chroma_loc: true,
                ..base.clone()
            },
        ]
    }

    #[test]
    fn every_shape_of_set_is_walked_to_its_timing_and_changes_in_nothing_else() {
        // 25 fps as 1/50, as 2/100 and as 1001/50050
        for timing in [(1, 50), (2, 100), (1001, 50050)] {
            for sps in shapes(timing) {
                let nal = sps.nal();
                let (out, said) = h264_sps(&nal).unwrap_or_else(|| panic!("{sps:?}"));
                assert_eq!(said.now, 30, "{sps:?}");
                // the same bits but for `time_scale`, which is 30 pictures a second of the camera's own tick, and the helper's parser reads the rate
                let (before, after) = (unescape(&nal[1..]), unescape(&out[1..]));
                let at = h264_timing(&before).unwrap().at;
                let mut expected = before.clone();
                put32(&mut expected, at + 32, 60 * timing.0);
                assert_eq!(expected, after, "{sps:?}");
                assert_eq!(sps::parse_h264(&out).unwrap().fps, Some(30.0), "{sps:?}");
            }
        }
        // 50 fps as 1/100 and 100 fps as 1/200
        for (timing, now) in [((1, 100), 60), ((2, 200), 60), ((1, 200), 120)] {
            let out = h264_sps(
                &Sps {
                    vui: Vui::Timing(timing.0, timing.1),
                    ..Sps::default()
                }
                .nal(),
            )
            .unwrap();
            assert_eq!(out.1.now, now);
        }
    }

    #[test]
    fn the_escaped_bytes_of_the_new_timing_keep_the_sets_zero_runs_apart() {
        // 1 and 60 are `00 00 00 01 00 00 00 3c` where the timing begins on a byte boundary: every run of zeros is broken by a 03
        let aligned = [0, 1, 3, 7, 15, 31, 63, 127]
            .into_iter()
            .flat_map(|lead| {
                [false, true].map(|overscan| {
                    Sps {
                        lead,
                        overscan,
                        ..Sps::default()
                    }
                    .nal()
                })
            })
            .find(|nal| h264_timing(&unescape(&nal[1..])).unwrap().at % 8 == 0)
            .expect("a shape that puts the timing on a byte boundary");
        let (out, _) = h264_sps(&aligned).unwrap();
        assert!(
            !out.windows(3).any(|w| w[0] == 0 && w[1] == 0 && w[2] < 3),
            "{out:02x?}"
        );
        assert!(
            out.windows(5).any(|w| w == [0, 0, 3, 0, 1])
                && out.windows(5).any(|w| w == [0, 0, 3, 0, 0x3c]),
            "{out:02x?}"
        );
        assert_eq!(
            escape(&unescape(&out[1..])),
            out[1..],
            "unescaped and escaped again, the unit is as it was"
        );
    }

    #[test]
    fn a_key_picture_has_its_parameter_sets_re_declared_and_every_other_unit_left_alone() {
        let pps = hex("68ce3c80");
        let idr = [0x65, 0x88, 0x84, 0x00, 0x00, 0x03, 0x01, 0xaa, 0xbb];
        let unit = |sps: &[u8]| {
            let mut au = Vec::new();
            for nal in [sps, &pps[..], &idr[..]] {
                au.extend_from_slice(&START_CODE);
                au.extend_from_slice(nal);
            }
            au
        };
        let (out, said) = keep_panel_rate(Codec::H264, &unit(&hex(X264_25_TO_30[0].1))).unwrap();
        assert_eq!(out, unit(&hex(X264_25_TO_30[0].2)));
        assert_eq!(said, Redeclared { was: 25.0, now: 30 });
        // three-byte start codes come out as four; a picture with no parameter set, a set that needs nothing and a stream of another codec are not touched
        let mut short = vec![0, 0, 1];
        short.extend_from_slice(&hex(X264_25_TO_30[0].1));
        assert_eq!(
            keep_panel_rate(Codec::H264, &short).unwrap().0,
            unit(&hex(X264_25_TO_30[0].2))[..4 + 25].to_vec()
        );
        let mut p = START_CODE.to_vec();
        p.extend_from_slice(&[0x41, 0x9a, 0x24, 1, 2, 3]);
        assert_eq!(keep_panel_rate(Codec::H264, &p), None);
        assert_eq!(
            keep_panel_rate(
                Codec::H264,
                &unit(&hex("674d401eeca05017fcb80880000003008000001e078b16cb"))
            ),
            None
        );
        assert_eq!(
            keep_panel_rate(Codec::H265, &unit(&hex(X264_25_TO_30[0].1))),
            None
        );
        assert_eq!(keep_panel_rate(Codec::H264, &[]), None);
    }

    #[test]
    fn a_cut_or_damaged_set_is_never_a_panic_and_never_a_change_the_parser_does_not_read_back() {
        let nal = hex(X264_25_TO_30[1].1);
        for len in 0..nal.len() {
            if let Some((out, _)) = h264_sps(&nal[..len]) {
                assert!(sps::parse_h264(&out).is_ok(), "cut at {len}");
            }
        }
        for at in 1..nal.len() {
            for bit in 0..8 {
                let mut damaged = nal.clone();
                damaged[at] ^= 1 << bit;
                if let Some((out, _)) = h264_sps(&damaged) {
                    assert!(sps::parse_h264(&out).is_ok(), "bit {bit} of byte {at}");
                }
            }
        }
    }

    /// x265's sequence parameter sets at 25 and 50 fps, with what `ffmpeg -bsf:v hevc_metadata=tick_rate=30/1` (60/1 for 50 fps) writes over
    /// them: the same bytes but for `time_scale`. (name, set, the filter's set, the rate declared now) `tests/fixtures/rate-vectors.py x265`.
    const X265: &[(&str, &str, &str, u32)] = &[
        (
            "720p",
            "42010101600000030090000003000003005da00280802d165959a4932bc05a020000030002000003003210",
            "42010101600000030090000003000003005da00280802d165959a4932bc05a020000030002000003003c10",
            30,
        ),
        (
            "1080p, B pictures",
            "420101016000000300900000030000030078a003c08010e596566924caf0168080000003008000000c84",
            "420101016000000300900000030000030078a003c08010e596566924caf0168080000003008000000f04",
            30,
        ),
        (
            "4K",
            "420101016000000300900000030000030096a001e020021c596566924caf016808000003000800000300c840",
            "420101016000000300900000030000030096a001e020021c596566924caf016808000003000800000300f040",
            30,
        ),
        (
            "two temporal layers",
            "42010101600000030090000003000003005da00280802d16595964932bc05a020000030002000003003210",
            "42010101600000030090000003000003005da00280802d16595964932bc05a020000030002000003003c10",
            30,
        ),
        (
            "three temporal layers",
            "42010101600000030090000003000003005da00280802d1659589924caf0168080000003008000000c84",
            "42010101600000030090000003000003005da00280802d1659589924caf0168080000003008000000f04",
            30,
        ),
        (
            "scaling lists on",
            "42010101600000030090000003000003005da00280802d165959a49395e02d010000030001000003001908",
            "42010101600000030090000003000003005da00280802d165959a49395e02d010000030001000003001e08",
            30,
        ),
        (
            "aspect ratio and colour description",
            "42010101600000030090000003000003005da00280802d165959a4932bc39a808080820000030002000003003210",
            "42010101600000030090000003000003005da00280802d165959a4932bc39a808080820000030002000003003c10",
            30,
        ),
        (
            "50 fps",
            "420101016000000300900000030000030078a00280802d165959a4932bc05a020000030002000003006410",
            "420101016000000300900000030000030078a00280802d165959a4932bc05a020000030002000003007810",
            60,
        ),
    ];

    /// Sets built bit by bit (sub-layers, scaling lists, PCM, reference sets predicted from each other, long-term pictures, a VUI with
    /// everything in it), with what the same filter writes over them. (name, set, the filter's set, the rate declared now)
    /// `tests/fixtures/rate-vectors.py h265` makes them, and says which the filter refuses.
    const H265_SHAPES: &[(&str, &str, &str, u32)] = &[
        (
            "two sub-layers",
            "42010501600000030090000003000003005dd00001600000030090000003000003005a5aa00280802d16595cae5792449ae6a020202080000003008000000c84",
            "42010501600000030090000003000003005dd00001600000030090000003000003005a5aa00280802d16595cae5792449ae6a020202080000003008000000f04",
            30,
        ),
        (
            "two sub-layers, 50 fps",
            "42010501600000030090000003000003005dd00001600000030090000003000003005a5aa00280802d16595cae5792449ae6a020202080000003008000001904",
            "42010501600000030090000003000003005dd00001600000030090000003000003005a5aa00280802d16595cae5792449ae6a020202080000003008000001e04",
            60,
        ),
        (
            "three sub-layers, one ordering",
            "42010701600000030090000003000003005d6c005a016000000300900000030000030001600000030090000003000003005aa00280802d16515e49126b8040000003004000000642",
            "42010701600000030090000003000003005d6c005a016000000300900000030000030001600000030090000003000003005aa00280802d16515e49126b8040000003004000000782",
            30,
        ),
        (
            "scaling list data",
            "42010101600000030090000003000003005da00280802d16595e4912dafd7ead7ebf56bf5fad7ebf5fafd7ebf5fafd7e9afd7ebf5fafd7ebf5fafd35fafd7ebf5fafd7ebf5fa5957ebf5fafd7ebf5fafd7e9957ebf5fafd7ebf5fafd7e9957ebf5fafd7ebf5fafd7ea55fafd7ebf5fafd7ebf5fa75ffe0008000602000000300200000030321",
            "42010101600000030090000003000003005da00280802d16595e4912dafd7ead7ebf56bf5fad7ebf5fafd7ebf5fafd7e9afd7ebf5fafd7ebf5fafd35fafd7ebf5fafd7ebf5fa5957ebf5fafd7ebf5fafd7e9957ebf5fafd7ebf5fafd7e9957ebf5fafd7ebf5fafd7ea55fafd7ebf5fafd7ebf5fa75ffe00080006020000003002000000303c1",
            30,
        ),
        (
            "PCM",
            "42010101600000030090000003000003005da00280802d16595e4912777bdc531ee000000300200000030321",
            "42010101600000030090000003000003005da00280802d16595e4912777bdc531ee0000003002000000303c1",
            30,
        ),
        (
            "short-term reference sets",
            "42010101600000030090000003000003005da00280802d16595e491266b9ad770080000003008000000c84",
            "42010101600000030090000003000003005da00280802d16595e491266b9ad770080000003008000000f04",
            30,
        ),
        (
            "reference sets predicted from each other",
            "42010101600000030090000003000003005da00280802d16595e4912623ebf751b804000000300400000064bb4100820",
            "42010101600000030090000003000003005da00280802d16595e4912623ebf751b804000000300400000078bb4100820",
            30,
        ),
        (
            "reference sets predicted from each other, 50 fps",
            "42010101600000030090000003000003005da00280802d16595e4912623ebf751b8040000003004000000c8bb4100820",
            "42010101600000030090000003000003005da00280802d16595e4912623ebf751b8040000003004000000f0bb4100820",
            60,
        ),
        (
            "long-term pictures",
            "42010101600000030090000003000003005da00280802d16595e491264bd81f21c0a0000030002000003003388",
            "42010101600000030090000003000003005da00280802d16595e491264bd81f21c0a0000030002000003003d88",
            30,
        ),
        (
            "cropped, with every part of the VUI",
            "42010101600000030090000003000003005da00280802d1f2e595e49126bffc0010000fd404040698f7000000300100000030192ed040208",
            "42010101600000030090000003000003005da00280802d1f2e595e49126bffc0010000fd404040698f70000003001000000301e2ed040208",
            30,
        ),
    ];

    #[test]
    fn h265_sequence_parameter_sets_are_written_as_ffmpegs_bitstream_filter_writes_them() {
        for (name, from, to, now) in X265.iter().chain(H265_SHAPES) {
            let (out, said) =
                h265_sps(&hex(from)).unwrap_or_else(|| panic!("{name}: not re-declared"));
            assert_eq!(hex(to), out, "{name}");
            assert_eq!(said.now, *now, "{name}");
            assert_eq!(said.was, if *now == 30 { 25.0 } else { 50.0 }, "{name}");
        }
    }

    #[test]
    fn an_h265_set_at_30_fps_or_with_no_timing_is_left_as_it_is() {
        // x265 at 30 fps, with its VUI timing switched off, and the filter's own 30
        assert_eq!(
            h265_sps(&hex(
                "42010101600000030090000003000003005da00280802d165959a4932bc05a020000030002000003003c10"
            )),
            None
        );
        assert_eq!(
            h265_sps(&hex(
                "42010101600000030090000003000003005da00280802d165959a4932bc05a0020"
            )),
            None
        );
    }

    #[test]
    fn an_h265_video_parameter_set_with_timing_is_re_declared_and_one_without_is_left_alone() {
        // the filter writes a VPS's timing too; x265 writes none
        let at = |rate: &str| {
            hex(&format!(
                "40010c01ffff01600000030090000003000003005d95980c0000030004000003{rate}40"
            ))
        };
        let (out, said) = h265_vps(&at("0065")).unwrap();
        assert_eq!((out, said), (at("0079"), Redeclared { was: 25.0, now: 30 }));
        assert_eq!(h265_vps(&at("0079")), None, "30");
        assert_eq!(
            h265_vps(&hex("40010c01ffff01600000030090000003000003005d959809")),
            None
        );
        let (out, said) = h265_vps(&hex(
            "40010c01ffff01600000030090000003000003007895980c000003000400000300c940",
        ))
        .unwrap();
        assert_eq!(
            out,
            hex("40010c01ffff01600000030090000003000003007895980c000003000400000300f140")
        );
        assert_eq!(said.now, 60);
    }

    #[test]
    fn an_h265_key_picture_has_its_video_and_sequence_parameter_sets_re_declared() {
        let vps = hex("40010c01ffff01600000030090000003000003005d95980c0000030004000003006540");
        let pps = hex("4401c172b46240");
        let idr = [0x26, 0x01, 0xaf, 0x08, 0x40];
        let unit = |vps: &[u8], sps: &[u8]| {
            let mut au = Vec::new();
            for nal in [vps, sps, &pps[..], &idr[..]] {
                au.extend_from_slice(&START_CODE);
                au.extend_from_slice(nal);
            }
            au
        };
        let (out, said) = keep_panel_rate(Codec::H265, &unit(&vps, &hex(X265[0].1))).unwrap();
        assert_eq!(
            out,
            unit(
                &hex("40010c01ffff01600000030090000003000003005d95980c0000030004000003007940"),
                &hex(X265[0].2)
            )
        );
        assert_eq!(said, Redeclared { was: 25.0, now: 30 });
        // a set with no timing of its own beside one with: the one that has it is changed
        let plain = hex("40010c01ffff01600000030090000003000003005d959809");
        assert_eq!(
            keep_panel_rate(Codec::H265, &unit(&plain, &hex(X265[0].1)))
                .unwrap()
                .0,
            unit(&plain, &hex(X265[0].2))
        );
        assert_eq!(
            keep_panel_rate(Codec::H264, &unit(&vps, &hex(X265[0].1))),
            None,
            "not an H.264 stream's sets"
        );
    }

    #[test]
    fn a_cut_or_damaged_h265_set_is_never_a_panic_and_never_a_change_the_parser_does_not_read_back()
    {
        for (_, from, ..) in [X265[1], H265_SHAPES[3], H265_SHAPES[6]] {
            let nal = hex(from);
            for len in 0..nal.len() {
                if let Some((out, _)) = h265_sps(&nal[..len]) {
                    assert!(sps::parse_h265(&out).is_ok(), "cut at {len}");
                }
            }
            for at in 2..nal.len() {
                for bit in 0..8 {
                    let mut damaged = nal.clone();
                    damaged[at] ^= 1 << bit;
                    if let Some((out, _)) = h265_sps(&damaged) {
                        assert!(sps::parse_h265(&out).is_ok(), "bit {bit} of byte {at}");
                    }
                }
            }
        }
    }

    /// A sequence parameter set built bit by bit: `lead` is the `log2_max_frame_num_minus4` that moves what follows, `overscan` a flag that
    /// moves the VUI's timing by a bit.
    #[derive(Clone, Debug)]
    struct Sps {
        profile: u32,
        chroma: u32,
        scaling: bool,
        lead: u32,
        poc: u32,
        frame_mbs_only: bool,
        crop: bool,
        extended_sar: bool,
        overscan: bool,
        signal: bool,
        chroma_loc: bool,
        vui: Vui,
    }

    #[derive(Clone, Copy, Debug)]
    enum Vui {
        None,
        NoTiming,
        Timing(u32, u32),
    }

    impl Default for Sps {
        fn default() -> Self {
            Sps {
                profile: 66,
                chroma: 1,
                scaling: false,
                lead: 0,
                poc: 0,
                frame_mbs_only: true,
                crop: false,
                extended_sar: false,
                overscan: false,
                signal: false,
                chroma_loc: false,
                vui: Vui::Timing(1, 50),
            }
        }
    }

    #[derive(Default)]
    struct Writer {
        bits: Vec<bool>,
    }

    impl Writer {
        fn put(&mut self, n: u32, v: u64) {
            for i in (0..n).rev() {
                self.bits.push((v >> i) & 1 == 1);
            }
        }
        fn ue(&mut self, v: u32) {
            let x = u64::from(v) + 1;
            let len = 64 - x.leading_zeros();
            self.put(len - 1, 0);
            self.put(len, x);
        }
        fn se(&mut self, v: i32) {
            self.ue(if v > 0 {
                (2 * v - 1) as u32
            } else {
                (-2 * v) as u32
            });
        }
    }

    impl Sps {
        /// The NAL unit, escaped, with its header byte.
        fn nal(&self) -> Vec<u8> {
            let mut w = Writer::default();
            w.put(8, u64::from(self.profile));
            w.put(8, 0);
            w.put(8, 30);
            w.ue(0);
            if matches!(self.profile, 100 | 110 | 122 | 244) {
                w.ue(self.chroma);
                if self.chroma == 3 {
                    w.put(1, 0);
                }
                w.ue(0);
                w.ue(0);
                w.put(1, 0);
                w.put(1, u64::from(self.scaling));
                if self.scaling {
                    // the first and the last list are coded, as a delta of -8 from 8: next_scale 0, the default matrix, and no more of that list
                    let lists = if self.chroma != 3 { 8 } else { 12 };
                    for i in 0..lists {
                        let coded = i == 0 || i == lists - 1;
                        w.put(1, u64::from(coded));
                        if coded {
                            w.se(-8);
                        }
                    }
                }
            }
            w.ue(self.lead);
            w.ue(self.poc);
            match self.poc {
                0 => w.ue(2),
                1 => {
                    w.put(1, 0);
                    w.se(0);
                    w.se(0);
                    w.ue(2);
                    w.se(2);
                    w.se(2);
                }
                _ => {}
            }
            w.ue(1);
            w.put(1, 0);
            w.ue(39);
            w.ue(22);
            w.put(1, u64::from(self.frame_mbs_only));
            if !self.frame_mbs_only {
                w.put(1, 0);
            }
            w.put(1, 1);
            w.put(1, u64::from(self.crop));
            if self.crop {
                for v in [0, 0, 0, 4] {
                    w.ue(v);
                }
            }
            match self.vui {
                Vui::None => w.put(1, 0),
                Vui::NoTiming | Vui::Timing(..) => {
                    w.put(1, 1);
                    w.put(1, u64::from(self.extended_sar));
                    if self.extended_sar {
                        w.put(8, 255);
                        w.put(32, 0x0001_0001);
                    }
                    w.put(1, u64::from(self.overscan));
                    if self.overscan {
                        w.put(1, 0);
                    }
                    w.put(1, u64::from(self.signal));
                    if self.signal {
                        w.put(3, 5);
                        w.put(1, 0);
                        w.put(1, 1);
                        w.put(24, 0x01_01_01);
                    }
                    w.put(1, u64::from(self.chroma_loc));
                    if self.chroma_loc {
                        w.ue(1);
                        w.ue(1);
                    }
                    match self.vui {
                        Vui::Timing(units, scale) => {
                            w.put(1, 1);
                            w.put(32, u64::from(units));
                            w.put(32, u64::from(scale));
                            w.put(1, 1);
                        }
                        _ => w.put(1, 0),
                    }
                    // no HRD, no picture structure, no bitstream restriction
                    w.put(4, 0);
                }
            }
            // rbsp_trailing_bits
            w.bits.push(true);
            while w.bits.len() % 8 != 0 {
                w.bits.push(false);
            }
            let rbsp: Vec<u8> = w
                .bits
                .chunks(8)
                .map(|c| c.iter().fold(0u8, |a, &b| (a << 1) | u8::from(b)))
                .collect();
            let mut nal = vec![0x67];
            nal.extend(escape(&rbsp));
            nal
        }
    }
}
