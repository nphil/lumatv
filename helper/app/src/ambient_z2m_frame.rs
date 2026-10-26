//! Hue's Zigbee entertainment frames, the way Bifrost packs them for Zigbee2MQTT (cluster `0xFC01`, manufacturer `0x100B`).
//!
//! Everything here is pure and reproduces Bifrost (`crates/hue/src/zigbee/{entertainment,stream}.rs`, `xy.rs`, `colorspace.rs`,
//! `src/backend/z2m/{entertainment,zclcommand}.rs` at `12d9e37`) byte for byte; the unit tests hold it to the values Bifrost's own
//! code produced (`tests/fixtures/bifrost-vectors.jsonl`). The notes are `docs/research/z2m-direct-result.md`.
//!
//! ```text
//! command 1 (a frame):   counter u32 LE | smoothing u16 LE | per light: addr u16 LE, (brightness << 5 | mode) u16 LE, xy 3 bytes
//! command 3 (sync):      0x00 0x01 counter u32 LE          told to every bulb before the first frame and after the last
//! command 7 (segments):  count u16 BE | addr u16 LE each    a gradient strip's virtual addresses, nwk + 0, nwk + 1, ...
//! attribute 5 (balance): u8, 0xFE = full                    written once per bulb before a stream
//! ```
//!
//! Colour: the 16-bit RGB of the engine is cut to 8 bits, taken as linear through Hue's wide-gamut matrix to CIE xy, and the
//! brightness is `Y` against the brightest `Y` the gamut allows at that xy, on 11 bits from 1 to 2047. Black has no chromaticity
//! and becomes the D50 white point at brightness 1: a frame cannot switch a bulb off, only dim it to its least.

use serde_json::{Value as Json, json};
use std::time::Duration;

/// The entertainment cluster and the commands used on it.
pub const CLUSTER: u16 = 0xFC01;
pub const CMD_FRAME: u8 = 1;
pub const CMD_RESET: u8 = 3;
pub const CMD_SEGMENT_MAP: u8 = 7;
/// The light-balance attribute of the cluster, and the value for "as bright as the bulb goes".
pub const ATTR_LIGHT_BALANCE: u8 = 5;
pub const BALANCE_FULL: u8 = 0xFE;
/// Signify's Zigbee manufacturer code.
pub const VENDOR: u16 = 0x100B;
/// The ZCL data type code of an unsigned 8-bit value.
const ZCL_U8: u8 = 0x20;
/// What Hue's own clients use for the fade between frames.
pub const DEFAULT_SMOOTHING: u16 = 0x0400;
/// A smoothing of 0xFFFF is a fade of 2.56 s (Bifrost, measured).
const SMOOTHING_MAX_MICROS: u64 = 2_560_000;
/// Hue's frames carry at most this many records (Bifrost's notes); a longer frame does not fit a Zigbee message.
pub const MAX_RECORDS: usize = 10;
/// The ZCL payload of a frame with `n` records.
pub const fn frame_len(records: usize) -> usize {
    6 + 7 * records
}

/// Which kind of thing a record addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// A bulb, by its network address.
    Device = 0b01011,
    /// One segment of a gradient strip, by the virtual address command 7 gave it.
    Segment = 0b00000,
}

/// One light's part of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    pub addr: u16,
    /// 1..=2047.
    pub brightness: u16,
    pub mode: Mode,
    /// The quantised xy.
    pub raw: [u8; 3],
}

impl Record {
    pub fn pack(&self) -> [u8; 7] {
        let field = (self.brightness.min(2047) << 5) | (self.mode as u16);
        let [a0, a1] = self.addr.to_le_bytes();
        let [f0, f1] = field.to_le_bytes();
        [a0, a1, f0, f1, self.raw[0], self.raw[1], self.raw[2]]
    }
}

// Hue's wide-gamut colour space as Bifrost has it (from viereck.ch/hue-xy-rgb): RGB to XYZ and back, no gamma.
const RGB_TO_XYZ: [f64; 9] = [
    0.7164, 0.1010, 0.1468, //
    0.2587, 0.7247, 0.0166, //
    0.0000, 0.0512, 0.7740, //
];
const XYZ_TO_RGB: [f64; 9] = [
    1.4625, -0.1845, -0.2734, //
    -0.5229, 1.4479, 0.0681, //
    0.0346, -0.0958, 1.2875, //
];
const WIDE_GAMUT_MAX_X: f64 = 0.7347;
const WIDE_GAMUT_MAX_Y: f64 = 0.8264;
const D50_WHITE_POINT: (f64, f64) = (0.34567, 0.35850);

#[allow(clippy::suboptimal_flops)] // The exact order of Bifrost's arithmetic, so the bytes agree.
fn mult(m: &[f64; 9], d: [f64; 3]) -> [f64; 3] {
    [
        d[0] * m[0] + d[1] * m[1] + d[2] * m[2],
        d[0] * m[3] + d[1] * m[4] + d[2] * m[5],
        d[0] * m[6] + d[1] * m[7] + d[2] * m[8],
    ]
}

