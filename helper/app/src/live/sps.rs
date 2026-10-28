//! The picture size from a stream's own parameter sets: what the feeder's load needs when the stream comes as bare access units (the
//! relay) and no container says it. H.264 (ITU-T H.264 7.3.2.1.1) and H.265 (ITU-T H.265 7.3.2.2.1), only as far as the size.

use super::fmp4::Codec;

/// A reader of bits over a NAL unit's payload with the emulation-prevention bytes taken out.
struct Bits {
    data: Vec<u8>,
    at: usize,
}

impl Bits {
    fn new(nal: &[u8]) -> Self {
        let mut data = Vec::with_capacity(nal.len());
        let mut zeros = 0;
        for &b in nal {
            if zeros >= 2 && b == 3 {
                zeros = 0;
                continue;
            }
            data.push(b);
            zeros = if b == 0 { zeros + 1 } else { 0 };
        }
        Self { data, at: 0 }
    }

    fn bit(&mut self) -> Option<u32> {
        let byte = *self.data.get(self.at / 8)?;
        let bit = (byte >> (7 - (self.at % 8))) & 1;
        self.at += 1;
        Some(u32::from(bit))
    }

    fn bits(&mut self, n: u32) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | self.bit()?;
        }
        Some(v)
    }

    fn skip(&mut self, n: usize) -> Option<()> {
        if self.at + n > self.data.len() * 8 {
            return None;
        }
        self.at += n;
        Some(())
    }

    /// Exp-Golomb, unsigned.
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
        let rest = self.bits(zeros)?;
        Some((1u32 << zeros) - 1 + rest)
    }

    /// Exp-Golomb, signed (the value is not needed, only its width).
    fn se(&mut self) -> Option<()> {
        self.ue().map(|_| ())
    }
}

/// The NAL units of Annex B data (the bytes between start codes).
pub fn nal_units(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut start: Option<usize> = None;
    while i + 2 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            if let Some(s) = start {
                let mut end = i;
                while end > s && data[end - 1] == 0 {
                    end -= 1;
                }
                out.push(&data[s..end]);
            }
            i += 3;
            start = Some(i);
            continue;
        }
        i += 1;
    }
    if let Some(s) = start {
        let mut end = data.len();
        while end > s && data[end - 1] == 0 {
            end -= 1;
        }
        if end > s {
            out.push(&data[s..end]);
        }
    }
    out
}

