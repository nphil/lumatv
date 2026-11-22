//! The colour tags of the video the cloud is sending, read from its keyframes: the one place on the TV path that can say whether a
//! stream really is HDR, because nothing downstream of the feeder reports it back.
//!
//! GFN states a stream's colour inside the codec's own headers, as H.273 code points (primaries, transfer characteristic, matrix)
//! and the full-range flag: in the HEVC sequence parameter set's VUI and in the AV1 sequence header's `color_config`. A keyframe
//! whose parameter set differs from the one read last is parsed as far as those fields, and a new result goes to the engine's side
//! socket as one `colour` line, and to the runtime, which sets the TV's HDR mode from the transfer characteristic (`HdrMode`). Frames
//! are only looked at: what is decoded, forwarded, dropped and timed does not change. H.264 is
//! not read (the capability report offers it as 8-bit only), and a frame that is not a keyframe costs the one branch that skips it.
//!
//! The parsers work in place on the frame's bytes, never allocate and never panic; a header they cannot follow is not reported.
use std::sync::Mutex;

use opennow_streamer_protocol::log::log_async;

use crate::types::{EncodedFrame, MediaCodec};

/// What a stream says about its colour: the code points as the encoder wrote them, 2 ("unspecified") for each when it wrote none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colour {
    primaries: u8,
    transfer: u8,
    matrix: u8,
    full_range: bool,
}

impl Colour {
    const fn unspecified(full_range: bool) -> Self {
        Self { primaries: 2, transfer: 2, matrix: 2, full_range }
    }

    pub const fn transfer(self) -> u8 {
        self.transfer
    }

    pub const fn hdr_mode(self) -> HdrMode {
        HdrMode::of(self.transfer)
    }
}

/// What a stream's transfer characteristic asks of the TV: PQ (SMPTE ST 2084, code point 16) is HDR10 and HLG (ARIB STD-B67, 18) is
/// HLG. Every other value, BT.709 and "unspecified" included, is SDR: the primaries and matrix do not matter here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HdrMode {
    Sdr,
    Hdr10,
    Hlg,
}

impl HdrMode {
    pub const fn of(transfer: u8) -> Self {
        match transfer {
            16 => Self::Hdr10,
            18 => Self::Hlg,
            _ => Self::Sdr,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Sdr => "SDR",
            Self::Hdr10 => "HDR10",
            Self::Hlg => "HLG",
        }
    }
}

/// Reads the colour tags off one media session's keyframes.
#[derive(Default)]
pub struct ColourWatch {
    state: Mutex<Seen>,
}

#[derive(Default)]
struct Seen {
    /// The parameter set read last (the SPS NAL unit, or the sequence header OBU's payload). Encoders repeat it on every keyframe;
    /// the same bytes are not read again.
    parameter_set: Vec<u8>,
    /// The last colour sent: a changed parameter set that says the same thing stays quiet.
    reported: Option<Colour>,
}

impl ColourWatch {
    /// Hands `send` the `colour` line when `frame` is a keyframe whose colour tags are new or have changed, and returns that colour. Any
    /// other frame returns at the first test, and `send` runs after the lock is released.
    pub fn observe(&self, frame: &EncodedFrame, send: impl FnOnce(String)) -> Option<Colour> {
        if !frame.keyframe {
            return None;
        }
        let (codec, changed) = match frame.codec {
            MediaCodec::H265 => ("h265", self.changed(hevc_sps(&frame.data), hevc_colour)),
            MediaCodec::Av1 => ("av1", self.changed(av1_sequence_header(&frame.data), av1_colour)),
            _ => return None,
        };
        let colour = changed?;
        send(colour_line(codec, colour));
        Some(colour)
    }

    /// The colour of `set` when it is a parameter set not read before and the colour is not the one already reported.
    fn changed(&self, set: Option<&[u8]>, read: fn(&[u8]) -> Option<Colour>) -> Option<Colour> {
        let set = set?;
        let mut seen = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if seen.parameter_set == set {
            return None;
        }
        seen.parameter_set.clear();
        seen.parameter_set.extend_from_slice(set);
        let Some(colour) = read(set) else {
            log_async("WARN", "ndl", &format!("the stream's colour tags could not be read from its {}-byte parameter set", set.len()));
            return None;
        };
        if seen.reported == Some(colour) {
            return None;
        }
        seen.reported = Some(colour);
        Some(colour)
    }
}

/// The wire line, with the keys in a fixed order. `codec` is the stream's codec label (`MediaVideoCodec::label`).
fn colour_line(codec: &str, c: Colour) -> String {
    format!(
        r#"{{"stream":"colour","codec":"{codec}","primaries":{},"transfer":{},"matrix":{},"full_range":{}}}"#,
        c.primaries, c.transfer, c.matrix, c.full_range
    )
}

/// A reader of MSB-first bit fields. With `unescape` it undoes an H.26x NAL unit's emulation prevention as it goes (the 0x03 after
/// two zero bytes is not data), so the unit is read in place.
struct Bits<'a> {
    data: &'a [u8],
    next: usize,
    current: u8,
    left: u8,
    zeros: u8,
    unescape: bool,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8], unescape: bool) -> Self {
        Self { data, next: 0, current: 0, left: 0, zeros: 0, unescape }
    }

    fn fetch(&mut self) -> Option<u8> {
        let mut byte = *self.data.get(self.next)?;
        self.next += 1;
        if self.unescape {
            if self.zeros == 2 && byte == 3 {
                byte = *self.data.get(self.next)?;
                self.next += 1;
                self.zeros = 0;
            }
            self.zeros = if byte == 0 { (self.zeros + 1).min(2) } else { 0 };
        }
        Some(byte)
    }

    fn bit(&mut self) -> Option<u32> {
        if self.left == 0 {
            self.current = self.fetch()?;
            self.left = 8;
        }
        self.left -= 1;
        Some(u32::from((self.current >> self.left) & 1))
    }

    fn flag(&mut self) -> Option<bool> {
        Some(self.bit()? == 1)
    }

    /// An unsigned field of `n` <= 32 bits.
    fn bits(&mut self, n: u32) -> Option<u32> {
        let mut value = 0;
        for _ in 0..n {
            value = (value << 1) | self.bit()?;
        }
        Some(value)
    }

    fn byte(&mut self) -> Option<u8> {
        u8::try_from(self.bits(8)?).ok()
    }

    fn skip(&mut self, n: u32) -> Option<()> {
        for _ in 0..n {
            self.bit()?;
        }
        Some(())
    }

    /// An Exp-Golomb code word, ue(v): the value of up to 32 bits. Skipping se(v) is the same call, the two share code words.
    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        let value = (1u64 << zeros) - 1 + u64::from(self.bits(zeros)?);
        u32::try_from(value).ok()
    }
}

