//! The frame the 32-bit capture helper hands to the 64-bit helper over a local socket: already reduced to one colour per
//! area, so the 24 KB of raw histograms per poll never leave the capture process. Little-endian, fixed size per frame.
//!
//! ```text
//!   0   u32  magic "AMB1"
//!   4   u8   preset (0..3)
//!   5   u8   areas (16, or 14 for preset 3)
//!   6   u16  reserved, 0
//!   8   u32  capture time in ms on the capture helper's monotonic clock
//!   12  per area, 12 bytes: u16 r, u16 g, u16 b (mean code value / 255 * 65535), u16 black (fraction * 65535), u32 pixel count
//! ```
//!
//! 12 + 16 * 12 = 204 bytes per frame; at 10 polls a second that is 2 KB/s. The helper maps the capture clock to its
//! own by the difference at the first frame (both are monotonic; only differences are used).

use crate::rgb::Rgb;
use crate::zones::{MAX_ZONES, Preset, ZoneFrame};

pub const MAGIC: u32 = u32::from_le_bytes(*b"AMB1");
pub const HEADER: usize = 12;
pub const ZONE_BYTES: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireError {
    Short,
    Magic,
    Preset(u8),
    Areas { preset: u8, areas: u8 },
    Length { expected: usize, got: usize },
}

pub fn frame_len(preset: Preset) -> usize {
    HEADER + preset.zone_count() * ZONE_BYTES
}

fn q16(v: f32) -> u16 {
    (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16
}

pub fn encode(f: &ZoneFrame, t_ms: u32) -> Vec<u8> {
    let n = f.zone_count();
    let mut b = Vec::with_capacity(frame_len(f.preset));
    b.extend_from_slice(&MAGIC.to_le_bytes());
    b.push(f.preset.index());
    b.push(n as u8);
    b.extend_from_slice(&[0, 0]);
    b.extend_from_slice(&t_ms.to_le_bytes());
    for i in 0..n {
        for v in [f.color[i].r, f.color[i].g, f.color[i].b, f.black[i]] {
            b.extend_from_slice(&q16(v).to_le_bytes());
        }
        b.extend_from_slice(&f.count[i].to_le_bytes());
    }
    b
}

/// Decodes exactly one frame; returns it with its capture time.
pub fn decode(b: &[u8]) -> Result<(ZoneFrame, u32), WireError> {
    if b.len() < HEADER {
        return Err(WireError::Short);
    }
    if u32::from_le_bytes(b[0..4].try_into().expect("4 bytes")) != MAGIC {
        return Err(WireError::Magic);
    }
    let preset = Preset::from_index(b[4]).ok_or(WireError::Preset(b[4]))?;
    if usize::from(b[5]) != preset.zone_count() {
        return Err(WireError::Areas {
            preset: b[4],
            areas: b[5],
        });
    }
    let expected = frame_len(preset);
    if b.len() != expected {
        return Err(WireError::Length {
            expected,
            got: b.len(),
        });
    }
    let t_ms = u32::from_le_bytes(b[8..12].try_into().expect("4 bytes"));
    let mut f = ZoneFrame::blank(preset);
    for i in 0..preset.zone_count().min(MAX_ZONES) {
        let z = &b[HEADER + i * ZONE_BYTES..HEADER + (i + 1) * ZONE_BYTES];
        let u = |k: usize| f32::from(u16::from_le_bytes([z[k], z[k + 1]])) / 65535.0;
        f.color[i] = Rgb::new(u(0), u(2), u(4));
        f.black[i] = u(6);
        f.count[i] = u32::from_le_bytes(z[8..12].try_into().expect("4 bytes"));
    }
    Ok((f, t_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(preset: Preset) -> ZoneFrame {
        let mut f = ZoneFrame::blank(preset);
        for i in 0..preset.zone_count() {
            f.color[i] = Rgb::new(i as f32 / 20.0, 0.5, 1.0 - i as f32 / 40.0);
            f.black[i] = i as f32 / 16.0;
            f.count[i] = 1000 + i as u32;
        }
        f
    }

    #[test]
    fn frames_round_trip_within_16_bit_precision() {
        for p in Preset::ALL {
            let f = sample(p);
            let raw = encode(&f, 123_456);
            assert_eq!(raw.len(), frame_len(p));
            let (g, t) = decode(&raw).unwrap();
            assert_eq!((g.preset, t), (p, 123_456));
            for i in 0..p.zone_count() {
                assert!(f.color[i].max_delta(g.color[i]) < 1.0 / 65535.0 + 1e-6);
                assert!((f.black[i] - g.black[i]).abs() < 1.0 / 65535.0 + 1e-6);
                assert_eq!(f.count[i], g.count[i]);
            }
        }
        assert_eq!(frame_len(Preset::Plain), 204);
        assert_eq!(frame_len(Preset::Games), 180);
    }

    #[test]
    fn layout_is_the_documented_one() {
        let raw = encode(&sample(Preset::Plain), 0x0102_0304);
        assert_eq!(&raw[..4], b"AMB1");
        assert_eq!(&raw[4..8], &[0, 16, 0, 0]);
        assert_eq!(&raw[8..12], &[4, 3, 2, 1]);
        // Zone 1: r = 0.05 -> 3277; count 1001.
        assert_eq!(u16::from_le_bytes([raw[24], raw[25]]), 3277);
        assert_eq!(u32::from_le_bytes(raw[32..36].try_into().unwrap()), 1001);
    }

    #[test]
    fn bad_frames_are_named() {
        let raw = encode(&sample(Preset::Plain), 1);
        assert_eq!(decode(&raw[..5]).err(), Some(WireError::Short));
        let mut m = raw.clone();
        m[0] = b'X';
        assert_eq!(decode(&m).err(), Some(WireError::Magic));
        let mut p = raw.clone();
        p[4] = 9;
        assert_eq!(decode(&p).err(), Some(WireError::Preset(9)));
        let mut a = raw.clone();
        a[5] = 14;
        assert_eq!(
            decode(&a).err(),
            Some(WireError::Areas {
                preset: 0,
                areas: 14
            })
        );
        assert_eq!(
            decode(&raw[..raw.len() - 1]).err(),
            Some(WireError::Length {
                expected: 204,
                got: 203
            })
        );
    }
}