/// The width and height of the pictures the SPS in `data` (Annex B) describes; `None` when there is no SPS it can read.
pub fn dimensions(codec: Codec, data: &[u8]) -> Option<(u32, u32)> {
    for nal in nal_units(data) {
        let found = match codec {
            Codec::H264 if nal.first().is_some_and(|b| b & 0x1F == 7) => h264(&nal[1..]),
            Codec::H265 if nal.first().is_some_and(|b| (b >> 1) & 0x3F == 33) => h265(&nal[2..]),
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

fn h264(payload: &[u8]) -> Option<(u32, u32)> {
    let mut b = Bits::new(payload);
    let profile = b.bits(8)?;
    b.skip(16)?; // constraint flags, level
    b.ue()?; // sps id
    let mut chroma_format = 1;
    if matches!(
        profile,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    ) {
        chroma_format = b.ue()?;
        if chroma_format == 3 {
            b.skip(1)?; // separate colour planes
        }
        b.ue()?; // bit depth luma
        b.ue()?; // bit depth chroma
        b.skip(1)?; // qpprime
        if b.bit()? == 1 {
            // scaling matrices: each present list is a run of deltas
            let lists = if chroma_format != 3 { 8 } else { 12 };
            for i in 0..lists {
                if b.bit()? == 1 {
                    let size = if i < 6 { 16 } else { 64 };
                    let mut last = 8i32;
                    let mut next = 8i32;
                    for _ in 0..size {
                        if next != 0 {
                            let delta = b.ue()?;
                            let signed = if delta & 1 == 1 {
                                (delta as i32 + 1) / 2
                            } else {
                                -((delta / 2) as i32)
                            };
                            next = (last + signed + 256) % 256;
                        }
                        last = if next == 0 { last } else { next };
                    }
                }
            }
        }
    }
    b.ue()?; // log2 max frame num
    let poc_type = b.ue()?;
    if poc_type == 0 {
        b.ue()?;
    } else if poc_type == 1 {
        b.skip(1)?;
        b.se()?;
        b.se()?;
        let n = b.ue()?;
        for _ in 0..n {
            b.se()?;
        }
    }
    b.ue()?; // max ref frames
    b.skip(1)?; // gaps allowed
    let width_mbs = b.ue()? + 1;
    let height_units = b.ue()? + 1;
    let frame_mbs_only = b.bit()?;
    if frame_mbs_only == 0 {
        b.skip(1)?;
    }
    b.skip(1)?; // direct 8x8
    let (mut left, mut right, mut top, mut bottom) = (0, 0, 0, 0);
    if b.bit()? == 1 {
        left = b.ue()?;
        right = b.ue()?;
        top = b.ue()?;
        bottom = b.ue()?;
    }
    let (unit_x, unit_y) = match chroma_format {
        0 | 3 => (1, 2 - frame_mbs_only),
        2 => (2, 2 - frame_mbs_only),
        _ => (2, 2 * (2 - frame_mbs_only)),
    };
    let width = width_mbs * 16 - unit_x * (left + right);
    let height = (2 - frame_mbs_only) * height_units * 16 - unit_y * (top + bottom);
    (width > 0 && height > 0).then_some((width, height))
}

fn h265(payload: &[u8]) -> Option<(u32, u32)> {
    let mut b = Bits::new(payload);
    b.skip(4)?; // vps id
    let max_sub_layers = b.bits(3)?;
    b.skip(1)?; // temporal id nesting
    // profile_tier_level: the general part, then the sub-layers' flags and their parts
    b.skip(2 + 1 + 5 + 32 + 4 + 43 + 1 + 8)?;
    let mut profile_present = [false; 8];
    let mut level_present = [false; 8];
    for i in 0..max_sub_layers as usize {
        profile_present[i] = b.bit()? == 1;
        level_present[i] = b.bit()? == 1;
    }
    if max_sub_layers > 0 {
        for _ in max_sub_layers..8 {
            b.skip(2)?;
        }
    }
    for i in 0..max_sub_layers as usize {
        if profile_present[i] {
            b.skip(2 + 1 + 5 + 32 + 4 + 43 + 1)?;
        }
        if level_present[i] {
            b.skip(8)?;
        }
    }
    b.ue()?; // sps id
    let chroma_format = b.ue()?;
    if chroma_format == 3 {
        b.skip(1)?;
    }
    let mut width = b.ue()?;
    let mut height = b.ue()?;
    if b.bit()? == 1 {
        let (left, right, top, bottom) = (b.ue()?, b.ue()?, b.ue()?, b.ue()?);
        let (unit_x, unit_y) = match chroma_format {
            1 => (2, 2),
            2 => (2, 1),
            _ => (1, 1),
        };
        width = width.saturating_sub(unit_x * (left + right));
        height = height.saturating_sub(unit_y * (top + bottom));
    }
    (width > 0 && height > 0).then_some((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_size_of_the_fixtures_h264_stream_is_read_from_its_sps() {
        // the HLS fixture's init segment carries the SPS in avcC form: its parameter sets in Annex B are what parse_init gives
        let init = include_bytes!("../../tests/fixtures/hls/init.mp4");
        let parsed = super::super::fmp4::parse_init(init).unwrap();
        assert_eq!(
            dimensions(Codec::H264, &parsed.parameter_sets),
            Some((320, 180))
        );
        // libx264, baseline, 1920x1080 (1920x1088 in macroblocks, 8 lines cropped)
        let sps = [
            0x67, 0x42, 0xC0, 0x28, 0xD9, 0x00, 0x78, 0x02, 0x27, 0xE5, 0xC0, 0x44, 0x00, 0x00,
            0x03, 0x00, 0x04, 0x00, 0x00, 0x03, 0x00, 0xF0, 0x3C, 0x60, 0xC9, 0x20,
        ];
        let mut data = vec![0, 0, 0, 1];
        data.extend_from_slice(&sps);
        assert_eq!(dimensions(Codec::H264, &data), Some((1920, 1080)));
        assert_eq!(dimensions(Codec::H264, b"\x00\x00\x00\x01\x65junk"), None);
        assert_eq!(dimensions(Codec::H264, &[]), None);
    }

    #[test]
    fn nal_units_are_split_at_start_codes_of_either_length() {
        let data = b"\x00\x00\x00\x01\x67a\x00\x00\x01\x68b\x00\x00\x00\x01\x65c\x00";
        assert_eq!(
            nal_units(data),
            vec![&b"\x67a"[..], &b"\x68b"[..], &b"\x65c"[..]]
        );
    }

    #[test]
    fn an_hevc_sps_gives_its_size() {
        // libx265 Main, 320x180 (coded 320x184 with a conformance window of 4 lines)
        let sps = [
            0x42, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x90, 0x00, 0x00, 0x03, 0x00,
            0x00, 0x03, 0x00, 0x3C, 0xA0, 0x0A, 0x08, 0x0B, 0x9F, 0x79, 0x65, 0x66, 0x92, 0x4C,
            0xAF, 0x01, 0x68, 0x08, 0x00, 0x00, 0x03, 0x00, 0x08, 0x00, 0x00, 0x03, 0x00, 0xF0,
            0x40,
        ];
        let mut data = vec![0, 0, 0, 1];
        data.extend_from_slice(&sps);
        assert_eq!(dimensions(Codec::H265, &data), Some((320, 180)));
    }
}