/// The first SPS NAL unit of an Annex B access unit, header included and still escaped. An SPS comes before the first slice of its
/// access unit, so the search stops at the first VCL NAL unit instead of scanning the picture.
fn hevc_sps(au: &[u8]) -> Option<&[u8]> {
    let mut start = next_start_code(au, 0)?;
    loop {
        let nal = start + 3;
        let header = au.get(nal..nal + 2)?;
        let kind = (header[0] >> 1) & 0x3f;
        if header[0] & 0x80 != 0 || kind < 32 {
            return None;
        }
        let end = next_start_code(au, nal + 2).unwrap_or(au.len());
        // nuh_layer_id 0: an enhancement layer's SPS describes another picture.
        if kind == 33 && header[0] & 1 == 0 && header[1] >> 3 == 0 {
            return Some(without_trailing_zeros(&au[nal..end]));
        }
        if end == au.len() {
            return None;
        }
        start = end;
    }
}

/// Where the next `00 00 01` is, at or after `from`. The zero byte of a four-byte start code ends up as the previous unit's tail.
fn next_start_code(data: &[u8], from: usize) -> Option<usize> {
    let tail = data.get(from..)?;
    tail.windows(3).position(|w| w == [0, 0, 1]).map(|p| from + p)
}

fn without_trailing_zeros(mut nal: &[u8]) -> &[u8] {
    while let [rest @ .., 0] = nal {
        nal = rest;
    }
    nal
}

/// The colour tags in an HEVC SPS (H.265 7.3.2.2), read through the VUI's `video_signal_type`. What precedes the VUI is only skipped,
/// but its length depends on earlier values, so every branch of it is followed.
fn hevc_colour(nal: &[u8]) -> Option<Colour> {
    let mut r = Bits::new(nal.get(2..)?, true);
    r.skip(4)?; // sps_video_parameter_set_id
    let max_sub_layers_minus1 = r.bits(3)?;
    if max_sub_layers_minus1 > 6 {
        return None;
    }
    r.skip(1)?; // sps_temporal_id_nesting_flag
    profile_tier_level(&mut r, max_sub_layers_minus1)?;
    r.ue()?; // sps_seq_parameter_set_id
    if r.ue()? == 3 {
        r.skip(1)?; // chroma_format_idc 3: separate_colour_plane_flag
    }
    r.ue()?; // pic_width_in_luma_samples
    r.ue()?; // pic_height_in_luma_samples
    if r.flag()? {
        for _ in 0..4 {
            r.ue()?; // conformance window offsets
        }
    }
    r.ue()?; // bit_depth_luma_minus8
    r.ue()?; // bit_depth_chroma_minus8
    let poc_lsb_bits = r.ue()?.checked_add(4).filter(|n| *n <= 16)?; // log2_max_pic_order_cnt_lsb_minus4 + 4
    let first = if r.flag()? { 0 } else { max_sub_layers_minus1 }; // sps_sub_layer_ordering_info_present_flag
    for _ in first..=max_sub_layers_minus1 {
        for _ in 0..3 {
            r.ue()?; // sps_max_dec_pic_buffering_minus1, sps_max_num_reorder_pics, sps_max_latency_increase_plus1
        }
    }
    for _ in 0..6 {
        r.ue()?; // coding and transform block sizes, transform hierarchy depths
    }
    if r.flag()? && r.flag()? {
        scaling_list_data(&mut r)?; // scaling_list_enabled_flag, sps_scaling_list_data_present_flag
    }
    r.skip(2)?; // amp_enabled_flag, sample_adaptive_offset_enabled_flag
    if r.flag()? {
        r.skip(8)?; // pcm_enabled_flag: the two sample bit depths
        r.ue()?; // log2_min_pcm_luma_coding_block_size_minus3
        r.ue()?; // log2_diff_max_min_pcm_luma_coding_block_size
        r.skip(1)?; // pcm_loop_filter_disabled_flag
    }
    let sets = r.ue()?; // num_short_term_ref_pic_sets
    if sets > 64 {
        return None;
    }
    let mut previous = None;
    for _ in 0..sets {
        previous = Some(short_term_ref_pic_set(&mut r, previous.as_ref())?);
    }
    if r.flag()? {
        let n = r.ue()?; // long_term_ref_pics_present_flag: num_long_term_ref_pics_sps
        if n > 32 {
            return None;
        }
        for _ in 0..n {
            r.skip(poc_lsb_bits + 1)?; // lt_ref_pic_poc_lsb_sps, used_by_curr_pic_lt_sps_flag
        }
    }
    r.skip(2)?; // sps_temporal_mvp_enabled_flag, strong_intra_smoothing_enabled_flag
    if !r.flag()? {
        return Some(Colour::unspecified(false)); // no VUI
    }
    if r.flag()? && r.byte()? == 255 {
        r.skip(32)?; // aspect_ratio_idc EXTENDED_SAR: sar_width, sar_height
    }
    if r.flag()? {
        r.skip(1)?; // overscan_info_present_flag: overscan_appropriate_flag
    }
    if !r.flag()? {
        return Some(Colour::unspecified(false)); // no video_signal_type
    }
    r.skip(3)?; // video_format
    let full_range = r.flag()?;
    if !r.flag()? {
        return Some(Colour::unspecified(full_range)); // no colour_description
    }
    Some(Colour { primaries: r.byte()?, transfer: r.byte()?, matrix: r.byte()?, full_range })
}

/// `profile_tier_level(1, max_sub_layers_minus1)`: 88 bits of general profile and a level byte, then the same for each sub-layer
/// that says it has them.
fn profile_tier_level(r: &mut Bits, max_sub_layers_minus1: u32) -> Option<()> {
    r.skip(88 + 8)?;
    let (mut profile, mut level) = (0u8, 0u8);
    for i in 0..max_sub_layers_minus1 {
        profile |= u8::from(r.flag()?) << i;
        level |= u8::from(r.flag()?) << i;
    }
    if max_sub_layers_minus1 > 0 {
        r.skip(2 * (8 - max_sub_layers_minus1))?; // reserved_zero_2bits up to eight sub-layers
    }
    for i in 0..max_sub_layers_minus1 {
        if (profile >> i) & 1 == 1 {
            r.skip(88)?;
        }
        if (level >> i) & 1 == 1 {
            r.skip(8)?;
        }
    }
    Some(())
}