/// The brightest `Y` the gamut allows at (x, y): Bifrost's ten rounds of dividing by the largest component.
fn find_maximum_y(x: f64, y: f64) -> f64 {
    let mut bri = 1.0;
    for _ in 0..10 {
        let z = 1.0 - x - y;
        let [r, g, b] = mult(&XYZ_TO_RGB, [(bri / y) * x, bri, (bri / y) * z]);
        let max = r.max(g).max(b);
        bri /= max;
    }
    bri
}

/// (x, y, brightness 0..=255) of an 8-bit RGB, as Bifrost's `XY::from_rgb`.
pub fn xy_of(red: u8, green: u8, blue: u8) -> (f64, f64, f64) {
    let unit = |v: u8| f64::from(v) / 255.0;
    let [cx, cy, cz] = mult(&RGB_TO_XYZ, [unit(red), unit(green), unit(blue)]);
    let x = cx / (cx + cy + cz);
    let y = cy / (cx + cy + cz);
    let max_y = find_maximum_y(x, y);
    if max_y > f64::EPSILON {
        (x, y, (cy / max_y * 255.0).min(255.0))
    } else {
        (D50_WHITE_POINT.0, D50_WHITE_POINT.1, 0.0)
    }
}

/// xy on 12 bits each, packed as Hue's three bytes.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn quantise(x: f64, y: f64) -> [u8; 3] {
    let x = ((x * f64::from(0xFFF)) / WIDE_GAMUT_MAX_X) as u16;
    let y = ((y * f64::from(0xFFF)) / WIDE_GAMUT_MAX_Y) as u16;
    [
        (x & 0xFF) as u8,
        (((x >> 8) & 0x0F) | ((y & 0x0F) << 4)) as u8,
        ((y >> 4) & 0xFF) as u8,
    ]
}

/// The record for a light at a 16-bit RGB, as Bifrost makes it from a HueStream RGB channel.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn record(addr: u16, rgb: [u16; 3], mode: Mode) -> Record {
    let (x, y, bright) = xy_of(
        (rgb[0] / 256) as u8,
        (rgb[1] / 256) as u8,
        (rgb[2] / 256) as u8,
    );
    let brightness = (bright / 255.0 * 2047.0).clamp(1.0, 2047.0) as u16;
    Record {
        addr,
        brightness,
        mode,
        raw: quantise(x, y),
    }
}

/// The frame's ZCL payload.
pub fn frame(counter: u32, smoothing: u16, records: &[Record]) -> Vec<u8> {
    let mut out = Vec::with_capacity(frame_len(records.len()));
    out.extend_from_slice(&counter.to_le_bytes());
    out.extend_from_slice(&smoothing.to_le_bytes());
    for r in records {
        out.extend_from_slice(&r.pack());
    }
    out
}

/// Command 3: the counter the bulbs should expect frames from.
pub fn reset(counter: u32) -> Vec<u8> {
    let mut out = vec![0x00, 0x01];
    out.extend_from_slice(&counter.to_le_bytes());
    out
}

/// Command 7: the virtual addresses of a strip's segments, in order.
pub fn segment_map(addrs: &[u16]) -> Vec<u8> {
    let count = u16::try_from(addrs.len()).unwrap_or(u16::MAX);
    let mut out = Vec::with_capacity(2 + 2 * addrs.len());
    out.extend_from_slice(&count.to_be_bytes());
    for a in addrs {
        out.extend_from_slice(&a.to_le_bytes());
    }
    out
}

/// The smoothing for a fade as long as one frame interval (Bifrost's trick: transitions stay smooth at any rate).
#[allow(clippy::cast_possible_truncation)]
pub fn smoothing_for(interval: Duration) -> u16 {
    let us = u64::try_from(interval.as_micros())
        .unwrap_or(u64::MAX)
        .min(SMOOTHING_MAX_MICROS - 1);
    (us * 0x10000 / SMOOTHING_MAX_MICROS) as u16
}

/// Zigbee2MQTT's `zclcommand` payload for a command on the entertainment cluster, in Bifrost's shape.
pub fn zclcommand(command: u8, data: &[u8]) -> Json {
    json!({
        "zclcommand": {
            "cluster": CLUSTER,
            "command": command,
            "payload": {"data": data},
            "frametype": 1,
            "options": {
                "manufacturerCode": VENDOR,
                "disableDefaultResponse": true,
                "direction": 0,
                "timeout": 100.0,
            },
        }
    })
}