fn scaling_list_data(r: &mut Bits) -> Option<()> {
    for size_id in 0..4u32 {
        // The 32x32 sizes have luma lists only, for intra and for inter.
        for _ in (0..6).step_by(if size_id == 3 { 3 } else { 1 }) {
            if r.flag()? {
                if size_id > 1 {
                    r.ue()?; // scaling_list_dc_coef_minus8
                }
                for _ in 0..(1u32 << (4 + (size_id << 1))).min(64) {
                    r.ue()?; // scaling_list_delta_coef
                }
            } else {
                r.ue()?; // scaling_list_pred_matrix_id_delta
            }
        }
    }
    Some(())
}

/// The POC distances of one short-term reference picture set. A set in the SPS may be predicted from the one before it, and it
/// carries a flag pair for every picture of that one, so what is kept of a set is what its successor has to count.
#[derive(Clone, Copy, Default)]
struct RefPicSet {
    before: [i64; 16],
    n_before: usize,
    after: [i64; 16],
    n_after: usize,
}

/// `st_ref_pic_set(idx)` of an SPS. `previous` is the set `idx - 1`, `None` for the first set (which has no prediction flag).
fn short_term_ref_pic_set(r: &mut Bits, previous: Option<&RefPicSet>) -> Option<RefPicSet> {
    let mut set = RefPicSet::default();
    let predicted = match previous {
        Some(prev) => r.flag()?.then_some(prev), // inter_ref_pic_set_prediction_flag
        None => None,
    };
    if let Some(prev) = predicted {
        // The set follows from the previous one shifted by delta_rps (7-61, 7-62).
        let negative = r.flag()?; // delta_rps_sign
        let magnitude = i64::from(r.ue()?) + 1; // abs_delta_rps_minus1 + 1
        let delta_rps = if negative { -magnitude } else { magnitude };
        let total = prev.n_before + prev.n_after;
        let mut use_delta = [true; 17];
        for keep in use_delta.iter_mut().take(total + 1) {
            if !r.flag()? {
                *keep = r.flag()?; // used_by_curr_pic_flag 0: use_delta_flag
            }
        }
        for j in (0..prev.n_after).rev() {
            let d = prev.after[j] + delta_rps;
            if d < 0 && use_delta[prev.n_before + j] {
                *set.before.get_mut(set.n_before)? = d;
                set.n_before += 1;
            }
        }
        if delta_rps < 0 && use_delta[total] {
            *set.before.get_mut(set.n_before)? = delta_rps;
            set.n_before += 1;
        }
        for j in 0..prev.n_before {
            let d = prev.before[j] + delta_rps;
            if d < 0 && use_delta[j] {
                *set.before.get_mut(set.n_before)? = d;
                set.n_before += 1;
            }
        }
        for j in (0..prev.n_before).rev() {
            let d = prev.before[j] + delta_rps;
            if d > 0 && use_delta[j] {
                *set.after.get_mut(set.n_after)? = d;
                set.n_after += 1;
            }
        }
        if delta_rps > 0 && use_delta[total] {
            *set.after.get_mut(set.n_after)? = delta_rps;
            set.n_after += 1;
        }
        for j in 0..prev.n_after {
            let d = prev.after[j] + delta_rps;
            if d > 0 && use_delta[prev.n_before + j] {
                *set.after.get_mut(set.n_after)? = d;
                set.n_after += 1;
            }
        }
    } else {
        let n_before = usize::try_from(r.ue()?).ok()?; // num_negative_pics
        let n_after = usize::try_from(r.ue()?).ok()?; // num_positive_pics
        if n_before > 16 || n_after > 16 - n_before {
            return None;
        }
        let mut poc = 0;
        for slot in &mut set.before[..n_before] {
            poc -= i64::from(r.ue()?) + 1; // delta_poc_s0_minus1
            *slot = poc;
            r.skip(1)?; // used_by_curr_pic_s0_flag
        }
        poc = 0;
        for slot in &mut set.after[..n_after] {
            poc += i64::from(r.ue()?) + 1; // delta_poc_s1_minus1
            *slot = poc;
            r.skip(1)?; // used_by_curr_pic_s1_flag
        }
        (set.n_before, set.n_after) = (n_before, n_after);
    }
    (set.n_before + set.n_after <= 16).then_some(set)
}

/// The payload of the first sequence header OBU of an AV1 temporal unit. The units arrive in the low-overhead form, every OBU
/// carrying its size, after the 8 or 20 byte GFN header.
fn av1_sequence_header(tu: &[u8]) -> Option<&[u8]> {
    let mut at = 0;
    while let Some(&header) = tu.get(at) {
        if header & 0x80 != 0 {
            return None; // obu_forbidden_bit
        }
        let mut payload = at + 1 + usize::from(header & 4 != 0); // obu_extension_flag: one more header byte
        let size = if header & 2 != 0 { leb128(tu, &mut payload)? } else { tu.len().checked_sub(payload)? };
        let end = payload.checked_add(size)?;
        if (header >> 3) & 0xf == 1 {
            return tu.get(payload..end);
        }
        at = end;
    }
    None
}

fn leb128(data: &[u8], at: &mut usize) -> Option<usize> {
    let mut value = 0u64;
    for i in 0..8 {
        let byte = *data.get(*at)?;
        *at += 1;
        value |= u64::from(byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            return usize::try_from(value).ok();
        }
    }
    None
}

/// The colour tags in an AV1 sequence header (AV1 5.5), read through `color_config`.
fn av1_colour(seq: &[u8]) -> Option<Colour> {
    let mut r = Bits::new(seq, false);
    let profile = r.bits(3)?;
    if profile > 2 {
        return None;
    }
    r.skip(1)?; // still_picture
    let reduced = r.flag()?; // reduced_still_picture_header
    if reduced {
        r.skip(5)?; // seq_level_idx[0]
    } else {
        let mut delay_bits = None; // decoder_model_info's buffer_delay_length_minus_1 + 1, when there is one
        if r.flag()? {
            r.skip(64)?; // timing_info_present_flag: num_units_in_display_tick, time_scale
            if r.flag()? {
                r.ue()?; // equal_picture_interval: num_ticks_per_picture_minus_1, a uvlc() with the same code words
            }
            if r.flag()? {
                delay_bits = Some(r.bits(5)? + 1); // decoder_model_info_present_flag
                r.skip(32 + 5 + 5)?; // num_units_in_decoding_tick, buffer_removal_time_length_minus_1, frame_presentation_time_length_minus_1
            }
        }
        let display_delay = r.flag()?; // initial_display_delay_present_flag
        let operating_points = r.bits(5)? + 1; // operating_points_cnt_minus_1 + 1
        for _ in 0..operating_points {
            r.skip(12)?; // operating_point_idc
            if r.bits(5)? > 7 {
                r.skip(1)?; // seq_level_idx: seq_tier
            }
            if let Some(n) = delay_bits {
                if r.flag()? {
                    r.skip(2 * n + 1)?; // decoder_model_present_for_this_op: decoder_buffer_delay, encoder_buffer_delay, low_delay_mode_flag
                }
            }
            if display_delay && r.flag()? {
                r.skip(4)?; // initial_display_delay_present_for_this_op: initial_display_delay_minus_1
            }
        }
    }
    let width_bits = r.bits(4)? + 1;
    let height_bits = r.bits(4)? + 1;
    r.skip(width_bits + height_bits)?; // max_frame_width_minus_1, max_frame_height_minus_1
    if !reduced && r.flag()? {
        r.skip(7)?; // frame_id_numbers_present_flag: delta_frame_id_length_minus_2, additional_frame_id_length_minus_1
    }
    r.skip(3)?; // use_128x128_superblock, enable_filter_intra, enable_intra_edge_filter
    if !reduced {
        r.skip(4)?; // enable_interintra_compound, enable_masked_compound, enable_warped_motion, enable_dual_filter
        let order_hint = r.flag()?; // enable_order_hint
        if order_hint {
            r.skip(2)?; // enable_jnt_comp, enable_ref_frame_mvs
        }
        // seq_choose_screen_content_tools selects per frame, which counts as on; otherwise seq_force_screen_content_tools says.
        let screen_content_tools = r.flag()? || r.flag()?;
        if screen_content_tools && !r.flag()? {
            r.skip(1)?; // seq_choose_integer_mv 0: seq_force_integer_mv
        }
        if order_hint {
            r.skip(3)?; // order_hint_bits_minus_1
        }
    }
    r.skip(3)?; // enable_superres, enable_cdef, enable_restoration
    // color_config()
    let high_bitdepth = r.flag()?;
    if profile == 2 && high_bitdepth {
        r.skip(1)?; // twelve_bit
    }
    let mono_chrome = profile != 1 && r.flag()?; // profile 1 is 4:4:4 only and has no bit for it
    let (primaries, transfer, matrix) = if r.flag()? { (r.byte()?, r.byte()?, r.byte()?) } else { (2, 2, 2) };
    // BT.709 primaries with the sRGB transfer and the identity matrix is RGB, which is full range and says so by omitting the bit.
    let full_range = if !mono_chrome && (primaries, transfer, matrix) == (1, 13, 0) { true } else { r.flag()? };
    Some(Colour { primaries, transfer, matrix, full_range })
}