/// Zigbee2MQTT's `write` payload that sets the light balance (attribute 5) before a stream.
pub fn light_balance(value: u8) -> Json {
    json!({
        "write": {
            "cluster": CLUSTER,
            "payload": {
                ATTR_LIGHT_BALANCE.to_string(): {
                    "manufacturerCode": VENDOR,
                    "type": ZCL_U8,
                    "value": value,
                }
            },
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn vectors() -> Vec<Value> {
        let text = include_str!("../tests/fixtures/bifrost-vectors.jsonl");
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn every_colour_bifrost_packed_is_packed_the_same() {
        let mut seen = 0;
        for v in vectors().iter().filter(|v| v.get("rgb").is_some()) {
            let rgb: Vec<u16> = v["rgb"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| u16::try_from(n.as_u64().unwrap()).unwrap())
                .collect();
            let rgb = [rgb[0], rgb[1], rgb[2]];
            let dev = record(0xD297, rgb, Mode::Device);
            let seg = record(0xD297, rgb, Mode::Segment);
            assert_eq!(hex(&dev.pack()), v["device"], "{rgb:?}");
            assert_eq!(hex(&seg.pack()), v["segment"], "{rgb:?}");
            assert_eq!(hex(&dev.raw), v["raw"], "{rgb:?}");
            assert_eq!(u64::from(dev.brightness), v["brightness"], "{rgb:?}");
            seen += 1;
        }
        assert!(seen >= 20, "{seen}");
    }

    #[test]
    fn the_frame_the_reset_the_segment_map_and_the_smoothing_match_bifrost() {
        let vs = vectors();
        let find = |key: &str| vs.iter().find(|v| v.get(key).is_some()).unwrap().clone();
        let f = find("frame20");
        let records = [
            record(0x1001, [65535, 0, 0], Mode::Device),
            record(0x1002, [0, 65535, 0], Mode::Device),
            record(0x1003, [0, 0, 65535], Mode::Device),
        ];
        let smoothing = smoothing_for(Duration::from_micros(1_000_000 / 20));
        assert_eq!(u64::from(smoothing), f["smoothing20"]);
        assert_eq!(hex(&frame(0x1122_3344, smoothing, &records)), f["frame20"]);
        for v in vs.iter().filter(|v| v.get("fps").is_some()) {
            let fps = v["fps"].as_u64().unwrap();
            let s = smoothing_for(Duration::from_micros(1_000_000 / fps));
            assert_eq!(u64::from(s), v["smoothing"], "{fps} fps");
        }
        let r = find("reset");
        assert_eq!(hex(&reset(0x1122_3345)), r["reset"]);
        assert_eq!(u64::from(CLUSTER), r["cluster"]);
        assert_eq!(u64::from(CMD_RESET), r["cmd"]);
        assert_eq!(
            hex(&segment_map(&[0xD297, 0xD298, 0xD299])),
            find("segmap")["segmap"]
        );
    }

    #[test]
    fn bifrosts_own_unit_test_values() {
        // crates/hue/src/zigbee/entertainment.rs tests: light_record_device, light_record_segment, hue_ent_segment_config
        let r = Record {
            addr: 0x1122,
            brightness: 0x7FF,
            mode: Mode::Device,
            raw: [0xAA, 0xBB, 0xCC],
        };
        assert_eq!(hex(&r.pack()), "2211ebffaabbcc");
        let s = Record {
            mode: Mode::Segment,
            ..r
        };
        assert_eq!(hex(&s.pack()), "2211e0ffaabbcc");
        assert_eq!(
            segment_map(&[1, 2, 3, 4]),
            [0x00, 0x04, 1, 0, 2, 0, 3, 0, 4, 0]
        );
        // stream.rs tests: ezs_reset, ezs_frame (an empty frame is the header alone), duration_zero/half/max
        assert_eq!(reset(0x1122), [0x00, 0x01, 0x22, 0x11, 0x00, 0x00]);
        assert_eq!(frame(0x1122, 0x0400, &[]), [0x22, 0x11, 0, 0, 0x00, 0x04]);
        assert_eq!(smoothing_for(Duration::ZERO), 0);
        assert_eq!(smoothing_for(Duration::from_micros(1_280_000)), 0x8000);
        assert_eq!(smoothing_for(Duration::from_micros(2_559_999)), 0xFFFF);
        assert_eq!(smoothing_for(Duration::from_secs(9)), 0xFFFF);
    }

    #[test]
    fn black_is_the_dimmest_white_not_off_and_a_frame_has_a_known_size() {
        let r = record(1, [0, 0, 0], Mode::Device);
        assert_eq!(
            (r.brightness, r.raw),
            (1, quantise(D50_WHITE_POINT.0, D50_WHITE_POINT.1))
        );
        assert_eq!(frame(0, 0, &[r; 10]).len(), frame_len(MAX_RECORDS));
        assert_eq!(frame_len(4), 34);
    }

    #[test]
    fn the_json_is_bifrosts_shape() {
        let v = zclcommand(CMD_FRAME, &[0x22, 0x11, 0, 0, 0, 4]);
        assert_eq!(
            v,
            serde_json::json!({"zclcommand": {"cluster": 64513, "command": 1, "payload": {"data": [34, 17, 0, 0, 0, 4]},
                "frametype": 1, "options": {"manufacturerCode": 4107, "disableDefaultResponse": true, "direction": 0, "timeout": 100.0}}})
        );
        assert_eq!(
            light_balance(BALANCE_FULL),
            serde_json::json!({"write": {"cluster": 64513, "payload": {"5": {"manufacturerCode": 4107, "type": 32, "value": 254}}}})
        );
    }
}