/// Real keyframes for the tests of what the tags decide elsewhere in the crate.
#[cfg(test)]
pub(crate) use tests::{SPS_BT709, SPS_FULL_NO_DESCRIPTION, SPS_HLG, SPS_PQ, hevc_keyframe};

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    // Parameter sets as real encoders wrote them, each with the colour that ffmpeg 5.1's header trace reads from the same stream. HEVC:
    // SPS NAL units, header included, from x265 3.5 (10-bit 4:2:0 unless the name says otherwise). AV1: sequence header payloads from
    // libaom 3.6.0, or SVT-AV1 1.4.1 where named.
    const VPS: &str = "40010c01ffff02200000030090000003000003003c959809";
    const PPS: &str = "4401c172b46240";
    pub(crate) const SPS_PQ: &str = "42010102200000030090000003000003003ca00a080b9f6d96566924caf016a1\
        22012080000003008000000f04";
    pub(crate) const SPS_HLG: &str = "42010102200000030090000003000003003ca00a080b9f6d96566924caf016a1\
        22412080000003008000000f04";
    const SPS_PQ_CTU16: &str = "42010102200000030090000003000003003ca00a080b9f6d96566abcaf016a12\
        201208000003000800000300f040";
    pub(crate) const SPS_BT709: &str = "42010101600000030090000003000003003ca00a080b9f796566924caf016a02\
        020208000003000800000300f040";
    const SPS_BT709_FULL: &str = "42010101600000030090000003000003003ca00a080b9f796566924caf016e02\
        020208000003000800000300f040";
    pub(crate) const SPS_FULL_NO_DESCRIPTION: &str = "42010101600000030090000003000003003ca00a080b9f796566924caf016c08\
        000003000800000300f040";
    const SPS_EXTENDED_SAR: &str = "42010102200000030090000003000003003ca00a080b9f6d96566924cafff001\
        b00116a122012080000003008000000f04";
    const SPS_444: &str = "4201010408000003009c0800000300003c9001410173e56cb2b349265780b509\
        1009040000030004000003007820";
    const SPS_SCALING_LISTS: &str = "42010102200000030090000003000003003ca00a080b9f6d96566924f84bfffc\
        29fffe16fffe2435fffe1cffff0f7fffffffffffffff820ffffffffffffffff0\
        45fffffffffffffffe093fffffffffffffff89051fffffffffffffffe0a17fff\
        ffffffffffff82a5fffffffffffffffe0b17fffffffffffffff120c17fffffff\
        ffffffff8325fffffffffffffffe0d17fffffffffffffff8365fffffffffffff\
        ffd5e02d424402410000030001000003001e08";
    const AV1_PQ: &str = "000000043cfeccdaf95091009040";
    const AV1_HLG: &str = "000000043cfeccdaf95091209040";
    const AV1_BT709: &str = "000000043cfeccdaf91010101040";
    const AV1_NONE: &str = "000000043cfeccdaf90040";
    const AV1_FULL: &str = "000000043cfeccdaf91010101840";
    const AV1_RGB: &str = "200000043cfeccdaf92021a004";
    const AV1_MONO: &str = "000000043cfeccdaf93010101a";
    const AV1_444: &str = "200000043cfeccdaf920202022";
    const AV1_422_10: &str = "400000043cfeccdaf94848804880";
    const AV1_TIMING_MODEL: &str = "040000000400000079780000000a530000035f915f90bc3cfeccdaf950910090\
        40";
    const AV1_TIMING_CONST: &str = "04000000040000007b400000bc3cfeccdaf95091009040";
    const AV1_FRAME_IDS: &str = "000000043cfecf81b5f2a122012080";
    const AV1_SVT_PQ: &str = "000000043cfeccfbf95091009040";

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn tags(primaries: u8, transfer: u8, matrix: u8, full_range: bool) -> Colour {
        Colour { primaries, transfer, matrix, full_range }
    }

    /// Builds a bit string field by field, for the syntax no encoder here writes.
    #[derive(Default)]
    struct BitWriter(Vec<bool>);

    impl BitWriter {
        fn u(&mut self, n: u32, value: u64) -> &mut Self {
            self.0.extend((0..n).rev().map(|i| (value >> i) & 1 == 1));
            self
        }

        fn ue(&mut self, value: u32) -> &mut Self {
            let code = u64::from(value) + 1;
            let width = 64 - code.leading_zeros();
            self.u(width - 1, 0).u(width, code)
        }

        /// The bytes, after rbsp_trailing_bits().
        fn finish(&mut self) -> Vec<u8> {
            self.0.push(true);
            self.0.resize(self.0.len().next_multiple_of(8), false);
            self.0.chunks(8).map(|byte| byte.iter().fold(0, |acc, bit| (acc << 1) | u8::from(*bit))).collect()
        }
    }

    /// An SPS NAL unit: the header, and the payload with emulation prevention applied.
    fn sps_nal(rbsp: &[u8]) -> Vec<u8> {
        let mut nal = vec![0x42, 0x01];
        let mut zeros = 0;
        for &byte in rbsp {
            if zeros == 2 && byte <= 3 {
                nal.push(3);
                zeros = 0;
            }
            nal.push(byte);
            zeros = if byte == 0 { zeros + 1 } else { 0 };
        }
        nal
    }

    /// `profile_tier_level(1, n)` with a (profile present, level present) pair given for each of the n sub-layers.
    fn write_profile_tier_level(w: &mut BitWriter, sub_layers: &[(bool, bool)]) {
        let profile = |w: &mut BitWriter| {
            w.u(2, 0).u(1, 0).u(5, 2).u(32, 0x2000_0000).u(4, 0b1001).u(44, 0);
        };
        profile(w);
        w.u(8, 120); // general_level_idc
        for &(profile_present, level_present) in sub_layers {
            w.u(1, u64::from(profile_present)).u(1, u64::from(level_present));
        }
        if !sub_layers.is_empty() {
            w.u(2 * (8 - u32::try_from(sub_layers.len()).unwrap()), 0); // reserved_zero_2bits up to eight sub-layers
        }
        for &(profile_present, level_present) in sub_layers {
            if profile_present {
                profile(w);
            }
            if level_present {
                w.u(8, 90);
            }
        }
    }

    /// Three short-term reference picture sets, the second and third predicted from the one before.
    fn write_reference_picture_sets(w: &mut BitWriter) {
        // 0: S0 = -1 (used), -3; S1 = +2 (used)
        w.ue(2).ue(1).ue(0).u(1, 1).ue(1).u(1, 0).ue(1).u(1, 1);
        // 1: from 0, delta_rps -1. Four flag groups, one for each picture of 0 and one for delta_rps itself: the first two drop their
        // picture, the last two keep theirs, as +1 and -1.
        w.u(1, 1).u(1, 1).ue(0).u(6, 0b00_0011);
        // 2: from 1, delta_rps +2. Three flag groups, the second with a use_delta_flag, all kept: +1, +2 and +3.
        w.u(1, 1).u(1, 0).ue(1).u(4, 0b1011);
    }

    /// An SPS with the syntax x265 never writes: sub-layers with their own profile and level, short-term reference picture sets
    /// predicted from one another, PCM and long-term pictures. `vui` writes everything from vui_parameters_present_flag on.
    fn sps_with_every_branch(vui: fn(&mut BitWriter)) -> Vec<u8> {
        let mut w = BitWriter::default();
        w.u(4, 0).u(3, 2).u(1, 1); // sps_video_parameter_set_id, sps_max_sub_layers_minus1 = 2, sps_temporal_id_nesting_flag
        write_profile_tier_level(&mut w, &[(true, true), (false, true)]);
        w.ue(0).ue(1).ue(1920).ue(1088); // sps_seq_parameter_set_id, chroma_format_idc, picture size
        w.u(1, 1).ue(0).ue(0).ue(0).ue(4); // conformance window
        w.ue(2).ue(2).ue(4); // bit depths; log2_max_pic_order_cnt_lsb_minus4 = 4, so lt_ref_pic_poc_lsb_sps is 8 bits
        w.u(1, 1); // sps_sub_layer_ordering_info_present_flag: three sets of three
        for _ in 0..3 {
            w.ue(4).ue(2).ue(5);
        }
        w.ue(0).ue(3).ue(0).ue(3).ue(1).ue(1); // block sizes, transform hierarchy depths
        w.u(1, 0); // scaling_list_enabled_flag
        w.u(2, 0b11); // amp_enabled_flag, sample_adaptive_offset_enabled_flag
        w.u(1, 1).u(4, 7).u(4, 7).ue(0).ue(2).u(1, 1); // pcm
        w.ue(3); // num_short_term_ref_pic_sets
        write_reference_picture_sets(&mut w);
        w.u(1, 1).ue(2).u(8, 0x55).u(1, 1).u(8, 0xaa).u(1, 0); // two long-term pictures
        w.u(2, 0b11); // sps_temporal_mvp_enabled_flag, strong_intra_smoothing_enabled_flag
        vui(&mut w);
        sps_nal(&w.finish())
    }

    fn vui_with_full_range_pq(w: &mut BitWriter) {
        w.u(1, 1); // vui_parameters_present_flag
        w.u(1, 1).u(8, 255).u(16, 1234).u(16, 777); // aspect_ratio_idc EXTENDED_SAR
        w.u(1, 1).u(1, 0); // overscan_info_present_flag, overscan_appropriate_flag
        w.u(1, 1).u(3, 5).u(1, 1).u(1, 1).u(8, 9).u(8, 16).u(8, 9); // video_signal_type: full range, colour description 9/16/9
        w.u(8, 0); // the other VUI flags and sps_extension_present_flag, all off
    }

    fn vui_without_signal_type(w: &mut BitWriter) {
        w.u(1, 1).u(1, 0).u(1, 0).u(1, 0).u(8, 0);
    }

    fn no_vui(w: &mut BitWriter) {
        w.u(2, 0); // vui_parameters_present_flag, sps_extension_present_flag
    }

    /// A sequence header with reduced_still_picture_header, which leaves out the operating points and the coding tool flags.
    fn reduced_still_picture_header() -> Vec<u8> {
        let mut w = BitWriter::default();
        w.u(3, 0).u(1, 1).u(1, 1).u(5, 8); // seq_profile, still_picture, reduced_still_picture_header, seq_level_idx[0]
        w.u(4, 10).u(4, 10).u(11, 1919).u(11, 1079); // frame size
        w.u(3, 0b011); // use_128x128_superblock, enable_filter_intra, enable_intra_edge_filter
        w.u(3, 0b010); // enable_superres, enable_cdef, enable_restoration
        w.u(1, 1).u(1, 0).u(1, 1).u(8, 9).u(8, 16).u(8, 9).u(1, 1).u(2, 0).u(1, 0); // color_config: 10-bit 4:2:0, full range PQ
        w.u(1, 0); // film_grain_params_present
        w.finish()
    }

    /// A sequence header with every optional part on: timing and decoder model info, three operating points (the first with a tier bit,
    /// a decoder model and a display delay), screen content tools and integer motion vectors forced, 12-bit 4:2:2 at profile 2.
    fn operating_points_and_forced_tools() -> Vec<u8> {
        let mut w = BitWriter::default();
        w.u(3, 2).u(1, 0).u(1, 0); // seq_profile, still_picture, reduced_still_picture_header
        w.u(1, 1).u(32, 1001).u(32, 60000).u(1, 0); // timing_info, without equal_picture_interval
        w.u(1, 1).u(5, 15).u(32, 1).u(5, 9).u(5, 9); // decoder_model_info: 16-bit delays
        w.u(1, 1).u(5, 2); // initial_display_delay_present_flag; three operating points
        w.u(12, 0x103).u(5, 9).u(1, 1).u(1, 1).u(16, 0x1234).u(16, 0x4321).u(1, 1).u(1, 1).u(4, 7);
        w.u(12, 0x101).u(5, 7).u(1, 0).u(1, 0); // level 7 has no tier bit
        w.u(12, 1).u(5, 8).u(1, 0).u(1, 1).u(16, 5).u(16, 6).u(1, 0).u(1, 0); // level 8 has
        w.u(4, 11).u(4, 10).u(12, 3839).u(11, 1079); // frame size
        w.u(1, 0); // frame_id_numbers_present_flag
        w.u(3, 0b111); // use_128x128_superblock, enable_filter_intra, enable_intra_edge_filter
        w.u(4, 0b1101); // enable_interintra_compound, enable_masked_compound, enable_warped_motion, enable_dual_filter
        w.u(1, 0); // enable_order_hint: no jnt_comp, ref_frame_mvs or order_hint_bits
        w.u(1, 0).u(1, 1).u(1, 0).u(1, 1); // seq_choose_screen_content_tools 0, seq_force_screen_content_tools 1; seq_choose_integer_mv 0, seq_force_integer_mv 1
        w.u(3, 0b101); // enable_superres, enable_cdef, enable_restoration
        w.u(1, 1).u(1, 1).u(1, 0).u(1, 1).u(8, 9).u(8, 16).u(8, 9).u(1, 0); // high_bitdepth, twelve_bit, mono_chrome, colour description 9/16/9, limited range
        w.u(1, 1).u(1, 0); // subsampling_x, subsampling_y of 12-bit profile 2: 4:2:2
        w.u(1, 0); // separate_uv_delta_q
        w.u(1, 1); // film_grain_params_present
        w.finish()
    }

    fn annex_b(nals: &[&[u8]]) -> Vec<u8> {
        nals.iter().flat_map(|nal| [&[0, 0, 0, 1][..], nal].concat()).collect()
    }

    /// A keyframe access unit as the transport hands it over: parameter sets, then an IDR slice whose payload is not read.
    pub(crate) fn hevc_keyframe(sps: &str) -> Vec<u8> {
        annex_b(&[&hex(VPS), &hex(sps), &hex(PPS), &[0x26, 0x01, 0xaf, 0x08, 0x42, 0x00]])
    }

    /// A temporal unit as the transport hands it over: temporal delimiter, sequence header, a frame OBU whose payload is not read.
    fn av1_keyframe(sequence_header: &str) -> Vec<u8> {
        let header = hex(sequence_header);
        [&[0x12, 0x00, 0x0a, u8::try_from(header.len()).unwrap()][..], &header, &[0x32, 0x01, 0xaa]].concat()
    }

    fn frame(codec: MediaCodec, data: Vec<u8>, keyframe: bool) -> EncodedFrame {
        EncodedFrame {
            mid: "0".into(),
            codec,
            data: Arc::from(data),
            frame_index: None,
            timestamp: 0,
            clock_rate_hz: 90_000,
            keyframe,
            contiguous: true,
            ssrc: None,
        }
    }

    /// What `watch` sends for `frame`.
    fn observed(watch: &ColourWatch, frame: &EncodedFrame) -> Vec<String> {
        let mut lines = Vec::new();
        watch.observe(frame, |line| lines.push(line));
        lines
    }

    #[test]
    fn colour_tags_in_real_hevc_sps() {
        for (what, sps, want) in [
            ("HDR10: PQ, BT.2020", SPS_PQ, tags(9, 16, 9, false)),
            ("HLG", SPS_HLG, tags(9, 18, 9, false)),
            ("8-bit BT.709", SPS_BT709, tags(1, 1, 1, false)),
            ("full range BT.709", SPS_BT709_FULL, tags(1, 1, 1, true)),
            ("full range flag with no colour description", SPS_FULL_NO_DESCRIPTION, Colour::unspecified(true)),
            ("extended sample aspect ratio ahead of the colour", SPS_EXTENDED_SAR, tags(9, 16, 9, false)),
            ("4:4:4, which adds separate_colour_plane_flag", SPS_444, tags(9, 16, 9, false)),
            ("explicit and copied scaling lists", SPS_SCALING_LISTS, tags(9, 16, 9, false)),
        ] {
            assert_eq!(hevc_colour(&hex(sps)), Some(want), "{what}");
        }
    }

    #[test]
    fn colour_tags_in_hevc_sps_syntax_no_encoder_here_writes() {
        let full_range_pq = Some(tags(9, 16, 9, true));
        assert_eq!(hevc_colour(&sps_with_every_branch(vui_with_full_range_pq)), full_range_pq);
        assert_eq!(hevc_colour(&sps_with_every_branch(vui_without_signal_type)), Some(Colour::unspecified(false)));
        assert_eq!(hevc_colour(&sps_with_every_branch(no_vui)), Some(Colour::unspecified(false)));
    }

    #[test]
    fn profile_tier_level_skips_exactly_what_its_sub_layers_carry() {
        for count in 0..=6 {
            let sub_layers: Vec<(bool, bool)> = (0..count).map(|i| (i % 2 == 0, i % 3 != 0)).collect();
            let mut w = BitWriter::default();
            write_profile_tier_level(&mut w, &sub_layers);
            let bytes = w.u(8, 0xa5).finish();
            let mut r = Bits::new(&bytes, false);
            assert_eq!(profile_tier_level(&mut r, count), Some(()));
            assert_eq!(r.bits(8), Some(0xa5), "{count} sub-layers");
        }
    }

    #[test]
    fn reference_picture_sets_are_derived_from_the_one_before() {
        let mut w = BitWriter::default();
        write_reference_picture_sets(&mut w);
        let bytes = w.u(8, 0xa5).finish();
        let mut r = Bits::new(&bytes, false);
        let first = short_term_ref_pic_set(&mut r, None).unwrap();
        let second = short_term_ref_pic_set(&mut r, Some(&first)).unwrap();
        let third = short_term_ref_pic_set(&mut r, Some(&second)).unwrap();
        let pictures = |set: &RefPicSet| (set.before[..set.n_before].to_vec(), set.after[..set.n_after].to_vec());
        assert_eq!(pictures(&first), (vec![-1, -3], vec![2]));
        assert_eq!(pictures(&second), (vec![-1], vec![1]));
        assert_eq!(pictures(&third), (vec![], vec![1, 2, 3]));
        assert_eq!(r.bits(8), Some(0xa5));
    }

    #[test]
    fn colour_tags_in_real_av1_sequence_headers() {
        for (what, header, want) in [
            ("BT.2020 PQ", AV1_PQ, tags(9, 16, 9, false)),
            ("BT.709", AV1_BT709, tags(1, 1, 1, false)),
            ("no colour description", AV1_NONE, Colour::unspecified(false)),
            ("full range", AV1_FULL, tags(1, 1, 1, true)),
            ("sRGB, whose full range is implied", AV1_RGB, tags(1, 13, 0, true)),
            ("monochrome, which has no chroma bits ahead of the range", AV1_MONO, tags(1, 1, 1, true)),
            ("profile 1 has no mono_chrome bit", AV1_444, tags(1, 1, 1, false)),
            ("profile 2 adds twelve_bit", AV1_422_10, tags(9, 16, 9, false)),
            ("timing info with equal_picture_interval", AV1_TIMING_CONST, tags(9, 16, 9, false)),
            ("decoder model and operating parameters", AV1_TIMING_MODEL, tags(9, 16, 9, false)),
            ("frame id numbers", AV1_FRAME_IDS, tags(9, 16, 9, false)),
            ("SVT-AV1", AV1_SVT_PQ, tags(9, 16, 9, false)),
        ] {
            assert_eq!(av1_colour(&hex(header)), Some(want), "{what}");
        }
    }

    #[test]
    fn colour_tags_in_av1_sequence_header_syntax_no_encoder_here_writes() {
        assert_eq!(av1_colour(&reduced_still_picture_header()), Some(tags(9, 16, 9, true)));
        assert_eq!(av1_colour(&operating_points_and_forced_tools()), Some(tags(9, 16, 9, false)));
    }

    #[test]
    fn the_hevc_sps_is_found_after_other_units_and_not_after_a_slice() {
        let (vps, sps, pps) = (hex(VPS), hex(SPS_PQ), hex(PPS));
        let (aud, slice) = ([0x46, 0x01, 0x50], [0x26, 0x01, 0xaf]);
        // Three and four byte start codes; the zero byte of a four byte one is not the previous unit's.
        let au = [&[0, 0, 1][..], &aud, &[0, 0, 0, 1], &vps, &[0, 0, 1], &sps, &[0, 0, 0, 1], &pps, &[0, 0, 0, 1], &slice].concat();
        assert_eq!(hevc_sps(&au), Some(&sps[..]));
        assert_eq!(hevc_sps(&[&[0, 0, 1][..], &sps, &[0, 0, 0]].concat()), Some(&sps[..]), "trailing zero bytes");
        assert_eq!(hevc_sps(&[&[0, 0, 1][..], &slice, &[0, 0, 1], &sps].concat()), None, "after the first slice");
        let mut enhancement_layer = sps.clone();
        enhancement_layer[1] = 0x09; // nuh_layer_id 1
        assert_eq!(hevc_sps(&[&[0, 0, 1][..], &enhancement_layer].concat()), None);
        for nothing in [&[][..], &[1, 2, 3], &[0, 0, 1], &[0, 0, 1, 0x42]] {
            assert_eq!(hevc_sps(nothing), None);
        }
    }

    #[test]
    fn the_av1_sequence_header_is_found_through_the_obu_headers() {
        let header = hex(AV1_PQ);
        let sequence_header = [&[0x0a, u8::try_from(header.len()).unwrap()][..], &header].concat();
        // A temporal delimiter with an extension byte, then padding with a two byte leb128 size.
        let padding = [&[0x7a, 0x81, 0x01][..], &[0; 129]].concat();
        let tu = [&[0x16, 0x00, 0x00][..], &padding, &sequence_header, &[0x32, 0x01, 0xaa]].concat();
        assert_eq!(av1_sequence_header(&tu), Some(&header[..]));
        // Without a size field the sequence header runs to the end of the unit.
        assert_eq!(av1_sequence_header(&[&[0x12, 0x00, 0x08][..], &header].concat()), Some(&header[..]));
        assert_eq!(av1_sequence_header(&[0x12, 0x00, 0x32, 0x01, 0xaa]), None, "no sequence header");
        assert_eq!(av1_sequence_header(&[0x12, 0x00, 0x0a, 0x20, 1, 2, 3]), None, "size past the end");
        assert_eq!(av1_sequence_header(&[0x82, 0x00]), None, "forbidden bit");
        assert_eq!(av1_sequence_header(&[0x0a, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80]), None, "leb128 too long");
    }

    #[test]
    fn emulation_prevention_bytes_are_not_data() {
        let escaped = [0, 0, 3, 1, 0, 0, 3, 0, 0xff];
        let mut unescaping = Bits::new(&escaped, true);
        assert_eq!(std::iter::from_fn(|| unescaping.byte()).collect::<Vec<_>>(), [0, 0, 1, 0, 0, 0, 0xff]);
        let mut raw = Bits::new(&escaped, false);
        assert_eq!(std::iter::from_fn(|| raw.byte()).count(), escaped.len());
        assert_eq!(Bits::new(&[0; 400], true).skip(3200), Some(()), "a long run of zeros is data");
    }

    #[test]
    fn exp_golomb_codes_round_trip_up_to_32_bits() {
        for value in [0, 1, 2, 3, 6, 7, 1920, 65_535, u32::MAX - 1] {
            let bytes = BitWriter::default().ue(value).finish();
            assert_eq!(Bits::new(&bytes, false).ue(), Some(value));
        }
        assert_eq!(Bits::new(&BitWriter::default().ue(u32::MAX).finish(), false).ue(), None);
    }

    /// Reads every truncation and every single-bit corruption of `unit`. None may panic, and a cut-off unit is never read as another colour.
    fn survives_damage(unit: &[u8], read: fn(&[u8]) -> Option<Colour>) {
        let whole = read(unit);
        assert!(whole.is_some());
        for n in 0..unit.len() {
            let cut = read(&unit[..n]);
            assert!(cut.is_none() || cut == whole, "cut at {n}");
        }
        for bit in 0..unit.len() * 8 {
            let mut damaged = unit.to_vec();
            damaged[bit / 8] ^= 0x80 >> (bit % 8);
            let _ = read(&damaged);
        }
    }

    #[test]
    fn damaged_input_never_panics_and_a_cut_off_unit_is_not_read() {
        for sps in [hex(SPS_PQ), hex(SPS_SCALING_LISTS), sps_with_every_branch(vui_with_full_range_pq)] {
            survives_damage(&sps, hevc_colour);
        }
        for header in [hex(AV1_PQ), hex(AV1_TIMING_MODEL), reduced_still_picture_header(), operating_points_and_forced_tools()] {
            survives_damage(&header, av1_colour);
        }
        survives_damage(&hevc_keyframe(SPS_PQ), |au| hevc_sps(au).and_then(hevc_colour));
        survives_damage(&av1_keyframe(AV1_PQ), |tu| av1_sequence_header(tu).and_then(av1_colour));
        // Bytes from a small alphabet, so that start codes, NAL unit headers and OBU headers turn up.
        let mut seed = 1u32;
        for _ in 0..500 {
            let junk: Vec<u8> = (0..96)
                .map(|_| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    [0, 0, 1, 3, 0x42, 0x01, 0x0a, 0x12, 0x80, 0xff][(seed >> 16) as usize % 10]
                })
                .collect();
            let _ = (hevc_sps(&junk).and_then(hevc_colour), av1_sequence_header(&junk).and_then(av1_colour));
            let _ = (hevc_colour(&junk), av1_colour(&junk));
        }
    }

    #[test]
    fn a_stream_reports_its_colour_on_the_first_keyframe_and_again_only_when_it_changes() {
        let pq = r#"{"stream":"colour","codec":"h265","primaries":9,"transfer":16,"matrix":9,"full_range":false}"#;
        let hlg = r#"{"stream":"colour","codec":"h265","primaries":9,"transfer":18,"matrix":9,"full_range":false}"#;
        let watch = ColourWatch::default();
        let keyframe = |sps| frame(MediaCodec::H265, hevc_keyframe(sps), true);
        assert_eq!(observed(&watch, &keyframe(SPS_PQ)), [pq]);
        assert!(observed(&watch, &keyframe(SPS_PQ)).is_empty(), "the same parameter set again");
        assert!(observed(&watch, &keyframe(SPS_PQ_CTU16)).is_empty(), "another parameter set with the same tags");
        assert_eq!(observed(&watch, &keyframe(SPS_HLG)), [hlg]);
        assert_eq!(observed(&watch, &keyframe(SPS_PQ)), [pq], "back to the first");
    }

    #[test]
    fn av1_keyframes_report_through_the_same_line() {
        let rgb = r#"{"stream":"colour","codec":"av1","primaries":1,"transfer":13,"matrix":0,"full_range":true}"#;
        let watch = ColourWatch::default();
        let keyframe = |header| frame(MediaCodec::Av1, av1_keyframe(header), true);
        assert_eq!(observed(&watch, &keyframe(AV1_RGB)), [rgb]);
        assert!(observed(&watch, &keyframe(AV1_RGB)).is_empty());
        assert_eq!(observed(&watch, &keyframe(AV1_HLG)).len(), 1);
        assert_eq!(observed(&watch, &keyframe(AV1_SVT_PQ)).len(), 1);
        assert!(observed(&watch, &keyframe(AV1_PQ)).is_empty(), "another sequence header with the same tags");
    }

    #[test]
    fn only_h265_and_av1_keyframes_are_read() {
        let watch = ColourWatch::default();
        assert!(observed(&watch, &frame(MediaCodec::H265, hevc_keyframe(SPS_PQ), false)).is_empty(), "a delta frame");
        assert!(observed(&watch, &frame(MediaCodec::Av1, av1_keyframe(AV1_PQ), false)).is_empty(), "a delta frame");
        assert!(observed(&watch, &frame(MediaCodec::H264, hevc_keyframe(SPS_PQ), true)).is_empty(), "H.264");
        assert!(observed(&watch, &frame(MediaCodec::Opus { channels: 2 }, hevc_keyframe(SPS_PQ), true)).is_empty(), "audio");
        // None of them used up the report.
        assert_eq!(observed(&watch, &frame(MediaCodec::H265, hevc_keyframe(SPS_PQ), true)).len(), 1);
    }

    #[test]
    fn a_parameter_set_that_cannot_be_read_does_not_block_the_next() {
        let watch = ColourWatch::default();
        let unreadable = frame(MediaCodec::H265, annex_b(&[&[0x42, 0x01, 0xff, 0xff]]), true);
        let no_parameter_sets = frame(MediaCodec::H265, annex_b(&[&[0x26, 0x01, 0xaf]]), true);
        assert!(observed(&watch, &unreadable).is_empty());
        assert!(observed(&watch, &no_parameter_sets).is_empty());
        assert_eq!(observed(&watch, &frame(MediaCodec::H265, hevc_keyframe(SPS_PQ), true)).len(), 1);
    }

    #[test]
    fn the_transfer_characteristic_alone_picks_the_hdr_mode() {
        for (transfer, mode) in [(16, HdrMode::Hdr10), (18, HdrMode::Hlg), (1, HdrMode::Sdr), (2, HdrMode::Sdr), (13, HdrMode::Sdr), (14, HdrMode::Sdr), (15, HdrMode::Sdr), (0, HdrMode::Sdr), (255, HdrMode::Sdr)] {
            assert_eq!(HdrMode::of(transfer), mode, "transfer {transfer}");
        }
    }

    #[test]
    fn observing_a_new_colour_returns_the_mode_its_tags_ask_for_and_nothing_for_a_repeat() {
        let hevc = ColourWatch::default();
        let hevc_mode = |sps| hevc.observe(&frame(MediaCodec::H265, hevc_keyframe(sps), true), |_| {}).map(Colour::hdr_mode);
        assert_eq!(hevc_mode(SPS_PQ), Some(HdrMode::Hdr10));
        assert_eq!(hevc_mode(SPS_PQ), None, "the same parameter set again");
        assert_eq!(hevc_mode(SPS_HLG), Some(HdrMode::Hlg));
        assert_eq!(hevc_mode(SPS_BT709), Some(HdrMode::Sdr));
        assert_eq!(hevc_mode(SPS_FULL_NO_DESCRIPTION), Some(HdrMode::Sdr), "tags that say nothing are SDR");
        let av1 = ColourWatch::default();
        let av1_mode = |header| av1.observe(&frame(MediaCodec::Av1, av1_keyframe(header), true), |_| {}).map(Colour::hdr_mode);
        assert_eq!([av1_mode(AV1_PQ), av1_mode(AV1_HLG), av1_mode(AV1_BT709)], [Some(HdrMode::Hdr10), Some(HdrMode::Hlg), Some(HdrMode::Sdr)]);
        assert_eq!(hevc.observe(&frame(MediaCodec::H265, hevc_keyframe(SPS_PQ), false), |_| {}), None, "a delta frame");
    }
}
